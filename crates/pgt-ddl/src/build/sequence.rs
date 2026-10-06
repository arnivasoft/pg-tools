//! Sequence üreticisi: CREATE (tüm seçenekler açıkça yazılır), ALTER, DROP.
//!
//! Tanım açık değerler taşır; üretici varsayılan UYDURMAZ (artan/azalan varsayılan min/max/start sunucuya aittir,
//! arayüz yükleme sırasında gerçek değerleri doldurur). Doğrulama yalnızca tutarlılığı denetler.

use super::{check_ident, check_schema_ident, check_text, err, stmt};
use crate::model::{ColumnRef, FieldError, SeqType, SequenceDef};
use crate::plan::{Plan, Risk};
use crate::quote::{qualified, quote_ident, quote_literal};

const START_NOTE: &str = "Başlangıç değeri yalnızca bir sonraki RESTART için kaydedilir; mevcut sayaç değişmez";

fn type_sql(t: SeqType) -> &'static str {
    match t {
        SeqType::Smallint => "smallint",
        SeqType::Integer => "integer",
        SeqType::Bigint => "bigint",
    }
}

/// PG'nin tür başına kabul ettiği alt/üst sınırlar (smallint: −32768..=32767).
fn type_bounds(t: SeqType) -> (i64, i64) {
    match t {
        SeqType::Smallint => (i16::MIN as i64, i16::MAX as i64),
        SeqType::Integer => (i32::MIN as i64, i32::MAX as i64),
        SeqType::Bigint => (i64::MIN, i64::MAX),
    }
}

fn column_sql(c: &ColumnRef) -> String {
    format!("{}.{}", qualified(&c.schema, &c.table), quote_ident(&c.column))
}

fn owned_by_sql(o: &Option<ColumnRef>) -> String {
    match o {
        Some(c) => column_sql(c),
        None => "NONE".to_string(),
    }
}

pub fn validate(d: &SequenceDef) -> Vec<FieldError> {
    let mut e = vec![];
    check_schema_ident("schema", &d.schema, &mut e);
    check_ident("name", &d.name, &mut e);
    if let Some(o) = &d.owner {
        check_ident("owner", o, &mut e);
    }
    if let Some(c) = &d.comment {
        check_text("comment", c, &mut e);
    }
    if d.increment == 0 {
        e.push(err("increment", "Artış sıfır olamaz."));
    }
    if d.cache < 1 {
        e.push(err("cache", "Önbellek en az 1 olmalı."));
    }
    let (lo, hi) = type_bounds(d.data_type);
    let in_type = |v: i64| (lo..=hi).contains(&v);
    let ty = type_sql(d.data_type);
    if let Some(m) = d.min_value
        && !in_type(m)
    {
        e.push(err("minValue", format!("Alt sınır {ty} aralığında değil.")));
    }
    if let Some(m) = d.max_value
        && !in_type(m)
    {
        e.push(err("maxValue", format!("Üst sınır {ty} aralığında değil.")));
    }
    if let (Some(mn), Some(mx)) = (d.min_value, d.max_value)
        && mn >= mx
    {
        e.push(err("minValue", "Alt sınır üst sınırdan küçük olmalı."));
    }
    if !in_type(d.start) {
        e.push(err("start", format!("Başlangıç değeri {ty} aralığında değil.")));
    }
    if let Some(mn) = d.min_value
        && d.start < mn
    {
        e.push(err("start", "Başlangıç değeri alt sınırdan küçük olamaz."));
    }
    if let Some(mx) = d.max_value
        && d.start > mx
    {
        e.push(err("start", "Başlangıç değeri üst sınırdan büyük olamaz."));
    }
    if let Some(c) = &d.owned_by {
        check_schema_ident("ownedBy.schema", &c.schema, &mut e);
        check_ident("ownedBy.table", &c.table, &mut e);
        check_ident("ownedBy.column", &c.column, &mut e);
    }
    e
}

fn comment_sql(q: &str, comment: &Option<String>) -> String {
    match comment {
        Some(c) => format!("COMMENT ON SEQUENCE {q} IS {}", quote_literal(c)),
        None => format!("COMMENT ON SEQUENCE {q} IS NULL"),
    }
}

fn bound_sql(kw: &str, v: Option<i64>) -> String {
    match v {
        Some(n) => format!("{kw} {n}"),
        None => format!("NO {kw}"),
    }
}

fn cycle_sql(c: bool) -> &'static str {
    if c { "CYCLE" } else { "NO CYCLE" }
}

pub fn create(d: &SequenceDef) -> Plan {
    let q = qualified(&d.schema, &d.name);
    let sql = format!(
        "CREATE SEQUENCE {q} AS {} INCREMENT BY {} {} {} START WITH {} CACHE {} {} OWNED BY {}",
        type_sql(d.data_type),
        d.increment,
        bound_sql("MINVALUE", d.min_value),
        bound_sql("MAXVALUE", d.max_value),
        d.start,
        d.cache,
        cycle_sql(d.cycle),
        owned_by_sql(&d.owned_by),
    );
    let mut statements = vec![stmt(sql, Risk::Safe, None)];
    if let Some(o) = &d.owner {
        statements.push(stmt(format!("ALTER SEQUENCE {q} OWNER TO {}", quote_ident(o)), Risk::Safe, None));
    }
    if d.comment.is_some() {
        statements.push(stmt(comment_sql(&q, &d.comment), Risk::Safe, None));
    }
    Plan { statements, warnings: vec![] }
}

/// Sıra: parametre değişiklikleri (tek ifade, eski ad), rename, set schema, owner, owned by, comment.
/// Sonraki ifadeler yeni adı/şemayı kullanır. Sahip `None`'a çekilemez (yok sayılır).
pub fn alter(old: &SequenceDef, new: &SequenceDef) -> Plan {
    let mut statements = vec![];
    let mut warnings = vec![];

    let mut parts: Vec<String> = vec![];
    if old.data_type != new.data_type {
        parts.push(format!("AS {}", type_sql(new.data_type)));
    }
    if old.increment != new.increment {
        parts.push(format!("INCREMENT BY {}", new.increment));
    }
    if old.min_value != new.min_value {
        parts.push(bound_sql("MINVALUE", new.min_value));
    }
    if old.max_value != new.max_value {
        parts.push(bound_sql("MAXVALUE", new.max_value));
    }
    let start_changed = old.start != new.start;
    if start_changed {
        parts.push(format!("START WITH {}", new.start));
    }
    if old.cache != new.cache {
        parts.push(format!("CACHE {}", new.cache));
    }
    if old.cycle != new.cycle {
        parts.push(cycle_sql(new.cycle).to_string());
    }
    if !parts.is_empty() {
        let sql = format!("ALTER SEQUENCE {} {}", qualified(&old.schema, &old.name), parts.join(" "));
        statements.push(stmt(sql, Risk::Safe, start_changed.then_some(START_NOTE)));
        if start_changed {
            warnings.push(START_NOTE.to_string());
        }
    }

    if old.name != new.name {
        statements.push(stmt(
            format!("ALTER SEQUENCE {} RENAME TO {}", qualified(&old.schema, &old.name), quote_ident(&new.name)),
            Risk::Safe,
            None,
        ));
    }
    if old.schema != new.schema {
        statements.push(stmt(
            format!("ALTER SEQUENCE {} SET SCHEMA {}", qualified(&old.schema, &new.name), quote_ident(&new.schema)),
            Risk::Safe,
            None,
        ));
    }
    let q = qualified(&new.schema, &new.name);
    if let Some(o) = &new.owner
        && new.owner != old.owner
    {
        statements.push(stmt(format!("ALTER SEQUENCE {q} OWNER TO {}", quote_ident(o)), Risk::Safe, None));
    }
    if old.owned_by != new.owned_by {
        statements.push(stmt(format!("ALTER SEQUENCE {q} OWNED BY {}", owned_by_sql(&new.owned_by)), Risk::Safe, None));
    }
    if old.comment != new.comment {
        statements.push(stmt(comment_sql(&q, &new.comment), Risk::Safe, None));
    }
    Plan { statements, warnings }
}

pub fn drop(d: &SequenceDef, cascade: bool) -> Plan {
    let q = qualified(&d.schema, &d.name);
    if cascade {
        let w = "Diziye bağlı nesneler (kolon varsayılanları gibi) da silinir";
        Plan { statements: vec![stmt(format!("DROP SEQUENCE {q} CASCADE"), Risk::DataLoss, Some(w))], warnings: vec![w.into()] }
    } else {
        Plan { statements: vec![stmt(format!("DROP SEQUENCE {q}"), Risk::Destructive, None)], warnings: vec![] }
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
    fn o(d: SequenceDef) -> ObjectDef {
        ObjectDef::Sequence(d)
    }
    fn base() -> SequenceDef {
        SequenceDef {
            schema: "public".into(),
            name: "s".into(),
            owner: None,
            data_type: SeqType::Bigint,
            increment: 1,
            min_value: Some(1),
            max_value: Some(i64::MAX),
            start: 1,
            cache: 1,
            cycle: false,
            owned_by: None,
            comment: None,
        }
    }
    fn col(s: &str, t: &str, c: &str) -> ColumnRef {
        ColumnRef { schema: s.into(), table: t.into(), column: c.into() }
    }
    fn sqls(p: &Plan) -> Vec<&str> {
        p.statements.iter().map(|s| s.sql.as_str()).collect()
    }

    #[test]
    fn create_exact_with_all_options_explicit() {
        let p = bcreate(&o(base()), &ctx()).unwrap();
        assert_eq!(
            sqls(&p),
            ["CREATE SEQUENCE public.s AS bigint INCREMENT BY 1 MINVALUE 1 MAXVALUE 9223372036854775807 START WITH 1 CACHE 1 NO CYCLE OWNED BY NONE"]
        );
        assert_eq!(p.risk(), Risk::Safe);
    }

    #[test]
    fn create_variants_snapshot() {
        let mut out = String::new();
        let cases = [
            base(),
            SequenceDef { min_value: None, max_value: None, ..base() },
            SequenceDef {
                data_type: SeqType::Smallint,
                increment: -2,
                min_value: Some(-32768),
                max_value: Some(-1),
                start: -1,
                cache: 20,
                cycle: true,
                ..base()
            },
            SequenceDef {
                schema: "My Schema".into(),
                name: "order".into(),
                owner: Some("Owner X".into()),
                data_type: SeqType::Integer,
                max_value: Some(i32::MAX as i64),
                owned_by: Some(col("My Schema", "t;x", "Id")),
                comment: Some("it's \\ fine".into()),
                ..base()
            },
        ];
        for d in cases {
            let p = bcreate(&o(d), &ctx()).unwrap();
            assert_single_statements(&p);
            out.push_str(&render(&p));
            out.push('\n');
        }
        insta::assert_snapshot!(out);
    }

    #[test]
    fn descending_carries_explicit_values_without_invented_defaults() {
        let d = SequenceDef { increment: -1, min_value: None, max_value: Some(-1), start: -1, ..base() };
        let p = bcreate(&o(d), &ctx()).unwrap();
        assert_eq!(
            p.statements[0].sql,
            "CREATE SEQUENCE public.s AS bigint INCREMENT BY -1 NO MINVALUE MAXVALUE -1 START WITH -1 CACHE 1 NO CYCLE OWNED BY NONE"
        );
    }

    #[test]
    fn alter_noop_is_empty() {
        let d = SequenceDef { owner: Some("a".into()), owned_by: Some(col("public", "t", "c")), comment: Some("x".into()), ..base() };
        let p = balter(&o(d.clone()), &o(d), &ctx()).unwrap();
        assert!(p.statements.is_empty() && p.warnings.is_empty());
    }

    #[test]
    fn alter_parameters_in_one_statement() {
        let new = SequenceDef {
            data_type: SeqType::Integer,
            increment: 5,
            min_value: None,
            max_value: Some(1000),
            cache: 10,
            cycle: true,
            ..base()
        };
        let p = balter(&o(base()), &o(new), &ctx()).unwrap();
        assert_eq!(
            sqls(&p),
            ["ALTER SEQUENCE public.s AS integer INCREMENT BY 5 NO MINVALUE MAXVALUE 1000 CACHE 10 CYCLE"]
        );
        assert!(p.warnings.is_empty() && p.statements[0].note.is_none());
        assert_eq!(p.risk(), Risk::Safe);
    }

    #[test]
    fn alter_start_adds_note_and_warning() {
        let p = balter(&o(base()), &o(SequenceDef { start: 100, ..base() }), &ctx()).unwrap();
        assert_eq!(sqls(&p), ["ALTER SEQUENCE public.s START WITH 100"]);
        assert!(p.statements[0].note.as_deref().unwrap().contains("sayaç değişmez"));
        assert_eq!(p.warnings.len(), 1);
        insta::assert_snapshot!(render(&p));
    }

    #[test]
    fn alter_combined_order_uses_new_name_and_schema() {
        let old = SequenceDef { owner: Some("a".into()), owned_by: Some(col("public", "t", "c")), comment: Some("x".into()), ..base() };
        let new = SequenceDef {
            schema: "other".into(),
            name: "s2".into(),
            owner: Some("b".into()),
            cache: 5,
            owned_by: None,
            comment: None,
            ..old.clone()
        };
        let p = balter(&o(old), &o(new), &ctx()).unwrap();
        assert_single_statements(&p);
        assert_eq!(
            sqls(&p),
            [
                "ALTER SEQUENCE public.s CACHE 5",
                "ALTER SEQUENCE public.s RENAME TO s2",
                "ALTER SEQUENCE public.s2 SET SCHEMA other",
                "ALTER SEQUENCE other.s2 OWNER TO b",
                "ALTER SEQUENCE other.s2 OWNED BY NONE",
                "COMMENT ON SEQUENCE other.s2 IS NULL",
            ]
        );
        insta::assert_snapshot!(render(&p));
    }

    #[test]
    fn alter_owned_by_to_column() {
        let p = balter(&o(base()), &o(SequenceDef { owned_by: Some(col("public", "t", "id")), ..base() }), &ctx()).unwrap();
        assert_eq!(sqls(&p), ["ALTER SEQUENCE public.s OWNED BY public.t.id"]);
    }

    #[test]
    fn drop_variants() {
        let p = bdrop(&o(base()), false, &ctx()).unwrap();
        assert_eq!((sqls(&p), p.statements[0].risk), (vec!["DROP SEQUENCE public.s"], Risk::Destructive));
        let p = bdrop(&o(base()), true, &ctx()).unwrap();
        assert_eq!((sqls(&p), p.statements[0].risk), (vec!["DROP SEQUENCE public.s CASCADE"], Risk::DataLoss));
        assert_eq!(p.warnings.len(), 1);
        insta::assert_snapshot!(render(&p));
    }

    #[test]
    fn validation_errors() {
        let cases: Vec<(SequenceDef, &str)> = vec![
            (SequenceDef { name: String::new(), ..base() }, "name"),
            (SequenceDef { name: "a".repeat(64), ..base() }, "name"),
            (SequenceDef { name: "a\0".into(), ..base() }, "name"),
            (SequenceDef { schema: "pg_catalog".into(), ..base() }, "schema"),
            (SequenceDef { schema: "information_schema".into(), ..base() }, "schema"),
            (SequenceDef { schema: String::new(), ..base() }, "schema"),
            (SequenceDef { increment: 0, ..base() }, "increment"),
            (SequenceDef { min_value: Some(10), max_value: Some(10), start: 10, ..base() }, "minValue"),
            (SequenceDef { min_value: Some(11), max_value: Some(10), start: 10, ..base() }, "minValue"),
            (SequenceDef { start: 0, ..base() }, "start"),
            (SequenceDef { max_value: Some(100), start: 101, ..base() }, "start"),
            (SequenceDef { cache: 0, ..base() }, "cache"),
            (SequenceDef { cache: -3, ..base() }, "cache"),
            (SequenceDef { owner: Some(String::new()), ..base() }, "owner"),
            (SequenceDef { comment: Some("\0".into()), ..base() }, "comment"),
            (SequenceDef { owned_by: Some(col("", "t", "c")), ..base() }, "ownedBy.schema"),
            (SequenceDef { owned_by: Some(col("public", "", "c")), ..base() }, "ownedBy.table"),
            (SequenceDef { owned_by: Some(col("public", "t", "")), ..base() }, "ownedBy.column"),
            (SequenceDef { owned_by: Some(col("pg_catalog", "t", "c")), ..base() }, "ownedBy.schema"),
            (SequenceDef { owned_by: Some(col("public", "t\0", "c")), ..base() }, "ownedBy.table"),
        ];
        for (d, path) in cases {
            let errs = validate(&d);
            assert!(!errs.is_empty(), "{d:?}");
            assert_eq!(errs[0].path, path, "{errs:?}");
            assert!(bcreate(&o(d.clone()), &ctx()).is_err());
            assert!(balter(&o(base()), &o(d), &ctx()).is_err());
        }
    }

    #[test]
    fn type_bounds_are_enforced() {
        let mk = |t, min, max, start| SequenceDef { data_type: t, min_value: min, max_value: max, start, ..base() };
        // Sınırdakiler geçerli.
        assert!(validate(&mk(SeqType::Smallint, Some(-32768), Some(32767), 1)).is_empty());
        assert!(validate(&mk(SeqType::Integer, Some(-2147483648), Some(2147483647), 1)).is_empty());
        assert!(validate(&mk(SeqType::Bigint, Some(i64::MIN), Some(i64::MAX), 1)).is_empty());
        // Aşanlar.
        assert_eq!(validate(&mk(SeqType::Smallint, Some(1), Some(32768), 1))[0].path, "maxValue");
        assert_eq!(validate(&mk(SeqType::Smallint, Some(-32769), Some(10), 1))[0].path, "minValue");
        assert_eq!(validate(&mk(SeqType::Integer, Some(1), Some(2147483648), 1))[0].path, "maxValue");
        assert_eq!(validate(&mk(SeqType::Smallint, None, None, 40000))[0].path, "start");
        assert_eq!(validate(&mk(SeqType::Integer, None, None, 3_000_000_000))[0].path, "start");
        assert_eq!(validate(&mk(SeqType::Integer, None, None, -3_000_000_000))[0].path, "start");
        // Sınır yok + start tür dışı değilse geçerli (üretici varsayılan uydurmaz).
        assert!(validate(&mk(SeqType::Smallint, None, None, 5)).is_empty());
    }

    #[test]
    fn multiple_errors_are_all_reported() {
        let d = SequenceDef { increment: 0, cache: 0, name: String::new(), ..base() };
        assert_eq!(validate(&d).len(), 3);
    }

    #[test]
    fn alter_validates_old_definition_too() {
        let bad_old = SequenceDef { name: "a\0".into(), ..base() };
        let errs = balter(&o(bad_old), &o(base()), &ctx()).unwrap_err();
        assert_eq!(errs[0].path, "old.name");
    }

    #[test]
    fn injection_names_stay_single_statement() {
        let evil = ["x\"; DROP SCHEMA public; --", "'", "\\", "a\nb", "a;b", "/*", "--", "$$", "Ünï;cödé", "select", "a b"];
        for n in evil {
            let d = SequenceDef {
                schema: n.into(),
                name: n.into(),
                owner: Some(n.into()),
                owned_by: Some(col(n, n, n)),
                comment: Some(n.into()),
                ..base()
            };
            let new = SequenceDef {
                schema: format!("{n}2"),
                name: format!("{n}3"),
                owner: Some(format!("{n}4")),
                owned_by: Some(col(&format!("{n}5"), n, n)),
                comment: Some(format!("{n}6")),
                start: 7,
                ..d.clone()
            };
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
        let d = SequenceDef { name: "x\"; DROP SCHEMA public; --".into(), ..base() };
        let p = bcreate(&o(d), &ctx()).unwrap();
        assert!(p.statements[0].sql.starts_with("CREATE SEQUENCE public.\"x\"\"; DROP SCHEMA public; --\" AS"));
    }

    #[test]
    fn unicode_and_keyword_names() {
        let d = SequenceDef { schema: "şema".into(), name: "user".into(), owned_by: Some(col("şema", "table", "ğ")), ..base() };
        let p = bcreate(&o(d), &ctx()).unwrap();
        assert!(p.statements[0].sql.starts_with("CREATE SEQUENCE \"şema\".\"user\" AS"));
        assert!(p.statements[0].sql.ends_with("OWNED BY \"şema\".\"table\".\"ğ\""));
    }

    #[test]
    fn boundary_63_bytes() {
        assert!(validate(&SequenceDef { name: "a".repeat(63), ..base() }).is_empty());
        assert!(!validate(&SequenceDef { name: "a".repeat(64), ..base() }).is_empty());
        assert!(!validate(&SequenceDef { name: format!("{}ş", "a".repeat(62)), ..base() }).is_empty());
    }
}
