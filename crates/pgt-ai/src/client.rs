//! Claude Messages API istemcisi (ham HTTP + SSE; Rust için resmî SDK yok).
//! Anahtar ve istem/yanıt metni günlüğe yazılmaz; yalnız durum kodu ve süre.

use std::sync::Arc;
use std::time::{Duration, Instant};

use pgt_core::ErrorPayload;
use serde_json::{Value, json};
use tokio::sync::watch;

use crate::model::AiEvent;
use crate::prompt;
use crate::sse::{Accumulator, Interp, SseParser, interpret};

const ENDPOINT: &str = "https://api.anthropic.com/v1/messages";
const API_VERSION: &str = "2026-10-21";
pub const MODEL: &str = "claude-opus-5-5";
/// Sunucu tarafı yedek model (ret kategorisine göre yönlendirilir).
const FALLBACK_BETA: &str = "server-side-fallback-2026-07-01";
const MAX_TOKENS: u32 = 16_000;

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
    pub async fn stream(
        &self,
        api_key: &str,
        messages: &[Value],
        allow_tools: bool,
        cancel: &Cancel,
        emit: &mut crate::agent::Emit<'_>,
    ) -> Result<Option<Outcome>, ErrorPayload> {
        let body = request_body(messages, allow_tools);
        let started = Instant::now();
        let send = self
            .http
            .post(ENDPOINT)
            .header("x-api-key", api_key)
            .header("anthropic-version", API_VERSION)
            .header("anthropic-beta", FALLBACK_BETA)
            .header("content-type", "application/json")
            .body(body.to_string())
            .send();
        let mut resp = tokio::select! {
            r = send => r.map_err(|e| network_error(&e))?,
            () = cancel.cancelled() => return Ok(None),
        };
        let status = resp.status();
        tracing::info!(status = status.as_u16(), ms = started.elapsed().as_millis() as u64, "asistan isteği yanıtladı");
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

fn request_body(messages: &[Value], allow_tools: bool) -> Value {
    json!({
        "model": MODEL,
        "max_tokens": MAX_TOKENS,
        "stream": true,
        "thinking": { "type": "adaptive" },
        // Opus 5.5'te varsayılan `medium`; açık yazılır.
        "output_config": { "effort": "medium" },
        "fallbacks": "default",
        "tools": crate::tools::definitions(),
        // Zorunlu araç seçimi (`any`/`tool`) Opus 5.5'te 400 döner; yalnız `auto` ve `none`.
        "tool_choice": { "type": if allow_tools { "auto" } else { "none" } },
        "system": prompt::SYSTEM,
        // Otomatik önbellek: geçmiş (son önbelleklenebilir bloğa kadar) sonraki turda ucuz okunur.
        "cache_control": { "type": "ephemeral" },
        "messages": messages,
    })
}

fn network_error(e: &reqwest::Error) -> ErrorPayload {
    let message = if e.is_timeout() { "Asistan zaman aşımına uğradı." } else { "Anthropic API'sine ulaşılamadı." };
    err("network", message, Some(e.to_string()))
}

fn status_error(status: u16, body: &str) -> ErrorPayload {
    let api_message = serde_json::from_str::<Value>(body)
        .ok()
        .and_then(|v| v.pointer("/error/message").and_then(Value::as_str).map(str::to_string));
    let (code, message) = match status {
        401 => ("ai_auth", "API anahtarı geçersiz. Asistan ayarlarından yeni bir anahtar girin."),
        403 => ("ai_auth", "Bu API anahtarının bu işleme yetkisi yok."),
        429 => ("ai_rate", "İstek sınırına ulaşıldı; biraz sonra yeniden deneyin."),
        500..=599 => ("ai", "Anthropic API'si şu an yanıt veremiyor; biraz sonra yeniden deneyin."),
        _ => ("ai", "Asistan isteği reddedildi."),
    };
    err(code, message, api_message.or_else(|| Some(format!("HTTP {status}"))))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn body_uses_streaming_adaptive_thinking_and_default_fallback() {
        let b = request_body(&[json!({"role": "user", "content": "merhaba"})], true);
        assert_eq!(b["cache_control"]["type"], "ephemeral");
        assert_eq!(b["tool_choice"]["type"], "auto");
        let last = request_body(&[json!({"role": "user", "content": "merhaba"})], false);
        assert_eq!(last["tool_choice"]["type"], "none");
        assert_eq!(last["tools"], b["tools"], "araç kümesi her istekte aynı");
        assert_eq!(b["model"], MODEL);
        assert_eq!(b["stream"], true);
        assert_eq!(b["thinking"]["type"], "adaptive");
        assert_eq!(b["fallbacks"], "default");
        assert_eq!(b["messages"][0]["content"], "merhaba");
        assert!(b.get("temperature").is_none());
    }

    #[test]
    fn status_errors_are_classified_and_keep_api_message() {
        let e = status_error(401, r#"{"type":"error","error":{"type":"authentication_error","message":"invalid x-api-key"}}"#);
        assert_eq!(e.code, "ai_auth");
        assert_eq!(e.detail.as_deref(), Some("invalid x-api-key"));
        assert_eq!(status_error(529, "").code, "ai");
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
