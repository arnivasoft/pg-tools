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
/// `options` başlangıç parametresini reddeden sunucular/havuzlayıcılar (pgbouncer) için yedek: aynı üç ayar
/// bağlantı kurulduktan hemen sonra `SET` ile uygulanır. `SESSION_OPTIONS` ile aynı değerleri taşımalıdır.
const SESSION_SETS: [&str; 3] =
    ["SET default_transaction_read_only = on", "SET statement_timeout = 30000", "SET search_path = pg_catalog"];

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

/// Sunucu `options` başlangıç parametresini mi reddetti? (pgbouncer: `unsupported startup parameter: options`,
/// SQLSTATE 08P01.) Saf çekirdek: yalnızca sunucu mesajına bakar; kimlik, ağ, TLS ve eksik veritabanı
/// hataları `false` döner, böylece asıl hata maskelenmez.
fn rejects_options_message(sqlstate: Option<&str>, message: &str) -> bool {
    if sqlstate.is_some_and(|c| c != "08P01") {
        return false;
    }
    let m = message.to_ascii_lowercase();
    m.contains("startup parameter") && m.contains("options")
}

fn rejects_options(err: &Error) -> bool {
    matches!(err, Error::Connect { sqlstate, message, .. } if rejects_options_message(sqlstate.as_deref(), message))
}

async fn open_inner(config: &ServerConfig, dbname: &str, password: Option<&str>) -> Result<PgConn> {
    match connect_once(config, dbname, password, true).await {
        Err(first) if rejects_options(&first) => {
            tracing::info!("sunucu `options` başlangıç parametresini reddetti, oturum ayarları SET ile uygulanıyor");
            connect_once(config, dbname, password, false).await
        }
        other => other,
    }
}

/// `statement_timeout` değerini milisaniyeye çevirir (`SHOW`/`current_setting` biçimi: "30s", "30000ms",
/// "1min", birimsiz = ms). Tanınmayan biçimde `None`.
fn timeout_ms(value: &str) -> Option<u64> {
    let v = value.trim();
    let split = v.find(|c: char| !c.is_ascii_digit()).unwrap_or(v.len());
    let (num, unit) = v.split_at(split);
    let n: u64 = num.parse().ok()?;
    let factor = match unit.trim() {
        "" | "ms" => 1,
        "s" => 1_000,
        "min" => 60_000,
        "h" => 3_600_000,
        "d" => 86_400_000,
        _ => return None,
    };
    n.checked_mul(factor)
}

/// Oturumda gerekli üç ayar gerçekten etkin mi? (`options`'ı sessizce yok sayan havuzlayıcılar,
/// ör. pgbouncer `ignore_startup_parameters = options`, bağlantıyı başarıyla kurar ama ayarları uygulamaz.)
fn session_settings_ok(read_only: &str, statement_timeout: &str, search_path: &str) -> bool {
    read_only.trim() == "on" && timeout_ms(statement_timeout) == Some(30_000) && search_path.trim() == "pg_catalog"
}

/// `sets` komutlarını tek `batch_execute` ile uygular; hata bağlantı hatası (`kind = Other`) olarak döner.
/// Çağıran hata durumunda istemciyi düşürmelidir (salt okunur korumasız oturum asla dönmemeli).
#[doc(hidden)]
pub async fn apply_session_sets(client: &tokio_postgres::Client, sets: &[&str]) -> Result<()> {
    client.batch_execute(&sets.join("; ")).await.map_err(|e| match Error::from_query(e) {
        Error::Query { sqlstate, message, .. } => Error::Connect { kind: ConnectErrorKind::Other, sqlstate, message },
        other => other,
    })
}

/// Tek bir bağlantı denemesi. `with_options=true`: oturum ayarları `options` başlangıç parametresiyle;
/// `false`: `options` gönderilmez (pgbouncer yedek yolu). Her iki durumda da sürüm sorgusu ayarları okur;
/// etkin değillerse (yedek yol ya da `options`'ı sessizce yok sayan havuzlayıcı) `SESSION_SETS` uygulanır.
#[doc(hidden)]
pub async fn connect_once(config: &ServerConfig, dbname: &str, password: Option<&str>, with_options: bool) -> Result<PgConn> {
    let mut pg = tokio_postgres::Config::new();
    pg.host(&config.host)
        .port(config.port)
        .user(&config.username)
        .dbname(dbname)
        .application_name("pg-tools")
        .ssl_mode(tls::pg_ssl_mode(config.ssl.mode))
        .connect_timeout(CONNECT_TIMEOUT)
        .keepalives(true)
        .keepalives_idle(KEEPALIVE_IDLE)
        .keepalives_interval(KEEPALIVE_INTERVAL)
        .keepalives_retries(KEEPALIVE_RETRIES);
    if with_options {
        pg.options(SESSION_OPTIONS);
    }
    if let Some(p) = password {
        pg.password(p);
    }
    let connector = tls::make_connector(&config.ssl)?;
    // Ayarlar ve sürüm sorgusu da aynı zaman penceresinde: kimlik doğrulamadan sonra askıda kalan bir
    // sunucu bağlantıyı sonsuza dek bekletmesin.
    let establish = async {
        let (client, connection) = pg.connect(connector).await.map_err(Error::from_connect)?;
        tokio::spawn(async move {
            if let Err(e) = connection.await {
                tracing::debug!(error = %e, "PostgreSQL bağlantısı kapandı");
            }
        });
        let row = client
            .query_one(
                "SELECT current_setting('server_version_num')::int4, current_setting('server_version'), \
                 current_setting('default_transaction_read_only'), current_setting('statement_timeout'), \
                 current_setting('search_path')",
                &[],
            )
            .await
            .map_err(Error::from_query)?;
        let (read_only, timeout, search_path): (String, String, String) = (row.get(2), row.get(3), row.get(4));
        if !session_settings_ok(&read_only, &timeout, &search_path) {
            if with_options {
                tracing::info!("sunucu `options` ayarlarını uygulamadı (havuzlayıcı yok saymış olabilir), oturum ayarları SET ile uygulanıyor");
            }
            // Hata olursa `client` düşer ve bağlantı kapanır; salt okunur korumasız oturum asla dönmez.
            apply_session_sets(&client, &SESSION_SETS).await?;
        }
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

    /// Sahte havuzlayıcı: her bağlantıda (SSLRequest'e `N` verip) başlangıç iletisini okur ve verilen
    /// SQLSTATE/iletiyle bir ErrorResponse gönderir. Kabul edilen bağlantı sayısını döner.
    async fn fake_pooler(code: &'static str, message: &'static str) -> (u16, Arc<std::sync::atomic::AtomicUsize>) {
        use std::sync::atomic::{AtomicUsize, Ordering};
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let count = Arc::new(AtomicUsize::new(0));
        let accepted = count.clone();
        tokio::spawn(async move {
            while let Ok((mut sock, _)) = listener.accept().await {
                accepted.fetch_add(1, Ordering::SeqCst);
                tokio::spawn(async move {
                    loop {
                        let mut len = [0u8; 4];
                        if sock.read_exact(&mut len).await.is_err() {
                            return;
                        }
                        let mut body = vec![0u8; (u32::from_be_bytes(len) as usize).saturating_sub(4)];
                        if sock.read_exact(&mut body).await.is_err() {
                            return;
                        }
                        if body.starts_with(&80877103u32.to_be_bytes()) {
                            let _ = sock.write_all(b"N").await; // SSLRequest: TLS yok
                            continue;
                        }
                        let mut fields = Vec::new();
                        for (tag, value) in [(b'S', "FATAL"), (b'V', "FATAL"), (b'C', code), (b'M', message)] {
                            fields.push(tag);
                            fields.extend_from_slice(value.as_bytes());
                            fields.push(0);
                        }
                        fields.push(0);
                        let mut msg = vec![b'E'];
                        msg.extend_from_slice(&((fields.len() + 4) as u32).to_be_bytes());
                        msg.extend_from_slice(&fields);
                        let _ = sock.write_all(&msg).await;
                        return;
                    }
                });
            }
        });
        (port, count)
    }

    #[tokio::test]
    async fn options_rejection_retries_once_without_options() {
        let (port, count) = fake_pooler("08P01", "unsupported startup parameter: options").await;
        let config = ServerConfig::new("sahte", "127.0.0.1", port, "postgres");
        let Err(err) = open_inner(&config, "postgres", Some("x")).await else { panic!("hata bekleniyordu") };
        assert!(matches!(&err, Error::Connect { sqlstate: Some(c), .. } if c == "08P01"), "{err:?}");
        assert_eq!(count.load(std::sync::atomic::Ordering::SeqCst), 2, "tek bir yeniden deneme bekleniyordu");
    }

    #[tokio::test]
    async fn other_startup_errors_do_not_retry() {
        let (port, count) = fake_pooler("28P01", "password authentication failed for user \"postgres\"").await;
        let config = ServerConfig::new("sahte", "127.0.0.1", port, "postgres");
        let Err(err) = open_inner(&config, "postgres", Some("x")).await else { panic!("hata bekleniyordu") };
        assert!(matches!(&err, Error::Connect { kind: ConnectErrorKind::Auth, .. }), "{err:?}");
        assert_eq!(count.load(std::sync::atomic::Ordering::SeqCst), 1);
    }

    #[test]
    fn timeout_ms_normalises_units() {
        for (v, want) in [("30s", Some(30_000)), ("30000ms", Some(30_000)), ("30000", Some(30_000)), (" 30s ", Some(30_000)), ("1min", Some(60_000)), ("0", Some(0)), ("2h", Some(7_200_000)), ("1d", Some(86_400_000)), ("", None), ("s", None), ("30us", None), ("abc", None)] {
            assert_eq!(timeout_ms(v), want, "{v:?}");
        }
    }

    #[test]
    fn session_settings_ok_requires_all_three() {
        assert!(session_settings_ok("on", "30s", "pg_catalog"));
        assert!(session_settings_ok("on", "30000ms", "pg_catalog"));
        for (ro, t, sp) in [
            ("off", "30s", "pg_catalog"),
            ("on", "0", "pg_catalog"),
            ("on", "1min", "pg_catalog"),
            ("on", "30s", "\"$user\", public"),
            ("on", "30s", "pg_catalog, public"),
            ("on", "garbage", "pg_catalog"),
        ] {
            assert!(!session_settings_ok(ro, t, sp), "{ro} {t} {sp}");
        }
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

    #[test]
    fn rejects_options_only_for_the_pooler_startup_parameter_error() {
        let cases: [(Option<&str>, &str, bool); 10] = [
            (Some("08P01"), "unsupported startup parameter: options", true),
            (None, "unsupported startup parameter: options", true),
            (Some("28P01"), "password authentication failed for user \"postgres\"", false),
            (None, "error connecting to server: Connection refused (os error 111)", false),
            (None, "error performing TLS handshake: invalid peer certificate: UnknownIssuer", false),
            (Some("3D000"), "database \"x\" does not exist", false),
            (Some("08P01"), "unsupported startup parameter: foo", false),
            // SQLSTATE koruması: ileti eşleşse bile başka bir kod yedek yolu tetiklemez.
            (Some("28P01"), "unsupported startup parameter: options", false),
            // "options" tek başına yetmez; "startup parameter" da geçmeli.
            (None, "invalid value for parameter \"options\"", false),
            (Some("08P01"), "invalid options in connection string", false),
        ];
        for (code, msg, want) in cases {
            assert_eq!(rejects_options_message(code, msg), want, "{code:?} {msg}");
        }
    }

    #[test]
    fn session_sets_match_session_options() {
        let from_options: Vec<String> = SESSION_OPTIONS
            .split("-c ")
            .map(str::trim)
            .filter(|p| !p.is_empty())
            .map(|p| {
                let (k, v) = p.split_once('=').unwrap();
                format!("SET {k} = {v}")
            })
            .collect();
        assert_eq!(from_options, SESSION_SETS);
    }
}
