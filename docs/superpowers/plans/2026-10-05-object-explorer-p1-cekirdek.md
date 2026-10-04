# Object Explorer — Plan 1/3: Çalışan Çekirdek (Vertical Slice) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Sunucu ekleyip bağlanabilen; Server → Databases → Schemas → Tables → Columns ağacında sanal kaydırmayla gezilebilen; seçili nesnenin Properties ve DDL'ini gösteren çalışan bir Tauri uygulaması.

**Architecture:** İş Rust'ta yapılır: `pgt-core` (kayıt defteri, kimlik bilgileri, TLS, bağlantılar) ve `pgt-catalog` (düğüm modeli, sürüme duyarlı katalog SQL'i, Properties, DDL). `src-tauri` yalnızca komutları yönlendirir. SolidJS ön yüz, ts-rs ile üretilmiş tiplerle Tauri komutlarını çağırır ve ağacı TanStack Virtual ile çizer.

**Tech Stack:** Rust 2024 (toolchain 1.99), Tauri 2.12, tokio-postgres 0.7 + tokio-postgres-rustls 0.14 + rustls 0.23 (ring) + rustls-platform-verifier 0.7, keyring-core 1 + windows-native-keyring-store 1, ts-rs 12, insta, rcgen (test); bun, Vite 8, SolidJS 1.9, Kobalte 0.13, Tailwind CSS 4, TanStack Solid Virtual 3, CodeMirror 6, lucide-solid 1.x, Vitest 5, TypeScript 7.

**Spec:** `docs/superpowers/specs/2026-10-05-object-explorer-design.md`

## Plan dizisi

Spec üç plana bölünmüştür; her plan kendi başına çalışan, test edilmiş yazılım üretir:

| Plan | Kapsam (spec bölümleri) |
|---|---|
| **1 (bu belge)** | Proje iskeleti; §4.1–4.4 kayıt defteri, Credential Manager, TLS, bağlantı modeli; §3 veri modeli; §5 yalnızca Server/Database/Schema/Table/Column için liste + Properties + temel DDL; §6.1, §6.2 (filtre ve sağ tık menüsü hariç), §6.3 sunucu diyaloğu + parola istemi, §6.4, §6.5; §7.1, §7.3 |
| 2 | §5'in geri kalanı: Constraints, Indexes, Triggers, RLS Policies, Partitions, Views, MatViews, Functions/Procedures/Trigger Functions, Sequences, Types/Domains, Extensions, Roles, Tablespaces; tam Table DDL (partition/inherits/index/trigger/policy, PG18 NOT NULL adları, identity seçenekleri, kolon yetkileri); DDL gidiş-dönüş testi; Docker sürüm matrisi (10–18); 5.000 tablo performans testi |
| 3 | §4.5 pgAdmin içe aktarma; §6.2 filtre, sağ tık menüsü, kopyalama eylemleri, sunucu silme, bağlantıyı kes; §7.2 hata rozeti detayları/toast/NotFound akışı; sistem şemaları ayarı; durum çubuğu uyarıları; açılış/bellek ölçüm listesi |

## Global Constraints

- Hedef istemci OS: Windows 10/11, Windows Server 2019/2022. Win7/Server 2012 desteklenmez.
- PostgreSQL 10–18, yalnızca vanilla. Sürüme özel SQL dosyaları `.pg10` / `.pg11` sonekiyle ayrılır.
- Her PostgreSQL oturumu: `application_name = 'pg-tools'`, `default_transaction_read_only = on`, `statement_timeout = 30s`. Bağlantı zaman aşımı 10 sn.
- Parola hiçbir dosyaya ve log'a yazılmaz; yalnızca Windows Credential Manager (`pg-tools` servisi) veya oturum belleği.
- Sunucu listesi: `%APPDATA%\pg-tools\servers.json`, atomik yazma. Loglar: `%LOCALAPPDATA%\pg-tools\logs`, son 5 dosya.
- Nesne kimlikleri OID tabanlıdır (`srv:<uuid>/db:<oid>/sch:<oid>/tbl:<oid>/col:<attnum>`); ön yüz kimlik string'ini asla ayrıştırmaz.
- Tırnaklama sunucuda yapılır (`quote_ident`, `quote_literal`, `format_type`); Rust'ta rezerve kelime listesi yoktur.
- Arayüz metinleri `src/app/strings.ts` içindedir: Türkçe komutlar, İngilizce PostgreSQL terimleri (Databases, Tables, Columns…).
- Web fontu yok (Segoe UI Variable, Cascadia Mono → Consolas). Ağaç satırı 24 px.
- CodeMirror yalnızca DDL sekmesi ilk açıldığında yüklenir (dinamik `import()`).
- TLS kripto sağlayıcısı **ring**'dir; hiçbir crate'te `aws-lc-rs` özelliği açılmaz (Windows'ta CMake/NASM gerektirir).
- Paket yöneticisi bun. `bun` PATH'te değilse `~/.bun/bin/bun` kullanın.
- Entegrasyon testleri `PGT_TEST_URLS` (virgülle ayrılmış `postgres://user:pass@host:port/postgres`) tanımlı değilse "atlanıyor" yazıp başarıyla döner. Test sunucusu parola kimlik doğrulaması (scram-sha-256) kullanmalı ve test kullanıcısı `CREATEDB` + `CREATEROLE` (veya superuser) yetkisine sahip olmalı.

### Spec'ten bilinçli sapmalar (plan hazırlanırken doğrulandı)

1. **tauri-specta yerine ts-rs:** tauri-specta'nın Tauri 2 sürümü hâlâ `2.0.0-rc.25`. Tipler ts-rs 12 ile Rust'tan üretilir (`src/ipc/bindings/`); ~10 komut sarmalayıcısı `src/ipc/commands.ts` içinde elle yazılır. Spec §10'daki yedek plan budur.
2. **"En fazla 2 bağlantılık havuz" yerine (sunucu, veritabanı) başına tek paylaşılan `tokio_postgres::Client`:** tokio-postgres tek bağlantı üzerinde eşzamanlı sorguları pipeline eder; gezgin için ikinci bağlantı gereksizdir. 10 dk boşta kapanma korunur.
3. **Şifreli (passphrase'li) istemci anahtarları desteklenmez:** rustls şifreli PEM okuyamaz. Hata mesajı `openssl pkcs8 -topk8 -nocrypt` dönüşümünü önerir. Bu yüzden passphrase saklama yoktur.
4. **Sürüm bilgisi bir sorguyla okunur:** tokio-postgres `ParameterStatus` değerlerini dışa açmaz; bağlantı kurulunca tek `SELECT current_setting('server_version_num')` çalışır.

## Review Focus

1. **Bağlantı koptuktan sonra ilk işlem** (sunucu yeniden başlatıldı / VPN düştü / `pg_terminate_backend`): kullanıcı hata görmeden sonuç almalı (bir kez sessiz yeniden bağlanma). Test: Task 4 `reconnects_after_backend_terminated`.
2. **Tırnak gerektiren isimler** (`"Order Items"`, `"Line No"`, büyük harf, Türkçe karakter): ağaçta ham ad, DDL'de doğru tırnaklı ad görünmeli ve DDL çalıştırılabilmeli. Test: Task 8 `ddl_executes_in_scratch_database` + `"Order Items"` snapshot'ı.
3. **Bağlantıya izin vermeyen veritabanı** (`datallowconn = false`) ve CONNECT yetkisi olmayan veritabanı: liste ve Properties çökmemeli. Test: Task 6 `databases_without_connect_are_leaves`, Task 7 `database_properties_without_connect_privilege`.
4. **Bozuk `servers.json`**: uygulama açılmalı, dosya yedeklenmeli, kullanıcı bilgilendirilmeli. Test: Task 1 `corrupt_file_is_backed_up`, Task 10 `startup_notice_after_corrupt_registry`.
5. **Kayıtlı parola artık yanlış** (şifre değişti): bağlanırken parola istemi yeniden açılmalı, yeni parola kaydedilebilmeli. Test: Task 12 `auth error re-prompts and retries with new password`.

## Dosya Haritası

```
Cargo.toml                         workspace + ortak bağımlılıklar + release profili
.cargo/config.toml                 TS_RS_EXPORT_DIR
crates/pgt-core/
  src/lib.rs                       modül dışa aktarımları
  src/error.rs                     Error, ErrorPayload, tokio-postgres hata sınıflandırma
  src/config.rs                    ServerConfig, SslConfig, SslMode, doğrulama
  src/registry.rs                  servers.json (atomik yazma, bozuk dosya yedeği)
  src/credentials.rs               SecretStore trait, Memory/Windows store, Credentials
  src/tls.rs                       sslmode → rustls ClientConfig, doğrulayıcılar
  src/connection.rs                ConnectionManager, PgConn, Target, run() + yeniden bağlanma
  src/testing.rs                   (feature "testing") PGT_TEST_URLS ayrıştırma
  tests/connection.rs              gerçek PostgreSQL entegrasyon testleri
crates/pgt-catalog/
  src/lib.rs                       Catalog (list_children / properties / ddl dağıtımı)
  src/node.rs                      NodeId, Seg, Coll, NodeKind, TreeNode, NodeFlags
  src/version.rs                   PgVersion, VersionedSql
  src/props.rs                     PropertySection, PropertyRow, approx_rows
  src/db.rs                        sorgu yardımcıları (Params, p! makrosu)
  src/kinds/{mod,server,database,schema,table,column}.rs
  src/ddl.rs                       header/owner/comment/grant yardımcıları
  src/sql/*.sql                    katalog sorguları
  tests/common/mod.rs              fixture veritabanı kurulumu
  tests/fixtures/schema.sql        test şeması
  tests/{listing,properties,ddl}.rs
src-tauri/                         Tauri kabuğu (main, state, commands, logging)
src/                               SolidJS ön yüz
  app/       App, styles.css, theme, strings, SplitPane, Toolbar, StatusBar
  ipc/       bindings/ (üretilmiş), commands.ts, errors.ts
  features/explorer/   tree-model.ts (+test), explorer-store.ts (+test), TreeView, TreeRow, NodeIcon
  features/servers/    ServerDialog, PasswordPrompt, server-form.ts (+test)
  features/inspector/  Inspector, PropertiesView, DdlView (lazy), codemirror-setup.ts
  ui/        Button, Dialog, Tabs, Field
```

---

### Task 1: Workspace iskeleti, hata modeli, sunucu yapılandırması ve kayıt defteri

**Files:**
- Create: `Cargo.toml`, `.cargo/config.toml`, `crates/pgt-core/Cargo.toml`, `crates/pgt-core/src/lib.rs`, `crates/pgt-core/src/error.rs`, `crates/pgt-core/src/config.rs`, `crates/pgt-core/src/registry.rs`
- Modify: `.gitignore`

**Interfaces:**
- Produces:
  - `pgt_core::{Error, Result, ConnectErrorKind, ErrorPayload}`
  - `Error::{Config(String), Invalid(String), Credential(String), PasswordRequired, Connect { kind: ConnectErrorKind, sqlstate: Option<String>, message: String }, Query { sqlstate: Option<String>, message: String, detail: Option<String>, hint: Option<String> }, NotFound(String), UnknownServer(Uuid), NotConnected}`
  - `Error::from_query(tokio_postgres::Error) -> Error`, `Error::from_connect(tokio_postgres::Error) -> Error`
  - `ErrorPayload { code: String, message: String, detail: Option<String>, hint: Option<String> }` + `From<Error>` ve `From<&Error>`. Kodlar: `config, invalid, credential, password_required, auth, network, tls, timeout, connect, query, not_found, unknown_server, not_connected`.
  - `pgt_core::config::{ServerConfig, SslConfig, SslMode}`; `ServerConfig::new(name, host, port: u16, username) -> ServerConfig`, `.normalized() -> ServerConfig`, `.validate() -> Result<()>`, `.group_name() -> Option<&str>`
  - `pgt_core::registry::{Registry, LoadOutcome { registry, recovered_backup: Option<PathBuf> }}`; `Registry::load(impl Into<PathBuf>) -> Result<LoadOutcome>`, `.path()`, `.servers() -> &[ServerConfig]`, `.get(Uuid) -> Option<&ServerConfig>`, `.upsert(ServerConfig) -> Result<ServerConfig>`, `.remove(Uuid) -> Result<Option<ServerConfig>>`

- [ ] **Step 1: Workspace dosyalarını oluştur**

`Cargo.toml`:

```toml
[workspace]
resolver = "3"
members = ["crates/pgt-core", "crates/pgt-catalog"]

[workspace.package]
version = "0.1.0"
edition = "2024"
rust-version = "1.88"

[workspace.dependencies]
pgt-core = { path = "crates/pgt-core" }
pgt-catalog = { path = "crates/pgt-catalog" }
tokio = { version = "1.53", features = ["rt-multi-thread", "macros", "sync", "time"] }
tokio-postgres = "0.7.18"
tokio-postgres-rustls = "0.14"
rustls = { version = "0.23", default-features = false, features = ["ring", "std", "tls12", "logging"] }
rustls-platform-verifier = "0.7"
keyring-core = "1"
windows-native-keyring-store = { version = "1.1", default-features = false }
serde = { version = "1", features = ["derive"] }
serde_json = "1"
uuid = { version = "1.27", features = ["v4", "serde"] }
thiserror = "2"
tracing = "0.1"
ts-rs = { version = "12", features = ["uuid-impl"] }
tempfile = "3"
insta = "1.49"
rcgen = "0.14"

[profile.release]
codegen-units = 1
lto = true
strip = true
```

> `pgt-catalog` henüz yok; bu task'ta `members` listesinden geçici olarak çıkarın, Task 5'te geri ekleyin.

`.cargo/config.toml`:

```toml
[env]
TS_RS_EXPORT_DIR = { value = "src/ipc/bindings", relative = true }
TS_RS_LARGE_INT = "number"
```

`crates/pgt-core/Cargo.toml`:

```toml
[package]
name = "pgt-core"
version.workspace = true
edition.workspace = true
rust-version.workspace = true

[features]
testing = []

[dependencies]
tokio.workspace = true
tokio-postgres.workspace = true
tokio-postgres-rustls.workspace = true
rustls.workspace = true
rustls-platform-verifier.workspace = true
keyring-core.workspace = true
serde.workspace = true
serde_json.workspace = true
uuid.workspace = true
thiserror.workspace = true
tracing.workspace = true
ts-rs.workspace = true

[target.'cfg(windows)'.dependencies]
windows-native-keyring-store.workspace = true

[dev-dependencies]
tempfile.workspace = true
rcgen.workspace = true
```

`crates/pgt-core/src/lib.rs` (config ve registry satırları Step 6'ya kadar yorumlu):

```rust
// pub mod config;
pub mod error;
// pub mod registry;

pub use error::{ConnectErrorKind, Error, ErrorPayload, Result};
```

`.gitignore` sonuna ekle:

```
src-tauri/gen/
*.json.tmp
```

- [ ] **Step 2: Hata modeli için başarısız testleri yaz**

`crates/pgt-core/src/error.rs` (şimdilik yalnızca testler):

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn auth_connect_error_gets_turkish_message() {
        let e = Error::Connect {
            kind: ConnectErrorKind::Auth,
            sqlstate: Some("28P01".into()),
            message: "password authentication failed for user \"x\"".into(),
        };
        let p = ErrorPayload::from(&e);
        assert_eq!(p.code, "auth");
        assert_eq!(p.message, "Kimlik doğrulama başarısız: kullanıcı adı veya parola hatalı.");
        assert_eq!(p.detail.as_deref(), Some("password authentication failed for user \"x\""));
    }

    #[test]
    fn network_error_wraps_original_message() {
        let e = Error::Connect { kind: ConnectErrorKind::Network, sqlstate: None, message: "connection refused".into() };
        let p = ErrorPayload::from(&e);
        assert_eq!(p.code, "network");
        assert_eq!(p.message, "Sunucuya ulaşılamadı: connection refused");
    }

    #[test]
    fn timeout_and_tls_codes() {
        let t = ErrorPayload::from(&Error::Connect { kind: ConnectErrorKind::Timeout, sqlstate: None, message: "x".into() });
        assert_eq!((t.code.as_str(), t.message.as_str()), ("timeout", "Bağlantı zaman aşımına uğradı (10 sn)."));
        let s = ErrorPayload::from(&Error::Connect { kind: ConnectErrorKind::Tls, sqlstate: None, message: "bad cert".into() });
        assert_eq!((s.code.as_str(), s.message.as_str()), ("tls", "TLS/sertifika hatası: bad cert"));
    }

    #[test]
    fn query_error_with_known_sqlstate_is_translated_and_keeps_hint() {
        let e = Error::Query {
            sqlstate: Some("42501".into()),
            message: "permission denied for table x".into(),
            detail: None,
            hint: Some("ipucu".into()),
        };
        let p = ErrorPayload::from(&e);
        assert_eq!(p.code, "query");
        assert_eq!(p.message, "Bu işlem için yetkiniz yok.");
        assert_eq!(p.detail.as_deref(), Some("permission denied for table x"));
        assert_eq!(p.hint.as_deref(), Some("ipucu"));
    }

    #[test]
    fn query_error_with_unknown_sqlstate_keeps_message() {
        let e = Error::Query { sqlstate: Some("XX000".into()), message: "boom".into(), detail: Some("d".into()), hint: None };
        let p = ErrorPayload::from(&e);
        assert_eq!((p.message.as_str(), p.detail.as_deref()), ("boom", Some("d")));
    }

    #[test]
    fn simple_variants_have_stable_codes() {
        let cases = [
            (Error::Config("c".into()), "config"),
            (Error::Invalid("i".into()), "invalid"),
            (Error::Credential("k".into()), "credential"),
            (Error::PasswordRequired, "password_required"),
            (Error::NotFound("n".into()), "not_found"),
            (Error::UnknownServer(uuid::Uuid::nil()), "unknown_server"),
            (Error::NotConnected, "not_connected"),
        ];
        for (e, code) in cases {
            let p = ErrorPayload::from(&e);
            assert_eq!(p.code, code);
            assert_eq!(p.message, e.to_string());
        }
    }
}
```

- [ ] **Step 3: Testin derlenmediğini doğrula**

Run: `cargo test -p pgt-core error`
Expected: FAIL — `cannot find type Error in this scope` vb.

- [ ] **Step 4: Hata modelini uygula** (testlerin üstüne)

```rust
use serde::Serialize;
use ts_rs::TS;
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConnectErrorKind {
    Auth,
    Network,
    Tls,
    Timeout,
    Other,
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("Yapılandırma hatası: {0}")]
    Config(String),
    #[error("Geçersiz ayar: {0}")]
    Invalid(String),
    #[error("Kimlik bilgisi deposu hatası: {0}")]
    Credential(String),
    #[error("Bu sunucu için parola gerekli.")]
    PasswordRequired,
    #[error("{message}")]
    Connect { kind: ConnectErrorKind, sqlstate: Option<String>, message: String },
    #[error("{message}")]
    Query { sqlstate: Option<String>, message: String, detail: Option<String>, hint: Option<String> },
    #[error("Nesne artık mevcut değil: {0}")]
    NotFound(String),
    #[error("Sunucu kayıtlı değil: {0}")]
    UnknownServer(Uuid),
    #[error("Sunucuya bağlı değil.")]
    NotConnected,
}

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ErrorPayload {
    pub code: String,
    pub message: String,
    pub detail: Option<String>,
    pub hint: Option<String>,
}

fn friendly_sqlstate(code: &str) -> Option<&'static str> {
    match code {
        "28P01" | "28000" => Some("Kimlik doğrulama başarısız: kullanıcı adı veya parola hatalı."),
        "3D000" => Some("Veritabanı bulunamadı."),
        "42501" => Some("Bu işlem için yetkiniz yok."),
        "57014" => Some("Sorgu zaman aşımına uğradı (statement_timeout)."),
        "53300" => Some("Sunucu bağlantı sınırına ulaşıldı."),
        _ => None,
    }
}

fn payload(code: &str, message: String, detail: Option<String>, hint: Option<String>) -> ErrorPayload {
    ErrorPayload { code: code.to_string(), message, detail, hint }
}

impl From<&Error> for ErrorPayload {
    fn from(e: &Error) -> Self {
        match e {
            Error::Config(_) => payload("config", e.to_string(), None, None),
            Error::Invalid(_) => payload("invalid", e.to_string(), None, None),
            Error::Credential(_) => payload("credential", e.to_string(), None, None),
            Error::PasswordRequired => payload("password_required", e.to_string(), None, None),
            Error::Connect { kind, sqlstate, message } => {
                let friendly = sqlstate.as_deref().and_then(friendly_sqlstate);
                let (code, text) = match kind {
                    ConnectErrorKind::Auth => ("auth", friendly.map(str::to_string).unwrap_or_else(|| message.clone())),
                    ConnectErrorKind::Network => ("network", format!("Sunucuya ulaşılamadı: {message}")),
                    ConnectErrorKind::Tls => ("tls", format!("TLS/sertifika hatası: {message}")),
                    ConnectErrorKind::Timeout => ("timeout", "Bağlantı zaman aşımına uğradı (10 sn).".to_string()),
                    ConnectErrorKind::Other => ("connect", friendly.map(str::to_string).unwrap_or_else(|| message.clone())),
                };
                let detail = (text != *message).then(|| message.clone());
                payload(code, text, detail, None)
            }
            Error::Query { sqlstate, message, detail, hint } => match sqlstate.as_deref().and_then(friendly_sqlstate) {
                Some(f) => payload("query", f.to_string(), detail.clone().or_else(|| Some(message.clone())), hint.clone()),
                None => payload("query", message.clone(), detail.clone(), hint.clone()),
            },
            Error::NotFound(_) => payload("not_found", e.to_string(), None, None),
            Error::UnknownServer(_) => payload("unknown_server", e.to_string(), None, None),
            Error::NotConnected => payload("not_connected", e.to_string(), None, None),
        }
    }
}

impl From<Error> for ErrorPayload {
    fn from(e: Error) -> Self {
        ErrorPayload::from(&e)
    }
}

impl Error {
    /// Sorgu sırasında oluşan tokio-postgres hatasını dönüştürür.
    pub fn from_query(e: tokio_postgres::Error) -> Self {
        match e.as_db_error() {
            Some(db) => Error::Query {
                sqlstate: Some(db.code().code().to_string()),
                message: db.message().to_string(),
                detail: db.detail().map(str::to_string),
                hint: db.hint().map(str::to_string),
            },
            None => Error::Query { sqlstate: None, message: chain_message(&e), detail: None, hint: None },
        }
    }

    /// Bağlantı kurulurken oluşan tokio-postgres hatasını sınıflandırır.
    pub fn from_connect(e: tokio_postgres::Error) -> Self {
        if let Some(db) = e.as_db_error() {
            let code = db.code().code();
            let kind = if code.starts_with("28") { ConnectErrorKind::Auth } else { ConnectErrorKind::Other };
            return Error::Connect { kind, sqlstate: Some(code.to_string()), message: db.message().to_string() };
        }
        let message = chain_message(&e);
        if message.contains("password missing") {
            return Error::PasswordRequired;
        }
        Error::Connect { kind: classify_source(&e), sqlstate: None, message }
    }
}

fn chain_message(e: &(dyn std::error::Error + 'static)) -> String {
    let mut parts = vec![e.to_string()];
    let mut cur = e.source();
    while let Some(err) = cur {
        let text = err.to_string();
        if !parts.iter().any(|p| p.contains(&text)) {
            parts.push(text);
        }
        cur = err.source();
    }
    parts.join(": ")
}

fn classify_source(e: &(dyn std::error::Error + 'static)) -> ConnectErrorKind {
    let mut cur: Option<&(dyn std::error::Error + 'static)> = Some(e);
    while let Some(err) = cur {
        if err.downcast_ref::<rustls::Error>().is_some() {
            return ConnectErrorKind::Tls;
        }
        if let Some(io) = err.downcast_ref::<std::io::Error>() {
            if io.get_ref().is_some_and(|inner| inner.downcast_ref::<rustls::Error>().is_some()) {
                return ConnectErrorKind::Tls;
            }
            if io.kind() == std::io::ErrorKind::TimedOut {
                return ConnectErrorKind::Timeout;
            }
            return ConnectErrorKind::Network;
        }
        cur = err.source();
    }
    ConnectErrorKind::Other
}
```

- [ ] **Step 5: Hata testlerinin geçtiğini doğrula**

Run: `cargo test -p pgt-core error`
Expected: PASS (6 test). Workspace kökünde `src/ipc/bindings/ErrorPayload.ts` oluşur.

- [ ] **Step 6: Yapılandırma ve kayıt defteri için başarısız testleri yaz**

`lib.rs`'deki `config` ve `registry` satırlarının yorumunu kaldır.

`crates/pgt-core/src/config.rs` (şimdilik yalnızca testler):

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_has_sane_defaults() {
        let c = ServerConfig::new("prod", "db.local", 5432, "app");
        assert_eq!(c.maintenance_db, "postgres");
        assert_eq!(c.ssl.mode, SslMode::Prefer);
        assert!(!c.save_password);
        assert!(c.group.is_none());
    }

    #[test]
    fn normalized_trims_and_drops_empty_optionals() {
        let mut c = ServerConfig::new("  prod ", " db.local ", 5432, " app ");
        c.group = Some("   ".into());
        c.ssl.root_cert = Some("".into());
        c.ssl.client_cert = Some(" C:\\certs\\c.pem ".into());
        let n = c.normalized();
        assert_eq!((n.name.as_str(), n.host.as_str(), n.username.as_str()), ("prod", "db.local", "app"));
        assert_eq!(n.group, None);
        assert_eq!(n.ssl.root_cert, None);
        assert_eq!(n.ssl.client_cert.as_deref(), Some("C:\\certs\\c.pem"));
    }

    #[test]
    fn validate_rejects_missing_fields() {
        let ok = ServerConfig::new("prod", "h", 5432, "u");
        assert!(ok.validate().is_ok());
        let mut c = ok.clone();
        c.name = "".into();
        assert!(matches!(c.validate(), Err(Error::Invalid(m)) if m.contains("Sunucu adı")));
        let mut c = ok.clone();
        c.port = 0;
        assert!(matches!(c.validate(), Err(Error::Invalid(m)) if m.contains("Port")));
        let mut c = ok.clone();
        c.ssl.client_cert = Some("c.pem".into());
        assert!(matches!(c.validate(), Err(Error::Invalid(m)) if m.contains("birlikte")));
    }

    #[test]
    fn json_uses_camel_case_and_kebab_ssl_mode() {
        let mut c = ServerConfig::new("p", "h", 5432, "u");
        c.ssl.mode = SslMode::VerifyFull;
        let v = serde_json::to_value(&c).unwrap();
        assert_eq!(v["maintenanceDb"], "postgres");
        assert_eq!(v["savePassword"], false);
        assert_eq!(v["ssl"]["mode"], "verify-full");
    }
}
```

`crates/pgt-core/src/registry.rs` (şimdilik yalnızca testler):

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::ServerConfig;

    fn path(dir: &tempfile::TempDir) -> std::path::PathBuf {
        dir.path().join("pg-tools").join("servers.json")
    }

    #[test]
    fn load_missing_file_gives_empty() {
        let dir = tempfile::tempdir().unwrap();
        let out = Registry::load(path(&dir)).unwrap();
        assert!(out.registry.servers().is_empty());
        assert!(out.recovered_backup.is_none());
    }

    #[test]
    fn upsert_persists_and_reloads() {
        let dir = tempfile::tempdir().unwrap();
        let mut reg = Registry::load(path(&dir)).unwrap().registry;
        let saved = reg.upsert(ServerConfig::new(" prod ", "h", 5432, "u")).unwrap();
        assert_eq!(saved.name, "prod");
        let again = Registry::load(path(&dir)).unwrap().registry;
        assert_eq!(again.servers(), &[saved.clone()]);
        assert_eq!(again.get(saved.id), Some(&saved));
        assert!(!path(&dir).with_extension("json.tmp").exists());
    }

    #[test]
    fn upsert_replaces_existing_by_id() {
        let dir = tempfile::tempdir().unwrap();
        let mut reg = Registry::load(path(&dir)).unwrap().registry;
        let mut c = reg.upsert(ServerConfig::new("a", "h", 5432, "u")).unwrap();
        c.name = "b".into();
        reg.upsert(c.clone()).unwrap();
        assert_eq!(reg.servers().len(), 1);
        assert_eq!(reg.servers()[0].name, "b");
    }

    #[test]
    fn upsert_rejects_invalid_and_does_not_save() {
        let dir = tempfile::tempdir().unwrap();
        let mut reg = Registry::load(path(&dir)).unwrap().registry;
        assert!(reg.upsert(ServerConfig::new("", "h", 5432, "u")).is_err());
        assert!(!path(&dir).exists());
    }

    #[test]
    fn remove_deletes_and_persists() {
        let dir = tempfile::tempdir().unwrap();
        let mut reg = Registry::load(path(&dir)).unwrap().registry;
        let c = reg.upsert(ServerConfig::new("a", "h", 5432, "u")).unwrap();
        assert_eq!(reg.remove(c.id).unwrap().map(|s| s.id), Some(c.id));
        assert!(reg.remove(c.id).unwrap().is_none());
        assert!(Registry::load(path(&dir)).unwrap().registry.servers().is_empty());
    }

    #[test]
    fn corrupt_file_is_backed_up() {
        let dir = tempfile::tempdir().unwrap();
        let p = path(&dir);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(&p, "{ bozuk json").unwrap();
        let out = Registry::load(&p).unwrap();
        assert!(out.registry.servers().is_empty());
        let backup = out.recovered_backup.expect("yedek yolu");
        assert!(backup.file_name().unwrap().to_str().unwrap().starts_with("servers.json.bak-"));
        assert_eq!(std::fs::read_to_string(backup).unwrap(), "{ bozuk json");
        assert!(!p.exists());
    }
}
```

- [ ] **Step 7: Testlerin başarısız olduğunu doğrula**

Run: `cargo test -p pgt-core config registry`
Expected: FAIL — `ServerConfig`, `Registry` tanımlı değil.

- [ ] **Step 8: `config.rs` uygulamasını yaz** (testlerin üstüne)

```rust
use serde::{Deserialize, Serialize};
use ts_rs::TS;
use uuid::Uuid;

use crate::{Error, Result};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "kebab-case")]
#[ts(export)]
pub enum SslMode {
    Disable,
    #[default]
    Prefer,
    Require,
    VerifyCa,
    VerifyFull,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct SslConfig {
    pub mode: SslMode,
    pub root_cert: Option<String>,
    pub client_cert: Option<String>,
    pub client_key: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ServerConfig {
    pub id: Uuid,
    pub name: String,
    pub group: Option<String>,
    pub host: String,
    pub port: u16,
    pub maintenance_db: String,
    pub username: String,
    pub save_password: bool,
    #[serde(default)]
    pub ssl: SslConfig,
}

fn clean(value: Option<String>) -> Option<String> {
    value.map(|v| v.trim().to_string()).filter(|v| !v.is_empty())
}

impl ServerConfig {
    pub fn new(name: impl Into<String>, host: impl Into<String>, port: u16, username: impl Into<String>) -> Self {
        Self {
            id: Uuid::new_v4(),
            name: name.into(),
            group: None,
            host: host.into(),
            port,
            maintenance_db: "postgres".into(),
            username: username.into(),
            save_password: false,
            ssl: SslConfig::default(),
        }
    }

    /// Boşlukları kırpar, boş isteğe bağlı alanları `None` yapar.
    pub fn normalized(mut self) -> Self {
        self.name = self.name.trim().to_string();
        self.host = self.host.trim().to_string();
        self.username = self.username.trim().to_string();
        self.maintenance_db = self.maintenance_db.trim().to_string();
        self.group = clean(self.group);
        self.ssl.root_cert = clean(self.ssl.root_cert);
        self.ssl.client_cert = clean(self.ssl.client_cert);
        self.ssl.client_key = clean(self.ssl.client_key);
        self
    }

    pub fn validate(&self) -> Result<()> {
        let invalid = |m: &str| Err(Error::Invalid(m.to_string()));
        if self.name.trim().is_empty() {
            return invalid("Sunucu adı boş olamaz.");
        }
        if self.host.trim().is_empty() {
            return invalid("Host boş olamaz.");
        }
        if self.port == 0 {
            return invalid("Port 1-65535 arasında olmalı.");
        }
        if self.username.trim().is_empty() {
            return invalid("Kullanıcı adı boş olamaz.");
        }
        if self.maintenance_db.trim().is_empty() {
            return invalid("Bakım veritabanı boş olamaz.");
        }
        if self.ssl.client_cert.is_some() != self.ssl.client_key.is_some() {
            return invalid("İstemci sertifikası ve anahtarı birlikte verilmelidir.");
        }
        Ok(())
    }

    pub fn group_name(&self) -> Option<&str> {
        self.group.as_deref().map(str::trim).filter(|g| !g.is_empty())
    }
}
```

- [ ] **Step 9: `registry.rs` uygulamasını yaz** (testlerin üstüne)

```rust
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::config::ServerConfig;
use crate::{Error, Result};

#[derive(Deserialize)]
struct RegistryFile {
    servers: Vec<ServerConfig>,
}

#[derive(Serialize)]
struct RegistryFileRef<'a> {
    version: u32,
    servers: &'a [ServerConfig],
}

pub struct Registry {
    path: PathBuf,
    servers: Vec<ServerConfig>,
}

pub struct LoadOutcome {
    pub registry: Registry,
    /// Dosya bozuk olduğu için yedeklendiyse yedeğin yolu.
    pub recovered_backup: Option<PathBuf>,
}

fn io_err(e: std::io::Error) -> Error {
    Error::Config(e.to_string())
}

impl Registry {
    pub fn load(path: impl Into<PathBuf>) -> Result<LoadOutcome> {
        let path = path.into();
        let text = match fs::read_to_string(&path) {
            Ok(t) => t,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                return Ok(LoadOutcome { registry: Registry { path, servers: Vec::new() }, recovered_backup: None });
            }
            Err(e) => return Err(Error::Config(format!("{}: {e}", path.display()))),
        };
        match serde_json::from_str::<RegistryFile>(&text) {
            Ok(file) => Ok(LoadOutcome { registry: Registry { path, servers: file.servers }, recovered_backup: None }),
            Err(parse_error) => {
                let backup = backup_path(&path);
                fs::rename(&path, &backup).map_err(io_err)?;
                tracing::warn!(error = %parse_error, backup = %backup.display(), "servers.json okunamadı, yedeklendi");
                Ok(LoadOutcome { registry: Registry { path, servers: Vec::new() }, recovered_backup: Some(backup) })
            }
        }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn servers(&self) -> &[ServerConfig] {
        &self.servers
    }

    pub fn get(&self, id: Uuid) -> Option<&ServerConfig> {
        self.servers.iter().find(|s| s.id == id)
    }

    pub fn upsert(&mut self, config: ServerConfig) -> Result<ServerConfig> {
        let config = config.normalized();
        config.validate()?;
        match self.servers.iter_mut().find(|s| s.id == config.id) {
            Some(slot) => *slot = config.clone(),
            None => self.servers.push(config.clone()),
        }
        self.save()?;
        Ok(config)
    }

    pub fn remove(&mut self, id: Uuid) -> Result<Option<ServerConfig>> {
        let Some(index) = self.servers.iter().position(|s| s.id == id) else {
            return Ok(None);
        };
        let removed = self.servers.remove(index);
        self.save()?;
        Ok(Some(removed))
    }

    fn save(&self) -> Result<()> {
        if let Some(dir) = self.path.parent() {
            fs::create_dir_all(dir).map_err(io_err)?;
        }
        let json = serde_json::to_string_pretty(&RegistryFileRef { version: 1, servers: &self.servers })
            .map_err(|e| Error::Config(e.to_string()))?;
        let tmp = self.path.with_extension("json.tmp");
        fs::write(&tmp, json).map_err(io_err)?;
        fs::rename(&tmp, &self.path).map_err(io_err)?;
        Ok(())
    }
}

fn backup_path(path: &Path) -> PathBuf {
    let secs = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("servers.json");
    path.with_file_name(format!("{name}.bak-{secs}"))
}
```

> `std::fs::rename` Windows'ta hedef dosya varsa üzerine yazar (`MOVEFILE_REPLACE_EXISTING`); atomik değiştirme bu sayede sağlanır.

- [ ] **Step 10: Tüm pgt-core testlerinin geçtiğini doğrula**

Run: `cargo test -p pgt-core`
Expected: PASS (error 6, config 4, registry 6). `src/ipc/bindings/` altında `ErrorPayload.ts`, `ServerConfig.ts`, `SslConfig.ts`, `SslMode.ts` oluşur.

- [ ] **Step 11: Commit**

```bash
git add Cargo.toml Cargo.lock .cargo .gitignore crates/pgt-core src/ipc/bindings
git commit -m "feat(core): workspace, hata modeli, sunucu yapılandırması ve kayıt defteri"
```

---

### Task 2: Kimlik bilgileri (Credential Manager + oturum belleği)

**Files:**
- Create: `crates/pgt-core/src/credentials.rs`
- Modify: `crates/pgt-core/src/lib.rs`

**Interfaces:**
- Consumes: `pgt_core::{Error, Result}` (Task 1)
- Produces:
  - `trait SecretStore: Send + Sync { fn get(&self, key: &str) -> Result<Option<String>>; fn set(&self, key: &str, secret: &str) -> Result<()>; fn delete(&self, key: &str) -> Result<()>; }`
  - `MemorySecretStore` (`Default`), `#[cfg(windows)] WindowsSecretStore::new() -> Result<WindowsSecretStore>`
  - `Credentials::new(Arc<dyn SecretStore>)`, `.password(Uuid) -> Result<Option<String>>`, `.remember(Uuid, &str, persist: bool) -> Result<()>`, `.forget(Uuid) -> Result<()>`
  - `password_key(Uuid) -> String` (`"server:<uuid>"`)

- [ ] **Step 1: Başarısız testleri yaz**

`crates/pgt-core/src/lib.rs`'e `pub mod credentials;` ekle. `crates/pgt-core/src/credentials.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    fn creds() -> (Arc<MemorySecretStore>, Credentials) {
        let store = Arc::new(MemorySecretStore::default());
        (store.clone(), Credentials::new(store))
    }

    #[test]
    fn persisted_password_survives_new_session() {
        let (store, c) = creds();
        let id = Uuid::new_v4();
        c.remember(id, "s3cret", true).unwrap();
        assert_eq!(store.get(&password_key(id)).unwrap().as_deref(), Some("s3cret"));
        let fresh = Credentials::new(store);
        assert_eq!(fresh.password(id).unwrap().as_deref(), Some("s3cret"));
    }

    #[test]
    fn session_only_password_is_not_persisted_and_clears_old_one() {
        let (store, c) = creds();
        let id = Uuid::new_v4();
        c.remember(id, "eski", true).unwrap();
        c.remember(id, "yeni", false).unwrap();
        assert_eq!(c.password(id).unwrap().as_deref(), Some("yeni"));
        assert_eq!(store.get(&password_key(id)).unwrap(), None);
    }

    #[test]
    fn forget_removes_session_and_store() {
        let (store, c) = creds();
        let id = Uuid::new_v4();
        c.remember(id, "x", true).unwrap();
        c.forget(id).unwrap();
        assert_eq!(c.password(id).unwrap(), None);
        assert_eq!(store.get(&password_key(id)).unwrap(), None);
        c.forget(id).unwrap(); // olmayan kaydı silmek hata değildir
    }

    #[cfg(windows)]
    #[test]
    #[ignore = "gerçek Windows Credential Manager'a yazar; elle çalıştırın: cargo test -p pgt-core -- --ignored"]
    fn windows_store_round_trip() {
        let store = WindowsSecretStore::new().unwrap();
        let key = format!("test:{}", Uuid::new_v4());
        assert_eq!(store.get(&key).unwrap(), None);
        store.set(&key, "değer-ğüş").unwrap();
        assert_eq!(store.get(&key).unwrap().as_deref(), Some("değer-ğüş"));
        store.delete(&key).unwrap();
        assert_eq!(store.get(&key).unwrap(), None);
        store.delete(&key).unwrap();
    }
}
```

- [ ] **Step 2: Testlerin başarısız olduğunu doğrula**

Run: `cargo test -p pgt-core credentials`
Expected: FAIL — `MemorySecretStore`, `Credentials` tanımlı değil.

- [ ] **Step 3: Uygula** (testlerin üstüne)

```rust
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use uuid::Uuid;

use crate::{Error, Result};

/// Gizli değer deposu. Üretimde Windows Credential Manager, testlerde bellek.
pub trait SecretStore: Send + Sync {
    fn get(&self, key: &str) -> Result<Option<String>>;
    fn set(&self, key: &str, secret: &str) -> Result<()>;
    /// Kayıt yoksa da `Ok(())` döner.
    fn delete(&self, key: &str) -> Result<()>;
}

#[derive(Default)]
pub struct MemorySecretStore(Mutex<HashMap<String, String>>);

impl SecretStore for MemorySecretStore {
    fn get(&self, key: &str) -> Result<Option<String>> {
        Ok(self.0.lock().unwrap().get(key).cloned())
    }
    fn set(&self, key: &str, secret: &str) -> Result<()> {
        self.0.lock().unwrap().insert(key.to_string(), secret.to_string());
        Ok(())
    }
    fn delete(&self, key: &str) -> Result<()> {
        self.0.lock().unwrap().remove(key);
        Ok(())
    }
}

#[cfg(windows)]
const SERVICE: &str = "pg-tools";

/// Windows Credential Manager. Aynı kayda farklı thread'lerden eşzamanlı erişim
/// sıralanmadığı için tüm işlemler tek kilitle serileştirilir.
#[cfg(windows)]
pub struct WindowsSecretStore {
    lock: Mutex<()>,
}

#[cfg(windows)]
fn cred_err(e: keyring_core::Error) -> Error {
    Error::Credential(e.to_string())
}

#[cfg(windows)]
impl WindowsSecretStore {
    pub fn new() -> Result<Self> {
        let store = windows_native_keyring_store::Store::new().map_err(cred_err)?;
        keyring_core::set_default_store(store);
        Ok(Self { lock: Mutex::new(()) })
    }

    fn entry(key: &str) -> Result<keyring_core::Entry> {
        keyring_core::Entry::new(SERVICE, key).map_err(cred_err)
    }
}

#[cfg(windows)]
impl SecretStore for WindowsSecretStore {
    fn get(&self, key: &str) -> Result<Option<String>> {
        let _guard = self.lock.lock().unwrap();
        match Self::entry(key)?.get_password() {
            Ok(p) => Ok(Some(p)),
            Err(keyring_core::Error::NoEntry) => Ok(None),
            Err(e) => Err(cred_err(e)),
        }
    }
    fn set(&self, key: &str, secret: &str) -> Result<()> {
        let _guard = self.lock.lock().unwrap();
        Self::entry(key)?.set_password(secret).map_err(cred_err)
    }
    fn delete(&self, key: &str) -> Result<()> {
        let _guard = self.lock.lock().unwrap();
        match Self::entry(key)?.delete_credential() {
            Ok(()) | Err(keyring_core::Error::NoEntry) => Ok(()),
            Err(e) => Err(cred_err(e)),
        }
    }
}

pub fn password_key(server: Uuid) -> String {
    format!("server:{server}")
}

/// Kalıcı depo + oturum belleği. Oturum belleği önce sorgulanır.
pub struct Credentials {
    store: Arc<dyn SecretStore>,
    session: Mutex<HashMap<Uuid, String>>,
}

impl Credentials {
    pub fn new(store: Arc<dyn SecretStore>) -> Self {
        Self { store, session: Mutex::new(HashMap::new()) }
    }

    pub fn password(&self, server: Uuid) -> Result<Option<String>> {
        if let Some(p) = self.session.lock().unwrap().get(&server) {
            return Ok(Some(p.clone()));
        }
        self.store.get(&password_key(server))
    }

    /// Parolayı oturumda tutar; `persist` ise depoya yazar, değilse depodaki eski kaydı siler.
    pub fn remember(&self, server: Uuid, password: &str, persist: bool) -> Result<()> {
        self.session.lock().unwrap().insert(server, password.to_string());
        if persist {
            self.store.set(&password_key(server), password)
        } else {
            self.store.delete(&password_key(server))
        }
    }

    pub fn forget(&self, server: Uuid) -> Result<()> {
        self.session.lock().unwrap().remove(&server);
        self.store.delete(&password_key(server))
    }
}
```

- [ ] **Step 4: Testlerin geçtiğini doğrula**

Run: `cargo test -p pgt-core credentials`
Expected: PASS (3 test, 1 ignored).

Run: `cargo test -p pgt-core credentials -- --ignored`
Expected: PASS (`windows_store_round_trip`). Sonrasında Windows "Kimlik Bilgisi Yöneticisi"nde `test:` kaydı kalmadığını gözle doğrulayın.

- [ ] **Step 5: Commit**

```bash
git add crates/pgt-core
git commit -m "feat(core): Credential Manager ve oturum parola deposu"
```

---

### Task 3: TLS yapılandırması (sslmode → rustls)

**Files:**
- Create: `crates/pgt-core/src/tls.rs`
- Modify: `crates/pgt-core/src/lib.rs`

**Interfaces:**
- Consumes: `config::{SslConfig, SslMode}`, `Error`
- Produces:
  - `tls::pg_ssl_mode(SslMode) -> tokio_postgres::config::SslMode`
  - `tls::client_config(&SslConfig) -> Result<rustls::ClientConfig>`
  - `tls::make_connector(&SslConfig) -> Result<tokio_postgres_rustls::MakeRustlsConnect>`

Davranış (libpq ile aynı):

| SslMode | tokio-postgres modu | Doğrulayıcı |
|---|---|---|
| disable | Disable | — (connector kullanılmaz) |
| prefer / require | Prefer / Require | `NoVerification` (yalnızca el sıkışma imzası doğrulanır) |
| verify-ca | Require | `IgnoreHostname(base)` — zincir doğrulanır, `NotValidForName*` hataları yok sayılır |
| verify-full | Require | `base` |

`base`: `root_cert` verilmişse yalnızca o dosyadaki CA'lar (`WebPkiServerVerifier`), verilmemişse Windows sertifika deposu (`rustls_platform_verifier::Verifier`).

- [ ] **Step 1: Başarısız testleri yaz**

`lib.rs`'e `pub mod tls;` ekle. `crates/pgt-core/src/tls.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use rustls::pki_types::{CertificateDer, ServerName, UnixTime};

    fn ssl(mode: SslMode) -> SslConfig {
        SslConfig { mode, ..SslConfig::default() }
    }

    fn write_ca(dir: &tempfile::TempDir) -> String {
        let key = rcgen::KeyPair::generate().unwrap();
        let mut params = rcgen::CertificateParams::new(vec!["test-ca".to_string()]).unwrap();
        params.is_ca = rcgen::IsCa::Ca(rcgen::BasicConstraints::Unconstrained);
        let cert = params.self_signed(&key).unwrap();
        let path = dir.path().join("ca.pem");
        std::fs::write(&path, cert.pem()).unwrap();
        path.to_string_lossy().into_owned()
    }

    #[test]
    fn maps_ssl_modes_to_tokio_postgres() {
        use tokio_postgres::config::SslMode as Pg;
        assert_eq!(pg_ssl_mode(SslMode::Disable), Pg::Disable);
        assert_eq!(pg_ssl_mode(SslMode::Prefer), Pg::Prefer);
        assert_eq!(pg_ssl_mode(SslMode::Require), Pg::Require);
        assert_eq!(pg_ssl_mode(SslMode::VerifyCa), Pg::Require);
        assert_eq!(pg_ssl_mode(SslMode::VerifyFull), Pg::Require);
    }

    #[test]
    fn builds_config_for_every_mode_without_files() {
        for mode in [SslMode::Disable, SslMode::Prefer, SslMode::Require, SslMode::VerifyCa, SslMode::VerifyFull] {
            assert!(client_config(&ssl(mode)).is_ok(), "{mode:?}");
        }
    }

    #[test]
    fn loads_root_cert_file() {
        let dir = tempfile::tempdir().unwrap();
        let mut c = ssl(SslMode::VerifyFull);
        c.root_cert = Some(write_ca(&dir));
        assert!(client_config(&c).is_ok());
    }

    #[test]
    fn missing_root_cert_file_is_config_error_with_path() {
        let mut c = ssl(SslMode::VerifyCa);
        c.root_cert = Some("C:\\yok\\ca.pem".into());
        match client_config(&c) {
            Err(Error::Config(m)) => assert!(m.contains("C:\\yok\\ca.pem"), "{m}"),
            Err(other) => panic!("beklenmeyen hata: {other:?}"),
            Ok(_) => panic!("hata bekleniyordu"),
        }
    }

    #[test]
    fn cert_without_key_is_rejected() {
        let mut c = ssl(SslMode::Require);
        c.client_cert = Some("c.pem".into());
        assert!(matches!(client_config(&c), Err(Error::Invalid(_))));
    }

    #[test]
    fn unreadable_client_key_mentions_encrypted_keys() {
        let dir = tempfile::tempdir().unwrap();
        let cert = write_ca(&dir);
        let key = dir.path().join("enc.key");
        std::fs::write(&key, "-----BEGIN ENCRYPTED PRIVATE KEY-----\nAAAA\n-----END ENCRYPTED PRIVATE KEY-----\n").unwrap();
        let mut c = ssl(SslMode::Require);
        c.client_cert = Some(cert);
        c.client_key = Some(key.to_string_lossy().into_owned());
        match client_config(&c) {
            Err(Error::Config(m)) => assert!(m.contains("pkcs8"), "{m}"),
            Err(other) => panic!("beklenmeyen hata: {other:?}"),
            Ok(_) => panic!("hata bekleniyordu"),
        }
    }

    #[derive(Debug)]
    struct Fixed(rustls::Error);
    impl ServerCertVerifier for Fixed {
        fn verify_server_cert(&self, _: &CertificateDer<'_>, _: &[CertificateDer<'_>], _: &ServerName<'_>, _: &[u8], _: UnixTime) -> std::result::Result<ServerCertVerified, rustls::Error> {
            Err(self.0.clone())
        }
        fn verify_tls12_signature(&self, _: &[u8], _: &CertificateDer<'_>, _: &DigitallySignedStruct) -> std::result::Result<HandshakeSignatureValid, rustls::Error> {
            Ok(HandshakeSignatureValid::assertion())
        }
        fn verify_tls13_signature(&self, _: &[u8], _: &CertificateDer<'_>, _: &DigitallySignedStruct) -> std::result::Result<HandshakeSignatureValid, rustls::Error> {
            Ok(HandshakeSignatureValid::assertion())
        }
        fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
            vec![]
        }
    }

    fn verify(v: &dyn ServerCertVerifier) -> std::result::Result<ServerCertVerified, rustls::Error> {
        let cert = CertificateDer::from(vec![0u8]);
        v.verify_server_cert(&cert, &[], &ServerName::try_from("db.local").unwrap(), &[], UnixTime::now())
    }

    #[test]
    fn ignore_hostname_accepts_name_mismatch_only() {
        let mismatch = IgnoreHostname(Arc::new(Fixed(rustls::Error::InvalidCertificate(CertificateError::NotValidForName))));
        assert!(verify(&mismatch).is_ok());
        let untrusted = IgnoreHostname(Arc::new(Fixed(rustls::Error::InvalidCertificate(CertificateError::UnknownIssuer))));
        assert!(verify(&untrusted).is_err());
    }
}
```

- [ ] **Step 2: Testlerin başarısız olduğunu doğrula**

Run: `cargo test -p pgt-core tls`
Expected: FAIL — `pg_ssl_mode`, `client_config`, `IgnoreHostname` tanımlı değil.

- [ ] **Step 3: Uygula** (testlerin üstüne)

```rust
use std::sync::Arc;

use rustls::client::WebPkiServerVerifier;
use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::crypto::{CryptoProvider, verify_tls12_signature, verify_tls13_signature};
use rustls::pki_types::pem::PemObject;
use rustls::pki_types::{CertificateDer, PrivateKeyDer, ServerName, UnixTime};
use rustls::{CertificateError, ClientConfig, DigitallySignedStruct, RootCertStore, SignatureScheme};
use tokio_postgres_rustls::MakeRustlsConnect;

use crate::config::{SslConfig, SslMode};
use crate::{Error, Result};

pub fn pg_ssl_mode(mode: SslMode) -> tokio_postgres::config::SslMode {
    use tokio_postgres::config::SslMode as Pg;
    match mode {
        SslMode::Disable => Pg::Disable,
        SslMode::Prefer => Pg::Prefer,
        SslMode::Require | SslMode::VerifyCa | SslMode::VerifyFull => Pg::Require,
    }
}

fn provider() -> Arc<CryptoProvider> {
    Arc::new(rustls::crypto::ring::default_provider())
}

fn config_err(e: impl std::fmt::Display) -> Error {
    Error::Config(format!("TLS yapılandırılamadı: {e}"))
}

pub fn client_config(ssl: &SslConfig) -> Result<ClientConfig> {
    if ssl.client_cert.is_some() != ssl.client_key.is_some() {
        return Err(Error::Invalid("İstemci sertifikası ve anahtarı birlikte verilmelidir.".into()));
    }
    let provider = provider();
    let verifier: Arc<dyn ServerCertVerifier> = match ssl.mode {
        SslMode::Disable | SslMode::Prefer | SslMode::Require => Arc::new(NoVerification(provider.clone())),
        SslMode::VerifyCa => Arc::new(IgnoreHostname(base_verifier(ssl, provider.clone())?)),
        SslMode::VerifyFull => base_verifier(ssl, provider.clone())?,
    };
    let builder = ClientConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .map_err(config_err)?
        .dangerous()
        .with_custom_certificate_verifier(verifier);
    match (&ssl.client_cert, &ssl.client_key) {
        (Some(cert), Some(key)) => builder.with_client_auth_cert(load_certs(cert)?, load_key(key)?).map_err(config_err),
        _ => Ok(builder.with_no_client_auth()),
    }
}

pub fn make_connector(ssl: &SslConfig) -> Result<MakeRustlsConnect> {
    Ok(MakeRustlsConnect::new(client_config(ssl)?))
}

fn base_verifier(ssl: &SslConfig, provider: Arc<CryptoProvider>) -> Result<Arc<dyn ServerCertVerifier>> {
    match &ssl.root_cert {
        Some(path) => {
            let mut roots = RootCertStore::empty();
            for cert in load_certs(path)? {
                roots.add(cert).map_err(config_err)?;
            }
            let verifier = WebPkiServerVerifier::builder_with_provider(Arc::new(roots), provider)
                .build()
                .map_err(config_err)?;
            Ok(verifier)
        }
        None => Ok(Arc::new(rustls_platform_verifier::Verifier::new(provider).map_err(config_err)?)),
    }
}

fn load_certs(path: &str) -> Result<Vec<CertificateDer<'static>>> {
    let read_err = |e: &dyn std::fmt::Display| Error::Config(format!("Sertifika dosyası okunamadı ({path}): {e}"));
    let certs = CertificateDer::pem_file_iter(path)
        .map_err(|e| read_err(&e))?
        .collect::<std::result::Result<Vec<_>, _>>()
        .map_err(|e| read_err(&e))?;
    if certs.is_empty() {
        return Err(read_err(&"dosyada sertifika yok"));
    }
    Ok(certs)
}

fn load_key(path: &str) -> Result<PrivateKeyDer<'static>> {
    PrivateKeyDer::from_pem_file(path).map_err(|e| {
        Error::Config(format!(
            "İstemci anahtarı okunamadı ({path}): {e}. Şifreli anahtarlar desteklenmez; \
             `openssl pkcs8 -topk8 -nocrypt -in eski.key -out yeni.key` ile dönüştürün."
        ))
    })
}

/// prefer/require: sertifika zinciri doğrulanmaz (libpq davranışı), el sıkışma imzası doğrulanır.
#[derive(Debug)]
struct NoVerification(Arc<CryptoProvider>);

impl ServerCertVerifier for NoVerification {
    fn verify_server_cert(&self, _: &CertificateDer<'_>, _: &[CertificateDer<'_>], _: &ServerName<'_>, _: &[u8], _: UnixTime) -> std::result::Result<ServerCertVerified, rustls::Error> {
        Ok(ServerCertVerified::assertion())
    }
    fn verify_tls12_signature(&self, message: &[u8], cert: &CertificateDer<'_>, dss: &DigitallySignedStruct) -> std::result::Result<HandshakeSignatureValid, rustls::Error> {
        verify_tls12_signature(message, cert, dss, &self.0.signature_verification_algorithms)
    }
    fn verify_tls13_signature(&self, message: &[u8], cert: &CertificateDer<'_>, dss: &DigitallySignedStruct) -> std::result::Result<HandshakeSignatureValid, rustls::Error> {
        verify_tls13_signature(message, cert, dss, &self.0.signature_verification_algorithms)
    }
    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.0.signature_verification_algorithms.supported_schemes()
    }
}

/// verify-ca: zinciri iç doğrulayıcıya bırakır, yalnızca host adı uyuşmazlığını yok sayar.
#[derive(Debug)]
struct IgnoreHostname(Arc<dyn ServerCertVerifier>);

impl ServerCertVerifier for IgnoreHostname {
    fn verify_server_cert(&self, end_entity: &CertificateDer<'_>, intermediates: &[CertificateDer<'_>], server_name: &ServerName<'_>, ocsp: &[u8], now: UnixTime) -> std::result::Result<ServerCertVerified, rustls::Error> {
        match self.0.verify_server_cert(end_entity, intermediates, server_name, ocsp, now) {
            Err(rustls::Error::InvalidCertificate(CertificateError::NotValidForName | CertificateError::NotValidForNameContext { .. })) => {
                Ok(ServerCertVerified::assertion())
            }
            other => other,
        }
    }
    fn verify_tls12_signature(&self, message: &[u8], cert: &CertificateDer<'_>, dss: &DigitallySignedStruct) -> std::result::Result<HandshakeSignatureValid, rustls::Error> {
        self.0.verify_tls12_signature(message, cert, dss)
    }
    fn verify_tls13_signature(&self, message: &[u8], cert: &CertificateDer<'_>, dss: &DigitallySignedStruct) -> std::result::Result<HandshakeSignatureValid, rustls::Error> {
        self.0.verify_tls13_signature(message, cert, dss)
    }
    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.0.supported_verify_schemes()
    }
}
```

> Not: `WebPkiServerVerifier` ve Windows doğrulayıcısı zincir hatasını host adı hatasından önce raporlar; bu yüzden `IgnoreHostname` güvenilmeyen zinciri yanlışlıkla kabul etmez. Derleyici `ServerCertVerifier`'da yeni zorunlu metot isterse (rustls 0.23.x küçük sürüm farkı), iç doğrulayıcıya yönlendiren bir uygulama ekleyin.

- [ ] **Step 4: Testlerin geçtiğini doğrula**

Run: `cargo test -p pgt-core tls`
Expected: PASS (7 test).

- [ ] **Step 5: Commit**

```bash
git add crates/pgt-core
git commit -m "feat(core): sslmode eşlemesi ve rustls doğrulayıcıları"
```

---

### Task 4: Bağlantı yöneticisi (oturum ayarları, sürüm, yeniden bağlanma, boşta kapanma)

**Files:**
- Create: `crates/pgt-core/src/connection.rs`, `crates/pgt-core/src/testing.rs`, `crates/pgt-core/tests/connection.rs`
- Modify: `crates/pgt-core/src/lib.rs`, `crates/pgt-core/Cargo.toml`

**Interfaces:**
- Consumes: `ServerConfig`, `tls::{make_connector, pg_ssl_mode}`, `Error::{from_connect, from_query}`
- Produces:
  - `connection::ServerInfo { version_num: u32, version: String }` (TS export, camelCase)
  - `connection::PgConn { pub client: tokio_postgres::Client, pub info: ServerInfo }`, `.version_num() -> u32`
  - `connection::Target::{Maintenance, Database(u32)}` (`Copy + Eq + Hash`)
  - `ConnectionManager::new()`, `ConnectionManager::test(&ServerConfig, Option<&str>) -> Result<ServerInfo>` (ilişkisiz fonksiyon), `.connect(ServerConfig, Option<String>) -> Result<ServerInfo>`, `.disconnect(Uuid)`, `.is_connected(Uuid) -> bool`, `.info(Uuid) -> Option<ServerInfo>` (async), `.run(server, target, f) -> Result<T>` where `f: Fn(Arc<PgConn>) -> Fut, Fut: Future<Output = Result<T, tokio_postgres::Error>>`, `.reap_idle(Duration)` (async)
  - `CONNECT_TIMEOUT: Duration` (10 sn)
  - (feature `testing`) `testing::{TestServer { config, password }, test_servers() -> Vec<TestServer>, test_server() -> Option<TestServer>, admin_client(&TestServer, dbname: &str) -> tokio_postgres::Client}` (async)

**Davranış:**
- Her bağlantı `application_name=pg-tools`, `options="-c default_transaction_read_only=on -c statement_timeout=30000"`, `connect_timeout=10s`, `keepalives=true` ile açılır.
- Bağlantılar (sunucu, Target) başına tek `Arc<PgConn>`; sorgular kilit dışında çalışır.
- `run`: sorgu bağlantı kopması (`is_closed()` ya da SQLSTATE `57P01/57P02/57P03`) ile başarısız olursa önbellekteki bağlantı atılır, bir kez yeniden bağlanılıp tekrar denenir. Yeniden bağlanma da başarısızsa oturum kaldırılır (sunucu "bağlı değil") ve hata döner.
- `Target::Database(oid)` için veritabanı adı bakım bağlantısından `pg_database` sorgusuyla çözülüp önbelleğe alınır; oid yoksa `Error::NotFound`.

- [ ] **Step 1: `testing` modülünü yaz**

`crates/pgt-core/Cargo.toml` `[dev-dependencies]` altına `tokio = { workspace = true }` ekleyin (zaten normal bağımlılık; makrolar için yeterli). `lib.rs`:

```rust
pub mod config;
pub mod connection;
pub mod credentials;
pub mod error;
pub mod registry;
#[cfg(any(test, feature = "testing"))]
pub mod testing;
pub mod tls;

pub use error::{ConnectErrorKind, Error, ErrorPayload, Result};
```

`crates/pgt-core/src/testing.rs`:

```rust
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
```

- [ ] **Step 2: Başarısız entegrasyon testlerini yaz**

`crates/pgt-core/tests/connection.rs`:

```rust
use std::time::Duration;

use pgt_core::config::ServerConfig;
use pgt_core::connection::{ConnectionManager, Target};
use pgt_core::testing::{admin_client, test_server};
use pgt_core::{ConnectErrorKind, Error};

macro_rules! require_server {
    () => {
        match test_server() {
            Some(s) => s,
            None => {
                eprintln!("PGT_TEST_URLS tanımlı değil, test atlanıyor");
                return;
            }
        }
    };
}

#[tokio::test]
async fn unreachable_host_is_network_error() {
    let config = ServerConfig::new("kapalı", "127.0.0.1", 1, "postgres");
    let err = ConnectionManager::test(&config, None).await.unwrap_err();
    assert!(matches!(err, Error::Connect { kind: ConnectErrorKind::Network, .. }), "{err:?}");
}

#[tokio::test]
async fn connects_and_reports_version() {
    let ts = require_server!();
    let mgr = ConnectionManager::new();
    let info = mgr.connect(ts.config.clone(), ts.password.clone()).await.unwrap();
    assert!(info.version_num >= 100000, "{info:?}");
    assert!(!info.version.contains(' '), "kısa sürüm bekleniyordu: {}", info.version);
    assert!(mgr.is_connected(ts.config.id));
    assert_eq!(mgr.info(ts.config.id).await.unwrap().version_num, info.version_num);
}

#[tokio::test]
async fn session_settings_are_applied() {
    let ts = require_server!();
    let mgr = ConnectionManager::new();
    mgr.connect(ts.config.clone(), ts.password.clone()).await.unwrap();
    let (read_only, app, timeout): (String, String, String) = mgr
        .run(ts.config.id, Target::Maintenance, |c| async move {
            let row = c
                .client
                .query_one(
                    "SELECT current_setting('default_transaction_read_only'), current_setting('application_name'), current_setting('statement_timeout')",
                    &[],
                )
                .await?;
            Ok((row.get(0), row.get(1), row.get(2)))
        })
        .await
        .unwrap();
    assert_eq!((read_only.as_str(), app.as_str(), timeout.as_str()), ("on", "pg-tools", "30s"));
}

#[tokio::test]
async fn writes_are_rejected() {
    let ts = require_server!();
    let mgr = ConnectionManager::new();
    mgr.connect(ts.config.clone(), ts.password.clone()).await.unwrap();
    let err = mgr
        .run(ts.config.id, Target::Maintenance, |c| async move { c.client.batch_execute("CREATE TEMP TABLE pgt_x(a int)").await })
        .await
        .unwrap_err();
    assert!(matches!(err, Error::Query { ref sqlstate, .. } if sqlstate.as_deref() == Some("25006")), "{err:?}");
}

#[tokio::test]
async fn wrong_password_is_auth_error() {
    let ts = require_server!();
    let err = ConnectionManager::test(&ts.config, Some("kesinlikle-yanlis-parola")).await.unwrap_err();
    assert!(matches!(err, Error::Connect { kind: ConnectErrorKind::Auth, .. }), "{err:?}");
}

#[tokio::test]
async fn missing_password_is_password_required() {
    let ts = require_server!();
    let err = ConnectionManager::test(&ts.config, None).await.unwrap_err();
    assert!(matches!(err, Error::PasswordRequired), "{err:?}");
}

#[tokio::test]
async fn run_without_connect_is_not_connected() {
    let mgr = ConnectionManager::new();
    let err = mgr
        .run(uuid::Uuid::new_v4(), Target::Maintenance, |c| async move { c.client.simple_query("SELECT 1").await })
        .await
        .unwrap_err();
    assert!(matches!(err, Error::NotConnected));
}

#[tokio::test]
async fn database_target_resolves_by_oid() {
    let ts = require_server!();
    let mgr = ConnectionManager::new();
    mgr.connect(ts.config.clone(), ts.password.clone()).await.unwrap();
    let oid: u32 = mgr
        .run(ts.config.id, Target::Maintenance, |c| async move {
            c.client.query_one("SELECT oid FROM pg_database WHERE datname = current_database()", &[]).await.map(|r| r.get(0))
        })
        .await
        .unwrap();
    let name: String = mgr
        .run(ts.config.id, Target::Database(oid), |c| async move {
            c.client.query_one("SELECT current_database()::text", &[]).await.map(|r| r.get(0))
        })
        .await
        .unwrap();
    assert_eq!(name, ts.config.maintenance_db);
    let missing = mgr
        .run(ts.config.id, Target::Database(4_000_000_000), |c| async move { c.client.simple_query("SELECT 1").await })
        .await
        .unwrap_err();
    assert!(matches!(missing, Error::NotFound(_)), "{missing:?}");
}

#[tokio::test]
async fn reconnects_after_backend_terminated() {
    let ts = require_server!();
    let mgr = ConnectionManager::new();
    mgr.connect(ts.config.clone(), ts.password.clone()).await.unwrap();
    let pid_query = |c: std::sync::Arc<pgt_core::connection::PgConn>| async move {
        c.client.query_one("SELECT pg_backend_pid()", &[]).await.map(|r| r.get::<_, i32>(0))
    };
    let pid = mgr.run(ts.config.id, Target::Maintenance, pid_query).await.unwrap();
    let admin = admin_client(&ts, &ts.config.maintenance_db).await;
    admin.execute("SELECT pg_terminate_backend($1)", &[&pid]).await.unwrap();
    tokio::time::sleep(Duration::from_millis(200)).await;
    let new_pid = mgr.run(ts.config.id, Target::Maintenance, pid_query).await.unwrap();
    assert_ne!(pid, new_pid);
    assert!(mgr.is_connected(ts.config.id));
}

#[tokio::test]
async fn reap_idle_closes_and_run_reopens() {
    let ts = require_server!();
    let mgr = ConnectionManager::new();
    mgr.connect(ts.config.clone(), ts.password.clone()).await.unwrap();
    mgr.reap_idle(Duration::ZERO).await;
    assert!(mgr.is_connected(ts.config.id), "oturum bağlı kalmalı");
    let one: i32 = mgr
        .run(ts.config.id, Target::Maintenance, |c| async move { c.client.query_one("SELECT 1", &[]).await.map(|r| r.get(0)) })
        .await
        .unwrap();
    assert_eq!(one, 1);
}

#[tokio::test]
async fn disconnect_removes_session() {
    let ts = require_server!();
    let mgr = ConnectionManager::new();
    mgr.connect(ts.config.clone(), ts.password.clone()).await.unwrap();
    mgr.disconnect(ts.config.id);
    assert!(!mgr.is_connected(ts.config.id));
}
```

- [ ] **Step 3: Testlerin başarısız olduğunu doğrula**

Run: `cargo test -p pgt-core --features testing --test connection`
Expected: FAIL — `connection` modülü yok (derleme hatası).

- [ ] **Step 4: `connection.rs` uygulamasını yaz**

```rust
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
```

- [ ] **Step 5: Birim/entegrasyon testlerini çalıştır**

Run (sunucusuz): `cargo test -p pgt-core --features testing --test connection`
Expected: PASS; `unreachable_host_is_network_error` gerçekten çalışır, diğerleri "atlanıyor" yazar.

Run (yerel PG 18 ile, PowerShell):

```powershell
$env:PGT_TEST_URLS = "postgres://postgres:PAROLA@localhost:5432/postgres"
cargo test -p pgt-core --features testing --test connection
```

Expected: PASS (11 test). `missing_password_is_password_required` ve `wrong_password_is_auth_error` için sunucu `pg_hba.conf`'ta `scram-sha-256` kullanmalıdır (Windows kurulumunun varsayılanı).

- [ ] **Step 6: Commit**

```bash
git add crates/pgt-core src/ipc/bindings
git commit -m "feat(core): bağlantı yöneticisi, salt okunur oturum, yeniden bağlanma ve boşta kapanma"
```

---

### Task 5: Katalog düğüm modeli, sürüm seçimi ve özellik tipleri

**Files:**
- Create: `crates/pgt-catalog/Cargo.toml`, `crates/pgt-catalog/src/lib.rs`, `crates/pgt-catalog/src/node.rs`, `crates/pgt-catalog/src/version.rs`, `crates/pgt-catalog/src/props.rs`
- Modify: `Cargo.toml` (workspace `members`'a `crates/pgt-catalog` ekle)

**Interfaces:**
- Consumes: `pgt_core::{Error, Result}`
- Produces:
  - `node::Coll::{Databases, Schemas, Tables, Columns}`
  - `node::Seg::{Coll(Coll), Db(u32), Schema(u32), Table(u32), Column(i16)}`
  - `node::NodeId::{Root, Group(String), Node { server: Uuid, path: Vec<Seg> }}`; `NodeId::parse(&str) -> Result<NodeId>`, `NodeId::server(Uuid) -> NodeId`, `.child(Seg) -> NodeId`, `Display` (string kimlik)
  - `node::NodeKind` (camelCase TS union): `serverGroup, server, databaseCollection, database, schemaCollection, schema, tableCollection, table, columnCollection, column`
  - `node::NodeFlags { connected, partitioned, system, primary_key, not_null: bool }` (`Default`)
  - `node::TreeNode { id: String, kind: NodeKind, label: String, detail: Option<String>, has_children: bool, server_id: Option<Uuid>, flags: NodeFlags }`; `TreeNode::new(&NodeId, NodeKind, impl Into<String>) -> TreeNode`, `.with_detail(Option<String>)`, `.leaf()`, `.with_flags(NodeFlags)`, `.with_server(Uuid)`
  - `version::PgVersion(pub u32)`; `.major() -> u32`; `version::VersionedSql(pub &'static [(u32, &'static str)])`; `.pick(PgVersion) -> &'static str`
  - `props::PropertySection { title: String, rows: Vec<PropertyRow> }`, `props::PropertyRow { label: String, value: String }`; `PropertySection::new(title).row(label, value).row_opt(label, Option<impl Into<String>>)`; `props::yes_no(bool) -> &'static str`; `props::approx_rows(f64) -> Option<String>`

Kimlik biçimi: `""` (kök), `grp:<ad>` (grup adı `/` içerebilir; ayrıştırılmaz), `srv:<uuid>[/seg]*`; seg: `dbs | schemas | tables | columns | db:<oid> | sch:<oid> | tbl:<oid> | col:<attnum>`.

- [ ] **Step 1: Crate'i oluştur**

Kök `Cargo.toml`'da `members = ["crates/pgt-core", "crates/pgt-catalog"]`.

`crates/pgt-catalog/Cargo.toml`:

```toml
[package]
name = "pgt-catalog"
version.workspace = true
edition.workspace = true
rust-version.workspace = true

[dependencies]
pgt-core.workspace = true
tokio.workspace = true
tokio-postgres.workspace = true
serde.workspace = true
uuid.workspace = true
tracing.workspace = true
ts-rs.workspace = true

[dev-dependencies]
pgt-core = { workspace = true, features = ["testing"] }
tempfile.workspace = true
insta.workspace = true
```

`crates/pgt-catalog/src/lib.rs`:

```rust
pub mod node;
pub mod props;
pub mod version;
```

- [ ] **Step 2: Başarısız testleri yaz**

`crates/pgt-catalog/src/node.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    const U: &str = "3f2a6d1e-8a4b-4c2e-9f00-0123456789ab";

    #[test]
    fn round_trips_all_shapes() {
        let cases = [
            "".to_string(),
            "grp:Üretim/Ankara".to_string(),
            format!("srv:{U}"),
            format!("srv:{U}/dbs"),
            format!("srv:{U}/db:16384"),
            format!("srv:{U}/db:16384/schemas"),
            format!("srv:{U}/db:16384/sch:2200"),
            format!("srv:{U}/db:16384/sch:2200/tables"),
            format!("srv:{U}/db:16384/sch:2200/tbl:24576"),
            format!("srv:{U}/db:16384/sch:2200/tbl:24576/columns"),
            format!("srv:{U}/db:16384/sch:2200/tbl:24576/col:3"),
        ];
        for s in cases {
            assert_eq!(NodeId::parse(&s).unwrap().to_string(), s);
        }
    }

    #[test]
    fn parses_into_typed_segments() {
        let id = NodeId::parse(&format!("srv:{U}/db:16384/sch:2200/tbl:24576/col:3")).unwrap();
        let NodeId::Node { server, path } = id else { panic!() };
        assert_eq!(server.to_string(), U);
        assert_eq!(path, vec![Seg::Db(16384), Seg::Schema(2200), Seg::Table(24576), Seg::Column(3)]);
        assert_eq!(NodeId::parse("grp:a/b").unwrap(), NodeId::Group("a/b".into()));
        assert_eq!(NodeId::parse("").unwrap(), NodeId::Root);
    }

    #[test]
    fn child_builds_paths() {
        let server = Uuid::parse_str(U).unwrap();
        let id = NodeId::server(server).child(Seg::Db(5)).child(Seg::Coll(Coll::Schemas));
        assert_eq!(id.to_string(), format!("srv:{U}/db:5/schemas"));
    }

    #[test]
    fn rejects_garbage() {
        for bad in ["x", "grp:", "srv:not-a-uuid", &format!("srv:{U}/db:abc"), &format!("srv:{U}/wat"), &format!("srv:{U}/col:99999")] {
            assert!(matches!(NodeId::parse(bad), Err(Error::Invalid(_))), "{bad}");
        }
    }

    #[test]
    fn tree_node_serializes_camel_case() {
        let server = Uuid::parse_str(U).unwrap();
        let n = TreeNode::new(&NodeId::server(server), NodeKind::Server, "prod").with_server(server);
        let v = serde_json::to_value(&n).unwrap();
        assert_eq!(v["kind"], "server");
        assert_eq!(v["hasChildren"], true);
        assert_eq!(v["serverId"], U);
        assert_eq!(v["flags"]["primaryKey"], false);
        assert_eq!(serde_json::to_value(NodeKind::TableCollection).unwrap(), "tableCollection");
    }
}
```

`crates/pgt-catalog/Cargo.toml` `[dev-dependencies]`'e `serde_json = "1"` ekleyin.

`crates/pgt-catalog/src/version.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    static Q: VersionedSql = VersionedSql(&[(10, "pg10"), (12, "pg12"), (18, "pg18")]);

    #[test]
    fn major_from_version_num() {
        assert_eq!(PgVersion(100023).major(), 10);
        assert_eq!(PgVersion(160004).major(), 16);
        assert_eq!(PgVersion(180000).major(), 18);
    }

    #[test]
    fn picks_highest_matching_variant() {
        assert_eq!(Q.pick(PgVersion(100000)), "pg10");
        assert_eq!(Q.pick(PgVersion(110005)), "pg10");
        assert_eq!(Q.pick(PgVersion(120000)), "pg12");
        assert_eq!(Q.pick(PgVersion(170002)), "pg12");
        assert_eq!(Q.pick(PgVersion(180001)), "pg18");
        assert_eq!(Q.pick(PgVersion(190000)), "pg18");
        assert_eq!(Q.pick(PgVersion(90600)), "pg10", "eski sürüm en eski varyantı kullanır");
    }
}
```

`crates/pgt-catalog/src/props.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builder_skips_none_and_empty() {
        let s = PropertySection::new("Genel")
            .row("Ad", "orders")
            .row_opt("Yorum", None::<String>)
            .row_opt("Boş", Some(""))
            .row_opt("Sahip", Some("app"));
        assert_eq!(s.title, "Genel");
        let labels: Vec<_> = s.rows.iter().map(|r| r.label.as_str()).collect();
        assert_eq!(labels, ["Ad", "Sahip"]);
    }

    #[test]
    fn approx_rows_formats_turkish() {
        assert_eq!(approx_rows(-1.0), None);
        assert_eq!(approx_rows(0.0).as_deref(), Some("~0 satır"));
        assert_eq!(approx_rows(950.0).as_deref(), Some("~950 satır"));
        assert_eq!(approx_rows(12_345.0).as_deref(), Some("~12,3 B satır"));
        assert_eq!(approx_rows(1_200_000.0).as_deref(), Some("~1,2 M satır"));
    }

    #[test]
    fn yes_no_is_turkish() {
        assert_eq!((yes_no(true), yes_no(false)), ("Evet", "Hayır"));
    }
}
```

`lib.rs`'e `pub use pgt_core::{Error, Result};` ekleyin.

- [ ] **Step 3: Testlerin başarısız olduğunu doğrula**

Run: `cargo test -p pgt-catalog`
Expected: FAIL — `NodeId`, `VersionedSql`, `PropertySection` tanımlı değil.

- [ ] **Step 4: `node.rs` uygulamasını yaz** (testlerin üstüne)

```rust
use std::fmt;

use serde::Serialize;
use ts_rs::TS;
use uuid::Uuid;

use crate::{Error, Result};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Coll {
    Databases,
    Schemas,
    Tables,
    Columns,
}

impl Coll {
    fn tag(self) -> &'static str {
        match self {
            Coll::Databases => "dbs",
            Coll::Schemas => "schemas",
            Coll::Tables => "tables",
            Coll::Columns => "columns",
        }
    }
    fn from_tag(tag: &str) -> Option<Self> {
        Some(match tag {
            "dbs" => Coll::Databases,
            "schemas" => Coll::Schemas,
            "tables" => Coll::Tables,
            "columns" => Coll::Columns,
            _ => return None,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Seg {
    Coll(Coll),
    Db(u32),
    Schema(u32),
    Table(u32),
    Column(i16),
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum NodeId {
    Root,
    Group(String),
    Node { server: Uuid, path: Vec<Seg> },
}

fn invalid(s: &str) -> Error {
    Error::Invalid(format!("geçersiz düğüm kimliği: {s}"))
}

impl NodeId {
    pub fn server(server: Uuid) -> Self {
        NodeId::Node { server, path: Vec::new() }
    }

    pub fn child(&self, seg: Seg) -> Self {
        match self {
            NodeId::Node { server, path } => {
                let mut path = path.clone();
                path.push(seg);
                NodeId::Node { server: *server, path }
            }
            _ => panic!("yalnızca sunucu altındaki düğümlerin çocuğu olabilir"),
        }
    }

    pub fn parse(s: &str) -> Result<Self> {
        if s.is_empty() {
            return Ok(NodeId::Root);
        }
        if let Some(name) = s.strip_prefix("grp:") {
            return if name.is_empty() { Err(invalid(s)) } else { Ok(NodeId::Group(name.to_string())) };
        }
        let mut parts = s.split('/');
        let server = parts
            .next()
            .and_then(|p| p.strip_prefix("srv:"))
            .and_then(|u| Uuid::parse_str(u).ok())
            .ok_or_else(|| invalid(s))?;
        let mut path = Vec::new();
        for part in parts {
            let seg = match part.split_once(':') {
                Some(("db", n)) => Seg::Db(n.parse().map_err(|_| invalid(s))?),
                Some(("sch", n)) => Seg::Schema(n.parse().map_err(|_| invalid(s))?),
                Some(("tbl", n)) => Seg::Table(n.parse().map_err(|_| invalid(s))?),
                Some(("col", n)) => Seg::Column(n.parse().map_err(|_| invalid(s))?),
                Some(_) => return Err(invalid(s)),
                None => Seg::Coll(Coll::from_tag(part).ok_or_else(|| invalid(s))?),
            };
            path.push(seg);
        }
        Ok(NodeId::Node { server, path })
    }
}

impl fmt::Display for NodeId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            NodeId::Root => Ok(()),
            NodeId::Group(name) => write!(f, "grp:{name}"),
            NodeId::Node { server, path } => {
                write!(f, "srv:{server}")?;
                for seg in path {
                    match seg {
                        Seg::Coll(c) => write!(f, "/{}", c.tag())?,
                        Seg::Db(o) => write!(f, "/db:{o}")?,
                        Seg::Schema(o) => write!(f, "/sch:{o}")?,
                        Seg::Table(o) => write!(f, "/tbl:{o}")?,
                        Seg::Column(n) => write!(f, "/col:{n}")?,
                    }
                }
                Ok(())
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum NodeKind {
    ServerGroup,
    Server,
    DatabaseCollection,
    Database,
    SchemaCollection,
    Schema,
    TableCollection,
    Table,
    ColumnCollection,
    Column,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct NodeFlags {
    pub connected: bool,
    pub partitioned: bool,
    pub system: bool,
    pub primary_key: bool,
    pub not_null: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct TreeNode {
    pub id: String,
    pub kind: NodeKind,
    pub label: String,
    pub detail: Option<String>,
    pub has_children: bool,
    pub server_id: Option<Uuid>,
    pub flags: NodeFlags,
}

impl TreeNode {
    pub fn new(id: &NodeId, kind: NodeKind, label: impl Into<String>) -> Self {
        Self {
            id: id.to_string(),
            kind,
            label: label.into(),
            detail: None,
            has_children: true,
            server_id: None,
            flags: NodeFlags::default(),
        }
    }
    pub fn with_detail(mut self, detail: Option<String>) -> Self {
        self.detail = detail;
        self
    }
    pub fn leaf(mut self) -> Self {
        self.has_children = false;
        self
    }
    pub fn with_flags(mut self, flags: NodeFlags) -> Self {
        self.flags = flags;
        self
    }
    pub fn with_server(mut self, server: Uuid) -> Self {
        self.server_id = Some(server);
        self
    }
}
```

- [ ] **Step 5: `version.rs` ve `props.rs` uygulamalarını yaz** (testlerin üstüne)

`version.rs`:

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct PgVersion(pub u32);

impl PgVersion {
    pub fn major(self) -> u32 {
        self.0 / 10_000
    }
}

/// Sürüme göre SQL varyantları: `(en düşük ana sürüm, sql)`, ana sürüme göre artan sırada.
pub struct VersionedSql(pub &'static [(u32, &'static str)]);

impl VersionedSql {
    pub fn pick(&self, version: PgVersion) -> &'static str {
        self.0
            .iter()
            .rev()
            .find(|(min, _)| version.major() >= *min)
            .or_else(|| self.0.first())
            .map(|(_, sql)| *sql)
            .expect("VersionedSql en az bir varyant içermeli")
    }
}
```

`props.rs`:

```rust
use serde::Serialize;
use ts_rs::TS;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct PropertyRow {
    pub label: String,
    pub value: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct PropertySection {
    pub title: String,
    pub rows: Vec<PropertyRow>,
}

impl PropertySection {
    pub fn new(title: impl Into<String>) -> Self {
        Self { title: title.into(), rows: Vec::new() }
    }
    pub fn row(mut self, label: impl Into<String>, value: impl Into<String>) -> Self {
        self.rows.push(PropertyRow { label: label.into(), value: value.into() });
        self
    }
    pub fn row_opt(self, label: impl Into<String>, value: Option<impl Into<String>>) -> Self {
        match value.map(Into::into).filter(|v: &String| !v.is_empty()) {
            Some(v) => self.row(label, v),
            None => self,
        }
    }
}

pub fn yes_no(value: bool) -> &'static str {
    if value { "Evet" } else { "Hayır" }
}

/// `pg_class.reltuples` tahminini okunur hale getirir; -1 (hiç analiz edilmemiş) için `None`.
pub fn approx_rows(reltuples: f64) -> Option<String> {
    if reltuples < 0.0 {
        return None;
    }
    let text = if reltuples < 1_000.0 {
        format!("~{} satır", reltuples.round() as u64)
    } else if reltuples < 1_000_000.0 {
        format!("~{:.1} B satır", reltuples / 1_000.0)
    } else {
        format!("~{:.1} M satır", reltuples / 1_000_000.0)
    };
    Some(text.replace('.', ","))
}
```

- [ ] **Step 6: Testlerin geçtiğini doğrula**

Run: `cargo test -p pgt-catalog`
Expected: PASS (node 5, version 2, props 3). `src/ipc/bindings/` altında `NodeKind.ts`, `NodeFlags.ts`, `TreeNode.ts`, `PropertySection.ts`, `PropertyRow.ts` oluşur.

- [ ] **Step 7: Commit**

```bash
git add Cargo.toml Cargo.lock crates/pgt-catalog src/ipc/bindings
git commit -m "feat(catalog): düğüm kimlikleri, sürüm seçimi ve özellik tipleri"
```

---

### Task 6: Catalog dağıtıcısı ve ağaç listeleme (kök → kolonlar)

**Files:**
- Create: `crates/pgt-catalog/src/db.rs`, `crates/pgt-catalog/src/kinds/mod.rs`, `crates/pgt-catalog/src/kinds/server.rs`, `crates/pgt-catalog/src/kinds/database.rs`, `crates/pgt-catalog/src/kinds/schema.rs`, `crates/pgt-catalog/src/kinds/table.rs`, `crates/pgt-catalog/src/kinds/column.rs`, `crates/pgt-catalog/src/sql/databases.sql`, `crates/pgt-catalog/src/sql/schemas.sql`, `crates/pgt-catalog/src/sql/tables.sql`, `crates/pgt-catalog/src/sql/columns.sql`, `crates/pgt-catalog/tests/common/mod.rs`, `crates/pgt-catalog/tests/fixtures/schema.sql`, `crates/pgt-catalog/tests/listing.rs`
- Modify: `crates/pgt-catalog/src/lib.rs`

**Interfaces:**
- Consumes: `pgt_core::{connection::{ConnectionManager, Target, PgConn}, registry::Registry, config::ServerConfig}`, Task 5 tipleri
- Produces:
  - `pgt_catalog::Catalog::new(Arc<ConnectionManager>, Arc<RwLock<Registry>>) -> Catalog` (`std::sync::RwLock`)
  - `Catalog::set_show_system(&self, bool)`
  - `Catalog::list_children(&self, &NodeId) -> Result<Vec<TreeNode>>` (async)
  - `db::{Params, query, query_one}` ve `p!` makrosu (crate içi)
  - Test yardımcıları (`tests/common`): `Fixture { catalog, mgr, registry, server: Uuid, db_oid: u32, ts: TestServer, _dir: TempDir }`, `fixtures() -> Vec<Fixture>` (async), `Fixture::db() -> NodeId`, `Fixture::schema(&str) -> NodeId` (async, oid'i katalogdan bulur), `Fixture::table(&str, &str) -> NodeId` (async), `find(&[TreeNode], &str) -> &TreeNode`

**Listeleme kuralları:**
- Kök: gruplar (alfabetik, büyük/küçük harf duyarsız) sonra grupsuz sunucular (alfabetik). Grup: o gruptaki sunucular.
- Sunucu düğümü: `detail` = bağlıysa kısa sürüm, `flags.connected`, `server_id`.
- Sunucu çocukları: yalnızca "Databases" koleksiyonu (Plan 2'de Roles/Tablespaces eklenir). Bağlı değilse `Error::NotConnected`.
- Databases: şablon olmayanlar; `datallowconn = false` olanlar yaprak (`has_children = false`).
- Database çocukları: "Schemas" koleksiyonu. Schemas: sistem şemaları `show_system` kapalıyken gizli; açıkken `flags.system = true`.
- Schema çocukları: "Tables". Tables: `relkind IN ('r','p')`, partition'lar hariç; `detail = approx_rows(reltuples)`; partitioned → `flags.partitioned`.
- Table çocukları: "Columns". Columns: `attnum` sırasıyla; `detail = format_type`, `flags.primary_key`, `flags.not_null`; yaprak.
- Tanınmayan yol şekli → `Error::Invalid`.

- [ ] **Step 1: Test fixture'ını yaz**

`crates/pgt-catalog/tests/fixtures/schema.sql`:

```sql
DO $$
BEGIN
    IF NOT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'pgt_reader') THEN
        CREATE ROLE pgt_reader NOLOGIN;
    END IF;
END
$$;

CREATE SCHEMA app;
COMMENT ON SCHEMA app IS 'Uygulama şeması';
GRANT USAGE ON SCHEMA app TO pgt_reader;

CREATE TABLE app.customers (
    id bigint GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    email text NOT NULL UNIQUE,
    name varchar(100) COLLATE "C",
    created_at timestamptz NOT NULL DEFAULT now(),
    CONSTRAINT customers_email_check CHECK (position('@' in email) > 1)
);
COMMENT ON TABLE app.customers IS 'Müşteriler';
COMMENT ON COLUMN app.customers.email IS 'E-posta';
GRANT SELECT, INSERT ON app.customers TO pgt_reader;

CREATE TABLE app.orders (
    id bigserial PRIMARY KEY,
    customer_id bigint NOT NULL REFERENCES app.customers (id) ON DELETE CASCADE,
    total numeric(12, 2) NOT NULL DEFAULT 0,
    placed_on date NOT NULL
) WITH (fillfactor = 80);

CREATE TABLE app.events (
    id bigint NOT NULL,
    happened_on date NOT NULL,
    payload jsonb
) PARTITION BY RANGE (happened_on);
CREATE TABLE app.events_2026 PARTITION OF app.events FOR VALUES FROM ('2026-01-01') TO ('2027-01-01');

CREATE TABLE app."Order Items" (
    "Line No" integer NOT NULL,
    order_id bigint NOT NULL REFERENCES app.orders (id),
    "Ürün Adı" text,
    PRIMARY KEY (order_id, "Line No")
);

CREATE UNLOGGED TABLE public.scratch (v text);
```

> Bu şema PG 10–18'in hepsinde çalışır (partitioned tabloda PK/FK yok; generated column yok — onlar Plan 2'de sürüme özel fixture'larla gelir).

`crates/pgt-catalog/tests/common/mod.rs`:

```rust
#![allow(dead_code)]

use std::sync::{Arc, RwLock};

use pgt_catalog::Catalog;
use pgt_catalog::node::{Coll, NodeId, Seg, TreeNode};
use pgt_core::connection::ConnectionManager;
use pgt_core::registry::Registry;
use pgt_core::testing::{TestServer, admin_client, test_servers};
use tokio::sync::OnceCell;
use uuid::Uuid;

pub const FIXTURE_DB: &str = "pgt_fixture";
const SCHEMA_SQL: &str = include_str!("../fixtures/schema.sql");

static SETUP: OnceCell<()> = OnceCell::const_new();

pub struct Fixture {
    pub catalog: Catalog,
    pub mgr: Arc<ConnectionManager>,
    pub registry: Arc<RwLock<Registry>>,
    pub server: Uuid,
    pub db_oid: u32,
    pub ts: TestServer,
    _dir: tempfile::TempDir,
}

/// Her test sunucusunda fixture veritabanını (süreç başına bir kez) yeniden kurar.
async fn setup_all() {
    for ts in test_servers() {
        let admin = admin_client(&ts, &ts.config.maintenance_db).await;
        admin
            .execute(
                "SELECT pg_terminate_backend(pid) FROM pg_stat_activity WHERE datname = $1 AND pid <> pg_backend_pid()",
                &[&FIXTURE_DB],
            )
            .await
            .unwrap();
        admin.batch_execute(&format!("DROP DATABASE IF EXISTS {FIXTURE_DB}")).await.unwrap();
        admin.batch_execute(&format!("CREATE DATABASE {FIXTURE_DB}")).await.unwrap();
        let db = admin_client(&ts, FIXTURE_DB).await;
        db.batch_execute(SCHEMA_SQL).await.unwrap();
        db.batch_execute("ANALYZE").await.unwrap();
    }
}

pub async fn fixtures() -> Vec<Fixture> {
    if test_servers().is_empty() {
        eprintln!("PGT_TEST_URLS tanımlı değil, test atlanıyor");
        return Vec::new();
    }
    SETUP.get_or_init(setup_all).await;
    let mut out = Vec::new();
    for ts in test_servers() {
        let dir = tempfile::tempdir().unwrap();
        let mut registry = Registry::load(dir.path().join("servers.json")).unwrap().registry;
        registry.upsert(ts.config.clone()).unwrap();
        let registry = Arc::new(RwLock::new(registry));
        let mgr = Arc::new(ConnectionManager::new());
        mgr.connect(ts.config.clone(), ts.password.clone()).await.unwrap();
        let admin = admin_client(&ts, &ts.config.maintenance_db).await;
        let db_oid: u32 = admin
            .query_one("SELECT oid FROM pg_database WHERE datname = $1", &[&FIXTURE_DB])
            .await
            .unwrap()
            .get(0);
        out.push(Fixture {
            catalog: Catalog::new(mgr.clone(), registry.clone()),
            mgr,
            registry,
            server: ts.config.id,
            db_oid,
            ts,
            _dir: dir,
        });
    }
    out
}

impl Fixture {
    pub fn server_id(&self) -> NodeId {
        NodeId::server(self.server)
    }

    pub fn db(&self) -> NodeId {
        self.server_id().child(Seg::Db(self.db_oid))
    }

    pub async fn schema(&self, name: &str) -> NodeId {
        let list = self.catalog.list_children(&self.db().child(Seg::Coll(Coll::Schemas))).await.unwrap();
        NodeId::parse(&find(&list, name).id).unwrap()
    }

    pub async fn table(&self, schema: &str, name: &str) -> NodeId {
        let schema = self.schema(schema).await;
        let list = self.catalog.list_children(&schema.child(Seg::Coll(Coll::Tables))).await.unwrap();
        NodeId::parse(&find(&list, name).id).unwrap()
    }

    /// DDL snapshot'larını ortamdan bağımsız kılar: sahip rolü test kullanıcısıdır ve
    /// `OWNER TO x;`, `AUTHORIZATION x;`, `GRANT ... TO x;` satırlarının sonunda geçer.
    pub fn normalize(&self, ddl: &str) -> String {
        let owner = &self.ts.config.username;
        ddl.replace(&format!(" {owner};"), " <owner>;")
    }

    /// Snapshot son eki için sunucunun ana sürümü (ör. 18).
    pub async fn major(&self) -> u32 {
        self.mgr.info(self.server).await.expect("bağlı").version_num / 10_000
    }
}

pub fn find<'a>(list: &'a [TreeNode], label: &str) -> &'a TreeNode {
    list.iter()
        .find(|n| n.label == label)
        .unwrap_or_else(|| panic!("'{label}' bulunamadı; mevcut: {:?}", list.iter().map(|n| &n.label).collect::<Vec<_>>()))
}

pub fn labels(list: &[TreeNode]) -> Vec<&str> {
    list.iter().map(|n| n.label.as_str()).collect()
}
```

- [ ] **Step 2: Başarısız listeleme testlerini yaz**

`crates/pgt-catalog/tests/listing.rs`:

```rust
mod common;

use common::{find, fixtures, labels};
use pgt_catalog::node::{Coll, NodeId, NodeKind, Seg};
use pgt_core::Error;
use pgt_core::config::ServerConfig;

#[tokio::test]
async fn root_lists_groups_then_servers() {
    for fx in fixtures().await {
        {
            let mut reg = fx.registry.write().unwrap();
            let mut grouped = ServerConfig::new("zeta", "h", 5432, "u");
            grouped.group = Some("Üretim".into());
            reg.upsert(grouped).unwrap();
            reg.upsert(ServerConfig::new("alfa", "h", 5432, "u")).unwrap();
        }
        let root = fx.catalog.list_children(&NodeId::Root).await.unwrap();
        assert_eq!(root[0].kind, NodeKind::ServerGroup);
        assert_eq!(root[0].label, "Üretim");
        assert_eq!(root[0].id, "grp:Üretim");
        let servers: Vec<_> = root[1..].iter().map(|n| (n.label.as_str(), n.flags.connected)).collect();
        assert_eq!(servers[0], ("alfa", false));
        let test = root.iter().find(|n| n.server_id == Some(fx.server)).unwrap();
        assert!(test.flags.connected);
        assert!(test.detail.is_some(), "bağlı sunucuda sürüm gösterilmeli");
        let group = fx.catalog.list_children(&NodeId::Group("Üretim".into())).await.unwrap();
        assert_eq!(labels(&group), ["zeta"]);
    }
}

#[tokio::test]
async fn server_has_databases_collection() {
    for fx in fixtures().await {
        let children = fx.catalog.list_children(&fx.server_id()).await.unwrap();
        assert_eq!(children.len(), 1);
        assert_eq!((children[0].kind, children[0].label.as_str()), (NodeKind::DatabaseCollection, "Databases"));
    }
}

#[tokio::test]
async fn databases_exclude_templates() {
    for fx in fixtures().await {
        let dbs = fx.catalog.list_children(&fx.server_id().child(Seg::Coll(Coll::Databases))).await.unwrap();
        let names = labels(&dbs);
        assert!(names.contains(&"pgt_fixture"), "{names:?}");
        assert!(!names.contains(&"template0") && !names.contains(&"template1"));
        assert!(find(&dbs, "pgt_fixture").has_children);
    }
}

#[tokio::test]
async fn databases_without_connect_are_leaves() {
    for fx in fixtures().await {
        let admin = pgt_core::testing::admin_client(&fx.ts, &fx.ts.config.maintenance_db).await;
        // CREATE/DROP DATABASE çok ifadeli sorguda (örtük transaction) çalışmaz; ayrı çağrılar gerekir.
        admin.batch_execute("DROP DATABASE IF EXISTS pgt_noconn").await.unwrap();
        admin.batch_execute("CREATE DATABASE pgt_noconn").await.unwrap();
        admin.batch_execute("ALTER DATABASE pgt_noconn ALLOW_CONNECTIONS false").await.unwrap();
        let dbs = fx.catalog.list_children(&fx.server_id().child(Seg::Coll(Coll::Databases))).await.unwrap();
        assert!(!find(&dbs, "pgt_noconn").has_children);
        admin.batch_execute("DROP DATABASE pgt_noconn").await.unwrap();
    }
}

#[tokio::test]
async fn schemas_hide_system_by_default() {
    for fx in fixtures().await {
        let coll = fx.db().child(Seg::Coll(Coll::Schemas));
        let names: Vec<String> = labels(&fx.catalog.list_children(&coll).await.unwrap()).into_iter().map(String::from).collect();
        assert!(names.contains(&"app".to_string()) && names.contains(&"public".to_string()), "{names:?}");
        assert!(!names.iter().any(|n| n == "pg_catalog" || n == "information_schema" || n.starts_with("pg_toast")));
        fx.catalog.set_show_system(true);
        let all = fx.catalog.list_children(&coll).await.unwrap();
        assert!(find(&all, "pg_catalog").flags.system);
        assert!(!find(&all, "app").flags.system);
        fx.catalog.set_show_system(false);
    }
}

#[tokio::test]
async fn tables_exclude_partitions_and_flag_partitioned() {
    for fx in fixtures().await {
        let schema = fx.schema("app").await;
        let tables = fx.catalog.list_children(&schema.child(Seg::Coll(Coll::Tables))).await.unwrap();
        assert_eq!(labels(&tables), ["Order Items", "customers", "events", "orders"]);
        assert!(find(&tables, "events").flags.partitioned);
        assert!(!find(&tables, "orders").flags.partitioned);
        assert!(tables.iter().all(|t| t.kind == NodeKind::Table && t.has_children));
    }
}

#[tokio::test]
async fn columns_in_order_with_types_and_flags() {
    for fx in fixtures().await {
        let table = fx.table("app", "customers").await;
        let cols = fx.catalog.list_children(&table.child(Seg::Coll(Coll::Columns))).await.unwrap();
        assert_eq!(labels(&cols), ["id", "email", "name", "created_at"]);
        let id = find(&cols, "id");
        assert_eq!(id.detail.as_deref(), Some("bigint"));
        assert!(id.flags.primary_key && id.flags.not_null && !id.has_children);
        assert_eq!(find(&cols, "name").detail.as_deref(), Some("character varying(100)"));
        assert!(!find(&cols, "name").flags.not_null);
        let quoted = fx.table("app", "Order Items").await;
        let cols = fx.catalog.list_children(&quoted.child(Seg::Coll(Coll::Columns))).await.unwrap();
        assert_eq!(labels(&cols), ["Line No", "order_id", "Ürün Adı"]);
    }
}

#[tokio::test]
async fn collection_children_of_table_and_schema() {
    for fx in fixtures().await {
        let schema = fx.schema("app").await;
        let kids = fx.catalog.list_children(&schema).await.unwrap();
        assert_eq!(kids.iter().map(|k| (k.kind, k.label.as_str())).collect::<Vec<_>>(), [(NodeKind::TableCollection, "Tables")]);
        let table = fx.table("app", "orders").await;
        let kids = fx.catalog.list_children(&table).await.unwrap();
        assert_eq!(kids.iter().map(|k| (k.kind, k.label.as_str())).collect::<Vec<_>>(), [(NodeKind::ColumnCollection, "Columns")]);
    }
}

#[tokio::test]
async fn invalid_shapes_and_disconnected_servers_error() {
    for fx in fixtures().await {
        let bad = fx.server_id().child(Seg::Table(1));
        assert!(matches!(fx.catalog.list_children(&bad).await, Err(Error::Invalid(_))));
        let other = NodeId::server(uuid::Uuid::new_v4());
        assert!(matches!(fx.catalog.list_children(&other).await, Err(Error::NotConnected)));
    }
}
```

> Liste SQL'leri `ORDER BY ... COLLATE "C"` (bayt sırası) kullanır; bu yüzden büyük harfle başlayan `"Order Items"` her sunucuda ilk gelir ve sıra sunucunun varsayılan collation'ından bağımsızdır.

- [ ] **Step 3: Testlerin başarısız olduğunu doğrula**

Run: `cargo test -p pgt-catalog --test listing`
Expected: FAIL — `Catalog` tanımlı değil.

- [ ] **Step 4: SQL dosyalarını yaz**

`crates/pgt-catalog/src/sql/databases.sql`:

```sql
SELECT d.oid, d.datname::text AS name, d.datallowconn AS allow_conn
FROM pg_database d
WHERE NOT d.datistemplate
ORDER BY d.datname COLLATE "C"
```

`crates/pgt-catalog/src/sql/schemas.sql`:

```sql
SELECT n.oid, n.nspname::text AS name,
       (n.nspname IN ('pg_catalog', 'information_schema')
        OR n.nspname LIKE 'pg\_toast%'
        OR n.nspname LIKE 'pg\_temp\_%') AS is_system
FROM pg_namespace n
ORDER BY is_system, n.nspname COLLATE "C"
```

`crates/pgt-catalog/src/sql/tables.sql`:

```sql
SELECT c.oid, c.relname::text AS name, c.relkind = 'p' AS partitioned, c.reltuples::float8 AS reltuples
FROM pg_class c
WHERE c.relnamespace = $1
  AND c.relkind IN ('r', 'p')
  AND NOT c.relispartition
ORDER BY c.relname COLLATE "C"
```

`crates/pgt-catalog/src/sql/columns.sql`:

```sql
SELECT a.attnum, a.attname::text AS name, format_type(a.atttypid, a.atttypmod) AS type_name, a.attnotnull AS not_null,
       EXISTS (
           SELECT 1 FROM pg_constraint k
           WHERE k.conrelid = a.attrelid AND k.contype = 'p' AND a.attnum = ANY (k.conkey)
       ) AS is_pk
FROM pg_attribute a
WHERE a.attrelid = $1 AND a.attnum > 0 AND NOT a.attisdropped
ORDER BY a.attnum
```

- [ ] **Step 5: `db.rs` sorgu yardımcılarını yaz**

```rust
use std::sync::Arc;

use pgt_core::connection::{ConnectionManager, Target};
use pgt_core::{Error, Result};
use tokio_postgres::Row;
use tokio_postgres::types::ToSql;
use uuid::Uuid;

use crate::version::{PgVersion, VersionedSql};

pub(crate) type Params = Arc<Vec<Box<dyn ToSql + Sync + Send>>>;

/// `p![oid, attnum]` → yeniden denemede tekrar kullanılabilen parametre listesi.
macro_rules! p {
    ($($x:expr),* $(,)?) => {
        std::sync::Arc::new(vec![$(Box::new($x) as Box<dyn tokio_postgres::types::ToSql + Sync + Send>),*])
    };
}
pub(crate) use p;

/// Sunucu sürümüne uygun SQL varyantını seçip çalıştırır.
pub(crate) async fn query(mgr: &ConnectionManager, server: Uuid, target: Target, sql: &'static VersionedSql, params: Params) -> Result<Vec<Row>> {
    mgr.run(server, target, move |c| {
        let params = params.clone();
        async move {
            let text = sql.pick(PgVersion(c.version_num()));
            let refs: Vec<&(dyn ToSql + Sync)> = params.iter().map(|p| p.as_ref() as &(dyn ToSql + Sync)).collect();
            c.client.query(text, &refs).await
        }
    })
    .await
}

/// Tek satır bekler; satır yoksa `Error::NotFound(what)`.
pub(crate) async fn query_one(mgr: &ConnectionManager, server: Uuid, target: Target, sql: &'static VersionedSql, params: Params, what: &str) -> Result<Row> {
    query(mgr, server, target, sql, params).await?.into_iter().next().ok_or_else(|| Error::NotFound(what.to_string()))
}
```

- [ ] **Step 6: `Catalog` ve tür modüllerini yaz**

`crates/pgt-catalog/src/lib.rs`:

```rust
mod db;
mod kinds;
pub mod node;
pub mod props;
pub mod version;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, RwLock};

use pgt_core::connection::ConnectionManager;
use pgt_core::registry::Registry;
pub use pgt_core::{Error, Result};

use node::{NodeId, TreeNode};

pub struct Catalog {
    pub(crate) mgr: Arc<ConnectionManager>,
    pub(crate) registry: Arc<RwLock<Registry>>,
    show_system: AtomicBool,
}

impl Catalog {
    pub fn new(mgr: Arc<ConnectionManager>, registry: Arc<RwLock<Registry>>) -> Self {
        Self { mgr, registry, show_system: AtomicBool::new(false) }
    }

    pub fn set_show_system(&self, value: bool) {
        self.show_system.store(value, Ordering::Relaxed);
    }

    pub(crate) fn show_system(&self) -> bool {
        self.show_system.load(Ordering::Relaxed)
    }

    pub async fn list_children(&self, id: &NodeId) -> Result<Vec<TreeNode>> {
        kinds::list_children(self, id).await
    }
}
```

`crates/pgt-catalog/src/kinds/mod.rs`:

```rust
mod column;
mod database;
mod schema;
mod server;
mod table;

use pgt_core::{Error, Result};

use crate::Catalog;
use crate::node::{Coll, NodeId, NodeKind, Seg, TreeNode};

pub(crate) fn invalid(id: &NodeId) -> Error {
    Error::Invalid(format!("desteklenmeyen düğüm: {id}"))
}

pub(crate) fn collection(parent: &NodeId, coll: Coll, kind: NodeKind, label: &str) -> TreeNode {
    TreeNode::new(&parent.child(Seg::Coll(coll)), kind, label)
}

pub(crate) async fn list_children(cat: &Catalog, id: &NodeId) -> Result<Vec<TreeNode>> {
    let (server, path) = match id {
        NodeId::Root => return server::root_children(cat).await,
        NodeId::Group(name) => return server::group_children(cat, name).await,
        NodeId::Node { server, path } => (*server, path.as_slice()),
    };
    if !cat.mgr.is_connected(server) {
        return Err(Error::NotConnected);
    }
    match path {
        [] => Ok(vec![collection(id, Coll::Databases, NodeKind::DatabaseCollection, "Databases")]),
        [Seg::Coll(Coll::Databases)] => database::list(cat, server).await,
        [Seg::Db(_)] => Ok(vec![collection(id, Coll::Schemas, NodeKind::SchemaCollection, "Schemas")]),
        [Seg::Db(db), Seg::Coll(Coll::Schemas)] => schema::list(cat, server, *db).await,
        [Seg::Db(_), Seg::Schema(_)] => Ok(vec![collection(id, Coll::Tables, NodeKind::TableCollection, "Tables")]),
        [Seg::Db(db), Seg::Schema(sch), Seg::Coll(Coll::Tables)] => table::list(cat, server, *db, *sch).await,
        [Seg::Db(_), Seg::Schema(_), Seg::Table(_)] => {
            Ok(vec![collection(id, Coll::Columns, NodeKind::ColumnCollection, "Columns")])
        }
        [Seg::Db(db), Seg::Schema(sch), Seg::Table(tbl), Seg::Coll(Coll::Columns)] => {
            column::list(cat, server, *db, *sch, *tbl).await
        }
        [Seg::Db(_), Seg::Schema(_), Seg::Table(_), Seg::Column(_)] => Ok(Vec::new()),
        _ => Err(invalid(id)),
    }
}
```

`crates/pgt-catalog/src/kinds/server.rs`:

```rust
use pgt_core::Result;
use pgt_core::config::ServerConfig;

use crate::Catalog;
use crate::node::{NodeFlags, NodeId, NodeKind, TreeNode};

fn sorted(mut servers: Vec<ServerConfig>) -> Vec<ServerConfig> {
    servers.sort_by_key(|s| s.name.to_lowercase());
    servers
}

async fn server_node(cat: &Catalog, s: &ServerConfig) -> TreeNode {
    let id = NodeId::server(s.id);
    let info = cat.mgr.info(s.id).await;
    TreeNode::new(&id, NodeKind::Server, s.name.clone())
        .with_detail(info.as_ref().map(|i| i.version.clone()))
        .with_server(s.id)
        .with_flags(NodeFlags { connected: info.is_some(), ..NodeFlags::default() })
}

pub(crate) async fn root_children(cat: &Catalog) -> Result<Vec<TreeNode>> {
    let servers = cat.registry.read().unwrap().servers().to_vec();
    let mut groups: Vec<String> = servers.iter().filter_map(|s| s.group_name().map(str::to_string)).collect();
    groups.sort_by_key(|g| g.to_lowercase());
    groups.dedup();
    let mut out: Vec<TreeNode> = groups
        .into_iter()
        .map(|g| TreeNode::new(&NodeId::Group(g.clone()), NodeKind::ServerGroup, g))
        .collect();
    for s in sorted(servers.into_iter().filter(|s| s.group_name().is_none()).collect()) {
        out.push(server_node(cat, &s).await);
    }
    Ok(out)
}

pub(crate) async fn group_children(cat: &Catalog, group: &str) -> Result<Vec<TreeNode>> {
    let servers = cat.registry.read().unwrap().servers().to_vec();
    let mut out = Vec::new();
    for s in sorted(servers.into_iter().filter(|s| s.group_name() == Some(group)).collect()) {
        out.push(server_node(cat, &s).await);
    }
    Ok(out)
}
```

`crates/pgt-catalog/src/kinds/database.rs`:

```rust
use pgt_core::Result;
use pgt_core::connection::Target;
use uuid::Uuid;

use crate::Catalog;
use crate::db::{p, query};
use crate::node::{NodeId, NodeKind, Seg, TreeNode};
use crate::version::VersionedSql;

static LIST: VersionedSql = VersionedSql(&[(10, include_str!("../sql/databases.sql"))]);

pub(crate) async fn list(cat: &Catalog, server: Uuid) -> Result<Vec<TreeNode>> {
    let rows = query(&cat.mgr, server, Target::Maintenance, &LIST, p![]).await?;
    Ok(rows
        .iter()
        .map(|r| {
            let oid: u32 = r.get("oid");
            let node = TreeNode::new(&NodeId::server(server).child(Seg::Db(oid)), NodeKind::Database, r.get::<_, String>("name"));
            if r.get::<_, bool>("allow_conn") { node } else { node.leaf() }
        })
        .collect())
}
```

`crates/pgt-catalog/src/kinds/schema.rs`:

```rust
use pgt_core::Result;
use pgt_core::connection::Target;
use uuid::Uuid;

use crate::Catalog;
use crate::db::{p, query};
use crate::node::{NodeFlags, NodeId, NodeKind, Seg, TreeNode};
use crate::version::VersionedSql;

static LIST: VersionedSql = VersionedSql(&[(10, include_str!("../sql/schemas.sql"))]);

pub(crate) async fn list(cat: &Catalog, server: Uuid, db: u32) -> Result<Vec<TreeNode>> {
    let rows = query(&cat.mgr, server, Target::Database(db), &LIST, p![]).await?;
    let show_system = cat.show_system();
    let base = NodeId::server(server).child(Seg::Db(db));
    Ok(rows
        .iter()
        .filter(|r| show_system || !r.get::<_, bool>("is_system"))
        .map(|r| {
            TreeNode::new(&base.child(Seg::Schema(r.get("oid"))), NodeKind::Schema, r.get::<_, String>("name"))
                .with_flags(NodeFlags { system: r.get("is_system"), ..NodeFlags::default() })
        })
        .collect())
}
```

`crates/pgt-catalog/src/kinds/table.rs`:

```rust
use pgt_core::Result;
use pgt_core::connection::Target;
use uuid::Uuid;

use crate::Catalog;
use crate::db::{p, query};
use crate::node::{NodeFlags, NodeId, NodeKind, Seg, TreeNode};
use crate::props::approx_rows;
use crate::version::VersionedSql;

static LIST: VersionedSql = VersionedSql(&[(10, include_str!("../sql/tables.sql"))]);

pub(crate) async fn list(cat: &Catalog, server: Uuid, db: u32, schema: u32) -> Result<Vec<TreeNode>> {
    let rows = query(&cat.mgr, server, Target::Database(db), &LIST, p![schema]).await?;
    let base = NodeId::server(server).child(Seg::Db(db)).child(Seg::Schema(schema));
    Ok(rows
        .iter()
        .map(|r| {
            TreeNode::new(&base.child(Seg::Table(r.get("oid"))), NodeKind::Table, r.get::<_, String>("name"))
                .with_detail(approx_rows(r.get("reltuples")))
                .with_flags(NodeFlags { partitioned: r.get("partitioned"), ..NodeFlags::default() })
        })
        .collect())
}
```

`crates/pgt-catalog/src/kinds/column.rs`:

```rust
use pgt_core::Result;
use pgt_core::connection::Target;
use uuid::Uuid;

use crate::Catalog;
use crate::db::{p, query};
use crate::node::{NodeFlags, NodeId, NodeKind, Seg, TreeNode};
use crate::version::VersionedSql;

static LIST: VersionedSql = VersionedSql(&[(10, include_str!("../sql/columns.sql"))]);

pub(crate) async fn list(cat: &Catalog, server: Uuid, db: u32, schema: u32, table: u32) -> Result<Vec<TreeNode>> {
    let rows = query(&cat.mgr, server, Target::Database(db), &LIST, p![table]).await?;
    let base = NodeId::server(server).child(Seg::Db(db)).child(Seg::Schema(schema)).child(Seg::Table(table));
    Ok(rows
        .iter()
        .map(|r| {
            TreeNode::new(&base.child(Seg::Column(r.get("attnum"))), NodeKind::Column, r.get::<_, String>("name"))
                .with_detail(Some(r.get("type_name")))
                .with_flags(NodeFlags { primary_key: r.get("is_pk"), not_null: r.get("not_null"), ..NodeFlags::default() })
                .leaf()
        })
        .collect())
}
```

- [ ] **Step 7: Testlerin geçtiğini doğrula**

Run (sunucusuz): `cargo test -p pgt-catalog`
Expected: PASS; listing testleri "atlanıyor" yazar.

Run (PG 18 ile):

```powershell
$env:PGT_TEST_URLS = "postgres://postgres:PAROLA@localhost:5432/postgres"
cargo test -p pgt-catalog --test listing
```

Expected: PASS (9 test).

- [ ] **Step 8: Commit**

```bash
git add crates/pgt-catalog
git commit -m "feat(catalog): kökten kolonlara ağaç listeleme ve test fixture'ı"
```

---

### Task 7: Properties (Server, Database, Schema, Table, Column)

**Files:**
- Create: `crates/pgt-catalog/src/sql/database_props.sql`, `schema_props.sql`, `table_props.sql`, `column_props.sql`, `column_props.pg10.sql` (hepsi `crates/pgt-catalog/src/sql/` altında), `crates/pgt-catalog/tests/properties.rs`
- Modify: `crates/pgt-catalog/src/lib.rs`, `crates/pgt-catalog/src/kinds/{mod,server,database,schema,table,column}.rs`

**Interfaces:**
- Consumes: Task 6 `Catalog`, `db::{query_one, p}`, `props::{PropertySection, yes_no, approx_rows}`
- Produces: `Catalog::properties(&self, &NodeId) -> Result<Vec<PropertySection>>` (async). Kök, grup ve koleksiyonlar için boş liste. Bulunamayan OID → `Error::NotFound`.

**Bölüm ve etiketler (arayüzde aynen görünür):**

| Tür | Bölüm → satırlar |
|---|---|
| Server | Genel: Ad, Grup, Host, Port, Kullanıcı, Bakım veritabanı, SSL modu · Sunucu (bağlıysa): Sürüm, Sürüm numarası |
| Database | Genel: Ad, OID, Sahip, Yorum · Tanım: Encoding, Collation, Character type, Tablespace, Bağlantı sınırı, Bağlantıya izin · İstatistik: Boyut |
| Schema | Genel: Ad, OID, Sahip, Yorum · Güvenlik: Yetkiler (ACL metni) |
| Table | Genel: Ad, Şema, OID, Sahip, Yorum · Tanım: Tür, Kalıcılık, Tablespace, Seçenekler · İstatistik: Tahmini satır, Toplam boyut, Tablo boyutu, İndeks boyutu, Son vacuum, Son autovacuum, Son analyze, Son autoanalyze |
| Column | Genel: Ad, Sıra (attnum), Veri tipi, Boş olamaz, Yorum · Tanım: Varsayılan / Üretilen ifade, Identity, Collation |

Değeri NULL veya boş olan satırlar gösterilmez (`row_opt`).

- [ ] **Step 1: Başarısız testleri yaz**

`crates/pgt-catalog/tests/properties.rs`:

```rust
mod common;

use common::fixtures;
use pgt_catalog::node::{Coll, NodeId, Seg};
use pgt_catalog::props::PropertySection;
use pgt_core::Error;

fn prop<'a>(sections: &'a [PropertySection], title: &str, label: &str) -> Option<&'a str> {
    sections.iter().find(|s| s.title == title)?.rows.iter().find(|r| r.label == label).map(|r| r.value.as_str())
}

#[tokio::test]
async fn server_properties_without_query() {
    for fx in fixtures().await {
        let s = fx.catalog.properties(&fx.server_id()).await.unwrap();
        assert_eq!(prop(&s, "Genel", "Host"), Some(fx.ts.config.host.as_str()));
        assert_eq!(prop(&s, "Genel", "SSL modu"), Some("prefer"));
        assert!(prop(&s, "Sunucu", "Sürüm").is_some());
    }
}

#[tokio::test]
async fn database_properties() {
    for fx in fixtures().await {
        let s = fx.catalog.properties(&fx.db()).await.unwrap();
        assert_eq!(prop(&s, "Genel", "Ad"), Some("pgt_fixture"));
        assert_eq!(prop(&s, "Genel", "OID"), Some(fx.db_oid.to_string().as_str()));
        assert!(prop(&s, "Tanım", "Encoding").is_some());
        assert!(prop(&s, "İstatistik", "Boyut").is_some());
    }
}

#[tokio::test]
async fn database_properties_without_connect_privilege() {
    for fx in fixtures().await {
        let admin = pgt_core::testing::admin_client(&fx.ts, &fx.ts.config.maintenance_db).await;
        admin.batch_execute("DROP DATABASE IF EXISTS pgt_private").await.unwrap();
        admin.batch_execute("CREATE DATABASE pgt_private").await.unwrap();
        admin.batch_execute("REVOKE CONNECT ON DATABASE pgt_private FROM PUBLIC").await.unwrap();
        admin.batch_execute("DO $$ BEGIN IF NOT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'pgt_limited') THEN CREATE ROLE pgt_limited LOGIN PASSWORD 'pgt_limited'; END IF; END $$").await.unwrap();
        let oid: u32 = admin.query_one("SELECT oid FROM pg_database WHERE datname = 'pgt_private'", &[]).await.unwrap().get(0);

        let mut limited = fx.ts.config.clone();
        limited.id = uuid::Uuid::new_v4();
        limited.username = "pgt_limited".into();
        fx.registry.write().unwrap().upsert(limited.clone()).unwrap();
        fx.mgr.connect(limited.clone(), Some("pgt_limited".into())).await.unwrap();

        let id = NodeId::server(limited.id).child(Seg::Db(oid));
        let s = fx.catalog.properties(&id).await.unwrap();
        assert_eq!(prop(&s, "Genel", "Ad"), Some("pgt_private"));
        assert_eq!(prop(&s, "İstatistik", "Boyut"), None, "yetki yoksa boyut gösterilmez");
        fx.mgr.disconnect(limited.id);
        admin.batch_execute("DROP DATABASE pgt_private").await.unwrap();
    }
}

#[tokio::test]
async fn schema_properties() {
    for fx in fixtures().await {
        let s = fx.catalog.properties(&fx.schema("app").await).await.unwrap();
        assert_eq!(prop(&s, "Genel", "Ad"), Some("app"));
        assert_eq!(prop(&s, "Genel", "Yorum"), Some("Uygulama şeması"));
        assert!(prop(&s, "Güvenlik", "Yetkiler").unwrap().contains("pgt_reader=U"));
    }
}

#[tokio::test]
async fn table_properties() {
    for fx in fixtures().await {
        let s = fx.catalog.properties(&fx.table("app", "customers").await).await.unwrap();
        assert_eq!(prop(&s, "Genel", "Ad"), Some("customers"));
        assert_eq!(prop(&s, "Genel", "Şema"), Some("app"));
        assert_eq!(prop(&s, "Genel", "Yorum"), Some("Müşteriler"));
        assert_eq!(prop(&s, "Tanım", "Tür"), Some("Table"));
        assert_eq!(prop(&s, "Tanım", "Kalıcılık"), Some("permanent"));
        assert!(prop(&s, "İstatistik", "Toplam boyut").is_some());
        let orders = fx.catalog.properties(&fx.table("app", "orders").await).await.unwrap();
        assert_eq!(prop(&orders, "Tanım", "Seçenekler"), Some("fillfactor=80"));
        let events = fx.catalog.properties(&fx.table("app", "events").await).await.unwrap();
        assert_eq!(prop(&events, "Tanım", "Tür"), Some("Partitioned table"));
        let scratch = fx.catalog.properties(&fx.table("public", "scratch").await).await.unwrap();
        assert_eq!(prop(&scratch, "Tanım", "Kalıcılık"), Some("unlogged"));
    }
}

#[tokio::test]
async fn column_properties() {
    for fx in fixtures().await {
        let table = fx.table("app", "customers").await;
        let col = |n: i16| table.child(Seg::Column(n));
        let id = fx.catalog.properties(&col(1)).await.unwrap();
        assert_eq!(prop(&id, "Genel", "Veri tipi"), Some("bigint"));
        assert_eq!(prop(&id, "Tanım", "Identity"), Some("ALWAYS"));
        let email = fx.catalog.properties(&col(2)).await.unwrap();
        assert_eq!(prop(&email, "Genel", "Boş olamaz"), Some("Evet"));
        assert_eq!(prop(&email, "Genel", "Yorum"), Some("E-posta"));
        let name = fx.catalog.properties(&col(3)).await.unwrap();
        assert_eq!(prop(&name, "Tanım", "Collation"), Some("\"C\""));
        let created = fx.catalog.properties(&col(4)).await.unwrap();
        assert_eq!(prop(&created, "Tanım", "Varsayılan"), Some("now()"));
    }
}

#[tokio::test]
async fn collections_have_no_properties_and_missing_oid_is_not_found() {
    for fx in fixtures().await {
        let coll = fx.db().child(Seg::Coll(Coll::Schemas));
        assert!(fx.catalog.properties(&coll).await.unwrap().is_empty());
        assert!(fx.catalog.properties(&NodeId::Root).await.unwrap().is_empty());
        let schema = fx.schema("app").await;
        let ghost = schema.child(Seg::Table(4_000_000_000));
        assert!(matches!(fx.catalog.properties(&ghost).await, Err(Error::NotFound(_))));
    }
}
```

- [ ] **Step 2: Testlerin başarısız olduğunu doğrula**

Run: `cargo test -p pgt-catalog --test properties`
Expected: FAIL — `properties` metodu yok.

- [ ] **Step 3: SQL dosyalarını yaz**

`sql/database_props.sql`:

```sql
SELECT d.datname::text AS name, d.oid, pg_get_userbyid(d.datdba)::text AS owner,
       shobj_description(d.oid, 'pg_database') AS comment,
       pg_encoding_to_char(d.encoding)::text AS encoding,
       d.datcollate::text AS collate, d.datctype::text AS ctype,
       t.spcname::text AS tablespace, d.datconnlimit AS conn_limit, d.datallowconn AS allow_conn,
       CASE WHEN has_database_privilege(d.oid, 'CONNECT') THEN pg_size_pretty(pg_database_size(d.oid)) END AS size
FROM pg_database d
JOIN pg_tablespace t ON t.oid = d.dattablespace
WHERE d.oid = $1
```

`sql/schema_props.sql`:

```sql
SELECT n.nspname::text AS name, n.oid, pg_get_userbyid(n.nspowner)::text AS owner,
       obj_description(n.oid, 'pg_namespace') AS comment,
       array_to_string(n.nspacl, ', ') AS acl
FROM pg_namespace n
WHERE n.oid = $1
```

`sql/table_props.sql`:

```sql
SELECT c.relname::text AS name, n.nspname::text AS schema, c.oid, pg_get_userbyid(c.relowner)::text AS owner,
       obj_description(c.oid, 'pg_class') AS comment,
       CASE c.relkind WHEN 'p' THEN 'Partitioned table' ELSE 'Table' END AS kind,
       CASE c.relpersistence WHEN 'u' THEN 'unlogged' WHEN 't' THEN 'temporary' ELSE 'permanent' END AS persistence,
       COALESCE(t.spcname::text, 'pg_default') AS tablespace,
       array_to_string(c.reloptions, ', ') AS options,
       c.reltuples::float8 AS reltuples,
       pg_size_pretty(pg_total_relation_size(c.oid)) AS total_size,
       pg_size_pretty(pg_relation_size(c.oid)) AS table_size,
       pg_size_pretty(pg_indexes_size(c.oid)) AS indexes_size,
       to_char(s.last_vacuum, 'YYYY-MM-DD HH24:MI:SS') AS last_vacuum,
       to_char(s.last_autovacuum, 'YYYY-MM-DD HH24:MI:SS') AS last_autovacuum,
       to_char(s.last_analyze, 'YYYY-MM-DD HH24:MI:SS') AS last_analyze,
       to_char(s.last_autoanalyze, 'YYYY-MM-DD HH24:MI:SS') AS last_autoanalyze
FROM pg_class c
JOIN pg_namespace n ON n.oid = c.relnamespace
LEFT JOIN pg_tablespace t ON t.oid = c.reltablespace
LEFT JOIN pg_stat_all_tables s ON s.relid = c.oid
WHERE c.oid = $1 AND c.relkind IN ('r', 'p')
```

`sql/column_props.sql` (PG 12+):

```sql
SELECT a.attname::text AS name, a.attnum, format_type(a.atttypid, a.atttypmod) AS type_name, a.attnotnull AS not_null,
       pg_get_expr(d.adbin, d.adrelid) AS default_expr,
       CASE a.attidentity WHEN 'a' THEN 'ALWAYS' WHEN 'd' THEN 'BY DEFAULT' END AS identity,
       CASE a.attgenerated WHEN 's' THEN 'STORED' WHEN 'v' THEN 'VIRTUAL' END AS generated,
       CASE WHEN a.attcollation <> 0 AND a.attcollation <> t.typcollation THEN quote_ident(co.collname) END AS collation,
       col_description(a.attrelid, a.attnum) AS comment
FROM pg_attribute a
JOIN pg_type t ON t.oid = a.atttypid
LEFT JOIN pg_attrdef d ON d.adrelid = a.attrelid AND d.adnum = a.attnum
LEFT JOIN pg_collation co ON co.oid = a.attcollation
WHERE a.attrelid = $1 AND a.attnum = $2 AND NOT a.attisdropped
```

`sql/column_props.pg10.sql` (PG 10–11; `attgenerated` yok):

```sql
SELECT a.attname::text AS name, a.attnum, format_type(a.atttypid, a.atttypmod) AS type_name, a.attnotnull AS not_null,
       pg_get_expr(d.adbin, d.adrelid) AS default_expr,
       CASE a.attidentity WHEN 'a' THEN 'ALWAYS' WHEN 'd' THEN 'BY DEFAULT' END AS identity,
       NULL::text AS generated,
       CASE WHEN a.attcollation <> 0 AND a.attcollation <> t.typcollation THEN quote_ident(co.collname) END AS collation,
       col_description(a.attrelid, a.attnum) AS comment
FROM pg_attribute a
JOIN pg_type t ON t.oid = a.atttypid
LEFT JOIN pg_attrdef d ON d.adrelid = a.attrelid AND d.adnum = a.attnum
LEFT JOIN pg_collation co ON co.oid = a.attcollation
WHERE a.attrelid = $1 AND a.attnum = $2 AND NOT a.attisdropped
```

- [ ] **Step 4: Dağıtıcıyı genişlet**

`lib.rs` `impl Catalog` içine:

```rust
    pub async fn properties(&self, id: &NodeId) -> Result<Vec<props::PropertySection>> {
        kinds::properties(self, id).await
    }
```

`kinds/mod.rs` sonuna:

```rust
pub(crate) async fn properties(cat: &Catalog, id: &NodeId) -> Result<Vec<crate::props::PropertySection>> {
    let (server, path) = match id {
        NodeId::Root | NodeId::Group(_) => return Ok(Vec::new()),
        NodeId::Node { server, path } => (*server, path.as_slice()),
    };
    if let [] = path {
        return server::properties(cat, server).await;
    }
    if !cat.mgr.is_connected(server) {
        return Err(Error::NotConnected);
    }
    match path {
        [.., Seg::Coll(_)] => Ok(Vec::new()),
        [Seg::Db(db)] => database::properties(cat, server, *db).await,
        [Seg::Db(db), Seg::Schema(sch)] => schema::properties(cat, server, *db, *sch).await,
        [Seg::Db(db), Seg::Schema(_), Seg::Table(tbl)] => table::properties(cat, server, *db, *tbl).await,
        [Seg::Db(db), Seg::Schema(_), Seg::Table(tbl), Seg::Column(col)] => column::properties(cat, server, *db, *tbl, *col).await,
        _ => Err(invalid(id)),
    }
}
```

- [ ] **Step 5: Tür modüllerine `properties` ekle**

`kinds/server.rs` sonuna:

```rust
pub(crate) async fn properties(cat: &Catalog, server: uuid::Uuid) -> Result<Vec<crate::props::PropertySection>> {
    use crate::props::PropertySection;
    let config = cat
        .registry
        .read()
        .unwrap()
        .get(server)
        .cloned()
        .ok_or(pgt_core::Error::UnknownServer(server))?;
    let ssl_mode = serde_json::to_value(config.ssl.mode).ok().and_then(|v| v.as_str().map(str::to_string)).unwrap_or_default();
    let mut out = vec![
        PropertySection::new("Genel")
            .row("Ad", config.name.clone())
            .row_opt("Grup", config.group_name())
            .row("Host", config.host.clone())
            .row("Port", config.port.to_string())
            .row("Kullanıcı", config.username.clone())
            .row("Bakım veritabanı", config.maintenance_db.clone())
            .row("SSL modu", ssl_mode),
    ];
    if let Some(info) = cat.mgr.info(server).await {
        out.push(PropertySection::new("Sunucu").row("Sürüm", info.version).row("Sürüm numarası", info.version_num.to_string()));
    }
    Ok(out)
}
```

`crates/pgt-catalog/Cargo.toml` `[dependencies]` altına `serde_json = "1"` ekleyin (sslmode'un kebab-case adını almak için).

`kinds/database.rs` sonuna:

```rust
static PROPS: VersionedSql = VersionedSql(&[(10, include_str!("../sql/database_props.sql"))]);

pub(crate) async fn properties(cat: &Catalog, server: Uuid, db: u32) -> Result<Vec<crate::props::PropertySection>> {
    use crate::props::{PropertySection, yes_no};
    let r = crate::db::query_one(&cat.mgr, server, Target::Maintenance, &PROPS, p![db], "veritabanı").await?;
    Ok(vec![
        PropertySection::new("Genel")
            .row("Ad", r.get::<_, String>("name"))
            .row("OID", r.get::<_, u32>("oid").to_string())
            .row("Sahip", r.get::<_, String>("owner"))
            .row_opt("Yorum", r.get::<_, Option<String>>("comment")),
        PropertySection::new("Tanım")
            .row("Encoding", r.get::<_, String>("encoding"))
            .row("Collation", r.get::<_, String>("collate"))
            .row("Character type", r.get::<_, String>("ctype"))
            .row("Tablespace", r.get::<_, String>("tablespace"))
            .row("Bağlantı sınırı", r.get::<_, i32>("conn_limit").to_string())
            .row("Bağlantıya izin", yes_no(r.get("allow_conn"))),
        PropertySection::new("İstatistik").row_opt("Boyut", r.get::<_, Option<String>>("size")),
    ])
}
```

`kinds/schema.rs` sonuna:

```rust
static PROPS: VersionedSql = VersionedSql(&[(10, include_str!("../sql/schema_props.sql"))]);

pub(crate) async fn properties(cat: &Catalog, server: Uuid, db: u32, schema: u32) -> Result<Vec<crate::props::PropertySection>> {
    use crate::props::PropertySection;
    let r = crate::db::query_one(&cat.mgr, server, Target::Database(db), &PROPS, p![schema], "şema").await?;
    Ok(vec![
        PropertySection::new("Genel")
            .row("Ad", r.get::<_, String>("name"))
            .row("OID", r.get::<_, u32>("oid").to_string())
            .row("Sahip", r.get::<_, String>("owner"))
            .row_opt("Yorum", r.get::<_, Option<String>>("comment")),
        PropertySection::new("Güvenlik").row_opt("Yetkiler", r.get::<_, Option<String>>("acl")),
    ])
}
```

`kinds/table.rs` sonuna:

```rust
static PROPS: VersionedSql = VersionedSql(&[(10, include_str!("../sql/table_props.sql"))]);

pub(crate) async fn properties(cat: &Catalog, server: Uuid, db: u32, table: u32) -> Result<Vec<crate::props::PropertySection>> {
    use crate::props::PropertySection;
    let r = crate::db::query_one(&cat.mgr, server, Target::Database(db), &PROPS, p![table], "tablo").await?;
    let text = |c: &str| r.get::<_, Option<String>>(c);
    Ok(vec![
        PropertySection::new("Genel")
            .row("Ad", r.get::<_, String>("name"))
            .row("Şema", r.get::<_, String>("schema"))
            .row("OID", r.get::<_, u32>("oid").to_string())
            .row("Sahip", r.get::<_, String>("owner"))
            .row_opt("Yorum", text("comment")),
        PropertySection::new("Tanım")
            .row("Tür", r.get::<_, String>("kind"))
            .row("Kalıcılık", r.get::<_, String>("persistence"))
            .row("Tablespace", r.get::<_, String>("tablespace"))
            .row_opt("Seçenekler", text("options")),
        PropertySection::new("İstatistik")
            .row_opt("Tahmini satır", approx_rows(r.get("reltuples")))
            .row_opt("Toplam boyut", text("total_size"))
            .row_opt("Tablo boyutu", text("table_size"))
            .row_opt("İndeks boyutu", text("indexes_size"))
            .row_opt("Son vacuum", text("last_vacuum"))
            .row_opt("Son autovacuum", text("last_autovacuum"))
            .row_opt("Son analyze", text("last_analyze"))
            .row_opt("Son autoanalyze", text("last_autoanalyze")),
    ])
}
```

`kinds/column.rs` sonuna:

```rust
static PROPS: VersionedSql = VersionedSql(&[
    (10, include_str!("../sql/column_props.pg10.sql")),
    (12, include_str!("../sql/column_props.sql")),
]);

pub(crate) async fn properties(cat: &Catalog, server: Uuid, db: u32, table: u32, attnum: i16) -> Result<Vec<crate::props::PropertySection>> {
    use crate::props::{PropertySection, yes_no};
    let r = crate::db::query_one(&cat.mgr, server, Target::Database(db), &PROPS, p![table, attnum], "kolon").await?;
    let text = |c: &str| r.get::<_, Option<String>>(c);
    let generated = text("generated");
    let (default_label, default_value) = match &generated {
        Some(kind) => (format!("Üretilen ifade ({kind})"), text("default_expr")),
        None => ("Varsayılan".to_string(), text("default_expr")),
    };
    Ok(vec![
        PropertySection::new("Genel")
            .row("Ad", r.get::<_, String>("name"))
            .row("Sıra", r.get::<_, i16>("attnum").to_string())
            .row("Veri tipi", r.get::<_, String>("type_name"))
            .row("Boş olamaz", yes_no(r.get("not_null")))
            .row_opt("Yorum", text("comment")),
        PropertySection::new("Tanım")
            .row_opt(default_label, default_value)
            .row_opt("Identity", text("identity"))
            .row_opt("Collation", text("collation")),
    ])
}
```

- [ ] **Step 6: Testlerin geçtiğini doğrula**

Run: `cargo test -p pgt-catalog --test properties` (PG 18 ile `PGT_TEST_URLS` tanımlıyken)
Expected: PASS (7 test).

Run: `cargo test -p pgt-catalog`
Expected: PASS (tüm test dosyaları).

- [ ] **Step 7: Commit**

```bash
git add crates/pgt-catalog
git commit -m "feat(catalog): Server/Database/Schema/Table/Column özellikleri"
```

---

### Task 8: DDL üretimi (Database, Schema, Table, Column)

**Files:**
- Create: `crates/pgt-catalog/src/ddl.rs`, `crates/pgt-catalog/src/sql/{database_ddl,schema_ddl,table_ddl_head,table_ddl_columns,table_ddl_columns.pg10,table_ddl_constraints,acl_database,acl_namespace,acl_relation}.sql`, `crates/pgt-catalog/tests/ddl.rs`
- Modify: `crates/pgt-catalog/src/lib.rs`, `crates/pgt-catalog/src/kinds/{mod,database,schema,table,column}.rs`

**Interfaces:**
- Consumes: Task 6–7 dağıtıcı ve sorgu yardımcıları
- Produces:
  - `Catalog::ddl(&self, &NodeId) -> Result<String>` (async). Kök, grup, sunucu ve koleksiyonlar için boş string.
  - `ddl::{header, owner, comment, grants, column_line, literal, AclEntry, ColumnDef}` (crate içi)

**Çıktı biçimi (pgAdmin benzeri):**

```sql
-- Table: app.customers

-- DROP TABLE IF EXISTS app.customers;

CREATE TABLE IF NOT EXISTS app.customers
(
    id bigint NOT NULL GENERATED ALWAYS AS IDENTITY,
    email text NOT NULL,
    name character varying(100) COLLATE pg_catalog."C",
    created_at timestamp with time zone NOT NULL DEFAULT now(),
    CONSTRAINT customers_pkey PRIMARY KEY (id),
    CONSTRAINT customers_email_key UNIQUE (email),
    CONSTRAINT customers_email_check CHECK ("position"(email, '@'::text) > 1)
);

ALTER TABLE IF EXISTS app.customers
    OWNER TO postgres;

COMMENT ON TABLE app.customers
    IS 'Müşteriler';

COMMENT ON COLUMN app.customers.email
    IS 'E-posta';

GRANT SELECT, INSERT ON TABLE app.customers TO pgt_reader;
```

Kurallar:
- Kolon satırı sırası: `ad tip [COLLATE x] [NOT NULL] [DEFAULT ifade | GENERATED ALWAYS AS (ifade) STORED|VIRTUAL | GENERATED ALWAYS|BY DEFAULT AS IDENTITY]`.
- Kısıtlar `pg_get_constraintdef(oid, true)` ile, sıra: PK, UNIQUE, CHECK, EXCLUDE, FK, sonra ad. `contype = 'n'` (PG 18 NOT NULL kısıtları) **hariç** — NOT NULL kolon satırında yazılır (adlı NOT NULL'lar Plan 2'de).
- `PARTITION BY` (`pg_get_partkeydef`), `WITH (reloptions)`, yalnızca varsayılan dışı `TABLESPACE`. `UNLOGGED` → `CREATE UNLOGGED TABLE`.
- GRANT: `aclexplode` satırları (grantee, grantable) çiftine göre gruplanır; yetkiler sabit sırayla: SELECT, INSERT, UPDATE, DELETE, TRUNCATE, REFERENCES, TRIGGER, MAINTAIN, USAGE, CREATE, CONNECT, TEMPORARY, EXECUTE (bilinmeyenler sona, alfabetik). `ALL` kısaltması kullanılmaz. ACL NULL ise (varsayılan yetkiler) GRANT yazılmaz.
- Bloklar arasında bir boş satır; dosya `\n` ile biter.

- [ ] **Step 1: Yardımcılar için başarısız birim testlerini yaz**

`crates/pgt-catalog/src/ddl.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    fn acl(grantee: &str, privilege: &str, grantable: bool) -> AclEntry {
        AclEntry { grantee: grantee.into(), privilege: privilege.into(), grantable }
    }

    #[test]
    fn header_owner_comment() {
        assert_eq!(header("Table", "app.t", "DROP TABLE IF EXISTS app.t;"), "-- Table: app.t\n\n-- DROP TABLE IF EXISTS app.t;\n\n");
        assert_eq!(owner("TABLE IF EXISTS", "app.t", "bob"), "ALTER TABLE IF EXISTS app.t\n    OWNER TO bob;\n");
        assert_eq!(comment("TABLE", "app.t", Some("'x'")), "COMMENT ON TABLE app.t\n    IS 'x';\n");
        assert_eq!(comment("TABLE", "app.t", None), "");
    }

    #[test]
    fn grants_group_and_order_privileges() {
        let entries = [
            acl("pgt_reader", "INSERT", false),
            acl("PUBLIC", "SELECT", false),
            acl("pgt_reader", "SELECT", false),
            acl("admin", "UPDATE", true),
            acl("admin", "SELECT", true),
        ];
        assert_eq!(
            grants("TABLE", "app.t", &entries),
            "GRANT SELECT, INSERT ON TABLE app.t TO pgt_reader;\n\
             GRANT SELECT ON TABLE app.t TO PUBLIC;\n\
             GRANT SELECT, UPDATE ON TABLE app.t TO admin WITH GRANT OPTION;\n"
        );
        assert_eq!(grants("TABLE", "app.t", &[]), "");
    }

    fn col() -> ColumnDef {
        ColumnDef {
            name: "c".into(),
            type_name: "integer".into(),
            not_null: false,
            default_expr: None,
            identity: String::new(),
            generated: String::new(),
            collation: None,
            comment: None,
        }
    }

    #[test]
    fn column_lines() {
        assert_eq!(column_line(&col()), "c integer");
        let c = ColumnDef { not_null: true, default_expr: Some("0".into()), ..col() };
        assert_eq!(column_line(&c), "c integer NOT NULL DEFAULT 0");
        let c = ColumnDef { not_null: true, identity: "a".into(), ..col() };
        assert_eq!(column_line(&c), "c integer NOT NULL GENERATED ALWAYS AS IDENTITY");
        let c = ColumnDef { identity: "d".into(), ..col() };
        assert_eq!(column_line(&c), "c integer GENERATED BY DEFAULT AS IDENTITY");
        let c = ColumnDef { generated: "s".into(), default_expr: Some("(a * 2)".into()), ..col() };
        assert_eq!(column_line(&c), "c integer GENERATED ALWAYS AS ((a * 2)) STORED");
        let c = ColumnDef { generated: "v".into(), default_expr: Some("(a * 2)".into()), ..col() };
        assert_eq!(column_line(&c), "c integer GENERATED ALWAYS AS ((a * 2)) VIRTUAL");
        let c = ColumnDef { type_name: "text".into(), collation: Some("pg_catalog.\"C\"".into()), ..col() };
        assert_eq!(column_line(&c), "c text COLLATE pg_catalog.\"C\"");
    }

    #[test]
    fn literal_escapes_quotes() {
        assert_eq!(literal("en_US.UTF-8"), "'en_US.UTF-8'");
        assert_eq!(literal("O'Brien"), "'O''Brien'");
    }
}
```

`lib.rs`'e `mod ddl;` ekle.

- [ ] **Step 2: Testlerin başarısız olduğunu doğrula**

Run: `cargo test -p pgt-catalog --lib ddl`
Expected: FAIL — `header`, `AclEntry`, `ColumnDef` tanımlı değil.

- [ ] **Step 3: Yardımcıları uygula** (testlerin üstüne)

```rust
use tokio_postgres::Row;

pub(crate) struct AclEntry {
    pub grantee: String,
    pub privilege: String,
    pub grantable: bool,
}

impl AclEntry {
    pub(crate) fn from_row(r: &Row) -> Self {
        Self { grantee: r.get("grantee"), privilege: r.get("privilege"), grantable: r.get("grantable") }
    }
}

pub(crate) struct ColumnDef {
    pub name: String,
    pub type_name: String,
    pub not_null: bool,
    pub default_expr: Option<String>,
    /// `pg_attribute.attidentity`: '' | 'a' | 'd'
    pub identity: String,
    /// `pg_attribute.attgenerated`: '' | 's' | 'v'
    pub generated: String,
    pub collation: Option<String>,
    /// `quote_literal` uygulanmış yorum
    pub comment: Option<String>,
}

impl ColumnDef {
    pub(crate) fn from_row(r: &Row) -> Self {
        Self {
            name: r.get("name"),
            type_name: r.get("type_name"),
            not_null: r.get("not_null"),
            default_expr: r.get("default_expr"),
            identity: r.get("identity"),
            generated: r.get("generated"),
            collation: r.get("collation"),
            comment: r.get("comment"),
        }
    }
}

const PRIVILEGE_ORDER: [&str; 13] = [
    "SELECT", "INSERT", "UPDATE", "DELETE", "TRUNCATE", "REFERENCES", "TRIGGER", "MAINTAIN", "USAGE", "CREATE", "CONNECT", "TEMPORARY", "EXECUTE",
];

pub(crate) fn header(label: &str, qualified: &str, drop: &str) -> String {
    format!("-- {label}: {qualified}\n\n-- {drop}\n\n")
}

pub(crate) fn owner(object_type: &str, qualified: &str, owner: &str) -> String {
    format!("ALTER {object_type} {qualified}\n    OWNER TO {owner};\n")
}

pub(crate) fn comment(object_type: &str, target: &str, literal: Option<&str>) -> String {
    literal.map(|c| format!("COMMENT ON {object_type} {target}\n    IS {c};\n")).unwrap_or_default()
}

pub(crate) fn grants(object_type: &str, qualified: &str, acl: &[AclEntry]) -> String {
    let mut groups: Vec<((&str, bool), Vec<&str>)> = Vec::new();
    for e in acl {
        let key = (e.grantee.as_str(), e.grantable);
        match groups.iter_mut().find(|(k, _)| *k == key) {
            Some((_, privs)) => privs.push(&e.privilege),
            None => groups.push((key, vec![&e.privilege])),
        }
    }
    let rank = |p: &str| PRIVILEGE_ORDER.iter().position(|o| *o == p).unwrap_or(PRIVILEGE_ORDER.len());
    groups
        .into_iter()
        .map(|((grantee, grantable), mut privs)| {
            privs.sort_by(|a, b| rank(a).cmp(&rank(b)).then(a.cmp(b)));
            privs.dedup();
            let option = if grantable { " WITH GRANT OPTION" } else { "" };
            format!("GRANT {} ON {object_type} {qualified} TO {grantee}{option};\n", privs.join(", "))
        })
        .collect()
}

pub(crate) fn column_line(c: &ColumnDef) -> String {
    let mut line = format!("{} {}", c.name, c.type_name);
    if let Some(collation) = &c.collation {
        line.push_str(&format!(" COLLATE {collation}"));
    }
    if c.not_null {
        line.push_str(" NOT NULL");
    }
    match (c.generated.as_str(), c.identity.as_str(), &c.default_expr) {
        ("s", _, Some(expr)) => line.push_str(&format!(" GENERATED ALWAYS AS ({expr}) STORED")),
        ("v", _, Some(expr)) => line.push_str(&format!(" GENERATED ALWAYS AS ({expr}) VIRTUAL")),
        (_, "a", _) => line.push_str(" GENERATED ALWAYS AS IDENTITY"),
        (_, "d", _) => line.push_str(" GENERATED BY DEFAULT AS IDENTITY"),
        (_, _, Some(expr)) => line.push_str(&format!(" DEFAULT {expr}")),
        _ => {}
    }
    line
}

/// Sunucudan gelmeyen değerler (ör. locale adları) için SQL string literal'i.
pub(crate) fn literal(value: &str) -> String {
    format!("'{}'", value.replace('\'', "''"))
}

/// Boş olmayan blokları aralarında bir boş satırla birleştirir.
pub(crate) fn join_blocks(blocks: Vec<String>) -> String {
    blocks.into_iter().filter(|b| !b.is_empty()).collect::<Vec<_>>().join("\n")
}
```

- [ ] **Step 4: Birim testlerinin geçtiğini doğrula**

Run: `cargo test -p pgt-catalog --lib ddl`
Expected: PASS (4 test).

- [ ] **Step 5: DDL entegrasyon testlerini yaz**

`crates/pgt-catalog/tests/ddl.rs`:

```rust
mod common;

use common::fixtures;
use pgt_catalog::node::{Coll, Seg};
use pgt_core::testing::admin_client;

macro_rules! snapshot {
    ($fx:expr, $name:expr, $ddl:expr) => {{
        let major = $fx.major().await;
        insta::with_settings!({ snapshot_suffix => format!("pg{major}") }, {
            insta::assert_snapshot!($name, $fx.normalize(&$ddl));
        });
    }};
}

#[tokio::test]
async fn table_ddl_snapshots() {
    for fx in fixtures().await {
        for name in ["customers", "orders", "events", "Order Items"] {
            let ddl = fx.catalog.ddl(&fx.table("app", name).await).await.unwrap();
            snapshot!(fx, format!("table_{}", name.replace(' ', "_")), ddl);
        }
        let scratch = fx.catalog.ddl(&fx.table("public", "scratch").await).await.unwrap();
        assert!(scratch.contains("CREATE UNLOGGED TABLE IF NOT EXISTS public.scratch"), "{scratch}");
    }
}

#[tokio::test]
async fn schema_and_column_ddl_snapshots() {
    for fx in fixtures().await {
        let schema = fx.catalog.ddl(&fx.schema("app").await).await.unwrap();
        snapshot!(fx, "schema_app", schema);
        let email = fx.table("app", "customers").await.child(Seg::Column(2));
        snapshot!(fx, "column_customers_email", fx.catalog.ddl(&email).await.unwrap());
    }
}

#[tokio::test]
async fn database_ddl_has_core_clauses() {
    for fx in fixtures().await {
        let ddl = fx.catalog.ddl(&fx.db()).await.unwrap();
        for part in ["-- Database: pgt_fixture", "CREATE DATABASE pgt_fixture", "ENCODING = '", "LC_COLLATE = '", "CONNECTION LIMIT = -1", "IS_TEMPLATE = False;"] {
            assert!(ddl.contains(part), "'{part}' eksik:\n{ddl}");
        }
    }
}

#[tokio::test]
async fn non_object_nodes_have_empty_ddl() {
    for fx in fixtures().await {
        assert_eq!(fx.catalog.ddl(&fx.server_id()).await.unwrap(), "");
        assert_eq!(fx.catalog.ddl(&fx.db().child(Seg::Coll(Coll::Schemas))).await.unwrap(), "");
    }
}

/// Üretilen DDL boş bir veritabanında hatasız çalışmalı ve aynı kolonları üretmeli.
#[tokio::test]
async fn ddl_executes_in_scratch_database() {
    for fx in fixtures().await {
        let admin = admin_client(&fx.ts, &fx.ts.config.maintenance_db).await;
        admin.batch_execute("DROP DATABASE IF EXISTS pgt_scratch").await.unwrap();
        admin.batch_execute("CREATE DATABASE pgt_scratch").await.unwrap();
        let scratch = admin_client(&fx.ts, "pgt_scratch").await;

        let schema_ddl = fx.catalog.ddl(&fx.schema("app").await).await.unwrap();
        scratch.batch_execute(&schema_ddl).await.unwrap_or_else(|e| panic!("{e}\n{schema_ddl}"));
        // bigserial'ın sequence'i Plan 2'deki Sequence DDL'ine kadar elle oluşturulur.
        scratch.batch_execute("CREATE SEQUENCE app.orders_id_seq").await.unwrap();

        for name in ["customers", "orders", "Order Items", "events"] {
            let ddl = fx.catalog.ddl(&fx.table("app", name).await).await.unwrap();
            scratch.batch_execute(&ddl).await.unwrap_or_else(|e| panic!("{name}: {e}\n{ddl}"));
            let expected: Vec<(String, String)> = scratch
                .query(
                    "SELECT a.attname::text, format_type(a.atttypid, a.atttypmod) FROM pg_attribute a \
                     WHERE a.attrelid = $1::text::regclass AND a.attnum > 0 AND NOT a.attisdropped ORDER BY a.attnum",
                    &[&format!("app.\"{name}\"")],
                )
                .await
                .unwrap()
                .iter()
                .map(|r| (r.get(0), r.get(1)))
                .collect();
            let table = fx.table("app", name).await;
            let original = fx.catalog.list_children(&table.child(Seg::Coll(Coll::Columns))).await.unwrap();
            let original: Vec<(String, String)> = original.iter().map(|n| (n.label.clone(), n.detail.clone().unwrap())).collect();
            assert_eq!(expected, original, "{name}");
        }
        drop(scratch);
        admin
            .execute("SELECT pg_terminate_backend(pid) FROM pg_stat_activity WHERE datname = 'pgt_scratch' AND pid <> pg_backend_pid()", &[])
            .await
            .unwrap();
        admin.batch_execute("DROP DATABASE pgt_scratch").await.unwrap();
    }
}
```

> `$1::text::regclass`: parametre `String` olarak gönderilir; doğrudan `$1::regclass` yazılırsa tokio-postgres parametre tipini `regclass` çıkarır ve `String`'i reddeder.

- [ ] **Step 6: Testlerin başarısız olduğunu doğrula**

Run: `cargo test -p pgt-catalog --test ddl`
Expected: FAIL — `ddl` metodu yok.

- [ ] **Step 7: SQL dosyalarını yaz** (`crates/pgt-catalog/src/sql/`)

`database_ddl.sql`:

```sql
SELECT quote_ident(d.datname) AS qname, quote_ident(pg_get_userbyid(d.datdba)) AS owner,
       pg_encoding_to_char(d.encoding)::text AS encoding, d.datcollate::text AS collate, d.datctype::text AS ctype,
       quote_ident(t.spcname) AS tablespace, d.datconnlimit AS conn_limit, d.datistemplate AS is_template,
       quote_literal(shobj_description(d.oid, 'pg_database')) AS comment
FROM pg_database d
JOIN pg_tablespace t ON t.oid = d.dattablespace
WHERE d.oid = $1
```

`schema_ddl.sql`:

```sql
SELECT quote_ident(n.nspname) AS qname, quote_ident(pg_get_userbyid(n.nspowner)) AS owner,
       quote_literal(obj_description(n.oid, 'pg_namespace')) AS comment
FROM pg_namespace n
WHERE n.oid = $1
```

`table_ddl_head.sql`:

```sql
SELECT quote_ident(n.nspname) || '.' || quote_ident(c.relname) AS qname,
       quote_ident(pg_get_userbyid(c.relowner)) AS owner,
       CASE WHEN c.relkind = 'p' THEN pg_get_partkeydef(c.oid) END AS partkey,
       c.relpersistence = 'u' AS unlogged,
       array_to_string(c.reloptions, ', ') AS options,
       quote_ident(t.spcname) AS tablespace,
       quote_literal(obj_description(c.oid, 'pg_class')) AS comment
FROM pg_class c
JOIN pg_namespace n ON n.oid = c.relnamespace
LEFT JOIN pg_tablespace t ON t.oid = c.reltablespace
WHERE c.oid = $1 AND c.relkind IN ('r', 'p')
```

`table_ddl_columns.sql` (PG 12+; `$2` NULL ise tüm kolonlar):

```sql
SELECT quote_ident(a.attname) AS name, format_type(a.atttypid, a.atttypmod) AS type_name, a.attnotnull AS not_null,
       pg_get_expr(d.adbin, d.adrelid) AS default_expr,
       a.attidentity::text AS identity, a.attgenerated::text AS generated,
       CASE WHEN a.attcollation <> 0 AND a.attcollation <> t.typcollation
            THEN quote_ident(cn.nspname) || '.' || quote_ident(co.collname) END AS collation,
       quote_literal(col_description(a.attrelid, a.attnum)) AS comment
FROM pg_attribute a
JOIN pg_type t ON t.oid = a.atttypid
LEFT JOIN pg_attrdef d ON d.adrelid = a.attrelid AND d.adnum = a.attnum
LEFT JOIN pg_collation co ON co.oid = a.attcollation
LEFT JOIN pg_namespace cn ON cn.oid = co.collnamespace
WHERE a.attrelid = $1 AND a.attnum > 0 AND NOT a.attisdropped
  AND ($2::int2 IS NULL OR a.attnum = $2::int2)
ORDER BY a.attnum
```

`table_ddl_columns.pg10.sql` (PG 10–11): yukarıdakiyle aynı, yalnızca `a.attgenerated::text AS generated` yerine `''::text AS generated`.

`table_ddl_constraints.sql`:

```sql
SELECT quote_ident(k.conname) AS name, pg_get_constraintdef(k.oid, true) AS def
FROM pg_constraint k
WHERE k.conrelid = $1 AND k.contype IN ('p', 'u', 'c', 'x', 'f')
ORDER BY CASE k.contype WHEN 'p' THEN 0 WHEN 'u' THEN 1 WHEN 'c' THEN 2 WHEN 'x' THEN 3 ELSE 4 END, k.conname COLLATE "C"
```

`acl_database.sql`, `acl_namespace.sql`, `acl_relation.sql` — aynı kalıp, yalnızca kaynak değişir:

```sql
-- acl_database.sql
SELECT CASE WHEN a.grantee = 0 THEN 'PUBLIC' ELSE quote_ident(pg_get_userbyid(a.grantee)) END AS grantee,
       a.privilege_type AS privilege, a.is_grantable AS grantable
FROM pg_database o, LATERAL aclexplode(o.datacl) a
WHERE o.oid = $1
ORDER BY 1 COLLATE "C", 2
```

```sql
-- acl_namespace.sql
SELECT CASE WHEN a.grantee = 0 THEN 'PUBLIC' ELSE quote_ident(pg_get_userbyid(a.grantee)) END AS grantee,
       a.privilege_type AS privilege, a.is_grantable AS grantable
FROM pg_namespace o, LATERAL aclexplode(o.nspacl) a
WHERE o.oid = $1
ORDER BY 1 COLLATE "C", 2
```

```sql
-- acl_relation.sql
SELECT CASE WHEN a.grantee = 0 THEN 'PUBLIC' ELSE quote_ident(pg_get_userbyid(a.grantee)) END AS grantee,
       a.privilege_type AS privilege, a.is_grantable AS grantable
FROM pg_class o, LATERAL aclexplode(o.relacl) a
WHERE o.oid = $1
ORDER BY 1 COLLATE "C", 2
```

(Dosyanın ilk satırındaki `-- acl_...` yorumu dosya adını gösterir; SQL'in parçası olarak kalabilir.)

- [ ] **Step 8: Dağıtıcıyı ve tür modüllerini genişlet**

`lib.rs` `impl Catalog` içine:

```rust
    pub async fn ddl(&self, id: &NodeId) -> Result<String> {
        kinds::ddl(self, id).await
    }
```

`kinds/mod.rs` sonuna:

```rust
pub(crate) async fn ddl(cat: &Catalog, id: &NodeId) -> Result<String> {
    let (server, path) = match id {
        NodeId::Root | NodeId::Group(_) => return Ok(String::new()),
        NodeId::Node { server, path } => (*server, path.as_slice()),
    };
    if path.is_empty() || matches!(path.last(), Some(Seg::Coll(_))) {
        return Ok(String::new());
    }
    if !cat.mgr.is_connected(server) {
        return Err(Error::NotConnected);
    }
    match path {
        [Seg::Db(db)] => database::ddl(cat, server, *db).await,
        [Seg::Db(db), Seg::Schema(sch)] => schema::ddl(cat, server, *db, *sch).await,
        [Seg::Db(db), Seg::Schema(_), Seg::Table(tbl)] => table::ddl(cat, server, *db, *tbl).await,
        [Seg::Db(db), Seg::Schema(_), Seg::Table(tbl), Seg::Column(col)] => column::ddl(cat, server, *db, *tbl, *col).await,
        _ => Err(invalid(id)),
    }
}

/// Bir nesnenin ACL satırlarını okur (`acl_*.sql`).
pub(crate) async fn acl(cat: &Catalog, server: uuid::Uuid, target: pgt_core::connection::Target, sql: &'static crate::version::VersionedSql, oid: u32) -> Result<Vec<crate::ddl::AclEntry>> {
    let rows = crate::db::query(&cat.mgr, server, target, sql, crate::db::p![oid]).await?;
    Ok(rows.iter().map(crate::ddl::AclEntry::from_row).collect())
}
```

`kinds/database.rs` sonuna:

```rust
static DDL: VersionedSql = VersionedSql(&[(10, include_str!("../sql/database_ddl.sql"))]);
static ACL: VersionedSql = VersionedSql(&[(10, include_str!("../sql/acl_database.sql"))]);

pub(crate) async fn ddl(cat: &Catalog, server: Uuid, db: u32) -> Result<String> {
    use crate::ddl::{comment, grants, header, join_blocks, literal};
    let r = crate::db::query_one(&cat.mgr, server, Target::Maintenance, &DDL, p![db], "veritabanı").await?;
    let q: String = r.get("qname");
    let create = format!(
        "CREATE DATABASE {q}\n    WITH\n    OWNER = {}\n    ENCODING = {}\n    LC_COLLATE = {}\n    LC_CTYPE = {}\n    TABLESPACE = {}\n    CONNECTION LIMIT = {}\n    IS_TEMPLATE = {};\n",
        r.get::<_, String>("owner"),
        literal(&r.get::<_, String>("encoding")),
        literal(&r.get::<_, String>("collate")),
        literal(&r.get::<_, String>("ctype")),
        r.get::<_, String>("tablespace"),
        r.get::<_, i32>("conn_limit"),
        if r.get::<_, bool>("is_template") { "True" } else { "False" },
    );
    let acl = super::acl(cat, server, Target::Maintenance, &ACL, db).await?;
    Ok(header("Database", &q, &format!("DROP DATABASE IF EXISTS {q};"))
        + &join_blocks(vec![create, comment("DATABASE", &q, r.get::<_, Option<String>>("comment").as_deref()), grants("DATABASE", &q, &acl)]))
}
```

`kinds/schema.rs` sonuna:

```rust
static DDL: VersionedSql = VersionedSql(&[(10, include_str!("../sql/schema_ddl.sql"))]);
static ACL: VersionedSql = VersionedSql(&[(10, include_str!("../sql/acl_namespace.sql"))]);

pub(crate) async fn ddl(cat: &Catalog, server: Uuid, db: u32, schema: u32) -> Result<String> {
    use crate::ddl::{comment, grants, header, join_blocks};
    let r = crate::db::query_one(&cat.mgr, server, Target::Database(db), &DDL, p![schema], "şema").await?;
    let q: String = r.get("qname");
    let create = format!("CREATE SCHEMA IF NOT EXISTS {q}\n    AUTHORIZATION {};\n", r.get::<_, String>("owner"));
    let acl = super::acl(cat, server, Target::Database(db), &ACL, schema).await?;
    Ok(header("SCHEMA", &q, &format!("DROP SCHEMA IF EXISTS {q} ;"))
        + &join_blocks(vec![create, comment("SCHEMA", &q, r.get::<_, Option<String>>("comment").as_deref()), grants("SCHEMA", &q, &acl)]))
}
```

`kinds/table.rs` sonuna:

```rust
static HEAD: VersionedSql = VersionedSql(&[(10, include_str!("../sql/table_ddl_head.sql"))]);
pub(crate) static COLUMNS: VersionedSql = VersionedSql(&[
    (10, include_str!("../sql/table_ddl_columns.pg10.sql")),
    (12, include_str!("../sql/table_ddl_columns.sql")),
]);
static CONSTRAINTS: VersionedSql = VersionedSql(&[(10, include_str!("../sql/table_ddl_constraints.sql"))]);
static ACL: VersionedSql = VersionedSql(&[(10, include_str!("../sql/acl_relation.sql"))]);

pub(crate) struct TableHead {
    pub qname: String,
    pub owner: String,
    pub partkey: Option<String>,
    pub unlogged: bool,
    pub options: Option<String>,
    pub tablespace: Option<String>,
    pub comment: Option<String>,
}

pub(crate) async fn head(cat: &Catalog, server: Uuid, db: u32, table: u32) -> Result<TableHead> {
    let r = crate::db::query_one(&cat.mgr, server, Target::Database(db), &HEAD, p![table], "tablo").await?;
    Ok(TableHead {
        qname: r.get("qname"),
        owner: r.get("owner"),
        partkey: r.get("partkey"),
        unlogged: r.get("unlogged"),
        options: r.get("options"),
        tablespace: r.get("tablespace"),
        comment: r.get("comment"),
    })
}

pub(crate) async fn ddl(cat: &Catalog, server: Uuid, db: u32, table: u32) -> Result<String> {
    use crate::ddl::{ColumnDef, column_line, comment, grants, header, join_blocks, owner};
    let h = head(cat, server, db, table).await?;
    let target = Target::Database(db);
    let cols: Vec<ColumnDef> = crate::db::query(&cat.mgr, server, target, &COLUMNS, p![table, None::<i16>])
        .await?
        .iter()
        .map(ColumnDef::from_row)
        .collect();
    let constraints = crate::db::query(&cat.mgr, server, target, &CONSTRAINTS, p![table]).await?;
    let acl = super::acl(cat, server, target, &ACL, table).await?;

    let mut lines: Vec<String> = cols.iter().map(column_line).collect();
    lines.extend(constraints.iter().map(|k| format!("CONSTRAINT {} {}", k.get::<_, String>("name"), k.get::<_, String>("def"))));
    let body = lines.iter().map(|l| format!("    {l}")).collect::<Vec<_>>().join(",\n");
    let mut create = format!(
        "CREATE {}TABLE IF NOT EXISTS {}\n(\n{body}\n)",
        if h.unlogged { "UNLOGGED " } else { "" },
        h.qname
    );
    if let Some(key) = &h.partkey {
        create.push_str(&format!("\nPARTITION BY {key}"));
    }
    if let Some(options) = h.options.as_deref().filter(|o| !o.is_empty()) {
        create.push_str(&format!("\nWITH ({options})"));
    }
    if let Some(ts) = &h.tablespace {
        create.push_str(&format!("\nTABLESPACE {ts}"));
    }
    create.push_str(";\n");

    let mut blocks = vec![create, owner("TABLE IF EXISTS", &h.qname, &h.owner), comment("TABLE", &h.qname, h.comment.as_deref())];
    for c in &cols {
        blocks.push(comment("COLUMN", &format!("{}.{}", h.qname, c.name), c.comment.as_deref()));
    }
    blocks.push(grants("TABLE", &h.qname, &acl));
    Ok(header("Table", &h.qname, &format!("DROP TABLE IF EXISTS {};", h.qname)) + &join_blocks(blocks))
}
```

`kinds/column.rs` sonuna:

```rust
pub(crate) async fn ddl(cat: &Catalog, server: Uuid, db: u32, table: u32, attnum: i16) -> Result<String> {
    use crate::ddl::{ColumnDef, column_line, comment, header, join_blocks};
    let h = super::table::head(cat, server, db, table).await?;
    let row = crate::db::query_one(&cat.mgr, server, Target::Database(db), &super::table::COLUMNS, p![table, Some(attnum)], "kolon").await?;
    let c = ColumnDef::from_row(&row);
    let target = format!("{}.{}", h.qname, c.name);
    let add = format!("ALTER TABLE IF EXISTS {}\n    ADD COLUMN {};\n", h.qname, column_line(&c));
    Ok(header("Column", &target, &format!("ALTER TABLE IF EXISTS {} DROP COLUMN IF EXISTS {};", h.qname, c.name))
        + &join_blocks(vec![add, comment("COLUMN", &target, c.comment.as_deref())]))
}
```

- [ ] **Step 9: Testleri çalıştır ve snapshot'ları gözden geçir**

Run: `cargo test -p pgt-catalog --test ddl` (PG 18 ile)
Expected: `table_ddl_snapshots` ve `schema_and_column_ddl_snapshots` yeni snapshot oluşturduğu için FAIL; diğerleri PASS.

Run: `cargo insta review` (gerekirse önce `cargo install cargo-insta`)
Her snapshot'ı yukarıdaki "Çıktı biçimi" örneğine göre kontrol edin: tırnaklı adlar (`app."Order Items"`, `"Line No"`, `"Ürün Adı"`), `PARTITION BY RANGE (happened_on)`, `WITH (fillfactor=80)`, `OWNER TO <owner>;`, sıralı GRANT'lar. Doğruysa kabul edin.

Run: `cargo test -p pgt-catalog`
Expected: PASS (tüm testler; `ddl_executes_in_scratch_database` dahil).

- [ ] **Step 10: Commit**

```bash
git add crates/pgt-catalog
git commit -m "feat(catalog): Database/Schema/Table/Column DDL üretimi ve snapshot testleri"
```

---

### Task 9: Ön yüz iskeleti (Vite + SolidJS + Tailwind 4), tema ve bölünmüş düzen

**Files:**
- Create: `package.json`, `tsconfig.json`, `vite.config.ts`, `vitest.config.ts`, `index.html`, `src/main.tsx`, `src/app/App.tsx`, `src/app/styles.css`, `src/app/theme.ts`, `src/app/theme.test.ts`, `src/app/storage.ts`, `src/app/strings.ts`, `src/app/split.ts`, `src/app/split.test.ts`, `src/app/SplitPane.tsx`

**Interfaces:**
- Produces:
  - `theme.ts`: `type ThemePreference = "system" | "light" | "dark"`, `resolveTheme(pref, systemDark: boolean): "light" | "dark"`, `nextPreference(pref): ThemePreference`, `loadThemePreference()`, `saveThemePreference(pref)`, `applyTheme(pref)`, `watchSystemTheme(getPref): () => void`
  - `storage.ts`: `readStorage(key): string | null`, `writeStorage(key, value): void` (hata yutulur)
  - `strings.ts`: `t` nesnesi (tüm arayüz metinleri; aşağıdaki içerik sonraki task'larda kullanılır)
  - `SplitPane(props: { left: JSX.Element; right: JSX.Element })`, `clampWidth(width, total): number`
  - `App.tsx`: düzen kabuğu (`header` araç çubuğu yuvası, `SplitPane`, `footer` durum çubuğu). Task 12–14 doldurur.

- [ ] **Step 1: Paketleri kur**

`package.json`:

```json
{
  "name": "pg-tools",
  "private": true,
  "version": "0.1.0",
  "type": "module",
  "scripts": {
    "dev": "vite",
    "build": "vite build",
    "typecheck": "tsc --noEmit",
    "test": "vitest run",
    "tauri": "tauri"
  }
}
```

```bash
bun add solid-js @kobalte/core @tanstack/solid-virtual @tauri-apps/api @tauri-apps/plugin-dialog lucide-solid @codemirror/state @codemirror/view @codemirror/language @codemirror/lang-sql @lezer/highlight
bun add -d vite vite-plugin-solid tailwindcss @tailwindcss/vite typescript vitest @tauri-apps/cli
```

`tsconfig.json`:

```json
{
  "compilerOptions": {
    "target": "ES2022",
    "module": "ESNext",
    "moduleResolution": "bundler",
    "lib": ["ES2022", "DOM", "DOM.Iterable"],
    "jsx": "preserve",
    "jsxImportSource": "solid-js",
    "strict": true,
    "noEmit": true,
    "skipLibCheck": true,
    "isolatedModules": true,
    "types": ["vite/client"]
  },
  "include": ["src"]
}
```

`vite.config.ts`:

```ts
import { defineConfig } from "vite";
import solid from "vite-plugin-solid";
import tailwindcss from "@tailwindcss/vite";

export default defineConfig({
  plugins: [solid(), tailwindcss()],
  clearScreen: false,
  server: {
    port: 1420,
    strictPort: true,
    watch: { ignored: ["**/src-tauri/**", "**/target/**", "**/crates/**"] },
  },
  build: {
    target: "chrome120",
    modulePreload: { polyfill: false },
  },
});
```

`vitest.config.ts`:

```ts
import { defineConfig } from "vitest/config";

export default defineConfig({
  test: { environment: "node", include: ["src/**/*.test.ts"] },
});
```

`index.html`:

```html
<!doctype html>
<html lang="tr">
  <head>
    <meta charset="UTF-8" />
    <meta name="viewport" content="width=device-width, initial-scale=1.0" />
    <title>pg-tools</title>
  </head>
  <body>
    <div id="root"></div>
    <script type="module" src="/src/main.tsx"></script>
  </body>
</html>
```

- [ ] **Step 2: Başarısız testleri yaz**

`src/app/theme.test.ts`:

```ts
import { describe, expect, it } from "vitest";
import { nextPreference, resolveTheme } from "./theme";

describe("theme", () => {
  it("resolves system preference from media query", () => {
    expect(resolveTheme("system", true)).toBe("dark");
    expect(resolveTheme("system", false)).toBe("light");
    expect(resolveTheme("light", true)).toBe("light");
    expect(resolveTheme("dark", false)).toBe("dark");
  });

  it("cycles system → light → dark → system", () => {
    expect(nextPreference("system")).toBe("light");
    expect(nextPreference("light")).toBe("dark");
    expect(nextPreference("dark")).toBe("system");
  });
});
```

`src/app/split.test.ts`:

```ts
import { describe, expect, it } from "vitest";
import { clampWidth } from "./split";

describe("clampWidth", () => {
  it("keeps the explorer between 180px and total-240px", () => {
    expect(clampWidth(100, 1200)).toBe(180);
    expect(clampWidth(500, 1200)).toBe(500);
    expect(clampWidth(1100, 1200)).toBe(960);
    expect(clampWidth(300, 300)).toBe(180);
  });
});
```

- [ ] **Step 3: Testlerin başarısız olduğunu doğrula**

Run: `bun run test`
Expected: FAIL — `./theme` ve `./split` modülleri yok.

- [ ] **Step 4: Uygula**

`src/app/storage.ts`:

```ts
/** localStorage erişimi; gizli pencere vb. durumlarda hata yutulur. */
export function readStorage(key: string): string | null {
  try {
    return localStorage.getItem(key);
  } catch {
    return null;
  }
}

export function writeStorage(key: string, value: string): void {
  try {
    localStorage.setItem(key, value);
  } catch {
    // yoksay
  }
}
```

`src/app/theme.ts`:

```ts
import { readStorage, writeStorage } from "./storage";

export type ThemePreference = "system" | "light" | "dark";
const KEY = "pgt.theme";

export function resolveTheme(pref: ThemePreference, systemDark: boolean): "light" | "dark" {
  return pref === "system" ? (systemDark ? "dark" : "light") : pref;
}

export function nextPreference(pref: ThemePreference): ThemePreference {
  return pref === "system" ? "light" : pref === "light" ? "dark" : "system";
}

export function loadThemePreference(): ThemePreference {
  const value = readStorage(KEY);
  return value === "light" || value === "dark" ? value : "system";
}

export function saveThemePreference(pref: ThemePreference): void {
  writeStorage(KEY, pref);
}

const media = () => window.matchMedia("(prefers-color-scheme: dark)");

export function applyTheme(pref: ThemePreference): void {
  document.documentElement.dataset.theme = resolveTheme(pref, media().matches);
}

/** Sistem teması değişince (yalnızca "system" tercihinde etkili) yeniden uygular. */
export function watchSystemTheme(getPref: () => ThemePreference): () => void {
  const m = media();
  const handler = () => applyTheme(getPref());
  m.addEventListener("change", handler);
  return () => m.removeEventListener("change", handler);
}
```

`src/app/split.ts`:

```ts
export const MIN_LEFT = 180;
export const MIN_RIGHT = 240;

export function clampWidth(width: number, total: number): number {
  return Math.min(Math.max(width, MIN_LEFT), Math.max(MIN_LEFT, total - MIN_RIGHT));
}
```

`src/app/SplitPane.tsx`:

```tsx
import { createSignal, type JSX } from "solid-js";
import { clampWidth } from "./split";
import { readStorage, writeStorage } from "./storage";

const KEY = "pgt.split.left";

export function SplitPane(props: { left: JSX.Element; right: JSX.Element }) {
  const [width, setWidth] = createSignal(Number(readStorage(KEY)) || 320);
  let container!: HTMLDivElement;

  const onPointerDown = (e: PointerEvent) => {
    e.preventDefault();
    const handle = e.currentTarget as HTMLElement;
    handle.setPointerCapture(e.pointerId);
    const move = (ev: PointerEvent) =>
      setWidth(clampWidth(ev.clientX - container.getBoundingClientRect().left, container.clientWidth));
    const up = () => {
      handle.removeEventListener("pointermove", move);
      handle.removeEventListener("pointerup", up);
      writeStorage(KEY, String(width()));
    };
    handle.addEventListener("pointermove", move);
    handle.addEventListener("pointerup", up);
  };

  return (
    <div ref={container} class="flex min-h-0 flex-1">
      <div class="min-w-0 shrink-0 border-r border-border bg-panel" style={{ width: `${width()}px` }}>
        {props.left}
      </div>
      <div
        role="separator"
        aria-orientation="vertical"
        class="w-1 shrink-0 cursor-col-resize hover:bg-accent/40"
        onPointerDown={onPointerDown}
      />
      <div class="min-w-0 flex-1">{props.right}</div>
    </div>
  );
}
```


`src/app/styles.css`:

```css
@import "tailwindcss";

@custom-variant dark (&:where([data-theme="dark"], [data-theme="dark"] *));

:root {
  --bg: #fbfbfc;
  --panel: #f3f4f6;
  --fg: #1f2328;
  --muted: #6b7280;
  --border: #e2e4e8;
  --accent: #2563eb;
  --accent-fg: #ffffff;
  --selected: #dbe6fd;
  --hover: #eceef2;
  --danger: #c62828;
  --syn-keyword: #7c3aed;
  --syn-string: #0f766e;
  --syn-number: #b45309;
  --syn-comment: #6b7280;
  --syn-type: #1d4ed8;
  color-scheme: light;
}

[data-theme="dark"] {
  --bg: #16181d;
  --panel: #1c1f26;
  --fg: #e6e8eb;
  --muted: #9aa1ac;
  --border: #2b2f38;
  --accent: #4c8dff;
  --accent-fg: #0b1020;
  --selected: #22324f;
  --hover: #232731;
  --danger: #ef5350;
  --syn-keyword: #c4a7ff;
  --syn-string: #5eead4;
  --syn-number: #fbbf24;
  --syn-comment: #8b93a1;
  --syn-type: #93c5fd;
  color-scheme: dark;
}

@theme inline {
  --font-sans: "Segoe UI Variable Text", "Segoe UI", system-ui, sans-serif;
  --font-mono: "Cascadia Mono", Consolas, monospace;
  --color-bg: var(--bg);
  --color-panel: var(--panel);
  --color-fg: var(--fg);
  --color-muted: var(--muted);
  --color-border: var(--border);
  --color-accent: var(--accent);
  --color-accent-fg: var(--accent-fg);
  --color-selected: var(--selected);
  --color-hover: var(--hover);
  --color-danger: var(--danger);
}

html,
body,
#root {
  height: 100%;
  margin: 0;
}

body {
  background: var(--bg);
  color: var(--fg);
  font-family: var(--font-sans);
  font-size: 13px;
  overflow: hidden;
}
```

`src/app/strings.ts` (tüm plan boyunca kullanılan metinler):

```ts
export const t = {
  toolbar: {
    addServer: "Sunucu ekle",
    editServer: "Sunucuyu düzenle",
    refresh: "Yenile (F5)",
    theme: { system: "Tema: sistem", light: "Tema: açık", dark: "Tema: koyu" },
  },
  explorer: {
    ariaLabel: "Nesne gezgini",
    empty: "Henüz sunucu yok. Başlamak için “Sunucu ekle”ye tıklayın.",
  },
  inspector: {
    properties: "Properties",
    ddl: "DDL",
    nothingSelected: "Ayrıntıları görmek için ağaçtan bir nesne seçin.",
    noProperties: "Bu düğüm için özellik yok.",
    noDdl: "Bu düğüm için DDL yok.",
    loading: "Yükleniyor…",
    copy: "Kopyala",
    copied: "Kopyalandı",
  },
  server: {
    titleNew: "Yeni sunucu",
    titleEdit: "Sunucuyu düzenle",
    tabs: { general: "Genel", connection: "Bağlantı", ssl: "SSL" },
    name: "Ad",
    group: "Grup",
    host: "Host",
    port: "Port",
    maintenanceDb: "Bakım veritabanı",
    username: "Kullanıcı",
    password: "Parola",
    passwordKeep: "Değiştirmek istemiyorsanız boş bırakın",
    savePassword: "Parolayı kaydet",
    sslMode: "SSL modu",
    rootCert: "CA sertifikası",
    clientCert: "İstemci sertifikası",
    clientKey: "İstemci anahtarı",
    browse: "Gözat…",
    test: "Bağlantıyı test et",
    testing: "Test ediliyor…",
    testOk: (version: string) => `Bağlantı başarılı (PostgreSQL ${version})`,
    save: "Kaydet",
    cancel: "Vazgeç",
    close: "Kapat",
  },
  password: {
    title: (server: string) => `${server} için parola`,
    retry: "Parola hatalı, tekrar deneyin.",
    connect: "Bağlan",
  },
  status: {
    ready: "Hazır",
    connected: "bağlı",
    notConnected: "bağlı değil",
  },
} as const;
```

`src/app/App.tsx` (iskelet; sonraki task'lar yuvaları doldurur):

```tsx
import { SplitPane } from "./SplitPane";

export default function App() {
  return (
    <div class="flex h-full flex-col">
      <header class="flex h-9 shrink-0 items-center gap-1 border-b border-border bg-panel px-2" />
      <SplitPane left={<div class="h-full" />} right={<div class="h-full" />} />
      <footer class="flex h-6 shrink-0 items-center border-t border-border bg-panel px-2 text-xs text-muted" />
    </div>
  );
}
```

`src/main.tsx`:

```tsx
import { render } from "solid-js/web";
import "./app/styles.css";
import App from "./app/App";
import { applyTheme, loadThemePreference } from "./app/theme";

applyTheme(loadThemePreference());
render(() => <App />, document.getElementById("root")!);
```

- [ ] **Step 5: Test, tip kontrolü ve derlemeyi doğrula**

Run: `bun run test`
Expected: PASS (2 dosya, 3 test).

Run: `bun run typecheck`
Expected: hata yok.

Run: `bun run build`
Expected: `dist/` oluşur; çıktıdaki JS paketi (gzip) ~15 kB altında.

- [ ] **Step 6: Commit**

```bash
git add package.json bun.lock tsconfig.json vite.config.ts vitest.config.ts index.html src/main.tsx src/app
git commit -m "feat(ui): Vite/Solid/Tailwind iskeleti, tema ve bölünmüş düzen"
```

---

### Task 10: Tauri kabuğu (durum, komutlar, loglama, boşta bağlantı temizliği)

**Files:**
- Create: `src-tauri/Cargo.toml`, `src-tauri/build.rs`, `src-tauri/tauri.conf.json`, `src-tauri/capabilities/default.json`, `src-tauri/src/main.rs`, `src-tauri/src/state.rs`, `src-tauri/src/commands.rs`, `src-tauri/src/logging.rs`, `app-icon.svg`, `src-tauri/icons/*` (üretilir)
- Modify: `Cargo.toml` (workspace `members`'a `src-tauri` ekle)

**Interfaces:**
- Consumes: `pgt_core::{registry::Registry, credentials::*, connection::{ConnectionManager, ServerInfo}, config::ServerConfig, Error, ErrorPayload}`, `pgt_catalog::{Catalog, node::{NodeId, TreeNode}, props::PropertySection}`
- Produces (Tauri komutları; JS'te argüman adları camelCase):

| Komut | JS argümanları | Dönüş |
|---|---|---|
| `list_servers` | — | `ServerConfig[]` |
| `save_server` | `{ config: ServerConfig, password: string \| null }` | `ServerConfig` |
| `test_connection` | `{ config: ServerConfig, password: string \| null }` | `ServerInfo` |
| `connect_server` | `{ serverId: string, password: string \| null, remember: boolean }` | `ServerInfo` |
| `disconnect_server` | `{ serverId: string }` | `null` |
| `list_children` | `{ id: string }` | `TreeNode[]` |
| `get_properties` | `{ id: string }` | `PropertySection[]` |
| `get_ddl` | `{ id: string }` | `string` |
| `startup_notice` | — | `string \| null` |

Hatalar her zaman `ErrorPayload` olarak reddedilir.

**Parola kuralları:**
- `save_server`: `password` verildi + `savePassword` → kalıcı kaydet; `savePassword=false` → depodaki parolayı sil; `password=null` + `savePassword` → mevcut kaydı koru. Kayıt sonrası sunucu bağlıysa bağlantı kesilir (ayarlar değişmiş olabilir).
- `connect_server`: `password` verilmemişse önce oturum, sonra Credential Manager denenir. `password` verildiyse başarıdan sonra `remember` true ise kalıcı kaydedilir ve yapılandırmada `savePassword=true` yapılır; false ise yalnızca oturumda tutulur.

- [ ] **Step 1: Kabuk dosyalarını oluştur**

Kök `Cargo.toml`: `members = ["crates/pgt-core", "crates/pgt-catalog", "src-tauri"]`.

`src-tauri/Cargo.toml`:

```toml
[package]
name = "pg-tools"
version.workspace = true
edition.workspace = true
rust-version.workspace = true

[build-dependencies]
tauri-build = { version = "2.7", features = [] }

[dependencies]
pgt-core.workspace = true
pgt-catalog.workspace = true
tauri = { version = "2.12", features = [] }
tauri-plugin-dialog = "2.8"
tokio.workspace = true
uuid.workspace = true
serde.workspace = true
tracing.workspace = true
tracing-subscriber = { version = "0.3", features = ["env-filter"] }
tracing-appender = "0.2"

[dev-dependencies]
tempfile.workspace = true
```

`src-tauri/build.rs`:

```rust
fn main() {
    tauri_build::build()
}
```

`src-tauri/tauri.conf.json`:

```json
{
  "$schema": "https://schema.tauri.app/config/2",
  "productName": "pg-tools",
  "version": "0.1.0",
  "identifier": "com.pgtools.desktop",
  "build": {
    "beforeDevCommand": "bun run dev",
    "devUrl": "http://localhost:1420",
    "beforeBuildCommand": "bun run build",
    "frontendDist": "../dist"
  },
  "app": {
    "windows": [
      { "label": "main", "title": "pg-tools", "width": 1280, "height": 800, "minWidth": 800, "minHeight": 500 }
    ],
    "security": {
      "csp": "default-src 'self' ipc: http://ipc.localhost; style-src 'self' 'unsafe-inline'; img-src 'self' data:"
    }
  },
  "bundle": {
    "active": true,
    "targets": ["nsis"],
    "icon": ["icons/32x32.png", "icons/128x128.png", "icons/128x128@2x.png", "icons/icon.ico"]
  }
}
```

`src-tauri/capabilities/default.json`:

```json
{
  "$schema": "../gen/schemas/desktop-schema.json",
  "identifier": "default",
  "description": "Ana pencere yetkileri",
  "windows": ["main"],
  "permissions": ["core:default", "dialog:allow-open"]
}
```

`app-icon.svg` (kök dizin):

```svg
<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 512 512">
  <rect width="512" height="512" rx="96" fill="#2563eb"/>
  <ellipse cx="256" cy="150" rx="150" ry="52" fill="#ffffff"/>
  <path d="M106 150v212c0 29 67 52 150 52s150-23 150-52V150c0 29-67 52-150 52s-150-23-150-52z" fill="#dbe6fd"/>
  <text x="256" y="340" text-anchor="middle" font-family="Segoe UI, sans-serif" font-size="120" font-weight="700" fill="#1d4ed8">pg</text>
</svg>
```

Run: `bunx tauri icon app-icon.svg`
Expected: `src-tauri/icons/` altında `icon.ico`, `32x32.png`, `128x128.png`, `128x128@2x.png` vb. oluşur.

- [ ] **Step 2: Durum için başarısız testi yaz**

`src-tauri/src/state.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use pgt_core::credentials::MemorySecretStore;

    #[test]
    fn startup_notice_after_corrupt_registry() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("servers.json"), "bozuk").unwrap();
        let state = AppState::with_store(dir.path(), Arc::new(MemorySecretStore::default())).unwrap();
        let notice = state.startup_notice.clone().expect("uyarı bekleniyordu");
        assert!(notice.contains("servers.json.bak-"), "{notice}");
        assert!(state.registry.read().unwrap().servers().is_empty());
    }

    #[test]
    fn clean_start_has_no_notice() {
        let dir = tempfile::tempdir().unwrap();
        let state = AppState::with_store(dir.path(), Arc::new(MemorySecretStore::default())).unwrap();
        assert!(state.startup_notice.is_none());
    }
}
```

`src-tauri/src/main.rs` (geçici; Step 4'te tamamlanır):

```rust
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod state;

fn main() {}
```

- [ ] **Step 3: Testin başarısız olduğunu doğrula**

Run: `cargo test -p pg-tools`
Expected: FAIL — `AppState` tanımlı değil. (`frontendDist` için `dist/` gerekir; yoksa önce `bun run build`.)

- [ ] **Step 4: Durumu, komutları, loglamayı ve `main`'i yaz**

`src-tauri/src/state.rs` (testlerin üstüne):

```rust
use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};

use pgt_catalog::Catalog;
use pgt_core::connection::ConnectionManager;
use pgt_core::credentials::{Credentials, SecretStore};
use pgt_core::registry::Registry;

pub struct AppState {
    pub registry: Arc<RwLock<Registry>>,
    pub credentials: Credentials,
    pub manager: Arc<ConnectionManager>,
    pub catalog: Catalog,
    pub startup_notice: Option<String>,
}

impl AppState {
    pub fn with_store(config_dir: &Path, store: Arc<dyn SecretStore>) -> pgt_core::Result<Self> {
        let outcome = Registry::load(config_dir.join("servers.json"))?;
        let startup_notice = outcome.recovered_backup.map(|b| {
            format!("servers.json okunamadı; {} olarak yedeklendi ve boş sunucu listesiyle başlandı.", b.display())
        });
        let registry = Arc::new(RwLock::new(outcome.registry));
        let manager = Arc::new(ConnectionManager::new());
        Ok(Self {
            catalog: Catalog::new(manager.clone(), registry.clone()),
            credentials: Credentials::new(store),
            registry,
            manager,
            startup_notice,
        })
    }

    pub fn init() -> pgt_core::Result<Self> {
        #[cfg(windows)]
        let store: Arc<dyn SecretStore> = Arc::new(pgt_core::credentials::WindowsSecretStore::new()?);
        #[cfg(not(windows))]
        let store: Arc<dyn SecretStore> = Arc::new(pgt_core::credentials::MemorySecretStore::default());
        Self::with_store(&config_dir(), store)
    }
}

fn env_dir(var: &str) -> PathBuf {
    std::env::var_os(var).map(PathBuf::from).unwrap_or_else(|| PathBuf::from("."))
}

/// `%APPDATA%\pg-tools`
pub fn config_dir() -> PathBuf {
    env_dir("APPDATA").join("pg-tools")
}

/// `%LOCALAPPDATA%\pg-tools\logs`
pub fn log_dir() -> PathBuf {
    env_dir("LOCALAPPDATA").join("pg-tools").join("logs")
}
```

`src-tauri/src/commands.rs`:

```rust
use pgt_catalog::node::{NodeId, TreeNode};
use pgt_catalog::props::PropertySection;
use pgt_core::config::ServerConfig;
use pgt_core::connection::{ConnectionManager, ServerInfo};
use pgt_core::{Error, ErrorPayload};
use tauri::State;
use uuid::Uuid;

use crate::state::AppState;

type CmdResult<T> = Result<T, ErrorPayload>;

#[tauri::command]
pub fn list_servers(state: State<'_, AppState>) -> CmdResult<Vec<ServerConfig>> {
    Ok(state.registry.read().unwrap().servers().to_vec())
}

#[tauri::command]
pub fn save_server(state: State<'_, AppState>, config: ServerConfig, password: Option<String>) -> CmdResult<ServerConfig> {
    let saved = state.registry.write().unwrap().upsert(config)?;
    state.manager.disconnect(saved.id);
    match (password, saved.save_password) {
        (Some(p), true) => state.credentials.remember(saved.id, &p, true)?,
        (_, false) => state.credentials.forget(saved.id)?,
        (None, true) => {}
    }
    Ok(saved)
}

#[tauri::command]
pub async fn test_connection(config: ServerConfig, password: Option<String>) -> CmdResult<ServerInfo> {
    let config = config.normalized();
    config.validate()?;
    Ok(ConnectionManager::test(&config, password.as_deref()).await?)
}

#[tauri::command]
pub async fn connect_server(state: State<'_, AppState>, server_id: Uuid, password: Option<String>, remember: bool) -> CmdResult<ServerInfo> {
    let config = state.registry.read().unwrap().get(server_id).cloned().ok_or(Error::UnknownServer(server_id))?;
    let effective = match &password {
        Some(p) => Some(p.clone()),
        None => state.credentials.password(server_id)?,
    };
    let info = state.manager.connect(config.clone(), effective).await?;
    if let Some(p) = password {
        if remember && !config.save_password {
            let mut updated = config.clone();
            updated.save_password = true;
            state.registry.write().unwrap().upsert(updated)?;
        }
        state.credentials.remember(server_id, &p, remember || config.save_password)?;
    }
    Ok(info)
}

#[tauri::command]
pub fn disconnect_server(state: State<'_, AppState>, server_id: Uuid) {
    state.manager.disconnect(server_id);
}

#[tauri::command]
pub async fn list_children(state: State<'_, AppState>, id: String) -> CmdResult<Vec<TreeNode>> {
    Ok(state.catalog.list_children(&NodeId::parse(&id)?).await?)
}

#[tauri::command]
pub async fn get_properties(state: State<'_, AppState>, id: String) -> CmdResult<Vec<PropertySection>> {
    Ok(state.catalog.properties(&NodeId::parse(&id)?).await?)
}

#[tauri::command]
pub async fn get_ddl(state: State<'_, AppState>, id: String) -> CmdResult<String> {
    Ok(state.catalog.ddl(&NodeId::parse(&id)?).await?)
}

#[tauri::command]
pub fn startup_notice(state: State<'_, AppState>) -> Option<String> {
    state.startup_notice.clone()
}
```

> `?` operatörü `pgt_core::Error`'u `From<Error> for ErrorPayload` ile dönüştürür. `std::sync::RwLock` kilitleri hiçbir `await` noktasını aşmaz (aşarsa future `Send` olmaz ve derleme hatası verir).

`src-tauri/src/logging.rs`:

```rust
use tracing_appender::non_blocking::WorkerGuard;
use tracing_appender::rolling::{Builder, Rotation};
use tracing_subscriber::EnvFilter;

/// `%LOCALAPPDATA%\pg-tools\logs\pg-tools.YYYY-MM-DD.log`, son 5 dosya. Seviye: `PGT_LOG` (varsayılan info).
pub fn init() -> Option<WorkerGuard> {
    let appender = Builder::new()
        .rotation(Rotation::DAILY)
        .filename_prefix("pg-tools")
        .filename_suffix("log")
        .max_log_files(5)
        .build(crate::state::log_dir())
        .ok()?;
    let (writer, guard) = tracing_appender::non_blocking(appender);
    tracing_subscriber::fmt()
        .with_writer(writer)
        .with_ansi(false)
        .with_env_filter(EnvFilter::try_from_env("PGT_LOG").unwrap_or_else(|_| EnvFilter::new("info")))
        .init();
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        tracing::error!(panic = %info, "panik");
        default_hook(info);
    }));
    Some(guard)
}
```

`src-tauri/src/main.rs`:

```rust
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod commands;
mod logging;
mod state;

use std::time::Duration;

use tauri::Manager;

fn main() {
    let _log_guard = logging::init();
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            let state = state::AppState::init().map_err(|e| e.to_string())?;
            let manager = state.manager.clone();
            tauri::async_runtime::spawn(async move {
                let mut tick = tokio::time::interval(Duration::from_secs(60));
                loop {
                    tick.tick().await;
                    manager.reap_idle(Duration::from_secs(600)).await;
                }
            });
            app.manage(state);
            tracing::info!("pg-tools başlatıldı");
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::list_servers,
            commands::save_server,
            commands::test_connection,
            commands::connect_server,
            commands::disconnect_server,
            commands::list_children,
            commands::get_properties,
            commands::get_ddl,
            commands::startup_notice,
        ])
        .run(tauri::generate_context!())
        .expect("pg-tools başlatılamadı");
}
```

- [ ] **Step 5: Testlerin geçtiğini ve uygulamanın açıldığını doğrula**

Run: `cargo test -p pg-tools`
Expected: PASS (2 test).

Run: `bun tauri dev`
Expected: "pg-tools" başlıklı boş pencere (araç çubuğu, iki panel, durum çubuğu) açılır; `%LOCALAPPDATA%\pg-tools\logs\` altında "pg-tools başlatıldı" satırı olan log dosyası oluşur. Pencereyi kapatın.

- [ ] **Step 6: Commit**

```bash
git add Cargo.toml Cargo.lock app-icon.svg src-tauri
git commit -m "feat(app): Tauri kabuğu, komutlar, loglama ve boşta bağlantı temizliği"
```

---

### Task 11: IPC sarmalayıcıları ve saf ağaç modeli

**Files:**
- Create: `src/ipc/commands.ts`, `src/ipc/errors.ts`, `src/ipc/errors.test.ts`, `src/features/explorer/tree-model.ts`, `src/features/explorer/tree-model.test.ts`

**Interfaces:**
- Consumes: `src/ipc/bindings/*.ts` (Task 1–5'te üretildi): `ServerConfig`, `SslConfig`, `SslMode`, `ErrorPayload`, `ServerInfo`, `TreeNode`, `NodeKind`, `NodeFlags`, `PropertySection`, `PropertyRow`
- Produces:
  - `errors.ts`: `class IpcError extends Error { payload: ErrorPayload }`, `toPayload(e: unknown): ErrorPayload`
  - `commands.ts`: `commands` nesnesi — `listServers()`, `saveServer(config, password)`, `testConnection(config, password)`, `connectServer(serverId, password, remember)`, `disconnectServer(serverId)`, `listChildren(id)`, `getProperties(id)`, `getDdl(id)`, `startupNotice()`; hepsi `Promise` döner, hata durumunda `IpcError` fırlatır. `type Api = typeof commands`.
  - `tree-model.ts`:
    - `ROOT = ""`
    - `interface TreeState { nodes: Map<string, TreeNode>; parent: Map<string, string>; children: Map<string, string[]>; errors: Map<string, ErrorPayload>; loading: Set<string>; expanded: Set<string>; selected: string | null }`
    - `emptyTree()`, `startLoading(s, id)`, `stopLoading(s, id)`, `setChildren(s, parentId, nodes)`, `setError(s, id, err)`, `patchNode(s, id, patch)`, `expand(s, id)`, `collapse(s, id)`, `select(s, id)`, `invalidate(s, id)`
    - `interface VisibleRow { id; node; depth; expanded; loading; error: ErrorPayload | null; childCount: number | null; setSize; posInSet }`, `flatten(s): VisibleRow[]`
    - `type Intent = { select?: string; expand?: string; collapse?: string; refresh?: string }`, `keyIntent(s, rows, key): Intent | null`
    - `isCollection(kind)`, `rowLabel(row)`

**Model kuralları:**
- `setChildren` id'ye göre uzlaştırır: artık olmayan çocukların alt ağaçları silinir; kalanların genişletme durumu korunur; seçili düğüm silindiyse seçim üst düğüme geçer (kökse `null`).
- Bir sunucu düğümü `connected: true → false` olursa alt ağacı silinir ve daraltılır (bağlantı kesildi/ayarlar değişti).
- `flatten`: kökten başlar, yalnızca genişletilmiş ve yüklenmiş düğümlere iner.
- Klavye: ↓/↑ komşu satır; Home/End ilk/son; → kapalıysa genişlet, açıksa ilk çocuğa geç; ← açıksa daralt, değilse üst düğüme geç; Enter aç/kapa; F5 seçiliyi yenile. Seçim yokken ok tuşları ilk satırı seçer.
- `rowLabel`: koleksiyonlar yüklendiyse `"Tables (1.243)"` (tr-TR binlik ayırıcı).

- [ ] **Step 1: Başarısız testleri yaz**

`src/ipc/errors.test.ts`:

```ts
import { describe, expect, it } from "vitest";
import { IpcError, toPayload } from "./errors";

describe("toPayload", () => {
  it("keeps ErrorPayload objects from Tauri", () => {
    const p = { code: "auth", message: "m", detail: null, hint: null };
    expect(toPayload(p)).toEqual(p);
    expect(toPayload(new IpcError(p))).toEqual(p);
  });

  it("wraps strings and unknown errors", () => {
    expect(toPayload("boom")).toEqual({ code: "unknown", message: "boom", detail: null, hint: null });
    expect(toPayload(new Error("x")).message).toBe("x");
  });
});
```

`src/features/explorer/tree-model.test.ts`:

```ts
import { describe, expect, it } from "vitest";
import type { TreeNode } from "../../ipc/bindings/TreeNode";
import {
  ROOT, collapse, emptyTree, expand, flatten, keyIntent, patchNode, rowLabel, select, setChildren, setError, startLoading,
} from "./tree-model";

const flags = { connected: false, partitioned: false, system: false, primaryKey: false, notNull: false };
function node(id: string, extra: Partial<TreeNode> = {}): TreeNode {
  return { id, kind: "table", label: id, detail: null, hasChildren: true, serverId: null, flags, ...extra };
}

function sample() {
  let s = emptyTree();
  s = setChildren(s, ROOT, [node("a"), node("b")]);
  s = expand(s, "a");
  s = setChildren(s, "a", [node("a1"), node("a2", { hasChildren: false })]);
  return s;
}

describe("tree-model", () => {
  it("flattens expanded nodes with depth and aria positions", () => {
    const rows = flatten(sample());
    expect(rows.map((r) => [r.id, r.depth, r.posInSet, r.setSize])).toEqual([
      ["a", 0, 1, 2], ["a1", 1, 1, 2], ["a2", 1, 2, 2], ["b", 0, 2, 2],
    ]);
    expect(rows[0].expanded).toBe(true);
    expect(rows[0].childCount).toBe(2);
    expect(rows[3].childCount).toBeNull();
  });

  it("collapsed nodes hide children but keep them loaded", () => {
    const s = collapse(sample(), "a");
    expect(flatten(s).map((r) => r.id)).toEqual(["a", "b"]);
    expect(s.children.get("a")).toEqual(["a1", "a2"]);
  });

  it("reconciles refreshed children: removes gone subtrees, keeps expansion", () => {
    let s = sample();
    s = expand(s, "a1");
    s = setChildren(s, "a1", [node("a1x")]);
    s = select(s, "a2");
    s = setChildren(s, "a", [node("a1"), node("a3")]);
    expect(s.nodes.has("a2")).toBe(false);
    expect(s.expanded.has("a1")).toBe(true);
    expect(s.children.get("a1")).toEqual(["a1x"]);
    expect(s.selected).toBe("a");
  });

  it("drops subtree when a server becomes disconnected", () => {
    let s = emptyTree();
    const on = { ...flags, connected: true };
    s = setChildren(s, ROOT, [node("srv", { kind: "server", flags: on })]);
    s = expand(s, "srv");
    s = setChildren(s, "srv", [node("dbs")]);
    s = setChildren(s, ROOT, [node("srv", { kind: "server", flags })]);
    expect(s.children.has("srv")).toBe(false);
    expect(s.expanded.has("srv")).toBe(false);
    expect(s.nodes.has("dbs")).toBe(false);
  });

  it("tracks loading and errors per node", () => {
    let s = startLoading(sample(), "b");
    expect(flatten(s).find((r) => r.id === "b")!.loading).toBe(true);
    s = setError(s, "b", { code: "network", message: "x", detail: null, hint: null });
    const b = flatten(s).find((r) => r.id === "b")!;
    expect([b.loading, b.error?.code]).toEqual([false, "network"]);
    s = setChildren(s, "b", []);
    expect(flatten(s).find((r) => r.id === "b")!.error).toBeNull();
  });

  it("patches nodes immutably", () => {
    const s = sample();
    const p = patchNode(s, "a", { detail: "16.4" });
    expect(p.nodes.get("a")!.detail).toBe("16.4");
    expect(s.nodes.get("a")!.detail).toBeNull();
  });

  it("maps keys to intents", () => {
    const s0 = sample();
    const rows = flatten(s0);
    expect(keyIntent(s0, rows, "ArrowDown")).toEqual({ select: "a" });
    const s = select(s0, "a1");
    expect(keyIntent(s, rows, "ArrowDown")).toEqual({ select: "a2" });
    expect(keyIntent(s, rows, "ArrowUp")).toEqual({ select: "a" });
    expect(keyIntent(s, rows, "End")).toEqual({ select: "b" });
    expect(keyIntent(s, rows, "Home")).toEqual({ select: "a" });
    expect(keyIntent(s, rows, "ArrowRight")).toEqual({ expand: "a1" });
    expect(keyIntent(s, rows, "ArrowLeft")).toEqual({ select: "a" });
    expect(keyIntent(s, rows, "F5")).toEqual({ refresh: "a1" });
    const onA = select(s0, "a");
    expect(keyIntent(onA, rows, "ArrowRight")).toEqual({ select: "a1" });
    expect(keyIntent(onA, rows, "ArrowLeft")).toEqual({ collapse: "a" });
    expect(keyIntent(onA, rows, "Enter")).toEqual({ collapse: "a" });
    const onLeaf = select(s0, "a2");
    expect(keyIntent(onLeaf, rows, "ArrowRight")).toBeNull();
    expect(keyIntent(onLeaf, rows, "x")).toBeNull();
  });

  it("labels loaded collections with Turkish thousands separator", () => {
    let s = emptyTree();
    s = setChildren(s, ROOT, [node("t", { kind: "tableCollection", label: "Tables" })]);
    expect(rowLabel(flatten(s)[0])).toBe("Tables");
    s = setChildren(s, "t", Array.from({ length: 1243 }, (_, i) => node(`t${i}`)));
    expect(rowLabel(flatten(s)[0])).toBe("Tables (1.243)");
  });
});
```

- [ ] **Step 2: Testlerin başarısız olduğunu doğrula**

Run: `bun run test`
Expected: FAIL — `./errors` ve `./tree-model` yok.

- [ ] **Step 3: IPC katmanını yaz**

`src/ipc/errors.ts`:

```ts
import type { ErrorPayload } from "./bindings/ErrorPayload";

export class IpcError extends Error {
  constructor(public payload: ErrorPayload) {
    super(payload.message);
  }
}

function isPayload(e: unknown): e is ErrorPayload {
  return typeof e === "object" && e !== null && "code" in e && "message" in e;
}

export function toPayload(e: unknown): ErrorPayload {
  if (e instanceof IpcError) return e.payload;
  if (isPayload(e)) return { code: e.code, message: e.message, detail: e.detail ?? null, hint: e.hint ?? null };
  const message = e instanceof Error ? e.message : String(e);
  return { code: "unknown", message, detail: null, hint: null };
}
```

`src/ipc/commands.ts`:

```ts
import { invoke } from "@tauri-apps/api/core";
import type { PropertySection } from "./bindings/PropertySection";
import type { ServerConfig } from "./bindings/ServerConfig";
import type { ServerInfo } from "./bindings/ServerInfo";
import type { TreeNode } from "./bindings/TreeNode";
import { IpcError, toPayload } from "./errors";

async function call<T>(cmd: string, args?: Record<string, unknown>): Promise<T> {
  try {
    return await invoke<T>(cmd, args);
  } catch (e) {
    throw new IpcError(toPayload(e));
  }
}

export const commands = {
  listServers: () => call<ServerConfig[]>("list_servers"),
  saveServer: (config: ServerConfig, password: string | null) => call<ServerConfig>("save_server", { config, password }),
  testConnection: (config: ServerConfig, password: string | null) => call<ServerInfo>("test_connection", { config, password }),
  connectServer: (serverId: string, password: string | null, remember: boolean) =>
    call<ServerInfo>("connect_server", { serverId, password, remember }),
  disconnectServer: (serverId: string) => call<null>("disconnect_server", { serverId }),
  listChildren: (id: string) => call<TreeNode[]>("list_children", { id }),
  getProperties: (id: string) => call<PropertySection[]>("get_properties", { id }),
  getDdl: (id: string) => call<string>("get_ddl", { id }),
  startupNotice: () => call<string | null>("startup_notice"),
};

export type Api = typeof commands;
```

- [ ] **Step 4: Ağaç modelini yaz**

`src/features/explorer/tree-model.ts`:

```ts
import type { ErrorPayload } from "../../ipc/bindings/ErrorPayload";
import type { NodeKind } from "../../ipc/bindings/NodeKind";
import type { TreeNode } from "../../ipc/bindings/TreeNode";

export const ROOT = "";

export interface TreeState {
  nodes: Map<string, TreeNode>;
  parent: Map<string, string>;
  children: Map<string, string[]>;
  errors: Map<string, ErrorPayload>;
  loading: Set<string>;
  expanded: Set<string>;
  selected: string | null;
}

export function emptyTree(): TreeState {
  return { nodes: new Map(), parent: new Map(), children: new Map(), errors: new Map(), loading: new Set(), expanded: new Set(), selected: null };
}

function clone(s: TreeState): TreeState {
  return {
    nodes: new Map(s.nodes),
    parent: new Map(s.parent),
    children: new Map(s.children),
    errors: new Map(s.errors),
    loading: new Set(s.loading),
    expanded: new Set(s.expanded),
    selected: s.selected,
  };
}

function removeChildrenMut(s: TreeState, id: string): void {
  for (const child of s.children.get(id) ?? []) removeSubtreeMut(s, child);
  s.children.delete(id);
}

function removeSubtreeMut(s: TreeState, id: string): void {
  removeChildrenMut(s, id);
  s.nodes.delete(id);
  s.parent.delete(id);
  s.errors.delete(id);
  s.loading.delete(id);
  s.expanded.delete(id);
  if (s.selected === id) s.selected = null;
}

export function startLoading(s: TreeState, id: string): TreeState {
  const next = clone(s);
  next.loading.add(id);
  next.errors.delete(id);
  return next;
}

export function stopLoading(s: TreeState, id: string): TreeState {
  const next = clone(s);
  next.loading.delete(id);
  return next;
}

export function setChildren(s: TreeState, parentId: string, list: TreeNode[]): TreeState {
  const next = clone(s);
  const selectedBefore = s.selected;
  const keep = new Set(list.map((n) => n.id));
  for (const old of s.children.get(parentId) ?? []) {
    if (!keep.has(old)) removeSubtreeMut(next, old);
  }
  for (const n of list) {
    const previous = s.nodes.get(n.id);
    if (previous?.flags.connected && !n.flags.connected) {
      removeChildrenMut(next, n.id);
      next.expanded.delete(n.id);
    }
    next.nodes.set(n.id, n);
    next.parent.set(n.id, parentId);
  }
  next.children.set(parentId, list.map((n) => n.id));
  next.loading.delete(parentId);
  next.errors.delete(parentId);
  if (selectedBefore !== null && next.selected === null) {
    next.selected = parentId === ROOT ? null : parentId;
  }
  return next;
}

export function setError(s: TreeState, id: string, error: ErrorPayload): TreeState {
  const next = clone(s);
  next.loading.delete(id);
  next.errors.set(id, error);
  return next;
}

export function patchNode(s: TreeState, id: string, patch: Partial<TreeNode>): TreeState {
  const current = s.nodes.get(id);
  if (!current) return s;
  const next = clone(s);
  next.nodes.set(id, { ...current, ...patch });
  return next;
}

export function expand(s: TreeState, id: string): TreeState {
  const next = clone(s);
  next.expanded.add(id);
  return next;
}

export function collapse(s: TreeState, id: string): TreeState {
  const next = clone(s);
  next.expanded.delete(id);
  return next;
}

export function select(s: TreeState, id: string | null): TreeState {
  return { ...s, selected: id };
}

/** Çocukları siler ve düğümü daraltır (yeniden yüklenmeleri gerekir). */
export function invalidate(s: TreeState, id: string): TreeState {
  const next = clone(s);
  removeChildrenMut(next, id);
  next.expanded.delete(id);
  if (next.selected !== null && !next.nodes.has(next.selected)) next.selected = id;
  return next;
}

export interface VisibleRow {
  id: string;
  node: TreeNode;
  depth: number;
  expanded: boolean;
  loading: boolean;
  error: ErrorPayload | null;
  childCount: number | null;
  setSize: number;
  posInSet: number;
}

export function flatten(s: TreeState): VisibleRow[] {
  const out: VisibleRow[] = [];
  const walk = (parentId: string, depth: number) => {
    const ids = s.children.get(parentId);
    if (!ids) return;
    ids.forEach((id, index) => {
      const node = s.nodes.get(id);
      if (!node) return;
      const expanded = s.expanded.has(id);
      const kids = s.children.get(id);
      out.push({
        id,
        node,
        depth,
        expanded,
        loading: s.loading.has(id),
        error: s.errors.get(id) ?? null,
        childCount: kids ? kids.length : null,
        setSize: ids.length,
        posInSet: index + 1,
      });
      if (expanded) walk(id, depth + 1);
    });
  };
  walk(ROOT, 0);
  return out;
}

export type Intent = { select?: string; expand?: string; collapse?: string; refresh?: string };

export function keyIntent(s: TreeState, rows: VisibleRow[], key: string): Intent | null {
  if (rows.length === 0) return null;
  const index = s.selected === null ? -1 : rows.findIndex((r) => r.id === s.selected);
  if (index < 0) {
    return ["ArrowDown", "ArrowUp", "Home", "End"].includes(key) ? { select: rows[0].id } : null;
  }
  const row = rows[index];
  switch (key) {
    case "ArrowDown":
      return index + 1 < rows.length ? { select: rows[index + 1].id } : null;
    case "ArrowUp":
      return index > 0 ? { select: rows[index - 1].id } : null;
    case "Home":
      return { select: rows[0].id };
    case "End":
      return { select: rows[rows.length - 1].id };
    case "ArrowRight":
      if (!row.node.hasChildren) return null;
      if (!row.expanded) return { expand: row.id };
      return rows[index + 1]?.depth === row.depth + 1 ? { select: rows[index + 1].id } : null;
    case "ArrowLeft": {
      if (row.expanded) return { collapse: row.id };
      const parent = s.parent.get(row.id);
      return parent && parent !== ROOT ? { select: parent } : null;
    }
    case "Enter":
      if (!row.node.hasChildren) return null;
      return row.expanded ? { collapse: row.id } : { expand: row.id };
    case "F5":
      return { refresh: row.id };
    default:
      return null;
  }
}

export function isCollection(kind: NodeKind): boolean {
  return kind.endsWith("Collection");
}

const numberFormat = new Intl.NumberFormat("tr-TR");

export function rowLabel(row: VisibleRow): string {
  return isCollection(row.node.kind) && row.childCount !== null
    ? `${row.node.label} (${numberFormat.format(row.childCount)})`
    : row.node.label;
}
```

- [ ] **Step 5: Testlerin geçtiğini doğrula**

Run: `bun run test`
Expected: PASS (errors 2, tree-model 8, önceki 3).

Run: `bun run typecheck`
Expected: hata yok.

- [ ] **Step 6: Commit**

```bash
git add src/ipc src/features/explorer
git commit -m "feat(ui): IPC sarmalayıcıları ve saf ağaç modeli"
```

---

### Task 12: Gezgin denetleyicisi (yükleme, bağlanma, parola istemi, klavye)

**Files:**
- Create: `src/features/explorer/controller.ts`, `src/features/explorer/controller.test.ts`, `src/features/explorer/explorer-store.ts`

**Interfaces:**
- Consumes: `tree-model.ts` (Task 11), `Api`, `toPayload` (Task 11)
- Produces:
  - `type PasswordAnswer = { password: string; remember: boolean }`
  - `interface ExplorerDeps { api: Pick<Api, "listChildren" | "connectServer">; requestPassword(node: TreeNode, retry: boolean): Promise<PasswordAnswer | null> }`
  - `class ExplorerController(get: () => TreeState, set: (s: TreeState) => void, deps: ExplorerDeps)` metotları: `load(id): Promise<void>`, `ensureConnected(id): Promise<boolean>`, `expand(id): Promise<void>`, `collapse(id): void`, `toggle(id): Promise<void>`, `select(id): void`, `refresh(id): Promise<void>`, `reloadServers(): Promise<void>`, `handleKey(key): boolean` (işlendiyse true; asenkron işi arka planda başlatır)
  - `explorer-store.ts`: `createExplorer(deps) → { state, rows, selectedNode, controller }`, `type Explorer`

Denetleyici Solid'den bağımsızdır (get/set fonksiyonlarıyla çalışır); bu sayede Vitest'te reaktivite olmadan test edilir.

**Bağlanma akışı (`ensureConnected`):**
1. Düğüm `server` değilse veya zaten bağlıysa → `true`.
2. `connectServer(serverId, password, remember)` çağrılır (ilk denemede `password = null`; Rust tarafı kayıtlı parolayı dener).
3. Hata kodu `password_required` → `requestPassword(node, false)`; `auth` → `requestPassword(node, true)`. Kullanıcı iptal ederse `false`; yanıt verirse 2'ye dön.
4. Diğer hatalar → düğümde hata gösterilir, `false`.
5. Başarı → düğüm `{ detail: version, flags.connected: true }` olarak güncellenir.

- [ ] **Step 1: Başarısız testleri yaz**

`src/features/explorer/controller.test.ts`:

```ts
import { describe, expect, it, vi } from "vitest";
import type { ErrorPayload } from "../../ipc/bindings/ErrorPayload";
import type { TreeNode } from "../../ipc/bindings/TreeNode";
import { IpcError } from "../../ipc/errors";
import { ExplorerController, type ExplorerDeps } from "./controller";
import { ROOT, emptyTree, flatten, type TreeState } from "./tree-model";

const flags = { connected: false, partitioned: false, system: false, primaryKey: false, notNull: false };
const n = (id: string, extra: Partial<TreeNode> = {}): TreeNode =>
  ({ id, kind: "table", label: id, detail: null, hasChildren: true, serverId: null, flags, ...extra });
const err = (code: string): IpcError => new IpcError({ code, message: code, detail: null, hint: null } as ErrorPayload);

function setup(children: Record<string, TreeNode[]>, deps: Partial<ExplorerDeps> = {}) {
  let state: TreeState = emptyTree();
  const listChildren = vi.fn(async (id: string) => children[id] ?? []);
  const connectServer = vi.fn(async (_serverId: string, _password: string | null, _remember: boolean) => ({
    versionNum: 180000,
    version: "18.0",
  }));
  const requestPassword = vi.fn(async () => null);
  const c = new ExplorerController(
    () => state,
    (s) => (state = s),
    { api: { listChildren, connectServer }, requestPassword, ...deps },
  );
  return { c, get: () => state, listChildren, connectServer, requestPassword };
}

describe("ExplorerController", () => {
  it("loads children once and toggles expansion", async () => {
    const { c, get, listChildren } = setup({ [ROOT]: [n("a")], a: [n("a1")] });
    await c.load(ROOT);
    await c.toggle("a");
    expect(flatten(get()).map((r) => r.id)).toEqual(["a", "a1"]);
    await c.toggle("a");
    await c.toggle("a");
    expect(listChildren).toHaveBeenCalledTimes(2);
  });

  it("connects a disconnected server before expanding", async () => {
    const srv = n("srv", { kind: "server", serverId: "u1" });
    const { c, get, connectServer } = setup({ [ROOT]: [srv], srv: [n("dbs")] });
    await c.load(ROOT);
    await c.expand("srv");
    expect(connectServer).toHaveBeenCalledWith("u1", null, false);
    const node = get().nodes.get("srv")!;
    expect([node.flags.connected, node.detail]).toEqual([true, "18.0"]);
    expect(flatten(get()).map((r) => r.id)).toEqual(["srv", "dbs"]);
  });

  it("asks for a password when required and remembers the choice", async () => {
    const srv = n("srv", { kind: "server", serverId: "u1" });
    const connectServer = vi
      .fn()
      .mockRejectedValueOnce(err("password_required"))
      .mockResolvedValueOnce({ versionNum: 160004, version: "16.4" });
    const requestPassword = vi.fn(async () => ({ password: "p", remember: true }));
    const { c } = setup({ [ROOT]: [srv] }, { requestPassword, api: { listChildren: async () => [], connectServer } });
    await c.load(ROOT);
    await c.expand("srv");
    expect(requestPassword).toHaveBeenCalledWith(expect.objectContaining({ id: "srv" }), false);
    expect(connectServer).toHaveBeenLastCalledWith("u1", "p", true);
  });

  it("auth error re-prompts and retries with new password", async () => {
    const srv = n("srv", { kind: "server", serverId: "u1" });
    const connectServer = vi
      .fn()
      .mockRejectedValueOnce(err("auth"))
      .mockRejectedValueOnce(err("auth"))
      .mockResolvedValueOnce({ versionNum: 180000, version: "18.0" });
    const requestPassword = vi
      .fn()
      .mockResolvedValueOnce({ password: "yanlis", remember: false })
      .mockResolvedValueOnce({ password: "dogru", remember: true });
    const { c, get } = setup({ [ROOT]: [srv] }, { requestPassword, api: { listChildren: async () => [], connectServer } });
    await c.load(ROOT);
    await c.expand("srv");
    expect(requestPassword.mock.calls.map((call) => call[1])).toEqual([true, true]);
    expect(connectServer).toHaveBeenLastCalledWith("u1", "dogru", true);
    expect(get().nodes.get("srv")!.flags.connected).toBe(true);
  });

  it("cancelled prompt leaves the server collapsed without error", async () => {
    const srv = n("srv", { kind: "server", serverId: "u1" });
    const connectServer = vi.fn().mockRejectedValue(err("password_required"));
    const { c, get } = setup({ [ROOT]: [srv] }, { api: { listChildren: async () => [], connectServer } });
    await c.load(ROOT);
    await c.expand("srv");
    const row = flatten(get())[0];
    expect([row.expanded, row.loading, row.error]).toEqual([false, false, null]);
  });

  it("network error is shown on the server node", async () => {
    const srv = n("srv", { kind: "server", serverId: "u1" });
    const connectServer = vi.fn().mockRejectedValue(err("network"));
    const { c, get } = setup({ [ROOT]: [srv] }, { api: { listChildren: async () => [], connectServer } });
    await c.load(ROOT);
    await c.expand("srv");
    expect(flatten(get())[0].error?.code).toBe("network");
  });

  it("load errors are attached to the parent and cleared by a successful reload", async () => {
    const listChildren = vi.fn().mockRejectedValueOnce(err("query")).mockResolvedValueOnce([n("a")]);
    const { c, get } = setup({}, { api: { listChildren, connectServer: vi.fn() } });
    await c.load(ROOT);
    expect(get().errors.get(ROOT)?.code).toBe("query");
    expect(get().loading.has(ROOT)).toBe(false);
    await c.load(ROOT);
    expect(get().errors.size).toBe(0);
    expect(flatten(get()).map((r) => r.id)).toEqual(["a"]);
  });

  it("keyboard intents select, expand and refresh", async () => {
    const { c, get, listChildren } = setup({ [ROOT]: [n("a"), n("b")], a: [n("a1")] });
    await c.load(ROOT);
    expect(c.handleKey("ArrowDown")).toBe(true);
    expect(get().selected).toBe("a");
    expect(c.handleKey("ArrowRight")).toBe(true);
    await vi.waitFor(() => expect(get().expanded.has("a")).toBe(true));
    expect(c.handleKey("q")).toBe(false);
    c.handleKey("F5");
    await vi.waitFor(() => expect(listChildren).toHaveBeenCalledTimes(3));
  });

  it("reloadServers refreshes root and loaded groups", async () => {
    const group = n("grp:G", { kind: "serverGroup" });
    const { c, listChildren } = setup({ [ROOT]: [group], "grp:G": [] });
    await c.load(ROOT);
    await c.expand("grp:G");
    listChildren.mockClear();
    await c.reloadServers();
    expect(listChildren.mock.calls.map((call) => call[0])).toEqual([ROOT, "grp:G"]);
  });
});
```

- [ ] **Step 2: Testlerin başarısız olduğunu doğrula**

Run: `bun run test`
Expected: FAIL — `./controller` yok.

- [ ] **Step 3: Denetleyiciyi yaz**

`src/features/explorer/controller.ts`:

```ts
import type { Api } from "../../ipc/commands";
import type { TreeNode } from "../../ipc/bindings/TreeNode";
import { toPayload } from "../../ipc/errors";
import {
  ROOT, collapse, expand, flatten, keyIntent, patchNode, select, setChildren, setError, startLoading, stopLoading,
  type Intent, type TreeState,
} from "./tree-model";

export type PasswordAnswer = { password: string; remember: boolean };

export interface ExplorerDeps {
  api: Pick<Api, "listChildren" | "connectServer">;
  requestPassword(node: TreeNode, retry: boolean): Promise<PasswordAnswer | null>;
}

export class ExplorerController {
  constructor(
    private readonly get: () => TreeState,
    private readonly set: (s: TreeState) => void,
    private readonly deps: ExplorerDeps,
  ) {}

  private update(f: (s: TreeState) => TreeState): void {
    this.set(f(this.get()));
  }

  async load(id: string): Promise<void> {
    this.update((s) => startLoading(s, id));
    try {
      const list = await this.deps.api.listChildren(id);
      this.update((s) => setChildren(s, id, list));
    } catch (e) {
      this.update((s) => setError(s, id, toPayload(e)));
    }
  }

  async ensureConnected(id: string): Promise<boolean> {
    const node = this.get().nodes.get(id);
    if (!node || node.kind !== "server" || node.flags.connected || !node.serverId) return true;
    let password: string | null = null;
    let remember = false;
    for (;;) {
      this.update((s) => startLoading(s, id));
      try {
        const info = await this.deps.api.connectServer(node.serverId, password, remember);
        this.update((s) => patchNode(stopLoading(s, id), id, { detail: info.version, flags: { ...node.flags, connected: true } }));
        return true;
      } catch (e) {
        const error = toPayload(e);
        if (error.code === "password_required" || error.code === "auth") {
          this.update((s) => stopLoading(s, id));
          const answer = await this.deps.requestPassword(node, error.code === "auth");
          if (!answer) return false;
          password = answer.password;
          remember = answer.remember;
          continue;
        }
        this.update((s) => setError(s, id, error));
        return false;
      }
    }
  }

  async expand(id: string): Promise<void> {
    const node = this.get().nodes.get(id);
    if (!node?.hasChildren) return;
    if (!(await this.ensureConnected(id))) return;
    this.update((s) => expand(s, id));
    if (!this.get().children.has(id)) await this.load(id);
  }

  collapse(id: string): void {
    this.update((s) => collapse(s, id));
  }

  async toggle(id: string): Promise<void> {
    if (this.get().expanded.has(id)) this.collapse(id);
    else await this.expand(id);
  }

  select(id: string): void {
    this.update((s) => select(s, id));
  }

  async refresh(id: string): Promise<void> {
    if (id === ROOT || this.get().children.has(id)) await this.load(id);
  }

  async reloadServers(): Promise<void> {
    await this.load(ROOT);
    const groups = (this.get().children.get(ROOT) ?? []).filter(
      (gid) => this.get().nodes.get(gid)?.kind === "serverGroup" && this.get().children.has(gid),
    );
    for (const gid of groups) await this.load(gid);
  }

  handleKey(key: string): boolean {
    const state = this.get();
    const intent = keyIntent(state, flatten(state), key);
    if (!intent) return false;
    void this.apply(intent);
    return true;
  }

  private async apply(intent: Intent): Promise<void> {
    if (intent.select !== undefined) this.select(intent.select);
    if (intent.collapse !== undefined) this.collapse(intent.collapse);
    if (intent.expand !== undefined) await this.expand(intent.expand);
    if (intent.refresh !== undefined) await this.refresh(intent.refresh);
  }
}
```

`src/features/explorer/explorer-store.ts`:

```ts
import { createMemo, createSignal } from "solid-js";
import { ExplorerController, type ExplorerDeps } from "./controller";
import { emptyTree, flatten } from "./tree-model";

export function createExplorer(deps: ExplorerDeps) {
  const [state, setState] = createSignal(emptyTree());
  const controller = new ExplorerController(state, setState, deps);
  const rows = createMemo(() => flatten(state()));
  const selectedNode = createMemo(() => {
    const s = state();
    return s.selected === null ? null : s.nodes.get(s.selected) ?? null;
  });
  return { state, rows, selectedNode, controller };
}

export type Explorer = ReturnType<typeof createExplorer>;
```

> F5 testi: `ROOT` yüklü olduğu ve seçili `a` da yüklü olduğu için `refresh("a")` üçüncü `listChildren` çağrısını yapar (1: ROOT, 2: a genişletme, 3: yenileme).

- [ ] **Step 4: Testlerin geçtiğini doğrula**

Run: `bun run test`
Expected: PASS (controller 9 test + öncekiler).

Run: `bun run typecheck`
Expected: hata yok.

- [ ] **Step 5: Commit**

```bash
git add src/features/explorer
git commit -m "feat(ui): gezgin denetleyicisi, bağlanma ve parola istemi akışı"
```

---

### Task 13: Sanal ağaç görünümü, parola istemi, araç çubuğu ve durum çubuğu

**Files:**
- Create: `src/ui/Button.tsx`, `src/ui/Dialog.tsx`, `src/ui/Field.tsx`, `src/ui/Tabs.tsx`, `src/features/explorer/NodeIcon.tsx`, `src/features/explorer/TreeRow.tsx`, `src/features/explorer/TreeView.tsx`, `src/features/servers/PasswordPrompt.tsx`, `src/app/Toolbar.tsx`, `src/app/StatusBar.tsx`
- Modify: `src/features/explorer/tree-model.ts`, `src/features/explorer/tree-model.test.ts`, `src/app/App.tsx`

**Interfaces:**
- Consumes: `createExplorer`, `Explorer`, `ExplorerController` (Task 12); `commands` (Task 11); `t` (Task 9)
- Produces:
  - `ui/Button.tsx`: `Button(props: ButtonHTMLAttributes & { variant?: "primary" | "secondary" | "ghost" })`
  - `ui/Dialog.tsx`: `Dialog(props: { open; onOpenChange; title; children; footer?; width? })`
  - `ui/Field.tsx`: `Field(props: { label; hint?; children })`, `inputClass: string`
  - `ui/Tabs.tsx`: `Tabs(props: { value; onChange; tabs: { value; label }[]; children; class? })`, `TabPanel` (= Kobalte `Tabs.Content`)
  - `tree-model.ts`: `serverOf(s: TreeState, id: string | null): TreeNode | null` (seçili düğümün sunucu atası)
  - `PasswordPrompt.tsx`: `createPasswordPrompt() → { ask(node, retry): Promise<PasswordAnswer | null>, View: Component }`
  - `TreeView(props: { explorer: Explorer })`, `Toolbar(props: { onRefresh; themePref; onCycleTheme; children? })`, `StatusBar(props: { explorer; notice: string | null })`

- [ ] **Step 1: `serverOf` için başarısız testi yaz**

`tree-model.test.ts` içindeki `describe` bloğuna ekle (ve import listesine `serverOf`):

```ts
  it("finds the server ancestor of a node", () => {
    let s = emptyTree();
    const on = { ...flags, connected: true };
    s = setChildren(s, ROOT, [node("grp:G", { kind: "serverGroup" })]);
    s = setChildren(s, "grp:G", [node("srv", { kind: "server", flags: on, detail: "18.0" })]);
    s = setChildren(s, "srv", [node("dbs", { kind: "databaseCollection" })]);
    s = setChildren(s, "dbs", [node("db", { kind: "database" })]);
    expect(serverOf(s, "db")?.id).toBe("srv");
    expect(serverOf(s, "srv")?.id).toBe("srv");
    expect(serverOf(s, "grp:G")).toBeNull();
    expect(serverOf(s, null)).toBeNull();
  });
```

- [ ] **Step 2: Testin başarısız olduğunu doğrula**

Run: `bun run test`
Expected: FAIL — `serverOf` export edilmemiş.

- [ ] **Step 3: `serverOf`'u uygula** (`tree-model.ts` sonuna)

```ts
export function serverOf(s: TreeState, id: string | null): TreeNode | null {
  let current = id;
  while (current !== null && current !== undefined && current !== ROOT) {
    const node = s.nodes.get(current);
    if (!node) return null;
    if (node.kind === "server") return node;
    current = s.parent.get(current) ?? null;
  }
  return null;
}
```

Run: `bun run test`
Expected: PASS.

- [ ] **Step 4: UI temel bileşenlerini yaz**

`src/ui/Button.tsx`:

```tsx
import { splitProps, type JSX } from "solid-js";

type Variant = "primary" | "secondary" | "ghost";

const variants: Record<Variant, string> = {
  primary: "bg-accent text-accent-fg hover:brightness-110",
  secondary: "border border-border bg-bg hover:bg-hover",
  ghost: "hover:bg-hover",
};

export function Button(props: JSX.ButtonHTMLAttributes<HTMLButtonElement> & { variant?: Variant }) {
  const [local, rest] = splitProps(props, ["variant", "class"]);
  return (
    <button
      type="button"
      {...rest}
      class={`inline-flex h-7 items-center gap-1.5 rounded px-2.5 text-[13px] disabled:opacity-50 ${variants[local.variant ?? "secondary"]} ${local.class ?? ""}`}
    />
  );
}
```

`src/ui/Field.tsx`:

```tsx
import { Show, type JSX } from "solid-js";

export const inputClass =
  "h-7 w-full rounded border border-border bg-bg px-2 text-[13px] outline-none focus:border-accent disabled:opacity-60";

export function Field(props: { label: string; hint?: string; children: JSX.Element }) {
  return (
    <label class="flex flex-col gap-1">
      <span class="text-xs text-muted">{props.label}</span>
      {props.children}
      <Show when={props.hint}>
        <span class="text-xs text-muted">{props.hint}</span>
      </Show>
    </label>
  );
}
```

`src/ui/Dialog.tsx`:

```tsx
import { Dialog as K } from "@kobalte/core/dialog";
import { X } from "lucide-solid";
import { Show, type JSX } from "solid-js";
import { t } from "../app/strings";

export function Dialog(props: {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  title: string;
  children: JSX.Element;
  footer?: JSX.Element;
  width?: string;
}) {
  return (
    <K open={props.open} onOpenChange={props.onOpenChange} modal>
      <K.Portal>
        <K.Overlay class="fixed inset-0 z-40 bg-black/40" />
        <div class="fixed inset-0 z-50 flex items-center justify-center p-4">
          <K.Content
            class="flex max-h-full flex-col rounded-md border border-border bg-bg shadow-xl"
            style={{ width: props.width ?? "480px" }}
          >
            <div class="flex items-center justify-between border-b border-border px-4 py-2.5">
              <K.Title class="font-semibold">{props.title}</K.Title>
              <K.CloseButton class="rounded p-1 text-muted hover:bg-hover" aria-label={t.server.close}>
                <X size={14} />
              </K.CloseButton>
            </div>
            <div class="min-h-0 overflow-auto p-4">{props.children}</div>
            <Show when={props.footer}>
              <div class="flex justify-end gap-2 border-t border-border px-4 py-2.5">{props.footer}</div>
            </Show>
          </K.Content>
        </div>
      </K.Portal>
    </K>
  );
}
```

`src/ui/Tabs.tsx`:

```tsx
import { Tabs as K } from "@kobalte/core/tabs";
import { For, type JSX } from "solid-js";

export function Tabs(props: {
  value: string;
  onChange: (value: string) => void;
  tabs: { value: string; label: string }[];
  children: JSX.Element;
  class?: string;
}) {
  return (
    <K value={props.value} onChange={props.onChange} class={`flex min-h-0 flex-col ${props.class ?? ""}`}>
      <K.List class="flex shrink-0 gap-1 border-b border-border px-2">
        <For each={props.tabs}>
          {(tab) => (
            <K.Trigger
              value={tab.value}
              class="border-b-2 border-transparent px-2 py-1.5 text-[13px] text-muted data-selected:border-accent data-selected:text-fg"
            >
              {tab.label}
            </K.Trigger>
          )}
        </For>
      </K.List>
      {props.children}
    </K>
  );
}

export const TabPanel = K.Content;
```

- [ ] **Step 5: Ağaç bileşenlerini yaz**

`src/features/explorer/NodeIcon.tsx`:

```tsx
import { Columns3, Database, Folder, KeyRound, Layers, Server, Table2, TableProperties } from "lucide-solid";
import { Dynamic } from "solid-js/web";
import type { TreeNode } from "../../ipc/bindings/TreeNode";
import { isCollection } from "./tree-model";

function iconFor(node: TreeNode) {
  switch (node.kind) {
    case "server":
      return Server;
    case "database":
      return Database;
    case "schema":
      return Layers;
    case "table":
      return node.flags.partitioned ? TableProperties : Table2;
    case "column":
      return node.flags.primaryKey ? KeyRound : Columns3;
    default:
      return Folder;
  }
}

export function NodeIcon(props: { node: TreeNode }) {
  const color = () => {
    if (props.node.kind === "server") return props.node.flags.connected ? "text-accent" : "text-muted";
    return isCollection(props.node.kind) ? "text-muted" : "";
  };
  return <Dynamic component={iconFor(props.node)} size={14} class={`shrink-0 ${color()}`} aria-hidden="true" />;
}
```

`src/features/explorer/TreeRow.tsx`:

```tsx
import { ChevronDown, ChevronRight, CircleAlert, LoaderCircle } from "lucide-solid";
import { Show } from "solid-js";
import { NodeIcon } from "./NodeIcon";
import { rowLabel, type VisibleRow } from "./tree-model";

export const ROW_HEIGHT = 24;

export function rowDomId(id: string): string {
  return `tn-${encodeURIComponent(id)}`;
}

export function TreeRow(props: {
  row: VisibleRow;
  top: number;
  selected: boolean;
  onSelect: () => void;
  onToggle: () => void;
}) {
  const r = () => props.row;
  return (
    <div
      id={rowDomId(r().id)}
      role="treeitem"
      aria-level={r().depth + 1}
      aria-setsize={r().setSize}
      aria-posinset={r().posInSet}
      aria-selected={props.selected}
      aria-expanded={r().node.hasChildren ? r().expanded : undefined}
      class="absolute left-0 right-0 flex cursor-default items-center gap-1 whitespace-nowrap pr-2 select-none"
      classList={{ "bg-selected": props.selected, "hover:bg-hover": !props.selected }}
      style={{ height: `${ROW_HEIGHT}px`, transform: `translateY(${props.top}px)`, "padding-left": `${r().depth * 14 + 4}px` }}
      onClick={props.onSelect}
      onDblClick={props.onToggle}
    >
      <span
        class="flex w-4 shrink-0 justify-center text-muted"
        onClick={(e) => {
          e.stopPropagation();
          props.onSelect();
          props.onToggle();
        }}
      >
        <Show when={r().node.hasChildren}>
          <Show when={!r().loading} fallback={<LoaderCircle size={12} class="animate-spin" />}>
            <Show when={r().expanded} fallback={<ChevronRight size={12} />}>
              <ChevronDown size={12} />
            </Show>
          </Show>
        </Show>
      </span>
      <NodeIcon node={r().node} />
      <span class="truncate">{rowLabel(r())}</span>
      <Show when={r().node.detail}>
        <span class="truncate text-xs text-muted">{r().node.detail}</span>
      </Show>
      <Show when={r().error}>
        {(error) => (
          <span class="ml-auto shrink-0 text-danger" title={error().detail ? `${error().message}\n${error().detail}` : error().message}>
            <CircleAlert size={13} />
          </span>
        )}
      </Show>
    </div>
  );
}
```

`src/features/explorer/TreeView.tsx`:

```tsx
import { createVirtualizer } from "@tanstack/solid-virtual";
import { For, Show, createEffect } from "solid-js";
import { t } from "../../app/strings";
import type { Explorer } from "./explorer-store";
import { ROW_HEIGHT, TreeRow, rowDomId } from "./TreeRow";
import { ROOT } from "./tree-model";

export function TreeView(props: { explorer: Explorer }) {
  const ex = props.explorer;
  let scrollEl!: HTMLDivElement;
  const virtualizer = createVirtualizer({
    get count() {
      return ex.rows().length;
    },
    getScrollElement: () => scrollEl,
    estimateSize: () => ROW_HEIGHT,
    overscan: 12,
  });

  createEffect(() => {
    const selected = ex.state().selected;
    if (selected === null) return;
    const index = ex.rows().findIndex((r) => r.id === selected);
    if (index >= 0) virtualizer.scrollToIndex(index, { align: "auto" });
  });

  const isEmpty = () => ex.rows().length === 0 && ex.state().children.has(ROOT);

  return (
    <div
      ref={scrollEl}
      role="tree"
      aria-label={t.explorer.ariaLabel}
      aria-activedescendant={ex.state().selected !== null ? rowDomId(ex.state().selected!) : undefined}
      tabIndex={0}
      class="relative h-full overflow-auto outline-none focus-visible:ring-1 focus-visible:ring-accent"
      onKeyDown={(e) => {
        if (ex.controller.handleKey(e.key)) e.preventDefault();
      }}
    >
      <Show when={!isEmpty()} fallback={<p class="p-4 text-muted">{t.explorer.empty}</p>}>
        <div style={{ height: `${virtualizer.getTotalSize()}px`, position: "relative" }}>
          <For each={virtualizer.getVirtualItems()}>
            {(item) => (
              <Show when={ex.rows()[item.index]}>
                {(row) => (
                  <TreeRow
                    row={row()}
                    top={item.start}
                    selected={ex.state().selected === row().id}
                    onSelect={() => ex.controller.select(row().id)}
                    onToggle={() => void ex.controller.toggle(row().id)}
                  />
                )}
              </Show>
            )}
          </For>
        </div>
      </Show>
    </div>
  );
}
```

- [ ] **Step 6: Parola istemini yaz**

`src/features/servers/PasswordPrompt.tsx`:

```tsx
import { Show, createSignal } from "solid-js";
import { t } from "../../app/strings";
import type { TreeNode } from "../../ipc/bindings/TreeNode";
import { Button } from "../../ui/Button";
import { Dialog } from "../../ui/Dialog";
import { Field, inputClass } from "../../ui/Field";
import type { PasswordAnswer } from "../explorer/controller";

type Request = { node: TreeNode; retry: boolean; resolve: (answer: PasswordAnswer | null) => void };

export function createPasswordPrompt() {
  const [request, setRequest] = createSignal<Request | null>(null);
  const [password, setPassword] = createSignal("");
  const [remember, setRemember] = createSignal(false);

  const ask = (node: TreeNode, retry: boolean) =>
    new Promise<PasswordAnswer | null>((resolve) => {
      setPassword("");
      setRequest({ node, retry, resolve });
    });

  const finish = (answer: PasswordAnswer | null) => {
    const current = request();
    if (!current) return;
    setRequest(null);
    setPassword("");
    current.resolve(answer);
  };

  const View = () => (
    <Dialog
      open={request() !== null}
      onOpenChange={(open) => !open && finish(null)}
      title={t.password.title(request()?.node.label ?? "")}
      width="360px"
      footer={
        <>
          <Button onClick={() => finish(null)}>{t.server.cancel}</Button>
          <Button variant="primary" type="submit" form="pgt-password-form">
            {t.password.connect}
          </Button>
        </>
      }
    >
      <form
        id="pgt-password-form"
        class="flex flex-col gap-3"
        onSubmit={(e) => {
          e.preventDefault();
          finish({ password: password(), remember: remember() });
        }}
      >
        <Show when={request()?.retry}>
          <p class="text-danger">{t.password.retry}</p>
        </Show>
        <Field label={t.server.password}>
          <input
            type="password"
            class={inputClass}
            autofocus
            value={password()}
            onInput={(e) => setPassword(e.currentTarget.value)}
          />
        </Field>
        <label class="flex items-center gap-2">
          <input type="checkbox" checked={remember()} onChange={(e) => setRemember(e.currentTarget.checked)} />
          {t.server.savePassword}
        </label>
      </form>
    </Dialog>
  );

  return { ask, View };
}
```

- [ ] **Step 7: Araç çubuğu, durum çubuğu ve `App`'i bağla**

`src/app/Toolbar.tsx`:

```tsx
import { Moon, RefreshCw, Sun, SunMoon } from "lucide-solid";
import type { JSX } from "solid-js";
import { Button } from "../ui/Button";
import { t } from "./strings";
import type { ThemePreference } from "./theme";

export function Toolbar(props: { onRefresh: () => void; themePref: ThemePreference; onCycleTheme: () => void; children?: JSX.Element }) {
  const ThemeIcon = () => (props.themePref === "light" ? <Sun size={14} /> : props.themePref === "dark" ? <Moon size={14} /> : <SunMoon size={14} />);
  return (
    <>
      {props.children}
      <Button variant="ghost" title={t.toolbar.refresh} aria-label={t.toolbar.refresh} onClick={props.onRefresh}>
        <RefreshCw size={14} />
      </Button>
      <div class="flex-1" />
      <Button variant="ghost" title={t.toolbar.theme[props.themePref]} aria-label={t.toolbar.theme[props.themePref]} onClick={props.onCycleTheme}>
        <ThemeIcon />
      </Button>
    </>
  );
}
```

`src/app/StatusBar.tsx`:

```tsx
import { Show } from "solid-js";
import type { Explorer } from "../features/explorer/explorer-store";
import { serverOf } from "../features/explorer/tree-model";
import { t } from "./strings";

export function StatusBar(props: { explorer: Explorer; notice: string | null }) {
  const server = () => serverOf(props.explorer.state(), props.explorer.state().selected);
  const text = () => {
    const s = server();
    if (!s) return t.status.ready;
    const parts = [s.label];
    if (s.detail) parts.push(`PostgreSQL ${s.detail}`);
    parts.push(s.flags.connected ? t.status.connected : t.status.notConnected);
    return parts.join(" · ");
  };
  return (
    <Show when={props.notice} fallback={<span class="truncate">{text()}</span>}>
      <span class="truncate text-danger">{props.notice}</span>
    </Show>
  );
}
```

`src/app/App.tsx`:

```tsx
import { createSignal, onCleanup, onMount } from "solid-js";
import { createExplorer } from "../features/explorer/explorer-store";
import { TreeView } from "../features/explorer/TreeView";
import { ROOT } from "../features/explorer/tree-model";
import { createPasswordPrompt } from "../features/servers/PasswordPrompt";
import { commands } from "../ipc/commands";
import { SplitPane } from "./SplitPane";
import { StatusBar } from "./StatusBar";
import { Toolbar } from "./Toolbar";
import { applyTheme, loadThemePreference, nextPreference, saveThemePreference, watchSystemTheme } from "./theme";

export default function App() {
  const prompt = createPasswordPrompt();
  const explorer = createExplorer({ api: commands, requestPassword: prompt.ask });
  const [themePref, setThemePref] = createSignal(loadThemePreference());
  const [notice, setNotice] = createSignal<string | null>(null);

  onMount(async () => {
    await explorer.controller.load(ROOT);
    setNotice(await commands.startupNotice().catch(() => null));
  });
  onCleanup(watchSystemTheme(themePref));

  const cycleTheme = () => {
    const next = nextPreference(themePref());
    setThemePref(next);
    saveThemePreference(next);
    applyTheme(next);
  };

  const refreshSelected = () => {
    const selected = explorer.state().selected;
    void (selected === null ? explorer.controller.reloadServers() : explorer.controller.refresh(selected));
  };

  return (
    <div class="flex h-full flex-col">
      <header class="flex h-9 shrink-0 items-center gap-1 border-b border-border bg-panel px-2">
        <Toolbar onRefresh={refreshSelected} themePref={themePref()} onCycleTheme={cycleTheme} />
      </header>
      <SplitPane left={<TreeView explorer={explorer} />} right={<div class="h-full" />} />
      <footer class="flex h-6 shrink-0 items-center border-t border-border bg-panel px-2 text-xs text-muted">
        <StatusBar explorer={explorer} notice={notice()} />
      </footer>
      <prompt.View />
    </div>
  );
}
```

- [ ] **Step 8: Tip kontrolü, testler ve elle doğrulama**

Run: `bun run typecheck && bun run test`
Expected: hata yok; tüm testler PASS.

Bu task'ta sunucu ekleme arayüzü henüz yok. Elle doğrulama için `%APPDATA%\pg-tools\servers.json` dosyasına geçici olarak bir kayıt yazın (parola istemi test edilecek):

```json
{ "version": 1, "servers": [ { "id": "11111111-1111-4111-8111-111111111111", "name": "yerel", "group": null, "host": "localhost", "port": 5432, "maintenanceDb": "postgres", "username": "postgres", "savePassword": false, "ssl": { "mode": "prefer", "rootCert": null, "clientCert": null, "clientKey": null } } ] }
```

Run: `bun tauri dev`
Expected:
- "yerel" sunucusu görünür; çift tık → parola istemi açılır; yanlış parola → "Parola hatalı" uyarısıyla istem yeniden açılır; doğru parola → sunucu yanında sürüm (ör. `18.0`) ve ağaçta "Databases" görünür.
- Databases → `postgres` → Schemas → `public` açılabilir; koleksiyon etiketleri sayı gösterir ("Schemas (1)").
- Klavye: ağaca tıklayıp ↑↓ ← → Enter F5 çalışır; seçim görünür alanda kalır.
- Tema düğmesi sistem → açık → koyu arasında döner, uygulama yeniden açıldığında tercih korunur.
- Durum çubuğu seçili düğümün sunucusunu gösterir: `yerel · PostgreSQL 18.0 · bağlı`.

Geçici `servers.json` kaydını Task 14'te arayüzden yönetebileceksiniz; şimdilik bırakabilirsiniz.

- [ ] **Step 9: Commit**

```bash
git add src
git commit -m "feat(ui): sanal ağaç, parola istemi, araç ve durum çubuğu"
```

---

### Task 14: Sunucu ekleme/düzenleme diyaloğu ve bağlantı testi

**Files:**
- Create: `src/features/servers/server-form.ts`, `src/features/servers/server-form.test.ts`, `src/features/servers/ServerDialog.tsx`
- Modify: `src/app/App.tsx`

**Interfaces:**
- Consumes: `commands.{listServers, saveServer, testConnection}`, `Dialog`, `Tabs`, `TabPanel`, `Field`, `inputClass`, `Button`, `t`, `@tauri-apps/plugin-dialog` `open`
- Produces:
  - `server-form.ts`: `interface ServerForm { name; group; host; port: string; maintenanceDb; username; password; savePassword: boolean; sslMode: SslMode; rootCert; clientCert; clientKey }` (string alanlar), `emptyForm()`, `formFromConfig(ServerConfig)`, `type FormTab = "general" | "connection" | "ssl"`, `toConfig(form, id): { ok: true; config: ServerConfig; password: string | null } | { ok: false; error: string; tab: FormTab }`, `groupsOf(servers: ServerConfig[]): string[]`
  - `ServerDialog(props: { open: boolean; editing: ServerConfig | null; groups: string[]; onClose(): void; onSaved(config: ServerConfig): void })`

**Kurallar:**
- Düzenlemede parola alanı boş gelir; boş bırakılırsa `password = null` gönderilir (mevcut kayıt korunur).
- Doğrulama hatası ilgili sekmeyi açar ve mesajı gösterir. Sunucu tarafı (`invalid`) hataları da diyalogda gösterilir.
- "Bağlantıyı test et" kaydetmez; sonucu diyalog altında yeşil (başarı) veya kırmızı (hata + detay) gösterir.
- Kaydedince gezgin `reloadServers()` ile yenilenir; düzenlenen sunucu bağlıysa Rust tarafı bağlantıyı keser, ağaç modeli de alt ağacı temizler (Task 11 kuralı).

- [ ] **Step 1: Başarısız testleri yaz**

`src/features/servers/server-form.test.ts`:

```ts
import { describe, expect, it } from "vitest";
import type { ServerConfig } from "../../ipc/bindings/ServerConfig";
import { emptyForm, formFromConfig, groupsOf, toConfig } from "./server-form";

const ID = "11111111-1111-4111-8111-111111111111";

function filled() {
  return { ...emptyForm(), name: " prod ", host: "db.local", username: "app" };
}

describe("server-form", () => {
  it("has sensible defaults", () => {
    const f = emptyForm();
    expect([f.port, f.maintenanceDb, f.sslMode, f.savePassword]).toEqual(["5432", "postgres", "prefer", false]);
  });

  it("converts a valid form to a trimmed config with nulls for empty optionals", () => {
    const r = toConfig({ ...filled(), group: "  ", rootCert: "", password: "" }, ID);
    expect(r.ok).toBe(true);
    if (!r.ok) return;
    expect(r.config).toEqual({
      id: ID, name: "prod", group: null, host: "db.local", port: 5432, maintenanceDb: "postgres", username: "app",
      savePassword: false, ssl: { mode: "prefer", rootCert: null, clientCert: null, clientKey: null },
    });
    expect(r.password).toBeNull();
  });

  it("passes a typed password through", () => {
    const r = toConfig({ ...filled(), password: "s3cret" }, ID);
    expect(r.ok && r.password).toBe("s3cret");
  });

  it("reports the tab of the first invalid field", () => {
    expect(toConfig({ ...filled(), name: "" }, ID)).toMatchObject({ ok: false, tab: "general" });
    expect(toConfig({ ...filled(), host: " " }, ID)).toMatchObject({ ok: false, tab: "connection" });
    expect(toConfig({ ...filled(), port: "70000" }, ID)).toMatchObject({ ok: false, tab: "connection" });
    expect(toConfig({ ...filled(), port: "54a" }, ID)).toMatchObject({ ok: false, tab: "connection" });
    expect(toConfig({ ...filled(), username: "" }, ID)).toMatchObject({ ok: false, tab: "connection" });
    expect(toConfig({ ...filled(), clientCert: "c.pem" }, ID)).toMatchObject({ ok: false, tab: "ssl" });
  });

  it("round-trips an existing config without the password", () => {
    const config: ServerConfig = {
      id: ID, name: "p", group: "Üretim", host: "h", port: 6432, maintenanceDb: "postgres", username: "u",
      savePassword: true, ssl: { mode: "verify-full", rootCert: "C:\\ca.pem", clientCert: null, clientKey: null },
    };
    const form = formFromConfig(config);
    expect(form.password).toBe("");
    const r = toConfig(form, ID);
    expect(r.ok && r.config).toEqual(config);
  });

  it("lists distinct groups alphabetically", () => {
    const base = { id: ID, host: "h", port: 1, maintenanceDb: "postgres", username: "u", savePassword: false,
      ssl: { mode: "prefer" as const, rootCert: null, clientCert: null, clientKey: null } };
    const servers: ServerConfig[] = [
      { ...base, name: "a", group: "Test" }, { ...base, name: "b", group: "Üretim" }, { ...base, name: "c", group: "Test" }, { ...base, name: "d", group: null },
    ];
    expect(groupsOf(servers)).toEqual(["Test", "Üretim"]);
  });
});
```

- [ ] **Step 2: Testlerin başarısız olduğunu doğrula**

Run: `bun run test`
Expected: FAIL — `./server-form` yok.

- [ ] **Step 3: Form mantığını yaz**

`src/features/servers/server-form.ts`:

```ts
import type { ServerConfig } from "../../ipc/bindings/ServerConfig";
import type { SslMode } from "../../ipc/bindings/SslMode";

export interface ServerForm {
  name: string;
  group: string;
  host: string;
  port: string;
  maintenanceDb: string;
  username: string;
  password: string;
  savePassword: boolean;
  sslMode: SslMode;
  rootCert: string;
  clientCert: string;
  clientKey: string;
}

export type FormTab = "general" | "connection" | "ssl";

export type FormResult =
  | { ok: true; config: ServerConfig; password: string | null }
  | { ok: false; error: string; tab: FormTab };

export function emptyForm(): ServerForm {
  return {
    name: "", group: "", host: "", port: "5432", maintenanceDb: "postgres", username: "", password: "",
    savePassword: false, sslMode: "prefer", rootCert: "", clientCert: "", clientKey: "",
  };
}

export function formFromConfig(c: ServerConfig): ServerForm {
  return {
    name: c.name, group: c.group ?? "", host: c.host, port: String(c.port), maintenanceDb: c.maintenanceDb,
    username: c.username, password: "", savePassword: c.savePassword, sslMode: c.ssl.mode,
    rootCert: c.ssl.rootCert ?? "", clientCert: c.ssl.clientCert ?? "", clientKey: c.ssl.clientKey ?? "",
  };
}

const opt = (v: string): string | null => (v.trim() === "" ? null : v.trim());

export function toConfig(f: ServerForm, id: string): FormResult {
  const fail = (error: string, tab: FormTab): FormResult => ({ ok: false, error, tab });
  if (!f.name.trim()) return fail("Sunucu adı boş olamaz.", "general");
  if (!f.host.trim()) return fail("Host boş olamaz.", "connection");
  const port = Number(f.port.trim());
  if (!/^\d+$/.test(f.port.trim()) || port < 1 || port > 65535) return fail("Port 1-65535 arasında olmalı.", "connection");
  if (!f.maintenanceDb.trim()) return fail("Bakım veritabanı boş olamaz.", "connection");
  if (!f.username.trim()) return fail("Kullanıcı adı boş olamaz.", "connection");
  if ((opt(f.clientCert) === null) !== (opt(f.clientKey) === null)) {
    return fail("İstemci sertifikası ve anahtarı birlikte verilmelidir.", "ssl");
  }
  return {
    ok: true,
    password: f.password === "" ? null : f.password,
    config: {
      id,
      name: f.name.trim(),
      group: opt(f.group),
      host: f.host.trim(),
      port,
      maintenanceDb: f.maintenanceDb.trim(),
      username: f.username.trim(),
      savePassword: f.savePassword,
      ssl: { mode: f.sslMode, rootCert: opt(f.rootCert), clientCert: opt(f.clientCert), clientKey: opt(f.clientKey) },
    },
  };
}

export function groupsOf(servers: ServerConfig[]): string[] {
  const set = new Set(servers.map((s) => s.group?.trim()).filter((g): g is string => !!g));
  return [...set].sort((a, b) => a.localeCompare(b, "tr"));
}
```

- [ ] **Step 4: Testlerin geçtiğini doğrula**

Run: `bun run test`
Expected: PASS (server-form 6 test + öncekiler).

- [ ] **Step 5: Diyaloğu yaz**

`src/features/servers/ServerDialog.tsx`:

```tsx
import { open as openFile } from "@tauri-apps/plugin-dialog";
import { For, Show, createEffect, createSignal } from "solid-js";
import { t } from "../../app/strings";
import type { ErrorPayload } from "../../ipc/bindings/ErrorPayload";
import type { ServerConfig } from "../../ipc/bindings/ServerConfig";
import type { SslMode } from "../../ipc/bindings/SslMode";
import { commands } from "../../ipc/commands";
import { toPayload } from "../../ipc/errors";
import { Button } from "../../ui/Button";
import { Dialog } from "../../ui/Dialog";
import { Field, inputClass } from "../../ui/Field";
import { TabPanel, Tabs } from "../../ui/Tabs";
import { emptyForm, formFromConfig, toConfig, type FormTab, type ServerForm } from "./server-form";

const SSL_MODES: SslMode[] = ["disable", "prefer", "require", "verify-ca", "verify-full"];
type Feedback = { kind: "ok"; text: string } | { kind: "error"; error: ErrorPayload } | null;

export function ServerDialog(props: {
  open: boolean;
  editing: ServerConfig | null;
  groups: string[];
  onClose: () => void;
  onSaved: (config: ServerConfig) => void;
}) {
  const [form, setForm] = createSignal<ServerForm>(emptyForm());
  const [tab, setTab] = createSignal<FormTab>("general");
  const [feedback, setFeedback] = createSignal<Feedback>(null);
  const [busy, setBusy] = createSignal(false);

  createEffect(() => {
    if (!props.open) return;
    setForm(props.editing ? formFromConfig(props.editing) : emptyForm());
    setTab("general");
    setFeedback(null);
  });

  const set = <K extends keyof ServerForm>(key: K, value: ServerForm[K]) => setForm((f) => ({ ...f, [key]: value }));
  const text = (key: keyof ServerForm) => ({
    value: form()[key] as string,
    onInput: (e: InputEvent & { currentTarget: HTMLInputElement }) => set(key, e.currentTarget.value as never),
  });

  const validated = () => {
    const result = toConfig(form(), props.editing?.id ?? crypto.randomUUID());
    if (!result.ok) {
      setTab(result.tab);
      setFeedback({ kind: "error", error: { code: "invalid", message: result.error, detail: null, hint: null } });
      return null;
    }
    return result;
  };

  const runTest = async () => {
    const r = validated();
    if (!r) return;
    setBusy(true);
    setFeedback(null);
    try {
      const info = await commands.testConnection(r.config, r.password);
      setFeedback({ kind: "ok", text: t.server.testOk(info.version) });
    } catch (e) {
      setFeedback({ kind: "error", error: toPayload(e) });
    } finally {
      setBusy(false);
    }
  };

  const save = async (e: SubmitEvent) => {
    e.preventDefault();
    const r = validated();
    if (!r) return;
    setBusy(true);
    try {
      props.onSaved(await commands.saveServer(r.config, r.password));
    } catch (err) {
      setFeedback({ kind: "error", error: toPayload(err) });
    } finally {
      setBusy(false);
    }
  };

  const pick = async (key: "rootCert" | "clientCert" | "clientKey") => {
    const path = await openFile({ multiple: false, directory: false, filters: [{ name: "PEM", extensions: ["pem", "crt", "cer", "key"] }] });
    if (typeof path === "string") set(key, path);
  };

  const FileField = (p: { label: string; key: "rootCert" | "clientCert" | "clientKey" }) => (
    <Field label={p.label}>
      <div class="flex gap-1">
        <input class={inputClass} {...text(p.key)} />
        <Button onClick={() => void pick(p.key)}>{t.server.browse}</Button>
      </div>
    </Field>
  );

  return (
    <Dialog
      open={props.open}
      onOpenChange={(o) => !o && props.onClose()}
      title={props.editing ? t.server.titleEdit : t.server.titleNew}
      width="520px"
      footer={
        <>
          <Button class="mr-auto" disabled={busy()} onClick={() => void runTest()}>
            {busy() ? t.server.testing : t.server.test}
          </Button>
          <Button onClick={props.onClose}>{t.server.cancel}</Button>
          <Button variant="primary" type="submit" form="pgt-server-form" disabled={busy()}>
            {t.server.save}
          </Button>
        </>
      }
    >
      <form id="pgt-server-form" class="flex flex-col gap-3" onSubmit={(e) => void save(e)}>
        <Tabs
          value={tab()}
          onChange={(v) => setTab(v as FormTab)}
          tabs={[
            { value: "general", label: t.server.tabs.general },
            { value: "connection", label: t.server.tabs.connection },
            { value: "ssl", label: t.server.tabs.ssl },
          ]}
        >
          <TabPanel value="general" class="flex flex-col gap-3 pt-3">
            <Field label={t.server.name}>
              <input class={inputClass} autofocus {...text("name")} />
            </Field>
            <Field label={t.server.group}>
              <input class={inputClass} list="pgt-groups" {...text("group")} />
              <datalist id="pgt-groups">
                <For each={props.groups}>{(g) => <option value={g} />}</For>
              </datalist>
            </Field>
          </TabPanel>
          <TabPanel value="connection" class="grid grid-cols-[1fr_96px] gap-3 pt-3">
            <Field label={t.server.host}>
              <input class={inputClass} {...text("host")} />
            </Field>
            <Field label={t.server.port}>
              <input class={inputClass} inputMode="numeric" {...text("port")} />
            </Field>
            <div class="col-span-2">
              <Field label={t.server.maintenanceDb}>
                <input class={inputClass} {...text("maintenanceDb")} />
              </Field>
            </div>
            <div class="col-span-2">
              <Field label={t.server.username}>
                <input class={inputClass} {...text("username")} />
              </Field>
            </div>
            <div class="col-span-2">
              <Field label={t.server.password} hint={props.editing ? t.server.passwordKeep : undefined}>
                <input type="password" class={inputClass} {...text("password")} />
              </Field>
            </div>
            <label class="col-span-2 flex items-center gap-2">
              <input type="checkbox" checked={form().savePassword} onChange={(e) => set("savePassword", e.currentTarget.checked)} />
              {t.server.savePassword}
            </label>
          </TabPanel>
          <TabPanel value="ssl" class="flex flex-col gap-3 pt-3">
            <Field label={t.server.sslMode}>
              <select class={inputClass} value={form().sslMode} onChange={(e) => set("sslMode", e.currentTarget.value as SslMode)}>
                <For each={SSL_MODES}>{(m) => <option value={m}>{m}</option>}</For>
              </select>
            </Field>
            <FileField label={t.server.rootCert} key="rootCert" />
            <FileField label={t.server.clientCert} key="clientCert" />
            <FileField label={t.server.clientKey} key="clientKey" />
          </TabPanel>
        </Tabs>
        <Show when={feedback()}>
          {(f) => {
            const fb = f();
            return fb.kind === "ok" ? (
              <p class="text-[13px] text-green-600 dark:text-green-400">{fb.text}</p>
            ) : (
              <div class="text-[13px] text-danger">
                <p>{fb.error.message}</p>
                <Show when={fb.error.detail}>
                  <p class="mt-1 text-xs opacity-80">{fb.error.detail}</p>
                </Show>
              </div>
            );
          }}
        </Show>
      </form>
    </Dialog>
  );
}
```

- [ ] **Step 6: Araç çubuğuna "Sunucu ekle" / "Sunucuyu düzenle" ekle**

`src/app/App.tsx` içinde importlara ekle:

```tsx
import { Pencil, Plus } from "lucide-solid";
import { ServerDialog } from "../features/servers/ServerDialog";
import { groupsOf } from "../features/servers/server-form";
import type { ServerConfig } from "../ipc/bindings/ServerConfig";
import { Button } from "../ui/Button";
import { t } from "./strings";
```

`App` fonksiyonuna (explorer tanımından sonra):

```tsx
  const [dialogOpen, setDialogOpen] = createSignal(false);
  const [editing, setEditing] = createSignal<ServerConfig | null>(null);
  const [groups, setGroups] = createSignal<string[]>([]);

  const openDialog = async (serverId: string | null) => {
    const servers = await commands.listServers();
    setGroups(groupsOf(servers));
    setEditing(serverId === null ? null : servers.find((s) => s.id === serverId) ?? null);
    setDialogOpen(true);
  };
  const selectedServerId = () => {
    const node = explorer.selectedNode();
    return node?.kind === "server" ? node.serverId : null;
  };
```

`<Toolbar ...>` öğesini çocuklarla değiştir:

```tsx
        <Toolbar onRefresh={refreshSelected} themePref={themePref()} onCycleTheme={cycleTheme}>
          <Button variant="ghost" onClick={() => void openDialog(null)}>
            <Plus size={14} /> {t.toolbar.addServer}
          </Button>
          <Button
            variant="ghost"
            disabled={selectedServerId() === null}
            title={t.toolbar.editServer}
            aria-label={t.toolbar.editServer}
            onClick={() => void openDialog(selectedServerId())}
          >
            <Pencil size={14} />
          </Button>
        </Toolbar>
```

`<prompt.View />` satırının altına:

```tsx
      <ServerDialog
        open={dialogOpen()}
        editing={editing()}
        groups={groups()}
        onClose={() => setDialogOpen(false)}
        onSaved={() => {
          setDialogOpen(false);
          void explorer.controller.reloadServers();
        }}
      />
```

- [ ] **Step 7: Tip kontrolü ve elle doğrulama**

Run: `bun run typecheck && bun run test`
Expected: hata yok; tüm testler PASS.

Run: `bun tauri dev`
Expected:
- "Sunucu ekle" → boş form; Ad boşken Kaydet → Genel sekmesi açık kalır, "Sunucu adı boş olamaz." görünür.
- Yerel PG 18 bilgileriyle "Bağlantıyı test et" → yeşil "Bağlantı başarılı (PostgreSQL 18.x)"; yanlış parola → kırmızı "Kimlik doğrulama başarısız…".
- Grup "Test", "Parolayı kaydet" işaretli kaydet → ağaçta "Test" grubu altında sunucu; Windows Kimlik Bilgisi Yöneticisi'nde `server:<uuid>` girişi (`pg-tools` servisi) görünür; bağlanırken parola sorulmaz.
- Sunucu seçiliyken kalem → mevcut değerlerle form; parola boş bırakılıp kaydedilince bağlantı hâlâ kayıtlı parolayla çalışır.
- Bağlıyken düzenleyip kaydet → sunucu "bağlı değil" olur, alt ağaç kapanır.
- SSL sekmesinde "Gözat…" Windows dosya seçicisini açar ve yolu alana yazar.
- Task 13'te elle eklenen "yerel" kaydını düzenleyip "Parolayı kaydet" seçeneğiyle güncelleyin.

- [ ] **Step 8: Commit**

```bash
git add src
git commit -m "feat(ui): sunucu ekleme/düzenleme diyaloğu ve bağlantı testi"
```

---

### Task 15: Inspector (Properties ve tembel yüklenen DDL görünümü)

**Files:**
- Create: `src/app/debounce.ts`, `src/features/inspector/labels.ts`, `src/features/inspector/labels.test.ts`, `src/features/inspector/Inspector.tsx`, `src/features/inspector/PropertiesView.tsx`, `src/features/inspector/DdlView.tsx`, `src/features/inspector/codemirror-setup.ts`
- Modify: `src/app/App.tsx`

**Interfaces:**
- Consumes: `Explorer.selectedNode` (Task 12), `commands.{getProperties, getDdl}`, `Tabs`, `TabPanel`, `Button`, `t`
- Produces:
  - `debounce.ts`: `createDebounced<T>(source: Accessor<T>, ms: number): Accessor<T>`
  - `labels.ts`: `kindLabel(kind: NodeKind): string`
  - `Inspector(props: { explorer: Explorer })`
  - `DdlView` (default export, `lazy` ile yüklenir): `(props: { text: string }) => JSX.Element`
  - `codemirror-setup.ts`: `createReadOnlyEditor(parent, doc): EditorView`, `replaceDoc(view, doc)`

**Davranış:**
- Seçim 150 ms debounce ile izlenir; eski istek yanıtı gelse bile Solid `createResource` yalnızca son kaynağın sonucunu gösterir.
- Properties her seçimde, DDL yalnızca DDL sekmesi açıkken istenir.
- `DdlView` ve CodeMirror yalnızca DDL sekmesi ilk açıldığında yüklenir (ayrı JS parçası).
- Boş DDL (sunucu, koleksiyon) → "Bu düğüm için DDL yok."; boş Properties → "Bu düğüm için özellik yok."; hata → mesaj + detay panel içinde.
- "Kopyala" DDL'i panoya yazar ve 1,5 sn "Kopyalandı" gösterir.

- [ ] **Step 1: Başarısız testi yaz**

`src/features/inspector/labels.test.ts`:

```ts
import { describe, expect, it } from "vitest";
import { kindLabel } from "./labels";

describe("kindLabel", () => {
  it("uses PostgreSQL terms", () => {
    expect(kindLabel("table")).toBe("Table");
    expect(kindLabel("serverGroup")).toBe("Server Group");
    expect(kindLabel("tableCollection")).toBe("Tables");
    expect(kindLabel("columnCollection")).toBe("Columns");
    expect(kindLabel("database")).toBe("Database");
  });
});
```

- [ ] **Step 2: Testin başarısız olduğunu doğrula**

Run: `bun run test`
Expected: FAIL — `./labels` yok.

- [ ] **Step 3: Yardımcıları yaz**

`src/features/inspector/labels.ts`:

```ts
import type { NodeKind } from "../../ipc/bindings/NodeKind";

const LABELS: Record<NodeKind, string> = {
  serverGroup: "Server Group",
  server: "Server",
  databaseCollection: "Databases",
  database: "Database",
  schemaCollection: "Schemas",
  schema: "Schema",
  tableCollection: "Tables",
  table: "Table",
  columnCollection: "Columns",
  column: "Column",
};

export function kindLabel(kind: NodeKind): string {
  return LABELS[kind];
}
```

> `Record<NodeKind, string>` sayesinde Plan 2'de yeni bir `NodeKind` eklendiğinde bu dosya derleme hatası verir ve güncellenmesi unutulmaz.

`src/app/debounce.ts`:

```ts
import { createEffect, createSignal, on, onCleanup, type Accessor } from "solid-js";

export function createDebounced<T>(source: Accessor<T>, ms: number): Accessor<T> {
  const [value, setValue] = createSignal<T>(source());
  createEffect(
    on(
      source,
      (next) => {
        const handle = setTimeout(() => setValue(() => next), ms);
        onCleanup(() => clearTimeout(handle));
      },
      { defer: true },
    ),
  );
  return value;
}
```

Run: `bun run test`
Expected: PASS.

- [ ] **Step 4: CodeMirror kurulumunu ve DDL görünümünü yaz**

`src/features/inspector/codemirror-setup.ts`:

```ts
import { PostgreSQL, sql } from "@codemirror/lang-sql";
import { HighlightStyle, syntaxHighlighting } from "@codemirror/language";
import { EditorState } from "@codemirror/state";
import { EditorView, lineNumbers } from "@codemirror/view";
import { tags } from "@lezer/highlight";

const highlight = HighlightStyle.define([
  { tag: tags.keyword, color: "var(--syn-keyword)" },
  { tag: [tags.string, tags.special(tags.string)], color: "var(--syn-string)" },
  { tag: tags.number, color: "var(--syn-number)" },
  { tag: [tags.comment, tags.lineComment, tags.blockComment], color: "var(--syn-comment)", fontStyle: "italic" },
  { tag: [tags.typeName, tags.standard(tags.name)], color: "var(--syn-type)" },
]);

const theme = EditorView.theme({
  "&": { height: "100%", backgroundColor: "var(--bg)", color: "var(--fg)", fontSize: "12.5px" },
  ".cm-scroller": { fontFamily: '"Cascadia Mono", Consolas, monospace', lineHeight: "1.5" },
  ".cm-gutters": { backgroundColor: "var(--panel)", color: "var(--muted)", border: "none" },
  "&.cm-focused": { outline: "none" },
  ".cm-selectionBackground, &.cm-focused .cm-selectionBackground": { backgroundColor: "var(--selected)" },
});

export function createReadOnlyEditor(parent: HTMLElement, doc: string): EditorView {
  return new EditorView({
    parent,
    state: EditorState.create({
      doc,
      extensions: [
        lineNumbers(),
        EditorState.readOnly.of(true),
        EditorView.editable.of(false),
        sql({ dialect: PostgreSQL }),
        syntaxHighlighting(highlight),
        theme,
      ],
    }),
  });
}

export function replaceDoc(view: EditorView, doc: string): void {
  view.dispatch({ changes: { from: 0, to: view.state.doc.length, insert: doc } });
}
```

`src/features/inspector/DdlView.tsx`:

```tsx
import type { EditorView } from "@codemirror/view";
import { createEffect, onCleanup, onMount } from "solid-js";
import { createReadOnlyEditor, replaceDoc } from "./codemirror-setup";

export default function DdlView(props: { text: string }) {
  let host!: HTMLDivElement;
  let view: EditorView | undefined;
  onMount(() => {
    view = createReadOnlyEditor(host, props.text);
  });
  createEffect(() => {
    const text = props.text;
    if (view && view.state.doc.toString() !== text) replaceDoc(view, text);
  });
  onCleanup(() => view?.destroy());
  return <div ref={host} class="h-full min-h-0" />;
}
```

- [ ] **Step 5: Properties görünümünü ve Inspector'ı yaz**

`src/features/inspector/PropertiesView.tsx`:

```tsx
import { For } from "solid-js";
import type { PropertySection } from "../../ipc/bindings/PropertySection";

export function PropertiesView(props: { sections: PropertySection[] }) {
  return (
    <div class="flex flex-col gap-4 p-3">
      <For each={props.sections}>
        {(section) => (
          <section>
            <h3 class="mb-1 text-xs font-semibold uppercase tracking-wide text-muted">{section.title}</h3>
            <dl class="grid grid-cols-[minmax(120px,max-content)_1fr] gap-x-4 gap-y-1">
              <For each={section.rows}>
                {(row) => (
                  <>
                    <dt class="text-muted">{row.label}</dt>
                    <dd class="m-0 break-words select-text">{row.value}</dd>
                  </>
                )}
              </For>
            </dl>
          </section>
        )}
      </For>
    </div>
  );
}
```

`src/features/inspector/Inspector.tsx`:

```tsx
import { Copy } from "lucide-solid";
import { Match, Show, Suspense, Switch, createResource, createSignal, lazy } from "solid-js";
import { createDebounced } from "../../app/debounce";
import { t } from "../../app/strings";
import type { ErrorPayload } from "../../ipc/bindings/ErrorPayload";
import { commands } from "../../ipc/commands";
import { toPayload } from "../../ipc/errors";
import { Button } from "../../ui/Button";
import { TabPanel, Tabs } from "../../ui/Tabs";
import type { Explorer } from "../explorer/explorer-store";
import { kindLabel } from "./labels";
import { PropertiesView } from "./PropertiesView";

const DdlView = lazy(() => import("./DdlView"));

function ErrorView(props: { error: unknown }) {
  const payload = (): ErrorPayload => toPayload(props.error);
  return (
    <div class="p-3 text-danger">
      <p>{payload().message}</p>
      <Show when={payload().detail}>
        <p class="mt-1 text-xs opacity-80">{payload().detail}</p>
      </Show>
    </div>
  );
}

const Muted = (props: { text: string }) => <p class="p-3 text-muted">{props.text}</p>;

export function Inspector(props: { explorer: Explorer }) {
  const [tab, setTab] = createSignal("properties");
  const [copied, setCopied] = createSignal(false);
  const selected = createDebounced(() => props.explorer.selectedNode(), 150);

  const [properties] = createResource(() => selected()?.id, (id) => commands.getProperties(id));
  const [ddl] = createResource(() => (tab() === "ddl" ? selected()?.id : undefined), (id) => commands.getDdl(id));

  // Hatalı bir resource okunursa Solid hatayı fırlatır; bu yüzden önce `.error` kontrol edilir.
  const ddlText = () => (ddl.error ? undefined : ddl.latest);

  const copy = async () => {
    const text = ddlText();
    if (!text) return;
    await navigator.clipboard.writeText(text);
    setCopied(true);
    setTimeout(() => setCopied(false), 1500);
  };

  return (
    <Show when={selected()} fallback={<Muted text={t.inspector.nothingSelected} />}>
      {(node) => (
        <div class="flex h-full flex-col">
          <div class="flex h-9 shrink-0 items-center gap-2 border-b border-border px-3">
            <span class="truncate font-semibold">{node().label}</span>
            <span class="text-muted">· {kindLabel(node().kind)}</span>
            <div class="flex-1" />
            <Show when={tab() === "ddl" && ddlText()}>
              <Button variant="ghost" onClick={() => void copy()}>
                <Copy size={14} /> {copied() ? t.inspector.copied : t.inspector.copy}
              </Button>
            </Show>
          </div>
          <Tabs
            value={tab()}
            onChange={setTab}
            class="min-h-0 flex-1"
            tabs={[
              { value: "properties", label: t.inspector.properties },
              { value: "ddl", label: t.inspector.ddl },
            ]}
          >
            <TabPanel value="properties" class="min-h-0 flex-1 overflow-auto">
              <Switch>
                <Match when={properties.error}>
                  <ErrorView error={properties.error} />
                </Match>
                <Match when={properties.loading && !properties.latest}>
                  <Muted text={t.inspector.loading} />
                </Match>
                <Match when={properties.latest?.length === 0}>
                  <Muted text={t.inspector.noProperties} />
                </Match>
                <Match when={properties.latest}>
                  {(sections) => <PropertiesView sections={sections()} />}
                </Match>
              </Switch>
            </TabPanel>
            <TabPanel value="ddl" class="min-h-0 flex-1">
              <Switch>
                <Match when={ddl.error}>
                  <ErrorView error={ddl.error} />
                </Match>
                <Match when={ddl.loading && ddl.latest === undefined}>
                  <Muted text={t.inspector.loading} />
                </Match>
                <Match when={ddl.latest === ""}>
                  <Muted text={t.inspector.noDdl} />
                </Match>
                <Match when={ddl.latest}>
                  {(text) => (
                    <Suspense fallback={<Muted text={t.inspector.loading} />}>
                      <DdlView text={text()} />
                    </Suspense>
                  )}
                </Match>
              </Switch>
            </TabPanel>
          </Tabs>
        </div>
      )}
    </Show>
  );
}
```

`src/app/App.tsx` içinde:

```tsx
import { Inspector } from "../features/inspector/Inspector";
```

ve `SplitPane`'in `right` değerini değiştir:

```tsx
      <SplitPane left={<TreeView explorer={explorer} />} right={<Inspector explorer={explorer} />} />
```

- [ ] **Step 6: Tip kontrolü, derleme ve elle doğrulama**

Run: `bun run typecheck && bun run test && bun run build`
Expected: hata yok; `dist/assets/` altında CodeMirror'ı içeren **ayrı** bir `DdlView-*.js` parçası oluşur (ana paket CodeMirror içermez).

Run: `bun tauri dev`
Expected:
- Tablo seçince sağda başlık `customers · Table`, Properties sekmesinde Genel/Tanım/İstatistik bölümleri.
- ↓ ile hızla gezinirken Properties yalnızca durunca güncellenir (150 ms).
- DDL sekmesi: sözdizimi renkli, satır numaralı, salt okunur DDL; "Kopyala" → Not Defteri'ne yapıştırınca aynı metin; buton 1,5 sn "Kopyalandı" der.
- Sunucu veya "Tables" koleksiyonu seçiliyken DDL sekmesi "Bu düğüm için DDL yok." der.
- Koyu temada DDL renkleri okunur.
- Bağlıyken PostgreSQL servisini durdurup bir tablo seçin → panelde anlaşılır hata (ör. "Sunucuya ulaşılamadı: …"); servisi başlatıp tekrar seçin → yeniden bağlanıp gösterir.

- [ ] **Step 7: Commit**

```bash
git add src
git commit -m "feat(ui): Inspector — Properties ve tembel yüklenen DDL görünümü"
```

---

### Task 16: Uçtan uca doğrulama, sürüm derlemesi ve duman testi listesi

**Files:**
- Create: `docs/smoke-test.md`
- Modify: `README.md` (yoksa oluştur)

**Interfaces:**
- Consumes: tüm önceki task'lar
- Produces: Plan 1 kabul kanıtı (test çıktıları, paket boyutu, açılış/bellek ölçümü)

- [ ] **Step 1: Tüm otomatik testleri çalıştır**

```powershell
$env:PGT_TEST_URLS = "postgres://postgres:PAROLA@localhost:5432/postgres"
cargo test --workspace --all-features
bun run typecheck
bun run test
```

Expected: tüm Rust ve TS testleri PASS; hiçbir entegrasyon testi "atlanıyor" yazmaz.

- [ ] **Step 2: Sürüm paketini derle ve boyutu ölç**

Run: `bun tauri build`
Expected: `target/release/bundle/nsis/pg-tools_0.1.0_x64-setup.exe` oluşur.

Run (PowerShell): `"{0:N1} MB" -f ((Get-Item target/release/bundle/nsis/*.exe).Length / 1MB)`
Expected: **< 15 MB** (spec §1.3). Aşıyorsa `cargo bloat` ile en büyük crate'leri bulun ve sonucu kaydedin; ölçümü düzeltmeden sonraki adıma geçmeyin.

- [ ] **Step 3: Açılış süresi ve belleği ölç**

Kurulumu yapın ve uygulamayı kapatın. Ardından PowerShell'de:

```powershell
$exe = "$env:LOCALAPPDATA\pg-tools\pg-tools.exe"
$sw = [Diagnostics.Stopwatch]::StartNew(); $p = Start-Process $exe -PassThru
while (-not $p.MainWindowHandle -or $p.MainWindowHandle -eq 0) { Start-Sleep -Milliseconds 20; $p.Refresh() }
"Pencere: {0} ms" -f $sw.ElapsedMilliseconds
Start-Sleep -Seconds 5
$ids = @($p.Id) + (Get-CimInstance Win32_Process | Where-Object { $_.ParentProcessId -eq $p.Id } | ForEach-Object ProcessId)
$webview = Get-CimInstance Win32_Process | Where-Object { $_.Name -eq "msedgewebview2.exe" -and $ids -contains $_.ParentProcessId } | ForEach-Object ProcessId
"Bellek: {0:N0} MB" -f ((Get-Process -Id ($ids + $webview) | Measure-Object WorkingSet64 -Sum).Sum / 1MB)
"Rust süreci: {0:N0} MB" -f ((Get-Process -Id $p.Id).WorkingSet64 / 1MB)
```

> Kurulum yolu NSIS varsayılanına göre değişebilir; `Get-ChildItem $env:LOCALAPPDATA -Recurse -Filter pg-tools.exe -Depth 2` ile bulun. WebView2 alt süreçleri yeni süreç ağacında torun olarak da görünebilir; toplam belleği Görev Yöneticisi'nde "pg-tools" grubu altında da doğrulayın.

Expected: pencere **< 1000 ms**, toplam **< 150 MB**, Rust süreci **< 30 MB** (boşta, sunucu bağlı değil). Değerleri `docs/smoke-test.md` sonundaki "Ölçümler" tablosuna yazın.

- [ ] **Step 4: Duman testi listesini yaz ve uygula**

`docs/smoke-test.md`:

```markdown
# pg-tools — Duman Testi (Plan 1)

Her sürümden önce elle uygulanır. Ortam: Windows 10/11 veya Server 2019/2022, PostgreSQL 10–18 sunucusu.

## Sunucu yönetimi
- [ ] "Sunucu ekle" → zorunlu alanlar boşken Kaydet hata gösterir, doğru sekmeyi açar
- [ ] "Bağlantıyı test et" doğru parolada sürümü, yanlış parolada Türkçe kimlik doğrulama hatasını gösterir
- [ ] "Parolayı kaydet" ile kaydedilen sunucu bir sonraki açılışta parola sormadan bağlanır
- [ ] Parolası kaydedilmemiş sunucu bağlanırken parola ister; yanlış parola istemi yeniden açar
- [ ] Grup adı verilen sunucular grup altında, diğerleri kökte alfabetik listelenir
- [ ] Bağlı sunucuyu düzenleyip kaydetmek bağlantıyı keser ve alt ağacı kapatır

## Gezinme
- [ ] Server → Databases → Database → Schemas → Schema → Tables → Table → Columns açılır
- [ ] Koleksiyon etiketleri sayı gösterir: "Tables (1.243)"
- [ ] Partition'lar "Tables" listesinde görünmez; partitioned tablo farklı ikonla görünür
- [ ] `datallowconn = false` veritabanı genişletilemez
- [ ] Klavye: ↑ ↓ ← → Enter Home End F5 çalışır
- [ ] 1.000+ tablolu şemada kaydırma akıcıdır

## Inspector
- [ ] Properties: Database, Schema, Table, Column için bölümler doluyor
- [ ] DDL: Database, Schema, Table, Column; tırnaklı adlar (`"Order Items"`) doğru
- [ ] DDL "Kopyala" panoya yazar
- [ ] Sunucu/koleksiyon seçiliyken "DDL yok" mesajı

## Dayanıklılık
- [ ] PostgreSQL servisini yeniden başlat → sonraki işlem hatasız sonuç verir (sessiz yeniden bağlanma)
- [ ] Sunucu tamamen kapalıyken işlem → sunucu düğümünde hata rozeti, uygulamanın geri kalanı çalışır
- [ ] `%APPDATA%\pg-tools\servers.json` bozulduğunda uygulama açılır, durum çubuğunda yedek uyarısı görünür
- [ ] `pg_stat_activity`'de `application_name = pg-tools` ve oturum salt okunur
- [ ] `%LOCALAPPDATA%\pg-tools\logs` altında log var; içinde parola yok

## Görünüm
- [ ] Tema düğmesi sistem/açık/koyu arasında döner; tercih kalıcı
- [ ] Sol panel genişliği sürüklenebilir ve kalıcı

## Ölçümler

| Tarih | Makine | Paket (MB) | Açılış (ms) | Toplam bellek (MB) | Rust süreci (MB) |
|---|---|---|---|---|---|
|  |  |  |  |  |  |
```

Listeyi baştan sona uygulayın; başarısız madde varsa düzeltmeden Plan 1'i tamamlanmış saymayın.

`README.md`:

```markdown
# pg-tools

pgAdmin4'e hafif, şirket içi alternatif: Tauri 2 + SolidJS + Rust.

## Geliştirme

    bun install
    bun tauri dev

## Testler

    cargo test --workspace --all-features   # PGT_TEST_URLS tanımlı değilse entegrasyon testleri atlanır
    bun run test
    bun run typecheck

`PGT_TEST_URLS`: virgülle ayrılmış `postgres://kullanici:parola@host:port/postgres` adresleri. Test kullanıcısı CREATEDB ve CREATEROLE yetkisine sahip olmalı.

Belgeler: `docs/superpowers/specs/`, `docs/superpowers/plans/`, `docs/smoke-test.md`.
```

- [ ] **Step 5: Commit**

```bash
git add docs/smoke-test.md README.md
git commit -m "docs: duman testi listesi, ölçümler ve README"
```

