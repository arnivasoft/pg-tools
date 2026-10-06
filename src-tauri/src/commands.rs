use pgt_catalog::node::{NodeId, QueryTarget, TreeNode};
use pgt_catalog::props::PropertySection;
use std::sync::RwLock;

use pgt_core::config::ServerConfig;
use pgt_core::credentials::Credentials;
use pgt_core::connection::{ConnectionManager, ServerInfo};
use pgt_core::registry::Registry;
use pgt_dash::DashManager;
use pgt_query::session::QueryManager;
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
pub async fn save_server(state: State<'_, AppState>, config: ServerConfig, password: Option<String>) -> CmdResult<ServerConfig> {
    let typed = password.is_some();
    let (saved, prev) = Box::pin(save_registered(&state.registry, &state.manager, &state.query, &state.dash, config, typed)).await?;
    apply_saved_password(&state.credentials, prev.as_ref(), &saved, password)?;
    Ok(saved)
}

/// Kayıttan sonra parola: yazılan parola tercihe göre saklanır. "Parolayı kaydet" kapalıyken parola yazılmadıysa
/// ve hedef aynıysa yalnızca kalıcı kayıt silinir; oturum parolası kalır (etiket/ad/renk kaydı yeniden sormasın).
fn apply_saved_password(creds: &Credentials, prev: Option<&ServerConfig>, saved: &ServerConfig, password: Option<String>) -> Result<(), Error> {
    let forget = should_forget_stored(prev, saved, password.as_deref());
    match (password, saved.save_password) {
        (Some(p), true) => creds.remember(saved.id, &p, true),
        // Kaydetme kapalı ama parola yazıldı: yalnızca oturumda tutulur (kalıcı kayıt silinir); hemen yeniden sorulmaz.
        (Some(p), false) => creds.remember(saved.id, &p, false),
        (None, false) if !forget => creds.forget_stored(saved.id),
        (None, false) => creds.forget(saved.id),
        (None, true) if forget => creds.forget(saved.id),
        (None, true) => Ok(()),
    }
}

/// Kaydı yazar; yazma başarısızsa hiçbir şeye dokunmaz. Katalog bağlantısı hedef/SSL değişince ya da parola
/// yazıldıysa kesilir. Hedef, kilit, üretim etiketi değiştiyse ya da parola yazıldıysa sorgu oturumları eski
/// ayarlarla/parolayla açılmıştır: kapatılır (Plan 6 yeniden bağlanmayı sunar).
async fn save_registered(
    registry: &RwLock<Registry>,
    manager: &ConnectionManager,
    query: &QueryManager,
    dash: &DashManager,
    config: ServerConfig,
    password_typed: bool,
) -> Result<(ServerConfig, Option<ServerConfig>), Error> {
    let (saved, prev) = {
        let mut registry = registry.write().unwrap();
        let prev = registry.get(config.id).cloned();
        (registry.upsert(config)?, prev)
    };
    if should_disconnect_catalog(prev.as_ref(), &saved, password_typed) {
        manager.disconnect(saved.id);
        dash.close_server(saved.id);
    }
    if password_typed || should_close_query_sessions(prev.as_ref(), &saved) {
        query.close_sessions_of(saved.id).await;
    }
    Ok((saved, prev))
}

/// Önce sorgu oturumları (açık transaction'lar geri alınır), sonra dashboard oturumları, sonra katalog bağlantısı.
async fn disconnect_and_close(manager: &ConnectionManager, query: &QueryManager, dash: &DashManager, id: Uuid) {
    query.close_sessions_of(id).await;
    // Önce katalog bağlantısı: eşzamanlı `DashManager::open` bağlı olmadığını görüp kendini geri alır.
    manager.disconnect(id);
    dash.close_server(id);
}

/// Kayıt silinemezse (bilinmeyen sunucu, salt-okunur kayıt) hiçbir oturum kapatılmaz.
async fn delete_and_close(
    registry: &RwLock<Registry>,
    manager: &ConnectionManager,
    creds: &Credentials,
    query: &QueryManager,
    dash: &DashManager,
    id: Uuid,
) -> Result<(), Error> {
    delete_registered(registry, manager, creds, id)?;
    query.close_sessions_of(id).await;
    dash.close_server(id);
    Ok(())
}

/// A stored password may only be used for the same host/port/user/maintenance db.
fn same_target(a: &ServerConfig, b: &ServerConfig) -> bool {
    a.host == b.host && a.port == b.port && a.username == b.username && a.maintenance_db == b.maintenance_db
}

/// Katalog bağlantısı yalnızca bağlantı parametreleri (hedef, SSL) değişince kesilir: önbellekteki yapılandırma
/// yalnızca bağlanmak için kullanılır; kilit/üretim koruması kayıttan okunur ve sorgu oturumları ayrıca kapanır.
/// Yeni yazılmış parola da bağlantıyı yeniler: yönetici parolayı havuz/özel bağlantılar için önbellekte tutar.
fn should_disconnect_catalog(prev: Option<&ServerConfig>, new: &ServerConfig, password_typed: bool) -> bool {
    password_typed || prev.is_none_or(|p| !same_target(p, new) || p.ssl != new.ssl)
}

/// Kayıtlı sunucunun sorgu oturumlarının kapatılması gerekir mi: bağlantı hedefi (host/port/kullanıcı/bakım
/// veritabanı/SSL), salt okunur kilit ya da etiketin üretim olup olmaması değiştiyse. Ad/grup/renk
/// değişimi oturumları etkilemez.
fn should_close_query_sessions(prev: Option<&ServerConfig>, new: &ServerConfig) -> bool {
    prev.is_some_and(|p| {
        !same_target(p, new)
            || p.ssl != new.ssl
            || p.read_only_lock != new.read_only_lock
            || p.is_production() != new.is_production()
    })
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
    Ok(Box::pin(ConnectionManager::test(&config, effective.as_deref())).await?)
}

#[tauri::command]
pub async fn connect_server(state: State<'_, AppState>, server_id: Uuid, password: Option<String>, remember: bool) -> CmdResult<ServerInfo> {
    let config = state.registry.read().unwrap().get(server_id).cloned().ok_or(Error::UnknownServer(server_id))?;
    let effective = match &password {
        Some(p) => Some(p.clone()),
        None => state.credentials.password(server_id)?,
    };
    let info = Box::pin(state.manager.connect(config.clone(), effective)).await?;
    // Bağlantı sürerken sunucu silinmiş olabilir: oturumu kapat; kaydı ve parolayı geri getirme.
    if state.registry.read().unwrap().get(server_id).is_none() {
        state.manager.disconnect(server_id);
        return Err(Error::UnknownServer(server_id).into());
    }
    if let Some(p) = password {
        remember_after_connect(&state.registry, &state.credentials, &config, &p, remember);
    }
    Ok(info)
}

/// Başarılı bir bağlantıdan sonra yazılan parolayı saklar. Bağlantı zaten kuruldu: kalıcı kayıt
/// (servers.json ya da Kimlik Bilgisi Yöneticisi) başarısız olursa yalnızca uyarı günlüklenir;
/// parola her durumda oturum belleğinde kalır (Credentials::remember önce oturumu günceller).
///
/// Kayıt kilidi `creds.remember` bitene dek tutulur: `delete_registered` kaydı aynı kilit altında
/// siler ve parolayı ancak ondan sonra unutur. Böylece "kayıt var" denetimi ile parolanın yazılması
/// arasına bir silme giremez (girseydi silinen sunucunun parolası depoda yetim kalırdı).
fn remember_after_connect(registry: &RwLock<Registry>, creds: &Credentials, config: &ServerConfig, password: &str, remember: bool) {
    let mut persist = config.save_password;
    let mut registry = registry.write().unwrap();
    // Bağlantı sürerken silinen sunucu: upsert kaydı geri getirir, kalıcı parola yetim kalır.
    if registry.get(config.id).is_none() {
        return;
    }
    if remember && !config.save_password {
        let mut updated = config.clone();
        updated.save_password = true;
        match registry.upsert(updated) {
            Ok(_) => persist = true,
            Err(e) => tracing::warn!(server = %config.id, error = %e, "\"parolayı kaydet\" tercihi kaydedilemedi"),
        }
    }
    if let Err(e) = creds.remember(config.id, password, persist) {
        tracing::warn!(server = %config.id, error = %e, "parola kalıcı depoya yazılamadı; yalnızca bu oturumda kullanılacak");
    }
    drop(registry);
}

#[tauri::command]
pub async fn disconnect_server(state: State<'_, AppState>, server_id: Uuid) -> CmdResult<()> {
    Box::pin(disconnect_and_close(&state.manager, &state.query, &state.dash, server_id)).await;
    Ok(())
}

// Katalog gelecekleri büyüktür (DDL ~70 KB); kutulanmazsa invoke işleyicisinin ana iş parçacığı
// yığın çerçevesine gömülür ve release derlemesinde 1 MB'lık ana yığın taşar.
#[tauri::command]
pub async fn list_children(state: State<'_, AppState>, id: String) -> CmdResult<Vec<TreeNode>> {
    Ok(Box::pin(state.catalog.list_children(&NodeId::parse(&id)?)).await?)
}

#[tauri::command]
pub async fn get_properties(state: State<'_, AppState>, id: String) -> CmdResult<Vec<PropertySection>> {
    Ok(Box::pin(state.catalog.properties(&NodeId::parse(&id)?)).await?)
}

#[tauri::command]
pub async fn get_ddl(state: State<'_, AppState>, id: String) -> CmdResult<String> {
    Ok(Box::pin(state.catalog.ddl(&NodeId::parse(&id)?)).await?)
}

#[tauri::command]
pub fn startup_notice(state: State<'_, AppState>) -> Option<String> {
    state.startup_notice.clone()
}

fn delete_registered(registry: &RwLock<Registry>, manager: &ConnectionManager, creds: &Credentials, id: Uuid) -> Result<(), Error> {
    let removed = registry.write().unwrap().remove(id)?;
    if removed.is_none() {
        return Err(Error::UnknownServer(id));
    }
    manager.disconnect(id);
    creds.forget(id)
}

#[tauri::command]
pub async fn delete_server(state: State<'_, AppState>, server_id: Uuid) -> CmdResult<()> {
    Ok(Box::pin(delete_and_close(&state.registry, &state.manager, &state.credentials, &state.query, &state.dash, server_id)).await?)
}

#[tauri::command]
pub async fn preview_pgadmin_import(state: State<'_, AppState>, path: String) -> CmdResult<Vec<pgt_core::import::ImportItem>> {
    let text = pgt_core::import::read_file(std::path::Path::new(&path))?;
    let existing = state.registry.read().unwrap().servers().to_vec();
    Ok(pgt_core::import::preview(&text, &existing)?)
}

#[tauri::command]
pub async fn apply_pgadmin_import(state: State<'_, AppState>, path: String, indices: Vec<u32>) -> CmdResult<u32> {
    let text = pgt_core::import::read_file(std::path::Path::new(&path))?;
    Ok(pgt_core::import::apply(&mut state.registry.write().unwrap(), &text, &indices)?)
}

#[tauri::command]
pub fn set_show_system(state: State<'_, AppState>, value: bool) {
    state.catalog.set_show_system(value);
}

#[tauri::command]
pub async fn quote_identifiers(state: State<'_, AppState>, server_id: Uuid, names: Vec<String>) -> CmdResult<Vec<String>> {
    Ok(Box::pin(state.catalog.quote_identifiers(server_id, names)).await?)
}

/// "Query Tool'da aç": düğümün sunucusu ve veritabanı OID'i (arayüz düğüm kimliğini ayrıştırmaz).
fn target_of(node_id: &str) -> Result<QueryTarget, Error> {
    NodeId::parse(node_id)?
        .query_target()
        .ok_or_else(|| Error::Invalid(format!("düğüm bir sunucuya ait değil: {node_id}")))
}

#[tauri::command]
pub fn qt_target_of(node_id: String) -> CmdResult<QueryTarget> {
    Ok(target_of(&node_id)?)
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

    #[test]
    fn remember_after_connect_skips_deleted_server() {
        let dir = tempfile::tempdir().unwrap();
        let mut config = cfg(Uuid::new_v4());
        config.save_password = false;
        let registry = registry_with(dir.path(), &config);
        registry.write().unwrap().remove(config.id).unwrap();
        let store = Arc::new(MemorySecretStore::default());
        let c = Credentials::new(store.clone());
        remember_after_connect(&registry, &c, &config, "typed", true);
        assert!(registry.read().unwrap().get(config.id).is_none(), "silinen kayıt geri gelmemeli");
        let reloaded = Registry::load(dir.path().join("servers.json")).unwrap().registry;
        assert!(reloaded.get(config.id).is_none());
        use pgt_core::credentials::{SecretStore, password_key};
        assert_eq!(store.get(&password_key(config.id)).unwrap(), None);
        assert_eq!(c.password(config.id).unwrap(), None);
    }

    #[test]
    fn delete_server_removes_record_and_stored_password() {
        let dir = tempfile::tempdir().unwrap();
        let config = cfg(Uuid::new_v4());
        let registry = registry_with(dir.path(), &config);
        let store = Arc::new(MemorySecretStore::default());
        let c = Credentials::new(store.clone());
        c.remember(config.id, "pw", true).unwrap();
        let manager = ConnectionManager::new();
        delete_registered(&registry, &manager, &c, config.id).unwrap();
        assert!(registry.read().unwrap().get(config.id).is_none());
        assert_eq!(c.password(config.id).unwrap(), None);
    }

    #[test]
    fn target_of_returns_server_and_database_oid() {
        let u = Uuid::new_v4();
        let t = target_of(&format!("srv:{u}/db:16384/sch:2200/tables/tbl:24576")).unwrap();
        assert_eq!((t.server_id, t.database_oid), (u, Some(16384)));
        assert_eq!(target_of(&format!("srv:{u}")).unwrap().database_oid, None);
        assert!(matches!(target_of("grp:A"), Err(Error::Invalid(_))));
        assert!(matches!(target_of(""), Err(Error::Invalid(_))));
        assert!(target_of("srv:nope").is_err());
    }

    #[test]
    fn delete_unknown_server_is_unknown_server_error() {
        let dir = tempfile::tempdir().unwrap();
        let registry = registry_with(dir.path(), &cfg(Uuid::new_v4()));
        let err = delete_registered(&registry, &ConnectionManager::new(), &creds(), Uuid::new_v4()).unwrap_err();
        assert!(matches!(err, Error::UnknownServer(_)));
    }

    #[test]
    fn delete_server_on_read_only_registry_keeps_everything() {
        let dir = tempfile::tempdir().unwrap();
        let id = Uuid::new_v4();
        let registry = RwLock::new(Registry::unavailable(dir.path().join("servers.json")));
        let c = creds();
        c.remember(id, "pw", true).unwrap();
        let err = delete_registered(&registry, &ConnectionManager::new(), &c, id).unwrap_err();
        assert!(matches!(err, Error::Config(_)), "salt-okunur kayıt Config hatası vermeli: {err:?}");
        assert_eq!(c.password(id).unwrap().as_deref(), Some("pw"), "kayıt silinemediyse parola da silinmemeli");
    }

    /// Kalıcı depoya yazılırken kayıt kilidinin tutulduğunu doğrular: o sırada `delete_registered`
    /// kaydı silemez, yani denetim ile yazma arasına silme giremez.
    struct LockProbeStore {
        registry: Arc<RwLock<Registry>>,
        inner: MemorySecretStore,
        locked_during_set: std::sync::Mutex<Vec<bool>>,
    }
    impl pgt_core::credentials::SecretStore for LockProbeStore {
        fn get(&self, key: &str) -> pgt_core::Result<Option<String>> {
            self.inner.get(key)
        }
        fn set(&self, key: &str, value: &str) -> pgt_core::Result<()> {
            let locked = matches!(self.registry.try_write(), Err(std::sync::TryLockError::WouldBlock));
            self.locked_during_set.lock().unwrap().push(locked);
            self.inner.set(key, value)
        }
        fn delete(&self, key: &str) -> pgt_core::Result<()> {
            self.inner.delete(key)
        }
    }

    #[test]
    fn remember_after_connect_writes_the_password_under_the_registry_lock() {
        use pgt_core::credentials::SecretStore;
        let dir = tempfile::tempdir().unwrap();
        let config = cfg(Uuid::new_v4()); // save_password = true: kalıcı depoya yazılır
        let registry = Arc::new(registry_with(dir.path(), &config));
        let store = Arc::new(LockProbeStore { registry: registry.clone(), inner: MemorySecretStore::default(), locked_during_set: Default::default() });
        let c = Credentials::new(store.clone());
        remember_after_connect(&registry, &c, &config, "typed", false);
        assert_eq!(*store.locked_during_set.lock().unwrap(), [true], "parola kayıt kilidi altında yazılmalı");
        assert!(registry.try_write().is_ok(), "kilit sonunda bırakılmalı");
        // Ardından silme parolayı da kaldırır (yetim parola kalmaz).
        delete_registered(&registry, &ConnectionManager::new(), &c, config.id).unwrap();
        assert_eq!(c.password(config.id).unwrap(), None);
        assert_eq!(store.get(&pgt_core::credentials::password_key(config.id)).unwrap(), None);
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
            color: None,
            label: None,
            read_only_lock: false,
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
    fn catalog_disconnects_only_for_target_or_ssl_changes() {
        let id = Uuid::new_v4();
        let prev = cfg(id);
        assert!(should_disconnect_catalog(None, &prev, false));
        let meta: [fn(&mut ServerConfig); 5] = [
            |x| x.label = Some("Üretim".into()),
            |x| x.name = "yeni".into(),
            |x| x.color = Some("red".into()),
            |x| x.group = Some("g".into()),
            |x| x.read_only_lock = true,
        ];
        for f in meta {
            let mut new = prev.clone();
            f(&mut new);
            assert!(!should_disconnect_catalog(Some(&prev), &new, false));
            assert!(should_disconnect_catalog(Some(&prev), &new, true), "yazılmış parola her zaman yeniler");
        }
        let target: [fn(&mut ServerConfig); 5] = [
            |x| x.host = "other".into(),
            |x| x.port = 1,
            |x| x.username = "u2".into(),
            |x| x.maintenance_db = "d2".into(),
            |x| x.ssl.mode = pgt_core::config::SslMode::Require,
        ];
        for f in target {
            let mut new = prev.clone();
            f(&mut new);
            assert!(should_disconnect_catalog(Some(&prev), &new, false));
        }
    }

    #[test]
    fn label_only_save_keeps_the_session_password() {
        use pgt_core::credentials::{SecretStore, password_key};
        let store = Arc::new(MemorySecretStore::default());
        let c = Credentials::new(store.clone());
        let id = Uuid::new_v4();
        let mut prev = cfg(id);
        prev.save_password = false;
        c.remember(id, "oturum", false).unwrap();
        store.set(&password_key(id), "eski-kalici").unwrap();
        let mut labelled = prev.clone();
        labelled.label = Some("Test".into());
        apply_saved_password(&c, Some(&prev), &labelled, None).unwrap();
        assert_eq!(store.get(&password_key(id)).unwrap(), None, "kalıcı kayıt yine silinir");
        assert_eq!(c.password(id).unwrap().as_deref(), Some("oturum"), "oturum parolası kalır");

        // Hedef değişince oturum parolası da unutulur (eski hedefin parolası yeni hedefe gitmesin).
        let mut moved = labelled.clone();
        moved.host = "baska".into();
        apply_saved_password(&c, Some(&labelled), &moved, None).unwrap();
        assert_eq!(c.password(id).unwrap(), None);

        // save_password açıkken: yazılan parola saklanır; yazılmadıysa ve hedef aynıysa dokunulmaz.
        let mut keep = prev.clone();
        keep.save_password = true;
        apply_saved_password(&c, Some(&prev), &keep, Some("yeni".into())).unwrap();
        assert_eq!(store.get(&password_key(id)).unwrap().as_deref(), Some("yeni"));
        apply_saved_password(&c, Some(&keep), &keep, None).unwrap();
        assert_eq!(c.password(id).unwrap().as_deref(), Some("yeni"));
        // save_password kapalı + yazılmış parola: oturumda tutulur, kalıcı depoya yazılmaz (eskisi silinir).
        apply_saved_password(&c, Some(&keep), &prev, Some("x".into())).unwrap();
        assert_eq!(c.password(id).unwrap().as_deref(), Some("x"));
        assert_eq!(store.get(&password_key(id)).unwrap(), None);
    }

    #[test]
    fn close_sessions_decision() {
        let id = Uuid::new_v4();
        let prev = cfg(id);
        let changed: [fn(&mut ServerConfig); 8] = [
            |x| x.host = "other".into(),
            |x| x.port = 1,
            |x| x.username = "u2".into(),
            |x| x.maintenance_db = "d2".into(),
            |x| x.ssl.mode = pgt_core::config::SslMode::Require,
            |x| x.read_only_lock = true,
            |x| x.label = Some("PROD".into()),
            |x| x.ssl.root_cert = Some("ca.pem".into()),
        ];
        for f in changed {
            let mut new = cfg(id);
            f(&mut new);
            assert!(should_close_query_sessions(Some(&prev), &new));
        }
        // Üretim etiketinin kalkması da bir değişimdir; üretim olmayan etiket değişimi değildir.
        let mut prod = cfg(id);
        prod.label = Some("prod".into());
        assert!(should_close_query_sessions(Some(&prod), &prev));
        let mut a = cfg(id);
        a.label = Some("test".into());
        let mut b = cfg(id);
        b.label = Some("dev".into());
        assert!(!should_close_query_sessions(Some(&a), &b));
        // Yalnızca ad/grup/renk/parola saklama: kapatma yok.
        let mut meta = cfg(id);
        meta.name = "yeni ad".into();
        meta.group = Some("g".into());
        meta.color = Some("red".into());
        meta.save_password = false;
        assert!(!should_close_query_sessions(Some(&prev), &meta));
        assert!(!should_close_query_sessions(None, &prev), "yeni kayıtta oturum yok");
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

    // ---- Docker'lı yaşam döngüsü testleri: kayıt/bağlantı işlemleri sorgu oturumlarını gerçekten kapatır ----

    use crate::query::testkit::{Fx, fixture};
    use pgt_query::session::{SessionId, TxMode, TxState};

    /// Manuel modda açık transaction'lı (tablo oluşturulmuş) bir oturum + sunucu pid'i.
    async fn session_with_open_tx(fx: &Fx, table: &str) -> (SessionId, i32) {
        let id = fx.open(TxMode::Manual).await;
        fx.exec(id, &format!("create table {table}(i int)")).await;
        assert_eq!(fx.qm.tx_state(id).unwrap(), TxState::InTransaction);
        let pid = fx.pid(id).await;
        (id, pid)
    }

    async fn assert_closed_and_rolled_back(fx: &Fx, id: SessionId, pid: i32, table: &str) {
        assert!(fx.qm.tx_state(id).is_err(), "oturum kapanmış olmalı");
        assert!(fx.wait_gone(pid).await, "backend pg_stat_activity'den gitmeli");
        assert!(!fx.table_exists(table).await, "açık transaction geri alınmalı");
    }

    async fn assert_dash_closed(dash: &DashManager, id: pgt_dash::DashId) {
        let err = Box::pin(dash.sample(id)).await.map_err(ErrorPayload::from).unwrap_err();
        assert_eq!(err.code, "not_found", "dashboard oturumu kapanmış olmalı");
    }

    async fn assert_alive(fx: &Fx, id: SessionId, pid: i32) {
        assert_eq!(fx.qm.tx_state(id).unwrap(), TxState::InTransaction);
        assert!(fx.backend_state(pid).await.is_some());
    }

    #[tokio::test]
    async fn delete_server_closes_query_sessions() {
        let Some(fx) = fixture().await else { return };
        let (id, pid) = session_with_open_tx(&fx, "pgt_qt8_del").await;
        let dash = DashManager::new(fx.manager.clone());
        let info = Box::pin(dash.open(fx.ts.config.id, None)).await.unwrap();
        Box::pin(dash.sample(info.id)).await.unwrap();
        delete_and_close(&fx.registry, &fx.manager, &creds(), &fx.qm, &dash, fx.ts.config.id).await.unwrap();
        assert_dash_closed(&dash, info.id).await;
        assert!(fx.registry.read().unwrap().get(fx.ts.config.id).is_none());
        assert_closed_and_rolled_back(&fx, id, pid, "pgt_qt8_del").await;
    }

    #[tokio::test]
    async fn failed_registry_write_closes_nothing_on_delete_and_save() {
        let Some(fx) = fixture().await else { return };
        let (id, pid) = session_with_open_tx(&fx, "pgt_qt8_fail").await;
        // servers.json yerine dizin: sonraki kayıt (rename) başarısız olur.
        let file = fx.dir.path().join("servers.json");
        std::fs::remove_file(&file).unwrap();
        std::fs::create_dir(&file).unwrap();
        let sid = fx.ts.config.id;
        let dash = DashManager::new(fx.manager.clone());
        assert!(delete_and_close(&fx.registry, &fx.manager, &creds(), &fx.qm, &dash, sid).await.is_err());
        assert_alive(&fx, id, pid).await;
        let mut changed = fx.ts.config.clone();
        changed.read_only_lock = true;
        assert!(save_registered(&fx.registry, &fx.manager, &fx.qm, &dash, changed, false).await.is_err());
        assert_alive(&fx, id, pid).await;
        // Bilinmeyen sunucu silme de hiçbir oturumu kapatmaz.
        assert!(delete_and_close(&fx.registry, &fx.manager, &creds(), &fx.qm, &dash, Uuid::new_v4()).await.is_err());
        assert_alive(&fx, id, pid).await;
        fx.qm.close(id, pgt_query::session::ClosePolicy::Rollback).await.unwrap();
    }

    #[tokio::test]
    async fn disconnect_server_closes_query_sessions() {
        let Some(fx) = fixture().await else { return };
        let (id, pid) = session_with_open_tx(&fx, "pgt_qt8_disc").await;
        disconnect_and_close(&fx.manager, &fx.qm, &DashManager::new(fx.manager.clone()), fx.ts.config.id).await;
        assert_closed_and_rolled_back(&fx, id, pid, "pgt_qt8_disc").await;
    }

    #[tokio::test]
    async fn disconnect_server_closes_dash_sessions() {
        let Some(fx) = fixture().await else { return };
        let dash = DashManager::new(fx.manager.clone());
        let sid = fx.ts.config.id;
        let info = Box::pin(dash.open(sid, None)).await.unwrap();
        Box::pin(dash.sample(info.id)).await.unwrap();
        disconnect_and_close(&fx.manager, &fx.qm, &dash, sid).await;
        assert_dash_closed(&dash, info.id).await;
    }

    #[tokio::test]
    async fn save_server_closes_sessions_only_for_target_lock_or_production_changes() {
        let Some(fx) = fixture().await else { return };
        let dash = DashManager::new(fx.manager.clone());

        // Yalnızca ad/grup/renk: oturum açık kalır.
        let (id, pid) = session_with_open_tx(&fx, "pgt_qt8_save_meta").await;
        let mut meta = fx.ts.config.clone();
        meta.name = "yeni ad".into();
        meta.group = Some("grup".into());
        meta.color = Some("blue".into());
        meta.label = Some("Test".into());
        save_registered(&fx.registry, &fx.manager, &fx.qm, &dash, meta, false).await.unwrap();
        assert_alive(&fx, id, pid).await;
        assert!(fx.manager.is_connected(fx.ts.config.id), "etiket/ad/renk kaydı katalog bağlantısını kesmez");
        fx.qm.close(id, pgt_query::session::ClosePolicy::Rollback).await.unwrap();

        // Her bir değişim (kilit, üretim etiketi, hedef) oturumu kapatır ve transaction'ı geri alır.
        type Change = fn(&mut ServerConfig);
        let changes: [(&str, Change); 3] = [
            ("pgt_qt8_save_lock", |c| c.read_only_lock = true),
            ("pgt_qt8_save_prod", |c| c.label = Some("PROD".into())),
            ("pgt_qt8_save_port", |c| c.port = c.port.wrapping_add(1)),
        ];
        for (table, change) in changes {
            // Önceki turun değişimini sıfırla (oturumlar kapalıyken); bu da bir değişim olabilir, hâlâ oturum yok.
            let mut base = fx.ts.config.clone();
            base.name = "yeni ad".into();
            save_registered(&fx.registry, &fx.manager, &fx.qm, &dash, base.clone(), false).await.unwrap();
            fx.manager.connect(base.clone(), fx.ts.password.clone()).await.unwrap();
            let (id, pid) = session_with_open_tx(&fx, table).await;
            let dash_id = Box::pin(dash.open(fx.ts.config.id, None)).await.unwrap().id;
            let mut new = base;
            change(&mut new);
            save_registered(&fx.registry, &fx.manager, &fx.qm, &dash, new, false).await.unwrap();
            assert_closed_and_rolled_back(&fx, id, pid, table).await;
            // Dashboard oturumu yalnızca katalog bağlantısı kesilince (hedef değişimi) kapanır.
            if table == "pgt_qt8_save_port" {
                assert_dash_closed(&dash, dash_id).await;
            } else {
                Box::pin(dash.sample(dash_id)).await.unwrap();
                dash.close(dash_id);
            }
            // Kilit/üretim değişimi katalog bağlantısını korur; hedef (port) değişimi keser.
            assert_eq!(fx.manager.is_connected(fx.ts.config.id), table != "pgt_qt8_save_port", "{table}");
        }

        // Yeni yazılmış parola (ör. parola değişimi): hedef aynı olsa da katalog bağlantısı kesilir, oturumlar kapanır.
        let mut base = fx.ts.config.clone();
        base.name = "yeni ad".into();
        save_registered(&fx.registry, &fx.manager, &fx.qm, &dash, base.clone(), false).await.unwrap();
        fx.manager.connect(base.clone(), fx.ts.password.clone()).await.unwrap();
        let (id, pid) = session_with_open_tx(&fx, "pgt_qt8_save_pw").await;
        let dash_id = Box::pin(dash.open(fx.ts.config.id, None)).await.unwrap().id;
        save_registered(&fx.registry, &fx.manager, &fx.qm, &dash, base, true).await.unwrap();
        assert!(!fx.manager.is_connected(fx.ts.config.id), "yazılmış parola katalog bağlantısını yeniler");
        assert_closed_and_rolled_back(&fx, id, pid, "pgt_qt8_save_pw").await;
        assert_dash_closed(&dash, dash_id).await;
    }
}
