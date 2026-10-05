use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use uuid::Uuid;

use crate::{Error, Result};

/// Gizli değer deposu. Üretimde Windows Credential Manager, testlerde bellek.
pub trait SecretStore: Send + Sync {
    fn get(&self, key: &str) -> Result<Option<String>>;
    fn set(&self, key: &str, secret: &str) -> Result<()>;
    /// Kayıt yoksa da `Ok(())` döner.
    fn delete(&self, key: &str) -> Result<()>;
}

#[derive(Default)]
pub struct MemorySecretStore(Mutex<HashMap<String, String>>);

impl SecretStore for MemorySecretStore {
    fn get(&self, key: &str) -> Result<Option<String>> {
        Ok(self.0.lock().unwrap().get(key).cloned())
    }
    fn set(&self, key: &str, secret: &str) -> Result<()> {
        self.0.lock().unwrap().insert(key.to_string(), secret.to_string());
        Ok(())
    }
    fn delete(&self, key: &str) -> Result<()> {
        self.0.lock().unwrap().remove(key);
        Ok(())
    }
}

#[cfg(windows)]
const SERVICE: &str = "pg-tools";

/// Windows Credential Manager. Aynı kayda farklı thread'lerden eşzamanlı erişim
/// sıralanmadığı için tüm işlemler tek kilitle serileştirilir.
#[cfg(windows)]
pub struct WindowsSecretStore {
    lock: Mutex<()>,
}

#[cfg(windows)]
fn cred_err(e: keyring_core::Error) -> Error {
    Error::Credential(e.to_string())
}

#[cfg(windows)]
impl WindowsSecretStore {
    pub fn new() -> Result<Self> {
        let store = windows_native_keyring_store::Store::new().map_err(cred_err)?;
        keyring_core::set_default_store(store);
        Ok(Self { lock: Mutex::new(()) })
    }

    fn entry(key: &str) -> Result<keyring_core::Entry> {
        keyring_core::Entry::new(SERVICE, key).map_err(cred_err)
    }
}

#[cfg(windows)]
impl SecretStore for WindowsSecretStore {
    fn get(&self, key: &str) -> Result<Option<String>> {
        let _guard = self.lock.lock().unwrap();
        match Self::entry(key)?.get_password() {
            Ok(p) => Ok(Some(p)),
            Err(keyring_core::Error::NoEntry) => Ok(None),
            Err(e) => Err(cred_err(e)),
        }
    }
    fn set(&self, key: &str, secret: &str) -> Result<()> {
        let _guard = self.lock.lock().unwrap();
        Self::entry(key)?.set_password(secret).map_err(cred_err)
    }
    fn delete(&self, key: &str) -> Result<()> {
        let _guard = self.lock.lock().unwrap();
        match Self::entry(key)?.delete_credential() {
            Ok(()) | Err(keyring_core::Error::NoEntry) => Ok(()),
            Err(e) => Err(cred_err(e)),
        }
    }
}

pub fn password_key(server: Uuid) -> String {
    format!("server:{server}")
}

/// Kalıcı depo + oturum belleği. Oturum belleği önce sorgulanır.
pub struct Credentials {
    store: Arc<dyn SecretStore>,
    session: Mutex<HashMap<Uuid, String>>,
}

impl Credentials {
    pub fn new(store: Arc<dyn SecretStore>) -> Self {
        Self { store, session: Mutex::new(HashMap::new()) }
    }

    pub fn password(&self, server: Uuid) -> Result<Option<String>> {
        if let Some(p) = self.session.lock().unwrap().get(&server) {
            return Ok(Some(p.clone()));
        }
        self.store.get(&password_key(server))
    }

    /// Parolayı oturumda tutar; `persist` ise depoya yazar, değilse depodaki eski kaydı siler.
    pub fn remember(&self, server: Uuid, password: &str, persist: bool) -> Result<()> {
        self.session.lock().unwrap().insert(server, password.to_string());
        if persist {
            self.store.set(&password_key(server), password)
        } else {
            self.store.delete(&password_key(server))
        }
    }

    /// Yalnızca kalıcı depodaki kaydı siler; oturumdaki (bellekteki) parola kalır.
    pub fn forget_stored(&self, server: Uuid) -> Result<()> {
        self.store.delete(&password_key(server))
    }

    pub fn forget(&self, server: Uuid) -> Result<()> {
        self.session.lock().unwrap().remove(&server);
        self.store.delete(&password_key(server))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn creds() -> (Arc<MemorySecretStore>, Credentials) {
        let store = Arc::new(MemorySecretStore::default());
        (store.clone(), Credentials::new(store))
    }

    #[test]
    fn persisted_password_survives_new_session() {
        let (store, c) = creds();
        let id = Uuid::new_v4();
        c.remember(id, "s3cret", true).unwrap();
        assert_eq!(store.get(&password_key(id)).unwrap().as_deref(), Some("s3cret"));
        let fresh = Credentials::new(store);
        assert_eq!(fresh.password(id).unwrap().as_deref(), Some("s3cret"));
    }

    #[test]
    fn session_only_password_is_not_persisted_and_clears_old_one() {
        let (store, c) = creds();
        let id = Uuid::new_v4();
        c.remember(id, "eski", true).unwrap();
        c.remember(id, "yeni", false).unwrap();
        assert_eq!(c.password(id).unwrap().as_deref(), Some("yeni"));
        assert_eq!(store.get(&password_key(id)).unwrap(), None);
    }

    #[test]
    fn forget_stored_keeps_the_session_password() {
        let (store, c) = creds();
        let id = Uuid::new_v4();
        c.remember(id, "x", true).unwrap();
        c.forget_stored(id).unwrap();
        assert_eq!(store.get(&password_key(id)).unwrap(), None);
        assert_eq!(c.password(id).unwrap().as_deref(), Some("x"));
    }

    #[test]
    fn forget_removes_session_and_store() {
        let (store, c) = creds();
        let id = Uuid::new_v4();
        c.remember(id, "x", true).unwrap();
        c.forget(id).unwrap();
        assert_eq!(c.password(id).unwrap(), None);
        assert_eq!(store.get(&password_key(id)).unwrap(), None);
        c.forget(id).unwrap(); // olmayan kaydı silmek hata değildir
    }

    #[cfg(windows)]
    #[test]
    #[ignore = "gerçek Windows Credential Manager'a yazar; elle çalıştırın: cargo test -p pgt-core -- --ignored"]
    fn windows_store_round_trip() {
        let store = WindowsSecretStore::new().unwrap();
        let key = format!("test:{}", Uuid::new_v4());
        assert_eq!(store.get(&key).unwrap(), None);
        store.set(&key, "değer-ğüş").unwrap();
        assert_eq!(store.get(&key).unwrap().as_deref(), Some("değer-ğüş"));
        store.delete(&key).unwrap();
        assert_eq!(store.get(&key).unwrap(), None);
        store.delete(&key).unwrap();
    }
}
