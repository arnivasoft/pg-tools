//! Database üreticisi: CREATE (create-only alanlar, ICU/builtin sürüm kapıları), ALTER (rename, sahip, WITH,
//! tablespace, ayarlar, yorum), DROP (`WITH (FORCE)` PG 13+).
//!
//! Spec §5 / Karar 7: TÜM ifadeler transaction dışında ve tek tek çalışır (`stmt_nt`); `CREATE/DROP DATABASE` ve
//! `SET TABLESPACE` transaction bloğunda çalışamaz, diğerleri tutarlılık için aynı kurala uyar. Hata ortasında kaç
//! ifadenin kalıcı olduğunu yönetici `committed` ile bildirir.
//!
//! Create-only alanlar (`template`, `encoding`, `localeProvider`, `lcCollate`, `lcCtype`, `providerLocale`) alter'da
//! eski ≠ yeni ise alan hatasıdır (PG bunları sonradan değiştiremez). Bağlı veritabanı / bakım veritabanı korumaları
//! (yeniden adlandırma, silme, taşıma) bu saf üreticide DEĞİL, yöneticidedir (Task 8, Karar 8).

use super::settings::{diff_settings, guc_assignment_sql, guc_name_sql, validate_old_setting_names, validate_settings,
    validate_written_settings};
use super::table::version_err;
use super::table_alter::TABLESPACE_CLEAR_MSG;
use super::type_syntax::is_spoofing_format;
use super::{BuildCtx, check_ident, check_old_ident, check_text, err, stmt_nt};
use crate::model::{ConfigSetting, DatabaseDef, FieldError, LocaleProvider};
use crate::plan::{Plan, Risk, Statement};
use crate::quote::{quote_ident, quote_literal};

/// PG SUNUCU kodlamaları (yalnızca istemci kodlaması olan `BIG5`, `GB18030`, `GBK`, `JOHAB`, `SJIS`, `SHIFT_JIS_2004`,
/// `UHC` hariç). Lookup `Encodings` da bu kümeyi döndürür.
pub const SERVER_ENCODINGS: &[&str] = &[
    "EUC_CN",
    "EUC_JP",
    "EUC_JIS_2004",
    "EUC_KR",
    "EUC_TW",
    "ISO_8859_5",
    "ISO_8859_6",
    "ISO_8859_7",
    "ISO_8859_8",
    "KOI8R",
    "KOI8U",
    "LATIN1",
    "LATIN2",
    "LATIN3",
    "LATIN4",
    "LATIN5",
    "LATIN6",
    "LATIN7",
    "LATIN8",
    "LATIN9",
    "LATIN10",
    "MULE_INTERNAL",
    "SQL_ASCII",
    "UTF8",
    "WIN866",
    "WIN874",
    "WIN1250",
    "WIN1251",
    "WIN1252",
    "WIN1253",
    "WIN1254",
    "WIN1255",
    "WIN1256",
    "WIN1257",
    "WIN1258",
];

/// Yerel ayar değeri üst sınırı (bayt).
pub const MAX_LOCALE_BYTES: usize = 255;

pub const CREATE_NOTE: &str = "Transaction dışında çalışır";
pub const TEMPLATE0_WARNING: &str = "Kodlama ya da yerel ayar şablondan farklıysa TEMPLATE = template0 seçin.";
pub const RENAME_NOTE: &str = "Veritabanına bağlı oturum olmamalı (bu uygulamanın Gezgin bağlantıları kapatılır)";
pub const TABLESPACE_NOTE: &str = "Veritabanı dosyaları taşınır; bağlı oturum olmamalı; uzun sürebilir";
pub const NO_CONNECTIONS_WARNING: &str = "Yeni bağlantılar reddedilir (Gezgin dahil)";
pub const IS_TEMPLATE_WARNING: &str = "Herkes (CREATEDB yetkili) bu veritabanını şablon olarak kopyalayabilir";
pub const DROP_NOTE: &str = "Tüm veriler kalıcı silinir";
pub const FORCE_NOTE: &str = "Bağlı tüm oturumlar sonlandırılır (bu uygulamanın Query Tool sekmeleri dahil)";
pub const CREATE_ONLY_MSG: &str = "Veritabanı oluşturulduktan sonra değiştirilemez.";
pub const IS_TEMPLATE_DROP_MSG: &str = "Şablon veritabanı silinemez; önce 'Şablon' işaretini kaldırın";
const ENCODING_MSG: &str = "Geçersiz sunucu kodlaması (ör. UTF8, LATIN5, WIN1254).";
const CONNECTION_LIMIT_MSG: &str = "Bağlantı sınırı -1 (sınırsız) ya da daha büyük olmalı.";
const PROVIDER_LOCALE_REQUIRED: &str = "icu ve builtin sağlayıcılarında sağlayıcı yerel ayarı zorunludur.";
const PROVIDER_LOCALE_FORBIDDEN: &str = "Sağlayıcı yerel ayarı yalnızca icu ya da builtin sağlayıcısıyla kullanılır.";

fn provider_sql(p: LocaleProvider) -> &'static str {
    match p {
        LocaleProvider::Libc => "libc",
        LocaleProvider::Icu => "icu",
        LocaleProvider::Builtin => "builtin",
    }
}

fn bool_sql(b: bool) -> &'static str {
    if b { "true" } else { "false" }
}

/// Yerel ayar değeri (lcCollate/lcCtype/providerLocale): boş değil, NUL/denetim karakteri ve görünmez biçim/yön
/// denetimi karakteri yok (önizleme satırını aldatmasın; ASCII'ye kısıtlanmaz: Windows yerel ayar adları ASCII dışı
/// olabilir), ≤ 255 bayt.
fn check_locale(path: &str, value: &str, e: &mut Vec<FieldError>) {
    if value.is_empty() {
        e.push(err(path, "Yerel ayar boş olamaz."));
    } else if value.contains('\0') {
        e.push(err(path, "Yerel ayar NUL karakteri içeremez."));
    } else if value.chars().any(char::is_control) {
        e.push(err(path, "Yerel ayar denetim karakteri (satır sonu, sekme vb.) içeremez."));
    } else if let Some(c) = value.chars().find(|&c| is_spoofing_format(c)) {
        e.push(err(
            path,
            format!("Yerel ayar görünmez biçim ya da yön denetimi karakteri içeremez (U+{:04X}).", c as u32),
        ));
    } else if value.len() > MAX_LOCALE_BYTES {
        e.push(err(path, format!("Yerel ayar en çok {MAX_LOCALE_BYTES} bayt olabilir.")));
    }
}

/// Kümedeki kanonik yazım (büyük/küçük harf duyarsız eşleme: `utf8` → `UTF8`); küme dışı → `None`.
pub fn canonical_encoding(value: &str) -> Option<&'static str> {
    SERVER_ENCODINGS.iter().copied().find(|s| s.eq_ignore_ascii_case(value))
}

/// İki kodlama değeri aynı mı (harfe duyarsız; yükleyici kanonik adı döndürür, istemci `utf8` yazmış olabilir).
fn same_encoding(a: &Option<String>, b: &Option<String>) -> bool {
    match (a, b) {
        (Some(a), Some(b)) => a.eq_ignore_ascii_case(b),
        (None, None) => true,
        _ => false,
    }
}

/// Kodlama: sabit sunucu kodlaması kümesi, büyük/küçük harf duyarsız (`utf8` geçerli; `utf-8` değil).
fn check_encoding(value: &str, e: &mut Vec<FieldError>) {
    if canonical_encoding(value).is_none() {
        e.push(err("encoding", ENCODING_MSG));
    }
}

fn check_connection_limit(d: &DatabaseDef, e: &mut Vec<FieldError>) {
    if d.connection_limit < -1 {
        e.push(err("connectionLimit", CONNECTION_LIMIT_MSG));
    }
}

/// Oluşturma doğrulaması (tüm alanlar).
pub fn validate(d: &DatabaseDef, ctx: &BuildCtx) -> Vec<FieldError> {
    let mut e = vec![];
    check_ident("name", &d.name, &mut e);
    if let Some(o) = &d.owner {
        check_ident("owner", o, &mut e);
    }
    if let Some(t) = &d.template {
        check_ident("template", t, &mut e);
    }
    if let Some(enc) = &d.encoding {
        check_encoding(enc, &mut e);
    }
    if let Some(p) = d.locale_provider {
        if ctx.version_num < 150000 {
            e.push(version_err("localeProvider", "LOCALE_PROVIDER", 15));
        } else if p == LocaleProvider::Builtin && ctx.version_num < 170000 {
            e.push(version_err("localeProvider", "builtin yerel ayar sağlayıcısı", 17));
        }
    }
    if let Some(v) = &d.lc_collate {
        check_locale("lcCollate", v, &mut e);
    }
    if let Some(v) = &d.lc_ctype {
        check_locale("lcCtype", v, &mut e);
    }
    let needs_provider_locale = matches!(d.locale_provider, Some(LocaleProvider::Icu | LocaleProvider::Builtin));
    match &d.provider_locale {
        Some(v) if needs_provider_locale => check_locale("providerLocale", v, &mut e),
        Some(_) => e.push(err("providerLocale", PROVIDER_LOCALE_FORBIDDEN)),
        None if needs_provider_locale => e.push(err("providerLocale", PROVIDER_LOCALE_REQUIRED)),
        None => {}
    }
    if let Some(ts) = &d.tablespace {
        // `pg_default` geçerli bir addır.
        check_ident("tablespace", ts, &mut e);
    }
    check_connection_limit(d, &mut e);
    validate_settings(&d.settings, &mut e);
    if let Some(c) = &d.comment {
        check_text("comment", c, &mut e);
    }
    e
}

fn comment_stmt(q: &str, comment: &Option<String>) -> Statement {
    let value = comment.as_deref().map_or_else(|| "NULL".to_string(), quote_literal);
    stmt_nt(format!("COMMENT ON DATABASE {q} IS {value}"), Risk::Safe, None)
}

fn set_stmt(q: &str, s: &ConfigSetting) -> Statement {
    stmt_nt(format!("ALTER DATABASE {q} SET {}", guc_assignment_sql(s)), Risk::Safe, None)
}

/// Doğrulanmış tanım varsayılır (`build::create` önce `validate` çağırır).
///
/// `CREATE DATABASE <q> [OWNER] [TEMPLATE] [ENCODING] [LOCALE_PROVIDER] [LC_COLLATE] [LC_CTYPE]
/// [ICU_LOCALE | BUILTIN_LOCALE] [TABLESPACE] ALLOW_CONNECTIONS CONNECTION LIMIT IS_TEMPLATE` (son üçü her zaman,
/// deterministik), ardından ayarlar (ad sırasıyla, her biri ayrı `ALTER DATABASE … SET`) ve yorum.
pub fn create(d: &DatabaseDef) -> Plan {
    let q = quote_ident(&d.name);
    let mut sql = format!("CREATE DATABASE {q}");
    if let Some(o) = &d.owner {
        sql.push_str(&format!(" OWNER = {}", quote_ident(o)));
    }
    if let Some(t) = &d.template {
        sql.push_str(&format!(" TEMPLATE = {}", quote_ident(t)));
    }
    if let Some(enc) = &d.encoding {
        // Kanonik yazım (inceleme M-3): yükleyici `pg_encoding_to_char` ile aynı adı döndürür → create → load eşit.
        // Doğrulama küme dışını eledi; yine de bulunamazsa yazıldığı gibi (quote_literal).
        let enc = canonical_encoding(enc).unwrap_or(enc);
        sql.push_str(&format!(" ENCODING = {}", quote_literal(enc)));
    }
    if let Some(p) = d.locale_provider {
        sql.push_str(&format!(" LOCALE_PROVIDER = {}", provider_sql(p)));
    }
    if let Some(v) = &d.lc_collate {
        sql.push_str(&format!(" LC_COLLATE = {}", quote_literal(v)));
    }
    if let Some(v) = &d.lc_ctype {
        sql.push_str(&format!(" LC_CTYPE = {}", quote_literal(v)));
    }
    if let Some(v) = &d.provider_locale {
        // Doğrulama: provider_locale yalnızca icu/builtin ile dolu olabilir.
        let kw = if d.locale_provider == Some(LocaleProvider::Builtin) { "BUILTIN_LOCALE" } else { "ICU_LOCALE" };
        sql.push_str(&format!(" {kw} = {}", quote_literal(v)));
    }
    if let Some(ts) = &d.tablespace {
        sql.push_str(&format!(" TABLESPACE = {}", quote_ident(ts)));
    }
    sql.push_str(&format!(
        " ALLOW_CONNECTIONS = {} CONNECTION LIMIT = {} IS_TEMPLATE = {}",
        bool_sql(d.allow_connections),
        d.connection_limit,
        bool_sql(d.is_template)
    ));
    let mut statements = vec![stmt_nt(sql, Risk::Safe, Some(CREATE_NOTE))];
    let (_, set) = diff_settings(&[], &d.settings);
    statements.extend(set.into_iter().map(|s| set_stmt(&q, s)));
    if d.comment.is_some() {
        statements.push(comment_stmt(&q, &d.comment));
    }
    let locale_given = d.encoding.is_some()
        || d.locale_provider.is_some()
        || d.lc_collate.is_some()
        || d.lc_ctype.is_some()
        || d.provider_locale.is_some();
    let mut warnings = vec![];
    if locale_given && d.template.as_deref() != Some("template0") {
        warnings.push(TEMPLATE0_WARNING.to_string());
    }
    // ALTER ile aynı uyarılar (inceleme N-2).
    if !d.allow_connections {
        warnings.push(NO_CONNECTIONS_WARNING.to_string());
    }
    if d.is_template {
        warnings.push(IS_TEMPLATE_WARNING.to_string());
    }
    Plan { statements, warnings }
}

/// Alter doğrulaması: eskinin yalnızca SQL'e giren alanları (ad, RESET hedefi ayar adları; gevşek), yeninin yalnızca
/// değişen ya da ifadeye yazılan alanları (katı); create-only alanlar eşit olmalı.
fn validate_alter(old: &DatabaseDef, new: &DatabaseDef) -> Vec<FieldError> {
    let mut e = vec![];
    check_old_ident("old.name", &old.name, &mut e);
    validate_old_setting_names("old.", &old.settings, &mut e);
    if !e.is_empty() {
        return e;
    }
    if new.name != old.name {
        check_ident("name", &new.name, &mut e);
    }
    if let Some(o) = &new.owner
        && new.owner != old.owner
    {
        check_ident("owner", o, &mut e);
    }
    let create_only: [(&str, bool); 6] = [
        ("template", old.template != new.template),
        // Harfe duyarsız: `utf8` ile yüklü `UTF8` aynı kodlamadır.
        ("encoding", !same_encoding(&old.encoding, &new.encoding)),
        ("localeProvider", old.locale_provider != new.locale_provider),
        ("lcCollate", old.lc_collate != new.lc_collate),
        ("lcCtype", old.lc_ctype != new.lc_ctype),
        ("providerLocale", old.provider_locale != new.provider_locale),
    ];
    for (path, changed) in create_only {
        if changed {
            e.push(err(path, CREATE_ONLY_MSG));
        }
    }
    match &new.tablespace {
        Some(ts) if new.tablespace != old.tablespace => check_ident("tablespace", ts, &mut e),
        // ALTER ile tablespace "kaldırılamaz" (yükleyici her zaman Some döndürür).
        None if old.tablespace.is_some() => e.push(err("tablespace", TABLESPACE_CLEAR_MSG)),
        _ => {}
    }
    if new.connection_limit != old.connection_limit {
        check_connection_limit(new, &mut e);
    }
    // Ayarlar: yalnızca yazılacak (eklenen/değişen) girdiler doğrulanır; yinelenen ad her zaman hata (M-1).
    validate_written_settings(&old.settings, &new.settings, &mut e);
    if let Some(c) = &new.comment
        && new.comment != old.comment
    {
        check_text("comment", c, &mut e);
    }
    e
}

/// Sıra (sonrakiler YENİ adı kullanır): rename, WITH (yalnız değişenler: ALLOW_CONNECTIONS, CONNECTION LIMIT,
/// IS_TEMPLATE), set tablespace, ayarlar (RESET sonra SET, ad sırasıyla), comment, owner.
///
/// `OWNER TO` bilerek EN SONDADIR (inceleme M-2): ifadeler transaction dışında tek tek çalışır ve sonraki her
/// `ALTER DATABASE`/`COMMENT` sahiplik ister. Süper kullanıcı olmayan sahip veritabanını INHERIT'siz bir role
/// devrederse, sahiplik önce değişseydi kalan ifadeler 42501 ile başarısız olur ve plan yarım kalırdı.
pub fn alter(old: &DatabaseDef, new: &DatabaseDef, _ctx: &BuildCtx) -> Result<Plan, Vec<FieldError>> {
    let errs = validate_alter(old, new);
    if !errs.is_empty() {
        return Err(errs);
    }
    let mut statements = vec![];
    let mut warnings = vec![];
    let q = quote_ident(&new.name);

    if old.name != new.name {
        // Bağlı veritabanı yeniden adlandırılamaz (PG 55006 / bakım veritabanı kapısı yöneticide, Task 8).
        statements.push(stmt_nt(
            format!("ALTER DATABASE {} RENAME TO {q}", quote_ident(&old.name)),
            Risk::Safe,
            Some(RENAME_NOTE),
        ));
    }
    let mut with = vec![];
    if new.allow_connections != old.allow_connections {
        with.push(format!("ALLOW_CONNECTIONS {}", bool_sql(new.allow_connections)));
        if !new.allow_connections {
            warnings.push(NO_CONNECTIONS_WARNING.to_string());
        }
    }
    if new.connection_limit != old.connection_limit {
        with.push(format!("CONNECTION LIMIT {}", new.connection_limit));
    }
    if new.is_template != old.is_template {
        with.push(format!("IS_TEMPLATE {}", bool_sql(new.is_template)));
        if new.is_template {
            warnings.push(IS_TEMPLATE_WARNING.to_string());
        }
    }
    if !with.is_empty() {
        statements.push(stmt_nt(format!("ALTER DATABASE {q} WITH {}", with.join(" ")), Risk::Safe, None));
    }
    if let Some(ts) = &new.tablespace
        && new.tablespace != old.tablespace
    {
        statements.push(stmt_nt(
            format!("ALTER DATABASE {q} SET TABLESPACE {}", quote_ident(ts)),
            Risk::Safe,
            Some(TABLESPACE_NOTE),
        ));
    }
    let (removed, set) = diff_settings(&old.settings, &new.settings);
    for name in removed {
        statements.push(stmt_nt(format!("ALTER DATABASE {q} RESET {}", guc_name_sql(name)), Risk::Safe, None));
    }
    statements.extend(set.into_iter().map(|s| set_stmt(&q, s)));
    if old.comment != new.comment {
        statements.push(comment_stmt(&q, &new.comment));
    }
    if let Some(o) = &new.owner
        && new.owner != old.owner
    {
        statements.push(stmt_nt(format!("ALTER DATABASE {q} OWNER TO {}", quote_ident(o)), Risk::Safe, None));
    }
    Ok(Plan { statements, warnings })
}

/// `DROP DATABASE <q>` / PG 13+ `DROP DATABASE <q> WITH (FORCE)` — her zaman DataLoss (Karar 3). Ortak kurallar
/// (`force` yalnızca Database, CASCADE yok, gevşek kimlik) `build::drop_with`'tedir. Bağlı/bakım veritabanı koruması
/// yöneticidedir (Task 8); burada reddedilmez.
pub fn drop(d: &DatabaseDef, force: bool, ctx: &BuildCtx) -> Result<Plan, Vec<FieldError>> {
    let mut e = vec![];
    if d.is_template {
        e.push(err("isTemplate", IS_TEMPLATE_DROP_MSG));
    }
    if force && ctx.version_num < 130000 {
        e.push(version_err("force", "WITH (FORCE)", 13));
    }
    if !e.is_empty() {
        return Err(e);
    }
    let q = quote_ident(&d.name);
    let statement = if force {
        stmt_nt(format!("DROP DATABASE {q} WITH (FORCE)"), Risk::DataLoss, Some(&format!("{DROP_NOTE}. {FORCE_NOTE}")))
    } else {
        stmt_nt(format!("DROP DATABASE {q}"), Risk::DataLoss, Some(DROP_NOTE))
    };
    Ok(Plan { statements: vec![statement], warnings: vec![] })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::build::{
        DropOptions, alter as balter, assert_single_statements, create as bcreate, drop_with, render, validate_identity,
    };
    use crate::model::ObjectDef;

    fn d() -> DatabaseDef {
        DatabaseDef {
            name: "d".into(),
            owner: None,
            template: None,
            encoding: None,
            locale_provider: None,
            lc_collate: None,
            lc_ctype: None,
            provider_locale: None,
            tablespace: None,
            connection_limit: -1,
            is_template: false,
            allow_connections: true,
            settings: vec![],
            comment: None,
        }
    }
    /// Yüklenmiş biçim: yükleyici tablespace'i her zaman Some, create-only alanları Some (template None) döndürür.
    fn loaded() -> DatabaseDef {
        DatabaseDef {
            owner: Some("o".into()),
            encoding: Some("UTF8".into()),
            lc_collate: Some("C".into()),
            lc_ctype: Some("C".into()),
            tablespace: Some("pg_default".into()),
            ..d()
        }
    }
    fn s(n: &str, v: &str) -> ConfigSetting {
        ConfigSetting { name: n.into(), value: v.into() }
    }
    fn o(d: DatabaseDef) -> ObjectDef {
        ObjectDef::Database(d)
    }
    fn ctx_v(v: u32) -> BuildCtx {
        BuildCtx { version_num: v, ..Default::default() }
    }
    fn ctx() -> BuildCtx {
        ctx_v(170000)
    }
    fn sqls(p: &Plan) -> Vec<&str> {
        p.statements.iter().map(|s| s.sql.as_str()).collect()
    }
    fn paths(errs: Vec<FieldError>) -> Vec<String> {
        errs.into_iter().map(|e| e.path).collect()
    }
    fn create_ok(d: DatabaseDef, v: u32) -> Plan {
        let p = bcreate(&o(d), &ctx_v(v)).unwrap();
        assert_single_statements(&p);
        p
    }
    fn create_err(d: DatabaseDef, v: u32) -> Vec<String> {
        paths(bcreate(&o(d), &ctx_v(v)).unwrap_err())
    }
    fn alter_ok(old: &DatabaseDef, new: &DatabaseDef) -> Plan {
        let p = balter(&o(old.clone()), &o(new.clone()), &ctx()).unwrap();
        assert_single_statements(&p);
        p
    }
    fn alter_err(old: &DatabaseDef, new: &DatabaseDef) -> Vec<FieldError> {
        balter(&o(old.clone()), &o(new.clone()), &ctx()).unwrap_err()
    }
    fn drop_ok(d: DatabaseDef, force: bool, v: u32) -> Plan {
        let p = drop_with(&o(d), DropOptions { cascade: false, force }, &ctx_v(v)).unwrap();
        assert_single_statements(&p);
        p
    }
    fn drop_err(d: DatabaseDef, force: bool, v: u32) -> Vec<FieldError> {
        drop_with(&o(d), DropOptions { cascade: false, force }, &ctx_v(v)).unwrap_err()
    }

    const TAIL: &str = "ALLOW_CONNECTIONS = true CONNECTION LIMIT = -1 IS_TEMPLATE = false";

    #[test]
    fn create_variants() {
        let mut out = String::new();
        // Yalnız ad.
        let p = create_ok(d(), 170000);
        assert_eq!(sqls(&p), [format!("CREATE DATABASE d {TAIL}")]);
        assert_eq!(p.statements[0].risk, Risk::Safe);
        assert_eq!(p.statements[0].note.as_deref(), Some(CREATE_NOTE));
        assert!(p.warnings.is_empty());
        out.push_str(&render(&p));
        out.push('\n');
        // Tüm seçenekler (libc; PG 15+'ta LOCALE_PROVIDER yazılır), template0 → uyarı yok.
        let all = DatabaseDef {
            name: "Sales DB".into(),
            owner: Some("Bob".into()),
            template: Some("template0".into()),
            encoding: Some("utf8".into()),
            locale_provider: Some(LocaleProvider::Libc),
            lc_collate: Some("tr_TR.UTF-8".into()),
            lc_ctype: Some("tr_TR.UTF-8".into()),
            tablespace: Some("Fast Disk".into()),
            connection_limit: 25,
            is_template: true,
            allow_connections: false,
            ..d()
        };
        let p = create_ok(all.clone(), 150000);
        assert_eq!(
            sqls(&p),
            ["CREATE DATABASE \"Sales DB\" OWNER = \"Bob\" TEMPLATE = template0 ENCODING = 'UTF8' LOCALE_PROVIDER = libc \
              LC_COLLATE = 'tr_TR.UTF-8' LC_CTYPE = 'tr_TR.UTF-8' TABLESPACE = \"Fast Disk\" ALLOW_CONNECTIONS = false \
              CONNECTION LIMIT = 25 IS_TEMPLATE = true"]
        );
        // ALTER ile aynı uyarılar (inceleme N-2): bağlantı kapalı, şablon.
        assert_eq!(p.warnings, [NO_CONNECTIONS_WARNING, IS_TEMPLATE_WARNING]);
        out.push_str(&render(&p));
        out.push('\n');
        // libc, sağlayıcısız (PG 10) ve şablon template0 değil → uyarı.
        let p = create_ok(DatabaseDef { locale_provider: None, template: Some("template1".into()), ..all.clone() }, 100000);
        assert!(!p.statements[0].sql.contains("LOCALE_PROVIDER"));
        assert_eq!(p.warnings, [TEMPLATE0_WARNING, NO_CONNECTIONS_WARNING, IS_TEMPLATE_WARNING]);
        out.push_str(&render(&p));
        out.push('\n');
        // Yalnızca kodlama ve şablonsuz → uyarı.
        let p = create_ok(DatabaseDef { encoding: Some("LATIN5".into()), ..d() }, 100000);
        assert_eq!(p.warnings, [TEMPLATE0_WARNING]);
        // ICU (PG 15+).
        let icu = DatabaseDef {
            locale_provider: Some(LocaleProvider::Icu),
            provider_locale: Some("tr-TR".into()),
            template: Some("template0".into()),
            ..d()
        };
        assert_eq!(create_err(icu.clone(), 140000), ["localeProvider"]);
        let p = create_ok(icu.clone(), 150000);
        assert_eq!(
            sqls(&p),
            [format!("CREATE DATABASE d TEMPLATE = template0 LOCALE_PROVIDER = icu ICU_LOCALE = 'tr-TR' {TAIL}")]
        );
        out.push_str(&render(&p));
        out.push('\n');
        // builtin (PG 17+); PG 15–16'da da hata.
        let builtin = DatabaseDef {
            locale_provider: Some(LocaleProvider::Builtin),
            provider_locale: Some("C.UTF-8".into()),
            ..icu.clone()
        };
        assert_eq!(create_err(builtin.clone(), 160000), ["localeProvider"]);
        assert_eq!(create_err(builtin.clone(), 140000), ["localeProvider"]);
        let e = bcreate(&o(builtin.clone()), &ctx_v(160000)).unwrap_err();
        assert!(e[0].message.contains("PG 17+"), "{e:?}");
        let p = create_ok(builtin.clone(), 170000);
        assert_eq!(
            sqls(&p),
            [format!("CREATE DATABASE d TEMPLATE = template0 LOCALE_PROVIDER = builtin BUILTIN_LOCALE = 'C.UTF-8' {TAIL}")]
        );
        out.push_str(&render(&p));
        out.push('\n');
        // Ayarlar (ad sırasıyla, her biri ayrı) + yorum sırası.
        let p = create_ok(
            DatabaseDef {
                settings: vec![s("work_mem", "64MB"), s("search_path", "\"$user\", public"), s("app.mode", "x")],
                comment: Some("it's".into()),
                ..d()
            },
            170000,
        );
        assert_eq!(
            sqls(&p),
            [
                format!("CREATE DATABASE d {TAIL}").as_str(),
                "ALTER DATABASE d SET app.\"mode\" = 'x'",
                "ALTER DATABASE d SET search_path = '$user', 'public'",
                "ALTER DATABASE d SET work_mem = '64MB'",
                "COMMENT ON DATABASE d IS 'it''s'",
            ]
        );
        assert!(p.statements.iter().all(|s| s.risk == Risk::Safe));
        assert!(p.statements[1..].iter().all(|s| s.note.is_none()));
        out.push_str(&render(&p));
        insta::assert_snapshot!(out);
    }

    #[test]
    fn provider_locale_required_or_forbidden() {
        for p in [LocaleProvider::Icu, LocaleProvider::Builtin] {
            let e = bcreate(&o(DatabaseDef { locale_provider: Some(p), ..d() }), &ctx()).unwrap_err();
            assert_eq!(paths(e.clone()), ["providerLocale"], "{p:?}");
            assert_eq!(e[0].message, PROVIDER_LOCALE_REQUIRED);
            assert_eq!(
                create_err(DatabaseDef { locale_provider: Some(p), provider_locale: Some(String::new()), ..d() }, 170000),
                ["providerLocale"]
            );
        }
        for p in [None, Some(LocaleProvider::Libc)] {
            let e = bcreate(&o(DatabaseDef { locale_provider: p, provider_locale: Some("tr-TR".into()), ..d() }), &ctx())
                .unwrap_err();
            assert_eq!(paths(e.clone()), ["providerLocale"], "{p:?}");
            assert_eq!(e[0].message, PROVIDER_LOCALE_FORBIDDEN);
        }
    }

    #[test]
    fn every_statement_is_non_transactional() {
        let full = DatabaseDef {
            owner: Some("a".into()),
            template: Some("template0".into()),
            encoding: Some("UTF8".into()),
            locale_provider: Some(LocaleProvider::Icu),
            provider_locale: Some("und".into()),
            tablespace: Some("t".into()),
            settings: vec![s("work_mem", "1MB")],
            comment: Some("c".into()),
            ..d()
        };
        let changed = DatabaseDef {
            name: "e".into(),
            owner: Some("b".into()),
            tablespace: Some("u".into()),
            connection_limit: 3,
            is_template: true,
            allow_connections: false,
            settings: vec![s("search_path", "public")],
            comment: None,
            ..full.clone()
        };
        let plans = [
            create_ok(d(), 170000),
            create_ok(full.clone(), 170000),
            alter_ok(&full, &changed),
            drop_ok(d(), false, 170000),
            drop_ok(d(), true, 170000),
        ];
        let mut n = 0;
        for p in &plans {
            for st in &p.statements {
                assert!(!st.transactional, "{}", st.sql);
                assert!(!st.redacted, "{}", st.sql);
                n += 1;
            }
        }
        // create 1 + create 3 (create, set, comment) + alter 7 (rename, owner, with, tablespace, reset, set, comment)
        // + drop 1 + drop 1.
        assert_eq!(n, 13);
        assert_eq!(plans[2].statements.len(), 7);
    }

    #[test]
    fn alter_single_changes() {
        let base = DatabaseDef { settings: vec![s("work_mem", "1MB")], comment: Some("c".into()), ..loaded() };
        type Case = (DatabaseDef, Vec<&'static str>, Option<&'static str>, Vec<&'static str>);
        let cases: Vec<Case> = vec![
            (DatabaseDef { name: "e".into(), ..base.clone() }, vec!["ALTER DATABASE d RENAME TO e"], Some(RENAME_NOTE), vec![]),
            (DatabaseDef { owner: Some("p".into()), ..base.clone() }, vec!["ALTER DATABASE d OWNER TO p"], None, vec![]),
            // Sahip `None`'a çekilemez: yok sayılır.
            (DatabaseDef { owner: None, ..base.clone() }, vec![], None, vec![]),
            (
                DatabaseDef { allow_connections: false, ..base.clone() },
                vec!["ALTER DATABASE d WITH ALLOW_CONNECTIONS false"],
                None,
                vec![NO_CONNECTIONS_WARNING],
            ),
            (
                DatabaseDef { connection_limit: 10, ..base.clone() },
                vec!["ALTER DATABASE d WITH CONNECTION LIMIT 10"],
                None,
                vec![],
            ),
            (
                DatabaseDef { is_template: true, ..base.clone() },
                vec!["ALTER DATABASE d WITH IS_TEMPLATE true"],
                None,
                vec![IS_TEMPLATE_WARNING],
            ),
            // WITH birleşimi: yalnız değişenler, sabit sıra.
            (
                DatabaseDef { allow_connections: false, connection_limit: 0, is_template: true, ..base.clone() },
                vec!["ALTER DATABASE d WITH ALLOW_CONNECTIONS false CONNECTION LIMIT 0 IS_TEMPLATE true"],
                None,
                vec![NO_CONNECTIONS_WARNING, IS_TEMPLATE_WARNING],
            ),
            (
                DatabaseDef { tablespace: Some("Fast".into()), ..base.clone() },
                vec!["ALTER DATABASE d SET TABLESPACE \"Fast\""],
                Some(TABLESPACE_NOTE),
                vec![],
            ),
            // Ayarlar: önce RESET (ad sırası), sonra SET (ad sırası).
            (
                DatabaseDef { settings: vec![s("zz.b", "1"), s("aa", "2")], ..base.clone() },
                vec!["ALTER DATABASE d RESET work_mem", "ALTER DATABASE d SET aa = '2'", "ALTER DATABASE d SET zz.b = '1'"],
                None,
                vec![],
            ),
            (
                DatabaseDef { settings: vec![s("work_mem", "2MB")], ..base.clone() },
                vec!["ALTER DATABASE d SET work_mem = '2MB'"],
                None,
                vec![],
            ),
            (DatabaseDef { comment: None, ..base.clone() }, vec!["COMMENT ON DATABASE d IS NULL"], None, vec![]),
            (DatabaseDef { comment: Some("x\\y".into()), ..base.clone() }, vec!["COMMENT ON DATABASE d IS E'x\\\\y'"], None, vec![]),
        ];
        let mut out = String::new();
        for (new, want, note, warns) in cases {
            let p = alter_ok(&base, &new);
            assert_eq!(sqls(&p), want, "{new:?}");
            assert!(p.statements.iter().all(|s| s.risk == Risk::Safe && !s.transactional));
            if let Some(st) = p.statements.first() {
                assert_eq!(st.note.as_deref(), note, "{new:?}");
            }
            assert_eq!(p.warnings, warns, "{new:?}");
            out.push_str(&render(&p));
            out.push('\n');
        }
        // Açma yönleri uyarısız (ALLOW_CONNECTIONS true, IS_TEMPLATE false).
        let closed = DatabaseDef { allow_connections: false, is_template: true, ..base.clone() };
        let p = alter_ok(&closed, &base);
        assert_eq!(sqls(&p), ["ALTER DATABASE d WITH ALLOW_CONNECTIONS true IS_TEMPLATE false"]);
        assert!(p.warnings.is_empty());
        insta::assert_snapshot!(out);
    }

    #[test]
    fn tablespace_cannot_be_cleared() {
        let e = alter_err(&loaded(), &DatabaseDef { tablespace: None, ..loaded() });
        assert_eq!(paths(e.clone()), ["tablespace"]);
        assert_eq!(e[0].message, TABLESPACE_CLEAR_MSG);
        assert_eq!(paths(alter_err(&loaded(), &DatabaseDef { tablespace: Some("a\nb".into()), ..loaded() })), ["tablespace"]);
        // Hiç tablespace yoksa (yükleme dışı) None → None serbest.
        assert!(alter_ok(&d(), &d()).statements.is_empty());
    }

    #[test]
    fn create_only_fields_cannot_change() {
        let base = loaded();
        let cases: Vec<(DatabaseDef, &str)> = vec![
            (DatabaseDef { template: Some("template0".into()), ..base.clone() }, "template"),
            (DatabaseDef { encoding: Some("LATIN5".into()), ..base.clone() }, "encoding"),
            (DatabaseDef { encoding: None, ..base.clone() }, "encoding"),
            (DatabaseDef { locale_provider: Some(LocaleProvider::Libc), ..base.clone() }, "localeProvider"),
            (DatabaseDef { lc_collate: Some("tr_TR".into()), ..base.clone() }, "lcCollate"),
            (DatabaseDef { lc_ctype: Some("tr_TR".into()), ..base.clone() }, "lcCtype"),
            (DatabaseDef { provider_locale: Some("und".into()), ..base.clone() }, "providerLocale"),
        ];
        for (new, path) in cases {
            let e = alter_err(&base, &new);
            assert_eq!(paths(e.clone()), [path], "{new:?}");
            assert_eq!(e[0].message, CREATE_ONLY_MSG);
        }
        // Hepsi birden: sabit sıra.
        let all = DatabaseDef {
            template: Some("t".into()),
            encoding: Some("SQL_ASCII".into()),
            locale_provider: Some(LocaleProvider::Icu),
            lc_collate: None,
            lc_ctype: Some("x".into()),
            provider_locale: Some("und".into()),
            ..base.clone()
        };
        assert_eq!(
            paths(alter_err(&base, &all)),
            ["template", "encoding", "localeProvider", "lcCollate", "lcCtype", "providerLocale"]
        );
        // Değişmeyen create-only değerler (kurala aykırı olsa bile) doğrulanmaz ve yazılmaz.
        let odd = DatabaseDef { encoding: Some("utf-8".into()), lc_collate: Some("a\nb".into()), ..base.clone() };
        assert!(alter_ok(&odd, &odd).statements.is_empty());
        assert_eq!(sqls(&alter_ok(&odd, &DatabaseDef { name: "e".into(), ..odd.clone() })), ["ALTER DATABASE d RENAME TO e"]);
    }

    #[test]
    fn alter_combined_order_uses_new_name() {
        let old = DatabaseDef {
            settings: vec![s("work_mem", "1MB"), s("timezone", "UTC")],
            comment: Some("c".into()),
            ..loaded()
        };
        let new = DatabaseDef {
            name: "Select".into(),
            owner: Some("p".into()),
            connection_limit: 5,
            is_template: true,
            tablespace: Some("fast".into()),
            settings: vec![s("work_mem", "2MB"), s("app.user", "x")],
            comment: Some("yeni".into()),
            ..old.clone()
        };
        let p = alter_ok(&old, &new);
        assert_eq!(
            sqls(&p),
            [
                "ALTER DATABASE d RENAME TO \"Select\"",
                "ALTER DATABASE \"Select\" WITH CONNECTION LIMIT 5 IS_TEMPLATE true",
                "ALTER DATABASE \"Select\" SET TABLESPACE fast",
                "ALTER DATABASE \"Select\" RESET timezone",
                "ALTER DATABASE \"Select\" SET app.\"user\" = 'x'",
                "ALTER DATABASE \"Select\" SET work_mem = '2MB'",
                "COMMENT ON DATABASE \"Select\" IS 'yeni'",
                // M-2: sahip en sonda (transaction yok; önce devretmek sonrakileri 42501'e düşürebilirdi).
                "ALTER DATABASE \"Select\" OWNER TO p",
            ]
        );
        assert_eq!(p.warnings, [IS_TEMPLATE_WARNING]);
        assert_eq!(p.risk(), Risk::Safe);
        insta::assert_snapshot!(render(&p));
    }

    #[test]
    fn alter_noop_is_empty() {
        let x = DatabaseDef {
            settings: vec![s("work_mem", "1MB"), s("search_path", "\"$user\", public")],
            comment: Some("c".into()),
            connection_limit: 7,
            is_template: true,
            allow_connections: false,
            ..loaded()
        };
        for x in [x.clone(), d(), DatabaseDef { locale_provider: Some(LocaleProvider::Icu), provider_locale: Some("und".into()), ..x }] {
            let p = alter_ok(&x, &x);
            assert!(p.statements.is_empty() && p.warnings.is_empty(), "{x:?}");
        }
    }

    #[test]
    fn alter_validates_only_changed_fields() {
        // Yüklü ama kurala aykırı değerler (sahip, yorum, ayar değeri, büyük harfli ayar adı) değişmedikçe engellemez.
        let old = DatabaseDef {
            owner: Some("a\u{200b}b".into()),
            comment: Some("a\0".into()),
            settings: vec![s("TimeZone", "UTC"), s("work_mem", "a\nb")],
            ..loaded()
        };
        let p = alter_ok(&old, &DatabaseDef { connection_limit: 3, ..old.clone() });
        assert_eq!(sqls(&p), ["ALTER DATABASE d WITH CONNECTION LIMIT 3"]);
        // Eski taraf ayar adı RESET hedefi: kanonik ad gevşek kabul, tırnaklı yazılır.
        let p = alter_ok(&old, &DatabaseDef { settings: vec![s("work_mem", "a\nb")], ..old.clone() });
        assert_eq!(sqls(&p), ["ALTER DATABASE d RESET \"TimeZone\""]);
        // Değişen/eklenen girdiler katı: yeni değer, yeni ad (küçük harf zorunlu), yinelenen ad.
        let e = alter_err(&old, &DatabaseDef { settings: vec![s("TimeZone", "UTC"), s("work_mem", "b\nc")], ..old.clone() });
        assert_eq!(paths(e), ["settings[1].value"]);
        let e = alter_err(&old, &DatabaseDef { settings: vec![s("DateStyle", "ISO")], ..old.clone() });
        assert_eq!(paths(e.clone()), ["settings[0].name"]);
        assert!(e[0].message.contains("'datestyle'"), "{e:?}");
        let e = alter_err(&loaded(), &DatabaseDef { settings: vec![s("a", "1"), s("a", "2")], ..loaded() });
        assert_eq!(paths(e), ["settings[1].name"]);
        // M-1: yinelenen girdi yüklü değerle aynı olsa da hata (iki sıra; aksi halde `a=2` sessizce kaybolurdu).
        let a1 = DatabaseDef { settings: vec![s("a", "1")], ..loaded() };
        for list in [vec![s("a", "2"), s("a", "1")], vec![s("a", "1"), s("a", "2")], vec![s("a", "1"), s("a", "1")]] {
            let e = alter_err(&a1, &DatabaseDef { settings: list.clone(), ..loaded() });
            assert_eq!(paths(e), ["settings[1].name"], "{list:?}");
        }
        // Değişen sahip/yorum katı.
        assert_eq!(paths(alter_err(&old, &DatabaseDef { owner: Some("x\u{202e}".into()), ..old.clone() })), ["owner"]);
        assert_eq!(paths(alter_err(&old, &DatabaseDef { comment: Some("b\0".into()), ..old.clone() })), ["comment"]);
        // Eski taraf ayar adı: yalnızca boş/NUL/denetim karakteri, `old.` önekiyle.
        let bad_old = DatabaseDef { settings: vec![s("ok", "1"), s("a\nb", "1")], ..loaded() };
        assert_eq!(paths(alter_err(&bad_old, &loaded())), ["old.settings[1].name"]);
    }

    #[test]
    fn connection_limit_bounds() {
        let e = bcreate(&o(DatabaseDef { connection_limit: -2, ..d() }), &ctx()).unwrap_err();
        assert_eq!(paths(e.clone()), ["connectionLimit"]);
        assert_eq!(e[0].message, CONNECTION_LIMIT_MSG);
        assert_eq!(create_err(DatabaseDef { connection_limit: i32::MIN, ..d() }, 170000), ["connectionLimit"]);
        assert_eq!(paths(alter_err(&d(), &DatabaseDef { connection_limit: -2, ..d() })), ["connectionLimit"]);
        for n in [-1, 0, i32::MAX] {
            let p = create_ok(DatabaseDef { connection_limit: n, ..d() }, 170000);
            assert!(p.statements[0].sql.contains(&format!("CONNECTION LIMIT = {n} ")), "{n}");
            let p = alter_ok(&DatabaseDef { connection_limit: 5, ..d() }, &DatabaseDef { connection_limit: n, ..d() });
            assert_eq!(sqls(&p), [format!("ALTER DATABASE d WITH CONNECTION LIMIT {n}")]);
        }
    }

    #[test]
    fn encoding_set_is_case_insensitive() {
        for bad in ["BIG5", "utf-8", "UTF-8", "SJIS", "GBK", "", "UTF8'; --", "utf8\0"] {
            let e = bcreate(&o(DatabaseDef { encoding: Some(bad.into()), ..d() }), &ctx()).unwrap_err();
            assert_eq!(paths(e.clone()), ["encoding"], "{bad:?}");
            assert_eq!(e[0].message, ENCODING_MSG);
        }
        // Kanonik yazım (inceleme M-3): kümedeki karşılık yazılır.
        for (ok, canon) in [
            ("utf8", "UTF8"),
            ("UTF8", "UTF8"),
            ("Latin5", "LATIN5"),
            ("win1254", "WIN1254"),
            ("sql_ascii", "SQL_ASCII"),
            ("euc_jis_2004", "EUC_JIS_2004"),
        ] {
            let p = create_ok(DatabaseDef { encoding: Some(ok.into()), ..d() }, 170000);
            assert!(p.statements[0].sql.contains(&format!("ENCODING = '{canon}' ")), "{ok:?}");
            assert_eq!(canonical_encoding(ok), Some(canon));
        }
        assert_eq!(canonical_encoding("utf-8"), None);
        // Alter: create-only karşılaştırması harfe duyarsız (yüklü `UTF8` ile istemcinin `utf8`'i aynı).
        let up = DatabaseDef { encoding: Some("UTF8".into()), ..d() };
        let low = DatabaseDef { encoding: Some("utf8".into()), ..d() };
        assert!(alter_ok(&up, &low).statements.is_empty());
        assert!(alter_ok(&low, &up).statements.is_empty());
        for other in [Some("LATIN5"), Some("utf-8"), None] {
            let e = alter_err(&up, &DatabaseDef { encoding: other.map(Into::into), ..d() });
            assert_eq!(paths(e), ["encoding"], "{other:?}");
        }
        assert_eq!(paths(alter_err(&d(), &low)), ["encoding"]);
        assert_eq!(SERVER_ENCODINGS.len(), 35);
        for client_only in ["BIG5", "GB18030", "GBK", "JOHAB", "SJIS", "SHIFT_JIS_2004", "UHC"] {
            assert!(!SERVER_ENCODINGS.contains(&client_only), "{client_only}");
        }
    }

    #[test]
    fn locale_values_are_validated() {
        let long = "a".repeat(256);
        for (bad, why) in [("", "boş"), ("en_US\0", "NUL"), ("a\nb", "denetim"), ("a\u{85}b", "denetim"), (long.as_str(), "bayt")] {
            for path in ["lcCollate", "lcCtype", "providerLocale"] {
                let mut x = DatabaseDef { locale_provider: Some(LocaleProvider::Icu), provider_locale: Some("und".into()), ..d() };
                match path {
                    "lcCollate" => x.lc_collate = Some(bad.into()),
                    "lcCtype" => x.lc_ctype = Some(bad.into()),
                    _ => x.provider_locale = Some(bad.into()),
                }
                let e = bcreate(&o(x), &ctx()).unwrap_err();
                assert_eq!(paths(e.clone()), [path], "{bad:?}");
                assert!(e[0].message.contains(why) || why == "bayt" && e[0].message.contains("255"), "{e:?}");
            }
        }
        assert!(create_ok(DatabaseDef { lc_collate: Some("a".repeat(255)), ..d() }, 170000).statements.len() == 1);
        // N-1: görünmez biçim/yön denetimi karakterleri reddedilir; ASCII dışı meşru adlar serbest.
        for c in ['\u{202e}', '\u{2067}', '\u{200b}', '\u{061c}', '\u{feff}'] {
            let e = bcreate(&o(DatabaseDef { lc_ctype: Some(format!("C{c}x")), ..d() }), &ctx()).unwrap_err();
            assert_eq!(paths(e.clone()), ["lcCtype"], "{c:?}");
            assert!(e[0].message.contains(&format!("U+{:04X}", c as u32)), "{e:?}");
        }
        for ok in ["Turkish_Türkiye.1254", "Norwegian Bokmål_Norway.1252", "tr_TR.UTF-8", "und-u-ks-level2"] {
            create_ok(DatabaseDef { lc_collate: Some(ok.into()), lc_ctype: Some(ok.into()), ..d() }, 170000);
        }
    }

    #[test]
    fn drop_variants() {
        let mut out = String::new();
        // FORCE: PG 12 hata, PG 13 tamam.
        let e = drop_err(d(), true, 120000);
        assert_eq!(paths(e.clone()), ["force"]);
        assert!(e[0].message.contains("PG 13+"), "{e:?}");
        for (force, v, want, note) in [
            (false, 100000, "DROP DATABASE d", DROP_NOTE.to_string()),
            (false, 170000, "DROP DATABASE d", DROP_NOTE.to_string()),
            (true, 130000, "DROP DATABASE d WITH (FORCE)", format!("{DROP_NOTE}. {FORCE_NOTE}")),
        ] {
            let p = drop_ok(d(), force, v);
            assert_eq!(sqls(&p), [want]);
            // CASCADE olmadan da DataLoss (Karar 3).
            assert_eq!(p.statements[0].risk, Risk::DataLoss);
            assert_eq!(p.risk(), Risk::DataLoss);
            assert_eq!(p.statements[0].note.as_deref(), Some(note.as_str()));
            assert!(!p.statements[0].transactional);
            assert!(p.warnings.is_empty());
            out.push_str(&render(&p));
            out.push('\n');
        }
        // Şablon veritabanı silinemez; hatalar birlikte raporlanır.
        let tpl = DatabaseDef { is_template: true, ..d() };
        let e = drop_err(tpl.clone(), false, 170000);
        assert_eq!(paths(e.clone()), ["isTemplate"]);
        assert_eq!(e[0].message, IS_TEMPLATE_DROP_MSG);
        assert_eq!(paths(drop_err(tpl, true, 120000)), ["isTemplate", "force"]);
        // FORCE + CASCADE: ortak kural (CASCADE yok).
        let e = drop_with(&o(d()), DropOptions { cascade: true, force: true }, &ctx()).unwrap_err();
        assert_eq!(paths(e), ["cascade"]);
        // Yüklü alanlar ne olursa olsun silinebilir; eski ad gevşek (yalnızca boş/NUL reddedilir).
        let odd = DatabaseDef { name: "a\u{200b}\nb".into(), encoding: Some("x".into()), connection_limit: -9, ..d() };
        assert_eq!(sqls(&drop_ok(odd, false, 170000)), ["DROP DATABASE \"a\u{200b}\nb\""]);
        assert_eq!(paths(drop_err(DatabaseDef { name: String::new(), ..d() }, false, 170000)), ["name"]);
        assert_eq!(paths(drop_err(DatabaseDef { name: "a\0".into(), ..d() }, false, 170000)), ["name"]);
        insta::assert_snapshot!(out);
    }

    #[test]
    fn old_name_lenient_new_name_strict() {
        let zw = DatabaseDef { name: "d\u{200b}x".into(), ..loaded() };
        assert!(validate_identity(&o(zw.clone())).is_empty());
        let p = alter_ok(&zw, &DatabaseDef { name: "dx".into(), ..zw.clone() });
        assert_eq!(sqls(&p), ["ALTER DATABASE \"d\u{200b}x\" RENAME TO dx"]);
        assert_single_statements(&alter_ok(&zw, &DatabaseDef { comment: Some("k".into()), ..zw.clone() }));
        assert_eq!(paths(alter_err(&DatabaseDef { name: String::new(), ..loaded() }, &loaded())), ["old.name"]);
        for bad in ["d\u{200b}y", "a\nb", "a\u{2067}b", "a\u{85}b", ""] {
            assert_eq!(paths(alter_err(&loaded(), &DatabaseDef { name: bad.into(), ..loaded() })), ["name"], "{bad:?}");
            assert_eq!(create_err(DatabaseDef { name: bad.into(), ..d() }, 170000), ["name"], "{bad:?}");
        }
        assert_eq!(create_err(DatabaseDef { name: "a".repeat(64), ..d() }, 170000), ["name"]);
    }

    #[test]
    fn injection_matrix() {
        let hostile = "C'; DROP DATABASE x; --";
        let long63 = "a".repeat(63);
        // Adlar (ad, owner, template, tablespace): tek ifade kalır.
        for n in [hostile, "d\"; DROP DATABASE x; --", "select", "Veri ş", "日本", long63.as_str(), "'", "\\", "/*", "$$"] {
            let x = DatabaseDef {
                name: n.into(),
                owner: Some(n.into()),
                template: Some(n.into()),
                tablespace: Some(n.into()),
                lc_collate: Some(n.into()),
                lc_ctype: Some(n.into()),
                comment: Some(n.into()),
                settings: vec![s("app.x", n), s("search_path", &format!("\"{}\"", n.replace('"', "\"\"")))],
                ..d()
            };
            let renamed = DatabaseDef {
                name: format!("{}x", &n[..n.len().min(60)]),
                owner: Some(format!("{}o", &n[..n.len().min(60)])),
                tablespace: Some(format!("{}t", &n[..n.len().min(60)])),
                settings: vec![s("app.y", n)],
                comment: Some(format!("{n}!")),
                ..x.clone()
            };
            for p in [create_ok(x.clone(), 170000), alter_ok(&x, &renamed), drop_ok(x.clone(), true, 170000)] {
                assert!(!p.statements.is_empty(), "{n:?}");
                assert!(p.statements.iter().all(|s| !s.transactional));
            }
        }
        let p = create_ok(
            DatabaseDef { name: "d\"; DROP DATABASE x; --".into(), lc_collate: Some(hostile.into()), ..d() },
            170000,
        );
        assert_eq!(
            p.statements[0].sql,
            format!("CREATE DATABASE \"d\"\"; DROP DATABASE x; --\" LC_COLLATE = 'C''; DROP DATABASE x; --' {TAIL}")
        );
        // Alan hataları.
        let long64 = "a".repeat(64);
        let icu = |v: &str| DatabaseDef { locale_provider: Some(LocaleProvider::Icu), provider_locale: Some(v.into()), ..d() };
        let cases: Vec<(DatabaseDef, &str)> = vec![
            (DatabaseDef { name: long64.clone(), ..d() }, "name"),
            (DatabaseDef { name: "a\0".into(), ..d() }, "name"),
            (DatabaseDef { owner: Some("a\nb".into()), ..d() }, "owner"),
            (DatabaseDef { owner: Some(long64.clone()), ..d() }, "owner"),
            (DatabaseDef { template: Some("t\0".into()), ..d() }, "template"),
            (DatabaseDef { template: Some("a\u{202e}b".into()), ..d() }, "template"),
            (DatabaseDef { tablespace: Some("a\nb".into()), ..d() }, "tablespace"),
            (DatabaseDef { tablespace: Some(String::new()), ..d() }, "tablespace"),
            (DatabaseDef { lc_collate: Some("en_US\0".into()), ..d() }, "lcCollate"),
            (DatabaseDef { lc_ctype: Some("a\nb".into()), ..d() }, "lcCtype"),
            (icu("en_US\0"), "providerLocale"),
            (icu("a\nb"), "providerLocale"),
            (DatabaseDef { settings: vec![s("a; DROP DATABASE x", "1")], ..d() }, "settings[0].name"),
            (DatabaseDef { settings: vec![s("a\0", "1")], ..d() }, "settings[0].name"),
            (DatabaseDef { settings: vec![s("work_mem", "1\0")], ..d() }, "settings[0].value"),
            (DatabaseDef { settings: vec![s("work_mem", "a\nb")], ..d() }, "settings[0].value"),
            (DatabaseDef { settings: vec![s("search_path", "\"x")], ..d() }, "settings[0].value"),
            (DatabaseDef { comment: Some("a\0b".into()), ..d() }, "comment"),
        ];
        for (x, want) in cases {
            assert_eq!(create_err(x.clone(), 170000), [want], "{x:?}");
        }
        // Alter: değişen ad/owner/tablespace/ayar/yorum aynı kapıdan geçer.
        for (x, want) in [
            (DatabaseDef { name: "a\0".into(), ..loaded() }, "name"),
            (DatabaseDef { owner: Some("a\nb".into()), ..loaded() }, "owner"),
            (DatabaseDef { tablespace: Some(long64.clone()), ..loaded() }, "tablespace"),
            (DatabaseDef { settings: vec![s("a b", "1")], ..loaded() }, "settings[0].name"),
            (DatabaseDef { settings: vec![s("x", "a\nb")], ..loaded() }, "settings[0].value"),
            (DatabaseDef { comment: Some("\0".into()), ..loaded() }, "comment"),
        ] {
            assert_eq!(paths(alter_err(&loaded(), &x)), [want], "{x:?}");
        }
    }
}
