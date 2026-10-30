//! Kaynak format algılama ve `pg_restore -l` arşiv özeti (spec §1 "Geri yükleme / Kaynak").
//!
//! Format yalnız içerikten belirlenir; kullanıcı formatı zorlayamaz (custom dosya psql'e, düz SQL pg_restore'a
//! verilemez).

use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// Algılanan kaynak formatı.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum SourceFormat {
    Custom,
    Directory,
    Tar,
    Plain,
}

/// `pg_restore -l` özeti.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ArchiveInfo {
    /// Arşivi üreten `pg_dump` sürümü ("Dumped by pg_dump version"); `pg_restore` en az bu sürüm olmalı.
    pub archive_version: Option<String>,
    /// Kaynak sunucu sürümü ("Dumped from database version").
    pub server_version: Option<String>,
    /// Kaynak veritabanı adı.
    pub db_name: Option<String>,
    /// Listelenen öğe sayısı (geri yükleme ilerlemesinin toplamı).
    pub items: u32,
    /// Öğe türü → sayı; çoktan aza, eşitlikte ada göre.
    pub counts: Vec<(String, u32)>,
}

/// Kaynak formatı: klasörde `toc.dat` → directory; dosya başı `PGDMP` → custom; tar başlığındaki ilk üye adı
/// `toc.dat` → tar (pg_dump tar arşivinde ilk üye budur); aksi plain.
/// `head` dosyanın ilk baytlarıdır (tar için en az 100 bayt; 512 önerilir). `toc.dat`'sız klasör `Plain` döner:
/// çağıran bunu ayrıca reddetmelidir (psql'e klasör verilemez).
pub fn sniff_format(head: &[u8], is_dir: bool, dir_has_toc: bool) -> SourceFormat {
    if is_dir {
        return if dir_has_toc {
            SourceFormat::Directory
        } else {
            SourceFormat::Plain
        };
    }
    if head.starts_with(b"PGDMP") {
        return SourceFormat::Custom;
    }
    // ustar başlığı: ad alanı 0..100, NUL ile biter.
    if head.len() >= 100 {
        let field = &head[..100];
        let name = &field[..field.iter().position(|b| *b == 0).unwrap_or(field.len())];
        if name == b"toc.dat" || name == b"./toc.dat" {
            return SourceFormat::Tar;
        }
    }
    SourceFormat::Plain
}

/// gzip ile sıkıştırılmış dosya mı (sıkıştırılmış plain yedek `.sql.gz`; psql doğrudan okuyamaz).
pub fn is_gzip(head: &[u8]) -> bool {
    head.starts_with(&[0x1f, 0x8b])
}

/// Boşluk içeren öğe türleri (pg_dump `te->desc`); `pg_restore -l` satırında tür, şema, ad ve sahip boşlukla
/// ayrıldığından en uzun eşleşme önce denenir. Listede olmayan tür ilk sözcükle sayılır.
const MULTI_WORD_KINDS: [&str; 33] = [
    "MATERIALIZED VIEW DATA",
    "PUBLICATION TABLES IN SCHEMA",
    "TEXT SEARCH CONFIGURATION",
    "TEXT SEARCH DICTIONARY",
    "TEXT SEARCH TEMPLATE",
    "TEXT SEARCH PARSER",
    "FOREIGN DATA WRAPPER",
    "DATABASE PROPERTIES",
    "PROCEDURAL LANGUAGE",
    "MATERIALIZED VIEW",
    "PUBLICATION TABLE",
    "SUBSCRIPTION TABLE",
    "SEQUENCE OWNED BY",
    "CHECK CONSTRAINT",
    "OPERATOR FAMILY",
    "STATISTICS DATA",
    "OPERATOR CLASS",
    "SECURITY LABEL",
    "FOREIGN SERVER",
    "BLOB METADATA",
    "FOREIGN TABLE",
    "LARGE OBJECTS",
    "ACCESS METHOD",
    "EVENT TRIGGER",
    "FK CONSTRAINT",
    "INDEX ATTACH",
    "LARGE OBJECT",
    "ROW SECURITY",
    "SEQUENCE SET",
    "TABLE ATTACH",
    "USER MAPPING",
    "DEFAULT ACL",
    "TABLE DATA",
];

/// TOC satırı: `<dumpId>; <tableoid> <oid> <tür> <şema> <ad> <sahip>`. Tür döner; satır değilse `None`.
fn toc_kind(line: &str) -> Option<String> {
    let (id, rest) = line.split_once("; ")?;
    if id.is_empty() || !id.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let mut it = rest.splitn(3, ' ');
    let (tableoid, oid, desc) = (it.next()?, it.next()?, it.next()?);
    if [tableoid, oid]
        .iter()
        .any(|n| n.is_empty() || !n.bytes().all(|b| b.is_ascii_digit()))
    {
        return None;
    }
    if let Some(k) = MULTI_WORD_KINDS
        .iter()
        .find(|k| desc.strip_prefix(**k).is_some_and(|r| r.starts_with(' ')))
    {
        return Some((*k).to_string());
    }
    let first = desc.split(' ').next()?;
    (!first.is_empty()).then(|| first.to_string())
}

/// `pg_restore -l` çıktısını özetler: başlık açıklamaları (`;` ile başlayan) ve öğe satırları.
/// Yorumlanmış öğe (`;215; …`) sayılmaz.
pub fn parse_list(output: &str) -> ArchiveInfo {
    let mut info = ArchiveInfo::default();
    let mut counts: std::collections::BTreeMap<String, u32> = std::collections::BTreeMap::new();
    for raw in output.lines() {
        let line = raw.trim_end_matches('\r');
        if let Some(comment) = line.strip_prefix(';') {
            if let Some((key, value)) = comment.trim().split_once(": ") {
                let value = Some(value.trim().to_string()).filter(|v| !v.is_empty());
                match key.trim() {
                    "dbname" => info.db_name = value,
                    "Dumped by pg_dump version" => info.archive_version = value,
                    "Dumped from database version" => info.server_version = value,
                    _ => {}
                }
            }
            continue;
        }
        if let Some(kind) = toc_kind(line) {
            info.items = info.items.saturating_add(1);
            *counts.entry(kind).or_insert(0) += 1;
        }
    }
    let mut counts: Vec<(String, u32)> = counts.into_iter().collect();
    // BTreeMap ad sırasını verir; kararlı sıralama eşitlikte onu korur.
    counts.sort_by_key(|(_, n)| std::cmp::Reverse(*n));
    info.counts = counts;
    info
}

#[cfg(test)]
mod tests {
    use super::*;

    /// pg_dump'ın tar arşivindeki ilk üye başlığı (512 bayt, ustar).
    fn tar_header(name: &str) -> Vec<u8> {
        let mut h = vec![0u8; 512];
        h[..name.len()].copy_from_slice(name.as_bytes());
        h[100..108].copy_from_slice(b"0000600\0");
        h[257..263].copy_from_slice(b"ustar\0");
        h[263..265].copy_from_slice(b"00");
        h
    }

    #[test]
    fn sniffs_formats_from_bytes() {
        let mut custom = b"PGDMP".to_vec();
        custom.extend_from_slice(&[1, 16, 0, 4, 8, 1]);
        assert_eq!(sniff_format(&custom, false, false), SourceFormat::Custom);
        assert_eq!(
            sniff_format(&tar_header("toc.dat"), false, false),
            SourceFormat::Tar
        );
        assert_eq!(
            sniff_format(&tar_header("./toc.dat"), false, false),
            SourceFormat::Tar
        );
        // Başka tar (ilk üye toc.dat değil) pg_dump arşivi değildir.
        assert_eq!(
            sniff_format(&tar_header("3001.dat"), false, false),
            SourceFormat::Plain
        );
        assert_eq!(
            sniff_format(&tar_header("toc.dat.bak"), false, false),
            SourceFormat::Plain
        );
        // Kısa baş (100 bayttan az) tar sayılmaz.
        assert_eq!(
            sniff_format(b"toc.dat\0", false, false),
            SourceFormat::Plain
        );
        assert_eq!(
            sniff_format(b"--\n-- PostgreSQL database dump\n--\n", false, false),
            SourceFormat::Plain
        );
        assert_eq!(
            sniff_format(b"\xEF\xBB\xBFSELECT 1;", false, false),
            SourceFormat::Plain
        );
        assert_eq!(sniff_format(b"", false, false), SourceFormat::Plain);
        assert_eq!(sniff_format(b"PGD", false, false), SourceFormat::Plain);
        assert_eq!(sniff_format(b"pgdmp", false, false), SourceFormat::Plain);
        // Klasör: toc.dat varsa directory; baş baytlarına bakılmaz.
        assert_eq!(sniff_format(b"", true, true), SourceFormat::Directory);
        assert_eq!(sniff_format(b"PGDMP", true, true), SourceFormat::Directory);
        // toc.dat'sız klasör sözleşme gereği Plain döner; çağıran (Task 4) bunu reddeder.
        assert_eq!(sniff_format(b"", true, false), SourceFormat::Plain);
    }

    #[test]
    fn detects_gzip_plain_dump() {
        assert!(is_gzip(&[0x1f, 0x8b, 0x08, 0x00]));
        assert!(!is_gzip(&[0x1f]));
        assert!(!is_gzip(b"PGDMP"));
    }

    /// `pg_restore -l` çıktısı kesiti (PG 18 aracı, PG 16 sunucu; custom). Açıklamalı satırlar `;` ile başlar,
    /// yorumlanmış öğe (`;215; …`) sayılmaz.
    const LIST: &str = "\
;
; Archive created at 2026-10-18 12:00:00 +03
;     dbname: satış db
;     TOC Entries: 14
;     Compression: gzip
;     Dump Version: 1.16-0
;     Format: CUSTOM
;     Integer: 4 bytes
;     Offset: 8 bytes
;     Dumped from database version: 16.4
;     Dumped by pg_dump version: 18.1
;
;
; Selected TOC Entries:
;
5; 2615 16384 SCHEMA - app postgres
2; 3079 13000 EXTENSION - plpgsql
4210; 0 0 COMMENT - EXTENSION plpgsql
215; 1259 16385 TABLE public t postgres
216; 1259 16390 SEQUENCE public t_id_seq postgres
217; 1259 16400 TABLE app Müşteri Tablosu postgres
218; 1259 16410 MATERIALIZED VIEW public mv postgres
3350; 0 16385 TABLE DATA public t postgres
3351; 0 16400 TABLE DATA app Müşteri Tablosu postgres
4211; 0 0 SEQUENCE SET public t_id_seq postgres
3201; 2606 16391 CONSTRAINT public t t_pkey postgres
3202; 2606 16392 FK CONSTRAINT app Müşteri Tablosu fk_t postgres
4212; 826 16420 DEFAULT ACL - DEFAULT PRIVILEGES FOR TABLES postgres
4213; 0 16410 MATERIALIZED VIEW DATA public mv postgres
;219; 1259 16430 TABLE public dropped postgres
";

    #[test]
    fn parses_restore_list() {
        let info = parse_list(LIST);
        assert_eq!(info.archive_version.as_deref(), Some("18.1"));
        assert_eq!(info.server_version.as_deref(), Some("16.4"));
        assert_eq!(info.db_name.as_deref(), Some("satış db"));
        assert_eq!(info.items, 14);
        let want: Vec<(String, u32)> = [
            ("TABLE", 2),
            ("TABLE DATA", 2),
            ("COMMENT", 1),
            ("CONSTRAINT", 1),
            ("DEFAULT ACL", 1),
            ("EXTENSION", 1),
            ("FK CONSTRAINT", 1),
            ("MATERIALIZED VIEW", 1),
            ("MATERIALIZED VIEW DATA", 1),
            ("SCHEMA", 1),
            ("SEQUENCE", 1),
            ("SEQUENCE SET", 1),
        ]
        .iter()
        .map(|(k, v)| (k.to_string(), *v))
        .collect();
        assert_eq!(info.counts, want);
        // CRLF ve boş çıktı.
        assert_eq!(parse_list(&LIST.replace('\n', "\r\n")), info);
        assert_eq!(parse_list(""), ArchiveInfo::default());
        // Bilinmeyen tür ilk sözcükle sayılır; bozuk satır atlanır.
        let odd = parse_list("7; 1 2 NEWKIND public x postgres\nbozuk satır\n8; x y TABLE a b c\n");
        assert_eq!(
            (odd.items, odd.counts),
            (1, vec![("NEWKIND".to_string(), 1)])
        );
    }
}
