//! Katalog tamamlama entegrasyon testleri (`PGT_TEST_URLS` yoksa atlanır; her sunucu için ayrı çalışır).

use std::sync::{Arc, RwLock};
use std::time::{Duration, Instant};

use pgt_core::connection::{ConnectionManager, Target};
use pgt_core::registry::Registry;
use pgt_core::testing::{TestServer, admin_client, test_servers};
use pgt_query::complete::{CompletionItem, CompletionKind, CompletionRequest};
use pgt_query::events::{EventSink, QueryEvent};
use pgt_query::session::{QueryManager, Scope, SessionId, SessionOptions, TxMode, TxState};
use tempfile::TempDir;

/// Testler aynı şemaları kurar; birbirini ezmesinler diye sırayla çalışır.
static SERIAL: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

fn servers() -> Vec<TestServer> {
    let servers = test_servers();
    if servers.is_empty() {
        eprintln!("PGT_TEST_URLS tanımlı değil, test atlanıyor");
    }
    servers
}

#[derive(Default)]
struct VecSink(Vec<QueryEvent>);

impl EventSink for VecSink {
    fn send(&mut self, ev: QueryEvent) -> bool {
        self.0.push(ev);
        true
    }
}

struct Fx {
    qm: Arc<QueryManager>,
    ts: TestServer,
    _dir: TempDir,
}

async fn fixture(ts: &TestServer) -> Fx {
    let dir = tempfile::tempdir().unwrap();
    let mut registry = Registry::load(dir.path().join("servers.json")).unwrap().registry;
    registry.upsert(ts.config.clone()).unwrap();
    let mgr = Arc::new(ConnectionManager::new());
    mgr.connect(ts.config.clone(), ts.password.clone()).await.unwrap();
    let qm = QueryManager::new(mgr, Arc::new(RwLock::new(registry)));
    Fx { qm: Arc::new(qm), ts: ts.clone(), _dir: dir }
}

impl Fx {
    async fn open(&self, mode: TxMode) -> SessionId {
        let opts = SessionOptions { mode, statement_timeout_ms: 0 };
        self.qm.open(self.ts.config.id, Target::Maintenance, opts).await.unwrap()
    }

    async fn run(&self, id: SessionId, sql: &str) -> Vec<QueryEvent> {
        let mut sink = VecSink::default();
        self.qm.run(id, sql, Scope::All, 1000, true, &mut sink).await.unwrap();
        sink.0
    }

    async fn setup(&self, sql: &str) {
        admin_client(&self.ts, &self.ts.config.maintenance_db).await.batch_execute(sql).await.unwrap();
    }

    async fn complete(&self, id: SessionId, req: CompletionRequest) -> Vec<CompletionItem> {
        self.qm.complete(id, req).await.unwrap()
    }
}

fn labels(items: &[CompletionItem]) -> Vec<&str> {
    items.iter().map(|i| i.label.as_str()).collect()
}

fn rel(schema: &str) -> CompletionRequest {
    CompletionRequest::Relations { schema: Some(schema.into()) }
}

const FIXTURE: &str = r#"
    drop schema if exists qc_app cascade;
    create schema qc_app;
    create table qc_app.orders(id int primary key, "Total Price" numeric(10,2), note text);
    create table qc_app."Order Items"(order_id int, qty int);
    create view qc_app.v_orders as select id from qc_app.orders;
    create materialized view qc_app.mv_orders as select id from qc_app.orders;
    alter table qc_app.orders add column dropped_me int;
    alter table qc_app.orders drop column dropped_me;
"#;

#[tokio::test]
async fn completes_schemas_relations_and_columns_independent_of_search_path() {
    let _serial = SERIAL.lock().await;
    for ts in servers() {
        let fx = fixture(&ts).await;
        fx.setup(FIXTURE).await;
        let id = fx.open(TxMode::Autocommit).await;
        // Oturumun arama yolu bozuk: sonuçlar yine de doğru olmalı.
        fx.run(id, "set search_path = nowhere").await;

        let schemas = fx.complete(id, CompletionRequest::Schemas { include_system: false }).await;
        let names = labels(&schemas);
        assert!(names.contains(&"qc_app") && names.contains(&"public"), "{names:?}");
        for sys in ["pg_catalog", "information_schema", "pg_toast"] {
            assert!(!names.contains(&sys), "{sys} gelmemeliydi: {names:?}");
        }
        assert!(schemas.iter().all(|s| s.kind == CompletionKind::Schema));
        let all = fx.complete(id, CompletionRequest::Schemas { include_system: true }).await;
        assert!(labels(&all).contains(&"pg_catalog") && labels(&all).contains(&"information_schema"));

        let rels = fx.complete(id, rel("qc_app")).await;
        let kinds: Vec<(&str, CompletionKind, &str)> =
            rels.iter().map(|r| (r.label.as_str(), r.kind, r.quoted.as_str())).collect();
        assert_eq!(
            kinds,
            vec![
                ("Order Items", CompletionKind::Table, "\"Order Items\""),
                ("mv_orders", CompletionKind::MatView, "mv_orders"),
                ("orders", CompletionKind::Table, "orders"),
                ("v_orders", CompletionKind::View, "v_orders"),
            ],
            "{rels:?}"
        );

        let cols = fx.complete(id, CompletionRequest::Columns { schema: Some("qc_app".into()), relation: "orders".into() }).await;
        let got: Vec<(&str, &str, Option<&str>)> =
            cols.iter().map(|c| (c.label.as_str(), c.quoted.as_str(), c.detail.as_deref())).collect();
        assert_eq!(
            got,
            vec![
                ("id", "id", Some("integer")),
                ("Total Price", "\"Total Price\"", Some("numeric(10,2)")),
                ("note", "note", Some("text")),
            ]
        );
        assert!(cols.iter().all(|c| c.kind == CompletionKind::Column));
        let cols = fx.complete(id, CompletionRequest::Columns { schema: Some("qc_app".into()), relation: "yok".into() }).await;
        assert!(cols.is_empty());

        // Arama yolunu kullanan istek (schema yok): yol bozukken boş, düzelince tablolar.
        assert!(fx.complete(id, CompletionRequest::Relations { schema: None }).await.is_empty());
        fx.run(id, "set search_path = qc_app").await;
        assert!(labels(&fx.complete(id, CompletionRequest::Relations { schema: None }).await).contains(&"orders"));
        let cols = fx.complete(id, CompletionRequest::Columns { schema: None, relation: "orders".into() }).await;
        assert_eq!(labels(&cols), vec!["id", "Total Price", "note"]);
    }
}

#[tokio::test]
async fn cache_is_invalidated_by_ddl_only() {
    let _serial = SERIAL.lock().await;
    for ts in servers() {
        let fx = fixture(&ts).await;
        fx.setup(FIXTURE).await;
        let id = fx.open(TxMode::Autocommit).await;
        let before = labels(&fx.complete(id, rel("qc_app")).await).len();
        assert_eq!(before, 4);

        // Oturum dışından yapılan değişiklik önbellek tazeyken görünmez.
        fx.setup("create table qc_app.other_conn(i int)").await;
        assert_eq!(fx.complete(id, rel("qc_app")).await.len(), 4, "önbellekten gelmeliydi");

        // Oturumun kendi DDL'i önbelleği boşaltır.
        fx.run(id, "create table qc_app.t_new(i int)").await;
        let after = fx.complete(id, rel("qc_app")).await;
        assert!(labels(&after).contains(&"t_new") && labels(&after).contains(&"other_conn"), "{after:?}");

        fx.setup("create table qc_app.other_two(i int)").await;
        fx.qm.invalidate_completion_cache(id);
        assert!(labels(&fx.complete(id, rel("qc_app")).await).contains(&"other_two"));
    }
}

#[tokio::test]
async fn result_count_is_capped_at_500() {
    let _serial = SERIAL.lock().await;
    for ts in servers() {
        let fx = fixture(&ts).await;
        fx.setup(
            "drop schema if exists qc_big cascade; create schema qc_big;
             do $$ begin for i in 1..600 loop execute format('create table qc_big.t%s(i int)', i); end loop; end $$;",
        )
        .await;
        let id = fx.open(TxMode::Autocommit).await;
        assert_eq!(fx.complete(id, rel("qc_big")).await.len(), 500);
    }
}

#[tokio::test]
async fn complete_never_waits_for_a_running_statement() {
    let _serial = SERIAL.lock().await;
    for ts in servers() {
        let fx = fixture(&ts).await;
        fx.setup(FIXTURE).await;
        let id = fx.open(TxMode::Autocommit).await;
        let qm = fx.qm.clone();
        let task = tokio::spawn(async move {
            let mut sink = VecSink::default();
            qm.run(id, "select pg_sleep(30)", Scope::All, 10, true, &mut sink).await
        });
        tokio::time::sleep(Duration::from_millis(500)).await;
        let started = Instant::now();
        let items = fx.complete(id, rel("qc_app")).await;
        assert!(items.is_empty());
        assert!(started.elapsed() < Duration::from_millis(100), "{:?}", started.elapsed());
        fx.qm.cancel(id);
        task.await.unwrap().unwrap();
        // Sorgu bitince tamamlama yine çalışır.
        assert_eq!(fx.complete(id, rel("qc_app")).await.len(), 4);
    }
}

#[tokio::test]
async fn complete_inside_transactions() {
    let _serial = SERIAL.lock().await;
    for ts in servers() {
        let fx = fixture(&ts).await;
        fx.setup(FIXTURE).await;
        let id = fx.open(TxMode::Manual).await;
        fx.run(id, "create table qc_app.in_tx(i int)").await;
        assert_eq!(fx.qm.tx_state(id).unwrap(), TxState::InTransaction);
        // Açık transaction'ın kendi DDL'i görünür ve transaction bozulmaz.
        let items = fx.complete(id, rel("qc_app")).await;
        assert!(labels(&items).contains(&"in_tx"), "{items:?}");
        assert_eq!(fx.qm.tx_state(id).unwrap(), TxState::InTransaction);

        // Transaction hata durumundayken boş liste.
        fx.run(id, "select 1/0").await;
        assert_eq!(fx.qm.tx_state(id).unwrap(), TxState::Failed);
        fx.qm.invalidate_completion_cache(id);
        assert!(fx.complete(id, rel("qc_app")).await.is_empty());
        fx.qm.tx(id, pgt_query::session::TxAction::Rollback).await.unwrap();
        // Geri alınan DDL önbellekten de silinmiştir.
        let items = fx.complete(id, rel("qc_app")).await;
        assert!(!labels(&items).contains(&"in_tx"), "{items:?}");
    }
}

#[tokio::test]
async fn complete_does_not_wake_a_sleeping_session() {
    let _serial = SERIAL.lock().await;
    for ts in servers() {
        let fx = fixture(&ts).await;
        fx.setup(FIXTURE).await;
        let id = fx.open(TxMode::Autocommit).await;
        fx.qm.reap_idle(Duration::ZERO);
        assert!(fx.qm.is_asleep(id).unwrap());
        assert!(fx.complete(id, rel("qc_app")).await.is_empty());
        assert!(fx.qm.is_asleep(id).unwrap(), "tamamlama oturumu uyandırmamalı");
        // Sonraki `run` yeniden bağlanır; sonra tamamlama çalışır.
        fx.run(id, "select 1").await;
        assert_eq!(fx.complete(id, rel("qc_app")).await.len(), 4);
    }
}

#[tokio::test]
async fn complete_keeps_the_grid_cursor_and_search_path_changes_invalidate() {
    let _serial = SERIAL.lock().await;
    for ts in servers() {
        let fx = fixture(&ts).await;
        fx.setup(FIXTURE).await;
        let id = fx.open(TxMode::Autocommit).await;
        let mut sink = VecSink::default();
        fx.qm.run(id, "select g from generate_series(1, 100) g", Scope::All, 10, true, &mut sink).await.unwrap();
        assert_eq!(labels(&fx.complete(id, rel("qc_app")).await).len(), 4);
        let mut more = VecSink::default();
        fx.qm.more(id, 0, &mut more).await.unwrap();
        assert!(!more.0.iter().any(|e| matches!(e, QueryEvent::Failed { .. })), "{:?}", more.0);

        // SET (Other sınıfı) arama yoluna bağlı önbelleği boşaltır: elle geçersiz kılmadan.
        fx.run(id, "set search_path = nowhere").await;
        assert!(fx.complete(id, CompletionRequest::Relations { schema: None }).await.is_empty());
        fx.run(id, "set search_path = qc_app").await;
        let items = fx.complete(id, CompletionRequest::Relations { schema: None }).await;
        assert!(labels(&items).contains(&"orders"), "{items:?}");
    }
}
