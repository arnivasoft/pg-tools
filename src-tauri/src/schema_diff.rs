//! Şema karşılaştırma Tauri komutları (`schema_diff_*`; spec 2026-10-31, Plan 29 Task 8).
//!
//! - **Kimlik:** `diffId` istemci tarafından üretilir; sonuç `DiffRegistry`'de tutulur (en çok `MAX_READY`).
//!   Sayfa yenilenince ve kapanışta kayıt boşaltılır (çalışanlar iptal edilir).
//! - **Gizlilik:** bu katman `tracing` yazmaz; hata iletileri ad/yol içermez. Betik yalnız dosyaya yazılır.
//! - **Hata kodları:** `same_schema`, `schema_not_found`/`system_schema` (`detail`: `source`|`target`), `cancelled`,
//!   `busy`, `diff_not_found`, `item_not_found`, `not_selectable`, `empty_selection`, `no_script`, `invalid_path`,
//!   `write_failed`; bağlantı hataları `ErrorPayload::from` ile (`not_connected`, `connection_lost` ...).
//!
//! Gelecekler `Box::pin` ile kutulanır (bkz. ddl.rs: ana iş parçacığı yığını).

use std::ffi::OsString;
use std::io::Write;
use std::path::Path;
use std::time::SystemTime;

use pgt_core::connection::Target;
use pgt_core::{Error, ErrorPayload};
use pgt_diff::{
    DiffData, DiffDatabase, DiffDetail, DiffEvent, DiffOptions, DiffResult, DiffScript, DiffSide, DiffSink,
    RegisterError, RunError, ScriptError, SchemaCheck, SideTag, run_diff,
};
use pgt_diff::script::utc_timestamp;
use tauri::State;
use tauri::ipc::Channel;
use uuid::Uuid;

use crate::state::AppState;

type CmdResult<T> = Result<T, ErrorPayload>;

const DATABASES_SQL: &str = "SELECT oid, datname::text FROM pg_catalog.pg_database WHERE datallowconn AND NOT datistemplate ORDER BY datname COLLATE \"C\"";
const PARTIAL_SUFFIX: &str = ".pgt-partial";

fn payload(code: &str, message: &str, detail: Option<&str>) -> ErrorPayload {
    ErrorPayload { code: code.to_string(), message: message.to_string(), detail: detail.map(str::to_string), hint: None }
}

fn side_name(side: SideTag) -> &'static str {
    match side {
        SideTag::Source => "source",
        SideTag::Target => "target",
    }
}

/// Yürütme hatası -> yük (saf; ileti ad içermez).
pub fn run_error_payload(e: &RunError) -> ErrorPayload {
    match e {
        RunError::SameSchema => payload("same_schema", "Kaynak ve hedef aynı şema olamaz.", None),
        RunError::Cancelled => payload("cancelled", "Karşılaştırma iptal edildi.", None),
        RunError::Schema { side, check: SchemaCheck::Missing } => {
            payload("schema_not_found", "Şema bulunamadı.", Some(side_name(*side)))
        }
        RunError::Schema { side, check: SchemaCheck::System } => {
            payload("system_schema", "Sistem şemaları karşılaştırılamaz.", Some(side_name(*side)))
        }
        RunError::Schema { side, check: SchemaCheck::Ok } => {
            payload("invalid", "Şema denetimi beklenmedik sonuç verdi.", Some(side_name(*side)))
        }
        RunError::Core(e) => ErrorPayload::from(e),
    }
}

fn register_payload(e: RegisterError) -> ErrorPayload {
    match e {
        RegisterError::DuplicateId => payload("busy", "Bu kimlikle bir karşılaştırma zaten var.", None),
    }
}

fn script_payload(e: ScriptError) -> ErrorPayload {
    match e {
        ScriptError::Empty => payload("empty_selection", "Hiçbir öğe seçilmedi.", None),
        ScriptError::UnknownKey => payload("item_not_found", "Öğe sonuçta bulunamadı.", None),
        ScriptError::NotSelectable => payload("not_selectable", "Seçilen öğelerden biri betiğe alınamaz.", None),
    }
}

fn not_found() -> ErrorPayload {
    payload("diff_not_found", "Karşılaştırma bulunamadı; yeniden başlatın.", None)
}

fn data_of(state: &AppState, diff_id: Uuid) -> CmdResult<std::sync::Arc<DiffData>> {
    state.diff.get(diff_id).ok_or_else(not_found)
}

/// `Channel<DiffEvent>` alıcısı: gönderim hatası (pencere gitti) işi iptal ettirir.
struct ChannelSink(Channel<DiffEvent>);

impl DiffSink for ChannelSink {
    fn send(&mut self, e: DiffEvent) -> bool {
        self.0.send(e).is_ok()
    }
}

/// Sunucudaki bağlanılabilir (şablon olmayan) veritabanları.
#[tauri::command]
pub async fn schema_diff_databases(state: State<'_, AppState>, server_id: Uuid) -> CmdResult<Vec<DiffDatabase>> {
    let list = Box::pin(state.manager.run(server_id, Target::Maintenance, |c| async move {
        let rows = c.client.query(DATABASES_SQL, &[]).await?;
        Ok(rows.iter().map(|r| DiffDatabase { oid: r.get(0), name: r.get(1) }).collect::<Vec<_>>())
    }))
    .await
    .map_err(|e: Error| ErrorPayload::from(&e))?;
    Ok(list)
}

#[tauri::command]
pub async fn schema_diff_start(
    state: State<'_, AppState>,
    diff_id: Uuid,
    source: DiffSide,
    target: DiffSide,
    options: DiffOptions,
    on_event: Channel<DiffEvent>,
) -> CmdResult<DiffResult> {
    let (token, _guard) = state.diff.register(diff_id).map_err(register_payload)?;
    let mut sink = ChannelSink(on_event);
    let data = Box::pin(run_diff(&state.manager, &source, &target, options, &mut sink, &token))
        .await
        .map_err(|e| run_error_payload(&e))?;
    let result = data.result().clone();
    state.diff.complete(diff_id, data);
    Ok(result)
}

#[tauri::command]
pub async fn schema_diff_cancel(state: State<'_, AppState>, diff_id: Uuid) -> CmdResult<()> {
    state.diff.cancel(diff_id);
    Ok(())
}

#[tauri::command]
pub async fn schema_diff_detail(state: State<'_, AppState>, diff_id: Uuid, item_key: String) -> CmdResult<DiffDetail> {
    data_of(&state, diff_id)?
        .detail(&item_key)
        .ok_or_else(|| payload("item_not_found", "Öğe sonuçta bulunamadı.", None))
}

#[tauri::command]
pub async fn schema_diff_script(state: State<'_, AppState>, diff_id: Uuid, item_keys: Vec<String>) -> CmdResult<DiffScript> {
    let data = data_of(&state, diff_id)?;
    data.script(&item_keys, &utc_timestamp(SystemTime::now())).map_err(script_payload)
}

/// Yol doğrulaması (saf): boş olmamalı, denetim karakteri içermemeli, Windows'ta mutlak olmalı.
pub fn save_path_check(path: &str) -> Result<(), &'static str> {
    if path.chars().any(char::is_control) {
        return Err("Yol geçersiz karakter içeriyor.");
    }
    if path.is_empty() {
        return Err("Tam (mutlak) yol gerekir.");
    }
    #[cfg(windows)]
    if !Path::new(path).is_absolute() {
        return Err("Tam (mutlak) yol gerekir.");
    }
    Ok(())
}

/// Metni `<yol>.pgt-partial`'a yazıp hedefe taşır (var olan değiştirilir). Hata olursa geçici dosya silinir.
/// UTF-8, BOM yok.
pub fn write_script(path: &Path, text: &str) -> std::io::Result<()> {
    let mut tmp_name: OsString = path.as_os_str().to_owned();
    tmp_name.push(PARTIAL_SUFFIX);
    let tmp = Path::new(&tmp_name);
    let result = (|| {
        let mut f = std::fs::File::create(tmp)?;
        f.write_all(text.as_bytes())?;
        f.sync_all()?;
        drop(f);
        std::fs::rename(tmp, path)
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(tmp);
    }
    result
}

/// Son üretilen betiği dosyaya kaydeder. Yol günlüğe ve hata iletisine yazılmaz.
#[tauri::command]
pub async fn schema_diff_save(state: State<'_, AppState>, diff_id: Uuid, path: String) -> CmdResult<()> {
    let data = data_of(&state, diff_id)?;
    let sql = data
        .last_script()
        .ok_or_else(|| payload("no_script", "Önce betik üretilmeli.", None))?;
    save_path_check(&path).map_err(|m| payload("invalid_path", m, None))?;
    let written = tauri::async_runtime::spawn_blocking(move || write_script(Path::new(&path), &sql)).await;
    match written {
        Ok(Ok(())) => Ok(()),
        _ => Err(payload("write_failed", "Dosya yazılamadı.", None)),
    }
}

#[tauri::command]
pub async fn schema_diff_close(state: State<'_, AppState>, diff_id: Uuid) -> CmdResult<()> {
    state.diff.close(diff_id);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn run_error_codes() {
        let code = |e: RunError| run_error_payload(&e);
        assert_eq!(code(RunError::SameSchema).code, "same_schema");
        assert_eq!(code(RunError::Cancelled).code, "cancelled");
        let m = code(RunError::Schema { side: SideTag::Source, check: SchemaCheck::Missing });
        assert_eq!((m.code.as_str(), m.detail.as_deref()), ("schema_not_found", Some("source")));
        let s = code(RunError::Schema { side: SideTag::Target, check: SchemaCheck::System });
        assert_eq!((s.code.as_str(), s.detail.as_deref()), ("system_schema", Some("target")));
        assert_eq!(code(RunError::Core(Error::NotConnected)).code, "not_connected");
        assert_eq!(code(RunError::Core(Error::ConnectionLost("x".into()))).code, "connection_lost");
    }

    #[test]
    fn register_and_script_codes() {
        assert_eq!(register_payload(RegisterError::DuplicateId).code, "busy");
        assert_eq!(script_payload(ScriptError::Empty).code, "empty_selection");
        assert_eq!(script_payload(ScriptError::UnknownKey).code, "item_not_found");
        assert_eq!(script_payload(ScriptError::NotSelectable).code, "not_selectable");
        assert_eq!(not_found().code, "diff_not_found");
    }

    #[test]
    #[cfg(windows)]
    fn save_path_relative_rejected_on_windows() {
        assert_eq!(save_path_check("a.sql"), Err("Tam (mutlak) yol gerekir."));
        assert_eq!(save_path_check(""), Err("Tam (mutlak) yol gerekir."));
        assert_eq!(save_path_check("C:\\x\\a.sql"), Ok(()));
    }

    #[test]
    fn save_path_control_chars_rejected() {
        let p = if cfg!(windows) { "C:\\x\\a\nb.sql" } else { "/x/a\nb.sql" };
        assert_eq!(save_path_check(p), Err("Yol geçersiz karakter içeriyor."));
        let p = if cfg!(windows) { "C:\\x\\a\0b.sql" } else { "/x/a\0b.sql" };
        assert_eq!(save_path_check(p), Err("Yol geçersiz karakter içeriyor."));
    }

    #[test]
    fn write_script_creates_and_replaces() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("fark.sql");
        write_script(&p, "SELECT 1;\n").unwrap();
        assert_eq!(std::fs::read(&p).unwrap(), b"SELECT 1;\n", "BOM yok");
        write_script(&p, "SELECT 2;\n").unwrap();
        assert_eq!(std::fs::read_to_string(&p).unwrap(), "SELECT 2;\n");
        assert!(!dir.path().join("fark.sql.pgt-partial").exists());
    }

    #[test]
    fn write_script_to_directory_fails_without_leftover() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("klasor");
        std::fs::create_dir(&target).unwrap();
        assert!(write_script(&target, "x").is_err());
        assert!(!dir.path().join("klasor.pgt-partial").exists());
        assert!(target.is_dir());
    }
}
