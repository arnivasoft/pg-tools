//! stderr satırlarının sınıflandırılması, ilerleme sayımı ve hata ipuçları (spec §1 "İlerleme", "Hata eşleme").
//!
//! Araçlar `LC_MESSAGES=C` ile çalıştırılır (bkz. `args`), bu yüzden istemci iletileri İngilizcedir. Sunucu
//! iletileri (psql'in yazdığı `ERROR:` vb.) sunucunun `lc_messages` diline bağlıdır; Türkçe karşılıklar da tanınır.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::model::{JobKind, ToolKind};

/// Satır önemi.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum LineSeverity {
    Info,
    Warning,
    Error,
}

const PASSWORD_HINT: &str =
    "Parola reddedildi ya da verilmedi: parolayı kaydedin veya sunucuya yeniden bağlanın.";
const DUMP_VERSION_HINT: &str = "Sunucu bu pg_dump'tan yeni: daha yeni bir pg_dump gerekir; Ayarlardan PostgreSQL bin klasörü seçin.";
const RESTORE_VERSION_HINT: &str = "Arşiv bu pg_restore'dan yeni bir pg_dump ile alınmış: daha yeni bir pg_restore \
     gerekir; Ayarlardan PostgreSQL bin klasörü seçin.";
/// 17+ araçlar (pg_dump arşivi ve pg_restore/psql) `SET transaction_timeout` yazar; 17'den eski sunucu bunu tanımaz
/// (ölçüldü: araç 18, sunucu 10–16).
const OLD_SERVER_HINT: &str = "Araç bu sunucudan yeni ve sunucunun tanımadığı bir ayar (transaction_timeout) gönderdi: \
     sunucuyla aynı ana sürümden pg_dump/pg_restore/psql kurup Ayarlardan bin klasörü olarak seçin; \
     \"hatada dur\" kapalıyken pg_restore bu satırı atlayıp devam eder.";

/// Düz SQL (psql) işinde ipucu 4: `SET transaction_timeout` satırı dosyanın kendisindedir (17+ pg_dump yazar);
/// psql sürümü önemsizdir.
const OLD_SERVER_PLAIN_HINT: &str = "Düz SQL yedeği bu sunucudan yeni bir pg_dump ile alınmış ve sunucunun tanımadığı bir \
     ayar (transaction_timeout) içeriyor: yedeği sunucuyla aynı ana sürümden pg_dump ile yeniden alın ya da custom \
     formatı kullanın.";

/// Sunucu iletisi önem sözcükleri (psql'in yazdığı `ERROR:  …`). Türkçe `lc_messages` karşılıkları sunucunun tr
/// çevirisinden: ERROR → `HATA:`, FATAL → `ÖLÜMCÜL (FATAL):`, PANIC → `KRİTİK:`, WARNING → `UYARI:`.
const ERROR_TOKENS: [&str; 6] = ["ERROR:", "FATAL:", "PANIC:", "HATA:", "ÖLÜMCÜL (FATAL):", "KRİTİK:"];
const WARNING_TOKENS: [&str; 2] = ["WARNING:", "UYARI:"];

/// Ayrıntılı (`--verbose`) bilgi satırlarının başları; bunlarda ad içindeki "ERROR:" önemi değiştirmez.
const VERBOSE_PREFIXES: [&str; 14] = [
    "creating ",
    "processing ",
    "dumping ",
    "executing ",
    "restoring ",
    "reading ",
    "saving ",
    "dropping ",
    "connecting ",
    "launching ",
    "finished ",
    "flagging ",
    "identifying ",
    "implied ",
];

/// `pg_dump: …` gibi araç önekini ayırır (Windows'ta olası `.exe` ekiyle). psql'in `psql:dosya:satır:` biçimi
/// önek sayılmaz.
fn strip_tool(line: &str) -> Option<(&'static str, &str)> {
    for tool in ["pg_dump", "pg_restore", "psql", "pg_dumpall"] {
        if let Some(r) = line.strip_prefix(tool) {
            let r = r.strip_prefix(".exe").unwrap_or(r);
            if let Some(rest) = r.strip_prefix(": ") {
                return Some((tool, rest));
            }
        }
    }
    None
}

/// Sunucu önem sözcüğü satır başında ya da bir `: ` sonrasında mı (`psql:a.sql:3: ERROR:`, `query: ERROR:`).
fn has_token(text: &str, tokens: &[&str]) -> bool {
    tokens
        .iter()
        .any(|t| text.starts_with(t) || text.contains(&format!(": {t}")))
}

/// stderr satırının önemi. PG 12+ istemci günlüğü (`pg_dump: error:`, `warning:`), PG 10/11 biçimi
/// (`pg_dump: [archiver (db)] …`), psql/sunucu iletileri (`psql:a.sql:12: ERROR:`, `WARNING:`) tanınır.
/// Devam satırları (`DETAIL:`, sekmeyle başlayan) bilgidir.
pub fn classify(line: &str) -> LineSeverity {
    let t = line.trim_end();
    if let Some((_, rest)) = strip_tool(t) {
        if let Some((word, _)) = rest.split_once(':') {
            match word.to_lowercase().as_str() {
                "error" | "fatal" | "hata" | "ölümcül" => return LineSeverity::Error,
                "warning" | "uyarı" => return LineSeverity::Warning,
                "detail" | "hint" | "ayrıntı" | "ipucu" => return LineSeverity::Info,
                _ => {}
            }
        }
        // PG 10/11: köşeli parantezli modül adıyla yazılan satırlar hatadır (uyarı sözcüğü yoksa).
        if rest.starts_with('[') {
            let lower = rest.to_lowercase();
            return if !has_token(rest, &ERROR_TOKENS) && lower.contains("warning") {
                LineSeverity::Warning
            } else {
                LineSeverity::Error
            };
        }
        if rest.starts_with("aborting because of") {
            return LineSeverity::Error;
        }
        if VERBOSE_PREFIXES.iter().any(|p| rest.starts_with(p)) {
            return LineSeverity::Info;
        }
    }
    if has_token(t, &ERROR_TOKENS) {
        LineSeverity::Error
    } else if has_token(t, &WARNING_TOKENS) {
        LineSeverity::Warning
    } else {
        LineSeverity::Info
    }
}

/// Satır ilerlemeye sayılır mı (spec §1).
/// - Yedekleme: `pg_dump: dumping contents of table …`.
/// - Geri yükleme: öğe başına bir satır: şema öğesi `creating …`, veri öğesi `processing data for table …`
///   (eski araçlarda `restoring data …`), diğer veri öğeleri `executing …` / `processing BLOBS`.
///   `processing item`/`launching item`/`finished item` paralel kipte aynı öğe için ek olarak yazıldığından
///   sayılmaz (çift sayım olmaz). psql satırları sayılmaz.
pub fn progress_tick(line: &str, kind: JobKind) -> bool {
    let Some((tool, rest)) = strip_tool(line.trim_end()) else {
        return false;
    };
    match kind {
        JobKind::Backup => tool == "pg_dump" && rest.starts_with("dumping contents of table "),
        JobKind::Restore => {
            tool == "pg_restore"
                && (rest.starts_with("creating ")
                    || rest.starts_with("executing ")
                    || rest.starts_with("restoring data for table ")
                    || (rest.starts_with("processing ")
                        && !rest.starts_with("processing item ")
                        && !rest.starts_with("processing missed item ")))
        }
    }
}

/// Başarısız işin satırlarından kullanıcı ipucu (spec §1 "Hata eşleme"). Parola sorunu önceliklidir.
pub fn failure_hint(lines: &[String]) -> Option<&'static str> {
    let lower: Vec<String> = lines.iter().map(|l| l.to_lowercase()).collect();
    let any = |needles: &[&str]| lower.iter().any(|l| needles.iter().any(|n| l.contains(n)));
    // Sunucu tarafı iletisi sunucunun `lc_messages` dilindedir; tr: `"%s" kullanıcısı için şifre doğrulaması
    // başarısız oldu`.
    if any(&["password authentication failed", "no password supplied", "şifre doğrulaması başarısız"]) {
        Some(PASSWORD_HINT)
    } else if any(&["server version mismatch"]) {
        Some(DUMP_VERSION_HINT)
    } else if any(&["unsupported version"]) {
        Some(RESTORE_VERSION_HINT)
    } else if any(&["unrecognized configuration parameter \"transaction_timeout\""]) {
        Some(OLD_SERVER_HINT)
    } else {
        None
    }
}

/// [`failure_hint`], çalışan araca göre: psql işinde ipucu 4 düz SQL metnidir.
pub fn failure_hint_for(lines: &[String], tool: ToolKind) -> Option<&'static str> {
    match failure_hint(lines) {
        Some(OLD_SERVER_HINT) if tool == ToolKind::Psql => Some(OLD_SERVER_PLAIN_HINT),
        h => h,
    }
}

/// `pg_restore`'un sonunda yazdığı atlanan hata sayısı: 12+ `pg_restore: warning: errors ignored on restore: N`,
/// 10/11 `WARNING: errors ignored on restore: N`. Başka satırda `None`.
pub fn ignored_errors(line: &str) -> Option<u32> {
    let t = line.trim_end();
    let rest = match strip_tool(t) {
        Some(("pg_restore", rest)) => rest.strip_prefix("warning: ")?,
        Some(_) => return None,
        None => t.strip_prefix("WARNING: ")?,
    };
    rest.strip_prefix("errors ignored on restore: ")?.trim().parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use LineSeverity::*;

    #[test]
    fn classifies_tool_and_server_lines() {
        let cases: &[(&str, LineSeverity)] = &[
            // PG 12+ istemci günlüğü
            (
                r#"pg_dump: error: connection to server at "h" (10.0.0.1), port 5432 failed: FATAL:  password authentication failed for user "u""#,
                Error,
            ),
            (
                r#"pg_restore: error: could not execute query: ERROR:  relation "t" already exists"#,
                Error,
            ),
            (
                "pg_dump: warning: there are circular foreign-key constraints on this table:",
                Warning,
            ),
            ("pg_restore: warning: errors ignored on restore: 3", Warning),
            ("pg_dump: fatal: could not open output file", Error), // PG 12–14
            ("pg_dump: detail: Query was: SELECT 1", Info),
            ("pg_dump: hint: Use the --clean option", Info),
            ("pg_dump.exe: error: x", Error),
            ("psql: error: connection to server failed", Error),
            // Ayrıntılı ilerleme satırları; ad içindeki "ERROR:" önemi değiştirmez.
            (r#"pg_dump: dumping contents of table "public.t""#, Info),
            (r#"pg_restore: creating TABLE "public.ERROR: x""#, Info),
            (
                r#"pg_dump: dumping contents of table "public.WARNING: y""#,
                Info,
            ),
            ("pg_restore: processing item 215 TABLE t", Info),
            // psql (-f) ve sunucu iletileri
            (
                r#"psql:C:/yedek/a.sql:12: ERROR:  syntax error at or near "x""#,
                Error,
            ),
            (
                r#"psql:a.sql:3: NOTICE:  table "t" does not exist, skipping"#,
                Info,
            ),
            (
                "psql:a.sql:4: WARNING:  there is no transaction in progress",
                Warning,
            ),
            ("psql:a.sql:9: FATAL:  terminating connection", Error),
            ("ERROR:  permission denied", Error),
            ("WARNING:  x", Warning),
            ("psql:a.sql:2: HATA:  sözdizimi hatası", Error),
            ("psql:a.sql:2: ÖLÜMCÜL (FATAL):  bağlantı sonlandırıldı", Error),
            ("ÖLÜMCÜL (FATAL):  x", Error),
            ("psql:a.sql:2: KRİTİK:  x", Error),
            // Bilgi iletisindeki "ÖLÜMCÜL" sözcüğü tek başına önem değildir.
            ("psql:a.sql:2: BİLGİ:  ÖLÜMCÜL değil", Info),
            ("psql:a.sql:2: UYARI:  x", Warning),
            // PG 10/11 araçları
            (
                r#"pg_dump: [archiver (db)] connection to database "d" failed: FATAL:  password authentication failed"#,
                Error,
            ),
            (
                "pg_restore: [archiver (db)] Error while PROCESSING TOC:",
                Error,
            ),
            (
                "pg_restore: [archiver] unsupported version (1.16) in file header",
                Error,
            ),
            (
                "pg_dump: aborting because of server version mismatch",
                Error,
            ),
            ("WARNING: errors ignored on restore: 3", Warning),
            // Yerelleştirilmiş istemci (LC_MESSAGES etkisizse)
            ("pg_dump: hata: bağlantı kurulamadı", Error),
            ("pg_dump: uyarı: x", Warning),
            // Devam satırları
            (
                "\tIs the server running on that host and accepting TCP/IP connections?",
                Info,
            ),
            ("Command was: CREATE TABLE public.t ();", Info),
            ("DETAIL:  Key (id)=(1) already exists.", Info),
            ("", Info),
        ];
        for (line, want) in cases {
            assert_eq!(classify(line), *want, "{line}");
        }
    }

    #[test]
    fn counts_progress_per_spec() {
        use JobKind::*;
        let yes: &[(&str, JobKind)] = &[
            (r#"pg_dump: dumping contents of table "public.t""#, Backup),
            (r#"pg_dump: dumping contents of table "s p"."T""#, Backup),
            (r#"pg_restore: creating TABLE "public.t""#, Restore),
            (r#"pg_restore: creating SCHEMA "s""#, Restore),
            (
                r#"pg_restore: processing data for table "public.t""#,
                Restore,
            ),
            ("pg_restore: executing SEQUENCE SET t_id_seq", Restore),
            ("pg_restore: processing BLOBS", Restore),
            (r#"pg_restore: restoring data for table "t""#, Restore), // eski araçlar
        ];
        for (line, kind) in yes {
            assert!(progress_tick(line, *kind), "{line}");
        }
        let no: &[(&str, JobKind)] = &[
            ("pg_dump: reading tables", Backup),
            ("pg_dump: saving encoding = UTF8", Backup),
            (r#"pg_restore: creating TABLE "public.t""#, Backup),
            (r#"pg_dump: dumping contents of table "public.t""#, Restore),
            // Paralel kipte aynı öğe için "creating"/"processing data" da yazılır: çift sayım olmasın.
            ("pg_restore: processing item 215 TABLE t", Restore),
            ("pg_restore: processing missed item 216 INDEX i", Restore),
            ("pg_restore: launching item 3350 TABLE DATA t", Restore),
            ("pg_restore: finished item 3350 TABLE DATA t", Restore),
            ("pg_restore: dropping TABLE t", Restore),
            ("pg_restore: connecting to database for restore", Restore),
            (
                "pg_restore: error: could not execute query: ERROR:  x",
                Restore,
            ),
            ("psql:a.sql:3: NOTICE:  creating x", Restore),
            ("creating TABLE x", Restore),
        ];
        for (line, kind) in no {
            assert!(!progress_tick(line, *kind), "{line}");
        }
    }

    #[test]
    fn parses_ignored_error_count() {
        assert_eq!(ignored_errors("pg_restore: warning: errors ignored on restore: 3"), Some(3));
        assert_eq!(ignored_errors("pg_restore.exe: warning: errors ignored on restore: 12\r"), Some(12));
        assert_eq!(ignored_errors("WARNING: errors ignored on restore: 1"), Some(1));
        assert_eq!(ignored_errors("pg_dump: warning: errors ignored on restore: 1"), None);
        assert_eq!(ignored_errors("pg_restore: warning: errors ignored on restore: x"), None);
        assert_eq!(ignored_errors("pg_restore: error: could not execute query"), None);
        assert_eq!(ignored_errors(""), None);
    }

    fn l(lines: &[&str]) -> Vec<String> {
        lines.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn hints_for_known_failures() {
        let tt = failure_hint(&l(&[
            "pg_restore: error: could not execute query: ERROR:  unrecognized configuration parameter \"transaction_timeout\"",
        ]))
        .unwrap();
        assert!(tt.contains("aynı ana sürüm") && tt.contains("bin klasörü"), "{tt}");
        let pw = failure_hint(&l(&[
            "pg_dump: error: connection to server at \"h\", port 5432 failed: FATAL:  password authentication failed for user \"u\"",
        ]))
        .unwrap();
        assert!(pw.contains("parola"), "{pw}");
        // Türkçe lc_messages'lı sunucu.
        assert_eq!(
            failure_hint(&l(&[
                "pg_dump: error: connection to server at \"h\", port 5432 failed: ÖLÜMCÜL (FATAL):  \"u\" kullanıcısı için şifre doğrulaması başarısız oldu"
            ])),
            Some(pw)
        );
        assert_eq!(
            failure_hint(&l(&[
                "pg_restore: error: connection to server failed: fe_sendauth: no password supplied"
            ])),
            Some(pw)
        );
        let ver = failure_hint(&l(&[
            "pg_dump: error: aborting because of server version mismatch",
            "pg_dump: detail: server version: 18.0; pg_dump version: 16.4",
        ]))
        .unwrap();
        assert!(
            ver.contains("pg_dump") && ver.contains("bin klasörü"),
            "{ver}"
        );
        let old = failure_hint(&l(&[
            "pg_dump: server version: 16.4; pg_dump version: 10.23",
            "pg_dump: aborting because of server version mismatch",
        ]));
        assert_eq!(old, Some(ver));
        let rv = failure_hint(&l(&[
            "pg_restore: error: unsupported version (1.16) in file header",
        ]))
        .unwrap();
        assert!(
            rv.contains("pg_restore") && rv.contains("bin klasörü"),
            "{rv}"
        );
        assert_eq!(
            failure_hint(&l(&[
                "pg_dump: error: could not open output file: Permission denied"
            ])),
            None
        );
        assert_eq!(failure_hint(&[]), None);
        // psql işinde ipucu 4 düz SQL yedeğini anlatır; diğer ipuçları araçtan bağımsızdır.
        let tt_lines = l(&[r#"psql:C:/y/a.sql:9: ERROR:  unrecognized configuration parameter "transaction_timeout""#]);
        let plain = failure_hint_for(&tt_lines, ToolKind::Psql).unwrap();
        assert!(plain.contains("pg_dump ile yeniden") && plain.contains("custom"), "{plain}");
        assert_eq!(failure_hint_for(&tt_lines, ToolKind::PgRestore), Some(tt));
        assert_eq!(failure_hint_for(&l(&["no password supplied"]), ToolKind::Psql), Some(pw));
        // Parola sorunu önceliklidir.
        assert_eq!(
            failure_hint(&l(&[
                "unsupported version",
                "password authentication failed"
            ])),
            Some(pw)
        );
    }
}
