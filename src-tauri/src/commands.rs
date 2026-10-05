use pgt_catalog::node::{NodeId, TreeNode};
use pgt_catalog::props::PropertySection;
use pgt_core::config::ServerConfig;
use pgt_core::connection::{ConnectionManager, ServerInfo};
use pgt_core::{Error, ErrorPayload};
use tauri::State;
use uuid::Uuid;

use crate::state::AppState;

type CmdResult<T> = Result<T, ErrorPayload>;

#[tauri::command]
pub fn list_servers(state: State<'_, AppState>) -> CmdResult<Vec<ServerConfig>> {
    Ok(state.registry.read().unwrap().servers().to_vec())
}

#[tauri::command]
pub fn save_server(state: State<'_, AppState>, config: ServerConfig, password: Option<String>) -> CmdResult<ServerConfig> {
    let saved = state.registry.write().unwrap().upsert(config)?;
    state.manager.disconnect(saved.id);
    match (password, saved.save_password) {
        (Some(p), true) => state.credentials.remember(saved.id, &p, true)?,
        (_, false) => state.credentials.forget(saved.id)?,
        (None, true) => {}
    }
    Ok(saved)
}

#[tauri::command]
pub async fn test_connection(config: ServerConfig, password: Option<String>) -> CmdResult<ServerInfo> {
    let config = config.normalized();
    config.validate()?;
    Ok(ConnectionManager::test(&config, password.as_deref()).await?)
}

#[tauri::command]
pub async fn connect_server(state: State<'_, AppState>, server_id: Uuid, password: Option<String>, remember: bool) -> CmdResult<ServerInfo> {
    let config = state.registry.read().unwrap().get(server_id).cloned().ok_or(Error::UnknownServer(server_id))?;
    let effective = match &password {
        Some(p) => Some(p.clone()),
        None => state.credentials.password(server_id)?,
    };
    let info = state.manager.connect(config.clone(), effective).await?;
    if let Some(p) = password {
        if remember && !config.save_password {
            let mut updated = config.clone();
            updated.save_password = true;
            state.registry.write().unwrap().upsert(updated)?;
        }
        state.credentials.remember(server_id, &p, remember || config.save_password)?;
    }
    Ok(info)
}

#[tauri::command]
pub fn disconnect_server(state: State<'_, AppState>, server_id: Uuid) {
    state.manager.disconnect(server_id);
}

#[tauri::command]
pub async fn list_children(state: State<'_, AppState>, id: String) -> CmdResult<Vec<TreeNode>> {
    Ok(state.catalog.list_children(&NodeId::parse(&id)?).await?)
}

#[tauri::command]
pub async fn get_properties(state: State<'_, AppState>, id: String) -> CmdResult<Vec<PropertySection>> {
    Ok(state.catalog.properties(&NodeId::parse(&id)?).await?)
}

#[tauri::command]
pub async fn get_ddl(state: State<'_, AppState>, id: String) -> CmdResult<String> {
    Ok(state.catalog.ddl(&NodeId::parse(&id)?).await?)
}

#[tauri::command]
pub fn startup_notice(state: State<'_, AppState>) -> Option<String> {
    state.startup_notice.clone()
}
