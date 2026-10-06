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

/// Arayüz ile arka uç arasındaki nesne tanımı (`kind` etiketi nesne türüdür).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(tag = "kind", rename_all = "snake_case")]
#[ts(export)]
pub enum ObjectDef {
    Schema(SchemaDef),
    Sequence(SequenceDef),
    Extension(ExtensionDef),
    Index(IndexDef),
}

impl ObjectDef {
    pub fn kind(&self) -> ObjectKind {
        match self {
            ObjectDef::Schema(_) => ObjectKind::Schema,
            ObjectDef::Sequence(_) => ObjectKind::Sequence,
            ObjectDef::Extension(_) => ObjectKind::Extension,
            ObjectDef::Index(_) => ObjectKind::Index,
        }
    }

    /// Nesnenin kendi adı (şema/tablo niteleyicisi olmadan).
    pub fn name(&self) -> &str {
        match self {
            ObjectDef::Schema(d) => &d.name,
            ObjectDef::Sequence(d) => &d.name,
            ObjectDef::Extension(d) => &d.name,
            ObjectDef::Index(d) => &d.name,
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
        assert!(serde_json::from_value::<ObjectDef>(json!({"kind": "table", "name": "x"})).is_err());
    }
}
