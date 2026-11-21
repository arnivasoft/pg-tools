mod column;
mod database;
pub(crate) mod engine;
mod foreign;
pub(crate) mod registry;
mod replication;
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
            engine::collection_node(id, &registry::FDWS),
            engine::collection_node(id, &registry::PUBLICATIONS),
            engine::collection_node(id, &registry::SUBSCRIPTIONS),
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
        Some(Seg::ForeignDataWrapper(_)) => Ok(vec![engine::collection_node(id, &registry::FOREIGN_SERVERS)]),
        Some(Seg::ForeignServer(_)) => Ok(vec![engine::collection_node(id, &registry::USER_MAPPINGS)]),
        Some(Seg::View(_) | Seg::ForeignTable(_)) => Ok(vec![collection(id, Coll::Columns, NodeKind::ColumnCollection, "Columns")]),
        Some(Seg::MatView(_)) => Ok(vec![
            collection(id, Coll::Columns, NodeKind::ColumnCollection, "Columns"),
            engine::collection_node(id, &registry::INDEXES),
        ]),
        // Yapraklar: kolon, constraint, index, trigger, policy, sequence, fonksiyonlar, tip, domain, extension, rol, tablespace,
        // user mapping, publication, subscription
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
        (Some(Seg::Db(_)), Coll::ForeignDataWrappers) => engine::list(cat, server, db, &registry::FDWS, &parent, None).await,
        (Some(Seg::Db(_)), Coll::Publications) => engine::list(cat, server, db, &registry::PUBLICATIONS, &parent, None).await,
        // `pg_subscription.oid` okunamazsa (42501) oid `pg_stat_subscription`'dan alınır; o da okunamazsa liste boş
        // döner (hata göstermek yerine düğüm boş kalır).
        (Some(Seg::Db(_)), Coll::Subscriptions) => {
            let denied = |e: &Error| matches!(e, Error::Query { sqlstate: Some(code), .. } if code == "42501");
            match engine::list(cat, server, db, &registry::SUBSCRIPTIONS, &parent, None).await {
                Err(e) if denied(&e) => match engine::list(cat, server, db, &registry::SUBSCRIPTIONS_BY_STATS, &parent, None).await {
                    Err(e) if denied(&e) => Ok(Vec::new()),
                    other => other,
                },
                other => other,
            }
        }
        (Some(Seg::ForeignDataWrapper(fdw)), Coll::ForeignServers) => {
            engine::list(cat, server, db, &registry::FOREIGN_SERVERS, &parent, Some(fdw)).await
        }
        (Some(Seg::ForeignServer(srv)), Coll::UserMappings) => {
            engine::list(cat, server, db, &registry::USER_MAPPINGS, &parent, Some(srv)).await
        }
        (Some(Seg::Schema(schema)), Coll::Tables) => table::list(cat, server, db_req(db)?, schema).await,
        (Some(Seg::Schema(schema)), c) => match registry::coll_def(c) {
            Some(def) if registry::schema_collections(version).iter().any(|d| d.coll == c) => {
                engine::list(cat, server, db, def, &parent, Some(schema)).await
            }
            _ => Err(invalid(id)),
        },
        (Some(Seg::Table(rel) | Seg::View(rel) | Seg::MatView(rel) | Seg::ForeignTable(rel)), Coll::Columns) => {
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
        seg @ (Seg::ForeignDataWrapper(_) | Seg::ForeignServer(_) | Seg::UserMapping(_) | Seg::ForeignTable(_) | Seg::Publication(_) | Seg::Subscription(_)) => {
            foreign_properties(cat, server, db_req(db)?, *seg).await
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
        // View/MatView kolonlarının ayrı DDL'i yok (tanım görünümün kendisinde).
        Seg::Column(_) if matches!(path.iter().rev().nth(1), Some(Seg::View(_) | Seg::MatView(_))) => Ok(String::new()),
        // Foreign table kolonu: `ALTER FOREIGN TABLE … ADD COLUMN` (kolon seçenekleriyle).
        Seg::Column(c) if matches!(path.iter().rev().nth(1), Some(Seg::ForeignTable(_))) => {
            let rel = parent_oid(path).ok_or_else(|| invalid(id))?;
            foreign_ddl(cat, server, db_req(db)?, Seg::Column(*c), Some(rel)).await
        }
        Seg::Column(c) => {
            let rel = parent_oid(path).ok_or_else(|| invalid(id))?;
            column::ddl(cat, server, db_req(db)?, rel, *c).await
        }
        seg @ (Seg::ForeignDataWrapper(_) | Seg::ForeignServer(_) | Seg::UserMapping(_) | Seg::ForeignTable(_) | Seg::Publication(_) | Seg::Subscription(_)) => {
            foreign_ddl(cat, server, db_req(db)?, *seg, None).await
        }
        seg => match registry::ddl_def(*seg) {
            Some(def) => {
                let oid = seg.oid().ok_or_else(|| invalid(id))?;
                engine::ddl(cat, server, db, def, oid).await
            }
            None => Err(invalid(id)),
        },
    }
}

type BoxFut<'a, T> = std::pin::Pin<Box<dyn std::future::Future<Output = Result<T>> + Send + 'a>>;

/// FDW/replikasyon özellikleri. Gelecek ayrı (satır içine alınmayan) işlevde kutulanır: büyük (`try_join`)
/// gelecekler `properties`/`ddl` durum makinesine gömülünce hata ayıklama derlemesinde test iş parçacığının
/// yığını taşıyordu.
#[inline(never)]
fn foreign_properties(cat: &Catalog, server: uuid::Uuid, db: u32, seg: Seg) -> BoxFut<'_, Vec<crate::props::PropertySection>> {
    match seg {
        Seg::ForeignDataWrapper(o) => Box::pin(foreign::fdw_properties(cat, server, db, o)),
        Seg::ForeignServer(o) => Box::pin(foreign::server_properties(cat, server, db, o)),
        Seg::UserMapping(o) => Box::pin(foreign::user_mapping_properties(cat, server, db, o)),
        Seg::ForeignTable(o) => Box::pin(foreign::foreign_table_properties(cat, server, db, o)),
        Seg::Publication(o) => Box::pin(replication::publication_properties(cat, server, db, o)),
        Seg::Subscription(o) => Box::pin(replication::subscription_properties(cat, server, db, o)),
        _ => Box::pin(async { Err(Error::Invalid("desteklenmeyen düğüm".into())) }),
    }
}

/// FDW/replikasyon DDL'i (`rel`: foreign table kolonunun tablosu); kutulama gerekçesi `foreign_properties`'te.
#[inline(never)]
fn foreign_ddl(cat: &Catalog, server: uuid::Uuid, db: u32, seg: Seg, rel: Option<u32>) -> BoxFut<'_, String> {
    match (seg, rel) {
        (Seg::Column(c), Some(rel)) => Box::pin(foreign::column_ddl(cat, server, db, rel, c)),
        (Seg::ForeignDataWrapper(o), _) => Box::pin(foreign::fdw_ddl(cat, server, db, o)),
        (Seg::ForeignServer(o), _) => Box::pin(foreign::server_ddl(cat, server, db, o)),
        (Seg::UserMapping(o), _) => Box::pin(foreign::user_mapping_ddl(cat, server, db, o)),
        (Seg::ForeignTable(o), _) => Box::pin(foreign::foreign_table_ddl(cat, server, db, o)),
        (Seg::Publication(o), _) => Box::pin(replication::publication_ddl(cat, server, db, o)),
        (Seg::Subscription(o), _) => Box::pin(replication::subscription_ddl(cat, server, db, o)),
        _ => Box::pin(async { Err(Error::Invalid("desteklenmeyen düğüm".into())) }),
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
