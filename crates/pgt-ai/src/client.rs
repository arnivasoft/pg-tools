//! Messages API istemcisi (ham HTTP + SSE; Rust için resmî SDK yok). İki uç nokta aynı biçimi kullanır:
//! Anthropic Claude API ve OpenRouter'ın Anthropic uyumlu `/api/v1/messages` uç noktası.
//! Anahtar ve istem/yanıt metni günlüğe yazılmaz; yalnız durum kodu, süre ve sağlayıcı.

use std::sync::Arc;
use std::time::{Duration, Instant};

use pgt_core::ErrorPayload;
use serde_json::{Value, json};
use tokio::sync::watch;

use crate::model::AiEvent;
use crate::prompt;
use crate::settings::{AiEffort, AiProvider, AiSettings};
use crate::sse::{Accumulator, Interp, SseParser, interpret};

const ANTHROPIC_ENDPOINT: &str = "https://api.anthropic.com/v1/messages";
const OPENROUTER_ENDPOINT: &str = "https://openrouter.ai/api/v1/messages";
const API_VERSION: &str = "2026-10-21";
/// Sunucu tarafı yedek model (ret kategorisine göre yönlendirilir). Yalnız Claude API'de ve destekleyen modellerde.
const FALLBACK_BETA: &str = "server-side-fallback-2026-07-01";
/// `fallbacks: "default"` kabul eden Claude API modelleri (diğerlerinde parametre gönderilmez).
const FALLBACK_MODELS: &[&str] = &["claude-fable-5-1", "claude-opus-5-5", "claude-opus-5", "claude-sonnet-5-5"];
const MAX_TOKENS: u32 = 16_000;

/// Bir isteğin sağlayıcı/model/effort seçimi (ayarlardan, istek başına anlık görüntü).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Profile {
    pub provider: AiProvider,
    pub model: String,
    pub effort: Option<AiEffort>,
}

impl Profile {
    pub fn from_settings(s: &AiSettings) -> Self {
        Self { provider: s.provider, model: s.model().to_string(), effort: s.effort }
    }

    /// Sohbet bağı: geçmiş (thinking blokları dahil) bu sağlayıcı + modelde üretildi; değişirse yeni sohbet gerekir.
    pub fn conversation_tag(&self) -> String {
        format!("{:?}|{}", self.provider, self.model)
    }

    fn endpoint(&self) -> &'static str {
        match self.provider {
            AiProvider::Anthropic => ANTHROPIC_ENDPOINT,
            AiProvider::OpenRouter => OPENROUTER_ENDPOINT,
        }
    }

    fn uses_fallbacks(&self) -> bool {
        self.provider == AiProvider::Anthropic && FALLBACK_MODELS.contains(&self.model.as_str())
    }

    /// Adaptive thinking: Claude modellerinde; OpenRouter'da yalnız `anthropic/` modellerinde (diğer modellerin
    /// akıl yürütmesi effort ile yönetilir, `thinking` alanı gönderilmez).
    fn adaptive_thinking(&self) -> bool {
        match self.provider {
            AiProvider::Anthropic => true,
            AiProvider::OpenRouter => self.model.starts_with("anthropic/"),
        }
    }

    /// Eager input streaming: Claude API'de belgeli; OpenRouter'da belgelenmediği için gönderilmez.
    fn eager_tools(&self) -> bool {
        self.provider == AiProvider::Anthropic
    }
}

fn err(code: &str, message: impl Into<String>, detail: Option<String>) -> ErrorPayload {
    ErrorPayload { code: code.into(), message: message.into(), detail, hint: None }
}

/// İptal belirteci: `cancel()` uçuştaki isteği keser.
#[derive(Clone)]
pub struct Cancel(Arc<watch::Sender<bool>>);

impl Default for Cancel {
    fn default() -> Self {
        Self(Arc::new(watch::channel(false).0))
    }
}

impl Cancel {
    pub fn cancel(&self) {
        self.0.send_replace(true);
    }
    pub fn is_cancelled(&self) -> bool {
        *self.0.borrow()
    }
    pub(crate) async fn cancelled(&self) {
        let mut rx = self.0.subscribe();
        let _ = rx.wait_for(|c| *c).await;
    }
}

pub struct AiClient {
    http: reqwest::Client,
}

impl AiClient {
    /// TLS: ring sağlayıcısı + işletim sistemi sertifika deposu (kurumsal kök sertifikalar dahil).
    pub fn new() -> Result<Self, ErrorPayload> {
        let provider = Arc::new(rustls::crypto::ring::default_provider());
        let tls_err = |e: &dyn std::fmt::Display| err("config", format!("TLS yapılandırılamadı: {e}"), None);
        let verifier = rustls_platform_verifier::Verifier::new(provider.clone()).map_err(|e| tls_err(&e))?;
        let tls = rustls::ClientConfig::builder_with_provider(provider)
            .with_safe_default_protocol_versions()
            .map_err(|e| tls_err(&e))?
            .dangerous()
            .with_custom_certificate_verifier(Arc::new(verifier))
            .with_no_client_auth();
        let http = reqwest::Client::builder()
            .tls_backend_preconfigured(tls)
            .connect_timeout(Duration::from_secs(15))
            // Parçalar arası en uzun bekleme; sunucu düşünürken de `ping` olayları gelir.
            .read_timeout(Duration::from_secs(120))
            .build()
            .map_err(|e| tls_err(&e))?;
        Ok(Self { http })
    }

    /// Tek bir Messages isteğini akışla gönderir; metin parçaları `emit` ile iletilir. `emit` `false` dönerse
    /// (alıcı gitti) istek iptal edilir. İptalde `None`. `allow_tools`: `false` ise `tool_choice: none`
    /// (araç kümesi yine gönderilir; küme değişirse thinking blokları ve önbellek bozulur).
    #[allow(clippy::too_many_arguments)]
    pub async fn stream(
        &self,
        profile: &Profile,
        api_key: &str,
        messages: &[Value],
        allow_tools: bool,
        cancel: &Cancel,
        emit: &mut crate::agent::Emit<'_>,
    ) -> Result<Option<Outcome>, ErrorPayload> {
        let body = request_body(profile, messages, allow_tools);
        let started = Instant::now();
        let mut req = self.http.post(profile.endpoint()).header("content-type", "application/json");
        req = match profile.provider {
            AiProvider::Anthropic => req.header("x-api-key", api_key).header("anthropic-version", API_VERSION),
            // OpenRouter: anahtar `Authorization: Bearer` ile (belgeli tek yöntem).
            AiProvider::OpenRouter => req.bearer_auth(api_key),
        };
        if profile.uses_fallbacks() {
            req = req.header("anthropic-beta", FALLBACK_BETA);
        }
        let send = req.body(body.to_string()).send();
        let mut resp = tokio::select! {
            r = send => r.map_err(|e| network_error(&e))?,
            () = cancel.cancelled() => return Ok(None),
        };
        let status = resp.status();
        tracing::info!(
            provider = ?profile.provider,
            status = status.as_u16(),
            ms = started.elapsed().as_millis() as u64,
            "asistan isteği yanıtladı"
        );
        if !status.is_success() {
            let text = tokio::select! {
                t = resp.text() => t.unwrap_or_default(),
                () = cancel.cancelled() => return Ok(None),
            };
            return Err(status_error(status.as_u16(), &text));
        }

        let mut parser = SseParser::default();
        let mut acc = Accumulator::default();
        let (mut usage, mut stop) = (Usage::default(), String::new());
        loop {
            let chunk = tokio::select! {
                c = resp.chunk() => c.map_err(|e| network_error(&e))?,
                () = cancel.cancelled() => return Ok(None),
            };
            let Some(chunk) = chunk else {
                return Err(err("network", "Asistan yanıtı yarıda kesildi.", None));
            };
            for data in parser.feed(&chunk) {
                let Ok(v) = serde_json::from_str::<Value>(&data) else { continue };
                acc.apply(&v);
                let event = match interpret(&v) {
                    Interp::Text(text) => AiEvent::Delta { text },
                    Interp::Fallback => AiEvent::Fallback,
                    Interp::InputTokens { input, cache_read } => {
                        usage.input_tokens = usage.input_tokens.saturating_add(input);
                        usage.cache_read_tokens = usage.cache_read_tokens.saturating_add(cache_read);
                        continue;
                    }
                    Interp::Stop { reason, output_tokens: n } => {
                        if let Some(r) = reason {
                            stop = r;
                        }
                        if let Some(n) = n {
                            usage.output_tokens = n;
                        }
                        continue;
                    }
                    Interp::End => {
                        tracing::info!(
                            input_tokens = usage.input_tokens,
                            cache_read = usage.cache_read_tokens,
                            output_tokens = usage.output_tokens,
                            stop = %stop,
                            "asistan isteği bitti"
                        );
                        let finished = acc.finish();
                        return Ok(Some(Outcome {
                            content: finished.content,
                            invalid_inputs: finished.invalid_inputs,
                            stop_reason: stop,
                            usage,
                        }));
                    }
                    Interp::Error(message) => return Err(err("ai", "Asistan hatası.", Some(message))),
                    Interp::Ignore => continue,
                };
                if !emit(event) {
                    cancel.cancel();
                    return Ok(None);
                }
            }
        }
    }
}

/// Tek isteğin belirteç kullanımı.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Usage {
    /// Önbellekten okunmayan girdi belirteçleri.
    pub input_tokens: u32,
    pub cache_read_tokens: u32,
    pub output_tokens: u32,
}

impl Usage {
    pub fn add(&mut self, o: Usage) {
        self.input_tokens = self.input_tokens.saturating_add(o.input_tokens);
        self.cache_read_tokens = self.cache_read_tokens.saturating_add(o.cache_read_tokens);
        self.output_tokens = self.output_tokens.saturating_add(o.output_tokens);
    }
}

/// Bir isteğin sonucu: geri gönderilecek asistan içeriği ve durma nedeni.
pub struct Outcome {
    pub content: Vec<Value>,
    /// Girdisi geçerli JSON olarak ayrıştırılamayan `tool_use` blokları: kimlik -> ham metin.
    pub invalid_inputs: Vec<(String, String)>,
    pub stop_reason: String,
    pub usage: Usage,
}

fn request_body(profile: &Profile, messages: &[Value], allow_tools: bool) -> Value {
    let mut body = json!({
        "model": profile.model,
        "max_tokens": MAX_TOKENS,
        "stream": true,
        "tools": crate::tools::definitions(profile.eager_tools()),
        // Zorunlu araç seçimi (`any`/`tool`) güncel Claude modellerinde 400 döner; yalnız `auto` ve `none`.
        "tool_choice": { "type": if allow_tools { "auto" } else { "none" } },
        "system": prompt::SYSTEM,
        // Otomatik önbellek: geçmiş (son önbelleklenebilir bloğa kadar) sonraki turda ucuz okunur.
        "cache_control": { "type": "ephemeral" },
        "messages": messages,
    });
    if profile.adaptive_thinking() {
        body["thinking"] = json!({ "type": "adaptive" });
    }
    // "Varsayılan" seçiliyse alan gönderilmez (modelin varsayılanı; Opus 5.5'te `medium`).
    if let Some(e) = profile.effort {
        body["output_config"] = json!({ "effort": e.as_str() });
    }
    if profile.uses_fallbacks() {
        body["fallbacks"] = json!("default");
    }
    body
}

fn network_error(e: &reqwest::Error) -> ErrorPayload {
    let message = if e.is_timeout() { "Asistan zaman aşımına uğradı." } else { "Yapay zekâ sağlayıcısına ulaşılamadı." };
    err("network", message, Some(e.to_string()))
}

fn status_error(status: u16, body: &str) -> ErrorPayload {
    let api_message = serde_json::from_str::<Value>(body)
        .ok()
        .and_then(|v| v.pointer("/error/message").and_then(Value::as_str).map(str::to_string));
    let (code, message) = match status {
        401 => ("ai_auth", "API anahtarı geçersiz. Asistan ayarlarından yeni bir anahtar girin."),
        402 => ("ai_billing", "Hesabınızda yeterli kredi yok (sağlayıcının faturalama sayfasını kontrol edin)."),
        403 => ("ai_auth", "Bu API anahtarının bu işleme yetkisi yok."),
        404 => ("ai", "Model bulunamadı: asistan ayarlarındaki model adını kontrol edin."),
        429 => ("ai_rate", "İstek sınırına ulaşıldı; biraz sonra yeniden deneyin."),
        500..=599 => ("ai", "Yapay zekâ sağlayıcısı şu an yanıt veremiyor; biraz sonra yeniden deneyin."),
        _ => ("ai", "Asistan isteği reddedildi."),
    };
    err(code, message, api_message.or_else(|| Some(format!("HTTP {status}"))))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn profile(provider: AiProvider, model: &str, effort: Option<AiEffort>) -> Profile {
        Profile { provider, model: model.into(), effort }
    }

    fn msgs() -> Vec<Value> {
        vec![json!({"role": "user", "content": "merhaba"})]
    }

    #[test]
    fn claude_body_uses_streaming_adaptive_thinking_and_default_fallback() {
        let p = profile(AiProvider::Anthropic, "claude-opus-5-5", Some(AiEffort::High));
        let b = request_body(&p, &msgs(), true);
        assert_eq!(b["cache_control"]["type"], "ephemeral");
        assert_eq!(b["tool_choice"]["type"], "auto");
        let last = request_body(&p, &msgs(), false);
        assert_eq!(last["tool_choice"]["type"], "none");
        assert_eq!(last["tools"], b["tools"], "araç kümesi her istekte aynı");
        assert_eq!(b["model"], "claude-opus-5-5");
        assert_eq!(b["stream"], true);
        assert_eq!(b["thinking"]["type"], "adaptive");
        assert_eq!(b["output_config"]["effort"], "high");
        assert_eq!(b["fallbacks"], "default");
        assert_eq!(b["tools"][0]["eager_input_streaming"], true);
        assert_eq!(b["messages"][0]["content"], "merhaba");
        assert!(b.get("temperature").is_none());
    }

    #[test]
    fn default_effort_and_unsupported_fallback_models_send_nothing() {
        let b = request_body(&profile(AiProvider::Anthropic, "claude-haiku-5-5", None), &msgs(), true);
        assert!(b.get("output_config").is_none(), "Varsayılan: alan gönderilmez");
        assert!(b.get("fallbacks").is_none(), "Haiku 5.5'te sunucu tarafı yedek yok");
        assert!(!profile(AiProvider::Anthropic, "claude-haiku-5-5", None).uses_fallbacks());
    }

    #[test]
    fn openrouter_body_omits_undocumented_fields() {
        let claude = request_body(&profile(AiProvider::OpenRouter, "anthropic/claude-opus-5.5", Some(AiEffort::Max)), &msgs(), true);
        assert_eq!(claude["model"], "anthropic/claude-opus-5.5");
        assert_eq!(claude["thinking"]["type"], "adaptive");
        assert_eq!(claude["output_config"]["effort"], "max");
        assert!(claude.get("fallbacks").is_none());
        assert!(claude["tools"][0].get("eager_input_streaming").is_none());
        let other = request_body(&profile(AiProvider::OpenRouter, "openai/gpt-5.6", Some(AiEffort::Low)), &msgs(), true);
        assert!(other.get("thinking").is_none(), "Claude dışı modelde thinking gönderilmez");
        assert_eq!(other["output_config"]["effort"], "low");
        assert_eq!(profile(AiProvider::OpenRouter, "openai/gpt-5.6", None).endpoint(), OPENROUTER_ENDPOINT);
    }

    #[test]
    fn conversation_tag_separates_provider_and_model() {
        let a = profile(AiProvider::Anthropic, "claude-opus-5-5", None).conversation_tag();
        let b = profile(AiProvider::OpenRouter, "claude-opus-5-5", None).conversation_tag();
        let c = profile(AiProvider::Anthropic, "claude-opus-5-5", Some(AiEffort::Max)).conversation_tag();
        assert_ne!(a, b);
        assert_eq!(a, c, "effort değişimi sohbeti bozmaz");
    }

    #[test]
    fn status_errors_are_classified_and_keep_api_message() {
        let e = status_error(401, r#"{"type":"error","error":{"type":"authentication_error","message":"invalid x-api-key"}}"#);
        assert_eq!(e.code, "ai_auth");
        assert_eq!(e.detail.as_deref(), Some("invalid x-api-key"));
        assert_eq!(status_error(529, "").code, "ai");
        assert_eq!(status_error(402, "").code, "ai_billing");
        assert_eq!(status_error(400, "").detail.as_deref(), Some("HTTP 400"));
    }

    #[tokio::test]
    async fn cancel_wakes_waiters() {
        let c = Cancel::default();
        let c2 = c.clone();
        let h = tokio::spawn(async move { c2.cancelled().await });
        c.cancel();
        h.await.unwrap();
        assert!(c.is_cancelled());
        c.cancelled().await; // zaten iptal: hemen döner
    }
}
