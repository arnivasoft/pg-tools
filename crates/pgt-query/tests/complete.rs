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

// ---------------------------------------------------------------------------------------------------------------
// Kapsamlı (alias'lı) tamamlama
// ---------------------------------------------------------------------------------------------------------------

/// `|` imleçtir; ofset Unicode scalar olarak hesaplanır.
fn scoped(s: &str, prefix: &str) -> CompletionRequest {
    let at = s.find('|').expect("imleç");
    let sql = format!("{}{}", &s[..at], &s[at + 1..]);
    CompletionRequest::Scoped { offset: s[..at].chars().count() as u32, sql, prefix: prefix.into() }
}

fn first_cell(events: &[QueryEvent]) -> String {
    events
        .iter()
        .find_map(|e| match e {
            QueryEvent::Rows { rows, .. } => rows.first().and_then(|r| r.first().cloned().flatten()),
            _ => None,
        })
        .unwrap_or_else(|| panic!("satır yok: {events:?}"))
}

const SCOPED_FIXTURE: &str = r#"
    drop schema if exists qc_a cascade;
    drop schema if exists qc_b cascade;
    create schema qc_a;
    create schema qc_b;
    create table qc_a.t(a_col int);
    create table qc_b.t(b_col text);
    create table qc_app.ins(id serial primary key, v int);
    create table qc_app.locked(k int);
"#;

#[tokio::test]
async fn scoped_resolves_aliases_quoted_names_and_search_path() {
    let _serial = SERIAL.lock().await;
    for ts in servers() {
        let fx = fixture(&ts).await;
        fx.setup(FIXTURE).await;
        fx.setup(SCOPED_FIXTURE).await;
        let id = fx.open(TxMode::Autocommit).await;
        fx.run(id, "set search_path = nowhere").await;

        let cols = fx.complete(id, scoped("select o.| from qc_app.orders o", "o")).await;
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
        let cols = fx.complete(id, scoped("select oi.| from qc_app.\"Order Items\" oi where true", "oi")).await;
        assert_eq!(labels(&cols), ["order_id", "qty"]);
        let cols = fx.complete(id, scoped("select \"OI\".| from qc_app.\"Order Items\" \"OI\"", "\"OI\"")).await;
        assert_eq!(labels(&cols), ["order_id", "qty"]);
        // Emoji imleçten önce (scalar ofset) ve ifade sınırı.
        let cols = fx.complete(id, scoped("select 1; select '\u{1F600}', o.| from qc_app.orders o; select 2", "o")).await;
        assert_eq!(labels(&cols), ["id", "Total Price", "note"]);
        // Alias kolon listesi ilk N kolonu yeniden adlandırır.
        let cols = fx.complete(id, scoped("select x.| from qc_app.orders x(k)", "x")).await;
        assert_eq!(labels(&cols), ["k", "Total Price", "note"]);
        // Fonksiyon kaynağı: alias tek kolon.
        let cols = fx.complete(id, scoped("select g.| from generate_series(1, 3) g", "g")).await;
        assert_eq!(labels(&cols), ["g"]);
        // Select listesi: alias.kolon.
        let cols = fx.complete(id, scoped("select | from qc_app.orders o, qc_app.\"Order Items\"", "")).await;
        let got: Vec<(&str, &str)> = cols.iter().map(|c| (c.label.as_str(), c.quoted.as_str())).collect();
        assert_eq!(
            got,
            vec![
                ("o.id", "o.id"),
                ("o.Total Price", "o.\"Total Price\""),
                ("o.note", "o.note"),
                ("Order Items.order_id", "\"Order Items\".order_id"),
                ("Order Items.qty", "\"Order Items\".qty"),
            ]
        );

        // Arama yolu: şemasız ad oturumun yoluyla çözülür; yol değişince (SET önbelleği boşaltır) diğer tablo.
        assert!(fx.complete(id, scoped("select x.| from t x", "x")).await.is_empty());
        fx.run(id, "set search_path = qc_b, qc_a").await;
        assert_eq!(labels(&fx.complete(id, scoped("select x.| from t x", "x")).await), ["b_col"]);
        fx.run(id, "set search_path = qc_a, qc_b").await;
        assert_eq!(labels(&fx.complete(id, scoped("select x.| from t x", "x")).await), ["a_col"]);
        assert_eq!(first_cell(&fx.run(id, "show search_path").await), "qc_a, qc_b", "arama yoluna dokunulmaz");

        // Kapsamda eşleşme yok: Plan 5 `Columns` (önek ilişki adıdır).
        fx.run(id, "set search_path = qc_app").await;
        let cols = fx.complete(id, scoped("select orders.| from elsewhere e", "orders")).await;
        assert_eq!(labels(&cols), ["id", "Total Price", "note"]);
    }
}

#[tokio::test]
async fn scoped_cte_and_subquery_columns_come_from_parse_without_executing() {
    let _serial = SERIAL.lock().await;
    for ts in servers() {
        let fx = fixture(&ts).await;
        fx.setup(FIXTURE).await;
        fx.setup(SCOPED_FIXTURE).await;
        let id = fx.open(TxMode::Autocommit).await;
        fx.run(id, "set lock_timeout = '3s'").await;

        let sql = "with x as (insert into qc_app.ins(v) values (1) returning id, v) select x.| from x";
        let cols = fx.complete(id, scoped(sql, "x")).await;
        let got: Vec<(&str, Option<&str>)> = cols.iter().map(|c| (c.label.as_str(), c.detail.as_deref())).collect();
        assert_eq!(got, [("id", Some("int4")), ("v", Some("int4"))]);
        let admin = admin_client(&fx.ts, &fx.ts.config.maintenance_db).await;
        let n: i64 = admin.query_one("select count(*) from qc_app.ins", &[]).await.unwrap().get(0);
        assert_eq!(n, 0, "CTE gövdesi çalıştırılmamalı");
        let called: bool = admin.query_one("select is_called from qc_app.ins_id_seq", &[]).await.unwrap().get(0);
        assert!(!called, "dizi ilerlememeli");

        let cols = fx
            .complete(id, scoped("with a as (select 1 as p), b as (select p, 2 as q from a) select b.| from b", "b"))
            .await;
        assert_eq!(labels(&cols), ["p", "q"]);
        let cols = fx.complete(id, scoped("select s.| from (select id as k, note from qc_app.orders) s", "s")).await;
        assert_eq!(labels(&cols), ["k", "note"]);
        // Açık kolon listesi yalnızca ilk N kolonu adlandırır (PostgreSQL gibi).
        let cols = fx.complete(id, scoped("with c(m) as (select 1, 2) select z.| from c z", "z")).await;
        assert_eq!(labels(&cols), ["m", "?column?"]);
        let cols = fx.complete(id, scoped("select s.| from (select id, note from qc_app.orders) s(k)", "s")).await;
        assert_eq!(labels(&cols), ["k", "note"]);
        // Tabloyu gölgeleyen CTE (RECURSIVE yok): gövdedeki ad tablodur.
        fx.run(id, "set search_path = qc_app").await;
        let cols = fx.complete(id, scoped("with orders as (select id, note from orders) select o.| from orders o", "o")).await;
        assert_eq!(labels(&cols), ["id", "note"]);
        fx.run(id, "set search_path = public").await;
        // Bozuk gövde: boş, oturum etkilenmez.
        assert!(fx.complete(id, scoped("select s.| from (select nope from qc_app.orders) s", "s")).await.is_empty());
        // Kullanıcının lock_timeout'u geri yüklendi.
        assert_eq!(first_cell(&fx.run(id, "show lock_timeout").await), "3s");

        // Başka bağlantı tabloyu kilitliyor: Parse kilit beklemez (200 ms), boş döner.
        let mut locker = admin_client(&fx.ts, &fx.ts.config.maintenance_db).await;
        let tx = locker.transaction().await.unwrap();
        tx.batch_execute("lock table qc_app.locked in access exclusive mode").await.unwrap();
        let started = Instant::now();
        let locked = "with c as (select * from qc_app.locked) select c.| from c";
        assert!(fx.complete(id, scoped(locked, "c")).await.is_empty());
        assert!(started.elapsed() < Duration::from_millis(1500), "{:?}", started.elapsed());
        tx.rollback().await.unwrap();
        // Kilit zaman aşımı önbelleğe alınmaz: kilit kalkınca kolonlar gelir.
        assert_eq!(labels(&fx.complete(id, scoped(locked, "c")).await), ["k"]);
        assert_eq!(first_cell(&fx.run(id, "show lock_timeout").await), "3s");
    }
}

#[tokio::test]
async fn scoped_parse_inside_a_transaction_leaves_it_and_its_locks_untouched() {
    let _serial = SERIAL.lock().await;
    for ts in servers() {
        let fx = fixture(&ts).await;
        fx.setup(FIXTURE).await;
        fx.setup(SCOPED_FIXTURE).await;
        let id = fx.open(TxMode::Manual).await;
        let pid: i32 = first_cell(&fx.run(id, "select pg_backend_pid()").await).parse().unwrap();
        assert_eq!(fx.qm.tx_state(id).unwrap(), TxState::InTransaction);
        fx.run(id, "set local lock_timeout = '4s'").await;
        let admin = admin_client(&fx.ts, &fx.ts.config.maintenance_db).await;
        let q = "select count(*) from pg_locks where pid = $1 and relation = 'qc_app.orders'::regclass";
        assert_eq!(admin.query_one(q, &[&pid]).await.unwrap().get::<_, i64>(0), 0);

        let sql = "with c as (select id, note from qc_app.orders) select c.| from c";
        assert_eq!(labels(&fx.complete(id, scoped(sql, "c")).await), ["id", "note"]);
        assert_eq!(fx.qm.tx_state(id).unwrap(), TxState::InTransaction);
        assert_eq!(
            admin.query_one(q, &[&pid]).await.unwrap().get::<_, i64>(0),
            0,
            "Parse'ın aldığı kilit savepoint'le bırakılmalı"
        );
        // Hatalı Parse da transaction'ı bozmaz.
        assert!(fx.complete(id, scoped("select s.| from (select nope from qc_app.orders) s", "s")).await.is_empty());
        assert_eq!(fx.qm.tx_state(id).unwrap(), TxState::InTransaction);
        let ev = fx.run(id, "select current_setting('lock_timeout')").await;
        assert!(!ev.iter().any(|e| matches!(e, QueryEvent::Failed { .. })), "{ev:?}");
        assert_eq!(first_cell(&ev), "4s", "SET LOCAL değeri korunur");
        assert_eq!(admin.query_one(q, &[&pid]).await.unwrap().get::<_, i64>(0), 0);

        // Failed iken boş.
        fx.run(id, "select 1/0").await;
        assert_eq!(fx.qm.tx_state(id).unwrap(), TxState::Failed);
        let other = "with d as (select note from qc_app.orders) select d.| from d";
        assert!(fx.complete(id, scoped(other, "d")).await.is_empty());
        assert!(fx.complete(id, scoped("select o.| from qc_app.orders o", "o")).await.is_empty());
        fx.qm.tx(id, pgt_query::session::TxAction::Rollback).await.unwrap();
        assert_eq!(labels(&fx.complete(id, scoped(other, "d")).await), ["note"]);
    }
}

#[tokio::test]
async fn scoped_returns_nothing_while_busy() {
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
        let sql = "with c as (select id from qc_app.orders) select c.| from c";
        assert!(fx.complete(id, scoped(sql, "c")).await.is_empty());
        assert!(started.elapsed() < Duration::from_millis(100), "{:?}", started.elapsed());
        fx.qm.cancel(id);
        task.await.unwrap().unwrap();
        assert_eq!(labels(&fx.complete(id, scoped(sql, "c")).await), ["id"]);
    }
}
