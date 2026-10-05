use std::sync::{Arc, RwLock};
use std::time::{Duration, Instant};

use pgt_catalog::Catalog;
use pgt_catalog::node::{Coll, NodeId, Seg};
use pgt_core::connection::ConnectionManager;
use pgt_core::registry::Registry;
use pgt_core::testing::{admin_client, test_servers};

const PERF_DB: &str = "pgt_perf";
const TABLES: usize = 5_000;
const RUNS: usize = 5;

fn factor() -> f64 {
    std::env::var("PGT_PERF_FACTOR").ok().and_then(|v| v.parse().ok()).unwrap_or(1.0)
}

fn median(mut v: Vec<Duration>) -> Duration {
    v.sort();
    v[v.len() / 2]
}

async fn timed<T, F, Fut>(f: F) -> (Duration, T)
where
    F: Fn() -> Fut,
    Fut: std::future::Future<Output = T>,
{
    let start = Instant::now();
    let value = f().await;
    (start.elapsed(), value)
}

#[tokio::test]
async fn five_thousand_tables_stay_within_budget() {
    let servers = test_servers();
    if servers.is_empty() {
        eprintln!("PGT_TEST_URLS tanımlı değil, test atlanıyor");
        return;
    }
    for ts in servers {
        let admin = admin_client(&ts, &ts.config.maintenance_db).await;
        admin
            .execute("SELECT pg_terminate_backend(pid) FROM pg_stat_activity WHERE datname = $1 AND pid <> pg_backend_pid()", &[&PERF_DB])
            .await
            .unwrap();
        admin.batch_execute(&format!("DROP DATABASE IF EXISTS {PERF_DB}")).await.unwrap();
        admin.batch_execute(&format!("CREATE DATABASE {PERF_DB} TEMPLATE template0 ENCODING 'UTF8'")).await.unwrap();
        let db = admin_client(&ts, PERF_DB).await;
        db.batch_execute("CREATE SCHEMA perf").await.unwrap();
        // 100'lük gruplar halinde: tek transaction'da 5.000 tablo kilit tablosunu doldurabilir.
        for batch in 0..(TABLES / 100) {
            let mut sql = String::new();
            for i in 0..100 {
                let n = batch * 100 + i;
                sql.push_str(&format!("CREATE TABLE perf.t_{n:05} (id integer PRIMARY KEY, note text, created_at timestamptz DEFAULT now());\n"));
            }
            db.batch_execute(&sql).await.unwrap();
        }
        db.batch_execute("ANALYZE").await.unwrap();
        let db_oid: u32 = admin.query_one("SELECT oid FROM pg_database WHERE datname = $1", &[&PERF_DB]).await.unwrap().get(0);

        let dir = tempfile::tempdir().unwrap();
        let mut registry = Registry::load(dir.path().join("servers.json")).unwrap().registry;
        registry.upsert(ts.config.clone()).unwrap();
        let mgr = Arc::new(ConnectionManager::new());
        mgr.connect(ts.config.clone(), ts.password.clone()).await.unwrap();
        let catalog = Catalog::new(mgr.clone(), Arc::new(RwLock::new(registry)));

        let db_id = NodeId::server(ts.config.id).child(Seg::Db(db_oid));
        let schemas = catalog.list_children(&db_id.child(Seg::Coll(Coll::Schemas))).await.unwrap();
        let schema_id = NodeId::parse(&schemas.iter().find(|s| s.label == "perf").expect("perf şeması").id).unwrap();
        let tables_id = schema_id.child(Seg::Coll(Coll::Tables));

        // ısınma: bağlantıyı ve katalog önbelleklerini hazırla
        let tables = catalog.list_children(&tables_id).await.unwrap();
        assert_eq!(tables.len(), TABLES);
        let sample = NodeId::parse(&tables[TABLES / 2].id).unwrap();

        let mut list_runs = Vec::new();
        let mut props_runs = Vec::new();
        let mut ddl_runs = Vec::new();
        for _ in 0..RUNS {
            let (t, list) = timed(|| catalog.list_children(&tables_id)).await;
            assert_eq!(list.unwrap().len(), TABLES);
            list_runs.push(t);
            let (t, props) = timed(|| catalog.properties(&sample)).await;
            props.unwrap();
            props_runs.push(t);
            let (t, ddl) = timed(|| catalog.ddl(&sample)).await;
            assert!(ddl.unwrap().contains("CREATE TABLE IF NOT EXISTS perf.t_02500"));
            ddl_runs.push(t);
        }
        let (list, props, ddl) = (median(list_runs), median(props_runs), median(ddl_runs));
        eprintln!("5.000 tablo: liste {list:?}, properties {props:?}, ddl {ddl:?} (medyan, {RUNS} çalıştırma)");

        let f = factor();
        let budget = |ms: f64| Duration::from_secs_f64(ms * f / 1000.0);
        assert!(list < budget(300.0), "Tables listesi {list:?} > {:?}", budget(300.0));
        assert!(ddl < budget(200.0), "Table DDL'i {ddl:?} > {:?}", budget(200.0));
        assert!(props < budget(200.0), "Table Properties {props:?} > {:?}", budget(200.0));

        drop(db);
        mgr.disconnect(ts.config.id);
        admin
            .execute("SELECT pg_terminate_backend(pid) FROM pg_stat_activity WHERE datname = $1 AND pid <> pg_backend_pid()", &[&PERF_DB])
            .await
            .unwrap();
        admin.batch_execute(&format!("DROP DATABASE {PERF_DB}")).await.unwrap();
    }
}
