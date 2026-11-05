//! Server-Sent Events ayrıştırma, Messages API akış olaylarının yorumlanması ve yanıt içeriğinin
//! (thinking blokları imzalarıyla birlikte) geri gönderilmek üzere biriktirilmesi.

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

/// Bir akış olayının arayüz için anlamı.
#[derive(Debug, PartialEq)]
pub enum Interp {
    Text(String),
    /// Sunucu tarafı yedek modele geçiş (`fallback` içerik bloğu). Gelen kısmi metin geçerli kalır.
    Fallback,
    InputTokens { input: u32, cache_read: u32 },
    Stop { reason: Option<String>, output_tokens: Option<u32> },
    End,
    Error(String),
    Ignore,
}

fn u32_at(v: &Value, ptr: &str) -> Option<u32> {
    v.pointer(ptr).and_then(Value::as_u64).map(|n| n.min(u32::MAX as u64) as u32)
}

pub fn interpret(v: &Value) -> Interp {
    match v.get("type").and_then(Value::as_str) {
        Some("message_start") => Interp::InputTokens {
            input: u32_at(v, "/message/usage/input_tokens").unwrap_or(0),
            cache_read: u32_at(v, "/message/usage/cache_read_input_tokens").unwrap_or(0),
        },
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
            output_tokens: u32_at(v, "/usage/output_tokens"),
        },
        Some("message_stop") => Interp::End,
        Some("error") => Interp::Error(
            v.pointer("/error/message").and_then(Value::as_str).unwrap_or("bilinmeyen hata").to_string(),
        ),
        _ => Interp::Ignore,
    }
}

/// Yanıtın içerik bloklarını akıştan yeniden kurar (sohbet geçmişine eklenmek üzere).
#[derive(Default)]
pub struct Accumulator {
    blocks: Vec<Value>,
    /// `tool_use` bloklarının parça parça gelen girdi metni (`input_json_delta`), blok sırasına göre.
    partial: std::collections::HashMap<usize, String>,
}

/// Biriktirilen yanıt.
pub struct Finished {
    pub content: Vec<Value>,
    /// Girdisi geçerli JSON nesnesi olmayan `tool_use` blokları: (kimlik, ham metin). Bu blokların `input`u `{}`
    /// yapılır; araç çalıştırılmaz, modele hata sonucu döner.
    pub invalid_inputs: Vec<(String, String)>,
}

fn append_str(block: &mut Value, field: &str, add: &str) {
    if let Some(obj) = block.as_object_mut() {
        let cur = obj.get(field).and_then(Value::as_str).unwrap_or("").to_string();
        obj.insert(field.into(), Value::String(cur + add));
    }
}

impl Accumulator {
    pub fn apply(&mut self, v: &Value) {
        let index = v.get("index").and_then(Value::as_u64).map(|i| i as usize);
        match (v.get("type").and_then(Value::as_str), index) {
            (Some("content_block_start"), Some(i)) => {
                let block = v.get("content_block").cloned().unwrap_or(Value::Null);
                if self.blocks.len() <= i {
                    self.blocks.resize(i + 1, Value::Null);
                }
                self.blocks[i] = block;
            }
            (Some("content_block_delta"), Some(i)) => {
                let Some(block) = self.blocks.get_mut(i) else { return };
                let delta = &v["delta"];
                let s = |k: &str| delta.get(k).and_then(Value::as_str).unwrap_or("");
                match delta.get("type").and_then(Value::as_str) {
                    Some("text_delta") => append_str(block, "text", s("text")),
                    Some("thinking_delta") => append_str(block, "thinking", s("thinking")),
                    Some("signature_delta") => append_str(block, "signature", s("signature")),
                    Some("input_json_delta") => self.partial.entry(i).or_default().push_str(s("partial_json")),
                    _ => {}
                }
            }
            _ => {}
        }
    }

    /// Geri gönderilecek asistan içeriği. Yanıt ortasında yedek modele geçildiyse son `fallback` bloğundan
    /// önceki thinking/redacted_thinking/tool_use ve tanınmayan model içi bloklar atlanır, metin blokları kalır;
    /// `fallback` bloğunun kendisi atılır. Boş metin blokları da atılır (API reddeder).
    /// `tool_use` girdileri katı biçimde ayrıştırılır (eager input streaming: API girdiyi doğrulamaz).
    pub fn finish(mut self) -> Finished {
        let mut invalid_inputs = Vec::new();
        for (i, block) in self.blocks.iter_mut().enumerate() {
            if block.get("type").and_then(Value::as_str) != Some("tool_use") {
                continue;
            }
            let raw = self.partial.remove(&i).unwrap_or_default();
            let parsed = if raw.trim().is_empty() {
                Some(Value::Object(Default::default()))
            } else {
                serde_json::from_str::<Value>(&raw).ok().filter(Value::is_object)
            };
            let input = parsed.unwrap_or_else(|| {
                let id = block.get("id").and_then(Value::as_str).unwrap_or("").to_string();
                invalid_inputs.push((id, raw));
                Value::Object(Default::default())
            });
            block["input"] = input;
        }
        Finished { content: self.echo_content(), invalid_inputs }
    }

    fn echo_content(self) -> Vec<Value> {
        let ty = |b: &Value| b.get("type").and_then(Value::as_str).map(str::to_string);
        let boundary = self.blocks.iter().rposition(|b| ty(b).as_deref() == Some("fallback"));
        self.blocks
            .into_iter()
            .enumerate()
            .filter(|(i, b)| {
                let t = ty(b);
                match t.as_deref() {
                    None | Some("fallback") => false,
                    Some("text") => b.get("text").and_then(Value::as_str).is_some_and(|s| !s.is_empty()),
                    Some(_) => boundary.is_none_or(|f| *i > f),
                }
            })
            .map(|(_, b)| b)
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn j(s: &str) -> Value {
        serde_json::from_str(s).unwrap()
    }

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
        assert_eq!(interpret(&j(&out[0])), Interp::Text("ğüş".into()));
    }

    #[test]
    fn interprets_message_events() {
        assert_eq!(
            interpret(&j(r#"{"type":"message_start","message":{"usage":{"input_tokens":42,"cache_read_input_tokens":7}}}"#)),
            Interp::InputTokens { input: 42, cache_read: 7 }
        );
        assert_eq!(
            interpret(&j(r#"{"type":"content_block_start","index":1,"content_block":{"type":"fallback"}}"#)),
            Interp::Fallback
        );
        assert_eq!(
            interpret(&j(r#"{"type":"content_block_delta","index":0,"delta":{"type":"thinking_delta","thinking":"x"}}"#)),
            Interp::Ignore
        );
        assert_eq!(
            interpret(&j(r#"{"type":"message_delta","delta":{"stop_reason":"end_turn"},"usage":{"output_tokens":12}}"#)),
            Interp::Stop { reason: Some("end_turn".into()), output_tokens: Some(12) }
        );
        assert_eq!(interpret(&j(r#"{"type":"message_stop"}"#)), Interp::End);
        assert_eq!(
            interpret(&j(r#"{"type":"error","error":{"type":"overloaded_error","message":"Overloaded"}}"#)),
            Interp::Error("Overloaded".into())
        );
    }

    fn feed(acc: &mut Accumulator, events: &[Value]) {
        for e in events {
            acc.apply(e);
        }
    }

    #[test]
    fn accumulates_thinking_with_signature_and_text() {
        let mut acc = Accumulator::default();
        feed(
            &mut acc,
            &[
                json!({"type":"content_block_start","index":0,"content_block":{"type":"thinking","thinking":"","signature":""}}),
                json!({"type":"content_block_delta","index":0,"delta":{"type":"thinking_delta","thinking":"ab"}}),
                json!({"type":"content_block_delta","index":0,"delta":{"type":"signature_delta","signature":"SIG"}}),
                json!({"type":"content_block_stop","index":0}),
                json!({"type":"content_block_start","index":1,"content_block":{"type":"text","text":""}}),
                json!({"type":"content_block_delta","index":1,"delta":{"type":"text_delta","text":"Mer"}}),
                json!({"type":"content_block_delta","index":1,"delta":{"type":"text_delta","text":"haba"}}),
            ],
        );
        assert_eq!(
            acc.finish().content,
            vec![json!({"type":"thinking","thinking":"ab","signature":"SIG"}), json!({"type":"text","text":"Merhaba"})]
        );
    }

    #[test]
    fn mid_output_fallback_drops_earlier_thinking_but_keeps_text() {
        let mut acc = Accumulator::default();
        feed(
            &mut acc,
            &[
                json!({"type":"content_block_start","index":0,"content_block":{"type":"thinking","thinking":"x","signature":"S1"}}),
                json!({"type":"content_block_start","index":1,"content_block":{"type":"text","text":"Kısmi "}}),
                json!({"type":"content_block_start","index":2,"content_block":{"type":"fallback","from":{},"to":{}}}),
                json!({"type":"content_block_start","index":3,"content_block":{"type":"thinking","thinking":"y","signature":"S2"}}),
                json!({"type":"content_block_start","index":4,"content_block":{"type":"text","text":"devam"}}),
                json!({"type":"content_block_start","index":5,"content_block":{"type":"text","text":""}}),
            ],
        );
        assert_eq!(
            acc.finish().content,
            vec![
                json!({"type":"text","text":"Kısmi "}),
                json!({"type":"thinking","thinking":"y","signature":"S2"}),
                json!({"type":"text","text":"devam"}),
            ]
        );
    }

    #[test]
    fn tool_use_input_is_parsed_strictly() {
        let mut acc = Accumulator::default();
        feed(
            &mut acc,
            &[
                json!({"type":"content_block_start","index":0,"content_block":{"type":"tool_use","id":"t1","name":"search_columns","input":{}}}),
                json!({"type":"content_block_delta","index":0,"delta":{"type":"input_json_delta","partial_json":"{\"name_con"}}),
                json!({"type":"content_block_delta","index":0,"delta":{"type":"input_json_delta","partial_json":"tains\": \"email\"}"}}),
                json!({"type":"content_block_start","index":1,"content_block":{"type":"tool_use","id":"t2","name":"list_relations","input":{}}}),
                json!({"type":"content_block_delta","index":1,"delta":{"type":"input_json_delta","partial_json":"{\"schema\": \"pub"}}),
            ],
        );
        let f = acc.finish();
        assert_eq!(f.content[0]["input"], json!({"name_contains": "email"}));
        assert_eq!(f.content[1]["input"], json!({}), "yarım girdi boş nesneye çevrilir");
        assert_eq!(f.invalid_inputs, vec![("t2".to_string(), "{\"schema\": \"pub".to_string())]);
    }
}
