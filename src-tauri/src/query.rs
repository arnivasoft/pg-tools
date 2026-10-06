//! Sorgu Aracı Tauri komutları (`qt_*`). Olaylar `tauri::ipc::Channel` ile akar; SQL metni asla günlüğe yazılmaz.
//!
//! Her komutun gelecekleri `Box::pin` ile kutulanır: büyük gelecekler invoke işleyicisinin ana iş parçacığı
//! yığın çerçevesine gömülürse release derlemesinde 1 MB'lık ana yığın taşar (bkz. commands.rs).
//!
//! Kapı/durum reddi hataları `Error::Invalid(<işaret>)` ile gelir; `pgt_query::error_codes` bunları ayrı
//! `code`lara çevirir (`confirmation_required`, `locked`, `busy`, `tx_failed`, `open_transaction`, `session_reset`; arayüz yalnızca
//! koda bakar). Geri kalan `Invalid` hataları `invalid` kalır.

use std::path::PathBuf;

use pgt_core::ErrorPayload;
use pgt_core::connection::Target;
use pgt_query::check::CheckDiag;
use pgt_query::complete::{CompletionItem, CompletionRequest};
use pgt_query::error_codes::query_payload;
use pgt_query::events::{EventSink, QueryEvent};
use pgt_query::export::{CsvOptions, ExportSummary};
use pgt_query::session::{ClosePolicy, QueryManager, Scope, SessionId, SessionOptions, StmtPlan, TxAction, TxState};
use tauri::State;
use tauri::ipc::Channel;
use uuid::Uuid;

use crate::state::AppState;

type CmdResult<T> = Result<T, ErrorPayload>;

/// `Channel`i çalıştırıcının `EventSink`ine köprüler. Gönderim hatası (webview gitti / sekme kapandı)
/// `false` döner: çalıştırıcı durur, açık imleç kapatılır ve ifade iptal edilir (sızıntı olmaz).
pub struct ChannelSink(pub Channel<QueryEvent>);

impl EventSink for ChannelSink {
    fn send(&mut self, ev: QueryEvent) -> bool {
        self.0.send(ev).is_ok()
    }
}

/// Sayfa (yeniden) yüklenirken çağrılır: webview yenilenince arayüzdeki oturum kimlikleri kaybolur, kimsenin
/// yönetemeyeceği oturumlar (özellikle manuel modda açık transaction tutanlar, kilit bırakmayanlar) sağ
/// kalmamalı. Çalışanlar iptal edilir, açık transaction'lar ROLLBACK edilir. İlk yüklemede kapatacak bir şey yoktur.
pub(crate) async fn close_all_on_reload(q: &QueryManager) {
    q.close_all().await;
}

pub(crate) async fn run_with_sink(
    q: &QueryManager,
    session: SessionId,
    sql: &str,
    scope: Scope,
    limit: u32,
    confirmed: bool,
    sink: &mut dyn EventSink,
) -> CmdResult<()> {
    q.run(session, sql, scope, limit, confirmed, sink).await.map_err(|e| query_payload(&e))
}

pub(crate) async fn more_with_sink(q: &QueryManager, session: SessionId, stmt_index: u32, sink: &mut dyn EventSink) -> CmdResult<()> {
    q.more(session, stmt_index, sink).await.map_err(|e| query_payload(&e))
}

pub(crate) async fn export_with_progress(
    q: &QueryManager,
    session: SessionId,
    sql: &str,
    path: &std::path::Path,
    options: CsvOptions,
    progress: &mut (dyn FnMut(u64) -> bool + Send),
) -> CmdResult<ExportSummary> {
    q.export_csv(session, sql, path, options, progress).await.map_err(|e| query_payload(&e))
}

#[tauri::command]
pub async fn qt_open(state: State<'_, AppState>, server_id: Uuid, database_oid: Option<u32>, options: SessionOptions) -> CmdResult<Uuid> {
    let target = database_oid.map_or(Target::Maintenance, Target::Database);
    Box::pin(state.query.open(server_id, target, options)).await.map_err(|e| query_payload(&e))
}

#[tauri::command]
pub async fn qt_analyze(state: State<'_, AppState>, session: Uuid, sql: String, scope: Scope) -> CmdResult<Vec<StmtPlan>> {
    state.query.analyze(session, &sql, scope).map_err(|e| query_payload(&e))
}

#[tauri::command]
pub async fn qt_run(
    state: State<'_, AppState>,
    session: Uuid,
    sql: String,
    scope: Scope,
    limit: u32,
    confirmed: bool,
    on_event: Channel<QueryEvent>,
) -> CmdResult<()> {
    let mut sink = ChannelSink(on_event);
    Box::pin(run_with_sink(&state.query, session, &sql, scope, limit, confirmed, &mut sink)).await
}

#[tauri::command]
pub async fn qt_more(state: State<'_, AppState>, session: Uuid, stmt_index: u32, on_event: Channel<QueryEvent>) -> CmdResult<()> {
    let mut sink = ChannelSink(on_event);
    Box::pin(more_with_sink(&state.query, session, stmt_index, &mut sink)).await
}

/// Eşzamanlı ve kilitsiz: çalışan ifadeyi (ya da dışa aktarmayı) iptal eder.
#[tauri::command]
pub fn qt_cancel(state: State<'_, AppState>, session: Uuid) {
    state.query.cancel(session);
}

#[tauri::command]
pub async fn qt_tx(state: State<'_, AppState>, session: Uuid, action: TxAction) -> CmdResult<TxState> {
    Box::pin(state.query.tx(session, action)).await.map_err(|e| query_payload(&e))
}

#[tauri::command]
pub fn qt_tx_state(state: State<'_, AppState>, session: Uuid) -> CmdResult<TxState> {
    state.query.tx_state(session).map_err(|e| query_payload(&e))
}

#[tauri::command]
pub async fn qt_set_options(state: State<'_, AppState>, session: Uuid, options: SessionOptions) -> CmdResult<()> {
    Box::pin(state.query.set_options(session, options)).await.map_err(|e| query_payload(&e))
}

#[tauri::command]
pub async fn qt_close(state: State<'_, AppState>, session: Uuid, policy: ClosePolicy) -> CmdResult<()> {
    Box::pin(state.query.close(session, policy)).await.map_err(|e| query_payload(&e))
}

#[tauri::command]
pub async fn qt_complete(state: State<'_, AppState>, session: Uuid, request: CompletionRequest) -> CmdResult<Vec<CompletionItem>> {
    Box::pin(state.query.complete(session, request)).await.map_err(|e| query_payload(&e))
}

/// Parse tabanlı hata denetimi (çalıştırmaz). Meşgul/uykudaki/hatalı transaction'daki oturumda boş liste.
#[tauri::command]
pub async fn qt_check(state: State<'_, AppState>, session: Uuid, sql: String, scope: Scope) -> CmdResult<Vec<CheckDiag>> {
    Box::pin(state.query.check(session, &sql, scope)).await.map_err(|e| query_payload(&e))
}

/// İlerleme kanalı kapanırsa (gönderim hatası) dışa aktarma iptal edilir.
#[tauri::command]
pub async fn qt_export_csv(
    state: State<'_, AppState>,
    session: Uuid,
    sql: String,
    path: String,
    options: CsvOptions,
    on_progress: Channel<u64>,
) -> CmdResult<ExportSummary> {
    let path = PathBuf::from(path);
    let mut progress = move |rows: u64| on_progress.send(rows).is_ok();
    Box::pin(export_with_progress(&state.query, session, &sql, &path, options, &mut progress)).await
}

/// Docker'lı entegrasyon testleri için ortak yardımcılar (`PGT_TEST_URLS` yoksa kurulum `None` döner).
#[cfg(test)]
pub(crate) mod testkit {
    use std::sync::{Arc, RwLock};
    use std::time::{Duration, Instant};

    use pgt_core::connection::{ConnectionManager, Target};
    use pgt_core::registry::Registry;
    use pgt_core::testing::{TestServer, test_servers};
    use pgt_query::events::{EventSink, QueryEvent};
    use pgt_query::session::{QueryManager, Scope, SessionId, SessionOptions, TxMode};

    pub struct Fx {
        pub ts: TestServer,
        pub registry: Arc<RwLock<Registry>>,
        pub manager: Arc<ConnectionManager>,
        pub qm: Arc<QueryManager>,
        /// Ayrı bir yönetici (kapatılan oturumlardan bağımsız) oturumu: `pg_stat_activity` vb. için.
        admin: (QueryManager, SessionId),
        pub dir: tempfile::TempDir,
    }

    pub async fn fixture() -> Option<Fx> {
        let Some(ts) = test_servers().into_iter().next() else {
            eprintln!("PGT_TEST_URLS tanımlı değil, test atlanıyor");
            return None;
        };
        let dir = tempfile::tempdir().unwrap();
        let mut registry = Registry::load(dir.path().join("servers.json")).unwrap().registry;
        registry.upsert(ts.config.clone()).unwrap();
        let registry = Arc::new(RwLock::new(registry));
        let manager = Arc::new(ConnectionManager::new());
        manager.connect(ts.config.clone(), ts.password.clone()).await.unwrap();
        let qm = Arc::new(QueryManager::new(manager.clone(), registry.clone()));
        let admin_qm = QueryManager::new(manager.clone(), registry.clone());
        let admin_id =
            admin_qm.open(ts.config.id, Target::Maintenance, SessionOptions { mode: TxMode::Autocommit, statement_timeout_ms: 0 }).await.unwrap();
        let admin = (admin_qm, admin_id);
        Some(Fx { ts, registry, manager, qm, admin, dir })
    }

    #[derive(Default)]
    pub struct Collect(pub Vec<QueryEvent>);
    impl EventSink for Collect {
        fn send(&mut self, ev: QueryEvent) -> bool {
            self.0.push(ev);
            true
        }
    }

    impl Fx {
        pub async fn open(&self, mode: TxMode) -> SessionId {
            let opts = SessionOptions { mode, statement_timeout_ms: 0 };
            self.qm.open(self.ts.config.id, Target::Maintenance, opts).await.unwrap()
        }

        pub async fn exec(&self, id: SessionId, sql: &str) -> Vec<QueryEvent> {
            let mut sink = Collect::default();
            self.qm.run(id, sql, Scope::All, 1000, true, &mut sink).await.unwrap();
            sink.0
        }

        pub async fn pid(&self, id: SessionId) -> i32 {
            for ev in self.exec(id, "select pg_backend_pid()").await {
                if let QueryEvent::Rows { rows, .. } = ev {
                    return rows[0][0].as_ref().unwrap().parse().unwrap();
                }
            }
            panic!("pid alınamadı");
        }

        async fn admin_rows(&self, sql: &str) -> Vec<Vec<Option<String>>> {
            let mut sink = Collect::default();
            self.admin.0.run(self.admin.1, sql, Scope::All, 1000, true, &mut sink).await.unwrap();
            sink.0
                .into_iter()
                .filter_map(|e| if let QueryEvent::Rows { rows, .. } = e { Some(rows) } else { None })
                .flatten()
                .collect()
        }

        /// `pg_stat_activity.state` (yoksa `None`: backend gitti).
        pub async fn backend_state(&self, pid: i32) -> Option<String> {
            let rows = self.admin_rows(&format!("select coalesce(state, '') from pg_stat_activity where pid = {pid}")).await;
            rows.first().map(|r| r[0].clone().unwrap_or_default())
        }

        pub async fn wait_gone(&self, pid: i32) -> bool {
            let start = Instant::now();
            while start.elapsed() < Duration::from_secs(8) {
                if self.backend_state(pid).await.is_none() {
                    return true;
                }
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
            false
        }

        pub async fn table_exists(&self, name: &str) -> bool {
            let rows = self.admin_rows(&format!("select to_regclass('{name}') is not null")).await;
            rows[0][0].as_deref() == Some("true")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use pgt_core::connection::ConnectionManager;
    use pgt_core::registry::Registry;
    use pgt_core::testing::test_servers;
    use pgt_query::session::TxMode;
    use tauri::ipc::InvokeResponseBody;

    fn ev() -> QueryEvent {
        QueryEvent::ScriptDone { stmts_run: 0, stopped_on_error: false }
    }

    #[test]
    fn channel_sink_returns_false_once_the_channel_errors() {
        let sent = Arc::new(AtomicUsize::new(0));
        let n = sent.clone();
        let ch = Channel::<QueryEvent>::new(move |_body: InvokeResponseBody| {
            if n.fetch_add(1, Ordering::SeqCst) < 2 { Ok(()) } else { Err(tauri::Error::WebviewNotFound) }
        });
        let mut sink = ChannelSink(ch);
        assert!(sink.send(ev()));
        assert!(sink.send(ev()));
        assert!(!sink.send(ev()), "gönderim hatası alıcının gittiğini bildirmeli");
    }

    #[tokio::test]
    async fn unknown_session_maps_to_not_found() {
        let qm = QueryManager::new(Arc::new(ConnectionManager::new()), Arc::new(std::sync::RwLock::new(Registry::unavailable("servers.json"))));
        let mut sink = ChannelSink(Channel::new(|_| Ok(())));
        let err = run_with_sink(&qm, Uuid::new_v4(), "select 1", Scope::All, 10, false, &mut sink).await.unwrap_err();
        assert_eq!(err.code, "not_found");
    }

    /// Docker'lı sunucu gerekir (`PGT_TEST_URLS`); yoksa atlanır.
    #[tokio::test]
    async fn gates_use_dedicated_codes_and_a_closed_channel_stops_the_statement() {
        let Some(ts) = test_servers().into_iter().next() else {
            eprintln!("PGT_TEST_URLS tanımlı değil, test atlanıyor");
            return;
        };
        let dir = tempfile::tempdir().unwrap();
        let mut registry = Registry::load(dir.path().join("servers.json")).unwrap().registry;
        registry.upsert(ts.config.clone()).unwrap();
        let mgr = Arc::new(ConnectionManager::new());
        mgr.connect(ts.config.clone(), ts.password.clone()).await.unwrap();
        let qm = QueryManager::new(mgr, Arc::new(std::sync::RwLock::new(registry)));
        let opts = SessionOptions { mode: TxMode::Autocommit, statement_timeout_ms: 0 };
        let id = qm.open(ts.config.id, Target::Maintenance, opts).await.unwrap();

        // Tehlikeli ifade onaysız: ayrı kod, "Geçersiz ayar" öneki yok.
        let mut sink = ChannelSink(Channel::new(|_| Ok(())));
        let err = run_with_sink(&qm, id, "drop table if exists pgt_qt8_none", Scope::All, 10, false, &mut sink).await.unwrap_err();
        assert_eq!(err.code, "confirmation_required");
        assert!(!err.message.contains("Geçersiz ayar"));

        assert_eq!(qm.tx_state(id).unwrap(), TxState::Idle);
        let count = Arc::new(AtomicUsize::new(0));
        let c = count.clone();
        let mut ok = ChannelSink(Channel::new(move |_| {
            c.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }));
        run_with_sink(&qm, id, "select 1", Scope::All, 10, true, &mut ok).await.unwrap();
        assert!(count.load(Ordering::SeqCst) >= 3, "started + rows + done + scriptDone bekleniyordu");

        // Meşgul oturum: ikinci çalıştırma `busy` koduyla reddedilir.
        let qm = Arc::new(qm);
        let q2 = qm.clone();
        let slow = tokio::spawn(async move {
            let mut s = ChannelSink(Channel::new(|_| Ok(())));
            run_with_sink(&q2, id, "select pg_sleep(2)", Scope::All, 10, true, &mut s).await
        });
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        let mut s = ChannelSink(Channel::new(|_| Ok(())));
        let err = run_with_sink(&qm, id, "select 1", Scope::All, 10, true, &mut s).await.unwrap_err();
        assert_eq!(err.code, "busy");
        qm.cancel(id);
        let _ = slow.await.unwrap();
        qm.close(id, ClosePolicy::Rollback).await.unwrap();
    }

    /// Yalnızca durdurma geçirir: sorgu ~100 milyon satır üretir (satır sınırı çok büyük; `select generate_series(..)` ProjectSet olduğundan tembel, FROM içindeki SRF gibi baştan gerçekleşmez); kanal ilk `Rows`tan sonra
    /// kapanır. Durmasaydı dakikalarca sürer.
    #[tokio::test]
    async fn closed_channel_stops_a_long_statement_and_leaves_no_open_cursor_or_transaction() {
        let Some(fx) = testkit::fixture().await else { return };
        let id = fx.open(TxMode::Autocommit).await;
        let pid = fx.pid(id).await;
        let msgs = Arc::new(AtomicUsize::new(0));
        let m = msgs.clone();
        // Started + ilk Rows geçer, sonrası Err (webview gitti).
        let mut sink = ChannelSink(Channel::new(move |_| {
            if m.fetch_add(1, Ordering::SeqCst) < 2 { Ok(()) } else { Err(tauri::Error::WebviewNotFound) }
        }));
        let t = std::time::Instant::now();
        run_with_sink(&fx.qm, id, "select generate_series(1, 100000000) as i", Scope::All, 100_000_000, true, &mut sink)
            .await
            .unwrap();
        assert!(t.elapsed() < std::time::Duration::from_secs(2), "durma bekleniyordu: {:?}", t.elapsed());
        assert!(msgs.load(Ordering::SeqCst) <= 4, "durduktan sonra gönderim sürmemeli");
        // Sunucuda ne etkin sorgu ne de açık transaction kalmış olmalı.
        let state = fx.backend_state(pid).await.expect("oturum sürüyor");
        assert_eq!(state, "idle", "idle in transaction / active kalmamalı");
        assert_eq!(fx.qm.tx_state(id).unwrap(), TxState::Idle);
        let evs = fx.exec(id, "select 1").await;
        assert!(evs.iter().any(|e| matches!(e, QueryEvent::Done { .. })), "oturum yeniden kullanılabilmeli");
        fx.qm.close(id, ClosePolicy::Rollback).await.unwrap();
    }

    /// Sayfa yenilenince: tüm oturumlar kapanır, çalışan ifade iptal edilir, açık transaction geri alınır.
    #[tokio::test]
    async fn reload_closes_every_session_rolling_back_open_transactions() {
        let Some(fx) = testkit::fixture().await else { return };
        let manual = fx.open(TxMode::Manual).await;
        let idle = fx.open(TxMode::Autocommit).await;
        let busy = fx.open(TxMode::Autocommit).await;
        fx.exec(manual, "create table pgt_qt8_reload(i int)").await;
        assert_eq!(fx.qm.tx_state(manual).unwrap(), TxState::InTransaction);
        let pids = [fx.pid(manual).await, fx.pid(idle).await, fx.pid(busy).await];
        let q2 = fx.qm.clone();
        let sleeper = tokio::spawn(async move {
            let mut s = testkit::Collect::default();
            q2.run(busy, "select pg_sleep(30)", Scope::All, 10, true, &mut s).await
        });
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        let t = std::time::Instant::now();
        close_all_on_reload(&fx.qm).await;
        assert!(t.elapsed() < std::time::Duration::from_secs(8));
        for id in [manual, idle, busy] {
            assert!(fx.qm.tx_state(id).is_err(), "oturum kapanmış olmalı");
        }
        for pid in pids {
            assert!(fx.wait_gone(pid).await, "backend {pid} kapanmalı");
        }
        assert!(!fx.table_exists("pgt_qt8_reload").await, "açık transaction geri alınmalı");
        let _ = sleeper.await;
        // İlk yükleme: kapatılacak oturum yok, sorunsuz döner.
        close_all_on_reload(&fx.qm).await;
    }
}
