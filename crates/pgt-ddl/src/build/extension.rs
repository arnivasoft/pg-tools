//! Extension üreticisi: CREATE, ALTER (UPDATE TO, SET SCHEMA), DROP. Yeniden adlandırma PG'de yoktur → doğrulama hatası.

use super::{check_ident, check_schema_ident, check_text, err, stmt};
use crate::model::{ExtensionDef, FieldError};
use crate::plan::{Plan, Risk};
use crate::quote::{quote_ident, quote_literal};

pub fn validate(d: &ExtensionDef) -> Vec<FieldError> {
    let mut e = vec![];
    check_ident("name", &d.name, &mut e);
    if let Some(s) = &d.schema {
        check_schema_ident("schema", s, &mut e);
    }
    if let Some(v) = &d.version {
        if v.is_empty() {
            e.push(err("version", "Sürüm boş olamaz."));
        }
        check_text("version", v, &mut e);
    }
    e
}

pub fn create(d: &ExtensionDef) -> Plan {
    let mut sql = format!("CREATE EXTENSION {}", quote_ident(&d.name));
    if let Some(s) = &d.schema {
        sql.push_str(&format!(" SCHEMA {}", quote_ident(s)));
    }
    if let Some(v) = &d.version {
        sql.push_str(&format!(" VERSION {}", quote_literal(v)));
    }
    if d.cascade {
        sql.push_str(" CASCADE");
    }
    Plan { statements: vec![stmt(sql, Risk::Safe, None)], warnings: vec![] }
}

/// Sıra: sürüm güncelleme, şema taşıma. `version`/`schema` `None` ise "belirtilmedi" demektir ve yok sayılır
/// (geri alınamaz/sunucuya bırakılır); `cascade` yalnızca oluşturma seçeneğidir, farkı yok sayılır.
pub fn alter(old: &ExtensionDef, new: &ExtensionDef) -> Result<Plan, Vec<FieldError>> {
    if old.name != new.name {
        return Err(vec![err("name", "Eklenti yeniden adlandırılamaz.")]);
    }
    let q = quote_ident(&new.name);
    let mut statements = vec![];
    if let Some(v) = &new.version
        && new.version != old.version
    {
        statements.push(stmt(
            format!("ALTER EXTENSION {q} UPDATE TO {}", quote_literal(v)),
            Risk::Safe,
            Some("Eklentinin güncelleme betiği çalışır"),
        ));
    }
    if let Some(s) = &new.schema
        && new.schema != old.schema
    {
        statements.push(stmt(format!("ALTER EXTENSION {q} SET SCHEMA {}", quote_ident(s)), Risk::Safe, None));
    }
    Ok(Plan { statements, warnings: vec![] })
}

pub fn drop(d: &ExtensionDef, cascade: bool) -> Plan {
    let q = quote_ident(&d.name);
    if cascade {
        let w = "Eklentiye bağlı nesneler (tipler, işlevler, bağımlı kolonlar) da silinir";
        Plan { statements: vec![stmt(format!("DROP EXTENSION {q} CASCADE"), Risk::DataLoss, Some(w))], warnings: vec![w.into()] }
    } else {
        Plan { statements: vec![stmt(format!("DROP EXTENSION {q}"), Risk::Destructive, None)], warnings: vec![] }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::build::{BuildCtx, alter as balter, assert_single_statements, create as bcreate, drop as bdrop, render};
    use crate::model::ObjectDef;

    fn ctx() -> BuildCtx {
        BuildCtx { version_num: 160000, ..Default::default() }
    }
    fn o(d: ExtensionDef) -> ObjectDef {
        ObjectDef::Extension(d)
    }
    fn ext(name: &str) -> ExtensionDef {
        ExtensionDef { name: name.into(), schema: None, version: None, cascade: false }
    }
    fn sqls(p: &Plan) -> Vec<&str> {
        p.statements.iter().map(|s| s.sql.as_str()).collect()
    }

    #[test]
    fn create_variants() {
        let mut out = String::new();
        let cases = [
            ext("hstore"),
            ExtensionDef { schema: Some("ext".into()), ..ext("pg_trgm") },
            ExtensionDef { schema: Some("My Ext".into()), version: Some("1.0".into()), cascade: true, ..ext("uuid-ossp") },
            ExtensionDef { version: Some("it's\\1".into()), ..ext("user") },
        ];
        for d in cases {
            let p = bcreate(&o(d), &ctx()).unwrap();
            assert_single_statements(&p);
            assert_eq!(p.risk(), Risk::Safe);
            out.push_str(&render(&p));
        }
        insta::assert_snapshot!(out);
        let p = bcreate(&o(ExtensionDef { schema: Some("s".into()), version: Some("2".into()), cascade: true, ..ext("x") }), &ctx()).unwrap();
        assert_eq!(sqls(&p), ["CREATE EXTENSION x SCHEMA s VERSION '2' CASCADE"]);
    }

    #[test]
    fn alter_noop_is_empty() {
        let d = ExtensionDef { schema: Some("s".into()), version: Some("1".into()), ..ext("x") };
        let p = balter(&o(d.clone()), &o(d), &ctx()).unwrap();
        assert!(p.statements.is_empty());
        // cascade farkı sessizce yok sayılır.
        let p = balter(&o(ext("x")), &o(ExtensionDef { cascade: true, ..ext("x") }), &ctx()).unwrap();
        assert!(p.statements.is_empty());
    }

    #[test]
    fn alter_version_and_schema_in_order() {
        let old = ExtensionDef { schema: Some("a".into()), version: Some("1.0".into()), ..ext("pg_trgm") };
        let new = ExtensionDef { schema: Some("b b".into()), version: Some("1.6".into()), ..ext("pg_trgm") };
        let p = balter(&o(old.clone()), &o(new.clone()), &ctx()).unwrap();
        assert_single_statements(&p);
        assert_eq!(sqls(&p), ["ALTER EXTENSION pg_trgm UPDATE TO '1.6'", "ALTER EXTENSION pg_trgm SET SCHEMA \"b b\""]);
        assert_eq!(p.statements[0].note.as_deref(), Some("Eklentinin güncelleme betiği çalışır"));
        assert_eq!(p.risk(), Risk::Safe);
        insta::assert_snapshot!(render(&p));
        let p = balter(&o(old.clone()), &o(ExtensionDef { schema: Some("c".into()), ..old.clone() }), &ctx()).unwrap();
        assert_eq!(sqls(&p), ["ALTER EXTENSION pg_trgm SET SCHEMA c"]);
        // None = belirtilmedi → yok sayılır.
        let p = balter(&o(old), &o(ext("pg_trgm")), &ctx()).unwrap();
        assert!(p.statements.is_empty());
    }

    #[test]
    fn rename_is_rejected() {
        let errs = balter(&o(ext("a")), &o(ext("b")), &ctx()).unwrap_err();
        assert_eq!(errs[0].path, "name");
        assert!(errs[0].message.contains("yeniden adlandırılamaz"));
    }

    #[test]
    fn drop_variants() {
        let p = bdrop(&o(ext("hstore")), false, &ctx()).unwrap();
        assert_eq!((sqls(&p), p.statements[0].risk), (vec!["DROP EXTENSION hstore"], Risk::Destructive));
        let p = bdrop(&o(ext("uuid-ossp")), true, &ctx()).unwrap();
        assert_eq!((sqls(&p), p.statements[0].risk), (vec!["DROP EXTENSION \"uuid-ossp\" CASCADE"], Risk::DataLoss));
        assert_eq!(p.warnings.len(), 1);
        insta::assert_snapshot!(render(&p));
    }

    #[test]
    fn validation_errors() {
        let cases: Vec<(ExtensionDef, &str)> = vec![
            (ext(""), "name"),
            (ext(&"a".repeat(64)), "name"),
            (ext("a\0"), "name"),
            (ExtensionDef { schema: Some("pg_catalog".into()), ..ext("a") }, "schema"),
            (ExtensionDef { schema: Some("information_schema".into()), ..ext("a") }, "schema"),
            (ExtensionDef { schema: Some(String::new()), ..ext("a") }, "schema"),
            (ExtensionDef { version: Some(String::new()), ..ext("a") }, "version"),
            (ExtensionDef { version: Some("1\0".into()), ..ext("a") }, "version"),
        ];
        for (d, path) in cases {
            let errs = validate(&d);
            assert_eq!(errs.len(), 1, "{d:?}");
            assert_eq!(errs[0].path, path);
            assert!(bcreate(&o(d.clone()), &ctx()).is_err());
            assert!(bdrop(&o(d.clone()), false, &ctx()).is_err());
            assert!(balter(&o(ext("a")), &o(d), &ctx()).is_err());
        }
    }

    #[test]
    fn injection_names_stay_single_statement() {
        let evil = ["x\"; DROP SCHEMA public; --", "'", "\\", "a\\nb", "a;b", "/*", "--", "$$", "Ünï;cödé", "select", "a b"];
        for n in evil {
            let d = ExtensionDef { name: n.into(), schema: Some(n.into()), version: Some(n.into()), cascade: true };
            let new = ExtensionDef { schema: Some(format!("{n}2")), version: Some(format!("{n}3")), ..d.clone() };
            for p in [
                bcreate(&o(d.clone()), &ctx()).unwrap(),
                balter(&o(d.clone()), &o(new), &ctx()).unwrap(),
                bdrop(&o(d.clone()), true, &ctx()).unwrap(),
                bdrop(&o(d), false, &ctx()).unwrap(),
            ] {
                assert!(!p.statements.is_empty());
                assert_single_statements(&p);
            }
        }
        let p = bcreate(&o(ext("x\"; DROP SCHEMA public; --")), &ctx()).unwrap();
        assert_eq!(p.statements[0].sql, "CREATE EXTENSION \"x\"\"; DROP SCHEMA public; --\"");
    }

    #[test]
    fn boundary_63_bytes_and_unicode() {
        assert!(validate(&ext(&"a".repeat(63))).is_empty());
        assert!(!validate(&ext(&"a".repeat(64))).is_empty());
        assert!(!validate(&ext(&"ş".repeat(32))).is_empty());
        let p = bcreate(&o(ext("şeker")), &ctx()).unwrap();
        assert_eq!(p.statements[0].sql, "CREATE EXTENSION \"şeker\"");
    }
}
