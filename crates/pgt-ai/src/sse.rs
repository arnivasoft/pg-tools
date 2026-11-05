//! Server-Sent Events ayrıştırma ve Messages API akış olaylarının yorumlanması.

use serde_json::Value;

/// Parça parça gelen baytlardan tamamlanmış SSE olaylarının `data` alanlarını çıkarır.
/// Satır sonu `\n` ya da `\r\n`; olaylar boş satırla ayrılır; çok satırlı `data` `\n` ile birleşir.
#[derive(Default)]
pub struct SseParser {
    buf: Vec<u8>,
    data: Option<String>,
}

impl SseParser {
    pub fn feed(&mut self, chunk: &[u8]) -> Vec<String> {
        self.buf.extend_from_slice(chunk);
        let mut out = Vec::new();
        while let Some(pos) = self.buf.iter().position(|&b| b == b'\n') {
            let mut line: Vec<u8> = self.buf.drain(..=pos).collect();
            line.pop();
            if line.last() == Some(&b'\r') {
                line.pop();
            }
            let line = String::from_utf8_lossy(&line);
            if line.is_empty() {
                if let Some(d) = self.data.take() {
                    out.push(d);
                }
            } else if let Some(rest) = line.strip_prefix("data:") {
                let rest = rest.strip_prefix(' ').unwrap_or(rest);
                match &mut self.data {
                    Some(d) => {
                        d.push('\n');
                        d.push_str(rest);
                    }
                    None => self.data = Some(rest.to_string()),
                }
            }
            // `event:`, `id:`, `retry:` ve `:` yorumları yok sayılır: tür `data` içindeki `type` alanından okunur.
        }
        out
    }
}

/// Bir akış olayının anlamı.
#[derive(Debug, PartialEq)]
pub enum Interp {
    Text(String),
    /// Sunucu tarafı yedek modele geçiş (`fallback` içerik bloğu).
    Fallback,
    InputTokens(u32),
    Stop { reason: Option<String>, output_tokens: Option<u32> },
    End,
    Error(String),
    Ignore,
}

fn u32_at(v: &Value, ptr: &str) -> Option<u32> {
    v.pointer(ptr).and_then(Value::as_u64).map(|n| n.min(u32::MAX as u64) as u32)
}

pub fn interpret(data: &str) -> Interp {
    let Ok(v) = serde_json::from_str::<Value>(data) else { return Interp::Ignore };
    match v.get("type").and_then(Value::as_str) {
        Some("message_start") => u32_at(&v, "/message/usage/input_tokens").map_or(Interp::Ignore, Interp::InputTokens),
        Some("content_block_start") => match v.pointer("/content_block/type").and_then(Value::as_str) {
            Some("fallback") => Interp::Fallback,
            _ => Interp::Ignore,
        },
        Some("content_block_delta") => match v.pointer("/delta/type").and_then(Value::as_str) {
            Some("text_delta") => v
                .pointer("/delta/text")
                .and_then(Value::as_str)
                .map_or(Interp::Ignore, |t| Interp::Text(t.to_string())),
            _ => Interp::Ignore,
        },
        Some("message_delta") => Interp::Stop {
            reason: v.pointer("/delta/stop_reason").and_then(Value::as_str).map(str::to_string),
            output_tokens: u32_at(&v, "/usage/output_tokens"),
        },
        Some("message_stop") => Interp::End,
        Some("error") => Interp::Error(
            v.pointer("/error/message").and_then(Value::as_str).unwrap_or("bilinmeyen hata").to_string(),
        ),
        _ => Interp::Ignore,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_events_across_chunks_and_crlf() {
        let mut p = SseParser::default();
        assert!(p.feed(b"event: ping\r\ndata: {\"type\":").is_empty());
        let out = p.feed(b"\"ping\"}\r\n\r\nevent: x\ndata: a\ndata: b\n\n: yorum\n");
        assert_eq!(out, vec!["{\"type\":\"ping\"}".to_string(), "a\nb".to_string()]);
    }

    #[test]
    fn keeps_multibyte_text_split_between_chunks() {
        let mut p = SseParser::default();
        let line = "data: {\"type\":\"content_block_delta\",\"delta\":{\"type\":\"text_delta\",\"text\":\"ğüş\"}}\n\n";
        let bytes = line.as_bytes();
        let mid = line.find('ü').unwrap() + 1; // 'ü' iki baytlık: ortadan böl
        assert!(p.feed(&bytes[..mid]).is_empty());
        let out = p.feed(&bytes[mid..]);
        assert_eq!(interpret(&out[0]), Interp::Text("ğüş".into()));
    }

    #[test]
    fn interprets_message_events() {
        assert_eq!(
            interpret(r#"{"type":"message_start","message":{"usage":{"input_tokens":42}}}"#),
            Interp::InputTokens(42)
        );
        assert_eq!(
            interpret(r#"{"type":"content_block_start","index":1,"content_block":{"type":"fallback"}}"#),
            Interp::Fallback
        );
        assert_eq!(
            interpret(r#"{"type":"content_block_delta","index":0,"delta":{"type":"thinking_delta","thinking":"x"}}"#),
            Interp::Ignore
        );
        assert_eq!(
            interpret(r#"{"type":"message_delta","delta":{"stop_reason":"end_turn"},"usage":{"output_tokens":12}}"#),
            Interp::Stop { reason: Some("end_turn".into()), output_tokens: Some(12) }
        );
        assert_eq!(interpret(r#"{"type":"message_stop"}"#), Interp::End);
        assert_eq!(
            interpret(r#"{"type":"error","error":{"type":"overloaded_error","message":"Overloaded"}}"#),
            Interp::Error("Overloaded".into())
        );
        assert_eq!(interpret("not json"), Interp::Ignore);
    }
}
