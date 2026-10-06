//! Dashboard oturum yöneticisi: kimlik → özel (havuz dışı) bağlantı. Her oturumun sorguları tek bir
//! tokio `Mutex` ile sıraya dizilir; bağlantı koptuysa sessizce yeniden açılır. SQL metinleri günlüğe yazılmaz.

use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicI32, Ordering};
use std::sync::{Arc, Mutex as StdMutex};

use pgt_core::connection::{ConnectionManager, SessionProfile, Target};
use pgt_core::{Error, Result};
use tokio_postgres::Client;
use uuid::Uuid;

use crate::queries;
use crate::types::{
    ActivityRow, DashId, DashInfo, LockReport, Overview, PreparedXact, Sample, Setting, SignalKind, SignalOutcome,
    Statements, StatementsOrder,
};

const CANCEL: &str = "SELECT pg_cancel_backend($1)";
const TERMINATE: &str = "SELECT pg_terminate_backend($1)";

/// Hata sonrası yeniden deneme ilkesi. Bağlantı yoksa/kapalıysa sorgudan önce yeniden açmak her zaman güvenlidir.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Retry {
    /// Salt okunur sorgular: bağlantı kopması hatasında bir kez yeniden denenir.
    Once,
    /// Yan etkili ifadeler (sinyal): ifade gönderildikten sonra asla yeniden çalıştırılmaz.
    BeforeSendOnly,
}

type BoxFut<'a, T> = Pin<Box<dyn Future<Output = Result<T>> + Send + 'a>>;

/// Canlı bağlantı. `DedicatedConn.notices` alıcısı bilerek düşürülür: pgt-core sürücüsü alıcısız kanala
/// gönderim hatasını yok sayar (bağlantı sürülmeye devam eder, panik/günlük yok).
struct Live {
    client: Client,
    version_num: u32,
}

struct DashSession {
    server: Uuid,
    target: Target,
    /// Veritabanı düzeyinde oturumda süzgeç adı; sunucu düzeyinde `None`.
    db_name: Option<String>,
    conn: tokio::sync::Mutex<Option<Live>>,
    self_pid: AtomicI32,
}

pub struct DashManager {
    manager: Arc<ConnectionManager>,
    /// Kilit asla `.await` boyunca tutulmaz: `Arc` klonlanıp bırakılır.
    sessions: StdMutex<HashMap<DashId, Arc<DashSession>>>,
}

impl DashManager {
    pub fn new(manager: Arc<ConnectionManager>) -> Self {
        Self { manager, sessions: StdMutex::new(HashMap::new()) }
    }

    /// Dashboard oturumu açar. `database_oid = Some(oid)` ise ad, bağlantı kurulunca `current_database()` ile
    /// okunur ve tüm sorgularda süzgeç olarak kullanılır.
    pub async fn open(&self, server: Uuid, database_oid: Option<u32>) -> Result<DashInfo> {
        let target = database_oid.map_or(Target::Maintenance, Target::Database);
        let live = self.connect(server, target).await?;
        let pid = backend_pid(&live.client).await?;
        let db_name = match database_oid {
            Some(_) => Some(
                live.client
                    .query_one("SELECT current_database()::text", &[])
                    .await
                    .map_err(Error::from_query)?
                    .get::<_, String>(0),
            ),
            None => None,
        };
        let id = Uuid::new_v4();
        let info = DashInfo { id, version_num: live.version_num, self_pid: pid, database: db_name.clone() };
        let session =
            DashSession { server, target, db_name, conn: tokio::sync::Mutex::new(Some(live)), self_pid: AtomicI32::new(pid) };
        self.sessions.lock().unwrap().insert(id, Arc::new(session));
        Ok(info)
    }

    /// Oturumu haritadan düşürür; bağlantı `Drop` ile kapanır.
    pub fn close(&self, id: DashId) {
        self.sessions.lock().unwrap().remove(&id);
    }

    /// Sunucu bağlantısı kesilince o sunucunun tüm dashboard oturumlarını düşürür.
    pub fn close_server(&self, server: Uuid) {
        self.sessions.lock().unwrap().retain(|_, s| s.server != server);
    }

    pub fn close_all(&self) {
        self.sessions.lock().unwrap().clear();
    }

    pub async fn sample(&self, id: DashId) -> Result<Sample> {
        let s = self.session(id)?;
        self.with_conn(&s, Retry::Once, |c, _, _, db| Box::pin(queries::sample(c, db))).await
    }

    pub async fn overview(&self, id: DashId) -> Result<Overview> {
        let s = self.session(id)?;
        self.with_conn(&s, Retry::Once, |c, _, _, db| Box::pin(queries::overview(c, db))).await
    }

    pub async fn activity(&self, id: DashId) -> Result<Vec<ActivityRow>> {
        let s = self.session(id)?;
        self.with_conn(&s, Retry::Once, |c, v, _, db| Box::pin(queries::activity(c, v, db))).await
    }

    pub async fn locks(&self, id: DashId) -> Result<LockReport> {
        let s = self.session(id)?;
        self.with_conn(&s, Retry::Once, |c, _, _, db| Box::pin(queries::locks(c, db))).await
    }

    pub async fn settings(&self, id: DashId) -> Result<Vec<Setting>> {
        let s = self.session(id)?;
        self.with_conn(&s, Retry::Once, |c, _, _, _| Box::pin(queries::settings(c))).await
    }

    pub async fn prepared(&self, id: DashId) -> Result<Vec<PreparedXact>> {
        let s = self.session(id)?;
        self.with_conn(&s, Retry::Once, |c, _, _, _| Box::pin(queries::prepared(c))).await
    }

    pub async fn statements(&self, id: DashId, order: StatementsOrder) -> Result<Statements> {
        let s = self.session(id)?;
        self.with_conn(&s, Retry::Once, |c, v, _, db| Box::pin(queries::statements(c, v, db, order))).await
    }

    /// Başka bir backend'e iptal/sonlandırma sinyali gönderir. Yetki hataları (42501) ve diğer SQL hataları
    /// `Error::Query` olarak yayılır; başarısız sinyal bağlantıyı bozmaz.
    pub async fn signal(&self, id: DashId, pid: i32, kind: SignalKind) -> Result<SignalOutcome> {
        let s = self.session(id)?;
        let sql = match kind {
            SignalKind::Cancel => CANCEL,
            SignalKind::Terminate => TERMINATE,
        };
        self.with_conn(&s, Retry::BeforeSendOnly, |c, _, self_pid, _| {
            Box::pin(async move {
                if pid == self_pid {
                    return Err(Error::Invalid("Kendi dashboard oturumunuzu durduramazsınız.".into()));
                }
                let row = c.query_one(sql, &[&pid]).await.map_err(Error::from_query)?;
                Ok(if row.get::<_, bool>(0) { SignalOutcome::Sent } else { SignalOutcome::NotFound })
            })
        })
        .await
    }

    fn session(&self, id: DashId) -> Result<Arc<DashSession>> {
        self.sessions.lock().unwrap().get(&id).cloned().ok_or_else(|| Error::NotFound(format!("Dashboard oturumu {id}")))
    }

    async fn connect(&self, server: Uuid, target: Target) -> Result<Live> {
        let conn = self.manager.open_dedicated(server, target, SessionProfile::Dashboard).await?;
        // `notices` alıcısı burada düşer (bkz. `Live`).
        Ok(Live { version_num: conn.info.version_num, client: conn.client })
    }

    /// Tüm sorgular buradan geçer: kilit tek uçuşu zorlar; bağlantı yoksa/kapalıysa yeniden açılır
    /// (açılamazsa hata döner, oturum kayıtlı kalır); bağlantı kopması tipi hatada bir kez yeniden denenir.
    async fn with_conn<T>(
        &self,
        s: &DashSession,
        retry: Retry,
        mut f: impl for<'a> FnMut(&'a Client, u32, i32, Option<&'a str>) -> BoxFut<'a, T>,
    ) -> Result<T> {
        let mut guard = s.conn.lock().await;
        for attempt in 0..2 {
            if guard.as_ref().is_none_or(|l| l.client.is_closed()) {
                *guard = None;
                let live = self.connect(s.server, s.target).await?;
                s.self_pid.store(backend_pid(&live.client).await?, Ordering::Relaxed);
                // Yeniden adlandırılmış veritabanı sessizce eski adla süzülmesin; oturum kayıtlı kalır.
                if let Some(name) = &s.db_name {
                    let now: String = live
                        .client
                        .query_one("SELECT current_database()::text", &[])
                        .await
                        .map_err(Error::from_query)?
                        .get(0);
                    if &now != name {
                        return Err(Error::NotFound(format!(
                            "Veritabanı adı değişmiş ({name} -> {now}); dashboard'u yeniden açın."
                        )));
                    }
                }
                *guard = Some(live);
            }
            let live = guard.as_ref().expect("bağlantı yukarıda kuruldu");
            match f(&live.client, live.version_num, s.self_pid.load(Ordering::Relaxed), s.db_name.as_deref()).await {
                Err(e) if attempt == 0 && should_retry(retry, live.client.is_closed(), &e) => {
                    tracing::debug!("dashboard bağlantısı koptu, yeniden açılıyor");
                    *guard = None;
                }
                other => return other,
            }
        }
        unreachable!("ikinci denemede sonuç her zaman döner")
    }
}

async fn backend_pid(c: &Client) -> Result<i32> {
    Ok(c.query_one("SELECT pg_backend_pid()", &[]).await.map_err(Error::from_query)?.get(0))
}

/// Hata sonrası yeniden deneme kararı. Yalnızca bağlantı kopması sayılır: istemci kapandıysa ya da SQLSTATE
/// sınıfı 08 / 57P01-03 ise. SQLSTATE'siz hata tek başına yetmez (istemci tarafı çözümleme hataları da öyledir).
fn should_retry(retry: Retry, client_closed: bool, e: &Error) -> bool {
    if retry == Retry::BeforeSendOnly {
        return false;
    }
    client_closed
        || matches!(e, Error::Query { sqlstate: Some(c), .. }
            if c.starts_with("08") || matches!(c.as_str(), "57P01" | "57P02" | "57P03"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn q(code: Option<&str>) -> Error {
        Error::Query { sqlstate: code.map(str::to_string), message: "x".into(), detail: None, hint: None }
    }

    #[test]
    fn retry_decision() {
        assert!(should_retry(Retry::Once, true, &q(None)));
        assert!(should_retry(Retry::Once, false, &q(Some("57P01"))));
        assert!(should_retry(Retry::Once, false, &q(Some("08006"))));
        assert!(!should_retry(Retry::Once, false, &q(None)), "SQLSTATE'siz hata tek başına bağlantı kaybı değildir");
        assert!(!should_retry(Retry::Once, false, &q(Some("42501"))));
        // Sinyal: bağlantı kopsa bile ifade yeniden çalıştırılmaz.
        assert!(!should_retry(Retry::BeforeSendOnly, true, &q(Some("57P01"))));
        assert!(!should_retry(Retry::BeforeSendOnly, true, &q(None)));
    }
}
