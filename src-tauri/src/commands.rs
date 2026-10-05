use pgt_catalog::node::{NodeId, TreeNode};
use pgt_catalog::props::PropertySection;
use pgt_core::config::ServerConfig;
use pgt_core::credentials::Credentials;
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

/// Typed password wins; otherwise fall back to the stored/session one.
fn effective_password(creds: &Credentials, id: Uuid, typed: Option<String>) -> Result<Option<String>, Error> {
    match typed {
        Some(p) => Ok(Some(p)),
        None => creds.password(id),
    }
}

#[tauri::command]
pub async fn test_connection(state: State<'_, AppState>, config: ServerConfig, password: Option<String>) -> CmdResult<ServerInfo> {
    let config = config.normalized();
    config.validate()?;
    let effective = effective_password(&state.credentials, config.id, password)?;
    Ok(ConnectionManager::test(&config, effective.as_deref()).await?)
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

#[cfg(test)]
mod tests {
    use super::*;
    use pgt_core::credentials::{Credentials, MemorySecretStore};
    use std::sync::Arc;

    fn creds() -> Credentials {
        Credentials::new(Arc::new(MemorySecretStore::default()))
    }

    #[test]
    fn typed_password_wins_over_stored() {
        let c = creds();
        let id = Uuid::new_v4();
        c.remember(id, "stored", true).unwrap();
        assert_eq!(effective_password(&c, id, Some("typed".into())).unwrap().as_deref(), Some("typed"));
    }

    #[test]
    fn missing_password_falls_back_to_remembered() {
        let c = creds();
        let id = Uuid::new_v4();
        c.remember(id, "stored", true).unwrap();
        assert_eq!(effective_password(&c, id, None).unwrap().as_deref(), Some("stored"));
    }

    #[test]
    fn nothing_stored_yields_none() {
        assert_eq!(effective_password(&creds(), Uuid::new_v4(), None).unwrap(), None);
    }
}
