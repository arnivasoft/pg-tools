use pgt_core::Result;
use pgt_core::connection::Target;
use uuid::Uuid;

use crate::Catalog;
use crate::db::{p, query};
use crate::node::{NodeFlags, NodeId, NodeKind, Seg, TreeNode};
use crate::version::VersionedSql;

static LIST: VersionedSql = VersionedSql(&[(10, include_str!("../sql/columns.sql"))]);

pub(crate) async fn list(cat: &Catalog, server: Uuid, db: u32, schema: u32, table: u32) -> Result<Vec<TreeNode>> {
    let rows = query(&cat.mgr, server, Target::Database(db), &LIST, p![table]).await?;
    let base = NodeId::server(server).child(Seg::Db(db)).child(Seg::Schema(schema)).child(Seg::Table(table));
    Ok(rows
        .iter()
        .map(|r| {
            TreeNode::new(&base.child(Seg::Column(r.get("attnum"))), NodeKind::Column, r.get::<_, String>("name"))
                .with_detail(Some(r.get("type_name")))
                .with_flags(NodeFlags { primary_key: r.get("is_pk"), not_null: r.get("not_null"), ..NodeFlags::default() })
                .leaf()
        })
        .collect())
}
