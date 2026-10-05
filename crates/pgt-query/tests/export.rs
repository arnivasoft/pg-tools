//! CSV dışa aktarma entegrasyon testleri (`PGT_TEST_URLS` yoksa atlanır; her sunucu için ayrı çalışır).

use std::path::Path;
use std::sync::{Arc, RwLock};
use std::time::Duration;

use pgt_core::Error;
use pgt_core::connection::{ConnectionManager, Target};
use pgt_core::registry::Registry;
use pgt_core::testing::{TestServer, admin_client, test_servers};
use pgt_query::events::{EventSink, QueryEvent};
use pgt_query::export::{CsvDelimiter, CsvOptions, EXPORT_CHUNK};
use pgt_query::session::{CANCELLED_MESSAGE, LOCKED_SETTING_MESSAGE, LOCKED_WRITE_MESSAGE, QueryManager, Scope, SessionId, SessionOptions, TxMode, TxState};
use tempfile::TempDir;

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
    dir: TempDir,
}

async fn fixture(ts: &TestServer) -> Fx {
    fixture_with(ts, false).await
}

async fn fixture_with(ts: &TestServer, locked: bool) -> Fx {
    let dir = tempfile::tempdir().unwrap();
    let mut registry = Registry::load(dir.path().join("servers.json")).unwrap().registry;
    let mut config = ts.config.clone();
    config.read_only_lock = locked;
    registry.upsert(config.clone()).unwrap();
    let mgr = Arc::new(ConnectionManager::new());
    mgr.connect(config, ts.password.clone()).await.unwrap();
    let qm = QueryManager::new(mgr, Arc::new(RwLock::new(registry)));
    Fx { qm: Arc::new(qm), ts: ts.clone(), dir }
}

impl Fx {
    async fn open(&self, mode: TxMode) -> SessionId {
        let opts = SessionOptions { mode, statement_timeout_ms: 0 };
        self.qm.open(self.ts.config.id, Target::Maintenance, opts).await.unwrap()
    }

    async fn run(&self, id: SessionId, sql: &str) {
        let mut sink = VecSink::default();
        self.qm.run(id, sql, Scope::All, 1000, true, &mut sink).await.unwrap();
        assert!(!sink.0.iter().any(|e| matches!(e, QueryEvent::Failed { .. })), "{:?}", sink.0);
    }

    async fn setup(&self, sql: &str) {
        admin_client(&self.ts, &self.ts.config.maintenance_db).await.batch_execute(sql).await.unwrap();
    }

    fn path(&self, name: &str) -> std::path::PathBuf {
        self.dir.path().join(name)
    }
}

fn no_bom() -> CsvOptions {
    CsvOptions { bom: false, ..CsvOptions::default() }
}

/// `<hedef>.<…>.tmp` biçiminde artık geçici dosya var mı (kullanıcının kendi `<hedef>.tmp`'si sayılmaz).
fn has_tmp(path: &Path) -> bool {
    let name = path.file_name().unwrap().to_string_lossy().to_string();
    std::fs::read_dir(path.parent().unwrap()).unwrap().any(|e| {
        let f = e.unwrap().file_name().to_string_lossy().to_string();
        f.starts_with(&name) && f.ends_with(".tmp") && f != format!("{name}.tmp")
    })
}

#[tokio::test]
async fn export_streams_without_buffering() {
    for ts in servers() {
        let fx = fixture(&ts).await;
        let id = fx.open(TxMode::Autocommit).await;
        let path = fx.path("big.csv");
        let mut seen: Vec<u64> = Vec::new();
        let summary = fx
            .qm
            .export_csv(id, "select g as n, 'row ' || g as label from generate_series(1, 200000) g", &path, no_bom(), &mut |n| {
                seen.push(n);
                true
            })
            .await
            .unwrap();
        assert_eq!(summary.rows, 200_000);
        let text = std::fs::read_to_string(&path).unwrap();
        assert_eq!(text.lines().count(), 200_001);
        assert!(text.starts_with("n,label\r\n1,row 1\r\n"));
        assert!(text.ends_with("200000,row 200000\r\n"));
        assert_eq!(summary.bytes, text.len() as u64);
        assert!(seen.windows(2).all(|w| w[0] < w[1]), "ilerleme artmalı: {seen:?}");
        assert_eq!(seen.first(), Some(&u64::from(EXPORT_CHUNK)));
        assert_eq!(seen.last(), Some(&200_000));
        let peak = fx.qm.peak_buffered_rows(id).unwrap();
        assert!(peak <= EXPORT_CHUNK as usize, "bellekteki en çok satır: {peak}");
        assert!(!has_tmp(&path));
        // Oturum temiz: açık transaction ya da imleç kalmadı.
        assert_eq!(fx.qm.tx_state(id).unwrap(), TxState::Idle);
        fx.run(id, "select 1").await;
        let mut again = |_| true;
        fx.qm.export_csv(id, "select 1", &fx.path("again.csv"), no_bom(), &mut again).await.unwrap();
    }
}

#[tokio::test]
async fn export_formats_values_and_options() {
    for ts in servers() {
        let fx = fixture(&ts).await;
        let id = fx.open(TxMode::Autocommit).await;
        let sql = "select 1 as a, null::text as \"b;c\", ''::text as d, E'x,\"y\"\\nz' as e, 'çğ日本' as f \
                   union all select 2, 'plain', ' sp ', E'cr\\rlf', '🙂'";
        let path = fx.path("fmt.csv");
        let mut ok = |_| true;
        let opts = CsvOptions { delimiter: CsvDelimiter::Semicolon, header: true, bom: true };
        let s = fx.qm.export_csv(id, sql, &path, opts, &mut ok).await.unwrap();
        let got = std::fs::read(&path).unwrap();
        assert_eq!(got.len() as u64, s.bytes);
        let expected = "\u{feff}a;\"b;c\";d;e;f\r\n1;;\"\";\"x,\"\"y\"\"\nz\";çğ日本\r\n2;plain; sp ;\"cr\rlf\";🙂\r\n";
        assert_eq!(String::from_utf8(got).unwrap(), expected);

        let opts = CsvOptions { delimiter: CsvDelimiter::Tab, header: false, bom: false };
        fx.qm.export_csv(id, "select 'a,b' as x, 'c\td' as y", &path, opts, &mut ok).await.unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "a,b\t\"c\td\"\r\n");
    }
}

#[tokio::test]
async fn export_cancel_leaves_no_file() {
    for ts in servers() {
        let fx = fixture(&ts).await;
        let id = fx.open(TxMode::Autocommit).await;
        let sql = "select g from generate_series(1, 100000) g";

        // Hedef yok: iptal sonrası ne hedef ne .tmp.
        let path = fx.path("cancel.csv");
        let mut calls = 0;
        let r = fx
            .qm
            .export_csv(id, sql, &path, no_bom(), &mut |_| {
                calls += 1;
                calls < 2
            })
            .await;
        let Err(Error::Query { message, .. }) = r else { panic!("iptal bekleniyordu: {r:?}") };
        assert_eq!(message, CANCELLED_MESSAGE);
        assert!(!path.exists() && !has_tmp(&path));

        // Önceki içerik durur.
        std::fs::write(&path, "previous").unwrap();
        let r = fx.qm.export_csv(id, sql, &path, no_bom(), &mut |_| false).await;
        assert!(r.is_err());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "previous");
        assert!(!has_tmp(&path));

        // Oturum iptali (cancel) sürmekte olan dışa aktarmayı durdurur.
        let qm = fx.qm.clone();
        let slow = fx.path("slow.csv");
        let slow2 = slow.clone();
        let task = tokio::spawn(async move {
            qm.export_csv(id, "select g, pg_sleep(0.01) from generate_series(1, 100000) g", &slow2, no_bom(), &mut |_| true)
                .await
        });
        tokio::time::sleep(Duration::from_millis(700)).await;
        assert!(slow.parent().unwrap().exists());
        fx.qm.cancel(id);
        let r = tokio::time::timeout(Duration::from_secs(10), task).await.unwrap().unwrap();
        let Err(Error::Query { message, .. }) = r else { panic!("iptal bekleniyordu: {r:?}") };
        assert_eq!(message, CANCELLED_MESSAGE);
        assert!(!slow.exists() && !has_tmp(&slow));
        assert_eq!(fx.qm.tx_state(id).unwrap(), TxState::Idle);
        fx.run(id, "select 1").await;
    }
}

#[tokio::test]
async fn export_marks_session_busy() {
    for ts in servers() {
        let fx = fixture(&ts).await;
        let id = fx.open(TxMode::Autocommit).await;
        let qm = fx.qm.clone();
        let path = fx.path("busy.csv");
        let p2 = path.clone();
        let task = tokio::spawn(async move {
            qm.export_csv(id, "select pg_sleep(30)", &p2, no_bom(), &mut |_| true).await
        });
        tokio::time::sleep(Duration::from_millis(500)).await;
        let mut sink = VecSink::default();
        let r = fx.qm.run(id, "select 1", Scope::All, 10, true, &mut sink).await;
        assert!(matches!(r, Err(Error::Invalid(m)) if m == pgt_query::session::BUSY_MESSAGE));
        let r = fx.qm.export_csv(id, "select 1", &fx.path("second.csv"), no_bom(), &mut |_| true).await;
        assert!(matches!(r, Err(Error::Invalid(_))));
        fx.qm.cancel(id);
        assert!(task.await.unwrap().is_err());
        assert!(!path.exists() && !has_tmp(&path));
    }
}

#[tokio::test]
async fn export_inside_manual_transaction_keeps_it() {
    for ts in servers() {
        let fx = fixture(&ts).await;
        fx.setup("drop table if exists qe_tx; create table qe_tx(i int);").await;
        let id = fx.open(TxMode::Manual).await;
        fx.run(id, "insert into qe_tx values (1), (2), (3)").await;
        assert_eq!(fx.qm.tx_state(id).unwrap(), TxState::InTransaction);
        let path = fx.path("tx.csv");
        let mut ok = |_| true;
        let s = fx.qm.export_csv(id, "select i from qe_tx order by i", &path, no_bom(), &mut ok).await.unwrap();
        // Dışa aktarma kullanıcının kendi (commit edilmemiş) satırlarını görür ve transaction'ı bozmaz.
        assert_eq!(s.rows, 3);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "i\r\n1\r\n2\r\n3\r\n");
        assert_eq!(fx.qm.tx_state(id).unwrap(), TxState::InTransaction);

        // Yarıda kesilen dışa aktarma da transaction'ı sürdürür.
        let r = fx.qm.export_csv(id, "select i from qe_tx", &fx.path("c.csv"), no_bom(), &mut |_| false).await;
        assert!(r.is_err());
        assert_eq!(fx.qm.tx_state(id).unwrap(), TxState::InTransaction);

        // Satırlar hâlâ aynı transaction'da: dışarıdan görünmez, ROLLBACK ile gider.
        let outside: i64 = admin_client(&ts, &ts.config.maintenance_db)
            .await
            .query_one("select count(*) from qe_tx", &[])
            .await
            .unwrap()
            .get(0);
        assert_eq!(outside, 0);
        fx.run(id, "select count(*) from qe_tx").await;
        assert_eq!(fx.qm.tx_state(id).unwrap(), TxState::InTransaction);
        fx.qm.tx(id, pgt_query::session::TxAction::Commit).await.unwrap();
        let after: i64 = admin_client(&ts, &ts.config.maintenance_db)
            .await
            .query_one("select count(*) from qe_tx", &[])
            .await
            .unwrap()
            .get(0);
        assert_eq!(after, 3);

        // Manuel kipte transaction yokken geçici transaction kullanılır: sonrasında Idle.
        let mut ok = |_| true;
        fx.qm.export_csv(id, "select 1", &fx.path("idle.csv"), no_bom(), &mut ok).await.unwrap();
        assert_eq!(fx.qm.tx_state(id).unwrap(), TxState::Idle);
    }
}

#[tokio::test]
async fn export_rejects_non_row_statements_and_failed_transactions() {
    for ts in servers() {
        let fx = fixture(&ts).await;
        fx.setup("drop table if exists qe_rej; create table qe_rej(i int);").await;
        let id = fx.open(TxMode::Autocommit).await;
        let path = fx.path("rej.csv");
        for sql in [
            "create table qe_rej2(i int)",
            "insert into qe_rej values (1)",
            "insert into qe_rej values (1) returning i",
            "update qe_rej set i = 1",
            "set search_path = public",
            "explain select 1",
            "select 1; select 2",
            "",
        ] {
            let mut ok = |_| true;
            let r = fx.qm.export_csv(id, sql, &path, no_bom(), &mut ok).await;
            assert!(matches!(r, Err(Error::Invalid(_))), "{sql}: {r:?}");
        }
        assert!(!path.exists() && !has_tmp(&path));
        let count: i64 = admin_client(&ts, &ts.config.maintenance_db)
            .await
            .query_one("select count(*) from qe_rej", &[])
            .await
            .unwrap()
            .get(0);
        assert_eq!(count, 0);

        // SQL hatası: Query hatası, dosya yok, oturum kullanılabilir.
        let mut ok = |_| true;
        let r = fx.qm.export_csv(id, "select * from yok_boyle_tablo", &path, no_bom(), &mut ok).await;
        assert!(matches!(r, Err(Error::Query { .. })), "{r:?}");
        assert!(!path.exists() && !has_tmp(&path));
        let r = fx.qm.export_csv(id, "select 1/(g-5) from generate_series(1,10) g", &path, no_bom(), &mut ok).await;
        assert!(matches!(r, Err(Error::Query { .. })), "{r:?}");
        assert!(!path.exists() && !has_tmp(&path));
        assert_eq!(fx.qm.tx_state(id).unwrap(), TxState::Idle);

        // Hata durumundaki transaction'da dışa aktarma reddedilir.
        let m = fx.open(TxMode::Manual).await;
        let mut sink = VecSink::default();
        fx.qm.run(m, "select 1/0", Scope::All, 10, true, &mut sink).await.unwrap();
        assert_eq!(fx.qm.tx_state(m).unwrap(), TxState::Failed);
        let r = fx.qm.export_csv(m, "select 1", &path, no_bom(), &mut ok).await;
        assert!(matches!(r, Err(Error::Invalid(_))), "{r:?}");
    }
}

#[tokio::test]
async fn export_target_errors_are_config_errors_without_secrets() {
    for ts in servers() {
        let fx = fixture(&ts).await;
        let id = fx.open(TxMode::Autocommit).await;
        let mut ok = |_| true;

        let missing = fx.path("yok_dizin").join("o.csv");
        let r = fx.qm.export_csv(id, "select 1", &missing, no_bom(), &mut ok).await;
        let Err(Error::Config(msg)) = r else { panic!("Config bekleniyordu: {r:?}") };
        assert!(msg.contains("yok_dizin"), "{msg}");
        assert!(!msg.contains("select"), "SQL iletiye girmemeli: {msg}");

        // Hedef bir dizin.
        let dir_target = fx.path("hedef");
        std::fs::create_dir(&dir_target).unwrap();
        let r = fx.qm.export_csv(id, "select 1", &dir_target, no_bom(), &mut ok).await;
        assert!(matches!(r, Err(Error::Config(_))), "{r:?}");

        if let Some(p) = ts.password.as_deref() {
            assert!(!msg.contains(p));
        }
        // Kullanıcının `<hedef>.tmp` dosyası ezilmez/silinmez; başarılı ve iptal edilen dışa aktarmada da.
        let target = fx.path("b.csv");
        let user_tmp = fx.path("b.csv.tmp");
        std::fs::write(&user_tmp, "user data").unwrap();
        fx.qm.export_csv(id, "select 1", &target, no_bom(), &mut ok).await.unwrap();
        assert!(fx.qm.export_csv(id, "select 1", &target, no_bom(), &mut |_| false).await.is_err());
        assert_eq!(std::fs::read_to_string(&user_tmp).unwrap(), "user data");
        assert!(target.exists() && !has_tmp(&target));
        // Oturum bozulmadı.
        assert_eq!(fx.qm.tx_state(id).unwrap(), TxState::Idle);
        fx.run(id, "select 1").await;
    }
}

fn more_ok(events: &[QueryEvent]) -> usize {
    assert!(!events.iter().any(|e| matches!(e, QueryEvent::Failed { .. })), "{events:?}");
    events
        .iter()
        .map(|e| match e {
            QueryEvent::Rows { rows, .. } => rows.len(),
            _ => 0,
        })
        .sum()
}

#[tokio::test]
async fn export_keeps_the_grids_open_cursor() {
    for ts in servers() {
        let fx = fixture(&ts).await;
        for mode in [TxMode::Autocommit, TxMode::Manual] {
            let id = fx.open(mode).await;
            let mut sink = VecSink::default();
            fx.qm.run(id, "select g from generate_series(1, 3000) g", Scope::All, 1000, true, &mut sink).await.unwrap();
            assert_eq!(more_ok(&sink.0), 1000);
            let state = fx.qm.tx_state(id).unwrap();

            let path = fx.path(&format!("grid-{mode:?}.csv"));
            let mut ok = |_| true;
            let s = fx.qm.export_csv(id, "select g from generate_series(1, 7000) g", &path, no_bom(), &mut ok).await.unwrap();
            assert_eq!(s.rows, 7000);
            // SQL hatası ve iptal de ızgara imlecini bozmaz.
            let mut ok = |_| true;
            let r = fx.qm.export_csv(id, "select 1/(g-5) from generate_series(1,10) g", &fx.path("e.csv"), no_bom(), &mut ok).await;
            assert!(matches!(r, Err(Error::Query { .. })), "{r:?}");
            assert!(fx.qm.export_csv(id, "select 1", &fx.path("e.csv"), no_bom(), &mut |_| false).await.is_err());
            assert_eq!(fx.qm.tx_state(id).unwrap(), state);

            let mut more = VecSink::default();
            fx.qm.more(id, 0, &mut more).await.unwrap();
            assert_eq!(more_ok(&more.0), 1000, "{mode:?}");
            let mut more = VecSink::default();
            fx.qm.more(id, 0, &mut more).await.unwrap();
            assert_eq!(more_ok(&more.0), 1000);
            fx.qm.close(id, pgt_query::session::ClosePolicy::Rollback).await.unwrap();
        }
    }
}

#[tokio::test]
async fn failed_or_cancelled_export_keeps_the_manual_transaction() {
    for ts in servers() {
        let fx = fixture(&ts).await;
        fx.setup("drop table if exists qe_sp; create table qe_sp(i int);").await;
        let id = fx.open(TxMode::Manual).await;
        fx.run(id, "insert into qe_sp values (1)").await;

        // SQL hatası (üçüncü parçada değil, ilk FETCH'te).
        let mut ok = |_| true;
        let r = fx.qm.export_csv(id, "select 1/(g-5) from generate_series(1,10) g", &fx.path("a.csv"), no_bom(), &mut ok).await;
        assert!(matches!(r, Err(Error::Query { .. })), "{r:?}");
        assert_eq!(fx.qm.tx_state(id).unwrap(), TxState::InTransaction);
        fx.run(id, "insert into qe_sp values (2)").await;

        // Sunucu tarafı iptal (cancel belirteci → FETCH sırasında pg_cancel).
        let qm = fx.qm.clone();
        let p = fx.path("b.csv");
        let p2 = p.clone();
        let task = tokio::spawn(async move {
            qm.export_csv(id, "select g, pg_sleep(0.01) from generate_series(1, 100000) g", &p2, no_bom(), &mut |_| true).await
        });
        tokio::time::sleep(Duration::from_millis(700)).await;
        fx.qm.cancel(id);
        let r = tokio::time::timeout(Duration::from_secs(10), task).await.unwrap().unwrap();
        let Err(Error::Query { message, .. }) = r else { panic!("iptal bekleniyordu: {r:?}") };
        assert_eq!(message, CANCELLED_MESSAGE);
        assert!(!p.exists() && !has_tmp(&p));
        assert_eq!(fx.qm.tx_state(id).unwrap(), TxState::InTransaction);

        // Transaction hâlâ kullanılabilir ve COMMIT öncekilerle birlikte çalışır.
        fx.run(id, "insert into qe_sp values (3)").await;
        fx.qm.tx(id, pgt_query::session::TxAction::Commit).await.unwrap();
        let n: i64 = admin_client(&ts, &ts.config.maintenance_db)
            .await
            .query_one("select count(*) from qe_sp", &[])
            .await
            .unwrap()
            .get(0);
        assert_eq!(n, 3);
    }
}

/// Kilitli oturumda dışa aktarma da kilit kapısından geçer: `select set_config(...)` ile salt okunur ayarı
/// kapatılamaz (sorgu hiç gönderilmez, ayar değişmez), yazma ifadesi de reddedilir.
#[tokio::test]
async fn locked_export_cannot_switch_off_read_only() {
    for ts in servers() {
        let fx = fixture_with(&ts, true).await;
        let id = fx.open(TxMode::Autocommit).await;
        let path = fx.path("gate.csv");
        let sql = "select set_config('default_transaction_read_only','off',false)";
        let err = fx.qm.export_csv(id, sql, &path, no_bom(), &mut |_| true).await.unwrap_err();
        assert!(matches!(&err, Error::Invalid(m) if m == LOCKED_SETTING_MESSAGE), "{err:?}");
        let err = fx.qm.export_csv(id, "insert into t values (1) returning *", &path, no_bom(), &mut |_| true).await.unwrap_err();
        assert!(matches!(&err, Error::Invalid(m) if m == LOCKED_WRITE_MESSAGE), "{err:?}");
        assert!(!path.exists());
        let check = fx.path("check.csv");
        fx.qm.export_csv(id, "select current_setting('default_transaction_read_only')", &check, no_bom(), &mut |_| true)
            .await
            .unwrap();
        let text = std::fs::read_to_string(&check).unwrap();
        assert!(text.lines().nth(1) == Some("on"), "{text}");
    }
}
