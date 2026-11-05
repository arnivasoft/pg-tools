//! Araç döngüsü: model araç isterse (`stop_reason: tool_use`) katalog araçları çalıştırılır, sonuçlar tek bir
//! kullanıcı mesajında geri gönderilir ve model yanıtını tamamlayana dek sürer. En çok `MAX_TOOL_ROUNDS` araç
//! turu; sonuncusu `tool_choice: none` ile gönderilir (model yanıt vermek zorunda kalır).
//!
//! Güvenlik: `max_tokens` ya da `refusal` ile biten yanıttaki araç çağrıları çalıştırılmaz; girdisi geçersiz
//! (yarım/bozuk JSON) ya da şemaya uymayan çağrı çalıştırılmaz, modele hata sonucu döner.

use std::future::Future;
use std::pin::Pin;

use pgt_core::ErrorPayload;
use serde_json::{Value, json};

use crate::client::{AiClient, Cancel, Outcome, Profile, Usage};
use crate::model::AiEvent;
use crate::tools;

pub const MAX_TOOL_ROUNDS: usize = 6;

pub type BoxFut<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;
pub type Emit<'a> = dyn FnMut(AiEvent) -> bool + Send + 'a;

/// Model çağrısı (testte sahte model kullanılır).
pub trait Model: Send + Sync {
    fn send<'a>(
        &'a self,
        messages: &'a [Value],
        allow_tools: bool,
        cancel: &'a Cancel,
        emit: &'a mut Emit<'_>,
    ) -> BoxFut<'a, Result<Option<Outcome>, ErrorPayload>>;
}

/// Gerçek istemci + sağlayıcı profili + anahtar.
pub struct Claude<'k> {
    pub client: &'k AiClient,
    pub profile: &'k Profile,
    pub api_key: &'k str,
}

impl Model for Claude<'_> {
    fn send<'a>(
        &'a self,
        messages: &'a [Value],
        allow_tools: bool,
        cancel: &'a Cancel,
        emit: &'a mut Emit<'_>,
    ) -> BoxFut<'a, Result<Option<Outcome>, ErrorPayload>> {
        Box::pin(self.client.stream(self.profile, self.api_key, messages, allow_tools, cancel, emit))
    }
}

/// Araç çalıştırıcı. `Err`: modele `is_error` olarak dönecek açıklama.
pub trait ToolExec: Send + Sync {
    fn call<'a>(&'a self, name: &'a str, input: &'a Value) -> BoxFut<'a, Result<String, String>>;
}

/// Bir sohbet turunu çalıştırır. `history`: önceki geçmiş + yeni kullanıcı mesajı. Başarıda kullanıcı
/// mesajından sonra geçmişe eklenecek mesajlar (asistan, araç sonuçları, ..., son asistan) döner;
/// iptalde ya da retde `None`. `Done` olayı yalnız tur bitince (tüm isteklerin toplam kullanımıyla) gider.
pub async fn run_turn(
    model: &dyn Model,
    history: &[Value],
    cancel: &Cancel,
    emit: &mut Emit<'_>,
    exec: &dyn ToolExec,
) -> Result<Option<Vec<Value>>, ErrorPayload> {
    let mut added: Vec<Value> = Vec::new();
    let mut usage = Usage::default();
    let mut emitted_text = false;
    for round in 0..=MAX_TOOL_ROUNDS {
        let allow_tools = round < MAX_TOOL_ROUNDS;
        let messages: Vec<Value> = history.iter().chain(added.iter()).cloned().collect();
        // Turlar arası metin bitişmesin: önceki turda metin geldiyse yeni turun ilk parçasından önce boş satır.
        let mut need_sep = emitted_text;
        let mut wrapped = |e: AiEvent| {
            if matches!(e, AiEvent::Delta { .. }) {
                if need_sep {
                    need_sep = false;
                    if !emit(AiEvent::Delta { text: "\n\n".into() }) {
                        return false;
                    }
                }
                emitted_text = true;
            }
            emit(e)
        };
        let out = model.send(&messages, allow_tools, cancel, &mut wrapped).await?;
        let Some(out) = out else { return Ok(None) };
        usage.add(out.usage);
        let done = |emit: &mut Emit<'_>, stop: &str| {
            emit(AiEvent::Done {
                stop_reason: stop.to_string(),
                input_tokens: usage.input_tokens,
                cache_read_tokens: usage.cache_read_tokens,
                output_tokens: usage.output_tokens,
            })
        };
        if out.stop_reason == "refusal" {
            done(emit, &out.stop_reason);
            return Ok(None);
        }
        let run_tools = allow_tools && out.stop_reason == "tool_use";
        let mut content = out.content;
        let calls: Vec<(String, String, Value)> = if run_tools {
            content
                .iter()
                .filter(|b| b["type"] == "tool_use")
                .map(|b| (b["id"].as_str().unwrap_or("").to_string(), b["name"].as_str().unwrap_or("").to_string(), b["input"].clone()))
                .collect()
        } else {
            // Sonuçsuz kalacak araç çağrısı geçmişe giremez (API reddeder).
            content.retain(|b| b["type"] != "tool_use");
            Vec::new()
        };
        if !content.is_empty() {
            added.push(json!({ "role": "assistant", "content": content }));
        }
        if calls.is_empty() {
            done(emit, &out.stop_reason);
            return Ok(Some(added));
        }
        let mut results = Vec::with_capacity(calls.len());
        for (id, name, input) in calls {
            let summary = tools::summary(&name, &input);
            if !emit(AiEvent::Tool { name: name.clone(), detail: summary }) {
                cancel.cancel();
                return Ok(None);
            }
            let res = match out.invalid_inputs.iter().find(|(i, _)| *i == id) {
                Some((_, raw)) => Err(format!("Invalid JSON input for {name}; send a complete JSON object. Received: {raw}")),
                None => match tools::validate(&name, &input) {
                    Err(e) => Err(e),
                    Ok(()) => tokio::select! {
                        r = exec.call(&name, &input) => r,
                        () = cancel.cancelled() => return Ok(None),
                    },
                },
            };
            let (text, is_error) = match res {
                Ok(t) => (t, false),
                Err(e) => (e, true),
            };
            results.push(json!({ "type": "tool_result", "tool_use_id": id, "content": text, "is_error": is_error }));
        }
        added.push(json!({ "role": "user", "content": results }));
    }
    unreachable!("son tur araçsız gönderilir ve döner")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    /// Sırayla hazır yanıtlar döner ve gönderilen istekleri kaydeder.
    struct Script {
        outcomes: Mutex<Vec<Outcome>>,
        seen: Mutex<Vec<(usize, bool)>>,
    }

    impl Script {
        fn new(mut outcomes: Vec<Outcome>) -> Self {
            outcomes.reverse();
            Self { outcomes: Mutex::new(outcomes), seen: Mutex::new(Vec::new()) }
        }
    }

    impl Model for Script {
        fn send<'a>(
            &'a self,
            messages: &'a [Value],
            allow_tools: bool,
            _cancel: &'a Cancel,
            emit: &'a mut Emit<'_>,
        ) -> BoxFut<'a, Result<Option<Outcome>, ErrorPayload>> {
            self.seen.lock().unwrap().push((messages.len(), allow_tools));
            let out = self.outcomes.lock().unwrap().pop().expect("beklenmeyen istek");
            for b in &out.content {
                if let Some(t) = b.get("text").and_then(Value::as_str) {
                    emit(AiEvent::Delta { text: t.to_string() });
                }
            }
            Box::pin(async move { Ok(Some(out)) })
        }
    }

    struct Exec(Mutex<Vec<String>>);
    impl ToolExec for Exec {
        fn call<'a>(&'a self, name: &'a str, input: &'a Value) -> BoxFut<'a, Result<String, String>> {
            self.0.lock().unwrap().push(format!("{name} {input}"));
            Box::pin(async move { Ok("schema\trelation\tcolumn\ttype\npublic\tusers\temail\ttext\n".to_string()) })
        }
    }

    fn outcome(content: Vec<Value>, stop: &str) -> Outcome {
        Outcome { content, invalid_inputs: Vec::new(), stop_reason: stop.into(), usage: Usage { input_tokens: 10, cache_read_tokens: 5, output_tokens: 2 } }
    }

    fn call(id: &str) -> Value {
        json!({"type":"tool_use","id": id,"name":"search_columns","input":{"name_contains":"email"}})
    }

    async fn run(script: &Script, exec: &Exec) -> (Result<Option<Vec<Value>>, ErrorPayload>, Vec<AiEvent>) {
        let mut events = Vec::new();
        let mut emit = |e: AiEvent| {
            events.push(e);
            true
        };
        let history = [json!({"role":"user","content":"e-posta nerede?"})];
        let r = run_turn(script, &history, &Cancel::default(), &mut emit, exec).await;
        (r, events)
    }

    #[tokio::test]
    async fn runs_tools_and_appends_results_in_one_user_message() {
        let script = Script::new(vec![
            outcome(vec![json!({"type":"thinking","thinking":"","signature":"S"}), call("a"), call("b")], "tool_use"),
            outcome(vec![json!({"type":"text","text":"users.email"})], "end_turn"),
        ]);
        let exec = Exec(Mutex::new(Vec::new()));
        let (r, events) = run(&script, &exec).await;
        let added = r.unwrap().unwrap();
        assert_eq!(added.len(), 3);
        assert_eq!(added[0]["content"][0]["signature"], "S", "thinking blokları korunur");
        let results = added[1]["content"].as_array().unwrap();
        assert_eq!(results.len(), 2, "paralel çağrıların sonuçları tek mesajda");
        assert_eq!(results[0]["tool_use_id"], "a");
        assert_eq!(results[1]["is_error"], false);
        assert_eq!(exec.0.lock().unwrap().len(), 2);
        assert_eq!(*script.seen.lock().unwrap(), vec![(1, true), (3, true)]);
        assert!(matches!(events.last(), Some(AiEvent::Done { input_tokens: 20, cache_read_tokens: 10, output_tokens: 4, .. })));
        assert_eq!(events.iter().filter(|e| matches!(e, AiEvent::Tool { .. })).count(), 2);
    }

    #[tokio::test]
    async fn invalid_or_unknown_input_is_not_executed() {
        let mut first = outcome(
            vec![call("a"), json!({"type":"tool_use","id":"b","name":"search_columns","input":{"x":1}})],
            "tool_use",
        );
        first.invalid_inputs = vec![("a".into(), "{\"name_con".into())];
        let script = Script::new(vec![first, outcome(vec![json!({"type":"text","text":"tamam"})], "end_turn")]);
        let exec = Exec(Mutex::new(Vec::new()));
        let added = run(&script, &exec).await.0.unwrap().unwrap();
        assert!(exec.0.lock().unwrap().is_empty());
        let results = added[1]["content"].as_array().unwrap();
        assert!(results.iter().all(|r| r["is_error"] == true));
    }

    #[tokio::test]
    async fn tool_calls_cut_by_max_tokens_are_not_run_and_dropped_from_history() {
        let script = Script::new(vec![outcome(vec![json!({"type":"text","text":"yarım"}), call("a")], "max_tokens")]);
        let exec = Exec(Mutex::new(Vec::new()));
        let added = run(&script, &exec).await.0.unwrap().unwrap();
        assert!(exec.0.lock().unwrap().is_empty());
        assert_eq!(added, vec![json!({"role":"assistant","content":[{"type":"text","text":"yarım"}]})]);
    }

    #[tokio::test]
    async fn refusal_is_not_recorded() {
        let script = Script::new(vec![outcome(vec![], "refusal")]);
        let (r, events) = run(&script, &Exec(Mutex::new(Vec::new()))).await;
        assert!(r.unwrap().is_none());
        assert!(matches!(events.last(), Some(AiEvent::Done { stop_reason, .. }) if stop_reason == "refusal"));
    }

    #[tokio::test]
    async fn last_round_disallows_tools() {
        let mut outs: Vec<Outcome> = (0..MAX_TOOL_ROUNDS).map(|i| outcome(vec![call(&format!("c{i}"))], "tool_use")).collect();
        outs.push(outcome(vec![json!({"type":"text","text":"son"})], "end_turn"));
        let script = Script::new(outs);
        let added = run(&script, &Exec(Mutex::new(Vec::new()))).await.0.unwrap().unwrap();
        let seen = script.seen.lock().unwrap();
        assert_eq!(seen.len(), MAX_TOOL_ROUNDS + 1);
        assert!(seen[..MAX_TOOL_ROUNDS].iter().all(|(_, allow)| *allow));
        assert!(!seen[MAX_TOOL_ROUNDS].1);
        assert_eq!(added.last().unwrap()["content"][0]["text"], "son");
    }

    #[tokio::test]
    async fn text_from_separate_rounds_is_separated() {
        let script = Script::new(vec![
            outcome(vec![json!({"type":"text","text":"Bakıyorum."}), call("a")], "tool_use"),
            outcome(vec![json!({"type":"text","text":"Buldum."})], "end_turn"),
        ]);
        let (_, events) = run(&script, &Exec(Mutex::new(Vec::new()))).await;
        let text: String = events
            .iter()
            .filter_map(|e| if let AiEvent::Delta { text } = e { Some(text.as_str()) } else { None })
            .collect();
        assert_eq!(text, "Bakıyorum.\n\nBuldum.");
    }
}
