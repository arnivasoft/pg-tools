//! Sequence üreticisi: CREATE (tüm seçenekler açıkça yazılır), ALTER, DROP.
//!
//! Tanım açık değerler taşır; üretici varsayılan UYDURMAZ (artan/azalan varsayılan min/max/start sunucuya aittir,
//! arayüz yükleme sırasında gerçek değerleri doldurur). Doğrulama yalnızca tutarlılığı denetler.

use super::{check_ident, check_schema_ident, check_text, err, stmt};
use crate::model::{ColumnRef, FieldError, SeqType, SequenceDef};
use crate::plan::{Plan, Risk};
use crate::quote::{qualified, quote_ident, quote_literal};

const OWNER_MATCH_WARNING: &str =
    "OWNED BY hedefi olan tablonun sahibi, dizinin sahibiyle aynı olmalı";
const START_NOTE: &str =
    "Başlangıç değeri yalnızca bir sonraki RESTART için kaydedilir; mevcut sayaç değişmez";

const RESTART_NOTE: &str =
    "Sayaç sıfırlanır: yeni başlangıç değerinden devam eder; tekrarlı değer üretilebilir";
const TIGHTEN_WARNING: &str =
    "Mevcut sayaç değeri yeni sınırların dışındaysa sunucu işlemi reddeder";

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

/// Örtük sınırlar uygulanmış etkin `(min, max)` (artan: 1..tür üst; azalan: tür alt..-1).
fn effective_bounds(d: &SequenceDef) -> (i64, i64) {
    let (lo, hi) = type_bounds(d.data_type);
    let (dmin, dmax) = if d.increment > 0 { (1, hi) } else { (lo, -1) };
    (d.min_value.unwrap_or(dmin), d.max_value.unwrap_or(dmax))
}

fn column_sql(c: &ColumnRef) -> String {
    format!(
        "{}.{}",
        qualified(&c.schema, &c.table),
        quote_ident(&c.column)
    )
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
        e.push(err(
            "start",
            format!("Başlangıç değeri {ty} aralığında değil."),
        ));
    } else if d.increment != 0 {
        // Belirtilmeyen sınırlar için PG'nin örtük değerleri: artan → min 1, max tür üst sınırı; azalan → min tür alt sınırı, max -1.
        let (imin, imax) = if d.increment > 0 { (1, hi) } else { (lo, -1) };
        let (mn, mx) = (d.min_value.unwrap_or(imin), d.max_value.unwrap_or(imax));
        if d.start < mn {
            e.push(err("start", "Başlangıç değeri alt sınırdan küçük olamaz."));
        } else if d.start > mx {
            e.push(err("start", "Başlangıç değeri üst sınırdan büyük olamaz."));
        }
    }
    if let Some(c) = &d.owned_by {
        check_schema_ident("ownedBy.schema", &c.schema, &mut e);
        check_ident("ownedBy.table", &c.table, &mut e);
        check_ident("ownedBy.column", &c.column, &mut e);
        if c.schema != d.schema {
            e.push(err(
                "ownedBy.schema",
                "OWNED BY tablosu dizi ile aynı şemada olmalı.",
            ));
        }
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
    // PG: sahip, bağlı tablonun sahibiyle aynı olmalı ve bağlı dizinin sahibi değiştirilemez.
    // Bu yüzden sahip belirtilmişse dizi bağsız oluşturulur, sahip atanır, OWNED BY en sonda bağlanır.
    let link_later = d.owner.is_some() && d.owned_by.is_some();
    let inline_owned_by = if link_later {
        "NONE".to_string()
    } else {
        owned_by_sql(&d.owned_by)
    };
    let sql = format!(
        "CREATE SEQUENCE {q} AS {} INCREMENT BY {} {} {} START WITH {} CACHE {} {} OWNED BY {inline_owned_by}",
        type_sql(d.data_type),
        d.increment,
        bound_sql("MINVALUE", d.min_value),
        bound_sql("MAXVALUE", d.max_value),
        d.start,
        d.cache,
        cycle_sql(d.cycle),
    );
    let mut statements = vec![stmt(sql, Risk::Safe, None)];
    let mut warnings = vec![];
    if let Some(o) = &d.owner {
        statements.push(stmt(
            format!("ALTER SEQUENCE {q} OWNER TO {}", quote_ident(o)),
            Risk::Safe,
            None,
        ));
    }
    if link_later {
        statements.push(stmt(
            format!("ALTER SEQUENCE {q} OWNED BY {}", owned_by_sql(&d.owned_by)),
            Risk::Safe,
            None,
        ));
        warnings.push(OWNER_MATCH_WARNING.to_string());
    }
    if d.comment.is_some() {
        statements.push(stmt(comment_sql(&q, &d.comment), Risk::Safe, None));
    }
    Plan {
        statements,
        warnings,
    }
}

/// Sıra: parametre değişiklikleri (tek ifade, eski ad), [OWNED BY çözme/değiştirme], rename, set schema, owner,
/// [OWNED BY yeniden bağlama], comment. PG kuralları: bağlı dizi başka şemaya taşınamaz ve sahibi değiştirilemez;
/// OWNED BY tablosu aynı şemada ve aynı sahipte olmalı. Bu yüzden taşıma/sahip değişiminde önce `OWNED BY NONE`,
/// sonra taşıma, en sonda yeni hedefe bağlama yapılır. Sonraki ifadeler yeni adı/şemayı kullanır.
/// Sahip `None`'a çekilemez (yok sayılır). `AS tür` değişiminde PG belirtilmeyen sınırları sıfırlar; bu yüzden
/// tür değişince ya da artışın işareti değişince MINVALUE/MAXVALUE yeni tanımdan her zaman açıkça yazılır.
pub fn alter(old: &SequenceDef, new: &SequenceDef) -> Plan {
    let mut statements = vec![];
    let mut warnings = vec![];
    let old_q = qualified(&old.schema, &old.name);

    let type_changed = old.data_type != new.data_type;
    // Artışın işareti değişince "varsayılan" (None) sınırlar yön değiştirir (artan: 1..max, azalan: min..-1) ama
    // PG ALTER'da belirtilmeyen sınırı eski değerinde bırakır; bu yüzden sınırlar her zaman yeniden yazılır.
    let sign_changed = old.increment.signum() != new.increment.signum();
    let rewrite_bounds = type_changed || sign_changed;
    let mut parts: Vec<String> = vec![];
    if type_changed {
        parts.push(format!("AS {}", type_sql(new.data_type)));
    }
    if old.increment != new.increment {
        parts.push(format!("INCREMENT BY {}", new.increment));
    }
    if rewrite_bounds || old.min_value != new.min_value {
        parts.push(bound_sql("MINVALUE", new.min_value));
    }
    if rewrite_bounds || old.max_value != new.max_value {
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
        // Artış işareti değişince PG, mevcut sayaç yeni sınırların dışında kaldığı için RESTART'sız ALTER'ı reddeder;
        // bu yüzden sayaç yeni başlangıçtan yeniden başlatılır (tekrarlı değer üretebilir → Destructive).
        if sign_changed {
            parts.push(format!("RESTART WITH {}", new.start));
        }
        let sql = format!("ALTER SEQUENCE {old_q} {}", parts.join(" "));
        let (risk, note) = if sign_changed {
            (Risk::Destructive, Some(RESTART_NOTE))
        } else {
            (Risk::Safe, start_changed.then_some(START_NOTE))
        };
        statements.push(stmt(sql, risk, note));
        if sign_changed {
            warnings.push(RESTART_NOTE.to_string());
        } else if start_changed {
            warnings.push(START_NOTE.to_string());
        }
        // Yön aynı kalıp sınırlar daralıyorsa saf üretici sayaç değerini bilemez: RESTART üretilmez, uyarılır.
        let (olo, ohi) = effective_bounds(old);
        let (nlo, nhi) = effective_bounds(new);
        if !sign_changed && (nlo > olo || nhi < ohi) {
            warnings.push(TIGHTEN_WARNING.to_string());
        }
    }

    let owner_changed = new.owner.is_some() && new.owner != old.owner;
    let moves = old.schema != new.schema || owner_changed;
    let link_changed = old.owned_by != new.owned_by;
    if moves {
        if old.owned_by.is_some() {
            statements.push(stmt(
                format!("ALTER SEQUENCE {old_q} OWNED BY NONE"),
                Risk::Safe,
                None,
            ));
        }
    } else if link_changed {
        statements.push(stmt(
            format!(
                "ALTER SEQUENCE {old_q} OWNED BY {}",
                owned_by_sql(&new.owned_by)
            ),
            Risk::Safe,
            None,
        ));
    }

    if old.name != new.name {
        statements.push(stmt(
            format!(
                "ALTER SEQUENCE {old_q} RENAME TO {}",
                quote_ident(&new.name)
            ),
            Risk::Safe,
            None,
        ));
    }
    if old.schema != new.schema {
        statements.push(stmt(
            format!(
                "ALTER SEQUENCE {} SET SCHEMA {}",
                qualified(&old.schema, &new.name),
                quote_ident(&new.schema)
            ),
            Risk::Safe,
            None,
        ));
    }
    let q = qualified(&new.schema, &new.name);
    if let Some(o) = &new.owner
        && owner_changed
    {
        statements.push(stmt(
            format!("ALTER SEQUENCE {q} OWNER TO {}", quote_ident(o)),
            Risk::Safe,
            None,
        ));
    }
    if moves && new.owned_by.is_some() {
        statements.push(stmt(
            format!(
                "ALTER SEQUENCE {q} OWNED BY {}",
                owned_by_sql(&new.owned_by)
            ),
            Risk::Safe,
            None,
        ));
        if owner_changed {
            warnings.push(OWNER_MATCH_WARNING.to_string());
        }
    }
    if old.comment != new.comment {
        statements.push(stmt(comment_sql(&q, &new.comment), Risk::Safe, None));
    }
    Plan {
        statements,
        warnings,
    }
}

pub fn drop(d: &SequenceDef, cascade: bool) -> Plan {
    let q = qualified(&d.schema, &d.name);
    if cascade {
        let w = "Diziye bağlı nesneler (kolon varsayılanları gibi) da silinir";
        Plan {
            statements: vec![stmt(
                format!("DROP SEQUENCE {q} CASCADE"),
                Risk::DataLoss,
                Some(w),
            )],
            warnings: vec![w.into()],
        }
    } else {
        Plan {
            statements: vec![stmt(format!("DROP SEQUENCE {q}"), Risk::Destructive, None)],
            warnings: vec![],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::build::{
        BuildCtx, alter as balter, assert_single_statements, create as bcreate, drop as bdrop,
        render,
    };
    use crate::model::ObjectDef;

    fn ctx() -> BuildCtx {
        BuildCtx {
            version_num: 160000,
            ..Default::default()
        }
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
        ColumnRef {
            schema: s.into(),
            table: t.into(),
            column: c.into(),
        }
    }
    fn sqls(p: &Plan) -> Vec<&str> {
        p.statements.iter().map(|s| s.sql.as_str()).collect()
    }

    #[test]
    fn create_exact_with_all_options_explicit() {
        let p = bcreate(&o(base()), &ctx()).unwrap();
        assert_eq!(
            sqls(&p),
            [
                "CREATE SEQUENCE public.s AS bigint INCREMENT BY 1 MINVALUE 1 MAXVALUE 9223372036854775807 START WITH 1 CACHE 1 NO CYCLE OWNED BY NONE"
            ]
        );
        assert_eq!(p.risk(), Risk::Safe);
    }

    #[test]
    fn create_variants_snapshot() {
        let mut out = String::new();
        let cases = [
            base(),
            SequenceDef {
                min_value: None,
                max_value: None,
                ..base()
            },
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
        let d = SequenceDef {
            increment: -1,
            min_value: None,
            max_value: Some(-1),
            start: -1,
            ..base()
        };
        let p = bcreate(&o(d), &ctx()).unwrap();
        assert_eq!(
            p.statements[0].sql,
            "CREATE SEQUENCE public.s AS bigint INCREMENT BY -1 NO MINVALUE MAXVALUE -1 START WITH -1 CACHE 1 NO CYCLE OWNED BY NONE"
        );
    }

    #[test]
    fn alter_noop_is_empty() {
        let d = SequenceDef {
            owner: Some("a".into()),
            owned_by: Some(col("public", "t", "c")),
            comment: Some("x".into()),
            ..base()
        };
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
            [
                "ALTER SEQUENCE public.s AS integer INCREMENT BY 5 NO MINVALUE MAXVALUE 1000 CACHE 10 CYCLE"
            ]
        );
        assert!(p.statements[0].note.is_none());
        assert_eq!(p.warnings, [TIGHTEN_WARNING]); // üst sınır 1000'e daralıyor
        assert_eq!(p.risk(), Risk::Safe);
    }

    /// Regresyon (Task 5 gidiş-dönüşü): azalan → artan geçişte `None` sınırlar yön değiştirir, ama PG ALTER'da
    /// belirtilmeyen sınırı eski değerinde bırakır; yeniden yazılmazsa MINVALUE eski (tür alt sınırı) kalırdı.
    #[test]
    fn alter_increment_sign_change_rewrites_default_bounds() {
        let desc = SequenceDef {
            increment: -1,
            min_value: None,
            max_value: None,
            start: -1,
            ..base()
        };
        let asc = SequenceDef {
            increment: 1,
            start: 1,
            ..desc.clone()
        };
        let p = balter(&o(desc.clone()), &o(asc.clone()), &ctx()).unwrap();
        assert_eq!(
            sqls(&p),
            ["ALTER SEQUENCE public.s INCREMENT BY 1 NO MINVALUE NO MAXVALUE START WITH 1 RESTART WITH 1"]
        );
        let p = balter(&o(asc), &o(desc), &ctx()).unwrap();
        assert_eq!(
            sqls(&p),
            ["ALTER SEQUENCE public.s INCREMENT BY -1 NO MINVALUE NO MAXVALUE START WITH -1 RESTART WITH -1"]
        );
        // İşaret aynı kalıyorsa sınırlar yeniden yazılmaz.
        let p = balter(
            &o(base()),
            &o(SequenceDef {
                increment: 5,
                ..base()
            }),
            &ctx(),
        )
        .unwrap();
        assert_eq!(sqls(&p), ["ALTER SEQUENCE public.s INCREMENT BY 5"]);
        assert!(p.warnings.is_empty());
    }

    #[test]
    fn sign_change_restarts_counter_and_is_destructive() {
        let new = SequenceDef {
            increment: -1,
            min_value: None,
            max_value: None,
            start: -1,
            ..base()
        };
        let p = balter(&o(base()), &o(new), &ctx()).unwrap();
        assert_eq!(p.statements.len(), 1);
        let s = &p.statements[0];
        assert!(s.sql.ends_with("START WITH -1 RESTART WITH -1"), "{}", s.sql);
        assert_eq!(s.risk, Risk::Destructive);
        assert_eq!(s.note.as_deref(), Some(RESTART_NOTE));
        assert_eq!(p.risk(), Risk::Destructive);
        assert!(p.warnings.contains(&RESTART_NOTE.to_string()));
        assert!(!p.warnings.contains(&TIGHTEN_WARNING.to_string()));
        assert_single_statements(&p);
    }

    #[test]
    fn tightening_bounds_without_sign_change_warns_but_never_restarts() {
        let cases = [
            SequenceDef { min_value: Some(10), start: 10, ..base() },
            SequenceDef { max_value: Some(1000), ..base() },
            SequenceDef { data_type: SeqType::Smallint, max_value: None, ..base() },
        ];
        for new in cases {
            let p = balter(&o(base()), &o(new), &ctx()).unwrap();
            assert!(!p.statements.iter().any(|s| s.sql.contains("RESTART")), "{:?}", sqls(&p));
            assert!(p.warnings.contains(&TIGHTEN_WARNING.to_string()), "{:?}", p.warnings);
            assert_eq!(p.risk(), Risk::Safe);
        }
        // Genişleyen sınırlarda uyarı yok.
        let p = balter(&o(SequenceDef { max_value: Some(1000), ..base() }), &o(base()), &ctx()).unwrap();
        assert!(!p.warnings.contains(&TIGHTEN_WARNING.to_string()));
    }

    #[test]
    fn alter_start_adds_note_and_warning() {
        let p = balter(
            &o(base()),
            &o(SequenceDef {
                start: 100,
                ..base()
            }),
            &ctx(),
        )
        .unwrap();
        assert_eq!(sqls(&p), ["ALTER SEQUENCE public.s START WITH 100"]);
        assert!(
            p.statements[0]
                .note
                .as_deref()
                .unwrap()
                .contains("sayaç değişmez")
        );
        assert_eq!(p.warnings.len(), 1);
        insta::assert_snapshot!(render(&p));
    }

    #[test]
    fn alter_combined_order_uses_new_name_and_schema() {
        let old = SequenceDef {
            owner: Some("a".into()),
            owned_by: Some(col("public", "t", "c")),
            comment: Some("x".into()),
            ..base()
        };
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
                "ALTER SEQUENCE public.s OWNED BY NONE",
                "ALTER SEQUENCE public.s RENAME TO s2",
                "ALTER SEQUENCE public.s2 SET SCHEMA other",
                "ALTER SEQUENCE other.s2 OWNER TO b",
                "COMMENT ON SEQUENCE other.s2 IS NULL",
            ]
        );
        insta::assert_snapshot!(render(&p));
    }

    #[test]
    fn alter_owned_by_to_column() {
        let p = balter(
            &o(base()),
            &o(SequenceDef {
                owned_by: Some(col("public", "t", "id")),
                ..base()
            }),
            &ctx(),
        )
        .unwrap();
        assert_eq!(sqls(&p), ["ALTER SEQUENCE public.s OWNED BY public.t.id"]);
    }

    #[test]
    fn drop_variants() {
        let p = bdrop(&o(base()), false, &ctx()).unwrap();
        assert_eq!(
            (sqls(&p), p.statements[0].risk),
            (vec!["DROP SEQUENCE public.s"], Risk::Destructive)
        );
        let p = bdrop(&o(base()), true, &ctx()).unwrap();
        assert_eq!(
            (sqls(&p), p.statements[0].risk),
            (vec!["DROP SEQUENCE public.s CASCADE"], Risk::DataLoss)
        );
        assert_eq!(p.warnings.len(), 1);
        insta::assert_snapshot!(render(&p));
    }

    #[test]
    fn validation_errors() {
        let cases: Vec<(SequenceDef, &str)> = vec![
            (
                SequenceDef {
                    name: String::new(),
                    ..base()
                },
                "name",
            ),
            (
                SequenceDef {
                    name: "a".repeat(64),
                    ..base()
                },
                "name",
            ),
            (
                SequenceDef {
                    name: "a\0".into(),
                    ..base()
                },
                "name",
            ),
            (
                SequenceDef {
                    schema: "pg_catalog".into(),
                    ..base()
                },
                "schema",
            ),
            (
                SequenceDef {
                    schema: "information_schema".into(),
                    ..base()
                },
                "schema",
            ),
            (
                SequenceDef {
                    schema: String::new(),
                    ..base()
                },
                "schema",
            ),
            (
                SequenceDef {
                    increment: 0,
                    ..base()
                },
                "increment",
            ),
            (
                SequenceDef {
                    min_value: Some(10),
                    max_value: Some(10),
                    start: 10,
                    ..base()
                },
                "minValue",
            ),
            (
                SequenceDef {
                    min_value: Some(11),
                    max_value: Some(10),
                    start: 10,
                    ..base()
                },
                "minValue",
            ),
            (SequenceDef { start: 0, ..base() }, "start"),
            (
                SequenceDef {
                    max_value: Some(100),
                    start: 101,
                    ..base()
                },
                "start",
            ),
            (SequenceDef { cache: 0, ..base() }, "cache"),
            (
                SequenceDef {
                    cache: -3,
                    ..base()
                },
                "cache",
            ),
            (
                SequenceDef {
                    owner: Some(String::new()),
                    ..base()
                },
                "owner",
            ),
            (
                SequenceDef {
                    comment: Some("\0".into()),
                    ..base()
                },
                "comment",
            ),
            (
                SequenceDef {
                    owned_by: Some(col("", "t", "c")),
                    ..base()
                },
                "ownedBy.schema",
            ),
            (
                SequenceDef {
                    owned_by: Some(col("public", "", "c")),
                    ..base()
                },
                "ownedBy.table",
            ),
            (
                SequenceDef {
                    owned_by: Some(col("public", "t", "")),
                    ..base()
                },
                "ownedBy.column",
            ),
            (
                SequenceDef {
                    owned_by: Some(col("pg_catalog", "t", "c")),
                    ..base()
                },
                "ownedBy.schema",
            ),
            (
                SequenceDef {
                    owned_by: Some(col("public", "t\0", "c")),
                    ..base()
                },
                "ownedBy.table",
            ),
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
        let mk = |t, min, max, start| SequenceDef {
            data_type: t,
            min_value: min,
            max_value: max,
            start,
            ..base()
        };
        // Sınırdakiler geçerli.
        assert!(validate(&mk(SeqType::Smallint, Some(-32768), Some(32767), 1)).is_empty());
        assert!(
            validate(&mk(
                SeqType::Integer,
                Some(-2147483648),
                Some(2147483647),
                1
            ))
            .is_empty()
        );
        assert!(validate(&mk(SeqType::Bigint, Some(i64::MIN), Some(i64::MAX), 1)).is_empty());
        // Aşanlar.
        assert_eq!(
            validate(&mk(SeqType::Smallint, Some(1), Some(32768), 1))[0].path,
            "maxValue"
        );
        assert_eq!(
            validate(&mk(SeqType::Smallint, Some(-32769), Some(10), 1))[0].path,
            "minValue"
        );
        assert_eq!(
            validate(&mk(SeqType::Integer, Some(1), Some(2147483648), 1))[0].path,
            "maxValue"
        );
        assert_eq!(
            validate(&mk(SeqType::Smallint, None, None, 40000))[0].path,
            "start"
        );
        assert_eq!(
            validate(&mk(SeqType::Integer, None, None, 3_000_000_000))[0].path,
            "start"
        );
        assert_eq!(
            validate(&mk(SeqType::Integer, None, None, -3_000_000_000))[0].path,
            "start"
        );
        // Sınır yok + start tür dışı değilse geçerli (üretici varsayılan uydurmaz).
        assert!(validate(&mk(SeqType::Smallint, None, None, 5)).is_empty());
    }

    #[test]
    fn multiple_errors_are_all_reported() {
        let d = SequenceDef {
            increment: 0,
            cache: 0,
            name: String::new(),
            ..base()
        };
        assert_eq!(validate(&d).len(), 3);
    }

    #[test]
    fn alter_validates_old_definition_too() {
        let bad_old = SequenceDef {
            name: "a\0".into(),
            ..base()
        };
        let errs = balter(&o(bad_old), &o(base()), &ctx()).unwrap_err();
        assert_eq!(errs[0].path, "old.name");
    }

    #[test]
    fn injection_names_stay_single_statement() {
        let evil = [
            "x\"; DROP SCHEMA public; --",
            "'",
            "\\",
            "a\\nb",
            "a;b",
            "/*",
            "--",
            "$$",
            "Ünï;cödé",
            "select",
            "a b",
        ];
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
                owned_by: Some(col(&format!("{n}2"), n, n)),
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
        let d = SequenceDef {
            name: "x\"; DROP SCHEMA public; --".into(),
            ..base()
        };
        let p = bcreate(&o(d), &ctx()).unwrap();
        assert!(
            p.statements[0]
                .sql
                .starts_with("CREATE SEQUENCE public.\"x\"\"; DROP SCHEMA public; --\" AS")
        );
    }

    #[test]
    fn unicode_and_keyword_names() {
        let d = SequenceDef {
            schema: "şema".into(),
            name: "user".into(),
            owned_by: Some(col("şema", "table", "ğ")),
            ..base()
        };
        let p = bcreate(&o(d), &ctx()).unwrap();
        assert!(
            p.statements[0]
                .sql
                .starts_with("CREATE SEQUENCE \"şema\".\"user\" AS")
        );
        assert!(
            p.statements[0]
                .sql
                .ends_with("OWNED BY \"şema\".\"table\".\"ğ\"")
        );
    }

    #[test]
    fn boundary_63_bytes() {
        assert!(
            validate(&SequenceDef {
                name: "a".repeat(63),
                ..base()
            })
            .is_empty()
        );
        assert!(
            !validate(&SequenceDef {
                name: "a".repeat(64),
                ..base()
            })
            .is_empty()
        );
        assert!(
            !validate(&SequenceDef {
                name: format!("{}ş", "a".repeat(62)),
                ..base()
            })
            .is_empty()
        );
    }

    #[test]
    fn alter_move_of_owned_sequence_unlinks_then_relinks() {
        let old = SequenceDef {
            owned_by: Some(col("public", "t", "c")),
            ..base()
        };
        let new = SequenceDef {
            schema: "other".into(),
            owned_by: Some(col("other", "t", "c")),
            ..base()
        };
        let p = balter(&o(old.clone()), &o(new), &ctx()).unwrap();
        assert_eq!(
            sqls(&p),
            [
                "ALTER SEQUENCE public.s OWNED BY NONE",
                "ALTER SEQUENCE public.s SET SCHEMA other",
                "ALTER SEQUENCE other.s OWNED BY other.t.c",
            ]
        );
        // Sahip değişimi de bağı önce çözer, sonra aynı hedefe yeniden bağlar + uyarı.
        let p = balter(
            &o(old.clone()),
            &o(SequenceDef {
                owner: Some("b".into()),
                ..old.clone()
            }),
            &ctx(),
        )
        .unwrap();
        assert_eq!(
            sqls(&p),
            [
                "ALTER SEQUENCE public.s OWNED BY NONE",
                "ALTER SEQUENCE public.s OWNER TO b",
                "ALTER SEQUENCE public.s OWNED BY public.t.c"
            ]
        );
        assert_eq!(p.warnings.len(), 1);
        // Yalnız bağ değişimi: tek ifade, taşıma yok.
        let p = balter(
            &o(old.clone()),
            &o(SequenceDef {
                owned_by: None,
                ..old.clone()
            }),
            &ctx(),
        )
        .unwrap();
        assert_eq!(sqls(&p), ["ALTER SEQUENCE public.s OWNED BY NONE"]);
        // Bağsız dizi taşınır, yeni hedefe sonda bağlanır.
        let moved = SequenceDef {
            schema: "other".into(),
            owned_by: Some(col("other", "t", "c")),
            ..base()
        };
        let p = balter(&o(base()), &o(moved), &ctx()).unwrap();
        assert_eq!(
            sqls(&p),
            [
                "ALTER SEQUENCE public.s SET SCHEMA other",
                "ALTER SEQUENCE other.s OWNED BY other.t.c"
            ]
        );
        insta::assert_snapshot!(render(&p));
    }

    #[test]
    fn create_with_owner_and_owned_by_links_last() {
        let d = SequenceDef {
            owner: Some("b".into()),
            owned_by: Some(col("public", "t", "c")),
            ..base()
        };
        let p = bcreate(&o(d), &ctx()).unwrap();
        assert_single_statements(&p);
        assert!(p.statements[0].sql.ends_with("OWNED BY NONE"));
        assert_eq!(p.statements[1].sql, "ALTER SEQUENCE public.s OWNER TO b");
        assert_eq!(
            p.statements[2].sql,
            "ALTER SEQUENCE public.s OWNED BY public.t.c"
        );
        assert_eq!(p.warnings.len(), 1);
        insta::assert_snapshot!(render(&p));
        // Sahipsiz: bağ CREATE içinde kalır.
        let d = SequenceDef {
            owned_by: Some(col("public", "t", "c")),
            ..base()
        };
        let p = bcreate(&o(d), &ctx()).unwrap();
        assert_eq!(p.statements.len(), 1);
        assert!(p.statements[0].sql.ends_with("OWNED BY public.t.c"));
    }

    #[test]
    fn owned_by_must_be_in_same_schema() {
        let d = SequenceDef {
            owned_by: Some(col("other", "t", "c")),
            ..base()
        };
        let errs = validate(&d);
        assert_eq!(errs.len(), 1);
        assert_eq!(errs[0].path, "ownedBy.schema");
    }

    #[test]
    fn alter_type_change_always_writes_explicit_bounds() {
        let old = SequenceDef {
            data_type: SeqType::Integer,
            max_value: Some(2147483647),
            ..base()
        };
        let p = balter(&o(old), &o(base()), &ctx()).unwrap();
        assert_eq!(
            sqls(&p),
            ["ALTER SEQUENCE public.s AS bigint MINVALUE 1 MAXVALUE 9223372036854775807"]
        );
        let old = base();
        let new = SequenceDef {
            data_type: SeqType::Smallint,
            max_value: Some(32767),
            ..base()
        };
        let p = balter(&o(old.clone()), &o(new), &ctx()).unwrap();
        assert_eq!(
            sqls(&p),
            ["ALTER SEQUENCE public.s AS smallint MINVALUE 1 MAXVALUE 32767"]
        );
        // Sınırsız (None) olarak da açıkça yazılır.
        let new = SequenceDef {
            data_type: SeqType::Integer,
            min_value: None,
            max_value: None,
            ..base()
        };
        let p = balter(&o(old), &o(new), &ctx()).unwrap();
        assert_eq!(
            sqls(&p),
            ["ALTER SEQUENCE public.s AS integer NO MINVALUE NO MAXVALUE"]
        );
    }

    #[test]
    fn start_is_checked_against_implicit_bounds() {
        let mk = |inc, min, max, start| SequenceDef {
            increment: inc,
            min_value: min,
            max_value: max,
            start,
            ..base()
        };
        // Artan: örtük min 1.
        assert_eq!(validate(&mk(1, None, None, 0))[0].path, "start");
        assert!(validate(&mk(1, None, None, 1)).is_empty());
        // Azalan: örtük max -1; örtük min tür alt sınırı.
        assert_eq!(validate(&mk(-1, None, None, 1))[0].path, "start");
        assert!(validate(&mk(-1, None, None, -1)).is_empty());
        assert!(validate(&mk(-1, None, None, i64::MIN)).is_empty());
        // Açık sınır örtüğü geçersiz kılar.
        assert!(validate(&mk(1, Some(-10), None, -5)).is_empty());
        assert!(validate(&mk(-1, None, Some(10), 5)).is_empty());
    }
}
