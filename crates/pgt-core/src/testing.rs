//! Entegrasyon testleri için yardımcılar. `PGT_TEST_URLS` virgülle ayrılmış
//! `postgres://kullanici:parola@host:port/postgres` adresleri içerir.

use tokio_postgres::config::Host;

use crate::config::{ServerConfig, SslMode};

#[derive(Clone)]
pub struct TestServer {
    pub config: ServerConfig,
    pub password: Option<String>,
}

pub fn test_servers() -> Vec<TestServer> {
    let Ok(urls) = std::env::var("PGT_TEST_URLS") else {
        return Vec::new();
    };
    urls.split(',').map(str::trim).filter(|u| !u.is_empty()).map(parse).collect()
}

pub fn test_server() -> Option<TestServer> {
    test_servers().into_iter().next()
}

fn parse(url: &str) -> TestServer {
    let pg: tokio_postgres::Config = url.parse().unwrap_or_else(|e| panic!("PGT_TEST_URLS geçersiz ({url}): {e}"));
    let host = match pg.get_hosts().first() {
        Some(Host::Tcp(h)) => h.clone(),
        #[allow(unreachable_patterns)]
        _ => "localhost".to_string(),
    };
    let port = pg.get_ports().first().copied().unwrap_or(5432);
    let mut config = ServerConfig::new(format!("test {host}:{port}"), host, port, pg.get_user().unwrap_or("postgres"));
    config.maintenance_db = pg.get_dbname().unwrap_or("postgres").to_string();
    config.ssl.mode = match pg.get_ssl_mode() {
        tokio_postgres::config::SslMode::Disable => SslMode::Disable,
        tokio_postgres::config::SslMode::Require => SslMode::Require,
        _ => SslMode::Prefer,
    };
    let password = pg.get_password().map(|p| String::from_utf8_lossy(p).into_owned());
    TestServer { config, password }
}

/// Fixture kurulumu için yazılabilir (read-only olmayan) bağlantı.
pub async fn admin_client(server: &TestServer, dbname: &str) -> tokio_postgres::Client {
    let mut pg = tokio_postgres::Config::new();
    pg.host(&server.config.host)
        .port(server.config.port)
        .user(&server.config.username)
        .dbname(dbname)
        .application_name("pg-tools-test")
        .ssl_mode(crate::tls::pg_ssl_mode(server.config.ssl.mode));
    if let Some(p) = &server.password {
        pg.password(p);
    }
    let connector = crate::tls::make_connector(&server.config.ssl).expect("TLS");
    let (client, connection) = pg.connect(connector).await.expect("test sunucusuna bağlanılamadı");
    tokio::spawn(async move {
        let _ = connection.await;
    });
    client
}
