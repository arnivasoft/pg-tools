//! İndeks üreticisi: CREATE (sürüm kapıları, CONCURRENTLY), ALTER (yerinde değişiklik ya da DROP+CREATE), DROP.
//!
//! Kararlar:
//! - `opclass`/`collation` TEK tanımlayıcıdır ve `quote_ident` ile tek parça tırnaklanır; şema nitelikli ad
//!   (`schema.opclass`) bu planın kapsamı dışındadır (nokta içeren değer tek bir ad olarak tırnaklanır).
//! - Serbest SQL alanları (`where_clause`, `Expression.sql`) parantez içinde olduğu gibi yerleştirilir; ama alan
//!   boş olamaz, `scan_semicolons` ile temiz olmalı (dış `;`/kapanmamış bölge yok), parantezleri dengeli olmalı
//!   (fazla `)` sarmalı kırar) ve birleşmiş NİHAİ ifade de aynı taramadan geçmelidir (ör. `--` ile biten alan
//!   kapanış parantezini yutar → nihai ifade dengesiz).
//! - `CONCURRENTLY` yalnızca `create()` ve `BuildCtx.concurrently` ile gelir. Alter kaynaklı DROP+CREATE ve açık `drop()` her
//!   zaman düz (transactional) DROP/CREATE kullanır; kesintisiz yeniden kurma ileri bir plana ertelendi.

use super::freesql::{check_free_sql, composed_is_clean, wrap};
use super::{BuildCtx, check_ident, check_schema_ident, check_text, err, stmt};
use crate::model::{FieldError, IndexColumn, IndexDef, IndexKey};
use crate::plan::{Plan, Risk, Statement};
use crate::quote::{qualified, quote_ident, quote_literal};

const METHODS: [&str; 6] = ["btree", "hash", "gist", "gin", "brin", "spgist"];
const REBUILD_WARNING: &str = "İndeks yeniden oluşturulur: önce silinir, sonra kurulur. DROP INDEX ACCESS EXCLUSIVE kilidi alır ve işlem bitene kadar tablo okuma ve yazmaya kapalı kalır; büyük tablolarda kesinti uzar";
const CONCURRENT_NOTE: &str =
    "Transaction dışında çalışır; hata olursa geçersiz (INVALID) indeks kalabilir";
const PARTITION_WARNING: &str = "CONCURRENTLY bölümlenmiş (partitioned) tablolarda desteklenmez; tablo bölümlenmişse ifade hata verir";
const TABLESPACE_NOTE: &str = "İndeks yeniden yazılır, kilit alır";
const FILLFACTOR_NOTE: &str = "Etkisi için REINDEX gerekir";

fn key_sql(c: &IndexColumn) -> String {
    let mut s = match &c.key {
        IndexKey::Column { name } => quote_ident(name),
        // Kapanış parantezi yeni satırda: alan `--` ile bitse bile `)` yorumda kalmaz.
        IndexKey::Expression { sql } => wrap(sql),
    };
    if let Some(cl) = &c.collation {
        s.push_str(&format!(" COLLATE {}", quote_ident(cl)));
    }
    if let Some(op) = &c.opclass {
        s.push_str(&format!(" {}", quote_ident(op)));
    }
    if c.descending {
        s.push_str(" DESC");
    }
    match c.nulls_first {
        Some(true) => s.push_str(" NULLS FIRST"),
        Some(false) => s.push_str(" NULLS LAST"),
        None => {}
    }
    s
}

/// CREATE INDEX ifadesi (ham, doğrulanmamış tanımdan da çalışır; doğrulama `validate` içinde nihai metni tarar).
fn create_sql(d: &IndexDef, concurrently: bool) -> String {
    let keys: Vec<String> = d.columns.iter().map(key_sql).collect();
    let mut sql = format!(
        "CREATE {}INDEX {}{} ON {} USING {} ({})",
        if d.unique { "UNIQUE " } else { "" },
        if concurrently { "CONCURRENTLY " } else { "" },
        quote_ident(&d.name),
        qualified(&d.schema, &d.table),
        d.method, // doğrulamada sabit kümeden biri olduğu denetlenir
        keys.join(", ")
    );
    if !d.include.is_empty() {
        let inc: Vec<String> = d.include.iter().map(|c| quote_ident(c)).collect();
        sql.push_str(&format!(" INCLUDE ({})", inc.join(", ")));
    }
    if let Some(ff) = d.fillfactor {
        sql.push_str(&format!(" WITH (fillfactor = {ff})"));
    }
    if let Some(ts) = &d.tablespace {
        sql.push_str(&format!(" TABLESPACE {}", quote_ident(ts)));
    }
    if let Some(w) = &d.where_clause {
        sql.push_str(&format!(" WHERE {}", wrap(w)));
    }
    sql
}

pub fn validate(d: &IndexDef, ctx: &BuildCtx) -> Vec<FieldError> {
    let mut e = vec![];
    check_schema_ident("schema", &d.schema, &mut e);
    check_ident("table", &d.table, &mut e);
    check_ident("name", &d.name, &mut e);
    if !METHODS.contains(&d.method.as_str()) {
        e.push(err(
            "method",
            "Yöntem btree, hash, gist, gin, brin ya da spgist olmalı.",
        ));
    }
    if d.columns.is_empty() {
        e.push(err("columns", "En az bir anahtar gerekli."));
    }
    for (i, c) in d.columns.iter().enumerate() {
        match &c.key {
            IndexKey::Column { name } => check_ident(&format!("columns[{i}].key"), name, &mut e),
            IndexKey::Expression { sql } => {
                check_free_sql(&format!("columns[{i}].key"), sql, &mut e)
            }
        }
        if let Some(o) = &c.opclass {
            check_ident(&format!("columns[{i}].opclass"), o, &mut e);
        }
        if let Some(o) = &c.collation {
            check_ident(&format!("columns[{i}].collation"), o, &mut e);
        }
    }
    for (i, c) in d.include.iter().enumerate() {
        check_ident(&format!("include[{i}]"), c, &mut e);
    }
    if !d.include.is_empty() && ctx.version_num < 110000 {
        e.push(err("include", "INCLUDE PG 11+ gerektirir."));
    }
    // PG: yalnızca btree benzersiz indeks destekler; hash ayrıca INCLUDE ve çok kolonu desteklemez.
    if d.unique && d.method != "btree" && METHODS.contains(&d.method.as_str()) {
        e.push(err(
            "unique",
            format!("{} yöntemi benzersiz indeks desteklemez.", d.method),
        ));
    }
    if d.method == "hash" {
        if !d.include.is_empty() {
            e.push(err("include", "hash yöntemi INCLUDE desteklemez."));
        }
        if d.columns.len() > 1 {
            e.push(err("columns", "hash yöntemi tek kolon destekler."));
        }
    }
    if matches!(d.method.as_str(), "gin" | "brin") && !d.include.is_empty() {
        e.push(err(
            "include",
            format!("{} yöntemi INCLUDE desteklemez.", d.method),
        ));
    }
    // PG: INCLUDE gist için 12+, spgist için 14+ (gin/brin/hash hiç desteklemez, yukarıda).
    if !d.include.is_empty()
        && ((d.method == "gist" && ctx.version_num < 120000)
            || (d.method == "spgist" && ctx.version_num < 140000))
    {
        e.push(err(
            "include",
            format!(
                "{} yöntemi için INCLUDE PG {}+ gerektirir.",
                d.method,
                if d.method == "gist" { 12 } else { 14 }
            ),
        ));
    }
    // ASC/DESC/NULLS FIRST|LAST yalnızca btree'de geçerlidir.
    if d.method != "btree" && METHODS.contains(&d.method.as_str()) {
        for (i, c) in d.columns.iter().enumerate() {
            if c.descending || c.nulls_first.is_some() {
                e.push(err(
                    &format!("columns[{i}]"),
                    format!("{} yöntemi DESC/NULLS FIRST/LAST desteklemez.", d.method),
                ));
            }
        }
    }
    if let Some(ts) = &d.tablespace {
        check_ident("tablespace", ts, &mut e);
    }
    if let Some(ff) = d.fillfactor
        && !(10..=100).contains(&ff)
    {
        e.push(err(
            "fillfactor",
            "Doldurma oranı 10 ile 100 arasında olmalı.",
        ));
    }
    // fillfactor: btree/hash/gist/spgist destekler; gin ve brin "unrecognized parameter" hatası verir.
    if d.fillfactor.is_some() && matches!(d.method.as_str(), "gin" | "brin") {
        e.push(err(
            "fillfactor",
            format!("{} yöntemi fillfactor desteklemez.", d.method),
        ));
    }
    if let Some(w) = &d.where_clause {
        check_free_sql("whereClause", w, &mut e);
    }
    if let Some(c) = &d.comment {
        check_text("comment", c, &mut e);
    }
    // Nihai ifade: alan sınırlarında bölge/parantez kaymasını yakalar (ifade sonunda `;` yoktur).
    if e.is_empty() && !composed_is_clean(&create_sql(d, ctx.concurrently)) {
        e.push(err(
            "columns",
            "Serbest SQL alanları birleşince geçersiz ifade oluşuyor (ör. satır yorumuyla biten ifade).",
        ));
    }
    e
}

fn comment_sql(q: &str, comment: &Option<String>) -> String {
    match comment {
        Some(c) => format!("COMMENT ON INDEX {q} IS {}", quote_literal(c)),
        None => format!("COMMENT ON INDEX {q} IS NULL"),
    }
}

fn create_statements(d: &IndexDef, concurrently: bool) -> Vec<Statement> {
    let sql = create_sql(d, concurrently);
    let mut v = vec![if concurrently {
        Statement {
            sql,
            risk: Risk::Safe,
            transactional: false,
            note: Some(CONCURRENT_NOTE.to_string()),
            redacted: false,
        }
    } else {
        stmt(sql, Risk::Safe, None)
    }];
    if d.comment.is_some() {
        v.push(stmt(
            comment_sql(&qualified(&d.schema, &d.name), &d.comment),
            Risk::Safe,
            None,
        ));
    }
    v
}

pub fn create(d: &IndexDef, ctx: &BuildCtx) -> Plan {
    Plan {
        statements: create_statements(d, ctx.concurrently),
        warnings: if ctx.concurrently {
            vec![PARTITION_WARNING.to_string()]
        } else {
            vec![]
        },
    }
}

fn constraint_error(c: &str) -> Vec<FieldError> {
    vec![err(
        "constraint",
        format!("Bu indeks '{c}' kısıtına aittir; kısıtı tablo düzenleyicisinden değiştirin"),
    )]
}

/// Yapısal (yerinde değiştirilemeyen) fark var mı.
fn structural_change(o: &IndexDef, n: &IndexDef) -> bool {
    o.method != n.method
        || o.unique != n.unique
        || o.columns != n.columns
        || o.include != n.include
        || o.where_clause != n.where_clause
        || o.table != n.table
        || o.schema != n.schema
}

/// Yapısal değişiklik → DROP + CREATE (yeni ad doğrudan CREATE'te; rename ayrıca yazılmaz).
/// Aksi halde yerinde: rename, tablespace, fillfactor, yorum. `constraint` alanı yalnızca yüklemede dolar ve
/// farka katılmaz; kısıt-bağlı indekste yapısal/rename isteği hatadır (tablespace/fillfactor/yorum serbest).
pub fn alter(old: &IndexDef, new: &IndexDef) -> Result<Plan, Vec<FieldError>> {
    let structural = structural_change(old, new);
    let renamed = old.name != new.name;
    if (structural || renamed)
        && let Some(c) = old.constraint.as_ref().or(new.constraint.as_ref())
    {
        return Err(constraint_error(c));
    }
    let old_q = qualified(&old.schema, &old.name);
    if structural {
        // Tek transaction'da DROP + CREATE (ya hep ya hiç). CONCURRENTLY yeniden kurma bilerek yok: DROP INDEX
        // CONCURRENTLY sonrası CREATE hatası indeksi hiç bırakmaz; kesintisiz yeniden kurma (geçici ad CONCURRENTLY
        // kur -> eskiyi sil -> yeniden adlandır) ileri bir plana ertelendi.
        let mut statements = vec![stmt(format!("DROP INDEX {old_q}"), Risk::Destructive, None)];
        statements.extend(create_statements(new, false));
        return Ok(Plan { statements, warnings: vec![REBUILD_WARNING.to_string()] });
    }

    if old.tablespace.is_some() && new.tablespace.is_none() {
        return Err(vec![err(
            "tablespace",
            "Tablespace kaldırılamaz; varsayılana dönmek için pg_default'u açıkça seçin.",
        )]);
    }
    let mut statements = vec![];
    let mut warnings = vec![];
    if renamed {
        statements.push(stmt(
            format!("ALTER INDEX {old_q} RENAME TO {}", quote_ident(&new.name)),
            Risk::Safe,
            None,
        ));
    }
    let q = qualified(&new.schema, &new.name);
    if old.tablespace != new.tablespace
        && let Some(ts) = &new.tablespace
    {
        statements.push(stmt(
            format!("ALTER INDEX {q} SET TABLESPACE {}", quote_ident(ts)),
            Risk::Safe,
            Some(TABLESPACE_NOTE),
        ));
    }
    if old.fillfactor != new.fillfactor {
        let sql = match new.fillfactor {
            Some(n) => format!("ALTER INDEX {q} SET (fillfactor = {n})"),
            None => format!("ALTER INDEX {q} RESET (fillfactor)"),
        };
        statements.push(stmt(sql, Risk::Safe, Some(FILLFACTOR_NOTE)));
        warnings.push(FILLFACTOR_NOTE.to_string());
    }
    if old.comment != new.comment {
        statements.push(stmt(comment_sql(&q, &new.comment), Risk::Safe, None));
    }
    Ok(Plan {
        statements,
        warnings,
    })
}

pub fn drop(d: &IndexDef, cascade: bool) -> Result<Plan, Vec<FieldError>> {
    if let Some(c) = &d.constraint {
        return Err(constraint_error(c));
    }
    let q = qualified(&d.schema, &d.name);
    Ok(if cascade {
        let w = "İndekse bağlı nesneler da silinir";
        Plan {
            statements: vec![stmt(
                format!("DROP INDEX {q} CASCADE"),
                Risk::DataLoss,
                Some(w),
            )],
            warnings: vec![w.into()],
        }
    } else {
        Plan {
            statements: vec![stmt(format!("DROP INDEX {q}"), Risk::Destructive, None)],
            warnings: vec![],
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::build::{
        alter as balter, assert_single_statements, create as bcreate, drop as bdrop, render,
        scan_semicolons, validate as bvalidate,
    };
    use crate::model::ObjectDef;

    fn ctx() -> BuildCtx {
        BuildCtx {
            version_num: 160000,
            ..Default::default()
        }
    }
    fn cctx() -> BuildCtx {
        BuildCtx {
            concurrently: true,
            ..ctx()
        }
    }
    fn o(d: IndexDef) -> ObjectDef {
        ObjectDef::Index(d)
    }
    fn col(n: &str) -> IndexColumn {
        IndexColumn {
            key: IndexKey::Column { name: n.into() },
            opclass: None,
            collation: None,
            descending: false,
            nulls_first: None,
        }
    }
    fn expr(s: &str) -> IndexColumn {
        IndexColumn {
            key: IndexKey::Expression { sql: s.into() },
            ..col("x")
        }
    }
    fn base() -> IndexDef {
        IndexDef {
            schema: "public".into(),
            table: "t".into(),
            name: "t_a_idx".into(),
            method: "btree".into(),
            unique: false,
            columns: vec![col("a")],
            include: vec![],
            where_clause: None,
            tablespace: None,
            fillfactor: None,
            comment: None,
            constraint: None,
            valid: true,
        }
    }
    fn sqls(p: &Plan) -> Vec<&str> {
        p.statements.iter().map(|s| s.sql.as_str()).collect()
    }
    fn first_err(d: &IndexDef) -> String {
        let e = bvalidate(&o(d.clone()), &ctx());
        assert!(!e.is_empty(), "hata bekleniyordu: {d:?}");
        assert!(bcreate(&o(d.clone()), &ctx()).is_err());
        e[0].path.clone()
    }

    #[test]
    fn create_exact_basic() {
        let p = bcreate(&o(base()), &ctx()).unwrap();
        assert_eq!(
            sqls(&p),
            ["CREATE INDEX t_a_idx ON public.t USING btree (a)"]
        );
        assert_eq!(p.risk(), Risk::Safe);
        assert!(p.statements[0].transactional && p.warnings.is_empty());
    }

    #[test]
    fn create_full_options_exact() {
        let d = IndexDef {
            unique: true,
            columns: vec![
                IndexColumn {
                    descending: true,
                    nulls_first: Some(false),
                    collation: Some("C".into()),
                    opclass: Some("text_pattern_ops".into()),
                    ..col("Name")
                },
                expr("lower(email)"),
            ],
            include: vec!["b".into(), "c".into()],
            where_clause: Some("deleted_at IS NULL".into()),
            tablespace: Some("fast".into()),
            fillfactor: Some(70),
            comment: Some("it's".into()),
            ..base()
        };
        let p = bcreate(&o(d), &ctx()).unwrap();
        assert_eq!(
            sqls(&p),
            [
                "CREATE UNIQUE INDEX t_a_idx ON public.t USING btree (\"Name\" COLLATE \"C\" text_pattern_ops DESC NULLS LAST, (lower(email)\n)) INCLUDE (b, c) WITH (fillfactor = 70) TABLESPACE fast WHERE (deleted_at IS NULL\n)",
                "COMMENT ON INDEX public.t_a_idx IS 'it''s'"
            ]
        );
    }

    #[test]
    fn create_variants_snapshot() {
        let mut out = String::new();
        let cases = [
            base(),
            IndexDef {
                method: "hash".into(),
                ..base()
            },
            IndexDef {
                method: "gin".into(),
                columns: vec![expr("to_tsvector('simple', body)")],
                ..base()
            },
            IndexDef {
                method: "brin".into(),
                ..base()
            },
            IndexDef {
                schema: "My Schema".into(),
                table: "order".into(),
                name: "ix;1".into(),
                method: "gist".into(),
                columns: vec![col("user")],
                include: vec!["Ünï".into()],
                tablespace: Some("Space X".into()),
                comment: Some("a\\b".into()),
                ..base()
            },
            IndexDef {
                method: "spgist".into(),
                fillfactor: Some(100),
                ..base()
            },
        ];
        for d in cases {
            let p = bcreate(&o(d), &ctx()).unwrap();
            assert_single_statements(&p);
            out.push_str(&render(&p));
            out.push('\n');
        }
        let p = bcreate(&o(base()), &cctx()).unwrap();
        out.push_str(&render(&p));
        insta::assert_snapshot!(out);
    }

    #[test]
    fn expression_key_is_parenthesised_and_where_too() {
        let d = IndexDef {
            columns: vec![expr("a + b"), expr("(c)")],
            where_clause: Some("x > 1 OR y < 2".into()),
            ..base()
        };
        let p = bcreate(&o(d), &ctx()).unwrap();
        assert_eq!(
            p.statements[0].sql,
            "CREATE INDEX t_a_idx ON public.t USING btree ((a + b\n), ((c)\n)) WHERE (x > 1 OR y < 2\n)"
        );
    }

    #[test]
    fn sort_options() {
        let mk = |desc, nf| IndexDef {
            columns: vec![IndexColumn {
                descending: desc,
                nulls_first: nf,
                ..col("a")
            }],
            ..base()
        };
        let sql = |d| bcreate(&o(d), &ctx()).unwrap().statements[0].sql.clone();
        assert!(sql(mk(false, None)).ends_with("(a)"));
        assert!(sql(mk(true, None)).ends_with("(a DESC)"));
        assert!(sql(mk(false, Some(true))).ends_with("(a NULLS FIRST)"));
        assert!(sql(mk(true, Some(false))).ends_with("(a DESC NULLS LAST)"));
    }

    #[test]
    fn include_version_gate() {
        let d = IndexDef {
            include: vec!["b".into()],
            ..base()
        };
        let old = BuildCtx {
            version_num: 100000,
            ..Default::default()
        };
        let new = BuildCtx {
            version_num: 110000,
            ..Default::default()
        };
        let e = bvalidate(&o(d.clone()), &old);
        assert_eq!(
            (e[0].path.as_str(), e[0].message.as_str()),
            ("include", "INCLUDE PG 11+ gerektirir.")
        );
        assert!(bcreate(&o(d.clone()), &old).is_err());
        assert!(bvalidate(&o(d.clone()), &new).is_empty());
        // INCLUDE yoksa eski sürümde sorun yok.
        assert!(bvalidate(&o(base()), &old).is_empty());
        // alter de aynı kapıdan geçer.
        assert!(balter(&o(base()), &o(d), &old).is_err());
    }

    #[test]
    fn method_rules() {
        for m in ["btree", "hash", "gist", "gin", "brin", "spgist"] {
            assert!(
                bvalidate(
                    &o(IndexDef {
                        method: m.into(),
                        ..base()
                    }),
                    &ctx()
                )
                .is_empty(),
                "{m}"
            );
        }
        for m in ["", "BTREE", "bloom", "btree; DROP", "btree "] {
            assert_eq!(
                first_err(&IndexDef {
                    method: m.into(),
                    ..base()
                }),
                "method",
                "{m:?}"
            );
        }
        let h = IndexDef {
            method: "hash".into(),
            ..base()
        };
        assert_eq!(
            first_err(&IndexDef {
                unique: true,
                ..h.clone()
            }),
            "unique"
        );
        assert_eq!(
            first_err(&IndexDef {
                include: vec!["b".into()],
                ..h.clone()
            }),
            "include"
        );
        assert_eq!(
            first_err(&IndexDef {
                columns: vec![col("a"), col("b")],
                ..h
            }),
            "columns"
        );
        assert_eq!(
            first_err(&IndexDef {
                method: "gin".into(),
                unique: true,
                ..base()
            }),
            "unique"
        );
        assert_eq!(
            first_err(&IndexDef {
                method: "gin".into(),
                include: vec!["b".into()],
                ..base()
            }),
            "include"
        );
        assert_eq!(
            first_err(&IndexDef {
                method: "brin".into(),
                include: vec!["b".into()],
                ..base()
            }),
            "include"
        );
        // btree çok kolon, unique, include birlikte geçerli.
        let ok = IndexDef {
            unique: true,
            columns: vec![col("a"), col("b")],
            include: vec!["c".into()],
            ..base()
        };
        assert!(bvalidate(&o(ok), &ctx()).is_empty());
    }

    #[test]
    fn fillfactor_bounds() {
        for ff in [10, 11, 99, 100] {
            assert!(
                bvalidate(
                    &o(IndexDef {
                        fillfactor: Some(ff),
                        ..base()
                    }),
                    &ctx()
                )
                .is_empty(),
                "{ff}"
            );
        }
        for ff in [0, 9, 101, u32::MAX] {
            assert_eq!(
                first_err(&IndexDef {
                    fillfactor: Some(ff),
                    ..base()
                }),
                "fillfactor",
                "{ff}"
            );
        }
    }

    #[test]
    fn no_columns_and_name_rules() {
        assert_eq!(
            first_err(&IndexDef {
                columns: vec![],
                ..base()
            }),
            "columns"
        );
        assert_eq!(
            first_err(&IndexDef {
                name: String::new(),
                ..base()
            }),
            "name"
        );
        assert_eq!(
            first_err(&IndexDef {
                table: String::new(),
                ..base()
            }),
            "table"
        );
        assert_eq!(
            first_err(&IndexDef {
                schema: String::new(),
                ..base()
            }),
            "schema"
        );
        assert_eq!(
            first_err(&IndexDef {
                schema: "pg_catalog".into(),
                ..base()
            }),
            "schema"
        );
        assert_eq!(
            first_err(&IndexDef {
                schema: "information_schema".into(),
                ..base()
            }),
            "schema"
        );
        assert!(
            bvalidate(
                &o(IndexDef {
                    schema: "PG_Data".into(),
                    ..base()
                }),
                &ctx()
            )
            .is_empty()
        );
        assert_eq!(
            first_err(&IndexDef {
                name: "a\0".into(),
                ..base()
            }),
            "name"
        );
        assert_eq!(
            first_err(&IndexDef {
                table: "t\0".into(),
                ..base()
            }),
            "table"
        );
        assert_eq!(
            first_err(&IndexDef {
                schema: "s\0".into(),
                ..base()
            }),
            "schema"
        );
        assert_eq!(
            first_err(&IndexDef {
                comment: Some("\0".into()),
                ..base()
            }),
            "comment"
        );
        assert_eq!(
            first_err(&IndexDef {
                tablespace: Some(String::new()),
                ..base()
            }),
            "tablespace"
        );
        assert_eq!(
            first_err(&IndexDef {
                tablespace: Some("a\0".into()),
                ..base()
            }),
            "tablespace"
        );
        assert_eq!(
            first_err(&IndexDef {
                include: vec![String::new()],
                ..base()
            }),
            "include[0]"
        );
        assert_eq!(
            first_err(&IndexDef {
                include: vec!["a\0".into()],
                ..base()
            }),
            "include[0]"
        );
        let bad_col = |c: IndexColumn| IndexDef {
            columns: vec![c],
            ..base()
        };
        assert_eq!(first_err(&bad_col(col(""))), "columns[0].key");
        assert_eq!(first_err(&bad_col(col("a\0"))), "columns[0].key");
        assert_eq!(
            first_err(&bad_col(IndexColumn {
                opclass: Some(String::new()),
                ..col("a")
            })),
            "columns[0].opclass"
        );
        assert_eq!(
            first_err(&bad_col(IndexColumn {
                opclass: Some("a\0".into()),
                ..col("a")
            })),
            "columns[0].opclass"
        );
        assert_eq!(
            first_err(&bad_col(IndexColumn {
                collation: Some(String::new()),
                ..col("a")
            })),
            "columns[0].collation"
        );
        assert_eq!(
            first_err(&bad_col(IndexColumn {
                collation: Some("a".repeat(64)),
                ..col("a")
            })),
            "columns[0].collation"
        );
        assert_eq!(
            first_err(&IndexDef {
                include: vec!["a".repeat(64)],
                ..base()
            }),
            "include[0]"
        );
    }

    #[test]
    fn boundary_63_bytes() {
        for field in 0..4 {
            let mk = |n: String| {
                let mut d = base();
                match field {
                    0 => d.name = n,
                    1 => d.table = n,
                    2 => d.schema = n,
                    _ => d.tablespace = Some(n),
                }
                d
            };
            assert!(bvalidate(&o(mk("a".repeat(63))), &ctx()).is_empty());
            assert!(!bvalidate(&o(mk("a".repeat(64))), &ctx()).is_empty());
            assert!(!bvalidate(&o(mk(format!("{}ş", "a".repeat(62)))), &ctx()).is_empty());
            assert!(bvalidate(&o(mk(format!("{}ş", "a".repeat(61)))), &ctx()).is_empty());
        }
    }

    #[test]
    fn free_sql_attacks_are_rejected() {
        let attacks = [
            "x); DROP TABLE t; --",
            "x; DROP TABLE t",
            "x'",
            "'; DROP TABLE t; --",
            "1) , (2",
            "1)",
            "(1",
            "$$",
            "$a$ x",
            "/*",
            "\"unterminated",
            "",
            "   \n\t ",
            "a\0b",
            "x) WHERE (y",
            "a -- yorum",
            "a --",
            "1$$'$$ ) ; DROP TABLE t; --'",
            "1'x'",
        ];
        for a in attacks {
            let w = IndexDef {
                where_clause: Some(a.into()),
                ..base()
            };
            let k = IndexDef {
                columns: vec![expr(a)],
                ..base()
            };
            let e = bvalidate(&o(w.clone()), &ctx());
            assert!(
                !e.is_empty() && e[0].path == "whereClause",
                "where {a:?}: {e:?}"
            );
            let e = bvalidate(&o(k.clone()), &ctx());
            assert!(
                !e.is_empty() && e[0].path == "columns[0].key",
                "key {a:?}: {e:?}"
            );
            assert!(bcreate(&o(w), &ctx()).is_err());
            assert!(bcreate(&o(k), &ctx()).is_err());
        }
    }

    #[test]
    fn free_sql_legit_uses_pass() {
        let oks = [
            "x > 1",
            "a = ';'",
            "a = 'it''s;'",
            "a = E'\\'; x'",
            "a = $$;)$$",
            "f(g(x), (y))",
            "a /* ; ) */ > 1",
            "a = \"we;ird\"",
            "a -- not closed by paren but newline\n> 1",
            "s = ')' AND (b OR c)",
        ];
        for a in oks {
            let d = IndexDef {
                columns: vec![expr(a)],
                where_clause: Some(a.into()),
                ..base()
            };
            let e = bvalidate(&o(d.clone()), &ctx());
            assert!(e.is_empty(), "{a:?}: {e:?}");
            assert_single_statements(&bcreate(&o(d), &ctx()).unwrap());
        }
    }

    #[test]
    fn composed_statement_is_scanned() {
        // `--` ile biten alan alan düzeyinde reddedilir; nihai ifade taraması ikinci savunma hattıdır.
        for d in [
            IndexDef {
                columns: vec![expr("a --")],
                ..base()
            },
            IndexDef {
                where_clause: Some("a -- yorum".into()),
                ..base()
            },
        ] {
            let e = bvalidate(&o(d), &ctx());
            assert_eq!(e.len(), 1, "{e:?}");
            assert!(e[0].message.contains("satır yorumu"), "{e:?}");
        }
        // Yeni satırla biten yorum geçerlidir ve `)` yeni satırda kalır.
        let d = IndexDef {
            where_clause: Some("a -- yorum\nb > 0".into()),
            ..base()
        };
        let p = bcreate(&o(d), &ctx()).unwrap();
        assert!(p.statements[0].sql.ends_with("WHERE (a -- yorum\nb > 0\n)"));
        assert!(scan_semicolons(&p.statements[0].sql).parens_balanced());
    }

    /// Gözden geçirme bulgusu: `--` ile biten serbest SQL + sonraki ad alanındaki satır sonu, kapanış parantezini
    /// yutup `WHERE (true) --` gibi sızdırıyordu. Her girdi reddedilmeli.
    #[test]
    fn comment_swallow_bypass_is_closed() {
        let e = |s: &str| IndexColumn {
            key: IndexKey::Expression { sql: s.into() },
            ..col("x")
        };
        let cases: Vec<IndexDef> = vec![
            IndexDef {
                columns: vec![e("a --")],
                include: vec!["x\n)) WHERE (true) --".into()],
                ..base()
            },
            IndexDef {
                columns: vec![e("a --")],
                tablespace: Some("x\n)) --".into()),
                ..base()
            },
            IndexDef {
                columns: vec![e("a --"), col("q\n)) WHERE (b > 0) --")],
                ..base()
            },
            IndexDef {
                columns: vec![e("a --"), col("q\r)) WHERE (b > 0) --")],
                ..base()
            },
            IndexDef {
                columns: vec![e("a --")],
                where_clause: Some("b".into()),
                tablespace: Some("x\n)) --".into()),
                ..base()
            },
            IndexDef {
                where_clause: Some("a --".into()),
                tablespace: Some("x\n) --".into()),
                ..base()
            },
            IndexDef {
                columns: vec![e("a --"), col("q\t)) /*")],
                ..base()
            },
            IndexDef {
                columns: vec![col("a")],
                include: vec!["x\r\n".into()],
                ..base()
            },
            IndexDef {
                columns: vec![IndexColumn {
                    opclass: Some("o\n--".into()),
                    ..col("a")
                }],
                ..base()
            },
            IndexDef {
                columns: vec![IndexColumn {
                    collation: Some("o\n--".into()),
                    ..col("a")
                }],
                ..base()
            },
            IndexDef {
                schema: "s\n".into(),
                ..base()
            },
            IndexDef {
                table: "t\n".into(),
                ..base()
            },
            IndexDef {
                name: "i\n".into(),
                ..base()
            },
        ];
        for d in cases {
            assert!(bcreate(&o(d.clone()), &ctx()).is_err(), "{d:?}");
            assert!(bcreate(&o(d.clone()), &cctx()).is_err(), "{d:?}");
            // alter kaynaklı yeniden oluşturma (yapısal fark) da aynı doğrulamadan geçer.
            let old = IndexDef {
                unique: true,
                ..base()
            };
            assert!(
                balter(&o(old.clone()), &o(d.clone()), &ctx()).is_err(),
                "{d:?}"
            );
            // Eski taraf yalnızca kimlikle doğrulanır: ifade/INCLUDE içeriği ne olursa olsun yeniden kurulabilir ve silinebilir.
            let identity_ok = crate::build::validate_identity(&o(d.clone())).is_empty();
            assert_eq!(balter(&o(d.clone()), &o(old), &ctx()).is_ok(), identity_ok, "{d:?}");
            assert_eq!(bdrop(&o(d.clone()), false, &ctx()).is_ok(), identity_ok, "{d:?}");
        }
        // Kabul edilen biçimlerde nihai ifade (tam metin taraması) temiz ve dengeli kalır.
        let d = IndexDef {
            columns: vec![e("a -- yorum\nb"), col("q")],
            where_clause: Some("c > 0 -- k\n".into()),
            tablespace: Some("ts".into()),
            ..base()
        };
        let p = bcreate(&o(d), &ctx()).unwrap();
        let r = scan_semicolons(&p.statements[0].sql);
        assert!(r.is_clean() && r.parens_balanced() && !r.ends_in_line_comment);
        assert_eq!(
            p.statements[0].sql,
            "CREATE INDEX t_a_idx ON public.t USING btree ((a -- yorum\nb\n), q) TABLESPACE ts WHERE (c > 0 -- k\n\n)"
        );
    }

    #[test]
    fn digit_adjacent_quotes_are_rejected() {
        for a in [
            "1$$'$$ ) ; DROP TABLE t; --'",
            "1'x'",
            "1e'x'",
            "x > 1$$a$$",
        ] {
            let w = IndexDef {
                where_clause: Some(a.into()),
                ..base()
            };
            let e = bvalidate(&o(w), &ctx());
            assert!(!e.is_empty() && e[0].path == "whereClause", "{a:?}: {e:?}");
            let k = IndexDef {
                columns: vec![expr(a)],
                ..base()
            };
            assert!(!bvalidate(&o(k), &ctx()).is_empty(), "{a:?}");
        }
        // Boşluklu/aralıklı biçimler geçerli.
        let w = IndexDef {
            where_clause: Some("a = 1 AND b = 'x1'".into()),
            ..base()
        };
        assert!(bvalidate(&o(w), &ctx()).is_empty());
    }

    #[test]
    fn fillfactor_only_for_supporting_methods() {
        for m in ["btree", "hash", "gist", "spgist"] {
            let d = IndexDef {
                method: m.into(),
                fillfactor: Some(50),
                ..base()
            };
            assert!(bvalidate(&o(d), &ctx()).is_empty(), "{m}");
        }
        for m in ["gin", "brin"] {
            assert_eq!(
                first_err(&IndexDef {
                    method: m.into(),
                    fillfactor: Some(50),
                    ..base()
                }),
                "fillfactor",
                "{m}"
            );
            assert!(
                bvalidate(
                    &o(IndexDef {
                        method: m.into(),
                        ..base()
                    }),
                    &ctx()
                )
                .is_empty()
            );
        }
    }

    #[test]
    fn tablespace_removal_is_an_error() {
        let old = IndexDef {
            tablespace: Some("x".into()),
            ..base()
        };
        let e = balter(&o(old.clone()), &o(base()), &ctx()).unwrap_err();
        assert_eq!(e[0].path, "tablespace");
        assert!(e[0].message.contains("pg_default"));
        // Açıkça pg_default seçmek serbest; yapısal yeniden oluşturmada kaldırma sorun değil.
        let p = balter(
            &o(old.clone()),
            &o(IndexDef {
                tablespace: Some("pg_default".into()),
                ..base()
            }),
            &ctx(),
        )
        .unwrap();
        assert_eq!(
            sqls(&p),
            ["ALTER INDEX public.t_a_idx SET TABLESPACE pg_default"]
        );
        let p = balter(
            &o(old),
            &o(IndexDef {
                unique: true,
                ..base()
            }),
            &ctx(),
        )
        .unwrap();
        assert_eq!(p.statements.len(), 2);
    }

    #[test]
    fn include_and_sort_gates_by_method() {
        let v = |n: u32| BuildCtx {
            version_num: n,
            ..Default::default()
        };
        let inc = |m: &str| IndexDef {
            method: m.into(),
            include: vec!["b".into()],
            ..base()
        };
        assert!(!bvalidate(&o(inc("gist")), &v(110000)).is_empty());
        assert!(bvalidate(&o(inc("gist")), &v(120000)).is_empty());
        assert!(!bvalidate(&o(inc("spgist")), &v(130000)).is_empty());
        assert!(bvalidate(&o(inc("spgist")), &v(140000)).is_empty());
        assert!(bvalidate(&o(inc("btree")), &v(110000)).is_empty());
        // ASC/DESC/NULLS yalnızca btree.
        for m in ["hash", "gin", "gist", "brin", "spgist"] {
            for c in [
                IndexColumn {
                    descending: true,
                    ..col("a")
                },
                IndexColumn {
                    nulls_first: Some(true),
                    ..col("a")
                },
                IndexColumn {
                    nulls_first: Some(false),
                    ..col("a")
                },
            ] {
                let d = IndexDef {
                    method: m.into(),
                    columns: vec![c],
                    ..base()
                };
                assert_eq!(first_err(&d), "columns[0]", "{m}");
            }
        }
        let d = IndexDef {
            columns: vec![IndexColumn {
                descending: true,
                nulls_first: Some(true),
                ..col("a")
            }],
            ..base()
        };
        assert!(bvalidate(&o(d), &ctx()).is_empty());
    }

    #[test]
    fn concurrently_changes_statement_and_flags() {
        let d = IndexDef {
            comment: Some("c".into()),
            ..base()
        };
        let p = bcreate(&o(d), &cctx()).unwrap();
        assert_eq!(
            p.statements[0].sql,
            "CREATE INDEX CONCURRENTLY t_a_idx ON public.t USING btree (a)"
        );
        assert!(!p.statements[0].transactional);
        assert_eq!(p.statements[0].note.as_deref(), Some(CONCURRENT_NOTE));
        assert!(p.statements[0].note.as_deref().unwrap().contains("INVALID"));
        assert!(p.statements[1].transactional);
        assert_eq!(p.warnings.len(), 1);
        assert!(p.warnings[0].contains("bölümlenmiş"));
        let u = bcreate(
            &o(IndexDef {
                unique: true,
                ..base()
            }),
            &cctx(),
        )
        .unwrap();
        assert!(
            u.statements[0]
                .sql
                .starts_with("CREATE UNIQUE INDEX CONCURRENTLY t_a_idx")
        );
        // Kapalıyken transactional, uyarısız.
        let p = bcreate(&o(base()), &ctx()).unwrap();
        assert!(p.statements[0].transactional && p.warnings.is_empty());
    }

    #[test]
    fn alter_noop_is_empty() {
        let d = IndexDef {
            comment: Some("x".into()),
            fillfactor: Some(80),
            tablespace: Some("t".into()),
            ..base()
        };
        let p = balter(&o(d.clone()), &o(d), &ctx()).unwrap();
        assert!(p.statements.is_empty() && p.warnings.is_empty());
        // Yalnız `constraint` alanı farkı no-op'tur (yükleme alanı).
        let p = balter(
            &o(base()),
            &o(IndexDef {
                constraint: Some("pk".into()),
                ..base()
            }),
            &ctx(),
        )
        .unwrap();
        assert!(p.statements.is_empty());
    }

    #[test]
    fn alter_rename_only() {
        let new = IndexDef {
            name: "yeni".into(),
            ..base()
        };
        let p = balter(&o(base()), &o(new), &ctx()).unwrap();
        assert_eq!(sqls(&p), ["ALTER INDEX public.t_a_idx RENAME TO yeni"]);
        assert_eq!(p.risk(), Risk::Safe);
        assert!(p.warnings.is_empty());
    }

    #[test]
    fn alter_tablespace_fillfactor_comment_in_place() {
        let p = balter(
            &o(base()),
            &o(IndexDef {
                tablespace: Some("fast".into()),
                ..base()
            }),
            &ctx(),
        )
        .unwrap();
        assert_eq!(sqls(&p), ["ALTER INDEX public.t_a_idx SET TABLESPACE fast"]);
        assert_eq!(p.statements[0].note.as_deref(), Some(TABLESPACE_NOTE));
        let p = balter(
            &o(base()),
            &o(IndexDef {
                fillfactor: Some(60),
                ..base()
            }),
            &ctx(),
        )
        .unwrap();
        assert_eq!(
            sqls(&p),
            ["ALTER INDEX public.t_a_idx SET (fillfactor = 60)"]
        );
        assert_eq!(
            p.statements[0].note.as_deref(),
            Some("Etkisi için REINDEX gerekir")
        );
        let ff = IndexDef {
            fillfactor: Some(60),
            ..base()
        };
        let p = balter(&o(ff.clone()), &o(base()), &ctx()).unwrap();
        assert_eq!(sqls(&p), ["ALTER INDEX public.t_a_idx RESET (fillfactor)"]);
        let p = balter(
            &o(ff.clone()),
            &o(IndexDef {
                fillfactor: Some(90),
                ..base()
            }),
            &ctx(),
        )
        .unwrap();
        assert_eq!(
            sqls(&p),
            ["ALTER INDEX public.t_a_idx SET (fillfactor = 90)"]
        );
        let p = balter(
            &o(base()),
            &o(IndexDef {
                comment: Some("c".into()),
                ..base()
            }),
            &ctx(),
        )
        .unwrap();
        assert_eq!(sqls(&p), ["COMMENT ON INDEX public.t_a_idx IS 'c'"]);
        let p = balter(
            &o(IndexDef {
                comment: Some("c".into()),
                ..base()
            }),
            &o(base()),
            &ctx(),
        )
        .unwrap();
        assert_eq!(sqls(&p), ["COMMENT ON INDEX public.t_a_idx IS NULL"]);
        // Tablespace kaldırma hatadır (bkz. tablespace_removal_is_an_error).
    }

    #[test]
    fn alter_in_place_combined_uses_new_name() {
        let old = IndexDef {
            comment: Some("a".into()),
            ..base()
        };
        let new = IndexDef {
            name: "n2".into(),
            tablespace: Some("ts".into()),
            fillfactor: Some(75),
            comment: Some("b".into()),
            ..base()
        };
        let p = balter(&o(old), &o(new), &ctx()).unwrap();
        assert_single_statements(&p);
        assert_eq!(
            sqls(&p),
            [
                "ALTER INDEX public.t_a_idx RENAME TO n2",
                "ALTER INDEX public.n2 SET TABLESPACE ts",
                "ALTER INDEX public.n2 SET (fillfactor = 75)",
                "COMMENT ON INDEX public.n2 IS 'b'",
            ]
        );
        insta::assert_snapshot!(render(&p));
    }

    #[test]
    fn alter_structural_rebuilds_with_warning() {
        let structural: Vec<IndexDef> = vec![
            IndexDef {
                method: "hash".into(),
                ..base()
            },
            IndexDef {
                unique: true,
                ..base()
            },
            IndexDef {
                columns: vec![col("b")],
                ..base()
            },
            IndexDef {
                include: vec!["c".into()],
                ..base()
            },
            IndexDef {
                where_clause: Some("a > 0".into()),
                ..base()
            },
            IndexDef {
                table: "t2".into(),
                ..base()
            },
            IndexDef {
                schema: "other".into(),
                ..base()
            },
        ];
        for new in structural {
            let p = balter(&o(base()), &o(new.clone()), &ctx()).unwrap();
            assert_single_statements(&p);
            assert_eq!(p.statements[0].sql, "DROP INDEX public.t_a_idx", "{new:?}");
            assert_eq!(p.statements[0].risk, Risk::Destructive);
            assert!(p.statements[1].sql.starts_with("CREATE "));
            assert_eq!(p.statements[1].risk, Risk::Safe);
            assert_eq!(p.risk(), Risk::Destructive);
            assert_eq!(p.warnings, [REBUILD_WARNING]);
            assert!(p.statements.iter().all(|s| s.transactional));
        }
        let p = balter(
            &o(base()),
            &o(IndexDef {
                unique: true,
                comment: Some("c".into()),
                fillfactor: Some(50),
                ..base()
            }),
            &ctx(),
        )
        .unwrap();
        assert_eq!(
            sqls(&p),
            [
                "DROP INDEX public.t_a_idx",
                "CREATE UNIQUE INDEX t_a_idx ON public.t USING btree (a) WITH (fillfactor = 50)",
                "COMMENT ON INDEX public.t_a_idx IS 'c'",
            ]
        );
        insta::assert_snapshot!(render(&p));
    }

    #[test]
    fn alter_rename_plus_structural_creates_under_new_name() {
        let new = IndexDef {
            name: "yeni".into(),
            method: "hash".into(),
            ..base()
        };
        let p = balter(&o(base()), &o(new), &ctx()).unwrap();
        assert_eq!(
            sqls(&p),
            [
                "DROP INDEX public.t_a_idx",
                "CREATE INDEX yeni ON public.t USING hash (a)"
            ]
        );
        assert!(p.statements.iter().all(|s| !s.sql.contains("RENAME")));
        insta::assert_snapshot!(render(&p));
    }

    #[test]
    fn alter_rebuild_is_never_concurrent() {
        // `concurrently` bağlamı alter'ı etkilemez: tek transaction'da DROP + CREATE, uyarı tek.
        let new = IndexDef { unique: true, ..base() };
        let p = balter(&o(base()), &o(new.clone()), &cctx()).unwrap();
        assert_eq!(sqls(&p), ["DROP INDEX public.t_a_idx", "CREATE UNIQUE INDEX t_a_idx ON public.t USING btree (a)"]);
        assert!(p.statements.iter().all(|s| s.transactional));
        assert_eq!(p.warnings, [REBUILD_WARNING]);
        assert_eq!(p, balter(&o(base()), &o(new), &ctx()).unwrap());
        // Uyarı kilit gerçeğini söyler.
        assert!(REBUILD_WARNING.contains("ACCESS EXCLUSIVE") && REBUILD_WARNING.contains("okuma ve yazmaya kapalı"));
        // Yerinde değişiklikler concurrently'den etkilenmez.
        let p = balter(&o(base()), &o(IndexDef { name: "z".into(), ..base() }), &cctx()).unwrap();
        assert!(p.statements.iter().all(|s| s.transactional) && p.warnings.is_empty());
    }

    #[test]
    fn drop_and_old_side_validate_identity_only() {
        // Yüklenmiş özel erişim yöntemi + muhafazakâr tarayıcıya takılan ifade: silinebilir, yeniden adlandırılabilir.
        let odd = IndexDef { method: "bloom".into(), columns: vec![expr("a -- x")], where_clause: Some("(".into()), ..base() };
        assert!(!bvalidate(&o(odd.clone()), &ctx()).is_empty());
        assert_eq!(sqls(&bdrop(&o(odd.clone()), false, &ctx()).unwrap()), ["DROP INDEX public.t_a_idx"]);
        let renamed = IndexDef { name: "z".into(), ..odd.clone() };
        let p = balter(&o(odd.clone()), &o(renamed), &ctx());
        assert!(p.is_err(), "yeni taraf tam doğrulanır (bozuk ifade/yöntem)");
        // Kimlik hâlâ doğrulanır: NUL / denetim karakteri / sistem şeması / boş ad.
        for bad in [
            IndexDef { name: "a\0".into(), ..odd.clone() },
            IndexDef { table: "a\nb".into(), ..odd.clone() },
            IndexDef { schema: "pg_catalog".into(), ..odd.clone() },
            IndexDef { name: String::new(), ..odd.clone() },
        ] {
            let e = bdrop(&o(bad.clone()), false, &ctx()).unwrap_err();
            assert!(!e.is_empty());
            let e = balter(&o(bad), &o(base()), &ctx()).unwrap_err();
            assert!(e[0].path.starts_with("old."), "{e:?}");
        }
        // Kısıt koruması kimlik doğrulamasından bağımsız sürer.
        let pk = IndexDef { constraint: Some("t_pkey".into()), ..odd };
        assert_eq!(bdrop(&o(pk), false, &ctx()).unwrap_err()[0].path, "constraint");
    }

    #[test]
    fn constraint_backed_index_is_protected() {
        let old = IndexDef {
            constraint: Some("t_pkey".into()),
            ..base()
        };
        for new in [
            IndexDef {
                name: "x".into(),
                ..old.clone()
            },
            IndexDef {
                unique: true,
                ..old.clone()
            },
            IndexDef {
                columns: vec![col("b")],
                ..old.clone()
            },
            IndexDef {
                method: "hash".into(),
                ..old.clone()
            },
        ] {
            let e = balter(&o(old.clone()), &o(new), &ctx()).unwrap_err();
            assert_eq!(e[0].path, "constraint");
            assert_eq!(
                e[0].message,
                "Bu indeks 't_pkey' kısıtına aittir; kısıtı tablo düzenleyicisinden değiştirin"
            );
        }
        // Kısıt yeni tanımda taşınsa da korunur.
        let e = balter(
            &o(base()),
            &o(IndexDef {
                name: "x".into(),
                constraint: Some("k".into()),
                ..base()
            }),
            &ctx(),
        );
        assert!(e.is_err());
        // Yerinde yan ayarlar serbest.
        let p = balter(
            &o(old.clone()),
            &o(IndexDef {
                fillfactor: Some(70),
                comment: Some("c".into()),
                ..old.clone()
            }),
            &ctx(),
        )
        .unwrap();
        assert_eq!(p.statements.len(), 2);
        // Silme hatası.
        for cascade in [false, true] {
            let e = bdrop(&o(old.clone()), cascade, &ctx()).unwrap_err();
            assert_eq!(e[0].path, "constraint");
        }
    }

    #[test]
    fn drop_variants() {
        let p = bdrop(&o(base()), false, &ctx()).unwrap();
        assert_eq!(
            (sqls(&p), p.statements[0].risk),
            (vec!["DROP INDEX public.t_a_idx"], Risk::Destructive)
        );
        let p = bdrop(&o(base()), true, &ctx()).unwrap();
        assert_eq!(
            (sqls(&p), p.statements[0].risk),
            (vec!["DROP INDEX public.t_a_idx CASCADE"], Risk::DataLoss)
        );
        assert_eq!(p.warnings.len(), 1);
        insta::assert_snapshot!(render(&p));
        // concurrently açık drop'u etkilemez (CASCADE ile uyumsuz).
        let p = bdrop(&o(base()), false, &cctx()).unwrap();
        assert_eq!(sqls(&p), ["DROP INDEX public.t_a_idx"]);
        assert!(p.statements[0].transactional);
        // Sistem şeması drop da reddedilir.
        assert!(
            bdrop(
                &o(IndexDef {
                    schema: "pg_catalog".into(),
                    ..base()
                }),
                false,
                &ctx()
            )
            .is_err()
        );
    }

    #[test]
    fn alter_validates_both_definitions() {
        let bad = IndexDef {
            name: "a\0".into(),
            ..base()
        };
        let e = balter(&o(bad.clone()), &o(base()), &ctx()).unwrap_err();
        assert_eq!(e[0].path, "old.name");
        assert!(balter(&o(base()), &o(bad), &ctx()).is_err());
    }

    #[test]
    fn injection_in_every_name_field_stays_single_statement() {
        let evil = [
            "x\"; DROP TABLE t; --",
            "'",
            "\\",
            "a\\nb",
            "a;b",
            "/*",
            "--",
            "$$",
            "(",
            ")",
            "Ünï;cödé",
            "select",
            "a b",
        ];
        for n in evil {
            let d = IndexDef {
                schema: n.into(),
                table: n.into(),
                name: n.into(),
                columns: vec![IndexColumn {
                    opclass: Some(n.into()),
                    collation: Some(n.into()),
                    ..col(n)
                }],
                include: vec![n.into()],
                tablespace: Some(n.into()),
                comment: Some(n.into()),
                ..base()
            };
            let renamed = IndexDef {
                name: format!("{n}2"),
                comment: Some(format!("{n}3")),
                tablespace: Some(format!("{n}4")),
                fillfactor: Some(20),
                ..d.clone()
            };
            let rebuilt = IndexDef {
                unique: true,
                ..renamed.clone()
            };
            for ctx in [ctx(), cctx()] {
                for p in [
                    bcreate(&o(d.clone()), &ctx).unwrap(),
                    balter(&o(d.clone()), &o(renamed.clone()), &ctx).unwrap(),
                    balter(&o(d.clone()), &o(rebuilt.clone()), &ctx).unwrap(),
                    bdrop(&o(d.clone()), true, &ctx).unwrap(),
                    bdrop(&o(d.clone()), false, &ctx).unwrap(),
                ] {
                    assert!(!p.statements.is_empty());
                    assert_single_statements(&p);
                }
            }
        }
        let d = IndexDef {
            name: "x\"; DROP TABLE t; --".into(),
            ..base()
        };
        let p = bcreate(&o(d), &ctx()).unwrap();
        assert!(
            p.statements[0]
                .sql
                .starts_with("CREATE INDEX \"x\"\"; DROP TABLE t; --\" ON public.t USING")
        );
    }

    #[test]
    fn keyword_and_unicode_names_are_quoted() {
        let d = IndexDef {
            schema: "şema".into(),
            table: "table".into(),
            name: "select".into(),
            columns: vec![col("order"), col("Ğ")],
            include: vec!["user".into()],
            ..base()
        };
        let p = bcreate(&o(d), &ctx()).unwrap();
        assert_eq!(
            p.statements[0].sql,
            "CREATE INDEX \"select\" ON \"şema\".\"table\" USING btree (\"order\", \"Ğ\") INCLUDE (\"user\")"
        );
    }

    #[test]
    fn non_index_free_fields_never_become_sql() {
        // Yöntem sabit kümeden; method alanı üzerinden enjeksiyon doğrulamada reddedilir.
        let d = IndexDef {
            method: "btree (a); DROP TABLE t; --".into(),
            ..base()
        };
        assert_eq!(first_err(&d), "method");
    }
}
