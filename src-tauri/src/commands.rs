use pgt_catalog::node::{NodeId, TreeNode};
use pgt_catalog::props::PropertySection;
use std::sync::RwLock;

use pgt_core::config::ServerConfig;
use pgt_core::credentials::Credentials;
use pgt_core::connection::{ConnectionManager, ServerInfo};
use pgt_core::registry::Registry;
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
    let (saved, prev) = {
        let mut registry = state.registry.write().unwrap();
        let prev = registry.get(config.id).cloned();
        (registry.upsert(config)?, prev)
    };
    state.manager.disconnect(saved.id);
    let forget = should_forget_stored(prev.as_ref(), &saved, password.as_deref());
    match (password, saved.save_password) {
        (Some(p), true) => state.credentials.remember(saved.id, &p, true)?,
        (_, false) => state.credentials.forget(saved.id)?,
        (None, true) if forget => state.credentials.forget(saved.id)?,
        (None, true) => {}
    }
    Ok(saved)
}

/// A stored password may only be used for the same host/port/user/maintenance db.
fn same_target(a: &ServerConfig, b: &ServerConfig) -> bool {
    a.host == b.host && a.port == b.port && a.username == b.username && a.maintenance_db == b.maintenance_db
}

/// Typed password wins; otherwise fall back to the stored/session one,
/// but only while the target equals the registered one.
fn effective_password(creds: &Credentials, registered: Option<&ServerConfig>, config: &ServerConfig, typed: Option<String>) -> Result<Option<String>, Error> {
    match typed {
        Some(p) => Ok(Some(p)),
        None if registered.is_some_and(|r| same_target(r, config)) => creds.password(config.id),
        None => Ok(None),
    }
}

/// True when a kept (not retyped) password would otherwise follow a changed target.
fn should_forget_stored(prev: Option<&ServerConfig>, new: &ServerConfig, typed: Option<&str>) -> bool {
    typed.is_none() && prev.is_some_and(|p| !same_target(p, new))
}

#[tauri::command]
pub async fn test_connection(state: State<'_, AppState>, config: ServerConfig, password: Option<String>) -> CmdResult<ServerInfo> {
    let config = config.normalized();
    config.validate()?;
    let registered = state.registry.read().unwrap().get(config.id).cloned();
    let effective = effective_password(&state.credentials, registered.as_ref(), &config, password)?;
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
        remember_after_connect(&state.registry, &state.credentials, &config, &p, remember);
    }
    Ok(info)
}

/// Başarılı bir bağlantıdan sonra yazılan parolayı saklar. Bağlantı zaten kuruldu: kalıcı kayıt
/// (servers.json ya da Kimlik Bilgisi Yöneticisi) başarısız olursa yalnızca uyarı günlüklenir;
/// parola her durumda oturum belleğinde kalır (Credentials::remember önce oturumu günceller).
fn remember_after_connect(registry: &RwLock<Registry>, creds: &Credentials, config: &ServerConfig, password: &str, remember: bool) {
    let mut persist = config.save_password;
    if remember && !config.save_password {
        let mut updated = config.clone();
        updated.save_password = true;
        match registry.write().unwrap().upsert(updated) {
            Ok(_) => persist = true,
            Err(e) => tracing::warn!(server = %config.id, error = %e, "\"parolayı kaydet\" tercihi kaydedilemedi"),
        }
    }
    if let Err(e) = creds.remember(config.id, password, persist) {
        tracing::warn!(server = %config.id, error = %e, "parola kalıcı depoya yazılamadı; yalnızca bu oturumda kullanılacak");
    }
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

    struct FailingStore;
    impl pgt_core::credentials::SecretStore for FailingStore {
        fn get(&self, _: &str) -> pgt_core::Result<Option<String>> {
            Ok(None)
        }
        fn set(&self, _: &str, _: &str) -> pgt_core::Result<()> {
            Err(Error::Credential("depo kilitli".into()))
        }
        fn delete(&self, _: &str) -> pgt_core::Result<()> {
            Err(Error::Credential("depo kilitli".into()))
        }
    }

    fn registry_with(dir: &std::path::Path, config: &ServerConfig) -> RwLock<Registry> {
        let mut r = Registry::load(dir.join("servers.json")).unwrap().registry;
        r.upsert(config.clone()).unwrap();
        RwLock::new(r)
    }

    #[test]
    fn credential_store_failure_keeps_the_session_password() {
        let dir = tempfile::tempdir().unwrap();
        let mut config = cfg(Uuid::new_v4());
        config.save_password = false;
        let registry = registry_with(dir.path(), &config);
        let c = Credentials::new(Arc::new(FailingStore));
        remember_after_connect(&registry, &c, &config, "typed", true);
        assert_eq!(c.password(config.id).unwrap().as_deref(), Some("typed"));
        assert!(registry.read().unwrap().get(config.id).unwrap().save_password);
    }

    #[test]
    fn registry_save_failure_keeps_the_session_password_and_does_not_persist() {
        let dir = tempfile::tempdir().unwrap();
        let mut config = cfg(Uuid::new_v4());
        config.save_password = false;
        let registry = registry_with(dir.path(), &config);
        // servers.json'ın yerine bir dizin koy: sonraki kayıt (rename) başarısız olur.
        std::fs::remove_file(dir.path().join("servers.json")).unwrap();
        std::fs::create_dir(dir.path().join("servers.json")).unwrap();
        let store = Arc::new(MemorySecretStore::default());
        let c = Credentials::new(store.clone());
        remember_after_connect(&registry, &c, &config, "typed", true);
        assert_eq!(c.password(config.id).unwrap().as_deref(), Some("typed"));
        use pgt_core::credentials::{SecretStore, password_key};
        assert_eq!(store.get(&password_key(config.id)).unwrap(), None, "tercih kaydedilemediyse kalıcı depoya yazılmamalı");
    }

    #[test]
    fn remember_persists_when_preference_saved() {
        let dir = tempfile::tempdir().unwrap();
        let mut config = cfg(Uuid::new_v4());
        config.save_password = false;
        let registry = registry_with(dir.path(), &config);
        let store = Arc::new(MemorySecretStore::default());
        let c = Credentials::new(store.clone());
        remember_after_connect(&registry, &c, &config, "typed", true);
        use pgt_core::credentials::{SecretStore, password_key};
        assert_eq!(store.get(&password_key(config.id)).unwrap().as_deref(), Some("typed"));
    }

    fn cfg(id: Uuid) -> ServerConfig {
        ServerConfig {
            id,
            name: "s".into(),
            group: None,
            host: "db".into(),
            port: 5432,
            maintenance_db: "postgres".into(),
            username: "app".into(),
            save_password: true,
            ssl: Default::default(),
        }
    }

    #[test]
    fn typed_password_wins_over_stored() {
        let c = creds();
        let id = Uuid::new_v4();
        c.remember(id, "stored", true).unwrap();
        let cur = cfg(id);
        assert_eq!(effective_password(&c, Some(&cur), &cur, Some("typed".into())).unwrap().as_deref(), Some("typed"));
    }

    #[test]
    fn missing_password_falls_back_when_target_is_same() {
        let c = creds();
        let id = Uuid::new_v4();
        c.remember(id, "stored", true).unwrap();
        let cur = cfg(id);
        assert_eq!(effective_password(&c, Some(&cur), &cur, None).unwrap().as_deref(), Some("stored"));
    }

    #[test]
    fn no_fallback_when_target_changed_or_unregistered() {
        let c = creds();
        let id = Uuid::new_v4();
        c.remember(id, "stored", true).unwrap();
        let reg = cfg(id);
        for f in [
            |x: &mut ServerConfig| x.host = "other".into(),
            |x: &mut ServerConfig| x.port = 1,
            |x: &mut ServerConfig| x.username = "u2".into(),
            |x: &mut ServerConfig| x.maintenance_db = "d2".into(),
        ] {
            let mut new = cfg(id);
            f(&mut new);
            assert_eq!(effective_password(&c, Some(&reg), &new, None).unwrap(), None);
        }
        assert_eq!(effective_password(&c, None, &reg, None).unwrap(), None);
    }

    #[test]
    fn nothing_stored_yields_none() {
        let id = Uuid::new_v4();
        let cur = cfg(id);
        assert_eq!(effective_password(&creds(), Some(&cur), &cur, None).unwrap(), None);
    }

    #[test]
    fn forget_decision() {
        let id = Uuid::new_v4();
        let prev = cfg(id);
        let mut moved = cfg(id);
        moved.host = "other".into();
        assert!(!should_forget_stored(Some(&prev), &prev, None));
        assert!(should_forget_stored(Some(&prev), &moved, None));
        assert!(!should_forget_stored(Some(&prev), &moved, Some("typed")));
        assert!(!should_forget_stored(None, &moved, None));
    }
}
