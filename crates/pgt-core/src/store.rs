//! Sorgu Aracı kalıcılığı: sorgu geçmişi, sekme taslakları ve küçük uygulama ayarları.
//!
//! Tüm yazımlar atomiktir (geçici dosya + rename). Bozuk bir dosya `Registry` gibi yedeklenir ve boş başlanır.
//! Bu modül `pgt-query`ye bağlı değildir: hassas SQL denetimi çağıranın verdiği yüklemle (`push_checked`) yapılır.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use ts_rs::TS;
use uuid::Uuid;

use crate::{Error, Result};

pub const MAX_HISTORY: usize = 500;
pub const MAX_HISTORY_SQL_BYTES: usize = 10 * 1024;

fn io_err(e: std::io::Error) -> Error {
    Error::Config(e.to_string())
}

fn write_atomic(path: &Path, bytes: &[u8]) -> Result<()> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir).map_err(io_err)?;
    }
    let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("dosya");
    let tmp = path.with_file_name(format!("{name}.{}-{}.tmp", std::process::id(), UNIQUE.fetch_add(1, Ordering::Relaxed)));
    let result = fs::write(&tmp, bytes).and_then(|()| fs::rename(&tmp, path));
    if let Err(e) = result {
        let _ = fs::remove_file(&tmp);
        return Err(io_err(e));
    }
    Ok(())
}

static UNIQUE: AtomicU64 = AtomicU64::new(0);

fn backup_path(path: &Path) -> PathBuf {
    let ms = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis()).unwrap_or(0);
    let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("dosya");
    path.with_file_name(format!("{name}.bak-{ms}-{}", UNIQUE.fetch_add(1, Ordering::Relaxed)))
}

/// Dosyayı okur; yoksa `None`, ayrıştırılamazsa yedekleyip `None` (+ yedek yolu) döner.
fn read_json<T: for<'de> Deserialize<'de>>(path: &Path) -> Result<(Option<T>, Option<PathBuf>)> {
    let text = match fs::read_to_string(path) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok((None, None)),
        Err(e) => return Err(Error::Config(format!("{}: {e}", path.display()))),
    };
    match serde_json::from_str::<T>(&text) {
        Ok(v) => Ok((Some(v), None)),
        Err(parse_error) => {
            let backup = backup_path(path);
            fs::rename(path, &backup).map_err(io_err)?;
            tracing::warn!(error = %parse_error, backup = %backup.display(), "dosya okunamadı, yedeklendi");
            Ok((None, Some(backup)))
        }
    }
}

fn to_json<T: Serialize>(value: &T) -> Result<Vec<u8>> {
    serde_json::to_vec_pretty(value).map_err(|e| Error::Config(e.to_string()))
}

// ---------------------------------------------------------------- Ayarlar

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct AppSettings {
    #[serde(default = "yes")]
    pub history_enabled: bool,
}

fn yes() -> bool {
    true
}

impl Default for AppSettings {
    fn default() -> Self {
        Self { history_enabled: true }
    }
}

impl AppSettings {
    /// Okunamayan/bozuk dosyada varsayılanlar döner (bozuk dosya yedeklenir).
    pub fn load(path: &Path) -> Self {
        match read_json::<AppSettings>(path) {
            Ok((Some(s), _)) => s,
            Ok(_) => Self::default(),
            Err(e) => {
                tracing::warn!(error = %e, "settings.json okunamadı; varsayılanlar kullanılıyor");
                Self::default()
            }
        }
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        write_atomic(path, &to_json(self)?)
    }
}

// ---------------------------------------------------------------- Geçmiş

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct HistoryEntry {
    pub id: Uuid,
    pub sql: String,
    pub server_id: Uuid,
    pub server_name: String,
    pub database: Option<String>,
    pub at_ms: i64,
    pub elapsed_ms: u64,
    pub ok: bool,
}

#[derive(Serialize, Deserialize)]
struct HistoryFile {
    #[serde(default)]
    version: u32,
    entries: Vec<HistoryEntry>,
}

/// `sql` 10 KB'ı aşıyorsa UTF-8 sınırında keser ve sonuna "…" ekler (toplam ≤ 10 KB).
pub fn truncate_sql(sql: &str) -> String {
    if sql.len() <= MAX_HISTORY_SQL_BYTES {
        return sql.to_string();
    }
    let mut end = MAX_HISTORY_SQL_BYTES - '…'.len_utf8();
    while !sql.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}…", &sql[..end])
}

pub struct History {
    path: PathBuf,
    /// Eskiden yeniye.
    entries: Vec<HistoryEntry>,
    enabled: bool,
    recovered_backup: Option<PathBuf>,
}

impl History {
    pub fn load(path: impl Into<PathBuf>) -> Result<Self> {
        let path = path.into();
        let (file, recovered_backup) = read_json::<HistoryFile>(&path)?;
        let mut entries = file.map(|f| f.entries).unwrap_or_default();
        if entries.len() > MAX_HISTORY {
            entries.drain(..entries.len() - MAX_HISTORY);
        }
        Ok(Self { path, entries, enabled: true, recovered_backup })
    }

    /// Dosya okunamadığında (izin vb.) boş geçmişle başlamak için; ilk başarılı yazım dosyayı değiştirir.
    pub fn empty(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into(), entries: Vec::new(), enabled: true, recovered_backup: None }
    }

    pub fn recovered_backup(&self) -> Option<&Path> {
        self.recovered_backup.as_deref()
    }

    pub fn set_enabled(&mut self, enabled: bool) {
        self.enabled = enabled;
    }

    pub fn enabled(&self) -> bool {
        self.enabled
    }

    /// Kapalıyken hiçbir şey yazmaz (`Ok(false)`).
    pub fn push(&mut self, entry: HistoryEntry) -> Result<bool> {
        self.push_checked(entry, |_| false)
    }

    /// `is_sensitive(sql)` doğruysa (ör. PASSWORD içeren DDL) kayıt reddedilir; denetim kesmeden önceki tam metinle yapılır.
    pub fn push_checked(&mut self, mut entry: HistoryEntry, is_sensitive: impl Fn(&str) -> bool) -> Result<bool> {
        if !self.enabled || is_sensitive(&entry.sql) {
            return Ok(false);
        }
        entry.sql = truncate_sql(&entry.sql);
        self.entries.push(entry);
        let overflow = self.entries.len().saturating_sub(MAX_HISTORY);
        let dropped: Vec<HistoryEntry> = self.entries.drain(..overflow).collect();
        if let Err(e) = self.save() {
            // Bellek ile dosya ayrışmasın.
            self.entries.pop();
            for (i, d) in dropped.into_iter().enumerate() {
                self.entries.insert(i, d);
            }
            return Err(e);
        }
        Ok(true)
    }

    /// Yeniden eskiye; `query` SQL'de veya sunucu adında (büyük/küçük harf duyarsız) aranır.
    pub fn list(&self, query: Option<&str>, limit: usize) -> Vec<HistoryEntry> {
        let needle = query.map(str::trim).filter(|q| !q.is_empty()).map(str::to_lowercase);
        self.entries
            .iter()
            .rev()
            .filter(|e| needle.as_deref().is_none_or(|n| e.sql.to_lowercase().contains(n) || e.server_name.to_lowercase().contains(n)))
            .take(limit)
            .cloned()
            .collect()
    }

    pub fn clear(&mut self) -> Result<()> {
        let old = std::mem::take(&mut self.entries);
        if let Err(e) = self.save() {
            self.entries = old;
            return Err(e);
        }
        Ok(())
    }

    fn save(&self) -> Result<()> {
        write_atomic(&self.path, &to_json(&HistoryFile { version: 1, entries: self.entries.clone() })?)
    }
}

// ---------------------------------------------------------------- Taslaklar

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum DraftMode {
    #[default]
    Autocommit,
    Manual,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct DraftOptions {
    pub mode: DraftMode,
    pub statement_timeout_ms: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct DraftTab {
    pub id: Uuid,
    pub title: String,
    pub server_id: Option<Uuid>,
    pub database_oid: Option<u32>,
    /// Görüntülenen veritabanı adı (taslak bağlanmadan geri yüklenince bağlam şeridinde gösterilir); eski dosyalarda yok.
    #[serde(default)]
    pub database: Option<String>,
    pub options: DraftOptions,
}

#[derive(Serialize, Deserialize)]
struct TabsFile {
    #[serde(default)]
    version: u32,
    tabs: Vec<DraftTab>,
}

pub struct Drafts {
    dir: PathBuf,
    /// `tabs.json` oku-değiştir-yaz döngülerini ve süpürmeyi sıralar (eşzamanlı komutlar sekme kaybetmesin).
    lock: Mutex<()>,
}

impl Drafts {
    pub fn new(dir: impl Into<PathBuf>) -> Self {
        Self { dir: dir.into(), lock: Mutex::new(()) }
    }

    fn guard(&self) -> std::sync::MutexGuard<'_, ()> {
        self.lock.lock().unwrap_or_else(|p| p.into_inner())
    }

    fn tabs_path(&self) -> PathBuf {
        self.dir.join("tabs.json")
    }

    fn sql_path(&self, id: Uuid) -> PathBuf {
        self.dir.join(format!("{id}.sql"))
    }

    fn read_tabs(&self) -> Result<Vec<DraftTab>> {
        Ok(read_json::<TabsFile>(&self.tabs_path())?.0.map(|f| f.tabs).unwrap_or_default())
    }

    fn write_tabs(&self, tabs: Vec<DraftTab>) -> Result<()> {
        write_atomic(&self.tabs_path(), &to_json(&TabsFile { version: 1, tabs })?)
    }

    /// Var olan sekmenin sırası korunur; yeni sekme sona eklenir. Önce metin yazılır: araya giren bir hata
    /// en kötü ihtimalle sahipsiz `.sql` bırakır, bu `load_all` sırasında süpürülür.
    pub fn save_tab(&self, tab: &DraftTab, sql: &str) -> Result<()> {
        let _g = self.guard();
        write_atomic(&self.sql_path(tab.id), sql.as_bytes())?;
        let mut tabs = self.read_tabs()?;
        match tabs.iter_mut().find(|t| t.id == tab.id) {
            Some(slot) => *slot = tab.clone(),
            None => tabs.push(tab.clone()),
        }
        self.write_tabs(tabs)
    }

    pub fn remove(&self, id: Uuid) -> Result<()> {
        let _g = self.guard();
        let mut tabs = self.read_tabs()?;
        tabs.retain(|t| t.id != id);
        self.write_tabs(tabs)?;
        match fs::remove_file(self.sql_path(id)) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(io_err(e)),
        }
    }

    /// `tabs.json` sırasıyla. Metni okunamayan taslak boş metinle gelir (günlüğe yalnızca kimlik yazılır);
    /// `tabs.json`ın anmadığı `<uuid>.sql` dosyaları süpürülür.
    pub fn load_all(&self) -> Result<Vec<(DraftTab, String)>> {
        let _g = self.guard();
        let tabs = self.read_tabs()?;
        self.sweep_orphans(&tabs);
        Ok(tabs
            .into_iter()
            .map(|t| {
                let sql = fs::read_to_string(self.sql_path(t.id)).unwrap_or_else(|e| {
                    tracing::warn!(draft = %t.id, error = %e, "taslak metni okunamadı; boş metinle açılıyor");
                    String::new()
                });
                (t, sql)
            })
            .collect())
    }

    fn sweep_orphans(&self, tabs: &[DraftTab]) {
        let Ok(entries) = fs::read_dir(&self.dir) else { return };
        for entry in entries.flatten() {
            let name = entry.file_name();
            let Some(stem) = name.to_str().and_then(|n| n.strip_suffix(".sql")) else { continue };
            if Uuid::parse_str(stem).is_ok_and(|id| !tabs.iter().any(|t| t.id == id)) {
                let _ = fs::remove_file(entry.path());
            }
        }
    }

    pub fn clear(&self) -> Result<()> {
        let _g = self.guard();
        let tabs = self.read_tabs()?;
        self.write_tabs(Vec::new())?;
        for t in tabs {
            let _ = fs::remove_file(self.sql_path(t.id));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(sql: &str) -> HistoryEntry {
        HistoryEntry {
            id: Uuid::new_v4(),
            sql: sql.into(),
            server_id: Uuid::new_v4(),
            server_name: "Üretim DB".into(),
            database: Some("app".into()),
            at_ms: 1,
            elapsed_ms: 2,
            ok: true,
        }
    }

    fn tab(title: &str) -> DraftTab {
        DraftTab { id: Uuid::new_v4(), title: title.into(), server_id: None, database_oid: Some(5), database: Some("app".into()), options: DraftOptions::default() }
    }

    #[test]
    fn keeps_only_the_newest_500() {
        let dir = tempfile::tempdir().unwrap();
        let mut h = History::load(dir.path().join("history.json")).unwrap();
        for i in 0..501 {
            h.push(entry(&format!("select {i}"))).unwrap();
        }
        let all = h.list(None, 1000);
        assert_eq!(all.len(), 500);
        assert_eq!(all[0].sql, "select 500");
        assert_eq!(all[499].sql, "select 1");
        let reloaded = History::load(dir.path().join("history.json")).unwrap();
        assert_eq!(reloaded.list(None, 1000).len(), 500);
    }

    #[test]
    fn truncates_on_a_utf8_boundary() {
        let long = "ş".repeat(MAX_HISTORY_SQL_BYTES); // 2 bayt/karakter
        let cut = truncate_sql(&long);
        assert!(cut.len() <= MAX_HISTORY_SQL_BYTES);
        assert!(cut.ends_with('…'));
        assert!(cut.trim_end_matches('…').chars().all(|c| c == 'ş'));
        let emoji = "😀".repeat(5000);
        let cut = truncate_sql(&emoji);
        assert!(cut.len() <= MAX_HISTORY_SQL_BYTES && cut.ends_with('…'));
        assert_eq!(truncate_sql("select 1"), "select 1");
        let exact = "a".repeat(MAX_HISTORY_SQL_BYTES);
        assert_eq!(truncate_sql(&exact), exact);
    }

    #[test]
    fn sensitive_sql_is_not_stored() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("history.json");
        let mut h = History::load(&path).unwrap();
        let stored = h.push_checked(entry("create role x password 'gizli'"), |s| s.contains("password")).unwrap();
        assert!(!stored);
        assert!(h.list(None, 10).is_empty());
        assert!(!path.exists());
    }

    #[test]
    fn disabled_history_writes_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("history.json");
        let mut h = History::load(&path).unwrap();
        h.set_enabled(false);
        assert!(!h.push(entry("select 1")).unwrap());
        assert!(!path.exists());
        assert!(h.list(None, 10).is_empty());
    }

    #[test]
    fn writes_are_atomic_and_leave_no_temp_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("history.json");
        let mut h = History::load(&path).unwrap();
        h.push(entry("select 1")).unwrap();
        assert!(!dir.path().join("history.json.tmp").exists());
        let names: Vec<_> = fs::read_dir(dir.path()).unwrap().map(|e| e.unwrap().file_name()).collect();
        assert_eq!(names.len(), 1);
        serde_json::from_str::<serde_json::Value>(&fs::read_to_string(&path).unwrap()).unwrap();
    }

    #[test]
    fn corrupt_history_is_backed_up() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("history.json");
        fs::write(&path, "{ yarım").unwrap();
        let mut h = History::load(&path).unwrap();
        let backup = h.recovered_backup().expect("yedek bekleniyordu").to_path_buf();
        assert_eq!(fs::read_to_string(backup).unwrap(), "{ yarım");
        assert!(h.list(None, 10).is_empty());
        h.push(entry("select 1")).unwrap();
        assert_eq!(h.list(None, 10).len(), 1);
    }

    #[test]
    fn list_filters_and_clear_empties() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("history.json");
        let mut h = History::load(&path).unwrap();
        h.push(entry("SELECT * FROM müşteri WHERE ad = 'Ankara 😀'")).unwrap();
        h.push(entry("select 2")).unwrap();
        assert_eq!(h.list(Some("MÜŞTERİ"), 10).len(), 0); // basit küçük harf eşlemesi: İ ≠ i
        assert_eq!(h.list(Some("MÜŞTERI"), 10).len(), 1);
        assert_eq!(h.list(Some("😀"), 10).len(), 1);
        assert_eq!(h.list(Some("select"), 1).len(), 1);
        h.clear().unwrap();
        assert!(History::load(&path).unwrap().list(None, 10).is_empty());
    }

    #[test]
    fn settings_default_on_and_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        assert!(AppSettings::load(&path).history_enabled);
        AppSettings { history_enabled: false }.save(&path).unwrap();
        assert!(!AppSettings::load(&path).history_enabled);
        fs::write(&path, "{}").unwrap();
        assert!(AppSettings::load(&path).history_enabled);
        fs::write(&path, "bozuk").unwrap();
        assert!(AppSettings::load(&path).history_enabled);
    }

    #[test]
    fn drafts_round_trip_keep_order_and_remove() {
        let dir = tempfile::tempdir().unwrap();
        let drafts = Drafts::new(dir.path().join("query"));
        let (a, b, c) = (tab("a"), tab("b"), tab("c"));
        let sql_b = "select 'Çağrı 😀', \"ş\";\n-- satır";
        drafts.save_tab(&a, "select 1").unwrap();
        drafts.save_tab(&b, sql_b).unwrap();
        drafts.save_tab(&c, "").unwrap();
        drafts.save_tab(&a, "select 11").unwrap(); // güncelleme sırayı bozmaz
        let all = drafts.load_all().unwrap();
        assert_eq!(all.iter().map(|(t, _)| t.title.as_str()).collect::<Vec<_>>(), ["a", "b", "c"]);
        assert_eq!(all[0].1, "select 11");
        assert_eq!(all[1].1, sql_b);
        assert_eq!(all[1].0, b);
        drafts.remove(b.id).unwrap();
        assert!(!dir.path().join("query").join(format!("{}.sql", b.id)).exists());
        assert_eq!(drafts.load_all().unwrap().len(), 2);
        drafts.remove(b.id).unwrap(); // olmayanı silmek hata değil
        drafts.clear().unwrap();
        assert!(drafts.load_all().unwrap().is_empty());
        assert!(!dir.path().join("query").join(format!("{}.sql", a.id)).exists());
    }

    #[test]
    fn old_tabs_json_without_database_name_loads() {
        let dir = tempfile::tempdir().unwrap();
        let drafts = Drafts::new(dir.path());
        let id = Uuid::new_v4();
        let json = format!(
            r#"{{"version":1,"tabs":[{{"id":"{id}","title":"t","serverId":null,"databaseOid":7,"options":{{"mode":"manual","statementTimeoutMs":0}}}}]}}"#
        );
        fs::write(dir.path().join("tabs.json"), json).unwrap();
        let all = drafts.load_all().unwrap();
        assert_eq!(all.len(), 1);
        assert_eq!((all[0].0.database_oid, all[0].0.database.as_deref()), (Some(7), None));
    }

    #[test]
    fn corrupt_tabs_json_is_backed_up_and_empty() {
        let dir = tempfile::tempdir().unwrap();
        let drafts = Drafts::new(dir.path());
        fs::write(dir.path().join("tabs.json"), "bozuk").unwrap();
        assert!(drafts.load_all().unwrap().is_empty());
        let backups = fs::read_dir(dir.path())
            .unwrap()
            .filter(|e| e.as_ref().unwrap().file_name().to_string_lossy().starts_with("tabs.json.bak-"))
            .count();
        assert_eq!(backups, 1);
        drafts.save_tab(&tab("x"), "select 1").unwrap();
        assert_eq!(drafts.load_all().unwrap().len(), 1);
    }

    #[test]
    fn concurrent_saves_and_removes_lose_no_tabs() {
        let dir = tempfile::tempdir().unwrap();
        let drafts = std::sync::Arc::new(Drafts::new(dir.path().join("query")));
        let tabs: Vec<DraftTab> = (0..16).map(|i| tab(&format!("t{i}"))).collect();
        let handles: Vec<_> = tabs
            .iter()
            .cloned()
            .enumerate()
            .map(|(i, t)| {
                let d = drafts.clone();
                std::thread::spawn(move || {
                    d.save_tab(&t, &format!("select {i}")).unwrap();
                    if i % 2 == 1 {
                        d.remove(t.id).unwrap();
                    }
                })
            })
            .collect();
        for h in handles {
            h.join().unwrap();
        }
        let all = drafts.load_all().unwrap();
        let mut got: Vec<Uuid> = all.iter().map(|(t, _)| t.id).collect();
        let mut want: Vec<Uuid> = tabs.iter().enumerate().filter(|(i, _)| i % 2 == 0).map(|(_, t)| t.id).collect();
        got.sort();
        want.sort();
        assert_eq!(got, want);
        for (t, sql) in &all {
            let i = tabs.iter().position(|x| x.id == t.id).unwrap();
            assert_eq!(sql, &format!("select {i}"));
        }
        let leftovers = fs::read_dir(dir.path().join("query")).unwrap().filter(|e| e.as_ref().unwrap().file_name().to_string_lossy().ends_with(".tmp")).count();
        assert_eq!(leftovers, 0);
    }

    #[test]
    fn orphan_sql_files_are_swept_on_load() {
        let dir = tempfile::tempdir().unwrap();
        let drafts = Drafts::new(dir.path());
        let keep = tab("k");
        drafts.save_tab(&keep, "select 1").unwrap();
        let orphan = dir.path().join(format!("{}.sql", Uuid::new_v4()));
        fs::write(&orphan, "x").unwrap();
        let other = dir.path().join("notlar.sql");
        fs::write(&other, "x").unwrap();
        assert_eq!(drafts.load_all().unwrap().len(), 1);
        assert!(!orphan.exists());
        assert!(other.exists());
    }

    #[test]
    fn failed_write_removes_its_temp_file() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("hedef");
        fs::create_dir(&target).unwrap(); // rename dosya -> dizin üzerine başarısız olur
        assert!(write_atomic(&target, b"x").is_err());
        let names: Vec<_> = fs::read_dir(dir.path()).unwrap().map(|e| e.unwrap().file_name()).collect();
        assert_eq!(names.len(), 1);
    }
}
