mod column;
mod database;
mod schema;
mod server;
mod table;

use pgt_core::{Error, Result};

use crate::Catalog;
use crate::node::{Coll, NodeId, NodeKind, Seg, TreeNode};

pub(crate) fn invalid(id: &NodeId) -> Error {
    Error::Invalid(format!("desteklenmeyen düğüm: {id}"))
}

pub(crate) fn collection(parent: &NodeId, coll: Coll, kind: NodeKind, label: &str) -> TreeNode {
    TreeNode::new(&parent.child(Seg::Coll(coll)), kind, label)
}

pub(crate) async fn list_children(cat: &Catalog, id: &NodeId) -> Result<Vec<TreeNode>> {
    let (server, path) = match id {
        NodeId::Root => return server::root_children(cat).await,
        NodeId::Group(name) => return server::group_children(cat, name).await,
        NodeId::Node { server, path } => (*server, path.as_slice()),
    };
    if !cat.mgr.is_connected(server) {
        return Err(Error::NotConnected);
    }
    match path {
        [] => Ok(vec![collection(id, Coll::Databases, NodeKind::DatabaseCollection, "Databases")]),
        [Seg::Coll(Coll::Databases)] => database::list(cat, server).await,
        [Seg::Db(_)] => Ok(vec![collection(id, Coll::Schemas, NodeKind::SchemaCollection, "Schemas")]),
        [Seg::Db(db), Seg::Coll(Coll::Schemas)] => schema::list(cat, server, *db).await,
        [Seg::Db(_), Seg::Schema(_)] => Ok(vec![collection(id, Coll::Tables, NodeKind::TableCollection, "Tables")]),
        [Seg::Db(db), Seg::Schema(sch), Seg::Coll(Coll::Tables)] => table::list(cat, server, *db, *sch).await,
        [Seg::Db(_), Seg::Schema(_), Seg::Table(_)] => {
            Ok(vec![collection(id, Coll::Columns, NodeKind::ColumnCollection, "Columns")])
        }
        [Seg::Db(db), Seg::Schema(sch), Seg::Table(tbl), Seg::Coll(Coll::Columns)] => {
            column::list(cat, server, *db, *sch, *tbl).await
        }
        [Seg::Db(_), Seg::Schema(_), Seg::Table(_), Seg::Column(_)] => Ok(Vec::new()),
        _ => Err(invalid(id)),
    }
}
