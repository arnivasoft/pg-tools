//! Yeni nesne türlerinin kayıtları. Bir türü eklemek: SQL dosyaları + burada bir kayıt.

use super::engine::{CollDef, Level};
use crate::node::{Coll, NodeKind, Seg};
use crate::version::{PgVersion, VersionedSql};

macro_rules! sql {
    ($name:literal) => {
        include_str!(concat!("../sql/", $name))
    };
}

pub(crate) static CONSTRAINTS: CollDef = CollDef {
    coll: Coll::Constraints,
    label: "Constraints",
    coll_kind: NodeKind::ConstraintCollection,
    child_kind: NodeKind::Constraint,
    seg: Seg::Constraint,
    list: &VersionedSql(&[(10, sql!("list_constraints.sql"))]),
    level: Level::Database,
    takes_parent: true,
    min_version: 10,
};

pub(crate) static INDEXES: CollDef = CollDef {
    coll: Coll::Indexes,
    label: "Indexes",
    coll_kind: NodeKind::IndexCollection,
    child_kind: NodeKind::Index,
    seg: Seg::Index,
    list: &VersionedSql(&[(10, sql!("list_indexes.sql"))]),
    level: Level::Database,
    takes_parent: true,
    min_version: 10,
};

pub(crate) static TRIGGERS: CollDef = CollDef {
    coll: Coll::Triggers,
    label: "Triggers",
    coll_kind: NodeKind::TriggerCollection,
    child_kind: NodeKind::Trigger,
    seg: Seg::Trigger,
    list: &VersionedSql(&[(10, sql!("list_triggers.sql"))]),
    level: Level::Database,
    takes_parent: true,
    min_version: 10,
};

pub(crate) static POLICIES: CollDef = CollDef {
    coll: Coll::Policies,
    label: "RLS Policies",
    coll_kind: NodeKind::PolicyCollection,
    child_kind: NodeKind::Policy,
    seg: Seg::Policy,
    list: &VersionedSql(&[(10, sql!("list_policies.sql"))]),
    level: Level::Database,
    takes_parent: true,
    min_version: 10,
};

/// Partition'lar `table` düğümleridir; yalnızca koleksiyon türü farklıdır.
pub(crate) static PARTITIONS: CollDef = CollDef {
    coll: Coll::Partitions,
    label: "Partitions",
    coll_kind: NodeKind::PartitionCollection,
    child_kind: NodeKind::Table,
    seg: Seg::Table,
    list: &VersionedSql(&[(10, sql!("list_partitions.sql"))]),
    level: Level::Database,
    takes_parent: true,
    min_version: 10,
};

pub(crate) static VIEWS: CollDef = CollDef {
    coll: Coll::Views,
    label: "Views",
    coll_kind: NodeKind::ViewCollection,
    child_kind: NodeKind::View,
    seg: Seg::View,
    list: &VersionedSql(&[(10, sql!("list_views.sql"))]),
    level: Level::Database,
    takes_parent: true,
    min_version: 10,
};

pub(crate) static MATVIEWS: CollDef = CollDef {
    coll: Coll::MatViews,
    label: "Materialized Views",
    coll_kind: NodeKind::MaterializedViewCollection,
    child_kind: NodeKind::MaterializedView,
    seg: Seg::MatView,
    list: &VersionedSql(&[(10, sql!("list_matviews.sql"))]),
    level: Level::Database,
    takes_parent: true,
    min_version: 10,
};

pub(crate) static FUNCTIONS: CollDef = CollDef {
    coll: Coll::Functions,
    label: "Functions",
    coll_kind: NodeKind::FunctionCollection,
    child_kind: NodeKind::Function,
    seg: Seg::Function,
    list: &VersionedSql(&[(10, sql!("list_functions.pg10.sql")), (11, sql!("list_functions.sql"))]),
    level: Level::Database,
    takes_parent: true,
    min_version: 10,
};

pub(crate) static PROCEDURES: CollDef = CollDef {
    coll: Coll::Procedures,
    label: "Procedures",
    coll_kind: NodeKind::ProcedureCollection,
    child_kind: NodeKind::Procedure,
    seg: Seg::Procedure,
    list: &VersionedSql(&[(11, sql!("list_procedures.sql"))]),
    level: Level::Database,
    takes_parent: true,
    min_version: 11,
};

pub(crate) static TRIGGER_FUNCTIONS: CollDef = CollDef {
    coll: Coll::TriggerFunctions,
    label: "Trigger Functions",
    coll_kind: NodeKind::TriggerFunctionCollection,
    child_kind: NodeKind::TriggerFunction,
    seg: Seg::TriggerFunction,
    list: &VersionedSql(&[(10, sql!("list_trigger_functions.pg10.sql")), (11, sql!("list_trigger_functions.sql"))]),
    level: Level::Database,
    takes_parent: true,
    min_version: 10,
};

pub(crate) static SEQUENCES: CollDef = CollDef {
    coll: Coll::Sequences,
    label: "Sequences",
    coll_kind: NodeKind::SequenceCollection,
    child_kind: NodeKind::Sequence,
    seg: Seg::Sequence,
    list: &VersionedSql(&[(10, sql!("list_sequences.sql"))]),
    level: Level::Database,
    takes_parent: true,
    min_version: 10,
};

pub(crate) static TYPES: CollDef = CollDef {
    coll: Coll::Types,
    label: "Types",
    coll_kind: NodeKind::TypeCollection,
    child_kind: NodeKind::Type,
    seg: Seg::Type,
    list: &VersionedSql(&[(10, sql!("list_types.sql"))]),
    level: Level::Database,
    takes_parent: true,
    min_version: 10,
};

pub(crate) static DOMAINS: CollDef = CollDef {
    coll: Coll::Domains,
    label: "Domains",
    coll_kind: NodeKind::DomainCollection,
    child_kind: NodeKind::Domain,
    seg: Seg::Domain,
    list: &VersionedSql(&[(10, sql!("list_domains.sql"))]),
    level: Level::Database,
    takes_parent: true,
    min_version: 10,
};

pub(crate) static EXTENSIONS: CollDef = CollDef {
    coll: Coll::Extensions,
    label: "Extensions",
    coll_kind: NodeKind::ExtensionCollection,
    child_kind: NodeKind::Extension,
    seg: Seg::Extension,
    list: &VersionedSql(&[(10, sql!("list_extensions.sql"))]),
    level: Level::Database,
    takes_parent: false,
    min_version: 10,
};

pub(crate) static ROLES: CollDef = CollDef {
    coll: Coll::Roles,
    label: "Login/Group Roles",
    coll_kind: NodeKind::RoleCollection,
    child_kind: NodeKind::Role,
    seg: Seg::Role,
    list: &VersionedSql(&[(10, sql!("list_roles.sql"))]),
    level: Level::Server,
    takes_parent: false,
    min_version: 10,
};

pub(crate) static TABLESPACES: CollDef = CollDef {
    coll: Coll::Tablespaces,
    label: "Tablespaces",
    coll_kind: NodeKind::TablespaceCollection,
    child_kind: NodeKind::Tablespace,
    seg: Seg::Tablespace,
    list: &VersionedSql(&[(10, sql!("list_tablespaces.sql"))]),
    level: Level::Server,
    takes_parent: false,
    min_version: 10,
};

/// Genel motorla yönetilen koleksiyonun tanımı (`Databases`, `Schemas`, `Tables`, `Columns` elle yazılmış modüllerdedir).
pub(crate) fn coll_def(coll: Coll) -> Option<&'static CollDef> {
    Some(match coll {
        Coll::Constraints => &CONSTRAINTS,
        Coll::Indexes => &INDEXES,
        Coll::Triggers => &TRIGGERS,
        Coll::Policies => &POLICIES,
        Coll::Partitions => &PARTITIONS,
        Coll::Views => &VIEWS,
        Coll::MatViews => &MATVIEWS,
        Coll::Functions => &FUNCTIONS,
        Coll::Procedures => &PROCEDURES,
        Coll::TriggerFunctions => &TRIGGER_FUNCTIONS,
        Coll::Sequences => &SEQUENCES,
        Coll::Types => &TYPES,
        Coll::Domains => &DOMAINS,
        Coll::Extensions => &EXTENSIONS,
        Coll::Roles => &ROLES,
        Coll::Tablespaces => &TABLESPACES,
        Coll::Databases | Coll::Schemas | Coll::Tables | Coll::Columns => return None,
    })
}

/// Şema altındaki genel koleksiyonlar (`Tables` hariç), sunucu sürümüne göre.
pub(crate) fn schema_collections(version: PgVersion) -> Vec<&'static CollDef> {
    [&VIEWS, &MATVIEWS, &FUNCTIONS, &PROCEDURES, &TRIGGER_FUNCTIONS, &SEQUENCES, &TYPES, &DOMAINS]
        .into_iter()
        .filter(|d| version.major() >= d.min_version)
        .collect()
}
