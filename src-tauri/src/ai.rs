//! Yapay zekâ asistanı komutları (`ai_*`). Sağlayıcı (Claude API / OpenRouter), model ve effort `ai.json`'da;
//! anahtarlar sağlayıcı başına Windows Kimlik Bilgisi Yöneticisi'nde tutulur ve arayüze hiç gönderilmez;
//! istem/yanıt metni günlüğe yazılmaz. Asistan yalnız metin üretir, SQL çalıştırmaz.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};

use pgt_ai::{
    AiClient, AiEvent, AiProvider, AiRequest, AiSettings, AiSettingsView, Cancel, Claude, Conversations, Profile, ToolExec,
    run_turn,
};
use pgt_core::ErrorPayload;
use pgt_core::connection::{ConnectionManager, Target};
use pgt_query::session::QueryManager;
use serde_json::Value;
use tauri::State;
use tauri::ipc::Channel;
use uuid::Uuid;

use crate::state::AppState;

type CmdResult<T> = Result<T, ErrorPayload>;

fn payload(code: &str, message: &str) -> ErrorPayload {
    ErrorPayload { code: code.into(), message: message.into(), detail: None, hint: None }
}

/// HTTP istemcisi ilk istekte kurulur (sertifika deposu okuması açılışı yavaşlatmasın).
pub struct AiState {
    client: OnceLock<AiClient>,
    running: Mutex<HashMap<Uuid, Cancel>>,
    /// Sohbet geçmişleri (bellekte; sayfa yenilenince silinir).
    conversations: Conversations,
    settings: Mutex<AiSettings>,
    settings_path: PathBuf,
}

impl AiState {
    /// `ai.json` okunamazsa varsayılanlar (Claude API, `claude-opus-5-5`, effort varsayılan).
    pub fn load(config_dir: &Path) -> Self {
        let settings_path = config_dir.join("ai.json");
        Self {
            client: OnceLock::new(),
            running: Mutex::new(HashMap::new()),
            conversations: Conversations::default(),
            settings: Mutex::new(AiSettings::load(&settings_path)),
            settings_path,
        }
    }

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

fn view(state: &AppState, settings: AiSettings) -> CmdResult<AiSettingsView> {
    let has = |p: AiProvider| -> CmdResult<bool> { Ok(state.credentials.ai_key(p.key_name())?.is_some()) };
    Ok(AiSettingsView {
        has_anthropic_key: has(AiProvider::Anthropic)?,
        has_openrouter_key: has(AiProvider::OpenRouter)?,
        settings,
    })
}

/// Ayarlar ve hangi sağlayıcının anahtarının kayıtlı olduğu (anahtarlar dönmez).
#[tauri::command(async)]
pub fn ai_settings_get(state: State<'_, AppState>) -> CmdResult<AiSettingsView> {
    let s = state.ai.settings.lock().unwrap().clone();
    view(&state, s)
}

/// Sağlayıcı, modeller ve effort. Geçersiz model adı `invalid`, dosyaya yazılmaz.
#[tauri::command(async)]
pub fn ai_settings_set(state: State<'_, AppState>, settings: AiSettings) -> CmdResult<AiSettingsView> {
    let settings = AiSettings {
        anthropic_model: settings.anthropic_model.trim().to_string(),
        openrouter_model: settings.openrouter_model.trim().to_string(),
        ..settings
    };
    settings.validate().map_err(|m| payload("invalid", &m))?;
    settings
        .save(&state.ai.settings_path)
        .map_err(|e| payload("write_failed", &format!("Asistan ayarları kaydedilemedi: {e}")))?;
    *state.ai.settings.lock().unwrap() = settings.clone();
    view(&state, settings)
}

#[tauri::command(async)]
pub fn ai_key_set(state: State<'_, AppState>, provider: AiProvider, key: String) -> CmdResult<AiSettingsView> {
    let key = key.trim();
    if key.is_empty() || key.chars().any(char::is_whitespace) {
        return Err(payload("invalid", "Geçerli bir API anahtarı girin."));
    }
    state.credentials.set_ai_key(provider.key_name(), key)?;
    let s = state.ai.settings.lock().unwrap().clone();
    view(&state, s)
}

#[tauri::command(async)]
pub fn ai_key_clear(state: State<'_, AppState>, provider: AiProvider) -> CmdResult<AiSettingsView> {
    state.credentials.forget_ai_key(provider.key_name())?;
    let s = state.ai.settings.lock().unwrap().clone();
    view(&state, s)
}

/// Sohbete yeni tur. `turns`: arayüzün bildiği tamamlanmış tur sayısı (uyuşmazsa `ai_stale`).
/// Olaylar `on_event` ile gelir; söz akış bitince (ya da iptalde) çözülür. Başarılı tur geçmişe eklenir.
/// `server_id`/`database_oid`: sekmenin veritabanı (katalog araçları için); yoksa araçlar hata döner.
/// `session`: sekmenin sorgu oturumu (plan aracı için); yoksa plan aracı hata döner.
#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub async fn ai_ask(
    state: State<'_, AppState>,
    id: Uuid,
    conversation: Uuid,
    turns: u32,
    request: AiRequest,
    server_id: Option<Uuid>,
    database_oid: Option<u32>,
    session: Option<Uuid>,
    on_event: Channel<AiEvent>,
) -> CmdResult<()> {
    // İstek başına ayarların anlık görüntüsü: akış sürerken ayar değişse de bu tur aynı profille biter.
    let profile = Profile::from_settings(&state.ai.settings.lock().unwrap());
    let key = state.credentials.ai_key(profile.provider.key_name())?.ok_or_else(|| {
        let name = match profile.provider {
            AiProvider::Anthropic => "Claude (Anthropic)",
            AiProvider::OpenRouter => "OpenRouter",
        };
        payload("ai_no_key", &format!("Asistan için önce {name} API anahtarını girin."))
    })?;
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
    let pending = state.ai.conversations.begin(conversation, turns, &profile.conversation_tag(), &request)?;
    let mut emit = |e: AiEvent| on_event.send(e).is_ok();
    let model = Claude { client, profile: &profile, api_key: &key };
    let tools = CatalogTools {
        manager: &state.manager,
        target: server_id.zip(database_oid),
        query: state.query.clone(),
        session,
    };
    let result = Box::pin(run_turn(&model, &pending.messages, &cancel, &mut emit, &tools)).await;
    let added = result.as_ref().ok().cloned().flatten();
    state.ai.conversations.finish(pending, added);
    result.map(|_| ())
}

/// Katalog araçları sekmenin veritabanında, uygulamanın katalog bağlantı havuzunda koşar (kullanıcının sorgu
/// oturumu ve açık transaction'ı etkilenmez; bağlantı katmanının 35 sn sınırı geçerlidir).
/// Plan aracı (`explain_query`) ise kullanıcının sorgu oturumunda koşar (bkz. `explain_in_session`).
struct CatalogTools<'a> {
    manager: &'a ConnectionManager,
    target: Option<(Uuid, u32)>,
    query: Arc<QueryManager>,
    session: Option<Uuid>,
}

impl ToolExec for CatalogTools<'_> {
    fn call<'a>(&'a self, name: &'a str, input: &'a Value) -> pgt_ai::agent::BoxFut<'a, Result<String, String>> {
        Box::pin(async move {
            if name == pgt_ai::tools::EXPLAIN_TOOL {
                let started = std::time::Instant::now();
                let sql = pgt_ai::tools::explain_sql(input).unwrap_or_default();
                let r = pgt_ai::session_tools::explain_in_session(self.query.clone(), self.session, sql).await;
                tracing::info!(tool = name, ms = started.elapsed().as_millis() as u64, ok = r.is_ok(), "asistan aracı");
                return r;
            }
            let Some((server, oid)) = self.target else {
                return Err("This query tab has no database connection; answer without catalog lookups.".into());
            };
            let started = std::time::Instant::now();
            let r = self
                .manager
                .run(server, Target::Database(oid), |c| {
                    let input = input.clone();
                    async move { pgt_ai::tools::run(&c.client, name, &input).await }
                })
                .await;
            // Araç adı ve süre günlüğe yazılır; girdi (tablo adları) ve sonuç yazılmaz.
            tracing::info!(tool = name, ms = started.elapsed().as_millis() as u64, ok = matches!(r, Ok(Ok(_))), "asistan aracı");
            match r {
                Ok(inner) => inner,
                Err(e) => Err(format!("Catalog query failed: {}", ErrorPayload::from(&e).message)),
            }
        })
    }
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
