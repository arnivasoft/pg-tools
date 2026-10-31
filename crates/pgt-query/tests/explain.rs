//! `QueryManager::explain` entegrasyon testleri (`PGT_TEST_URLS` yoksa atlanır; her sunucu için ayrı çalışır).
//!
//! `PGT_WRITE_PLAN_FIXTURES=1` ile her sürümün `EXPLAIN (FORMAT JSON, ANALYZE, BUFFERS)` örneği ön yüz testleri için
//! `src/features/query/fixtures/plan-pg<NN>.json` olarak yazılır (planda yalnız test tablo adları bulunur).

use std::sync::{Arc, RwLock};
use std::time::{Duration, Instant};

use pgt_core::connection::{ConnectionManager, Target};
use pgt_core::registry::Registry;
use pgt_core::testing::{TestServer, admin_client, test_servers};
use pgt_core::Error;
use pgt_query::error_codes::query_payload;
use pgt_query::events::{EventSink, QueryEvent};
use pgt_query::explain::{ExplainOptions, ExplainResult, IN_TX_NOTE, ROLLED_BACK_NOTE};
use pgt_query::session::{CANCELLED_MESSAGE, ClosePolicy, QueryManager, Scope, SessionId, SessionOptions, TxAction, TxMode, TxState};
use tempfile::TempDir;

fn servers() -> Vec<TestServer> {
    let servers = test_servers();
    if servers.is_empty() {
        eprintln!("PGT_TEST_URLS tanımlı değil, test atlanıyor");
    }
    servers
}

#[derive(Default)]
struct Collect(Vec<QueryEvent>);

impl EventSink for Collect {
    fn send(&mut self, ev: QueryEvent) -> bool {
        self.0.push(ev);
        true
    }
}

impl Collect {
    fn first_value(&self) -> Option<String> {
        self.0.iter().find_map(|e| match e {
            QueryEvent::Rows { rows, .. } => rows.first().and_then(|r| r[0].clone()),
            _ => None,
        })
    }
}

struct Fx {
    qm: Arc<QueryManager>,
    ts: TestServer,
    _dir: TempDir,
}

async fn fixture_with(ts: &TestServer, locked: bool, label: Option<&str>) -> Fx {
    let dir = tempfile::tempdir().unwrap();
    let mut registry = Registry::load(dir.path().join("servers.json")).unwrap().registry;
    let mut config = ts.config.clone();
    config.read_only_lock = locked;
    config.label = label.map(str::to_string);
    registry.upsert(config.clone()).unwrap();
    let mgr = Arc::new(ConnectionManager::new());
    mgr.connect(config, ts.password.clone()).await.unwrap();
    let qm = QueryManager::new(mgr, Arc::new(RwLock::new(registry)));
    Fx { qm: Arc::new(qm), ts: ts.clone(), _dir: dir }
}

async fn fixture(ts: &TestServer) -> Fx {
    fixture_with(ts, false, None).await
}

fn analyze() -> ExplainOptions {
    ExplainOptions { analyze: true, ..ExplainOptions::default() }
}

fn code(r: &pgt_core::Result<ExplainResult>) -> String {
    match r {
        Err(e) => query_payload(e).code,
        Ok(_) => "ok".into(),
    }
}

/// JSON'un üst düzeyi `[{"Plan": {...}, …}]` olmalı.
fn plan_of(res: &ExplainResult) -> serde_json::Value {
    let v: serde_json::Value = serde_json::from_str(&res.json).expect("plan JSON ayrıştırılmalı");
    assert!(v[0]["Plan"].is_object(), "Plan anahtarı yok");
    v
}

impl Fx {
    async fn open(&self, mode: TxMode) -> SessionId {
        let opts = SessionOptions { mode, statement_timeout_ms: 0 };
        self.qm.open(self.ts.config.id, Target::Maintenance, opts).await.unwrap()
    }

    async fn explain(&self, id: SessionId, sql: &str, opts: ExplainOptions, confirmed: bool) -> pgt_core::Result<ExplainResult> {
        self.qm.explain(id, sql, Scope::All, opts, confirmed).await
    }

    async fn run(&self, id: SessionId, sql: &str) -> Collect {
        let mut sink = Collect::default();
        self.qm.run(id, sql, Scope::All, 1000, true, &mut sink).await.unwrap();
        sink
    }

    async fn admin(&self) -> tokio_postgres::Client {
        admin_client(&self.ts, &self.ts.config.maintenance_db).await
    }

    async fn setup(&self, sql: &str) {
        self.admin().await.batch_execute(sql).await.unwrap();
    }

    async fn count(&self, table: &str) -> i64 {
        self.admin().await.query_one(&format!("select count(*) from {table}"), &[]).await.unwrap().get(0)
    }

    async fn version_num(&self) -> u32 {
        let v: String = self.admin().await.query_one("show server_version_num", &[]).await.unwrap().get(0);
        v.parse().unwrap()
    }

    async fn pid(&self, id: SessionId) -> i32 {
        self.run(id, "select pg_backend_pid()").await.first_value().unwrap().parse().unwrap()
    }

    async fn backend_state(&self, pid: i32) -> Option<String> {
        self.admin()
            .await
            .query_opt("select state from pg_stat_activity where pid = $1", &[&pid])
            .await
            .unwrap()
            .map(|r| r.get(0))
    }

    /// Başka bir bağlantıdan satır kilidi alınabiliyor mu (`NOWAIT`).
    async fn row_free(&self, table: &str) -> bool {
        self.admin().await.batch_execute(&format!("select * from {table} where i = 1 for update nowait")).await.is_ok()
    }
}

const FIXTURE_QUERY: &str = "with totals as (
  select customer_id, sum(amount) as total, count(*) as n
  from qe_fx_orders
  group by customer_id
)
select c.name, t.total, t.n,
       (select max(o.amount) from qe_fx_orders o where o.customer_id = c.id) as max_amount
from qe_fx_customers c
join totals t on t.customer_id = c.id
where t.total > (select avg(total) from totals)
order by t.total desc
limit 10;";

fn has_nested(plan: &serde_json::Value) -> bool {
    plan["Plans"].as_array().is_some_and(|c| c.iter().any(|p| p["Plans"].is_array()))
}

#[tokio::test]
async fn plan_json_notices_and_fixtures() {
    for ts in servers() {
        let fx = fixture(&ts).await;
        fx.setup(
            "drop table if exists qe_fx_orders, qe_fx_customers;
             create table qe_fx_customers(id int primary key, name text not null);
             create table qe_fx_orders(id int primary key, customer_id int not null references qe_fx_customers(id),
                                       amount numeric not null);
             insert into qe_fx_customers select i, 'customer ' || i from generate_series(1, 200) i;
             insert into qe_fx_orders select i, 1 + i % 200, (i % 97) * 1.5 from generate_series(1, 5000) i;
             create index qe_fx_orders_customer on qe_fx_orders(customer_id);
             analyze qe_fx_customers; analyze qe_fx_orders;
             create or replace function qe_fx_notice() returns int language plpgsql
               as $$ begin raise notice 'qe bildirim'; return 1; end $$;",
        )
        .await;
        let id = fx.open(TxMode::Autocommit).await;
        let version = fx.version_num().await;

        // Düz EXPLAIN: ANALYZE yok, çalıştırılmaz.
        let res = fx.explain(id, "select * from qe_fx_customers where id = 1", ExplainOptions::default(), false).await.unwrap();
        let v = plan_of(&res);
        assert!(v[0]["Execution Time"].is_null());
        assert!(v[0]["Plan"]["Actual Rows"].is_null());
        assert!(!res.rolled_back);
        assert_eq!(res.note, None);
        assert!(res.sql.starts_with("EXPLAIN (FORMAT JSON"), "{}", res.sql);

        // ANALYZE + BUFFERS: iç içe düğümler (CTE, InitPlan, SubPlan, join, toplama).
        let opts = ExplainOptions { buffers: true, ..analyze() };
        let res = fx.explain(id, FIXTURE_QUERY, opts, false).await.unwrap();
        assert!(res.sql.starts_with("EXPLAIN (FORMAT JSON, ANALYZE, BUFFERS) with totals"), "{}", res.sql);
        assert!(!res.sql.ends_with(';'));
        let v = plan_of(&res);
        assert!(v[0]["Execution Time"].is_number());
        assert!(v[0]["Plan"]["Actual Loops"].is_number());
        assert!(has_nested(&v[0]["Plan"]), "iç içe plan bekleniyordu");
        assert!(res.json.contains("Shared Hit Blocks"));
        assert!(res.json.contains("CTE Scan") && res.json.contains("InitPlan") && res.json.contains("SubPlan"));
        if std::env::var("PGT_WRITE_PLAN_FIXTURES").as_deref() == Ok("1") {
            let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../src/features/query/fixtures");
            std::fs::create_dir_all(&dir).unwrap();
            let mut json = res.json.clone();
            if !json.ends_with('\n') {
                json.push('\n');
            }
            std::fs::write(dir.join(format!("plan-pg{}.json", version / 10000)), json).unwrap();
        }

        // NOTICE'lar sonuçla döner.
        let res = fx.explain(id, "select qe_fx_notice()", analyze(), false).await.unwrap();
        assert!(res.notices.iter().any(|n| n.message == "qe bildirim"), "{:?}", res.notices);
        assert_eq!(fx.qm.tx_state(id).unwrap(), TxState::Idle);
        fx.qm.close(id, ClosePolicy::Refuse).await.unwrap();
    }
}

#[tokio::test]
async fn analyze_writes_are_rolled_back() {
    for ts in servers() {
        let fx = fixture(&ts).await;
        fx.setup(
            "drop table if exists qe_rb_t, qe_rb_seq_t; create table qe_rb_t(i int);
             insert into qe_rb_t select generate_series(1, 10);",
        )
        .await;
        let id = fx.open(TxMode::Autocommit).await;
        let pid = fx.pid(id).await;

        // Tehlikeli ifade (WHERE'siz DELETE) onay ister.
        let r = fx.explain(id, "delete from qe_rb_t", analyze(), false).await;
        assert_eq!(code(&r), "confirmation_required");
        let res = fx.explain(id, "delete from qe_rb_t", analyze(), true).await.unwrap();
        plan_of(&res);
        assert!(res.rolled_back);
        assert_eq!(res.note.as_deref(), Some(ROLLED_BACK_NOTE));
        assert!(res.json.contains("\"Actual Rows\": 10"), "satırlar gerçekten silinmiş olmalı (planda)");
        assert_eq!(fx.count("qe_rb_t").await, 10, "satırlar geri gelmeli");
        assert_eq!(fx.qm.tx_state(id).unwrap(), TxState::Idle);
        assert_eq!(fx.backend_state(pid).await.as_deref(), Some("idle"));

        // EXECUTE'un gövdesi görünmez: yazıyor sayılır ve geri alınır.
        fx.run(id, "prepare qe_rb_del as delete from qe_rb_t where i > 5").await;
        let res = fx.explain(id, "execute qe_rb_del", analyze(), true).await.unwrap();
        assert!(res.rolled_back);
        assert_eq!(fx.count("qe_rb_t").await, 10);

        // CREATE TABLE AS da geri alınır.
        let res = fx.explain(id, "create table qe_rb_seq_t as select * from qe_rb_t", analyze(), true).await.unwrap();
        assert!(res.rolled_back);
        let exists: bool =
            fx.admin().await.query_one("select to_regclass('qe_rb_seq_t') is not null", &[]).await.unwrap().get(0);
        assert!(!exists);

        // Geri alma kapalıysa ifade kalıcıdır (autocommit).
        let off = ExplainOptions { rollback: false, ..analyze() };
        let res = fx.explain(id, "delete from qe_rb_t where i <= 3", off, true).await.unwrap();
        assert!(!res.rolled_back);
        assert_eq!(res.note, None);
        assert_eq!(fx.count("qe_rb_t").await, 7);
        assert_eq!(fx.qm.tx_state(id).unwrap(), TxState::Idle);

        // Manuel kip, transaction yok: yine geri alınır ve transaction açık kalmaz.
        let manual = fx.open(TxMode::Manual).await;
        let res = fx.explain(manual, "update qe_rb_t set i = i + 100 where i > 0", analyze(), true).await.unwrap();
        assert!(res.rolled_back);
        assert_eq!(fx.qm.tx_state(manual).unwrap(), TxState::Idle);
        let max: i32 = fx.admin().await.query_one("select max(i) from qe_rb_t", &[]).await.unwrap().get(0);
        assert_eq!(max, 10);
        fx.qm.close(manual, ClosePolicy::Refuse).await.unwrap();
        fx.qm.close(id, ClosePolicy::Refuse).await.unwrap();
    }
}

#[tokio::test]
async fn rollback_survives_error_cancel_and_timeout() {
    const SLOW_INSERT: &str = "insert into qe_er_t select 1 from pg_sleep(30)";
    for ts in servers() {
        let fx = fixture(&ts).await;
        fx.setup("drop table if exists qe_er_t; create table qe_er_t(i int); insert into qe_er_t values (1), (2);").await;
        let id = fx.open(TxMode::Autocommit).await;
        let pid = fx.pid(id).await;

        // Çalışma hatası: geri alınır, oturum Idle (Failed değil).
        let r = fx.explain(id, "update qe_er_t set i = i / 0 where i > 0", analyze(), true).await;
        assert_eq!(code(&r), "query", "{r:?}");
        assert_eq!(fx.qm.tx_state(id).unwrap(), TxState::Idle);
        assert_eq!(fx.backend_state(pid).await.as_deref(), Some("idle"));

        // İptal: geri alınır, ileti kullanıcı iptali.
        let qm = fx.qm.clone();
        let slow = tokio::spawn(async move { qm.explain(id, SLOW_INSERT, Scope::All, analyze(), true).await });
        tokio::time::sleep(Duration::from_millis(500)).await;
        let t = Instant::now();
        fx.qm.cancel(id);
        let r = slow.await.unwrap();
        assert!(t.elapsed() < Duration::from_secs(10));
        match &r {
            Err(Error::Query { message, .. }) => assert_eq!(message, CANCELLED_MESSAGE),
            other => panic!("iptal bekleniyordu: {other:?}"),
        }
        assert_eq!(fx.qm.tx_state(id).unwrap(), TxState::Idle);
        assert_eq!(fx.backend_state(pid).await.as_deref(), Some("idle"));
        assert_eq!(fx.count("qe_er_t").await, 2);

        // statement_timeout: zaman aşımı iletisi (iptal değil), geri alınır.
        fx.qm.set_options(id, SessionOptions { mode: TxMode::Autocommit, statement_timeout_ms: 300 }).await.unwrap();
        let r = fx.explain(id, SLOW_INSERT, analyze(), true).await;
        let p = query_payload(r.as_ref().unwrap_err());
        assert_eq!(p.code, "query");
        assert_ne!(p.message, CANCELLED_MESSAGE);
        assert_eq!(fx.qm.tx_state(id).unwrap(), TxState::Idle);
        assert_eq!(fx.backend_state(pid).await.as_deref(), Some("idle"));
        fx.qm.set_options(id, SessionOptions { mode: TxMode::Autocommit, statement_timeout_ms: 0 }).await.unwrap();

        // Oturum yeniden kullanılabilir.
        let res = fx.explain(id, "insert into qe_er_t values (3)", analyze(), true).await.unwrap();
        assert!(res.rolled_back);
        assert_eq!(fx.count("qe_er_t").await, 2);
        fx.qm.close(id, ClosePolicy::Refuse).await.unwrap();
    }
}

#[tokio::test]
async fn open_and_failed_transactions() {
    for ts in servers() {
        let fx = fixture(&ts).await;
        fx.setup("drop table if exists qe_tx_t; create table qe_tx_t(i int); insert into qe_tx_t select generate_series(1, 5);")
            .await;
        let id = fx.open(TxMode::Manual).await;

        // Manuel kip, transaction yok: düz EXPLAIN transaction açmaz; ANALYZE (yazmayan) `run` gibi BEGIN alır.
        fx.explain(id, "select * from qe_tx_t", ExplainOptions::default(), false).await.unwrap();
        assert_eq!(fx.qm.tx_state(id).unwrap(), TxState::Idle);
        fx.explain(id, "select * from qe_tx_t", analyze(), false).await.unwrap();
        assert_eq!(fx.qm.tx_state(id).unwrap(), TxState::InTransaction);
        fx.qm.tx(id, TxAction::Rollback).await.unwrap();

        // Açık transaction'a dokunulmaz: geri alma yapılmaz, etkiler transaction'da kalır.
        fx.run(id, "delete from qe_tx_t where i = 1").await;
        assert_eq!(fx.qm.tx_state(id).unwrap(), TxState::InTransaction);
        let res = fx.explain(id, "delete from qe_tx_t where i = 2", analyze(), true).await.unwrap();
        assert!(!res.rolled_back);
        assert_eq!(res.note.as_deref(), Some(IN_TX_NOTE));
        assert_eq!(fx.qm.tx_state(id).unwrap(), TxState::InTransaction);
        assert_eq!(fx.run(id, "select count(*) from qe_tx_t").await.first_value().as_deref(), Some("3"));
        fx.qm.tx(id, TxAction::Rollback).await.unwrap();
        assert_eq!(fx.count("qe_tx_t").await, 5);

        // Transaction içindeki EXPLAIN hatası transaction'ı Failed yapar; Failed'da plan alınamaz.
        fx.run(id, "select 1").await;
        assert_eq!(fx.qm.tx_state(id).unwrap(), TxState::InTransaction);
        let r = fx.explain(id, "select * from qe_tx_yok", ExplainOptions::default(), false).await;
        assert_eq!(code(&r), "query");
        assert_eq!(fx.qm.tx_state(id).unwrap(), TxState::Failed);
        let r = fx.explain(id, "select 1", ExplainOptions::default(), false).await;
        assert_eq!(code(&r), "tx_failed");
        fx.qm.tx(id, TxAction::Rollback).await.unwrap();
        fx.qm.close(id, ClosePolicy::Refuse).await.unwrap();
    }
}

#[tokio::test]
async fn analyze_row_locks_are_released() {
    for ts in servers() {
        let fx = fixture(&ts).await;
        fx.setup("drop table if exists qe_lk_t; create table qe_lk_t(i int); insert into qe_lk_t values (1), (2);").await;
        // Autocommit: `FOR UPDATE` kilidi ifadeyle biter.
        let auto = fx.open(TxMode::Autocommit).await;
        let res = fx.explain(auto, "select * from qe_lk_t where i = 1 for update", analyze(), false).await.unwrap();
        assert!(!res.rolled_back);
        assert!(fx.row_free("qe_lk_t").await, "kilit bırakılmalı");
        // Manuel kip, transaction yok: yazan ANALYZE geri alınır, satır kilidi de bırakılır.
        let manual = fx.open(TxMode::Manual).await;
        let pid = fx.pid(manual).await;
        // Manuel kipte pid sorgusu transaction açar; geri alma koşulu (transaction dışı) için kapatılır.
        fx.run(manual, "rollback").await;
        let res = fx.explain(manual, "update qe_lk_t set i = i where i = 1", analyze(), false).await.unwrap();
        assert!(res.rolled_back);
        assert!(fx.row_free("qe_lk_t").await, "geri alma kilidi bırakmalı");
        assert_eq!(fx.backend_state(pid).await.as_deref(), Some("idle"));
        fx.qm.close(auto, ClosePolicy::Refuse).await.unwrap();
        fx.qm.close(manual, ClosePolicy::Refuse).await.unwrap();
    }
}

#[tokio::test]
async fn locked_and_production_gates() {
    for ts in servers() {
        let fx = fixture_with(&ts, true, None).await;
        fx.setup("drop table if exists qe_gt_t; create table qe_gt_t(i int); insert into qe_gt_t values (1);").await;
        let id = fx.open(TxMode::Autocommit).await;
        let r = fx.explain(id, "update qe_gt_t set i = 2 where i = 1", analyze(), true).await;
        assert_eq!(code(&r), "locked");
        let res = fx.explain(id, "update qe_gt_t set i = 2 where i = 1", ExplainOptions::default(), false).await.unwrap();
        plan_of(&res);
        let res = fx.explain(id, "select * from qe_gt_t", analyze(), false).await.unwrap();
        plan_of(&res);
        assert_eq!(fx.count("qe_gt_t").await, 1);

        let prod = fixture_with(&ts, false, Some("Üretim")).await;
        let id = prod.open(TxMode::Autocommit).await;
        let r = prod.explain(id, "insert into qe_gt_t values (5)", analyze(), false).await;
        assert_eq!(code(&r), "confirmation_required");
        prod.explain(id, "insert into qe_gt_t values (5)", ExplainOptions::default(), false).await.unwrap();
        prod.explain(id, "select * from qe_gt_t", analyze(), false).await.unwrap();
        let res = prod.explain(id, "insert into qe_gt_t values (5)", analyze(), true).await.unwrap();
        assert!(res.rolled_back);
        assert_eq!(prod.count("qe_gt_t").await, 1);
    }
}

#[tokio::test]
async fn scope_option_gates_and_busy() {
    for ts in servers() {
        let fx = fixture(&ts).await;
        fx.setup("drop table if exists qe_sc_t; create table qe_sc_t(i int); insert into qe_sc_t values (1);").await;
        let id = fx.open(TxMode::Autocommit).await;
        let version = fx.version_num().await;
        let plain = ExplainOptions::default();

        // Birden çok ifade: hiçbiri çalışmaz.
        let r = fx.qm.explain(id, "select 1; drop table qe_sc_t", Scope::All, plain, true).await;
        assert_eq!(code(&r), "explain_scope");
        assert_eq!(fx.count("qe_sc_t").await, 1);
        // Seçimde sondaki `;` ve yorum atılır.
        let sql = "select * from qe_sc_t; -- son\n";
        let res = fx.qm.explain(id, sql, Scope::Selection { start: 0, end: sql.chars().count() as u32 }, plain, false).await.unwrap();
        assert!(res.sql.ends_with("select * from qe_sc_t"), "{}", res.sql);
        let r = fx.explain(id, "explain select 1", plain, false).await;
        assert_eq!(code(&r), "explain_already");
        let r = fx.explain(id, "vacuum qe_sc_t", plain, false).await;
        assert_eq!(code(&r), "explain_unsupported");

        // Sürüm kapıları: sessiz atlama yok.
        let settings = fx.explain(id, "select 1", ExplainOptions { settings: true, ..plain }, false).await;
        assert_eq!(code(&settings), if version >= 120_000 { "ok" } else { "explain_option" });
        let wal = fx.explain(id, "insert into qe_sc_t values (2)", ExplainOptions { wal: true, ..analyze() }, false).await;
        assert_eq!(code(&wal), if version >= 130_000 { "ok" } else { "explain_option" });
        if version >= 130_000 {
            assert!(wal.unwrap().json.contains("WAL Records"));
        }
        let buffers = fx.explain(id, "select 1", ExplainOptions { buffers: true, ..plain }, false).await;
        assert_eq!(code(&buffers), if version >= 130_000 { "ok" } else { "explain_option" });
        let merge = fx
            .explain(id, "merge into qe_sc_t t using (select 1 as i) s on t.i = s.i when matched then delete", plain, false)
            .await;
        assert_eq!(code(&merge), if version >= 150_000 { "ok" } else { "explain_unsupported" });
        assert_eq!(fx.count("qe_sc_t").await, 1);

        // Meşgul oturum.
        let qm = fx.qm.clone();
        let slow = tokio::spawn(async move { qm.explain(id, "select pg_sleep(5)", Scope::All, analyze(), false).await });
        tokio::time::sleep(Duration::from_millis(500)).await;
        let r = fx.explain(id, "select 1", plain, false).await;
        assert_eq!(code(&r), "busy");
        fx.qm.cancel(id);
        assert!(slow.await.unwrap().is_err());
        assert_eq!(fx.qm.tx_state(id).unwrap(), TxState::Idle);
        fx.qm.close(id, ClosePolicy::Refuse).await.unwrap();
    }
}
