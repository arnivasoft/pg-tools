//! Çalıştırıcı entegrasyon testleri (`PGT_TEST_URLS` yoksa atlanır; her sunucu için ayrı çalışır).

use std::time::{Duration, Instant};

use pgt_core::connection::{ConnectionManager, SessionProfile, Target};
use pgt_core::testing::{TestServer, admin_client, test_servers};
use pgt_query::events::{EventSink, QueryEvent};
use pgt_query::exec::{ExecConfig, Executor, Path, StmtOutcome};
use pgt_query::split::split;

const QUERY_RW: SessionProfile = SessionProfile::Query { read_only: false, statement_timeout_ms: 0 };
const AUTO: ExecConfig = ExecConfig { limit: 1000, chunk: 1000, autocommit: true };

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

    fn last_more(&self) -> Option<bool> {
        self.events.iter().rev().find_map(|e| match e {
            QueryEvent::Rows { more, .. } => Some(*more),
            _ => None,
        })
    }

    fn done(&self) -> (String, Option<u64>, u64, bool, Vec<pgt_query::events::NoticeInfo>) {
        match self.events.last() {
            Some(QueryEvent::Done { tag, rows_affected, rows_sent, truncated, notices, .. }) => {
                (tag.clone(), *rows_affected, *rows_sent, *truncated, notices.clone())
            }
            other => panic!("Done bekleniyordu: {other:?}"),
        }
    }

    fn failed(&self) -> (pgt_core::ErrorPayload, Option<u32>, u32) {
        match self.events.last() {
            Some(QueryEvent::Failed { error, position, char_offset, .. }) => (error.clone(), *position, *char_offset),
            other => panic!("Failed bekleniyordu: {other:?}"),
        }
    }
}

struct Fixture {
    _mgr: ConnectionManager,
    exec: Executor,
}

async fn connect(ts: &TestServer) -> Fixture {
    let mgr = ConnectionManager::new();
    mgr.connect(ts.config.clone(), ts.password.clone()).await.unwrap();
    let conn = mgr.open_dedicated(ts.config.id, Target::Maintenance, QUERY_RW).await.unwrap();
    Fixture { _mgr: mgr, exec: Executor::new(conn) }
}

async fn run(exec: &mut Executor, sql: &str, cfg: &ExecConfig) -> (StmtOutcome, VecSink) {
    let mut sink = VecSink::default();
    let outcome = exec.run_statement(0, sql, 0, cfg, &mut sink).await;
    (outcome, sink)
}

async fn run_ok(exec: &mut Executor, sql: &str, cfg: &ExecConfig) -> VecSink {
    let (outcome, sink) = run(exec, sql, cfg).await;
    assert_eq!(outcome, StmtOutcome::Completed, "{:?}", sink.events);
    sink
}

fn s(v: &str) -> Option<String> {
    Some(v.to_string())
}

/// Çalıştırıcının oturumunun sunucudaki durumu (yönetici bağlantısından).
async fn backend_state(ts: &TestServer, pid: &str) -> String {
    let admin = admin_client(ts, &ts.config.maintenance_db).await;
    let pid: i32 = pid.parse().unwrap();
    admin.query_one("SELECT state FROM pg_stat_activity WHERE pid = $1", &[&pid]).await.unwrap().get(0)
}

async fn backend_pid(exec: &mut Executor) -> String {
    run_ok(exec, "select pg_backend_pid()", &AUTO).await.rows()[0][0].clone().unwrap()
}

#[tokio::test]
async fn select_streams_columns_then_rows_then_done() {
    for ts in servers() {
        let mut f = connect(&ts).await;
        let sink =
            run_ok(&mut f.exec, "select i, 'x'::text t, null::int n from generate_series(1,3) i", &AUTO).await;
        let QueryEvent::Started { columns, .. } = &sink.events[0] else { panic!("{:?}", sink.events) };
        let cols: Vec<(&str, &str, u32)> =
            columns.iter().map(|c| (c.name.as_str(), c.type_name.as_str(), c.type_oid)).collect();
        assert_eq!(cols, [("i", "int4", 23), ("t", "text", 25), ("n", "int4", 23)]);
        assert_eq!(
            sink.rows(),
            (1..=3).map(|i| vec![s(&i.to_string()), s("x"), None]).collect::<Vec<_>>()
        );
        assert_eq!(sink.last_more(), Some(false));
        let (tag, _, rows_sent, truncated, _) = sink.done();
        assert_eq!((tag.as_str(), rows_sent, truncated), ("SELECT", 3, false));
        assert_eq!(f.exec.last_path(), Some(Path::Cursor));

        let empty = run_ok(&mut f.exec, "select '' as e, null::text as n", &AUTO).await;
        assert_eq!(empty.rows(), vec![vec![s(""), None]], "boş dize NULL değildir");

        // `split` ifadesi sondaki yorumu içerebilir; metin olduğu gibi gönderilir.
        let commented = run_ok(&mut f.exec, "select 2 -- son yorum", &AUTO).await;
        assert_eq!((commented.rows(), f.exec.last_path()), (vec![vec![s("2")]], Some(Path::Cursor)));
        let commented = run_ok(&mut f.exec, "show server_version /* yorum */ -- son", &AUTO).await;
        assert_eq!((commented.rows().len(), f.exec.last_path()), (1, Some(Path::Direct)));
    }
}

#[tokio::test]
async fn row_limit_bounds_memory() {
    for ts in servers() {
        let mut f = connect(&ts).await;
        let cfg = ExecConfig { limit: 2500, chunk: 1000, autocommit: true };
        let sink = run_ok(&mut f.exec, "select * from generate_series(1,100000)", &cfg).await;
        assert_eq!(sink.rows().len(), 2500);
        assert_eq!(sink.last_more(), Some(true));
        let (_, _, rows_sent, truncated, _) = sink.done();
        assert_eq!((rows_sent, truncated), (2500, true));
        assert!(f.exec.has_open_cursor());

        let mut more = VecSink::default();
        f.exec.more(0, &mut more).await.unwrap();
        let rows = more.rows();
        assert_eq!(rows.len(), 1000);
        assert_eq!(rows[0][0], s("2501"));
        assert_eq!(more.last_more(), Some(true));
        assert!(f.exec.peak_buffered_rows() <= 1000, "{}", f.exec.peak_buffered_rows());
        f.exec.close_cursor().await.unwrap();
        assert!(!f.exec.has_open_cursor());
    }
}

#[tokio::test]
async fn more_exhausts_and_closes_cursor() {
    for ts in servers() {
        let mut f = connect(&ts).await;
        let cfg = ExecConfig { limit: 10, chunk: 10, autocommit: true };
        let sink = run_ok(&mut f.exec, "select * from generate_series(1,15)", &cfg).await;
        assert_eq!((sink.rows().len(), sink.last_more()), (10, Some(true)));
        let mut more = VecSink::default();
        f.exec.more(0, &mut more).await.unwrap();
        assert_eq!((more.rows().len(), more.last_more()), (5, Some(false)));
        assert!(!f.exec.has_open_cursor());
        assert!(f.exec.more(0, &mut more).await.is_err(), "açık imleç yok");
        let pid = backend_pid(&mut f.exec).await;
        assert_eq!(backend_state(&ts, &pid).await, "idle");
    }
}

#[tokio::test]
async fn insert_returning_uses_direct_path() {
    for ts in servers() {
        let mut f = connect(&ts).await;
        run_ok(&mut f.exec, "create temp table r(id int)", &AUTO).await;
        let cfg = ExecConfig { limit: 2, chunk: 1000, autocommit: true };
        let sink = run_ok(&mut f.exec, "insert into r select generate_series(1,5) returning id", &cfg).await;
        assert_eq!(f.exec.last_path(), Some(Path::Direct));
        assert_eq!(sink.rows(), vec![vec![s("1")], vec![s("2")]]);
        assert_eq!(sink.last_more(), Some(false));
        let (tag, affected, rows_sent, truncated, _) = sink.done();
        assert_eq!((tag.as_str(), affected, rows_sent, truncated), ("INSERT", Some(5), 2, true));
        let all = run_ok(&mut f.exec, "select count(*) from r", &AUTO).await;
        assert_eq!(all.rows(), vec![vec![s("5")]], "tüm satırlar eklenmiş olmalı");
    }
}

#[tokio::test]
async fn explain_and_show_work() {
    for ts in servers() {
        let mut f = connect(&ts).await;
        let explain = run_ok(&mut f.exec, "explain select 1", &AUTO).await;
        assert!(!explain.rows().is_empty());
        assert_eq!(f.exec.last_path(), Some(Path::Direct));
        let show = run_ok(&mut f.exec, "show server_version", &AUTO).await;
        let version = show.rows()[0][0].clone().unwrap();
        assert!(version.starts_with(&f.exec.server_info().version), "{version}");
        assert_eq!(show.done().0, "SHOW");
        let values = run_ok(&mut f.exec, "values (1, 'a'), (2, null)", &AUTO).await;
        assert_eq!(values.rows(), vec![vec![s("1"), s("a")], vec![s("2"), None]]);
    }
}

#[tokio::test]
async fn non_row_statement_reports_tag_and_count() {
    for ts in servers() {
        let mut f = connect(&ts).await;
        let cases = [
            ("create temp table w(i int)", "CREATE TABLE", None),
            ("insert into w values (1),(2)", "INSERT", Some(2)),
            ("update w set i = i + 1", "UPDATE", Some(2)),
            ("delete from w where i > 0", "DELETE", Some(2)),
        ];
        for (sql, want_tag, want_affected) in cases {
            let sink = run_ok(&mut f.exec, sql, &AUTO).await;
            assert_eq!(f.exec.last_path(), Some(Path::NoRows));
            let QueryEvent::Started { columns, .. } = &sink.events[0] else { panic!("{:?}", sink.events) };
            assert!(columns.is_empty());
            let (tag, affected, rows_sent, _, _) = sink.done();
            assert_eq!((tag.as_str(), affected, rows_sent), (want_tag, want_affected, 0), "{sql}");
        }
    }
}

#[tokio::test]
async fn text_formats_cover_exotic_types() {
    for ts in servers() {
        let mut f = connect(&ts).await;
        run_ok(&mut f.exec, "set timezone = 'UTC'", &AUTO).await;
        let sink = run_ok(
            &mut f.exec,
            r#"select '{1,2}'::int[], '[1,5)'::int4range, '{"a":1}'::jsonb, '\xDEAD'::bytea, 1.50::numeric, '2026-01-02 03:04:05+00'::timestamptz, 'a b'::name"#,
            &AUTO,
        )
        .await;
        assert_eq!(
            sink.rows(),
            vec![vec![
                s("{1,2}"),
                s("[1,5)"),
                s(r#"{"a": 1}"#),
                s(r"\xdead"),
                s("1.50"),
                s("2026-01-02 03:04:05+00"),
                s("a b")
            ]]
        );
    }
}

#[tokio::test]
async fn notices_are_collected() {
    for ts in servers() {
        let mut f = connect(&ts).await;
        let sink = run_ok(&mut f.exec, "do $$ begin raise notice 'merhaba'; raise warning 'uyarı'; end $$", &AUTO).await;
        let (tag, _, _, _, notices) = sink.done();
        assert_eq!(tag, "DO");
        let got: Vec<(&str, &str)> = notices.iter().map(|n| (n.severity.as_str(), n.message.as_str())).collect();
        assert_eq!(got, [("NOTICE", "merhaba"), ("WARNING", "uyarı")]);
        // Sonraki ifadeye taşınmaz.
        let next = run_ok(&mut f.exec, "select 1", &AUTO).await;
        assert!(next.done().4.is_empty());
    }
}

#[tokio::test]
async fn notices_are_capped_and_clipped() {
    for ts in servers() {
        let mut f = connect(&ts).await;
        let sink = run_ok(
            &mut f.exec,
            "do $$ begin for i in 1..150 loop raise notice '%', repeat('ş', 3000); end loop; end $$",
            &AUTO,
        )
        .await;
        let notices = sink.done().4;
        assert_eq!(notices.len(), 100);
        assert!(notices.iter().all(|n| n.message.chars().count() == 2000));
    }
}

#[tokio::test]
async fn error_has_position_and_stops_script() {
    for ts in servers() {
        let mut f = connect(&ts).await;
        let (outcome, sink) = run(&mut f.exec, "select 1 + 'x'::text", &AUTO).await;
        assert_eq!(outcome, StmtOutcome::Failed);
        let (error, position, _) = sink.failed();
        assert_eq!(error.code, "query");
        assert_eq!(position, Some(10), "{error:?}");
        assert!(sink.events.iter().all(|e| !matches!(e, QueryEvent::Started { .. })), "{:?}", sink.events);

        let (outcome, sink) = run(&mut f.exec, "selectt 1", &AUTO).await;
        assert_eq!(outcome, StmtOutcome::Failed);
        let (error, position, _) = sink.failed();
        assert_eq!((error.code.as_str(), position), ("query", Some(1)));
        assert!(error.message.contains("selectt"), "{error:?}");

        // Çalışma zamanı hatası imleç yolunda: imlecin transaction'ı geri alınır.
        let (outcome, sink) = run(&mut f.exec, "select 1 / (i - 2) from generate_series(1, 3) i", &AUTO).await;
        assert_eq!(outcome, StmtOutcome::Failed);
        assert_eq!(sink.failed().1, None);
        assert!(!f.exec.has_open_cursor());

        let ok = run_ok(&mut f.exec, "select 1", &AUTO).await;
        assert_eq!(ok.rows(), vec![vec![s("1")]]);
        let pid = backend_pid(&mut f.exec).await;
        assert_eq!(backend_state(&ts, &pid).await, "idle");
    }
}

#[tokio::test]
async fn error_position_is_relative_to_statement_chars() {
    for ts in servers() {
        let mut f = connect(&ts).await;
        let script = "select 'ş';\n-- yorum ✓\nselect 'çğü💥' || x";
        let stmts = split(script);
        assert_eq!(stmts.len(), 2);
        let st = stmts[1];
        let sql = &script[st.start..st.end];
        let char_offset = script[..st.start].chars().count() as u32;
        let mut sink = VecSink::default();
        let outcome = f.exec.run_statement(1, sql, char_offset, &AUTO, &mut sink).await;
        assert_eq!(outcome, StmtOutcome::Failed);
        let (error, position, got_offset) = sink.failed();
        assert_eq!(got_offset, char_offset);
        let position = position.unwrap_or_else(|| panic!("konum yok: {error:?}"));
        let at = script.chars().nth((char_offset + position - 1) as usize);
        assert_eq!(at, Some('x'), "{error:?}");
    }
}

#[tokio::test]
async fn cancel_stops_a_long_query() {
    for ts in servers() {
        let mut f = connect(&ts).await;
        let cancel = f.exec.cancel_handle();
        let started = Instant::now();
        let mut sink = VecSink::default();
        let run = f.exec.run_statement(0, "select pg_sleep(30)", 0, &AUTO, &mut sink);
        let canceller = async {
            tokio::time::sleep(Duration::from_millis(300)).await;
            cancel.cancel().await
        };
        let (outcome, cancelled) = tokio::join!(run, canceller);
        cancelled.unwrap();
        assert_eq!(outcome, StmtOutcome::Failed);
        assert!(started.elapsed() < Duration::from_secs(2), "{:?}", started.elapsed());
        let (error, _, _) = sink.failed();
        assert_eq!(error.detail.as_deref().map(|d| d.contains("cancel")), Some(true), "{error:?}");
        assert!(!f.exec.has_open_cursor());
        let ok = run_ok(&mut f.exec, "select 1", &AUTO).await;
        assert_eq!(ok.rows(), vec![vec![s("1")]]);
        let pid = backend_pid(&mut f.exec).await;
        assert_eq!(backend_state(&ts, &pid).await, "idle");
    }
}

#[tokio::test]
async fn cursor_is_closed_before_next_statement() {
    for ts in servers() {
        let mut f = connect(&ts).await;
        let pid = backend_pid(&mut f.exec).await;
        let cfg = ExecConfig { limit: 10, chunk: 1000, autocommit: true };
        let sink = run_ok(&mut f.exec, "select * from generate_series(1,1000)", &cfg).await;
        assert_eq!(sink.last_more(), Some(true));
        assert_eq!(backend_state(&ts, &pid).await, "idle in transaction", "imleç açıkken transaction açık");
        let next = run_ok(&mut f.exec, "select 1", &cfg).await;
        assert_eq!(next.rows(), vec![vec![s("1")]]);
        assert!(!f.exec.has_open_cursor());
        assert_eq!(backend_state(&ts, &pid).await, "idle");
    }
}

#[tokio::test]
async fn manual_mode_cursor_does_not_end_user_transaction() {
    for ts in servers() {
        let mut f = connect(&ts).await;
        let manual = ExecConfig { limit: 10, chunk: 1000, autocommit: false };
        let pid = backend_pid(&mut f.exec).await;
        // Transaction dışında manuel kipte imleç açılamaz (BEGIN eklenmez): doğrudan yol.
        let direct = run_ok(&mut f.exec, "select * from generate_series(1,50)", &manual).await;
        assert_eq!(f.exec.last_path(), Some(Path::Direct));
        assert_eq!((direct.rows().len(), direct.done().3), (10, true));
        assert_eq!(backend_state(&ts, &pid).await, "idle");

        run_ok(&mut f.exec, "begin", &manual).await;
        assert!(f.exec.in_transaction());
        run_ok(&mut f.exec, "create temp table m(i int)", &manual).await;
        let sink = run_ok(&mut f.exec, "select * from generate_series(1,50)", &manual).await;
        assert_eq!(f.exec.last_path(), Some(Path::Cursor));
        assert_eq!(sink.last_more(), Some(true));
        run_ok(&mut f.exec, "select 1", &manual).await;
        assert_eq!(backend_state(&ts, &pid).await, "idle in transaction", "kullanıcının transaction'ı sürer");
        run_ok(&mut f.exec, "rollback", &manual).await;
        assert!(!f.exec.in_transaction());
        assert_eq!(backend_state(&ts, &pid).await, "idle");
        let gone =
            run_ok(&mut f.exec, "select count(*) from pg_class where relname = 'm' and relpersistence = 't'", &manual)
                .await;
        assert_eq!(gone.rows(), vec![vec![s("0")]], "çalıştırıcı kullanıcının transaction'ını onaylamamalı");
    }
}

#[tokio::test]
async fn autocommit_cursor_inside_user_transaction_keeps_it_open() {
    for ts in servers() {
        let mut f = connect(&ts).await;
        let cfg = ExecConfig { limit: 5, chunk: 1000, autocommit: true };
        run_ok(&mut f.exec, "begin", &cfg).await;
        run_ok(&mut f.exec, "create temp table k(i int)", &cfg).await;
        let sink = run_ok(&mut f.exec, "select * from generate_series(1,20)", &cfg).await;
        assert_eq!(sink.last_more(), Some(true));
        let all = run_ok(&mut f.exec, "select * from generate_series(1,3)", &cfg).await;
        assert_eq!(all.rows().len(), 3);
        run_ok(&mut f.exec, "rollback", &cfg).await;
        let gone =
            run_ok(&mut f.exec, "select count(*) from pg_class where relname = 'k' and relpersistence = 't'", &cfg).await;
        assert_eq!(gone.rows(), vec![vec![s("0")]], "imleç kapanışı kullanıcının transaction'ını onaylamamalı");
    }
}

#[tokio::test]
async fn statement_timeout_error_is_friendly() {
    for ts in servers() {
        let mut f = connect(&ts).await;
        run_ok(&mut f.exec, "set statement_timeout = 100", &AUTO).await;
        let (outcome, sink) = run(&mut f.exec, "select pg_sleep(1)", &AUTO).await;
        assert_eq!(outcome, StmtOutcome::Failed);
        let (error, _, _) = sink.failed();
        assert!(error.message.starts_with("Sorgu zaman aşımına uğradı"), "{error:?}");
        run_ok(&mut f.exec, "reset statement_timeout", &AUTO).await;
        run_ok(&mut f.exec, "select 1", &AUTO).await;
    }
}

#[tokio::test]
async fn sink_disconnect_stops_execution() {
    for ts in servers() {
        let mut f = connect(&ts).await;
        let pid = backend_pid(&mut f.exec).await;
        let cfg = ExecConfig { limit: 100_000, chunk: 10, autocommit: true };
        // Started + bir Rows kabul edilir, sonra alıcı gider.
        let mut sink = VecSink { accept: Some(2), ..Default::default() };
        let outcome = f.exec.run_statement(0, "select * from generate_series(1,100000)", 0, &cfg, &mut sink).await;
        assert_eq!(outcome, StmtOutcome::Failed);
        assert_eq!(sink.events.len(), 2);
        assert!(!f.exec.has_open_cursor());
        assert_eq!(backend_state(&ts, &pid).await, "idle");
    }
}

#[tokio::test]
async fn invalid_limit_and_copy_stdin_fail_without_running() {
    for ts in servers() {
        let mut f = connect(&ts).await;
        let zero = ExecConfig { limit: 0, chunk: 10, autocommit: true };
        let (outcome, sink) = run(&mut f.exec, "select 1", &zero).await;
        assert_eq!(outcome, StmtOutcome::Failed);
        assert_eq!(sink.failed().0.code, "invalid");
        let (outcome, sink) = run(&mut f.exec, "copy pg_class from stdin", &AUTO).await;
        assert_eq!(outcome, StmtOutcome::Failed);
        assert_eq!(sink.failed().0.code, "invalid");
        run_ok(&mut f.exec, "select 1", &AUTO).await;
    }
}
