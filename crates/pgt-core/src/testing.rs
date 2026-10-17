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
    urls.split(',').map(str::trim).filter(|u| !u.is_empty()).enumerate().map(|(i, u)| parse(i, u)).collect()
}

pub fn test_server() -> Option<TestServer> {
    test_servers().into_iter().next()
}

/// Adres parola içerir: hata mesajında asla adresin kendisi geçmez, yalnızca sırası ve ayrıştırma hatası.
fn parse(index: usize, url: &str) -> TestServer {
    let pg: tokio_postgres::Config =
        url.parse().unwrap_or_else(|e| panic!("PGT_TEST_URLS içindeki {}. adres geçersiz: {e}", index + 1));
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

/// Veritabanını (varsa) siler: önce oturumlarını sonlandırır. `pg_terminate_backend` yalnızca sinyal
/// gönderdiği için oturumların gerçekten kapanması `pg_stat_activity` üzerinden beklenir (en çok ~5 sn) ve
/// DROP, 55006 (object_in_use) hatasında birkaç kez yeniden denenir. `WITH (FORCE)` PG 13+ olduğundan
/// kullanılmaz (sürüm matrisi PG 10'dan başlar). `name` güvenilir test adı olmalıdır (tırnaklanmaz).
pub async fn drop_database(admin: &tokio_postgres::Client, name: &str) {
    const TERMINATE: &str =
        "SELECT pg_terminate_backend(pid) FROM pg_stat_activity WHERE datname = $1 AND pid <> pg_backend_pid()";
    const REMAINING: &str = "SELECT count(*) FROM pg_stat_activity WHERE datname = $1 AND pid <> pg_backend_pid()";
    for attempt in 0..5 {
        admin.execute(TERMINATE, &[&name]).await.expect("oturumlar sonlandırılamadı");
        for _ in 0..50 {
            let left: i64 = admin.query_one(REMAINING, &[&name]).await.expect("pg_stat_activity").get(0);
            if left == 0 {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        }
        match admin.batch_execute(&format!("DROP DATABASE IF EXISTS {name}")).await {
            Ok(()) => return,
            Err(e) if attempt < 4 && e.code() == Some(&tokio_postgres::error::SqlState::OBJECT_IN_USE) => {
                tokio::time::sleep(std::time::Duration::from_millis(200)).await;
            }
            Err(e) => panic!("{name} silinemedi: {e}"),
        }
    }
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

/// PG 14+: `u8` şemasında adı geçersiz UTF-8 olan bir multirange tipi ve onu kullanan nesneler.
///
/// PG `makeMultirangeTypeName` otomatik adı ham 52 bayta keser: `'a' || 'ş' × 31` (63 bayt) adlı range'in multirange'i
/// `a` + 25 × `ş` + yarım `ş` (0xC5) + `_multirange` olur. Ad SQL'e yazılamadığı (istemci girdisi doğrulanır) için
/// kullanan nesneler `regtype::text` ile sunucu tarafında `EXECUTE` edilir: `u8.t.m` sütunu (varsayılanı ve
/// CHECK'i tip adını içerir), `u8.dm` domain'i, `u8.f(x)` işlevi, `u8.v` görünümü; `u8.t`'de bir satır.
pub const INVALID_UTF8_TYPE_NAME_SQL: &str = r#"
CREATE SCHEMA u8;
CREATE TYPE u8."aşşşşşşşşşşşşşşşşşşşşşşşşşşşşşşş" AS RANGE (SUBTYPE = int4);
DO $$
DECLARE m text := (SELECT r.rngmultitypid::regtype::text FROM pg_range r JOIN pg_type t ON t.oid = r.rngtypid
                   WHERE t.typnamespace = 'u8'::regnamespace);
BEGIN
  EXECUTE format('CREATE TABLE u8.t (id int PRIMARY KEY, m %s DEFAULT ''{}'' CHECK (m <> ''{[5,6)}''))', m);
  EXECUTE format('CREATE DOMAIN u8.dm AS %s', m);
  EXECUTE format('CREATE FUNCTION u8.f(x %s) RETURNS int LANGUAGE sql AS ''SELECT 1''', m);
END $$;
INSERT INTO u8.t VALUES (1, '{[1,3)}');
CREATE VIEW u8.v AS SELECT m FROM u8.t;
"#;

/// `INVALID_UTF8_TYPE_NAME_SQL`'deki range adı (geçerli UTF-8).
pub fn invalid_utf8_range_name() -> String {
    format!("a{}", "ş".repeat(31))
}

#[cfg(test)]
mod tests {
    #[test]
    fn invalid_utf8_fixture_range_name_matches_sql() {
        assert_eq!(super::invalid_utf8_range_name().len(), 63);
        assert!(super::INVALID_UTF8_TYPE_NAME_SQL.contains(&format!("\"{}\"", super::invalid_utf8_range_name())));
    }

    #[test]
    fn parse_failure_does_not_leak_the_url() {
        let secret = "postgres://u:cok-gizli-parola@h:notaport/db";
        let Err(panic) = std::panic::catch_unwind(|| super::parse(1, secret)) else { panic!("ayrıştırma başarısız olmalıydı") };
        let msg = panic.downcast_ref::<String>().cloned().unwrap_or_default();
        assert!(msg.contains("2. adres"), "{msg}");
        assert!(!msg.contains("cok-gizli-parola"), "{msg}");
    }
}
