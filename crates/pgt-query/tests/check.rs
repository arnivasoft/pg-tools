//! Parse tabanlı hata denetimi entegrasyon testleri (`PGT_TEST_URLS` yoksa atlanır; her sunucu için ayrı çalışır).
//! Odak: denetim hiçbir şeyi çalıştırmaz (yan etkisizlik), kullanıcı transaction'ına ve kilitlerine dokunmaz, kilit
//! beklemez ve kullanıcının çalıştırmasını bekletmez.

use std::sync::{Arc, RwLock};
use std::time::{Duration, Instant};

use pgt_core::connection::{ConnectionManager, Target};
use pgt_core::registry::Registry;
use pgt_core::testing::{TestServer, admin_client, test_servers};
use pgt_query::check::{CheckDiag, CheckSeverity, CheckTarget};
use pgt_query::events::{EventSink, QueryEvent};
use pgt_query::session::{QueryManager, Scope, SessionId, SessionOptions, TxAction, TxMode, TxState};
use tempfile::TempDir;

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
    let fx = Fx { qm: Arc::new(qm), ts: ts.clone(), _dir: dir };
    fx.setup(FIXTURE).await;
    if fx.version().await >= 110000 {
        fx.setup(
            "create procedure qc_chk.p() language plpgsql as $$ begin insert into qc_chk.t values (100); end $$",
        )
        .await;
    }
    fx
}

const FIXTURE: &str = r#"
    drop schema if exists qc_chk cascade;
    create schema qc_chk;
    create table qc_chk.t(i int);
    insert into qc_chk.t values (1), (2), (3);
    create sequence qc_chk.s;
    create table qc_chk.locked(k int);
    create function qc_chk.f() returns int language plpgsql as $$ begin insert into qc_chk.t values (42); return 1; end $$;
"#;

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

    async fn cell(&self, id: SessionId, sql: &str) -> String {
        let events = self.run(id, sql).await;
        events
            .iter()
            .find_map(|e| match e {
                QueryEvent::Rows { rows, .. } => rows.first().and_then(|r| r.first().cloned().flatten()),
                _ => None,
            })
            .unwrap_or_else(|| panic!("satır yok: {events:?}"))
    }

    async fn setup(&self, sql: &str) {
        admin_client(&self.ts, &self.ts.config.maintenance_db).await.batch_execute(sql).await.unwrap();
    }

    async fn admin_one(&self, sql: &str) -> String {
        let c = admin_client(&self.ts, &self.ts.config.maintenance_db).await;
        c.query_one(&format!("select ({sql})::text"), &[]).await.unwrap().get(0)
    }

    async fn version(&self) -> i32 {
        self.admin_one("current_setting('server_version_num')").await.parse().unwrap()
    }

    /// Betiğin tamamı görünür, imleç başta.
    async fn check(&self, id: SessionId, sql: &str) -> Vec<CheckDiag> {
        self.qm.check(id, sql, all(sql)).await.unwrap()
    }

    /// Kalıcı durum özeti: ilişkiler (tablo/dizin/dizi), satır sayısı ve toplamı (UPDATE/DELETE/INSERT), `t`'nin kolon
    /// sayısı (ALTER), dizi durumu (nextval), VACUUM sayacı.
    async fn snapshot(&self) -> String {
        self.admin_one(
            "(select string_agg(relname, ',' order by relname) from pg_class c join pg_namespace n on n.oid = c.relnamespace \
               where n.nspname = 'qc_chk') || ' rows=' || (select count(*) || '/' || sum(i) from qc_chk.t) \
             || ' cols=' || (select count(*) from pg_attribute where attrelid = 'qc_chk.t'::regclass and attnum > 0) \
             || ' seq=' || (select last_value || '/' || is_called from qc_chk.s) \
             || ' vac=' || (select vacuum_count from pg_stat_user_tables where relid = 'qc_chk.t'::regclass)",
        )
        .await
    }
}

fn all(sql: &str) -> CheckTarget {
    CheckTarget { cursor: 0, visible_start: 0, visible_end: sql.chars().count() as u32 }
}

/// Denetlenen ve çalıştırılmaması gereken ifadeler (Review Focus 1). Sondaki nöbetçi ifade (17) hata verir: Parse
/// zaman aşımında/iptalde kalanları atladığından teşhisinin gelmesi önceki her ifadenin Parse edildiğini kanıtlar.
const SIDE_EFFECTS: &str = "create table qc_chk.newt(i int);
drop table qc_chk.t;
truncate qc_chk.t;
create index on qc_chk.t(i);
vacuum qc_chk.t;
set lock_timeout = '9s';
set search_path = nowhere;
do $$ begin insert into qc_chk.t values (99); end $$;
call qc_chk.p();
select nextval('qc_chk.s');
select qc_chk.f();
insert into qc_chk.t values (5);
update qc_chk.t set i = 0;
delete from qc_chk.t;
alter table qc_chk.t add column j int;
create temp table tmp_chk(i int);
select pg_advisory_lock(4242);
select pgt_sentinel_nope";

#[tokio::test]
async fn check_executes_nothing() {
    let _serial = SERIAL.lock().await;
    for ts in servers() {
        let fx = fixture(&ts).await;
        let v = fx.version().await;
        tokio::time::sleep(Duration::from_millis(1200)).await; // istatistikler otursun
        let before = fx.snapshot().await;
        // Autocommit geçişi asıl kanıttır: çalıştırılan bir ifade orada kalıcı olurdu. Manuel geçişte denetim
        // savepoint'te yapılır (ROLLBACK TO yazmaları gizlerdi); orada transaction/oturum güvenliği sınanır.
        for mode in [TxMode::Autocommit, TxMode::Manual] {
            let id = fx.open(mode).await;
            let lock_timeout = fx.cell(id, "show lock_timeout").await;
            let search_path = fx.cell(id, "show search_path").await;
            if mode == TxMode::Manual {
                assert_eq!(fx.qm.tx_state(id).unwrap(), TxState::InTransaction);
            }
            let diags = fx.check(id, SIDE_EFFECTS).await;
            // PG 10'da CALL yok: sözdizimi hatası. Nöbetçi (17) her sürümde.
            let expected: Vec<(u32, &str)> =
                if v < 110000 { vec![(8, "42601"), (17, "42703")] } else { vec![(17, "42703")] };
            let got: Vec<(u32, &str)> =
                diags.iter().map(|d| (d.stmt_index, d.sqlstate.as_deref().unwrap_or(""))).collect();
            assert_eq!(got, expected, "{diags:?}");
            assert_eq!(fx.cell(id, "show lock_timeout").await, lock_timeout);
            assert_eq!(fx.cell(id, "show search_path").await, search_path);
            let tmp = fx.cell(id, "select count(*) from pg_class where relname = 'tmp_chk'").await;
            assert_eq!(tmp, "0");
            let adv = fx.cell(id, "select count(*) from pg_locks where locktype = 'advisory' and pid = pg_backend_pid()").await;
            assert_eq!(adv, "0");
            if mode == TxMode::Manual {
                assert_eq!(fx.qm.tx_state(id).unwrap(), TxState::InTransaction);
                fx.qm.tx(id, TxAction::Rollback).await.unwrap();
            }
        }
        tokio::time::sleep(Duration::from_millis(1200)).await;
        assert_eq!(fx.snapshot().await, before);
    }
}

#[tokio::test]
async fn check_in_transaction_keeps_state_and_locks_and_skips_failed() {
    let _serial = SERIAL.lock().await;
    for ts in servers() {
        let fx = fixture(&ts).await;
        let id = fx.open(TxMode::Manual).await;
        let pid = fx.cell(id, "select pg_backend_pid()").await;
        fx.run(id, "select * from qc_chk.locked").await;
        let locks_sql = format!(
            "select coalesce(string_agg(locktype || ':' || coalesce(relation::regclass::text, '') || ':' || mode, ',' \
               order by locktype, relation::regclass::text, mode), '') from pg_locks where pid = {pid} \
               and locktype <> 'virtualxid'"
        );
        let before = fx.admin_one(&locks_sql).await;
        assert!(before.contains("qc_chk.locked"), "{before}");
        let diags = fx
            .check(id, "select * from qc_chk.t; insert into qc_chk.t values (1); select nope from qc_chk.t; vacuum qc_chk.t")
            .await;
        let got: Vec<(u32, &str)> = diags.iter().map(|d| (d.stmt_index, d.sqlstate.as_deref().unwrap())).collect();
        assert_eq!(got, [(2, "42703")]);
        assert_eq!(fx.qm.tx_state(id).unwrap(), TxState::InTransaction);
        assert_eq!(fx.admin_one(&locks_sql).await, before, "Parse kilitleri kalmamalı");
        assert_eq!(fx.cell(id, "select count(*) from qc_chk.t").await, "3", "transaction sağlam");

        // Failed: denetim yok.
        fx.run(id, "select 1/0").await;
        assert_eq!(fx.qm.tx_state(id).unwrap(), TxState::Failed);
        assert!(fx.check(id, "select nope").await.is_empty());
        assert_eq!(fx.qm.tx_state(id).unwrap(), TxState::Failed);
        fx.qm.tx(id, TxAction::Rollback).await.unwrap();
        assert_eq!(fx.check(id, "select nope").await.len(), 1);
    }
}

#[tokio::test]
async fn check_gives_up_on_locked_tables_quickly() {
    let _serial = SERIAL.lock().await;
    for ts in servers() {
        let fx = fixture(&ts).await;
        let id = fx.open(TxMode::Autocommit).await;
        let mut locker = admin_client(&fx.ts, &fx.ts.config.maintenance_db).await;
        let tx = locker.transaction().await.unwrap();
        tx.batch_execute("lock table qc_chk.locked in access exclusive mode").await.unwrap();
        fx.check(id, "select 1").await; // bağlantıyı ısıt
        let started = Instant::now();
        let diags = fx.check(id, "select * from qc_chk.locked").await;
        let took = started.elapsed();
        assert!(diags.is_empty(), "{diags:?}");
        // 200 ms kilit beklemesi + SHOW/SET/geri yükleme gidiş-dönüşleri; matriste paralel kapsayıcılar için pay.
        assert!(took <= Duration::from_millis(500), "{took:?}");
        tx.rollback().await.unwrap();
        assert_eq!(fx.cell(id, "show lock_timeout").await, "0");
    }
}

#[tokio::test]
async fn run_started_during_a_check_is_not_busy() {
    let _serial = SERIAL.lock().await;
    for ts in servers() {
        let fx = fixture(&ts).await;
        let id = fx.open(TxMode::Autocommit).await;
        let mut locker = admin_client(&fx.ts, &fx.ts.config.maintenance_db).await;
        let tx = locker.transaction().await.unwrap();
        tx.batch_execute("lock table qc_chk.locked in access exclusive mode").await.unwrap();
        // Her ifade 200 ms kilit bekler: denetim ~1 sn sürer.
        let script = vec!["select * from qc_chk.locked"; 20].join(";\n");
        let qm = fx.qm.clone();
        let target = all(&script);
        let checker = tokio::spawn(async move { qm.check(id, &script, target).await });
        tokio::time::sleep(Duration::from_millis(150)).await;
        let started = Instant::now();
        let mut sink = VecSink::default();
        fx.qm.run(id, "select 7", Scope::All, 10, true, &mut sink).await.expect("busy dönmemeli");
        let took = started.elapsed();
        assert!(took <= Duration::from_millis(1200), "{took:?}");
        assert!(sink.0.iter().any(|e| matches!(e, QueryEvent::Rows { .. })), "{:?}", sink.0);
        let diags = checker.await.unwrap().unwrap();
        assert!(diags.is_empty(), "{diags:?}");
        tx.rollback().await.unwrap();
        assert_eq!(fx.cell(id, "show lock_timeout").await, "0");
    }
}

#[tokio::test]
async fn check_returns_nothing_while_busy() {
    let _serial = SERIAL.lock().await;
    for ts in servers() {
        let fx = fixture(&ts).await;
        let id = fx.open(TxMode::Autocommit).await;
        let qm = fx.qm.clone();
        let task = tokio::spawn(async move {
            let mut sink = VecSink::default();
            qm.run(id, "select pg_sleep(30)", Scope::All, 10, true, &mut sink).await
        });
        tokio::time::sleep(Duration::from_millis(400)).await;
        let started = Instant::now();
        assert!(fx.check(id, "select nope").await.is_empty());
        assert!(started.elapsed() < Duration::from_millis(100));
        fx.qm.cancel(id);
        task.await.unwrap().unwrap();
    }
}

#[tokio::test]
async fn positions_are_scalar_offsets_and_ddl_context_lowers_severity() {
    let _serial = SERIAL.lock().await;
    for ts in servers() {
        let fx = fixture(&ts).await;
        let id = fx.open(TxMode::Autocommit).await;
        let script = "select '😀' as e;\nselect 'ğ😀', 1 frm xyz;";
        let diags = fx.check(id, script).await;
        let [d] = diags.as_slice() else { panic!("{diags:?}") };
        assert_eq!((d.stmt_index, d.severity, d.sqlstate.as_deref()), (1, CheckSeverity::Error, Some("42601")));
        let chars: Vec<char> = script.chars().collect();
        let at = (d.char_offset + d.position.unwrap() - 1) as usize;
        assert_eq!(chars[at..at + 3].iter().collect::<String>(), "xyz", "{d:?}");

        // Önceki DDL'e bağlı isim hataları bilgi; DDL yokken hata; $1 türü bastırılır.
        let diags = fx
            .check(id, "create table qc_chk.later(i int);\nselect * from qc_chk.later;\nselect zz from qc_chk.t")
            .await;
        let got: Vec<(u32, CheckSeverity, &str)> =
            diags.iter().map(|d| (d.stmt_index, d.severity, d.sqlstate.as_deref().unwrap())).collect();
        assert_eq!(got, [(1, CheckSeverity::Info, "42P01"), (2, CheckSeverity::Info, "42703")]);
        let got: Vec<CheckSeverity> = fx.check(id, "select * from qc_chk.later").await.iter().map(|d| d.severity).collect();
        assert_eq!(got, [CheckSeverity::Error]);
        assert!(fx.check(id, "select $1").await.is_empty());
        assert_eq!(fx.cell(id, "select count(*) from pg_class where relname = 'later'").await, "0");

        // Yalnızca imleç (boş görünür alan imleçte).
        let script = "select 1;\nselect nope;\nselect 2";
        let only = |c: u32| CheckTarget { cursor: c, visible_start: c, visible_end: c };
        let d = fx.qm.check(id, script, only(12)).await.unwrap();
        assert_eq!(d.iter().map(|d| (d.stmt_index, d.char_offset)).collect::<Vec<_>>(), [(1, 10)]);
        assert!(fx.qm.check(id, script, only(2)).await.unwrap().is_empty());
        // "end of input": konum ifade sonunu aşmaz.
        let d = fx.check(id, "select 1 +").await;
        assert_eq!(d.iter().map(|d| d.position).collect::<Vec<_>>(), [Some(10)]);
    }
}

/// Uzun betik: görünür alan betiğin sonunda ve çok satırlı ifadeleri ortadan kesiyor. Kesilen ifadeler bütün
/// denetlenir (sahte 42601 yok), `stmt_index` betik genelindedir, görünür alan dışındaki hata raporlanmaz ve imleç
/// ifadesi her zaman önce denetlenir.
#[tokio::test]
async fn visible_window_of_a_long_script() {
    let _serial = SERIAL.lock().await;
    for ts in servers() {
        let fx = fixture(&ts).await;
        let id = fx.open(TxMode::Autocommit).await;
        let mut script = String::from("select nope_at_top;\n");
        for i in 1..120 {
            script.push_str(&format!("select {i} as a,\n  {i} as b,\n  {i} as c\nfrom qc_chk.t;\n"));
        }
        script.push_str("select nope_at_bottom from qc_chk.t;\n");
        let chars = |s: &str| s.chars().count() as u32;
        let n = chars(&script);
        // Görünür alan: son ~8 ifade, ilk satırı bir ifadenin ortasında ("  " ile başlayan satır).
        let cut = script.match_indices("  110 as b").next().unwrap().0;
        let target = CheckTarget { cursor: n, visible_start: chars(&script[..cut]), visible_end: n };
        let diags = fx.qm.check(id, &script, target).await.unwrap();
        let got: Vec<(u32, &str)> = diags.iter().map(|d| (d.stmt_index, d.sqlstate.as_deref().unwrap())).collect();
        assert_eq!(got, [(120, "42703")], "kesilen ifade için sahte 42601 olmamalı: {diags:?}");
        assert_eq!(diags[0].char_offset + diags[0].position.unwrap() - 1, chars(&script[..script.find("nope_at_bottom").unwrap()]));

        // İmleç betiğin başında, görünür alan sonda: imleç ifadesi de denetlenir.
        let target = CheckTarget { cursor: 3, ..target };
        let got: Vec<u32> = fx.qm.check(id, &script, target).await.unwrap().iter().map(|d| d.stmt_index).collect();
        assert_eq!(got, [0, 120]);

        // Görünür alan betiğin ortasında (sınır 20 ifade): alt ve üstteki hatalar denetlenmez.
        let mid = script.match_indices("select 50 as a").next().unwrap().0;
        let target = CheckTarget { cursor: chars(&script[..mid]), visible_start: chars(&script[..mid]), visible_end: n };
        assert!(fx.qm.check(id, &script, target).await.unwrap().is_empty(), "20 ifade sınırı");
        assert!(fx.qm.check(id, "select 1", CheckTarget { cursor: 0, visible_start: 5, visible_end: 1 }).await.is_err());
    }
}
