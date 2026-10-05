mod column;
mod database;
pub(crate) mod engine;
pub(crate) mod registry;
mod schema;
mod server;
mod table;

use pgt_core::{Error, Result};

use crate::Catalog;
use crate::db::{p, query};
use crate::node::{Coll, NodeId, NodeKind, Seg, TreeNode};
use crate::version::{PgVersion, VersionedSql};

pub(crate) fn invalid(id: &NodeId) -> Error {
    Error::Invalid(format!("desteklenmeyen düğüm: {id}"))
}

/// Elle yazılmış koleksiyonlar için (`Databases`, `Schemas`, `Tables`, `Columns`) koleksiyon düğümü.
pub(crate) fn collection(parent: &NodeId, coll: Coll, kind: NodeKind, label: &str) -> TreeNode {
    TreeNode::new(&parent.child(Seg::Coll(coll)), kind, label)
}

fn db_req(db: Option<u32>) -> Result<u32> {
    db.ok_or_else(|| Error::Invalid("veritabanı kimliği yok".into()))
}

/// Yolun sondan ikinci parçasının OID'i (koleksiyonun veya yaprağın üst nesnesi).
fn parent_oid(path: &[Seg]) -> Option<u32> {
    path.len().checked_sub(2).and_then(|i| path[i].oid())
}

static IS_PARTITIONED: VersionedSql = VersionedSql(&[(10, "SELECT c.relkind = 'p' AS partitioned FROM pg_class c WHERE c.oid = $1")]);

async fn is_partitioned(cat: &Catalog, server: uuid::Uuid, db: u32, table: u32) -> Result<bool> {
    let rows = query(&cat.mgr, server, pgt_core::connection::Target::Database(db), &IS_PARTITIONED, p![table]).await?;
    Ok(rows.first().map(|r| r.get::<_, bool>("partitioned")).unwrap_or(false))
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
    let version = PgVersion(cat.mgr.info(server).ok_or(Error::NotConnected)?.version_num);
    let db = match path.first() {
        Some(Seg::Db(d)) => Some(*d),
        _ => None,
    };
    match path.last() {
        None => Ok(vec![
            collection(id, Coll::Databases, NodeKind::DatabaseCollection, "Databases"),
            engine::collection_node(id, &registry::ROLES),
            engine::collection_node(id, &registry::TABLESPACES),
        ]),
        Some(Seg::Coll(coll)) => list_collection(cat, server, db, id, path, *coll, version).await,
        Some(Seg::Db(_)) => Ok(vec![
            collection(id, Coll::Schemas, NodeKind::SchemaCollection, "Schemas"),
            engine::collection_node(id, &registry::EXTENSIONS),
        ]),
        Some(Seg::Schema(_)) => {
            let mut out = vec![collection(id, Coll::Tables, NodeKind::TableCollection, "Tables")];
            out.extend(registry::schema_collections(version).into_iter().map(|d| engine::collection_node(id, d)));
            Ok(out)
        }
        Some(Seg::Table(table)) => {
            let mut out = vec![collection(id, Coll::Columns, NodeKind::ColumnCollection, "Columns")];
            let mut defs = vec![&registry::CONSTRAINTS, &registry::INDEXES, &registry::TRIGGERS, &registry::POLICIES];
            if is_partitioned(cat, server, db_req(db)?, *table).await? {
                defs.push(&registry::PARTITIONS);
            }
            out.extend(defs.into_iter().map(|d| engine::collection_node(id, d)));
            Ok(out)
        }
        Some(Seg::View(_)) => Ok(vec![collection(id, Coll::Columns, NodeKind::ColumnCollection, "Columns")]),
        Some(Seg::MatView(_)) => Ok(vec![
            collection(id, Coll::Columns, NodeKind::ColumnCollection, "Columns"),
            engine::collection_node(id, &registry::INDEXES),
        ]),
        // Yapraklar: kolon, constraint, index, trigger, policy, sequence, fonksiyonlar, tip, domain, extension, rol, tablespace
        Some(_) => Ok(Vec::new()),
    }
}

async fn list_collection(
    cat: &Catalog,
    server: uuid::Uuid,
    db: Option<u32>,
    id: &NodeId,
    path: &[Seg],
    coll: Coll,
    version: PgVersion,
) -> Result<Vec<TreeNode>> {
    let parent = id.parent();
    let above = path.len().checked_sub(2).map(|i| path[i]);
    match (above, coll) {
        (None, Coll::Databases) => database::list(cat, server).await,
        (None, Coll::Roles) => engine::list(cat, server, None, &registry::ROLES, &parent, None).await,
        (None, Coll::Tablespaces) => engine::list(cat, server, None, &registry::TABLESPACES, &parent, None).await,
        (Some(Seg::Db(_)), Coll::Schemas) => schema::list(cat, server, db_req(db)?).await,
        (Some(Seg::Db(_)), Coll::Extensions) => engine::list(cat, server, db, &registry::EXTENSIONS, &parent, None).await,
        (Some(Seg::Schema(schema)), Coll::Tables) => table::list(cat, server, db_req(db)?, schema).await,
        (Some(Seg::Schema(schema)), c) => match registry::coll_def(c) {
            Some(def) if registry::schema_collections(version).iter().any(|d| d.coll == c) => {
                engine::list(cat, server, db, def, &parent, Some(schema)).await
            }
            _ => Err(invalid(id)),
        },
        (Some(Seg::Table(rel) | Seg::View(rel) | Seg::MatView(rel)), Coll::Columns) => {
            column::list(cat, server, db_req(db)?, &parent, rel).await
        }
        (Some(Seg::Table(rel)), c @ (Coll::Constraints | Coll::Indexes | Coll::Triggers | Coll::Policies | Coll::Partitions)) => {
            let def = registry::coll_def(c).ok_or_else(|| invalid(id))?;
            engine::list(cat, server, db, def, &parent, Some(rel)).await
        }
        (Some(Seg::MatView(rel)), Coll::Indexes) => engine::list(cat, server, db, &registry::INDEXES, &parent, Some(rel)).await,
        _ => Err(invalid(id)),
    }
}

pub(crate) async fn properties(cat: &Catalog, id: &NodeId) -> Result<Vec<crate::props::PropertySection>> {
    let (server, path) = match id {
        NodeId::Root | NodeId::Group(_) => return Ok(Vec::new()),
        NodeId::Node { server, path } => (*server, path.as_slice()),
    };
    if path.is_empty() {
        return server::properties(cat, server).await;
    }
    if !cat.mgr.is_connected(server) {
        return Err(Error::NotConnected);
    }
    let db = match path.first() {
        Some(Seg::Db(d)) => Some(*d),
        _ => None,
    };
    match path.last().expect("boş olmayan yol") {
        Seg::Coll(_) => Ok(Vec::new()),
        Seg::Db(d) => database::properties(cat, server, *d).await,
        Seg::Schema(s) => schema::properties(cat, server, db_req(db)?, *s).await,
        Seg::Table(t) => table::properties(cat, server, db_req(db)?, *t).await,
        Seg::Column(c) => {
            let rel = parent_oid(path).ok_or_else(|| invalid(id))?;
            column::properties(cat, server, db_req(db)?, rel, *c).await
        }
        seg => match registry::props_def(*seg) {
            Some(def) => {
                let oid = seg.oid().ok_or_else(|| invalid(id))?;
                engine::properties(cat, server, db, def, oid).await
            }
            None => Err(invalid(id)),
        },
    }
}

pub(crate) async fn ddl(cat: &Catalog, id: &NodeId) -> Result<String> {
    let (server, path) = match id {
        NodeId::Root | NodeId::Group(_) => return Ok(String::new()),
        NodeId::Node { server, path } => (*server, path.as_slice()),
    };
    if path.is_empty() || matches!(path.last(), Some(Seg::Coll(_))) {
        return Ok(String::new());
    }
    if !cat.mgr.is_connected(server) {
        return Err(Error::NotConnected);
    }
    let db = match path.first() {
        Some(Seg::Db(d)) => Some(*d),
        _ => None,
    };
    match path.last().expect("boş olmayan yol") {
        Seg::Db(d) => database::ddl(cat, server, *d).await,
        Seg::Schema(s) => schema::ddl(cat, server, db_req(db)?, *s).await,
        Seg::Table(t) => table::ddl(cat, server, db_req(db)?, *t).await,
        Seg::Column(c) => {
            let rel = parent_oid(path).ok_or_else(|| invalid(id))?;
            column::ddl(cat, server, db_req(db)?, rel, *c).await
        }
        _ => Err(invalid(id)),
    }
}

/// Bir nesnenin ACL satırlarını okur (`acl_*.sql`).
pub(crate) async fn acl(
    cat: &Catalog,
    server: uuid::Uuid,
    target: pgt_core::connection::Target,
    sql: &'static VersionedSql,
    oid: u32,
) -> Result<Vec<crate::ddl::AclEntry>> {
    let rows = query(&cat.mgr, server, target, sql, p![oid]).await?;
    Ok(rows.iter().map(crate::ddl::AclEntry::from_row).collect())
}
