//! Yapay zekâ asistanı komutları (`ai_*`). Anahtar Windows Kimlik Bilgisi Yöneticisi'nde tutulur ve arayüze
//! hiç gönderilmez; istem/yanıt metni günlüğe yazılmaz. Asistan yalnız metin üretir, SQL çalıştırmaz.

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

use pgt_ai::{AiClient, AiEvent, AiRequest, Cancel};
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
}

impl AiState {
    fn client(&self) -> CmdResult<&AiClient> {
        if let Some(c) = self.client.get() {
            return Ok(c);
        }
        let c = AiClient::new()?;
        Ok(self.client.get_or_init(|| c))
    }

    pub fn cancel_all(&self) {
        for (_, c) in self.running.lock().unwrap().drain() {
            c.cancel();
        }
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

/// Akışlı istek. Olaylar `on_event` ile gelir; söz akış bitince (ya da iptalde) çözülür.
#[tauri::command]
pub async fn ai_ask(state: State<'_, AppState>, id: Uuid, request: AiRequest, on_event: Channel<AiEvent>) -> CmdResult<()> {
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
    let mut emit = |e: AiEvent| on_event.send(e).is_ok();
    Box::pin(client.stream(&key, &request, &cancel, &mut emit)).await
}

#[tauri::command(async)]
pub fn ai_cancel(state: State<'_, AppState>, id: Uuid) -> CmdResult<()> {
    if let Some(c) = state.ai.running.lock().unwrap().get(&id) {
        c.cancel();
    }
    Ok(())
}
