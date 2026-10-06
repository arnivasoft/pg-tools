//! Saf DDL üreticileri: tanım → `Plan` (veritabanı/IO yok). Tüm ad/değer yerleştirmeleri `quote` yardımcılarından geçer.
//!
//! Sözleşme: her `Statement.sql` **tek** ifadedir ve sonda `;` taşımaz (çalıştırıcı gerekirse ekler);
//! çıktı deterministiktir (aynı girdi → aynı metin ve sıra).

pub mod extension;
pub mod schema;
pub mod sequence;
mod sqlscan;

pub use sqlscan::outer_semicolon_count;

use crate::model::{FieldError, ObjectDef, ObjectKind};
use crate::plan::{Plan, Risk, Statement};

/// PG tanımlayıcı üst sınırı (BAYT, UTF-8).
pub const MAX_IDENT_BYTES: usize = 63;

/// Üreticilerin bağlamı.
#[derive(Debug, Clone, Default)]
pub struct BuildCtx {
    /// `server_version_num` (ör. 160002). Şu an sürüme özel kapı yok; indeks üreticisi (INCLUDE PG 11+) kullanacak.
    pub version_num: u32,
    /// `CREATE INDEX CONCURRENTLY` istenir mi (yalnızca indeks üreticisi kullanır).
    pub concurrently: bool,
}

pub(crate) fn err(path: &str, message: impl Into<String>) -> FieldError {
    FieldError { path: path.to_string(), message: message.into() }
}

/// Ad kuralları: boş değil, NUL yok, en çok 63 bayt.
pub(crate) fn check_ident(path: &str, value: &str, errs: &mut Vec<FieldError>) {
    if value.is_empty() {
        errs.push(err(path, "Ad boş olamaz."));
    } else if value.contains('\0') {
        errs.push(err(path, "Ad NUL karakteri içeremez."));
    } else if value.len() > MAX_IDENT_BYTES {
        errs.push(err(path, format!("Ad en çok {MAX_IDENT_BYTES} bayt olabilir (şu an {} bayt).", value.len())));
    }
}

/// Şema olarak kullanılacak ad: ad kuralları + sistem şemaları yasak (`pg_*`, `information_schema`).
pub(crate) fn check_schema_ident(path: &str, value: &str, errs: &mut Vec<FieldError>) {
    let before = errs.len();
    check_ident(path, value, errs);
    if errs.len() == before && is_system_schema(value) {
        errs.push(err(path, "Sistem şemaları (pg_*, information_schema) kullanılamaz."));
    }
}

pub(crate) fn is_system_schema(name: &str) -> bool {
    let l = name.to_ascii_lowercase();
    l.starts_with("pg_") || l == "information_schema"
}

/// Serbest metin değeri (yorum, sürüm): NUL yasak.
pub(crate) fn check_text(path: &str, value: &str, errs: &mut Vec<FieldError>) {
    if value.contains('\0') {
        errs.push(err(path, "Değer NUL karakteri içeremez."));
    }
}

/// Transaction içinde çalışabilir ifade.
pub(crate) fn stmt(sql: String, risk: Risk, note: Option<&str>) -> Statement {
    Statement { sql, risk, transactional: true, note: note.map(str::to_string) }
}

fn unsupported(kind: ObjectKind) -> Vec<FieldError> {
    vec![err("kind", format!("{kind:?} türü henüz desteklenmiyor."))]
}

fn mismatch(old: &ObjectDef, new: &ObjectDef) -> Vec<FieldError> {
    vec![err("kind", format!("Tür uyuşmazlığı: {:?} → {:?}.", old.kind(), new.kind()))]
}

/// Alan düzeyi doğrulama; boş liste = geçerli.
pub fn validate(def: &ObjectDef, _ctx: &BuildCtx) -> Vec<FieldError> {
    match def {
        ObjectDef::Schema(d) => schema::validate(d),
        ObjectDef::Sequence(d) => sequence::validate(d),
        ObjectDef::Extension(d) => extension::validate(d),
        // Task 4 indeks üreticisi bunu değiştirecek.
        ObjectDef::Index(_) => unsupported(ObjectKind::Index),
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
        ObjectDef::Index(_) => Err(unsupported(ObjectKind::Index)),
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
        _ => Err(unsupported(new.kind())),
    }
}

pub fn drop(def: &ObjectDef, cascade: bool, ctx: &BuildCtx) -> Result<Plan, Vec<FieldError>> {
    checked(def, ctx)?;
    match def {
        ObjectDef::Schema(d) => Ok(schema::drop(d, cascade)),
        ObjectDef::Sequence(d) => Ok(sequence::drop(d, cascade)),
        ObjectDef::Extension(d) => Ok(extension::drop(d, cascade)),
        ObjectDef::Index(_) => Err(unsupported(ObjectKind::Index)),
    }
}

/// Testler için okunabilir döküm: `[risk] sql  -- not`, ardından uyarılar.
#[cfg(test)]
pub(crate) fn render(plan: &Plan) -> String {
    let mut out = String::new();
    for s in &plan.statements {
        out.push_str(&format!("[{:?}{}] {}", s.risk, if s.transactional { "" } else { " non-tx" }, s.sql));
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
        assert_eq!(outer_semicolon_count(&s.sql), 0, "dış ';' var: {}", s.sql);
        assert!(!s.sql.trim_end().ends_with(';'), "sonda ';' var: {}", s.sql);
        assert!(!s.sql.contains('\0'));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::*;

    fn ctx() -> BuildCtx {
        BuildCtx { version_num: 160000, ..Default::default() }
    }

    #[test]
    fn default_ctx_is_not_concurrent() {
        assert!(!BuildCtx::default().concurrently);
    }

    #[test]
    fn index_is_not_supported_yet() {
        let d = ObjectDef::Index(IndexDef {
            schema: "public".into(),
            table: "t".into(),
            name: "i".into(),
            method: "btree".into(),
            unique: false,
            columns: vec![],
            include: vec![],
            where_clause: None,
            tablespace: None,
            fillfactor: None,
            comment: None,
            constraint: None,
        });
        for r in [validate(&d, &ctx()), create(&d, &ctx()).unwrap_err(), drop(&d, false, &ctx()).unwrap_err()] {
            assert_eq!(r.len(), 1);
            assert!(r[0].message.contains("henüz desteklenmiyor"));
        }
        assert!(alter(&d, &d, &ctx()).is_err());
    }

    #[test]
    fn alter_rejects_kind_mismatch() {
        let s = ObjectDef::Schema(SchemaDef { name: "a".into(), owner: None, comment: None });
        let e = ObjectDef::Extension(ExtensionDef { name: "a".into(), schema: None, version: None, cascade: false });
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
    fn system_schema_detection() {
        for s in ["pg_catalog", "pg_toast", "pg_temp_3", "information_schema", "PG_CATALOG", "Information_Schema"] {
            assert!(is_system_schema(s), "{s}");
        }
        for s in ["public", "pgx", "pg", "my_pg_schema", "information_schemas"] {
            assert!(!is_system_schema(s), "{s}");
        }
    }
}
