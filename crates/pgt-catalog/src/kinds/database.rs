use pgt_core::Result;
use pgt_core::connection::Target;
use uuid::Uuid;

use crate::Catalog;
use crate::db::{p, query};
use crate::node::{NodeId, NodeKind, Seg, TreeNode};
use crate::version::VersionedSql;

static LIST: VersionedSql = VersionedSql(&[(10, include_str!("../sql/databases.sql"))]);

pub(crate) async fn list(cat: &Catalog, server: Uuid) -> Result<Vec<TreeNode>> {
    let rows = query(&cat.mgr, server, Target::Maintenance, &LIST, p![]).await?;
    Ok(rows
        .iter()
        .map(|r| {
            let oid: u32 = r.get("oid");
            let node = TreeNode::new(&NodeId::server(server).child(Seg::Db(oid)), NodeKind::Database, r.get::<_, String>("name"));
            if r.get::<_, bool>("allow_conn") { node } else { node.leaf() }
        })
        .collect())
}
