//! CSV dışa aktarma: sonuç ikinci bir sunucu imleciyle (`pgt_x`) 5.000'er satırlık parçalarla akıtılır; bellekte
//! en çok bir parça tutulur. Dosya `<hedef>.tmp` olarak yazılır, başarıda yeniden adlandırılır.
//! SQL metni ve parola hata iletilerine ve günlüğe girmez.

use std::io::Write;
use std::path::{Path, PathBuf};

use pgt_core::{Error, Result};
use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// Tek `FETCH` parçasındaki satır sayısı.
pub const EXPORT_CHUNK: u32 = 5_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum CsvDelimiter {
    #[default]
    Comma,
    Semicolon,
    Tab,
}

impl CsvDelimiter {
    fn byte(self) -> u8 {
        match self {
            CsvDelimiter::Comma => b',',
            CsvDelimiter::Semicolon => b';',
            CsvDelimiter::Tab => b'\t',
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase", default)]
#[ts(export)]
pub struct CsvOptions {
    pub delimiter: CsvDelimiter,
    /// İlk satır sütun adları.
    pub header: bool,
    /// UTF-8 BOM (Excel için varsayılan açık).
    pub bom: bool,
}

impl Default for CsvOptions {
    fn default() -> Self {
        CsvOptions { delimiter: CsvDelimiter::Comma, header: true, bom: true }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ExportSummary {
    pub rows: u64,
    pub bytes: u64,
    pub elapsed_ms: u64,
}

const BOM: &[u8] = b"\xEF\xBB\xBF";

/// Bir alanı yazar: `NULL` boş (tırnaksız), boş dize `""`; ayırıcı, `"`, CR ya da LF içeren alan tırnaklanır
/// (`"` → `""`). Yazılan bayt sayısını döner.
pub(crate) fn write_field(out: &mut impl Write, value: Option<&str>, delimiter: u8) -> std::io::Result<u64> {
    let Some(v) = value else { return Ok(0) };
    if v.is_empty() {
        out.write_all(b"\"\"")?;
        return Ok(2);
    }
    let needs = v.bytes().any(|b| b == delimiter || b == b'"' || b == b'\r' || b == b'\n');
    if !needs {
        out.write_all(v.as_bytes())?;
        return Ok(v.len() as u64);
    }
    let mut n = 2u64;
    out.write_all(b"\"")?;
    let mut rest = v;
    while let Some(i) = rest.find('"') {
        out.write_all(&rest.as_bytes()[..=i])?;
        out.write_all(b"\"")?;
        n += i as u64 + 2;
        rest = &rest[i + 1..];
    }
    out.write_all(rest.as_bytes())?;
    out.write_all(b"\"")?;
    Ok(n + rest.len() as u64)
}

/// Bir kaydı (`\r\n` sonlu) yazar; yazılan bayt sayısını döner.
pub(crate) fn write_record<'a>(
    out: &mut impl Write,
    fields: impl IntoIterator<Item = Option<&'a str>>,
    delimiter: u8,
) -> std::io::Result<u64> {
    let mut n = 0u64;
    for (i, f) in fields.into_iter().enumerate() {
        if i > 0 {
            out.write_all(&[delimiter])?;
            n += 1;
        }
        n += write_field(out, f, delimiter)?;
    }
    out.write_all(b"\r\n")?;
    Ok(n + 2)
}

/// Dosya yazıcısı: sayaçlı `BufWriter`; BOM ve başlığı kendisi yazar. Başarıyla `commit` edilmeden bırakılırsa
/// `.tmp` silinir; hedefe dokunulmaz.
pub(crate) struct CsvFile {
    out: Option<std::io::BufWriter<std::fs::File>>,
    tmp: PathBuf,
    target: PathBuf,
    delimiter: u8,
    pub(crate) bytes: u64,
}

fn io_error(what: &str, path: &Path, e: &std::io::Error) -> Error {
    Error::Config(format!("{what}: {} ({})", path.display(), e.kind()))
}

pub(crate) fn tmp_path(path: &Path) -> PathBuf {
    let mut s = path.as_os_str().to_os_string();
    s.push(".tmp");
    PathBuf::from(s)
}

/// Hedef dizin var mı (yazılabilirlik dosya açılırken sınanır).
pub(crate) fn check_target(path: &Path) -> Result<()> {
    let parent = match path.parent() {
        Some(p) if !p.as_os_str().is_empty() => p,
        _ => Path::new("."),
    };
    if path.file_name().is_none() || path.is_dir() {
        return Err(Error::Config(format!("Geçersiz dışa aktarma dosyası: {}", path.display())));
    }
    if !parent.is_dir() {
        return Err(Error::Config(format!("Hedef dizin yok: {}", parent.display())));
    }
    Ok(())
}

impl CsvFile {
    pub(crate) fn create(path: &Path, opts: &CsvOptions, columns: &[String]) -> Result<Self> {
        let tmp = tmp_path(path);
        let file = std::fs::File::create(&tmp).map_err(|e| io_error("Dosya oluşturulamadı", &tmp, &e))?;
        let mut f = CsvFile {
            out: Some(std::io::BufWriter::new(file)),
            tmp,
            target: path.to_path_buf(),
            delimiter: opts.delimiter.byte(),
            bytes: 0,
        };
        f.write_head(opts, columns)?;
        Ok(f)
    }

    fn write_head(&mut self, opts: &CsvOptions, columns: &[String]) -> Result<()> {
        let delimiter = self.delimiter;
        let mut n = 0u64;
        let r = (|| -> std::io::Result<()> {
            let out = self.out.as_mut().expect("açık yazıcı");
            if opts.bom {
                out.write_all(BOM)?;
                n += BOM.len() as u64;
            }
            if opts.header {
                n += write_record(out, columns.iter().map(|c| Some(c.as_str())), delimiter)?;
            }
            Ok(())
        })();
        self.bytes += n;
        r.map_err(|e| io_error("Dosya yazılamadı", &self.tmp, &e))
    }

    pub(crate) fn write_rows(&mut self, rows: &[Vec<Option<String>>]) -> Result<()> {
        let delimiter = self.delimiter;
        let out = self.out.as_mut().expect("açık yazıcı");
        let mut total = 0u64;
        let mut r = Ok(());
        for row in rows {
            match write_record(out, row.iter().map(Option::as_deref), delimiter) {
                Ok(n) => total += n,
                Err(e) => {
                    r = Err(e);
                    break;
                }
            }
        }
        self.bytes += total;
        r.map_err(|e| io_error("Dosya yazılamadı", &self.tmp, &e))
    }

    /// Yazmayı bitirir ve `.tmp`'yi hedefe yeniden adlandırır.
    pub(crate) fn commit(mut self) -> Result<()> {
        let mut out = self.out.take().expect("açık yazıcı");
        out.flush().map_err(|e| io_error("Dosya yazılamadı", &self.tmp, &e))?;
        drop(out); // Windows'ta açık tutamaç yeniden adlandırmayı engeller.
        match std::fs::rename(&self.tmp, &self.target) {
            Ok(()) => {
                // Drop'ta silinmesin.
                self.tmp = PathBuf::new();
                Ok(())
            }
            Err(e) => Err(io_error("Dosya yeniden adlandırılamadı", &self.target, &e)),
        }
    }
}

impl Drop for CsvFile {
    fn drop(&mut self) {
        drop(self.out.take());
        if !self.tmp.as_os_str().is_empty() {
            let _ = std::fs::remove_file(&self.tmp);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn field(v: Option<&str>, d: u8) -> String {
        let mut out = Vec::new();
        let n = write_field(&mut out, v, d).unwrap();
        assert_eq!(n as usize, out.len());
        String::from_utf8(out).unwrap()
    }

    #[test]
    fn csv_quoting_rules() {
        assert_eq!(field(Some("abc"), b','), "abc");
        assert_eq!(field(Some("a,b"), b','), "\"a,b\"");
        assert_eq!(field(Some("a;b"), b','), "a;b");
        assert_eq!(field(Some("a;b"), b';'), "\"a;b\"");
        assert_eq!(field(Some("a\tb"), b'\t'), "\"a\tb\"");
        assert_eq!(field(Some("a\tb"), b','), "a\tb");
        assert_eq!(field(Some("he said \"hi\""), b','), "\"he said \"\"hi\"\"\"");
        assert_eq!(field(Some("\""), b','), "\"\"\"\"");
        assert_eq!(field(Some("x\ny"), b','), "\"x\ny\"");
        assert_eq!(field(Some("x\ry"), b','), "\"x\ry\"");
        assert_eq!(field(Some(" lead trail "), b','), " lead trail ");
        assert_eq!(field(None, b','), "");
        assert_eq!(field(Some(""), b','), "\"\"");
        assert_eq!(field(Some("çğşıöü 日本語 🙂"), b','), "çğşıöü 日本語 🙂");
        assert_eq!(field(Some("ğ,\"ü"), b','), "\"ğ,\"\"ü\"");
    }

    #[test]
    fn records_use_crlf_and_distinguish_null_from_empty() {
        let mut out = Vec::new();
        let n = write_record(&mut out, [Some("a"), None, Some(""), Some("b,c")], b',').unwrap();
        assert_eq!(out, b"a,,\"\",\"b,c\"\r\n");
        assert_eq!(n as usize, out.len());
    }

    #[test]
    fn file_has_bom_header_and_atomic_rename() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("o.csv");
        std::fs::write(&path, "old").unwrap();
        let cols = vec!["id".to_string(), "n;ame".to_string()];
        let opts = CsvOptions { delimiter: CsvDelimiter::Semicolon, header: true, bom: true };
        let mut f = CsvFile::create(&path, &opts, &cols).unwrap();
        f.write_rows(&[vec![Some("1".into()), None]]).unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"old", "hedefe yalnızca başarıda dokunulur");
        let bytes = f.bytes;
        f.commit().unwrap();
        let got = std::fs::read(&path).unwrap();
        assert_eq!(got, b"\xEF\xBB\xBFid;\"n;ame\"\r\n1;\r\n");
        assert_eq!(bytes as usize, got.len());
        assert!(!tmp_path(&path).exists());
    }

    #[test]
    fn dropped_file_removes_tmp_and_keeps_target() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("o.csv");
        std::fs::write(&path, "old").unwrap();
        let opts = CsvOptions { header: false, bom: false, ..CsvOptions::default() };
        drop(CsvFile::create(&path, &opts, &[]).unwrap());
        assert!(!tmp_path(&path).exists());
        assert_eq!(std::fs::read(&path).unwrap(), b"old");
    }

    #[test]
    fn missing_directory_is_a_config_error_with_path() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("yok").join("o.csv");
        let Err(Error::Config(msg)) = check_target(&path) else { panic!("Config bekleniyordu") };
        assert!(msg.contains("yok"), "{msg}");
    }
}
