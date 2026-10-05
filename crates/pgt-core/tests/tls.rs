//! TLS zincir entegrasyon testi: `scripts/tls-test.ps1` iki TLS'li PostgreSQL kapsayıcısı açar ve bu testleri
//! ortam değişkenleriyle çalıştırır. Değişkenler yoksa her test atlanır.
//!
//! - `PGT_TLS_URL`: `host=127.0.0.1 port=… user=…` — sertifikası yalnızca `DNS:pgt-tls.test` taşıyan sunucu (IP yok).
//! - `PGT_TLS_IP_URL`: aynı CA'dan, `DNS:pgt-tls.test` + `IP:127.0.0.1` taşıyan sertifikalı ikinci sunucu.
//! - `PGT_TLS_PASSWORD`: iki sunucunun ortak parolası (URL'de parola yoktur).
//! - `PGT_TLS_CA`: sunucu sertifikalarını imzalayan CA (pem); `PGT_TLS_OTHER_CA`: bağımsız, ilgisiz bir CA.
//!
//! İki sunucu da yalnızca `hostssl` kabul eder.

use pgt_core::config::{ServerConfig, SslMode};
use pgt_core::connection::ConnectionManager;
use pgt_core::{ConnectErrorKind, Error};

struct Env {
    url: String,
    ip_url: String,
    password: String,
    ca: String,
    other_ca: String,
}

fn env() -> Option<Env> {
    let get = |k: &str| std::env::var(k).ok().filter(|v| !v.trim().is_empty());
    match (get("PGT_TLS_URL"), get("PGT_TLS_IP_URL"), get("PGT_TLS_PASSWORD"), get("PGT_TLS_CA"), get("PGT_TLS_OTHER_CA")) {
        (Some(url), Some(ip_url), Some(password), Some(ca), Some(other_ca)) => Some(Env { url, ip_url, password, ca, other_ca }),
        _ => {
            eprintln!("PGT_TLS_* tanımlı değil, TLS testi atlanıyor (scripts/tls-test.ps1 ile çalıştırın)");
            None
        }
    }
}

/// `host=… port=… user=…` biçimindeki adresten yapılandırma kurar.
fn config(url: &str, mode: SslMode, root_cert: Option<&str>) -> ServerConfig {
    let mut host = None;
    let mut port = None;
    let mut user = None;
    for part in url.split_whitespace() {
        match part.split_once('=') {
            Some(("host", v)) => host = Some(v.to_string()),
            Some(("port", v)) => port = Some(v.parse::<u16>().expect("port")),
            Some(("user", v)) => user = Some(v.to_string()),
            _ => panic!("tanınmayan TLS test adresi parçası: {part}"),
        }
    }
    let mut c = ServerConfig::new("tls", host.expect("host"), port.expect("port"), user.expect("user"));
    c.ssl.mode = mode;
    c.ssl.root_cert = root_cert.map(str::to_string);
    c
}

async fn connect(env: &Env, url: &str, mode: SslMode, root_cert: Option<&str>) -> pgt_core::Result<u32> {
    ConnectionManager::test(&config(url, mode, root_cert), Some(&env.password)).await.map(|i| i.version_num)
}

/// `kind = Tls` bekler; `expect` verilirse iletinin o parçayı içerdiğini de doğrular. Yalnızca kararlı
/// olan durumda verilir: açık kök sertifikayla rustls/webpki zincir hatası `UnknownIssuer` olarak görünür;
/// Windows deposu (rustls-platform-verifier) ve ad uyuşmazlığı iletileri sürüme/platforma göre değişebilir.
fn assert_tls_error(result: pgt_core::Result<u32>, expect: Option<&str>) {
    match result {
        Err(Error::Connect { kind: ConnectErrorKind::Tls, message, .. }) => {
            eprintln!("beklenen TLS hatası: {message}");
            if let Some(part) = expect {
                assert!(message.contains(part), "TLS iletisinde {part:?} bekleniyordu: {message}");
            }
        }
        other => panic!("TLS hatası bekleniyordu: {other:?}"),
    }
}

#[tokio::test]
async fn disable_is_rejected_by_hostssl_only_server() {
    let Some(env) = env() else { return };
    match connect(&env, &env.url, SslMode::Disable, None).await {
        // pg_hba'da şifresiz bağlantıya uyan satır yok: 28000 (invalid_authorization_specification).
        Err(Error::Connect { sqlstate: Some(code), .. }) => assert_eq!(code, "28000"),
        other => panic!("pg_hba reddi bekleniyordu: {other:?}"),
    }
}

#[tokio::test]
async fn prefer_establishes_tls() {
    let Some(env) = env() else { return };
    // Sunucu yalnızca hostssl kabul ettiğinden başarı, TLS'in kurulduğunu da gösterir.
    connect(&env, &env.url, SslMode::Prefer, None).await.unwrap();
}

#[tokio::test]
async fn require_does_not_verify_certificate() {
    let Some(env) = env() else { return };
    // Kök sertifika verilmedi ve CA Windows deposunda yok; yine de bağlanır.
    connect(&env, &env.url, SslMode::Require, None).await.unwrap();
}

#[tokio::test]
async fn verify_ca_with_right_ca_ignores_host_name() {
    let Some(env) = env() else { return };
    // Sertifika yalnızca pgt-tls.test adını taşır, 127.0.0.1'e bağlanılır: ad uyuşmazlığı yok sayılmalı.
    connect(&env, &env.url, SslMode::VerifyCa, Some(&env.ca)).await.unwrap();
}

#[tokio::test]
async fn verify_ca_with_other_ca_is_tls_error() {
    let Some(env) = env() else { return };
    assert_tls_error(connect(&env, &env.url, SslMode::VerifyCa, Some(&env.other_ca)).await, Some("UnknownIssuer"));
}

#[tokio::test]
async fn verify_ca_with_windows_store_is_tls_error() {
    let Some(env) = env() else { return };
    // Kök sertifika yok: sistem deposu (rustls-platform-verifier) kullanılır; özel test CA'sı orada değildir.
    assert_tls_error(connect(&env, &env.url, SslMode::VerifyCa, None).await, None);
}

#[tokio::test]
async fn verify_full_with_right_ca_rejects_name_mismatch() {
    let Some(env) = env() else { return };
    // Zincir geçerli ama 127.0.0.1 sertifikanın SAN'ında yok.
    assert_tls_error(connect(&env, &env.url, SslMode::VerifyFull, Some(&env.ca)).await, None);
}

#[tokio::test]
async fn verify_full_with_right_ca_and_ip_san_succeeds() {
    let Some(env) = env() else { return };
    // Olumlu verify-full: ikinci sunucunun sertifikası IP:127.0.0.1 SAN'ı taşır; zincir ve ad doğrulanır.
    connect(&env, &env.ip_url, SslMode::VerifyFull, Some(&env.ca)).await.unwrap();
    // Aynı sunucu ilgisiz CA ile reddedilir: başarı ad eşleşmesinden değil, zincir doğrulamasından da geçer.
    assert_tls_error(connect(&env, &env.ip_url, SslMode::VerifyFull, Some(&env.other_ca)).await, Some("UnknownIssuer"));
}

// `verify-full` + `host=pgt-tls.test` satırı (hosts girdisi olmadan) burada yoktur: bu, adın TCP için
// 127.0.0.1'e çözülmesini ama TLS'te `pgt-tls.test` olarak doğrulanmasını gerektirir (libpq/tokio-postgres
// `hostaddr`). tokio-postgres `hostaddr`'ı destekler, ancak `ServerConfig` yalnızca `host` taşır ve
// `connect_once` `hostaddr` ayarlamaz; ürün kodu değiştirilmeden ifade edilemez. Olumlu verify-full yolu
// yukarıda IP SAN'lı ikinci sunucuyla sınanır.
