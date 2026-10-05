//! Sorgu geçmişi, sekme taslakları ve ayar komutları. Dosyalar küçüktür; komutlar senkron çalışır.
//! SQL metni asla günlüğe yazılmaz.

use pgt_core::ErrorPayload;
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

#[tauri::command]
pub fn qt_history_push(state: State<'_, AppState>, entry: HistoryEntry) -> CmdResult<bool> {
    let mut h = state.store.history.lock().unwrap();
    Ok(h.push_checked(entry, is_sensitive_sql)?)
}

#[tauri::command]
pub fn qt_history_list(state: State<'_, AppState>, query: Option<String>, limit: u32) -> CmdResult<Vec<HistoryEntry>> {
    let h = state.store.history.lock().unwrap();
    Ok(h.list(query.as_deref(), (limit as usize).min(MAX_LIST)))
}

#[tauri::command]
pub fn qt_history_clear(state: State<'_, AppState>) -> CmdResult<()> {
    Ok(state.store.history.lock().unwrap().clear()?)
}

#[tauri::command]
pub fn qt_drafts_save(state: State<'_, AppState>, tab: DraftTab, sql: String) -> CmdResult<()> {
    Ok(state.store.drafts.save_tab(&tab, &sql)?)
}

#[tauri::command]
pub fn qt_drafts_remove(state: State<'_, AppState>, id: Uuid) -> CmdResult<()> {
    Ok(state.store.drafts.remove(id)?)
}

#[tauri::command]
pub fn qt_drafts_load(state: State<'_, AppState>) -> CmdResult<Vec<(DraftTab, String)>> {
    Ok(state.store.drafts.load_all()?)
}

#[tauri::command]
pub fn qt_drafts_clear(state: State<'_, AppState>) -> CmdResult<()> {
    Ok(state.store.drafts.clear()?)
}

#[tauri::command]
pub fn settings_get(state: State<'_, AppState>) -> CmdResult<AppSettings> {
    Ok(*state.store.settings.lock().unwrap())
}

#[tauri::command]
pub fn settings_set_history(state: State<'_, AppState>, enabled: bool) -> CmdResult<AppSettings> {
    let mut settings = state.store.settings.lock().unwrap();
    let next = AppSettings { history_enabled: enabled };
    next.save(&state.store.settings_path)?;
    *settings = next;
    state.store.history.lock().unwrap().set_enabled(enabled);
    Ok(next)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_sensitive_in_scripts() {
        assert!(is_sensitive_sql("create role x login password 'p'"));
        assert!(is_sensitive_sql("select 1; alter user bob with password 'p'"));
        assert!(!is_sensitive_sql("select 1; select 2"));
    }
}
