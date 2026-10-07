//! Veri düzenleyici SELECT üreticisi (saf). Biçim:
//! `SELECT "c1", …, xmin::pg_catalog.text AS "__pgt_xmin" FROM "s"."t"[ WHERE (<filtre>\n)][ ORDER BY "k1" ASC, …]`.
//!
//! Kolon listesi kaynağın **tüm** kolonlarıdır (`attnum` sırası); sürüm kolonu (`xmin::pg_catalog.text`; tip adı
//! `search_path`'ten bağımsız) yalnızca
//! `Table`/`Partitioned`'da eklenir ve hep sondadır (`version_index = columns.len()`). Sıralama kolonları kaynak
//! kolonlarından seçilir; istek sıralaması boşsa anahtar kolonları `ASC` (anahtarsızda `ORDER BY` yok).
//!
//! Filtre tek serbest SQL alanıdır: ≤ `MAX_FILTER_BYTES`, NUL/denetim karakteri yok (`\t \n \r` hariç),
//! `check_free_sql` kapısı, `wrap` ile `(<filtre>\n)` sarma ve nihai ifadede `composed_is_clean`. Hata iletileri
//! filtre metnini ya da kolon adlarını içermez.

use crate::build::err;
use crate::build::freesql::{check_free_sql, composed_is_clean, wrap};
use crate::data::model::{DataSelect, DataSource, RelKind, SelectRequest};
use crate::model::FieldError;
use crate::quote::{qualified, quote_ident};

pub const MAX_FILTER_BYTES: usize = 4096;
pub const MAX_ORDER_KEYS: usize = 8;

/// Saf. `Err` alan hatalarıdır (yol: `filter`, `order`, `order[i].column`).
pub fn select_sql(source: &DataSource, req: &SelectRequest) -> std::result::Result<DataSelect, Vec<FieldError>> {
    let mut errs = Vec::new();
    let order = order_by(source, req, &mut errs);
    let filter = req.filter.as_deref().and_then(|f| check_filter(f, &mut errs));
    if !errs.is_empty() {
        return Err(errs);
    }

    let has_version = matches!(source.relkind, RelKind::Table | RelKind::Partitioned);
    let mut items: Vec<String> = source.columns.iter().map(|c| quote_ident(&c.name)).collect();
    if has_version {
        items.push(format!("xmin::pg_catalog.text AS {}", quote_ident(&version_alias(source))));
    }
    // Kolonsuz görünüm (`SELECT FROM v`) PG 9.4+'ta geçerlidir.
    let mut sql = if items.is_empty() { "SELECT".to_string() } else { format!("SELECT {}", items.join(", ")) };
    sql.push_str(" FROM ");
    sql.push_str(&qualified(&source.schema, &source.name));
    if let Some(f) = filter {
        sql.push_str(" WHERE ");
        sql.push_str(&wrap(f));
    }
    if !order.is_empty() {
        sql.push_str(" ORDER BY ");
        sql.push_str(&order.join(", "));
    }
    // Alan sınırında bölge/parantez kayması: kapı kabul etse de nihai ifade temiz olmalı. Filtresiz ifade yapısı
    // gereği temizdir (adlar `quote_ident`'li).
    if !composed_is_clean(&sql) {
        debug_assert!(req.filter.is_some(), "filtresiz SELECT temiz olmalı");
        return Err(vec![err("filter", "Filtre sorguya güvenle yerleştirilemedi; tek bir koşul ifadesi yazın.")]);
    }
    Ok(DataSelect { sql, version_index: source.columns.len() as u32, has_version })
}

/// Sıralama ifadeleri: istek boşsa anahtar kolonları `ASC`; değilse istek doğrulanır (kaynak kolonu, tekrarsız,
/// en çok `MAX_ORDER_KEYS`).
fn order_by(source: &DataSource, req: &SelectRequest, errs: &mut Vec<FieldError>) -> Vec<String> {
    if req.order.is_empty() {
        return source
            .key
            .as_ref()
            .map(|k| k.columns.iter().map(|c| format!("{} ASC", quote_ident(c))).collect())
            .unwrap_or_default();
    }
    if req.order.len() > MAX_ORDER_KEYS {
        errs.push(err("order", format!("En çok {MAX_ORDER_KEYS} sıralama kolonu seçilebilir.")));
    }
    let mut out = Vec::with_capacity(req.order.len());
    for (i, o) in req.order.iter().enumerate() {
        let path = format!("order[{i}].column");
        if !source.columns.iter().any(|c| c.name == o.column) {
            errs.push(err(&path, "Sıralama kolonu kaynakta yok."));
        } else if req.order[..i].iter().any(|p| p.column == o.column) {
            errs.push(err(&path, "Bir kolon sıralamada yalnızca bir kez kullanılabilir."));
        } else {
            out.push(format!("{} {}", quote_ident(&o.column), if o.desc { "DESC" } else { "ASC" }));
        }
    }
    out
}

/// Filtre kapısı. Boş/yalnız boşluk → `None` (yok sayılır); hatalıysa `filter` alan hatası ekler ve `None` döner.
fn check_filter<'a>(f: &'a str, errs: &mut Vec<FieldError>) -> Option<&'a str> {
    let before = errs.len();
    if f.len() > MAX_FILTER_BYTES {
        errs.push(err("filter", format!("Filtre en çok {MAX_FILTER_BYTES} bayt olabilir.")));
    } else if f.contains('\0') {
        errs.push(err("filter", "Filtre NUL karakteri içeremez."));
    } else if f.chars().any(|c| c.is_control() && !matches!(c, '\t' | '\n' | '\r')) {
        errs.push(err("filter", "Filtre denetim karakteri içeremez (sekme ve satır sonu dışında)."));
    } else if f.trim().is_empty() {
        return None;
    } else {
        check_free_sql("filter", f, errs);
    }
    (errs.len() == before).then_some(f)
}

/// Sürüm kolonu takma adı: kaynakta çakışmayan ilk ad (`__pgt_xmin`, `__pgt_xmin_1`, …).
pub fn version_alias(source: &DataSource) -> String {
    let taken = |n: &str| source.columns.iter().any(|c| c.name == n);
    let base = "__pgt_xmin";
    if !taken(base) {
        return base.to_string();
    }
    (1..).map(|i| format!("{base}_{i}")).find(|n| !taken(n)).expect("sonsuz aday")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::data::model::{DataColumn, DataKey, Identity, KeySource, OrderBy};
    use crate::load::ObjectRef;
    use crate::model::ObjectKind;

    fn col(attnum: i16, name: &str) -> DataColumn {
        DataColumn {
            attnum,
            name: name.into(),
            type_oid: 23,
            type_display: "integer".into(),
            not_null: false,
            has_default: false,
            identity: Identity::None,
            generated: false,
            insertable: true,
            updatable: true,
        }
    }

    fn source(relkind: RelKind, schema: &str, name: &str, cols: &[&str], key: Option<&[&str]>) -> DataSource {
        DataSource {
            object: ObjectRef { kind: ObjectKind::Table, oid: 1 },
            schema: schema.into(),
            name: name.into(),
            relkind,
            columns: cols.iter().enumerate().map(|(i, c)| col(i as i16 + 1, c)).collect(),
            key: key.map(|k| DataKey {
                source: KeySource::PrimaryKey,
                name: "pk".into(),
                columns: k.iter().map(|c| c.to_string()).collect(),
            }),
            can_select: true,
            can_insert: true,
            can_update: true,
            can_delete: true,
            locked: false,
            editable: key.is_some(),
            read_only_reason: None,
            fingerprint: "0000000000000000".into(),
        }
    }

    fn table(cols: &[&str], key: Option<&[&str]>) -> DataSource {
        source(RelKind::Table, "public", "t", cols, key)
    }

    fn req(order: &[(&str, bool)], filter: Option<&str>) -> SelectRequest {
        SelectRequest {
            order: order.iter().map(|(c, d)| OrderBy { column: c.to_string(), desc: *d }).collect(),
            filter: filter.map(str::to_string),
        }
    }

    fn ok(s: &DataSource, r: &SelectRequest) -> DataSelect {
        let out = select_sql(s, r).unwrap_or_else(|e| panic!("hata beklenmiyordu: {e:?}"));
        assert!(composed_is_clean(&out.sql), "{}", out.sql);
        assert_eq!(out.version_index as usize, s.columns.len());
        assert_eq!(out.has_version, matches!(s.relkind, RelKind::Table | RelKind::Partitioned));
        // Sürüm ifadesi tip adını nitelikli yazar (`search_path`'ten bağımsız).
        assert_eq!(out.sql.contains("xmin::pg_catalog.text AS "), out.has_version, "{}", out.sql);
        assert!(!out.sql.contains("xmin::text"), "{}", out.sql);
        out
    }

    fn errs(s: &DataSource, r: &SelectRequest) -> Vec<FieldError> {
        select_sql(s, r).expect_err("hata bekleniyordu")
    }

    fn paths(e: &[FieldError]) -> Vec<&str> {
        e.iter().map(|f| f.path.as_str()).collect()
    }

    fn render(cases: &[(&str, DataSelect)]) -> String {
        let mut out = String::new();
        for (label, sel) in cases {
            out.push_str(&format!("-- {label} (version_index={}, has_version={})\n{}\n\n", sel.version_index, sel.has_version, sel.sql));
        }
        out
    }

    #[test]
    fn select_variants() {
        let pk = table(&["id", "name", "status"], Some(&["id"]));
        let multi = source(RelKind::Table, "App", "Orders", &["a", "b", "c"], Some(&["c", "a"]));
        let cases = vec![
            ("PK'lı tablo varsayılan", ok(&pk, &req(&[], None))),
            ("çok kolonlu anahtar varsayılan sıralama", ok(&multi, &req(&[], None))),
            ("istek sıralaması (desc)", ok(&multi, &req(&[("b", true), ("a", false)], None))),
            ("filtreli", ok(&pk, &req(&[], Some("status = 'a'")))),
            ("anahtarsız tablo", ok(&table(&["x", "y"], None), &req(&[], None))),
            (
                "bölümlü tablo",
                ok(&source(RelKind::Partitioned, "public", "p", &["id", "k"], Some(&["id", "k"])), &req(&[], None)),
            ),
            ("görünüm", ok(&source(RelKind::View, "public", "v", &["id", "v"], None), &req(&[("v", true)], None))),
            ("MV", ok(&source(RelKind::MatView, "public", "m", &["id"], None), &req(&[], None))),
            ("yabancı tablo", ok(&source(RelKind::Foreign, "public", "f", &["id"], None), &req(&[], None))),
            ("boşluk filtre yok sayılır", ok(&pk, &req(&[], Some(" \n\t ")))),
            ("kolonsuz tablo", ok(&table(&[], None), &req(&[], None))),
            ("kolonsuz görünüm", ok(&source(RelKind::View, "public", "v0", &[], None), &req(&[], None))),
        ];
        insta::assert_snapshot!(render(&cases));
    }

    #[test]
    fn version_alias_avoids_collisions() {
        let s = table(&["id", "__pgt_xmin"], Some(&["id"]));
        assert_eq!(version_alias(&s), "__pgt_xmin_1");
        let s2 = table(&["id", "__pgt_xmin_1", "__pgt_xmin"], Some(&["id"]));
        assert_eq!(version_alias(&s2), "__pgt_xmin_2");
        assert_eq!(version_alias(&table(&["id"], None)), "__pgt_xmin");
        // Büyük/küçük harf farklı ad çakışma değildir (tırnaklı ad).
        assert_eq!(version_alias(&table(&["__PGT_XMIN"], None)), "__pgt_xmin");
        let cases = vec![
            ("__pgt_xmin çakışması", ok(&s, &req(&[], None))),
            ("__pgt_xmin ve _1 çakışması", ok(&s2, &req(&[], None))),
        ];
        insta::assert_snapshot!(render(&cases));
    }

    #[test]
    fn hostile_names_stay_one_statement() {
        let s = source(
            RelKind::Table,
            "my \"schema\"",
            "t;DROP",
            &["a\"b", "ç", "with space", "user", "x'); DROP TABLE t; --", "a/*", "a\\b"],
            Some(&["user", "a\"b"]),
        );
        let out = ok(&s, &req(&[], None));
        let out2 = ok(&s, &req(&[("x'); DROP TABLE t; --", true), ("ç", false)], Some("\"user\" > 1")));
        assert!(crate::build::scan_semicolons(&out2.sql).is_clean());
        insta::assert_snapshot!(render(&[("hostile varsayılan", out), ("hostile sıralama + filtre", out2)]));
    }

    #[test]
    fn rejected_filters_are_field_errors() {
        let s = table(&["id", "a"], Some(&["id"]));
        let big = format!("a = '{}'", "x".repeat(MAX_FILTER_BYTES));
        let exact = format!("a = '{}'", "x".repeat(MAX_FILTER_BYTES - 6));
        assert_eq!(exact.len(), MAX_FILTER_BYTES);
        ok(&s, &req(&[], Some(&exact)));
        let over = format!("a = '{}'", "x".repeat(MAX_FILTER_BYTES - 5));
        assert_eq!(over.len(), MAX_FILTER_BYTES + 1);
        for bad in [
            "1=1; DROP TABLE x",
            "1=1); DROP TABLE x; --",
            "x) OR (1=1",
            "a = 'b",
            "a /* b",
            "a = 'x\\y'",
            "a = 1\0",
            "a = '\0'",
            "a = 1\u{7}",
            "a = '\u{1b}'",
            "a = 1\u{85}",
            "a = 1 --",
            "a = $x$ b",
            big.as_str(),
            over.as_str(),
        ] {
            let e = errs(&s, &req(&[], Some(bad)));
            assert_eq!(paths(&e), ["filter"], "{bad:?}");
            // İleti filtre metnini yansıtmaz.
            assert!(!e[0].message.contains("DROP") && !e[0].message.contains("xxxx"), "{}", e[0].message);
        }
        // İzinli boşluk denetimleri: sekme, satır sonu, satır başı.
        ok(&s, &req(&[], Some("a = 1\r\n\tAND id > 0")));
    }

    /// Mevcut kapının dolar tırnağı ve tırnak içi `;`/`)` kabulü: kabul edilen her filtre sarıldığında nihai ifade
    /// tek ve temiz kalır (`ok` `composed_is_clean`'i denetler).
    #[test]
    fn accepted_tricky_filters_compose_cleanly() {
        let s = table(&["id", "a"], Some(&["id"]));
        for f in [
            "a = $$x$$",
            "a = $$;) DROP TABLE x; --$$",
            "a = $q$ ' $q$",
            "a = ';'",
            "a = ')'",
            "a = 'it''s; )'",
            "a = E'\\'; x'",
            "a /* ; ) */ = '1'",
            "a = '1' -- yorum ) ;\n",
            "a = \"we;ird\"",
        ] {
            let out = ok(&s, &req(&[], Some(f)));
            assert!(out.sql.contains(&format!(" WHERE ({f}\n)")), "{}", out.sql);
        }
    }

    #[test]
    fn order_errors() {
        let s = table(&["id", "a", "b", "c", "d", "e", "f", "g", "h"], Some(&["id"]));
        let e = errs(&s, &req(&[("a", false), ("gizli_kolon", false)], None));
        assert_eq!(paths(&e), ["order[1].column"]);
        assert!(!e[0].message.contains("gizli_kolon"), "{}", e[0].message);
        let e = errs(&s, &req(&[("a", false), ("b", true), ("a", true)], None));
        assert_eq!(paths(&e), ["order[2].column"]);
        // Büyük/küçük harf duyarlı: `ID` kaynakta yok.
        assert_eq!(paths(&errs(&s, &req(&[("ID", false)], None))), ["order[0].column"]);
        let eight: Vec<(&str, bool)> = ["a", "b", "c", "d", "e", "f", "g", "h"].iter().map(|c| (*c, false)).collect();
        ok(&s, &req(&eight, None));
        let mut nine = eight.clone();
        nine.push(("id", false));
        assert_eq!(paths(&errs(&s, &req(&nine, None))), ["order"]);
        // Sıralama ve filtre hataları birlikte döner.
        let e = errs(&s, &req(&[("gizli_kolon", false)], Some("1=1; x")));
        assert_eq!(paths(&e), ["order[0].column", "filter"]);
    }
}
