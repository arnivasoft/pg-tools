//! Diyalogların düzenlediği nesne tanımları (arayüzle JSON; camelCase, tür etiketi `kind`).

use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// i64 sözleşmesi: JS `number` 2^53'ün üstünde kesinlik kaybeder (sequence sınırları bigint aralığındadır). Bu yüzden
/// SequenceDef'in sayısal alanları JSON'da ondalık DİZGE olarak gider; girişte dizge de tamsayı da kabul edilir.
mod i64_str {
    use serde::de::{self, Visitor};
    use serde::{Deserializer, Serializer};
    use std::fmt;

    struct V;

    impl Visitor<'_> for V {
        type Value = i64;
        fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            f.write_str("ondalık tamsayı dizgesi ya da tamsayı")
        }
        fn visit_i64<E: de::Error>(self, v: i64) -> Result<i64, E> {
            Ok(v)
        }
        fn visit_u64<E: de::Error>(self, v: u64) -> Result<i64, E> {
            i64::try_from(v).map_err(|_| E::custom("tamsayı i64 aralığının dışında"))
        }
        fn visit_str<E: de::Error>(self, v: &str) -> Result<i64, E> {
            v.parse::<i64>().map_err(|_| E::custom("geçersiz tamsayı dizgesi"))
        }
    }

    pub fn serialize<S: Serializer>(v: &i64, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&v.to_string())
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<i64, D::Error> {
        d.deserialize_any(V)
    }

    /// `Option<i64>`: `null` -> `None`.
    pub mod opt {
        use serde::{Deserialize, Deserializer, Serializer};

        struct Wrap(i64);

        impl<'de> Deserialize<'de> for Wrap {
            fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
                super::deserialize(d).map(Wrap)
            }
        }

        pub fn serialize<S: Serializer>(v: &Option<i64>, s: S) -> Result<S::Ok, S::Error> {
            match v {
                Some(v) => s.serialize_str(&v.to_string()),
                None => s.serialize_none(),
            }
        }

        pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Option<i64>, D::Error> {
            Ok(Option::<Wrap>::deserialize(d)?.map(|w| w.0))
        }
    }
}

/// Desteklenen nesne türleri.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum ObjectKind {
    Schema,
    Sequence,
    Extension,
    Index,
    Table,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct SchemaDef {
    pub name: String,
    pub owner: Option<String>,
    pub comment: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum SeqType {
    Smallint,
    Integer,
    Bigint,
}

/// `OWNED BY` hedefi.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ColumnRef {
    pub schema: String,
    pub table: String,
    pub column: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct SequenceDef {
    pub schema: String,
    pub name: String,
    pub owner: Option<String>,
    pub data_type: SeqType,
    #[serde(with = "i64_str")]
    #[ts(type = "string")]
    pub increment: i64,
    #[serde(default, with = "i64_str::opt")]
    #[ts(type = "string | null")]
    pub min_value: Option<i64>,
    #[serde(default, with = "i64_str::opt")]
    #[ts(type = "string | null")]
    pub max_value: Option<i64>,
    #[serde(with = "i64_str")]
    #[ts(type = "string")]
    pub start: i64,
    #[serde(with = "i64_str")]
    #[ts(type = "string")]
    pub cache: i64,
    pub cycle: bool,
    pub owned_by: Option<ColumnRef>,
    pub comment: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ExtensionDef {
    pub name: String,
    pub schema: Option<String>,
    pub version: Option<String>,
    pub cascade: bool,
}

/// İndeks anahtarı: sütun adı ya da serbest ifade (kullanıcı SQL'i).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(tag = "kind", rename_all = "snake_case")]
#[ts(export)]
pub enum IndexKey {
    Column { name: String },
    Expression { sql: String },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct IndexColumn {
    pub key: IndexKey,
    pub opclass: Option<String>,
    pub collation: Option<String>,
    pub descending: bool,
    pub nulls_first: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct IndexDef {
    pub schema: String,
    pub table: String,
    pub name: String,
    pub method: String,
    pub unique: bool,
    pub columns: Vec<IndexColumn>,
    pub include: Vec<String>,
    pub where_clause: Option<String>,
    pub tablespace: Option<String>,
    #[ts(type = "number | null")]
    pub fillfactor: Option<u32>,
    pub comment: Option<String>,
    /// Yalnızca yüklemede dolar: indeks bir kısıta (PK/UNIQUE/EXCLUDE) aitse kısıt adı. Üreticiler bu indeksi değiştirmez/silmez.
    pub constraint: Option<String>,
    /// Yalnızca yüklemede dolar (`pg_index.indisvalid`): başarısız CONCURRENTLY inşası geçersiz indeks bırakır. Üreticiler
    /// bunu okumaz/yazmaz; alter farkına girmez. Yeni tanımda varsayılan `true`.
    #[serde(default = "default_true")]
    pub valid: bool,
}

fn default_true() -> bool {
    true
}

/// Tablo kalıcılığı (`UNLOGGED`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum Persistence {
    Permanent,
    Unlogged,
}

/// `GENERATED { ALWAYS | BY DEFAULT } AS IDENTITY`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum IdentityKind {
    Always,
    ByDefault,
}

/// Üretilmiş kolon: `GENERATED ALWAYS AS (expr) { STORED | VIRTUAL }`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct GeneratedDef {
    /// Serbest SQL.
    pub expr: String,
    /// `false` = VIRTUAL (PG 18+).
    pub stored: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ColumnDef {
    /// Yalnızca yüklemede dolar (`pg_attribute.attnum`): fark eşlemesinin kararlı kimliği. Yeni kolonda `None`.
    #[ts(type = "number | null")]
    pub attnum: Option<i16>,
    pub name: String,
    /// Tip ifadesi metni (ör. `numeric(10,2)`); serbest SQL değil, `type_syntax` dilbilgisiyle doğrulanır.
    pub data_type: String,
    pub not_null: bool,
    /// Serbest SQL.
    pub default: Option<String>,
    pub identity: Option<IdentityKind>,
    pub generated: Option<GeneratedDef>,
    pub collation: Option<String>,
    pub comment: Option<String>,
    /// Yalnızca alter, kalıcı değil: tip değişiminde `USING <ifade>` (serbest SQL). Yüklemede `None`.
    #[serde(default)]
    pub using: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum FkAction {
    NoAction,
    Restrict,
    Cascade,
    SetNull,
    SetDefault,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum FkMatch {
    Simple,
    Full,
}

/// Kısıt türü (`type` etiketi; alanlar camelCase).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(tag = "type", rename_all = "snake_case", rename_all_fields = "camelCase")]
#[ts(export)]
pub enum ConstraintKind {
    PrimaryKey {
        columns: Vec<String>,
        include: Vec<String>,
    },
    Unique {
        columns: Vec<String>,
        include: Vec<String>,
        nulls_not_distinct: bool,
    },
    Check {
        /// Serbest SQL.
        expr: String,
        no_inherit: bool,
    },
    ForeignKey {
        columns: Vec<String>,
        ref_schema: String,
        ref_table: String,
        ref_columns: Vec<String>,
        match_type: FkMatch,
        on_update: FkAction,
        on_delete: FkAction,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ConstraintDef {
    /// Yalnızca yüklemede dolar (`pg_constraint.oid`): fark eşlemesinin kararlı kimliği. Yeni kısıtta `None`.
    #[ts(type = "number | null")]
    pub oid: Option<u32>,
    /// `None` → adı sunucu verir.
    pub name: Option<String>,
    pub kind: ConstraintKind,
    /// PK/UNIQUE/FK.
    #[serde(default)]
    pub deferrable: bool,
    #[serde(default)]
    pub initially_deferred: bool,
    /// Yalnızca ADD sırasında anlamlı (`NOT VALID`); yüklemede `convalidated`'ın tersi.
    #[serde(default)]
    pub not_valid: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum PartitionStrategy {
    Range,
    List,
    Hash,
}

/// Bölümleme anahtarı: kolon adı ya da serbest ifade.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(tag = "kind", rename_all = "snake_case")]
#[ts(export)]
pub enum PartitionKey {
    Column { name: String },
    Expression { sql: String },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct PartitionByDef {
    pub strategy: PartitionStrategy,
    pub keys: Vec<PartitionKey>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct TableDef {
    pub schema: String,
    pub name: String,
    pub owner: Option<String>,
    pub tablespace: Option<String>,
    pub persistence: Persistence,
    pub columns: Vec<ColumnDef>,
    pub constraints: Vec<ConstraintDef>,
    /// Yalnızca oluşturma (`PARTITION BY`); alter'da değiştirilemez.
    pub partition_by: Option<PartitionByDef>,
    #[ts(type = "number | null")]
    pub fillfactor: Option<u32>,
    pub comment: Option<String>,
    /// Yalnızca yüklemede dolar: `pg_get_partkeydef` (salt okunur gösterim). İstemcinin kurduğu yeni tabloda yok.
    #[serde(default)]
    pub partition_text: Option<String>,
    /// Yalnızca yüklemede dolar: tablo bir bölümdür (kolon/kısıtlar ana tablodan yönetilir).
    #[serde(default)]
    pub is_partition: bool,
    /// Yalnızca yüklemede dolar: tablo kalıtımla bir üst tabloya bağlıdır.
    #[serde(default)]
    pub inherits: bool,
    /// Yalnızca yüklemede dolar: modellenmeyen ve alter'ın dokunmadığı kısıt/özelliklerin insan okunur listesi.
    #[serde(default)]
    pub unsupported: Vec<String>,
}

/// Arayüz ile arka uç arasındaki nesne tanımı (`kind` etiketi nesne türüdür).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(tag = "kind", rename_all = "snake_case")]
#[ts(export)]
pub enum ObjectDef {
    Schema(SchemaDef),
    Sequence(SequenceDef),
    Extension(ExtensionDef),
    Index(IndexDef),
    Table(TableDef),
}

impl ObjectDef {
    pub fn kind(&self) -> ObjectKind {
        match self {
            ObjectDef::Schema(_) => ObjectKind::Schema,
            ObjectDef::Sequence(_) => ObjectKind::Sequence,
            ObjectDef::Extension(_) => ObjectKind::Extension,
            ObjectDef::Index(_) => ObjectKind::Index,
            ObjectDef::Table(_) => ObjectKind::Table,
        }
    }

    /// Nesnenin kendi adı (şema/tablo niteleyicisi olmadan).
    pub fn name(&self) -> &str {
        match self {
            ObjectDef::Schema(d) => &d.name,
            ObjectDef::Sequence(d) => &d.name,
            ObjectDef::Extension(d) => &d.name,
            ObjectDef::Index(d) => &d.name,
            ObjectDef::Table(d) => &d.name,
        }
    }
}

/// Alan düzeyi doğrulama hatası; `path` arayüzdeki alanı gösterir (ör. `columns[1].key`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct FieldError {
    pub path: String,
    pub message: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn seq() -> SequenceDef {
        SequenceDef {
            schema: "public".into(),
            name: "s".into(),
            owner: None,
            data_type: SeqType::Bigint,
            increment: 1,
            min_value: None,
            max_value: Some(100),
            start: 1,
            cache: 1,
            cycle: false,
            owned_by: Some(ColumnRef { schema: "public".into(), table: "t".into(), column: "id".into() }),
            comment: None,
        }
    }

    #[test]
    fn object_def_is_tagged_with_kind_and_camel_case() {
        let v = serde_json::to_value(ObjectDef::Sequence(seq())).unwrap();
        assert_eq!(v["kind"], "sequence");
        assert_eq!(v["dataType"], "bigint");
        assert_eq!(v["maxValue"], "100");
        assert_eq!(v["increment"], "1");
        assert!(v["minValue"].is_null());
        assert_eq!(v["ownedBy"]["column"], "id");
        let back: ObjectDef = serde_json::from_value(v).unwrap();
        assert_eq!(back, ObjectDef::Sequence(seq()));
    }

    #[test]
    fn sequence_numbers_are_strings_and_accept_integers() {
        let mut s = seq();
        s.min_value = Some(i64::MIN);
        s.max_value = Some(i64::MAX);
        s.start = 9_007_199_254_740_993; // 2^53 + 1: JS number'a sığmaz
        let v = serde_json::to_value(&s).unwrap();
        assert_eq!((v["minValue"].clone(), v["maxValue"].clone(), v["start"].clone()), (json!("-9223372036854775808"), json!("9223372036854775807"), json!("9007199254740993")));
        assert_eq!(serde_json::from_value::<SequenceDef>(v).unwrap(), s);
        // Giriş: tamsayı ve dizge birlikte; eksik/`null` seçenekler `None`.
        let mut v = serde_json::to_value(seq()).unwrap();
        v["increment"] = json!(5);
        v["start"] = json!("7");
        v["minValue"] = json!(null);
        v.as_object_mut().unwrap().remove("maxValue");
        let d: SequenceDef = serde_json::from_value(v.clone()).unwrap();
        assert_eq!((d.increment, d.start, d.min_value, d.max_value), (5, 7, None, None));
        for bad in [json!("abc"), json!(1.5), json!(true), json!("9223372036854775808"), json!(18446744073709551615u64)] {
            v["increment"] = bad.clone();
            assert!(serde_json::from_value::<SequenceDef>(v.clone()).is_err(), "{bad}");
        }
    }

    #[test]
    fn index_key_round_trips_tagged() {
        let cols = vec![
            IndexColumn { key: IndexKey::Column { name: "a".into() }, opclass: None, collation: None, descending: false, nulls_first: None },
            IndexColumn {
                key: IndexKey::Expression { sql: "lower(b)".into() },
                opclass: Some("text_pattern_ops".into()),
                collation: None,
                descending: true,
                nulls_first: Some(true),
            },
        ];
        let v = serde_json::to_value(&cols).unwrap();
        assert_eq!(v[0]["key"], json!({"kind": "column", "name": "a"}));
        assert_eq!(v[1]["key"], json!({"kind": "expression", "sql": "lower(b)"}));
        assert_eq!(v[1]["nullsFirst"], true);
        let back: Vec<IndexColumn> = serde_json::from_value(v).unwrap();
        assert_eq!(back, cols);
    }

    #[test]
    fn index_def_round_trips_and_exposes_helpers() {
        let d = ObjectDef::Index(IndexDef {
            schema: "public".into(),
            table: "t".into(),
            name: "t_a_idx".into(),
            method: "btree".into(),
            unique: true,
            columns: vec![],
            include: vec!["c".into()],
            where_clause: Some("a > 0".into()),
            tablespace: None,
            fillfactor: Some(70),
            comment: None,
            constraint: None,
            valid: true,
        });
        let v = serde_json::to_value(&d).unwrap();
        assert_eq!(v["kind"], "index");
        assert_eq!(v["whereClause"], "a > 0");
        assert_eq!(serde_json::from_value::<ObjectDef>(v).unwrap(), d);
        assert_eq!((d.kind(), d.name()), (ObjectKind::Index, "t_a_idx"));
    }

    #[test]
    fn kind_and_name_for_every_variant() {
        let s = ObjectDef::Schema(SchemaDef { name: "x".into(), owner: None, comment: None });
        assert_eq!((s.kind(), s.name()), (ObjectKind::Schema, "x"));
        let e = ObjectDef::Extension(ExtensionDef { name: "hstore".into(), schema: None, version: None, cascade: false });
        assert_eq!((e.kind(), e.name()), (ObjectKind::Extension, "hstore"));
        assert_eq!(ObjectDef::Sequence(seq()).kind(), ObjectKind::Sequence);
        assert_eq!(serde_json::to_value(ObjectKind::Extension).unwrap(), "extension");
    }

    #[test]
    fn unknown_kind_is_rejected() {
        assert!(serde_json::from_value::<ObjectDef>(json!({"kind": "view", "name": "x"})).is_err());
        // Tanınan tür ama zorunlu alanları eksik.
        assert!(serde_json::from_value::<ObjectDef>(json!({"kind": "table", "name": "x"})).is_err());
    }

    fn table_json() -> serde_json::Value {
        json!({
            "kind": "table", "schema": "public", "name": "t", "owner": null, "tablespace": null, "persistence": "unlogged",
            "columns": [
                {"attnum": 1, "name": "id", "dataType": "bigint", "notNull": true, "default": null, "identity": "by_default",
                 "generated": null, "collation": null, "comment": null, "using": null},
                {"attnum": null, "name": "full", "dataType": "text", "notNull": false, "default": null, "identity": null,
                 "generated": {"expr": "a || b", "stored": true}, "collation": "C", "comment": "c", "using": "x::text"}
            ],
            "constraints": [
                {"oid": 16400, "name": "t_pkey", "kind": {"type": "primary_key", "columns": ["id"], "include": []},
                 "deferrable": false, "initiallyDeferred": false, "notValid": false},
                {"oid": null, "name": null, "kind": {"type": "unique", "columns": ["a"], "include": ["b"], "nullsNotDistinct": true},
                 "deferrable": true, "initiallyDeferred": true, "notValid": false},
                {"oid": null, "name": "ck", "kind": {"type": "check", "expr": "a > 0", "noInherit": true},
                 "deferrable": false, "initiallyDeferred": false, "notValid": true},
                {"oid": null, "name": "fk", "kind": {"type": "foreign_key", "columns": ["r"], "refSchema": "public", "refTable": "u",
                 "refColumns": ["id"], "matchType": "full", "onUpdate": "no_action", "onDelete": "set_null"},
                 "deferrable": false, "initiallyDeferred": false, "notValid": false}
            ],
            "partitionBy": {"strategy": "range", "keys": [{"kind": "column", "name": "id"}, {"kind": "expression", "sql": "lower(x)"}]},
            "fillfactor": 70, "comment": null,
            "partitionText": "RANGE (id)", "isPartition": false, "inherits": true, "unsupported": ["EXCLUDE ex"]
        })
    }

    #[test]
    fn table_def_round_trips_tagged_camel_case() {
        let v = table_json();
        let d: ObjectDef = serde_json::from_value(v.clone()).unwrap();
        assert_eq!(serde_json::to_value(&d).unwrap(), v);
        let ObjectDef::Table(t) = &d else { panic!() };
        assert_eq!((d.kind(), d.name()), (ObjectKind::Table, "t"));
        assert_eq!(t.persistence, Persistence::Unlogged);
        assert_eq!(t.columns[0].identity, Some(IdentityKind::ByDefault));
        assert_eq!((t.columns[0].attnum, t.columns[1].attnum), (Some(1), None));
        assert_eq!(t.columns[1].generated, Some(GeneratedDef { expr: "a || b".into(), stored: true }));
        assert!(matches!(
            &t.constraints[3].kind,
            ConstraintKind::ForeignKey { match_type: FkMatch::Full, on_update: FkAction::NoAction, on_delete: FkAction::SetNull, .. }
        ));
        assert!(matches!(&t.constraints[1].kind, ConstraintKind::Unique { nulls_not_distinct: true, .. }));
        assert_eq!(t.partition_by.as_ref().unwrap().strategy, PartitionStrategy::Range);
        assert_eq!(serde_json::to_value(ObjectKind::Table).unwrap(), "table");
        for (a, j) in [
            (FkAction::Restrict, "restrict"),
            (FkAction::Cascade, "cascade"),
            (FkAction::SetDefault, "set_default"),
        ] {
            assert_eq!(serde_json::to_value(a).unwrap(), j);
        }
        assert_eq!(serde_json::to_value(IdentityKind::Always).unwrap(), "always");
        assert_eq!(serde_json::to_value(PartitionStrategy::Hash).unwrap(), "hash");
        assert_eq!(serde_json::to_value(FkMatch::Simple).unwrap(), "simple");
        assert_eq!(serde_json::to_value(Persistence::Permanent).unwrap(), "permanent");
    }

    #[test]
    fn table_load_only_fields_default_when_absent() {
        // İstemcinin kurduğu yeni tablo: yükleme alanları, `using` ve kısıt bayrakları yok.
        let mut v = table_json();
        let o = v.as_object_mut().unwrap();
        for k in ["partitionText", "isPartition", "inherits", "unsupported"] {
            o.remove(k);
        }
        for c in v["columns"].as_array_mut().unwrap() {
            c.as_object_mut().unwrap().remove("using");
        }
        for c in v["constraints"].as_array_mut().unwrap() {
            for k in ["deferrable", "initiallyDeferred", "notValid"] {
                c.as_object_mut().unwrap().remove(k);
            }
        }
        let ObjectDef::Table(t) = serde_json::from_value::<ObjectDef>(v).unwrap() else { panic!() };
        assert_eq!((t.partition_text, t.is_partition, t.inherits, t.unsupported.len()), (None, false, false, 0));
        assert!(t.columns.iter().all(|c| c.using.is_none()));
        assert!(t.constraints.iter().all(|c| !c.deferrable && !c.initially_deferred && !c.not_valid));
    }

    #[test]
    fn table_rejects_invalid_enums_and_tags() {
        let bad = |path: &[&str], val: serde_json::Value| {
            let mut v = table_json();
            let mut cur = &mut v;
            for p in &path[..path.len() - 1] {
                cur = match p.parse::<usize>() {
                    Ok(i) => &mut cur[i],
                    Err(_) => &mut cur[*p],
                };
            }
            cur[path[path.len() - 1]] = val;
            assert!(serde_json::from_value::<ObjectDef>(v).is_err(), "{path:?}");
        };
        bad(&["persistence"], json!("temporary"));
        bad(&["persistence"], json!("Unlogged"));
        bad(&["columns", "0", "identity"], json!("byDefault"));
        bad(&["columns", "0", "attnum"], json!(40000));
        bad(&["columns", "0", "attnum"], json!("1"));
        bad(&["constraints", "0", "kind", "type"], json!("exclude"));
        bad(&["constraints", "0", "kind", "type"], json!("primaryKey"));
        bad(&["constraints", "0", "oid"], json!(-1));
        bad(&["constraints", "3", "kind", "onDelete"], json!("setNull"));
        bad(&["constraints", "3", "kind", "matchType"], json!("partial"));
        bad(&["partitionBy", "strategy"], json!("RANGE"));
        bad(&["partitionBy", "keys", "0", "kind"], json!("columns"));
        bad(&["fillfactor"], json!(-5));
        // Kısıt alanları camelCase olmalı: snake_case alan adı zorunlu alanı eksik bırakır.
        let mut v = table_json();
        let k = v["constraints"][1]["kind"].as_object_mut().unwrap();
        let x = k.remove("nullsNotDistinct").unwrap();
        k.insert("nulls_not_distinct".into(), x);
        assert!(serde_json::from_value::<ObjectDef>(v).is_err());
    }
}
