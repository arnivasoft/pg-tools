//! Yetki (GRANT/REVOKE) ve varsayılan yetki (ALTER DEFAULT PRIVILEGES) modeli ile yetki kataloğu (Plan 20).
//! Saf veri ve küçük yardımcılar; üreticiler `build::acl`, yükleyiciler `load::acl` altındadır.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::model::ObjectKind;

/// Tek bir yetki (`aclexplode` `privilege_type` ile birebir eşleşir).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum Privilege {
    Select,
    Insert,
    Update,
    Delete,
    Truncate,
    References,
    Trigger,
    /// PG 17+.
    Maintain,
    Usage,
    Execute,
    Create,
    Connect,
    Temporary,
}

impl Privilege {
    /// SQL anahtar sözcüğü (GRANT/REVOKE listesinde yazılan).
    pub fn sql(self) -> &'static str {
        match self {
            Privilege::Select => "SELECT",
            Privilege::Insert => "INSERT",
            Privilege::Update => "UPDATE",
            Privilege::Delete => "DELETE",
            Privilege::Truncate => "TRUNCATE",
            Privilege::References => "REFERENCES",
            Privilege::Trigger => "TRIGGER",
            Privilege::Maintain => "MAINTAIN",
            Privilege::Usage => "USAGE",
            Privilege::Execute => "EXECUTE",
            Privilege::Create => "CREATE",
            Privilege::Connect => "CONNECT",
            Privilege::Temporary => "TEMPORARY",
        }
    }

    /// `aclexplode` `privilege_type` metninden; bilinmeyen (gelecek sürüm) → `None`.
    pub fn from_acl(text: &str) -> Option<Privilege> {
        [
            Privilege::Select,
            Privilege::Insert,
            Privilege::Update,
            Privilege::Delete,
            Privilege::Truncate,
            Privilege::References,
            Privilege::Trigger,
            Privilege::Maintain,
            Privilege::Usage,
            Privilege::Execute,
            Privilege::Create,
            Privilege::Connect,
            Privilege::Temporary,
        ]
        .into_iter()
        .find(|p| p.sql() == text)
    }
}

/// Bir yetki ve `WITH GRANT OPTION` durumu.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct PrivilegeGrant {
    pub privilege: Privilege,
    pub grant_option: bool,
}

/// Bir alıcının (sahibin verdiği) yetkileri; kanonik sırada.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct AclGrant {
    /// `None` = `PUBLIC`.
    pub grantee: Option<String>,
    pub privileges: Vec<PrivilegeGrant>,
}

/// Sahipten başkası tarafından verilmiş girdi (salt okunur).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ForeignGrant {
    pub grantor: String,
    pub grantee: Option<String>,
    pub privilege: Privilege,
    pub grant_option: bool,
}

/// Kolon düzeyi yetki (`pg_attribute.attacl`; salt okunur).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ColumnGrant {
    pub column: String,
    pub grantor: String,
    pub grantee: Option<String>,
    pub privilege: Privilege,
    pub grant_option: bool,
}

/// Bir nesnenin etkin ACL'si.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct AclDef {
    pub kind: ObjectKind,
    /// Schema/Database: `None`.
    pub schema: Option<String>,
    pub name: String,
    /// Yalnız Function/Procedure: `pg_get_function_identity_arguments`.
    pub signature: Option<String>,
    pub owner: String,
    /// Bağlı rol (kendini kilitleme uyarıları için).
    pub viewer: String,
    /// ACL sütunu NULL değil (NULL ise etkin değer `acldefault` ile gösterilir).
    pub explicit: bool,
    pub grants: Vec<AclGrant>,
    pub foreign: Vec<ForeignGrant>,
    pub columns: Vec<ColumnGrant>,
    pub editable: bool,
    pub unsupported: Vec<String>,
}

/// Varsayılan yetki nesne türü (PG harfleri `r, S, f, T, n`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum DefaultAclObjectType {
    Tables,
    Sequences,
    Functions,
    Types,
    Schemas,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct DefaultAclSection {
    pub object_type: DefaultAclObjectType,
    pub grants: Vec<AclGrant>,
    /// Şema kapsamında genel kapsamın etkin değeri (salt okunur); genel kapsamda boş.
    pub baseline: Vec<AclGrant>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct DefaultAclDef {
    pub role: String,
    pub schema: Option<String>,
    pub viewer: String,
    pub sections: Vec<DefaultAclSection>,
    pub editable: bool,
    pub unsupported: Vec<String>,
}

/// Varsayılan yetki kapsamı (yükleme/istek).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct DefaultAclScope {
    /// `None` → bağlı rol.
    pub role: Option<String>,
    #[ts(type = "number | null")]
    pub schema_oid: Option<u32>,
}

/// Yetki düzenlenebilen nesne türleri (Plan 20 Karar 1).
pub fn acl_supported(kind: ObjectKind) -> bool {
    matches!(
        kind,
        ObjectKind::Table
            | ObjectKind::View
            | ObjectKind::MaterializedView
            | ObjectKind::Sequence
            | ObjectKind::Function
            | ObjectKind::Procedure
            | ObjectKind::Schema
            | ObjectKind::Database
            | ObjectKind::Type
            | ObjectKind::Domain
    )
}

use Privilege::*;

const TABLE: &[Privilege] = &[Select, Insert, Update, Delete, Truncate, References, Trigger];
const TABLE_17: &[Privilege] = &[Select, Insert, Update, Delete, Truncate, References, Trigger, Maintain];
const SEQUENCE: &[Privilege] = &[Usage, Select, Update];
const EXECUTE: &[Privilege] = &[Execute];
const SCHEMA: &[Privilege] = &[Usage, Create];
const DATABASE: &[Privilege] = &[Create, Temporary, Connect];
const USAGE: &[Privilege] = &[Usage];

fn table_set(version_num: u32) -> &'static [Privilege] {
    if version_num >= 170000 { TABLE_17 } else { TABLE }
}

/// Türün yetki kümesi, kanonik sırada (Karar 2); `Maintain` PG 17+. Desteklenmeyen tür → boş.
pub fn privileges_for(kind: ObjectKind, version_num: u32) -> &'static [Privilege] {
    match kind {
        ObjectKind::Table | ObjectKind::View | ObjectKind::MaterializedView => table_set(version_num),
        ObjectKind::Sequence => SEQUENCE,
        ObjectKind::Function | ObjectKind::Procedure => EXECUTE,
        ObjectKind::Schema => SCHEMA,
        ObjectKind::Database => DATABASE,
        ObjectKind::Type | ObjectKind::Domain => USAGE,
        _ => &[],
    }
}

/// Varsayılan yetki bölümünün yetki kümesi (Karar 12).
pub fn default_privileges_for(t: DefaultAclObjectType, version_num: u32) -> &'static [Privilege] {
    match t {
        DefaultAclObjectType::Tables => table_set(version_num),
        DefaultAclObjectType::Sequences => SEQUENCE,
        DefaultAclObjectType::Functions => EXECUTE,
        DefaultAclObjectType::Types => USAGE,
        DefaultAclObjectType::Schemas => SCHEMA,
    }
}

/// Bölüm sırası: şema kapsamında `Schemas` yok.
pub fn default_sections(schema_scoped: bool) -> &'static [DefaultAclObjectType] {
    use DefaultAclObjectType::*;
    if schema_scoped { &[Tables, Sequences, Functions, Types] } else { &[Tables, Sequences, Functions, Types, Schemas] }
}

/// `acldefault(<harf>, <sahip>)` nesne türü harfi (Karar 4). Desteklenmeyen tür için `r` (kullanılmaz).
pub fn acldefault_letter(kind: ObjectKind) -> char {
    match kind {
        ObjectKind::Sequence => 's',
        ObjectKind::Function | ObjectKind::Procedure => 'f',
        ObjectKind::Schema => 'n',
        ObjectKind::Database => 'd',
        ObjectKind::Type | ObjectKind::Domain => 'T',
        _ => 'r',
    }
}

/// Karar 5 sırası: `PUBLIC` önce, sonra ad (bayt sırası = `COLLATE "C"`); her alıcının yetkileri `order` sırasında.
pub fn sort_grants(grants: &mut [AclGrant], order: &[Privilege]) {
    let rank = |p: Privilege| order.iter().position(|o| *o == p).unwrap_or(usize::MAX);
    for g in grants.iter_mut() {
        g.privileges.sort_by_key(|pg| rank(pg.privilege));
    }
    // `Option<String>`: `None` < `Some`; `String` bayt sırası.
    grants.sort_by(|a, b| a.grantee.cmp(&b.grantee));
}

#[cfg(test)]
mod tests {
    use super::*;

    const ALL: [Privilege; 13] = [
        Select, Insert, Update, Delete, Truncate, References, Trigger, Maintain, Usage, Execute, Create, Connect, Temporary,
    ];

    #[test]
    fn privilege_serde_snake_case_and_acl_text() {
        assert_eq!(serde_json::to_value(Temporary).unwrap(), "temporary");
        assert_eq!(serde_json::from_value::<Privilege>("references".into()).unwrap(), References);
        for p in ALL {
            assert_eq!(Privilege::from_acl(p.sql()), Some(p), "{p:?}");
        }
        assert_eq!(Privilege::from_acl("TEMPORARY"), Some(Temporary));
        assert_eq!(Privilege::from_acl("SET"), None);
        assert_eq!(Privilege::from_acl("select"), None);
    }

    #[test]
    fn privileges_per_kind_and_maintain_by_version() {
        let t16 = privileges_for(ObjectKind::Table, 160000);
        let t17 = privileges_for(ObjectKind::Table, 170000);
        assert!(!t16.contains(&Maintain) && t17.last() == Some(&Maintain) && t17.len() == t16.len() + 1);
        assert_eq!(privileges_for(ObjectKind::View, 170000), t17);
        assert_eq!(privileges_for(ObjectKind::MaterializedView, 100000), t16);
        assert_eq!(privileges_for(ObjectKind::Sequence, 160000), [Usage, Select, Update]);
        assert_eq!(privileges_for(ObjectKind::Function, 160000), [Execute]);
        assert_eq!(privileges_for(ObjectKind::Procedure, 160000), [Execute]);
        assert_eq!(privileges_for(ObjectKind::Schema, 160000), [Usage, Create]);
        assert_eq!(privileges_for(ObjectKind::Database, 160000), [Create, Temporary, Connect]);
        assert_eq!(privileges_for(ObjectKind::Type, 160000), [Usage]);
        assert_eq!(privileges_for(ObjectKind::Domain, 160000), [Usage]);
        assert!(privileges_for(ObjectKind::Index, 170000).is_empty());
    }

    #[test]
    fn supported_kinds() {
        for k in [
            ObjectKind::Table,
            ObjectKind::View,
            ObjectKind::MaterializedView,
            ObjectKind::Sequence,
            ObjectKind::Function,
            ObjectKind::Procedure,
            ObjectKind::Schema,
            ObjectKind::Database,
            ObjectKind::Type,
            ObjectKind::Domain,
        ] {
            assert!(acl_supported(k), "{k:?}");
            assert!(!privileges_for(k, 170000).is_empty(), "{k:?}");
        }
        for k in [ObjectKind::Index, ObjectKind::Extension, ObjectKind::Role, ObjectKind::Trigger] {
            assert!(!acl_supported(k), "{k:?}");
        }
    }

    #[test]
    fn default_sections_and_privileges() {
        use DefaultAclObjectType::*;
        assert_eq!(default_sections(true), [Tables, Sequences, Functions, Types]);
        assert_eq!(default_sections(false), [Tables, Sequences, Functions, Types, Schemas]);
        assert_eq!(default_privileges_for(Tables, 170000), privileges_for(ObjectKind::Table, 170000));
        assert_eq!(default_privileges_for(Tables, 160000), privileges_for(ObjectKind::Table, 160000));
        assert_eq!(default_privileges_for(Sequences, 160000), [Usage, Select, Update]);
        assert_eq!(default_privileges_for(Functions, 160000), [Execute]);
        assert_eq!(default_privileges_for(Types, 160000), [Usage]);
        assert_eq!(default_privileges_for(Schemas, 160000), [Usage, Create]);
    }

    #[test]
    fn acldefault_letters() {
        assert_eq!(acldefault_letter(ObjectKind::Table), 'r');
        assert_eq!(acldefault_letter(ObjectKind::View), 'r');
        assert_eq!(acldefault_letter(ObjectKind::MaterializedView), 'r');
        assert_eq!(acldefault_letter(ObjectKind::Sequence), 's');
        assert_eq!(acldefault_letter(ObjectKind::Function), 'f');
        assert_eq!(acldefault_letter(ObjectKind::Procedure), 'f');
        assert_eq!(acldefault_letter(ObjectKind::Schema), 'n');
        assert_eq!(acldefault_letter(ObjectKind::Database), 'd');
        assert_eq!(acldefault_letter(ObjectKind::Type), 'T');
        assert_eq!(acldefault_letter(ObjectKind::Domain), 'T');
    }

    fn g(who: Option<&str>, ps: &[Privilege]) -> AclGrant {
        AclGrant {
            grantee: who.map(Into::into),
            privileges: ps.iter().map(|p| PrivilegeGrant { privilege: *p, grant_option: false }).collect(),
        }
    }

    #[test]
    fn sort_grants_public_first_bytes_then_canonical() {
        let mut v = vec![g(Some("a"), &[Update, Select]), g(Some("B"), &[]), g(None, &[Trigger, Insert]), g(Some("_x"), &[])];
        sort_grants(&mut v, privileges_for(ObjectKind::Table, 160000));
        let who: Vec<_> = v.iter().map(|x| x.grantee.as_deref()).collect();
        assert_eq!(who, [None, Some("B"), Some("_x"), Some("a")]);
        assert_eq!(v[0].privileges.iter().map(|p| p.privilege).collect::<Vec<_>>(), [Insert, Trigger]);
        assert_eq!(v[3].privileges.iter().map(|p| p.privilege).collect::<Vec<_>>(), [Select, Update]);
    }
}
