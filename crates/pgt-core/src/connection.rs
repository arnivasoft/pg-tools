use std::collections::HashMap;
use std::future::Future;
use std::sync::{Arc, Mutex as StdMutex};
use std::time::{Duration, Instant};

use serde::Serialize;
use tokio::sync::Mutex;
use ts_rs::TS;
use uuid::Uuid;

use crate::config::ServerConfig;
use crate::{ConnectErrorKind, Error, Result, tls};

pub const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const SESSION_OPTIONS: &str = "-c default_transaction_read_only=on -c statement_timeout=30000";

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
    info: ServerInfo,
    conns: HashMap<Target, Arc<PgConn>>,
    db_names: HashMap<u32, String>,
}

#[derive(Default)]
pub struct ConnectionManager {
    sessions: StdMutex<HashMap<Uuid, Arc<Mutex<Session>>>>,
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
            info: info.clone(),
            conns: HashMap::from([(Target::Maintenance, Arc::new(conn))]),
            db_names: HashMap::new(),
        };
        self.sessions.lock().unwrap().insert(config.id, Arc::new(Mutex::new(session)));
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

    pub async fn info(&self, server: Uuid) -> Option<ServerInfo> {
        let session = self.sessions.lock().unwrap().get(&server).cloned()?;
        let info = session.lock().await.info.clone();
        Some(info)
    }

    fn session(&self, server: Uuid) -> Result<Arc<Mutex<Session>>> {
        self.sessions.lock().unwrap().get(&server).cloned().ok_or(Error::NotConnected)
    }

    async fn conn(&self, server: Uuid, target: Target) -> Result<Arc<PgConn>> {
        let session = self.session(server)?;
        let mut s = session.lock().await;
        conn_locked(&mut s, target).await
    }

    async fn evict(&self, server: Uuid, target: Target) {
        if let Ok(session) = self.session(server) {
            session.lock().await.conns.remove(&target);
        }
    }

    /// `f`'yi hedef bağlantıda çalıştırır; bağlantı kopmuşsa bir kez yeniden bağlanıp tekrar dener.
    pub async fn run<T, F, Fut>(&self, server: Uuid, target: Target, f: F) -> Result<T>
    where
        F: Fn(Arc<PgConn>) -> Fut,
        Fut: Future<Output = std::result::Result<T, tokio_postgres::Error>>,
    {
        let conn = self.conn(server, target).await?;
        conn.touch();
        match f(conn.clone()).await {
            Ok(v) => Ok(v),
            Err(e) if is_disconnect(&e) || !conn.usable() => {
                tracing::info!(%server, error = %e, "bağlantı kopmuş, yeniden bağlanılıyor");
                self.evict(server, target).await;
                let conn = match self.conn(server, target).await {
                    Ok(c) => c,
                    Err(err) => {
                        tracing::warn!(%server, error = %err, "yeniden bağlanılamadı, oturum kapatıldı");
                        self.disconnect(server);
                        return Err(err);
                    }
                };
                conn.touch();
                f(conn).await.map_err(Error::from_query)
            }
            Err(e) => Err(Error::from_query(e)),
        }
    }

    /// `max_idle` süresinden uzun boşta kalan bağlantıları kapatır; oturumlar bağlı kalır.
    pub async fn reap_idle(&self, max_idle: Duration) {
        let sessions: Vec<_> = self.sessions.lock().unwrap().values().cloned().collect();
        for session in sessions {
            let mut s = session.lock().await;
            s.conns.retain(|_, c| c.usable() && c.idle_for() < max_idle);
        }
    }
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
    let mut pg = tokio_postgres::Config::new();
    pg.host(&config.host)
        .port(config.port)
        .user(&config.username)
        .dbname(dbname)
        .application_name("pg-tools")
        .options(SESSION_OPTIONS)
        .ssl_mode(tls::pg_ssl_mode(config.ssl.mode))
        .connect_timeout(CONNECT_TIMEOUT)
        .keepalives(true);
    if let Some(p) = password {
        pg.password(p);
    }
    let connector = tls::make_connector(&config.ssl)?;
    let (client, connection) = match tokio::time::timeout(CONNECT_TIMEOUT + Duration::from_secs(1), pg.connect(connector)).await {
        Ok(result) => result.map_err(Error::from_connect)?,
        Err(_) => {
            return Err(Error::Connect { kind: ConnectErrorKind::Timeout, sqlstate: None, message: "zaman aşımı".into() });
        }
    };
    tokio::spawn(async move {
        if let Err(e) = connection.await {
            tracing::debug!(error = %e, "PostgreSQL bağlantısı kapandı");
        }
    });
    let row = client
        .query_one("SELECT current_setting('server_version_num')::int4, current_setting('server_version')", &[])
        .await
        .map_err(Error::from_query)?;
    let num: i32 = row.get(0);
    let full: String = row.get(1);
    let version = full.split_whitespace().next().unwrap_or(&full).to_string();
    Ok(PgConn { client, info: ServerInfo { version_num: num as u32, version }, last_used: StdMutex::new(Instant::now()) })
}
