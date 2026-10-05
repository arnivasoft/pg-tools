use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::config::ServerConfig;
use crate::{Error, Result};

#[derive(Deserialize)]
struct RegistryFile {
    servers: Vec<ServerConfig>,
}

#[derive(Serialize)]
struct RegistryFileRef<'a> {
    version: u32,
    servers: &'a [ServerConfig],
}

pub struct Registry {
    path: PathBuf,
    servers: Vec<ServerConfig>,
    /// Dosya okunamadığında (izin, kilit vb.) boş ve salt-okunur açılır: okunamayan dosyanın
    /// üzerine boş bir listeyle yazıp kullanıcının sunucularını silmemek için kayıt reddedilir.
    read_only: bool,
}

pub struct LoadOutcome {
    pub registry: Registry,
    /// Dosya bozuk olduğu için yedeklendiyse yedeğin yolu.
    pub recovered_backup: Option<PathBuf>,
}

fn io_err(e: std::io::Error) -> Error {
    Error::Config(e.to_string())
}

impl Registry {
    pub fn load(path: impl Into<PathBuf>) -> Result<LoadOutcome> {
        let path = path.into();
        let text = match fs::read_to_string(&path) {
            Ok(t) => t,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                return Ok(LoadOutcome { registry: Registry { path, servers: Vec::new(), read_only: false }, recovered_backup: None });
            }
            Err(e) => return Err(Error::Config(format!("{}: {e}", path.display()))),
        };
        match serde_json::from_str::<RegistryFile>(&text) {
            Ok(file) => Ok(LoadOutcome { registry: Registry { path, servers: file.servers, read_only: false }, recovered_backup: None }),
            Err(parse_error) => {
                let backup = backup_path(&path);
                fs::rename(&path, &backup).map_err(io_err)?;
                tracing::warn!(error = %parse_error, backup = %backup.display(), "servers.json okunamadı, yedeklendi");
                Ok(LoadOutcome { registry: Registry { path, servers: Vec::new(), read_only: false }, recovered_backup: Some(backup) })
            }
        }
    }

    /// `path` okunamadığında kullanılacak boş, salt-okunur kayıt; dosyaya asla yazmaz.
    pub fn unavailable(path: impl Into<PathBuf>) -> Self {
        Registry { path: path.into(), servers: Vec::new(), read_only: true }
    }

    pub fn is_read_only(&self) -> bool {
        self.read_only
    }

    fn ensure_writable(&self) -> Result<()> {
        if self.read_only {
            return Err(Error::Config(format!(
                "{} başlangıçta okunamadığı için değişiklikler kaydedilemiyor; dosyayı düzeltip uygulamayı yeniden başlatın.",
                self.path.display()
            )));
        }
        Ok(())
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn servers(&self) -> &[ServerConfig] {
        &self.servers
    }

    pub fn get(&self, id: Uuid) -> Option<&ServerConfig> {
        self.servers.iter().find(|s| s.id == id)
    }

    pub fn upsert(&mut self, config: ServerConfig) -> Result<ServerConfig> {
        self.ensure_writable()?;
        let config = config.normalized();
        config.validate()?;
        // Kayıt başarısız olursa bellekteki değişiklik geri alınır (dosya ile bellek ayrışmasın).
        let previous = match self.servers.iter_mut().find(|s| s.id == config.id) {
            Some(slot) => Some(std::mem::replace(slot, config.clone())),
            None => {
                self.servers.push(config.clone());
                None
            }
        };
        if let Err(e) = self.save() {
            match previous {
                Some(prev) => {
                    if let Some(slot) = self.servers.iter_mut().find(|s| s.id == config.id) {
                        *slot = prev;
                    }
                }
                None => self.servers.retain(|s| s.id != config.id),
            }
            return Err(e);
        }
        Ok(config)
    }

    pub fn remove(&mut self, id: Uuid) -> Result<Option<ServerConfig>> {
        self.ensure_writable()?;
        let Some(index) = self.servers.iter().position(|s| s.id == id) else {
            return Ok(None);
        };
        let removed = self.servers.remove(index);
        if let Err(e) = self.save() {
            self.servers.insert(index, removed);
            return Err(e);
        }
        Ok(Some(removed))
    }

    fn save(&self) -> Result<()> {
        if let Some(dir) = self.path.parent() {
            fs::create_dir_all(dir).map_err(io_err)?;
        }
        let json = serde_json::to_string_pretty(&RegistryFileRef { version: 1, servers: &self.servers })
            .map_err(|e| Error::Config(e.to_string()))?;
        let tmp = self.path.with_extension("json.tmp");
        fs::write(&tmp, json).map_err(io_err)?;
        fs::rename(&tmp, &self.path).map_err(io_err)?;
        Ok(())
    }
}

fn backup_path(path: &Path) -> PathBuf {
    let secs = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("servers.json");
    path.with_file_name(format!("{name}.bak-{secs}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::ServerConfig;

    fn path(dir: &tempfile::TempDir) -> std::path::PathBuf {
        dir.path().join("pg-tools").join("servers.json")
    }

    #[test]
    fn load_missing_file_gives_empty() {
        let dir = tempfile::tempdir().unwrap();
        let out = Registry::load(path(&dir)).unwrap();
        assert!(out.registry.servers().is_empty());
        assert!(out.recovered_backup.is_none());
    }

    #[test]
    fn upsert_persists_and_reloads() {
        let dir = tempfile::tempdir().unwrap();
        let mut reg = Registry::load(path(&dir)).unwrap().registry;
        let saved = reg.upsert(ServerConfig::new(" prod ", "h", 5432, "u")).unwrap();
        assert_eq!(saved.name, "prod");
        let again = Registry::load(path(&dir)).unwrap().registry;
        assert_eq!(again.servers(), &[saved.clone()]);
        assert_eq!(again.get(saved.id), Some(&saved));
        assert!(!path(&dir).with_extension("json.tmp").exists());
    }

    #[test]
    fn upsert_replaces_existing_by_id() {
        let dir = tempfile::tempdir().unwrap();
        let mut reg = Registry::load(path(&dir)).unwrap().registry;
        let mut c = reg.upsert(ServerConfig::new("a", "h", 5432, "u")).unwrap();
        c.name = "b".into();
        reg.upsert(c.clone()).unwrap();
        assert_eq!(reg.servers().len(), 1);
        assert_eq!(reg.servers()[0].name, "b");
    }

    #[test]
    fn upsert_rejects_invalid_and_does_not_save() {
        let dir = tempfile::tempdir().unwrap();
        let mut reg = Registry::load(path(&dir)).unwrap().registry;
        assert!(reg.upsert(ServerConfig::new("", "h", 5432, "u")).is_err());
        assert!(!path(&dir).exists());
    }

    #[test]
    fn remove_deletes_and_persists() {
        let dir = tempfile::tempdir().unwrap();
        let mut reg = Registry::load(path(&dir)).unwrap().registry;
        let c = reg.upsert(ServerConfig::new("a", "h", 5432, "u")).unwrap();
        assert_eq!(reg.remove(c.id).unwrap().map(|s| s.id), Some(c.id));
        assert!(reg.remove(c.id).unwrap().is_none());
        assert!(Registry::load(path(&dir)).unwrap().registry.servers().is_empty());
    }

    #[test]
    fn failed_save_rolls_back_memory() {
        let dir = tempfile::tempdir().unwrap();
        let p = path(&dir);
        let mut reg = Registry::load(&p).unwrap().registry;
        let a = reg.upsert(ServerConfig::new("a", "h", 5432, "u")).unwrap();
        let b = reg.upsert(ServerConfig::new("b", "h", 5432, "u")).unwrap();
        // servers.json'ın yerine bir dizin koy: sonraki kayıt (rename) başarısız olur.
        std::fs::remove_file(&p).unwrap();
        std::fs::create_dir(&p).unwrap();

        assert!(reg.remove(a.id).is_err());
        assert_eq!(reg.servers(), &[a.clone(), b.clone()], "silme geri alınmalı, sıra korunmalı");

        let mut renamed = a.clone();
        renamed.name = "yeni".into();
        assert!(reg.upsert(renamed).is_err());
        assert_eq!(reg.get(a.id), Some(&a), "güncelleme geri alınmalı");

        assert!(reg.upsert(ServerConfig::new("c", "h", 5432, "u")).is_err());
        assert_eq!(reg.servers(), &[a, b], "eklenen kayıt geri alınmalı");
    }

    #[test]
    fn unavailable_registry_never_writes() {
        let dir = tempfile::tempdir().unwrap();
        let p = path(&dir);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(&p, "korunmalı").unwrap();
        let mut reg = Registry::unavailable(&p);
        assert!(reg.is_read_only());
        let c = ServerConfig::new("a", "h", 5432, "u");
        let id = c.id;
        assert!(matches!(reg.upsert(c), Err(Error::Config(_))));
        assert!(reg.servers().is_empty(), "başarısız kayıt belleği de değiştirmemeli");
        assert!(matches!(reg.remove(id), Err(Error::Config(_))));
        assert_eq!(std::fs::read_to_string(&p).unwrap(), "korunmalı");
    }

    #[test]
    fn corrupt_file_is_backed_up() {
        let dir = tempfile::tempdir().unwrap();
        let p = path(&dir);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(&p, "{ bozuk json").unwrap();
        let out = Registry::load(&p).unwrap();
        assert!(out.registry.servers().is_empty());
        let backup = out.recovered_backup.expect("yedek yolu");
        assert!(backup.file_name().unwrap().to_str().unwrap().starts_with("servers.json.bak-"));
        assert_eq!(std::fs::read_to_string(backup).unwrap(), "{ bozuk json");
        assert!(!p.exists());
    }
}
