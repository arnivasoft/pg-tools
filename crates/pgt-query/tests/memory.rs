//! Bellek sözleşmesi: 1 milyon satırlık sorgu ne kadar büyük olursa olsun belleğe yalnızca sınırlı sayıda satır
//! girer (`PGT_TEST_URLS` yoksa atlanır). Yalnızca sayaçlar ve boyutlar assert edilir, süre asla.
//! `PGT_QUERY_BIG=1` ile CSV dışa aktarma (1M satır) ve gecikme ölçümleri de çalışır.

use std::sync::{Arc, RwLock};
use std::time::{Duration, Instant};

use pgt_core::connection::{ConnectionManager, Target};
use pgt_core::registry::Registry;
use pgt_core::testing::{TestServer, admin_client, test_servers};
use pgt_query::complete::CompletionRequest;
use pgt_query::events::{EventSink, QueryEvent};
use pgt_query::export::{CsvOptions, EXPORT_CHUNK};
use pgt_query::session::{ClosePolicy, QueryManager, Scope, SessionId, SessionOptions, TxMode};
use tempfile::TempDir;

const BIG_SQL: &str = "select * from generate_series(1, 1000000)";

fn servers() -> Vec<TestServer> {
    let servers = test_servers();
    if servers.is_empty() {
        eprintln!("PGT_TEST_URLS tanımlı değil, test atlanıyor");
    }
    servers
}

fn big() -> bool {
    std::env::var("PGT_QUERY_BIG").is_ok_and(|v| v == "1")
}

/// Her `Rows` olayının boyutunu sayar; satırları saklamaz (yalnızca ilk hücreyi tutar).
#[derive(Default)]
struct CountSink {
    sizes: Vec<usize>,
    first: Option<String>,
    failed: bool,
}

impl EventSink for CountSink {
    fn send(&mut self, ev: QueryEvent) -> bool {
        match ev {
            QueryEvent::Rows { rows, .. } => {
                if self.first.is_none() {
                    self.first = rows.first().and_then(|r| r.first().cloned().flatten());
                }
                self.sizes.push(rows.len());
            }
            QueryEvent::Failed { .. } => self.failed = true,
            _ => {}
        }
        true
    }
}

impl CountSink {
    fn total(&self) -> usize {
        self.sizes.iter().sum()
    }
    fn max(&self) -> usize {
        self.sizes.iter().copied().max().unwrap_or(0)
    }
}

struct Fx {
    qm: Arc<QueryManager>,
    ts: TestServer,
    dir: TempDir,
}

async fn fixture(ts: &TestServer) -> Fx {
    let dir = tempfile::tempdir().unwrap();
    let mut registry = Registry::load(dir.path().join("servers.json")).unwrap().registry;
    registry.upsert(ts.config.clone()).unwrap();
    let mgr = Arc::new(ConnectionManager::new());
    mgr.connect(ts.config.clone(), ts.password.clone()).await.unwrap();
    let qm = QueryManager::new(mgr, Arc::new(RwLock::new(registry)));
    Fx { qm: Arc::new(qm), ts: ts.clone(), dir }
}

impl Fx {
    async fn open(&self) -> SessionId {
        let opts = SessionOptions { mode: TxMode::Autocommit, statement_timeout_ms: 0 };
        self.qm.open(self.ts.config.id, Target::Maintenance, opts).await.unwrap()
    }

    async fn state_of(&self, pid: i32) -> Option<String> {
        admin_client(&self.ts, &self.ts.config.maintenance_db)
            .await
            .query_opt("select state from pg_stat_activity where pid = $1", &[&pid])
            .await
            .unwrap()
            .map(|r| r.get(0))
    }
}

#[tokio::test]
async fn row_budget_holds_for_one_million_rows() {
    for ts in servers() {
        let fx = fixture(&ts).await;
        let id = fx.open().await;

        let mut pid_sink = CountSink::default();
        fx.qm.run(id, "select pg_backend_pid()", Scope::All, 10, true, &mut pid_sink).await.unwrap();
        let pid: i32 = pid_sink.first.expect("pid").parse().unwrap();

        let mut sink = CountSink::default();
        fx.qm.run(id, BIG_SQL, Scope::All, 10_000, true, &mut sink).await.unwrap();
        assert!(!sink.failed);
        assert_eq!(sink.total(), 10_000, "tam satır sınırı kadar satır");
        assert!(sink.max() <= 1000, "Rows olayı en çok 1000 satır: {}", sink.max());
        // İfade bitmeden imleç açık: autocommit ızgara imleci transaction'ı açık tutar.
        assert_eq!(fx.state_of(pid).await.as_deref(), Some("idle in transaction"));

        for round in 1..=3 {
            let mut more = CountSink::default();
            fx.qm.more(id, 0, &mut more).await.unwrap();
            assert!(!more.failed);
            assert_eq!(more.total(), 1000, "more() #{round}");
            assert!(more.max() <= 1000);
            sink.sizes.extend(more.sizes);
        }
        assert_eq!(sink.total(), 13_000);
        let peak = fx.qm.peak_buffered_rows(id).unwrap();
        assert!(peak <= 1000, "bellekteki en çok satır: {peak}");

        // Kapatma imleci ve transaction'ı bırakır; sunucuda `idle in transaction` kalmaz.
        fx.qm.close(id, ClosePolicy::Rollback).await.unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            match fx.state_of(pid).await {
                None => break,
                Some(s) => assert!(
                    s != "idle in transaction" || Instant::now() < deadline,
                    "kapatma sonrası idle in transaction kaldı"
                ),
            }
            if Instant::now() >= deadline {
                break;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        assert_ne!(fx.state_of(pid).await.as_deref(), Some("idle in transaction"));
    }
}

#[tokio::test]
async fn csv_export_streams_one_million_rows() {
    if !big() {
        eprintln!("PGT_QUERY_BIG=1 değil, 1M satırlık CSV testi atlanıyor");
        return;
    }
    for ts in servers() {
        let fx = fixture(&ts).await;
        let id = fx.open().await;
        let path = fx.dir.path().join("big.csv");
        let mut seen: Vec<u64> = Vec::new();
        let started = Instant::now();
        let summary = fx
            .qm
            .export_csv(id, BIG_SQL, &path, CsvOptions { bom: false, ..CsvOptions::default() }, &mut |n| {
                seen.push(n);
                true
            })
            .await
            .unwrap();
        let elapsed = started.elapsed();
        assert_eq!(summary.rows, 1_000_000);
        assert!(seen.windows(2).all(|w| w[0] < w[1]), "ilerleme monoton artmalı");
        assert_eq!(seen.last(), Some(&1_000_000));
        let peak = fx.qm.peak_buffered_rows(id).unwrap();
        assert!(peak <= EXPORT_CHUNK as usize, "bellekteki en çok satır: {peak}");
        assert_eq!(std::fs::metadata(&path).unwrap().len(), summary.bytes);
        eprintln!(
            "MEASURE csv_1m: {:.2} s, {:.0} satır/s, {} bayt, {} ilerleme olayı, tepe tampon {peak}",
            elapsed.as_secs_f64(),
            1_000_000.0 / elapsed.as_secs_f64(),
            summary.bytes,
            seen.len()
        );
        fx.qm.close(id, ClosePolicy::Rollback).await.unwrap();
    }
}

/// Bilgi amaçlı: tamamlama gecikmesi ve iptal süresi (yalnızca `PGT_QUERY_BIG=1`; süre assert edilmez).
#[tokio::test]
async fn measure_complete_and_cancel_latency() {
    if !big() {
        return;
    }
    for ts in servers() {
        let fx = fixture(&ts).await;
        let id = fx.open().await;
        let t = Instant::now();
        let schemas = fx.qm.complete(id, CompletionRequest::Schemas { include_system: true }).await.unwrap();
        let cold = t.elapsed();
        let t = Instant::now();
        fx.qm.complete(id, CompletionRequest::Schemas { include_system: true }).await.unwrap();
        let warm = t.elapsed();
        let t = Instant::now();
        let rels = fx.qm.complete(id, CompletionRequest::Relations { schema: Some("pg_catalog".into()) }).await.unwrap();
        let rel = t.elapsed();
        eprintln!(
            "MEASURE complete: schemas({}) soğuk {cold:?}, önbellekli {warm:?}; relations pg_catalog({}) {rel:?}",
            schemas.len(),
            rels.len()
        );

        let qm = fx.qm.clone();
        let task = tokio::spawn(async move {
            let mut sink = CountSink::default();
            let r = qm.run(id, "select pg_sleep(30)", Scope::All, 100, true, &mut sink).await;
            (r, sink)
        });
        tokio::time::sleep(Duration::from_millis(500)).await;
        let t = Instant::now();
        fx.qm.cancel(id);
        let (r, sink) = task.await.unwrap();
        let cancel = t.elapsed();
        r.unwrap();
        assert!(sink.failed);
        eprintln!("MEASURE cancel: pg_sleep(30) iptali {cancel:?}");
        fx.qm.close(id, ClosePolicy::Rollback).await.unwrap();
    }
}
