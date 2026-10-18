//! İstemci araçlarını bulma (spec §1 "Araç bulma ve sürüm uyumu").
//!
//! Aday klasörler sırayla: ayardaki `pgBinDir`, `PATH`, `%ProgramFiles%\PostgreSQL\*\bin` (ve x86),
//! `%ProgramFiles%\pgAdmin 4\runtime` (ve `v*\runtime`). Her adayda araçlar `--version` ile (5 sn) denenir.

use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use crate::model::{ToolDetection, ToolKind, ToolSet, ToolVersion};

/// `--version` zaman aşımı (Global Constraints).
const VERSION_TIMEOUT: Duration = Duration::from_secs(5);

const NO_TOOLS_HINT: &str = "Ayarlardan PostgreSQL bin klasörü seçin.";

/// `pg_dump (PostgreSQL) 16.4` gibi ilk satırdan sürümü okur. Ad kısmından sonraki ilk rakamla başlayan sözcük
/// sürümdür: "10.23", "18devel", "17beta1", "9.6.24". Okunamazsa `None`.
pub fn parse_version(stdout: &str) -> Option<ToolVersion> {
    let line = stdout.lines().next()?.trim();
    // "(PostgreSQL)" varsa sonrasına bakılır; yoksa ilk sözcük (araç adı) atlanır.
    let rest = match line.find(')') {
        Some(i) => &line[i + 1..],
        None => line.split_once(char::is_whitespace).map(|(_, r)| r)?,
    };
    let text = rest.split_whitespace().next()?;
    let mut parts = Vec::new();
    for piece in text.split('.') {
        let digits: String = piece.chars().take_while(char::is_ascii_digit).collect();
        if digits.is_empty() {
            break;
        }
        parts.push(digits.parse::<u32>().ok()?);
        if digits.len() != piece.len() {
            break; // "18devel", "17beta1": ek sonrası sayılmaz
        }
    }
    let major = *parts.first()?;
    let minor = parts.get(1).copied().unwrap_or(0);
    // Taşmasız aritmetik; alan sınırını aşan parça (10+ için minor ≥ 10000, 9.x için ≥ 100) başka sürüme
    // karışacağından reddedilir.
    let version_num = if major >= 10 {
        if minor >= 10_000 {
            return None;
        }
        major.checked_mul(10_000)?.checked_add(minor)?
    } else {
        let patch = parts.get(2).copied().unwrap_or(0);
        if minor >= 100 || patch >= 100 {
            return None;
        }
        major * 10_000 + minor * 100 + patch
    };
    Some(ToolVersion { major, version_num, text: text.to_string() })
}

/// Tekrar eleme anahtarı: sondaki ayraçlar atılır; Windows'ta büyük/küçük harf duyarsız ve `/` ile `\` aynıdır.
fn dedupe_key(dir: &str) -> String {
    let t = dir.trim_end_matches(['\\', '/']);
    if cfg!(windows) { t.replace('/', "\\").to_lowercase() } else { t.to_string() }
}

fn join(base: &str, rest: &[&str]) -> String {
    let mut p = PathBuf::from(base);
    for r in rest {
        p.push(r);
    }
    p.to_string_lossy().into_owned()
}

/// Sürüm adlı klasörleri ("18", "9.6", "v8") yeniden eskiye sıralar; sayısal olmayanlar sona.
fn sort_versions_desc(names: &mut [String]) {
    let key = |n: &String| -> Vec<u32> {
        n.trim_start_matches(['v', 'V']).split('.').map(|p| p.parse::<u32>().unwrap_or(0)).collect()
    };
    names.sort_by_key(|a| std::cmp::Reverse(key(a)));
}

/// Aday klasörleri spec §1 sırasıyla üretir (saf: dosya sistemi yalnız `list_dir` ile, alt klasör adları).
/// Tekrarlar elenir (sondaki ayraç normalize, Windows'ta büyük/küçük harf duyarsız); ilk görülen biçim korunur.
pub fn candidate_dirs(
    setting: Option<&str>,
    path_var: Option<&str>,
    program_files: &[String],
    list_dir: &dyn Fn(&str) -> Vec<String>,
) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut seen = std::collections::HashSet::new();
    let mut push = |dir: String| {
        let trimmed = dir.trim();
        if trimmed.is_empty() {
            return;
        }
        let normalized = trimmed.trim_end_matches(['\\', '/']);
        // Kök ("/" ya da "C:\") ayraçsız kalmasın.
        let normalized = if normalized.is_empty() || normalized.ends_with(':') { trimmed } else { normalized };
        // Güvenlik: göreli klasör ("." ya da göreli ayar) `--version` ile CWD'deki bir ikiliyi çalıştırabilir.
        if !Path::new(normalized).is_absolute() {
            return;
        }
        if seen.insert(dedupe_key(normalized)) {
            out.push(normalized.to_string());
        }
    };
    if let Some(s) = setting {
        push(s.to_string());
    }
    if let Some(p) = path_var {
        for d in std::env::split_paths(OsStr::new(p)) {
            push(d.to_string_lossy().into_owned());
        }
    }
    let mut pfs: Vec<&String> = Vec::new();
    for pf in program_files {
        if !pf.trim().is_empty() && !pfs.iter().any(|x| dedupe_key(x) == dedupe_key(pf)) {
            pfs.push(pf);
        }
    }
    for pf in &pfs {
        let mut versions = list_dir(&join(pf, &["PostgreSQL"]));
        sort_versions_desc(&mut versions);
        for v in versions {
            push(join(pf, &["PostgreSQL", &v, "bin"]));
        }
    }
    for pf in &pfs {
        push(join(pf, &["pgAdmin 4", "runtime"]));
        let mut versions: Vec<String> = list_dir(&join(pf, &["pgAdmin 4"]))
            .into_iter()
            .filter(|n| n.len() > 1 && n.starts_with(['v', 'V']) && n[1..].chars().all(|c| c.is_ascii_digit() || c == '.'))
            .collect();
        sort_versions_desc(&mut versions);
        for v in versions {
            push(join(pf, &["pgAdmin 4", &v, "runtime"]));
        }
    }
    out
}

pub(crate) fn exe_path(dir: &str, kind: ToolKind) -> PathBuf {
    Path::new(dir).join(format!("{}{}", kind.program(), std::env::consts::EXE_SUFFIX))
}

/// Aracı `--version` ile çalıştırır (kabuksuz, 5 sn, Windows'ta konsol penceresi açmadan).
async fn run_version(exe: &Path) -> Option<ToolVersion> {
    let mut cmd = tokio::process::Command::new(exe);
    cmd.arg("--version").stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::null()).kill_on_drop(true);
    #[cfg(windows)]
    {
        // CREATE_NO_WINDOW
        cmd.creation_flags(0x0800_0000);
    }
    let child = cmd.spawn().ok()?;
    // Zaman aşımında gelecek düşer; `kill_on_drop` çocuğu sonlandırır.
    let out = tokio::time::timeout(VERSION_TIMEOUT, child.wait_with_output()).await.ok()?.ok()?;
    if !out.status.success() {
        return None;
    }
    parse_version(&String::from_utf8_lossy(&out.stdout))
}

/// Bir klasördeki araçları dener; hiçbiri sürüm vermezse `None`.
async fn probe_dir(dir: &str) -> Option<ToolSet> {
    let probe = |kind: ToolKind| {
        let exe = exe_path(dir, kind);
        async move { if exe.is_file() { run_version(&exe).await } else { None } }
    };
    let (dump, restore, psql) = tokio::join!(probe(ToolKind::PgDump), probe(ToolKind::PgRestore), probe(ToolKind::Psql));
    let (pg_dump, pg_restore, has_psql) = (dump.is_some(), restore.is_some(), psql.is_some());
    let version = dump.or(restore).or(psql)?;
    Some(ToolSet { dir: dir.to_string(), version, pg_dump, pg_restore, psql: has_psql })
}

fn list_subdirs(dir: &str) -> Vec<String> {
    let Ok(rd) = std::fs::read_dir(dir) else { return Vec::new() };
    rd.filter_map(|e| e.ok())
        .filter(|e| e.file_type().map(|t| t.is_dir()).unwrap_or(false))
        .filter_map(|e| e.file_name().to_str().map(str::to_string))
        .collect()
}

/// Gerçek dosya sistemi ve `PATH` ile araç kümelerini bulur (aday sırasıyla).
pub async fn detect(setting: Option<&str>) -> ToolDetection {
    let path_var = std::env::var_os("PATH").map(|p| p.to_string_lossy().into_owned());
    let program_files: Vec<String> =
        ["ProgramFiles", "ProgramW6432", "ProgramFiles(x86)"].iter().filter_map(|k| std::env::var(k).ok()).collect();
    let setting = setting.map(str::trim).filter(|s| !s.is_empty());
    let candidates = tokio::task::spawn_blocking({
        let setting = setting.map(str::to_string);
        move || {
            candidate_dirs(setting.as_deref(), path_var.as_deref(), &program_files, &list_subdirs)
                .into_iter()
                // Klasör ve en az bir aracın varlığı engelleyici G/Ç ile burada süzülür.
                .filter(|d| [ToolKind::PgDump, ToolKind::PgRestore, ToolKind::Psql].iter().any(|k| exe_path(d, *k).is_file()))
                .collect::<Vec<_>>()
        }
    })
    .await
    .unwrap_or_default();
    let mut sets = Vec::new();
    for dir in &candidates {
        if let Some(set) = probe_dir(dir).await {
            sets.push(set);
        }
    }
    let problem = if sets.is_empty() {
        Some(format!("PostgreSQL istemci araçları (pg_dump, pg_restore, psql) bulunamadı. {NO_TOOLS_HINT}"))
    } else if let Some(s) = setting
        && !sets.first().is_some_and(|f| dedupe_key(&f.dir) == dedupe_key(s))
    {
        Some("Ayarlardaki bin klasöründe çalışan PostgreSQL aracı bulunamadı; diğer bulunan araçlar kullanılacak.".into())
    } else {
        None
    };
    tracing::debug!(sets = sets.len(), problem = problem.is_some(), "araç bulma tamamlandı");
    ToolDetection { sets, problem }
}

/// Ana sürüm karşılaştırma anahtarı: 10+ için `major*100`, 9.x için `9*100+minor` ("9.6" ayrı ana sürümdür).
fn major_key(version_num: u32) -> u32 {
    if version_num >= 100_000 { version_num / 10_000 * 100 } else { version_num / 100 }
}

fn major_text(version_num: u32) -> String {
    if version_num >= 100_000 {
        (version_num / 10_000).to_string()
    } else {
        format!("{}.{}", version_num / 10_000, version_num / 100 % 100)
    }
}

/// Spec §1 seçim kuralı.
/// - `pg_dump`: sürümü sunucunun ana sürümüne eşit ya da büyük olmalı; aynı ana sürüm tercih edilir, yoksa en
///   küçük büyük sürüm. Eşitlikte aday sırası (ayardaki klasör önce).
/// - `pg_restore`, `psql`: en yeni araç (arşiv sürümü bilinmiyor; uyumsuzluğu PG kendi hatasıyla bildirir).
///   `server_version_num` yalnız hata iletisinde kullanılır.
pub fn choose(sets: &[ToolSet], kind: ToolKind, server_version_num: u32) -> Result<&ToolSet, String> {
    let with_tool = || sets.iter().filter(move |s| kind.has(s));
    let picked = match kind {
        ToolKind::PgDump => {
            let server = major_key(server_version_num);
            with_tool()
                .filter(|s| major_key(s.version.version_num) >= server)
                .min_by_key(|s| major_key(s.version.version_num))
        }
        // `max_by_key` eşitlikte sonuncuyu verir; aday sırası için ters gezilir.
        ToolKind::PgRestore | ToolKind::Psql => with_tool().rev().max_by_key(|s| s.version.version_num),
    };
    picked.ok_or_else(|| {
        let newest = with_tool().max_by_key(|s| s.version.version_num);
        let found = match newest {
            Some(s) => format!("bulunan en yeni {}: {}", kind.program(), s.version.text),
            None => format!("hiç {} bulunamadı", kind.program()),
        };
        // Sürüm bilinmiyorsa (0; pg_restore/psql çağrıları) sürüm cümlesi yazılmaz.
        let target = if server_version_num == 0 {
            String::new()
        } else {
            format!("Sunucu sürümü {} için ", major_text(server_version_num))
        };
        let program = kind.program();
        let head = if target.is_empty() { format!("Uygun {program} yok") } else { format!("{target}uygun {program} yok") };
        format!("{head} ({found}). {NO_TOOLS_HINT}")
    })
}

/// `pgBinDir` ayarını doğrular: klasör var ve içinde `pg_dump` (Windows'ta `pg_dump.exe`) bulunur.
pub fn validate_bin_dir(dir: &str) -> pgt_core::Result<()> {
    let p = Path::new(dir.trim());
    if !p.is_dir() {
        return Err(pgt_core::Error::Invalid("Seçilen bin klasörü bulunamadı.".into()));
    }
    if !exe_path(&p.to_string_lossy(), ToolKind::PgDump).is_file() {
        return Err(pgt_core::Error::Invalid(format!(
            "Seçilen klasörde pg_dump{} yok.",
            std::env::consts::EXE_SUFFIX
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    fn v(text: &str, major: u32, num: u32) -> Option<ToolVersion> {
        Some(ToolVersion { major, version_num: num, text: text.into() })
    }

    #[test]
    fn parses_versions() {
        assert_eq!(parse_version("pg_dump (PostgreSQL) 10.23\n"), v("10.23", 10, 100023));
        assert_eq!(parse_version("pg_dump (PostgreSQL) 16.4"), v("16.4", 16, 160004));
        assert_eq!(parse_version("pg_dump (PostgreSQL) 18.1\r\n"), v("18.1", 18, 180001));
        assert_eq!(parse_version("pg_dump (PostgreSQL) 18devel"), v("18devel", 18, 180000));
        assert_eq!(parse_version("pg_restore (PostgreSQL) 17beta1"), v("17beta1", 17, 170000));
        assert_eq!(parse_version("psql (PostgreSQL) 9.6.24"), v("9.6.24", 9, 90624));
        assert_eq!(parse_version("pg_dump (PostgreSQL) 16.4 (Ubuntu 16.4-1.pgdg22.04+1)"), v("16.4", 16, 160004));
        assert_eq!(parse_version(""), None);
        assert_eq!(parse_version("pg_dump (PostgreSQL)"), None);
        assert_eq!(parse_version("bozuk çıktı"), None);
        assert_eq!(parse_version("pg_dump (PostgreSQL) x.y"), None);
        // Saçma girdi taşma yapmaz; ara sürüm alanı taşarsa reddedilir.
        assert_eq!(parse_version("pg_dump (PostgreSQL) 4294967295.1"), None);
        assert_eq!(parse_version("pg_dump (PostgreSQL) 500000.0"), None);
        assert_eq!(parse_version("pg_dump (PostgreSQL) 16.10000"), None);
        assert_eq!(parse_version("pg_dump (PostgreSQL) 16.9999"), v("16.9999", 16, 169999));
        assert_eq!(parse_version("psql (PostgreSQL) 9.100.1"), None);
        assert_eq!(parse_version("psql (PostgreSQL) 9.6.100"), None);
    }

    fn j(a: &str, rest: &[&str]) -> String {
        let mut p = Path::new(a).to_path_buf();
        for r in rest {
            p = p.join(r);
        }
        p.to_string_lossy().into_owned()
    }

    #[test]
    fn candidates_in_order_without_duplicates() {
        let pf = if cfg!(windows) { r"C:\Program Files" } else { "/pf" };
        let pf86 = if cfg!(windows) { r"C:\Program Files (x86)" } else { "/pf86" };
        let pg = j(pf, &["PostgreSQL"]);
        let pga = j(pf, &["pgAdmin 4"]);
        let pg18 = j(pf, &["PostgreSQL", "18", "bin"]);
        let list = |d: &str| -> Vec<String> {
            if d == pg {
                vec!["16".into(), "18".into()]
            } else if d == pga {
                vec!["v8".into(), "runtime".into(), "docs".into()]
            } else {
                vec![]
            }
        };
        let sep = if cfg!(windows) { ";" } else { ":" };
        // PATH'te 18\bin büyük harfle ve sondaki ayraçla yine yer alır; boş girdi atlanır.
        let mut pg18_variant = pg18.to_uppercase();
        pg18_variant.push(std::path::MAIN_SEPARATOR);
        let usr = if cfg!(windows) { r"D:\usr\bin".to_string() } else { "/usr/bin".to_string() };
        let path_var = [usr.clone(), String::new(), pg18_variant].join(sep);
        let setting = j(pf, &["Custom", "bin"]);
        let got = candidate_dirs(Some(&setting), Some(&path_var), &[pf.to_string(), pf86.to_string(), pf.to_string()], &list);
        // Yeni sürüm önce; PATH'teki biçim korunur ve Program Files'taki 18 (Windows'ta) tekrar sayılmaz.
        let mut want = vec![setting.clone(), usr.clone(), pg18.to_uppercase()];
        if !cfg!(windows) {
            want.push(pg18.clone());
        }
        want.push(j(pf, &["PostgreSQL", "16", "bin"]));
        want.push(j(pf, &["pgAdmin 4", "runtime"]));
        want.push(j(pf, &["pgAdmin 4", "v8", "runtime"]));
        want.push(j(pf86, &["pgAdmin 4", "runtime"]));
        assert_eq!(got, want);
        assert_eq!(candidate_dirs(None, None, &[], &list), Vec::<String>::new());
        // Boş ayar yok sayılır.
        let a = j(pf, &["a"]);
        assert_eq!(candidate_dirs(Some("  "), Some(&a), &[], &list), vec![a.clone()]);
    }

    #[test]
    fn candidates_drop_relative_dirs() {
        // Göreli klasör (".", "bin", göreli ayar) CWD'deki ikiliyi çalıştırabilir; elenir.
        let pf = if cfg!(windows) { r"C:\Program Files" } else { "/pf" };
        let a = j(pf, &["a"]);
        let sep = if cfg!(windows) { ";" } else { ":" };
        let path_var = [".", "bin", a.as_str(), r"..\x", "../y"].join(sep);
        let got = candidate_dirs(Some("tools"), Some(&path_var), &[], &|_| vec![]);
        assert_eq!(got, vec![a]);
    }

    #[cfg(windows)]
    #[test]
    fn candidates_dedupe_mixed_separators_on_windows() {
        let got = candidate_dirs(
            Some(r"C:\PG\18\bin"),
            Some(r"c:/pg/18/bin;C:\PG\18\bin/;C:/Other;\NoDrive;C:rel"),
            &[],
            &|_| vec![],
        );
        assert_eq!(got, vec![r"C:\PG\18\bin".to_string(), "C:/Other".to_string()]);
    }

    fn set(dir: &str, text: &str, num: u32, dump: bool, restore: bool, psql: bool) -> ToolSet {
        let major = num / 10_000;
        ToolSet {
            dir: dir.into(),
            version: ToolVersion { major, version_num: num, text: text.into() },
            pg_dump: dump,
            pg_restore: restore,
            psql,
        }
    }

    #[test]
    fn choose_follows_spec_rules() {
        let s15 = set("a15", "15.8", 150008, true, true, true);
        let s18 = set("a18", "18.1", 180001, true, true, true);
        let s16 = set("a16", "16.4", 160004, true, true, true);
        let s16b = set("b16", "16.9", 160009, true, true, true);
        let s17 = set("a17", "17.2", 170002, true, false, false);
        // Sunucu 16, araçlar 15 ve 18 → 18.
        assert_eq!(choose(&[s15.clone(), s18.clone()], ToolKind::PgDump, 160002).unwrap().dir, "a18");
        // Sunucu 18, araçlar yalnız 16 → net hata (sürümler iletide).
        let err = choose(std::slice::from_ref(&s16), ToolKind::PgDump, 180000).unwrap_err();
        assert!(err.contains("18") && err.contains("16.4") && err.contains("Ayarlardan"), "{err}");
        // Aynı ana sürüm tercih edilir (daha yeni 18 olsa da); eşitlikte aday sırası.
        assert_eq!(choose(&[s18.clone(), s16.clone(), s16b.clone()], ToolKind::PgDump, 160000).unwrap().dir, "a16");
        // Eşit yoksa en küçük büyük sürüm.
        assert_eq!(choose(&[s18.clone(), s17.clone()], ToolKind::PgDump, 150000).unwrap().dir, "a17");
        // pg_dump'ı olmayan küme sayılmaz.
        let no_dump = set("x18", "18.0", 180000, false, true, true);
        assert!(choose(std::slice::from_ref(&no_dump), ToolKind::PgDump, 160000).is_err());
        // 9.6 sunucu: 9.6 araç uygun, 9.5 değil.
        let s95 = set("a95", "9.5.25", 90525, true, true, true);
        let s96 = set("a96", "9.6.24", 90624, true, true, true);
        assert_eq!(choose(&[s95.clone(), s96.clone()], ToolKind::PgDump, 90620).unwrap().dir, "a96");
        let e95 = choose(std::slice::from_ref(&s95), ToolKind::PgDump, 90620).unwrap_err();
        assert!(e95.contains("9.6") && e95.contains("9.5.25"), "{e95}");
        // pg_restore ve psql: en yeni.
        assert_eq!(choose(&[s16.clone(), s18.clone(), s17.clone()], ToolKind::PgRestore, 100000).unwrap().dir, "a18");
        assert_eq!(choose(&[s16.clone(), no_dump.clone(), s15.clone()], ToolKind::Psql, 180000).unwrap().dir, "x18");
        assert!(choose(std::slice::from_ref(&s17), ToolKind::Psql, 160000).unwrap_err().contains("psql"));
        // Hiç araç yok.
        let none = choose(&[], ToolKind::PgDump, 160000).unwrap_err();
        assert!(none.contains("16") && none.contains("Ayarlardan"), "{none}");
        // Sunucu sürümü bilinmiyorsa (0) iletide sürüm cümlesi yok.
        let unknown = choose(&[], ToolKind::PgRestore, 0).unwrap_err();
        assert!(
            !unknown.contains("Sunucu sürümü") && unknown.contains("pg_restore") && unknown.contains("Ayarlardan"),
            "{unknown}"
        );
    }

    #[test]
    fn validate_bin_dir_requires_pg_dump() {
        let dir = tempfile::tempdir().unwrap();
        let d = dir.path().to_string_lossy().into_owned();
        assert!(validate_bin_dir(&d).is_err());
        assert!(validate_bin_dir(&dir.path().join("yok").to_string_lossy()).is_err());
        let exe = dir.path().join(format!("pg_dump{}", std::env::consts::EXE_SUFFIX));
        std::fs::write(&exe, b"").unwrap();
        validate_bin_dir(&d).unwrap();
        // Dosya klasör sanılmaz.
        assert!(validate_bin_dir(&exe.to_string_lossy()).is_err());
    }

    /// Gerçek makine testi: PostgreSQL istemci araçları kuruluysa bulunur; değilse atlanır.
    #[tokio::test]
    async fn detects_installed_tools() {
        let dir = std::env::var("PGT_TEST_PGBIN").unwrap_or_else(|_| r"C:\Program Files\PostgreSQL\18\bin".into());
        if !Path::new(&dir).join(format!("pg_dump{}", std::env::consts::EXE_SUFFIX)).is_file() {
            eprintln!("atlandı: PostgreSQL istemci araçları bulunamadı (PGT_TEST_PGBIN)");
            return;
        }
        let d = detect(Some(&dir)).await;
        let first = d.sets.first().expect("ayardaki klasör ilk küme olmalı");
        assert!(first.dir.eq_ignore_ascii_case(dir.trim_end_matches(['\\', '/'])), "{}", first.dir);
        assert!(first.pg_dump && first.pg_restore && first.psql);
        if dir.contains(r"PostgreSQL\18") {
            assert_eq!(first.version.major, 18);
        }
        assert!(d.problem.is_none(), "{:?}", d.problem);
        // Aynı klasör iki kez sayılmaz.
        let mut keys: Vec<String> = d.sets.iter().map(|s| s.dir.to_lowercase()).collect();
        keys.sort();
        keys.dedup();
        assert_eq!(keys.len(), d.sets.len());
        let picked = choose(&d.sets, ToolKind::PgDump, first.version.version_num).unwrap();
        assert_eq!(picked.version.major, first.version.major);
    }

    #[tokio::test]
    async fn detect_reports_problem_for_bad_setting() {
        let dir = tempfile::tempdir().unwrap();
        let d = detect(Some(&dir.path().to_string_lossy())).await;
        assert!(d.problem.is_some());
        assert!(d.sets.iter().all(|s| !s.dir.eq_ignore_ascii_case(&dir.path().to_string_lossy())));
    }
}
