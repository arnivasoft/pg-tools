use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};

use pgt_catalog::Catalog;
use pgt_core::connection::ConnectionManager;
use pgt_core::credentials::{Credentials, SecretStore};
use pgt_core::registry::Registry;

pub struct AppState {
    pub registry: Arc<RwLock<Registry>>,
    pub credentials: Credentials,
    pub manager: Arc<ConnectionManager>,
    pub catalog: Catalog,
    pub startup_notice: Option<String>,
}

impl AppState {
    pub fn with_store(config_dir: &Path, store: Arc<dyn SecretStore>) -> pgt_core::Result<Self> {
        let outcome = Registry::load(config_dir.join("servers.json"))?;
        let startup_notice = outcome.recovered_backup.map(|b| {
            format!("servers.json okunamadı; {} olarak yedeklendi ve boş sunucu listesiyle başlandı.", b.display())
        });
        let registry = Arc::new(RwLock::new(outcome.registry));
        let manager = Arc::new(ConnectionManager::new());
        Ok(Self {
            catalog: Catalog::new(manager.clone(), registry.clone()),
            credentials: Credentials::new(store),
            registry,
            manager,
            startup_notice,
        })
    }

    pub fn init() -> pgt_core::Result<Self> {
        #[cfg(windows)]
        let store: Arc<dyn SecretStore> = Arc::new(pgt_core::credentials::WindowsSecretStore::new()?);
        #[cfg(not(windows))]
        let store: Arc<dyn SecretStore> = Arc::new(pgt_core::credentials::MemorySecretStore::default());
        Self::with_store(&config_dir(), store)
    }
}

fn env_dir(var: &str) -> PathBuf {
    std::env::var_os(var).map(PathBuf::from).unwrap_or_else(|| PathBuf::from("."))
}

/// `%APPDATA%\pg-tools`
pub fn config_dir() -> PathBuf {
    env_dir("APPDATA").join("pg-tools")
}

/// `%LOCALAPPDATA%\pg-tools\logs`
pub fn log_dir() -> PathBuf {
    env_dir("LOCALAPPDATA").join("pg-tools").join("logs")
}

#[cfg(test)]
mod tests {
    use super::*;
    use pgt_core::credentials::MemorySecretStore;

    #[test]
    fn startup_notice_after_corrupt_registry() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("servers.json"), "bozuk").unwrap();
        let state = AppState::with_store(dir.path(), Arc::new(MemorySecretStore::default())).unwrap();
        let notice = state.startup_notice.clone().expect("uyarı bekleniyordu");
        assert!(notice.contains("servers.json.bak-"), "{notice}");
        assert!(state.registry.read().unwrap().servers().is_empty());
    }

    #[test]
    fn clean_start_has_no_notice() {
        let dir = tempfile::tempdir().unwrap();
        let state = AppState::with_store(dir.path(), Arc::new(MemorySecretStore::default())).unwrap();
        assert!(state.startup_notice.is_none());
    }
}
