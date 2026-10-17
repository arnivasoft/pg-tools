//! Adı geçersiz UTF-8 olan multirange tipi (PG 14+; `pgt_core::testing::INVALID_UTF8_TYPE_NAME_SQL`) bulunan
//! veritabanında Explorer listeleri, özellikler ve DDL (Docker; `PGT_TEST_URLS` yoksa ya da PG < 14 ise atlanır).
//!
//! Beklenen: gösterimde ad kayıplı çözülür (U+FFFD), hiçbir yol çökmez; DDL betiği kayıplı ad içeremeyeceği için
//! adlandırılmış bir hatayla reddedilir.

use std::sync::{Arc, RwLock};

use pgt_catalog::Catalog;
use pgt_catalog::node::{Coll, NodeId, Seg, TreeNode};
use pgt_core::Error;
use pgt_core::connection::ConnectionManager;
use pgt_core::registry::Registry;
use pgt_core::testing::{INVALID_UTF8_TYPE_NAME_SQL, admin_client, drop_database, invalid_utf8_range_name, test_servers};
use tokio::sync::OnceCell;

const DB: &str = "pgt_cat_utf8";
static SETUP: OnceCell<()> = OnceCell::const_new();

struct Fx {
    catalog: Catalog,
    db: NodeId,
    _dir: tempfile::TempDir,
}

/// PG 14+ sunucular için (süreç başına bir kez kurulan) veritabanı.
async fn fixtures() -> Vec<Fx> {
    let mut out = vec![];
    let servers = test_servers();
    if servers.is_empty() {
        eprintln!("PGT_TEST_URLS tanımlı değil, test atlanıyor");
    }
    SETUP
        .get_or_init(|| async {
            for ts in test_servers() {
                let admin = admin_client(&ts, &ts.config.maintenance_db).await;
                drop_database(&admin, DB).await;
                let v: i32 = admin.query_one("SELECT current_setting('server_version_num')::int4", &[]).await.unwrap().get(0);
                if v < 140000 {
                    continue;
                }
                admin.batch_execute(&format!("CREATE DATABASE {DB} TEMPLATE template0 ENCODING 'UTF8'")).await.unwrap();
                admin_client(&ts, DB).await.batch_execute(INVALID_UTF8_TYPE_NAME_SQL).await.unwrap();
            }
        })
        .await;
    for ts in servers {
        let admin = admin_client(&ts, &ts.config.maintenance_db).await;
        let Some(row) = admin.query_opt("SELECT oid FROM pg_database WHERE datname = $1", &[&DB]).await.unwrap() else {
            eprintln!("PG < 14: multirange yok, atlanıyor");
            continue;
        };
        let dir = tempfile::tempdir().unwrap();
        let mut registry = Registry::load(dir.path().join("servers.json")).unwrap().registry;
        registry.upsert(ts.config.clone()).unwrap();
        let mgr = Arc::new(ConnectionManager::new());
        mgr.connect(ts.config.clone(), ts.password.clone()).await.unwrap();
        let db = NodeId::server(ts.config.id).child(Seg::Db(row.get(0)));
        out.push(Fx { catalog: Catalog::new(mgr, Arc::new(RwLock::new(registry))), db, _dir: dir });
    }
    out
}

impl Fx {
    async fn list(&self, parent: &NodeId, coll: Coll) -> Vec<TreeNode> {
        self.catalog.list_children(&parent.child(Seg::Coll(coll))).await.unwrap()
    }

    async fn schema(&self) -> NodeId {
        node(&self.list(&self.db, Coll::Schemas).await, |l| l == "u8")
    }
}

fn node(list: &[TreeNode], pred: impl Fn(&str) -> bool) -> NodeId {
    let n = list.iter().find(|n| pred(&n.label)).unwrap_or_else(|| panic!("yok: {:?}", list.iter().map(|n| &n.label).collect::<Vec<_>>()));
    NodeId::parse(&n.id).unwrap()
}

fn lossy(s: &str) -> bool {
    s.contains('\u{FFFD}')
}

/// DDL betiği kayıplı ad içermez: adlandırılmış `Invalid` hatası.
fn assert_ddl_refused(r: pgt_core::Result<String>) {
    match r {
        Err(Error::Invalid(m)) => assert!(m.contains("UTF-8"), "{m}"),
        other => panic!("UTF-8 reddi bekleniyordu: {other:?}"),
    }
}

#[tokio::test]
async fn types_listing_props_and_ddl() {
    for fx in fixtures().await {
        let s = fx.schema().await;
        let types = fx.list(&s, Coll::Types).await;
        let range = node(&types, |l| l == invalid_utf8_range_name());
        assert!(types.iter().all(|n| !lossy(&n.label)), "multirange/dizi listelenmez");
        fx.catalog.properties(&range).await.unwrap();
        assert!(fx.catalog.ddl(&range).await.unwrap().contains("AS RANGE"));
    }
}

#[tokio::test]
async fn domain_over_broken_type() {
    for fx in fixtures().await {
        let s = fx.schema().await;
        let doms = fx.list(&s, Coll::Domains).await;
        let d = doms.iter().find(|n| n.label == "dm").unwrap();
        assert!(d.detail.as_deref().is_some_and(lossy), "{:?}", d.detail);
        let id = NodeId::parse(&d.id).unwrap();
        let props = fx.catalog.properties(&id).await.unwrap();
        let base = props.iter().flat_map(|s| &s.rows).find(|r| r.label == "Taban tip").map(|r| r.value.clone());
        assert!(base.as_deref().is_some_and(lossy), "{base:?}");
        assert_ddl_refused(fx.catalog.ddl(&id).await);
    }
}

#[tokio::test]
async fn function_with_broken_arg_type() {
    for fx in fixtures().await {
        let s = fx.schema().await;
        let funcs = fx.list(&s, Coll::Functions).await;
        let f = node(&funcs, |l| l.starts_with("f(") && lossy(l));
        fx.catalog.properties(&f).await.unwrap();
        assert_ddl_refused(fx.catalog.ddl(&f).await);
    }
}

#[tokio::test]
async fn table_with_broken_column_type() {
    for fx in fixtures().await {
        let s = fx.schema().await;
        let t = node(&fx.list(&s, Coll::Tables).await, |l| l == "t");
        let cols = fx.list(&t, Coll::Columns).await;
        let m = cols.iter().find(|n| n.label == "m").unwrap();
        assert!(m.detail.as_deref().is_some_and(lossy), "{:?}", m.detail);
        let mid = NodeId::parse(&m.id).unwrap();
        assert_ddl_refused(fx.catalog.ddl(&mid).await);
        let props = fx.catalog.properties(&mid).await.unwrap();
        let ty = props.iter().flat_map(|s| &s.rows).find(|r| r.label == "Veri tipi").map(|r| r.value.clone());
        assert!(ty.as_deref().is_some_and(lossy), "{ty:?}");
        fx.catalog.properties(&t).await.unwrap();
        assert_ddl_refused(fx.catalog.ddl(&t).await);
    }
}
