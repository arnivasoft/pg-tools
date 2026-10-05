use pgt_core::Result;
use pgt_core::connection::Target;
use uuid::Uuid;

use crate::Catalog;
use crate::db::{p, query};
use crate::node::{NodeFlags, NodeId, NodeKind, Seg, TreeNode};
use crate::props::approx_rows;
use crate::version::VersionedSql;

static LIST: VersionedSql = VersionedSql(&[(10, include_str!("../sql/tables.sql"))]);

pub(crate) async fn list(cat: &Catalog, server: Uuid, db: u32, schema: u32) -> Result<Vec<TreeNode>> {
    let rows = query(&cat.mgr, server, Target::Database(db), &LIST, p![schema]).await?;
    let base = NodeId::server(server).child(Seg::Db(db)).child(Seg::Schema(schema));
    Ok(rows
        .iter()
        .map(|r| {
            TreeNode::new(&base.child(Seg::Table(r.get("oid"))), NodeKind::Table, r.get::<_, String>("name"))
                .with_detail(approx_rows(r.get("reltuples")))
                .with_flags(NodeFlags { partitioned: r.get("partitioned"), ..NodeFlags::default() })
        })
        .collect())
}
