//! Schema üreticisi: CREATE/ALTER (rename, owner, comment)/DROP.

use super::{check_ident, check_schema_ident, check_text, stmt};
use crate::model::{FieldError, SchemaDef};
use crate::plan::{Plan, Risk};
use crate::quote::{quote_ident, quote_literal};

pub fn validate(d: &SchemaDef) -> Vec<FieldError> {
    let mut e = vec![];
    check_schema_ident("name", &d.name, &mut e);
    if let Some(o) = &d.owner {
        check_ident("owner", o, &mut e);
    }
    if let Some(c) = &d.comment {
        check_text("comment", c, &mut e);
    }
    e
}

fn comment_sql(name: &str, comment: &Option<String>) -> String {
    match comment {
        Some(c) => format!("COMMENT ON SCHEMA {} IS {}", quote_ident(name), quote_literal(c)),
        None => format!("COMMENT ON SCHEMA {} IS NULL", quote_ident(name)),
    }
}

pub fn create(d: &SchemaDef) -> Plan {
    let mut sql = format!("CREATE SCHEMA {}", quote_ident(&d.name));
    if let Some(o) = &d.owner {
        sql.push_str(&format!(" AUTHORIZATION {}", quote_ident(o)));
    }
    let mut statements = vec![stmt(sql, Risk::Safe, None)];
    if d.comment.is_some() {
        statements.push(stmt(comment_sql(&d.name, &d.comment), Risk::Safe, None));
    }
    Plan { statements, warnings: vec![] }
}

/// Sıra: rename, owner, comment (sonrakiler yeni adı kullanır). Sahip/yorum `None`'a çekilemez
/// (sahip kaldırılamaz → yok sayılır); yorum `None` ise `IS NULL`.
pub fn alter(old: &SchemaDef, new: &SchemaDef) -> Plan {
    let mut statements = vec![];
    if old.name != new.name {
        statements.push(stmt(
            format!("ALTER SCHEMA {} RENAME TO {}", quote_ident(&old.name), quote_ident(&new.name)),
            Risk::Safe,
            None,
        ));
    }
    if let Some(o) = &new.owner
        && new.owner != old.owner
    {
        statements.push(stmt(format!("ALTER SCHEMA {} OWNER TO {}", quote_ident(&new.name), quote_ident(o)), Risk::Safe, None));
    }
    if new.comment != old.comment {
        statements.push(stmt(comment_sql(&new.name, &new.comment), Risk::Safe, None));
    }
    Plan { statements, warnings: vec![] }
}

pub fn drop(d: &SchemaDef, cascade: bool) -> Plan {
    let q = quote_ident(&d.name);
    if cascade {
        Plan {
            statements: vec![stmt(format!("DROP SCHEMA {q} CASCADE"), Risk::DataLoss, Some("Şemadaki tüm nesneler silinir"))],
            warnings: vec!["Şemadaki tüm nesneler silinir".into()],
        }
    } else {
        Plan { statements: vec![stmt(format!("DROP SCHEMA {q}"), Risk::Destructive, None)], warnings: vec![] }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::build::{BuildCtx, assert_single_statements, create as bcreate, alter as balter, drop as bdrop, render};
    use crate::model::ObjectDef;

    fn s(name: &str) -> SchemaDef {
        SchemaDef { name: name.into(), owner: None, comment: None }
    }
    fn o(d: SchemaDef) -> ObjectDef {
        ObjectDef::Schema(d)
    }
    fn ctx() -> BuildCtx {
        BuildCtx { version_num: 160000, ..Default::default() }
    }

    #[test]
    fn create_variants() {
        let mut out = String::new();
        let full = SchemaDef { name: "App Data".into(), owner: Some("Bob".into()), comment: Some("it's a \\ test".into()) };
        for d in [s("sales"), SchemaDef { owner: Some("bob".into()), ..s("sales") }, full] {
            let p = bcreate(&o(d), &ctx()).unwrap();
            assert_single_statements(&p);
            out.push_str(&render(&p));
            out.push('\n');
        }
        insta::assert_snapshot!(out);
    }

    #[test]
    fn create_exact() {
        let p = bcreate(&o(SchemaDef { name: "a".into(), owner: Some("b".into()), comment: Some("c".into()) }), &ctx()).unwrap();
        let sql: Vec<_> = p.statements.iter().map(|s| s.sql.as_str()).collect();
        assert_eq!(sql, ["CREATE SCHEMA a AUTHORIZATION b", "COMMENT ON SCHEMA a IS 'c'"]);
        assert!(p.statements.iter().all(|s| s.risk == Risk::Safe && s.transactional));
    }

    #[test]
    fn alter_noop_is_empty() {
        let d = SchemaDef { name: "a".into(), owner: Some("b".into()), comment: Some("c".into()) };
        let p = balter(&o(d.clone()), &o(d), &ctx()).unwrap();
        assert!(p.statements.is_empty() && p.warnings.is_empty());
    }

    #[test]
    fn alter_combined_order_uses_new_name() {
        let old = SchemaDef { name: "old".into(), owner: Some("x".into()), comment: Some("a".into()) };
        let new = SchemaDef { name: "new name".into(), owner: Some("y".into()), comment: None };
        let p = balter(&o(old.clone()), &o(new.clone()), &ctx()).unwrap();
        assert_single_statements(&p);
        let sql: Vec<_> = p.statements.iter().map(|s| s.sql.as_str()).collect();
        assert_eq!(
            sql,
            ["ALTER SCHEMA \"old\" RENAME TO \"new name\"", "ALTER SCHEMA \"new name\" OWNER TO y", "COMMENT ON SCHEMA \"new name\" IS NULL"]
        );
        insta::assert_snapshot!(render(&p));
        // Yalnız ad / yalnız yorum
        let p = balter(&o(old.clone()), &o(SchemaDef { name: "n".into(), ..old.clone() }), &ctx()).unwrap();
        assert_eq!(p.statements.len(), 1);
        let p = balter(&o(old.clone()), &o(SchemaDef { comment: Some("z".into()), ..old }), &ctx()).unwrap();
        assert_eq!(p.statements[0].sql, "COMMENT ON SCHEMA \"old\" IS 'z'");
    }

    #[test]
    fn drop_variants_and_risk() {
        let p = bdrop(&o(s("sales")), false, &ctx()).unwrap();
        assert_eq!(p.statements[0].sql, "DROP SCHEMA sales");
        assert_eq!(p.statements[0].risk, Risk::Destructive);
        assert!(p.warnings.is_empty());
        let p = bdrop(&o(s("select")), true, &ctx()).unwrap();
        assert_eq!(p.statements[0].sql, "DROP SCHEMA \"select\" CASCADE");
        assert_eq!(p.statements[0].risk, Risk::DataLoss);
        assert_eq!(p.warnings, ["Şemadaki tüm nesneler silinir"]);
        insta::assert_snapshot!(render(&p));
    }

    #[test]
    fn validation_errors() {
        let long = "a".repeat(64);
        let cases: Vec<(SchemaDef, &str)> = vec![
            (s(""), "name"),
            (s(&long), "name"),
            (s("a\0b"), "name"),
            (s("pg_catalog"), "name"),
            (s("pg_custom"), "name"),
            (s("information_schema"), "name"),
            (SchemaDef { owner: Some(String::new()), ..s("a") }, "owner"),
            (SchemaDef { owner: Some("a\0".into()), ..s("a") }, "owner"),
            (SchemaDef { comment: Some("a\0".into()), ..s("a") }, "comment"),
        ];
        for (d, path) in cases {
            let errs = validate(&d);
            assert_eq!(errs.len(), 1, "{d:?}");
            assert_eq!(errs[0].path, path);
            assert!(bcreate(&o(d.clone()), &ctx()).is_err());
            if path == "name" {
                assert!(bdrop(&o(d.clone()), false, &ctx()).is_err());
            }
            assert!(balter(&o(s("ok")), &o(d), &ctx()).is_err());
        }
    }

    #[test]
    fn boundary_names() {
        assert!(validate(&s(&"a".repeat(63))).is_empty());
        assert!(validate(&s(&"ş".repeat(31))).is_empty());
        assert!(!validate(&s(&"ş".repeat(32))).is_empty());
        assert!(validate(&s("pgx")).is_empty());
    }

    #[test]
    fn injection_names_stay_single_statement() {
        let evil = ["x\"; DROP SCHEMA public; --", "'", "\\", "a\nb", "a;b", "/*", "--", "$$", "Ünï;cödé", "select", "a b"];
        for n in evil {
            let d = SchemaDef { name: n.into(), owner: Some(n.into()), comment: Some(n.into()) };
            let new = SchemaDef { name: format!("{n}2"), owner: Some(format!("{n}3")), comment: Some(format!("{n}4")) };
            for p in [
                bcreate(&o(d.clone()), &ctx()).unwrap(),
                balter(&o(d.clone()), &o(new), &ctx()).unwrap(),
                bdrop(&o(d.clone()), true, &ctx()).unwrap(),
                bdrop(&o(d.clone()), false, &ctx()).unwrap(),
            ] {
                assert!(!p.statements.is_empty());
                assert_single_statements(&p);
            }
        }
        let p = bcreate(&o(s("x\"; DROP SCHEMA public; --")), &ctx()).unwrap();
        assert_eq!(p.statements[0].sql, "CREATE SCHEMA \"x\"\"; DROP SCHEMA public; --\"");
    }

    #[test]
    fn unicode_and_keyword_names() {
        let p = bcreate(&o(s("şema")), &ctx()).unwrap();
        assert_eq!(p.statements[0].sql, "CREATE SCHEMA \"şema\"");
        let p = bcreate(&o(s("user")), &ctx()).unwrap();
        assert_eq!(p.statements[0].sql, "CREATE SCHEMA \"user\"");
    }
}
