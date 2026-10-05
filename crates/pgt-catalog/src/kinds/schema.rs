use pgt_core::Result;
use pgt_core::connection::Target;
use uuid::Uuid;

use crate::Catalog;
use crate::db::{p, query};
use crate::node::{NodeFlags, NodeId, NodeKind, Seg, TreeNode};
use crate::version::VersionedSql;

static LIST: VersionedSql = VersionedSql(&[(10, include_str!("../sql/schemas.sql"))]);

pub(crate) async fn list(cat: &Catalog, server: Uuid, db: u32) -> Result<Vec<TreeNode>> {
    let rows = query(&cat.mgr, server, Target::Database(db), &LIST, p![]).await?;
    let show_system = cat.show_system();
    let base = NodeId::server(server).child(Seg::Db(db));
    Ok(rows
        .iter()
        .filter(|r| show_system || !r.get::<_, bool>("is_system"))
        .map(|r| {
            TreeNode::new(&base.child(Seg::Schema(r.get("oid"))), NodeKind::Schema, r.get::<_, String>("name"))
                .with_flags(NodeFlags { system: r.get("is_system"), ..NodeFlags::default() })
        })
        .collect())
}
