//! Tablespace üreticisi (sunucu kapsamlı): CREATE (konum + seçenekler + yorum), ALTER (rename, SET/RESET seçenekler,
//! yorum, sahip), DROP. Plan 34 Karar 5–8.
//!
//! - **Transaction:** TÜM ifadeler `stmt_nt` (Database kalıbı): `CREATE/DROP TABLESPACE` transaction bloğunda
//!   çalışamaz; diğerleri tutarlılık için aynı kurala uyar (kısmi uygulama `committed` ile bildirilir).
//! - **Konum create-only:** alter'da `location` eski ≠ yeni ise alan hatasıdır. Yükleyici sistem tablespace'inde
//!   (`pg_default`/`pg_global`) boş dizge yazar; bunlar `editable = false` olduğundan alter/drop zaten reddedilir.
//! - **Seçenekler:** değerler sayısal METİN; doğrulanan metnin kendisi tek tırnaklı literal olarak yazılır (kanonik
//!   biçim: ayrıştırılan sayı yeniden biçimlendirilmez), ifadede ad sırası alfabetiktir. Sabit nokta: yükleyici
//!   `spcoptions` değerini aynı metinle okur → `alter(x, x)` boş plandır; yalnız değişen/yazılan seçenek doğrulanır.
//! - **Sıra (alter):** RENAME → SET → RESET → COMMENT → OWNER TO (en sonda: sonraki ifadeler sahiplik ister).
//! - **Drop:** CASCADE yok (dağıtıcının `cascade` bayrağı yok sayılır); Destructive.

use super::table::version_err;
use super::{BuildCtx, check_ident, check_old_ident, check_text, err, stmt_nt};
use crate::model::{FieldError, TablespaceDef, TablespaceOptions};
use crate::plan::{Plan, Risk, Statement};
use crate::quote::{quote_ident, quote_literal};

pub const READ_ONLY_MSG: &str = "Bu tablespace salt okunurdur (sistem tablespace'i); değiştirilemez ve silinemez.";
pub const LOCATION_CHANGE_MSG: &str = "Tablespace konumu oluşturulduktan sonra değiştirilemez.";
pub const DROP_NOTE: &str = "Tablespace boş olmalıdır; içinde nesne varsa sunucu reddeder.";
const PG_PREFIX_MSG: &str = "'pg_' öneki sistem tablespace'lerine ayrılmıştır.";
const MAX_LOCATION_BYTES: usize = 1024;

/// Seçenek tanımı: (SQL adı, yol, ondalık mı). Ad sırası alfabetiktir (ifade sırası bunu izler).
const OPTIONS: [(&str, &str, bool); 4] = [
    ("effective_io_concurrency", "options.effectiveIoConcurrency", false),
    ("maintenance_io_concurrency", "options.maintenanceIoConcurrency", false),
    ("random_page_cost", "options.randomPageCost", true),
    ("seq_page_cost", "options.seqPageCost", true),
];

fn option_value<'a>(o: &'a TablespaceOptions, sql_name: &str) -> &'a Option<String> {
    match sql_name {
        "effective_io_concurrency" => &o.effective_io_concurrency,
        "maintenance_io_concurrency" => &o.maintenance_io_concurrency,
        "random_page_cost" => &o.random_page_cost,
        _ => &o.seq_page_cost,
    }
}

/// Yorum doludur (`Some("")` yorumsuz sayılır; PG `IS ''` yorumu siler).
fn has_comment(c: &Option<String>) -> bool {
    c.as_deref().is_some_and(|s| !s.is_empty())
}

fn norm_comment(c: &Option<String>) -> Option<&str> {
    c.as_deref().filter(|s| !s.is_empty())
}

/// Konum: boş değil, NUL yok, mutlak (`/…` ya da `X:\…` / `X:/…`), ≤ 1024 bayt.
fn check_location(value: &str, e: &mut Vec<FieldError>) {
    let b = value.as_bytes();
    if value.is_empty() {
        e.push(err("location", "Konum boş olamaz."));
    } else if value.contains('\0') {
        e.push(err("location", "Konum NUL karakteri içeremez."));
    } else if value.contains('\'') {
        // PG CreateTableSpace tek tırnağı her zaman reddeder (42602).
        e.push(err("location", "Konum tek tırnak içeremez."));
    } else if value.len() > MAX_LOCATION_BYTES {
        e.push(err("location", format!("Konum en çok {MAX_LOCATION_BYTES} bayt olabilir.")));
    } else if !(b[0] == b'/' || (b.len() >= 3 && b[0].is_ascii_alphabetic() && b[1] == b':' && (b[2] == b'\\' || b[2] == b'/'))) {
        e.push(err("location", "Konum mutlak bir dizin yolu olmalıdır (/… ya da C:\\…)."));
    }
}

/// Seçenek değeri: maliyetler sonlu ondalık ≥ 0, eşzamanlılıklar 0–1000 tamsayı.
fn check_option(path: &str, value: &str, decimal: bool, e: &mut Vec<FieldError>) {
    let ok = if decimal {
        value.chars().all(|c| c.is_ascii_digit() || matches!(c, '.' | 'e' | 'E' | '+' | '-'))
            && value.parse::<f64>().is_ok_and(|n| n.is_finite() && n >= 0.0)
    } else {
        !value.is_empty()
            && value.len() <= 4
            && value.chars().all(|c| c.is_ascii_digit())
            && value.parse::<u32>().is_ok_and(|n| n <= 1000)
    };
    if !ok {
        let msg = if decimal {
            "Değer sonlu bir ondalık sayı ve ≥ 0 olmalıdır."
        } else {
            "Değer 0–1000 arası tamsayı olmalıdır."
        };
        e.push(err(path, msg));
    }
}

/// Yazılan bir seçenek: değer doğrulaması + sürüm kapısı.
fn check_written_option(sql_name: &str, path: &str, decimal: bool, value: &str, ctx: &BuildCtx, e: &mut Vec<FieldError>) {
    if sql_name == "maintenance_io_concurrency" && ctx.version_num < 130000 {
        e.push(version_err(path, "maintenance_io_concurrency", 13));
        return;
    }
    check_option(path, value, decimal, e);
}

/// Oluşturma doğrulaması (tüm alanlar katı).
pub fn validate(ctx: &BuildCtx, d: &TablespaceDef) -> Vec<FieldError> {
    let mut e = vec![];
    check_ident("name", &d.name, &mut e);
    if d.name.starts_with("pg_") {
        e.push(err("name", PG_PREFIX_MSG));
    }
    if let Some(o) = &d.owner {
        check_ident("owner", o, &mut e);
    }
    check_location(&d.location, &mut e);
    for (sql_name, path, decimal) in OPTIONS {
        if let Some(v) = option_value(&d.options, sql_name) {
            check_written_option(sql_name, path, decimal, v, ctx, &mut e);
        }
    }
    if let Some(c) = &d.comment {
        check_text("comment", c, &mut e);
    }
    e
}

fn comment_stmt(q: &str, comment: Option<&str>) -> Statement {
    let value = comment.map_or_else(|| "NULL".to_string(), quote_literal);
    stmt_nt(format!("COMMENT ON TABLESPACE {q} IS {value}"), Risk::Safe, None)
}

/// Sıra: CREATE TABLESPACE (+ WITH seçenekleri), COMMENT (doluysa). Hepsi Safe, transaction dışı.
pub fn create(ctx: &BuildCtx, d: &TablespaceDef) -> Result<Plan, Vec<FieldError>> {
    let errs = validate(ctx, d);
    if !errs.is_empty() {
        return Err(errs);
    }
    let q = quote_ident(&d.name);
    let mut sql = format!("CREATE TABLESPACE {q}");
    if let Some(o) = &d.owner {
        sql.push_str(&format!(" OWNER {}", quote_ident(o)));
    }
    sql.push_str(&format!(" LOCATION {}", quote_literal(&d.location)));
    let with: Vec<String> = OPTIONS
        .iter()
        .filter_map(|(n, _, _)| option_value(&d.options, n).as_ref().map(|v| format!("{n} = {}", quote_literal(v))))
        .collect();
    if !with.is_empty() {
        sql.push_str(&format!(" WITH ({})", with.join(", ")));
    }
    let mut statements = vec![stmt_nt(sql, Risk::Safe, None)];
    if has_comment(&d.comment) {
        statements.push(comment_stmt(&q, norm_comment(&d.comment)));
    }
    Ok(Plan { statements, warnings: vec![] })
}

/// Alter: eski yalnızca ad (gevşek); yeni yalnızca değişen/yazılan alanlar (katı); konum eşit olmalı. Salt okunur eski
/// tanımda her değişiklik `object` hatasıdır (değişiklik yoksa boş plan).
pub fn alter(ctx: &BuildCtx, old: &TablespaceDef, new: &TablespaceDef) -> Result<Plan, Vec<FieldError>> {
    let mut e = vec![];
    check_old_ident("old.name", &old.name, &mut e);
    if !e.is_empty() {
        return Err(e);
    }
    let name_changed = old.name != new.name;
    let owner_changed = new.owner.is_some() && new.owner != old.owner;
    let comment_changed = norm_comment(&old.comment) != norm_comment(&new.comment);
    let location_changed = old.location != new.location;
    let mut set: Vec<(&str, &str)> = vec![];
    let mut reset: Vec<&str> = vec![];
    for (sql_name, path, decimal) in OPTIONS {
        let (o, n) = (option_value(&old.options, sql_name), option_value(&new.options, sql_name));
        match (o, n) {
            (_, Some(v)) if o != n => {
                check_written_option(sql_name, path, decimal, v, ctx, &mut e);
                set.push((sql_name, v));
            }
            (Some(_), None) => reset.push(sql_name),
            _ => {}
        }
    }
    let changed =
        name_changed || owner_changed || comment_changed || location_changed || !set.is_empty() || !reset.is_empty();
    if !old.editable && changed {
        return Err(vec![err("object", READ_ONLY_MSG)]);
    }
    if location_changed {
        e.push(err("location", LOCATION_CHANGE_MSG));
    }
    if name_changed {
        check_ident("name", &new.name, &mut e);
        if new.name.starts_with("pg_") {
            e.push(err("name", PG_PREFIX_MSG));
        }
    }
    if owner_changed && let Some(o) = &new.owner {
        check_ident("owner", o, &mut e);
    }
    if comment_changed && let Some(c) = &new.comment {
        check_text("comment", c, &mut e);
    }
    if !e.is_empty() {
        return Err(e);
    }

    let q = quote_ident(&new.name);
    let mut statements = vec![];
    if name_changed {
        statements.push(stmt_nt(format!("ALTER TABLESPACE {} RENAME TO {q}", quote_ident(&old.name)), Risk::Safe, None));
    }
    if !set.is_empty() {
        let list: Vec<String> = set.iter().map(|(n, v)| format!("{n} = {}", quote_literal(v))).collect();
        statements.push(stmt_nt(format!("ALTER TABLESPACE {q} SET ({})", list.join(", ")), Risk::Safe, None));
    }
    if !reset.is_empty() {
        statements.push(stmt_nt(format!("ALTER TABLESPACE {q} RESET ({})", reset.join(", ")), Risk::Safe, None));
    }
    if comment_changed {
        statements.push(comment_stmt(&q, norm_comment(&new.comment)));
    }
    if owner_changed && let Some(o) = &new.owner {
        statements.push(stmt_nt(format!("ALTER TABLESPACE {q} OWNER TO {}", quote_ident(o)), Risk::Safe, None));
    }
    Ok(Plan { statements, warnings: vec![] })
}

/// `DROP TABLESPACE <q>` (CASCADE yok): Destructive. Kimlik dağıtıcıda doğrulanır; salt okunur silinemez.
pub fn drop(d: &TablespaceDef) -> Result<Plan, Vec<FieldError>> {
    if !d.editable {
        return Err(vec![err("object", READ_ONLY_MSG)]);
    }
    let sql = format!("DROP TABLESPACE {}", quote_ident(&d.name));
    Ok(Plan { statements: vec![stmt_nt(sql, Risk::Destructive, Some(DROP_NOTE))], warnings: vec![] })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::build::{alter as balter, assert_single_statements, create as bcreate, drop as bdrop, render};
    use crate::model::ObjectDef;

    fn t() -> TablespaceDef {
        TablespaceDef {
            name: "ts".into(),
            owner: None,
            location: "/data/ts1".into(),
            options: TablespaceOptions::default(),
            comment: None,
            editable: true,
            unsupported: vec![],
        }
    }
    fn ctx_v(v: u32) -> BuildCtx {
        BuildCtx { version_num: v, ..Default::default() }
    }
    fn o(d: &TablespaceDef) -> ObjectDef {
        ObjectDef::Tablespace(d.clone())
    }
    fn s(x: &str) -> Option<String> {
        Some(x.into())
    }
    fn sqls(p: &Plan) -> Vec<&str> {
        p.statements.iter().map(|x| x.sql.as_str()).collect()
    }
    fn create_ok(d: &TablespaceDef, v: u32) -> Plan {
        let p = bcreate(&o(d), &ctx_v(v)).unwrap();
        assert_single_statements(&p);
        assert!(p.statements.iter().all(|x| !x.transactional));
        p
    }
    fn create_err(d: &TablespaceDef, v: u32) -> Vec<String> {
        match bcreate(&o(d), &ctx_v(v)) {
            Ok(_) => vec![],
            Err(e) => e.into_iter().map(|e| e.path).collect(),
        }
    }
    fn alter_ok(old: &TablespaceDef, new: &TablespaceDef) -> Plan {
        let p = balter(&o(old), &o(new), &ctx_v(160000)).unwrap();
        assert_single_statements(&p);
        assert!(p.statements.iter().all(|x| !x.transactional));
        p
    }
    fn alter_err(old: &TablespaceDef, new: &TablespaceDef) -> Vec<String> {
        balter(&o(old), &o(new), &ctx_v(160000)).unwrap_err().into_iter().map(|e| e.path).collect()
    }
    /// Yüklenmiş biçim.
    fn loaded() -> TablespaceDef {
        TablespaceDef { owner: s("o"), ..t() }
    }
    fn opts(seq: Option<&str>, rnd: Option<&str>, eio: Option<&str>, mio: Option<&str>) -> TablespaceOptions {
        TablespaceOptions {
            seq_page_cost: seq.map(Into::into),
            random_page_cost: rnd.map(Into::into),
            effective_io_concurrency: eio.map(Into::into),
            maintenance_io_concurrency: mio.map(Into::into),
        }
    }

    #[test]
    fn create_variants() {
        let mut out = String::new();
        let cases = [
            t(),
            TablespaceDef { owner: s("Bob Sahip"), ..t() },
            TablespaceDef { location: "C:\\pgdata\\ts".into(), ..t() },
            TablespaceDef { location: "d:/pgdata/ts".into(), ..t() },
            TablespaceDef { location: "/data/a\\b".into(), ..t() },
            TablespaceDef {
                owner: s("o"),
                options: opts(Some("1.5"), Some("1.1"), Some("200"), Some("10")),
                comment: s("it's"),
                ..t()
            },
            TablespaceDef { comment: s(""), ..t() },
        ];
        for d in cases {
            let p = create_ok(&d, 160000);
            assert!(p.warnings.is_empty());
            out.push_str(&render(&p));
            out.push('\n');
        }
        let d = TablespaceDef { owner: s("o"), options: opts(Some("1.5"), None, None, None), comment: s("c"), ..t() };
        let p = create_ok(&d, 160000);
        assert_eq!(
            sqls(&p),
            ["CREATE TABLESPACE ts OWNER o LOCATION '/data/ts1' WITH (seq_page_cost = '1.5')", "COMMENT ON TABLESPACE ts IS 'c'"]
        );
        insta::assert_snapshot!(out);
    }

    #[test]
    fn create_validation() {
        assert_eq!(create_err(&TablespaceDef { name: "".into(), ..t() }, 160000), ["name"]);
        assert_eq!(create_err(&TablespaceDef { name: "pg_x".into(), ..t() }, 160000), ["name"]);
        assert!(create_err(&TablespaceDef { name: "PG_x".into(), ..t() }, 160000).is_empty());
        assert_eq!(create_err(&TablespaceDef { owner: s("a\nb"), ..t() }, 160000), ["owner"]);
        let long = format!("/{}", "a".repeat(1024));
        for loc in ["", "rel/path", "./x", "C:", "C:x", "1:\\x", "/a\0b", "/data/o'brien", long.as_str()] {
            assert_eq!(create_err(&TablespaceDef { location: loc.into(), ..t() }, 160000), ["location"], "{loc:?}");
        }
        assert!(create_err(&TablespaceDef { location: format!("/{}", "a".repeat(1023)), ..t() }, 160000).is_empty());
        assert_eq!(create_err(&TablespaceDef { comment: s("a\0b"), ..t() }, 160000), ["comment"]);
    }

    #[test]
    fn option_validation_table() {
        let costs = [
            ("0", true),
            ("1", true),
            ("1.5", true),
            ("0.0", true),
            ("4", true),
            ("-1", false),
            ("NaN", false),
            ("inf", false),
            ("1e400", false),
            ("abc", false),
            ("", false),
            (" 1", false),
            ("1 ", false),
            ("1,5", false),
        ];
        for (v, ok) in costs {
            for path in ["options.seqPageCost", "options.randomPageCost"] {
                let options =
                    if path.ends_with("seqPageCost") { opts(Some(v), None, None, None) } else { opts(None, Some(v), None, None) };
                let r = create_err(&TablespaceDef { options, ..t() }, 160000);
                assert_eq!(r.is_empty(), ok, "{path} {v:?}");
                if !ok {
                    assert_eq!(r, [path]);
                }
            }
        }
        let ints = [
            ("0", true),
            ("1000", true),
            ("200", true),
            ("1001", false),
            ("-1", false),
            ("1.5", false),
            ("abc", false),
            ("", false),
            ("+5", false),
            ("99999", false),
        ];
        for (v, ok) in ints {
            for path in ["options.effectiveIoConcurrency", "options.maintenanceIoConcurrency"] {
                let options =
                    if path.contains("effective") { opts(None, None, Some(v), None) } else { opts(None, None, None, Some(v)) };
                let r = create_err(&TablespaceDef { options, ..t() }, 160000);
                assert_eq!(r.is_empty(), ok, "{path} {v:?}");
            }
        }
        // Sürüm kapısı: maintenance_io_concurrency PG 13+.
        let d = TablespaceDef { options: opts(None, None, None, Some("10")), ..t() };
        assert_eq!(create_err(&d, 120000), ["options.maintenanceIoConcurrency"]);
        assert!(create_err(&d, 130000).is_empty());
        let e = bcreate(&o(&d), &ctx_v(120000)).unwrap_err();
        assert!(e[0].message.contains("PG 13+"), "{e:?}");
        // effective_io_concurrency PG 12'de serbest.
        let d = TablespaceDef { options: opts(None, None, Some("5"), None), ..t() };
        assert!(create_err(&d, 120000).is_empty());
        // Değer metni kanonik: ayrıştırılan sayı değil, metnin kendisi yazılır.
        let d = TablespaceDef { options: opts(Some("1.50"), Some("2e0"), None, None), ..t() };
        assert_eq!(
            sqls(&create_ok(&d, 160000))[0],
            "CREATE TABLESPACE ts LOCATION '/data/ts1' WITH (random_page_cost = '2e0', seq_page_cost = '1.50')"
        );
    }

    #[test]
    fn alter_noop_and_single_changes() {
        let l = loaded();
        assert!(alter_ok(&l, &l).statements.is_empty());
        let full = TablespaceDef { comment: s("c"), options: opts(Some("1"), Some("1.1"), Some("5"), Some("7")), ..loaded() };
        assert!(alter_ok(&full, &full).statements.is_empty());
        let mut out = String::new();
        let cases: Vec<(TablespaceDef, TablespaceDef)> = vec![
            (l.clone(), TablespaceDef { name: "ts2".into(), ..l.clone() }),
            (l.clone(), TablespaceDef { options: opts(Some("1"), Some("1.1"), None, None), ..l.clone() }),
            (full.clone(), TablespaceDef { options: opts(Some("2"), None, None, None), ..full.clone() }),
            (l.clone(), TablespaceDef { comment: s("yeni"), ..l.clone() }),
            (full.clone(), TablespaceDef { comment: None, ..full.clone() }),
            (l.clone(), TablespaceDef { owner: s("Bob"), ..l.clone() }),
            // Hepsi birden: sıra rename → set → reset → comment → owner; sonrakiler yeni adı kullanır.
            (
                full.clone(),
                TablespaceDef {
                    name: "ts9".into(),
                    owner: s("bob"),
                    comment: s("x"),
                    options: opts(Some("3"), None, Some("6"), None),
                    ..full.clone()
                },
            ),
        ];
        for (old, new) in &cases {
            let p = alter_ok(old, new);
            assert!(!p.statements.is_empty());
            assert!(p.statements.iter().all(|x| x.risk == Risk::Safe));
            out.push_str(&render(&p));
            out.push('\n');
        }
        let p = alter_ok(&cases[6].0, &cases[6].1);
        assert_eq!(
            sqls(&p),
            [
                "ALTER TABLESPACE ts RENAME TO ts9",
                "ALTER TABLESPACE ts9 SET (effective_io_concurrency = '6', seq_page_cost = '3')",
                "ALTER TABLESPACE ts9 RESET (maintenance_io_concurrency, random_page_cost)",
                "COMMENT ON TABLESPACE ts9 IS 'x'",
                "ALTER TABLESPACE ts9 OWNER TO bob",
            ]
        );
        insta::assert_snapshot!(out);
    }

    #[test]
    fn alter_errors() {
        let l = loaded();
        assert_eq!(alter_err(&l, &TablespaceDef { location: "/other".into(), ..l.clone() }), ["location"]);
        assert_eq!(alter_err(&l, &TablespaceDef { name: "pg_x".into(), ..l.clone() }), ["name"]);
        assert_eq!(alter_err(&l, &TablespaceDef { name: "".into(), ..l.clone() }), ["name"]);
        assert_eq!(
            alter_err(&l, &TablespaceDef { options: opts(Some("-1"), None, None, None), ..l.clone() }),
            ["options.seqPageCost"]
        );
        assert_eq!(alter_err(&l, &TablespaceDef { owner: s("a\nb"), ..l.clone() }), ["owner"]);
        let old_bad = TablespaceDef { name: "".into(), ..l.clone() };
        assert_eq!(alter_err(&old_bad, &l), ["old.name"]);
        // Sürüm kapısı yalnız yazılan seçenekte; yüklü maintenance değeri PG 12'de dokunulmazsa sorun değil.
        let kept = TablespaceDef { options: opts(None, None, None, Some("5")), ..l.clone() };
        assert!(balter(&o(&kept), &o(&kept), &ctx_v(120000)).unwrap().statements.is_empty());
        let w = TablespaceDef { options: opts(None, None, None, Some("6")), ..l.clone() };
        assert!(balter(&o(&kept), &o(&w), &ctx_v(120000)).is_err());
        // Yüklü (eski) alanlar gevşek: dokunulmayan yorum geçersiz görünse de düzenleme geçer.
        let weird = TablespaceDef { comment: s("a\nb"), ..l.clone() };
        let p = alter_ok(&weird, &TablespaceDef { name: "ts2".into(), ..weird.clone() });
        assert_eq!(sqls(&p), ["ALTER TABLESPACE ts RENAME TO ts2"]);
    }

    #[test]
    fn read_only_is_rejected() {
        let sys = TablespaceDef {
            name: "pg_default".into(),
            location: "".into(),
            editable: false,
            unsupported: vec!["Sistem tablespace'i".into()],
            ..t()
        };
        assert!(alter_ok(&sys, &sys).statements.is_empty());
        assert_eq!(alter_err(&sys, &TablespaceDef { comment: s("x"), ..sys.clone() }), ["object"]);
        assert_eq!(alter_err(&sys, &TablespaceDef { name: "x".into(), ..sys.clone() }), ["object"]);
        assert_eq!(bdrop(&o(&sys), false, &ctx_v(160000)).unwrap_err()[0].path, "object");
    }

    #[test]
    fn drop_is_destructive_non_tx() {
        let d = loaded();
        for cascade in [false, true] {
            let p = bdrop(&o(&d), cascade, &ctx_v(160000)).unwrap();
            assert_single_statements(&p);
            assert_eq!(sqls(&p), ["DROP TABLESPACE ts"]);
            assert_eq!(p.statements[0].risk, Risk::Destructive);
            assert!(!p.statements[0].transactional);
        }
        let h = TablespaceDef { name: "Ts \"x\"".into(), ..d };
        insta::assert_snapshot!(render(&bdrop(&o(&h), false, &ctx_v(160000)).unwrap()));
    }
}
