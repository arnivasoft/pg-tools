//! Hücre görüntüleyici komutları (`cell_*`; spec 2026-11-05 §2.4, Plan 31 Task 1).
//!
//! - **PDF açma:** `bytea` metni çözülür, `%PDF-` imzası doğrulanır, `app_cache_dir()/cell-view` altına benzersiz adla
//!   yazılır ve Windows Gezgini aracılığıyla varsayılan uygulamada açılır. Dosya kapanışta silinir; açılışta 24 saatten
//!   eskiler süpürülür.
//! - **Kaydetme:** çözülen baytlar kullanıcının seçtiği yola yazılır (üzerine yazma onayı yerel diyalogda alınmıştır).
//! - **Gizlilik:** bu katman `tracing` yazmaz; değer, bayt ve yollar iletilere girmez.
//! - **Hata kodları:** `too_large`, `invalid`, `write_failed`.

use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, SystemTime};

use pgt_core::ErrorPayload;
use tauri::Manager;

type CmdResult<T> = Result<T, ErrorPayload>;

/// Dosya işlemleri için çözülmüş bayt üst sınırı.
pub const MAX_FILE_BYTES: usize = 64 * 1024 * 1024;
/// Açılışta bu süreden eski geçici dosyalar silinir.
pub const STALE_AFTER: Duration = Duration::from_secs(24 * 60 * 60);

const MSG_BAD_BYTEA: &str = "Değer çözülemedi (geçersiz bytea).";
const MSG_NOT_PDF: &str = "Değer bir PDF belgesi değil.";
const MSG_TOO_LARGE: &str = "Değer dosya işlemi için çok büyük (sınır 64 MB).";
const MSG_WRITE: &str = "Dosya yazılamadı.";

fn payload(code: &str, message: &str) -> ErrorPayload {
    ErrorPayload { code: code.to_string(), message: message.to_string(), detail: None, hint: None }
}

/// `\x` önekli onaltılık metnin çözülmüş bayt sayısı; önek yoksa `None` (çözme yapılmaz).
pub fn decoded_len(text: &str) -> Option<usize> {
    text.strip_prefix("\\x").map(|h| h.len() / 2)
}

/// `\x…` bytea metnini baytlara çözer: önek zorunlu, çift uzunluk, yalnız onaltılık rakam.
pub fn decode_bytea_hex(text: &str) -> Result<Vec<u8>, &'static str> {
    let hex = text.strip_prefix("\\x").ok_or(MSG_BAD_BYTEA)?;
    let b = hex.as_bytes();
    if b.len() % 2 != 0 {
        return Err(MSG_BAD_BYTEA);
    }
    let nibble = |c: u8| -> Result<u8, &'static str> {
        match c {
            b'0'..=b'9' => Ok(c - b'0'),
            b'a'..=b'f' => Ok(c - b'a' + 10),
            b'A'..=b'F' => Ok(c - b'A' + 10),
            _ => Err(MSG_BAD_BYTEA),
        }
    };
    b.as_chunks::<2>().0.iter().map(|[hi, lo]| Ok(nibble(*hi)? << 4 | nibble(*lo)?)).collect()
}

/// PDF imzası.
pub fn is_pdf(bytes: &[u8]) -> bool {
    bytes.starts_with(b"%PDF-")
}

/// Gezgin bağımsız değişkeni: tırnaklı yol (saf; yol günlüğe yazılmaz). Tırnak ve denetim karakteri reddedilir;
/// Windows'ta yol mutlak olmalıdır.
pub fn path_arg(path: &str) -> Result<String, &'static str> {
    if path.chars().any(|c| c == '"' || c.is_control()) {
        return Err("Yol geçersiz karakter içeriyor.");
    }
    #[cfg(windows)]
    if !Path::new(path).is_absolute() {
        return Err("Tam (mutlak) yol gerekir.");
    }
    Ok(format!("\"{path}\""))
}

/// Geçici PDF dosya adı.
pub fn temp_file_name(id: uuid::Uuid) -> String {
    format!("cell-{id}.pdf")
}

/// `dir` altında adı `cell-` ile başlayıp `.pdf` ile biten **dosyalardan** `max_age`'den eski olanları siler;
/// silinen sayısını döndürür. Dizin yoksa 0. Hatalar sessizdir.
pub fn sweep_stale(dir: &Path, now: SystemTime, max_age: Duration) -> usize {
    let Ok(entries) = std::fs::read_dir(dir) else { return 0 };
    let mut removed = 0;
    for entry in entries.flatten() {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if !(name.starts_with("cell-") && name.ends_with(".pdf")) {
            continue;
        }
        let Ok(meta) = entry.metadata() else { continue };
        if !meta.is_file() {
            continue;
        }
        let Ok(modified) = meta.modified() else { continue };
        if modified + max_age < now && std::fs::remove_file(entry.path()).is_ok() {
            removed += 1;
        }
    }
    removed
}

/// Bu oturumda açılan geçici PDF'ler (kapanışta silinir).
#[derive(Default)]
pub struct CellTemp {
    files: Mutex<Vec<PathBuf>>,
}

impl CellTemp {
    pub fn remember(&self, path: PathBuf) {
        self.files.lock().unwrap_or_else(|e| e.into_inner()).push(path);
    }

    /// Kayıtlı dosyaları siler; gerçekten silinen sayısını döndürür. Hata sessizdir.
    pub fn remove_all(&self) -> usize {
        let files = std::mem::take(&mut *self.files.lock().unwrap_or_else(|e| e.into_inner()));
        files.iter().filter(|p| std::fs::remove_file(p).is_ok()).count()
    }
}

fn cache_dir(app: &tauri::AppHandle) -> Option<PathBuf> {
    app.path().app_cache_dir().ok().map(|d| d.join("cell-view"))
}

/// Uygulama başlarken: durum kaydı + eski geçici dosyaların süpürülmesi.
pub fn setup(app: &tauri::App) {
    app.manage(CellTemp::default());
    if let Some(dir) = cache_dir(app.handle()) {
        sweep_stale(&dir, SystemTime::now(), STALE_AFTER);
    }
}

/// Kapanışta bu oturumun geçici dosyalarını siler.
pub fn on_exit(app: &tauri::AppHandle) {
    app.state::<CellTemp>().remove_all();
}

/// Boyut denetimi (çözmeden) + çözme.
fn decode_checked(data: &str) -> CmdResult<Vec<u8>> {
    if decoded_len(data).is_some_and(|n| n > MAX_FILE_BYTES) {
        return Err(payload("too_large", MSG_TOO_LARGE));
    }
    decode_bytea_hex(data).map_err(|m| payload("invalid", m))
}

/// `bytea` PDF'ini geçici dosyaya yazıp varsayılan uygulamada açar (beklenmez).
#[tauri::command]
pub async fn cell_open_pdf(app: tauri::AppHandle, temp: tauri::State<'_, CellTemp>, data: String) -> CmdResult<()> {
    let bytes = decode_checked(&data)?;
    if !is_pdf(&bytes) {
        return Err(payload("invalid", MSG_NOT_PDF));
    }
    #[cfg(windows)]
    {
        use std::io::Write;
        use std::os::windows::process::CommandExt;
        let dir = cache_dir(&app).ok_or_else(|| payload("write_failed", MSG_WRITE))?;
        std::fs::create_dir_all(&dir).map_err(|_| payload("write_failed", MSG_WRITE))?;
        let path = dir.join(temp_file_name(uuid::Uuid::new_v4()));
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
            .map_err(|_| payload("write_failed", MSG_WRITE))?;
        let written = file.write_all(&bytes).and_then(|_| file.flush());
        drop(file);
        // Yarım yazılmış dosya da kaydedilir ki kapanışta silinsin.
        temp.remember(path.clone());
        written.map_err(|_| payload("write_failed", MSG_WRITE))?;
        let arg = path_arg(&path.to_string_lossy()).map_err(|m| payload("invalid", m))?;
        let root = std::env::var_os("SystemRoot").unwrap_or_else(|| "C:\\Windows".into());
        let exe = Path::new(&root).join("explorer.exe");
        std::process::Command::new(exe)
            .raw_arg(arg)
            .spawn()
            .map(drop)
            .map_err(|_| payload("invalid", "Varsayılan uygulama başlatılamadı."))
    }
    #[cfg(not(windows))]
    {
        let _ = (app, temp);
        Err(payload("invalid", "Bu işlem yalnız Windows'ta desteklenir."))
    }
}

/// Çözülen baytları `path`'e yazar (varsa üzerine yazar).
#[tauri::command]
pub async fn cell_save_bytes(data: String, path: String) -> CmdResult<()> {
    path_arg(&path).map_err(|m| payload("invalid", m))?;
    let bytes = decode_checked(&data)?;
    std::fs::write(&path, bytes).map_err(|_| payload("write_failed", MSG_WRITE))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decode_hex_accepts_both_cases_and_rejects_bad_input() {
        assert_eq!(decode_bytea_hex("\\x"), Ok(vec![]));
        assert_eq!(decode_bytea_hex("\\x89504e47"), Ok(vec![0x89, 0x50, 0x4E, 0x47]));
        assert_eq!(decode_bytea_hex("\\x89504E47"), Ok(vec![0x89, 0x50, 0x4E, 0x47]));
        assert!(decode_bytea_hex("\\x123").is_err());
        assert!(decode_bytea_hex("\\xzz").is_err());
        assert!(decode_bytea_hex("89504e47").is_err());
    }

    #[test]
    fn decoded_len_needs_prefix() {
        assert_eq!(decoded_len("\\x0a0b"), Some(2));
        assert_eq!(decoded_len("abc"), None);
    }

    #[test]
    fn pdf_signature() {
        assert!(is_pdf(b"%PDF-1.7\n"));
        assert!(!is_pdf(b" %PDF-"));
        assert!(!is_pdf(b""));
    }

    #[test]
    fn path_arg_quotes_absolute_paths_and_rejects_bad_ones() {
        #[cfg(windows)]
        {
            assert_eq!(path_arg("C:\\Temp\\ş ı.pdf"), Ok("\"C:\\Temp\\ş ı.pdf\"".to_string()));
            assert_eq!(path_arg("temp\\a.pdf"), Err("Tam (mutlak) yol gerekir."));
        }
        assert_eq!(path_arg("C:\\x\"y"), Err("Yol geçersiz karakter içeriyor."));
        assert_eq!(path_arg("C:\\x\ny"), Err("Yol geçersiz karakter içeriyor."));
    }

    #[test]
    fn temp_name_is_stable() {
        assert_eq!(temp_file_name(uuid::Uuid::nil()), "cell-00000000-0000-0000-0000-000000000000.pdf");
    }

    #[test]
    fn sweep_removes_only_stale_cell_pdf_files() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path();
        std::fs::write(p.join("cell-a.pdf"), b"x").unwrap();
        std::fs::write(p.join("other.pdf"), b"x").unwrap();
        std::fs::write(p.join("cell-c.txt"), b"x").unwrap();
        std::fs::create_dir(p.join("cell-d.pdf")).unwrap();
        let later = SystemTime::now() + Duration::from_secs(25 * 3600);
        assert_eq!(sweep_stale(p, later, STALE_AFTER), 1);
        assert!(!p.join("cell-a.pdf").exists());
        assert!(p.join("other.pdf").exists());
        assert!(p.join("cell-c.txt").exists());
        assert!(p.join("cell-d.pdf").is_dir());
        // Yeni dosya (şimdiki zamana göre) silinmez.
        std::fs::write(p.join("cell-b.pdf"), b"x").unwrap();
        assert_eq!(sweep_stale(p, SystemTime::now(), STALE_AFTER), 0);
        assert!(p.join("cell-b.pdf").exists());
        assert_eq!(sweep_stale(&p.join("yok"), later, STALE_AFTER), 0);
    }

    #[test]
    fn cell_temp_removes_only_remembered_files() {
        let dir = tempfile::tempdir().unwrap();
        let (a, b, c) = (dir.path().join("a.pdf"), dir.path().join("b.pdf"), dir.path().join("c.pdf"));
        for f in [&a, &b, &c] {
            std::fs::write(f, b"x").unwrap();
        }
        let temp = CellTemp::default();
        temp.remember(a.clone());
        temp.remember(b.clone());
        std::fs::remove_file(&a).unwrap();
        assert_eq!(temp.remove_all(), 1);
        assert_eq!(temp.remove_all(), 0);
        assert!(!b.exists());
        assert!(c.exists());
    }
}
