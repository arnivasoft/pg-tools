//! `QueryManager` entegrasyon testleri (`PGT_TEST_URLS` yoksa atlanır; her sunucu için ayrı çalışır).

use std::sync::{Arc, RwLock};
use std::time::{Duration, Instant};

use pgt_core::connection::{ConnectionManager, Target};
use pgt_core::registry::Registry;
use pgt_core::testing::{TestServer, admin_client, test_servers};
use pgt_core::{Error, ErrorPayload};
use pgt_query::events::{EventSink, QueryEvent};
use pgt_query::session::{
    BUSY_MESSAGE, CANCELLED_MESSAGE, ClosePolicy, LOCKED_SETTING_MESSAGE, LOCKED_WRITE_MESSAGE, MODE_IN_TX_MESSAGE,
    OPEN_TX_MESSAGE, QueryManager, Scope, SessionId, SessionOptions, TX_FAILED_MESSAGE, TxAction, TxMode, TxState,
};
use tempfile::TempDir;

fn servers() -> Vec<TestServer> {
    let servers = test_servers();
    if servers.is_empty() {
        eprintln!("PGT_TEST_URLS tanımlı değil, test atlanıyor");
    }
    servers
}

/// Olayları toplar; `accept` kadar olaydan sonra `false` döner (alıcı gitti).
#[derive(Default)]
struct VecSink {
    events: Vec<QueryEvent>,
    accept: Option<usize>,
}

impl EventSink for VecSink {
    fn send(&mut self, ev: QueryEvent) -> bool {
        if self.accept.is_some_and(|n| self.events.len() >= n) {
            return false;
        }
        self.events.push(ev);
        true
    }
}

impl VecSink {
    fn rows(&self) -> Vec<Vec<Option<String>>> {
        self.events
            .iter()
            .flat_map(|e| match e {
                QueryEvent::Rows { rows, .. } => rows.clone(),
                _ => Vec::new(),
            })
            .collect()
    }

    fn first_value(&self) -> String {
        self.rows()[0][0].clone().unwrap()
    }

    fn failed(&self) -> Option<(u32, ErrorPayload, u32)> {
        self.events.iter().find_map(|e| match e {
            QueryEvent::Failed { stmt_index, error, char_offset, .. } => Some((*stmt_index, error.clone(), *char_offset)),
            _ => None,
        })
    }

    fn script_done(&self) -> Option<(u32, bool)> {
        match self.events.last() {
            Some(QueryEvent::ScriptDone { stmts_run, stopped_on_error }) => Some((*stmts_run, *stopped_on_error)),
            _ => None,
        }
    }

    fn assert_ok(&self) {
        assert_eq!(self.failed(), None, "{:?}", self.events);
        assert!(matches!(self.script_done(), Some((_, false))), "{:?}", self.events);
    }

    fn last_more(&self) -> Option<bool> {
        self.events.iter().rev().find_map(|e| match e {
            QueryEvent::Rows { more, .. } => Some(*more),
            _ => None,
        })
    }
}

struct Fx {
    qm: Arc<QueryManager>,
    ts: TestServer,
    _dir: TempDir,
}

async fn fixture_with(ts: &TestServer, locked: bool, label: Option<&str>, cursor_idle: Option<Duration>) -> Fx {
    let dir = tempfile::tempdir().unwrap();
    let mut registry = Registry::load(dir.path().join("servers.json")).unwrap().registry;
    let mut config = ts.config.clone();
    config.read_only_lock = locked;
    config.label = label.map(str::to_string);
    registry.upsert(config.clone()).unwrap();
    let mgr = Arc::new(ConnectionManager::new());
    mgr.connect(config, ts.password.clone()).await.unwrap();
    let mut qm = QueryManager::new(mgr, Arc::new(RwLock::new(registry)));
    if let Some(d) = cursor_idle {
        qm.cursor_idle = d;
    }
    Fx { qm: Arc::new(qm), ts: ts.clone(), _dir: dir }
}

async fn fixture(ts: &TestServer) -> Fx {
    fixture_with(ts, false, None, None).await
}

impl Fx {
    async fn open(&self, mode: TxMode) -> SessionId {
        let opts = SessionOptions { mode, statement_timeout_ms: 0 };
        self.qm.open(self.ts.config.id, Target::Maintenance, opts).await.unwrap()
    }

    async fn run_with(&self, id: SessionId, sql: &str, limit: u32) -> (pgt_core::Result<()>, VecSink) {
        let mut sink = VecSink::default();
        let r = self.qm.run(id, sql, Scope::All, limit, true, &mut sink).await;
        (r, sink)
    }

    /// Onaylı çalıştırma; `Err` dönmemeli.
    async fn run(&self, id: SessionId, sql: &str) -> VecSink {
        let (r, sink) = self.run_with(id, sql, 1000).await;
        r.unwrap_or_else(|e| panic!("run hata verdi: {e:?}"));
        sink
    }

    async fn run_ok(&self, id: SessionId, sql: &str) -> VecSink {
        let sink = self.run(id, sql).await;
        sink.assert_ok();
        sink
    }

    async fn admin(&self) -> tokio_postgres::Client {
        admin_client(&self.ts, &self.ts.config.maintenance_db).await
    }

    async fn count(&self, table: &str) -> i64 {
        self.admin().await.query_one(&format!("select count(*) from {table}"), &[]).await.unwrap().get(0)
    }

    async fn setup(&self, sql: &str) {
        self.admin().await.batch_execute(sql).await.unwrap();
    }

    async fn backend_state(&self, pid: i32) -> Option<String> {
        self.admin()
            .await
            .query_opt("select state from pg_stat_activity where pid = $1", &[&pid])
            .await
            .unwrap()
            .map(|r| r.get(0))
    }
}

fn invalid_message(r: pgt_core::Result<()>) -> String {
    match r {
        Err(Error::Invalid(m)) => m,
        other => panic!("Invalid bekleniyordu: {other:?}"),
    }
}

#[tokio::test]
async fn locked_session_rejects_writes_even_if_classifier_misses() {
    for ts in servers() {
        let fx = fixture_with(&ts, true, None, None).await;
        fx.setup(
            "drop table if exists qs_locked; create table qs_locked(i int);
             create or replace function qs_f_that_writes() returns int language plpgsql
               as $$ begin insert into qs_locked values (1); return 1; end $$;",
        )
        .await;
        let id = fx.open(TxMode::Autocommit).await;
        assert_eq!(fx.run_ok(id, "show default_transaction_read_only").await.first_value(), "on");

        // İstemci tarafı: sınıflandırıcının yazma olarak gördükleri hiç gönderilmez.
        for sql in ["insert into qs_locked values (1)", "with x as (select 1) insert into qs_locked select * from x"] {
            let (r, sink) = fx.run_with(id, sql, 100).await;
            assert_eq!(invalid_message(r), LOCKED_WRITE_MESSAGE, "{sql}");
            assert!(sink.events.is_empty());
        }
        // Sunucu tarafı: sınıflandırıcıyı atlatanlar 25006 (read-only transaction) ile reddedilir.
        for sql in ["do $$ begin insert into qs_locked values (1); end $$", "select qs_f_that_writes()"] {
            let sink = fx.run(id, sql).await;
            let (_, error, _) = sink.failed().unwrap_or_else(|| panic!("{sql}: {:?}", sink.events));
            assert!(error.message.contains("read-only"), "{sql}: {error:?}");
            assert_eq!(sink.script_done(), Some((1, true)));
            assert_eq!(fx.qm.tx_state(id).unwrap(), TxState::Idle);
        }
        assert_eq!(fx.count("qs_locked").await, 0);
        fx.qm.close(id, ClosePolicy::Refuse).await.unwrap();
    }
}

#[tokio::test]
async fn read_only_escapes_are_rejected_only_when_locked() {
    const ESCAPES: [&str; 4] = [
        "set default_transaction_read_only = off",
        "begin read write",
        "select set_config('default_transaction_read_only','off',false)",
        "reset all",
    ];
    // Hesaplanmış adla `set_config` yalnızca kilitliyken reddedilir (her `set_config` çağrısı işaretlenir).
    const COMPUTED: &str = "select set_config('default_transaction_' || 'read_only','off',false)";
    for ts in servers() {
        let locked = fixture_with(&ts, true, None, None).await;
        let id = locked.open(TxMode::Autocommit).await;
        for sql in ESCAPES {
            let (r, sink) = locked.run_with(id, &format!("select 1; {sql}"), 100).await;
            assert_eq!(invalid_message(r), LOCKED_SETTING_MESSAGE, "{sql}");
            assert!(sink.events.is_empty(), "betiğin hiçbir ifadesi çalışmaz");
        }
        let (r, _) = locked.run_with(id, COMPUTED, 100).await;
        assert_eq!(invalid_message(r), LOCKED_SETTING_MESSAGE);
        assert_eq!(locked.run_ok(id, "show default_transaction_read_only").await.first_value(), "on");

        let plain = fixture(&ts).await;
        let id = plain.open(TxMode::Autocommit).await;
        for sql in ESCAPES.into_iter().chain([COMPUTED]) {
            plain.run_ok(id, sql).await;
            if sql.starts_with("begin") {
                plain.run_ok(id, "rollback").await;
            }
        }
        assert_eq!(plain.qm.tx_state(id).unwrap(), TxState::Idle);
    }
}

/// Bilinen sınır: kilit bir **korumadır, güvenlik sınırı değildir**. Sınıflandırıcı dinamik SQL'i göremez;
/// kullanıcı `default_transaction_read_only`'yi böyle kapatırsa sonraki (sınıflandırıcının yazma görmediği)
/// ifadeler yazabilir. Gerçek yetki sınırı rolün veritabanı yetkileridir; bu test davranışı belgelemek içindir.
#[tokio::test]
async fn lock_is_a_guard_not_a_security_boundary() {
    let evade = "do $$ begin execute 'set default_transaction_' || 'read_only = off'; end $$";
    assert!(!pgt_query::classify::analyze(evade).changes_read_only, "örnek sınıflandırıcıyı gerçekten atlatmalı");
    for ts in servers() {
        let fx = fixture_with(&ts, true, None, None).await;
        fx.setup("drop table if exists qs_guard; create table qs_guard(i int);").await;
        let id = fx.open(TxMode::Autocommit).await;
        fx.run_ok(id, evade).await;
        assert_eq!(fx.run_ok(id, "show default_transaction_read_only").await.first_value(), "off");
        fx.run_ok(id, "do $$ begin insert into qs_guard values (1); end $$").await;
        assert_eq!(fx.count("qs_guard").await, 1, "kilit atlatıldı: belgelenmiş sınır");
        // Doğrudan yazma ifadeleri yine istemci tarafında reddedilir.
        let (r, _) = fx.run_with(id, "insert into qs_guard values (2)", 100).await;
        assert_eq!(invalid_message(r), LOCKED_WRITE_MESSAGE);
    }
}

#[tokio::test]
async fn tx_state_follows_user_begin_commit() {
    for ts in servers() {
        let fx = fixture(&ts).await;
        let id = fx.open(TxMode::Autocommit).await;
        fx.run_ok(id, "create temp table t(i int)").await;
        fx.run_ok(id, "begin; insert into t values (1)").await;
        assert_eq!(fx.qm.tx_state(id).unwrap(), TxState::InTransaction);
        fx.run_ok(id, "savepoint a; insert into t values (2); rollback to a").await;
        assert_eq!(fx.qm.tx_state(id).unwrap(), TxState::InTransaction);
        fx.run_ok(id, "commit").await;
        assert_eq!(fx.qm.tx_state(id).unwrap(), TxState::Idle);
        assert_eq!(fx.run_ok(id, "select count(*) from t").await.first_value(), "1");
        fx.qm.close(id, ClosePolicy::Refuse).await.unwrap();
    }
}

#[tokio::test]
async fn failed_transaction_blocks_until_rollback() {
    for ts in servers() {
        let fx = fixture(&ts).await;
        let id = fx.open(TxMode::Autocommit).await;
        let sink = fx.run(id, "begin; select 1/0").await;
        assert_eq!(sink.failed().map(|f| f.0), Some(1));
        assert_eq!(fx.qm.tx_state(id).unwrap(), TxState::Failed);
        let (r, sink) = fx.run_with(id, "select 1", 100).await;
        assert_eq!(invalid_message(r), TX_FAILED_MESSAGE);
        assert!(sink.events.is_empty());
        assert!(matches!(fx.qm.tx(id, TxAction::Commit).await, Err(Error::Invalid(m)) if m == TX_FAILED_MESSAGE));
        fx.run_ok(id, "rollback").await;
        assert_eq!(fx.qm.tx_state(id).unwrap(), TxState::Idle);
        fx.run_ok(id, "select 1").await;

        // ROLLBACK TO savepoint transaction'ı sürdürür.
        fx.run(id, "begin; savepoint s; select 1/0").await;
        assert_eq!(fx.qm.tx_state(id).unwrap(), TxState::Failed);
        fx.run_ok(id, "rollback to s").await;
        assert_eq!(fx.qm.tx_state(id).unwrap(), TxState::InTransaction);
        assert_eq!(fx.qm.tx(id, TxAction::Rollback).await.unwrap(), TxState::Idle);
    }
}

#[tokio::test]
async fn script_stops_at_first_error_with_indices() {
    for ts in servers() {
        let fx = fixture(&ts).await;
        let id = fx.open(TxMode::Autocommit).await;
        let sink = fx.run(id, "select 1; select 1/0; select 2").await;
        let started: Vec<(u32, u32)> = sink
            .events
            .iter()
            .filter_map(|e| match e {
                QueryEvent::Started { stmt_index, char_offset, .. } => Some((*stmt_index, *char_offset)),
                _ => None,
            })
            .collect();
        assert_eq!(started, [(0, 0), (1, 10)]);
        let (idx, _, offset) = sink.failed().unwrap();
        assert_eq!((idx, offset), (1, 10));
        assert_eq!(sink.script_done(), Some((2, true)));
        assert_eq!(fx.qm.tx_state(id).unwrap(), TxState::Idle);

        // Seçim kapsamı: indeksler seçim içi, ofsetler editör metnine göre.
        let mut sel = VecSink::default();
        let sql = "select 1; select 'ğ'; select 3";
        fx.qm.run(id, sql, Scope::Selection { start: 10, end: 30 }, 100, false, &mut sel).await.unwrap();
        let started: Vec<(u32, u32)> = sel
            .events
            .iter()
            .filter_map(|e| match e {
                QueryEvent::Started { stmt_index, char_offset, .. } => Some((*stmt_index, *char_offset)),
                _ => None,
            })
            .collect();
        assert_eq!(started, [(0, 10), (1, 22)]);
        assert_eq!(sel.script_done(), Some((2, false)));
    }
}

#[tokio::test]
async fn manual_mode_auto_begins() {
    for ts in servers() {
        let fx = fixture(&ts).await;
        fx.setup("drop table if exists qs_manual; create table qs_manual(i int);").await;
        let id = fx.open(TxMode::Manual).await;
        fx.run_ok(id, "insert into qs_manual values (1)").await;
        assert_eq!(fx.qm.tx_state(id).unwrap(), TxState::InTransaction);
        assert_eq!(fx.count("qs_manual").await, 0, "onaylanmadı");
        assert_eq!(fx.qm.tx(id, TxAction::Rollback).await.unwrap(), TxState::Idle);
        assert_eq!(fx.count("qs_manual").await, 0);

        fx.run_ok(id, "insert into qs_manual values (2)").await;
        fx.run_ok(id, "select * from generate_series(1, 10)").await;
        assert_eq!(fx.qm.tx_state(id).unwrap(), TxState::InTransaction);
        assert_eq!(fx.qm.tx(id, TxAction::Commit).await.unwrap(), TxState::Idle);
        assert_eq!(fx.count("qs_manual").await, 1);

        // Kullanıcının COMMIT'i de transaction'ı bitirir; sonraki ifade yeniden BEGIN alır.
        fx.run_ok(id, "insert into qs_manual values (3); commit").await;
        assert_eq!(fx.qm.tx_state(id).unwrap(), TxState::Idle);
        assert_eq!(fx.count("qs_manual").await, 2);
    }
}

#[tokio::test]
async fn close_refuses_open_transaction() {
    for ts in servers() {
        let fx = fixture(&ts).await;
        let id = fx.open(TxMode::Autocommit).await;
        fx.run_ok(id, "begin").await;
        let r = fx.qm.close(id, ClosePolicy::Refuse).await;
        assert_eq!(invalid_message(r), OPEN_TX_MESSAGE);
        assert_eq!(fx.qm.tx_state(id).unwrap(), TxState::InTransaction, "oturum açık kalır");
        fx.qm.close(id, ClosePolicy::Rollback).await.unwrap();
        assert!(matches!(fx.qm.tx_state(id), Err(Error::NotFound(_))));

        fx.setup("drop table if exists qs_close_commit; create table qs_close_commit(i int);").await;
        let id = fx.open(TxMode::Manual).await;
        fx.run_ok(id, "insert into qs_close_commit values (1)").await;
        fx.qm.close(id, ClosePolicy::Commit).await.unwrap();
        assert_eq!(fx.count("qs_close_commit").await, 1);
    }
}

/// Sekme bir ifade çalışırken kapatılırsa (`close(Rollback)`): ifade iptal edilir, transaction geri alınır,
/// bağlantı kapanır ve oturum kaldırılır. `Commit` meşgulken reddedilir, `Refuse` açık transaction'ı bildirir.
#[tokio::test]
async fn busy_close_rollback_cancels_and_drops_the_session() {
    for ts in servers() {
        let fx = fixture(&ts).await;
        fx.setup("drop table if exists qs_busy_close; create table qs_busy_close(i int);").await;
        let id = fx.open(TxMode::Manual).await;
        fx.run_ok(id, "insert into qs_busy_close values (1)").await;
        let pid: i32 = fx.run_ok(id, "select pg_backend_pid()").await.first_value().parse().unwrap();
        assert_eq!(fx.qm.tx_state(id).unwrap(), TxState::InTransaction);
        let qm = fx.qm.clone();
        let task = tokio::spawn(async move {
            let mut sink = VecSink::default();
            let r = qm.run(id, "select pg_sleep(30)", Scope::All, 100, false, &mut sink).await;
            (r, sink)
        });
        tokio::time::sleep(Duration::from_millis(500)).await;
        assert_eq!(invalid_message(fx.qm.close(id, ClosePolicy::Commit).await), BUSY_MESSAGE);
        assert_eq!(invalid_message(fx.qm.close(id, ClosePolicy::Refuse).await), OPEN_TX_MESSAGE);

        let started = Instant::now();
        fx.qm.close(id, ClosePolicy::Rollback).await.unwrap();
        assert!(started.elapsed() < Duration::from_secs(10), "{:?}", started.elapsed());
        assert!(matches!(fx.qm.tx_state(id), Err(Error::NotFound(_))));
        let (r, sink) = task.await.unwrap();
        r.unwrap();
        assert_eq!(sink.failed().unwrap().1.message, CANCELLED_MESSAGE);

        let deadline = Instant::now() + Duration::from_secs(5);
        while fx.backend_state(pid).await.is_some() {
            assert!(Instant::now() < deadline, "oturumun bağlantısı kapanmalı");
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        assert_eq!(fx.count("qs_busy_close").await, 0, "transaction geri alındı");
        assert!(matches!(fx.qm.run(id, "select 1", Scope::All, 100, false, &mut VecSink::default()).await, Err(Error::NotFound(_))));
    }
}

/// Manuel kipte SAVEPOINT de otomatik BEGIN alır (transaction dışında 25P01 vermez).
#[tokio::test]
async fn manual_mode_auto_begins_before_savepoint() {
    for ts in servers() {
        let fx = fixture(&ts).await;
        let id = fx.open(TxMode::Manual).await;
        fx.run_ok(id, "savepoint a; select 1; rollback to a; release a").await;
        assert_eq!(fx.qm.tx_state(id).unwrap(), TxState::InTransaction);
        assert_eq!(fx.qm.tx(id, TxAction::Rollback).await.unwrap(), TxState::Idle);
    }
}

#[tokio::test]
async fn second_run_is_rejected_while_running() {
    for ts in servers() {
        let fx = fixture(&ts).await;
        let id = fx.open(TxMode::Autocommit).await;
        let qm = fx.qm.clone();
        let task = tokio::spawn(async move {
            let mut sink = VecSink::default();
            let r = qm.run(id, "select pg_sleep(5)", Scope::All, 100, false, &mut sink).await;
            (r, sink)
        });
        tokio::time::sleep(Duration::from_millis(500)).await;
        let (r, _) = fx.run_with(id, "select 1", 100).await;
        assert_eq!(invalid_message(r), BUSY_MESSAGE);
        assert_eq!(invalid_message(fx.qm.more(id, 0, &mut VecSink::default()).await), BUSY_MESSAGE);
        assert!(matches!(fx.qm.tx(id, TxAction::Rollback).await, Err(Error::Invalid(m)) if m == BUSY_MESSAGE));
        fx.qm.cancel(id);
        let (r, _) = task.await.unwrap();
        r.unwrap();
        fx.run_ok(id, "select 1").await;
    }
}

#[tokio::test]
async fn cancel_while_running() {
    for ts in servers() {
        let fx = fixture(&ts).await;
        let id = fx.open(TxMode::Autocommit).await;
        let qm = fx.qm.clone();
        let started = Instant::now();
        let task = tokio::spawn(async move {
            let mut sink = VecSink::default();
            let r = qm.run(id, "select 1; select pg_sleep(30); select 2", Scope::All, 100, false, &mut sink).await;
            (r, sink)
        });
        tokio::time::sleep(Duration::from_millis(700)).await;
        fx.qm.cancel(id);
        let (r, sink) = task.await.unwrap();
        r.unwrap();
        assert!(started.elapsed() < Duration::from_secs(10), "{:?}", started.elapsed());
        let (idx, error, _) = sink.failed().expect("Failed bekleniyordu");
        assert_eq!((idx, error.code.as_str(), error.message.as_str()), (1, "query", CANCELLED_MESSAGE));
        assert!(error.detail.is_some_and(|d| d.contains("cancel")));
        assert_eq!(sink.script_done(), Some((2, true)));
        assert_eq!(fx.qm.tx_state(id).unwrap(), TxState::Idle);

        // Boştayken gelen iptal sonraki ifadeyi vurmaz.
        fx.qm.cancel(id);
        fx.run_ok(id, "select pg_sleep(0.3)").await;

        // statement_timeout kendi iletisini korur.
        fx.qm.set_options(id, SessionOptions { mode: TxMode::Autocommit, statement_timeout_ms: 200 }).await.unwrap();
        let sink = fx.run(id, "select pg_sleep(3)").await;
        let (_, error, _) = sink.failed().unwrap();
        assert!(error.message.contains("statement_timeout"), "{error:?}");
        assert_ne!(error.message, CANCELLED_MESSAGE);

        // Kullanıcı transaction'ında iptal: transaction hata durumuna geçer.
        fx.qm.set_options(id, SessionOptions::default()).await.unwrap();
        fx.run_ok(id, "begin").await;
        let qm = fx.qm.clone();
        let task = tokio::spawn(async move {
            let mut sink = VecSink::default();
            qm.run(id, "select pg_sleep(30)", Scope::All, 100, false, &mut sink).await.unwrap();
            sink
        });
        tokio::time::sleep(Duration::from_millis(500)).await;
        fx.qm.cancel(id);
        let sink = task.await.unwrap();
        assert_eq!(sink.failed().unwrap().1.message, CANCELLED_MESSAGE);
        assert_eq!(fx.qm.tx_state(id).unwrap(), TxState::Failed);
        fx.run_ok(id, "rollback").await;
    }
}

/// Doğrudan yoldaki ifadenin alıcısı giderse çalıştırma durur ve sunucuya iptal isteği gönderilir. Bilinen
/// sınır: doğrudan yoldaki ifadeler (INSERT … RETURNING, EXPLAIN, EXECUTE, SHOW…) PostgreSQL'de önce tamamen
/// çalışır, sonuç sonra gönderilir; ilk satırlar geldiğinde ifade bitmiştir ve iptal onu geri alamaz (sunucu
/// gönderim sırasında iptali işlemez). İptal, satır üreterek akan bir ifade için savunmadır. Bu test, gönderilen
/// iptalin sonraki ifadeleri vurmadığını ve oturumun tutarlı kaldığını doğrular.
#[tokio::test]
async fn sink_disconnect_stops_direct_query_and_keeps_session_usable() {
    for ts in servers() {
        let fx = fixture(&ts).await;
        let id = fx.open(TxMode::Autocommit).await;
        fx.run_ok(id, "create temp table d(i int, t text)").await;
        let mut sink = VecSink { events: Vec::new(), accept: Some(1) };
        let sql = "insert into d select i, repeat('x', 100) from generate_series(1, 200000) i returning *";
        fx.qm.run(id, sql, Scope::All, 5000, false, &mut sink).await.unwrap();
        assert!(matches!(sink.events.as_slice(), [QueryEvent::Started { .. }]), "ScriptDone gönderilmez: {:?}", sink.events);
        assert_eq!(fx.qm.tx_state(id).unwrap(), TxState::Idle);
        // İptal isteği bekleyen bir sonraki ifadeye taşmaz.
        fx.run_ok(id, "select pg_sleep(0.5)").await;
        assert_eq!(fx.run_ok(id, "select count(*) from d").await.first_value(), "200000");
    }
}

#[tokio::test]
async fn set_options_changes_statement_timeout() {
    for ts in servers() {
        let fx = fixture(&ts).await;
        let id = fx.open(TxMode::Autocommit).await;
        assert_eq!(fx.run_ok(id, "show statement_timeout").await.first_value(), "0");
        fx.qm.set_options(id, SessionOptions { mode: TxMode::Autocommit, statement_timeout_ms: 1234 }).await.unwrap();
        assert_eq!(fx.run_ok(id, "show statement_timeout").await.first_value(), "1234ms");

        // Transaction içinde değiştirilen değer ROLLBACK sonrası yeniden uygulanır.
        fx.run_ok(id, "begin").await;
        fx.qm.set_options(id, SessionOptions { mode: TxMode::Autocommit, statement_timeout_ms: 2000 }).await.unwrap();
        assert_eq!(fx.run_ok(id, "show statement_timeout").await.first_value(), "2s");
        fx.run_ok(id, "rollback").await;
        assert_eq!(fx.run_ok(id, "show statement_timeout").await.first_value(), "2s");

        let too_long = SessionOptions { mode: TxMode::Autocommit, statement_timeout_ms: u32::MAX };
        assert!(matches!(fx.qm.set_options(id, too_long).await, Err(Error::Invalid(_))));
    }
}

#[tokio::test]
async fn mode_change_rejected_in_transaction() {
    for ts in servers() {
        let fx = fixture(&ts).await;
        let id = fx.open(TxMode::Autocommit).await;
        fx.run_ok(id, "begin").await;
        let manual = SessionOptions { mode: TxMode::Manual, statement_timeout_ms: 0 };
        assert_eq!(invalid_message(fx.qm.set_options(id, manual).await), MODE_IN_TX_MESSAGE);
        fx.run_ok(id, "rollback").await;
        fx.qm.set_options(id, manual).await.unwrap();
        fx.run_ok(id, "select 1").await;
        assert_eq!(fx.qm.tx_state(id).unwrap(), TxState::InTransaction, "manuel kip");
        let auto = SessionOptions::default();
        assert_eq!(invalid_message(fx.qm.set_options(id, auto).await), MODE_IN_TX_MESSAGE);
        fx.qm.tx(id, TxAction::Rollback).await.unwrap();
        fx.qm.set_options(id, auto).await.unwrap();
    }
}

/// Uyku `reap_idle(Duration::ZERO)` ile tetiklenir: `start_paused` kullanılmaz, çünkü duraklatılmış saat
/// ağ beklerken bağlantı zaman aşımlarını anında ileri sarar.
#[tokio::test]
async fn reap_idle_sleeps_and_reopens() {
    for ts in servers() {
        let fx = fixture(&ts).await;
        let id = fx.open(TxMode::Autocommit).await;
        fx.qm.set_options(id, SessionOptions { mode: TxMode::Autocommit, statement_timeout_ms: 4321 }).await.unwrap();
        let pid: i32 = fx.run_ok(id, "select pg_backend_pid()").await.first_value().parse().unwrap();

        fx.qm.reap_idle(Duration::from_secs(1800));
        assert!(!fx.qm.is_asleep(id).unwrap(), "30 dk dolmadı");
        fx.qm.reap_idle(Duration::ZERO);
        assert!(fx.qm.is_asleep(id).unwrap());
        let deadline = Instant::now() + Duration::from_secs(5);
        while fx.backend_state(pid).await.is_some() {
            assert!(Instant::now() < deadline, "uyuyan oturumun bağlantısı kapanmalı");
            tokio::time::sleep(Duration::from_millis(100)).await;
        }

        let pid2: i32 = fx.run_ok(id, "select pg_backend_pid()").await.first_value().parse().unwrap();
        assert_ne!(pid, pid2, "sessizce yeniden bağlandı");
        assert!(!fx.qm.is_asleep(id).unwrap());
        assert_eq!(fx.run_ok(id, "show statement_timeout").await.first_value(), "4321ms", "ayarlar korunur");

        // Açık transaction varsa dokunulmaz.
        fx.run_ok(id, "begin").await;
        fx.qm.reap_idle(Duration::ZERO);
        assert!(!fx.qm.is_asleep(id).unwrap());
        assert_eq!(fx.qm.tx_state(id).unwrap(), TxState::InTransaction);
        let pid3: i32 = fx.run_ok(id, "select pg_backend_pid()").await.first_value().parse().unwrap();
        assert_eq!(pid2, pid3);
        fx.qm.tx(id, TxAction::Rollback).await.unwrap();
    }
}

#[tokio::test]
async fn idle_cursor_closes_after_timeout() {
    for ts in servers() {
        let fx = fixture_with(&ts, false, None, Some(Duration::from_millis(300))).await;
        let id = fx.open(TxMode::Autocommit).await;
        let pid: i32 = fx.run_ok(id, "select pg_backend_pid()").await.first_value().parse().unwrap();

        // Hemen gelen more() çalışır ve zamanlayıcıyı yeniden kurar.
        let (r, sink) = fx.run_with(id, "select * from generate_series(1, 5000)", 100).await;
        r.unwrap();
        assert_eq!((sink.rows().len(), sink.last_more()), (100, Some(true)));
        let mut more = VecSink::default();
        fx.qm.more(id, 0, &mut more).await.unwrap();
        assert_eq!(more.rows().len(), 1000);
        assert_eq!(fx.backend_state(pid).await.as_deref(), Some("idle in transaction"));

        let deadline = Instant::now() + Duration::from_secs(5);
        while fx.backend_state(pid).await.as_deref() != Some("idle") {
            assert!(Instant::now() < deadline, "imleç kapanmalı ve transaction bitmeli");
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        let r = fx.qm.more(id, 0, &mut VecSink::default()).await;
        assert!(matches!(r, Err(Error::Invalid(_))), "{r:?}");
        assert_eq!(fx.qm.tx_state(id).unwrap(), TxState::Idle);
        fx.run_ok(id, "select 1").await;
    }
}

#[tokio::test]
async fn close_sessions_of_closes_every_session_of_the_server() {
    for ts in servers() {
        let fx = fixture(&ts).await;
        fx.setup("drop table if exists qs_close_all; create table qs_close_all(i int);").await;
        let a = fx.open(TxMode::Autocommit).await;
        let b = fx.open(TxMode::Autocommit).await;
        fx.run_ok(a, "begin; insert into qs_close_all values (1)").await;
        let qm = fx.qm.clone();
        let task = tokio::spawn(async move {
            let mut sink = VecSink::default();
            let r = qm.run(b, "select pg_sleep(30)", Scope::All, 100, false, &mut sink).await;
            (r, sink)
        });
        tokio::time::sleep(Duration::from_millis(500)).await;
        let started = Instant::now();
        fx.qm.close_sessions_of(ts.config.id).await;
        assert!(started.elapsed() < Duration::from_secs(10));
        let (r, sink) = task.await.unwrap();
        r.unwrap();
        assert_eq!(sink.failed().unwrap().1.message, CANCELLED_MESSAGE);
        for id in [a, b] {
            assert!(matches!(fx.qm.tx_state(id), Err(Error::NotFound(_))));
        }
        assert_eq!(fx.count("qs_close_all").await, 0, "açık transaction geri alındı");
        // Sunucu hâlâ bağlı: yeni oturum açılabilir.
        let c = fx.open(TxMode::Autocommit).await;
        fx.run_ok(c, "select 1").await;
    }
}

#[tokio::test]
async fn production_label_requires_confirmation_for_writes() {
    for ts in servers() {
        let fx = fixture_with(&ts, false, Some("Üretim"), None).await;
        let id = fx.open(TxMode::Autocommit).await;
        let mut sink = VecSink::default();
        let r = fx.qm.run(id, "create temp table p(i int)", Scope::All, 100, false, &mut sink).await;
        assert_eq!(invalid_message(r), "confirmation_required");
        assert!(sink.events.is_empty());
        let plan = fx.qm.analyze(id, "select 1; create temp table p(i int)", Scope::All).unwrap();
        assert_eq!(plan.iter().map(|p| p.production_write).collect::<Vec<_>>(), [false, true]);
        fx.run_ok(id, "create temp table p(i int)").await;
        let mut sink = VecSink::default();
        fx.qm.run(id, "select 1", Scope::All, 100, false, &mut sink).await.unwrap();
        sink.assert_ok();
    }
}
