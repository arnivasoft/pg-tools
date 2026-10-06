//! Sorgu geçmişi, sekme taslakları ve ayar komutları.
//! SQL metni asla günlüğe yazılmaz. Komutlar `async` işaretlidir: Tauri senkron komutları ana iş parçacığında
//! çalıştırır; dosya G/Ç'si (5 MB'a varan geçmiş, sınırsız taslak metni) pencereyi dondurmasın.

use pgt_core::ErrorPayload;
use pgt_core::format_options::FormatOptions;
use pgt_core::store::{AppSettings, DraftTab, HistoryEntry};
use tauri::State;
use uuid::Uuid;

use crate::state::AppState;

type CmdResult<T> = Result<T, ErrorPayload>;

const MAX_LIST: usize = 500;

/// Hassas ifadeler (PASSWORD/CONNECTION içeren CREATE/ALTER) istemciye güvenilmeden arka uçta da elenir.
/// Betik birden çok ifade içerebileceğinden her ifade ayrı denetlenir.
pub(crate) fn is_sensitive_sql(sql: &str) -> bool {
    pgt_query::classify::analyze(sql).sensitive
        || pgt_query::split::split(sql).iter().any(|s| pgt_query::classify::analyze(&sql[s.start..s.end]).sensitive)
}

/// Bir çalıştırmanın girdileri tek çağrıda, tek dosya yazımıyla. Döner: eklenen girdi sayısı (kapalıyken 0).
#[tauri::command(async)]
pub fn qt_history_push_many(state: State<'_, AppState>, entries: Vec<HistoryEntry>) -> CmdResult<u32> {
    let mut h = state.store.history.lock().unwrap();
    Ok(h.push_many(entries, is_sensitive_sql)? as u32)
}

#[tauri::command(async)]
pub fn qt_history_list(state: State<'_, AppState>, query: Option<String>, limit: u32) -> CmdResult<Vec<HistoryEntry>> {
    let h = state.store.history.lock().unwrap();
    Ok(h.list(query.as_deref(), (limit as usize).min(MAX_LIST)))
}

#[tauri::command(async)]
pub fn qt_history_clear(state: State<'_, AppState>) -> CmdResult<()> {
    Ok(state.store.history.lock().unwrap().clear()?)
}

#[tauri::command(async)]
pub fn qt_drafts_save(state: State<'_, AppState>, tab: DraftTab, sql: String) -> CmdResult<()> {
    Ok(state.store.drafts.save_tab(&tab, &sql)?)
}

#[tauri::command(async)]
pub fn qt_drafts_remove(state: State<'_, AppState>, id: Uuid) -> CmdResult<()> {
    Ok(state.store.drafts.remove(id)?)
}

#[tauri::command(async)]
pub fn qt_drafts_load(state: State<'_, AppState>) -> CmdResult<Vec<(DraftTab, String)>> {
    Ok(state.store.drafts.load_all()?)
}

#[tauri::command(async)]
pub fn qt_drafts_clear(state: State<'_, AppState>) -> CmdResult<()> {
    Ok(state.store.drafts.clear()?)
}

#[tauri::command(async)]
pub fn settings_get(state: State<'_, AppState>) -> CmdResult<AppSettings> {
    Ok(*state.store.settings.lock().unwrap())
}

#[tauri::command(async)]
pub fn settings_set_history(state: State<'_, AppState>, enabled: bool) -> CmdResult<AppSettings> {
    let mut settings = state.store.settings.lock().unwrap();
    let next = AppSettings { history_enabled: enabled, ..*settings };
    next.save(&state.store.settings_path)?;
    *settings = next;
    state.store.history.lock().unwrap().set_enabled(enabled);
    Ok(next)
}

/// Editör ayarları: yazarken denetim ve biçimlendirici seçenekleri (geçersiz seçenek `invalid`, dosyaya yazılmaz).
#[tauri::command(async)]
pub fn settings_set_editor(state: State<'_, AppState>, check_enabled: bool, format: FormatOptions) -> CmdResult<AppSettings> {
    format.validate()?;
    let mut settings = state.store.settings.lock().unwrap();
    let next = AppSettings { check_enabled, format, ..*settings };
    next.save(&state.store.settings_path)?;
    *settings = next;
    Ok(next)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn push_many_filters_sensitive_scripts_per_entry() {
        use pgt_core::store::History;
        let dir = tempfile::tempdir().unwrap();
        let mut h = History::load(dir.path().join("history.json")).unwrap();
        let e = |sql: &str| HistoryEntry {
            id: Uuid::new_v4(),
            sql: sql.into(),
            server_id: Uuid::new_v4(),
            server_name: "s".into(),
            database: None,
            at_ms: 1,
            elapsed_ms: 1,
            ok: true,
        };
        let n = h.push_many(vec![e("select 1"), e("select 1; alter user bob with password 'p'"), e("select 2")], is_sensitive_sql).unwrap();
        assert_eq!(n, 2);
        assert!(h.list(None, 10).iter().all(|x| !x.sql.contains("password")));
    }

    #[test]
    fn detects_sensitive_in_scripts() {
        assert!(is_sensitive_sql("create role x login password 'p'"));
        assert!(is_sensitive_sql("select 1; alter user bob with password 'p'"));
        assert!(!is_sensitive_sql("select 1; select 2"));
    }
}
