//! Saf argüman/ortam üreticisi ve kural doğrulaması (spec §1 "Yedekleme", "Geri yükleme", "Ortak kurallar").
//!
//! Dış süreç ya da dosya sistemi kullanılmaz. Parola hiçbir çıktıda yoktur; yürütücü `PGPASSWORD` olarak ekler.
//! Nesne adları ve yollar ayrı argümandır; kabuk kullanılmaz.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use pgt_ddl::{FieldError, Risk};

use crate::detect::SourceFormat;
use crate::model::{ToolKind, ToolSet};

/// Atomik yazımda geçici çıktı eki.
pub const PARTIAL_SUFFIX: &str = ".pgt-partial";

/// `pg_dump` çıktı formatı.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum DumpFormat {
    Custom,
    Plain,
    Directory,
    Tar,
}

/// Yedekleme kapsamı (Explorer düğümü).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(tag = "kind", rename_all = "snake_case")]
#[ts(export)]
pub enum BackupScope {
    Database,
    Schema {
        name: String,
    },
    /// Tablo, MV ya da sequence.
    Table {
        schema: String,
        name: String,
    },
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct BackupOptions {
    pub schema_only: bool,
    pub data_only: bool,
    pub no_owner: bool,
    pub no_privileges: bool,
    /// `clean` gerektirir.
    pub if_exists: bool,
    /// Yalnız plain.
    pub clean: bool,
    /// Plain'de veritabanını oluşturur; diğer formatlarda arşive yazılır.
    pub create: bool,
    /// Yalnız plain; `column_inserts` ile birlikte olmaz.
    pub inserts: bool,
    /// Yalnız plain.
    pub column_inserts: bool,
    /// 0–9; tar'da yok.
    pub compress: Option<u8>,
    /// 1–16; yalnız directory.
    pub jobs: Option<u8>,
    pub encoding: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct BackupRequest {
    pub scope: BackupScope,
    pub format: DumpFormat,
    pub options: BackupOptions,
    pub output_path: String,
}

/// Geri yükleme hedefi.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum RestoreTarget {
    /// İstek bağlamındaki (düğüm ya da seçici) veritabanı.
    Existing,
    /// `--create`: bakım veritabanına bağlanılır, veritabanı arşivdeki adla oluşturulur.
    CreateFromArchive,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct RestoreOptions {
    pub clean: bool,
    /// `clean` gerektirir.
    pub if_exists: bool,
    /// Yalnız `CreateFromArchive` hedefiyle (hedef zaten `--create` demektir).
    pub create: bool,
    pub single_transaction: bool,
    /// 1–16; `single_transaction` ile birlikte olmaz; tar'da yok.
    pub jobs: Option<u8>,
    pub schema_only: bool,
    pub data_only: bool,
    pub no_owner: bool,
    pub no_privileges: bool,
    pub exit_on_error: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct RestoreRequest {
    pub path: String,
    pub target: RestoreTarget,
    pub options: RestoreOptions,
}

/// Bağlantı bilgisi (parolasız). `sslmode`: `disable|prefer|require|verify-ca|verify-full`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Conn {
    pub host: String,
    pub port: u16,
    pub user: String,
    pub database: String,
    pub sslmode: &'static str,
    pub root_cert: Option<String>,
    pub client_cert: Option<String>,
    pub client_key: Option<String>,
}

impl Conn {
    /// Sunucu kaydından bağlantı bilgisi. SSL eşlemesi uygulamanın kendi doğrulama anlamını izler
    /// (`pgt_core::tls`):
    /// - `disable`/`prefer`/`require`: sunucu sertifikası doğrulanmaz. libpq'da `require` ile `PGSSLROOTCERT`
    ///   verilirse `verify-ca` gibi davranır; bu yüzden kök sertifika yalnız `verify-*` modlarında geçirilir.
    /// - `verify-ca`/`verify-full`: kayıttaki kök sertifika `PGSSLROOTCERT` olur. Kök sertifika yoksa uygulama
    ///   sistem deposunu kullanır; libpq 16+ bunu `PGSSLROOTCERT=system` ile yapar ama yalnız `verify-full`
    ///   ile. `verify-ca` + kök sertifikasız kayıtta libpq varsayılan dosyasına (`%APPDATA%/postgresql/root.crt`)
    ///   bakar; yoksa araç bunu kendi hatasıyla bildirir. 16'dan eski araçta `system` geçirilmez.
    /// - İstemci sertifikası/anahtarı `disable` dışında geçirilir (uygulama da her TLS modunda gönderir).
    pub fn from_server(config: &pgt_core::config::ServerConfig, database: &str, tool_version_num: u32) -> Conn {
        use pgt_core::config::SslMode;
        let ssl = &config.ssl;
        let (sslmode, root_cert) = match ssl.mode {
            SslMode::Disable => ("disable", None),
            SslMode::Prefer => ("prefer", None),
            SslMode::Require => ("require", None),
            SslMode::VerifyCa => ("verify-ca", ssl.root_cert.clone()),
            SslMode::VerifyFull => (
                "verify-full",
                ssl.root_cert
                    .clone()
                    .or_else(|| (tool_version_num >= 160_000).then(|| "system".to_string())),
            ),
        };
        let tls = ssl.mode != SslMode::Disable;
        Conn {
            host: config.host.clone(),
            port: config.port,
            user: config.username.clone(),
            database: database.to_string(),
            sslmode,
            root_cert,
            client_cert: ssl.client_cert.clone().filter(|_| tls),
            client_key: ssl.client_key.clone().filter(|_| tls),
        }
    }
}

/// Atomik çıktı planı: araç `partial_path`'e yazar, başarıda `final_path`'e taşınır.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutputPlan {
    pub final_path: String,
    pub partial_path: String,
    pub is_dir: bool,
}

/// Çalıştırılacak komut. `env` parolasızdır; `preview` kullanıcıya gösterilen tek satırdır.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Invocation {
    /// Bulunan araç kümesindeki tam yol.
    pub exe: String,
    pub args: Vec<String>,
    pub env: Vec<(String, String)>,
    /// Yürütücünün miras ortamdan kaldıracağı (PG* temizliğine ek) anahtarlar. `LC_ALL` `LC_MESSAGES=C`'yi
    /// geçersiz kılar (ölçüldü: `LC_ALL=tr_TR.UTF-8` ile araç Türkçe yazar); `LANGUAGE` da gettext'i etkiler.
    pub env_remove: Vec<String>,
    pub preview: String,
    pub risk: Risk,
    pub notes: Vec<String>,
    pub output: Option<OutputPlan>,
    /// Çalıştırılan araç (ipucu metni araca göre seçilir).
    pub tool: ToolKind,
    /// `pg_restore` "hatada dur" kapalı: hatalar atlanır ve araç sonunda `errors ignored on restore: N` yazıp
    /// 1 ile çıkar. Yürütücü bu durumu `completed_with_errors` sayar (bkz. `runner`).
    pub tolerates_ignored_errors: bool,
}

/// Bağlantı zaman aşımı (sn), spec §1.
const CONNECT_TIMEOUT: &str = "15";

/// `pg_dump -n/-t` desen kaçışı: ad çift tırnak içine alınır, içteki `"` iki kez yazılır. pg_dump/psql desen
/// sözdiziminde (`processSQLNamePattern`) tırnak içindeki her karakter düzdür: `*`, `?`, `.` ve düzenli ifade
/// karakterleri joker sayılmaz, büyük/küçük harf korunur. Şema+tablo için `"şema"."ad"` biçiminde birleştirilir.
///
/// **ASCII dışı karakterler** tırnak dışında `?` (tek karakter joker) olur: `"B"?"y"?"k Ad"`. Ölçüldü: Windows'ta
/// araçlar argv'yi ANSI kod sayfasında (ör. 1254) alır ve baytları olduğu gibi UTF-8 bağlantıya yollar; `ü` içeren
/// desen sunucuda "invalid byte sequence for encoding UTF8" ile düşer. Joker yalnız o karaktere denk gelir; aynı
/// desene başka nesne de uyuyorsa çağıran ([`pattern_regex`] ile) bunu reddetmelidir.
pub fn dump_pattern(ident: &str) -> String {
    let mut out = String::with_capacity(ident.len() + 2);
    let mut open = false;
    for c in ident.chars() {
        if c.is_ascii() {
            if !open {
                out.push('"');
                open = true;
            }
            if c == '"' {
                out.push('"');
            }
            out.push(c);
        } else {
            if open {
                out.push('"');
                open = false;
            }
            out.push('?');
        }
    }
    if open || ident.is_empty() {
        out.push('"');
        if ident.is_empty() {
            out.push('"');
        }
    }
    out
}

/// [`dump_pattern`] deseninden pg_dump'ın sunucuda kullandığı tam eşleşme düzenli ifadesi (`^(…)$`): ASCII dışı
/// karakter `.`, düzenli ifade özel karakterleri kaçışlı. Joker deseninin kaç nesneye uyduğunu saymak içindir.
pub fn pattern_regex(ident: &str) -> String {
    let mut out = String::from("^(");
    for c in ident.chars() {
        if !c.is_ascii() {
            out.push('.');
        } else {
            if "|*+?()[]{}.^$\\".contains(c) {
                out.push('\\');
            }
            out.push(c);
        }
    }
    out.push_str(")$");
    out
}

/// Windows'ta istemci araçları ortam ve argv değerlerini ANSI kod sayfasına çevirir: ASCII dışı veritabanı ya da
/// kullanıcı adı sunucuya bozuk bayt olarak gider ve bağlantı kurulamaz (ölçüldü). Bunlar plan hatasıdır.
pub fn windows_name_problems(conn: &Conn) -> Vec<FieldError> {
    let mut errors = Vec::new();
    if !conn.database.is_ascii() {
        errors.push(field(
            "database",
            "Windows'ta PostgreSQL araçları ASCII dışı karakter içeren veritabanı adına bağlanamaz.",
        ));
    }
    if !conn.user.is_ascii() {
        errors.push(field("user", "Windows'ta PostgreSQL araçları ASCII dışı karakter içeren kullanıcı adıyla bağlanamaz."));
    }
    errors
}

/// Windows kod sayfasında birebir temsil edilemeyen yol iletisi (bkz. [`crate::ansi`]).
pub const ANSI_PATH_MESSAGE: &str = "Bu yol Windows kod sayfasında güvenle temsil edilemiyor; araç onu farklı karakterlerle \
     alır. Yalnız sistem kod sayfasındaki karakterleri içeren bir yol seçin.";
const ANSI_VALUE_MESSAGE: &str = "Bu değer Windows kod sayfasında güvenle temsil edilemiyor; araç onu farklı karakterlerle alır.";

/// Ortam anahtarının plan alan yolu (ANSI denetimi); sabit ASCII değerler atlanır. `PGDATABASE`/`PGUSER` da
/// denetlenir: çağıran [`windows_name_problems`]'i (daha katı) çağırmasa bile kitaplık kendi başına kapalı kalır.
/// `--dbname` bağlantı dizesi argümanı `PGDATABASE` ile aynı adı ve yalnız ASCII tırnak/kaçış ekler.
fn env_field(key: &str) -> Option<&'static str> {
    match key {
        "PGHOST" => Some("host"),
        "PGDATABASE" => Some("database"),
        "PGUSER" => Some("user"),
        "PGSSLROOTCERT" => Some("ssl.rootCert"),
        "PGSSLCERT" => Some("ssl.clientCert"),
        "PGSSLKEY" => Some("ssl.clientKey"),
        _ => None,
    }
}

/// Kurulan çağrının ANSI gidiş-dönüş denetimi (Windows'ta best-fit argv enjeksiyonuna karşı; diğer sistemlerde
/// etkisiz). Yol alanları çağıranca ayrıca (alan yoluyla) denetlenir; bu süpürme çalıştırılabilir yolu, ortam
/// değerlerini ve geri kalan her argümanı kapsar. Yürütücü başlatmadan önce aynı denetimi yineler (son savunma).
fn ansi_sweep(exe: &str, args: &[String], env: &[(String, String)], errors: &mut Vec<FieldError>) {
    use crate::ansi::ensure_ansi_roundtrip as ok;
    if !ok(exe) {
        errors.push(field("tool", ANSI_PATH_MESSAGE));
    }
    for (key, value) in env {
        if let Some(path) = env_field(key)
            && !ok(value)
        {
            let msg = if key.starts_with("PGSSL") { ANSI_PATH_MESSAGE } else { ANSI_VALUE_MESSAGE };
            errors.push(field(path, msg));
        }
    }
    // Yol alanı hatası zaten yazıldıysa aynı argüman için ikinci, genel hata eklenmez.
    if errors.is_empty()
        && args.iter().any(|a| !ok(a))
    {
        errors.push(field("request", ANSI_VALUE_MESSAGE));
    }
}

fn check_ansi_path(path: &str, name: &str, errors: &mut Vec<FieldError>) {
    if !path.is_empty() && !crate::ansi::ensure_ansi_roundtrip(path) {
        errors.push(field(name, ANSI_PATH_MESSAGE));
    }
}

/// Düz SQL yedeği için sunucu sürümü notu: 17+ `pg_dump` düz çıktıya `SET transaction_timeout = 0;` yazar
/// (hedef sunucu bilinmez); 17'den eski sunucu bunu tanımaz ve `psql` (`ON_ERROR_STOP`) ilk satırda durur.
/// Arşiv formatlarında `pg_restore` bağlandığı sunucuya göre davranır; not yalnız plain içindir.
pub const PLAIN_OLD_SERVER_NOTE: &str = "Bu yedek `SET transaction_timeout` içerir; PostgreSQL < 17 sunucuya düz SQL olarak geri \
     yüklenemez. Aynı ana sürüm pg_dump kullanın ya da custom formatı seçin.";

/// Plain yedek, 17+ `pg_dump` ve 17'den eski sunucu ise [`PLAIN_OLD_SERVER_NOTE`] (sunucu sürümü 0: bilinmiyor).
pub fn plain_backup_note(format: DumpFormat, tool_version_num: u32, server_version_num: u32) -> Option<&'static str> {
    (format == DumpFormat::Plain && tool_version_num >= 170_000 && server_version_num > 0 && server_version_num < 170_000)
        .then_some(PLAIN_OLD_SERVER_NOTE)
}

/// Düz SQL geri yükleme güvenlik notu (psql meta komutları).
pub const PLAIN_META_NOTE: &str = "Düz SQL dosyası psql meta komutları (\\!, \\connect, \\copy … program) çalıştırabilir; \
     yalnız güvendiğiniz dosyaları yükleyin.";

fn field(path: &str, message: impl Into<String>) -> FieldError {
    FieldError {
        path: path.into(),
        message: message.into(),
    }
}

/// Mutlak yol doğrulaması; klasörde sondaki ayraçlar atılır. Göreli yol CWD'ye bağlıdır ve `-` ile başlarsa
/// araç onu seçenek sanabilir; reddedilir.
fn check_path(path: &str, name: &str, is_dir: bool, errors: &mut Vec<FieldError>) -> String {
    if path.trim().is_empty() {
        errors.push(field(name, "Yol boş olamaz."));
        return String::new();
    }
    if path.contains('\0') {
        errors.push(field(name, "Yol geçersiz karakter içeriyor."));
        return String::new();
    }
    let normalized = if is_dir {
        path.trim_end_matches(['\\', '/'])
    } else {
        path
    };
    if !is_dir && path.ends_with(['\\', '/']) {
        errors.push(field(name, "Dosya yolu klasör ayracıyla bitemez."));
    } else if !std::path::Path::new(normalized).is_absolute() {
        errors.push(field(name, "Tam (mutlak) yol gerekir."));
    }
    normalized.to_string()
}

fn check_name(value: &str, name: &str, errors: &mut Vec<FieldError>) {
    if value.trim().is_empty() {
        errors.push(field(name, "Ad boş olamaz."));
    } else if value.contains('\0') {
        errors.push(field(name, "Ad geçersiz karakter içeriyor."));
    }
}

fn check_jobs(jobs: Option<u8>) -> Option<&'static str> {
    match jobs {
        Some(n) if !(1..=16).contains(&n) => Some("Paralel iş sayısı 1–16 olmalı."),
        _ => None,
    }
}

/// Ortak ortam (parolasız). `LC_MESSAGES=C`: istemci iletileri İngilizce olsun; satır sınıflandırma, ilerleme
/// ve ipuçları bu iletilere dayanır (Windows'taki PostgreSQL araçları yerel dilde yazar). Miras `LC_ALL`
/// bunu ezdiğinden `env_remove` ile kaldırılır.
/// Miras ortamdan kaldırılacak yerel ayar anahtarları (bkz. `Invocation::env_remove`).
pub const LOCALE_OVERRIDES: [&str; 2] = ["LC_ALL", "LANGUAGE"];

fn env_remove() -> Vec<String> {
    LOCALE_OVERRIDES.iter().map(|k| k.to_string()).collect()
}

fn base_env(conn: &Conn, database: &str, app: &str) -> Vec<(String, String)> {
    let mut env = vec![
        ("PGHOST".to_string(), conn.host.clone()),
        ("PGPORT".to_string(), conn.port.to_string()),
        ("PGUSER".to_string(), conn.user.clone()),
        ("PGDATABASE".to_string(), database.to_string()),
        ("PGSSLMODE".to_string(), conn.sslmode.to_string()),
    ];
    for (key, value) in [
        ("PGSSLROOTCERT", &conn.root_cert),
        ("PGSSLCERT", &conn.client_cert),
        ("PGSSLKEY", &conn.client_key),
    ] {
        if let Some(v) = value {
            env.push((key.to_string(), v.clone()));
        }
    }
    env.push(("PGAPPNAME".to_string(), app.to_string()));
    env.push(("PGCONNECT_TIMEOUT".to_string(), CONNECT_TIMEOUT.to_string()));
    env.push(("LC_MESSAGES".to_string(), "C".to_string()));
    env
}

/// Önizleme için tırnaklama: güvenli ASCII karakterler çıplak, diğerleri `"…"` içinde; denetim karakterleri
/// kaçışlanır (önizleme tek satırdır). `"` öncesindeki (kapanış tırnağı dahil) ters bölüler Windows argv kuralıyla
/// ikilenir (`"C:\dir\\"`), böylece kapanış tırnağı kaçışlı görünmez. Yalnız gösterimdir; çalıştırmada kabuk
/// kullanılmaz.
fn quote_preview(s: &str) -> String {
    let safe = |c: char| c.is_ascii_alphanumeric() || "-_=./:\\,+@%".contains(c);
    if !s.is_empty() && s.chars().all(safe) {
        return s.to_string();
    }
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    let mut backslashes = 0usize;
    for c in s.chars() {
        if c == '\\' {
            backslashes += 1;
            out.push(c);
            continue;
        }
        if c == '"' {
            out.push_str(&"\\".repeat(backslashes));
        }
        backslashes = 0;
        match c {
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if c.is_control() => out.push_str(&format!("\\u{{{:x}}}", c as u32)),
            c => out.push(c),
        }
    }
    out.push_str(&"\\".repeat(backslashes));
    out.push('"');
    out
}

/// Tek satır önizleme: ortam özeti (`PGPASSWORD=***` dahil) + tırnaklı komut.
fn preview(exe: &str, args: &[String], env: &[(String, String)]) -> String {
    let mut parts: Vec<String> = env
        .iter()
        .map(|(k, v)| format!("{k}={}", quote_preview(v)))
        .collect();
    parts.push("PGPASSWORD=***".to_string());
    parts.push(quote_preview(exe));
    parts.extend(args.iter().map(|a| quote_preview(a)));
    parts.join(" ")
}

/// `pg_restore -d` değeri: `=` içeren ya da `postgresql://` ile başlayan ad bağlantı dizesi sayılır; bunu önlemek
/// için ad her zaman `dbname='…'` biçiminde (ters bölü ve tek tırnak kaçışlı) verilir. Diğer değerler ortamdan gelir.
fn conninfo_dbname(db: &str) -> String {
    format!("dbname='{}'", db.replace('\\', "\\\\").replace('\'', "\\'"))
}

fn exe(tool: &ToolSet, kind: ToolKind) -> String {
    crate::tools::exe_path(&tool.dir, kind)
        .to_string_lossy()
        .into_owned()
}

fn check_tool(tool: &ToolSet, kind: ToolKind, errors: &mut Vec<FieldError>) {
    if !kind.has(tool) {
        errors.push(field(
            "tool",
            format!("Seçilen araç klasöründe {} yok.", kind.program()),
        ));
    }
}

/// `pg_dump` çağrısı (spec §1 "Yedekleme"). Çıktı önce `<hedef>.pgt-partial`'a yazılır.
pub fn backup_invocation(
    tool: &ToolSet,
    conn: &Conn,
    req: &BackupRequest,
) -> Result<Invocation, Vec<FieldError>> {
    let mut errors = Vec::new();
    check_tool(tool, ToolKind::PgDump, &mut errors);
    match &req.scope {
        BackupScope::Database => {}
        BackupScope::Schema { name } => check_name(name, "scope.name", &mut errors),
        BackupScope::Table { schema, name } => {
            check_name(schema, "scope.schema", &mut errors);
            check_name(name, "scope.name", &mut errors);
        }
    }
    let is_dir = req.format == DumpFormat::Directory;
    let final_path = check_path(&req.output_path, "outputPath", is_dir, &mut errors);
    check_ansi_path(&final_path, "outputPath", &mut errors);

    let o = &req.options;
    let plain = req.format == DumpFormat::Plain;
    const PLAIN_ONLY: &str = "Yalnız düz SQL (plain) formatında kullanılabilir.";
    if o.schema_only && o.data_only {
        errors.push(field(
            "options.dataOnly",
            "Yalnız şema ile yalnız veri birlikte seçilemez.",
        ));
    }
    if o.clean && !plain {
        errors.push(field("options.clean", PLAIN_ONLY));
    } else if o.clean && o.data_only {
        errors.push(field(
            "options.clean",
            "Temizle (--clean) yalnız veri ile birlikte kullanılamaz.",
        ));
    }
    if o.inserts && !plain {
        errors.push(field("options.inserts", PLAIN_ONLY));
    }
    if o.column_inserts && !plain {
        errors.push(field("options.columnInserts", PLAIN_ONLY));
    } else if o.column_inserts && o.inserts {
        errors.push(field(
            "options.columnInserts",
            "--inserts ile --column-inserts birlikte seçilemez.",
        ));
    }
    // --if-exists -c gerektirir, -c yalnız plain: diğer formatlarda tek, net hata.
    if o.if_exists && !plain {
        errors.push(field(
            "options.ifExists",
            "--if-exists yalnız düz SQL (plain) formatında temizle (--clean) ile kullanılabilir.",
        ));
    } else if o.if_exists && !o.clean {
        errors.push(field(
            "options.ifExists",
            "--if-exists, temizle (--clean) seçeneğini gerektirir.",
        ));
    }
    if let Some(z) = o.compress {
        if req.format == DumpFormat::Tar {
            errors.push(field("options.compress", "Tar formatında sıkıştırma yok."));
        } else if z > 9 {
            errors.push(field("options.compress", "Sıkıştırma düzeyi 0–9 olmalı."));
        }
    }
    if o.jobs.is_some() && !is_dir {
        errors.push(field(
            "options.jobs",
            "Paralel iş yalnız directory formatında kullanılabilir.",
        ));
    } else if let Some(msg) = check_jobs(o.jobs) {
        errors.push(field("options.jobs", msg));
    }
    let encoding = o
        .encoding
        .as_deref()
        .map(str::trim)
        .filter(|e| !e.is_empty());
    if let Some(e) = encoding
        && (e.len() > 64
            || e.starts_with('-')
            || !e
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-'))
    {
        errors.push(field("options.encoding", "Geçersiz kodlama adı."));
    }
    if !errors.is_empty() {
        return Err(errors);
    }

    let partial_path = format!("{final_path}{PARTIAL_SUFFIX}");
    let format = match req.format {
        DumpFormat::Custom => "custom",
        DumpFormat::Plain => "plain",
        DumpFormat::Directory => "directory",
        DumpFormat::Tar => "tar",
    };
    let mut args: Vec<String> = vec![
        "--verbose".into(),
        format!("--format={format}"),
        "--file".into(),
        partial_path.clone(),
    ];
    let flags = [
        (o.schema_only, "--schema-only"),
        (o.data_only, "--data-only"),
        (o.clean, "--clean"),
        (o.if_exists, "--if-exists"),
        (o.create, "--create"),
        (o.inserts, "--inserts"),
        (o.column_inserts, "--column-inserts"),
        (o.no_owner, "--no-owner"),
        (o.no_privileges, "--no-privileges"),
    ];
    args.extend(
        flags
            .iter()
            .filter(|(on, _)| *on)
            .map(|(_, f)| f.to_string()),
    );
    if let Some(z) = o.compress {
        args.push(format!("--compress={z}"));
    }
    if let Some(j) = o.jobs {
        args.push(format!("--jobs={j}"));
    }
    if let Some(e) = encoding {
        args.push("--encoding".into());
        args.push(e.to_string());
    }
    match &req.scope {
        BackupScope::Database => {}
        BackupScope::Schema { name } => {
            args.push("--schema".into());
            args.push(dump_pattern(name));
        }
        BackupScope::Table { schema, name } => {
            args.push("--table".into());
            args.push(format!("{}.{}", dump_pattern(schema), dump_pattern(name)));
        }
    }

    let mut notes = Vec::new();
    if is_dir {
        notes.push("Hedef klasör boş olmalı ya da hiç olmamalı.".to_string());
    }
    if plain && o.compress.is_some_and(|z| z > 0) {
        notes.push(
            "Çıktı gzip ile sıkıştırılır (.sql.gz); psql ile yüklemeden önce açılmalıdır."
                .to_string(),
        );
    }
    if o.create && !plain {
        notes.push("--create arşive yazılır; geri yüklemede \"arşivdeki adla oluştur\" hedefiyle kullanılır.".to_string());
    }

    let exe = exe(tool, ToolKind::PgDump);
    let env = base_env(conn, &conn.database, "pg-tools backup");
    ansi_sweep(&exe, &args, &env, &mut errors);
    if !errors.is_empty() {
        return Err(errors);
    }
    Ok(Invocation {
        preview: preview(&exe, &args, &env),
        exe,
        args,
        env,
        env_remove: env_remove(),
        risk: Risk::Safe,
        notes,
        output: Some(OutputPlan {
            final_path,
            partial_path,
            is_dir,
        }),
        tool: ToolKind::PgDump,
        tolerates_ignored_errors: false,
    })
}

/// Geri yükleme çağrısı (spec §1 "Geri yükleme"): arşiv formatları `pg_restore`, düz SQL `psql`. Format çağıranın
/// algıladığı `src`'dir; kullanıcı zorlayamaz. `tools`: (`pg_restore` kümesi, `psql` kümesi).
pub fn restore_invocation(
    tools: (&ToolSet, &ToolSet),
    conn: &Conn,
    maintenance_db: &str,
    req: &RestoreRequest,
    src: SourceFormat,
) -> Result<Invocation, Vec<FieldError>> {
    let (restore_tool, psql_tool) = tools;
    let o = &req.options;
    let mut errors = Vec::new();
    let mut notes = Vec::new();
    let partial_note = "İş yarıda kalırsa veritabanı yarım kalabilir (tek transaction kapalı).";

    if src == SourceFormat::Plain {
        check_tool(psql_tool, ToolKind::Psql, &mut errors);
        if req.target == RestoreTarget::CreateFromArchive {
            errors.push(field(
                "target",
                "Düz SQL yalnız mevcut bir veritabanına yüklenir.",
            ));
        }
        const NOT_PLAIN: &str = "Düz SQL geri yüklemede kullanılamaz (SQL içeriğine bağlıdır).";
        let unsupported = [
            (o.clean, "options.clean"),
            (o.if_exists, "options.ifExists"),
            (o.create, "options.create"),
            (o.jobs.is_some(), "options.jobs"),
            (o.schema_only, "options.schemaOnly"),
            (o.data_only, "options.dataOnly"),
            (o.no_owner, "options.noOwner"),
            (o.no_privileges, "options.noPrivileges"),
        ];
        for (on, path) in unsupported {
            if on {
                errors.push(field(path, NOT_PLAIN));
            }
        }
        let path = check_path(&req.path, "path", false, &mut errors);
        check_ansi_path(&path, "path", &mut errors);
        if !errors.is_empty() {
            return Err(errors);
        }
        // ON_ERROR_STOP her zaman açık: ilk hatada durulur ve çıkış kodu sıfırdan farklı olur.
        let mut args: Vec<String> = vec![
            "--no-psqlrc".into(),
            "--set".into(),
            "ON_ERROR_STOP=1".into(),
        ];
        if o.single_transaction {
            args.push("--single-transaction".into());
        }
        args.push("--file".into());
        args.push(path);
        notes.push(
            "Düz SQL psql ile çalıştırılır; içerik taranmaz ve ilk hatada durulur.".to_string(),
        );
        notes.push(PLAIN_META_NOTE.to_string());
        if !o.single_transaction {
            notes.push(partial_note.to_string());
        }
        let exe = exe(psql_tool, ToolKind::Psql);
        let env = base_env(conn, &conn.database, "pg-tools restore");
        ansi_sweep(&exe, &args, &env, &mut errors);
        if !errors.is_empty() {
            return Err(errors);
        }
        return Ok(Invocation {
            preview: preview(&exe, &args, &env),
            exe,
            args,
            env,
            env_remove: env_remove(),
            risk: Risk::DataLoss,
            notes,
            output: None,
            tool: ToolKind::Psql,
            tolerates_ignored_errors: false,
        });
    }

    check_tool(restore_tool, ToolKind::PgRestore, &mut errors);
    let create = req.target == RestoreTarget::CreateFromArchive;
    if create && maintenance_db.trim().is_empty() {
        errors.push(field("target", "Bakım veritabanı bilinmiyor."));
    }
    if o.create && !create {
        errors.push(field(
            "options.create",
            "Oluştur (--create) için hedef \"arşivdeki adla oluştur\" olmalı.",
        ));
    }
    if o.schema_only && o.data_only {
        errors.push(field(
            "options.dataOnly",
            "Yalnız şema ile yalnız veri birlikte seçilemez.",
        ));
    }
    if o.clean && o.data_only {
        errors.push(field(
            "options.clean",
            "Temizle (--clean) yalnız veri ile birlikte kullanılamaz.",
        ));
    }
    if o.if_exists && !o.clean {
        errors.push(field(
            "options.ifExists",
            "--if-exists, temizle (--clean) seçeneğini gerektirir.",
        ));
    }
    if create && o.single_transaction {
        errors.push(field(
            "options.singleTransaction",
            "Oluştur (--create) ile tek transaction birlikte kullanılamaz.",
        ));
    }
    if let Some(msg) = check_jobs(o.jobs) {
        errors.push(field("options.jobs", msg));
    } else if o.jobs.is_some() && o.single_transaction {
        errors.push(field(
            "options.jobs",
            "Paralel iş ile tek transaction birlikte kullanılamaz.",
        ));
    } else if o.jobs.is_some() && src == SourceFormat::Tar {
        errors.push(field(
            "options.jobs",
            "Tar arşivinde paralel geri yükleme yapılamaz.",
        ));
    }
    let path = check_path(
        &req.path,
        "path",
        src == SourceFormat::Directory,
        &mut errors,
    );
    check_ansi_path(&path, "path", &mut errors);
    if !errors.is_empty() {
        return Err(errors);
    }

    let database = if create {
        maintenance_db
    } else {
        conn.database.as_str()
    };
    let format = match src {
        SourceFormat::Custom => "custom",
        SourceFormat::Directory => "directory",
        _ => "tar",
    };
    let mut args: Vec<String> = vec![
        "--verbose".into(),
        format!("--format={format}"),
        "--dbname".into(),
        conninfo_dbname(database),
    ];
    let flags = [
        (o.clean, "--clean"),
        (o.if_exists, "--if-exists"),
        (create, "--create"),
        (o.single_transaction, "--single-transaction"),
    ];
    args.extend(
        flags
            .iter()
            .filter(|(on, _)| *on)
            .map(|(_, f)| f.to_string()),
    );
    if let Some(j) = o.jobs {
        args.push(format!("--jobs={j}"));
    }
    let flags = [
        (o.schema_only, "--schema-only"),
        (o.data_only, "--data-only"),
        (o.no_owner, "--no-owner"),
        (o.no_privileges, "--no-privileges"),
        (o.exit_on_error, "--exit-on-error"),
    ];
    args.extend(
        flags
            .iter()
            .filter(|(on, _)| *on)
            .map(|(_, f)| f.to_string()),
    );
    args.push(path);

    if create {
        notes.push(
            "Bakım veritabanına bağlanılır; veritabanı arşivdeki adla oluşturulur.".to_string(),
        );
    }
    if o.clean {
        notes.push("Arşivdeki nesneler hedefte varsa önce silinir.".to_string());
    }
    if !o.single_transaction {
        notes.push(partial_note.to_string());
    }
    if !o.exit_on_error {
        notes.push(
            "Hatalar atlanır ve sonunda sayıları bildirilir; durmak için \"hatada dur\" seçin."
                .to_string(),
        );
    }
    let exe = exe(restore_tool, ToolKind::PgRestore);
    let env = base_env(conn, database, "pg-tools restore");
    ansi_sweep(&exe, &args, &env, &mut errors);
    if !errors.is_empty() {
        return Err(errors);
    }
    Ok(Invocation {
        preview: preview(&exe, &args, &env),
        exe,
        args,
        env,
        env_remove: env_remove(),
        risk: if o.clean || create {
            Risk::DataLoss
        } else {
            Risk::Destructive
        },
        notes,
        output: None,
        tool: ToolKind::PgRestore,
        tolerates_ignored_errors: !o.exit_on_error,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::ToolVersion;
    use std::path::MAIN_SEPARATOR as SEP;

    fn bin_dir() -> &'static str {
        if cfg!(windows) {
            r"C:\Program Files\PostgreSQL\18\bin"
        } else {
            "/opt/pg/18/bin"
        }
    }

    fn out_dir() -> &'static str {
        if cfg!(windows) {
            r"C:\Yedek Klasörü"
        } else {
            "/Yedek Klasörü"
        }
    }

    fn out(name: &str) -> String {
        format!("{}{SEP}{name}", out_dir())
    }

    fn tools() -> ToolSet {
        ToolSet {
            dir: bin_dir().into(),
            version: ToolVersion {
                major: 18,
                version_num: 180001,
                text: "18.1".into(),
            },
            pg_dump: true,
            pg_restore: true,
            psql: true,
        }
    }

    fn conn() -> Conn {
        Conn {
            host: "db.local".into(),
            port: 5432,
            user: "app user".into(),
            database: "satış".into(),
            sslmode: "prefer",
            root_cert: None,
            client_cert: None,
            client_key: None,
        }
    }

    fn ssl_conn() -> Conn {
        Conn {
            sslmode: "verify-full",
            root_cert: Some(out("root.crt")),
            client_cert: Some(out("client.crt")),
            client_key: Some(out("client.key")),
            ..conn()
        }
    }

    /// Anlık görüntü metni: platforma bağlı kökler yer tutucuyla değiştirilir.
    fn render(inv: &Invocation) -> String {
        // Yalnız yer tutucudan sonraki ayraç "/" yapılır; önizlemedeki `\"` kaçışları korunur.
        let norm = |s: &str| {
            s.replace(bin_dir(), "<BIN>")
                .replace(out_dir(), "<DIR>")
                .replace(std::env::consts::EXE_SUFFIX, "")
                .replace(&format!("<BIN>{SEP}"), "<BIN>/")
                .replace(&format!("<DIR>{SEP}"), "<DIR>/")
        };
        let mut s = format!("exe: {}\nargs:\n", norm(&inv.exe));
        for a in &inv.args {
            s.push_str(&format!("  {}\n", norm(a)));
        }
        s.push_str("env:\n");
        for (k, v) in &inv.env {
            s.push_str(&format!("  {k}={}\n", norm(v)));
        }
        s.push_str(&format!(
            "preview: {}\nrisk: {:?}\nnotes:\n",
            norm(&inv.preview),
            inv.risk
        ));
        for n in &inv.notes {
            s.push_str(&format!("  - {n}\n"));
        }
        match &inv.output {
            Some(o) => s.push_str(&format!(
                "output: final={} partial={} dir={}\n",
                norm(&o.final_path),
                norm(&o.partial_path),
                o.is_dir
            )),
            None => s.push_str("output: -\n"),
        }
        s
    }

    fn breq(
        scope: BackupScope,
        format: DumpFormat,
        options: BackupOptions,
        path: &str,
    ) -> BackupRequest {
        BackupRequest {
            scope,
            format,
            options,
            output_path: out(path),
        }
    }

    fn backup_ok(req: &BackupRequest) -> Invocation {
        backup_invocation(&tools(), &conn(), req).unwrap_or_else(|e| panic!("{e:?}"))
    }

    #[test]
    fn backup_custom_database_defaults() {
        let req = breq(
            BackupScope::Database,
            DumpFormat::Custom,
            BackupOptions::default(),
            "satış.backup",
        );
        insta::assert_snapshot!(render(&backup_ok(&req)));
    }

    #[test]
    fn backup_plain_schema_all_plain_options() {
        let options = BackupOptions {
            clean: true,
            if_exists: true,
            create: true,
            column_inserts: true,
            no_owner: true,
            no_privileges: true,
            compress: Some(6),
            encoding: Some("UTF8".into()),
            ..Default::default()
        };
        let req = breq(
            BackupScope::Schema {
                name: "Satış \"Şema\"".into(),
            },
            DumpFormat::Plain,
            options,
            "a.sql.gz",
        );
        insta::assert_snapshot!(render(&backup_ok(&req)));
    }

    #[test]
    fn backup_directory_table_parallel() {
        let options = BackupOptions {
            data_only: true,
            compress: Some(9),
            jobs: Some(4),
            ..Default::default()
        };
        let scope = BackupScope::Table {
            schema: "public".into(),
            name: "a.b*?".into(),
        };
        // Sondaki ayraç atılır; geçici klasör hedefin kardeşidir.
        let req = breq(
            scope,
            DumpFormat::Directory,
            options,
            &format!("dizin{SEP}"),
        );
        insta::assert_snapshot!(render(&backup_ok(&req)));
    }

    #[test]
    fn backup_tar_schema_only_with_ssl() {
        let options = BackupOptions {
            schema_only: true,
            inserts: false,
            create: true,
            ..Default::default()
        };
        let req = breq(BackupScope::Database, DumpFormat::Tar, options, "a.tar");
        let inv = backup_invocation(&tools(), &ssl_conn(), &req).unwrap();
        insta::assert_snapshot!(render(&inv));
    }

    fn rreq(target: RestoreTarget, options: RestoreOptions) -> RestoreRequest {
        RestoreRequest {
            path: out("kaynak"),
            target,
            options,
        }
    }

    fn restore_ok(req: &RestoreRequest, src: SourceFormat) -> Invocation {
        restore_invocation((&tools(), &tools()), &conn(), "postgres", req, src)
            .unwrap_or_else(|e| panic!("{e:?}"))
    }

    #[test]
    fn restore_custom_existing_clean() {
        let options = RestoreOptions {
            clean: true,
            if_exists: true,
            no_owner: true,
            exit_on_error: true,
            ..Default::default()
        };
        insta::assert_snapshot!(render(&restore_ok(
            &rreq(RestoreTarget::Existing, options),
            SourceFormat::Custom
        )));
    }

    #[test]
    fn restore_directory_create_parallel() {
        let options = RestoreOptions {
            jobs: Some(4),
            data_only: false,
            ..Default::default()
        };
        let inv = restore_ok(
            &rreq(RestoreTarget::CreateFromArchive, options),
            SourceFormat::Directory,
        );
        insta::assert_snapshot!(render(&inv));
    }

    #[test]
    fn restore_tar_single_transaction() {
        let options = RestoreOptions {
            single_transaction: true,
            schema_only: true,
            no_privileges: true,
            ..Default::default()
        };
        insta::assert_snapshot!(render(&restore_ok(
            &rreq(RestoreTarget::Existing, options),
            SourceFormat::Tar
        )));
    }

    #[test]
    fn restore_plain_with_psql() {
        let options = RestoreOptions {
            single_transaction: true,
            ..Default::default()
        };
        let inv = restore_invocation(
            (&tools(), &tools()),
            &ssl_conn(),
            "postgres",
            &rreq(RestoreTarget::Existing, options),
            SourceFormat::Plain,
        )
        .unwrap();
        insta::assert_snapshot!(render(&inv));
    }

    fn paths(r: Result<Invocation, Vec<FieldError>>) -> Vec<String> {
        match r {
            Ok(_) => vec![],
            Err(e) => e.into_iter().map(|e| e.path).collect(),
        }
    }

    #[test]
    fn backup_rule_exclusions() {
        let d = BackupOptions::default;
        type Case = (DumpFormat, BackupOptions, &'static [&'static str]);
        let cases: Vec<Case> = vec![
            (
                DumpFormat::Custom,
                BackupOptions {
                    schema_only: true,
                    data_only: true,
                    ..d()
                },
                &["options.dataOnly"],
            ),
            (
                DumpFormat::Plain,
                BackupOptions {
                    inserts: true,
                    column_inserts: true,
                    ..d()
                },
                &["options.columnInserts"],
            ),
            (
                DumpFormat::Custom,
                BackupOptions {
                    jobs: Some(2),
                    ..d()
                },
                &["options.jobs"],
            ),
            (
                DumpFormat::Plain,
                BackupOptions {
                    jobs: Some(2),
                    ..d()
                },
                &["options.jobs"],
            ),
            (
                DumpFormat::Directory,
                BackupOptions {
                    jobs: Some(0),
                    ..d()
                },
                &["options.jobs"],
            ),
            (
                DumpFormat::Directory,
                BackupOptions {
                    jobs: Some(17),
                    ..d()
                },
                &["options.jobs"],
            ),
            (
                DumpFormat::Directory,
                BackupOptions {
                    jobs: Some(16),
                    ..d()
                },
                &[],
            ),
            (
                DumpFormat::Tar,
                BackupOptions {
                    compress: Some(5),
                    ..d()
                },
                &["options.compress"],
            ),
            (
                DumpFormat::Custom,
                BackupOptions {
                    compress: Some(10),
                    ..d()
                },
                &["options.compress"],
            ),
            (
                DumpFormat::Custom,
                BackupOptions {
                    compress: Some(0),
                    ..d()
                },
                &[],
            ),
            (
                DumpFormat::Custom,
                BackupOptions { clean: true, ..d() },
                &["options.clean"],
            ),
            (
                DumpFormat::Directory,
                BackupOptions {
                    inserts: true,
                    ..d()
                },
                &["options.inserts"],
            ),
            (
                DumpFormat::Tar,
                BackupOptions {
                    column_inserts: true,
                    ..d()
                },
                &["options.columnInserts"],
            ),
            // Sürüme bağlı değil ama pg_dump'ın kendi kuralı: --if-exists, -c gerektirir; -c ile -a olmaz.
            (
                DumpFormat::Plain,
                BackupOptions {
                    if_exists: true,
                    ..d()
                },
                &["options.ifExists"],
            ),
            (
                DumpFormat::Custom,
                BackupOptions {
                    if_exists: true,
                    ..d()
                },
                &["options.ifExists"],
            ),
            (
                DumpFormat::Plain,
                BackupOptions {
                    clean: true,
                    data_only: true,
                    ..d()
                },
                &["options.clean"],
            ),
            (
                DumpFormat::Custom,
                BackupOptions {
                    create: true,
                    ..d()
                },
                &[],
            ),
            (
                DumpFormat::Plain,
                BackupOptions {
                    encoding: Some("UTF8; DROP".into()),
                    ..d()
                },
                &["options.encoding"],
            ),
            (
                DumpFormat::Plain,
                BackupOptions {
                    encoding: Some("  ".into()),
                    ..d()
                },
                &[],
            ),
            (
                DumpFormat::Plain,
                BackupOptions {
                    encoding: Some("-x".into()),
                    ..d()
                },
                &["options.encoding"],
            ),
            // Birden çok hata birlikte döner.
            (
                DumpFormat::Tar,
                BackupOptions {
                    schema_only: true,
                    data_only: true,
                    compress: Some(1),
                    jobs: Some(2),
                    clean: true,
                    ..d()
                },
                &[
                    "options.dataOnly",
                    "options.clean",
                    "options.compress",
                    "options.jobs",
                ],
            ),
        ];
        for (format, options, want) in cases {
            let req = breq(BackupScope::Database, format, options.clone(), "x");
            let got = paths(backup_invocation(&tools(), &conn(), &req));
            assert_eq!(got, *want, "{format:?} {options:?}");
        }
    }

    #[test]
    fn backup_rejects_bad_scope_path_and_tool() {
        let d = BackupOptions::default();
        let bad = |scope: BackupScope, path: &str| {
            let req = BackupRequest {
                scope,
                format: DumpFormat::Custom,
                options: d.clone(),
                output_path: path.into(),
            };
            paths(backup_invocation(&tools(), &conn(), &req))
        };
        assert_eq!(
            bad(BackupScope::Schema { name: " ".into() }, &out("a")),
            vec!["scope.name"]
        );
        assert_eq!(
            bad(
                BackupScope::Schema {
                    name: "a\0b".into()
                },
                &out("a")
            ),
            vec!["scope.name"]
        );
        assert_eq!(
            bad(
                BackupScope::Table {
                    schema: "".into(),
                    name: "".into()
                },
                &out("a")
            ),
            vec!["scope.schema", "scope.name"]
        );
        assert_eq!(bad(BackupScope::Database, ""), vec!["outputPath"]);
        // Göreli yol (CWD'ye yazar; "-" ile başlayan seçenek sanılabilir) reddedilir.
        assert_eq!(
            bad(BackupScope::Database, "yedek.backup"),
            vec!["outputPath"]
        );
        assert_eq!(bad(BackupScope::Database, "-Z9"), vec!["outputPath"]);
        let no_dump = ToolSet {
            pg_dump: false,
            ..tools()
        };
        let req = breq(BackupScope::Database, DumpFormat::Custom, d, "a");
        assert_eq!(
            paths(backup_invocation(&no_dump, &conn(), &req)),
            vec!["tool"]
        );
    }

    #[test]
    fn restore_rule_exclusions() {
        let d = RestoreOptions::default;
        use RestoreTarget::*;
        use SourceFormat::*;
        type Case = (
            SourceFormat,
            RestoreTarget,
            RestoreOptions,
            &'static [&'static str],
        );
        let cases: Vec<Case> = vec![
            (
                Custom,
                Existing,
                RestoreOptions {
                    schema_only: true,
                    data_only: true,
                    ..d()
                },
                &["options.dataOnly"],
            ),
            (
                Custom,
                Existing,
                RestoreOptions {
                    if_exists: true,
                    ..d()
                },
                &["options.ifExists"],
            ),
            (
                Custom,
                Existing,
                RestoreOptions {
                    clean: true,
                    data_only: true,
                    ..d()
                },
                &["options.clean"],
            ),
            (
                Custom,
                Existing,
                RestoreOptions {
                    jobs: Some(2),
                    single_transaction: true,
                    ..d()
                },
                &["options.jobs"],
            ),
            (
                Custom,
                Existing,
                RestoreOptions {
                    jobs: Some(17),
                    ..d()
                },
                &["options.jobs"],
            ),
            (
                Directory,
                Existing,
                RestoreOptions {
                    jobs: Some(16),
                    ..d()
                },
                &[],
            ),
            // pg_restore tar arşivinde paralel geri yükleme yapamaz.
            (
                Tar,
                Existing,
                RestoreOptions {
                    jobs: Some(2),
                    ..d()
                },
                &["options.jobs"],
            ),
            // -C hedefi bakım veritabanıdır: "Mevcut veritabanı" hedefiyle oluştur seçilemez.
            (
                Custom,
                Existing,
                RestoreOptions {
                    create: true,
                    ..d()
                },
                &["options.create"],
            ),
            (
                Custom,
                CreateFromArchive,
                RestoreOptions {
                    create: true,
                    ..d()
                },
                &[],
            ),
            // pg_restore: -C ile -1 birlikte olmaz.
            (
                Custom,
                CreateFromArchive,
                RestoreOptions {
                    single_transaction: true,
                    ..d()
                },
                &["options.singleTransaction"],
            ),
            // psql: temizle/oluştur ve pg_restore'a özgü seçenekler yok; hedef yalnız mevcut veritabanı.
            (Plain, CreateFromArchive, d(), &["target"]),
            (
                Plain,
                Existing,
                RestoreOptions {
                    clean: true,
                    if_exists: true,
                    create: true,
                    jobs: Some(2),
                    schema_only: true,
                    data_only: true,
                    no_owner: true,
                    no_privileges: true,
                    ..d()
                },
                &[
                    "options.clean",
                    "options.ifExists",
                    "options.create",
                    "options.jobs",
                    "options.schemaOnly",
                    "options.dataOnly",
                    "options.noOwner",
                    "options.noPrivileges",
                ],
            ),
            // psql her zaman ON_ERROR_STOP=1 ile çalışır; "hatada dur" zararsızdır.
            (
                Plain,
                Existing,
                RestoreOptions {
                    exit_on_error: true,
                    single_transaction: true,
                    ..d()
                },
                &[],
            ),
        ];
        for (src, target, options, want) in cases {
            let req = rreq(target, options.clone());
            let got = paths(restore_invocation(
                (&tools(), &tools()),
                &conn(),
                "postgres",
                &req,
                src,
            ));
            assert_eq!(got, *want, "{src:?} {target:?} {options:?}");
        }
    }

    #[test]
    fn restore_rejects_bad_path_and_tools() {
        let req = |path: &str| RestoreRequest {
            path: path.into(),
            target: RestoreTarget::Existing,
            options: Default::default(),
        };
        let run = |r: &RestoreRequest, src| {
            paths(restore_invocation(
                (&tools(), &tools()),
                &conn(),
                "postgres",
                r,
                src,
            ))
        };
        assert_eq!(run(&req(""), SourceFormat::Custom), vec!["path"]);
        assert_eq!(run(&req("a.backup"), SourceFormat::Custom), vec!["path"]);
        assert_eq!(run(&req("--create"), SourceFormat::Plain), vec!["path"]);
        let no_restore = ToolSet {
            pg_restore: false,
            ..tools()
        };
        let no_psql = ToolSet {
            psql: false,
            ..tools()
        };
        let ok = rreq(RestoreTarget::Existing, Default::default());
        assert_eq!(
            paths(restore_invocation(
                (&no_restore, &tools()),
                &conn(),
                "postgres",
                &ok,
                SourceFormat::Tar
            )),
            vec!["tool"]
        );
        assert_eq!(
            paths(restore_invocation(
                (&tools(), &no_psql),
                &conn(),
                "postgres",
                &ok,
                SourceFormat::Plain
            )),
            vec!["tool"]
        );
        // Plain için pg_restore kümesi, arşiv için psql kümesi gerekmez.
        assert!(
            restore_invocation(
                (&no_restore, &tools()),
                &conn(),
                "postgres",
                &ok,
                SourceFormat::Plain
            )
            .is_ok()
        );
        assert!(
            restore_invocation(
                (&tools(), &no_psql),
                &conn(),
                "postgres",
                &ok,
                SourceFormat::Custom
            )
            .is_ok()
        );
        let create = rreq(RestoreTarget::CreateFromArchive, Default::default());
        assert_eq!(
            paths(restore_invocation(
                (&tools(), &tools()),
                &conn(),
                " ",
                &create,
                SourceFormat::Custom
            )),
            vec!["target"]
        );
    }

    #[test]
    fn risk_follows_spec() {
        let b = breq(
            BackupScope::Database,
            DumpFormat::Plain,
            BackupOptions {
                clean: true,
                ..Default::default()
            },
            "a.sql",
        );
        assert_eq!(backup_ok(&b).risk, Risk::Safe);
        let r = |target, options, src| restore_ok(&rreq(target, options), src).risk;
        use RestoreTarget::*;
        let d = RestoreOptions::default;
        assert_eq!(r(Existing, d(), SourceFormat::Custom), Risk::Destructive);
        assert_eq!(
            r(
                Existing,
                RestoreOptions {
                    single_transaction: true,
                    ..d()
                },
                SourceFormat::Tar
            ),
            Risk::Destructive
        );
        assert_eq!(
            r(
                Existing,
                RestoreOptions { clean: true, ..d() },
                SourceFormat::Custom
            ),
            Risk::DataLoss
        );
        assert_eq!(
            r(CreateFromArchive, d(), SourceFormat::Directory),
            Risk::DataLoss
        );
        assert_eq!(r(Existing, d(), SourceFormat::Plain), Risk::DataLoss);
    }

    #[test]
    fn preview_has_no_password_and_is_one_line() {
        let scope = BackupScope::Table {
            schema: "a\nb".into(),
            name: "x\r\ny\t\"z\"".into(),
        };
        let inv = backup_ok(&breq(
            scope,
            DumpFormat::Custom,
            BackupOptions::default(),
            "a",
        ));
        assert!(
            !inv.preview.contains('\n') && !inv.preview.contains('\r'),
            "{}",
            inv.preview
        );
        assert!(inv.preview.contains("PGPASSWORD=***"), "{}", inv.preview);
        assert_eq!(inv.preview.matches("PGPASSWORD").count(), 1);
        // Ortamda parola yoktur (yürütücü ekler); argümanlar da parola taşımaz.
        assert!(
            inv.env
                .iter()
                .all(|(k, _)| k != "PGPASSWORD" && k != "PGPASSFILE" && k != "PGSERVICE")
        );
        assert!(
            inv.args
                .iter()
                .all(|a| !a.to_ascii_lowercase().contains("password"))
        );
        // Argüman ham kalır (yalnız önizleme kaçışlıdır).
        assert!(
            inv.args
                .contains(&"\"a\nb\".\"x\r\ny\t\"\"z\"\"\"".to_string()),
            "{:?}",
            inv.args
        );
        let r = restore_ok(
            &rreq(RestoreTarget::Existing, Default::default()),
            SourceFormat::Plain,
        );
        assert!(
            r.preview.contains("PGPASSWORD=***") && r.env.iter().all(|(k, _)| k != "PGPASSWORD")
        );
    }

    #[test]
    fn env_contains_connection_values() {
        let inv = backup_ok(&breq(
            BackupScope::Database,
            DumpFormat::Custom,
            BackupOptions::default(),
            "a",
        ));
        let get = |k: &str| {
            inv.env
                .iter()
                .find(|(n, _)| n == k)
                .map(|(_, v)| v.as_str())
        };
        assert_eq!(get("PGHOST"), Some("db.local"));
        assert_eq!(get("PGPORT"), Some("5432"));
        assert_eq!(get("PGUSER"), Some("app user"));
        assert_eq!(get("PGDATABASE"), Some("satış"));
        assert_eq!(get("PGSSLMODE"), Some("prefer"));
        assert_eq!(get("PGAPPNAME"), Some("pg-tools backup"));
        assert_eq!(get("PGCONNECT_TIMEOUT"), Some("15"));
        // Sınıflandırma ve ipuçları İngilizce iletilere dayanır (yerel Windows araçları Türkçe yazar).
        assert_eq!(get("LC_MESSAGES"), Some("C"));
        // Miras LC_ALL, LC_MESSAGES=C'yi ezer: yürütücü bunları kaldırır (ortamda da yoklar).
        assert_eq!(inv.env_remove, vec!["LC_ALL".to_string(), "LANGUAGE".to_string()]);
        assert!(inv.env.iter().all(|(k, _)| !inv.env_remove.contains(k)));
        assert_eq!(get("PGSSLROOTCERT"), None);
        let r = restore_ok(
            &rreq(RestoreTarget::CreateFromArchive, Default::default()),
            SourceFormat::Custom,
        );
        let rget = |k: &str| r.env.iter().find(|(n, _)| n == k).map(|(_, v)| v.as_str());
        assert_eq!(rget("PGAPPNAME"), Some("pg-tools restore"));
        // --create: bakım veritabanına bağlanılır.
        assert_eq!(rget("PGDATABASE"), Some("postgres"));
    }

    #[test]
    fn atomic_output_paths() {
        let inv = backup_ok(&breq(
            BackupScope::Database,
            DumpFormat::Custom,
            BackupOptions::default(),
            "a.backup",
        ));
        let o = inv.output.clone().unwrap();
        assert_eq!(o.final_path, out("a.backup"));
        assert_eq!(
            o.partial_path,
            format!("{}{PARTIAL_SUFFIX}", out("a.backup"))
        );
        assert!(!o.is_dir);
        // Araç geçici yola yazar.
        let i = inv.args.iter().position(|a| a == "--file").unwrap();
        assert_eq!(inv.args[i + 1], o.partial_path);
        let dir = backup_ok(&breq(
            BackupScope::Database,
            DumpFormat::Directory,
            BackupOptions::default(),
            &format!("d{SEP}{SEP}"),
        ));
        let o = dir.output.unwrap();
        assert_eq!((o.final_path.as_str(), o.is_dir), (out("d").as_str(), true));
        assert_eq!(o.partial_path, format!("{}{PARTIAL_SUFFIX}", out("d")));
        assert!(
            restore_ok(
                &rreq(RestoreTarget::Existing, Default::default()),
                SourceFormat::Custom
            )
            .output
            .is_none()
        );
    }

    #[test]
    fn preview_quoting_doubles_backslashes_before_quotes() {
        assert_eq!(quote_preview(r"C:\a"), r"C:\a");
        assert_eq!(quote_preview(r"C:\Yedek Klasörü\a"), r#""C:\Yedek Klasörü\a""#);
        // Kapanış tırnağından önceki ters bölü ikilenir; iç `"` öncesindekiler de.
        assert_eq!(quote_preview(r"C:\Yeni Klasör\"), r#""C:\Yeni Klasör\\""#);
        assert_eq!(quote_preview(r#"a\"b c"#), r#""a\\\"b c""#);
        assert_eq!(quote_preview(""), r#""""#);
    }

    #[test]
    fn backup_if_exists_outside_plain_is_one_clear_error() {
        let req = breq(
            BackupScope::Database,
            DumpFormat::Custom,
            BackupOptions { if_exists: true, ..Default::default() },
            "x",
        );
        let errs = backup_invocation(&tools(), &conn(), &req).unwrap_err();
        assert_eq!(errs.len(), 1);
        assert!(errs[0].message.contains("yalnız düz SQL"), "{}", errs[0].message);
    }

    #[test]
    fn quotes_dump_patterns() {
        // pg_dump desenlerinde çift tırnak içindeki her karakter düzdür; içteki `"` iki kez yazılır.
        assert_eq!(dump_pattern("t"), "\"t\"");
        assert_eq!(dump_pattern("Tablo"), "\"Tablo\"");
        assert_eq!(dump_pattern("a\"b"), "\"a\"\"b\"");
        assert_eq!(dump_pattern("a.b*?"), "\"a.b*?\"");
        assert_eq!(dump_pattern("$x|(y)[z]^"), "\"$x|(y)[z]^\"");
        assert_eq!(dump_pattern("Müşteri Tablosu"), "\"M\"??\"teri Tablosu\"");
        assert_eq!(dump_pattern("Büyük Ad"), "\"B\"?\"y\"?\"k Ad\"");
        assert_eq!(dump_pattern("şş"), "??");
        assert_eq!(dump_pattern("ş\"x"), "?\"\"\"x\"");
        assert_eq!(dump_pattern(""), "\"\"");
        assert_eq!(pattern_regex("Büyük Ad"), "^(B.y.k Ad)$");
        assert_eq!(pattern_regex("a.b*?$"), r"^(a\.b\*\?\$)$");
        let mut c = conn();
        assert_eq!(windows_name_problems(&c).len(), 1);
        c.database = "app".into();
        assert!(windows_name_problems(&c).is_empty());
        c.user = "kullanıcı".into();
        assert_eq!(windows_name_problems(&c)[0].path, "user");
        assert_eq!(dump_pattern("\"\""), "\"\"\"\"\"\"");
        assert_eq!(dump_pattern(" "), "\" \"");
    }

    #[test]
    fn restore_dbname_is_never_a_connection_string() {
        // "host=… " ya da "postgresql://" gibi adlar -d'de bağlantı dizesi sayılırdı; her zaman dbname='…'.
        assert_eq!(conninfo_dbname("a'b\\c"), r"dbname='a\'b\\c'");
        assert_eq!(
            conninfo_dbname("host=evil port=1"),
            "dbname='host=evil port=1'"
        );
        let c = Conn {
            database: "postgresql://x".into(),
            ..conn()
        };
        let inv = restore_invocation(
            (&tools(), &tools()),
            &c,
            "postgres",
            &rreq(RestoreTarget::Existing, Default::default()),
            SourceFormat::Custom,
        )
        .unwrap();
        assert!(
            inv.args.contains(&"dbname='postgresql://x'".to_string()),
            "{:?}",
            inv.args
        );
    }

    #[test]
    fn options_serde_shape() {
        let req = BackupRequest {
            scope: BackupScope::Table {
                schema: "s".into(),
                name: "t".into(),
            },
            format: DumpFormat::Directory,
            options: BackupOptions {
                jobs: Some(2),
                ..Default::default()
            },
            output_path: "C:\\x".into(),
        };
        let v = serde_json::to_value(&req).unwrap();
        assert_eq!(
            v["scope"],
            serde_json::json!({ "kind": "table", "schema": "s", "name": "t" })
        );
        assert_eq!(v["format"], "directory");
        assert_eq!(v["options"]["schemaOnly"], false);
        assert_eq!(v["outputPath"], "C:\\x");
        let r: RestoreRequest = serde_json::from_value(serde_json::json!({
            "path": "C:\\a", "target": "create_from_archive",
            "options": { "clean": false, "ifExists": false, "create": false, "singleTransaction": false, "jobs": null,
                         "schemaOnly": false, "dataOnly": false, "noOwner": false, "noPrivileges": false, "exitOnError": true }
        }))
        .unwrap();
        assert_eq!(
            (r.target, r.options.exit_on_error),
            (RestoreTarget::CreateFromArchive, true)
        );
    }
    #[test]
    fn conn_from_server_maps_ssl_like_the_app() {
        use pgt_core::config::{ServerConfig, SslMode};
        let mut c = ServerConfig::new("s", "db.local", 5433, "u");
        c.ssl.root_cert = Some("root.crt".into());
        c.ssl.client_cert = Some("c.crt".into());
        c.ssl.client_key = Some("c.key".into());
        let map = |c: &ServerConfig, v: u32| {
            let k = Conn::from_server(c, "d", v);
            (k.sslmode, k.root_cert, k.client_cert.is_some() && k.client_key.is_some())
        };
        let cases = [
            (SslMode::Disable, "disable", None, false),
            (SslMode::Prefer, "prefer", None, true),
            // require + kök sertifika libpq'da verify-ca olurdu: kök sertifika geçirilmez.
            (SslMode::Require, "require", None, true),
            (SslMode::VerifyCa, "verify-ca", Some("root.crt"), true),
            (SslMode::VerifyFull, "verify-full", Some("root.crt"), true),
        ];
        for (mode, want, root, client) in cases {
            c.ssl.mode = mode;
            assert_eq!(map(&c, 180000), (want, root.map(str::to_string), client), "{mode:?}");
        }
        // Kök sertifikasız verify-full: libpq 16+ sistem deposu; eski araçta geçirilmez.
        c.ssl.root_cert = None;
        assert_eq!(map(&c, 160000).1.as_deref(), Some("system"));
        assert_eq!(map(&c, 150008).1, None);
        c.ssl.mode = SslMode::VerifyCa;
        assert_eq!(map(&c, 180000).1, None);
        let k = Conn::from_server(&c, "satış", 180000);
        assert_eq!((k.host.as_str(), k.port, k.user.as_str(), k.database.as_str()), ("db.local", 5433, "u", "satış"));
        // Ortamda kök sertifika yalnız verify-* ile görünür.
        c.ssl.mode = SslMode::Require;
        c.ssl.root_cert = Some("root.crt".into());
        let env = base_env(&Conn::from_server(&c, "d", 180000), "d", "x");
        assert!(env.iter().all(|(k, _)| k != "PGSSLROOTCERT"), "{env:?}");
        assert!(env.contains(&("PGSSLMODE".into(), "require".into())));
    }

    /// Windows'ta best-fit eşlemesiyle değişen değerler (sistem ACP'si UTF-8 ise birebir taşınır: beklenti
    /// sistemden okunur). Diğer sistemlerde hiçbir değer reddedilmez.
    fn ansi_rejects(s: &str) -> bool {
        !crate::ansi::ensure_ansi_roundtrip(s)
    }

    #[test]
    fn best_fit_paths_are_plan_errors() {
        // U+FF02 `＂` best-fit ile `"` olur: araç argv'sinde tırnağı kapatıp `-h` enjekte ederdi.
        let evil = format!("{}{SEP}x＂ -h evil.example ＂.dump", out_dir());
        let dash = format!("{}{SEP}－－clean.sql", out_dir());
        let want = |name: &str, bad: bool| if bad { vec![name.to_string()] } else { vec![] };
        let breq_at = |p: &str| BackupRequest {
            scope: BackupScope::Database,
            format: DumpFormat::Custom,
            options: BackupOptions::default(),
            output_path: p.into(),
        };
        let b = |p: &str| paths(backup_invocation(&tools(), &conn(), &breq_at(p)));
        assert_eq!(b(&evil), want("outputPath", ansi_rejects("＂")));
        assert_eq!(b(&dash), want("outputPath", ansi_rejects("－")));
        let r = |p: &str, src| {
            let req = RestoreRequest { path: p.into(), target: RestoreTarget::Existing, options: RestoreOptions::default() };
            paths(restore_invocation((&tools(), &tools()), &conn(), "postgres", &req, src))
        };
        assert_eq!(r(&evil, SourceFormat::Custom), want("path", ansi_rejects("＂")));
        assert_eq!(r(&dash, SourceFormat::Plain), want("path", ansi_rejects("－")));
        if cfg!(windows) && ansi_rejects("＂") {
            let e = backup_invocation(&tools(), &conn(), &breq_at(&evil)).unwrap_err();
            assert_eq!(e[0].message, ANSI_PATH_MESSAGE);
        }
        // Türkçe yol 1254'te birebir taşınır (sistem ACP'si buna göre karar verir).
        let tr = format!("{}{SEP}ğüşıöç.dump", out_dir());
        assert_eq!(b(&tr), want("outputPath", ansi_rejects("ğüşıöç")));
    }

    #[test]
    fn best_fit_env_values_and_tool_dir_are_plan_errors() {
        let bad = ansi_rejects("＂");
        let k = Conn { client_cert: Some(format!("{}{SEP}c＂.crt", out_dir())), host: "h－x".into(), ..ssl_conn() };
        let req = breq(BackupScope::Database, DumpFormat::Custom, BackupOptions::default(), "a.backup");
        let got = paths(backup_invocation(&tools(), &k, &req));
        assert_eq!(got, if bad { vec!["host".to_string(), "ssl.clientCert".to_string()] } else { vec![] });
        // Ad alanları da (çağıran `windows_name_problems`'i çağırmasa bile): `--dbname` bağlantı dizesi dahil.
        let n = Conn { database: "d＂b".into(), user: "u＂".into(), ..conn() };
        let want_names = if bad { vec!["user".to_string(), "database".to_string()] } else { vec![] };
        assert_eq!(paths(backup_invocation(&tools(), &n, &req)), want_names);
        let rr = RestoreRequest { path: out("kaynak"), target: RestoreTarget::Existing, options: RestoreOptions::default() };
        assert_eq!(paths(restore_invocation((&tools(), &tools()), &n, "postgres", &rr, SourceFormat::Custom)), want_names);
        let rc = RestoreRequest { target: RestoreTarget::CreateFromArchive, ..rr };
        let want_db = if bad { vec!["database".to_string()] } else { vec![] };
        assert_eq!(paths(restore_invocation((&tools(), &tools()), &conn(), "bakim＂", &rc, SourceFormat::Custom)), want_db);
        let t = ToolSet { dir: format!("{}{SEP}bin＂ -h x", out_dir()), ..tools() };
        assert_eq!(paths(backup_invocation(&t, &conn(), &req)), if bad { vec!["tool".to_string()] } else { vec![] });
    }

    #[test]
    fn plain_backup_warns_for_old_server_with_new_pg_dump() {
        assert_eq!(plain_backup_note(DumpFormat::Plain, 180001, 160004), Some(PLAIN_OLD_SERVER_NOTE));
        assert_eq!(plain_backup_note(DumpFormat::Plain, 170000, 100023), Some(PLAIN_OLD_SERVER_NOTE));
        assert_eq!(plain_backup_note(DumpFormat::Plain, 160004, 160004), None);
        assert_eq!(plain_backup_note(DumpFormat::Plain, 180001, 170002), None);
        assert_eq!(plain_backup_note(DumpFormat::Custom, 180001, 160004), None);
        assert_eq!(plain_backup_note(DumpFormat::Plain, 180001, 0), None);
        assert!(PLAIN_OLD_SERVER_NOTE.contains("transaction_timeout") && PLAIN_OLD_SERVER_NOTE.contains("custom"));
    }

    #[test]
    fn restore_invocation_marks_tool_and_ignored_errors() {
        let r = |exit_on_error, src| {
            let o = RestoreOptions { exit_on_error, ..Default::default() };
            let i = restore_ok(&rreq(RestoreTarget::Existing, o), src);
            (i.tool, i.tolerates_ignored_errors)
        };
        assert_eq!(r(false, SourceFormat::Custom), (ToolKind::PgRestore, true));
        assert_eq!(r(true, SourceFormat::Custom), (ToolKind::PgRestore, false));
        assert_eq!(r(false, SourceFormat::Plain), (ToolKind::Psql, false));
        let i = restore_ok(&rreq(RestoreTarget::Existing, RestoreOptions::default()), SourceFormat::Plain);
        assert!(i.notes.iter().any(|n| n == PLAIN_META_NOTE), "{:?}", i.notes);
        assert!(PLAIN_META_NOTE.contains(r"\!") && PLAIN_META_NOTE.contains(r"\connect"));
        let b = backup_ok(&breq(BackupScope::Database, DumpFormat::Custom, BackupOptions::default(), "a.backup"));
        assert_eq!((b.tool, b.tolerates_ignored_errors), (ToolKind::PgDump, false));
    }
}
