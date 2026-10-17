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
//! - View/MatView: bkz. `load_view`/`load_matview` (`query` = `pg_get_viewdef` sondaki `;` soyulmuş; MV `refresh`
//!   her zaman `None`; yeniden kurmada kaybolacak ekler `unsupported` listesinde).
//! - Database/Role (sunucu kapsamlı): bakım bağlantısında okunur (`Target::Maintenance`, `database_oid` yok sayılır);
//!   bkz. `load_database`/`load_role` (ayar adları küçük harf, ada göre sıralı; rol parolası hiç okunmaz).
//! - Function/Procedure: bkz. `load_function` (tipler `format_type`, varsayılanlar `pg_get_function_arg_default`, gövde
//!   `prosrc` baytı baytına, COST/ROWS üstelsiz ondalık metin; salt okunurluk `unsupported` doluysa `editable = false`).
//!
//! - Trigger/Type/Domain: bkz. `load_trigger`/`load_type`/`load_domain` (WHEN `pg_get_triggerdef`'ten, enum/composite
//!   `original = Some(ad)`, domain kısıtları ad sırasıyla `pg_get_constraintdef` gövdesi; `unsupported` doluysa
//!   `editable = false`).
//!
//! Okuma salt okunur katalog bağlantısındandır; SQL/değerler günlüğe yazılmaz.

use pgt_catalog::version::{PgVersion, VersionedSql};
use pgt_core::connection::{ConnectionManager, Target};
use pgt_core::pgtext::{is_utf8, lossy, lossy_opt, lossy_vec};
use pgt_core::{Error, Result};
use serde::{Deserialize, Serialize};
use tokio_postgres::Row;
use tokio_postgres::types::ToSql;
use ts_rs::TS;
use uuid::Uuid;

use crate::build::database::canonical_encoding;
use crate::model::*;

/// Yüklenen satırlarda geçerli UTF-8 olmayan metin (PG'nin kestiği tip adı ya da onu içeren `format_type`/ifade
/// metni) varsa `unsupported` gerekçesi: tanım yalnızca gösterim için kayıplı çözülür (`pgt_core::pgtext`) ve
/// üretici (`build`) bu gerekçeli tanımda her değişikliği, drop'u ve create'i reddeder — kayıplı ad SQL'e yazılmaz.
pub const INVALID_UTF8_REASON: &str = "geçerli UTF-8 olmayan ad (PostgreSQL uzun çok baytlı adı kesmiş; salt okunur)";

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
/// `security_invoker` (PG 15+) reloptions'ta yer alır; eski sürümlerde anahtar bulunmaz → tek dosya.
static VIEW_SQL: VersionedSql = VersionedSql(&[(10, include_str!("sql/load_view.sql"))]);
static MATVIEW_SQL: VersionedSql = VersionedSql(&[(10, include_str!("sql/load_matview.sql"))]);
const RELATION_COLUMNS_SQL: &str = include_str!("sql/load_relation_columns.sql");
/// PG 15: `datlocprovider`, `daticulocale`; PG 17: `datlocale` (icu ve builtin).
static DATABASE_SQL: VersionedSql = VersionedSql(&[
    (10, include_str!("sql/load_database.pg10.sql")),
    (15, include_str!("sql/load_database.pg15.sql")),
    (17, include_str!("sql/load_database.sql")),
]);
static ROLE_SQL: VersionedSql = VersionedSql(&[(10, include_str!("sql/load_role.sql"))]);
/// PG 16: üyelik başına `inherit_option`/`set_option` ve birden çok grantor.
static ROLE_MEMBERS_SQL: VersionedSql = VersionedSql(&[
    (10, include_str!("sql/load_role_members.pg10.sql")),
    (16, include_str!("sql/load_role_members.sql")),
]);
const ROLE_SETTINGS_SQL: &str = include_str!("sql/load_db_role_settings.sql");
/// PG 10: `prokind` yok (`proisagg`/`proiswindow`); PG 12: `prosupport`; PG 14: `prosqlbody` (BEGIN ATOMIC).
static FUNCTION_SQL: VersionedSql = VersionedSql(&[
    (10, include_str!("sql/load_function.pg10.sql")),
    (11, include_str!("sql/load_function.pg11.sql")),
    (12, include_str!("sql/load_function.pg12.sql")),
    (14, include_str!("sql/load_function.sql")),
]);
const FUNCTION_ARGS_SQL: &str = include_str!("sql/load_function_args.sql");
/// PG 13: `tgparentid` (bölüm klonu).
static TRIGGER_SQL: VersionedSql = VersionedSql(&[
    (10, include_str!("sql/load_trigger.sql")),
    (13, include_str!("sql/load_trigger.pg13.sql")),
]);
const TYPE_SQL: &str = include_str!("sql/load_type.sql");
const ENUM_VALUES_SQL: &str = include_str!("sql/load_enum_values.sql");
const COMPOSITE_ATTRS_SQL: &str = include_str!("sql/load_composite_attrs.sql");
/// PG 14: `rngmultitypid` (multirange).
static RANGE_SQL: VersionedSql = VersionedSql(&[
    (10, include_str!("sql/load_range.sql")),
    (14, include_str!("sql/load_range.pg14.sql")),
]);
const DOMAIN_SQL: &str = include_str!("sql/load_domain.sql");
const DOMAIN_CHECKS_SQL: &str = include_str!("sql/load_domain_checks.sql");

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

/// Satır kümelerinden birinde geçersiz UTF-8 metin varsa `INVALID_UTF8_REASON` (bir kez). Bu gerekçeyi taşıyan
/// yükleyiciler adı/tip metnini taşıyabilen alanları kayıplı okur (`lossy*`; yalnızca gösterim).
pub(crate) fn note_invalid_utf8(sets: &[&[Row]], unsupported: &mut Vec<String>) {
    if sets.iter().any(|rows| !rows.iter().all(is_utf8)) && !unsupported.iter().any(|u| u == INVALID_UTF8_REASON) {
        unsupported.push(INVALID_UTF8_REASON.to_string());
    }
}

/// Gerekçe alanı olmayan (`unsupported`suz) tanımlar: geçersiz UTF-8 metin → `Invalid` (çökme ve kayıplı ad yok).
fn refuse_invalid_utf8(sets: &[&[Row]]) -> Result<()> {
    if sets.iter().all(|rows| rows.iter().all(is_utf8)) { Ok(()) } else { Err(Error::Invalid(pgt_core::pgtext::invalid_utf8_message())) }
}

fn not_found(what: &str) -> Error {
    Error::NotFound(what.to_string())
}

/// Yükleme/uygulama hedefi (tek kural; `DdlManager::apply` de bunu kullanır, inceleme G-13): sunucu kapsamlı türler
/// (Database/Role) bakım veritabanında (`database_oid` yok sayılır); diğerleri verilen veritabanında, yoksa
/// `Invalid(NEEDS_DATABASE)`.
pub(crate) fn target_for(kind: ObjectKind, database_oid: Option<u32>) -> Result<Target> {
    match (kind.server_scoped(), database_oid) {
        (true, _) => Ok(Target::Maintenance),
        (false, Some(db)) => Ok(Target::Database(db)),
        (false, None) => Err(Error::Invalid(crate::error_codes::NEEDS_DATABASE.to_string())),
    }
}

/// Katalogdan tanım okur; yok ise `Error::NotFound`, sistem nesnesi ise `Error::Invalid`. Hedef `target_for` ile.
pub async fn load(mgr: &ConnectionManager, server: Uuid, database_oid: impl Into<Option<u32>> + Send, obj: ObjectRef) -> Result<ObjectDef> {
    let database_oid = match target_for(obj.kind, database_oid.into())? {
        Target::Database(db) => db,
        // Sunucu kapsamlı yükleyiciler `Target::Maintenance` alır; bu değer kullanılmaz.
        Target::Maintenance => 0,
    };
    match obj.kind {
        ObjectKind::Schema => load_schema(mgr, server, database_oid, obj.oid).await,
        ObjectKind::Sequence => load_sequence(mgr, server, database_oid, obj.oid).await,
        ObjectKind::Extension => load_extension(mgr, server, database_oid, obj.oid).await,
        ObjectKind::Index => load_index(mgr, server, database_oid, obj.oid).await,
        ObjectKind::Table => load_table(mgr, server, database_oid, obj.oid).await,
        ObjectKind::View => load_view(mgr, server, database_oid, obj.oid).await,
        ObjectKind::MaterializedView => load_matview(mgr, server, database_oid, obj.oid).await,
        // Sunucu kapsamlı türler: bakım veritabanında (veritabanı oid'i yok sayılır).
        ObjectKind::Database => load_database(mgr, server, Target::Maintenance, obj.oid).await,
        ObjectKind::Role => load_role(mgr, server, Target::Maintenance, obj.oid).await,
        ObjectKind::Function | ObjectKind::Procedure => load_function(mgr, server, database_oid, obj.kind, obj.oid).await,
        ObjectKind::Trigger => load_trigger(mgr, server, database_oid, obj.oid).await,
        ObjectKind::Type => load_type(mgr, server, database_oid, obj.oid).await,
        ObjectKind::Domain => load_domain(mgr, server, database_oid, obj.oid).await,
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
    refuse_invalid_utf8(&[&head, &cols])?;
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
    let columns = lossy_vec(r, "columns");
    let include = lossy_vec(r, "include");
    let kind = match contype.as_str() {
        "p" => ConstraintKind::PrimaryKey { columns, include },
        "u" => ConstraintKind::Unique { columns, include, nulls_not_distinct: r.get("nulls_not_distinct") },
        "c" => ConstraintKind::Check {
            expr: strip_outer_parens(&lossy_opt(r, "check_expr").unwrap_or_default()),
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
                ref_columns: lossy_vec(r, "ref_columns"),
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
    let expr = lossy_opt(r, "expr").map(|x| strip_outer_parens(&x));
    let generated = match r.get::<_, String>("generated").as_str() {
        "" => None,
        g => Some(GeneratedDef { expr: expr.clone().unwrap_or_default(), stored: g == "s" }),
    };
    ColumnDef {
        attnum: Some(r.get("attnum")),
        name: r.get("name"),
        data_type: lossy(r, "data_type"),
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
    for o in lossy_vec(h, "other_options") {
        unsupported.push(format!("WITH ({o})"));
    }
    let columns: Vec<ColumnDef> = cols.iter().map(column_from_row).collect();
    for (r, c) in cols.iter().zip(&columns) {
        if r.get::<_, bool>("identity_options") {
            unsupported.push(format!("kimlik dizisi seçenekleri ({})", c.name));
        }
    }
    note_invalid_utf8(&[&head, &cols, &cons], &mut unsupported);
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

// ---------------------------------------------------------------------------------------------------------------
// View / Materialized View
// ---------------------------------------------------------------------------------------------------------------

/// `pg_get_viewdef` metni → `query`: baş/son boşluk kırpılır, sondaki TEK `;` soyulur, kalan sağdan kırpılır. Dış
/// parantez soyulmaz (PG sarmalamaz; üreticinin `(…\n)` sarmalı yeniden ayrıştırmada kaybolur). Sabit nokta: yüklü
/// sorguyla yeniden oluşturulan görünümün yüklü sorgusu aynıdır (Docker testleri).
fn view_query(def: &str) -> String {
    let t = def.trim();
    t.strip_suffix(';').unwrap_or(t).trim_end().to_string()
}

/// PG `parse_bool` doğru biçimleri (büyük/küçük harf duyarsız; `t`/`tr`/`tru`, `y`/`ye` önekleri dahil).
fn reloption_true(v: &str) -> bool {
    let v = v.to_ascii_lowercase();
    matches!(v.as_str(), "t" | "tr" | "tru" | "true" | "y" | "ye" | "yes" | "on" | "1")
}

/// Görünüm reloptions'ı: `check_option`, `security_barrier`, `security_invoker`; başka anahtar (beklenmez) `unsupported`'a.
fn view_options(opts: &[String]) -> (Option<CheckOption>, bool, bool, Vec<String>) {
    let (mut check, mut barrier, mut invoker, mut other) = (None, false, false, vec![]);
    for o in opts {
        let (k, v) = o.split_once('=').unwrap_or((o.as_str(), ""));
        match (k, v.to_ascii_lowercase().as_str()) {
            ("check_option", "local") => check = Some(CheckOption::Local),
            ("check_option", "cascaded") => check = Some(CheckOption::Cascaded),
            ("security_barrier", _) => barrier = reloption_true(v),
            ("security_invoker", _) => invoker = reloption_true(v),
            _ => other.push(format!("WITH ({o})")),
        }
    }
    (check, barrier, invoker, other)
}

/// Kolon satırları → (kolonlar, varsayılanı olan kolon adları, yorumlu kolon adları); adlar ad sırasıyla.
fn relation_columns(rows: &[Row]) -> (Vec<ViewColumn>, Vec<String>, Vec<String>) {
    let columns = rows.iter().map(|r| ViewColumn { name: r.get("name"), data_type: lossy(r, "data_type") }).collect();
    let mut defaults: Vec<String> =
        rows.iter().filter(|r| r.get::<_, bool>("has_default")).map(|r| r.get("name")).collect();
    let mut comments: Vec<String> = rows
        .iter()
        .filter(|r| r.get::<_, Option<String>>("comment").is_some_and(|c| !c.is_empty()))
        .map(|r| r.get("name"))
        .collect();
    defaults.sort();
    comments.sort();
    (columns, defaults, comments)
}

/// Başlık + kolon satırları (başlık yoksa kolon sorgusu çalışmaz).
async fn fetch_relation(
    mgr: &ConnectionManager,
    server: Uuid,
    db: u32,
    sql: &'static VersionedSql,
    oid: u32,
) -> Result<(Vec<Row>, Vec<Row>)> {
    mgr.run(server, Target::Database(db), move |c| async move {
        let params: [&(dyn ToSql + Sync); 1] = [&oid];
        let head = c.client.query(sql.pick(PgVersion(c.version_num())), &params).await?;
        let cols = if head.is_empty() { Vec::new() } else { c.client.query(RELATION_COLUMNS_SQL, &params).await? };
        Ok((head, cols))
    })
    .await
}

/// Görünüm/MV ortak kapıları: geçici → `Invalid` (geçici şema `pg_temp_N` sistem sayılmadan önce), sistem → `Invalid`.
fn refuse_temp_or_system(r: &Row) -> Result<()> {
    if r.get::<_, String>("persistence") == "t" {
        return Err(Error::Invalid("geçici görünüm düzenlenemez".to_string()));
    }
    refuse_system(r)
}

/// View yükleyicisi. Normalleştirme: sahip `Some`; boş yorum `None`; `query` = `view_query(pg_get_viewdef(oid, true))`;
/// seçenekler reloptions'tan (`security_*` yalnızca doğru değerlerde `true`); `recreate = false`; `columns` attnum
/// sırasıyla; `unsupported` = `TRIGGER t`, `RULE r`, `DEFAULT c`, `COMMENT c` (her grup ad sırasıyla).
async fn load_view(mgr: &ConnectionManager, server: Uuid, db: u32, oid: u32) -> Result<ObjectDef> {
    let (head, cols) = fetch_relation(mgr, server, db, &VIEW_SQL, oid).await?;
    let h = head.first().ok_or_else(|| not_found("görünüm"))?;
    refuse_temp_or_system(h)?;
    let (check_option, security_barrier, security_invoker, other) = view_options(&lossy_vec(h, "options"));
    let (columns, defaults, comments) = relation_columns(&cols);
    let mut unsupported = vec![];
    unsupported.extend(lossy_vec(h, "triggers").into_iter().map(|t| format!("TRIGGER {t}")));
    unsupported.extend(lossy_vec(h, "rules").into_iter().map(|r| format!("RULE {r}")));
    unsupported.extend(defaults.into_iter().map(|c| format!("DEFAULT {c}")));
    unsupported.extend(comments.into_iter().map(|c| format!("COMMENT {c}")));
    unsupported.extend(other);
    note_invalid_utf8(&[&head, &cols], &mut unsupported);
    Ok(ObjectDef::View(ViewDef {
        schema: h.get("schema"),
        name: h.get("name"),
        owner: Some(h.get("owner")),
        query: view_query(&lossy_opt(h, "query").unwrap_or_default()),
        check_option,
        security_barrier,
        security_invoker,
        comment: h.get::<_, Option<String>>("comment").filter(|c| !c.is_empty()),
        recreate: false,
        columns,
        unsupported,
    }))
}

/// MatView yükleyicisi. Normalleştirme: sahip `Some`; boş yorum `None`; tablespace yalnızca `reltablespace = 0`
/// (veritabanı varsayılanı) ise `None`, aksi halde ad (varsayılanı başka olan veritabanında açık `pg_default` dahil); `query` View
/// gibi; `with_data = relispopulated`; `refresh = None`; `unsupported` = `WITH (k=v)` (fillfactor dışı seçenekler),
/// `INDEX ad`, `COMMENT c` (her grup ad sırasıyla). Üretici yeniden kurmada `INDEX ` önekini arar.
async fn load_matview(mgr: &ConnectionManager, server: Uuid, db: u32, oid: u32) -> Result<ObjectDef> {
    let (head, cols) = fetch_relation(mgr, server, db, &MATVIEW_SQL, oid).await?;
    let h = head.first().ok_or_else(|| not_found("materialized view"))?;
    refuse_temp_or_system(h)?;
    let (columns, _defaults, comments) = relation_columns(&cols);
    let mut unsupported: Vec<String> =
        lossy_vec(h, "other_options").into_iter().map(|o| format!("WITH ({o})")).collect();
    unsupported.extend(lossy_vec(h, "indexes").into_iter().map(|i| format!("INDEX {i}")));
    unsupported.extend(comments.into_iter().map(|c| format!("COMMENT {c}")));
    note_invalid_utf8(&[&head, &cols], &mut unsupported);
    Ok(ObjectDef::MaterializedView(MatViewDef {
        schema: h.get("schema"),
        name: h.get("name"),
        owner: Some(h.get("owner")),
        query: view_query(&lossy_opt(h, "query").unwrap_or_default()),
        tablespace: h.get("tablespace"),
        fillfactor: h.get::<_, Option<String>>("fillfactor").and_then(|s| s.parse::<u32>().ok()),
        with_data: h.get("populated"),
        comment: h.get::<_, Option<String>>("comment").filter(|c| !c.is_empty()),
        refresh: None,
        columns,
        unsupported,
    }))
}

// ---------------------------------------------------------------------------------------------------------------
// Database / Role (sunucu kapsamlı)
// ---------------------------------------------------------------------------------------------------------------

/// `setconfig` öğeleri (`ad=değer`, ilk `=`'den bölünür) → ayarlar, ada göre sıralı (üreticinin `diff_settings` sırası).
/// Ad: NOKTASIZ (yerleşik) adlar ASCII küçük harfe çevrilir (PG `TimeZone`/`DateStyle` gibi kanonik adı saklar; arama
/// harfe duyarsız, üretici dilbilgisi küçük harf); NOKTALI özel adlar saklandığı gibi kalır (PG yazımı korur ve başka
/// oturumda RESET/SET harfe duyarlı eşleşir — küçültmek RESET'i etkisiz, SET'i yinelenen kılardı, inceleme G-2).
/// Katlama yalnızca ASCII'dir (PG `guc_name_compare`). `=` içermeyen öğe (beklenmez) boş değerli ayar olur.
fn config_settings(items: &[String]) -> Vec<ConfigSetting> {
    let mut out: Vec<ConfigSetting> = items
        .iter()
        .map(|i| {
            let (k, v) = i.split_once('=').unwrap_or((i.as_str(), ""));
            let name = if k.contains('.') { k.to_string() } else { k.to_ascii_lowercase() };
            ConfigSetting { name, value: v.to_string() }
        })
        .collect();
    out.sort_by(|a, b| a.name.cmp(&b.name));
    out
}

fn locale_provider(c: &str) -> Option<LocaleProvider> {
    match c {
        "c" => Some(LocaleProvider::Libc),
        "i" => Some(LocaleProvider::Icu),
        "b" => Some(LocaleProvider::Builtin),
        _ => None,
    }
}

/// Database yükleyicisi (`target` = bakım bağlantısı). Normalleştirme: sahip `Some`; `template = None` (katalogda
/// yok); `encoding` = `SERVER_ENCODINGS` kanonik yazımı (`pg_encoding_to_char`); `localeProvider` PG 15+ `Some`, öncesi
/// `None`; `providerLocale` yalnızca icu/builtin'de (PG 15–16 `daticulocale`, 17+ `datlocale`), libc'de `None`;
/// `tablespace` her zaman `Some`; ayarlar `config_settings` ile; boş yorum `None`.
///
/// Create-only yerel ayar alanları (`lcCollate`, `lcCtype`, `providerLocale`) SUNUCUNUN normalleştirdiği biçimde
/// döner ve istemcide kanonikleştirilmez: glibc'de `POSIX` → `C` (PG 10–18'de gözlendi), ICU'da `en_US` → `en-US`
/// (PG 16+ BCP 47). Bu yüzden "yeniden yüklenen ≠ gönderilen tanım" türünden olası bir karşılaştırma bu alanları
/// dışarıda bırakmalıdır; `alter(load, load)` boş kalır (yüklü değer sabit noktadır).
async fn load_database(mgr: &ConnectionManager, server: Uuid, target: Target, oid: u32) -> Result<ObjectDef> {
    let rows = mgr
        .run(server, target, move |c| async move {
            let params: [&(dyn ToSql + Sync); 1] = [&oid];
            c.client.query(DATABASE_SQL.pick(PgVersion(c.version_num())), &params).await
        })
        .await?;
    let r = rows.first().ok_or_else(|| not_found("veritabanı"))?;
    let encoding: String = r.get("encoding");
    let locale_provider = r.get::<_, Option<String>>("locale_provider").as_deref().and_then(locale_provider);
    let provider_locale = match locale_provider {
        Some(LocaleProvider::Icu | LocaleProvider::Builtin) => {
            r.get::<_, Option<String>>("provider_locale").filter(|v| !v.is_empty())
        }
        _ => None,
    };
    Ok(ObjectDef::Database(DatabaseDef {
        name: r.get("name"),
        owner: Some(r.get("owner")),
        template: None,
        encoding: Some(canonical_encoding(&encoding).map_or(encoding.clone(), str::to_string)),
        locale_provider,
        lc_collate: Some(r.get("lc_collate")),
        lc_ctype: Some(r.get("lc_ctype")),
        provider_locale,
        tablespace: Some(r.get("tablespace")),
        connection_limit: r.get("connection_limit"),
        is_template: r.get("is_template"),
        allow_connections: r.get("allow_connections"),
        settings: config_settings(&r.get::<_, Vec<String>>("settings")),
        comment: r.get::<_, Option<String>>("comment").filter(|c| !c.is_empty()),
    }))
}

const MEMBER_PREFIX: &str = "MEMBER ";
const MULTI_GRANTOR_SUFFIX: &str = " (birden çok grantor)";
const GRANTOR_INFIX: &str = " (grantor ";

/// Salt okunur üyelik etiketi: aynı rol için birden çok grantor (PG 16+).
pub(crate) fn multi_grantor_label(role: &str) -> String {
    format!("{MEMBER_PREFIX}{role}{MULTI_GRANTOR_SUFFIX}")
}

/// Salt okunur üyelik etiketi: önyükleme süper kullanıcısından başka grantor (PG 16+).
pub(crate) fn grantor_label(role: &str, grantor: &str) -> String {
    format!("{MEMBER_PREFIX}{role}{GRANTOR_INFIX}{grantor})")
}

/// `unsupported` girdisi salt okunur bir üyelik etiketiyse (yukarıdaki iki üreticinin biçimi) üye olunan rolün adı.
/// Ad ilk `" (grantor "` ayıracına kadar alınır (rol adında bu dizi beklenmez; grantor adında olabilir).
pub(crate) fn unsupported_membership_role(entry: &str) -> Option<&str> {
    let rest = entry.strip_prefix(MEMBER_PREFIX)?;
    if let Some(role) = rest.strip_suffix(MULTI_GRANTOR_SUFFIX) {
        return Some(role);
    }
    let (role, grantor) = rest.split_once(GRANTOR_INFIX)?;
    grantor.ends_with(')').then_some(role)
}

/// Üyelik satırları → (üyelikler, desteklenmeyen). PG 16+'da aynı rol için birden çok grantor satırı → `MEMBER r
/// (birden çok grantor)`; tek satırlı üyeliği önyükleme süper kullanıcısından (oid 10) başkası vermişse → `MEMBER r
/// (grantor g)` (inceleme M-1: GRANTED BY'sız REVOKE PG 16+'da WARNING ile etkisizdir, sessizce "başarılı" görünürdü).
/// İkisi de salt okunurdur (üretici görmez). Tek satırda `inheritOption`/`setOption` dolu (PG < 16 SQL'i NULL ve
/// `grantor_is_bootstrap = true` döndürür: grantor REVOKE'u etkilemez).
fn memberships(rows: &[Row]) -> (Vec<RoleMembership>, Vec<String>) {
    let (mut list, mut unsupported) = (vec![], vec![]);
    for r in rows {
        let role: String = r.get("role");
        if r.get::<_, i32>("grants") > 1 {
            unsupported.push(multi_grantor_label(&role));
            continue;
        }
        if !r.get::<_, bool>("grantor_is_bootstrap") {
            let grantor: Option<String> = r.get("grantor");
            unsupported.push(grantor_label(&role, &grantor.unwrap_or_default()));
            continue;
        }
        list.push(RoleMembership {
            role,
            admin: r.get("admin"),
            inherit_option: r.get("inherit_option"),
            set_option: r.get("set_option"),
        });
    }
    // Üreticinin BTreeMap (bayt) sırası.
    list.sort_by(|a, b| a.role.cmp(&b.role));
    (list, unsupported)
}

/// Role yükleyicisi (`target` = bakım bağlantısı; `pg_roles`, süper kullanıcı gerekmez, parola sütunu hiç seçilmez).
/// Normalleştirme: `pg_` önekli rol → `Invalid`; `password = keep`; `validUntil` sabit UTC biçimi (SQL'de) ya da
/// `infinity`; üyelikler rol adı sırasıyla (`memberships`); ayarlar yalnızca tüm veritabanları için olanlar
/// (`config_settings`), `IN DATABASE d` ayarları `unsupported` ("IN DATABASE d: k=v", veritabanı ve ad sırasıyla);
/// boş yorum `None`; sahip yok (Karar 5).
async fn load_role(mgr: &ConnectionManager, server: Uuid, target: Target, oid: u32) -> Result<ObjectDef> {
    let (head, members, settings) = mgr
        .run(server, target, move |c| async move {
            let params: [&(dyn ToSql + Sync); 1] = [&oid];
            let v = PgVersion(c.version_num());
            let head = c.client.query(ROLE_SQL.pick(v), &params).await?;
            if head.is_empty() {
                return Ok((head, vec![], vec![]));
            }
            let members = c.client.query(ROLE_MEMBERS_SQL.pick(v), &params).await?;
            let settings = c.client.query(ROLE_SETTINGS_SQL, &params).await?;
            Ok((head, members, settings))
        })
        .await?;
    let h = head.first().ok_or_else(|| not_found(crate::error_codes::ROLE_SUBJECT))?;
    if h.get::<_, bool>("is_system") {
        return Err(Error::Invalid("sistem rolü düzenlenemez".to_string()));
    }
    let (member_of, mut unsupported) = memberships(&members);
    let mut global = vec![];
    let mut per_db: Vec<(String, ConfigSetting)> = vec![];
    for r in &settings {
        let items = config_settings(&r.get::<_, Vec<String>>("settings"));
        match r.get::<_, Option<String>>("database") {
            None => global.extend(items),
            Some(db) => per_db.extend(items.into_iter().map(|s| (db.clone(), s))),
        }
    }
    global.sort_by(|a, b| a.name.cmp(&b.name));
    per_db.sort_by(|a, b| (&a.0, &a.1.name).cmp(&(&b.0, &b.1.name)));
    unsupported.extend(per_db.into_iter().map(|(db, s)| format!("IN DATABASE {db}: {}={}", s.name, s.value)));
    Ok(ObjectDef::Role(RoleDef {
        name: h.get("name"),
        login: h.get("login"),
        superuser: h.get("superuser"),
        createdb: h.get("createdb"),
        createrole: h.get("createrole"),
        inherit: h.get("inherit"),
        replication: h.get("replication"),
        bypassrls: h.get("bypassrls"),
        connection_limit: h.get("connection_limit"),
        valid_until: h.get("valid_until"),
        password: PasswordChange::Keep,
        member_of,
        settings: global,
        comment: h.get::<_, Option<String>>("comment").filter(|c| !c.is_empty()),
        unsupported,
    }))
}

// ---------------------------------------------------------------------------------------------------------------
// Function / Procedure
// ---------------------------------------------------------------------------------------------------------------

/// `pg_proc` argüman satırı (`load_function_args.sql`; saf işlemeyi test edilebilir kılan ara biçim).
#[derive(Debug, Clone, PartialEq)]
struct ProcArg {
    mode: String,
    name: Option<String>,
    type_name: String,
    default: Option<String>,
}

/// Argüman satırları → (argümanlar, TABLE kolonları — `t` modu yoksa `None`, desteklenmeyenler). Bilinmeyen mod ya da
/// adsız TABLE kolonu (beklenmez) `unsupported`'a yazılır ve argüman atlanır (zaten salt okunur olur).
fn routine_args(rows: &[ProcArg]) -> (Vec<FunctionArg>, Option<Vec<ReturnColumn>>, Vec<String>) {
    let (mut args, mut table, mut unsupported) = (vec![], None::<Vec<ReturnColumn>>, vec![]);
    for r in rows {
        let mode = match r.mode.as_str() {
            "i" => ArgMode::In,
            "o" => ArgMode::Out,
            "b" => ArgMode::Inout,
            "v" => ArgMode::Variadic,
            "t" => {
                match &r.name {
                    Some(name) => table.get_or_insert_with(Vec::new).push(ReturnColumn { name: name.clone(), type_name: r.type_name.clone() }),
                    None => unsupported.push("adsız TABLE kolonu".to_string()),
                }
                continue;
            }
            other => {
                unsupported.push(format!("argüman modu '{other}'"));
                continue;
            }
        };
        args.push(FunctionArg { mode, name: r.name.clone(), type_name: r.type_name.clone(), default: r.default.clone() });
    }
    (args, table, unsupported)
}

/// `float4` → üstelsiz ondalık metin (`1e+10` → `10000000000`, `0.5` → `0.5`): Rust `f32` gösterimi en kısa kesin
/// ondalığı üs kullanmadan yazar; üreticinin `parse_positive` dilbilgisi (`rakam+ ('.' rakam+)?`) bunu kabul eder.
fn float_text(v: f32) -> String {
    format!("{v}")
}

/// Pozitif sayı metni ya da (üreticinin dilbilgisinden geçmiyorsa — ör. 32 karakteri aşan çok büyük değer) gerekçe.
fn positive_text(label: &str, v: f32, unsupported: &mut Vec<String>) -> String {
    let t = float_text(v);
    if crate::build::routine::parse_positive(&t).is_none() {
        unsupported.push(format!("{label} {t}"));
    }
    t
}

/// Liste ayarı (`search_path` vb.) değeri PG `SplitGUCList` kuralıyla bölünemiyorsa üretici onu tek öğe yazardı (anlam
/// değişir; `OR REPLACE` her seferinde tüm `SET` listesini yeniden yazar) → salt okunur gerekçesi.
fn unsplittable_settings(settings: &[ConfigSetting]) -> Vec<String> {
    use crate::build::settings::{LIST_QUOTE_GUCS, split_guc_list};
    settings
        .iter()
        .filter(|s| LIST_QUOTE_GUCS.contains(&s.name.as_str()) && split_guc_list(&s.value).is_err())
        .map(|s| format!("SET {} (liste değeri ayrıştırılamıyor)", s.name))
        .collect()
}

/// Function/Procedure yükleyicisi. Normalleştirme (üreticiyle gidiş-dönüşün ve `alter(yüklü, yüklü)` boş planının dayanağı):
/// - Sahip `Some`; boş yorum `None`; `recreate = false`; dil `pg_language` adı (küçük harf).
/// - Argümanlar konum sırasıyla; tip `format_type(oid, NULL)` (görünmeyen şema nitelikli, değiştiricisiz); adsız →
///   `None`; varsayılan `pg_get_function_arg_default` metni (giriş argümanlarında). `t` modlu argümanlar
///   `returns = Table` kolonlarıdır (argüman listesinde yer almaz).
/// - Dönüş: Procedure `None`; TABLE (yukarıda); `prorettype = trigger` → `Trigger`; küme olmayan `void` → `Void`; diğer
///   her şey `Type { format_type, proretset }` (OUT argümanlı fonksiyonda `record` ya da tek OUT tipi).
/// - Gövde `prosrc` baytı baytına (CRLF, Unicode, `$pgt$` dahil).
/// - COST: fonksiyonda HER ZAMAN `Some(procost)` (varsayılan 100 dahil; `None` yalnızca create'te "varsayılan" demektir);
///   ROWS: küme dönen fonksiyonda `Some(prorows)` (varsayılan 1000 dahil), diğerlerinde `None` (`prorows = 0`).
///   Procedure'de ikisi de `None` (PG yazdırmaz; üretici reddeder). Metin `float_text` (üstelsiz ondalık).
/// - Ayarlar `config_settings` (noktasız adlar küçük harf, ada göre sıralı; değer saklandığı gibi).
/// - `unsupported` (Karar 4 + R5; doluysa `editable = false`, üretici ad/sahip/yorum dahil her değişikliği ve drop'u
///   reddeder): `LANGUAGE c|internal`, `BEGIN ATOMIC` (prosqlbody), `AGGREGATE`, `WINDOW`, `EXTENSION e` (eklentiye ait),
///   `sistem şeması`, `SUPPORT f` (PG 12+), `TRANSFORM` (protrftypes), üretici dilbilgisinden geçmeyen tip/COST/ROWS,
///   bölünemeyen liste ayarı, bilinmeyen argüman modu. Bunlar modelin ifade edemediği ve bir `OR REPLACE`/yeniden
///   kurmanın sessizce düşüreceği özelliklerdir.
/// - Tür uyuşmazlığı (Function istenip `prokind = 'p'` ya da tersi) → `Invalid`.
async fn load_function(mgr: &ConnectionManager, server: Uuid, db: u32, kind: ObjectKind, oid: u32) -> Result<ObjectDef> {
    let (head, arg_rows) = mgr
        .run(server, Target::Database(db), move |c| async move {
            let params: [&(dyn ToSql + Sync); 1] = [&oid];
            let head = c.client.query(FUNCTION_SQL.pick(PgVersion(c.version_num())), &params).await?;
            let args = if head.is_empty() { Vec::new() } else { c.client.query(FUNCTION_ARGS_SQL, &params).await? };
            Ok((head, args))
        })
        .await?;
    let procedure = kind == ObjectKind::Procedure;
    let h = head.first().ok_or_else(|| not_found(if procedure { "procedure" } else { "fonksiyon" }))?;
    let prokind: String = h.get("kind");
    if (prokind == "p") != procedure {
        return Err(Error::Invalid(if procedure { "nesne bir procedure değil" } else { "nesne bir procedure; fonksiyon değil" }.to_string()));
    }
    let rows: Vec<ProcArg> = arg_rows
        .iter()
        .map(|r| ProcArg { mode: r.get("mode"), name: r.get("name"), type_name: lossy(r, "type_name"), default: lossy_opt(r, "default") })
        .collect();
    let (args, table, mut unsupported) = routine_args(&rows);
    note_invalid_utf8(&[&head, &arg_rows], &mut unsupported);

    let language: String = h.get("language");
    if matches!(language.as_str(), "c" | "internal") {
        unsupported.push(format!("LANGUAGE {language}"));
    }
    if h.get::<_, bool>("sql_body") {
        unsupported.push("BEGIN ATOMIC (SQL standart gövde)".to_string());
    }
    match prokind.as_str() {
        "a" => unsupported.push("AGGREGATE".to_string()),
        "w" => unsupported.push("WINDOW".to_string()),
        _ => {}
    }
    if let Some(e) = h.get::<_, Option<String>>("extension") {
        unsupported.push(format!("EXTENSION {e} (eklentiye ait)"));
    }
    if h.get::<_, bool>("is_system") {
        unsupported.push("sistem şeması".to_string());
    }
    if let Some(s) = h.get::<_, Option<String>>("support") {
        unsupported.push(format!("SUPPORT {s}"));
    }
    if h.get::<_, bool>("has_transform") {
        unsupported.push("TRANSFORM FOR TYPE".to_string());
    }

    let retset: bool = h.get("retset");
    let returns = if procedure {
        None
    } else if let Some(columns) = table {
        Some(FunctionReturn::Table { columns })
    } else if h.get::<_, bool>("ret_trigger") && !retset {
        Some(FunctionReturn::Trigger)
    } else if h.get::<_, bool>("ret_void") && !retset {
        Some(FunctionReturn::Void)
    } else {
        Some(FunctionReturn::Type { type_name: lossy(h, "ret_type"), setof: retset })
    };
    // Üreticinin tip dilbilgisinden geçmeyen yüklü tip ifadeye yazılamaz (ALTER/DROP imzası dahil).
    let mut types: Vec<&str> = args.iter().map(|a| a.type_name.as_str()).collect();
    match &returns {
        Some(FunctionReturn::Type { type_name, .. }) => types.push(type_name),
        Some(FunctionReturn::Table { columns }) => types.extend(columns.iter().map(|c| c.type_name.as_str())),
        _ => {}
    }
    for t in types {
        if crate::build::type_syntax::validate_type(t).is_err() {
            unsupported.push(format!("tip {t}"));
        }
    }

    let volatility = match h.get::<_, String>("volatility").as_str() {
        "i" => Volatility::Immutable,
        "s" => Volatility::Stable,
        _ => Volatility::Volatile,
    };
    let parallel = match h.get::<_, String>("parallel").as_str() {
        "s" => Parallel::Safe,
        "r" => Parallel::Restricted,
        _ => Parallel::Unsafe,
    };
    let (cost, rows) = if procedure {
        (None, None)
    } else {
        let cost = Some(positive_text("COST", h.get("cost"), &mut unsupported));
        let prorows: f32 = h.get("rows");
        let rows = (retset && prorows > 0.0).then(|| positive_text("ROWS", prorows, &mut unsupported));
        (cost, rows)
    };
    let settings = config_settings(&lossy_vec(h, "settings"));
    unsupported.extend(unsplittable_settings(&settings));

    let def = FunctionDef {
        schema: h.get("schema"),
        name: h.get("name"),
        owner: Some(h.get("owner")),
        comment: h.get::<_, Option<String>>("comment").filter(|c| !c.is_empty()),
        language,
        args,
        returns,
        body: h.get("body"),
        volatility,
        strict: h.get("strict"),
        security_definer: h.get("security_definer"),
        leakproof: h.get("leakproof"),
        parallel,
        cost,
        rows,
        settings,
        recreate: false,
        editable: unsupported.is_empty(),
        unsupported,
    };
    Ok(if procedure { ObjectDef::Procedure(def) } else { ObjectDef::Function(def) })
}

// ---------------------------------------------------------------------------------------------------------------
// Trigger
// ---------------------------------------------------------------------------------------------------------------

/// `pg_trigger.tgtype` bitleri (`trigger.h`).
const TG_ROW: i32 = 1;
const TG_BEFORE: i32 = 2;
const TG_INSERT: i32 = 4;
const TG_DELETE: i32 = 8;
const TG_UPDATE: i32 = 16;
const TG_TRUNCATE: i32 = 32;
const TG_INSTEAD: i32 = 64;

/// `tgtype` → (zamanlama, düzey, olaylar; `update_columns` boş).
fn trigger_kind(tgtype: i32) -> (TriggerTiming, TriggerLevel, TriggerEvents) {
    let timing = if tgtype & TG_INSTEAD != 0 {
        TriggerTiming::InsteadOf
    } else if tgtype & TG_BEFORE != 0 {
        TriggerTiming::Before
    } else {
        TriggerTiming::After
    };
    let level = if tgtype & TG_ROW != 0 { TriggerLevel::Row } else { TriggerLevel::Statement };
    let events = TriggerEvents {
        insert: tgtype & TG_INSERT != 0,
        update: tgtype & TG_UPDATE != 0,
        update_columns: vec![],
        delete: tgtype & TG_DELETE != 0,
        truncate: tgtype & TG_TRUNCATE != 0,
    };
    (timing, level, events)
}

/// `tgargs` (her argüman `\0` ile biter) → `nargs` metin. Bayt kodlaması sunucu kodlamasıdır: UTF8 değilse yalnız ASCII
/// argümanlar güvenle çözülür. Çözülemezse `None` (salt okunur gerekçesi).
fn trigger_args(raw: &[u8], nargs: i32, utf8: bool) -> Option<Vec<String>> {
    let n = usize::try_from(nargs).ok()?;
    if n == 0 {
        return Some(vec![]);
    }
    // Yalnız `\0` ile SONLANAN parçalar argümandır (son parça sonlandırıcıdan sonraki kalıntı).
    let parts: Vec<&[u8]> = raw.split(|b| *b == 0).collect();
    if parts.len() <= n {
        return None;
    }
    parts[..n].iter().map(|p| if utf8 || p.is_ascii() { String::from_utf8(p.to_vec()).ok() } else { None }).collect()
}

/// `pg_get_triggerdef(oid, true)` metninden `WHEN (…)` koşulunun iç metni (`Ok(None)` = koşul yok). `EXECUTE`'tan önce
/// yalnız çift tırnaklı tanımlayıcılar bulunabilir (dizge sabiti yok); bunların içi atlanır. Koşul gövdesinde parantez
/// dengesi `'…'` ve `"…"` içleri sayılmadan izlenir (ruleutils `E'…'` üretmez). Üretici `WHEN (<ifade>\n)` yazar ve PG
/// ifadeyi yeniden ayrıştırıp yazdırır → yüklü metin sabit noktadır. Biçim beklenmedikse `Err` (salt okunur gerekçesi).
fn trigger_when(def: &str) -> std::result::Result<Option<String>, ()> {
    const WHEN: &[u8] = b" WHEN (";
    const EXECUTE: &[u8] = b" EXECUTE ";
    let b = def.as_bytes();
    let mut i = 0;
    let mut in_ident = false;
    while i < b.len() {
        let c = b[i];
        if in_ident {
            in_ident = c != b'"';
            i += 1;
            continue;
        }
        match c {
            b'"' => in_ident = true,
            b'\'' => return Err(()),
            _ if b[i..].starts_with(EXECUTE) => return Ok(None),
            _ if b[i..].starts_with(WHEN) => {
                let start = i + WHEN.len();
                let (mut depth, mut quote, mut j) = (1u32, None::<u8>, start);
                while j < b.len() {
                    let c = b[j];
                    match quote {
                        Some(q) => {
                            if c == q {
                                quote = None;
                            }
                        }
                        None => match c {
                            b'\'' | b'"' => quote = Some(c),
                            b'(' => depth += 1,
                            b')' => {
                                depth -= 1;
                                if depth == 0 {
                                    break;
                                }
                            }
                            _ => {}
                        },
                    }
                    j += 1;
                }
                if depth != 0 || !b[j + 1..].starts_with(EXECUTE) {
                    return Err(());
                }
                // `start` ve `j` ASCII baytlarının (`(`/`)`) sınırındadır.
                return Ok(Some(def[start..j].to_string()));
            }
            _ => {}
        }
        i += 1;
    }
    Err(())
}

fn trigger_enabled(c: &str) -> Option<TriggerEnabled> {
    match c {
        "O" => Some(TriggerEnabled::Origin),
        "R" => Some(TriggerEnabled::Replica),
        "A" => Some(TriggerEnabled::Always),
        "D" => Some(TriggerEnabled::Disabled),
        _ => None,
    }
}

/// Trigger yükleyicisi. Normalleştirme (üreticiyle gidiş-dönüşün ve `alter(yüklü, yüklü)` boş planının dayanağı):
/// - `tgtype` bitleri → zamanlama/düzey/olaylar; `update_columns` `tgattr` saklandığı sırayla.
/// - `when`: `pg_get_triggerdef(oid, true)`'deki `WHEN (…)` gövdesi (`trigger_when`); argümanlar `tgargs` baytları.
/// - Fonksiyon şema + ad (her zaman nitelikli yazılır); transition tablo adları `tgoldtable`/`tgnewtable`.
/// - `constraint = tgconstraint <> 0`; ertelenebilirlik `pg_constraint`'ten. Boş yorum `None`.
/// - `unsupported` (R5; doluysa `editable = false`, üretici her değişikliği ve drop'u reddeder): iç trigger
///   (`tgisinternal`; FK/kısıt, PG 11–14 bölüm klonları), bölüm klonu (PG 13+ `tgparentid`; ilk gerekçe), `FROM` başvurulan tablolu
///   constraint trigger (`tgconstrrelid`; modellenmez), görünüm/yabancı tablo trigger'ı (üretici yalnız tablo trigger'ı
///   kurar), sistem şeması, ayrıştırılamayan WHEN, çözülemeyen argümanlar, bilinmeyen `tgenabled`.
async fn load_trigger(mgr: &ConnectionManager, server: Uuid, db: u32, oid: u32) -> Result<ObjectDef> {
    let rows = fetch(mgr, server, db, &TRIGGER_SQL, oid).await?;
    let r = rows.first().ok_or_else(|| not_found("trigger"))?;
    let mut unsupported = vec![];
    // Klon önce: PG 13–14 klonu `tgisinternal` de taşır; gösterilen ilk gerekçe asıl neden olsun.
    if r.get::<_, bool>("is_clone") {
        unsupported.push("bölümlenmiş üst tablodan klonlanmış (üst tablodan yönetilir)".to_string());
    }
    if r.get::<_, bool>("is_internal") {
        unsupported.push("iç trigger (kısıt ya da sistem tarafından yönetilir)".to_string());
    }
    if let Some(t) = r.get::<_, Option<String>>("from_table") {
        unsupported.push(format!("FROM {t} (constraint trigger başvurulan tablosu)"));
    }
    match r.get::<_, String>("relkind").as_str() {
        "r" | "p" => {}
        "v" => unsupported.push("görünüm trigger'ı".to_string()),
        "f" => unsupported.push("yabancı tablo trigger'ı".to_string()),
        other => unsupported.push(format!("'{other}' türü ilişkinin trigger'ı")),
    }
    if r.get::<_, bool>("is_system") {
        unsupported.push("sistem şeması".to_string());
    }
    let (timing, level, mut events) = trigger_kind(r.get("tgtype"));
    events.update_columns = lossy_vec(r, "update_columns");
    note_invalid_utf8(&[&rows], &mut unsupported);
    let when = match trigger_when(&lossy(r, "def")) {
        Ok(w) => w,
        Err(()) => {
            unsupported.push("WHEN koşulu ayrıştırılamadı".to_string());
            None
        }
    };
    let raw: Vec<u8> = r.get("args");
    let utf8 = r.get::<_, String>("encoding").eq_ignore_ascii_case("UTF8");
    let args = trigger_args(&raw, r.get("nargs"), utf8).unwrap_or_else(|| {
        unsupported.push("trigger argümanları çözülemedi".to_string());
        vec![]
    });
    let enabled_code: String = r.get("enabled");
    let enabled = trigger_enabled(&enabled_code).unwrap_or_else(|| {
        unsupported.push(format!("etkinlik durumu '{enabled_code}'"));
        TriggerEnabled::Origin
    });
    let constraint: bool = r.get("is_constraint");
    Ok(ObjectDef::Trigger(TriggerDef {
        schema: r.get("schema"),
        table: r.get("table_name"),
        name: r.get("name"),
        timing,
        events,
        level,
        when,
        function_schema: r.get("function_schema"),
        function_name: r.get("function_name"),
        args,
        old_table: r.get("old_table"),
        new_table: r.get("new_table"),
        constraint,
        deferrable: constraint && r.get::<_, bool>("deferrable"),
        initially_deferred: constraint && r.get::<_, bool>("initially_deferred"),
        enabled,
        comment: r.get::<_, Option<String>>("comment").filter(|c| !c.is_empty()),
        editable: unsupported.is_empty(),
        unsupported,
    }))
}

// ---------------------------------------------------------------------------------------------------------------
// Type
// ---------------------------------------------------------------------------------------------------------------

/// PG 14+ otomatik multirange adı, PG'nin `makeMultirangeTypeName`'i ile bayt bayt aynı: adda `range` varsa ilk
/// geçişin önüne `multi` eklenir, yoksa ad **ham** 52 bayta kesilir (`pnstrdup`; çok baytlı karakter ortasından
/// kesebilir → geçersiz UTF-8) ve `_multirange` eklenir; sonuç `pg_mbcliplen` ile 63 bayta kırpılır.
fn default_multirange_name(range: &str) -> Vec<u8> {
    let r = range.as_bytes();
    let mut buf = match range.find("range") {
        Some(p) => [&r[..p], b"multi", &r[p..]].concat(),
        None => [&r[..r.len().min(52)], b"_multirange"].concat(),
    };
    buf.truncate(pg_utf8_mbcliplen(&buf, 63));
    buf
}

/// PG `pg_mbcliplen` (UTF-8): karakterleri öncü baytın bildirdiği uzunlukla (`pg_utf_mblen`) yürür; `limit`i aşacak
/// karakterden önce durur.
fn pg_utf8_mbcliplen(s: &[u8], limit: usize) -> usize {
    let mut clen = 0;
    while clen < s.len() && s[clen] != 0 {
        let c = s[clen];
        let l = match c {
            _ if c & 0x80 == 0 => 1,
            _ if c & 0xe0 == 0xc0 => 2,
            _ if c & 0xf0 == 0xe0 => 3,
            _ if c & 0xf8 == 0xf0 => 4,
            _ => 1,
        };
        if clen + l > limit {
            break;
        }
        clen += l;
        if clen == limit {
            break;
        }
    }
    clen.min(s.len())
}

/// Composite öznitelik satırları → model (`original = Some(ad)`); üreticinin tip dilbilgisinden geçmeyen tip gerekçesi.
fn composite_attrs(rows: &[Row], unsupported: &mut Vec<String>) -> Vec<CompositeAttr> {
    rows.iter()
        .map(|r| {
            let name: String = r.get("name");
            let type_name = lossy(r, "type_name");
            if crate::build::type_syntax::validate_type(&type_name).is_err() {
                unsupported.push(format!("tip {type_name}"));
            }
            CompositeAttr { original: Some(name.clone()), name, type_name, collation: r.get("collation") }
        })
        .collect()
}

/// Type yükleyicisi. Normalleştirme:
/// - Sahip `Some`; boş yorum `None`.
/// - Enum: etiketler `enumsortorder` sırasıyla, `original = Some(etiket)`.
/// - Composite: öznitelikler `attnum` sırasıyla (silinmişler hariç), `original = Some(ad)`; tip `format_type`;
///   collation yalnızca tipin varsayılanından farklıysa.
/// - Range: alt tip `format_type`; opclass yalnız varsayılan değilse, collation yalnız alt tipin varsayılanından
///   farklıysa; canonical/subtype_diff `regproc::text`; multirange adı (PG 14+) yalnız otomatik addan farklıysa.
/// - Domain oid'i → `Invalid` (Domain türüyle yüklenir).
/// - `unsupported` (R5; doluysa `editable = false`): base/dizi/pseudo/shell/multirange tip (gövdesi modellenmez; boş
///   composite gövdeyle döner), tablo/görünüm satır tipi (öznitelikleriyle), `EXTENSION e`, `sistem şeması`, üretici
///   dilbilgisinden geçmeyen tip ya da fonksiyon adı.
async fn load_type(mgr: &ConnectionManager, server: Uuid, db: u32, oid: u32) -> Result<ObjectDef> {
    let (head, values, attrs, range) = mgr
        .run(server, Target::Database(db), move |c| async move {
            let params: [&(dyn ToSql + Sync); 1] = [&oid];
            let head = c.client.query(TYPE_SQL, &params).await?;
            let (mut values, mut attrs, mut range) = (vec![], vec![], vec![]);
            if let Some(h) = head.first() {
                match h.get::<_, String>("typtype").as_str() {
                    "e" => values = c.client.query(ENUM_VALUES_SQL, &params).await?,
                    "c" => attrs = c.client.query(COMPOSITE_ATTRS_SQL, &params).await?,
                    "r" => range = c.client.query(RANGE_SQL.pick(PgVersion(c.version_num())), &params).await?,
                    _ => {}
                }
            }
            Ok((head, values, attrs, range))
        })
        .await?;
    let h = head.first().ok_or_else(|| not_found("tip"))?;
    let mut unsupported = vec![];
    note_invalid_utf8(&[&head, &values, &attrs, &range], &mut unsupported);
    // Multirange/dizi tipinin adı PG'nin kestiği ad olabilir (gösterim; gerekçe yukarıda).
    let name = lossy(h, "name");
    let typtype: String = h.get("typtype");
    let empty = || TypeBody::Composite { attributes: vec![] };
    let body = match typtype.as_str() {
        "d" => return Err(Error::Invalid("nesne bir domain; tip değil".to_string())),
        "e" => TypeBody::Enum {
            values: values
                .iter()
                .map(|r| {
                    let label: String = r.get("label");
                    EnumValue { original: Some(label.clone()), label }
                })
                .collect(),
        },
        "c" => {
            match h.get::<_, Option<String>>("relkind").as_deref() {
                Some("c") => {}
                Some(k) => unsupported.push(format!("tablo satır tipi (relkind '{k}'; ilişkiden yönetilir)")),
                None => unsupported.push("ilişkisi bulunamayan composite".to_string()),
            }
            TypeBody::Composite { attributes: composite_attrs(&attrs, &mut unsupported) }
        }
        "r" => match range.first() {
            None => {
                unsupported.push("range bilgisi bulunamadı".to_string());
                empty()
            }
            Some(r) => {
                let subtype = lossy(r, "subtype");
                if crate::build::type_syntax::validate_type(&subtype).is_err() {
                    unsupported.push(format!("tip {subtype}"));
                }
                let (canonical, subtype_diff): (Option<String>, Option<String>) = (r.get("canonical"), r.get("subtype_diff"));
                for f in [&canonical, &subtype_diff].into_iter().flatten() {
                    if crate::build::pgtype::parse_func_name(f).is_err() {
                        unsupported.push(format!("fonksiyon {f}"));
                    }
                }
                // Ham baytlar: otomatik ad geçersiz UTF-8 olabilir (bkz. `default_multirange_name`).
                let multirange = r
                    .get::<_, Option<Vec<u8>>>("multirange")
                    .filter(|m| *m != default_multirange_name(&name))
                    .map(|m| String::from_utf8_lossy(&m).into_owned());
                TypeBody::Range {
                    subtype,
                    subtype_opclass: r.get("opclass"),
                    collation: r.get("collation"),
                    canonical,
                    subtype_diff,
                    multirange_type_name: multirange,
                }
            }
        },
        "m" => {
            unsupported.push("MULTIRANGE (range tipinden yönetilir)".to_string());
            empty()
        }
        "b" if h.get::<_, String>("category") == "A" => {
            unsupported.push("dizi tipi (ARRAY; öğe tipinden yönetilir)".to_string());
            empty()
        }
        "b" => {
            unsupported.push("BASE TYPE".to_string());
            empty()
        }
        "p" if !h.get::<_, bool>("defined") => {
            unsupported.push("SHELL TYPE (tanımsız)".to_string());
            empty()
        }
        "p" => {
            unsupported.push("PSEUDO TYPE".to_string());
            empty()
        }
        other => {
            unsupported.push(format!("'{other}' türü tip"));
            empty()
        }
    };
    if let Some(e) = h.get::<_, Option<String>>("extension") {
        unsupported.push(format!("EXTENSION {e} (eklentiye ait)"));
    }
    if h.get::<_, bool>("is_system") {
        unsupported.push("sistem şeması".to_string());
    }
    Ok(ObjectDef::Type(TypeDef {
        schema: h.get("schema"),
        name,
        owner: Some(h.get("owner")),
        comment: h.get::<_, Option<String>>("comment").filter(|c| !c.is_empty()),
        body,
        editable: unsupported.is_empty(),
        unsupported,
    }))
}

// ---------------------------------------------------------------------------------------------------------------
// Domain
// ---------------------------------------------------------------------------------------------------------------

/// `pg_get_constraintdef(oid, true)` CHECK metni (`CHECK (<ifade>)[ NOT VALID]`) → iç ifade. Üretici `CHECK (<ifade>\n)`
/// yazar ve PG ifadeyi yeniden yazdırır → yüklü metin sabit noktadır. Biçim beklenmedikse `None`.
fn domain_check_expr(def: &str) -> Option<String> {
    let d = def.trim();
    let d = d.strip_suffix(" NOT VALID").unwrap_or(d);
    Some(d.strip_prefix("CHECK (")?.strip_suffix(')')?.to_string())
}

/// Domain yükleyicisi. Normalleştirme:
/// - Sahip `Some`; boş yorum `None`; temel tip `format_type` (değiştiricili); collation yalnız temel tipin
///   varsayılanından farklıysa; varsayılan `pg_get_expr(typdefaultbin, 0, true)` (tamamını saran dış parantez soyulur);
///   `not_null = typnotnull`.
/// - Kısıtlar (yalnız CHECK) ad sırasıyla (bayt): `expression` = `domain_check_expr`, `validated = convalidated`,
///   `not_valid = false`, `original = Some(ad)`, `validate = false`. PG 17+ `contype = 'n'` NOT NULL satırları listeye
///   girmez (`not_null` ile modellenir); doğrulanmamış NOT NULL satırı ve tanınmayan kısıt `unsupported`'a.
/// - Domain olmayan tip oid'i → `Invalid`.
/// - `unsupported` (R5): `EXTENSION e`, `sistem şeması`, dilbilgisinden geçmeyen temel tip, ayrıştırılamayan kısıt.
async fn load_domain(mgr: &ConnectionManager, server: Uuid, db: u32, oid: u32) -> Result<ObjectDef> {
    let (head, cons) = mgr
        .run(server, Target::Database(db), move |c| async move {
            let params: [&(dyn ToSql + Sync); 1] = [&oid];
            let head = c.client.query(DOMAIN_SQL, &params).await?;
            let cons = if head.is_empty() { vec![] } else { c.client.query(DOMAIN_CHECKS_SQL, &params).await? };
            Ok((head, cons))
        })
        .await?;
    let h = head.first().ok_or_else(|| not_found("domain"))?;
    if h.get::<_, String>("typtype") != "d" {
        return Err(Error::Invalid("nesne bir domain değil".to_string()));
    }
    let mut unsupported = vec![];
    note_invalid_utf8(&[&head, &cons], &mut unsupported);
    let base_type = lossy(h, "base_type");
    // Kayıplı (yalnızca gösterim) tip metni dilbilgisinden geçmez; o durumda asıl gerekçe yukarıdaki UTF-8 gerekçesidir
    // (tek gerekçe kalsın: DROP yalnızca adı yazar ve izinlidir, bkz. `build::droppable_despite_invalid_utf8`).
    let lossy_text = unsupported.iter().any(|u| u == INVALID_UTF8_REASON);
    if !lossy_text && crate::build::type_syntax::validate_type(&base_type).is_err() {
        unsupported.push(format!("tip {base_type}"));
    }
    let mut constraints = vec![];
    for r in &cons {
        let name: String = r.get("name");
        let validated: bool = r.get("validated");
        match r.get::<_, String>("contype").as_str() {
            "c" => match domain_check_expr(&lossy(r, "def")) {
                Some(expression) => constraints.push(DomainCheck {
                    name: Some(name.clone()),
                    expression,
                    not_valid: false,
                    validated,
                    original: Some(name),
                    validate: false,
                }),
                None => unsupported.push(format!("CHECK {name} (ayrıştırılamadı)")),
            },
            "n" if validated => {}
            "n" => unsupported.push(format!("NOT NULL {name} (NOT VALID)")),
            other => unsupported.push(format!("'{other}' türü kısıt {name}")),
        }
    }
    constraints.sort_by(|a, b| a.name.cmp(&b.name));
    if let Some(e) = h.get::<_, Option<String>>("extension") {
        unsupported.push(format!("EXTENSION {e} (eklentiye ait)"));
    }
    if h.get::<_, bool>("is_system") {
        unsupported.push("sistem şeması".to_string());
    }
    Ok(ObjectDef::Domain(DomainDef {
        schema: h.get("schema"),
        name: h.get("name"),
        owner: Some(h.get("owner")),
        comment: h.get::<_, Option<String>>("comment").filter(|c| !c.is_empty()),
        base_type,
        collation: h.get("collation"),
        default: lossy_opt(h, "default").map(|d| strip_outer_parens(&d)),
        not_null: h.get("not_null"),
        constraints,
        editable: unsupported.is_empty(),
        unsupported,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_settings_split_lowercase_and_sort() {
        let s = |v: &[&str]| v.iter().map(|x| x.to_string()).collect::<Vec<_>>();
        let got = config_settings(&s(&["work_mem=64MB", "TimeZone=UTC", "search_path=\"$user\", public", "x.y=a=b", "odd"]));
        let pairs: Vec<(&str, &str)> = got.iter().map(|c| (c.name.as_str(), c.value.as_str())).collect();
        assert_eq!(
            pairs,
            [("odd", ""), ("search_path", "\"$user\", public"), ("timezone", "UTC"), ("work_mem", "64MB"), ("x.y", "a=b")]
        );
    }

    #[test]
    fn membership_labels_round_trip_to_the_role_name() {
        assert_eq!(grantor_label("app", "creator"), "MEMBER app (grantor creator)");
        assert_eq!(multi_grantor_label("app"), "MEMBER app (birden çok grantor)");
        for role in ["app", "Ünï kod", "a)b", "x (y)"] {
            assert_eq!(unsupported_membership_role(&grantor_label(role, "g")), Some(role), "{role}");
            assert_eq!(unsupported_membership_role(&grantor_label(role, "g (grantor h)")), Some(role), "{role}");
            assert_eq!(unsupported_membership_role(&multi_grantor_label(role)), Some(role), "{role}");
        }
        for other in ["IN DATABASE d: work_mem=1", "MEMBER app", "INDEX m_i", "MEMBER app (grantor g"] {
            assert_eq!(unsupported_membership_role(other), None, "{other}");
        }
    }

    #[test]
    fn config_settings_keeps_custom_names_and_folds_ascii_only() {
        let s = |v: &[&str]| v.iter().map(|x| x.to_string()).collect::<Vec<_>>();
        let got = config_settings(&s(&["App.Mode=x", "DateStyle=ISO, MDY", "ÇAĞ=1", "ext.Ölçü=2", "pg_stat_statements.track=all"]));
        let names: Vec<&str> = got.iter().map(|c| c.name.as_str()).collect();
        // Noktalı adlar korunur; noktasızda yalnızca ASCII harfler küçülür.
        assert_eq!(names, ["App.Mode", "datestyle", "ext.Ölçü", "pg_stat_statements.track", "ÇaĞ"]);
    }

    #[test]
    fn locale_provider_codes() {
        assert_eq!(locale_provider("c"), Some(LocaleProvider::Libc));
        assert_eq!(locale_provider("i"), Some(LocaleProvider::Icu));
        assert_eq!(locale_provider("b"), Some(LocaleProvider::Builtin));
        assert_eq!(locale_provider("d"), None);
    }

    #[test]
    fn view_query_strips_one_trailing_semicolon_only() {
        assert_eq!(view_query(" SELECT 1 AS x;"), "SELECT 1 AS x");
        assert_eq!(view_query(" SELECT t.a,\n    t.b\n   FROM t;\n"), "SELECT t.a,\n    t.b\n   FROM t");
        assert_eq!(view_query(" SELECT ';'::text AS s;"), "SELECT ';'::text AS s");
        assert_eq!(view_query("SELECT 1 ;"), "SELECT 1");
        assert_eq!(view_query("SELECT 1"), "SELECT 1");
        assert_eq!(view_query("SELECT 1;;"), "SELECT 1;");
        // Dış parantez korunur.
        assert_eq!(view_query(" (SELECT 1) UNION (SELECT 2);"), "(SELECT 1) UNION (SELECT 2)");
    }

    #[test]
    fn view_options_follow_reloptions() {
        let s = |v: &[&str]| v.iter().map(|x| x.to_string()).collect::<Vec<_>>();
        assert_eq!(view_options(&[]), (None, false, false, vec![]));
        assert_eq!(
            view_options(&s(&["check_option=local", "security_barrier=true", "security_invoker=on"])),
            (Some(CheckOption::Local), true, true, vec![])
        );
        assert_eq!(view_options(&s(&["check_option=CASCADED"])).0, Some(CheckOption::Cascaded));
        for f in ["false", "off", "0", "no", "f"] {
            assert!(!view_options(&s(&[&format!("security_barrier={f}")])).1, "{f}");
        }
        for t in ["true", "on", "1", "yes", "t", "TRUE"] {
            assert!(view_options(&s(&[&format!("security_invoker={t}")])).2, "{t}");
        }
        assert_eq!(view_options(&s(&["x=1"])).3, ["WITH (x=1)"]);
    }

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

    fn parg(mode: &str, name: Option<&str>, t: &str, default: Option<&str>) -> ProcArg {
        ProcArg { mode: mode.into(), name: name.map(Into::into), type_name: t.into(), default: default.map(Into::into) }
    }

    #[test]
    fn routine_args_split_table_columns_and_modes() {
        let rows = [
            parg("i", Some("a"), "integer", None),
            parg("b", None, "text", Some("'x'::text")),
            parg("v", Some("v"), "integer[]", None),
            parg("o", Some("o"), "numeric", None),
        ];
        let (args, table, unsupported) = routine_args(&rows);
        let modes: Vec<ArgMode> = args.iter().map(|a| a.mode).collect();
        assert_eq!(modes, [ArgMode::In, ArgMode::Inout, ArgMode::Variadic, ArgMode::Out]);
        assert_eq!((args[1].name.clone(), args[1].default.as_deref()), (None, Some("'x'::text")));
        assert_eq!((table, unsupported.len()), (None, 0));
        // TABLE kolonları argüman listesinden çıkar.
        let rows = [parg("i", Some("a"), "integer", Some("1")), parg("t", Some("x"), "integer", None), parg("t", Some("Y z"), "text", None)];
        let (args, table, unsupported) = routine_args(&rows);
        assert_eq!(args.len(), 1);
        let cols: Vec<(String, String)> = table.unwrap().into_iter().map(|c| (c.name, c.type_name)).collect();
        assert_eq!(cols, [("x".to_string(), "integer".to_string()), ("Y z".to_string(), "text".to_string())]);
        assert!(unsupported.is_empty());
        // Bilinmeyen mod ve adsız TABLE kolonu salt okunur gerekçesi.
        let (args, _, unsupported) = routine_args(&[parg("z", None, "integer", None), parg("t", None, "integer", None)]);
        assert!(args.is_empty());
        assert_eq!(unsupported, ["argüman modu 'z'", "adsız TABLE kolonu"]);
    }

    #[test]
    fn float_text_has_no_exponent_and_passes_builder_grammar() {
        for (v, want) in [(100.0f32, "100"), (1e10, "10000000000"), (0.5, "0.5"), (0.1, "0.1"), (1000.0, "1000"), (2.5e-5, "0.000025")] {
            assert_eq!(float_text(v), want);
            assert!(crate::build::routine::parse_positive(want).is_some(), "{want}");
        }
        // Dilbilgisinin 32 karakter sınırını aşan değer salt okunur gerekçesi olur.
        let mut u = vec![];
        let t = positive_text("COST", 1e35, &mut u);
        assert!(!t.contains('e') && t.len() > 32);
        assert_eq!(u, [format!("COST {t}")]);
        let mut u = vec![];
        assert_eq!(positive_text("ROWS", 1000.0, &mut u), "1000");
        assert!(u.is_empty());
    }

    #[test]
    fn unsplittable_list_settings_are_read_only() {
        let s = |n: &str, v: &str| ConfigSetting { name: n.into(), value: v.into() };
        assert!(unsplittable_settings(&[s("search_path", "\"$user\", public"), s("work_mem", "a b"), s("app.x", "\"")]).is_empty());
        assert_eq!(unsplittable_settings(&[s("search_path", "a b")]), ["SET search_path (liste değeri ayrıştırılamıyor)"]);
    }

    #[test]
    fn trigger_kind_decodes_tgtype_bits() {
        // BEFORE ROW INSERT|UPDATE
        let (t, l, e) = trigger_kind(TG_ROW | TG_BEFORE | TG_INSERT | TG_UPDATE);
        assert_eq!((t, l), (TriggerTiming::Before, TriggerLevel::Row));
        assert!(e.insert && e.update && !e.delete && !e.truncate);
        // AFTER STATEMENT DELETE|TRUNCATE
        let (t, l, e) = trigger_kind(TG_DELETE | TG_TRUNCATE);
        assert_eq!((t, l), (TriggerTiming::After, TriggerLevel::Statement));
        assert!(!e.insert && !e.update && e.delete && e.truncate);
        // INSTEAD OF ROW INSERT (view)
        assert_eq!(trigger_kind(TG_ROW | TG_INSTEAD | TG_INSERT).0, TriggerTiming::InsteadOf);
        // FK iç trigger'ları (gözlenen tgtype: 5, 9, 17).
        assert_eq!(trigger_kind(9).0, TriggerTiming::After);
        assert!(trigger_kind(17).2.update && trigger_kind(5).2.insert);
    }

    #[test]
    fn trigger_args_split_on_nul_exactly() {
        let raw = b"a\0it's\0c:\\yol\0\xc5\x9f \xe6\x97\xa5\0\0";
        assert_eq!(trigger_args(raw, 5, true).unwrap(), ["a", "it's", "c:\\yol", "ş 日", ""]);
        assert_eq!(trigger_args(b"", 0, true).unwrap(), Vec::<String>::new());
        assert_eq!(trigger_args(b"x\0", 1, false).unwrap(), ["x"]);
        // UTF8 olmayan sunucuda ASCII dışı bayt → çözülemez; eksik argüman; geçersiz UTF-8.
        assert_eq!(trigger_args(b"\xc5\x9f\0", 1, false), None);
        assert_eq!(trigger_args(b"a\0", 2, true), None);
        assert_eq!(trigger_args(b"\xff\0", 1, true), None);
    }

    #[test]
    fn trigger_when_extracts_condition_body() {
        let w = |s: &str| trigger_when(s);
        assert_eq!(w("CREATE TRIGGER tr BEFORE INSERT ON t FOR EACH ROW EXECUTE FUNCTION tf()"), Ok(None));
        assert_eq!(
            w("CREATE TRIGGER tr BEFORE UPDATE OF b, a ON t FOR EACH ROW WHEN (new.a > 0 AND old.b IS DISTINCT FROM new.b) EXECUTE FUNCTION tf('x', 'it''s', '')"),
            Ok(Some("new.a > 0 AND old.b IS DISTINCT FROM new.b".into()))
        );
        // Tanımlayıcı içinde " WHEN (" ve " EXECUTE " atlanır; koşulda tırnak içi parantezler sayılmaz.
        assert_eq!(
            w(r#"CREATE TRIGGER "Ad ""x"" WHEN (" BEFORE INSERT ON "T EXECUTE x" FOR EACH ROW WHEN (new."Kolon )" IS NOT NULL AND new.b <> 'x) WHEN ('::text) EXECUTE PROCEDURE f()"#),
            Ok(Some(r#"new."Kolon )" IS NOT NULL AND new.b <> 'x) WHEN ('::text"#.into()))
        );
        assert_eq!(w("CREATE TRIGGER tr AFTER INSERT ON t FOR EACH ROW WHEN ((a)) EXECUTE FUNCTION f()"), Ok(Some("(a)".into())));
        for bad in ["CREATE TRIGGER tr", "CREATE TRIGGER tr ... WHEN (a EXECUTE FUNCTION f()", "X WHEN (a) junk EXECUTE f()", "'x' EXECUTE f()"] {
            assert_eq!(w(bad), Err(()), "{bad}");
        }
    }

    #[test]
    fn default_multirange_name_follows_pg() {
        let s = |v: Vec<u8>| String::from_utf8(v).unwrap();
        assert_eq!(s(default_multirange_name("int4range")), "int4multirange");
        assert_eq!(s(default_multirange_name("f8r")), "f8r_multirange");
        assert_eq!(s(default_multirange_name("My Thing")), "My Thing_multirange");
        assert_eq!(s(default_multirange_name("a_range_range")), "a_multirange_range");
        let long = "x".repeat(60);
        assert_eq!(s(default_multirange_name(&long)), format!("{}_multirange", "x".repeat(52)));
        let long_range = format!("{}range", "y".repeat(58));
        assert_eq!(s(default_multirange_name(&long_range)), format!("{}multi", "y".repeat(58)));
        // Çok baytlı ad: 52 bayt karakter sınırına denk gelir.
        assert_eq!(s(default_multirange_name(&"ş".repeat(40))), format!("{}_multirange", "ş".repeat(26)));
        // PG ham 52 bayta keser: karakter ortası → öncü bayt (0xC5) kalır, geçersiz UTF-8 (PG 16'da doğrulandı).
        assert_eq!(
            default_multirange_name(&format!("a{}", "ş".repeat(40))),
            [format!("a{}", "ş".repeat(25)).as_bytes(), &[0xC5], b"_multirange"].concat()
        );
        // `range` içeren uzun çok baytlı ad: 63 baytta karakter sınırına geri çekilir (pg_mbcliplen).
        assert_eq!(s(default_multirange_name(&format!("xx{}range", "ş".repeat(31)))), format!("xx{}", "ş".repeat(30)));
    }

    #[test]
    fn domain_check_expr_strips_check_and_not_valid() {
        assert_eq!(domain_check_expr("CHECK (VALUE::text <> 'y'::text)").as_deref(), Some("VALUE::text <> 'y'::text"));
        assert_eq!(domain_check_expr("CHECK (VALUE > 'a'::text) NOT VALID").as_deref(), Some("VALUE > 'a'::text"));
        assert_eq!(domain_check_expr("CHECK ((VALUE > 0) OR (VALUE < -5))").as_deref(), Some("(VALUE > 0) OR (VALUE < -5)"));
        assert_eq!(domain_check_expr("NOT NULL"), None);
    }

    #[test]
    fn object_ref_is_camel_case_json() {
        let v = serde_json::to_value(ObjectRef { kind: ObjectKind::Index, oid: 42 }).unwrap();
        assert_eq!(v, serde_json::json!({"kind": "index", "oid": 42}));
    }
}
