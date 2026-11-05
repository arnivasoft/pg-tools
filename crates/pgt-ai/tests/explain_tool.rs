//! Asistanın plan aracı (`explain_query`) gerçek PostgreSQL'de. `PGT_TEST_URLS` yoksa atlanır.
//! Odak: kullanıcının arama yolu geçerli, ifade çalıştırılmaz, kullanıcının transaction'ına dokunulmaz, hatalı
//! istek oturumu bozmaz, tek ifade zorunlu. Bakım veritabanında `ai_ex` şemasını kurar ve siler.

use std::sync::{Arc, RwLock};

use pgt_ai::session_tools::explain_in_session;
use pgt_core::connection::{ConnectionManager, Target};
use pgt_core::registry::Registry;
use pgt_core::testing::{TestServer, admin_client, test_servers};
use pgt_query::events::{EventSink, QueryEvent};
use pgt_query::session::{QueryManager, Scope, SessionId, SessionOptions, TxMode, TxState};

const FIXTURE: &str = r#"
    drop schema if exists ai_ex cascade;
    create schema ai_ex;
    create table ai_ex.orders (id int primary key, c int);
    create index orders_c_idx on ai_ex.orders (c);
    insert into ai_ex.orders select g, g % 10 from generate_series(1, 100) g;
"#;

#[derive(Default)]
struct VecSink(Vec<QueryEvent>);

impl EventSink for VecSink {
    fn send(&mut self, ev: QueryEvent) -> bool {
        self.0.push(ev);
        true
    }
}

async fn run(qm: &QueryManager, id: SessionId, sql: &str) -> Vec<QueryEvent> {
    let mut sink = VecSink::default();
    qm.run(id, sql, Scope::All, 1000, true, &mut sink).await.unwrap();
    sink.0
}

async fn cell(qm: &QueryManager, id: SessionId, sql: &str) -> String {
    run(qm, id, sql)
        .await
        .iter()
        .find_map(|e| match e {
            QueryEvent::Rows { rows, .. } => rows.first().and_then(|r| r.first().cloned().flatten()),
            _ => None,
        })
        .expect("satır yok")
}

async fn roundtrip(ts: TestServer) {
    let admin = admin_client(&ts, &ts.config.maintenance_db).await;
    admin.batch_execute(FIXTURE).await.unwrap();

    let dir = tempfile::tempdir().unwrap();
    let mut registry = Registry::load(dir.path().join("servers.json")).unwrap().registry;
    registry.upsert(ts.config.clone()).unwrap();
    let mgr = Arc::new(ConnectionManager::new());
    mgr.connect(ts.config.clone(), ts.password.clone()).await.unwrap();
    let qm = Arc::new(QueryManager::new(mgr, Arc::new(RwLock::new(registry))));
    let opts = SessionOptions { mode: TxMode::Autocommit, statement_timeout_ms: 0 };
    let id = qm.open(ts.config.id, Target::Maintenance, opts).await.unwrap();
    run(&qm, id, "set search_path = ai_ex, public").await;

    // Kullanıcının arama yolu geçerli: şemasız ad çözülür; plan JSON'u döner.
    let plan = explain_in_session(qm.clone(), Some(id), "select * from orders where c = 5".into()).await.unwrap();
    assert!(plan.contains("\"Node Type\"") && plan.contains("orders"), "{plan}");

    // DML planlanır ama çalıştırılmaz.
    let plan = explain_in_session(qm.clone(), Some(id), "delete from orders".into()).await.unwrap();
    assert!(plan.contains("Delete") || plan.contains("ModifyTable"), "{plan}");
    assert_eq!(cell(&qm, id, "select count(*) from orders").await, "100");

    // Hatalı istek: araç hatası; oturum boşta kalır ve kullanılabilir.
    let err = explain_in_session(qm.clone(), Some(id), "selec 1".into()).await.unwrap_err();
    assert!(err.starts_with("EXPLAIN failed"), "{err}");
    assert_eq!(qm.tx_state(id).unwrap(), TxState::Idle);
    assert_eq!(cell(&qm, id, "select 7").await, "7");

    // Tek ifade zorunlu.
    let err = explain_in_session(qm.clone(), Some(id), "select 1; delete from orders".into()).await.unwrap_err();
    assert!(err.starts_with("EXPLAIN failed"), "{err}");
    assert_eq!(cell(&qm, id, "select count(*) from orders").await, "100");

    // Kullanıcının açık transaction'ı varsa çalışmaz ve transaction'a dokunmaz.
    run(&qm, id, "begin").await;
    run(&qm, id, "insert into orders values (1000, 1)").await;
    let err = explain_in_session(qm.clone(), Some(id), "selec 1".into()).await.unwrap_err();
    assert!(err.contains("open or failed transaction"), "{err}");
    assert_eq!(qm.tx_state(id).unwrap(), TxState::InTransaction, "transaction bozulmamalı");
    assert_eq!(cell(&qm, id, "select count(*) from orders").await, "101");
    run(&qm, id, "rollback").await;
    assert_eq!(cell(&qm, id, "select count(*) from orders").await, "100");

    // Oturum yoksa açıklayıcı hata.
    let err = explain_in_session(qm.clone(), None, "select 1".into()).await.unwrap_err();
    assert!(err.contains("no open session"), "{err}");

    qm.close_all().await;
    admin.batch_execute("drop schema ai_ex cascade").await.unwrap();
}

#[tokio::test]
async fn explain_tool_on_every_test_server() {
    let servers = test_servers();
    if servers.is_empty() {
        eprintln!("PGT_TEST_URLS yok; atlandı");
        return;
    }
    for ts in servers {
        roundtrip(ts).await;
    }
}
