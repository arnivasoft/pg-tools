use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// Asistan görevi.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum AiMode {
    /// Serbest istek ya da soru (gerekirse SQL yazar); sohbetin devam soruları da bu kipte.
    Generate,
    /// Verilen SQL'i açıkla.
    Explain,
    /// Hata veren SQL'i düzelt.
    Fix,
}

/// Arayüzün topladığı bağlam. Boş alanlar isteme eklenmez.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct AiRequest {
    pub mode: AiMode,
    /// Kullanıcının serbest metni (Generate için zorunlu, diğerlerinde ek not).
    pub prompt: String,
    /// Editördeki seçim ya da imleçteki ifade.
    pub sql: String,
    /// Son çalıştırmanın hata metni (Fix).
    pub error: String,
    /// Arama yolundaki ilişkiler ve kolonlarının kısa özeti.
    pub schema: String,
    /// Sunucu sürümü ("16.4" gibi); bilinmiyorsa boş.
    pub server_version: String,
}

/// Akış olayları (`Channel<AiEvent>`).
#[derive(Debug, Clone, PartialEq, Serialize, TS)]
#[serde(tag = "type", rename_all = "camelCase", rename_all_fields = "camelCase")]
#[ts(export)]
pub enum AiEvent {
    /// Yanıta eklenecek metin parçası.
    Delta { text: String },
    /// Sunucu isteği yedek modele devretti; gelen metin geçerli kalır ve yedek model kaldığı yerden sürer.
    Fallback,
    /// Akış bitti. `stopReason`: `end_turn`, `max_tokens`, `refusal`...
    Done { stop_reason: String, input_tokens: u32, output_tokens: u32 },
}
