//! Yapay zekâ asistanı komutları (`ai_*`). Anahtar Windows Kimlik Bilgisi Yöneticisi'nde tutulur ve arayüze
//! hiç gönderilmez; istem/yanıt metni günlüğe yazılmaz. Asistan yalnız metin üretir, SQL çalıştırmaz.

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

use pgt_ai::{AiClient, AiEvent, AiRequest, Cancel, Conversations};
use pgt_core::ErrorPayload;
use tauri::State;
use tauri::ipc::Channel;
use uuid::Uuid;

use crate::state::AppState;

type CmdResult<T> = Result<T, ErrorPayload>;

fn payload(code: &str, message: &str) -> ErrorPayload {
    ErrorPayload { code: code.into(), message: message.into(), detail: None, hint: None }
}

/// HTTP istemcisi ilk istekte kurulur (sertifika deposu okuması açılışı yavaşlatmasın).
#[derive(Default)]
pub struct AiState {
    client: OnceLock<AiClient>,
    running: Mutex<HashMap<Uuid, Cancel>>,
    /// Sohbet geçmişleri (bellekte; sayfa yenilenince silinir).
    conversations: Conversations,
}

impl AiState {
    fn client(&self) -> CmdResult<&AiClient> {
        if let Some(c) = self.client.get() {
            return Ok(c);
        }
        let c = AiClient::new()?;
        Ok(self.client.get_or_init(|| c))
    }

    /// Sayfa yenilemede: akışlar kesilir, sohbetler (arayüzle birlikte kaybolan) silinir.
    pub fn reset(&self) {
        for (_, c) in self.running.lock().unwrap().drain() {
            c.cancel();
        }
        self.conversations.clear();
    }
}

/// Uçuştaki isteği kayıttan düşürür (iptal edilmiş ya da bitmiş olsun).
struct Running<'a>(&'a AiState, Uuid);

impl Drop for Running<'_> {
    fn drop(&mut self) {
        self.0.running.lock().unwrap().remove(&self.1);
    }
}

#[tauri::command(async)]
pub fn ai_key_status(state: State<'_, AppState>) -> CmdResult<bool> {
    Ok(state.credentials.ai_key()?.is_some())
}

#[tauri::command(async)]
pub fn ai_key_set(state: State<'_, AppState>, key: String) -> CmdResult<()> {
    let key = key.trim();
    if key.is_empty() || key.chars().any(char::is_whitespace) {
        return Err(payload("invalid", "Geçerli bir API anahtarı girin."));
    }
    Ok(state.credentials.set_ai_key(key)?)
}

#[tauri::command(async)]
pub fn ai_key_clear(state: State<'_, AppState>) -> CmdResult<()> {
    Ok(state.credentials.forget_ai_key()?)
}

/// Sohbete yeni tur. `turns`: arayüzün bildiği tamamlanmış tur sayısı (uyuşmazsa `ai_stale`).
/// Olaylar `on_event` ile gelir; söz akış bitince (ya da iptalde) çözülür. Başarılı tur geçmişe eklenir.
#[tauri::command]
pub async fn ai_ask(
    state: State<'_, AppState>,
    id: Uuid,
    conversation: Uuid,
    turns: u32,
    request: AiRequest,
    on_event: Channel<AiEvent>,
) -> CmdResult<()> {
    let key = state
        .credentials
        .ai_key()?
        .ok_or_else(|| payload("ai_no_key", "Asistan için önce bir Anthropic API anahtarı girin."))?;
    let client = state.ai.client()?;
    let cancel = Cancel::default();
    {
        let mut running = state.ai.running.lock().unwrap();
        if running.contains_key(&id) {
            return Err(payload("busy", "Bu istek zaten çalışıyor."));
        }
        running.insert(id, cancel.clone());
    }
    let _guard = Running(&state.ai, id);
    let pending = state.ai.conversations.begin(conversation, turns, &request)?;
    let mut emit = |e: AiEvent| on_event.send(e).is_ok();
    let result = Box::pin(client.stream(&key, &pending.messages, &cancel, &mut emit)).await;
    let content = result.as_ref().ok().cloned().flatten();
    state.ai.conversations.finish(pending, content);
    result.map(|_| ())
}

/// Sohbeti bellekten siler ("Yeni sohbet").
#[tauri::command(async)]
pub fn ai_forget(state: State<'_, AppState>, conversation: Uuid) -> CmdResult<()> {
    state.ai.conversations.forget(conversation);
    Ok(())
}

#[tauri::command(async)]
pub fn ai_cancel(state: State<'_, AppState>, id: Uuid) -> CmdResult<()> {
    if let Some(c) = state.ai.running.lock().unwrap().get(&id) {
        c.cancel();
    }
    Ok(())
}
