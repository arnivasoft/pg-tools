//! Veri düzenleyici modeli (arayüzle JSON; camelCase, etiketli enum'lar `kind`/`op`/`outcome`).
//!
//! Gizlilik: hücre/anahtar/sürüm değerleri ve kolon adları `Debug` çıktısına girmez (`CellValue`, `CellEdit`,
//! `RowChange`, `DataChangeSet` için elle `Debug`; yalnızca hâl adı ve sayılar).

use std::fmt;

use pgt_core::ErrorPayload;
use serde::{Deserialize, Serialize};
use ts_rs::TS;
use uuid::Uuid;

use crate::load::ObjectRef;
use crate::model::FieldError;

/// Kaynağın `pg_class.relkind` karşılığı (`r`, `p`, `v`, `m`, `f`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum RelKind {
    Table,
    Partitioned,
    View,
    #[serde(rename = "matview")]
    #[ts(rename = "matview")]
    MatView,
    Foreign,
}

/// Kaynak neden salt okunur (`DataSource.editable = false`). Öncelik: `UnsupportedKind` > `SystemObject` >
/// `NoKey` > `NoPrivilege`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum ReadOnlyReason {
    NoKey,
    NoPrivilege,
    SystemObject,
    UnsupportedKind,
}

/// `pg_attribute.attidentity`: `''` / `'a'` (GENERATED ALWAYS) / `'d'` (BY DEFAULT).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum Identity {
    None,
    Always,
    ByDefault,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct DataColumn {
    #[ts(type = "number")]
    pub attnum: i16,
    pub name: String,
    #[ts(type = "number")]
    pub type_oid: u32,
    pub type_display: String,
    pub not_null: bool,
    pub has_default: bool,
    pub identity: Identity,
    /// `attgenerated <> ''` (PG 12+; PG 18 `virtual` dahil).
    pub generated: bool,
    /// Kolon INSERT yetkisi ∧ !generated.
    pub insertable: bool,
    /// Kolon UPDATE yetkisi ∧ !generated ∧ identity != Always.
    pub updatable: bool,
}

/// Satır kimliği anahtarının kaynağı.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum KeySource {
    PrimaryKey,
    UniqueIndex,
}

/// Satır kimliği anahtarı: PK ya da (PK yoksa) en küçük geçerli NOT NULL benzersiz indeks.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct DataKey {
    pub source: KeySource,
    /// Kısıt/indeks adı.
    pub name: String,
    /// Anahtar kolonları (indeks sırasıyla; `RowChange.key` bu sırayı izler).
    pub columns: Vec<String>,
}

/// Kaynak tanımı.
///
/// `editable` kuralı: `relkind ∈ {Table, Partitioned}` ∧ sistem şeması değil ∧ `key.is_some()` ∧
/// (`can_insert ∨ can_update ∨ can_delete`). `editable = false` ise `read_only_reason` `Some`'dır (öncelik
/// `ReadOnlyReason` belgesinde). `locked` `editable`'ı değiştirmez (arayüz ayrıca gösterir; `apply` `locked` kapısı verir).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct DataSource {
    pub object: ObjectRef,
    pub schema: String,
    pub name: String,
    pub relkind: RelKind,
    pub columns: Vec<DataColumn>,
    pub key: Option<DataKey>,
    pub can_select: bool,
    pub can_insert: bool,
    pub can_update: bool,
    pub can_delete: bool,
    /// Sunucu `read_only_lock`.
    pub locked: bool,
    pub editable: bool,
    pub read_only_reason: Option<ReadOnlyReason>,
    /// Kaynak tanımının 16 haneli hex özeti (kolonlar/anahtar değişince farklılaşır → `plan_changed`).
    pub fingerprint: String,
}

/// Hücre değeri (PostgreSQL metin biçimi). `Debug` metni yazmaz.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(tag = "kind", rename_all = "snake_case")]
#[ts(export)]
pub enum CellValue {
    Null,
    /// `DEFAULT` anahtar sözcüğü (`INSERT … VALUES` ve `UPDATE … SET`).
    Default,
    Text { value: String },
}

impl fmt::Debug for CellValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CellValue::Null => f.write_str("Null"),
            CellValue::Default => f.write_str("Default"),
            CellValue::Text { .. } => f.write_str("Text(…)"),
        }
    }
}

/// Tek hücre değişikliği. `Debug` kolon adını ve değeri yazmaz.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct CellEdit {
    pub column: String,
    pub value: CellValue,
}

impl fmt::Debug for CellEdit {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CellEdit").field("column", &"…").field("value", &self.value).finish()
    }
}

/// Bir satır değişikliği. `key`: `DataKey.columns` sırasıyla anahtar değerleri; `version`: okumadaki `xmin::pg_catalog.text`.
/// `Debug` yalnızca hâl adı ve sayıları yazar.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(tag = "op", rename_all = "snake_case", rename_all_fields = "camelCase")]
#[ts(export)]
pub enum RowChange {
    Insert { values: Vec<CellEdit> },
    Update { key: Vec<String>, version: String, set: Vec<CellEdit> },
    Delete { key: Vec<String>, version: String },
}

impl fmt::Debug for RowChange {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RowChange::Insert { values } => f.debug_struct("Insert").field("values", &values.len()).finish(),
            RowChange::Update { key, set, .. } => {
                f.debug_struct("Update").field("key", &key.len()).field("version", &"…").field("set", &set.len()).finish()
            }
            RowChange::Delete { key, .. } => {
                f.debug_struct("Delete").field("key", &key.len()).field("version", &"…").finish()
            }
        }
    }
}

/// İstemcinin gönderdiği değişiklik listesi (sıra korunur). `Debug` yalnızca nesne ve değişiklik sayısını yazar.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct DataChangeSet {
    pub object: ObjectRef,
    /// İstemcinin yüklediği `DataSource.fingerprint`.
    pub fingerprint: String,
    pub changes: Vec<RowChange>,
}

impl fmt::Debug for DataChangeSet {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DataChangeSet").field("object", &self.object).field("changes", &self.changes.len()).finish()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct OrderBy {
    pub column: String,
    pub desc: bool,
}

/// `data_select` isteği: sıralama (kaynak kolonları) ve isteğe bağlı doğrulanan filtre gövdesi.
// `Debug` elle: yalnızca sıralama sayısı ve filtrenin bayt uzunluğu (filtre sabit değer içerebilir).
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct SelectRequest {
    pub order: Vec<OrderBy>,
    pub filter: Option<String>,
}

impl fmt::Debug for SelectRequest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SelectRequest")
            .field("order", &self.order.len())
            .field("filter_bytes", &self.filter.as_ref().map(String::len))
            .finish()
    }
}

/// Üretilen SELECT. `version_index`: sürüm (`xmin::pg_catalog.text`) kolonunun sırası (= kaynak kolon sayısı, hep son);
/// `has_version`: yalnızca Table/Partitioned (görünüm/MV/yabancı tabloda `xmin` sistem kolonu yok).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct DataSelect {
    pub sql: String,
    #[ts(type = "number")]
    pub version_index: u32,
    pub has_version: bool,
}

/// `data_select` sonucu: filtre/sıralama alan hataları kapı hatası değil **veri** olarak döner (önizleme gibi).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(tag = "result", rename_all = "snake_case")]
#[ts(export)]
pub enum DataSelectResult {
    Ok { select: DataSelect },
    Invalid { errors: Vec<FieldError> },
}

impl From<Result<DataSelect, Vec<FieldError>>> for DataSelectResult {
    fn from(r: Result<DataSelect, Vec<FieldError>>) -> Self {
        match r {
            Ok(select) => DataSelectResult::Ok { select },
            Err(errors) => DataSelectResult::Invalid { errors },
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct DataApplyOptions {
    /// İptal için istemci üretir.
    pub apply_id: Uuid,
    /// Kilit bekleme sınırı (ms; `0` = sınırsız).
    #[ts(type = "number")]
    pub lock_timeout_ms: u32,
    /// Önizlemedeki `PlanView.hash`.
    pub expected_hash: String,
    /// Yıkıcı plan (DELETE) ya da üretim sunucusu için kullanıcı onayı.
    pub confirmed: bool,
}

/// Uygulanan bir ifadenin sonucu. `returned`: `RETURNING` satırı (kaynak kolonları + yeni `xmin`); `DELETE` → `None`.
// `Debug` elle: yalnızca sıra ve dönen hücre sayısı.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct RowResult {
    #[ts(type = "number")]
    pub index: u32,
    pub returned: Option<Vec<Option<String>>>,
}

impl fmt::Debug for RowResult {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RowResult").field("index", &self.index).field("returned", &self.returned.as_ref().map(Vec::len)).finish()
    }
}

/// `data_apply` sonucu. Başarısızlıkta hiçbir ifade kalıcı değildir (tek transaction).
///
/// `Failed.conflict`: satır bayat/silinmiş (`row_conflict`; bu durumda `sqlstate = None`). `position`: PG'nin hata
/// konumu, başarısız ifadenin metnine göre. `ErrorPayload` yalnızca `Serialize` olduğundan bu tip de yalnızca
/// arayüze gider (`Deserialize` yok; `ApplyOutcome` kalıbı).
// `Debug` elle: dönen satırları ve hata metnini (PG `detail`'i satır verisi içerebilir) yazmaz; yalnızca sayılar,
// `sqlstate`, konum, hata **kodu** ve `conflict`.
#[derive(Clone, PartialEq, Eq, Serialize, TS)]
#[serde(tag = "outcome", rename_all = "snake_case", rename_all_fields = "camelCase")]
#[ts(export)]
pub enum DataOutcome {
    Applied {
        statements: u32,
        #[ts(type = "number")]
        elapsed_ms: u64,
        results: Vec<RowResult>,
    },
    Failed {
        failed_index: u32,
        sqlstate: Option<String>,
        position: Option<u32>,
        error: ErrorPayload,
        conflict: bool,
    },
}

impl fmt::Debug for DataOutcome {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DataOutcome::Applied { statements, elapsed_ms, results } => f
                .debug_struct("Applied")
                .field("statements", statements)
                .field("elapsed_ms", elapsed_ms)
                .field("results", &results.len())
                .finish(),
            DataOutcome::Failed { failed_index, sqlstate, position, error, conflict } => f
                .debug_struct("Failed")
                .field("failed_index", failed_index)
                .field("sqlstate", sqlstate)
                .field("position", position)
                .field("code", &error.code)
                .field("conflict", conflict)
                .finish(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::ObjectKind;
    use serde_json::json;

    fn obj() -> ObjectRef {
        ObjectRef { kind: ObjectKind::Table, oid: 42 }
    }

    fn roundtrip<T: Serialize + for<'de> Deserialize<'de> + PartialEq + std::fmt::Debug>(v: &T, want: serde_json::Value) {
        let got = serde_json::to_value(v).unwrap();
        assert_eq!(got, want);
        assert_eq!(&serde_json::from_value::<T>(got).unwrap(), v);
    }

    #[test]
    fn small_enums_are_snake_case() {
        roundtrip(&RelKind::Table, json!("table"));
        roundtrip(&RelKind::Partitioned, json!("partitioned"));
        roundtrip(&RelKind::View, json!("view"));
        roundtrip(&RelKind::MatView, json!("matview"));
        roundtrip(&RelKind::Foreign, json!("foreign"));
        roundtrip(&ReadOnlyReason::NoKey, json!("no_key"));
        roundtrip(&ReadOnlyReason::NoPrivilege, json!("no_privilege"));
        roundtrip(&ReadOnlyReason::SystemObject, json!("system_object"));
        roundtrip(&ReadOnlyReason::UnsupportedKind, json!("unsupported_kind"));
        roundtrip(&Identity::None, json!("none"));
        roundtrip(&Identity::Always, json!("always"));
        roundtrip(&Identity::ByDefault, json!("by_default"));
        roundtrip(&KeySource::PrimaryKey, json!("primary_key"));
        roundtrip(&KeySource::UniqueIndex, json!("unique_index"));
    }

    #[test]
    fn data_source_is_camel_case() {
        let src = DataSource {
            object: obj(),
            schema: "s".into(),
            name: "t".into(),
            relkind: RelKind::Table,
            columns: vec![DataColumn {
                attnum: 1,
                name: "id".into(),
                type_oid: 23,
                type_display: "integer".into(),
                not_null: true,
                has_default: false,
                identity: Identity::ByDefault,
                generated: false,
                insertable: true,
                updatable: true,
            }],
            key: Some(DataKey { source: KeySource::PrimaryKey, name: "t_pkey".into(), columns: vec!["id".into()] }),
            can_select: true,
            can_insert: true,
            can_update: false,
            can_delete: false,
            locked: false,
            editable: true,
            read_only_reason: None,
            fingerprint: "0123456789abcdef".into(),
        };
        roundtrip(
            &src,
            json!({
                "object": {"kind": "table", "oid": 42},
                "schema": "s", "name": "t", "relkind": "table",
                "columns": [{
                    "attnum": 1, "name": "id", "typeOid": 23, "typeDisplay": "integer", "notNull": true,
                    "hasDefault": false, "identity": "by_default", "generated": false, "insertable": true, "updatable": true
                }],
                "key": {"source": "primary_key", "name": "t_pkey", "columns": ["id"]},
                "canSelect": true, "canInsert": true, "canUpdate": false, "canDelete": false,
                "locked": false, "editable": true, "readOnlyReason": null, "fingerprint": "0123456789abcdef"
            }),
        );
    }

    #[test]
    fn cell_value_and_row_change_are_tagged() {
        roundtrip(&CellValue::Null, json!({"kind": "null"}));
        roundtrip(&CellValue::Default, json!({"kind": "default"}));
        roundtrip(&CellValue::Text { value: "x".into() }, json!({"kind": "text", "value": "x"}));
        roundtrip(
            &CellEdit { column: "c".into(), value: CellValue::Null },
            json!({"column": "c", "value": {"kind": "null"}}),
        );
        roundtrip(
            &RowChange::Insert { values: vec![CellEdit { column: "c".into(), value: CellValue::Default }] },
            json!({"op": "insert", "values": [{"column": "c", "value": {"kind": "default"}}]}),
        );
        roundtrip(
            &RowChange::Update {
                key: vec!["1".into()],
                version: "77".into(),
                set: vec![CellEdit { column: "c".into(), value: CellValue::Text { value: "v".into() } }],
            },
            json!({"op": "update", "key": ["1"], "version": "77",
                   "set": [{"column": "c", "value": {"kind": "text", "value": "v"}}]}),
        );
        roundtrip(
            &RowChange::Delete { key: vec!["1".into()], version: "77".into() },
            json!({"op": "delete", "key": ["1"], "version": "77"}),
        );
        roundtrip(
            &DataChangeSet { object: obj(), fingerprint: "f".into(), changes: vec![] },
            json!({"object": {"kind": "table", "oid": 42}, "fingerprint": "f", "changes": []}),
        );
    }

    #[test]
    fn select_and_apply_types_are_camel_case() {
        roundtrip(
            &SelectRequest { order: vec![OrderBy { column: "c".into(), desc: true }], filter: Some("a > 1".into()) },
            json!({"order": [{"column": "c", "desc": true}], "filter": "a > 1"}),
        );
        roundtrip(
            &DataSelect { sql: "SELECT 1".into(), version_index: 3, has_version: true },
            json!({"sql": "SELECT 1", "versionIndex": 3, "hasVersion": true}),
        );
        let id = Uuid::nil();
        roundtrip(
            &DataApplyOptions { apply_id: id, lock_timeout_ms: 5000, expected_hash: "h".into(), confirmed: false },
            json!({"applyId": id.to_string(), "lockTimeoutMs": 5000, "expectedHash": "h", "confirmed": false}),
        );
        roundtrip(
            &RowResult { index: 2, returned: Some(vec![Some("1".into()), None]) },
            json!({"index": 2, "returned": ["1", null]}),
        );
    }

    #[test]
    fn data_outcome_is_tagged_by_outcome() {
        let applied = DataOutcome::Applied {
            statements: 2,
            elapsed_ms: 7,
            results: vec![RowResult { index: 0, returned: None }],
        };
        assert_eq!(
            serde_json::to_value(&applied).unwrap(),
            json!({"outcome": "applied", "statements": 2, "elapsedMs": 7, "results": [{"index": 0, "returned": null}]})
        );
        let failed = DataOutcome::Failed {
            failed_index: 1,
            sqlstate: None,
            position: None,
            error: ErrorPayload { code: "row_conflict".into(), message: "m".into(), detail: None, hint: None },
            conflict: true,
        };
        let v = serde_json::to_value(&failed).unwrap();
        assert_eq!(v["outcome"], "failed");
        assert_eq!(v["failedIndex"], 1);
        assert_eq!(v["sqlstate"], serde_json::Value::Null);
        assert_eq!(v["position"], serde_json::Value::Null);
        assert_eq!(v["error"]["code"], "row_conflict");
        assert_eq!(v["conflict"], true);
    }

    const SECRET: &str = "gizli-değer";

    #[test]
    fn debug_never_prints_values_keys_versions_or_column_names() {
        let text = CellValue::Text { value: SECRET.into() };
        let d = format!("{text:?}");
        assert!(!d.contains(SECRET), "{d}");
        assert_eq!(d, "Text(…)");
        assert_eq!(format!("{:?}", CellValue::Null), "Null");
        assert_eq!(format!("{:?}", CellValue::Default), "Default");

        let edit = CellEdit { column: "gizli_kolon".into(), value: CellValue::Text { value: SECRET.into() } };
        let changes = vec![
            RowChange::Insert { values: vec![edit.clone()] },
            RowChange::Update { key: vec!["gizli_anahtar".into()], version: "gizli_surum".into(), set: vec![edit.clone()] },
            RowChange::Delete { key: vec!["gizli_anahtar".into()], version: "gizli_surum".into() },
        ];
        let set = DataChangeSet { object: obj(), fingerprint: "gizli_parmak".into(), changes: changes.clone() };
        let mut outputs: Vec<String> = changes.iter().map(|c| format!("{c:?}")).collect();
        outputs.push(format!("{edit:?}"));
        outputs.push(format!("{set:?}"));
        outputs.push(format!("{set:#?}"));
        for d in &outputs {
            for needle in [SECRET, "gizli_kolon", "gizli_anahtar", "gizli_surum"] {
                assert!(!d.contains(needle), "{needle} sızdı: {d}");
            }
        }
        // Yalnızca sayılar: kaç anahtar, kaç hücre, kaç değişiklik.
        assert_eq!(outputs[0], "Insert { values: 1 }");
        assert_eq!(outputs[1], r#"Update { key: 1, version: "…", set: 1 }"#);
        assert_eq!(outputs[2], r#"Delete { key: 1, version: "…" }"#);
        assert_eq!(outputs[3], r#"CellEdit { column: "…", value: Text(…) }"#);
        assert_eq!(outputs[4], "DataChangeSet { object: ObjectRef { kind: Table, oid: 42 }, changes: 3 }");
    }

    #[test]
    fn debug_of_results_outcomes_and_select_hides_data() {
        let row = RowResult { index: 2, returned: Some(vec![Some(SECRET.into()), None]) };
        assert_eq!(format!("{row:?}"), "RowResult { index: 2, returned: Some(2) }");
        assert_eq!(format!("{:?}", RowResult { index: 0, returned: None }), "RowResult { index: 0, returned: None }");

        let applied = DataOutcome::Applied { statements: 2, elapsed_ms: 7, results: vec![row.clone()] };
        assert_eq!(format!("{applied:?}"), "Applied { statements: 2, elapsed_ms: 7, results: 1 }");

        const DETAIL: &str = "Key (gizli_kolon)=(gizli_anahtar) already exists.";
        let failed = DataOutcome::Failed {
            failed_index: 1,
            sqlstate: Some("23505".into()),
            position: Some(3),
            error: ErrorPayload {
                code: "query".into(),
                message: format!("duplicate {SECRET}"),
                detail: Some(DETAIL.into()),
                hint: Some("gizli_ipucu".into()),
            },
            conflict: false,
        };
        let d = format!("{failed:?}");
        assert_eq!(
            d,
            r#"Failed { failed_index: 1, sqlstate: Some("23505"), position: Some(3), code: "query", conflict: false }"#
        );
        let pretty = format!("{failed:#?}");
        for needle in [SECRET, "gizli_kolon", "gizli_anahtar", "gizli_ipucu", "duplicate"] {
            assert!(!d.contains(needle) && !pretty.contains(needle), "{needle} sızdı: {d} / {pretty}");
        }

        let req = SelectRequest {
            order: vec![OrderBy { column: "gizli_kolon".into(), desc: false }],
            filter: Some(format!("x = '{SECRET}'")),
        };
        let d = format!("{req:?}");
        assert_eq!(d, format!("SelectRequest {{ order: 1, filter_bytes: Some({}) }}", req.filter.as_ref().unwrap().len()));
        assert_eq!(
            format!("{:?}", SelectRequest { order: vec![], filter: None }),
            "SelectRequest { order: 0, filter_bytes: None }"
        );
    }
}
