use serde::{Deserialize, Serialize};
use ts_rs::TS;
use uuid::Uuid;

use crate::{Error, Result};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "kebab-case")]
#[ts(export)]
pub enum SslMode {
    Disable,
    #[default]
    Prefer,
    Require,
    VerifyCa,
    VerifyFull,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct SslConfig {
    pub mode: SslMode,
    pub root_cert: Option<String>,
    pub client_cert: Option<String>,
    pub client_key: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ServerConfig {
    pub id: Uuid,
    pub name: String,
    pub group: Option<String>,
    pub host: String,
    pub port: u16,
    pub maintenance_db: String,
    pub username: String,
    pub save_password: bool,
    #[serde(default)]
    pub ssl: SslConfig,
}

fn clean(value: Option<String>) -> Option<String> {
    value.map(|v| v.trim().to_string()).filter(|v| !v.is_empty())
}

impl ServerConfig {
    pub fn new(name: impl Into<String>, host: impl Into<String>, port: u16, username: impl Into<String>) -> Self {
        Self {
            id: Uuid::new_v4(),
            name: name.into(),
            group: None,
            host: host.into(),
            port,
            maintenance_db: "postgres".into(),
            username: username.into(),
            save_password: false,
            ssl: SslConfig::default(),
        }
    }

    /// Boşlukları kırpar, boş isteğe bağlı alanları `None` yapar.
    pub fn normalized(mut self) -> Self {
        self.name = self.name.trim().to_string();
        self.host = self.host.trim().to_string();
        self.username = self.username.trim().to_string();
        self.maintenance_db = self.maintenance_db.trim().to_string();
        self.group = clean(self.group);
        self.ssl.root_cert = clean(self.ssl.root_cert);
        self.ssl.client_cert = clean(self.ssl.client_cert);
        self.ssl.client_key = clean(self.ssl.client_key);
        self
    }

    pub fn validate(&self) -> Result<()> {
        let invalid = |m: &str| Err(Error::Invalid(m.to_string()));
        if self.name.trim().is_empty() {
            return invalid("Sunucu adı boş olamaz.");
        }
        if self.host.trim().is_empty() {
            return invalid("Host boş olamaz.");
        }
        if self.port == 0 {
            return invalid("Port 1-65535 arasında olmalı.");
        }
        if self.username.trim().is_empty() {
            return invalid("Kullanıcı adı boş olamaz.");
        }
        if self.maintenance_db.trim().is_empty() {
            return invalid("Bakım veritabanı boş olamaz.");
        }
        if self.ssl.client_cert.is_some() != self.ssl.client_key.is_some() {
            return invalid("İstemci sertifikası ve anahtarı birlikte verilmelidir.");
        }
        Ok(())
    }

    pub fn group_name(&self) -> Option<&str> {
        self.group.as_deref().map(str::trim).filter(|g| !g.is_empty())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_has_sane_defaults() {
        let c = ServerConfig::new("prod", "db.local", 5432, "app");
        assert_eq!(c.maintenance_db, "postgres");
        assert_eq!(c.ssl.mode, SslMode::Prefer);
        assert!(!c.save_password);
        assert!(c.group.is_none());
    }

    #[test]
    fn normalized_trims_and_drops_empty_optionals() {
        let mut c = ServerConfig::new("  prod ", " db.local ", 5432, " app ");
        c.group = Some("   ".into());
        c.ssl.root_cert = Some("".into());
        c.ssl.client_cert = Some(r" C:\certs\c.pem ".into());
        let n = c.normalized();
        assert_eq!((n.name.as_str(), n.host.as_str(), n.username.as_str()), ("prod", "db.local", "app"));
        assert_eq!(n.group, None);
        assert_eq!(n.ssl.root_cert, None);
        assert_eq!(n.ssl.client_cert.as_deref(), Some(r"C:\certs\c.pem"));
    }

    #[test]
    fn validate_rejects_missing_fields() {
        let ok = ServerConfig::new("prod", "h", 5432, "u");
        assert!(ok.validate().is_ok());
        let mut c = ok.clone();
        c.name = "".into();
        assert!(matches!(c.validate(), Err(Error::Invalid(m)) if m.contains("Sunucu adı")));
        let mut c = ok.clone();
        c.port = 0;
        assert!(matches!(c.validate(), Err(Error::Invalid(m)) if m.contains("Port")));
        let mut c = ok.clone();
        c.ssl.client_cert = Some("c.pem".into());
        assert!(matches!(c.validate(), Err(Error::Invalid(m)) if m.contains("birlikte")));
    }

    #[test]
    fn json_uses_camel_case_and_kebab_ssl_mode() {
        let mut c = ServerConfig::new("p", "h", 5432, "u");
        c.ssl.mode = SslMode::VerifyFull;
        let v = serde_json::to_value(&c).unwrap();
        assert_eq!(v["maintenanceDb"], "postgres");
        assert_eq!(v["savePassword"], false);
        assert_eq!(v["ssl"]["mode"], "verify-full");
    }
}
