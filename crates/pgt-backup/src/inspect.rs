//! Geri yükleme kaynağını inceleme (spec §1 "Geri yükleme / Kaynak"): format içerikten algılanır, arşivlerde
//! `pg_restore --list` özeti okunur. Kullanıcı formatı zorlayamaz.

use std::io::Read;
use std::path::Path;

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::detect::{ArchiveInfo, SourceFormat, is_gzip, parse_list, sniff_format};
use crate::model::{ToolKind, ToolSet};

/// İncelenen kaynak. `archive` yalnız custom/directory/tar için doludur.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct RestoreSource {
    pub path: String,
    pub format: SourceFormat,
    pub archive: Option<ArchiveInfo>,
    pub is_dir: bool,
}

/// Format algılamada okunan baş bayt sayısı (tar başlığı 512 bayttır).
const HEAD_BYTES: usize = 512;

pub const NO_TOC_MESSAGE: &str = "Klasörde toc.dat yok; directory formatında alınmış bir pg_dump yedeği seçin.";
pub const GZIP_MESSAGE: &str =
    "Dosya gzip ile sıkıştırılmış (.sql.gz); psql bunu doğrudan okuyamaz. Önce açın (ör. 7-Zip) ve .sql dosyasını seçin.";

/// Kaynağın formatını algılar (engelleyici dosya sistemi G/Ç'si). `toc.dat`'sız klasör ve gzip'li düz SQL net
/// iletiyle reddedilir. İletiler yol içermez.
pub fn sniff_path(path: &str) -> Result<(SourceFormat, bool), String> {
    if path.trim().is_empty() || path.contains('\0') {
        return Err("Kaynak yolu geçersiz.".into());
    }
    let p = Path::new(path);
    if !p.is_absolute() {
        return Err("Kaynak için tam (mutlak) yol gerekir.".into());
    }
    let meta = std::fs::metadata(p).map_err(|e| match e.kind() {
        std::io::ErrorKind::NotFound => "Kaynak bulunamadı.".to_string(),
        _ => format!("Kaynak okunamadı: {e}"),
    })?;
    if meta.is_dir() {
        let has_toc = p.join("toc.dat").is_file();
        return match sniff_format(&[], true, has_toc) {
            SourceFormat::Directory => Ok((SourceFormat::Directory, true)),
            _ => Err(NO_TOC_MESSAGE.into()),
        };
    }
    let mut head = Vec::with_capacity(HEAD_BYTES);
    std::fs::File::open(p)
        .and_then(|f| f.take(HEAD_BYTES as u64).read_to_end(&mut head))
        .map_err(|e| format!("Kaynak okunamadı: {e}"))?;
    let format = sniff_format(&head, false, false);
    if format == SourceFormat::Plain && is_gzip(&head) {
        return Err(GZIP_MESSAGE.into());
    }
    Ok((format, false))
}

/// `pg_restore --list` için format adı.
pub(crate) fn format_arg(format: SourceFormat) -> &'static str {
    match format {
        SourceFormat::Custom => "custom",
        SourceFormat::Directory => "directory",
        SourceFormat::Tar => "tar",
        SourceFormat::Plain => "plain",
    }
}

/// Kaynağı inceler: format algılanır; arşivde en yeni `pg_restore` ile özet okunur (sunucuya bağlanılmaz).
pub async fn inspect(path: &str, sets: &[ToolSet]) -> Result<RestoreSource, String> {
    let owned = path.to_string();
    let (format, is_dir) = tokio::task::spawn_blocking(move || sniff_path(&owned))
        .await
        .map_err(|_| "Kaynak okunamadı.".to_string())??;
    let archive = if format == SourceFormat::Plain {
        None
    } else {
        let tool = crate::tools::choose(sets, ToolKind::PgRestore, 0)?;
        let exe = crate::tools::exe_path(&tool.dir, ToolKind::PgRestore).to_string_lossy().into_owned();
        let listing = crate::runner::list_archive(&exe, format_arg(format), path).await?;
        Some(parse_list(&listing))
    };
    Ok(RestoreSource { path: path.to_string(), format, archive, is_dir })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_dir_without_toc_and_gzip_plain() {
        let dir = tempfile::tempdir().unwrap();
        let d = dir.path().to_string_lossy().into_owned();
        assert_eq!(sniff_path(&d).unwrap_err(), NO_TOC_MESSAGE);
        std::fs::write(dir.path().join("toc.dat"), b"PGDMP").unwrap();
        assert_eq!(sniff_path(&d).unwrap(), (SourceFormat::Directory, true));

        let gz = dir.path().join("a.sql.gz");
        std::fs::write(&gz, [0x1f, 0x8b, 8, 0, 0, 0]).unwrap();
        assert_eq!(sniff_path(&gz.to_string_lossy()).unwrap_err(), GZIP_MESSAGE);

        let sql = dir.path().join("a.sql");
        std::fs::write(&sql, b"SELECT 1;\n").unwrap();
        assert_eq!(sniff_path(&sql.to_string_lossy()).unwrap(), (SourceFormat::Plain, false));
        let custom = dir.path().join("a.dump");
        std::fs::write(&custom, b"PGDMP\x01\x0e").unwrap();
        assert_eq!(sniff_path(&custom.to_string_lossy()).unwrap(), (SourceFormat::Custom, false));

        assert_eq!(sniff_path(&dir.path().join("yok").to_string_lossy()).unwrap_err(), "Kaynak bulunamadı.");
        assert!(sniff_path("göreli.sql").unwrap_err().contains("mutlak"));
        assert!(sniff_path("").is_err());
    }

    #[tokio::test]
    async fn inspect_plain_needs_no_tool_and_archive_needs_pg_restore() {
        let dir = tempfile::tempdir().unwrap();
        let sql = dir.path().join("a.sql");
        std::fs::write(&sql, b"SELECT 1;\n").unwrap();
        let src = inspect(&sql.to_string_lossy(), &[]).await.unwrap();
        assert_eq!((src.format, src.archive, src.is_dir), (SourceFormat::Plain, None, false));
        let custom = dir.path().join("a.dump");
        std::fs::write(&custom, b"PGDMP\x01\x0e").unwrap();
        let err = inspect(&custom.to_string_lossy(), &[]).await.unwrap_err();
        assert!(err.contains("pg_restore"), "{err}");
    }
}
