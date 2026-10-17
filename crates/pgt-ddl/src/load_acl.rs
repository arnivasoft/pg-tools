//! Nesne yetkileri yükleyicisi (Plan 20 Karar 3–5, 11): katalogdaki etkin ACL → `AclDef`.
//!
//! Normalleştirme (üreticiyle gidiş-dönüşün ve `privileges(yüklü, yüklü)` boş planının dayanağı):
//! - Etkin ACL: NULL ACL `acldefault(<harf>, <sahip>)` ile açık gösterilir; `explicit = <acl> IS NOT NULL`.
//! - Veren = sahip girdileri `grants`'a (alıcı başına tek satır, yetkiler kanonik sırada; `PUBLIC` önce, sonra ad bayt
//!   sırası — `sort_grants`); başka verenlerinkiler `foreign`'a (veren, alıcı, kanonik yetki sırasıyla).
//! - Kolon ACL'leri (yalnız Table/View/MV, açık `attacl`) `columns`'a: kolon `attnum`, alıcı, veren, yetki sırasıyla.
//! - Adlar ham metin (tırnaklama üreticide); imza `pg_get_function_identity_arguments` (yalnız Function/Procedure).
//! - Tür uyuşmazlığı (Table istenip relkind 'S', Function istenip procedure, Type istenip domain …) → `NotFound`.
//! - `unsupported` (doluysa `editable = false`): `sistem şeması`, dizi/multirange/tablo satır tipi, bilinmeyen yetki
//!   (`yetki <metin>`), denetim karakterli imza (`imza`), geçersiz UTF-8 ad (`INVALID_UTF8_REASON`; adlar kayıplı).
//!
//! Database bakım bağlantısından okunur (`database_oid` yok sayılır); diğerleri veritabanı gerektirir. Adlar/yetkiler
//! günlüğe yazılmaz.

use pgt_catalog::version::{PgVersion, VersionedSql};
use pgt_core::connection::ConnectionManager;
use pgt_core::pgtext::{lossy, lossy_opt};
use pgt_core::{Error, Result};
use tokio_postgres::types::ToSql;
use uuid::Uuid;

use crate::acl::{AclDef, AclGrant, ColumnGrant, ForeignGrant, Privilege, PrivilegeGrant, acl_supported, privileges_for, sort_grants};
use crate::load::{ObjectRef, note_invalid_utf8, target_for};
use crate::model::ObjectKind;

static RELATION_SQL: VersionedSql = VersionedSql(&[(10, include_str!("sql/load_acl_relation.sql"))]);
/// PG 10: `prokind` yok.
static PROC_SQL: VersionedSql = VersionedSql(&[
    (10, include_str!("sql/load_acl_proc.pg10.sql")),
    (11, include_str!("sql/load_acl_proc.sql")),
]);
static NAMESPACE_SQL: VersionedSql = VersionedSql(&[(10, include_str!("sql/load_acl_namespace.sql"))]);
static DATABASE_SQL: VersionedSql = VersionedSql(&[(10, include_str!("sql/load_acl_database.sql"))]);
static TYPE_SQL: VersionedSql = VersionedSql(&[(10, include_str!("sql/load_acl_type.sql"))]);
const COLUMNS_SQL: &str = include_str!("sql/load_acl_columns.sql");

const UNSUPPORTED_KIND_MSG: &str = "Bu nesne türünün yetkileri yoktur.";
/// Karar 11 gerekçeleri (sabit metin; ad içermez).
pub const SYSTEM_SCHEMA_REASON: &str = "sistem şeması";
pub const SIGNATURE_REASON: &str = "imza";
pub const ARRAY_REASON: &str = "dizi tipi (yetkiler öğe tipinden yönetilir)";
pub const MULTIRANGE_REASON: &str = "MULTIRANGE (yetkiler range tipinden yönetilir)";
pub const ROW_TYPE_REASON: &str = "tablo satır tipi (yetkiler ilişkiden yönetilir)";

fn not_found() -> Error {
    Error::NotFound("nesne".to_string())
}

/// `aclexplode` satırı (yetki sütunu NULL olan boş-ACL satırı dahil edilmez).
#[derive(Debug, Clone, PartialEq, Eq)]
struct Entry {
    grantee: Option<String>,
    grantor: String,
    by_owner: bool,
    privilege: String,
    grantable: bool,
}

/// Kanonik sıra konumu (tüm sürümlerin kümesi, MAINTAIN dahil).
fn rank(kind: ObjectKind, p: Privilege) -> usize {
    privileges_for(kind, 170000).iter().position(|o| *o == p).unwrap_or(usize::MAX)
}

/// Karar 3: türün kümesinde olmayan ya da tanınmayan yetki metni → `unsupported` (`yetki <metin>`, bir kez).
fn known_privilege(kind: ObjectKind, text: &str, unsupported: &mut Vec<String>) -> Option<Privilege> {
    let p = Privilege::from_acl(text).filter(|p| privileges_for(kind, 170000).contains(p));
    if p.is_none() {
        let reason = format!("yetki {text}");
        if !unsupported.contains(&reason) {
            unsupported.push(reason);
        }
    }
    p
}

/// Karar 5: sahibin verdikleri `grants`'a (alıcı başına tek satır, kanonik sıra), diğerleri `foreign`'a.
fn split_entries(kind: ObjectKind, entries: Vec<Entry>, unsupported: &mut Vec<String>) -> (Vec<AclGrant>, Vec<ForeignGrant>) {
    let (mut grants, mut foreign): (Vec<AclGrant>, Vec<ForeignGrant>) = (vec![], vec![]);
    for e in entries {
        let Some(privilege) = known_privilege(kind, &e.privilege, unsupported) else { continue };
        if !e.by_owner {
            foreign.push(ForeignGrant { grantor: e.grantor, grantee: e.grantee, privilege, grant_option: e.grantable });
            continue;
        }
        let pg = PrivilegeGrant { privilege, grant_option: e.grantable };
        match grants.iter_mut().find(|g| g.grantee == e.grantee) {
            Some(g) => g.privileges.push(pg),
            None => grants.push(AclGrant { grantee: e.grantee, privileges: vec![pg] }),
        }
    }
    sort_grants(&mut grants, privileges_for(kind, 170000));
    foreign.sort_by(|a, b| {
        (&a.grantor, &a.grantee, rank(kind, a.privilege)).cmp(&(&b.grantor, &b.grantee, rank(kind, b.privilege)))
    });
    (grants, foreign)
}

/// Kolon yetkileri: `attnum`, alıcı (`PUBLIC` önce), veren, kanonik yetki sırası.
fn sort_columns(kind: ObjectKind, mut keyed: Vec<(i32, ColumnGrant)>) -> Vec<ColumnGrant> {
    keyed.sort_by(|(a, x), (b, y)| {
        (a, &x.grantee, &x.grantor, rank(kind, x.privilege)).cmp(&(b, &y.grantee, &y.grantor, rank(kind, y.privilege)))
    });
    keyed.into_iter().map(|(_, c)| c).collect()
}

/// İstenen tür ile katalogdaki alt tür (`relkind`/`prokind`/`typtype`) uyumlu mu.
fn kind_matches(kind: ObjectKind, subkind: &str) -> bool {
    match kind {
        // Bölümlenmiş tablo da Table'dır (Karar 1); yabancı tablo backlog.
        ObjectKind::Table => matches!(subkind, "r" | "p"),
        ObjectKind::View => subkind == "v",
        ObjectKind::MaterializedView => subkind == "m",
        ObjectKind::Sequence => subkind == "S",
        // Aggregate/window da `ON FUNCTION` ile yetkilendirilir.
        ObjectKind::Function => matches!(subkind, "f" | "a" | "w"),
        ObjectKind::Procedure => subkind == "p",
        ObjectKind::Type => subkind != "d",
        ObjectKind::Domain => subkind == "d",
        ObjectKind::Schema | ObjectKind::Database => true,
        _ => false,
    }
}

/// Karar 11: yetkisi doğrudan düzenlenemeyen tipler (PG dizi/multirange için reddeder; satır tipi ilişkiden yönetilir).
fn type_reason(subkind: &str, is_array: bool, row_relkind: Option<&str>) -> Option<&'static str> {
    if is_array {
        Some(ARRAY_REASON)
    } else if subkind == "m" {
        Some(MULTIRANGE_REASON)
    } else if subkind == "c" && row_relkind != Some("c") {
        Some(ROW_TYPE_REASON)
    } else {
        None
    }
}

/// Nesnenin etkin ACL'sini okur. Yetkisi olmayan tür → `Invalid`; yok ya da tür uyuşmazlığı → `NotFound`;
/// veritabanı gerektiren türde `database_oid` yok → `Invalid(NEEDS_DATABASE)` (`target_for`).
pub async fn load_acl(mgr: &ConnectionManager, server: Uuid, database_oid: impl Into<Option<u32>> + Send, obj: ObjectRef) -> Result<AclDef> {
    let kind = obj.kind;
    if !acl_supported(kind) {
        return Err(Error::Invalid(UNSUPPORTED_KIND_MSG.to_string()));
    }
    let target = target_for(kind, database_oid.into())?;
    let sql: &'static VersionedSql = match kind {
        ObjectKind::Function | ObjectKind::Procedure => &PROC_SQL,
        ObjectKind::Schema => &NAMESPACE_SQL,
        ObjectKind::Database => &DATABASE_SQL,
        ObjectKind::Type | ObjectKind::Domain => &TYPE_SQL,
        _ => &RELATION_SQL,
    };
    let with_columns = matches!(kind, ObjectKind::Table | ObjectKind::View | ObjectKind::MaterializedView);
    let oid = obj.oid;
    let (head, cols) = mgr
        .run(server, target, move |c| async move {
            let params: [&(dyn ToSql + Sync); 1] = [&oid];
            let head = c.client.query(sql.pick(PgVersion(c.version_num())), &params).await?;
            let cols = if with_columns && !head.is_empty() { c.client.query(COLUMNS_SQL, &params).await? } else { Vec::new() };
            Ok((head, cols))
        })
        .await?;
    let h = head.first().ok_or_else(not_found)?;
    let subkind: String = h.get("subkind");
    if !kind_matches(kind, &subkind) {
        return Err(not_found());
    }

    let mut unsupported = vec![];
    note_invalid_utf8(&[&head, &cols], &mut unsupported);
    if h.get::<_, bool>("is_system") {
        unsupported.push(SYSTEM_SCHEMA_REASON.to_string());
    }
    if kind == ObjectKind::Type {
        let row_relkind: Option<String> = h.get("row_relkind");
        if let Some(r) = type_reason(&subkind, h.get("is_array"), row_relkind.as_deref()) {
            unsupported.push(r.to_string());
        }
    }
    let signature = lossy_opt(h, "signature");
    // NUL/denetim karakterli imza ifadeye güvenle yazılamaz (Global Constraints).
    if signature.as_deref().is_some_and(|s| s.chars().any(char::is_control)) {
        unsupported.push(SIGNATURE_REASON.to_string());
    }

    let entries = head
        .iter()
        .filter_map(|r| {
            let privilege: Option<String> = r.get("privilege");
            Some(Entry {
                grantee: lossy_opt(r, "grantee"),
                grantor: lossy(r, "grantor"),
                by_owner: r.get("by_owner"),
                privilege: privilege?,
                grantable: r.get("grantable"),
            })
        })
        .collect();
    let (grants, foreign) = split_entries(kind, entries, &mut unsupported);
    let mut keyed = vec![];
    for r in &cols {
        let text: String = r.get("privilege");
        let Some(privilege) = known_privilege(kind, &text, &mut unsupported) else { continue };
        let column = ColumnGrant {
            column: lossy(r, "column_name"),
            grantor: lossy(r, "grantor"),
            grantee: lossy_opt(r, "grantee"),
            privilege,
            grant_option: r.get("grantable"),
        };
        keyed.push((r.get::<_, i32>("attnum"), column));
    }

    Ok(AclDef {
        kind,
        schema: lossy_opt(h, "schema"),
        name: lossy(h, "name"),
        signature,
        owner: lossy(h, "owner"),
        viewer: lossy(h, "viewer"),
        explicit: h.get("explicit"),
        grants,
        foreign,
        columns: sort_columns(kind, keyed),
        editable: unsupported.is_empty(),
        unsupported,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use Privilege::*;

    fn e(grantee: Option<&str>, grantor: &str, by_owner: bool, privilege: &str, grantable: bool) -> Entry {
        Entry { grantee: grantee.map(Into::into), grantor: grantor.into(), by_owner, privilege: privilege.into(), grantable }
    }

    #[test]
    fn split_groups_owner_grants_and_separates_foreign() {
        let mut u = vec![];
        let (grants, foreign) = split_entries(
            ObjectKind::Table,
            vec![
                e(Some("o"), "o", true, "UPDATE", false),
                e(Some("b"), "o", true, "SELECT", true),
                e(Some("o"), "o", true, "SELECT", false),
                e(None, "o", true, "INSERT", false),
                e(Some("c"), "b", false, "UPDATE", false),
                e(Some("c"), "b", false, "SELECT", true),
                e(Some("c"), "a", false, "SELECT", false),
            ],
            &mut u,
        );
        assert!(u.is_empty());
        let who: Vec<_> = grants.iter().map(|g| g.grantee.as_deref()).collect();
        assert_eq!(who, [None, Some("b"), Some("o")]);
        assert_eq!(grants[2].privileges, [PrivilegeGrant { privilege: Select, grant_option: false }, PrivilegeGrant { privilege: Update, grant_option: false }]);
        assert_eq!(grants[1].privileges, [PrivilegeGrant { privilege: Select, grant_option: true }]);
        let f: Vec<_> = foreign.iter().map(|f| (f.grantor.as_str(), f.privilege, f.grant_option)).collect();
        assert_eq!(f, [("a", Select, false), ("b", Select, true), ("b", Update, false)]);
    }

    #[test]
    fn unknown_or_foreign_kind_privilege_is_unsupported_once() {
        let mut u = vec![];
        let (grants, foreign) = split_entries(
            ObjectKind::Sequence,
            vec![e(Some("o"), "o", true, "SET", false), e(Some("x"), "o", true, "SET", false), e(Some("o"), "o", true, "TRUNCATE", false), e(Some("o"), "o", true, "USAGE", false)],
            &mut u,
        );
        assert_eq!(u, ["yetki SET", "yetki TRUNCATE"]);
        assert_eq!(grants.len(), 1);
        assert!(foreign.is_empty());
    }

    #[test]
    fn columns_sorted_by_attnum_grantee_grantor_privilege() {
        let c = |col: &str, grantee: Option<&str>, grantor: &str, privilege| ColumnGrant {
            column: col.into(),
            grantor: grantor.into(),
            grantee: grantee.map(Into::into),
            privilege,
            grant_option: false,
        };
        let v = sort_columns(
            ObjectKind::Table,
            vec![(2, c("b", Some("r"), "o", Insert)), (1, c("a", Some("r"), "o", Update)), (1, c("a", None, "o", Select)), (1, c("a", Some("r"), "o", Select))],
        );
        let k: Vec<_> = v.iter().map(|x| (x.column.as_str(), x.grantee.as_deref(), x.privilege)).collect();
        assert_eq!(k, [("a", None, Select), ("a", Some("r"), Select), ("a", Some("r"), Update), ("b", Some("r"), Insert)]);
    }

    #[test]
    fn kind_matching() {
        use ObjectKind as K;
        assert!(kind_matches(K::Table, "r") && kind_matches(K::Table, "p"));
        assert!(!kind_matches(K::Table, "S") && !kind_matches(K::Table, "f") && !kind_matches(K::Table, "i"));
        assert!(kind_matches(K::Sequence, "S") && !kind_matches(K::Sequence, "r"));
        assert!(kind_matches(K::View, "v") && kind_matches(K::MaterializedView, "m") && !kind_matches(K::View, "m"));
        assert!(kind_matches(K::Function, "f") && kind_matches(K::Function, "a") && !kind_matches(K::Function, "p"));
        assert!(kind_matches(K::Procedure, "p") && !kind_matches(K::Procedure, "f"));
        assert!(kind_matches(K::Type, "e") && !kind_matches(K::Type, "d"));
        assert!(kind_matches(K::Domain, "d") && !kind_matches(K::Domain, "e"));
        assert!(!kind_matches(K::Index, "i"));
    }

    #[test]
    fn type_read_only_reasons() {
        assert_eq!(type_reason("b", true, None), Some(ARRAY_REASON));
        assert_eq!(type_reason("m", false, None), Some(MULTIRANGE_REASON));
        assert_eq!(type_reason("c", false, Some("r")), Some(ROW_TYPE_REASON));
        assert_eq!(type_reason("c", false, None), Some(ROW_TYPE_REASON));
        assert_eq!(type_reason("c", false, Some("c")), None);
        assert_eq!(type_reason("e", false, None), None);
        assert_eq!(type_reason("r", false, None), None);
    }
}
