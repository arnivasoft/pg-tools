//! Sorgu Aracı Tauri komutları (`qt_*`). Olaylar `tauri::ipc::Channel` ile akar; SQL metni asla günlüğe yazılmaz.
//!
//! Her komutun gelecekleri `Box::pin` ile kutulanır: büyük gelecekler invoke işleyicisinin ana iş parçacığı
//! yığın çerçevesine gömülürse release derlemesinde 1 MB'lık ana yığın taşar (bkz. commands.rs).
//!
//! Kapı/durum reddi hataları `Error::Invalid(<işaret>)` ile gelir; `pgt_query::error_codes` bunları ayrı
//! `code`lara çevirir (`confirmation_required`, `locked`, `busy`, `tx_failed`, `open_transaction`; arayüz yalnızca
//! koda bakar). Geri kalan `Invalid` hataları `invalid` kalır.

use std::path::PathBuf;

use pgt_core::ErrorPayload;
use pgt_core::connection::Target;
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
pub fn qt_analyze(state: State<'_, AppState>, session: Uuid, sql: String, scope: Scope) -> CmdResult<Vec<StmtPlan>> {
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

        // Kanal ilk gönderimde kapanır: çalıştırma biter, oturum meşgul kalmaz ve yeniden kullanılabilir.
        let mut closed = ChannelSink(Channel::new(|_| Err(tauri::Error::WebviewNotFound)));
        run_with_sink(&qm, id, "select g from generate_series(1, 200000) g", Scope::All, 1000, true, &mut closed).await.unwrap();
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
}
