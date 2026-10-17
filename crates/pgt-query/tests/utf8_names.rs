//! Adı geçersiz UTF-8 olan multirange tipi (PG 14+; `pgt_core::testing::INVALID_UTF8_TYPE_NAME_SQL`) bulunan
//! veritabanında Query Tool (sonuç sütunu tipi, değer) ve tamamlama (Docker; `PGT_TEST_URLS` yoksa ya da PG < 14 ise
//! atlanır). Beklenen: hiçbir yol çökmez. tokio-postgres `prepare` böyle bir sonuç tipini çözemediği için ifade tip
//! bilgisi olmadan metin protokolüyle çalışır (sütun tipi `?`); tamamlamada tip adı kayıplı (U+FFFD); geçersiz UTF-8
//! değer U+FFFD gösterilir.

use std::sync::{Arc, RwLock};

use pgt_core::connection::{ConnectionManager, Target};
use pgt_core::registry::Registry;
use pgt_core::testing::{INVALID_UTF8_TYPE_NAME_SQL, TestServer, admin_client, drop_database, test_servers};
use pgt_query::complete::{CompletionItem, CompletionRequest};
use pgt_query::events::{EventSink, QueryEvent};
use pgt_query::session::{QueryManager, Scope, SessionId, SessionOptions, TxMode};
use tempfile::TempDir;

const DB: &str = "pgt_query_utf8";

#[derive(Default)]
struct VecSink(Vec<QueryEvent>);

impl EventSink for VecSink {
    fn send(&mut self, ev: QueryEvent) -> bool {
        self.0.push(ev);
        true
    }
}

struct Fx {
    qm: QueryManager,
    session: SessionId,
    _dir: TempDir,
}

/// PG 14+ ise fixture'lı veritabanında açık oturum.
async fn fixture(ts: &TestServer, tag: &str) -> Option<Fx> {
    let admin = admin_client(ts, &ts.config.maintenance_db).await;
    let v: i32 = admin.query_one("SELECT current_setting('server_version_num')::int4", &[]).await.unwrap().get(0);
    if v < 140000 {
        eprintln!("PG {}: multirange yok, atlanıyor", v / 10000);
        return None;
    }
    let db = format!("{DB}_{tag}");
    drop_database(&admin, &db).await;
    admin.batch_execute(&format!("CREATE DATABASE {db} TEMPLATE template0 ENCODING 'UTF8'")).await.unwrap();
    admin_client(ts, &db).await.batch_execute(INVALID_UTF8_TYPE_NAME_SQL).await.unwrap();
    let oid: u32 = admin.query_one("SELECT oid FROM pg_database WHERE datname = $1", &[&db]).await.unwrap().get(0);
    let dir = tempfile::tempdir().unwrap();
    let mut registry = Registry::load(dir.path().join("servers.json")).unwrap().registry;
    registry.upsert(ts.config.clone()).unwrap();
    let mgr = Arc::new(ConnectionManager::new());
    mgr.connect(ts.config.clone(), ts.password.clone()).await.unwrap();
    let qm = QueryManager::new(mgr, Arc::new(RwLock::new(registry)));
    let session = qm.open(ts.config.id, Target::Database(oid), SessionOptions { mode: TxMode::Autocommit, statement_timeout_ms: 0 }).await.unwrap();
    Some(Fx { qm, session, _dir: dir })
}

async fn cleanup(ts: &TestServer, tag: &str) {
    let admin = admin_client(ts, &ts.config.maintenance_db).await;
    drop_database(&admin, &format!("{DB}_{tag}")).await;
}

fn lossy(s: &str) -> bool {
    s.contains('\u{FFFD}')
}

impl Fx {
    async fn run(&self, sql: &str) -> Vec<QueryEvent> {
        let mut sink = VecSink::default();
        self.qm.run(self.session, sql, Scope::All, 1000, true, &mut sink).await.unwrap();
        sink.0
    }

    async fn complete(&self, req: CompletionRequest) -> Vec<CompletionItem> {
        self.qm.complete(self.session, req).await.unwrap()
    }
}

#[tokio::test]
async fn select_of_invalid_utf8_type_runs_without_type_info() {
    for ts in test_servers() {
        let Some(fx) = fixture(&ts, "sel").await else { continue };
        for sql in ["SELECT m FROM u8.t", "SELECT id, m FROM u8.t ORDER BY id", "SELECT m::text AS s, m FROM u8.t"] {
            let ev = fx.run(sql).await;
            let Some(QueryEvent::Started { columns, .. }) = ev.iter().find(|e| matches!(e, QueryEvent::Started { .. })) else {
                panic!("{sql}: başlamadı: {ev:?}")
            };
            let m = columns.iter().rfind(|c| c.name == "m").unwrap();
            assert_eq!((m.type_name.as_str(), m.type_display.as_str(), m.type_oid), ("?", "?", 0), "{sql}: {m:?}");
            let rows: Vec<_> = ev.iter().filter_map(|e| if let QueryEvent::Rows { rows, .. } = e { Some(rows) } else { None }).flatten().collect();
            assert_eq!(rows.len(), 1, "{sql}: {ev:?}");
            assert_eq!(rows[0].last().unwrap().as_deref(), Some("{[1,3)}"), "{sql}");
            assert!(ev.iter().any(|e| matches!(e, QueryEvent::Done { .. })), "{sql}: {ev:?}");
        }
        // Geçersiz UTF-8 değer (tip adının kendisi) de çökmez: U+FFFD gösterilir; satırsız ifade de çalışır.
        for sql in [
            "SELECT typname FROM pg_type WHERE typtype = 'm' AND typnamespace = 'u8'::regnamespace",
            "SELECT typname::text, format_type(oid, NULL) FROM pg_type WHERE typtype = 'm' AND typnamespace = 'u8'::regnamespace",
        ] {
            let ev = fx.run(sql).await;
            let rows: Vec<_> = ev.iter().filter_map(|e| if let QueryEvent::Rows { rows, .. } = e { Some(rows) } else { None }).flatten().collect();
            assert!(rows.len() == 1 && rows[0].iter().all(|v| v.as_deref() == Some("\u{FFFD}")), "{sql}: {ev:?}");
            assert!(ev.iter().any(|e| matches!(e, QueryEvent::Done { .. })), "{sql}: {ev:?}");
        }
        let ev = fx.run("UPDATE u8.t SET m = m WHERE id = 1").await;
        assert!(ev.iter().any(|e| matches!(e, QueryEvent::Done { rows_affected: Some(1), .. })), "{ev:?}");
        drop(fx);
        cleanup(&ts, "sel").await;
    }
}

#[tokio::test]
async fn column_completion_shows_lossy_type_name() {
    for ts in test_servers() {
        let Some(fx) = fixture(&ts, "cmp").await else { continue };
        let items = fx.complete(CompletionRequest::Columns { schema: Some("u8".into()), relation: "t".into() }).await;
        let m = items.iter().find(|i| i.label == "m").expect("m");
        assert!(m.detail.as_deref().is_some_and(lossy), "{m:?}");
        let items = fx.complete(CompletionRequest::Scoped { sql: "SELECT x. FROM u8.t x".into(), offset: 9, prefix: "x".into() }).await;
        let m = items.iter().find(|i| i.label == "m").expect("m (kapsamlı)");
        assert!(m.detail.as_deref().is_some_and(lossy), "{m:?}");
        drop(fx);
        cleanup(&ts, "cmp").await;
    }
}
