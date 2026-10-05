#![allow(dead_code)]

use std::sync::{Arc, RwLock};

use pgt_catalog::Catalog;
use pgt_catalog::node::{Coll, NodeId, Seg, TreeNode};
use pgt_core::connection::ConnectionManager;
use pgt_core::registry::Registry;
use pgt_core::testing::{TestServer, admin_client, drop_database, test_servers};
use tokio::sync::OnceCell;
use uuid::Uuid;

pub const FIXTURE_DB: &str = "pgt_fixture";
const SCHEMA_SQL: &str = include_str!("../fixtures/schema.sql");
const SCHEMA_PG11_SQL: &str = include_str!("../fixtures/schema_pg11.sql");
const SCHEMA_PG12_SQL: &str = include_str!("../fixtures/schema_pg12.sql");

static SETUP: OnceCell<()> = OnceCell::const_new();

pub struct Fixture {
    pub catalog: Catalog,
    pub mgr: Arc<ConnectionManager>,
    pub registry: Arc<RwLock<Registry>>,
    pub server: Uuid,
    pub db_oid: u32,
    pub ts: TestServer,
    _dir: tempfile::TempDir,
}

/// Her test sunucusunda fixture veritabanını (süreç başına bir kez) yeniden kurar.
async fn setup_all() {
    for ts in test_servers() {
        let admin = admin_client(&ts, &ts.config.maintenance_db).await;
        drop_database(&admin, FIXTURE_DB).await;
        // template1 yerel eklentiler/nesneler içerebilir ve kodlaması sunucuya göre değişir;
        // fixture her sunucuda aynı, temiz ve UTF8 olsun.
        admin
            .batch_execute(&format!("CREATE DATABASE {FIXTURE_DB} TEMPLATE template0 ENCODING 'UTF8'"))
            .await
            .unwrap();
        let db = admin_client(&ts, FIXTURE_DB).await;
        db.batch_execute(SCHEMA_SQL).await.unwrap();
        let version_num: i32 = db
            .query_one("SELECT current_setting('server_version_num')::int4", &[])
            .await
            .unwrap()
            .get(0);
        if version_num / 10_000 >= 11 {
            db.batch_execute(SCHEMA_PG11_SQL).await.unwrap();
        }
        if version_num / 10_000 >= 12 {
            db.batch_execute(SCHEMA_PG12_SQL).await.unwrap();
        }
        db.batch_execute("ANALYZE").await.unwrap();
    }
}

pub async fn fixtures() -> Vec<Fixture> {
    if test_servers().is_empty() {
        eprintln!("PGT_TEST_URLS tanımlı değil, test atlanıyor");
        return Vec::new();
    }
    SETUP.get_or_init(setup_all).await;
    let mut out = Vec::new();
    for ts in test_servers() {
        let dir = tempfile::tempdir().unwrap();
        let mut registry = Registry::load(dir.path().join("servers.json")).unwrap().registry;
        registry.upsert(ts.config.clone()).unwrap();
        let registry = Arc::new(RwLock::new(registry));
        let mgr = Arc::new(ConnectionManager::new());
        mgr.connect(ts.config.clone(), ts.password.clone()).await.unwrap();
        let admin = admin_client(&ts, &ts.config.maintenance_db).await;
        let db_oid: u32 = admin
            .query_one("SELECT oid FROM pg_database WHERE datname = $1", &[&FIXTURE_DB])
            .await
            .unwrap()
            .get(0);
        out.push(Fixture {
            catalog: Catalog::new(mgr.clone(), registry.clone()),
            mgr,
            registry,
            server: ts.config.id,
            db_oid,
            ts,
            _dir: dir,
        });
    }
    out
}

impl Fixture {
    pub fn server_id(&self) -> NodeId {
        NodeId::server(self.server)
    }

    pub fn db(&self) -> NodeId {
        self.server_id().child(Seg::Db(self.db_oid))
    }

    pub async fn schema(&self, name: &str) -> NodeId {
        let list = self.catalog.list_children(&self.db().child(Seg::Coll(Coll::Schemas))).await.unwrap();
        NodeId::parse(&find(&list, name).id).unwrap()
    }

    pub async fn table(&self, schema: &str, name: &str) -> NodeId {
        let schema = self.schema(schema).await;
        let list = self.catalog.list_children(&schema.child(Seg::Coll(Coll::Tables))).await.unwrap();
        NodeId::parse(&find(&list, name).id).unwrap()
    }

    /// `parent`'ın `coll` koleksiyonunun çocukları.
    pub async fn nodes_of(&self, parent: &NodeId, coll: Coll) -> Vec<TreeNode> {
        self.catalog.list_children(&parent.child(Seg::Coll(coll))).await.unwrap()
    }

    pub async fn labels_of(&self, parent: &NodeId, coll: Coll) -> Vec<String> {
        self.nodes_of(parent, coll).await.into_iter().map(|n| n.label).collect()
    }

    /// Koleksiyonda etiketi bulur ve düğümün kimliğini döndürür.
    pub async fn child_of(&self, parent: &NodeId, coll: Coll, label: &str) -> NodeId {
        let list = self.nodes_of(parent, coll).await;
        NodeId::parse(&find(&list, label).id).unwrap()
    }

    /// DDL snapshot'larını ortamdan bağımsız kılar: sahip rolü test kullanıcısıdır ve
    /// `OWNER TO x;`, `AUTHORIZATION x;`, `GRANT ... TO x;` satırlarının sonunda geçer.
    pub fn normalize(&self, ddl: &str) -> String {
        let owner = &self.ts.config.username;
        ddl.replace(&format!(" {owner};"), " <owner>;")
    }

    /// Snapshot son eki için sunucunun ana sürümü (ör. 18).
    pub async fn major(&self) -> u32 {
        self.mgr.info(self.server).expect("bağlı").version_num / 10_000
    }
}

pub fn find<'a>(list: &'a [TreeNode], label: &str) -> &'a TreeNode {
    list.iter()
        .find(|n| n.label == label)
        .unwrap_or_else(|| panic!("'{label}' bulunamadı; mevcut: {:?}", list.iter().map(|n| &n.label).collect::<Vec<_>>()))
}

pub fn labels(list: &[TreeNode]) -> Vec<&str> {
    list.iter().map(|n| n.label.as_str()).collect()
}
