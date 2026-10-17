//! Kaynak yükleyici: katalogdan kolonlar, satır kimliği anahtarı (saf `choose_key`), yetkiler, yazılabilirlik
//! (saf `editability`) ve kaynak parmak izi (saf `fingerprint`).
//!
//! Okuma statik, `$1 = oid` parametreli SQL dosyalarıyladır (`sql/data_source_*.sql`); tanımlayıcı birleştirilmez.
//! Adlar/değerler günlüğe yazılmaz.

use std::hash::{DefaultHasher, Hash, Hasher};

use pgt_catalog::version::{PgVersion, VersionedSql};
use pgt_core::{Error, Result};
use tokio_postgres::Client;
use tokio_postgres::types::ToSql;

use super::model::{DataColumn, DataKey, DataSource, Identity, KeySource, ReadOnlyReason, RelKind};
use crate::load::ObjectRef;
use crate::model::ObjectKind;

static INFO_SQL: VersionedSql = VersionedSql(&[(10, include_str!("../sql/data_source_info.sql"))]);
/// PG 10–11: `attgenerated` yok (üretilmiş kolon PG 12+).
static COLUMNS_SQL: VersionedSql = VersionedSql(&[
    (10, include_str!("../sql/data_source_columns.pg10.sql")),
    (12, include_str!("../sql/data_source_columns.sql")),
]);
/// PG 10: `indnkeyatts` yok (INCLUDE PG 11+; tüm indeks kolonları anahtardır → `indnatts`).
static KEYS_SQL: VersionedSql = VersionedSql(&[
    (10, include_str!("../sql/data_source_keys.pg10.sql")),
    (11, include_str!("../sql/data_source_keys.sql")),
]);

/// Görüntülenemeyen nesne (desteklenmeyen tür ya da `relkind`).
pub const NOT_VIEWABLE: &str = "Bu nesnenin verisi görüntülenemez.";

/// Salt okunur sistem şemaları (`pg_toast_temp_N` dahil).
fn is_system_schema(schema: &str) -> bool {
    schema == "pg_catalog" || schema == "information_schema" || schema.starts_with("pg_toast")
}

/// Satır kimliği anahtarı adayı: PK kısıtı ya da (PK'nın kendi indeksi hariç) benzersiz indeks.
// `Debug` yok: ad ve kolon adları günlüğe/`Debug`'a girmez.
#[derive(Clone, PartialEq, Eq)]
pub struct KeyCandidate {
    pub source: KeySource,
    pub name: String,
    /// Anahtar kolonları (indeks sırasıyla; `INCLUDE` kolonları hariç).
    pub columns: Vec<String>,
    pub valid: bool,
    pub ready: bool,
    pub partial: bool,
    pub has_expr: bool,
    pub all_not_null: bool,
}

/// Katalogdan kaynak tanımı. `locked`: çağıran (yönetici) registry'den doldurur → burada `false`.
///
/// `object.kind` yalnızca `Table`/`View`/`MaterializedView` olabilir; `relkind` `r p v m f` dışındaysa (sıra,
/// indeks, bileşik tip…) `Invalid(NOT_VIEWABLE)`; oid yoksa `NotFound`. `major`: sunucu ana sürümü (10–18).
pub async fn load_source(client: &Client, object: ObjectRef, major: u32) -> Result<DataSource> {
    if !matches!(object.kind, ObjectKind::Table | ObjectKind::View | ObjectKind::MaterializedView) {
        return Err(Error::Invalid(NOT_VIEWABLE.to_string()));
    }
    let version = PgVersion(major * 10_000);
    let oid = object.oid;
    let params: [&(dyn ToSql + Sync); 1] = [&oid];

    let info = client.query(INFO_SQL.pick(version), &params).await.map_err(Error::from_query)?;
    let info = info.first().ok_or_else(|| Error::NotFound("veri kaynağı".to_string()))?;
    let relkind = match info.get::<_, String>("relkind").as_str() {
        "r" => RelKind::Table,
        "p" => RelKind::Partitioned,
        "v" => RelKind::View,
        "m" => RelKind::MatView,
        "f" => RelKind::Foreign,
        _ => return Err(Error::Invalid(NOT_VIEWABLE.to_string())),
    };
    let schema: String = info.get("schema");
    let name: String = info.get("name");
    let can_select: bool = info.get("can_select");
    let can_insert: bool = info.get("can_insert");
    let can_update: bool = info.get("can_update");
    let can_delete: bool = info.get("can_delete");

    let rows = client.query(COLUMNS_SQL.pick(version), &params).await.map_err(Error::from_query)?;
    let columns: Vec<DataColumn> = rows
        .iter()
        .map(|r| {
            let identity = match r.get::<_, String>("identity").as_str() {
                "a" => Identity::Always,
                "d" => Identity::ByDefault,
                _ => Identity::None,
            };
            let generated: bool = r.get("generated");
            // Yazılabilirlik kuralı tek yerde (SQL yalnızca ham yetkileri döndürür).
            let insertable = r.get::<_, bool>("col_insert") && !generated;
            let updatable = r.get::<_, bool>("col_update") && !generated && identity != Identity::Always;
            DataColumn {
                attnum: r.get("attnum"),
                name: r.get("name"),
                type_oid: r.get("type_oid"),
                // Yalnızca gösterim: PG'nin kestiği tip adı kayıplı (`pgt_core::pgtext`).
                type_display: pgt_core::pgtext::lossy(r, "type_display"),
                not_null: r.get("not_null"),
                has_default: r.get("has_default"),
                identity,
                generated,
                insertable,
                updatable,
            }
        })
        .collect();

    let rows = client.query(KEYS_SQL.pick(version), &params).await.map_err(Error::from_query)?;
    let candidates: Vec<KeyCandidate> = rows
        .iter()
        .map(|r| KeyCandidate {
            source: if r.get::<_, bool>("is_primary") { KeySource::PrimaryKey } else { KeySource::UniqueIndex },
            name: r.get("name"),
            columns: r.get("columns"),
            valid: r.get("valid"),
            ready: r.get("ready"),
            partial: r.get("partial"),
            has_expr: r.get("has_expr"),
            all_not_null: r.get("all_not_null"),
        })
        .collect();
    let key = choose_key(&candidates);

    let (editable, read_only_reason) = editability(relkind, &schema, key.as_ref(), can_insert, can_update, can_delete);
    let fingerprint = fingerprint(&schema, &name, &columns, key.as_ref());
    Ok(DataSource {
        object,
        schema,
        name,
        relkind,
        columns,
        key,
        can_select,
        can_insert,
        can_update,
        can_delete,
        locked: false,
        editable,
        read_only_reason,
        fingerprint,
    })
}

/// Saf: aday indekslerden satır kimliği anahtarını seçer (spec §2 kuralı).
///
/// (1) İlk PK adayı `valid ∧ ready ∧ !has_expr` ise o (PK için `partial`/`all_not_null` aranmaz); (2) aksi halde
/// `valid ∧ ready ∧ !partial ∧ !has_expr ∧ all_not_null` benzersiz indekslerden en az kolonlu, eşitlikte ad sırası
/// (bayt); (3) yoksa `None`.
pub fn choose_key(candidates: &[KeyCandidate]) -> Option<DataKey> {
    let to_key = |c: &KeyCandidate| DataKey { source: c.source, name: c.name.clone(), columns: c.columns.clone() };
    if let Some(pk) = candidates.iter().find(|c| c.source == KeySource::PrimaryKey)
        && pk.valid
        && pk.ready
        && !pk.has_expr
        && !pk.columns.is_empty()
    {
        return Some(to_key(pk));
    }
    candidates
        .iter()
        .filter(|c| {
            c.source == KeySource::UniqueIndex
                && c.valid
                && c.ready
                && !c.partial
                && !c.has_expr
                && c.all_not_null
                && !c.columns.is_empty()
        })
        .min_by(|a, b| a.columns.len().cmp(&b.columns.len()).then_with(|| a.name.as_bytes().cmp(b.name.as_bytes())))
        .map(to_key)
}

/// Saf: kolonlar + anahtar + şema/ad → 16 hane hex.
///
/// Girdi: şema, ad, her kolon için (ad, tip OID'si, üretilmiş, kimlik) kaynak sırasıyla ve anahtar (kaynak, ad,
/// kolonlar). `type_display` ve yetkiler dahil değildir (yetki değişimi parmak izini bozmaz; yazma yetkisi PG'de
/// yine denetlenir).
///
/// Kararlılık: `DefaultHasher`'ın algoritması Rust/std sürümleri ve derlemeler arasında **kararlı değildir**. Parmak
/// izi yalnızca **aynı süreç içinde** karşılaştırılır (yükle → seç/önizle/uygula); asla kalıcılaştırılmaz (diske,
/// ayara, geçmişe yazılmaz) ve başka bir derlemenin ürettiğiyle karşılaştırılmaz.
pub fn fingerprint(schema: &str, name: &str, columns: &[DataColumn], key: Option<&DataKey>) -> String {
    let mut h = DefaultHasher::new();
    schema.hash(&mut h);
    name.hash(&mut h);
    columns.len().hash(&mut h);
    for c in columns {
        c.name.hash(&mut h);
        c.type_oid.hash(&mut h);
        c.generated.hash(&mut h);
        let identity: u8 = match c.identity {
            Identity::None => 0,
            Identity::Always => 1,
            Identity::ByDefault => 2,
        };
        identity.hash(&mut h);
    }
    match key {
        None => 0u8.hash(&mut h),
        Some(k) => {
            let source: u8 = match k.source {
                KeySource::PrimaryKey => 1,
                KeySource::UniqueIndex => 2,
            };
            source.hash(&mut h);
            k.name.hash(&mut h);
            k.columns.hash(&mut h);
        }
    }
    format!("{:016x}", h.finish())
}

/// Saf: relkind + sistem şeması + anahtar + yetkiler → (editable, reason).
///
/// Öncelik: `UnsupportedKind` (Table/Partitioned dışı) > `SystemObject` > `NoKey` > `NoPrivilege` (INSERT, UPDATE,
/// DELETE'in hiçbiri yok).
pub fn editability(
    relkind: RelKind,
    schema: &str,
    key: Option<&DataKey>,
    can_insert: bool,
    can_update: bool,
    can_delete: bool,
) -> (bool, Option<ReadOnlyReason>) {
    let reason = if !matches!(relkind, RelKind::Table | RelKind::Partitioned) {
        Some(ReadOnlyReason::UnsupportedKind)
    } else if is_system_schema(schema) {
        Some(ReadOnlyReason::SystemObject)
    } else if key.is_none() {
        Some(ReadOnlyReason::NoKey)
    } else if !(can_insert || can_update || can_delete) {
        Some(ReadOnlyReason::NoPrivilege)
    } else {
        None
    };
    (reason.is_none(), reason)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cand(source: KeySource, name: &str, cols: &[&str]) -> KeyCandidate {
        KeyCandidate {
            source,
            name: name.into(),
            columns: cols.iter().map(|c| c.to_string()).collect(),
            valid: true,
            ready: true,
            partial: false,
            has_expr: false,
            all_not_null: true,
        }
    }

    fn pk(name: &str, cols: &[&str]) -> KeyCandidate {
        cand(KeySource::PrimaryKey, name, cols)
    }

    fn uq(name: &str, cols: &[&str]) -> KeyCandidate {
        cand(KeySource::UniqueIndex, name, cols)
    }

    fn key_of(c: &KeyCandidate) -> DataKey {
        DataKey { source: c.source, name: c.name.clone(), columns: c.columns.clone() }
    }

    #[test]
    fn primary_key_wins_even_over_smaller_unique_index() {
        let small = uq("a_small", &["x"]);
        let p = pk("t_pkey", &["a", "b"]);
        assert_eq!(choose_key(&[small.clone(), p.clone()]), Some(key_of(&p)));
        // PK için `partial`/`all_not_null` aranmaz.
        let odd = KeyCandidate { all_not_null: false, ..p.clone() };
        assert_eq!(choose_key(&[small, odd.clone()]), Some(key_of(&odd)));
    }

    #[test]
    fn invalid_primary_key_falls_back_to_unique_index() {
        let u = uq("u", &["x"]);
        for bad in [
            KeyCandidate { valid: false, ..pk("p", &["a"]) },
            KeyCandidate { ready: false, ..pk("p", &["a"]) },
            KeyCandidate { has_expr: true, ..pk("p", &["a"]) },
        ] {
            assert_eq!(choose_key(&[bad, u.clone()]), Some(key_of(&u)));
        }
    }

    #[test]
    fn without_pk_smallest_valid_not_null_unique_index() {
        let wide = uq("a_wide", &["x", "y"]);
        let narrow = uq("z_narrow", &["y"]);
        assert_eq!(choose_key(&[wide.clone(), narrow.clone()]), Some(key_of(&narrow)));
        assert_eq!(choose_key(&[narrow.clone(), wide.clone()]), Some(key_of(&narrow)));
    }

    #[test]
    fn unusable_unique_indexes_are_skipped() {
        let good = uq("z_good", &["a", "b", "c"]);
        let bads = [
            KeyCandidate { partial: true, ..uq("a1", &["x"]) },
            KeyCandidate { has_expr: true, ..uq("a2", &["x"]) },
            KeyCandidate { valid: false, ..uq("a3", &["x"]) },
            KeyCandidate { ready: false, ..uq("a4", &["x"]) },
            KeyCandidate { all_not_null: false, ..uq("a5", &["x"]) },
        ];
        for bad in &bads {
            assert_eq!(choose_key(&[bad.clone(), good.clone()]), Some(key_of(&good)));
            assert_eq!(choose_key(std::slice::from_ref(bad)), None);
        }
        let mut all = bads.to_vec();
        all.push(good.clone());
        assert_eq!(choose_key(&all), Some(key_of(&good)));
    }

    #[test]
    fn ties_break_by_byte_name_order() {
        let b = uq("b", &["x"]);
        let a_upper = uq("B", &["y"]); // 'B' (0x42) < 'b' (0x62)
        let a = uq("a", &["z"]);
        assert_eq!(choose_key(&[b.clone(), a_upper.clone()]), Some(key_of(&a_upper)));
        assert_eq!(choose_key(&[b, a.clone(), a_upper.clone()]), Some(key_of(&a_upper)));
        // Çok baytlı ad bayt sırasıyla ASCII'den sonra gelir.
        let u = uq("ü", &["x"]);
        assert_eq!(choose_key(&[u, a.clone()]), Some(key_of(&a)));
    }

    #[test]
    fn empty_list_has_no_key() {
        assert_eq!(choose_key(&[]), None);
    }

    #[test]
    fn multi_column_order_is_preserved() {
        let p = pk("p", &["c", "a", "b"]);
        assert_eq!(choose_key(std::slice::from_ref(&p)).unwrap().columns, vec!["c", "a", "b"]);
        let u = uq("u", &["z", "y"]);
        assert_eq!(choose_key(std::slice::from_ref(&u)).unwrap().columns, vec!["z", "y"]);
    }

    fn some_key() -> DataKey {
        DataKey { source: KeySource::PrimaryKey, name: "t_pkey".into(), columns: vec!["id".into()] }
    }

    #[test]
    fn editability_rules_and_priority() {
        let k = some_key();
        assert_eq!(editability(RelKind::Table, "public", Some(&k), true, true, true), (true, None));
        assert_eq!(editability(RelKind::Partitioned, "public", Some(&k), false, true, false), (true, None));
        for kind in [RelKind::View, RelKind::MatView, RelKind::Foreign] {
            assert_eq!(
                editability(kind, "public", Some(&k), true, true, true),
                (false, Some(ReadOnlyReason::UnsupportedKind))
            );
        }
        for schema in ["pg_catalog", "information_schema", "pg_toast"] {
            assert_eq!(
                editability(RelKind::Table, schema, Some(&k), true, true, true),
                (false, Some(ReadOnlyReason::SystemObject))
            );
        }
        // `pg_temp_N` (geçici tablo) sistem şeması sayılmaz.
        assert_eq!(editability(RelKind::Table, "pg_temp_3", Some(&k), true, true, true), (true, None));
        assert_eq!(editability(RelKind::Table, "public", None, true, true, true), (false, Some(ReadOnlyReason::NoKey)));
        assert_eq!(
            editability(RelKind::Table, "public", Some(&k), false, false, false),
            (false, Some(ReadOnlyReason::NoPrivilege))
        );
        for (i, u, d) in [(true, false, false), (false, true, false), (false, false, true)] {
            assert_eq!(editability(RelKind::Table, "public", Some(&k), i, u, d), (true, None));
        }
        // Öncelik.
        assert_eq!(editability(RelKind::View, "public", None, false, false, false), (false, Some(ReadOnlyReason::UnsupportedKind)));
        assert_eq!(
            editability(RelKind::Table, "pg_catalog", None, false, false, false),
            (false, Some(ReadOnlyReason::SystemObject))
        );
        assert_eq!(editability(RelKind::Table, "public", None, false, false, false), (false, Some(ReadOnlyReason::NoKey)));
    }

    fn col(name: &str, type_oid: u32) -> DataColumn {
        DataColumn {
            attnum: 1,
            name: name.into(),
            type_oid,
            type_display: "integer".into(),
            not_null: false,
            has_default: false,
            identity: Identity::None,
            generated: false,
            insertable: true,
            updatable: true,
        }
    }

    #[test]
    fn fingerprint_is_stable_and_sensitive() {
        let cols = vec![col("id", 23), col("v", 25)];
        let k = some_key();
        let base = fingerprint("s", "t", &cols, Some(&k));
        assert_eq!(base.len(), 16);
        assert!(base.chars().all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()), "{base}");
        assert_eq!(base, fingerprint("s", "t", &cols.clone(), Some(&k.clone())));

        let differs = |fp: String, why: &str| assert_ne!(fp, base, "{why}");
        differs(fingerprint("s2", "t", &cols, Some(&k)), "şema");
        differs(fingerprint("s", "t2", &cols, Some(&k)), "ad");
        let mut added = cols.clone();
        added.push(col("w", 23));
        differs(fingerprint("s", "t", &added, Some(&k)), "kolon eklendi");
        differs(fingerprint("s", "t", &[cols[1].clone(), cols[0].clone()], Some(&k)), "kolon sırası");
        let mut retyped = cols.clone();
        retyped[0].type_oid = 20;
        differs(fingerprint("s", "t", &retyped, Some(&k)), "tip OID'si");
        let mut renamed = cols.clone();
        renamed[1].name = "v2".into();
        differs(fingerprint("s", "t", &renamed, Some(&k)), "kolon adı");
        let mut generated = cols.clone();
        generated[1].generated = true;
        differs(fingerprint("s", "t", &generated, Some(&k)), "üretilmiş");
        let mut ident = cols.clone();
        ident[0].identity = Identity::Always;
        differs(fingerprint("s", "t", &ident, Some(&k)), "kimlik");
        differs(fingerprint("s", "t", &cols, None), "anahtar düştü");
        differs(fingerprint("s", "t", &cols, Some(&DataKey { name: "other".into(), ..k.clone() })), "anahtar adı");
        differs(
            fingerprint("s", "t", &cols, Some(&DataKey { source: KeySource::UniqueIndex, ..k.clone() })),
            "anahtar kaynağı",
        );
        differs(
            fingerprint("s", "t", &cols, Some(&DataKey { columns: vec!["id".into(), "v".into()], ..k.clone() })),
            "anahtar kolonları",
        );

        // `type_display` ve yetkiler parmak izine girmez.
        let mut cosmetic = cols.clone();
        cosmetic[0].type_display = "int4".into();
        cosmetic[0].insertable = false;
        cosmetic[1].updatable = false;
        cosmetic[1].not_null = true;
        cosmetic[1].has_default = true;
        assert_eq!(fingerprint("s", "t", &cosmetic, Some(&k)), base);
    }
}
