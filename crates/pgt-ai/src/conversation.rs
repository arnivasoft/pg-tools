//! Sohbet geçmişi (bellekte, kalıcı değil). Geçmiş yalnız eklenerek büyür: önceki mesajlar ve asistan
//! içerik blokları (thinking imzaları dahil) hiç değiştirilmeden geri gönderilir — böylece hem thinking
//! blokları geçerli kalır hem istem önbelleği isabet eder. Başarısız/iptal edilen tur geçmişe girmez.

use std::collections::HashMap;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};

use pgt_core::ErrorPayload;
use serde_json::{Value, json};
use uuid::Uuid;

use crate::model::AiRequest;
use crate::prompt;

/// Bellekte tutulan en çok sohbet; aşılınca en uzun süredir kullanılmayan düşer.
pub const MAX_CONVERSATIONS: usize = 20;
/// Bir sohbette en çok soru-cevap turu.
pub const MAX_TURNS: u32 = 40;
/// Sohbet geçmişinin en çok karakteri (bağlam penceresinin güvenli altında).
pub const MAX_CHARS: usize = 1_200_000;

fn err(code: &str, message: impl Into<String>) -> ErrorPayload {
    ErrorPayload { code: code.into(), message: message.into(), detail: None, hint: None }
}

#[derive(Default)]
struct Conversation {
    messages: Vec<Value>,
    last_schema: Option<String>,
    /// Tamamlanmış tur sayısı (bir tur araç sonuçlarıyla birden çok mesaj olabilir).
    turns: u32,
    chars: usize,
    busy: bool,
    /// Son kullanım sırası (eşit zaman damgası sorunu olmasın diye sayaç).
    last_used: u64,
    /// Geçmişi üreten sağlayıcı + model (`Profile::conversation_tag`); ilk başarılı turda bağlanır.
    tag: Option<String>,
}

/// Gönderilmeye hazır tur: tam mesaj dizisi ve başarıda geçmişe eklenecek kullanıcı mesajı.
pub struct Pending {
    pub conversation: Uuid,
    pub messages: Vec<Value>,
    user: Value,
    schema: Option<String>,
    chars: usize,
}

#[derive(Default)]
pub struct Conversations {
    map: Mutex<HashMap<Uuid, Conversation>>,
    tick: AtomicU64,
}

impl Conversations {
    /// Yeni turu hazırlar ve sohbeti meşgul işaretler. `turns`: arayüzün bildiği tamamlanmış tur sayısı;
    /// arka uçtaki geçmişle uyuşmazsa (sohbet bellekten düştü) `ai_stale`. `tag`: sağlayıcı + model; geçmiş başka
    /// bir sağlayıcı/modelde üretildiyse `ai_stale` (thinking blokları ve araç geçmişi başka modele taşınmaz).
    pub fn begin(&self, id: Uuid, turns: u32, tag: &str, req: &AiRequest) -> Result<Pending, ErrorPayload> {
        let mut map = self.map.lock().unwrap();
        if !map.contains_key(&id) {
            if turns > 0 {
                return Err(err("ai_stale", "Sohbet geçmişi artık bellekte yok; yeni sohbet başlatın."));
            }
            evict(&mut map);
        }
        let conv = map.entry(id).or_default();
        if conv.busy {
            return Err(err("busy", "Bu sohbette yanıt bekleniyor."));
        }
        let have = conv.turns;
        if have != turns {
            return Err(err("ai_stale", "Sohbet geçmişi arayüzle uyuşmuyor; yeni sohbet başlatın."));
        }
        if have > 0 && conv.tag.as_deref() != Some(tag) {
            return Err(err("ai_stale", "Sağlayıcı ya da model değişti; bu sohbet önceki modelle sürdü. Yeni sohbet başlatın."));
        }
        conv.tag = Some(tag.to_string());
        if have >= MAX_TURNS {
            return Err(err("ai_long", "Sohbet çok uzadı; yeni sohbet başlatın."));
        }
        let schema = req.schema.trim().to_string();
        let include_schema = conv.last_schema.as_deref() != Some(schema.as_str());
        let text = prompt::user_message(req, include_schema).map_err(|m| err("invalid", m))?;
        let chars = text.chars().count();
        if conv.chars + chars > MAX_CHARS {
            return Err(err("ai_long", "Sohbet çok uzadı; yeni sohbet başlatın."));
        }
        let user = json!({ "role": "user", "content": text });
        let mut messages = conv.messages.clone();
        messages.push(user.clone());
        conv.busy = true;
        conv.last_used = self.tick.fetch_add(1, Ordering::Relaxed) + 1;
        Ok(Pending { conversation: id, messages, user, schema: include_schema.then_some(schema), chars })
    }

    /// Turu bitirir. `content` varsa (başarılı yanıt) kullanıcı ve asistan mesajları geçmişe eklenir.
    /// Turu bitirir. `added` varsa (başarılı tur: asistan mesajları ve araç sonuçları, `agent::run_turn`)
    /// kullanıcı mesajıyla birlikte geçmişe eklenir.
    pub fn finish(&self, p: Pending, added: Option<Vec<Value>>) {
        let mut map = self.map.lock().unwrap();
        let Some(conv) = map.get_mut(&p.conversation) else { return };
        conv.busy = false;
        // Arayüzle aynı kural (`isCommitted`): asistanın görünür metni olmayan tur geçmişe girmez.
        let has_text = |msgs: &Vec<Value>| {
            msgs.iter().filter(|m| m["role"] == "assistant").any(|m| {
                m["content"].as_array().is_some_and(|c| {
                    c.iter().any(|b| b["type"] == "text" && b["text"].as_str().is_some_and(|s| !s.trim().is_empty()))
                })
            })
        };
        let Some(added) = added.filter(has_text) else { return };
        let added_chars: usize = added
            .iter()
            .flat_map(|m| m["content"].as_array().cloned().unwrap_or_default())
            .map(|b| {
                ["text", "thinking", "content"]
                    .iter()
                    .filter_map(|k| b.get(*k)?.as_str())
                    .map(|s| s.chars().count())
                    .sum::<usize>()
            })
            .sum();
        conv.messages.push(p.user);
        conv.messages.extend(added);
        conv.turns += 1;
        conv.chars += p.chars + added_chars;
        if let Some(s) = p.schema {
            conv.last_schema = Some(s);
        }
    }

    pub fn forget(&self, id: Uuid) {
        self.map.lock().unwrap().remove(&id);
    }

    pub fn clear(&self) {
        self.map.lock().unwrap().clear();
    }
}

fn evict(map: &mut HashMap<Uuid, Conversation>) {
    while map.len() >= MAX_CONVERSATIONS {
        let oldest = map.iter().filter(|(_, c)| !c.busy).min_by_key(|(_, c)| c.last_used).map(|(k, _)| *k);
        match oldest {
            Some(k) => map.remove(&k),
            None => return,
        };
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::AiMode;

    fn ask(prompt: &str, schema: &str) -> AiRequest {
        AiRequest {
            mode: AiMode::Generate,
            prompt: prompt.into(),
            sql: String::new(),
            error: String::new(),
            schema: schema.into(),
            server_version: String::new(),
            extra: String::new(),
        }
    }

    fn blocks(text: &str) -> Value {
        json!([{"type":"thinking","thinking":"","signature":"S"}, {"type":"text","text": text}])
    }

    /// Araçsız tur: tek asistan mesajı.
    fn answer(text: &str) -> Vec<Value> {
        vec![json!({"role":"assistant","content": blocks(text)})]
    }

    #[test]
    fn tool_rounds_count_as_one_turn() {
        let c = Conversations::default();
        let id = Uuid::new_v4();
        let p = c.begin(id, 0, "t", &ask("e-posta nerede?", "")).unwrap();
        let tool_use = json!({"role":"assistant","content":[{"type":"tool_use","id":"a","name":"search_columns","input":{"name_contains":"email"}}]});
        let result = json!({"role":"user","content":[{"type":"tool_result","tool_use_id":"a","content":"public\tusers\temail\ttext","is_error":false}]});
        let final_answer = json!({"role":"assistant","content": blocks("users.email")});
        c.finish(p, Some(vec![tool_use.clone(), result.clone(), final_answer.clone()]));
        let p = c.begin(id, 1, "t", &ask("peki telefon?", "")).unwrap();
        assert_eq!(p.messages.len(), 5);
        assert_eq!(p.messages[1], tool_use);
        assert_eq!(p.messages[2], result);
        assert_eq!(p.messages[3], final_answer);
    }

    #[test]
    fn history_is_append_only_and_schema_is_sent_once() {
        let c = Conversations::default();
        let id = Uuid::new_v4();
        let p1 = c.begin(id, 0, "t", &ask("ilk", "public.t(id)")).unwrap();
        assert_eq!(p1.messages.len(), 1);
        assert!(p1.messages[0]["content"].as_str().unwrap().contains("<schema>"));
        let first_user = p1.messages[0].clone();
        c.finish(p1, Some(answer("A1")));

        let p2 = c.begin(id, 1, "t", &ask("ikinci", "public.t(id)")).unwrap();
        assert_eq!(p2.messages.len(), 3);
        assert_eq!(p2.messages[0], first_user, "önceki mesaj değişmeden gönderilir");
        assert_eq!(p2.messages[1]["content"], blocks("A1"), "thinking imzası korunur");
        assert!(!p2.messages[2]["content"].as_str().unwrap().contains("<schema>"));
        c.finish(p2, Some(answer("A2")));

        let p3 = c.begin(id, 2, "t", &ask("üçüncü", "public.t(id, ad)")).unwrap();
        assert!(p3.messages[4]["content"].as_str().unwrap().contains("<schema>"), "şema değişince yeniden gönderilir");
    }

    #[test]
    fn failed_or_cancelled_turn_is_not_recorded() {
        let c = Conversations::default();
        let id = Uuid::new_v4();
        let p = c.begin(id, 0, "t", &ask("x", "")).unwrap();
        c.finish(p, None);
        let p = c.begin(id, 0, "t", &ask("z", "")).unwrap();
        // Görünür metni olmayan yanıt (yalnız thinking ya da boşluk) da girmez: arayüzün `isCommitted` kuralı.
        c.finish(p, Some(answer("  ")));
        let p = c.begin(id, 0, "t", &ask("y", "")).unwrap();
        assert_eq!(p.messages.len(), 1);
        assert!(p.messages[0]["content"].as_str().unwrap().contains("<request>\ny\n</request>"));
    }

    #[test]
    fn switching_provider_or_model_requires_a_new_chat() {
        let c = Conversations::default();
        let id = Uuid::new_v4();
        let p = c.begin(id, 0, "Anthropic|claude-opus-5-5", &ask("x", "")).unwrap();
        c.finish(p, Some(answer("a")));
        let e = c.begin(id, 1, "OpenRouter|openai/gpt-5.6", &ask("y", "")).err().unwrap();
        assert_eq!(e.code, "ai_stale");
        let p = c.begin(id, 1, "Anthropic|claude-opus-5-5", &ask("y", "")).unwrap();
        c.finish(p, None);
        // Boş sohbet (henüz tur yok) sağlayıcı değiştirebilir.
        let fresh = Uuid::new_v4();
        let p = c.begin(fresh, 0, "Anthropic|claude-opus-5-5", &ask("x", "")).unwrap();
        c.finish(p, None);
        assert!(c.begin(fresh, 0, "OpenRouter|openai/gpt-5.6", &ask("x", "")).is_ok());
    }

    #[test]
    fn busy_and_stale_conversations_are_rejected() {
        let c = Conversations::default();
        let id = Uuid::new_v4();
        let p = c.begin(id, 0, "t", &ask("x", "")).unwrap();
        assert_eq!(c.begin(id, 0, "t", &ask("y", "")).err().unwrap().code, "busy");
        c.finish(p, Some(answer("a")));
        assert_eq!(c.begin(id, 0, "t", &ask("y", "")).err().unwrap().code, "ai_stale");
        assert_eq!(c.begin(Uuid::new_v4(), 3, "t", &ask("y", "")).err().unwrap().code, "ai_stale");
        c.forget(id);
        assert_eq!(c.begin(id, 1, "t", &ask("y", "")).err().unwrap().code, "ai_stale");
    }

    #[test]
    fn oldest_idle_conversation_is_evicted() {
        let c = Conversations::default();
        let ids: Vec<Uuid> = (0..MAX_CONVERSATIONS).map(|_| Uuid::new_v4()).collect();
        for id in &ids {
            let p = c.begin(*id, 0, "t", &ask("x", "")).unwrap();
            c.finish(p, Some(answer("a")));
        }
        let p = c.begin(Uuid::new_v4(), 0, "t", &ask("x", "")).unwrap();
        c.finish(p, None);
        assert_eq!(c.begin(ids[0], 1, "t", &ask("y", "")).err().unwrap().code, "ai_stale");
        assert!(c.begin(ids[1], 1, "t", &ask("y", "")).is_ok());
    }
}
