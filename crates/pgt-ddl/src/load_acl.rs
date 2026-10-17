//! Nesne yetkileri yükleyicisi (Plan 20 Karar 3–5, 11): katalogdaki etkin ACL → `AclDef`.
//!
//! Normalleştirme (üreticiyle gidiş-dönüşün ve `privileges(yüklü, yüklü)` boş planının dayanağı):
//! - Etkin ACL: NULL ACL `acldefault(<harf>, <sahip>)` ile açık gösterilir; `explicit = <acl> IS NOT NULL`.
//! - Veren = sahip girdileri `grants`'a (alıcı başına tek satır, yetkiler kanonik sırada; `PUBLIC` önce, sonra ad bayt
//!   sırası — `sort_grants`); başka verenlerinkiler `foreign`'a (veren, alıcı, kanonik yetki sırasıyla).
//! - Kolon ACL'leri (yalnız Table/View/MV, açık `attacl`) `columns`'a: kolon `attnum`, alıcı, veren, yetki sırasıyla.
//! - Adlar ham metin (tırnaklama üreticide); imza `pg_get_function_identity_arguments` (yalnız Function/Procedure).
//! - Tür uyuşmazlığı (Table istenip relkind 'S', Function istenip procedure, Type istenip domain …) → `NotFound`.
//! - `unsupported` (doluysa `editable = false`): `sistem şeması`, aggregate (`AGGREGATE …`; R4a), dizi/multirange/tablo
//!   satır tipi, bilinmeyen yetki (`yetki <metin>`), denetim karakterli imza (`imza`), geçersiz UTF-8 ad (`INVALID_UTF8_REASON`; adlar kayıplı).
//!
//! Database bakım bağlantısından okunur (`database_oid` yok sayılır); diğerleri veritabanı gerektirir. Adlar/yetkiler
//! günlüğe yazılmaz.
//!
//! Varsayılan yetkiler (`load_default_acl`, Karar 12): genel kapsamda her tür için etkin değer
//! `coalesce(defaclacl, acldefault(...))`; şema kapsamında `grants` yalnız şema girdileri, `baseline` genel etkin değer.
//! Bölümler `default_sections` sırasıyla ve tam; satırlar `sort_grants` ile sıralı.

use pgt_catalog::version::{PgVersion, VersionedSql};
use pgt_core::connection::ConnectionManager;
use pgt_core::pgtext::{lossy, lossy_opt};
use pgt_core::{Error, Result};
use tokio_postgres::types::ToSql;
use uuid::Uuid;

use crate::acl::{
    AclDef, AclGrant, ColumnGrant, DefaultAclDef, DefaultAclObjectType, DefaultAclScope, DefaultAclSection, ForeignGrant, Privilege, PrivilegeGrant,
    acl_supported, default_privileges_for, default_sections, privileges_for, sort_grants,
};
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

/// Karar 1: yetkisi olmayan tür (yükleyici ve yönetici kapısı aynı ileti).
pub const UNSUPPORTED_KIND_MSG: &str = "Bu nesne türünün yetkileri yoktur.";
/// Karar 11 gerekçeleri (sabit metin; ad içermez).
pub const SYSTEM_SCHEMA_REASON: &str = "sistem şeması";
pub const SIGNATURE_REASON: &str = "imza";
pub const ARRAY_REASON: &str = "dizi tipi (yetkiler öğe tipinden yönetilir)";
pub const MULTIRANGE_REASON: &str = "MULTIRANGE (yetkiler range tipinden yönetilir)";
pub const ROW_TYPE_REASON: &str = "tablo satır tipi (yetkiler ilişkiden yönetilir)";
/// R4a: aggregate salt okunur (Plan 18 `read_only_reason` "AGGREGATE" önekini tanır). Sıralı küme/varsayımsal
/// aggregate imzası `ORDER BY` içerir ve `ON FUNCTION` hedefinde ayrıştırılamaz; Explorer/Plan 18 de salt okunur sayar.
pub const AGGREGATE_REASON: &str = "AGGREGATE (aggregate yetkileri buradan düzenlenmez)";

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
        // Aggregate/window da Function olarak yüklenir (aggregate salt okunur: `routine_reason`).
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

/// Fonksiyon alt türünün salt okunurluk gerekçesi: aggregate (`prokind 'a'`; PG 10 `proisagg`). Pencere düzenlenebilir.
fn routine_reason(subkind: &str) -> Option<&'static str> {
    (subkind == "a").then_some(AGGREGATE_REASON)
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
            // Kolon yetkileri yalnız Table/View/MV ve tür eşleştiyse (uyuşmazlık NotFound döner; boşuna sorgu yok).
            let matched = head.first().is_some_and(|h| kind_matches(kind, &h.get::<_, String>("subkind")));
            let cols = if with_columns && matched { c.client.query(COLUMNS_SQL, &params).await? } else { Vec::new() };
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
    if kind == ObjectKind::Function
        && let Some(r) = routine_reason(&subkind)
    {
        unsupported.push(r.to_string());
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
            // Boş ACL ('{}'): `aclexplode` satır vermez; LEFT JOIN'in tek satırında tüm girdi sütunları NULL'dur →
            // önce yetki okunur, diğer (NULL olamayan) sütunlara dokunulmaz.
            let privilege: String = r.get::<_, Option<String>>("privilege")?;
            Some(Entry {
                grantee: lossy_opt(r, "grantee"),
                grantor: lossy(r, "grantor"),
                by_owner: r.get("by_owner"),
                privilege,
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

// ================================================================================================================
// varsayılan yetkiler (Plan 20 Karar 12)
// ================================================================================================================

static DEFAULT_SCOPE_SQL: VersionedSql = VersionedSql(&[(10, include_str!("sql/load_default_acl_scope.sql"))]);
static DEFAULT_ACL_SQL: VersionedSql = VersionedSql(&[(10, include_str!("sql/load_default_acl.sql"))]);

/// PG 18 `defaclobjtype = 'L'` (large object): modelde bölümü yok; üretici dokunmaz → bilgi notu, `editable` korunur.
pub const LARGE_OBJECTS_REASON: &str = "LARGE OBJECTS varsayılan yetkileri";
/// Veren rolden farklı girdi (PG her zaman rolü yazar; beklenmedik durum).
pub const DEFAULT_GRANTOR_REASON: &str = "veren rolden farklı";

/// Varsayılan ACL satırı (`aclexplode`; yetki sütunu NULL olan boş-ACL satırı dahil edilmez).
#[derive(Debug, Clone, PartialEq, Eq)]
struct DefaultEntry {
    /// `true`: şema girdisi (`IN SCHEMA`); `false`: genel kapsamın etkin değeri.
    schema_entry: bool,
    /// `pg_default_acl.defaclobjtype`.
    objtype: String,
    grantee: Option<String>,
    by_role: bool,
    privilege: String,
    grantable: bool,
}

fn object_type_of(letter: &str) -> Option<DefaultAclObjectType> {
    match letter {
        "r" => Some(DefaultAclObjectType::Tables),
        "S" => Some(DefaultAclObjectType::Sequences),
        "f" => Some(DefaultAclObjectType::Functions),
        "T" => Some(DefaultAclObjectType::Types),
        "n" => Some(DefaultAclObjectType::Schemas),
        _ => None,
    }
}

fn note(reason: String, unsupported: &mut Vec<String>) {
    if !unsupported.contains(&reason) {
        unsupported.push(reason);
    }
}

/// Satırları bölümlere dağıtır. Genel kapsamda genel satırlar `grants`'a; şema kapsamında şema satırları `grants`'a,
/// genel satırlar `baseline`'a (şema kapsamında `Schemas` bölümü yok: genel `n` satırları sessizce düşer).
/// Desteklenmeyen tür (`L` → bilgi notu; diğerleri `varsayılan yetki türü <harf>`), türün kümesi dışındaki ya da
/// tanınmayan yetki (`yetki <metin>`) ve rolden farklı veren → `unsupported` (bir kez).
fn assemble_sections(schema_scoped: bool, entries: Vec<DefaultEntry>, unsupported: &mut Vec<String>) -> Vec<DefaultAclSection> {
    let mut sections: Vec<DefaultAclSection> = default_sections(schema_scoped)
        .iter()
        .map(|&object_type| DefaultAclSection { object_type, grants: vec![], baseline: vec![] })
        .collect();
    for e in entries {
        let known = object_type_of(&e.objtype);
        let Some(section) = known.and_then(|t| sections.iter_mut().find(|s| s.object_type == t)) else {
            // Şema kapsamında genel `Schemas` satırı beklenir; başka her şey desteklenmeyen türdür.
            if !(known.is_some() && !e.schema_entry) {
                let reason = if e.objtype == "L" { LARGE_OBJECTS_REASON.to_string() } else { format!("varsayılan yetki türü {}", e.objtype) };
                note(reason, unsupported);
            }
            continue;
        };
        let set = default_privileges_for(section.object_type, 170000);
        let Some(privilege) = Privilege::from_acl(&e.privilege).filter(|p| set.contains(p)) else {
            note(format!("yetki {}", e.privilege), unsupported);
            continue;
        };
        if !e.by_role {
            note(DEFAULT_GRANTOR_REASON.to_string(), unsupported);
            continue;
        }
        let target = if schema_scoped && !e.schema_entry { &mut section.baseline } else { &mut section.grants };
        let pg = PrivilegeGrant { privilege, grant_option: e.grantable };
        match target.iter_mut().find(|g| g.grantee == e.grantee) {
            Some(g) => g.privileges.push(pg),
            None => target.push(AclGrant { grantee: e.grantee, privileges: vec![pg] }),
        }
    }
    for s in &mut sections {
        let order = default_privileges_for(s.object_type, 170000);
        sort_grants(&mut s.grants, order);
        sort_grants(&mut s.baseline, order);
    }
    sections
}

/// Bilgi notu dışında gerekçe varsa salt okunur.
fn default_editable(unsupported: &[String]) -> bool {
    unsupported.iter().all(|u| u == LARGE_OBJECTS_REASON)
}

/// Bir rolün (genel ya da şema kapsamlı) varsayılan yetkilerini okur. `scope.role` `None` → bağlı rol; rol yok →
/// `NotFound("rol")`; şema yok → `NotFound("şema")`; sistem şeması → salt okunur. `pg_default_acl` veritabanı başınadır:
/// hedef veritabanından okunur.
pub async fn load_default_acl(mgr: &ConnectionManager, server: Uuid, database_oid: u32, scope: DefaultAclScope) -> Result<DefaultAclDef> {
    let target = target_for(ObjectKind::Schema, Some(database_oid))?;
    let DefaultAclScope { role, schema_oid } = scope;
    let (head, rows) = mgr
        .run(server, target, move |c| {
            let role = role.clone();
            async move {
                let v = PgVersion(c.version_num());
                let params: [&(dyn ToSql + Sync); 2] = [&role, &schema_oid];
                let head = c.client.query(DEFAULT_SCOPE_SQL.pick(v), &params).await?;
                // Rol ya da şema yoksa girdi sorgusu çalışmaz.
                let Some(h) = head.first().filter(|h| h.get::<_, bool>("schema_found") || schema_oid.is_none()) else {
                    return Ok((head, Vec::new()));
                };
                let role_oid: u32 = h.get("role_oid");
                let params: [&(dyn ToSql + Sync); 2] = [&role_oid, &schema_oid];
                let rows = c.client.query(DEFAULT_ACL_SQL.pick(v), &params).await?;
                Ok((head, rows))
            }
        })
        .await?;
    let h = head.first().ok_or_else(|| Error::NotFound("rol".to_string()))?;
    if schema_oid.is_some() && !h.get::<_, bool>("schema_found") {
        return Err(Error::NotFound("şema".to_string()));
    }

    let mut unsupported = vec![];
    note_invalid_utf8(&[&head, &rows], &mut unsupported);
    if h.get::<_, bool>("is_system") {
        unsupported.push(SYSTEM_SCHEMA_REASON.to_string());
    }
    let entries = rows
        .iter()
        .filter_map(|r| {
            // Boş ACL ('{}'): önce yetki okunur; NULL ise diğer sütunlara dokunulmaz (bkz. `load_acl`).
            let privilege: String = r.get::<_, Option<String>>("privilege")?;
            Some(DefaultEntry {
                schema_entry: r.get::<_, String>("scope") == "s",
                objtype: r.get("objtype"),
                grantee: lossy_opt(r, "grantee"),
                by_role: r.get("by_role"),
                privilege,
                grantable: r.get("grantable"),
            })
        })
        .collect();
    let sections = assemble_sections(schema_oid.is_some(), entries, &mut unsupported);
    Ok(DefaultAclDef {
        role: lossy(h, "role"),
        schema: lossy_opt(h, "schema"),
        viewer: lossy(h, "viewer"),
        sections,
        editable: default_editable(&unsupported),
        unsupported,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use Privilege::*;

    fn de(schema_entry: bool, objtype: &str, grantee: Option<&str>, privilege: &str, grantable: bool) -> DefaultEntry {
        DefaultEntry { schema_entry, objtype: objtype.into(), grantee: grantee.map(Into::into), by_role: true, privilege: privilege.into(), grantable }
    }
    fn pgr(privilege: Privilege, grant_option: bool) -> PrivilegeGrant {
        PrivilegeGrant { privilege, grant_option }
    }

    #[test]
    fn default_general_scope_sections() {
        let mut u = vec![];
        let s = assemble_sections(
            false,
            vec![
                de(false, "r", Some("o"), "UPDATE", false),
                de(false, "r", Some("o"), "SELECT", false),
                de(false, "r", Some("a"), "SELECT", true),
                de(false, "f", Some("o"), "EXECUTE", false),
                de(false, "f", None, "EXECUTE", false),
                de(false, "n", Some("o"), "CREATE", false),
                de(false, "n", Some("o"), "USAGE", false),
            ],
            &mut u,
        );
        assert!(u.is_empty() && default_editable(&u), "{u:?}");
        use DefaultAclObjectType::*;
        assert_eq!(s.iter().map(|x| x.object_type).collect::<Vec<_>>(), [Tables, Sequences, Functions, Types, Schemas]);
        let who = |i: usize| s[i].grants.iter().map(|g| g.grantee.as_deref()).collect::<Vec<_>>();
        assert_eq!(who(0), [Some("a"), Some("o")]);
        assert_eq!(s[0].grants[1].privileges, [pgr(Select, false), pgr(Update, false)]);
        assert_eq!(s[0].grants[0].privileges, [pgr(Select, true)]);
        assert!(s[1].grants.is_empty() && s[3].grants.is_empty());
        assert_eq!(who(2), [None, Some("o")]);
        assert_eq!(s[4].grants[0].privileges, [pgr(Usage, false), pgr(Create, false)]);
        assert!(s.iter().all(|x| x.baseline.is_empty()));
    }

    #[test]
    fn default_schema_scope_grants_and_baseline() {
        let mut u = vec![];
        let s = assemble_sections(
            true,
            vec![
                de(false, "r", Some("o"), "SELECT", false),
                de(false, "n", Some("o"), "USAGE", false),
                de(true, "S", Some("b"), "SELECT", false),
                de(true, "S", Some("b"), "USAGE", true),
                de(false, "T", None, "USAGE", false),
            ],
            &mut u,
        );
        assert!(u.is_empty(), "genel Schemas satırı şema kapsamında sessizce düşer: {u:?}");
        assert_eq!(s.len(), 4);
        assert!(s[0].grants.is_empty());
        assert_eq!(s[0].baseline, [AclGrant { grantee: Some("o".into()), privileges: vec![pgr(Select, false)] }]);
        assert_eq!(s[1].grants, [AclGrant { grantee: Some("b".into()), privileges: vec![pgr(Usage, true), pgr(Select, false)] }]);
        assert!(s[1].baseline.is_empty());
        assert_eq!(s[3].baseline, [AclGrant { grantee: None, privileges: vec![pgr(Usage, false)] }]);
    }

    #[test]
    fn default_unsupported_reasons() {
        // PG 18 large object: bilgi notu, düzenlenebilir.
        let mut u = vec![];
        assemble_sections(false, vec![de(false, "L", Some("o"), "SELECT", false), de(false, "L", None, "UPDATE", false)], &mut u);
        assert_eq!(u, [LARGE_OBJECTS_REASON]);
        assert!(default_editable(&u));
        // Bilinmeyen tür, tür dışı ve tanınmayan yetki, rolden farklı veren: salt okunur.
        let mut u = vec![];
        let mut foreign = de(false, "r", Some("x"), "SELECT", false);
        foreign.by_role = false;
        let s = assemble_sections(
            false,
            vec![
                de(false, "Z", Some("o"), "SELECT", false),
                de(false, "S", Some("o"), "TRUNCATE", false),
                de(false, "r", Some("o"), "SET", false),
                de(false, "r", Some("o"), "SET", false),
                foreign,
            ],
            &mut u,
        );
        assert_eq!(u, ["varsayılan yetki türü Z", "yetki TRUNCATE", "yetki SET", DEFAULT_GRANTOR_REASON]);
        assert!(!default_editable(&u));
        assert!(s.iter().all(|x| x.grants.is_empty()));
        // Şema kapsamında şema girdisi olarak gelen `n` desteklenmeyen türdür.
        let mut u = vec![];
        assemble_sections(true, vec![de(true, "n", Some("o"), "USAGE", false)], &mut u);
        assert_eq!(u, ["varsayılan yetki türü n"]);
        let mut u = vec![LARGE_OBJECTS_REASON.to_string(), SYSTEM_SCHEMA_REASON.to_string()];
        assert!(!default_editable(&u));
        u.pop();
        assert!(default_editable(&u));
    }

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
    fn aggregates_read_only_windows_editable() {
        let r = routine_reason("a").unwrap();
        assert!(r.starts_with("AGGREGATE"), "{r}");
        for k in ["f", "w", "p"] {
            assert_eq!(routine_reason(k), None, "{k}");
        }
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
