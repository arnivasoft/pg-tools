//! Veri düzenleyici değişiklik planlayıcı (saf): istemcinin `DataChangeSet`'ini doğrular ve
//! `INSERT`/`UPDATE`/`DELETE` ifadelerine çevirir. Biçimler:
//!
//! - `INSERT INTO "s"."t" ("c1", …) VALUES (<v1>, …) RETURNING <dönüş>` (hiç kolon yoksa `DEFAULT VALUES`)
//! - `UPDATE "s"."t" SET "c1" = <v1>, … WHERE "k1" = '<a>' AND … AND xmin::pg_catalog.text = '<sürüm>' RETURNING <dönüş>`
//! - `DELETE FROM "s"."t" WHERE "k1" = '<a>' AND … AND xmin::pg_catalog.text = '<sürüm>'`
//!
//! `<dönüş>` = `returning_clause` (kaynak kolon sırası + `xmin::pg_catalog.text`; şema yolundan bağımsız tip adı).
//! Kolon listeleri/atamalar **kaynak kolon sırasıyla** yazılır; ifadeler istemcinin değişiklik sırasını korur.
//! Hata iletileri değer, anahtar, sürüm ya da kolon adı içermez.

use std::collections::HashSet;

use crate::build::{err, stmt};
use crate::data::model::{
    CellEdit, CellValue, DataChangeSet, DataColumn, DataKey, DataSource, Identity, ReadOnlyReason, RowChange,
};
use crate::model::FieldError;
use crate::plan::{Plan, Risk};
use crate::quote::{qualified, quote_ident, quote_literal};

pub const MAX_CHANGES: usize = 2000;
pub const MAX_VALUE_BYTES: usize = 16 * 1024 * 1024;
pub const MAX_TOTAL_BYTES: usize = 64 * 1024 * 1024;
/// `xmin` (32 bit) metni en çok 10 hanedir; 20 hane 64 bit sürümlere de yer bırakır.
pub const MAX_VERSION_DIGITS: usize = 20;

/// Sürüm karşılaştırma ifadesi (tip adı nitelikli: `search_path`'ten bağımsız).
const VERSION_EXPR: &str = "xmin::pg_catalog.text";

pub const DELETE_NOTE: &str = "Satır silinir (geri alınamaz).";
pub const KEY_CHANGE_WARNING: &str = "Anahtar kolonu değiştiriliyor; bu satırı gösteren yabancı anahtarlar etkilenebilir.";
pub const SAME_ROW_WARNING: &str = "Aynı satır için birden çok değişiklik var; ikincisi çakışma verebilir (satır sürümü değişir).";

/// Bayt sınırları (testler küçük sınırla çağırır; bellek şişirmemek için).
#[derive(Debug, Clone, Copy)]
struct Limits {
    value: usize,
    total: usize,
}

const LIMITS: Limits = Limits { value: MAX_VALUE_BYTES, total: MAX_TOTAL_BYTES };

/// Saf. `source` güncel yüklenen kaynak; `source.fingerprint == set.fingerprint` denetimi YÖNETİCİDEDİR.
/// Alan hatası yolları: `changes` (sayı/toplam boyut), `changes[i]` (genel), `changes[i].values[j]`/`changes[i].set[j]`
/// (hücre), `changes[i].key` (anahtar uzunluğu), `changes[i].key[j]`, `changes[i].version`, `object` (salt okunur kaynak).
/// Boş `changes` → boş plan yalnızca düzenlenebilir kaynakta; salt okunur kaynak yine `object` hatası verir.
pub fn plan_changes(source: &DataSource, set: &DataChangeSet) -> std::result::Result<Plan, Vec<FieldError>> {
    plan_with(source, &set.changes, LIMITS)
}

/// Her ifadenin `RETURNING` listesi: kaynak kolonları (sırasıyla, `quote_ident`) + `xmin::pg_catalog.text`.
pub fn returning_clause(source: &DataSource) -> String {
    let mut items: Vec<String> = source.columns.iter().map(|c| quote_ident(&c.name)).collect();
    items.push(VERSION_EXPR.to_string());
    items.join(", ")
}

fn read_only_text(reason: Option<ReadOnlyReason>) -> &'static str {
    match reason {
        Some(ReadOnlyReason::NoKey) => "birincil anahtar ya da NOT NULL kolonlu benzersiz indeks yok.",
        Some(ReadOnlyReason::NoPrivilege) => "satır ekleme, güncelleme ya da silme yetkiniz yok.",
        Some(ReadOnlyReason::SystemObject) => "sistem nesnesi.",
        Some(ReadOnlyReason::UnsupportedKind) => "yalnızca tablolar düzenlenebilir.",
        None => "kaynak salt okunur.",
    }
}

fn read_only(reason: Option<ReadOnlyReason>) -> FieldError {
    err("object", format!("Bu nesne düzenlenemez: {}", read_only_text(reason)))
}

fn plan_with(source: &DataSource, changes: &[RowChange], lim: Limits) -> std::result::Result<Plan, Vec<FieldError>> {
    // `editable` anahtarı gerektirir; anahtarsız kaynak savunma amaçlı yine salt okunur sayılır.
    let key = match (&source.key, source.editable) {
        (Some(k), true) => k,
        (None, true) => return Err(vec![read_only(Some(ReadOnlyReason::NoKey))]),
        (_, false) => return Err(vec![read_only(source.read_only_reason)]),
    };
    if changes.is_empty() {
        return Ok(Plan::default());
    }
    if changes.len() > MAX_CHANGES {
        return Err(vec![err("changes", format!("Bir uygulamada en çok {MAX_CHANGES} değişiklik olabilir."))]);
    }

    let mut errs = Vec::new();
    let mut total = 0usize;
    for (i, ch) in changes.iter().enumerate() {
        validate_change(source, key, i, ch, lim, &mut total, &mut errs);
    }
    if total > lim.total {
        errs.push(err("changes", format!("Değerlerin toplam boyutu en çok {} MiB olabilir.", MAX_TOTAL_BYTES >> 20)));
    }
    if !errs.is_empty() {
        return Err(errs);
    }

    let table = qualified(&source.schema, &source.name);
    let returning = returning_clause(source);
    let mut statements = Vec::with_capacity(changes.len());
    let mut key_change = false;
    let mut same_row = false;
    let mut seen: HashSet<&[String]> = HashSet::new();
    for ch in changes {
        match ch {
            RowChange::Insert { values } => {
                let cells = ordered(source, values);
                let sql = if cells.is_empty() {
                    format!("INSERT INTO {table} DEFAULT VALUES RETURNING {returning}")
                } else {
                    let names: Vec<String> = cells.iter().map(|(c, _)| quote_ident(&c.name)).collect();
                    let vals: Vec<String> = cells.iter().map(|(_, v)| value_sql(v)).collect();
                    format!("INSERT INTO {table} ({}) VALUES ({}) RETURNING {returning}", names.join(", "), vals.join(", "))
                };
                statements.push(stmt(sql, Risk::Safe, None));
            }
            RowChange::Update { key: kv, version, set } => {
                key_change |= set.iter().any(|c| key.columns.contains(&c.column));
                same_row |= !seen.insert(kv.as_slice());
                let assigns: Vec<String> =
                    ordered(source, set).iter().map(|(c, v)| format!("{} = {}", quote_ident(&c.name), value_sql(v))).collect();
                let sql = format!(
                    "UPDATE {table} SET {} WHERE {} RETURNING {returning}",
                    assigns.join(", "),
                    where_sql(key, kv, version)
                );
                statements.push(stmt(sql, Risk::Safe, None));
            }
            RowChange::Delete { key: kv, version } => {
                same_row |= !seen.insert(kv.as_slice());
                let sql = format!("DELETE FROM {table} WHERE {}", where_sql(key, kv, version));
                statements.push(stmt(sql, Risk::Destructive, Some(DELETE_NOTE)));
            }
        }
    }
    let mut warnings = Vec::new();
    if key_change {
        warnings.push(KEY_CHANGE_WARNING.to_string());
    }
    if same_row {
        warnings.push(SAME_ROW_WARNING.to_string());
    }
    Ok(Plan { statements, warnings })
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Op {
    Insert,
    Update,
}

/// Tek değişikliğin doğrulaması. Hata sırası: genel (`changes[i]`), anahtar, sürüm, hücreler.
fn validate_change(
    source: &DataSource,
    key: &DataKey,
    i: usize,
    ch: &RowChange,
    lim: Limits,
    total: &mut usize,
    errs: &mut Vec<FieldError>,
) {
    let general = format!("changes[{i}]");
    match ch {
        RowChange::Insert { values } => {
            if !source.can_insert {
                errs.push(err(&general, "Bu tabloya satır ekleme yetkiniz yok."));
            }
            check_cells(source, (i, "values"), values, Op::Insert, lim, total, errs);
        }
        RowChange::Update { key: kv, version, set } => {
            if !source.can_update {
                errs.push(err(&general, "Bu tabloda satır güncelleme yetkiniz yok."));
            }
            if set.is_empty() {
                errs.push(err(&general, "Güncellenecek kolon yok."));
            }
            check_key(key, i, kv, version, lim, total, errs);
            check_cells(source, (i, "set"), set, Op::Update, lim, total, errs);
        }
        RowChange::Delete { key: kv, version } => {
            if !source.can_delete {
                errs.push(err(&general, "Bu tablodan satır silme yetkiniz yok."));
            }
            check_key(key, i, kv, version, lim, total, errs);
        }
    }
}

/// Anahtar değerleri (`DataKey.columns` sayısı kadar, NUL'suz, boyut sınırlı) ve sürüm (`^[0-9]{1,20}$`).
fn check_key(
    key: &DataKey,
    i: usize,
    kv: &[String],
    version: &str,
    lim: Limits,
    total: &mut usize,
    errs: &mut Vec<FieldError>,
) {
    if kv.len() != key.columns.len() {
        errs.push(err(&format!("changes[{i}].key"), "Anahtar değeri sayısı tablonun anahtar kolon sayısıyla uyuşmuyor."));
    } else {
        for (j, v) in kv.iter().enumerate() {
            if let Some(m) = text_error(v, lim, total) {
                errs.push(err(&format!("changes[{i}].key[{j}]"), m));
            }
        }
    }
    if version.is_empty() || version.len() > MAX_VERSION_DIGITS || !version.bytes().all(|b| b.is_ascii_digit()) {
        errs.push(err(&format!("changes[{i}].version"), "Satır sürümü geçersiz; veriyi yeniden yükleyin."));
    }
}

/// Hücre başına en çok bir hata: bilinmeyen/yinelenen kolon > kolon kuralı > değer kuralı. `at` = (değişiklik
/// sırası, alan adı).
fn check_cells(
    source: &DataSource,
    at: (usize, &str),
    cells: &[CellEdit],
    op: Op,
    lim: Limits,
    total: &mut usize,
    errs: &mut Vec<FieldError>,
) {
    let (i, field) = at;
    for (j, c) in cells.iter().enumerate() {
        let path = format!("changes[{i}].{field}[{j}]");
        let Some(col) = source.columns.iter().find(|k| k.name == c.column) else {
            errs.push(err(&path, "Kolon kaynakta yok."));
            continue;
        };
        if cells[..j].iter().any(|p| p.column == c.column) {
            errs.push(err(&path, "Bir kolon aynı değişiklikte yalnızca bir kez kullanılabilir."));
            continue;
        }
        let rule = column_error(col, &c.value, op).map(str::to_string);
        let value = match &c.value {
            CellValue::Text { value } => text_error(value, lim, total),
            CellValue::Null | CellValue::Default => None,
        };
        if let Some(m) = rule.or(value) {
            errs.push(err(&path, m));
        }
    }
}

fn column_error(col: &DataColumn, value: &CellValue, op: Op) -> Option<&'static str> {
    if col.generated {
        return Some("Üretilmiş (GENERATED) kolona değer yazılamaz.");
    }
    match op {
        Op::Insert if col.identity == Identity::Always && *value != CellValue::Default => {
            Some("GENERATED ALWAYS kimlik kolonuna yalnızca varsayılan (DEFAULT) yazılabilir.")
        }
        Op::Insert if !col.insertable => Some("Bu kolona değer ekleme yetkiniz yok."),
        Op::Insert => None,
        Op::Update if col.identity == Identity::Always => Some("GENERATED ALWAYS kimlik kolonu güncellenemez."),
        Op::Update if !col.updatable => Some("Bu kolonu güncelleme yetkiniz yok."),
        Op::Update => None,
    }
}

/// Metin değeri (hücre ya da anahtar): NUL yok, ≤ `lim.value` bayt; boyutu toplama eklenir.
fn text_error(v: &str, lim: Limits, total: &mut usize) -> Option<String> {
    *total = total.saturating_add(v.len());
    if v.contains('\0') {
        Some("Değer NUL karakteri içeremez.".into())
    } else if v.len() > lim.value {
        Some(format!("Değer en çok {} MiB olabilir.", MAX_VALUE_BYTES >> 20))
    } else {
        None
    }
}

/// Hücreleri kaynak kolon sırasına dizer (doğrulanmış girdi: her kolon kaynakta ve tekil).
fn ordered<'a>(source: &'a DataSource, cells: &'a [CellEdit]) -> Vec<(&'a DataColumn, &'a CellValue)> {
    source.columns.iter().filter_map(|c| cells.iter().find(|e| e.column == c.name).map(|e| (c, &e.value))).collect()
}

fn value_sql(v: &CellValue) -> String {
    match v {
        CellValue::Null => "NULL".into(),
        CellValue::Default => "DEFAULT".into(),
        CellValue::Text { value } => quote_literal(value),
    }
}

/// `"k1" = '<a>' AND … AND xmin::pg_catalog.text = '<sürüm>'` (anahtar sabitleri tipsiz: PG kolon tipine çevirir).
fn where_sql(key: &DataKey, kv: &[String], version: &str) -> String {
    let mut parts: Vec<String> =
        key.columns.iter().zip(kv).map(|(c, v)| format!("{} = {}", quote_ident(c), quote_literal(v))).collect();
    parts.push(format!("{VERSION_EXPR} = {}", quote_literal(version)));
    parts.join(" AND ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::build::{assert_single_statements, render, scan_semicolons};
    use crate::data::model::{KeySource, RelKind};
    use crate::load::ObjectRef;
    use crate::model::ObjectKind;
    use crate::plan::plan_hash;

    fn col(attnum: i16, name: &str) -> DataColumn {
        DataColumn {
            attnum,
            name: name.into(),
            type_oid: 25,
            type_display: "text".into(),
            not_null: false,
            has_default: false,
            identity: Identity::None,
            generated: false,
            insertable: true,
            updatable: true,
        }
    }

    fn source_with(schema: &str, name: &str, cols: Vec<DataColumn>, key: &[&str]) -> DataSource {
        DataSource {
            object: ObjectRef { kind: ObjectKind::Table, oid: 1 },
            schema: schema.into(),
            name: name.into(),
            relkind: RelKind::Table,
            columns: cols,
            key: Some(DataKey {
                source: KeySource::PrimaryKey,
                name: "pk".into(),
                columns: key.iter().map(|c| c.to_string()).collect(),
            }),
            can_select: true,
            can_insert: true,
            can_update: true,
            can_delete: true,
            locked: false,
            editable: true,
            read_only_reason: None,
            fingerprint: "0000000000000000".into(),
        }
    }

    /// `public.t (id, name, status, note)`, anahtar `id`.
    fn table() -> DataSource {
        let cols = ["id", "name", "status", "note"].iter().enumerate().map(|(i, c)| col(i as i16 + 1, c)).collect();
        source_with("public", "t", cols, &["id"])
    }

    /// `"App"."Order Lines" (order_id, line_no, qty)`, anahtar `(order_id, line_no)`.
    fn multi() -> DataSource {
        let cols = ["order_id", "line_no", "qty"].iter().enumerate().map(|(i, c)| col(i as i16 + 1, c)).collect();
        source_with("App", "Order Lines", cols, &["order_id", "line_no"])
    }

    /// Özel kolonlar: `id` (ALWAYS kimlik), `bd` (BY DEFAULT kimlik), `gen` (üretilmiş), `ro` (yetkisiz), `v`.
    fn special() -> DataSource {
        let mut id = col(1, "id");
        id.identity = Identity::Always;
        id.updatable = false;
        let mut bd = col(2, "bd");
        bd.identity = Identity::ByDefault;
        let mut gen_ = col(3, "gen");
        gen_.generated = true;
        gen_.insertable = false;
        gen_.updatable = false;
        let mut ro = col(4, "ro");
        ro.insertable = false;
        ro.updatable = false;
        source_with("public", "s", vec![id, bd, gen_, ro, col(5, "v")], &["id"])
    }

    fn t(v: &str) -> CellValue {
        CellValue::Text { value: v.into() }
    }

    fn e(c: &str, v: CellValue) -> CellEdit {
        CellEdit { column: c.into(), value: v }
    }

    fn ins(values: Vec<CellEdit>) -> RowChange {
        RowChange::Insert { values }
    }

    fn upd(key: &[&str], version: &str, set: Vec<CellEdit>) -> RowChange {
        RowChange::Update { key: key.iter().map(|k| k.to_string()).collect(), version: version.into(), set }
    }

    fn del(key: &[&str], version: &str) -> RowChange {
        RowChange::Delete { key: key.iter().map(|k| k.to_string()).collect(), version: version.into() }
    }

    fn set_of(changes: Vec<RowChange>) -> DataChangeSet {
        DataChangeSet { object: ObjectRef { kind: ObjectKind::Table, oid: 1 }, fingerprint: "0000000000000000".into(), changes }
    }

    fn ok(s: &DataSource, changes: Vec<RowChange>) -> Plan {
        let n = changes.len();
        let p = plan_changes(s, &set_of(changes)).unwrap_or_else(|e| panic!("hata beklenmiyordu: {e:?}"));
        assert_eq!(p.statements.len(), n, "değişiklik başına tek ifade");
        assert_single_statements(&p);
        assert!(p.statements.iter().all(|s| s.transactional && !s.redacted));
        p
    }

    fn errs(s: &DataSource, changes: Vec<RowChange>) -> Vec<FieldError> {
        plan_changes(s, &set_of(changes)).expect_err("hata bekleniyordu")
    }

    fn paths(e: &[FieldError]) -> Vec<&str> {
        e.iter().map(|f| f.path.as_str()).collect()
    }

    fn sqls(p: &Plan) -> Vec<&str> {
        p.statements.iter().map(|s| s.sql.as_str()).collect()
    }

    fn render_cases(cases: &[(&str, Plan)]) -> String {
        let mut out = String::new();
        for (label, p) in cases {
            out.push_str(&format!("-- {label}\n{}\n", render(p)));
        }
        out
    }

    #[test]
    fn empty_changes_is_empty_plan() {
        assert_eq!(plan_changes(&table(), &set_of(vec![])).unwrap(), Plan::default());
    }

    #[test]
    fn returning_clause_quotes_names_and_appends_version() {
        assert_eq!(returning_clause(&table()), "id, \"name\", status, note, xmin::pg_catalog.text");
        let hostile = source_with(
            "public",
            "h",
            ["a\"b", "x'); DROP TABLE t; --", "user", "ç"].iter().enumerate().map(|(i, c)| col(i as i16 + 1, c)).collect(),
            &["user"],
        );
        let r = returning_clause(&hostile);
        assert_eq!(r, "\"a\"\"b\", \"x'); DROP TABLE t; --\", \"user\", \"ç\", xmin::pg_catalog.text");
        assert!(scan_semicolons(&format!("SELECT {r} FROM x")).is_clean());
        let empty = source_with("public", "e", vec![], &[]);
        assert_eq!(returning_clause(&empty), "xmin::pg_catalog.text");
    }

    #[test]
    fn exact_statement_shapes() {
        let s = table();
        let p = ok(&s, vec![ins(vec![e("status", t("a")), e("id", t("1"))])]);
        assert_eq!(
            sqls(&p),
            ["INSERT INTO public.t (id, status) VALUES ('1', 'a') RETURNING id, \"name\", status, note, xmin::pg_catalog.text"]
        );
        let p = ok(&s, vec![upd(&["7"], "123", vec![e("note", CellValue::Null), e("name", t("x"))])]);
        assert_eq!(
            sqls(&p),
            ["UPDATE public.t SET \"name\" = 'x', note = NULL WHERE id = '7' AND xmin::pg_catalog.text = '123' \
              RETURNING id, \"name\", status, note, xmin::pg_catalog.text"]
        );
        let p = ok(&s, vec![del(&["7"], "123")]);
        assert_eq!(sqls(&p), ["DELETE FROM public.t WHERE id = '7' AND xmin::pg_catalog.text = '123'"]);
        assert_eq!(p.statements[0].note.as_deref(), Some(DELETE_NOTE));
        let p = ok(&s, vec![ins(vec![])]);
        assert_eq!(
            sqls(&p),
            ["INSERT INTO public.t DEFAULT VALUES RETURNING id, \"name\", status, note, xmin::pg_catalog.text"]
        );
    }

    #[test]
    fn plan_variants() {
        let s = table();
        let m = multi();
        let cases = vec![
            (
                "tek Insert (karışık Text/Null/Default; kaynak sırası)",
                ok(&s, vec![ins(vec![e("note", CellValue::Default), e("name", t("Ayşe")), e("status", CellValue::Null), e("id", t("1"))])]),
            ),
            ("DEFAULT VALUES", ok(&s, vec![ins(vec![])])),
            ("Update iki kolon", ok(&s, vec![upd(&["1"], "736", vec![e("status", t("b")), e("name", t("Ali"))])])),
            (
                "Update DEFAULT/NULL (kaynak sırası)",
                ok(&s, vec![upd(&["1"], "736", vec![e("note", CellValue::Default), e("status", CellValue::Null)])]),
            ),
            ("çok kolonlu anahtarlı Update", ok(&m, vec![upd(&["10", "2"], "99", vec![e("qty", t("5"))])])),
            ("Delete", ok(&s, vec![del(&["1"], "736")])),
            ("çok kolonlu anahtarlı Delete", ok(&m, vec![del(&["10", "2"], "99")])),
            (
                "karışık sıralı küme (istemci sırası korunur)",
                ok(
                    &s,
                    vec![
                        del(&["3"], "10"),
                        ins(vec![e("id", t("4"))]),
                        upd(&["2"], "11", vec![e("note", t("n"))]),
                        ins(vec![]),
                    ],
                ),
            ),
            ("anahtar kolonu güncelleme", ok(&s, vec![upd(&["1"], "5", vec![e("id", t("100")), e("name", t("x"))])])),
            (
                "aynı satır iki kez",
                ok(&s, vec![upd(&["1"], "5", vec![e("name", t("a"))]), upd(&["1"], "5", vec![e("status", t("b"))])]),
            ),
            ("aynı satır Update + Delete", ok(&s, vec![upd(&["1"], "5", vec![e("name", t("a"))]), del(&["1"], "5")])),
        ];
        insta::assert_snapshot!(render_cases(&cases));
    }

    #[test]
    fn warnings_are_emitted_once() {
        let s = table();
        let p = ok(&s, vec![upd(&["1"], "5", vec![e("id", t("2"))]), upd(&["3"], "5", vec![e("id", t("4"))])]);
        assert_eq!(p.warnings, [KEY_CHANGE_WARNING]);
        let p = ok(&s, vec![del(&["1"], "5"), upd(&["1"], "5", vec![e("name", t("a"))]), del(&["1"], "5")]);
        assert_eq!(p.warnings, [SAME_ROW_WARNING]);
        // Farklı anahtarlar ve Insert'ler uyarı vermez; çok kolonlu anahtarda kısmi eşitlik aynı satır değildir.
        let p = ok(&s, vec![upd(&["1"], "5", vec![e("name", t("a"))]), del(&["2"], "5"), ins(vec![e("id", t("1"))])]);
        assert!(p.warnings.is_empty());
        let p = ok(&multi(), vec![del(&["1", "2"], "5"), del(&["1", "3"], "5")]);
        assert!(p.warnings.is_empty());
        let p = ok(&multi(), vec![upd(&["1", "2"], "5", vec![e("line_no", t("9"))]), del(&["1", "2"], "6")]);
        assert_eq!(p.warnings, [KEY_CHANGE_WARNING, SAME_ROW_WARNING]);
    }

    const HOSTILE: [&str; 14] = [
        "it's",
        "a\\b",
        "x'; DROP TABLE t; --",
        "--",
        ";",
        "/*",
        "çğüşöı",
        "😀 emoji",
        "\u{202E}bidi\u{202C}",
        "satır1\nsatır2\r\n\tsatır3",
        "",
        "NULL",
        "DEFAULT",
        "$$ $q$ \\' '' \"",
    ];

    #[test]
    fn hostile_values_stay_one_statement() {
        let s = table();
        let mut cases = Vec::new();
        for (i, v) in HOSTILE.iter().enumerate() {
            let p = ok(
                &s,
                vec![
                    ins(vec![e("id", t(v)), e("name", t(v))]),
                    upd(&[v], "1", vec![e("name", t(v))]),
                    del(&[v], "1"),
                ],
            );
            for st in &p.statements {
                assert!(scan_semicolons(&st.sql).is_clean(), "{}", st.sql);
            }
            cases.push((format!("değer {i}"), p));
        }
        let labelled: Vec<(&str, Plan)> = cases.iter().map(|(l, p)| (l.as_str(), p.clone())).collect();
        insta::assert_snapshot!(render_cases(&labelled));

        // insta satır sonlarını normalleştirir; çok satırlı değerin `\r\n\t` baytları ifadede aynen kalmalı.
        let p = ok(&s, vec![ins(vec![e("name", t(HOSTILE[9]))])]);
        assert!(p.statements[0].sql.contains("VALUES ('satır1\nsatır2\r\n\tsatır3')"), "{:?}", p.statements[0].sql);
        // Boş dize NULL değildir; `NULL`/`DEFAULT` metinleri tırnaklı kalır; `\` → `E'…'`.
        let p = ok(&s, vec![ins(vec![e("id", t("")), e("name", t("NULL")), e("status", t("DEFAULT")), e("note", CellValue::Null)])]);
        assert!(p.statements[0].sql.contains("VALUES ('', 'NULL', 'DEFAULT', NULL)"), "{}", p.statements[0].sql);
        let p = ok(&s, vec![upd(&["a\\b"], "1", vec![e("name", t("c\\d"))])]);
        assert!(p.statements[0].sql.contains("SET \"name\" = E'c\\\\d' WHERE id = E'a\\\\b'"), "{}", p.statements[0].sql);
    }

    #[test]
    fn hostile_names_are_quoted() {
        let cols = ["a\"b", "x'); DROP TABLE t; --", "user", "a/*", "a\\b"].iter().enumerate().map(|(i, c)| col(i as i16 + 1, c)).collect();
        let s = source_with("my \"schema\"", "t;DROP", cols, &["user", "a\"b"]);
        let p = ok(
            &s,
            vec![
                ins(vec![e("x'); DROP TABLE t; --", t("v")), e("a/*", t("w"))]),
                upd(&["u", "k"], "9", vec![e("a\\b", t("z"))]),
                del(&["u", "k"], "9"),
            ],
        );
        insta::assert_snapshot!(render(&p));
    }

    #[test]
    fn value_size_limits_use_parameterised_limits() {
        let s = table();
        let lim = Limits { value: 8, total: 20 };
        let run = |changes: Vec<RowChange>| plan_with(&s, &changes, lim);
        // Sınırda geçer, +1 bayt hata (çok baytlı karakter bayt olarak sayılır).
        assert!(run(vec![ins(vec![e("name", t("12345678"))])]).is_ok());
        assert_eq!(paths(&run(vec![ins(vec![e("name", t("123456789"))])]).unwrap_err()), ["changes[0].values[0]"]);
        assert_eq!(paths(&run(vec![ins(vec![e("name", t("ççççç"))])]).unwrap_err()), ["changes[0].values[0]"]);
        assert_eq!(paths(&run(vec![upd(&["1"], "1", vec![e("name", t("123456789"))])]).unwrap_err()), ["changes[0].set[0]"]);
        // Anahtar değeri de aynı sınıra tabidir.
        assert_eq!(paths(&run(vec![del(&["123456789"], "1")]).unwrap_err()), ["changes[0].key[0]"]);
        // Toplam: 20 bayt geçer, 21 bayt `changes` hatası.
        let ten = |c: &str| e(c, t("12345"));
        assert!(run(vec![ins(vec![ten("name"), ten("note")]), ins(vec![ten("name"), ten("note")])]).is_ok());
        let e2 = run(vec![ins(vec![ten("name"), ten("note")]), ins(vec![ten("name"), ten("note"), e("status", t("x"))])]).unwrap_err();
        assert_eq!(paths(&e2), ["changes"]);
        // Gerçek sınırlar sabitlerle aynıdır.
        assert_eq!((LIMITS.value, LIMITS.total), (16 * 1024 * 1024, 64 * 1024 * 1024));
    }

    #[test]
    fn nul_values_and_keys_are_rejected() {
        let s = table();
        let e1 = errs(&s, vec![ins(vec![e("name", t("a\0b"))]), upd(&["1\0"], "1", vec![e("note", t("\0"))]), del(&["\0"], "1")]);
        assert_eq!(paths(&e1), ["changes[0].values[0]", "changes[1].key[0]", "changes[1].set[0]", "changes[2].key[0]"]);
        for f in &e1 {
            assert!(!f.message.contains('\0'));
        }
    }

    #[test]
    fn rejected_changes_are_field_errors() {
        let s = table();
        let sp = special();
        let cases: Vec<(&DataSource, RowChange, &[&str])> = vec![
            (&s, ins(vec![e("gizli_kolon", t("v"))]), &["changes[0].values[0]"]),
            // Büyük/küçük harf duyarlı.
            (&s, ins(vec![e("ID", t("v"))]), &["changes[0].values[0]"]),
            (&s, ins(vec![e("name", t("a")), e("name", t("b"))]), &["changes[0].values[1]"]),
            (&s, upd(&["1"], "1", vec![e("name", t("a")), e("name", t("b"))]), &["changes[0].set[1]"]),
            (&sp, ins(vec![e("gen", t("1"))]), &["changes[0].values[0]"]),
            (&sp, ins(vec![e("gen", CellValue::Default)]), &["changes[0].values[0]"]),
            (&sp, ins(vec![e("id", t("1"))]), &["changes[0].values[0]"]),
            (&sp, ins(vec![e("id", CellValue::Null)]), &["changes[0].values[0]"]),
            (&sp, ins(vec![e("ro", t("1"))]), &["changes[0].values[0]"]),
            (&sp, upd(&["1"], "1", vec![e("id", t("2"))]), &["changes[0].set[0]"]),
            (&sp, upd(&["1"], "1", vec![e("gen", t("2"))]), &["changes[0].set[0]"]),
            (&sp, upd(&["1"], "1", vec![e("ro", t("2"))]), &["changes[0].set[0]"]),
            (&sp, upd(&["1"], "1", vec![e("id", CellValue::Default)]), &["changes[0].set[0]"]),
            (&sp, upd(&["1"], "1", vec![e("gen", CellValue::Default)]), &["changes[0].set[0]"]),
            (&sp, upd(&["1"], "1", vec![e("ro", CellValue::Default)]), &["changes[0].set[0]"]),
            (&s, upd(&["1"], "1", vec![]), &["changes[0]"]),
            (&s, upd(&["1", "2"], "1", vec![e("name", t("a"))]), &["changes[0].key"]),
            (&s, del(&[], "1"), &["changes[0].key"]),
            (&s, upd(&["1"], "12a", vec![e("name", t("a"))]), &["changes[0].version"]),
            (&s, upd(&["1"], "", vec![e("name", t("a"))]), &["changes[0].version"]),
            (&s, del(&["1"], &"1".repeat(21)), &["changes[0].version"]),
            (&s, del(&["1"], "-1"), &["changes[0].version"]),
            (&s, del(&["1"], " 1"), &["changes[0].version"]),
            (&s, del(&["1"], "١"), &["changes[0].version"]),
        ];
        for (src, ch, want) in cases {
            let dbg = format!("{ch:?}");
            let got = errs(src, vec![ch]);
            assert_eq!(paths(&got), want, "{dbg}");
            for f in &got {
                assert!(!f.message.contains("gizli_kolon") && !f.message.contains("12a"), "{}", f.message);
            }
        }
        // Sınırda: 20 hane geçer; ALWAYS kimliğe Default, BY DEFAULT kimliğe değer geçer.
        ok(&s, vec![del(&["1"], &"9".repeat(20))]);
        ok(&sp, vec![ins(vec![e("id", CellValue::Default), e("bd", t("5")), e("v", t("x"))])]);
        ok(&sp, vec![upd(&["1"], "1", vec![e("bd", t("6"))])]);
        // Sıradan (güncellenebilir) kolonda `SET c = DEFAULT` serbesttir.
        let p = ok(&s, vec![upd(&["1"], "1", vec![e("name", CellValue::Default)])]);
        assert!(p.statements[0].sql.contains("SET \"name\" = DEFAULT WHERE"), "{}", p.statements[0].sql);
        ok(&sp, vec![upd(&["1"], "1", vec![e("bd", CellValue::Default), e("v", CellValue::Default)])]);
    }

    #[test]
    fn errors_are_collected_in_one_pass() {
        let s = table();
        let got = errs(
            &s,
            vec![
                ins(vec![e("nope", t("v")), e("name", t("a\0"))]),
                upd(&["1"], "x", vec![]),
                del(&["1", "2"], "1"),
            ],
        );
        assert_eq!(
            paths(&got),
            ["changes[0].values[0]", "changes[0].values[1]", "changes[1]", "changes[1].version", "changes[2].key"]
        );
    }

    #[test]
    fn table_level_privileges_are_checked() {
        let mut s = table();
        s.can_insert = false;
        assert_eq!(paths(&errs(&s, vec![ins(vec![e("name", t("a"))])])), ["changes[0]"]);
        ok(&s, vec![upd(&["1"], "1", vec![e("name", t("a"))]), del(&["1"], "1")]);
        let mut s = table();
        s.can_update = false;
        assert_eq!(paths(&errs(&s, vec![upd(&["1"], "1", vec![e("name", t("a"))])])), ["changes[0]"]);
        let mut s = table();
        s.can_delete = false;
        assert_eq!(paths(&errs(&s, vec![ins(vec![]), del(&["1"], "1")])), ["changes[1]"]);
    }

    #[test]
    fn read_only_source_is_single_object_error() {
        for reason in [ReadOnlyReason::NoKey, ReadOnlyReason::NoPrivilege, ReadOnlyReason::SystemObject, ReadOnlyReason::UnsupportedKind] {
            let mut s = table();
            s.editable = false;
            s.read_only_reason = Some(reason);
            let got = errs(&s, vec![ins(vec![e("nope", t("v"))]), del(&[], "")]);
            assert_eq!(paths(&got), ["object"], "{reason:?}");
            assert!(got[0].message.starts_with("Bu nesne düzenlenemez: "), "{}", got[0].message);
            // Boş küme de salt okunur kaynakta hata verir.
            assert_eq!(paths(&errs(&s, vec![])), ["object"]);
        }
        let texts: HashSet<String> = [ReadOnlyReason::NoKey, ReadOnlyReason::NoPrivilege, ReadOnlyReason::SystemObject, ReadOnlyReason::UnsupportedKind]
            .into_iter()
            .map(|r| {
                let mut s = table();
                s.editable = false;
                s.read_only_reason = Some(r);
                errs(&s, vec![]).remove(0).message
            })
            .collect();
        assert_eq!(texts.len(), 4, "her nedenin ayrı metni olmalı");
        // `editable` görünse de anahtar yoksa salt okunurdur (savunma).
        let mut s = table();
        s.key = None;
        assert_eq!(paths(&errs(&s, vec![del(&["1"], "1")])), ["object"]);
    }

    #[test]
    fn too_many_changes_is_rejected() {
        let s = table();
        let many = |n: usize| (0..n).map(|_| ins(vec![])).collect::<Vec<_>>();
        ok(&s, many(MAX_CHANGES));
        assert_eq!(paths(&errs(&s, many(MAX_CHANGES + 1))), ["changes"]);
    }

    #[test]
    fn risk_and_hash() {
        let s = table();
        let safe = ok(&s, vec![ins(vec![e("name", t("a"))]), upd(&["1"], "1", vec![e("name", t("b"))])]);
        assert_eq!(safe.risk(), Risk::Safe);
        assert!(safe.statements.iter().all(|s| s.risk == Risk::Safe && s.note.is_none()));
        let destructive = ok(&s, vec![ins(vec![e("name", t("a"))]), del(&["1"], "1")]);
        assert_eq!(destructive.risk(), Risk::Destructive);
        assert_eq!(destructive.statements[1].risk, Risk::Destructive);

        let changes = || vec![ins(vec![e("name", t("a"))]), upd(&["1"], "1", vec![e("name", t("b"))])];
        let h1 = plan_hash(&ok(&s, changes()).statements);
        assert_eq!(h1, plan_hash(&ok(&s, changes()).statements));
        let mut other = changes();
        other[1] = upd(&["1"], "1", vec![e("name", t("c"))]);
        assert_ne!(h1, plan_hash(&ok(&s, other).statements));
        let mut other = changes();
        other[1] = upd(&["1"], "2", vec![e("name", t("b"))]);
        assert_ne!(h1, plan_hash(&ok(&s, other).statements));
    }
}
