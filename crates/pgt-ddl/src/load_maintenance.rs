//! Bakım hedefi yükleyicisi (Plan 24): katalogdaki nesne → `MaintenanceTarget` (salt okunur kimlik).
//!
//! - Tür eşlemesi: `Table` → relkind `r`/`p` (`partitioned = 'p'`), `MaterializedView` → `m`, `Index` → `i`/`I`
//!   (bölümlenmiş indeks `I` salt okunur: `PARTITIONED_INDEX_REASON`), `Schema`, `Database`. Diğer türler →
//!   `Invalid(NO_MAINTENANCE_MSG)` (katalog okunmadan); tür uyuşmazlığı ya da yok → `NotFound`.
//! - Hedef her türde `Target::Database(database_oid)`: Database türü de **veritabanı kapsamlıdır** (bakım bağlı
//!   veritabanında çalışır); istenen veritabanı bağlı olunan değilse `Invalid(WRONG_DATABASE_MSG)`.
//! - `system`: `pg_catalog`, `information_schema`, `pg_toast%` şemaları (yalnız bayrak; üretici kuralları uygular).
//! - `unsupported` (doluysa `editable = false`): geçersiz UTF-8 ad (`INVALID_UTF8_REASON`; adlar kayıplı okunur),
//!   bölümlenmiş indeks, geçici şema (uygulayıcı her zaman yeni bağlantıdır; geçici nesneler başka oturumundur).
//! - `indexes`: CLUSTER USING'de kullanılabilir indeksler, ad (bayt) sırasıyla; `cluster_index`: `indisclustered`.
//!
//! Adlar günlüğe yazılmaz.

use pgt_catalog::version::{PgVersion, VersionedSql};
use pgt_core::connection::{ConnectionManager, Target};
use pgt_core::pgtext::{lossy, lossy_opt, lossy_vec};
use pgt_core::{Error, Result};
use tokio_postgres::types::ToSql;
use uuid::Uuid;

use crate::load::{ObjectRef, note_invalid_utf8};
use crate::maintenance::{MaintenanceTarget, MaintenanceTargetKind};
use crate::model::ObjectKind;

static RELATION_SQL: VersionedSql = VersionedSql(&[(10, include_str!("sql/load_maintenance_relation.sql"))]);
static INDEX_SQL: VersionedSql = VersionedSql(&[(10, include_str!("sql/load_maintenance_index.sql"))]);
static SCHEMA_SQL: VersionedSql = VersionedSql(&[(10, include_str!("sql/load_maintenance_schema.sql"))]);
static DATABASE_SQL: VersionedSql = VersionedSql(&[(10, include_str!("sql/load_maintenance_database.sql"))]);

/// Bakım işlemi olmayan tür (view, sequence, fonksiyon …).
pub const NO_MAINTENANCE_MSG: &str = "Bu nesne için bakım işlemi yoktur.";
/// Database hedefi bağlı olunan veritabanı değil (REINDEX DATABASE / VACUUM yalnız bağlı veritabanında çalışır).
pub const WRONG_DATABASE_MSG: &str = "Veritabanı bakımı yalnız bağlı olunan veritabanında yapılabilir.";
/// Bölümlenmiş indeks (relkind `I`): ilk sürümde salt okunur.
pub const PARTITIONED_INDEX_REASON: &str = "bölümlenmiş indeks";
/// Geçici şema (`pg_temp_N` / `pg_toast_temp_N`): nesneler başka oturumundur; PG bakımı atlar ya da reddeder.
pub const TEMP_SCHEMA_REASON: &str = "geçici şema (başka oturumun nesneleri)";

/// `ObjectKind` → bakım hedefi türü; bakımı olmayan tür `None`.
pub(crate) fn target_kind(kind: ObjectKind) -> Option<MaintenanceTargetKind> {
    match kind {
        ObjectKind::Table => Some(MaintenanceTargetKind::Table),
        ObjectKind::MaterializedView => Some(MaintenanceTargetKind::MaterializedView),
        ObjectKind::Index => Some(MaintenanceTargetKind::Index),
        ObjectKind::Schema => Some(MaintenanceTargetKind::Schema),
        ObjectKind::Database => Some(MaintenanceTargetKind::Database),
        _ => None,
    }
}

/// Katalogdaki alt tür istenen hedef türüne uyuyor mu (şema/veritabanı sorguları sabit alt tür döndürür).
fn relkind_matches(kind: MaintenanceTargetKind, relkind: &str) -> bool {
    use MaintenanceTargetKind::*;
    match kind {
        Table => matches!(relkind, "r" | "p"),
        MaterializedView => relkind == "m",
        Index => matches!(relkind, "i" | "I"),
        Schema => relkind == "n",
        Database => relkind == "d",
    }
}

/// Hedef türüne ve alt türüne göre `partitioned` ve salt okunurluk gerekçesi.
fn partitioned_of(kind: MaintenanceTargetKind, relkind: &str) -> (bool, Option<&'static str>) {
    match (kind, relkind) {
        (MaintenanceTargetKind::Table, "p") => (true, None),
        (MaintenanceTargetKind::Index, "I") => (true, Some(PARTITIONED_INDEX_REASON)),
        _ => (false, None),
    }
}

/// Bakım hedefini yükler. Bakımı olmayan tür → `Invalid(NO_MAINTENANCE_MSG)`; nesne yok ya da tür uyuşmazlığı →
/// `NotFound`; Database türünde istenen veritabanı bağlı olunan değil → `Invalid(WRONG_DATABASE_MSG)`.
pub async fn load_maintenance_target(mgr: &ConnectionManager, server: Uuid, database_oid: u32, obj: ObjectRef) -> Result<MaintenanceTarget> {
    let kind = target_kind(obj.kind).ok_or_else(|| Error::Invalid(NO_MAINTENANCE_MSG.to_string()))?;
    let sql: &'static VersionedSql = match kind {
        MaintenanceTargetKind::Table | MaintenanceTargetKind::MaterializedView => &RELATION_SQL,
        MaintenanceTargetKind::Index => &INDEX_SQL,
        MaintenanceTargetKind::Schema => &SCHEMA_SQL,
        MaintenanceTargetKind::Database => &DATABASE_SQL,
    };
    let oid = obj.oid;
    let rows = mgr
        .run(server, Target::Database(database_oid), move |c| async move {
            let params: [&(dyn ToSql + Sync); 1] = [&oid];
            c.client.query(sql.pick(PgVersion(c.version_num())), &params).await
        })
        .await?;
    let not_found = || Error::NotFound("nesne".to_string());
    let r = rows.first().ok_or_else(not_found)?;
    let relkind: String = r.get("relkind");
    if !relkind_matches(kind, &relkind) {
        return Err(not_found());
    }
    if kind == MaintenanceTargetKind::Database && !r.get::<_, bool>("is_current") {
        return Err(Error::Invalid(WRONG_DATABASE_MSG.to_string()));
    }

    let mut unsupported = vec![];
    note_invalid_utf8(&[&rows], &mut unsupported);
    let (partitioned, reason) = partitioned_of(kind, &relkind);
    if let Some(reason) = reason {
        unsupported.push(reason.to_string());
    }
    if r.get::<_, bool>("is_temp") {
        unsupported.push(TEMP_SCHEMA_REASON.to_string());
    }
    Ok(MaintenanceTarget {
        kind,
        schema: lossy_opt(r, "schema"),
        name: lossy(r, "name"),
        table: lossy_opt(r, "table_name"),
        partitioned,
        system: r.get("is_system"),
        cluster_index: lossy_opt(r, "cluster_index"),
        indexes: lossy_vec(r, "indexes"),
        editable: unsupported.is_empty(),
        unsupported,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use MaintenanceTargetKind as K;

    #[test]
    fn kind_mapping() {
        assert_eq!(target_kind(ObjectKind::Table), Some(K::Table));
        assert_eq!(target_kind(ObjectKind::MaterializedView), Some(K::MaterializedView));
        assert_eq!(target_kind(ObjectKind::Index), Some(K::Index));
        assert_eq!(target_kind(ObjectKind::Schema), Some(K::Schema));
        assert_eq!(target_kind(ObjectKind::Database), Some(K::Database));
        for k in [
            ObjectKind::View,
            ObjectKind::Sequence,
            ObjectKind::Extension,
            ObjectKind::Role,
            ObjectKind::Function,
            ObjectKind::Procedure,
            ObjectKind::Trigger,
            ObjectKind::Type,
            ObjectKind::Domain,
        ] {
            assert_eq!(target_kind(k), None, "{k:?}");
        }
    }

    #[test]
    fn relkind_matching() {
        assert!(relkind_matches(K::Table, "r") && relkind_matches(K::Table, "p"));
        for rk in ["v", "m", "S", "i", "I", "f", "t", "c"] {
            assert!(!relkind_matches(K::Table, rk), "{rk}");
        }
        assert!(relkind_matches(K::MaterializedView, "m") && !relkind_matches(K::MaterializedView, "r"));
        assert!(relkind_matches(K::Index, "i") && relkind_matches(K::Index, "I") && !relkind_matches(K::Index, "r"));
        assert!(relkind_matches(K::Schema, "n") && relkind_matches(K::Database, "d"));
    }

    #[test]
    fn partitioned_flags() {
        assert_eq!(partitioned_of(K::Table, "p"), (true, None));
        assert_eq!(partitioned_of(K::Table, "r"), (false, None));
        assert_eq!(partitioned_of(K::Index, "I"), (true, Some(PARTITIONED_INDEX_REASON)));
        assert_eq!(partitioned_of(K::Index, "i"), (false, None));
        assert_eq!(partitioned_of(K::MaterializedView, "m"), (false, None));
    }
}
