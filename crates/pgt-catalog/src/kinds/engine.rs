//! Veri sözleşmeli SQL dosyalarını `TreeNode` / `PropertySection` / DDL metnine çeviren genel motor.

use pgt_core::connection::Target;
use pgt_core::{Error, Result};
use uuid::Uuid;

use crate::Catalog;
use crate::db::{p, query};
use crate::node::{Coll, NodeFlags, NodeId, NodeKind, Seg, TreeNode};
use crate::version::VersionedSql;

/// Sorgunun hangi bağlantıda çalışacağı.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Level {
    /// Sunucu geneli kataloglar (roller, tablespace'ler): bakım veritabanı bağlantısı.
    Server,
    /// Veritabanına özel kataloglar.
    Database,
}

pub(crate) fn target(level: Level, db: Option<u32>) -> Result<Target> {
    match (level, db) {
        (Level::Server, _) => Ok(Target::Maintenance),
        (Level::Database, Some(d)) => Ok(Target::Database(d)),
        (Level::Database, None) => Err(Error::Invalid("veritabanı kimliği yok".into())),
    }
}

/// Bir koleksiyonun (ve çocuklarının) tanımı.
pub(crate) struct CollDef {
    pub coll: Coll,
    pub label: &'static str,
    pub coll_kind: NodeKind,
    pub child_kind: NodeKind,
    pub seg: fn(u32) -> Seg,
    pub list: &'static VersionedSql,
    pub level: Level,
    /// Liste sorgusu `$1` olarak üst nesnenin OID'ini alır mı.
    pub takes_parent: bool,
    /// Koleksiyonun gösterildiği en düşük PostgreSQL ana sürümü.
    pub min_version: u32,
}

/// `parent` altında koleksiyon düğümünü üretir.
pub(crate) fn collection_node(parent: &NodeId, def: &CollDef) -> TreeNode {
    TreeNode::new(&parent.child(Seg::Coll(def.coll)), def.coll_kind, def.label)
}

/// Koleksiyonun çocuklarını listeler. Çocuk kimlikleri koleksiyon parçasını atlar: `parent.child(seg(oid))`.
pub(crate) async fn list(
    cat: &Catalog,
    server: Uuid,
    db: Option<u32>,
    def: &CollDef,
    parent: &NodeId,
    parent_oid: Option<u32>,
) -> Result<Vec<TreeNode>> {
    let params = match (def.takes_parent, parent_oid) {
        (true, Some(oid)) => p![oid],
        (true, None) => return Err(Error::Invalid(format!("{} için üst nesne yok", def.label))),
        (false, _) => p![],
    };
    let rows = query(&cat.mgr, server, target(def.level, db)?, def.list, params).await?;
    let show_system = cat.show_system();
    let mut out = Vec::with_capacity(rows.len());
    for r in &rows {
        let flag = |c: &str| r.try_get::<_, bool>(c).unwrap_or(false);
        if flag("system") && !show_system {
            continue;
        }
        let oid: u32 = r.get("oid");
        let mut node = TreeNode::new(&parent.child((def.seg)(oid)), def.child_kind, r.get::<_, String>("name"))
            .with_detail(r.try_get::<_, Option<String>>("detail").ok().flatten())
            .with_flags(NodeFlags {
                partitioned: flag("partitioned"),
                system: flag("system"),
                primary_key: flag("primary_key"),
                not_null: flag("not_null"),
                from_extension: flag("from_extension"),
                ..NodeFlags::default()
            });
        if r.try_get::<_, bool>("leaf").unwrap_or(true) {
            node = node.leaf();
        }
        out.push(node);
    }
    Ok(out)
}
