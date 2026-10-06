//! Saf DDL üreticileri: tanım → `Plan` (veritabanı/IO yok). Tüm ad/değer yerleştirmeleri `quote` yardımcılarından geçer.
//!
//! Sözleşme: her `Statement.sql` **tek** ifadedir ve sonda `;` taşımaz (çalıştırıcı gerekirse ekler);
//! çıktı deterministiktir (aynı girdi → aynı metin ve sıra).
//!
//! Serbest SQL alanları (Task 4): her alan `scan_semicolons` ile taranır ve `!is_clean()` ise (dış `;` ya da
//! kapanmamış tırnak/yorum) REDDEDİLİR; ayrıca alanlar birleştirildikten sonra **nihai ifade** de taranmalıdır
//! (alan sınırlarında kapanma/açılma kaymasını yakalamak için). Tarayıcı `standard_conforming_strings = on` varsayar.

pub mod extension;
pub mod index;
pub mod schema;
pub mod sequence;
mod sqlscan;

pub use sqlscan::{ScanResult, scan_semicolons};

use crate::model::{FieldError, ObjectDef};
use crate::plan::{Plan, Risk, Statement};

/// PG tanımlayıcı üst sınırı (BAYT, UTF-8).
pub const MAX_IDENT_BYTES: usize = 63;

/// Üreticilerin bağlamı.
#[derive(Debug, Clone, Default)]
pub struct BuildCtx {
    /// `server_version_num` (ör. 160002). İndeks üreticisi INCLUDE kapısı (PG 11+) için kullanır.
    pub version_num: u32,
    /// `CREATE INDEX CONCURRENTLY` istenir mi (indeks üreticisi: CREATE/DROP INDEX CONCURRENTLY).
    pub concurrently: bool,
}

pub(crate) fn err(path: &str, message: impl Into<String>) -> FieldError {
    FieldError {
        path: path.to_string(),
        message: message.into(),
    }
}

/// Ad kuralları: boş değil, NUL/ASCII denetim karakteri (satır sonu, sekme, 0x7f vb.) yok, en çok 63 bayt.
pub(crate) fn check_ident(path: &str, value: &str, errs: &mut Vec<FieldError>) {
    if value.is_empty() {
        errs.push(err(path, "Ad boş olamaz."));
    } else if value.contains('\0') {
        errs.push(err(path, "Ad NUL karakteri içeremez."));
    } else if value.chars().any(|c| c.is_ascii_control()) {
        // Satır sonu/sekme vb.: tırnaklı ad içinde ham kalır; `--` yorumunu bitirip sonrasını SQL yapabilir.
        errs.push(err(
            path,
            "Ad denetim karakteri (satır sonu, sekme vb.) içeremez.",
        ));
    } else if value.len() > MAX_IDENT_BYTES {
        errs.push(err(
            path,
            format!(
                "Ad en çok {MAX_IDENT_BYTES} bayt olabilir (şu an {} bayt).",
                value.len()
            ),
        ));
    }
}

/// Şema olarak kullanılacak ad: ad kuralları + sistem şemaları yasak (`pg_*`, `information_schema`).
pub(crate) fn check_schema_ident(path: &str, value: &str, errs: &mut Vec<FieldError>) {
    let before = errs.len();
    check_ident(path, value, errs);
    if errs.len() == before && is_system_schema(value) {
        errs.push(err(
            path,
            "Sistem şemaları (pg_*, information_schema) kullanılamaz.",
        ));
    }
}

pub(crate) fn is_system_schema(name: &str) -> bool {
    // PG'nin ayrılmış ad kuralı büyük/küçük harfe duyarlıdır ("PG_Data" geçerli kullanıcı şemasıdır).
    name.starts_with("pg_") || name == "information_schema"
}

/// Serbest metin değeri (yorum, sürüm): NUL yasak.
pub(crate) fn check_text(path: &str, value: &str, errs: &mut Vec<FieldError>) {
    if value.contains('\0') {
        errs.push(err(path, "Değer NUL karakteri içeremez."));
    }
}

/// Transaction içinde çalışabilir ifade.
pub(crate) fn stmt(sql: String, risk: Risk, note: Option<&str>) -> Statement {
    Statement {
        sql,
        risk,
        transactional: true,
        note: note.map(str::to_string),
    }
}

fn mismatch(old: &ObjectDef, new: &ObjectDef) -> Vec<FieldError> {
    vec![err(
        "kind",
        format!("Tür uyuşmazlığı: {:?} → {:?}.", old.kind(), new.kind()),
    )]
}

/// Alan düzeyi doğrulama; boş liste = geçerli.
pub fn validate(def: &ObjectDef, ctx: &BuildCtx) -> Vec<FieldError> {
    match def {
        ObjectDef::Schema(d) => schema::validate(d),
        ObjectDef::Sequence(d) => sequence::validate(d),
        ObjectDef::Extension(d) => extension::validate(d),
        ObjectDef::Index(d) => index::validate(d, ctx),
    }
}

fn checked(def: &ObjectDef, ctx: &BuildCtx) -> Result<(), Vec<FieldError>> {
    let errs = validate(def, ctx);
    if errs.is_empty() { Ok(()) } else { Err(errs) }
}

pub fn create(def: &ObjectDef, ctx: &BuildCtx) -> Result<Plan, Vec<FieldError>> {
    checked(def, ctx)?;
    match def {
        ObjectDef::Schema(d) => Ok(schema::create(d)),
        ObjectDef::Sequence(d) => Ok(sequence::create(d)),
        ObjectDef::Extension(d) => Ok(extension::create(d)),
        ObjectDef::Index(d) => Ok(index::create(d, ctx)),
    }
}

/// Aynı türde iki tanım arasındaki fark; ad değişimi `RENAME`. Fark yoksa boş plan.
/// Hem yeni hem eski tanım doğrulanır.
pub fn alter(old: &ObjectDef, new: &ObjectDef, ctx: &BuildCtx) -> Result<Plan, Vec<FieldError>> {
    if old.kind() != new.kind() {
        return Err(mismatch(old, new));
    }
    checked(new, ctx)?;
    // Eski tanımdaki ad/şema da SQL'e girer (RENAME/ALTER hedefi); o da doğrulanır.
    let mut old_errs = validate(old, ctx);
    if !old_errs.is_empty() {
        for e in &mut old_errs {
            e.path = format!("old.{}", e.path);
        }
        return Err(old_errs);
    }
    match (old, new) {
        (ObjectDef::Schema(o), ObjectDef::Schema(n)) => Ok(schema::alter(o, n)),
        (ObjectDef::Sequence(o), ObjectDef::Sequence(n)) => Ok(sequence::alter(o, n)),
        (ObjectDef::Extension(o), ObjectDef::Extension(n)) => extension::alter(o, n),
        (ObjectDef::Index(o), ObjectDef::Index(n)) => index::alter(o, n, ctx),
        _ => Err(mismatch(old, new)),
    }
}

pub fn drop(def: &ObjectDef, cascade: bool, ctx: &BuildCtx) -> Result<Plan, Vec<FieldError>> {
    checked(def, ctx)?;
    match def {
        ObjectDef::Schema(d) => Ok(schema::drop(d, cascade)),
        ObjectDef::Sequence(d) => Ok(sequence::drop(d, cascade)),
        ObjectDef::Extension(d) => Ok(extension::drop(d, cascade)),
        ObjectDef::Index(d) => index::drop(d, cascade),
    }
}

/// Testler için okunabilir döküm: `[risk] sql  -- not`, ardından uyarılar.
#[cfg(test)]
pub(crate) fn render(plan: &Plan) -> String {
    let mut out = String::new();
    for s in &plan.statements {
        out.push_str(&format!(
            "[{:?}{}] {}",
            s.risk,
            if s.transactional { "" } else { " non-tx" },
            s.sql
        ));
        if let Some(n) = &s.note {
            out.push_str(&format!("  -- {n}"));
        }
        out.push('\n');
    }
    for w in &plan.warnings {
        out.push_str(&format!("WARN: {w}\n"));
    }
    out
}

/// Her ifade tek ifade ve sonda `;` yok (enjeksiyon denetimi).
#[cfg(test)]
pub(crate) fn assert_single_statements(plan: &Plan) {
    for s in &plan.statements {
        assert!(
            scan_semicolons(&s.sql).is_clean(),
            "dış ';' ya da kapanmamış bölge var: {}",
            s.sql
        );
        assert!(!s.sql.trim_end().ends_with(';'), "sonda ';' var: {}", s.sql);
        assert!(!s.sql.contains('\0'));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::*;

    fn ctx() -> BuildCtx {
        BuildCtx {
            version_num: 160000,
            ..Default::default()
        }
    }

    #[test]
    fn default_ctx_is_not_concurrent() {
        assert!(!BuildCtx::default().concurrently);
    }

    #[test]
    fn alter_rejects_kind_mismatch() {
        let s = ObjectDef::Schema(SchemaDef {
            name: "a".into(),
            owner: None,
            comment: None,
        });
        let e = ObjectDef::Extension(ExtensionDef {
            name: "a".into(),
            schema: None,
            version: None,
            cascade: false,
        });
        let errs = alter(&s, &e, &ctx()).unwrap_err();
        assert_eq!(errs[0].path, "kind");
        assert!(errs[0].message.contains("uyuşmazlığı"));
    }

    fn errs_for(v: &str) -> usize {
        let mut e = vec![];
        check_ident("n", v, &mut e);
        e.len()
    }

    #[test]
    fn ident_byte_boundary() {
        assert_eq!(errs_for(&"a".repeat(63)), 0);
        assert_eq!(errs_for(&"a".repeat(64)), 1);
        // 31 × 'ş' (2 bayt) = 62 bayt; + 'a' = 63 tamam; 32 × 'ş' = 64 bayt red.
        assert_eq!(errs_for(&"ş".repeat(31)), 0);
        assert_eq!(errs_for(&format!("{}a", "ş".repeat(31))), 0);
        assert_eq!(errs_for(&"ş".repeat(32)), 1);
        // 21 × '€' (3 bayt) = 63 tamam; sonuna 'a' → 64 red.
        assert_eq!(errs_for(&"€".repeat(21)), 0);
        assert_eq!(errs_for(&format!("{}a", "€".repeat(21))), 1);
        // 63 karakter ama 126 bayt: bayt sınırı karakter sınırından önce çalışır.
        assert_eq!(errs_for(&"ş".repeat(63)), 1);
        // Sınırı çok baytlı karakter ortasında aşan ad (62 + 'ş' = 64 bayt).
        assert_eq!(errs_for(&format!("{}ş", "a".repeat(62))), 1);
        assert_eq!(errs_for(&format!("{}ş", "a".repeat(61))), 0);
    }

    #[test]
    fn ident_rejects_empty_and_nul() {
        assert_eq!(errs_for(""), 1);
        assert_eq!(errs_for("a\0b"), 1);
    }

    #[test]
    fn ident_rejects_control_characters() {
        for v in ["a\nb", "a\rb", "a\tb", "a\x7fb", "\n", "a\x1bb", "x\n)) --"] {
            assert_eq!(errs_for(v), 1, "{v:?}");
        }
        assert_eq!(errs_for("a b"), 0);
        // ASCII dışı denetim karakteri yasaklı değil (yalnız < 0x20 ve 0x7f).
        assert_eq!(errs_for("ş\u{85}"), 0);
    }

    #[test]
    fn system_schema_detection() {
        for s in ["pg_catalog", "pg_toast", "pg_temp_3", "information_schema"] {
            assert!(is_system_schema(s), "{s}");
        }
        for s in [
            "public",
            "pgx",
            "pg",
            "my_pg_schema",
            "information_schemas",
            "PG_Data",
            "Information_Schema",
        ] {
            assert!(!is_system_schema(s), "{s}");
        }
    }
}
