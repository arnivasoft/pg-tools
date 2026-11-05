//! Asistan ayarları (`%APPDATA%\pg-tools\ai.json`): sağlayıcı, sağlayıcı başına model ve effort. Anahtarlar burada
//! değil, Windows Kimlik Bilgisi Yöneticisi'ndedir (sağlayıcı başına ayrı kayıt).

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// Asistanın konuştuğu uç nokta. İkisi de Anthropic Messages biçimini kullanır (OpenRouter `/api/v1/messages`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum AiProvider {
    /// Anthropic Claude API (`api.anthropic.com`).
    #[default]
    Anthropic,
    /// OpenRouter (`openrouter.ai`); model başka sağlayıcılara da yönlendirilebilir.
    OpenRouter,
}

impl AiProvider {
    /// Kimlik Bilgisi Yöneticisi'ndeki anahtar kaydının adı.
    pub fn key_name(self) -> &'static str {
        match self {
            AiProvider::Anthropic => "ai:anthropic",
            AiProvider::OpenRouter => "ai:openrouter",
        }
    }
}

/// `output_config.effort`. `None` (ayarlarda "Varsayılan"): alan gönderilmez, modelin varsayılanı geçerli.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "lowercase")]
#[ts(export)]
pub enum AiEffort {
    Low,
    Medium,
    High,
    Xhigh,
    Max,
}

impl AiEffort {
    pub fn as_str(self) -> &'static str {
        match self {
            AiEffort::Low => "low",
            AiEffort::Medium => "medium",
            AiEffort::High => "high",
            AiEffort::Xhigh => "xhigh",
            AiEffort::Max => "max",
        }
    }
}

pub const DEFAULT_ANTHROPIC_MODEL: &str = "claude-opus-5-5";
pub const DEFAULT_OPENROUTER_MODEL: &str = "anthropic/claude-opus-5.5";

fn d_anthropic() -> String {
    DEFAULT_ANTHROPIC_MODEL.into()
}
fn d_openrouter() -> String {
    DEFAULT_OPENROUTER_MODEL.into()
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct AiSettings {
    #[serde(default)]
    pub provider: AiProvider,
    /// Claude API model kimliği (ör. `claude-opus-5-5`).
    #[serde(default = "d_anthropic")]
    pub anthropic_model: String,
    /// OpenRouter model adı, kuruluş önekiyle (ör. `anthropic/claude-opus-5.5`).
    #[serde(default = "d_openrouter")]
    pub openrouter_model: String,
    #[serde(default)]
    pub effort: Option<AiEffort>,
}

impl Default for AiSettings {
    fn default() -> Self {
        Self { provider: AiProvider::Anthropic, anthropic_model: d_anthropic(), openrouter_model: d_openrouter(), effort: None }
    }
}

impl AiSettings {
    /// Etkin sağlayıcının modeli.
    pub fn model(&self) -> &str {
        match self.provider {
            AiProvider::Anthropic => &self.anthropic_model,
            AiProvider::OpenRouter => &self.openrouter_model,
        }
    }

    /// Model adları boş olamaz, boşluk içeremez, 200 karakteri aşamaz. `Err`: Türkçe açıklama.
    pub fn validate(&self) -> Result<(), String> {
        for (label, m) in [("Claude", &self.anthropic_model), ("OpenRouter", &self.openrouter_model)] {
            if m.is_empty() || m.chars().any(char::is_whitespace) || m.chars().count() > 200 {
                return Err(format!("{label} model adı geçersiz: boşluksuz bir model kimliği girin."));
            }
        }
        Ok(())
    }

    /// Okunamayan ya da bozuk dosyada varsayılanlar (asistan ayarı kritik değildir; sessizce sıfırlanır).
    pub fn load(path: &Path) -> Self {
        std::fs::read(path)
            .ok()
            .and_then(|b| serde_json::from_slice::<AiSettings>(&b).ok())
            .filter(|s| s.validate().is_ok())
            .unwrap_or_default()
    }

    /// Geçici dosyaya yazıp yeniden adlandırır (yarım yazılmış dosya kalmaz).
    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let tmp: PathBuf = path.with_extension("json.tmp");
        std::fs::write(&tmp, serde_json::to_vec_pretty(self).map_err(std::io::Error::other)?)?;
        std::fs::rename(&tmp, path)
    }
}

/// Arayüze dönen görünüm: ayarlar ve hangi sağlayıcının anahtarının kayıtlı olduğu (anahtarın kendisi asla dönmez).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct AiSettingsView {
    pub settings: AiSettings,
    pub has_anthropic_key: bool,
    pub has_openrouter_key: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_and_defaults() {
        let dir = std::env::temp_dir().join(format!("pgt-ai-settings-{}", std::process::id()));
        let path = dir.join("ai.json");
        let _ = std::fs::remove_file(&path);
        assert_eq!(AiSettings::load(&path), AiSettings::default());
        let s = AiSettings {
            provider: AiProvider::OpenRouter,
            anthropic_model: "claude-sonnet-5-5".into(),
            openrouter_model: "openai/gpt-5.6".into(),
            effort: Some(AiEffort::Xhigh),
        };
        s.save(&path).unwrap();
        assert_eq!(AiSettings::load(&path), s);
        assert_eq!(s.model(), "openai/gpt-5.6");
        // Eksik alanlar varsayılan; bozuk dosya varsayılana döner.
        std::fs::write(&path, r#"{"provider":"openRouter"}"#).unwrap();
        let p = AiSettings::load(&path);
        assert_eq!(p.provider, AiProvider::OpenRouter);
        assert_eq!(p.openrouter_model, DEFAULT_OPENROUTER_MODEL);
        std::fs::write(&path, "bozuk").unwrap();
        assert_eq!(AiSettings::load(&path), AiSettings::default());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn model_names_are_validated() {
        let mut s = AiSettings::default();
        assert!(s.validate().is_ok());
        s.openrouter_model = "  ".into();
        assert!(s.validate().is_err());
        s.openrouter_model = "anthropic/claude opus".into();
        assert!(s.validate().is_err());
    }

    #[test]
    fn effort_serializes_lowercase() {
        assert_eq!(serde_json::to_string(&AiEffort::Xhigh).unwrap(), "\"xhigh\"");
        assert_eq!(AiEffort::Max.as_str(), "max");
    }
}
