//! Dashboard Tauri komutları (`dash_*`). Sorgu metni/parola/URL asla günlüğe yazılmaz.
//!
//! Her komutun gelecekleri `Box::pin` ile kutulanır: büyük gelecekler invoke işleyicisinin ana iş parçacığı
//! yığın çerçevesine gömülürse release derlemesinde 1 MB'lık ana yığın taşar (bkz. commands.rs).
//! Tauri'de argüman adları arayüzde camelCase görünür (`server_id` -> `serverId`, `database_oid` -> `databaseOid`).

use pgt_core::ErrorPayload;
use pgt_dash::{
    ActivityRow, DashInfo, LockReport, Overview, PreparedXact, Sample, Setting, SignalKind, SignalOutcome, Statements, StatementsOrder,
};
use tauri::State;
use uuid::Uuid;

use crate::state::AppState;

type CmdResult<T> = Result<T, ErrorPayload>;

#[tauri::command]
pub async fn dash_open(state: State<'_, AppState>, server_id: Uuid, database_oid: Option<u32>) -> CmdResult<DashInfo> {
    Box::pin(state.dash.open(server_id, database_oid)).await.map_err(ErrorPayload::from)
}

#[tauri::command]
pub async fn dash_close(state: State<'_, AppState>, id: Uuid) -> CmdResult<()> {
    state.dash.close(id);
    Ok(())
}

#[tauri::command]
pub async fn dash_sample(state: State<'_, AppState>, id: Uuid) -> CmdResult<Sample> {
    Box::pin(state.dash.sample(id)).await.map_err(ErrorPayload::from)
}

#[tauri::command]
pub async fn dash_overview(state: State<'_, AppState>, id: Uuid) -> CmdResult<Overview> {
    Box::pin(state.dash.overview(id)).await.map_err(ErrorPayload::from)
}

#[tauri::command]
pub async fn dash_activity(state: State<'_, AppState>, id: Uuid) -> CmdResult<Vec<ActivityRow>> {
    Box::pin(state.dash.activity(id)).await.map_err(ErrorPayload::from)
}

#[tauri::command]
pub async fn dash_locks(state: State<'_, AppState>, id: Uuid) -> CmdResult<LockReport> {
    Box::pin(state.dash.locks(id)).await.map_err(ErrorPayload::from)
}

#[tauri::command]
pub async fn dash_settings(state: State<'_, AppState>, id: Uuid) -> CmdResult<Vec<Setting>> {
    Box::pin(state.dash.settings(id)).await.map_err(ErrorPayload::from)
}

#[tauri::command]
pub async fn dash_prepared(state: State<'_, AppState>, id: Uuid) -> CmdResult<Vec<PreparedXact>> {
    Box::pin(state.dash.prepared(id)).await.map_err(ErrorPayload::from)
}

#[tauri::command]
pub async fn dash_statements(state: State<'_, AppState>, id: Uuid, order: StatementsOrder) -> CmdResult<Statements> {
    Box::pin(state.dash.statements(id, order)).await.map_err(ErrorPayload::from)
}

#[tauri::command]
pub async fn dash_signal(state: State<'_, AppState>, id: Uuid, pid: i32, kind: SignalKind) -> CmdResult<SignalOutcome> {
    Box::pin(state.dash.signal(id, pid, kind)).await.map_err(ErrorPayload::from)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    use pgt_core::connection::ConnectionManager;
    use pgt_dash::DashManager;

    #[tokio::test]
    async fn unknown_session_maps_to_not_found() {
        let dash = DashManager::new(Arc::new(ConnectionManager::new()));
        let err = Box::pin(dash.sample(Uuid::new_v4())).await.map_err(ErrorPayload::from).unwrap_err();
        assert_eq!(err.code, "not_found");
    }

    /// Docker'lı sunucu gerekir (`PGT_TEST_URLS`); yoksa atlanır. Kendi pid'ine sinyal `invalid` koduyla reddedilir.
    #[tokio::test]
    async fn signalling_own_backend_is_invalid() {
        let Some(fx) = crate::query::testkit::fixture().await else { return };
        let dash = DashManager::new(fx.manager.clone());
        let info = Box::pin(dash.open(fx.ts.config.id, None)).await.unwrap();
        let pid = info.self_pid;
        let err = Box::pin(dash.signal(info.id, pid, SignalKind::Cancel)).await.map_err(ErrorPayload::from).unwrap_err();
        assert_eq!(err.code, "invalid");
        dash.close(info.id);
    }
}
