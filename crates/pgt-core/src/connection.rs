use std::collections::HashMap;
use std::future::Future;
use std::hash::Hash;
use std::sync::{Arc, Mutex as StdMutex};
use std::time::{Duration, Instant};

use serde::Serialize;
use tokio::sync::Mutex;
use ts_rs::TS;
use uuid::Uuid;

use crate::config::ServerConfig;
use crate::{ConnectErrorKind, Error, Result, tls};

/// Bağlantı kurma (TCP + TLS + kimlik doğrulama + sürüm sorgusu) için üst sınır.
pub const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
/// Her `run` kapanışı için istemci tarafı süre sınırı. Sunucudaki `statement_timeout`'tan (30 sn)
/// biraz uzundur: bu süre dolduysa sorgu yavaş değil, bağlantı askıda demektir (ör. ağ sessizce
/// koptu). Dolunca bağlantı kopmuş sayılır ve bir kez yeniden bağlanılıp denenir.
pub const QUERY_DEADLINE: Duration = Duration::from_secs(35);
/// TCP keepalive: 30 sn boşta kalınca yoklamaya başla, 10 sn arayla 3 deneme; ölü bağlantı
/// yaklaşık bir dakikada fark edilir (NAT/güvenlik duvarı boşta bağlantıları sessizce düşürür).
const KEEPALIVE_IDLE: Duration = Duration::from_secs(30);
const KEEPALIVE_INTERVAL: Duration = Duration::from_secs(10);
const KEEPALIVE_RETRIES: u32 = 3;
/// `search_path=pg_catalog`: `pg_get_*def`, `format_type` ve `regclass` çıktısı yalnızca yolda görünmeyen
/// adları şema ile niteler; rol/veritabanı düzeyindeki `search_path` ayarı üretilen DDL'i niteliksiz bırakmasın.
const SESSION_OPTIONS: &str = "-c default_transaction_read_only=on -c statement_timeout=30000 -c search_path=pg_catalog";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ServerInfo {
    pub version_num: u32,
    /// Kısa sürüm, ör. "16.4".
    pub version: String,
}

pub struct PgConn {
    pub client: tokio_postgres::Client,
    pub info: ServerInfo,
    last_used: StdMutex<Instant>,
}

impl PgConn {
    pub fn version_num(&self) -> u32 {
        self.info.version_num
    }
    fn touch(&self) {
        *self.last_used.lock().unwrap() = Instant::now();
    }
    fn idle_for(&self) -> Duration {
        self.last_used.lock().unwrap().elapsed()
    }
    fn usable(&self) -> bool {
        !self.client.is_closed()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Target {
    Maintenance,
    Database(u32),
}

struct Session {
    config: ServerConfig,
    password: Option<String>,
    conns: HashMap<Target, Arc<PgConn>>,
    db_names: HashMap<u32, String>,
}

/// Oturum kaydı. `info` değişmez ve kilitsiz okunur; `state` (bağlantılar) yavaş bir yeniden
/// bağlanma süresince tutulabilen async kilittir — `info()`/`is_connected()` onu beklemez.
struct SessionEntry {
    info: ServerInfo,
    state: Mutex<Session>,
}

type SessionRef = Arc<SessionEntry>;

#[derive(Default)]
pub struct ConnectionManager {
    sessions: StdMutex<HashMap<Uuid, SessionRef>>,
}

impl ConnectionManager {
    pub fn new() -> Self {
        Self::default()
    }

    /// Kaydetmeden bağlantıyı dener ("Bağlantıyı test et").
    pub async fn test(config: &ServerConfig, password: Option<&str>) -> Result<ServerInfo> {
        Ok(open(config, &config.maintenance_db, password).await?.info.clone())
    }

    pub async fn connect(&self, config: ServerConfig, password: Option<String>) -> Result<ServerInfo> {
        let conn = open(&config, &config.maintenance_db, password.as_deref()).await?;
        let info = conn.info.clone();
        let session = Session {
            config: config.clone(),
            password,
            conns: HashMap::from([(Target::Maintenance, Arc::new(conn))]),
            db_names: HashMap::new(),
        };
        let entry = Arc::new(SessionEntry { info: info.clone(), state: Mutex::new(session) });
        self.sessions.lock().unwrap().insert(config.id, entry);
        tracing::info!(server = %config.id, version = %info.version, "sunucuya bağlanıldı");
        Ok(info)
    }

    /// Oturumu ve tüm bağlantılarını kapatır (Client'lar düşünce bağlantılar kapanır).
    pub fn disconnect(&self, server: Uuid) {
        self.sessions.lock().unwrap().remove(&server);
    }

    pub fn is_connected(&self, server: Uuid) -> bool {
        self.sessions.lock().unwrap().contains_key(&server)
    }

    /// Bağlıysa sunucu bilgisi. Oturum kilidini almaz; yeniden bağlanan bir oturumu beklemez.
    pub fn info(&self, server: Uuid) -> Option<ServerInfo> {
        self.sessions.lock().unwrap().get(&server).map(|e| e.info.clone())
    }

    fn session(&self, server: Uuid) -> Result<SessionRef> {
        self.sessions.lock().unwrap().get(&server).cloned().ok_or(Error::NotConnected)
    }

    /// Hedef bağlantıyı verir (gerekirse açar). Başarısızlıkta düşürme politikası uygulanır:
    /// sunucu düzeyindeki hatalarda (bakım hedefi ya da ağ/zaman aşımı/kimlik/TLS) oturum kapatılır
    /// ve `ConnectionLost` döner; yalnızca bir veritabanı hedefine özgü hatada (ör. veritabanı
    /// silinmiş, 3D000) sadece o bağlantı ve ad önbelleği düşürülür, hata olduğu gibi döner.
    async fn acquire(&self, server: Uuid, session: &SessionRef, target: Target) -> Result<Arc<PgConn>> {
        let result = {
            let mut s = session.state.lock().await;
            conn_locked(&mut s, target).await
        };
        match result {
            Ok(conn) => {
                conn.touch();
                Ok(conn)
            }
            Err(err) if drops_session(target, &err) => {
                tracing::warn!(%server, ?target, error = %err, "yeniden bağlanılamadı, oturum kapatıldı");
                self.drop_session(server, session);
                Err(Error::ConnectionLost(err.to_string()))
            }
            Err(err) => {
                tracing::warn!(%server, ?target, error = %err, "veritabanı bağlantısı açılamadı");
                if let Target::Database(oid) = target {
                    let mut s = session.state.lock().await;
                    s.conns.remove(&target);
                    s.db_names.remove(&oid);
                }
                Err(err)
            }
        }
    }

    /// Oturumu yalnızca hâlâ aynı oturumsa kaldırır (araya yeni bir `connect` girmişse dokunmaz).
    fn drop_session(&self, server: Uuid, session: &SessionRef) {
        remove_if_same(&mut self.sessions.lock().unwrap(), &server, session);
    }

    /// Bozulan bağlantıyı çıkarır — yalnızca hâlâ aynı bağlantıysa; eşzamanlı bir hata, araya
    /// başka bir görevin açtığı sağlıklı bağlantıyı düşürmesin.
    async fn evict(session: &SessionRef, target: Target, failed: &Arc<PgConn>) {
        remove_if_same(&mut session.state.lock().await.conns, &target, failed);
    }

    /// `f`'yi hedef bağlantıda çalıştırır; bağlantı kopmuşsa bir kez yeniden bağlanıp tekrar dener.
    pub async fn run<T, F, Fut>(&self, server: Uuid, target: Target, f: F) -> Result<T>
    where
        F: Fn(Arc<PgConn>) -> Fut,
        Fut: Future<Output = std::result::Result<T, tokio_postgres::Error>>,
    {
        let session = self.session(server)?;
        let conn = self.acquire(server, &session, target).await?;
        let cause = match with_deadline(QUERY_DEADLINE, f(conn.clone())).await {
            Some(Ok(v)) => return Ok(v),
            Some(Err(e)) if is_disconnect(&e) || !conn.usable() => e.to_string(),
            Some(Err(e)) => return Err(Error::from_query(e)),
            None => format!("{} sn içinde yanıt yok", QUERY_DEADLINE.as_secs()),
        };
        tracing::warn!(%server, ?target, %cause, "bağlantı kopmuş ya da yanıt vermiyor, yeniden bağlanılıyor");
        Self::evict(&session, target, &conn).await;
        let conn = self.acquire(server, &session, target).await?;
        match with_deadline(QUERY_DEADLINE, f(conn.clone())).await {
            Some(result) => result.map_err(Error::from_query),
            None => {
                tracing::warn!(%server, ?target, "yeniden bağlandıktan sonra da yanıt yok");
                Self::evict(&session, target, &conn).await;
                Err(Error::Query {
                    sqlstate: None,
                    message: format!("Sunucu {} sn içinde yanıt vermedi (iki deneme).", QUERY_DEADLINE.as_secs()),
                    detail: None,
                    hint: Some("Ağ bağlantısını ve sunucunun yükünü kontrol edin.".into()),
                })
            }
        }
    }

    /// `max_idle` süresinden uzun boşta kalan bağlantıları kapatır; oturumlar bağlı kalır.
    pub async fn reap_idle(&self, max_idle: Duration) {
        let sessions: Vec<_> = self.sessions.lock().unwrap().values().cloned().collect();
        for session in sessions {
            let mut s = session.state.lock().await;
            s.conns.retain(|_, c| c.usable() && c.idle_for() < max_idle);
        }
    }
}

/// Bu hata oturumun tamamını geçersiz kılar mı? Bakım bağlantısı her zaman sunucunun kendisidir;
/// veritabanı hedefinde yalnızca sunucu düzeyindeki bağlantı hataları oturumu düşürür.
fn drops_session(target: Target, err: &Error) -> bool {
    target == Target::Maintenance
        || matches!(
            err,
            Error::PasswordRequired
                | Error::Connect {
                    kind: ConnectErrorKind::Network | ConnectErrorKind::Timeout | ConnectErrorKind::Auth | ConnectErrorKind::Tls,
                    ..
                }
        )
}

/// `map[key]` tam olarak `expected` ise (aynı `Arc`) kaldırır.
fn remove_if_same<K: Eq + Hash, V>(map: &mut HashMap<K, Arc<V>>, key: &K, expected: &Arc<V>) -> bool {
    if map.get(key).is_some_and(|cur| Arc::ptr_eq(cur, expected)) {
        map.remove(key);
        true
    } else {
        false
    }
}

/// `fut`'u en fazla `limit` kadar bekler; süre dolarsa `None` (gelecek iptal edilir).
async fn with_deadline<F: Future>(limit: Duration, fut: F) -> Option<F::Output> {
    tokio::time::timeout(limit, fut).await.ok()
}

fn is_disconnect(e: &tokio_postgres::Error) -> bool {
    e.is_closed() || e.code().is_some_and(|c| matches!(c.code(), "57P01" | "57P02" | "57P03"))
}

async fn maintenance_locked(s: &mut Session) -> Result<Arc<PgConn>> {
    if let Some(c) = s.conns.get(&Target::Maintenance).filter(|c| c.usable()) {
        return Ok(c.clone());
    }
    let conn = Arc::new(open(&s.config, &s.config.maintenance_db, s.password.as_deref()).await?);
    s.conns.insert(Target::Maintenance, conn.clone());
    Ok(conn)
}

async fn conn_locked(s: &mut Session, target: Target) -> Result<Arc<PgConn>> {
    let oid = match target {
        Target::Maintenance => return maintenance_locked(s).await,
        Target::Database(oid) => oid,
    };
    if let Some(c) = s.conns.get(&target).filter(|c| c.usable()) {
        return Ok(c.clone());
    }
    let name = match s.db_names.get(&oid) {
        Some(n) => n.clone(),
        None => {
            let m = maintenance_locked(s).await?;
            let row = m
                .client
                .query_opt("SELECT datname::text FROM pg_database WHERE oid = $1", &[&oid])
                .await
                .map_err(Error::from_query)?
                .ok_or_else(|| Error::NotFound(format!("veritabanı (oid {oid})")))?;
            let name: String = row.get(0);
            s.db_names.insert(oid, name.clone());
            name
        }
    };
    let conn = Arc::new(open(&s.config, &name, s.password.as_deref()).await?);
    s.conns.insert(target, conn.clone());
    Ok(conn)
}

async fn open(config: &ServerConfig, dbname: &str, password: Option<&str>) -> Result<PgConn> {
    let result = open_inner(config, dbname, password).await;
    if let Err(e) = &result {
        tracing::warn!(server = %config.id, db = dbname, error = %e, "bağlantı açılamadı");
    }
    result
}

async fn open_inner(config: &ServerConfig, dbname: &str, password: Option<&str>) -> Result<PgConn> {
    let mut pg = tokio_postgres::Config::new();
    pg.host(&config.host)
        .port(config.port)
        .user(&config.username)
        .dbname(dbname)
        .application_name("pg-tools")
        .options(SESSION_OPTIONS)
        .ssl_mode(tls::pg_ssl_mode(config.ssl.mode))
        .connect_timeout(CONNECT_TIMEOUT)
        .keepalives(true)
        .keepalives_idle(KEEPALIVE_IDLE)
        .keepalives_interval(KEEPALIVE_INTERVAL)
        .keepalives_retries(KEEPALIVE_RETRIES);
    if let Some(p) = password {
        pg.password(p);
    }
    let connector = tls::make_connector(&config.ssl)?;
    // Sürüm sorgusu da aynı zaman penceresinde: kimlik doğrulamadan sonra askıda kalan bir
    // sunucu bağlantıyı sonsuza dek bekletmesin.
    let establish = async {
        let (client, connection) = pg.connect(connector).await.map_err(Error::from_connect)?;
        tokio::spawn(async move {
            if let Err(e) = connection.await {
                tracing::debug!(error = %e, "PostgreSQL bağlantısı kapandı");
            }
        });
        let row = client
            .query_one("SELECT current_setting('server_version_num')::int4, current_setting('server_version')", &[])
            .await
            .map_err(Error::from_query)?;
        Ok::<_, Error>((client, row))
    };
    let (client, row) = match with_deadline(CONNECT_TIMEOUT + Duration::from_secs(1), establish).await {
        Some(result) => result?,
        None => {
            return Err(Error::Connect { kind: ConnectErrorKind::Timeout, sqlstate: None, message: "zaman aşımı".into() });
        }
    };
    let num: i32 = row.get(0);
    let full: String = row.get(1);
    let version = full.split_whitespace().next().unwrap_or(&full).to_string();
    Ok(PgConn { client, info: ServerInfo { version_num: num as u32, version }, last_used: StdMutex::new(Instant::now()) })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn connect_err(kind: ConnectErrorKind) -> Error {
        Error::Connect { kind, sqlstate: None, message: "x".into() }
    }

    #[tokio::test(start_paused = true)]
    async fn with_deadline_cuts_off_a_hung_future() {
        let hung = with_deadline(QUERY_DEADLINE, async {
            tokio::time::sleep(Duration::from_secs(3600)).await;
            1
        });
        assert_eq!(hung.await, None);
        let quick = with_deadline(QUERY_DEADLINE, async {
            tokio::time::sleep(QUERY_DEADLINE - Duration::from_secs(1)).await;
            2
        });
        assert_eq!(quick.await, Some(2));
    }

    #[test]
    fn deadline_is_longer_than_server_statement_timeout() {
        assert!(SESSION_OPTIONS.contains("statement_timeout=30000"));
        assert!(QUERY_DEADLINE > Duration::from_secs(30));
    }

    #[test]
    fn remove_if_same_only_removes_the_failed_arc() {
        let failed = Arc::new(1);
        let fresh = Arc::new(1);
        let mut map = HashMap::from([(Target::Maintenance, fresh.clone())]);
        assert!(!remove_if_same(&mut map, &Target::Maintenance, &failed), "taze bağlantı düşürülmemeli");
        assert!(Arc::ptr_eq(&map[&Target::Maintenance], &fresh));
        assert!(remove_if_same(&mut map, &Target::Maintenance, &fresh));
        assert!(map.is_empty());
        assert!(!remove_if_same(&mut map, &Target::Maintenance, &fresh));
    }

    #[tokio::test]
    async fn info_does_not_wait_for_a_held_session_lock() {
        let mgr = ConnectionManager::new();
        let id = Uuid::new_v4();
        let info = ServerInfo { version_num: 180000, version: "18.0".into() };
        let session = Session {
            config: ServerConfig::new("s", "127.0.0.1", 1, "u"),
            password: None,
            conns: HashMap::new(),
            db_names: HashMap::new(),
        };
        let entry = Arc::new(SessionEntry { info: info.clone(), state: Mutex::new(session) });
        mgr.sessions.lock().unwrap().insert(id, entry.clone());
        let _held = entry.state.lock().await; // yavaş bir yeniden bağlanmayı temsil eder
        assert_eq!(mgr.info(id), Some(info));
        assert!(mgr.is_connected(id));
    }

    #[test]
    fn maintenance_failures_always_drop_the_session() {
        for err in [connect_err(ConnectErrorKind::Other), Error::NotFound("x".into()), connect_err(ConnectErrorKind::Network)] {
            assert!(drops_session(Target::Maintenance, &err), "{err:?}");
        }
    }

    #[test]
    fn database_target_drops_session_only_for_server_level_failures() {
        for kind in [ConnectErrorKind::Network, ConnectErrorKind::Timeout, ConnectErrorKind::Auth, ConnectErrorKind::Tls] {
            assert!(drops_session(Target::Database(1), &connect_err(kind)), "{kind:?}");
        }
        assert!(drops_session(Target::Database(1), &Error::PasswordRequired));
        let missing_db = Error::Connect { kind: ConnectErrorKind::Other, sqlstate: Some("3D000".into()), message: "yok".into() };
        for err in [missing_db, Error::NotFound("db".into())] {
            assert!(!drops_session(Target::Database(1), &err), "{err:?}");
        }
    }
}
