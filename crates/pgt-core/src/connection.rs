use std::borrow::Cow;
use std::collections::HashMap;
use std::future::Future;
use std::hash::Hash;
use std::sync::{Arc, Mutex as StdMutex};
use std::time::{Duration, Instant};

use futures_util::{StreamExt, stream};
use serde::Serialize;
use tokio::sync::{Mutex, mpsc};
use tokio_postgres::AsyncMessage;
use ts_rs::TS;
use uuid::Uuid;

use crate::config::{ServerConfig, SslConfig};
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
/// `statement_timeout`'un PostgreSQL'deki üst sınırı (ms, `INT_MAX`); fazlası oturum açılışında reddedilir.
pub const MAX_STATEMENT_TIMEOUT_MS: u32 = 2_147_483_647;
/// Sorgu oturumlarının `application_name`'i: `pg_stat_activity`'de katalog oturumlarından ayırt edilsin.
const QUERY_APPLICATION_NAME: &str = "pg-tools query";

/// Oturum ayarları profili.
/// - `Catalog`: havuzdaki salt okunur katalog oturumu (`SESSION_OPTIONS`/`SESSION_SETS`, 30 sn, `search_path=pg_catalog`).
/// - `Query`: kullanıcının SQL'ini çalıştıran ayrı (havuzsuz) oturum. `search_path` **verilmez**: kullanıcının
///   rol/veritabanı düzeyindeki yolu geçerli kalır. `statement_timeout_ms = 0` sınırsız demektir; en fazla
///   `MAX_STATEMENT_TIMEOUT_MS` (PostgreSQL üst sınırı). `read_only` yalnızca oturum varsayılanıdır
///   (`default_transaction_read_only`), bir güvenlik sınırı değildir: kullanıcı SQL'i onu `SET` ya da
///   `BEGIN READ WRITE` ile değiştirebilir. Gerçek salt okunurluk rol yetkileriyle sağlanır.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionProfile {
    Catalog,
    Query { read_only: bool, statement_timeout_ms: u32 },
}

impl SessionProfile {
    fn application_name(self) -> &'static str {
        match self {
            SessionProfile::Catalog => "pg-tools",
            SessionProfile::Query { .. } => QUERY_APPLICATION_NAME,
        }
    }
}

fn on_off(value: bool) -> &'static str {
    if value { "on" } else { "off" }
}

/// Profilin `options` başlangıç parametresi. `Catalog` için `SESSION_OPTIONS`'ın kendisi.
fn profile_session_options(profile: SessionProfile) -> Cow<'static, str> {
    match profile {
        SessionProfile::Catalog => Cow::Borrowed(SESSION_OPTIONS),
        SessionProfile::Query { read_only, statement_timeout_ms } => Cow::Owned(format!(
            "-c default_transaction_read_only={} -c statement_timeout={statement_timeout_ms}",
            on_off(read_only)
        )),
    }
}

/// `options` uygulanmadığında aynı ayarları kuran `SET` komutları (`profile_session_options` ile aynı değerler).
fn profile_session_sets(profile: SessionProfile) -> Vec<Cow<'static, str>> {
    match profile {
        SessionProfile::Catalog => SESSION_SETS.iter().map(|s| Cow::Borrowed(*s)).collect(),
        SessionProfile::Query { read_only, statement_timeout_ms } => vec![
            Cow::Owned(format!("SET default_transaction_read_only = {}", on_off(read_only))),
            Cow::Owned(format!("SET statement_timeout = {statement_timeout_ms}")),
        ],
    }
}

/// Sunucunun gönderdiği bir NOTICE/WARNING/INFO iletisi (sorgu oturumunda `RAISE NOTICE` vb.).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Notice {
    pub severity: String,
    pub code: String,
    pub message: String,
    pub detail: Option<String>,
    pub hint: Option<String>,
}

impl From<&tokio_postgres::error::DbError> for Notice {
    fn from(db: &tokio_postgres::error::DbError) -> Self {
        Notice {
            severity: db.severity().to_string(),
            code: db.code().code().to_string(),
            message: db.message().to_string(),
            detail: db.detail().map(str::to_string),
            hint: db.hint().map(str::to_string),
        }
    }
}

/// Sorgu oturumundaki çalışan sorguyu iptal eder. `CancelToken::cancel_query` yeni bir bağlantı açtığı için
/// TLS bağlayıcısı gerekir: sunucunun TLS ayarı saklanır ve her iptalde bağlayıcı yeniden kurulur.
#[derive(Clone)]
pub struct CancelHandle {
    token: tokio_postgres::CancelToken,
    ssl: SslConfig,
}

impl CancelHandle {
    /// İptal isteğini gönderir (en fazla `CONNECT_TIMEOUT` bekler). Başarı, sorgunun iptal edildiğini değil
    /// isteğin iletildiğini gösterir; sorgu 57014 ile biter (ya da zaten bitmişse etkisizdir).
    pub async fn cancel(&self) -> Result<()> {
        let tls = tls::make_connector(&self.ssl)?;
        match with_deadline(CONNECT_TIMEOUT, self.token.cancel_query(tls)).await {
            Some(result) => result.map_err(Error::from_connect),
            None => Err(Error::Connect { kind: ConnectErrorKind::Timeout, sqlstate: None, message: "zaman aşımı".into() }),
        }
    }
}

impl std::fmt::Debug for CancelHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CancelHandle").finish_non_exhaustive()
    }
}

/// Havuz dışı, tek sahipli bağlantı (Query Tool). `client` düşünce bağlantı kapanır, sürücü görevi biter ve
/// `notices` kanalı kapanır (`recv()` → `None`).
pub struct DedicatedConn {
    pub client: tokio_postgres::Client,
    pub info: ServerInfo,
    pub notices: mpsc::UnboundedReceiver<Notice>,
    pub cancel: CancelHandle,
}

impl std::fmt::Debug for DedicatedConn {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DedicatedConn").field("info", &self.info).finish_non_exhaustive()
    }
}

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

    /// Bağlı sunucuda havuz dışı, tek sahipli bir bağlantı açar (Query Tool). Yapılandırma ve parola
    /// oturumdan alınır; `Target::Database(oid)` adı önbellekten ya da bakım bağlantısıyla çözülür
    /// (bilinmeyen oid → `NotFound`). Oturum kilidi yalnızca ad çözümü boyunca tutulur; yavaş bağlantı
    /// kilitsiz kurulur, havuzdaki `run`'ları bekletmez. Havuz (`conns`) değişmez.
    pub async fn open_dedicated(&self, server: Uuid, target: Target, profile: SessionProfile) -> Result<DedicatedConn> {
        validate_profile(profile)?;
        let session = self.session(server)?;
        let (config, password, dbname) = {
            let mut s = session.state.lock().await;
            let dbname = match target {
                Target::Maintenance => s.config.maintenance_db.clone(),
                Target::Database(oid) => db_name_locked(&mut s, oid).await?,
            };
            (s.config.clone(), s.password.clone(), dbname)
        };
        let result = open_dedicated_conn(&config, &dbname, password.as_deref(), profile).await;
        if let (Err(e), Target::Database(oid)) = (&result, target)
            && is_missing_database(e)
        {
            // Veritabanı silinmiş ya da yeniden adlandırılmış: ad bir sonraki denemede yeniden çözülsün. Sunucu
            // düzeyindeki hatalarda (ağ, kimlik, TLS) ad geçerlidir, önbellek korunur.
            session.state.lock().await.db_names.remove(&oid);
        }
        result
    }

    /// Test yardımcısı: oturumun havuzundaki bağlantı sayısı (bağlı değilse 0).
    #[cfg(any(test, feature = "testing"))]
    #[doc(hidden)]
    pub async fn pooled_connections(&self, server: Uuid) -> usize {
        let Ok(session) = self.session(server) else { return 0 };
        session.state.lock().await.conns.len()
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

/// Profil değerleri sunucunun kabul edeceği aralıkta mı? (Aşım, açılışta anlaşılmaz bir bağlantı hatası verirdi.)
fn validate_profile(profile: SessionProfile) -> Result<()> {
    match profile {
        SessionProfile::Query { statement_timeout_ms, .. } if statement_timeout_ms > MAX_STATEMENT_TIMEOUT_MS => {
            Err(Error::Invalid("statement_timeout en fazla 2147483647 ms olabilir.".into()))
        }
        _ => Ok(()),
    }
}

/// Bağlantı hatası, hedef veritabanının artık olmadığını mı söylüyor? (3D000 invalid_catalog_name)
fn is_missing_database(err: &Error) -> bool {
    matches!(err, Error::Connect { sqlstate: Some(code), .. } if code == "3D000")
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
    let name = db_name_locked(s, oid).await?;
    let conn = Arc::new(open(&s.config, &name, s.password.as_deref()).await?);
    s.conns.insert(target, conn.clone());
    Ok(conn)
}

/// Veritabanı adını önbellekten ya da bakım bağlantısıyla çözer ve önbelleğe yazar; yoksa `NotFound`.
async fn db_name_locked(s: &mut Session, oid: u32) -> Result<String> {
    if let Some(n) = s.db_names.get(&oid) {
        return Ok(n.clone());
    }
    let m = maintenance_locked(s).await?;
    let row = m
        .client
        .query_opt("SELECT datname::text FROM pg_database WHERE oid = $1", &[&oid])
        .await
        .map_err(Error::from_query)?
        .ok_or_else(|| Error::NotFound(format!("veritabanı (oid {oid})")))?;
    let name: String = row.get(0);
    s.db_names.insert(oid, name.clone());
    Ok(name)
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

/// `attempt(true)` (`options` ile) dener; sunucu `options`'ı reddettiyse bir kez `attempt(false)` dener.
async fn with_options_fallback<T, F, Fut>(mut attempt: F) -> Result<T>
where
    F: FnMut(bool) -> Fut,
    Fut: Future<Output = Result<T>>,
{
    match attempt(true).await {
        Err(first) if rejects_options(&first) => {
            tracing::info!("sunucu `options` başlangıç parametresini reddetti, oturum ayarları SET ile uygulanıyor");
            attempt(false).await
        }
        other => other,
    }
}

async fn open_inner(config: &ServerConfig, dbname: &str, password: Option<&str>) -> Result<PgConn> {
    with_options_fallback(|with_options| connect_once(config, dbname, password, with_options)).await
}

async fn open_dedicated_conn(
    config: &ServerConfig,
    dbname: &str,
    password: Option<&str>,
    profile: SessionProfile,
) -> Result<DedicatedConn> {
    let result =
        with_options_fallback(|with_options| dedicated_once(config, dbname, password, profile, with_options)).await;
    if let Err(e) = &result {
        tracing::warn!(server = %config.id, db = dbname, error = %e, "sorgu bağlantısı açılamadı");
    }
    result
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

/// Katalog profilinin üç ayarı gerçekten etkin mi? (`settings_ok_for(Catalog, …)`.)
#[cfg(test)]
fn session_settings_ok(read_only: &str, statement_timeout: &str, search_path: &str) -> bool {
    settings_ok_for(SessionProfile::Catalog, read_only, statement_timeout, search_path)
}

/// Profilin gerektirdiği oturum ayarları gerçekten etkin mi? (`options`'ı sessizce yok sayan havuzlayıcılar,
/// ör. pgbouncer `ignore_startup_parameters = options`, bağlantıyı başarıyla kurar ama ayarları uygulamaz.)
/// `Catalog`: salt okunur, 30 sn ve `search_path = pg_catalog`; `Query`: yalnızca salt okunurluk ve süre sınırı.
fn settings_ok_for(profile: SessionProfile, read_only: &str, statement_timeout: &str, search_path: &str) -> bool {
    match profile {
        SessionProfile::Catalog => {
            read_only.trim() == "on" && timeout_ms(statement_timeout) == Some(30_000) && search_path.trim() == "pg_catalog"
        }
        SessionProfile::Query { read_only: want_read_only, statement_timeout_ms } => {
            read_only.trim() == on_off(want_read_only)
                && timeout_ms(statement_timeout) == Some(u64::from(statement_timeout_ms))
        }
    }
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

/// Bağlantı sürücüsü: havuz bağlantısı `connection.await` ile sürülür (iletiler yok sayılır); sorgu
/// bağlantısı `poll_message` ile sürülür ve NOTICE'lar kanala iletilir.
enum Driver {
    Await,
    Notices(mpsc::UnboundedSender<Notice>),
}

/// Tek bir bağlantı denemesi. `with_options=true`: oturum ayarları `options` başlangıç parametresiyle;
/// `false`: `options` gönderilmez (pgbouncer yedek yolu). Her iki durumda da sürüm sorgusu ayarları okur;
/// etkin değillerse (yedek yol ya da `options`'ı sessizce yok sayan havuzlayıcı) profilin `SET`'leri uygulanır.
/// Hata olursa istemci düşer ve bağlantı kapanır; istenen ayarları taşımayan oturum asla dönmez.
async fn connect_with(
    config: &ServerConfig,
    dbname: &str,
    password: Option<&str>,
    profile: SessionProfile,
    with_options: bool,
    driver: Driver,
) -> Result<(tokio_postgres::Client, ServerInfo)> {
    let mut pg = tokio_postgres::Config::new();
    pg.host(&config.host)
        .port(config.port)
        .user(&config.username)
        .dbname(dbname)
        .application_name(profile.application_name())
        .ssl_mode(tls::pg_ssl_mode(config.ssl.mode))
        .connect_timeout(CONNECT_TIMEOUT)
        .keepalives(true)
        .keepalives_idle(KEEPALIVE_IDLE)
        .keepalives_interval(KEEPALIVE_INTERVAL)
        .keepalives_retries(KEEPALIVE_RETRIES);
    if with_options {
        pg.options(profile_session_options(profile).as_ref());
    }
    if let Some(p) = password {
        pg.password(p);
    }
    let connector = tls::make_connector(&config.ssl)?;
    // Ayarlar ve sürüm sorgusu da aynı zaman penceresinde: kimlik doğrulamadan sonra askıda kalan bir
    // sunucu bağlantıyı sonsuza dek bekletmesin.
    let establish = async {
        let (client, connection) = pg.connect(connector).await.map_err(Error::from_connect)?;
        match driver {
            Driver::Await => {
                tokio::spawn(async move {
                    if let Err(e) = connection.await {
                        tracing::debug!(error = %e, "PostgreSQL bağlantısı kapandı");
                    }
                });
            }
            Driver::Notices(tx) => {
                let mut connection = connection;
                tokio::spawn(async move {
                    // İstemci düşünce bağlantı kapanır, akış `None` verir ve görev (ile `tx`) biter.
                    let mut messages = stream::poll_fn(move |cx| connection.poll_message(cx));
                    while let Some(message) = messages.next().await {
                        match message {
                            Ok(AsyncMessage::Notice(db)) => {
                                // Alıcı düşmüş olabilir; bağlantı yine de sürülmeye devam eder.
                                let _ = tx.send(Notice::from(&db));
                            }
                            Ok(_) => {} // LISTEN/NOTIFY kapsam dışı
                            Err(e) => {
                                tracing::debug!(error = %e, "PostgreSQL sorgu bağlantısı kapandı");
                                break;
                            }
                        }
                    }
                });
            }
        }
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
        if !settings_ok_for(profile, &read_only, &timeout, &search_path) {
            if with_options {
                tracing::info!("sunucu `options` ayarlarını uygulamadı (havuzlayıcı yok saymış olabilir), oturum ayarları SET ile uygulanıyor");
            }
            // Hata olursa `client` düşer ve bağlantı kapanır; istenen ayarları taşımayan oturum asla dönmez.
            let sets = profile_session_sets(profile);
            let sets: Vec<&str> = sets.iter().map(|s| s.as_ref()).collect();
            apply_session_sets(&client, &sets).await?;
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
    Ok((client, ServerInfo { version_num: num as u32, version }))
}

/// Katalog profiliyle tek bir havuz bağlantısı denemesi (bkz. `connect_with`).
#[doc(hidden)]
pub async fn connect_once(config: &ServerConfig, dbname: &str, password: Option<&str>, with_options: bool) -> Result<PgConn> {
    let (client, info) = connect_with(config, dbname, password, SessionProfile::Catalog, with_options, Driver::Await).await?;
    Ok(PgConn { client, info, last_used: StdMutex::new(Instant::now()) })
}

/// Test girişi: tek bir havuz dışı bağlantı denemesi (`options` yedek yolu olmadan).
#[cfg(any(test, feature = "testing"))]
#[doc(hidden)]
pub async fn connect_dedicated_once(
    config: &ServerConfig,
    dbname: &str,
    password: Option<&str>,
    profile: SessionProfile,
    with_options: bool,
) -> Result<DedicatedConn> {
    dedicated_once(config, dbname, password, profile, with_options).await
}

/// Tek bir havuz dışı bağlantı denemesi (`options` yedek yolu olmadan; bkz. `connect_with`).
async fn dedicated_once(
    config: &ServerConfig,
    dbname: &str,
    password: Option<&str>,
    profile: SessionProfile,
    with_options: bool,
) -> Result<DedicatedConn> {
    let (tx, notices) = mpsc::unbounded_channel();
    let (client, info) = connect_with(config, dbname, password, profile, with_options, Driver::Notices(tx)).await?;
    let cancel = CancelHandle { token: client.cancel_token(), ssl: config.ssl.clone() };
    Ok(DedicatedConn { client, info, notices, cancel })
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

    const Q_RO: SessionProfile = SessionProfile::Query { read_only: true, statement_timeout_ms: 0 };
    const Q_RW: SessionProfile = SessionProfile::Query { read_only: false, statement_timeout_ms: 300_000 };

    #[test]
    fn profile_session_options_per_profile() {
        assert_eq!(profile_session_options(Q_RO), "-c default_transaction_read_only=on -c statement_timeout=0");
        assert_eq!(profile_session_options(Q_RW), "-c default_transaction_read_only=off -c statement_timeout=300000");
        assert_eq!(profile_session_options(SessionProfile::Catalog), SESSION_OPTIONS);
        for p in [Q_RO, Q_RW] {
            assert!(!profile_session_options(p).contains("search_path"), "{p:?}");
            assert!(!profile_session_sets(p).iter().any(|s| s.contains("search_path")), "{p:?}");
        }
        assert_eq!(SessionProfile::Catalog.application_name(), "pg-tools");
        assert_eq!(Q_RO.application_name(), "pg-tools query");
    }

    /// Her profilde `SET` yedeği `options` ile aynı ayarları taşır (Catalog: `SESSION_SETS`'in kendisi).
    #[test]
    fn profile_session_sets_match_profile_options() {
        for p in [SessionProfile::Catalog, Q_RO, Q_RW] {
            let from_options: Vec<String> = profile_session_options(p)
                .split("-c ")
                .map(str::trim)
                .filter(|p| !p.is_empty())
                .map(|p| {
                    let (k, v) = p.split_once('=').unwrap();
                    format!("SET {k} = {v}")
                })
                .collect();
            let sets: Vec<String> = profile_session_sets(p).into_iter().map(Cow::into_owned).collect();
            assert_eq!(from_options, sets, "{p:?}");
        }
        assert_eq!(profile_session_sets(SessionProfile::Catalog), SESSION_SETS);
    }

    #[test]
    fn settings_ok_for_table() {
        let five_min = SessionProfile::Query { read_only: false, statement_timeout_ms: 300_000 };
        let cases: [(SessionProfile, &str, &str, &str, bool); 13] = [
            (SessionProfile::Catalog, "on", "30s", "pg_catalog", true),
            (SessionProfile::Catalog, "on", "30s", "\"$user\", public", false),
            (SessionProfile::Catalog, "off", "30s", "pg_catalog", false),
            (SessionProfile::Catalog, "on", "0", "pg_catalog", false),
            // Query: search_path önemsiz, yalnızca salt okunurluk ve süre.
            (Q_RO, "on", "0", "\"$user\", public", true),
            (Q_RO, "on", "0", "pg_catalog", true),
            (Q_RO, "off", "0", "\"$user\", public", false),
            (Q_RO, "on", "30s", "\"$user\", public", false),
            (five_min, "off", "5min", "x", true),
            (five_min, "off", "300s", "x", true),
            (five_min, "off", "300000", "x", true),
            (five_min, "on", "5min", "x", false),
            (SessionProfile::Query { read_only: true, statement_timeout_ms: 30_000 }, "on", "30s", "x", true),
        ];
        for (profile, ro, t, sp, want) in cases {
            assert_eq!(settings_ok_for(profile, ro, t, sp), want, "{profile:?} {ro} {t} {sp}");
        }
        assert!(!settings_ok_for(Q_RO, "on", "garbage", "x"));
    }

    /// Sorgu bağlantısı da `options` reddinde tek bir yedek deneme yapar.
    #[tokio::test]
    async fn dedicated_options_rejection_retries_once_without_options() {
        let (port, count) = fake_pooler("08P01", "unsupported startup parameter: options").await;
        let config = ServerConfig::new("sahte", "127.0.0.1", port, "postgres");
        let err = open_dedicated_conn(&config, "postgres", Some("x"), Q_RW).await.unwrap_err();
        assert!(matches!(&err, Error::Connect { sqlstate: Some(c), .. } if c == "08P01"), "{err:?}");
        assert_eq!(count.load(std::sync::atomic::Ordering::SeqCst), 2, "tek bir yeniden deneme bekleniyordu");
    }

    /// Aralık dışı süre, bağlantı denenmeden (oturum aranmadan bile) `Invalid` döner.
    #[tokio::test]
    async fn open_dedicated_rejects_out_of_range_statement_timeout() {
        let mgr = ConnectionManager::new();
        let too_long = SessionProfile::Query { read_only: false, statement_timeout_ms: MAX_STATEMENT_TIMEOUT_MS + 1 };
        let err = mgr.open_dedicated(Uuid::new_v4(), Target::Maintenance, too_long).await.unwrap_err();
        assert!(matches!(&err, Error::Invalid(m) if m.contains("2147483647")), "{err:?}");
        let max = SessionProfile::Query { read_only: false, statement_timeout_ms: MAX_STATEMENT_TIMEOUT_MS };
        assert!(validate_profile(max).is_ok());
        assert!(validate_profile(SessionProfile::Catalog).is_ok());
        assert_eq!(profile_session_options(max), "-c default_transaction_read_only=off -c statement_timeout=2147483647");
    }

    #[test]
    fn only_missing_database_evicts_the_cached_name() {
        let missing = Error::Connect { kind: ConnectErrorKind::Other, sqlstate: Some("3D000".into()), message: "yok".into() };
        assert!(is_missing_database(&missing));
        for err in [
            connect_err(ConnectErrorKind::Network),
            connect_err(ConnectErrorKind::Auth),
            connect_err(ConnectErrorKind::Tls),
            Error::Connect { kind: ConnectErrorKind::Other, sqlstate: Some("28000".into()), message: "x".into() },
            Error::NotFound("db".into()),
        ] {
            assert!(!is_missing_database(&err), "{err:?}");
        }
    }

    #[tokio::test]
    async fn open_dedicated_requires_a_connected_server() {
        let mgr = ConnectionManager::new();
        let err = mgr.open_dedicated(Uuid::new_v4(), Target::Maintenance, Q_RO).await.unwrap_err();
        assert!(matches!(err, Error::NotConnected), "{err:?}");
    }
}
