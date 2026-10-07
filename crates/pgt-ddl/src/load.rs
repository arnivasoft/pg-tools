//! Katalogdan tanım okuma: `ObjectDef`'i üreticilerin `create` girdisiyle AYNI biçimde ve normalleştirmeyle döndürür.
//!
//! Normalleştirme kuralları (üreticiyle gidiş-dönüşün dayanağı):
//! - Sahip her zaman `Some(rol)`; yorum yoksa `None`.
//! - Sequence: tipin/artışın varsayılan alt-üst sınırı `None` (üretici `NO MINVALUE`/`NO MAXVALUE` yazar).
//! - Extension: `cascade = false`, şema ve sürüm `Some`.
//! - Index: opclass yalnızca varsayılan değilse, collation yalnızca doğal collation'dan farklıysa, tablespace
//!   yalnızca varsayılan (`pg_default`) değilse; `nulls_first` yalnızca yönün varsayılanından farklıysa
//!   (ASC → NULLS LAST, DESC → NULLS FIRST varsayılandır); `constraint` kısıta ait indekste kısıt adı.
//! - Table: bkz. `load_table` (kolonlar attnum, kısıtlar ad sırasıyla; `partition_by` hiç dolmaz, bölümleme
//!   `partition_text` ile salt okunur gösterilir; modellenmeyen kısıt/özellikler `unsupported` listesindedir).
//!
//! Okuma salt okunur katalog bağlantısındandır; SQL/değerler günlüğe yazılmaz.

use pgt_catalog::version::{PgVersion, VersionedSql};
use pgt_core::connection::{ConnectionManager, Target};
use pgt_core::{Error, Result};
use serde::{Deserialize, Serialize};
use tokio_postgres::Row;
use tokio_postgres::types::ToSql;
use ts_rs::TS;
use uuid::Uuid;

use crate::model::*;

/// Yüklenecek nesne: tür + oid (şema için `pg_namespace`, sequence/indeks için `pg_class`, eklenti için `pg_extension`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ObjectRef {
    pub kind: ObjectKind,
    #[ts(type = "number")]
    pub oid: u32,
}

static SCHEMA_SQL: VersionedSql = VersionedSql(&[(10, include_str!("sql/load_schema.sql"))]);
static SEQUENCE_SQL: VersionedSql = VersionedSql(&[(10, include_str!("sql/load_sequence.sql"))]);
static EXTENSION_SQL: VersionedSql = VersionedSql(&[(10, include_str!("sql/load_extension.sql"))]);
/// PG 10'da `indnkeyatts` yok (INCLUDE PG 11+).
static INDEX_SQL: VersionedSql = VersionedSql(&[
    (10, include_str!("sql/load_index.pg10.sql")),
    (11, include_str!("sql/load_index.sql")),
]);
const INDEX_COLUMNS_SQL: &str = include_str!("sql/load_index_columns.sql");
static TABLE_SQL: VersionedSql = VersionedSql(&[(10, include_str!("sql/load_table.sql"))]);
/// PG 10–11: `attgenerated` yok.
static TABLE_COLUMNS_SQL: VersionedSql = VersionedSql(&[
    (10, include_str!("sql/load_table_columns.pg10.sql")),
    (12, include_str!("sql/load_table_columns.sql")),
]);
/// PG 10: INCLUDE/conparentid yok; PG 15: NULLS NOT DISTINCT; PG 18: zamansal/NOT ENFORCED kısıtlar, NOT NULL kısıtları.
static TABLE_CONSTRAINTS_SQL: VersionedSql = VersionedSql(&[
    (10, include_str!("sql/load_table_constraints.pg10.sql")),
    (11, include_str!("sql/load_table_constraints.pg11.sql")),
    (15, include_str!("sql/load_table_constraints.pg15.sql")),
    (18, include_str!("sql/load_table_constraints.sql")),
]);

const SYSTEM_ERROR: &str = "sistem nesnesi düzenlenemez";

/// `server_version_num` (ör. 160002); sunucuya bağlı değilse `NotConnected`.
pub async fn server_version(mgr: &ConnectionManager, server: Uuid) -> Result<u32> {
    mgr.info(server).map(|i| i.version_num).ok_or(Error::NotConnected)
}

/// Sürüme uygun SQL'i `oid` parametresiyle çalıştırır.
async fn fetch(mgr: &ConnectionManager, server: Uuid, db: u32, sql: &'static VersionedSql, oid: u32) -> Result<Vec<Row>> {
    mgr.run(server, Target::Database(db), move |c| async move {
        let params: [&(dyn ToSql + Sync); 1] = [&oid];
        c.client.query(sql.pick(PgVersion(c.version_num())), &params).await
    })
    .await
}

fn not_found(what: &str) -> Error {
    Error::NotFound(what.to_string())
}

/// Katalogdan tanım okur; yok ise `Error::NotFound`, sistem nesnesi ise `Error::Invalid`.
pub async fn load(mgr: &ConnectionManager, server: Uuid, database_oid: u32, obj: ObjectRef) -> Result<ObjectDef> {
    match obj.kind {
        ObjectKind::Schema => load_schema(mgr, server, database_oid, obj.oid).await,
        ObjectKind::Sequence => load_sequence(mgr, server, database_oid, obj.oid).await,
        ObjectKind::Extension => load_extension(mgr, server, database_oid, obj.oid).await,
        ObjectKind::Index => load_index(mgr, server, database_oid, obj.oid).await,
        ObjectKind::Table => load_table(mgr, server, database_oid, obj.oid).await,
        // Yer tutucu: yükleyiciler Plan 14 Task 6'da.
        ObjectKind::View | ObjectKind::MaterializedView | ObjectKind::Database | ObjectKind::Role => {
            Err(Error::Invalid("Bu tür henüz desteklenmiyor.".into()))
        }
    }
}

fn refuse_system(row: &Row) -> Result<()> {
    if row.get::<_, bool>("is_system") {
        return Err(Error::Invalid(SYSTEM_ERROR.to_string()));
    }
    Ok(())
}

async fn load_schema(mgr: &ConnectionManager, server: Uuid, db: u32, oid: u32) -> Result<ObjectDef> {
    let rows = fetch(mgr, server, db, &SCHEMA_SQL, oid).await?;
    let r = rows.first().ok_or_else(|| not_found("şema"))?;
    refuse_system(r)?;
    Ok(ObjectDef::Schema(SchemaDef { name: r.get("name"), owner: r.get("owner"), comment: r.get("comment") }))
}

/// Tipin doğal alt/üst sınırı (üreticinin `type_bounds` kuralıyla aynı).
fn type_bounds(t: SeqType) -> (i64, i64) {
    match t {
        SeqType::Smallint => (i16::MIN as i64, i16::MAX as i64),
        SeqType::Integer => (i32::MIN as i64, i32::MAX as i64),
        SeqType::Bigint => (i64::MIN, i64::MAX),
    }
}

/// Artışın işaretine göre PG'nin varsayılan `(min, max)` değeri: artan → (1, tür üst), azalan → (tür alt, -1).
fn default_bounds(t: SeqType, increment: i64) -> (i64, i64) {
    let (lo, hi) = type_bounds(t);
    if increment > 0 { (1, hi) } else { (lo, -1) }
}

async fn load_sequence(mgr: &ConnectionManager, server: Uuid, db: u32, oid: u32) -> Result<ObjectDef> {
    let rows = fetch(mgr, server, db, &SEQUENCE_SQL, oid).await?;
    let r = rows.first().ok_or_else(|| not_found("sequence"))?;
    refuse_system(r)?;
    if r.get::<_, bool>("is_identity") {
        return Err(Error::Invalid("kimlik (identity) dizisi tablo tanımından yönetilir".to_string()));
    }
    let data_type = match r.get::<_, String>("data_type").as_str() {
        "smallint" => SeqType::Smallint,
        "integer" => SeqType::Integer,
        "bigint" => SeqType::Bigint,
        other => return Err(Error::Invalid(format!("desteklenmeyen sequence türü: {other}"))),
    };
    let increment: i64 = r.get("increment");
    let (dmin, dmax) = default_bounds(data_type, increment);
    let (min, max): (i64, i64) = (r.get("min_value"), r.get("max_value"));
    let owned_by = match (
        r.get::<_, Option<String>>("owned_schema"),
        r.get::<_, Option<String>>("owned_table"),
        r.get::<_, Option<String>>("owned_column"),
    ) {
        (Some(schema), Some(table), Some(column)) => Some(ColumnRef { schema, table, column }),
        _ => None,
    };
    Ok(ObjectDef::Sequence(SequenceDef {
        schema: r.get("schema"),
        name: r.get("name"),
        owner: r.get("owner"),
        data_type,
        increment,
        min_value: (min != dmin).then_some(min),
        max_value: (max != dmax).then_some(max),
        start: r.get("start"),
        cache: r.get("cache"),
        cycle: r.get("cycle"),
        owned_by,
        comment: r.get("comment"),
    }))
}

async fn load_extension(mgr: &ConnectionManager, server: Uuid, db: u32, oid: u32) -> Result<ObjectDef> {
    let rows = fetch(mgr, server, db, &EXTENSION_SQL, oid).await?;
    let r = rows.first().ok_or_else(|| not_found("eklenti"))?;
    refuse_system(r)?;
    Ok(ObjectDef::Extension(ExtensionDef {
        name: r.get("name"),
        schema: Some(r.get("schema")),
        version: Some(r.get("version")),
        cascade: false,
    }))
}

/// İfadenin tamamını saran dış parantezleri soyar (`pg_get_indexdef` işlev olmayan ifadeleri parantezler;
/// üretici zaten kendi parantezini ekler). Tırnaklı dizge/tanımlayıcı içindeki parantezler sayılmaz.
fn strip_outer_parens(s: &str) -> String {
    let mut cur = s.trim();
    loop {
        if !(cur.starts_with('(') && cur.ends_with(')')) {
            break;
        }
        let (mut depth, mut quote) = (0i32, None::<char>);
        let mut wraps_all = true;
        let last = cur.len() - 1;
        for (i, ch) in cur.char_indices() {
            match quote {
                // Kaçışlı çift tırnak (`''`, `""`) iki ardışık kapanış+açılış olarak doğru işlenir.
                Some(q) => {
                    if ch == q {
                        quote = None;
                    }
                }
                None => match ch {
                    '\'' | '"' => quote = Some(ch),
                    '(' => depth += 1,
                    ')' => {
                        depth -= 1;
                        if depth == 0 && i != last {
                            wraps_all = false;
                            break;
                        }
                    }
                    _ => {}
                },
            }
        }
        if !wraps_all || depth != 0 {
            break;
        }
        cur = cur[1..cur.len() - 1].trim();
    }
    cur.to_string()
}

async fn load_index(mgr: &ConnectionManager, server: Uuid, db: u32, oid: u32) -> Result<ObjectDef> {
    let (head, cols) = mgr
        .run(server, Target::Database(db), move |c| async move {
            let params: [&(dyn ToSql + Sync); 1] = [&oid];
            let head = c.client.query(INDEX_SQL.pick(PgVersion(c.version_num())), &params).await?;
            let cols = if head.is_empty() { Vec::new() } else { c.client.query(INDEX_COLUMNS_SQL, &params).await? };
            Ok((head, cols))
        })
        .await?;
    let h = head.first().ok_or_else(|| not_found("indeks"))?;
    refuse_system(h)?;
    let method: String = h.get("method");
    let nkeyatts: i32 = h.get("nkeyatts");

    let mut columns = Vec::new();
    let mut include = Vec::new();
    for r in &cols {
        let key_text: String = r.get("key_text");
        if r.get::<_, i32>("pos") > nkeyatts {
            include.push(key_text);
            continue;
        }
        let key = if r.get::<_, i32>("attnum") == 0 {
            IndexKey::Expression { sql: strip_outer_parens(&key_text) }
        } else {
            IndexKey::Column { name: key_text }
        };
        // indoption: bit 0 = DESC, bit 1 = NULLS FIRST; varsayılan ASC→NULLS LAST, DESC→NULLS FIRST.
        let options: i32 = r.get("options");
        let descending = options & 1 != 0;
        let nulls_first = options & 2 != 0;
        columns.push(IndexColumn {
            key,
            opclass: r.get("opclass"),
            collation: r.get("collation"),
            descending,
            nulls_first: (nulls_first != descending).then_some(nulls_first),
        });
    }
    let fillfactor = h.get::<_, Option<String>>("fillfactor").and_then(|s| s.parse::<u32>().ok());
    Ok(ObjectDef::Index(IndexDef {
        schema: h.get("schema"),
        table: h.get("table_name"),
        name: h.get("name"),
        method,
        unique: h.get("is_unique"),
        columns,
        include,
        where_clause: h.get::<_, Option<String>>("where_clause").map(|w| strip_outer_parens(&w)),
        tablespace: h.get("tablespace"),
        fillfactor,
        comment: h.get("comment"),
        constraint: h.get("constraint_name"),
        valid: h.get("is_valid"),
    }))
}

// ---------------------------------------------------------------------------------------------------------------
// Tablo
// ---------------------------------------------------------------------------------------------------------------

fn fk_action(c: &str) -> Option<FkAction> {
    match c {
        "a" => Some(FkAction::NoAction),
        "r" => Some(FkAction::Restrict),
        "c" => Some(FkAction::Cascade),
        "n" => Some(FkAction::SetNull),
        "d" => Some(FkAction::SetDefault),
        _ => None,
    }
}

/// `unsupported` listesindeki CHECK kısıtı etiketinin türü (`"CHECK ad (ayrıntı)"`). Alter'ın PG 18 sanal kolon
/// kuralı bu listede CHECK arar: etiket ve arama tek kaynaktan (bkz. `is_unsupported_check`).
pub(crate) const CHECK_LABEL: &str = "CHECK";

/// `unsupported` girdisi bir CHECK kısıtı mı (yükleyicinin `CHECK_LABEL` etiketiyle).
pub(crate) fn is_unsupported_check(entry: &str) -> bool {
    entry.strip_prefix(CHECK_LABEL).is_some_and(|rest| rest.starts_with(' '))
}

/// Kısıt satırının yükleme sonucu.
#[derive(Debug, PartialEq)]
enum LoadedConstraint {
    Modelled(ConstraintDef),
    /// Modellenmeyen kısıt: insan okunur "tür ad (ayrıntı)" metni.
    Unsupported(String),
    /// Kolonun `notNull` alanıyla zaten modellenen düz PG 18 NOT NULL kısıtı.
    Hidden,
}

/// `pg_constraint` satırı → model. Modellenen türlerde (p/u/c/f) modellenmeyen bir özellik (`extra`, MATCH PARTIAL,
/// bilinmeyen FK eylemi) varsa kısıt bütünüyle desteklenmeyen listesine gider: alter onu hiç görmez, dokunmaz.
fn constraint_from_row(r: &Row) -> LoadedConstraint {
    let name: String = r.get("name");
    let contype: String = r.get("contype");
    let extra: Option<String> = r.get("extra");
    let unsupported = |what: &str, why: Option<&str>| {
        LoadedConstraint::Unsupported(match why {
            Some(w) => format!("{what} {name} ({w})"),
            None => format!("{what} {name}"),
        })
    };
    let label = match contype.as_str() {
        "p" => "PRIMARY KEY",
        "u" => "UNIQUE",
        "c" => CHECK_LABEL,
        "f" => "FOREIGN KEY",
        "x" => return unsupported("EXCLUDE", None),
        "t" => return unsupported("CONSTRAINT TRIGGER", None),
        "n" => {
            // PG 18: her NOT NULL kolonun bir kısıt satırı vardır; düz olanı `notNull` ile modellenir (görünmez).
            let mut why = vec![];
            if !r.get::<_, bool>("validated") {
                why.push("NOT VALID");
            }
            if r.get::<_, bool>("no_inherit") {
                why.push("NO INHERIT");
            }
            return if why.is_empty() { LoadedConstraint::Hidden } else { unsupported("NOT NULL", Some(&why.join(", "))) };
        }
        other => return unsupported(&format!("'{other}' türü kısıt"), None),
    };
    if let Some(x) = &extra {
        return unsupported(label, Some(x));
    }
    let columns: Vec<String> = r.get("columns");
    let include: Vec<String> = r.get("include");
    let kind = match contype.as_str() {
        "p" => ConstraintKind::PrimaryKey { columns, include },
        "u" => ConstraintKind::Unique { columns, include, nulls_not_distinct: r.get("nulls_not_distinct") },
        "c" => ConstraintKind::Check {
            expr: strip_outer_parens(&r.get::<_, Option<String>>("check_expr").unwrap_or_default()),
            no_inherit: r.get("no_inherit"),
        },
        _ => {
            let match_type = match r.get::<_, String>("match_type").as_str() {
                "s" => FkMatch::Simple,
                "f" => FkMatch::Full,
                _ => return unsupported(label, Some("MATCH PARTIAL")),
            };
            let (Some(on_update), Some(on_delete)) =
                (fk_action(&r.get::<_, String>("on_update")), fk_action(&r.get::<_, String>("on_delete")))
            else {
                return unsupported(label, Some("bilinmeyen eylem"));
            };
            ConstraintKind::ForeignKey {
                columns,
                ref_schema: r.get::<_, Option<String>>("ref_schema").unwrap_or_default(),
                ref_table: r.get::<_, Option<String>>("ref_table").unwrap_or_default(),
                ref_columns: r.get("ref_columns"),
                match_type,
                on_update,
                on_delete,
            }
        }
    };
    LoadedConstraint::Modelled(ConstraintDef {
        oid: Some(r.get::<_, u32>("oid")),
        name: Some(name),
        kind,
        deferrable: r.get("deferrable"),
        initially_deferred: r.get("initially_deferred"),
        not_valid: !r.get::<_, bool>("validated"),
    })
}

/// Kolon satırı → model. Varsayılan yalnızca kimlik ve üretilmiş OLMAYAN kolonda (pg_attrdef ifadesi üretilmiş
/// kolonda `generated.expr`'dir). İfadelerin tamamını saran dış parantez soyulur (üretici kendi parantezini ekler).
fn column_from_row(r: &Row) -> ColumnDef {
    let identity = match r.get::<_, String>("identity").as_str() {
        "a" => Some(IdentityKind::Always),
        "d" => Some(IdentityKind::ByDefault),
        _ => None,
    };
    let expr = r.get::<_, Option<String>>("expr").map(|x| strip_outer_parens(&x));
    let generated = match r.get::<_, String>("generated").as_str() {
        "" => None,
        g => Some(GeneratedDef { expr: expr.clone().unwrap_or_default(), stored: g == "s" }),
    };
    ColumnDef {
        attnum: Some(r.get("attnum")),
        name: r.get("name"),
        data_type: r.get("data_type"),
        not_null: r.get("not_null"),
        default: if identity.is_none() && generated.is_none() { expr } else { None },
        identity,
        generated,
        collation: r.get("collation"),
        comment: r.get::<_, Option<String>>("comment").filter(|c| !c.is_empty()),
        using: None,
    }
}

/// Tablo yükleyicisi. Normalleştirme (üreticiyle gidiş-dönüşün dayanağı):
/// - Sahip `Some(rol)`; tablespace varsayılan (`0`/`pg_default`) ise `None`; boş yorum `None`.
/// - Kolonlar attnum sırasıyla, `attnum` dolu; `data_type` = `format_type` (değiştiricili, görünmeyen şema nitelikli);
///   collation yalnızca tipin varsayılanından farklıysa.
/// - Kısıtlar ad sırasıyla (C collation; aynı adda oid): yükleme her seferinde aynı listeyi verir.
/// - `partition_by` her zaman `None` (bölümleme anahtarı alter'da değişmez); `partition_text` = `pg_get_partkeydef`.
/// - `unsupported`: EXCLUDE, kısıt tetikleyicileri, düz olmayan PG 18 NOT NULL kısıtları, modellenmeyen özellikli
///   kısıtlar, varsayılan dışı kimlik dizisi seçenekleri, fillfactor dışı depolama seçenekleri, `OF tip` tablolar.
async fn load_table(mgr: &ConnectionManager, server: Uuid, db: u32, oid: u32) -> Result<ObjectDef> {
    let (head, cols, cons) = mgr
        .run(server, Target::Database(db), move |c| async move {
            let params: [&(dyn ToSql + Sync); 1] = [&oid];
            let v = PgVersion(c.version_num());
            let head = c.client.query(TABLE_SQL.pick(v), &params).await?;
            if head.is_empty() {
                return Ok((head, vec![], vec![]));
            }
            let cols = c.client.query(TABLE_COLUMNS_SQL.pick(v), &params).await?;
            let cons = c.client.query(TABLE_CONSTRAINTS_SQL.pick(v), &params).await?;
            Ok((head, cols, cons))
        })
        .await?;
    let h = head.first().ok_or_else(|| not_found("tablo"))?;
    refuse_system(h)?;
    let persistence = match h.get::<_, String>("persistence").as_str() {
        "p" => Persistence::Permanent,
        "u" => Persistence::Unlogged,
        _ => return Err(Error::Invalid("geçici tablo düzenlenemez".to_string())),
    };
    let mut unsupported = vec![];
    if let Some(t) = h.get::<_, Option<String>>("of_type") {
        unsupported.push(format!("OF {t} (tipli tablo)"));
    }
    for o in h.get::<_, Vec<String>>("other_options") {
        unsupported.push(format!("WITH ({o})"));
    }
    let columns: Vec<ColumnDef> = cols.iter().map(column_from_row).collect();
    for (r, c) in cols.iter().zip(&columns) {
        if r.get::<_, bool>("identity_options") {
            unsupported.push(format!("kimlik dizisi seçenekleri ({})", c.name));
        }
    }
    let mut constraints = vec![];
    for r in &cons {
        match constraint_from_row(r) {
            LoadedConstraint::Modelled(c) => constraints.push(c),
            LoadedConstraint::Unsupported(u) => unsupported.push(u),
            LoadedConstraint::Hidden => {}
        }
    }
    Ok(ObjectDef::Table(TableDef {
        schema: h.get("schema"),
        name: h.get("name"),
        owner: h.get("owner"),
        tablespace: h.get("tablespace"),
        persistence,
        columns,
        constraints,
        partition_by: None,
        fillfactor: h.get::<_, Option<String>>("fillfactor").and_then(|s| s.parse::<u32>().ok()),
        comment: h.get::<_, Option<String>>("comment").filter(|c| !c.is_empty()),
        partition_text: h.get("partition_text"),
        is_partition: h.get("is_partition"),
        inherits: h.get("inherits"),
        unsupported,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_only_fully_wrapping_parens() {
        assert_eq!(strip_outer_parens("(a + b)"), "a + b");
        assert_eq!(strip_outer_parens("((a + b))"), "a + b");
        assert_eq!(strip_outer_parens("(a) + (b)"), "(a) + (b)");
        assert_eq!(strip_outer_parens("lower(name)"), "lower(name)");
        assert_eq!(strip_outer_parens("(x = ')')"), "x = ')'");
        assert_eq!(strip_outer_parens("(\"a)\" > 1)"), "\"a)\" > 1");
        assert_eq!(strip_outer_parens("a > 0"), "a > 0");
    }

    #[test]
    fn unsupported_check_label_is_single_source() {
        assert!(is_unsupported_check(&format!("{CHECK_LABEL} c (NOT ENFORCED)")));
        assert!(!is_unsupported_check(&format!("{CHECK_LABEL}X c")));
        assert!(!is_unsupported_check("EXCLUDE c"));
    }

    #[test]
    fn default_bounds_follow_increment_sign() {
        assert_eq!(default_bounds(SeqType::Integer, 1), (1, i32::MAX as i64));
        assert_eq!(default_bounds(SeqType::Smallint, -3), (i16::MIN as i64, -1));
        assert_eq!(default_bounds(SeqType::Bigint, 5), (1, i64::MAX));
    }

    #[test]
    fn object_ref_is_camel_case_json() {
        let v = serde_json::to_value(ObjectRef { kind: ObjectKind::Index, oid: 42 }).unwrap();
        assert_eq!(v, serde_json::json!({"kind": "index", "oid": 42}));
    }
}
