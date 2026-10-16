//! Trigger üreticisi: CREATE [CONSTRAINT] TRIGGER, ALTER (RENAME, yeniden kurma, etkinlik, COMMENT), DROP.
//!
//! - **Kapsam:** tablo trigger'ları (tablo kapsamlı tür; `schema`/`table` bağlamdan gelir, alter'da değiştirilemez).
//!   `INSTEAD OF` (view trigger'ları) bu planda reddedilir (backlog).
//! - **Kurallar** (PG `CREATE TRIGGER` belgesi ve `trigger.c`'deki hatalar önden; sunucuya gitmeden alan hatası):
//!   en az bir olay; `TRUNCATE` yalnız `FOR EACH STATEMENT`; constraint trigger yalnız `AFTER … FOR EACH ROW`;
//!   `DEFERRABLE` yalnız constraint trigger'da (`INITIALLY DEFERRED` → `DEFERRABLE`); `UPDATE OF` yalnız `UPDATE` ile,
//!   kolon tekrarı yok; `REFERENCING` (PG 10+, taban) yalnız `AFTER`, constraint trigger'da yok, kolon listesiyle yok,
//!   `TRUNCATE` ile yok, tam olarak bir `INSERT|UPDATE|DELETE` olayıyla; `OLD TABLE` yalnız UPDATE/DELETE, `NEW TABLE`
//!   yalnız INSERT/UPDATE; iki ad farklı.
//! - **Serbest SQL:** yalnız `WHEN` (`check_free_sql` + `wrap`; nihai ifade `composed_is_clean`). Argümanlar
//!   `quote_literal` (NUL yok, ≤ 4096 bayt). Fonksiyon şeması `check_ident` (sistem şeması serbest: yerleşik trigger
//!   fonksiyonları `pg_catalog`'dadır).
//! - **Sürüm:** PG < 11 `EXECUTE PROCEDURE`; `CREATE OR REPLACE TRIGGER` PG 14+ (Karar 5).
//! - **Alter sırası:** (1) `ALTER TRIGGER … RENAME TO`; (2) yapı değiştiyse PG 14+ ve iki taraf da constraint değilse
//!   `CREATE OR REPLACE TRIGGER` (Safe), aksi hâlde aynı transaction'da `DROP TRIGGER` (Destructive + not) + `CREATE`;
//!   (3) etkinlik (`ALTER TABLE … ENABLE/DISABLE … TRIGGER`; kurma etkinliği ORIGIN'e döndürür → hedef ORIGIN değilse
//!   yeniden yazılır); (4) `COMMENT ON TRIGGER` (DROP + CREATE yorumu kaybettirir → doluysa yeniden yazılır).

use super::freesql::{check_free_sql, composed_is_clean, wrap};
use super::{BuildCtx, check_ident, check_old_ident, check_old_ref, check_old_schema_ident, check_schema_ident, check_text, err, stmt};
use crate::model::{FieldError, TriggerDef, TriggerEnabled, TriggerEvents, TriggerLevel, TriggerTiming};
use crate::plan::{Plan, Risk};
use crate::quote::{qualified, quote_ident, quote_literal};

pub const REBUILD_NOTE: &str = "aynı transaction içinde yeniden kurulur";
pub const CASCADE_WARNING: &str = "Bağımlı nesneler de silinir";
pub const READ_ONLY_MSG: &str =
    "Bu trigger salt okunurdur (iç ya da bölümlenmiş üst tablodan klonlanmış); değiştirilemez ve silinemez.";
pub const TABLE_CHANGE_MSG: &str = "Trigger'ın tablosu değiştirilemez.";
/// Tek trigger argümanının üst sınırı (bayt).
pub const MAX_ARG_BYTES: usize = 4096;
const COMPOSED_BROKEN: &str = "WHEN koşulu, yerleştirildiği ifadeyi bozuyor";

fn table_sql(d: &TriggerDef) -> String {
    qualified(&d.schema, &d.table)
}

fn timing_sql(t: TriggerTiming) -> &'static str {
    match t {
        TriggerTiming::Before => "BEFORE",
        TriggerTiming::After => "AFTER",
        TriggerTiming::InsteadOf => "INSTEAD OF",
    }
}

/// `INSERT OR UPDATE [OF <q kolon>, …] OR DELETE OR TRUNCATE` (belge sırası).
fn events_sql(e: &TriggerEvents) -> String {
    let mut parts = vec![];
    if e.insert {
        parts.push("INSERT".to_string());
    }
    if e.update {
        if e.update_columns.is_empty() {
            parts.push("UPDATE".into());
        } else {
            let cols: Vec<String> = e.update_columns.iter().map(|c| quote_ident(c)).collect();
            parts.push(format!("UPDATE OF {}", cols.join(", ")));
        }
    }
    if e.delete {
        parts.push("DELETE".into());
    }
    if e.truncate {
        parts.push("TRUNCATE".into());
    }
    parts.join(" OR ")
}

/// `CREATE [OR REPLACE] [CONSTRAINT] TRIGGER …` (çağıran doğrulamış olmalı; OR REPLACE constraint'te çağrılmaz).
fn define_sql(ctx: &BuildCtx, replace: bool, d: &TriggerDef) -> String {
    let verb = match (replace, d.constraint) {
        (true, _) => "CREATE OR REPLACE TRIGGER",
        (false, true) => "CREATE CONSTRAINT TRIGGER",
        (false, false) => "CREATE TRIGGER",
    };
    let mut s = format!("{verb} {} {} {} ON {}", quote_ident(&d.name), timing_sql(d.timing), events_sql(&d.events), table_sql(d));
    if d.constraint {
        s.push_str(match (d.deferrable, d.initially_deferred) {
            (true, true) => " DEFERRABLE INITIALLY DEFERRED",
            (true, false) => " DEFERRABLE",
            (false, _) => " NOT DEFERRABLE",
        });
    }
    if d.old_table.is_some() || d.new_table.is_some() {
        s.push_str(" REFERENCING");
        if let Some(o) = &d.old_table {
            s.push_str(&format!(" OLD TABLE AS {}", quote_ident(o)));
        }
        if let Some(n) = &d.new_table {
            s.push_str(&format!(" NEW TABLE AS {}", quote_ident(n)));
        }
    }
    s.push_str(match d.level {
        TriggerLevel::Row => " FOR EACH ROW",
        TriggerLevel::Statement => " FOR EACH STATEMENT",
    });
    if let Some(w) = &d.when {
        s.push_str(&format!(" WHEN {}", wrap(w)));
    }
    // PG 10 yalnız `EXECUTE PROCEDURE` bilir; PG 11+ `EXECUTE FUNCTION` (eşanlamlı).
    let exec = if ctx.version_num < 110000 { "PROCEDURE" } else { "FUNCTION" };
    let args: Vec<String> = d.args.iter().map(|a| quote_literal(a)).collect();
    s.push_str(&format!(" EXECUTE {exec} {}({})", qualified(&d.function_schema, &d.function_name), args.join(", ")));
    s
}

/// `ALTER TABLE <tablo> {ENABLE|DISABLE|ENABLE REPLICA|ENABLE ALWAYS} TRIGGER <q>`.
fn enabled_sql(d: &TriggerDef) -> String {
    let verb = match d.enabled {
        TriggerEnabled::Origin => "ENABLE",
        TriggerEnabled::Disabled => "DISABLE",
        TriggerEnabled::Replica => "ENABLE REPLICA",
        TriggerEnabled::Always => "ENABLE ALWAYS",
    };
    format!("ALTER TABLE {} {verb} TRIGGER {}", table_sql(d), quote_ident(&d.name))
}

fn comment_sql(d: &TriggerDef) -> String {
    let value = d.comment.as_deref().map(quote_literal).unwrap_or_else(|| "NULL".into());
    format!("COMMENT ON TRIGGER {} ON {} IS {value}", quote_ident(&d.name), table_sql(d))
}

/// Ad yolu: alter'da aynı değeri taşıyan yüklü ad gevşek (`check_old_ref`), diğerleri katı (`check_ident`).
fn check_name(path: &str, value: &str, loaded: bool, e: &mut Vec<FieldError>) {
    if loaded {
        check_old_ref(path, value, e);
    } else {
        check_ident(path, value, e);
    }
}

/// `CREATE` ifadesine yazılan alanlar (ad/şema/tablo/yorum/etkinlik hariç). `old` verilirse değişmeyen yüklü adlar
/// (fonksiyon, UPDATE OF kolonları, transition tablo adları) gevşek denetlenir.
fn validate_definition(d: &TriggerDef, old: Option<&TriggerDef>, e: &mut Vec<FieldError>) {
    let ev = &d.events;
    let after = d.timing == TriggerTiming::After;

    // Olaylar.
    if !(ev.insert || ev.update || ev.delete || ev.truncate) {
        e.push(err("events", "En az bir olay (INSERT, UPDATE, DELETE, TRUNCATE) seçilmeli."));
    }
    if !ev.update_columns.is_empty() && !ev.update {
        e.push(err("events.updateColumns", "UPDATE OF kolonları yalnızca UPDATE olayıyla kullanılır."));
    }
    let old_cols = old.map(|o| o.events.update_columns.as_slice()).unwrap_or_default();
    for (i, c) in ev.update_columns.iter().enumerate() {
        let path = format!("events.updateColumns[{i}]");
        let before = e.len();
        check_name(&path, c, old_cols.get(i) == Some(c), e);
        if e.len() == before && ev.update_columns[..i].contains(c) {
            e.push(err(&path, "Kolon birden çok kez belirtilmiş."));
        }
    }
    if ev.truncate && d.level == TriggerLevel::Row {
        e.push(err("events.truncate", "TRUNCATE trigger'ı yalnızca FOR EACH STATEMENT olabilir."));
    }

    // Zamanlama, düzey, constraint.
    if d.timing == TriggerTiming::InsteadOf {
        e.push(err("timing", "INSTEAD OF yalnızca view trigger'larında kullanılır; tablo trigger'ı BEFORE ya da AFTER olmalı."));
    } else if d.constraint && !after {
        e.push(err("timing", "Constraint trigger yalnızca AFTER olabilir."));
    }
    if d.constraint && d.level != TriggerLevel::Row {
        e.push(err("level", "Constraint trigger yalnızca FOR EACH ROW olabilir."));
    }
    if d.deferrable && !d.constraint {
        e.push(err("deferrable", "DEFERRABLE yalnızca constraint trigger'da kullanılır."));
    }
    if d.initially_deferred && !d.deferrable {
        e.push(err("initiallyDeferred", "INITIALLY DEFERRED için DEFERRABLE seçilmeli."));
    }

    // REFERENCING (transition tabloları). Genel kural ihlali dolu ilk alana yazılır.
    let ref_path = if d.old_table.is_some() { "oldTable" } else { "newTable" };
    if d.old_table.is_some() || d.new_table.is_some() {
        let one_event = [ev.insert, ev.update, ev.delete].iter().filter(|x| **x).count() == 1;
        let general = if !after {
            Some("REFERENCING yalnızca AFTER trigger'da kullanılır.")
        } else if d.constraint {
            Some("Constraint trigger REFERENCING kullanamaz.")
        } else if !ev.update_columns.is_empty() {
            Some("REFERENCING, UPDATE OF kolon listesiyle birlikte kullanılamaz.")
        } else if ev.truncate {
            Some("REFERENCING, TRUNCATE trigger'ında kullanılamaz.")
        } else if !one_event {
            Some("REFERENCING yalnızca tek olaylı (INSERT, UPDATE ya da DELETE) trigger'da kullanılır.")
        } else {
            None
        };
        if let Some(m) = general {
            e.push(err(ref_path, m));
        } else {
            if let Some(n) = &d.old_table {
                let before = e.len();
                check_name("oldTable", n, old.is_some_and(|o| o.old_table.as_ref() == Some(n)), e);
                if e.len() == before && !(ev.update || ev.delete) {
                    e.push(err("oldTable", "OLD TABLE yalnızca UPDATE ya da DELETE trigger'ında kullanılır."));
                }
            }
            if let Some(n) = &d.new_table {
                let before = e.len();
                check_name("newTable", n, old.is_some_and(|o| o.new_table.as_ref() == Some(n)), e);
                if e.len() == before {
                    if !(ev.insert || ev.update) {
                        e.push(err("newTable", "NEW TABLE yalnızca INSERT ya da UPDATE trigger'ında kullanılır."));
                    } else if d.old_table.as_ref() == Some(n) {
                        e.push(err("newTable", "OLD TABLE ve NEW TABLE adları farklı olmalı."));
                    }
                }
            }
        }
    }

    // WHEN (serbest SQL).
    if let Some(w) = &d.when {
        check_free_sql("when", w, e);
    }

    // Fonksiyon ve argümanlar.
    let same_fn = old.is_some_and(|o| o.function_schema == d.function_schema && o.function_name == d.function_name);
    check_name("functionSchema", &d.function_schema, same_fn, e);
    check_name("functionName", &d.function_name, same_fn, e);
    for (i, a) in d.args.iter().enumerate() {
        let path = format!("args[{i}]");
        if a.contains('\0') {
            e.push(err(&path, "Argüman NUL karakteri içeremez."));
        } else if a.len() > MAX_ARG_BYTES {
            e.push(err(&path, format!("Argüman en çok {MAX_ARG_BYTES} bayt olabilir.")));
        }
    }
}

/// Nihai ifade taraması (yalnız diğer her şey geçerliyse). Tek serbest parça `WHEN`'dir.
fn check_composed(sql: impl FnOnce() -> String, e: &mut Vec<FieldError>) {
    if e.is_empty() && !composed_is_clean(&sql()) {
        e.push(err("when", COMPOSED_BROKEN));
    }
}

/// Oluşturma doğrulaması (tüm alanlar katı).
pub fn validate(ctx: &BuildCtx, d: &TriggerDef) -> Vec<FieldError> {
    let mut e = vec![];
    check_schema_ident("schema", &d.schema, &mut e);
    check_ident("table", &d.table, &mut e);
    check_ident("name", &d.name, &mut e);
    if let Some(c) = &d.comment {
        check_text("comment", c, &mut e);
    }
    validate_definition(d, None, &mut e);
    check_composed(|| define_sql(ctx, false, d), &mut e);
    e
}

/// Sıra: CREATE, etkinlik (ORIGIN değilse), COMMENT (doluysa).
pub fn create(ctx: &BuildCtx, d: &TriggerDef) -> Result<Plan, Vec<FieldError>> {
    let errs = validate(ctx, d);
    if !errs.is_empty() {
        return Err(errs);
    }
    let mut statements = vec![stmt(define_sql(ctx, false, d), Risk::Safe, None)];
    if d.enabled != TriggerEnabled::Origin {
        statements.push(stmt(enabled_sql(d), Risk::Safe, None));
    }
    if d.comment.is_some() {
        statements.push(stmt(comment_sql(d), Risk::Safe, None));
    }
    Ok(Plan { statements, warnings: vec![] })
}

/// `CREATE` ifadesine yazılan yapı alanlarından biri değişti mi (ad/tablo/etkinlik/yorum hariç)?
fn structure_changed(o: &TriggerDef, n: &TriggerDef) -> bool {
    o.timing != n.timing
        || o.events != n.events
        || o.level != n.level
        || o.when != n.when
        || o.function_schema != n.function_schema
        || o.function_name != n.function_name
        || o.args != n.args
        || o.old_table != n.old_table
        || o.new_table != n.new_table
        || o.constraint != n.constraint
        || o.deferrable != n.deferrable
        || o.initially_deferred != n.initially_deferred
}

/// Sıra (Global Constraints): RENAME → yeniden kurma (yeni adla) → etkinlik → COMMENT. `editable = false` eski
/// tanımda her değişiklik `object` hatasıdır (değişiklik yoksa boş plan).
pub fn alter(ctx: &BuildCtx, old: &TriggerDef, new: &TriggerDef) -> Result<Plan, Vec<FieldError>> {
    let mut e = vec![];
    check_old_schema_ident("old.schema", &old.schema, &mut e);
    check_old_ident("old.table", &old.table, &mut e);
    check_old_ident("old.name", &old.name, &mut e);
    if !e.is_empty() {
        return Err(e);
    }
    if old.schema != new.schema {
        e.push(err("schema", TABLE_CHANGE_MSG));
    }
    if old.table != new.table {
        e.push(err("table", TABLE_CHANGE_MSG));
    }
    if !e.is_empty() {
        return Err(e);
    }
    let rebuild = structure_changed(old, new);
    let name_changed = old.name != new.name;
    let enabled_changed = old.enabled != new.enabled;
    let comment_changed = old.comment != new.comment;
    if !old.editable && (rebuild || name_changed || enabled_changed || comment_changed) {
        return Err(vec![err("object", READ_ONLY_MSG)]);
    }
    // PG 14+ OR REPLACE; constraint trigger (her iki taraf) OR REPLACE ile kurulamaz ve değiştirilemez.
    let replace = rebuild && ctx.version_num >= 140000 && !old.constraint && !new.constraint;
    let recreate = rebuild && !replace;
    let write_comment = comment_changed || (recreate && new.comment.is_some());

    // Ad aynıysa yüklü ad `old.name` olarak zaten (gevşek) denetlendi.
    if name_changed {
        check_ident("name", &new.name, &mut e);
    }
    if write_comment && let Some(c) = &new.comment {
        check_text("comment", c, &mut e);
    }
    let mut define = None;
    if rebuild {
        validate_definition(new, Some(old), &mut e);
        if e.is_empty() {
            let sql = define_sql(ctx, replace, new);
            check_composed(|| sql.clone(), &mut e);
            define = Some(sql);
        }
    }
    if !e.is_empty() {
        return Err(e);
    }

    let mut statements = vec![];
    if name_changed {
        statements.push(stmt(
            format!("ALTER TRIGGER {} ON {} RENAME TO {}", quote_ident(&old.name), table_sql(old), quote_ident(&new.name)),
            Risk::Safe,
            None,
        ));
    }
    if let Some(sql) = define {
        if recreate {
            statements.push(stmt(
                format!("DROP TRIGGER {} ON {}", quote_ident(&new.name), table_sql(new)),
                Risk::Destructive,
                Some(REBUILD_NOTE),
            ));
        }
        statements.push(stmt(sql, Risk::Safe, None));
    }
    // Kurma (OR REPLACE dahil) etkinliği ORIGIN'e döndürür.
    if (rebuild && new.enabled != TriggerEnabled::Origin) || (!rebuild && enabled_changed) {
        statements.push(stmt(enabled_sql(new), Risk::Safe, None));
    }
    if write_comment {
        statements.push(stmt(comment_sql(new), Risk::Safe, None));
    }
    Ok(Plan { statements, warnings: vec![] })
}

/// `DROP TRIGGER <q> ON <tablo> [CASCADE]`; kimlik dağıtıcıda (`validate_identity`) doğrulanır. Salt okunur silinemez.
pub fn drop(d: &TriggerDef, cascade: bool) -> Result<Plan, Vec<FieldError>> {
    if !d.editable {
        return Err(vec![err("object", READ_ONLY_MSG)]);
    }
    let sql = format!("DROP TRIGGER {} ON {}", quote_ident(&d.name), table_sql(d));
    Ok(if cascade {
        Plan {
            statements: vec![stmt(format!("{sql} CASCADE"), Risk::DataLoss, Some(CASCADE_WARNING))],
            warnings: vec![CASCADE_WARNING.into()],
        }
    } else {
        Plan { statements: vec![stmt(sql, Risk::Destructive, None)], warnings: vec![] }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::build::freesql::composed_is_clean;
    use crate::build::{alter as balter, assert_single_statements, create as bcreate, drop as bdrop, render};
    use crate::model::{ObjectDef, TriggerEnabled, TriggerEvents, TriggerLevel, TriggerTiming};
    use crate::plan::Risk;

    fn ev() -> TriggerEvents {
        TriggerEvents { insert: false, update: false, update_columns: vec![], delete: false, truncate: false }
    }
    fn ins() -> TriggerEvents {
        TriggerEvents { insert: true, ..ev() }
    }
    fn upd(cols: &[&str]) -> TriggerEvents {
        TriggerEvents { update: true, update_columns: cols.iter().map(|c| (*c).to_string()).collect(), ..ev() }
    }
    fn t() -> TriggerDef {
        TriggerDef {
            schema: "public".into(),
            table: "t".into(),
            name: "tr".into(),
            timing: TriggerTiming::Before,
            events: ins(),
            level: TriggerLevel::Row,
            when: None,
            function_schema: "public".into(),
            function_name: "tf".into(),
            args: vec![],
            old_table: None,
            new_table: None,
            constraint: false,
            deferrable: false,
            initially_deferred: false,
            enabled: TriggerEnabled::Origin,
            comment: None,
            editable: true,
            unsupported: vec![],
        }
    }
    /// AFTER … FOR EACH ROW constraint trigger.
    fn ct() -> TriggerDef {
        TriggerDef { timing: TriggerTiming::After, constraint: true, ..t() }
    }
    fn ctx_v(version_num: u32) -> BuildCtx {
        BuildCtx { version_num, ..Default::default() }
    }
    fn ctx() -> BuildCtx {
        ctx_v(160000)
    }
    fn o(d: &TriggerDef) -> ObjectDef {
        ObjectDef::Trigger(d.clone())
    }
    fn sqls(p: &Plan) -> Vec<&str> {
        p.statements.iter().map(|s| s.sql.as_str()).collect()
    }
    fn paths(errs: Vec<FieldError>) -> Vec<String> {
        errs.into_iter().map(|e| e.path).collect()
    }
    fn create_ok_v(d: &TriggerDef, v: u32) -> Plan {
        let p = bcreate(&o(d), &ctx_v(v)).unwrap();
        assert_single_statements(&p);
        p
    }
    fn create_err(d: &TriggerDef) -> Vec<String> {
        paths(bcreate(&o(d), &ctx()).unwrap_err())
    }
    fn alter_ok_v(old: &TriggerDef, new: &TriggerDef, v: u32) -> Plan {
        let p = balter(&o(old), &o(new), &ctx_v(v)).unwrap();
        assert_single_statements(&p);
        p
    }
    fn alter_ok(old: &TriggerDef, new: &TriggerDef) -> Plan {
        alter_ok_v(old, new, 160000)
    }
    fn alter_err(old: &TriggerDef, new: &TriggerDef) -> Vec<String> {
        paths(balter(&o(old), &o(new), &ctx()).unwrap_err())
    }

    #[test]
    fn create_variants() {
        let cases: Vec<(&str, TriggerDef)> = vec![
            ("before row insert or update of", TriggerDef { events: TriggerEvents { insert: true, ..upd(&["a", "Kolon B"]) }, ..t() }),
            (
                "after statement truncate",
                TriggerDef { timing: TriggerTiming::After, level: TriggerLevel::Statement, events: TriggerEvents { truncate: true, ..ev() }, ..t() },
            ),
            ("tüm olaylar statement", TriggerDef {
                level: TriggerLevel::Statement,
                events: TriggerEvents { insert: true, update: true, update_columns: vec![], delete: true, truncate: true },
                ..t()
            }),
            ("when", TriggerDef { events: upd(&[]), when: Some("OLD.x IS DISTINCT FROM NEW.x -- yorum
".into()), ..t() }),
            ("argümanlı", TriggerDef { args: vec!["a".into(), "it's".into(), "c:\\yol".into(), "ş 日本".into(), String::new()], ..t() }),
            (
                "referencing statement",
                TriggerDef {
                    timing: TriggerTiming::After,
                    level: TriggerLevel::Statement,
                    events: upd(&[]),
                    old_table: Some("eski".into()),
                    new_table: Some("Yeni".into()),
                    ..t()
                },
            ),
            ("referencing row", TriggerDef { timing: TriggerTiming::After, new_table: Some("n".into()), ..t() }),
            ("constraint not deferrable", ct()),
            ("constraint deferrable", TriggerDef { deferrable: true, ..ct() }),
            ("constraint deferrable initially deferred", TriggerDef { deferrable: true, initially_deferred: true, events: TriggerEvents { delete: true, ..upd(&["a"]) }, ..ct() }),
            ("disabled + yorum", TriggerDef { enabled: TriggerEnabled::Disabled, comment: Some("it's".into()), ..t() }),
            ("replica", TriggerDef { enabled: TriggerEnabled::Replica, ..t() }),
            ("always", TriggerDef { enabled: TriggerEnabled::Always, ..t() }),
            ("fonksiyon pg_catalog", TriggerDef { function_schema: "pg_catalog".into(), function_name: "suppress_redundant_updates_trigger".into(), events: upd(&[]), ..t() }),
        ];
        let mut out = String::new();
        for (label, d) in cases {
            let p = create_ok_v(&d, 160000);
            assert!(p.statements.iter().all(|s| s.risk == Risk::Safe && s.transactional), "{label}");
            assert!(p.warnings.is_empty(), "{label}");
            out.push_str(&format!("# {label}\n{}\n", render(&p)));
        }
        insta::assert_snapshot!(out);
        assert_eq!(
            sqls(&create_ok_v(&t(), 160000)),
            ["CREATE TRIGGER tr BEFORE INSERT ON public.t FOR EACH ROW EXECUTE FUNCTION public.tf()"]
        );
        let full = TriggerDef {
            events: TriggerEvents { insert: true, ..upd(&["a"]) },
            when: Some("NEW.a > 0".into()),
            args: vec!["x".into(), "it's".into()],
            enabled: TriggerEnabled::Disabled,
            comment: Some("c".into()),
            ..t()
        };
        assert_eq!(
            sqls(&create_ok_v(&full, 160000)),
            [
                "CREATE TRIGGER tr BEFORE INSERT OR UPDATE OF a ON public.t FOR EACH ROW WHEN (NEW.a > 0\n) EXECUTE FUNCTION public.tf('x', 'it''s')",
                "ALTER TABLE public.t DISABLE TRIGGER tr",
                "COMMENT ON TRIGGER tr ON public.t IS 'c'",
            ]
        );
        let cons = TriggerDef { deferrable: true, initially_deferred: true, ..ct() };
        assert_eq!(
            sqls(&create_ok_v(&cons, 160000)),
            ["CREATE CONSTRAINT TRIGGER tr AFTER INSERT ON public.t DEFERRABLE INITIALLY DEFERRED FOR EACH ROW EXECUTE FUNCTION public.tf()"]
        );
        let refs = TriggerDef { timing: TriggerTiming::After, level: TriggerLevel::Statement, events: TriggerEvents { delete: true, ..ev() }, old_table: Some("o".into()), ..t() };
        assert_eq!(
            sqls(&create_ok_v(&refs, 160000)),
            ["CREATE TRIGGER tr AFTER DELETE ON public.t REFERENCING OLD TABLE AS o FOR EACH STATEMENT EXECUTE FUNCTION public.tf()"]
        );
    }

    #[test]
    fn pg10_uses_execute_procedure() {
        let d = TriggerDef { args: vec!["a".into()], ..t() };
        assert_eq!(sqls(&create_ok_v(&d, 100000)), ["CREATE TRIGGER tr BEFORE INSERT ON public.t FOR EACH ROW EXECUTE PROCEDURE public.tf('a')"]);
        assert_eq!(sqls(&create_ok_v(&d, 109999)), ["CREATE TRIGGER tr BEFORE INSERT ON public.t FOR EACH ROW EXECUTE PROCEDURE public.tf('a')"]);
        assert_eq!(sqls(&create_ok_v(&d, 110000)), ["CREATE TRIGGER tr BEFORE INSERT ON public.t FOR EACH ROW EXECUTE FUNCTION public.tf('a')"]);
        // PG 10 yeniden kurması da PROCEDURE yazar.
        let pl = alter_ok_v(&t(), &TriggerDef { timing: TriggerTiming::After, ..d.clone() }, 100000);
        assert!(pl.statements[1].sql.ends_with(" EXECUTE PROCEDURE public.tf('a')"), "{}", pl.statements[1].sql);
    }

    #[test]
    fn create_rule_violations() {
        let long = "x".repeat(MAX_ARG_BYTES + 1);
        let a = TriggerTiming::After;
        let st = TriggerLevel::Statement;
        let cases: Vec<(&str, TriggerDef, &str)> = vec![
            ("olay yok", TriggerDef { events: ev(), ..t() }, "events"),
            ("UPDATE OF olmadan UPDATE", TriggerDef { events: TriggerEvents { insert: true, update_columns: vec!["a".into()], ..ev() }, ..t() }, "events.updateColumns"),
            ("UPDATE OF kolon boş", TriggerDef { events: upd(&[""]), ..t() }, "events.updateColumns[0]"),
            ("UPDATE OF kolon satır sonu", TriggerDef { events: upd(&["a", "b\nc"]), ..t() }, "events.updateColumns[1]"),
            ("UPDATE OF kolon tekrar", TriggerDef { events: upd(&["a", "a"]), ..t() }, "events.updateColumns[1]"),
            ("TRUNCATE ROW", TriggerDef { events: TriggerEvents { truncate: true, ..ev() }, ..t() }, "events.truncate"),
            ("INSTEAD OF", TriggerDef { timing: TriggerTiming::InsteadOf, ..t() }, "timing"),
            ("constraint BEFORE", TriggerDef { timing: TriggerTiming::Before, ..ct() }, "timing"),
            ("constraint INSTEAD OF", TriggerDef { timing: TriggerTiming::InsteadOf, ..ct() }, "timing"),
            ("constraint STATEMENT", TriggerDef { level: st, ..ct() }, "level"),
            ("DEFERRABLE constraint değil", TriggerDef { deferrable: true, ..t() }, "deferrable"),
            ("INITIALLY DEFERRED DEFERRABLE değil", TriggerDef { initially_deferred: true, ..ct() }, "initiallyDeferred"),
            ("REFERENCING BEFORE", TriggerDef { new_table: Some("n".into()), ..t() }, "newTable"),
            ("REFERENCING constraint", TriggerDef { new_table: Some("n".into()), ..ct() }, "newTable"),
            ("REFERENCING UPDATE OF", TriggerDef { timing: a, events: upd(&["a"]), old_table: Some("o".into()), ..t() }, "oldTable"),
            ("REFERENCING çok olay", TriggerDef { timing: a, events: TriggerEvents { insert: true, ..upd(&[]) }, new_table: Some("n".into()), ..t() }, "newTable"),
            ("REFERENCING TRUNCATE", TriggerDef { timing: a, level: st, events: TriggerEvents { truncate: true, ..ins() }, new_table: Some("n".into()), ..t() }, "newTable"),
            ("OLD TABLE INSERT", TriggerDef { timing: a, old_table: Some("o".into()), ..t() }, "oldTable"),
            ("NEW TABLE DELETE", TriggerDef { timing: a, events: TriggerEvents { delete: true, ..ev() }, new_table: Some("n".into()), ..t() }, "newTable"),
            ("OLD = NEW adı", TriggerDef { timing: a, events: upd(&[]), old_table: Some("x".into()), new_table: Some("x".into()), ..t() }, "newTable"),
            ("OLD TABLE adı boş", TriggerDef { timing: a, events: upd(&[]), old_table: Some(String::new()), ..t() }, "oldTable"),
            ("WHEN enjeksiyon", TriggerDef { when: Some("true) EXECUTE FUNCTION x(); --".into()), ..t() }, "when"),
            ("WHEN ;", TriggerDef { when: Some("true; DROP TABLE t".into()), ..t() }, "when"),
            ("WHEN boş", TriggerDef { when: Some(" ".into()), ..t() }, "when"),
            ("WHEN \\ sıradan dizede", TriggerDef { when: Some("NEW.a = 'a\\b'".into()), ..t() }, "when"),
            ("fonksiyon şeması boş", TriggerDef { function_schema: String::new(), ..t() }, "functionSchema"),
            ("fonksiyon adı boş", TriggerDef { function_name: String::new(), ..t() }, "functionName"),
            ("fonksiyon adı NUL", TriggerDef { function_name: "f\0".into(), ..t() }, "functionName"),
            ("argüman NUL", TriggerDef { args: vec!["a".into(), "b\0".into()], ..t() }, "args[1]"),
            ("argüman uzun", TriggerDef { args: vec![long.clone()], ..t() }, "args[0]"),
            ("ad boş", TriggerDef { name: String::new(), ..t() }, "name"),
            ("ad 64 bayt", TriggerDef { name: "a".repeat(64), ..t() }, "name"),
            ("şema sistem", TriggerDef { schema: "pg_catalog".into(), ..t() }, "schema"),
            ("tablo boş", TriggerDef { table: String::new(), ..t() }, "table"),
            ("yorum NUL", TriggerDef { comment: Some("a\0".into()), ..t() }, "comment"),
        ];
        let mut out = String::new();
        for (label, d, want) in cases {
            assert_eq!(create_err(&d), [want], "{label}");
            assert_eq!(paths(validate(&ctx(), &d)), [want], "{label}");
            out.push_str(&format!("{label}: {want}\n"));
        }
        insta::assert_snapshot!(out);
        // Sınırda argüman (tam 4096 bayt) kabul edilir; REFERENCING ROW düzeyinde AFTER'da serbest.
        assert!(bcreate(&o(&TriggerDef { args: vec!["x".repeat(MAX_ARG_BYTES)], ..t() }), &ctx()).is_ok());
        assert!(bcreate(&o(&TriggerDef { timing: a, events: upd(&[]), old_table: Some("o".into()), ..t() }), &ctx()).is_ok());
    }

    #[test]
    fn alter_single_changes() {
        let base = TriggerDef { comment: Some("c".into()), ..t() };
        let cases: Vec<(&str, TriggerDef, Vec<&str>)> = vec![
            ("ad", TriggerDef { name: "Yeni Ad".into(), ..base.clone() }, vec!["ALTER TRIGGER tr ON public.t RENAME TO \"Yeni Ad\""]),
            ("disabled", TriggerDef { enabled: TriggerEnabled::Disabled, ..base.clone() }, vec!["ALTER TABLE public.t DISABLE TRIGGER tr"]),
            ("replica", TriggerDef { enabled: TriggerEnabled::Replica, ..base.clone() }, vec!["ALTER TABLE public.t ENABLE REPLICA TRIGGER tr"]),
            ("always", TriggerDef { enabled: TriggerEnabled::Always, ..base.clone() }, vec!["ALTER TABLE public.t ENABLE ALWAYS TRIGGER tr"]),
            ("yorum", TriggerDef { comment: Some("d".into()), ..base.clone() }, vec!["COMMENT ON TRIGGER tr ON public.t IS 'd'"]),
            ("yorum None", TriggerDef { comment: None, ..base.clone() }, vec!["COMMENT ON TRIGGER tr ON public.t IS NULL"]),
            (
                "zamanlama (PG 14+ OR REPLACE)",
                TriggerDef { timing: TriggerTiming::After, ..base.clone() },
                vec!["CREATE OR REPLACE TRIGGER tr AFTER INSERT ON public.t FOR EACH ROW EXECUTE FUNCTION public.tf()"],
            ),
            (
                "olay",
                TriggerDef { events: TriggerEvents { delete: true, ..ins() }, ..base.clone() },
                vec!["CREATE OR REPLACE TRIGGER tr BEFORE INSERT OR DELETE ON public.t FOR EACH ROW EXECUTE FUNCTION public.tf()"],
            ),
            (
                "WHEN",
                TriggerDef { when: Some("NEW.a > 1".into()), ..base.clone() },
                vec!["CREATE OR REPLACE TRIGGER tr BEFORE INSERT ON public.t FOR EACH ROW WHEN (NEW.a > 1\n) EXECUTE FUNCTION public.tf()"],
            ),
            (
                "argüman",
                TriggerDef { args: vec!["z".into()], ..base.clone() },
                vec!["CREATE OR REPLACE TRIGGER tr BEFORE INSERT ON public.t FOR EACH ROW EXECUTE FUNCTION public.tf('z')"],
            ),
            (
                "fonksiyon",
                TriggerDef { function_schema: "app".into(), function_name: "g".into(), ..base.clone() },
                vec!["CREATE OR REPLACE TRIGGER tr BEFORE INSERT ON public.t FOR EACH ROW EXECUTE FUNCTION app.g()"],
            ),
            (
                "düzey",
                TriggerDef { level: TriggerLevel::Statement, ..base.clone() },
                vec!["CREATE OR REPLACE TRIGGER tr BEFORE INSERT ON public.t FOR EACH STATEMENT EXECUTE FUNCTION public.tf()"],
            ),
            // Yükleme alanları üretici tarafından okunmaz.
            ("değişmez", TriggerDef { unsupported: vec!["x".into()], ..base.clone() }, vec![]),
        ];
        let mut out = String::new();
        for (label, new, want) in cases {
            let pl = alter_ok(&base, &new);
            assert_eq!(sqls(&pl), want, "{label}");
            assert!(pl.statements.iter().all(|s| s.risk == Risk::Safe && s.transactional), "{label}");
            assert!(pl.warnings.is_empty(), "{label}");
            out.push_str(&format!("# {label}\n{}\n", render(&pl)));
        }
        insta::assert_snapshot!(out);
    }

    #[test]
    fn or_replace_reemits_non_origin_enabled_state() {
        // CREATE OR REPLACE TRIGGER tgenabled'ı ORIGIN'e döndürür: hedef etkinlik ORIGIN değilse yeniden yazılır.
        let old = TriggerDef { enabled: TriggerEnabled::Disabled, ..t() };
        let new = TriggerDef { args: vec!["a".into()], ..old.clone() };
        assert_eq!(
            sqls(&alter_ok(&old, &new)),
            [
                "CREATE OR REPLACE TRIGGER tr BEFORE INSERT ON public.t FOR EACH ROW EXECUTE FUNCTION public.tf('a')",
                "ALTER TABLE public.t DISABLE TRIGGER tr",
            ]
        );
        // Hedef ORIGIN: yeniden yazım yok (kurma zaten ORIGIN'dir), eski DISABLED olsa bile.
        let new = TriggerDef { enabled: TriggerEnabled::Origin, ..new };
        assert_eq!(sqls(&alter_ok(&old, &new)).len(), 1);
        // OR REPLACE yorumu korur (aynı OID): yorum yeniden yazılmaz.
        let old = TriggerDef { comment: Some("c".into()), ..t() };
        assert_eq!(sqls(&alter_ok(&old, &TriggerDef { args: vec!["a".into()], ..old.clone() })).len(), 1);
    }

    #[test]
    fn rebuild_pg13_drops_and_recreates_in_same_transaction() {
        let old = TriggerDef { enabled: TriggerEnabled::Always, comment: Some("c".into()), ..t() };
        let new = TriggerDef { timing: TriggerTiming::After, ..old.clone() };
        let pl = alter_ok_v(&old, &new, 130000);
        assert_eq!(
            sqls(&pl),
            [
                "DROP TRIGGER tr ON public.t",
                "CREATE TRIGGER tr AFTER INSERT ON public.t FOR EACH ROW EXECUTE FUNCTION public.tf()",
                "ALTER TABLE public.t ENABLE ALWAYS TRIGGER tr",
                "COMMENT ON TRIGGER tr ON public.t IS 'c'",
            ]
        );
        assert_eq!(pl.statements[0].risk, Risk::Destructive);
        assert_eq!(pl.statements[0].note.as_deref(), Some(REBUILD_NOTE));
        assert!(pl.statements[1..].iter().all(|s| s.risk == Risk::Safe));
        assert!(pl.statements.iter().all(|s| s.transactional));
        assert_eq!(pl.risk(), Risk::Destructive);
        insta::assert_snapshot!(render(&pl));
        // ORIGIN hedef ve yorumsuz: yalnız DROP + CREATE.
        let pl = alter_ok_v(&t(), &TriggerDef { timing: TriggerTiming::After, ..t() }, 130000);
        assert_eq!(sqls(&pl).len(), 2);
        // PG 14 sınırı.
        assert!(alter_ok_v(&t(), &TriggerDef { timing: TriggerTiming::After, ..t() }, 140000).statements[0].sql.starts_with("CREATE OR REPLACE TRIGGER"));
        assert!(alter_ok_v(&t(), &TriggerDef { timing: TriggerTiming::After, ..t() }, 139999).statements[0].sql.starts_with("DROP TRIGGER"));
    }

    #[test]
    fn constraint_trigger_always_rebuilds() {
        // PG CREATE OR REPLACE CONSTRAINT TRIGGER'ı desteklemez ve constraint trigger'ı OR REPLACE ile değiştiremez.
        let old = TriggerDef { deferrable: true, ..ct() };
        let new = TriggerDef { initially_deferred: true, ..old.clone() };
        let pl = alter_ok_v(&old, &new, 160000);
        assert_eq!(
            sqls(&pl),
            [
                "DROP TRIGGER tr ON public.t",
                "CREATE CONSTRAINT TRIGGER tr AFTER INSERT ON public.t DEFERRABLE INITIALLY DEFERRED FOR EACH ROW EXECUTE FUNCTION public.tf()",
            ]
        );
        assert_eq!(pl.statements[0].risk, Risk::Destructive);
        // constraint → sıradan ve sıradan → constraint de yeniden kurma.
        let plain = TriggerDef { timing: TriggerTiming::After, ..t() };
        for (a, b) in [(&ct(), &plain), (&plain, &ct())] {
            let pl = alter_ok_v(a, b, 160000);
            assert!(pl.statements[0].sql.starts_with("DROP TRIGGER tr ON public.t"), "{:?}", sqls(&pl));
            assert!(pl.statements[1].sql.starts_with("CREATE "), "{:?}", sqls(&pl));
            assert!(!pl.statements[1].sql.contains("OR REPLACE"));
        }
    }

    #[test]
    fn alter_combined_order() {
        let old = TriggerDef { comment: Some("c".into()), ..t() };
        let new = TriggerDef {
            name: "select".into(),
            timing: TriggerTiming::After,
            enabled: TriggerEnabled::Replica,
            comment: Some("d".into()),
            ..old.clone()
        };
        let pl = alter_ok_v(&old, &new, 160000);
        assert_eq!(
            sqls(&pl),
            [
                "ALTER TRIGGER tr ON public.t RENAME TO \"select\"",
                "CREATE OR REPLACE TRIGGER \"select\" AFTER INSERT ON public.t FOR EACH ROW EXECUTE FUNCTION public.tf()",
                "ALTER TABLE public.t ENABLE REPLICA TRIGGER \"select\"",
                "COMMENT ON TRIGGER \"select\" ON public.t IS 'd'",
            ]
        );
        insta::assert_snapshot!("alter_combined_order_pg16", render(&pl));
        let pl = alter_ok_v(&old, &new, 130000);
        assert_eq!(
            sqls(&pl),
            [
                "ALTER TRIGGER tr ON public.t RENAME TO \"select\"",
                "DROP TRIGGER \"select\" ON public.t",
                "CREATE TRIGGER \"select\" AFTER INSERT ON public.t FOR EACH ROW EXECUTE FUNCTION public.tf()",
                "ALTER TABLE public.t ENABLE REPLICA TRIGGER \"select\"",
                "COMMENT ON TRIGGER \"select\" ON public.t IS 'd'",
            ]
        );
        insta::assert_snapshot!("alter_combined_order_pg13", render(&pl));
        // Yalnız etkinlik ORIGIN'e dönüşü.
        let dis = TriggerDef { enabled: TriggerEnabled::Disabled, ..t() };
        assert_eq!(sqls(&alter_ok(&dis, &t())), ["ALTER TABLE public.t ENABLE TRIGGER tr"]);
    }

    #[test]
    fn table_cannot_change() {
        assert_eq!(alter_err(&t(), &TriggerDef { table: "u".into(), ..t() }), ["table"]);
        assert_eq!(alter_err(&t(), &TriggerDef { schema: "app".into(), ..t() }), ["schema"]);
    }

    #[test]
    fn read_only_rejects_changes_and_drop() {
        let ro = TriggerDef { editable: false, unsupported: vec!["iç trigger".into()], ..t() };
        assert!(alter_ok(&ro, &ro).statements.is_empty());
        for new in [
            TriggerDef { name: "x".into(), ..ro.clone() },
            TriggerDef { enabled: TriggerEnabled::Disabled, ..ro.clone() },
            TriggerDef { comment: Some("k".into()), ..ro.clone() },
            TriggerDef { timing: TriggerTiming::After, ..ro.clone() },
        ] {
            let e = balter(&o(&ro), &o(&new), &ctx()).unwrap_err();
            assert_eq!(paths(e.clone()), ["object"]);
            assert_eq!(e[0].message, READ_ONLY_MSG);
        }
        assert_eq!(paths(bdrop(&o(&ro), false, &ctx()).unwrap_err()), ["object"]);
        assert_eq!(paths(bdrop(&o(&ro), true, &ctx()).unwrap_err()), ["object"]);
    }

    #[test]
    fn drop_matrix() {
        let mut out = String::new();
        for (d, cascade, want, risk) in [
            (t(), false, "DROP TRIGGER tr ON public.t", Risk::Destructive),
            (t(), true, "DROP TRIGGER tr ON public.t CASCADE", Risk::DataLoss),
            (
                TriggerDef { schema: "Ş".into(), table: "My \"T\"".into(), name: "a b".into(), ..t() },
                false,
                "DROP TRIGGER \"a b\" ON \"Ş\".\"My \"\"T\"\"\"",
                Risk::Destructive,
            ),
        ] {
            let pl = bdrop(&o(&d), cascade, &ctx()).unwrap();
            assert_single_statements(&pl);
            assert_eq!(sqls(&pl), [want]);
            assert_eq!(pl.statements[0].risk, risk);
            assert_eq!(pl.warnings.is_empty(), !cascade);
            out.push_str(&render(&pl));
            out.push('\n');
        }
        insta::assert_snapshot!(out);
        // Tanım alanları doğrulanmaz (yalnız kimlik); sistem şeması reddedilir.
        assert!(bdrop(&o(&TriggerDef { events: ev(), when: Some(";".into()), ..t() }), false, &ctx()).is_ok());
        assert_eq!(paths(bdrop(&o(&TriggerDef { schema: "pg_catalog".into(), ..t() }), false, &ctx()).unwrap_err()), ["schema"]);
        assert_eq!(paths(bdrop(&o(&TriggerDef { table: String::new(), ..t() }), false, &ctx()).unwrap_err()), ["table"]);
    }

    #[test]
    fn hostile_args_and_when_stay_single_statement() {
        for a in ["'); DROP TABLE t; --", "\\'", "$$", "$pgt$", "/*", "--", "a\nb", "\r\n", "\"", "\u{202e}x", "E'x'"] {
            let d = TriggerDef { args: vec![a.into(), "ok".into()], ..t() };
            for pl in [create_ok_v(&d, 160000), alter_ok(&t(), &d), alter_ok_v(&t(), &d, 130000), create_ok_v(&d, 100000)] {
                assert!(!pl.statements.is_empty(), "{a:?}");
                assert!(pl.statements.iter().all(|s| composed_is_clean(&s.sql)), "{a:?}");
            }
        }
        for w in ["NEW.a = 'x; y'", "NEW.a > 0 -- son yorum
", "NEW.a = $$)$$", "/* ( */ true", "NEW.\"a;b\" IS NULL", "NEW.a = E'\\\\'"] {
            let d = TriggerDef { when: Some(w.into()), ..t() };
            for pl in [create_ok_v(&d, 160000), alter_ok(&t(), &d), alter_ok_v(&t(), &d, 130000)] {
                let s = pl.statements.iter().find(|s| s.sql.contains(" WHEN (")).unwrap();
                assert!(s.sql.contains(&format!(" WHEN ({w}\n) EXECUTE ")), "{w:?}: {}", s.sql);
            }
        }
    }

    #[test]
    fn hostile_names_stay_single_statement_or_fail_on_field() {
        let long63 = "a".repeat(63);
        for n in ["t\"; DROP TABLE x; --", "select", "Tablo ş", "日本", long63.as_str(), "'", "\\", "/*", "$$", "$pgt$"] {
            let d = TriggerDef {
                schema: n.into(),
                table: n.into(),
                name: n.into(),
                function_schema: n.into(),
                function_name: n.into(),
                comment: Some(n.into()),
                events: upd(&[n]),
                timing: TriggerTiming::After,
                ..t()
            };
            let refs = TriggerDef { events: upd(&[]), old_table: Some(n.into()), new_table: Some(format!("{}2", &n[..n.len().min(60)])), ..d.clone() };
            let new = TriggerDef { name: format!("{}x", &n[..n.len().min(60)]), level: TriggerLevel::Statement, enabled: TriggerEnabled::Disabled, ..d.clone() };
            for pl in [
                create_ok_v(&d, 160000),
                create_ok_v(&refs, 160000),
                alter_ok_v(&d, &new, 160000),
                alter_ok_v(&d, &new, 130000),
                alter_ok_v(&TriggerDef { constraint: true, ..d.clone() }, &new, 160000),
                bdrop(&o(&d), true, &ctx()).unwrap(),
                bdrop(&o(&d), false, &ctx()).unwrap(),
            ] {
                assert!(!pl.statements.is_empty(), "{n:?}");
                assert_single_statements(&pl);
            }
        }
        let pl = create_ok_v(&TriggerDef { name: "t\"; DROP TABLE x; --".into(), ..t() }, 160000);
        assert!(pl.statements[0].sql.starts_with("CREATE TRIGGER \"t\"\"; DROP TABLE x; --\" BEFORE"));
    }

    #[test]
    fn alter_validates_only_changed_or_written_fields() {
        // Eski taraf kimliği gevşek: sıfır genişlikli ad yeniden adlandırılabilir; yeni ad katı.
        let zw = TriggerDef { name: "t\u{200b}r".into(), ..t() };
        assert_eq!(sqls(&alter_ok(&zw, &TriggerDef { name: "tr".into(), ..zw.clone() })), ["ALTER TRIGGER \"t\u{200b}r\" ON public.t RENAME TO tr"]);
        assert_eq!(alter_err(&t(), &zw), ["name"]);
        assert_eq!(alter_err(&TriggerDef { name: String::new(), ..t() }, &t()), ["old.name"]);
        assert_eq!(alter_err(&TriggerDef { table: String::new(), ..t() }, &t()), ["old.table"]);
        assert_eq!(alter_err(&TriggerDef { schema: "pg_catalog".into(), ..t() }, &t()), ["old.schema"]);
        // Etkinlik/ad değişimi tanımı doğrulamaz (yüklü WHEN serbest SQL kapısına takılsa bile).
        let odd = TriggerDef { when: Some("NEW.a = 'a\\b'".into()), function_name: "f\u{200b}".into(), ..t() };
        assert_eq!(sqls(&alter_ok(&odd, &TriggerDef { enabled: TriggerEnabled::Disabled, ..odd.clone() })), ["ALTER TABLE public.t DISABLE TRIGGER tr"]);
        assert!(balter(&o(&odd), &o(&TriggerDef { name: "x".into(), ..odd.clone() }), &ctx()).is_ok());
        // Yapı değişince yeniden yazılan WHEN doğrulanır; değişmeyen yüklü fonksiyon adı gevşek.
        let odd_fn = TriggerDef { when: None, ..odd.clone() };
        assert!(balter(&o(&odd_fn), &o(&TriggerDef { args: vec!["a".into()], ..odd_fn.clone() }), &ctx()).is_ok());
        assert_eq!(alter_err(&odd, &TriggerDef { args: vec!["a".into()], ..odd.clone() }), ["when"]);
        assert_eq!(alter_err(&t(), &TriggerDef { function_name: "f\u{200b}".into(), ..t() }), ["functionName"]);
        // Değişmeyen NUL'lu yorum: yalnızca yeniden yazıldığında (PG < 14 yeniden kurma) hata.
        let nul = TriggerDef { comment: Some("a\0".into()), ..t() };
        assert!(balter(&o(&nul), &o(&TriggerDef { name: "x".into(), ..nul.clone() }), &ctx()).is_ok());
        assert!(balter(&o(&nul), &o(&TriggerDef { timing: TriggerTiming::After, ..nul.clone() }), &ctx()).is_ok());
        assert_eq!(
            paths(balter(&o(&nul), &o(&TriggerDef { timing: TriggerTiming::After, ..nul.clone() }), &ctx_v(130000)).unwrap_err()),
            ["comment"]
        );
        // Yeni tarafta kural ihlali alter'da da yakalanır.
        assert_eq!(alter_err(&t(), &TriggerDef { timing: TriggerTiming::InsteadOf, ..t() }), ["timing"]);
    }

    #[test]
    fn alter_noop_is_empty() {
        for d in [
            t(),
            TriggerDef { deferrable: true, initially_deferred: true, comment: Some("c".into()), enabled: TriggerEnabled::Disabled, ..ct() },
            TriggerDef { timing: TriggerTiming::After, events: upd(&[]), old_table: Some("o".into()), new_table: Some("n".into()), args: vec!["a".into()], when: Some("true".into()), ..t() },
        ] {
            for v in [100000, 130000, 160000] {
                let pl = alter_ok_v(&d, &d, v);
                assert!(pl.statements.is_empty() && pl.warnings.is_empty());
            }
        }
    }
}
