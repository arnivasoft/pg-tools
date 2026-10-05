use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};

use pgt_catalog::Catalog;
use pgt_core::connection::ConnectionManager;
use pgt_core::credentials::{Credentials, MemorySecretStore, SecretStore};
use pgt_core::registry::Registry;

pub struct AppState {
    pub registry: Arc<RwLock<Registry>>,
    pub credentials: Credentials,
    pub manager: Arc<ConnectionManager>,
    pub catalog: Catalog,
    pub startup_notice: Option<String>,
}

const NO_SECRET_STORE: &str =
    "Windows Kimlik Bilgisi Yöneticisi açılamadı; parolalar kaydedilemez, yalnızca bu oturumda bellekte tutulur.";

impl AppState {
    pub fn with_store(config_dir: &Path, store: Arc<dyn SecretStore>) -> Self {
        Self::build(config_dir, Ok(store))
    }

    /// Başlangıç asla başarısız olmaz (pencere alt sisteminde sessizce kapanmamak için):
    /// okunamayan servers.json ya da açılamayan kimlik deposu için güvenli bir yedek kullanılır
    /// ve durum çubuğunda gösterilecek bir başlangıç uyarısı üretilir.
    fn build(config_dir: &Path, store: pgt_core::Result<Arc<dyn SecretStore>>) -> Self {
        let mut notices = Vec::new();
        let path = config_dir.join("servers.json");
        let registry = match Registry::load(&path) {
            Ok(outcome) => {
                if let Some(b) = outcome.recovered_backup {
                    notices.push(format!("servers.json okunamadı; {} olarak yedeklendi ve boş sunucu listesiyle başlandı.", b.display()));
                }
                outcome.registry
            }
            Err(e) => {
                tracing::error!(error = %e, "sunucu kaydı okunamadı; boş ve salt-okunur listeyle başlanıyor");
                notices.push(format!(
                    "Sunucu listesi okunamadı ({e}). Dosyaya dokunulmadı; liste boş ve bu oturumda değişiklikler kaydedilmeyecek."
                ));
                Registry::unavailable(path)
            }
        };
        let store = store.unwrap_or_else(|e| {
            tracing::error!(error = %e, "kimlik deposu açılamadı; bellek deposu kullanılıyor");
            notices.push(NO_SECRET_STORE.to_string());
            Arc::new(MemorySecretStore::default())
        });
        let registry = Arc::new(RwLock::new(registry));
        let manager = Arc::new(ConnectionManager::new());
        Self {
            catalog: Catalog::new(manager.clone(), registry.clone()),
            credentials: Credentials::new(store),
            registry,
            manager,
            startup_notice: (!notices.is_empty()).then(|| notices.join(" ")),
        }
    }

    pub fn init() -> Self {
        #[cfg(windows)]
        let store = pgt_core::credentials::WindowsSecretStore::new().map(|s| Arc::new(s) as Arc<dyn SecretStore>);
        #[cfg(not(windows))]
        let store: pgt_core::Result<Arc<dyn SecretStore>> = Ok(Arc::new(MemorySecretStore::default()));
        Self::build(&config_dir(), store)
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

    #[test]
    fn startup_notice_after_corrupt_registry() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("servers.json"), "bozuk").unwrap();
        let state = AppState::with_store(dir.path(), Arc::new(MemorySecretStore::default()));
        let notice = state.startup_notice.clone().expect("uyarı bekleniyordu");
        assert!(notice.contains("servers.json.bak-"), "{notice}");
        assert!(state.registry.read().unwrap().servers().is_empty());
    }

    #[test]
    fn clean_start_has_no_notice() {
        let dir = tempfile::tempdir().unwrap();
        let state = AppState::with_store(dir.path(), Arc::new(MemorySecretStore::default()));
        assert!(state.startup_notice.is_none());
    }

    #[test]
    fn unreadable_registry_starts_empty_read_only_and_keeps_the_file() {
        let dir = tempfile::tempdir().unwrap();
        // Dizin olan servers.json okunamaz (NotFound ya da ayrıştırma hatası değil).
        std::fs::create_dir(dir.path().join("servers.json")).unwrap();
        std::fs::write(dir.path().join("servers.json").join("icerik"), "x").unwrap();
        let state = AppState::with_store(dir.path(), Arc::new(MemorySecretStore::default()));
        let notice = state.startup_notice.clone().expect("uyarı bekleniyordu");
        assert!(notice.contains("Sunucu listesi okunamadı"), "{notice}");
        let mut registry = state.registry.write().unwrap();
        assert!(registry.servers().is_empty());
        assert!(registry.is_read_only());
        assert!(registry.upsert(pgt_core::config::ServerConfig::new("a", "h", 5432, "u")).is_err());
        assert!(dir.path().join("servers.json").join("icerik").exists());
    }

    #[test]
    fn secret_store_failure_falls_back_to_memory_with_notice() {
        let dir = tempfile::tempdir().unwrap();
        let state = AppState::build(dir.path(), Err(pgt_core::Error::Credential("erişim reddedildi".into())));
        assert_eq!(state.startup_notice.as_deref(), Some(NO_SECRET_STORE));
        let id = uuid::Uuid::new_v4();
        state.credentials.remember(id, "p", true).unwrap();
        assert_eq!(state.credentials.password(id).unwrap().as_deref(), Some("p"));
    }
}
