//! İş yürütücüsü (spec §1 "Ortak kurallar"): istemci aracını kabuksuz çalıştırır, stderr satırlarını sınıflandırıp
//! olay olarak akıtır, ilerlemeyi sayar, iptal eder ve çıktıyı atomik olarak hedefe taşır.
//!
//! - **Ortam:** `env_clear` kullanılmaz (Windows'un `SystemRoot`/`PATH` gibi değişkenleri gerekir). Miras alınan
//!   ve adı `PG` ile başlayan her değişken (harf duyarsız: Windows'ta ortam adları harf duyarsızdır, diğer
//!   sistemlerde fazladan silmek zararsızdır), ardından `Invocation::env_remove` kaldırılır; sonra
//!   `Invocation::env` ve verilmişse `PGPASSWORD` eklenir.
//! - **Akışlar:** stdin ve stdout `null`'dır: çıktı `--file`/`-f` ile yazılır; psql'in stdout'a yazdığı komut
//!   etiketleri (`SET`, `CREATE TABLE`) gereksizdir ve boru hiç olmadığı için tıkanamaz. stderr satır satır,
//!   kayıplı UTF-8 çözümüyle okunur.
//! - **Atomik çıktı:** araç `<hedef>.pgt-partial`'a yazar. Başlamadan önce hedef denetlenir ([`check_output`]) ve
//!   önceki bir çöküşten kalan geçici çıktı silinir (ad bize aittir; araç, klasör formatında klasör varsa ya da
//!   dosya varsa üzerine yazacağı için kalıntı hiçbir durumda korunmaz). Başarıda geçici çıktı hedefe taşınır;
//!   hata, iptal ya da alıcı gitmesinde (bağlantıdan önce oluşmuş dosya dahil) silinir. Taşıma başarısız olursa
//!   kopyala+sil yapılmaz: geçici çıktı korunur ve hata verilir.
//! - **Günlük:** `tracing`'e yalnız iş türü, sonuç, çıkış kodu ve süre yazılır.

use std::collections::VecDeque;
use std::ffi::OsStr;
use std::io;
use std::path::Path;
use std::process::{ExitStatus, Stdio};
use std::time::{Duration, Instant};

use serde::{Serialize, Serializer};
use tokio::io::{AsyncBufRead, AsyncBufReadExt, BufReader};
use tokio::process::Command;

use crate::args::{Invocation, OutputPlan};
use crate::events::{BackupEvent, EventSink, JobOutcome, JobSummary};
use crate::lines::{LineSeverity, classify, failure_hint, failure_hint_for, ignored_errors, progress_tick};
use crate::model::{JobKind, ToolKind};
use crate::registry::CancelToken;

/// Gösterilen en çok stderr satırı; fazlası sayılır, son satırla bildirilir.
pub const MAX_LOG_LINES: usize = 2000;
/// Satır başına en çok bayt; fazlası UTF-8 sınırında kesilir ve `…` eklenir.
pub const MAX_LINE_BYTES: usize = 4096;
/// Okunan ham satırın üst sınırı (kesme kararı çözümden sonra verilir).
const RAW_LINE_CAP: usize = MAX_LINE_BYTES + 4;
/// Sınır aştıktan sonra ipucu için tutulan son satırlar.
const HINT_TAIL: usize = 100;
const PROGRESS_INTERVAL: Duration = Duration::from_secs(1);
/// Windows'ta sonlandırılan sürecin dosya tutamakları hemen bırakılmayabilir; silme birkaç kez denenir.
const REMOVE_ATTEMPTS: u32 = 10;
#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// Parola sarmalayıcısı: `Debug` ve `Serialize` değeri yazmaz; değer yalnız çocuk süreç ortamına girer.
pub struct Secret(String);

impl Secret {
    pub fn new(value: String) -> Self {
        Secret(value)
    }

    fn expose(&self) -> &str {
        &self.0
    }
}

impl From<String> for Secret {
    fn from(value: String) -> Self {
        Secret(value)
    }
}

impl std::fmt::Debug for Secret {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Secret(***)")
    }
}

impl Serialize for Secret {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str("***")
    }
}

fn is_pg_key(key: &OsStr) -> bool {
    key.to_string_lossy().get(..2).is_some_and(|p| p.eq_ignore_ascii_case("PG"))
}

/// Kabuksuz komut: miras `PG*` ve `env_remove` anahtarları kaldırılır, stdin `null`, stderr boru; Windows'ta konsol
/// penceresi açılmaz. Yürütücü geleceği düşerse (uygulama kapanışı) çocuk süreç kalmaz.
fn base_command(exe: &str, args: &[String], env_remove: &[String]) -> Command {
    let mut cmd = Command::new(exe);
    cmd.args(args).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::piped()).kill_on_drop(true);
    for (key, _) in std::env::vars_os() {
        if is_pg_key(&key) {
            // Windows'ta silme harf duyarsızdır. Aynı anahtar sonra eklenirse (`PGHOST`) komut ortamı mirastaki
            // yazımı (`pghost`) korur ama değer yenisidir; Windows ortam adları harf duyarsız olduğundan eşdeğerdir.
            cmd.env_remove(&key);
        }
    }
    for key in env_remove {
        cmd.env_remove(key);
    }
    #[cfg(windows)]
    cmd.creation_flags(CREATE_NO_WINDOW);
    cmd
}

fn command(inv: &Invocation, password: Option<&Secret>) -> Command {
    let mut cmd = base_command(&inv.exe, &inv.args, &inv.env_remove);
    cmd.envs(inv.env.iter().map(|(k, v)| (k, v)));
    if let Some(p) = password {
        cmd.env("PGPASSWORD", p.expose());
    }
    cmd
}

/// Yürütücünün ANSI denetimi iletisi (değer yazılmaz: parola olabilir).
pub(crate) const ANSI_RUN_MESSAGE: &str = "Araç çağrısındaki bir değer (yol, ad ya da parola) Windows kod sayfasında güvenle \n     temsil edilemiyor; araç başlatılmadı.";

/// Son savunma hattı (bkz. [`crate::ansi`]): çocuğa giden her değer (çalıştırılabilir yol, argümanlar, eklenen ortam
/// ve parola) sistem ANSI kod sayfasında birebir gidiş-dönüş yapmalı. Plan denetimi alan hatası olarak aynısını
/// yapar; bu denetim planı atlayan ya da yeni değer ekleyen bir çağıranı da kapalı tutar.
fn ansi_safe<'a>(exe: &str, args: &[String], env: impl IntoIterator<Item = &'a str>) -> bool {
    use crate::ansi::ensure_ansi_roundtrip as ok;
    ok(exe) && args.iter().all(|a| ok(a)) && env.into_iter().all(ok)
}

fn invocation_ansi_safe(inv: &Invocation, password: Option<&Secret>) -> bool {
    let env = inv.env.iter().flat_map(|(k, v)| [k.as_str(), v.as_str()]).chain(password.map(Secret::expose));
    ansi_safe(&inv.exe, &inv.args, env)
}

/// `pg_restore --list` zaman aşımı.
const LIST_TIMEOUT: Duration = Duration::from_secs(60);
/// `pg_restore --list` çıktısının üst sınırı.
const LIST_MAX_BYTES: u64 = 64 * 1024 * 1024;

/// `pg_restore --list` ile arşivin içindekiler listesini okur (sunucuya bağlanmaz). Hata iletisi aracın ilk hata
/// satırını ve bilinen bir ipucunu içerir; günlüğe yazılmaz.
pub(crate) async fn list_archive(exe: &str, format: &str, path: &str) -> Result<String, String> {
    use tokio::io::AsyncReadExt;
    let args = vec!["--list".to_string(), format!("--format={format}"), path.to_string()];
    if !ansi_safe(exe, &args, []) {
        return Err(ANSI_RUN_MESSAGE.into());
    }
    let remove: Vec<String> = crate::args::LOCALE_OVERRIDES.iter().map(|k| k.to_string()).collect();
    let mut cmd = base_command(exe, &args, &remove);
    cmd.stdout(Stdio::piped()).env("LC_MESSAGES", "C");
    let mut child = cmd.spawn().map_err(|e| format!("pg_restore başlatılamadı: {e}"))?;
    let (Some(stdout), Some(stderr)) = (child.stdout.take(), child.stderr.take()) else {
        return Err("pg_restore çıktısı okunamadı.".into());
    };
    let work = async {
        let mut out = Vec::new();
        let mut err = Vec::new();
        let mut limited = stdout.take(LIST_MAX_BYTES + 1);
        let mut err_limited = stderr.take(64 * 1024);
        let (a, b) = tokio::join!(limited.read_to_end(&mut out), err_limited.read_to_end(&mut err));
        a.and(b).map_err(|e| format!("pg_restore çıktısı okunamadı: {e}"))?;
        if out.len() as u64 > LIST_MAX_BYTES {
            return Err("Arşiv içindekiler listesi çok büyük.".to_string());
        }
        let status = child.wait().await.map_err(|e| format!("pg_restore beklenemedi: {e}"))?;
        Ok::<_, String>((status, out, err))
    };
    let (status, out, err) = match tokio::time::timeout(LIST_TIMEOUT, work).await {
        Ok(r) => r?,
        Err(_) => return Err(format!("pg_restore {} sn içinde yanıt vermedi.", LIST_TIMEOUT.as_secs())),
    };
    if status.success() {
        return Ok(String::from_utf8_lossy(&out).into_owned());
    }
    let err = String::from_utf8_lossy(&err);
    let lines: Vec<String> = err.lines().map(str::to_string).collect();
    let first = lines.iter().find(|l| classify(l) == LineSeverity::Error).or(lines.first());
    let mut msg = match first {
        Some(l) => format!("Arşiv okunamadı: {}", l.trim()),
        None => "Arşiv okunamadı.".to_string(),
    };
    if let Some(h) = failure_hint(&lines) {
        msg.push(' ');
        msg.push_str(h);
    }
    Err(msg)
}

/// Hedef denetimi (başlamadan önce; dosya sistemine yazmaz). Klasör formatında hedef yok ya da boş bir klasör
/// olmalıdır; dosya formatında hedef bir klasör olamaz (var olan dosyanın üzerine yazılır). İletiler yol içermez.
pub fn check_output(plan: &OutputPlan) -> Result<(), String> {
    let target = Path::new(&plan.final_path);
    match std::fs::metadata(target) {
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(format!("Hedef denetlenemedi: {e}")),
        Ok(m) if plan.is_dir && m.is_dir() => match std::fs::read_dir(target).map(|mut d| d.next().is_none()) {
            Ok(true) => Ok(()),
            Ok(false) => Err("Hedef klasör boş değil; boş bir klasör ya da yeni bir ad seçin.".into()),
            Err(e) => Err(format!("Hedef klasör okunamadı: {e}")),
        },
        Ok(_) if plan.is_dir => Err("Hedef bir dosya; klasör formatı için boş bir klasör ya da yeni bir ad seçin.".into()),
        Ok(m) if m.is_dir() => Err("Hedef bir klasör; bir dosya adı seçin.".into()),
        Ok(_) => Ok(()),
    }
}

/// Geçici çıktıyı (dosya, klasör ya da bağlantı) siler; yoksa başarıdır. Bağlantı izlenmez, yalnız kendisi silinir.
fn remove_partial(path: &Path) -> io::Result<()> {
    let mut attempt = 0;
    loop {
        let result = match std::fs::symlink_metadata(path) {
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(()),
            Err(e) => Err(e),
            Ok(m) if m.file_type().is_symlink() => std::fs::remove_file(path).or_else(|_| std::fs::remove_dir(path)),
            Ok(m) if m.is_dir() => std::fs::remove_dir_all(path),
            Ok(_) => std::fs::remove_file(path),
        };
        match result {
            Err(e) if e.kind() != io::ErrorKind::NotFound => {
                attempt += 1;
                if attempt >= REMOVE_ATTEMPTS {
                    return Err(e);
                }
                std::thread::sleep(Duration::from_millis(100));
            }
            _ => return Ok(()),
        }
    }
}

/// Geçici çıktının boyutu (klasörde dosyaların toplamı); yoksa `None`.
fn output_size(path: &Path, is_dir: bool) -> Option<u64> {
    let m = std::fs::metadata(path).ok()?;
    if !is_dir || !m.is_dir() {
        return Some(m.len());
    }
    let total = std::fs::read_dir(path)
        .ok()?
        .filter_map(|e| e.ok()?.metadata().ok())
        .filter(|m| m.is_file())
        .map(|m| m.len())
        .sum();
    Some(total)
}

/// Başarıda geçici çıktıyı hedefe taşır. Dosyada `rename` var olan hedefin yerine geçer (Windows'ta
/// `MOVEFILE_REPLACE_EXISTING`): hedefin hiç olmadığı bir an yoktur. Klasörde boş hedef önce silinir.
/// Hata iletisi geçici çıktının yerini söyler (yalnız kullanıcıya gider, günlüğe yazılmaz).
fn finalize(plan: &OutputPlan) -> Result<(), String> {
    let (partial, target) = (Path::new(&plan.partial_path), Path::new(&plan.final_path));
    if std::fs::symlink_metadata(partial).is_err() {
        return Err("Araç başarıyla bitti ama çıktı oluşmadı.".into());
    }
    let kept = |why: String| {
        format!(
            "{why}; çıktı geçici konumda bırakıldı: {} (aynı hedefe bir sonraki yedekleme bunu siler ve üzerine yazar)",
            plan.partial_path
        )
    };
    if plan.is_dir {
        match std::fs::metadata(target) {
            Ok(m) if m.is_dir() => {
                std::fs::remove_dir(target).map_err(|e| kept(format!("Hedef klasör boşaltılamadı ({e})")))?
            }
            Ok(_) => return Err(kept("Hedef bir dosya".into())),
            Err(_) => {}
        }
    }
    std::fs::rename(partial, target).map_err(|e| kept(format!("Çıktı hedefe taşınamadı ({e})")))
}

async fn blocking<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> Option<T> {
    tokio::task::spawn_blocking(f).await.ok()
}

/// Sınırlı satır okuyucu: satır başına en çok `RAW_LINE_CAP` bayt tutar, fazlasını satır sonuna dek atar.
/// İptal güvenlidir: okunan bayt durumda tutulur, `select!` dalı düşse de kaybolmaz.
struct LineReader<R> {
    inner: R,
    buf: Vec<u8>,
    over: bool,
}

impl<R: AsyncBufRead + Unpin> LineReader<R> {
    fn new(inner: R) -> Self {
        LineReader { inner, buf: Vec::new(), over: false }
    }

    async fn next_line(&mut self) -> io::Result<Option<String>> {
        loop {
            let avail = self.inner.fill_buf().await?;
            if avail.is_empty() {
                return Ok((!self.buf.is_empty() || self.over).then(|| self.take()));
            }
            let (chunk, used, end) = match avail.iter().position(|b| *b == b'\n') {
                Some(i) => (&avail[..i], i + 1, true),
                None => (avail, avail.len(), false),
            };
            let room = RAW_LINE_CAP.saturating_sub(self.buf.len());
            if chunk.len() > room {
                self.over = true;
            }
            self.buf.extend_from_slice(&chunk[..chunk.len().min(room)]);
            self.inner.consume(used);
            if end {
                return Ok(Some(self.take()));
            }
        }
    }

    fn take(&mut self) -> String {
        let mut raw = std::mem::take(&mut self.buf);
        let over = std::mem::replace(&mut self.over, false);
        if !over && raw.last() == Some(&b'\r') {
            raw.pop();
        }
        let mut text = String::from_utf8_lossy(&raw).into_owned();
        if over || text.len() > MAX_LINE_BYTES {
            let mut cut = MAX_LINE_BYTES.min(text.len());
            while !text.is_char_boundary(cut) {
                cut -= 1;
            }
            text.truncate(cut);
            text.push('…');
        }
        text
    }
}

/// Satır sayaçları ve ilerleme durumu.
struct State {
    kind: JobKind,
    tool: ToolKind,
    /// `pg_restore`'un bildirdiği atlanan hata sayısı (`errors ignored on restore: N`).
    ignored: Option<u32>,
    total: Option<u32>,
    shown: usize,
    truncated: u32,
    warnings: u32,
    errors: u32,
    done: u32,
    /// İpucu için: gösterilen satırlar ve sınırdan sonraki son satırlar.
    kept: Vec<String>,
    tail: VecDeque<String>,
    /// Son gönderilen `(done, bytes)`; satır kaynaklı `Progress` yalnız değişince gider (tık her zaman gönderir).
    last_progress: (u32, Option<u64>),
}

impl State {
    fn new(kind: JobKind, tool: ToolKind, total: Option<u32>) -> Self {
        State {
            kind,
            tool,
            ignored: None,
            total,
            shown: 0,
            truncated: 0,
            warnings: 0,
            errors: 0,
            done: 0,
            kept: Vec::new(),
            tail: VecDeque::new(),
            last_progress: (0, None),
        }
    }

    /// Araç satırı; alıcı gittiyse `false`.
    fn line(&mut self, text: String, sink: &mut dyn EventSink, bytes: Option<u64>) -> bool {
        let severity = classify(&text);
        match severity {
            LineSeverity::Warning => self.warnings += 1,
            LineSeverity::Error => self.errors += 1,
            LineSeverity::Info => {}
        }
        if self.kind == JobKind::Restore
            && self.tool == ToolKind::PgRestore
            && let Some(n) = ignored_errors(&text)
        {
            self.ignored = Some(n);
        }
        let tick = progress_tick(&text, self.kind);
        if tick {
            self.done += 1;
        }
        if self.shown < MAX_LOG_LINES {
            self.shown += 1;
            self.kept.push(text.clone());
            if !sink.send(BackupEvent::Line { severity, text }) {
                return false;
            }
        } else {
            self.truncated += 1;
            if self.tail.len() == HINT_TAIL {
                self.tail.pop_front();
            }
            self.tail.push_back(text);
        }
        !tick || self.progress(sink, bytes)
    }

    /// `done` ya da `bytes` değiştiyse `Progress` gönderir; alıcı gittiyse `false`.
    fn progress(&mut self, sink: &mut dyn EventSink, bytes: Option<u64>) -> bool {
        if (self.done, bytes) == self.last_progress {
            return true;
        }
        self.heartbeat(sink, bytes)
    }

    /// Değişmemiş olsa da `Progress` gönderir (1 sn tıkı): araç sessizken de alıcının gittiği anlaşılır ve iş
    /// iptal edilir. Alıcı gittiyse `false`.
    fn heartbeat(&mut self, sink: &mut dyn EventSink, bytes: Option<u64>) -> bool {
        self.last_progress = (self.done, bytes);
        sink.send(BackupEvent::Progress { done: self.done, total: self.total, bytes })
    }

    /// Yürütücünün kendi hata satırı: satır sınırına bağlı değildir, ipucuna katılmaz.
    fn own_error(&mut self, sink: &mut dyn EventSink, text: String) {
        self.errors += 1;
        sink.send(BackupEvent::Line { severity: LineSeverity::Error, text });
    }

    fn hint(&self) -> Option<String> {
        let mut lines = self.kept.clone();
        lines.extend(self.tail.iter().cloned());
        failure_hint_for(&lines, self.tool).map(str::to_string)
    }
}

enum Step {
    Cancel,
    Exited(io::Result<ExitStatus>),
}

/// Aracı çalıştırır, bitene ya da iptal edilene dek olayları akıtır. Çıktı taşıma/silme çağırana kalır.
async fn execute(
    inv: &Invocation,
    password: Option<Secret>,
    st: &mut State,
    sink: &mut dyn EventSink,
    cancel: &CancelToken,
) -> (JobOutcome, Option<i32>) {
    if !sink.send(BackupEvent::Started { preview: inv.preview.clone() }) || cancel.is_cancelled() {
        return (JobOutcome::Cancelled, None);
    }
    if let Some(plan) = &inv.output {
        if let Err(m) = check_output(plan) {
            st.own_error(sink, m);
            return (JobOutcome::Failed, None);
        }
        let partial = plan.partial_path.clone();
        match blocking(move || remove_partial(Path::new(&partial))).await {
            Some(Ok(())) => {}
            Some(Err(e)) => {
                st.own_error(sink, format!("Önceki bir işten kalan geçici çıktı silinemedi: {e}"));
                return (JobOutcome::Failed, None);
            }
            None => {
                st.own_error(sink, "Önceki bir işten kalan geçici çıktı silinemedi.".into());
                return (JobOutcome::Failed, None);
            }
        }
    }
    if !invocation_ansi_safe(inv, password.as_ref()) {
        st.own_error(sink, ANSI_RUN_MESSAGE.into());
        return (JobOutcome::Failed, None);
    }
    let mut cmd = command(inv, password.as_ref());
    drop(password);
    let spawned = cmd.spawn();
    // Ortam kopyası (parola dahil) komutla birlikte hemen düşer.
    drop(cmd);
    let mut child = match spawned {
        Ok(c) => c,
        Err(e) => {
            st.own_error(sink, format!("Araç başlatılamadı: {e}"));
            return (JobOutcome::Failed, None);
        }
    };
    let Some(stderr) = child.stderr.take() else {
        let _ = child.kill().await;
        st.own_error(sink, "Araç çıktısı okunamadı.".into());
        return (JobOutcome::Failed, None);
    };
    let mut reader = LineReader::new(BufReader::new(stderr));
    let mut ticker = tokio::time::interval_at(tokio::time::Instant::now() + PROGRESS_INTERVAL, PROGRESS_INTERVAL);
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    let mut bytes: Option<u64> = None;
    let mut eof = false;
    let step = loop {
        tokio::select! {
            biased;
            () = cancel.cancelled() => break Step::Cancel,
            line = reader.next_line(), if !eof => match line {
                Ok(Some(text)) => {
                    if !st.line(text, sink, bytes) {
                        break Step::Cancel;
                    }
                }
                // Okuma hatası akışın sonu sayılır; çıkış kodu sonucu belirler.
                Ok(None) | Err(_) => eof = true,
            },
            _ = ticker.tick() => {
                if let Some(plan) = &inv.output {
                    let (path, is_dir) = (plan.partial_path.clone(), plan.is_dir);
                    bytes = blocking(move || output_size(Path::new(&path), is_dir)).await.flatten();
                }
                if !st.heartbeat(sink, bytes) {
                    break Step::Cancel;
                }
            }
            // Bekleme stderr kapanınca: satırlar kaybolmaz.
            status = child.wait(), if eof => break Step::Exited(status),
        }
    };
    match step {
        Step::Cancel => {
            // `kill` sonlandırır ve bekler (Windows'ta `TerminateProcess`).
            let _ = child.kill().await;
            (JobOutcome::Cancelled, None)
        }
        Step::Exited(Ok(status)) if status.success() => (JobOutcome::Success, status.code()),
        // pg_restore "hatada dur" kapalıyken atlanan hatalarla 1 döner; sayı satırı görüldüyse iş sonuna dek
        // yürümüştür (bağlantı ya da arşiv okuma hatası gibi erken çıkışlar bu satırı yazmaz).
        Step::Exited(Ok(status))
            if status.code() == Some(1)
                && inv.tolerates_ignored_errors
                && st.kind == JobKind::Restore
                && st.tool == ToolKind::PgRestore
                && st.ignored.is_some_and(|n| n > 0) =>
        {
            (JobOutcome::CompletedWithErrors, status.code())
        }
        Step::Exited(Ok(status)) => (JobOutcome::Failed, status.code()),
        Step::Exited(Err(e)) => {
            st.own_error(sink, format!("Aracın bitişi beklenemedi: {e}"));
            (JobOutcome::Failed, None)
        }
    }
}

fn kind_str(kind: JobKind) -> &'static str {
    match kind {
        JobKind::Backup => "backup",
        JobKind::Restore => "restore",
    }
}

/// İşi çalıştırır ve özetini döner. Olaylar: `Started` → `Line`/`Progress` → (taşma bildirimi) → `Finished`.
/// `total` ilerlemenin paydasıdır (bilinmiyorsa `None`). `sink` `false` dönerse iş iptal edilir.
pub async fn run(
    inv: Invocation,
    password: Option<Secret>,
    kind: JobKind,
    total: Option<u32>,
    sink: &mut dyn EventSink,
    cancel: CancelToken,
) -> JobSummary {
    let started = Instant::now();
    let mut st = State::new(kind, inv.tool, total);
    let (mut outcome, exit_code) = execute(&inv, password, &mut st, sink, &cancel).await;

    let mut output_path = None;
    if let Some(plan) = inv.output.clone() {
        if outcome == JobOutcome::Success {
            let p = plan.clone();
            match blocking(move || finalize(&p)).await {
                Some(Ok(())) => output_path = Some(plan.final_path),
                Some(Err(m)) => {
                    st.own_error(sink, m);
                    outcome = JobOutcome::Failed;
                }
                None => {
                    st.own_error(sink, "Çıktı hedefe taşınamadı.".into());
                    outcome = JobOutcome::Failed;
                }
            }
        } else {
            let partial = plan.partial_path.clone();
            if !matches!(blocking(move || remove_partial(Path::new(&partial))).await, Some(Ok(()))) {
                sink.send(BackupEvent::Line {
                    severity: LineSeverity::Warning,
                    text: format!("Geçici çıktı silinemedi: {}", plan.partial_path),
                });
                st.warnings += 1;
            }
        }
    }

    if st.truncated > 0 {
        sink.send(BackupEvent::Line {
            severity: LineSeverity::Info,
            text: format!("… {} satır daha gösterilmedi (günlük sınırı {MAX_LOG_LINES} satır).", st.truncated),
        });
    }
    let summary = JobSummary {
        outcome,
        exit_code,
        elapsed_ms: u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX),
        warnings: st.warnings,
        errors: st.errors,
        output_path,
        hint: if matches!(outcome, JobOutcome::Failed | JobOutcome::CompletedWithErrors) { st.hint() } else { None },
        truncated_lines: st.truncated,
    };
    sink.send(BackupEvent::from(&summary));
    tracing::info!(
        job = kind_str(kind),
        outcome = outcome.as_str(),
        exit_code = summary.exit_code,
        elapsed_ms = summary.elapsed_ms,
        "iş bitti"
    );
    summary
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn read_all(input: &[u8]) -> Vec<String> {
        let mut r = LineReader::new(input);
        let mut out = Vec::new();
        while let Some(l) = r.next_line().await.unwrap() {
            out.push(l);
        }
        out
    }

    #[tokio::test]
    async fn line_reader_splits_and_bounds() {
        assert_eq!(read_all(b"a\r\nb\n\nson").await, vec!["a", "b", "", "son"]);
        assert_eq!(read_all(b"").await, Vec::<String>::new());
        // Geçersiz UTF-8 kayıplı çözülür.
        assert_eq!(read_all(b"x\xffy\n").await, vec!["x\u{fffd}y"]);
        let long = format!("{}\nsonraki\n", "a".repeat(10_000));
        let lines = read_all(long.as_bytes()).await;
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0], format!("{}…", "a".repeat(MAX_LINE_BYTES)));
        assert_eq!(lines[1], "sonraki");
    }

    #[test]
    fn secret_is_not_printed() {
        let s = Secret::new("gizli".into());
        assert_eq!(format!("{s:?}"), "Secret(***)");
        assert_eq!(serde_json::to_string(&s).unwrap(), "\"***\"");
        assert_eq!(s.expose(), "gizli");
    }

    #[test]
    fn runner_refuses_values_that_do_not_round_trip() {
        let args = vec!["--file".to_string(), "C:/y/a.dump".to_string()];
        assert!(ansi_safe("C:/bin/pg_dump.exe", &args, ["PGHOST", "db.local"]));
        // Best-fit ile değişen değer (sistem ACP'si UTF-8 değilse; Windows dışında her değer geçer).
        let bad = !crate::ansi::ensure_ansi_roundtrip("＂");
        assert_eq!(cfg!(windows) && bad, !ansi_safe("C:/bin/pg_dump.exe", &args, ["PGPASSWORD", "a＂b"]));
        assert_eq!(cfg!(windows) && bad, !ansi_safe("C:/bin＂/pg_dump.exe", &args, []));
        let args = vec!["C:/y/x＂ -h evil ＂.dump".to_string()];
        assert_eq!(cfg!(windows) && bad, !ansi_safe("C:/bin/pg_dump.exe", &args, []));
    }

    #[test]
    fn pg_keys_are_case_insensitive() {
        for k in ["PGPASSWORD", "pgservice", "PgOptions", "PG"] {
            assert!(is_pg_key(OsStr::new(k)), "{k}");
        }
        for k in ["PATH", "P", "", "SYSTEMROOT", "ĞPG"] {
            assert!(!is_pg_key(OsStr::new(k)), "{k}");
        }
    }
}
