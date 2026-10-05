//! Sorgu oturumları (`QueryManager`): sekme başına ayrılmış bir bağlantı, transaction durum makinesi, salt okunur
//! kilit ve onay kapısı, iptal, boşta imleç ve boşta oturum yönetimi.
//!
//! - **Kilit bir korumadır, güvenlik sınırı değildir.** Kilitli sunucuda oturum `default_transaction_read_only = on`
//!   ile açılır; `run` yazma/şema ifadelerini ve oturumun salt okunur ayarını değiştirebilecek ifadeleri
//!   (`Analysis.changes_read_only`) çalıştırmadan reddeder. Sınıflandırıcının göremediği yollar (dinamik SQL,
//!   fonksiyon gövdeleri) yine de ayarı değiştirebilir; asıl yetki sınırı veritabanı rolünün yetkileridir.
//! - **Onay kapısı** arayüzden bağımsızdır: tehlikeli ya da üretim sunucusunda yazan bir ifade `confirmed = true`
//!   olmadan çalışmaz (`Error::Invalid("confirmation_required")`).
//! - **Transaction durumu** sınıflandırmaya dayalı bir durum makinesiyle izlenir; her `Tx` sınıfı ifade ve her hata
//!   sonrası sunucu yoklanır (`statement_timestamp() <> transaction_timestamp()`) ve çelişkide yoklama kazanır.
//!
//! SQL metni asla günlüğe yazılmaz.

use std::collections::HashMap;
use std::future::Future;
use std::ops::Range;
use std::path::Path as StdPath;
use std::sync::{Arc, Mutex as StdMutex, PoisonError, RwLock};
use std::time::Duration;

use pgt_core::connection::{CancelHandle, ConnectionManager, MAX_STATEMENT_TIMEOUT_MS, SessionProfile, Target};
use pgt_core::registry::Registry;
use pgt_core::{Error, ErrorPayload, Result};
use serde::{Deserialize, Serialize};
use tokio::sync::{Mutex, Notify};
use tokio::task::JoinHandle;
use tokio::time::Instant;
use ts_rs::TS;
use uuid::Uuid;

use crate::complete::{CompletionItem, CompletionRequest, lookup};
use crate::classify::{Danger, StmtKind, TxOp, analyze as classify, tx_op};
use crate::events::{EventSink, QueryEvent};
use crate::exec::{DEFAULT_CHUNK, ExecConfig, Executor, Path, StmtOutcome, cursor_capable};
use crate::export::{CsvFile, CsvOptions, EXPORT_CHUNK, ExportSummary, check_target};
use crate::split::{split, stmt_at};

pub type SessionId = Uuid;

/// Onay gerektiren betikte `run(…, confirmed = false)` hatasının iletisi (arayüz onay diyaloğuna çevirir).
pub const CONFIRMATION_REQUIRED: &str = "confirmation_required";
pub const LOCKED_WRITE_MESSAGE: &str = "Bu sunucu salt okunur kilitli: yazma ve şema değiştiren ifadeler çalıştırılamaz.";
pub const LOCKED_SETTING_MESSAGE: &str = "Bu sunucu salt okunur kilitli: oturumun salt okunur ayarı değiştirilemez.";
pub const TX_FAILED_MESSAGE: &str = "Transaction hata durumunda; önce ROLLBACK yapın.";
pub const BUSY_MESSAGE: &str = "Bu sekmede bir sorgu zaten çalışıyor.";
pub const OPEN_TX_MESSAGE: &str = "Açık transaction var.";
pub const MODE_IN_TX_MESSAGE: &str = "Açık transaction varken mod değiştirilemez.";
/// Kullanıcının iptal ettiği ifadenin `Failed` iletisi (zaman aşımından ayırt edilir; sunucu iletisi `detail`'de).
pub const CANCELLED_MESSAGE: &str = "Sorgu kullanıcı tarafından iptal edildi.";
/// `more()` almayan açık imlecin kapatılma süresi (varsayılan).
pub const DEFAULT_CURSOR_IDLE: Duration = Duration::from_secs(5 * 60);

/// Tamamlama önbelleğinin tazelik süresi, bir aramanın en uzun süresi ve önbellek girdisi sınırı.
pub const COMPLETION_TTL: Duration = Duration::from_secs(60);
const COMPLETION_TIMEOUT: Duration = Duration::from_secs(5);
const COMPLETION_CACHE_MAX: usize = 256;

const PROBE: &str = "SELECT statement_timestamp() <> transaction_timestamp()";
/// Oturumlar zorla kapatılırken ROLLBACK ve kilit için en çok bekleme.
const FORCE_CLOSE_WAIT: Duration = Duration::from_secs(5);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum TxState {
    Idle,
    InTransaction,
    /// Transaction hata aldı (PostgreSQL `aborted`): yalnızca ROLLBACK / ROLLBACK TO kabul edilir.
    Failed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum TxMode {
    #[default]
    Autocommit,
    /// İlk ifadeden önce otomatik `BEGIN`; COMMIT/ROLLBACK kullanıcıdadır.
    Manual,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct SessionOptions {
    pub mode: TxMode,
    /// 0 = sınırsız; en çok `MAX_STATEMENT_TIMEOUT_MS`.
    pub statement_timeout_ms: u32,
}

/// Çalıştırılacak ifadelerin seçimi. Ofsetler editör metnindeki **karakter** (Unicode scalar) ofsetleridir.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(tag = "type", rename_all = "camelCase", rename_all_fields = "camelCase")]
#[ts(export)]
pub enum Scope {
    All,
    /// `[start, end)` seçili metin kendi başına bir betik olarak ayrılır.
    Selection { start: u32, end: u32 },
    /// İmlecin bulunduğu tek ifade.
    AtCursor { offset: u32 },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum TxAction {
    Commit,
    Rollback,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum ClosePolicy {
    Commit,
    Rollback,
    /// Açık transaction varsa kapatma (`Err("Açık transaction var.")`).
    Refuse,
}

/// Kilitli sunucuda bir ifadenin neden reddedildiği.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum LockedReason {
    Write,
    Ddl,
    /// Oturumun salt okunur ayarını değiştirebilecek ifade (`SET … read_only`, `BEGIN READ WRITE`, `RESET ALL`…).
    ReadOnlySetting,
}

/// Çalıştırmadan önceki ifade planı. `index` olaylardaki `stmt_index` ile aynıdır (kapsam içi sıra);
/// `char_offset` ifadenin editör metnindeki karakter ofsetidir.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct StmtPlan {
    pub index: u32,
    pub kind: StmtKind,
    pub danger: Option<Danger>,
    /// Üretim etiketli sunucuda yazma/şema ifadesi (onay gerekir).
    pub production_write: bool,
    /// Kilitli sunucuda reddedilecek ifade (betiğin tamamı başlamaz).
    pub locked_reject: bool,
    pub locked_reason: Option<LockedReason>,
    /// Parola/bağlantı dizesi içerebilir; geçmişe yazılmamalı.
    pub sensitive: bool,
    pub char_offset: u32,
}

// ---------------------------------------------------------------------------------------------------------------
// Saf kurallar: plan, kapılar, durum makinesi
// ---------------------------------------------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) struct Guard {
    pub(crate) locked: bool,
    pub(crate) production: bool,
}

#[derive(Debug, Clone)]
pub(crate) struct Planned {
    pub(crate) plan: StmtPlan,
    /// İfadenin `sql` içindeki bayt aralığı.
    pub(crate) range: Range<usize>,
    pub(crate) tx: Option<TxOp>,
}

fn byte_at_char(sql: &str, ch: u32) -> usize {
    sql.char_indices().nth(ch as usize).map_or(sql.len(), |(b, _)| b)
}

/// Kapsamın ifadelerinin bayt aralıkları (sırayla).
fn scope_ranges(sql: &str, scope: Scope) -> Result<Vec<Range<usize>>> {
    match scope {
        Scope::All => Ok(split(sql).into_iter().map(|s| s.start..s.end).collect()),
        Scope::Selection { start, end } => {
            if start > end {
                return Err(Error::Invalid("Seçim aralığı geçersiz.".into()));
            }
            let (b0, b1) = (byte_at_char(sql, start), byte_at_char(sql, end));
            Ok(split(&sql[b0..b1]).into_iter().map(|s| b0 + s.start..b0 + s.end).collect())
        }
        Scope::AtCursor { offset } => {
            let at = stmt_at(sql, byte_at_char(sql, offset));
            Ok(at.and_then(|i| split(sql).get(i).map(|s| s.start..s.end)).into_iter().collect())
        }
    }
}

pub(crate) fn plan_script(sql: &str, scope: Scope, guard: Guard) -> Result<Vec<Planned>> {
    let ranges = scope_ranges(sql, scope)?;
    let (mut chars, mut at) = (0u32, 0usize);
    let mut out = Vec::with_capacity(ranges.len());
    for (i, range) in ranges.into_iter().enumerate() {
        chars += sql[at..range.start].chars().count() as u32;
        at = range.start;
        let text = &sql[range.clone()];
        let a = classify(text);
        let writes = matches!(a.kind, StmtKind::Write | StmtKind::Ddl);
        let locked_reason = if !guard.locked {
            None
        } else if a.kind == StmtKind::Write {
            Some(LockedReason::Write)
        } else if a.kind == StmtKind::Ddl {
            Some(LockedReason::Ddl)
        } else if a.changes_read_only {
            Some(LockedReason::ReadOnlySetting)
        } else {
            None
        };
        out.push(Planned {
            plan: StmtPlan {
                index: i as u32,
                kind: a.kind,
                danger: a.danger,
                production_write: guard.production && writes,
                locked_reject: locked_reason.is_some(),
                locked_reason,
                sensitive: a.sensitive,
                char_offset: chars,
            },
            range,
            tx: tx_op(text),
        });
    }
    Ok(out)
}

/// Çalıştırma öncesi kapılar: önce kilit (tek ifade bile tüm betiği engeller), sonra onay.
pub(crate) fn check_gates(planned: &[Planned], confirmed: bool) -> Result<()> {
    if let Some(p) = planned.iter().find(|p| p.plan.locked_reject) {
        let msg = match p.plan.locked_reason {
            Some(LockedReason::ReadOnlySetting) => LOCKED_SETTING_MESSAGE,
            _ => LOCKED_WRITE_MESSAGE,
        };
        return Err(Error::Invalid(msg.into()));
    }
    if !confirmed && planned.iter().any(|p| p.plan.danger.is_some() || p.plan.production_write) {
        return Err(Error::Invalid(CONFIRMATION_REQUIRED.into()));
    }
    Ok(())
}

/// Durum makinesinin girdisi: ifadenin transaction işlemi (yoksa `None`) ve başarılı olup olmadığı.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TxEvent {
    Ok(Option<TxOp>),
    Error(Option<TxOp>),
}

/// Sınıflandırmaya dayalı geçiş. `Failed` iken yalnızca ROLLBACK / ROLLBACK TO kabul edilir (diğerleri `Err`;
/// `COMMIT` de reddedilir: PostgreSQL onu sessizce ROLLBACK'e çevirirdi).
pub(crate) fn step(state: TxState, ev: TxEvent) -> Result<TxState> {
    use TxState::{Failed, Idle, InTransaction};
    let op = match ev {
        TxEvent::Ok(op) | TxEvent::Error(op) => op,
    };
    if state == Failed && !matches!(op, Some(TxOp::Rollback | TxOp::RollbackTo)) {
        return Err(Error::Invalid(TX_FAILED_MESSAGE.into()));
    }
    Ok(match ev {
        TxEvent::Ok(Some(TxOp::Begin | TxOp::RollbackTo)) => InTransaction,
        TxEvent::Ok(Some(TxOp::Commit | TxOp::Rollback)) => Idle,
        TxEvent::Ok(_) => state,
        // Başarısız COMMIT transaction'ı bitirir (geri alınır).
        TxEvent::Error(Some(TxOp::Commit)) => Idle,
        TxEvent::Error(_) if state == Idle => Idle,
        TxEvent::Error(_) => Failed,
    })
}

// ---------------------------------------------------------------------------------------------------------------
// İptal
// ---------------------------------------------------------------------------------------------------------------

#[derive(Default)]
struct CancelFlags {
    /// Bir `run`/`more` sürüyor.
    active: bool,
    requested: bool,
    /// Oturum kapatıldı (kalıcı). Kapatma, oturum kilidini beklemeden işaretlenir: `begin()` iptali sıfırlamaz,
    /// kilidi sonradan alan işlemler reddedilir ve kaldırılmış bir oturum yeniden bağlanmaz.
    closed: bool,
}

/// Oturum kilidinden bağımsız iptal belirteci. Boştayken gelen iptal yok sayılır (sonraki bir ifadeyi vurmasın).
#[derive(Default)]
struct CancelToken {
    flags: StdMutex<CancelFlags>,
    notify: Notify,
}

impl CancelToken {
    fn flags(&self) -> std::sync::MutexGuard<'_, CancelFlags> {
        self.flags.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn begin(&self) {
        let mut f = self.flags();
        f.active = true;
        f.requested = f.closed;
    }

    fn end(&self) {
        let mut f = self.flags();
        f.active = false;
        f.requested = false;
    }

    /// Oturumu kapalı işaretler ve çalışan işlemi iptal eder.
    fn close(&self) {
        let mut f = self.flags();
        f.closed = true;
        f.requested = true;
        self.notify.notify_one();
    }

    fn closed(&self) -> bool {
        self.flags().closed
    }

    /// Çalışma sürüyorsa iptal ister (`true`); boştaysa yok sayar.
    fn request(&self) -> bool {
        let mut f = self.flags();
        if f.active {
            f.requested = true;
            self.notify.notify_one();
        }
        f.active
    }

    fn requested(&self) -> bool {
        self.flags().requested
    }
}

async fn cancel_quietly(handle: &CancelHandle) {
    if let Err(e) = handle.cancel().await {
        tracing::debug!(code = %ErrorPayload::from(&e).code, "iptal isteği gönderilemedi");
    }
}

/// `fut`'u çalıştırır; sürerken her iptal isteği (`request()` başına bir `notify`) sunucuya bir iptal isteği
/// gönderir. PostgreSQL, iki gidiş-dönüş arasında (arka uç boştayken) gelen iptali yok sayar; tekrar basılan iptal
/// bu yüzden yeniden gönderilmelidir. Önceden kalmış `Notify` izni `requested` bayrağı olmadan yok sayılır
/// (boştayken gelen iptal `request()` tarafından zaten düşürülür).
async fn cancellable<F: Future>(token: &CancelToken, handle: &CancelHandle, fut: F) -> F::Output {
    cancellable_with(token, || cancel_quietly(handle), fut).await
}

async fn cancellable_with<F, C, CF>(token: &CancelToken, mut cancel: C, fut: F) -> F::Output
where
    F: Future,
    C: FnMut() -> CF,
    CF: Future<Output = ()>,
{
    let mut fut = std::pin::pin!(fut);
    loop {
        tokio::select! {
            biased;
            out = &mut fut => return out,
            () = token.notify.notified() => {
                if token.requested() {
                    cancel().await;
                }
            }
        }
    }
}

fn timeout_text() -> String {
    let e = Error::Query { sqlstate: Some("57014".into()), message: String::new(), detail: None, hint: None };
    ErrorPayload::from(&e).message
}

fn is_cancel_error(e: &Error) -> bool {
    matches!(e, Error::Query { sqlstate: Some(s), .. } if s == "57014")
}

/// İfadeler arasında (ya da yeniden bağlanma/otomatik BEGIN/yoklama sırasında) gelen iptal: sıradaki ifade
/// başlamaz ama kullanıcı yine iptal iletisini görür.
fn cancelled_event(stmt_index: u32, char_offset: u32) -> QueryEvent {
    QueryEvent::Failed {
        stmt_index,
        error: ErrorPayload { code: "query".into(), message: CANCELLED_MESSAGE.into(), detail: None, hint: None },
        position: None,
        char_offset,
        notices: Vec::new(),
    }
}

/// Manuel kipte otomatik BEGIN'i atlayan ifadeler: transaction'ı kendisi açan/bitirenler ve transaction bloğu
/// dışında çalışması gerekenler (`COMMIT/ROLLBACK PREPARED`, `PREPARE TRANSACTION`). SAVEPOINT, RELEASE,
/// ROLLBACK TO ve SET TRANSACTION otomatik BEGIN alır.
fn skips_auto_begin(sql: &str, tx: Option<TxOp>) -> bool {
    match tx {
        Some(TxOp::Begin | TxOp::Commit | TxOp::Rollback) => true,
        Some(TxOp::Other) => {
            let first = crate::lex::lex(sql).into_iter().find(|t| t.kind == crate::lex::Kind::Word);
            !first.is_some_and(|t| t.is_word(sql, "set"))
        }
        _ => false,
    }
}

/// Olayları kullanıcı alıcısına aktarır: kullanıcı iptalinde 57014 iletisini değiştirir, hata ve alıcının
/// gidişini kaydeder.
struct GuardSink<'a> {
    inner: &'a mut dyn EventSink,
    cancel: &'a CancelToken,
    timeout_text: String,
    gone: bool,
    failed: bool,
}

impl<'a> GuardSink<'a> {
    fn new(inner: &'a mut dyn EventSink, cancel: &'a CancelToken) -> Self {
        GuardSink { inner, cancel, timeout_text: timeout_text(), gone: false, failed: false }
    }
}

impl EventSink for GuardSink<'_> {
    fn send(&mut self, mut ev: QueryEvent) -> bool {
        if self.gone {
            return false;
        }
        if let QueryEvent::Failed { error, .. } = &mut ev {
            self.failed = true;
            if self.cancel.requested() && error.code == "query" && error.message == self.timeout_text {
                error.message = CANCELLED_MESSAGE.into();
            }
        }
        let ok = self.inner.send(ev);
        self.gone = !ok;
        ok
    }
}

// ---------------------------------------------------------------------------------------------------------------
// Oturumlar
// ---------------------------------------------------------------------------------------------------------------

struct Inner {
    /// `None`: oturum uykuda (boşta kaldığı için bağlantı kapatıldı); sonraki işlem yeniden bağlanır.
    exec: Option<Executor>,
    opts: SessionOptions,
    /// Sunucuda oturum düzeyinde (transaction dışında) geçerli olduğu bilinen `statement_timeout`.
    applied_timeout: Option<u32>,
    last_used: Instant,
    cursor_timer: Option<JoinHandle<()>>,
    /// Her işlemde artar; boşta imleç zamanlayıcısı yalnızca kendi kuşağındaysa imleci kapatır.
    cursor_gen: u64,
}

struct SessionSlot {
    server: Uuid,
    target: Target,
    guard: Guard,
    /// Kilitsiz okunur (`tx_state`); yalnızca `inner` kilidi tutulurken yazılır.
    state: StdMutex<TxState>,
    inner: Mutex<Inner>,
    cancel: CancelToken,
    /// Tamamlama önbelleği: istek -> (zaman, sonuç); 60 sn tazelik, DDL/transaction sonrası boşaltılır.
    completions: StdMutex<HashMap<CompletionRequest, (Instant, Vec<CompletionItem>)>>,
}

impl SessionSlot {
    fn state(&self) -> TxState {
        *self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn completions(&self) -> std::sync::MutexGuard<'_, HashMap<CompletionRequest, (Instant, Vec<CompletionItem>)>> {
        self.completions.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn invalidate_completions(&self) {
        self.completions().clear();
    }

    fn set_state(&self, s: TxState) {
        *self.state.lock().unwrap_or_else(PoisonError::into_inner) = s;
    }

    fn profile(&self, opts: &SessionOptions) -> SessionProfile {
        SessionProfile::Query { read_only: self.guard.locked, statement_timeout_ms: opts.statement_timeout_ms }
    }
}

fn busy() -> Error {
    Error::Invalid(BUSY_MESSAGE.into())
}

fn session_gone() -> Error {
    Error::NotFound("sorgu oturumu".into())
}

/// Oturum kilidini bekletmeden alır; meşgulse `BUSY`, bu arada kapatıldıysa `NotFound`.
fn lock_open(slot: &SessionSlot) -> Result<tokio::sync::MutexGuard<'_, Inner>> {
    let inner = slot.inner.try_lock().map_err(|_| busy())?;
    if slot.cancel.closed() {
        return Err(session_gone());
    }
    Ok(inner)
}

/// Oturumu zorla kapatır: kapalı işaretler (çalışan ifade iptal edilir), kilidi en çok `FORCE_CLOSE_WAIT`
/// bekler, açık transaction'ı geri alır ve bağlantıyı bırakır. Kilit zamanında alınamazsa çalışan işlem
/// bitince `finish` bağlantıyı bırakır (sunucu transaction'ı geri alır). Oturum haritadan önceden çıkarılmalıdır.
async fn force_close(slot: &SessionSlot) {
    slot.cancel.close();
    let Ok(mut inner) = tokio::time::timeout(FORCE_CLOSE_WAIT, slot.inner.lock()).await else {
        tracing::warn!(server = %slot.server, "sorgu oturumu zamanında durmadı; bitince kapanacak");
        return;
    };
    disarm(&mut inner);
    if let Some(mut exec) = inner.exec.take()
        && (slot.state() != TxState::Idle || exec.cursor_open())
    {
        // Bağlantı kapanınca sunucu da geri alır; açık ROLLBACK kısa bir süre sınırıyla denenir.
        let _ = tokio::time::timeout(FORCE_CLOSE_WAIT, exec.exec_internal("ROLLBACK")).await;
    }
    slot.set_state(TxState::Idle);
}

fn disarm(inner: &mut Inner) {
    if let Some(h) = inner.cursor_timer.take() {
        h.abort();
    }
    inner.cursor_gen += 1;
}

/// Açık imleç varsa `idle` sonra kapatan görevi kurar (autocommit'te çalıştırıcının COMMIT'li kapanış yolu;
/// böylece `idle in transaction` kalmaz).
fn arm_cursor_timer(slot: &Arc<SessionSlot>, inner: &mut Inner, idle: Duration) {
    disarm(inner);
    if !inner.exec.as_ref().is_some_and(Executor::cursor_open) {
        return;
    }
    let generation = inner.cursor_gen;
    let weak = Arc::downgrade(slot);
    inner.cursor_timer = Some(tokio::spawn(async move {
        tokio::time::sleep(idle).await;
        let Some(slot) = weak.upgrade() else { return };
        let mut inner = slot.inner.lock().await;
        if inner.cursor_gen != generation {
            return;
        }
        inner.cursor_timer = None;
        if let Some(exec) = inner.exec.as_mut()
            && exec.cursor_open()
        {
            match exec.close_cursor().await {
                Ok(()) => tracing::debug!("boşta kalan imleç kapatıldı"),
                Err(e) => tracing::debug!(code = %ErrorPayload::from(&e).code, "boşta kalan imleç kapatılamadı"),
            }
        }
    }));
}

async fn close_cursor_quietly(exec: &mut Executor) {
    if let Err(e) = exec.close_cursor().await {
        tracing::debug!(code = %ErrorPayload::from(&e).code, "imleç kapatılamadı");
    }
}

/// Sunucuyu yoklayarak transaction durumunu düzeltir; çelişkide yoklama kazanır.
async fn resync(slot: &SessionSlot, exec: &mut Executor) {
    let machine = slot.state();
    let probed = match exec.exec_internal(PROBE).await {
        Ok(rows) => {
            if rows.first().and_then(|r| r.first()).and_then(|v| v.as_deref()) == Some("t") {
                TxState::InTransaction
            } else {
                TxState::Idle
            }
        }
        Err(Error::Query { sqlstate: Some(code), .. }) if code == "25P02" => TxState::Failed,
        Err(e) => {
            tracing::warn!(code = %ErrorPayload::from(&e).code, "transaction durumu yoklanamadı");
            return;
        }
    };
    if probed != machine {
        tracing::warn!(?machine, ?probed, "transaction durumu yoklamayla düzeltildi");
    }
    slot.set_state(probed);
    exec.set_in_tx(probed != TxState::Idle);
}

fn user_cancelled() -> Error {
    Error::Query { sqlstate: None, message: CANCELLED_MESSAGE.into(), detail: None, hint: None }
}

/// `export_csv` gövdesi. Hata/iptalde `.tmp` (CsvFile düşünce) silinir. Transaction yönetimi:
/// - oturum transaction'da değilse ve açık ızgara imleci yoksa çalıştırıcı kendi `BEGIN`/`COMMIT`'ini kullanır;
/// - aksi halde (manuel mod ya da ızgara imlecinin transaction'ı) `SAVEPOINT pgt_export` açılır: hata/iptalde
///   `ROLLBACK TO` ile kullanıcının transaction'ı sağlam kalır, başarıda `RELEASE` edilir. Izgaranın `pgt_c`
///   imlecine dokunulmaz (yalnızca `pgt_x` açılır/kapanır).
///   Bu komutlar `exec_internal` ile gider: izlenen durum ve `in_tx` değişmez.
async fn export_run(
    slot: &SessionSlot,
    exec: &mut Executor,
    text: &str,
    path: &StdPath,
    opts: CsvOptions,
    progress: &mut (dyn FnMut(u64) -> bool + Send),
) -> Result<ExportSummary> {
    let started = std::time::Instant::now();
    let grid_cursor = exec.cursor_open();
    if !grid_cursor {
        resync(slot, exec).await;
    }
    let state = slot.state();
    if state == TxState::Failed {
        return Err(Error::Invalid(TX_FAILED_MESSAGE.into()));
    }
    let own_tx = !grid_cursor && state == TxState::Idle;
    exec.exec_internal(if own_tx { "BEGIN" } else { SAVEPOINT_EXPORT }).await?;
    let body = export_body(slot, exec, text, path, &opts, progress).await;
    match body {
        Ok((file, rows)) => {
            let finish = if own_tx { "COMMIT" } else { RELEASE_EXPORT };
            if let Err(e) = exec.exec_internal(finish).await {
                let _ = exec.exec_internal(if own_tx { "ROLLBACK" } else { ROLLBACK_EXPORT }).await;
                return Err(e);
            }
            let bytes = file.bytes;
            file.commit()?;
            Ok(ExportSummary { rows, bytes, elapsed_ms: started.elapsed().as_millis() as u64 })
        }
        Err(e) => {
            let _ = exec.exec_internal(if own_tx { "ROLLBACK" } else { ROLLBACK_EXPORT }).await;
            Err(e)
        }
    }
}

const SAVEPOINT_EXPORT: &str = "SAVEPOINT pgt_export";
const RELEASE_EXPORT: &str = "RELEASE SAVEPOINT pgt_export";
const ROLLBACK_EXPORT: &str = "ROLLBACK TO SAVEPOINT pgt_export; RELEASE SAVEPOINT pgt_export";

async fn export_body(
    slot: &SessionSlot,
    exec: &mut Executor,
    text: &str,
    path: &StdPath,
    opts: &CsvOptions,
    progress: &mut (dyn FnMut(u64) -> bool + Send),
) -> Result<(CsvFile, u64)> {
    let columns = exec.prepare_columns(text).await?;
    if columns.is_empty() {
        return Err(Error::Invalid("Yalnızca satır döndüren ifadeler dışa aktarılabilir.".into()));
    }
    let mut file = CsvFile::create(path, opts, &columns)?;
    exec.exec_internal(&format!("DECLARE pgt_x NO SCROLL CURSOR FOR {text}")).await?;
    let rows = stream_chunks(slot, exec, &mut file, progress).await?;
    exec.exec_internal("CLOSE pgt_x").await?;
    Ok((file, rows))
}

async fn stream_chunks(
    slot: &SessionSlot,
    exec: &mut Executor,
    file: &mut CsvFile,
    progress: &mut (dyn FnMut(u64) -> bool + Send),
) -> Result<u64> {
    let mut total = 0u64;
    loop {
        if slot.cancel.requested() {
            return Err(user_cancelled());
        }
        let chunk = exec.fetch_export_chunk(EXPORT_CHUNK).await?;
        file.write_rows(&chunk)?;
        total += chunk.len() as u64;
        if (!chunk.is_empty() || total == 0) && !progress(total) {
            return Err(user_cancelled());
        }
        if chunk.len() < EXPORT_CHUNK as usize {
            return Ok(total);
        }
    }
}

/// Sorgu Aracı oturumları. `Arc` ile paylaşılır; oturum başına aynı anda tek işlem (`run`/`more`/`tx`/…).
pub struct QueryManager {
    mgr: Arc<ConnectionManager>,
    registry: Arc<RwLock<Registry>>,
    sessions: StdMutex<HashMap<SessionId, Arc<SessionSlot>>>,
    /// `more()` almayan açık imlecin kapatılma süresi (varsayılan `DEFAULT_CURSOR_IDLE`).
    pub cursor_idle: Duration,
}

impl QueryManager {
    pub fn new(mgr: Arc<ConnectionManager>, registry: Arc<RwLock<Registry>>) -> Self {
        QueryManager { mgr, registry, sessions: StdMutex::new(HashMap::new()), cursor_idle: DEFAULT_CURSOR_IDLE }
    }

    fn map(&self) -> std::sync::MutexGuard<'_, HashMap<SessionId, Arc<SessionSlot>>> {
        self.sessions.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn slot(&self, session: SessionId) -> Result<Arc<SessionSlot>> {
        self.map().get(&session).cloned().ok_or_else(|| Error::NotFound("sorgu oturumu".into()))
    }

    /// Bağlı bir sunucuda yeni bir sorgu oturumu açar. Sunucunun salt okunur kilidi ve üretim etiketi açılışta
    /// okunur (değişirse oturumlar `close_sessions_of` ile kapatılmalıdır).
    pub async fn open(&self, server: Uuid, db: Target, opts: SessionOptions) -> Result<SessionId> {
        if !self.mgr.is_connected(server) {
            return Err(Error::NotConnected);
        }
        let guard = {
            let registry = self.registry.read().unwrap_or_else(PoisonError::into_inner);
            let config = registry.get(server).ok_or(Error::UnknownServer(server))?;
            Guard { locked: config.read_only_lock, production: config.is_production() }
        };
        let profile = SessionProfile::Query { read_only: guard.locked, statement_timeout_ms: opts.statement_timeout_ms };
        let conn = self.mgr.open_dedicated(server, db, profile).await?;
        let id = Uuid::new_v4();
        let slot = Arc::new(SessionSlot {
            server,
            target: db,
            guard,
            state: StdMutex::new(TxState::Idle),
            inner: Mutex::new(Inner {
                exec: Some(Executor::new(conn)),
                opts,
                applied_timeout: Some(opts.statement_timeout_ms),
                last_used: Instant::now(),
                cursor_timer: None,
                cursor_gen: 0,
            }),
            cancel: CancelToken::default(),
            completions: StdMutex::default(),
        });
        self.map().insert(id, slot);
        tracing::debug!(%server, session = %id, locked = guard.locked, "sorgu oturumu açıldı");
        Ok(id)
    }

    /// Ağa gitmeden betiğin planı (kilit/onay bayrakları dahil).
    pub fn analyze(&self, session: SessionId, sql: &str, scope: Scope) -> Result<Vec<StmtPlan>> {
        let slot = self.slot(session)?;
        Ok(plan_script(sql, scope, slot.guard)?.into_iter().map(|p| p.plan).collect())
    }

    /// Kapsamdaki ifadeleri sırayla çalıştırır; ilk hatada durur. Kapılardan geçmeyen betiğin hiçbir ifadesi
    /// çalışmaz (`Err`). Olay alıcısı giderse çalıştırma sessizce durur (`ScriptDone` gönderilmez).
    pub async fn run(
        &self,
        session: SessionId,
        sql: &str,
        scope: Scope,
        limit: u32,
        confirmed: bool,
        sink: &mut dyn EventSink,
    ) -> Result<()> {
        let slot = self.slot(session)?;
        let planned = plan_script(sql, scope, slot.guard)?;
        check_gates(&planned, confirmed)?;
        if limit == 0 {
            return Err(Error::Invalid("satır sınırı 0 olamaz".into()));
        }
        let mut inner = lock_open(&slot)?;
        if let Some(first) = planned.first() {
            step(slot.state(), TxEvent::Ok(first.tx))?;
        }
        disarm(&mut inner);
        slot.cancel.begin();
        let result = self.run_script(&slot, &mut inner, sql, &planned, limit, sink).await;
        slot.cancel.end();
        self.finish(&slot, &mut inner).await;
        result
    }

    async fn run_script(
        &self,
        slot: &SessionSlot,
        inner: &mut Inner,
        sql: &str,
        planned: &[Planned],
        limit: u32,
        sink: &mut dyn EventSink,
    ) -> Result<()> {
        if planned.is_empty() {
            sink.send(QueryEvent::ScriptDone { stmts_run: 0, stopped_on_error: false });
            return Ok(());
        }
        self.wake(slot, inner).await?;
        let mode = inner.opts.mode;
        let Some(exec) = inner.exec.as_mut() else { return Err(Error::NotConnected) };
        let cfg = ExecConfig { limit, chunk: DEFAULT_CHUNK, autocommit: mode == TxMode::Autocommit };
        let (mut stmts_run, mut stopped) = (0u32, false);
        for p in planned {
            let (idx, offset) = (p.plan.index, p.plan.char_offset);
            if slot.cancel.requested() {
                sink.send(cancelled_event(idx, offset));
                stopped = true;
                break;
            }
            let state = slot.state();
            if let Err(e) = step(state, TxEvent::Ok(p.tx)) {
                sink.send(QueryEvent::Failed {
                    stmt_index: idx,
                    error: ErrorPayload::from(&e),
                    position: None,
                    char_offset: offset,
                    notices: Vec::new(),
                });
                stopped = true;
                break;
            }
            let text = &sql[p.range.clone()];
            if mode == TxMode::Manual && state == TxState::Idle && !skips_auto_begin(text, p.tx) {
                close_cursor_quietly(exec).await;
                if let Err(e) = exec.exec_internal("BEGIN").await {
                    sink.send(QueryEvent::Failed {
                        stmt_index: idx,
                        error: ErrorPayload::from(&e),
                        position: None,
                        char_offset: offset,
                        notices: Vec::new(),
                    });
                    resync(slot, exec).await;
                    stopped = true;
                    break;
                }
                exec.set_in_tx(true);
                slot.set_state(TxState::InTransaction);
                if slot.cancel.requested() {
                    sink.send(cancelled_event(idx, offset));
                    stopped = true;
                    break;
                }
            }

            let handle = exec.cancel_handle();
            let mut guarded = GuardSink::new(&mut *sink, &slot.cancel);
            let outcome = cancellable(&slot.cancel, &handle, exec.run_statement(idx, text, offset, &cfg, &mut guarded)).await;
            let (gone, failed) = (guarded.gone, guarded.failed);
            stmts_run += 1;
            if gone && exec.last_path() == Some(Path::Direct) {
                // Doğrudan yolun akışı bırakıldı ama sunucu ifadeyi sürdürüyor olabilir: iptal edilir.
                cancel_quietly(&handle).await;
            }
            let ok = outcome == StmtOutcome::Completed;
            if ok && p.plan.kind != StmtKind::Select {
                // DDL, transaction komutları ve SET/DO/CALL… (arama yolu, katalog) önbelleği bayatlatabilir.
                slot.invalidate_completions();
            }
            let event = if ok {
                Some(TxEvent::Ok(p.tx))
            } else if failed {
                Some(TxEvent::Error(p.tx))
            } else {
                None // alıcı gitti; ifadenin sonucu belirsiz, yoklama karar verir
            };
            if let Some(ev) = event {
                let before = slot.state();
                slot.set_state(step(before, ev).unwrap_or(before));
            }
            if !ok || p.plan.kind == StmtKind::Tx {
                resync(slot, exec).await;
            }
            if gone {
                return Ok(());
            }
            if !ok {
                stopped = true;
                break;
            }
        }
        sink.send(QueryEvent::ScriptDone { stmts_run, stopped_on_error: stopped });
        Ok(())
    }

    /// Uykudaki ya da bağlantısı kopmuş oturumu yeniden bağlar. Kopan bağlantıda açık transaction varsa
    /// bir kez hata döner (transaction kayboldu), sonraki çağrı yeniden bağlanır.
    async fn wake(&self, slot: &SessionSlot, inner: &mut Inner) -> Result<()> {
        if slot.cancel.closed() {
            return Err(session_gone());
        }
        if inner.exec.as_ref().is_some_and(Executor::is_closed) {
            inner.exec = None;
            let lost = slot.state() != TxState::Idle;
            slot.set_state(TxState::Idle);
            if lost {
                tracing::warn!(server = %slot.server, "sorgu oturumunun bağlantısı koptu, açık transaction kayboldu");
                return Err(Error::Query {
                    sqlstate: None,
                    message: "Sorgu oturumunun bağlantısı koptu; açık transaction geri alındı.".into(),
                    detail: None,
                    hint: Some("Sonraki çalıştırma yeniden bağlanır.".into()),
                });
            }
        }
        if inner.exec.is_none() {
            let conn = self.mgr.open_dedicated(slot.server, slot.target, slot.profile(&inner.opts)).await?;
            inner.exec = Some(Executor::new(conn));
            inner.applied_timeout = Some(inner.opts.statement_timeout_ms);
            slot.set_state(TxState::Idle);
            tracing::debug!(server = %slot.server, "sorgu oturumu yeniden bağlandı");
        }
        Ok(())
    }

    /// İşlem sonu: transaction dışındaysa bekleyen `statement_timeout`'u uygular, boşta imleç zamanlayıcısını
    /// kurar, son kullanım zamanını günceller.
    async fn finish(&self, slot: &Arc<SessionSlot>, inner: &mut Inner) {
        if slot.cancel.closed() {
            // Kapatma kilidi alamadan döndü: bağlantı burada bırakılır (sunucu açık transaction'ı geri alır).
            disarm(inner);
            inner.exec = None;
            return;
        }
        let want = inner.opts.statement_timeout_ms;
        if slot.state() == TxState::Idle
            && inner.applied_timeout != Some(want)
            && let Some(exec) = inner.exec.as_mut()
            && !exec.cursor_open()
        {
            match exec.exec_internal(&format!("SET statement_timeout = {want}")).await {
                Ok(_) => inner.applied_timeout = Some(want),
                Err(e) => tracing::warn!(code = %ErrorPayload::from(&e).code, "statement_timeout uygulanamadı"),
            }
        }
        arm_cursor_timer(slot, inner, self.cursor_idle);
        inner.last_used = Instant::now();
    }

    /// Açık imleçten sonraki parça. İmleç yoksa (kapandı, oturum uyudu) `Error::Invalid`.
    pub async fn more(&self, session: SessionId, stmt_index: u32, sink: &mut dyn EventSink) -> Result<()> {
        let slot = self.slot(session)?;
        let mut inner = lock_open(&slot)?;
        disarm(&mut inner);
        let result = match inner.exec.as_mut() {
            None => Err(Error::Invalid("açık imleç yok".into())),
            Some(exec) => {
                slot.cancel.begin();
                let handle = exec.cancel_handle();
                let mut guarded = GuardSink::new(&mut *sink, &slot.cancel);
                let result = cancellable(&slot.cancel, &handle, exec.more(stmt_index, &mut guarded)).await;
                let requested = slot.cancel.requested();
                slot.cancel.end();
                if matches!(result, Err(Error::Query { .. })) {
                    resync(&slot, exec).await;
                }
                let is_cancel = result.as_ref().err().is_some_and(is_cancel_error);
                result.map_err(|e| match e {
                    Error::Query { message, .. } if requested && is_cancel => Error::Query {
                        sqlstate: None,
                        message: CANCELLED_MESSAGE.into(),
                        detail: Some(message),
                        hint: None,
                    },
                    e => e,
                })
            }
        };
        self.finish(&slot, &mut inner).await;
        result
    }

    /// Çalışan ifadeyi iptal eder (oturum kilidini beklemez). Boştayken etkisizdir.
    pub fn cancel(&self, session: SessionId) {
        if let Ok(slot) = self.slot(session)
            && slot.cancel.request()
        {
            tracing::debug!(session = %session, "iptal istendi");
        }
    }

    /// COMMIT / ROLLBACK. `Failed` iken COMMIT reddedilir.
    pub async fn tx(&self, session: SessionId, op: TxAction) -> Result<TxState> {
        let slot = self.slot(session)?;
        let mut inner = lock_open(&slot)?;
        disarm(&mut inner);
        let result = Self::tx_locked(&slot, &mut inner, op).await;
        slot.invalidate_completions();
        self.finish(&slot, &mut inner).await;
        result
    }

    async fn tx_locked(slot: &SessionSlot, inner: &mut Inner, op: TxAction) -> Result<TxState> {
        let tx = match op {
            TxAction::Commit => TxOp::Commit,
            TxAction::Rollback => TxOp::Rollback,
        };
        step(slot.state(), TxEvent::Ok(Some(tx)))?;
        // Uykudaki oturumda transaction yoktur.
        let Some(exec) = inner.exec.as_mut() else { return Ok(slot.state()) };
        close_cursor_quietly(exec).await;
        let sql = match op {
            TxAction::Commit => "COMMIT",
            TxAction::Rollback => "ROLLBACK",
        };
        let result = exec.exec_internal(sql).await;
        // Başarılı ya da başarısız, COMMIT/ROLLBACK transaction'ı bitirir.
        slot.set_state(TxState::Idle);
        exec.set_in_tx(false);
        resync(slot, exec).await;
        result.map(|_| slot.state())
    }

    /// Makinenin bildiği transaction durumu (kilitsiz).
    pub fn tx_state(&self, session: SessionId) -> Result<TxState> {
        Ok(self.slot(session)?.state())
    }

    /// Mod ve `statement_timeout`. Mod değişikliği açık transaction varken reddedilir. Zaman aşımı hemen
    /// uygulanır; transaction içindeyse (ROLLBACK onu geri alabileceği için) transaction bitince yeniden uygulanır.
    pub async fn set_options(&self, session: SessionId, opts: SessionOptions) -> Result<()> {
        if opts.statement_timeout_ms > MAX_STATEMENT_TIMEOUT_MS {
            return Err(Error::Invalid("statement_timeout en fazla 2147483647 ms olabilir.".into()));
        }
        let slot = self.slot(session)?;
        let mut inner = lock_open(&slot)?;
        disarm(&mut inner);
        let result = Self::set_options_locked(&slot, &mut inner, opts).await;
        self.finish(&slot, &mut inner).await;
        result
    }

    async fn set_options_locked(slot: &SessionSlot, inner: &mut Inner, opts: SessionOptions) -> Result<()> {
        if let Some(exec) = inner.exec.as_mut() {
            close_cursor_quietly(exec).await;
        }
        let state = slot.state();
        if opts.mode != inner.opts.mode && state != TxState::Idle {
            return Err(Error::Invalid(MODE_IN_TX_MESSAGE.into()));
        }
        inner.opts = opts;
        if state == TxState::Failed {
            return Ok(()); // ROLLBACK sonrası `finish` uygular
        }
        if let Some(exec) = inner.exec.as_mut() {
            exec.exec_internal(&format!("SET statement_timeout = {}", opts.statement_timeout_ms)).await?;
            if state == TxState::Idle {
                inner.applied_timeout = Some(opts.statement_timeout_ms);
            }
        }
        Ok(())
    }

    /// Oturumu kapatır. Açık transaction `policy`'ye göre onaylanır, geri alınır ya da kapanış reddedilir.
    ///
    /// Oturum meşgulse (bir ifade çalışıyorsa): `Rollback` çalışan ifadeyi iptal eder, en çok 5 sn bekler,
    /// transaction'ı geri alır ve oturumu kapatır (sekme kapanınca bağlantı açık transaction'la kalmasın);
    /// `Commit` reddedilir (`BUSY`: yarıda kalan bir çalışma onaylanamaz); `Refuse` açık transaction varsa
    /// `Açık transaction var.`, yoksa `BUSY` döner.
    pub async fn close(&self, session: SessionId, policy: ClosePolicy) -> Result<()> {
        let slot = self.slot(session)?;
        let mut inner = match slot.inner.try_lock() {
            Ok(_) if slot.cancel.closed() => return Err(session_gone()),
            Ok(inner) => inner,
            Err(_) => {
                return match policy {
                    ClosePolicy::Rollback => {
                        self.map().remove(&session);
                        force_close(&slot).await;
                        tracing::debug!(session = %session, "meşgul sorgu oturumu zorla kapatıldı");
                        Ok(())
                    }
                    ClosePolicy::Refuse if slot.state() != TxState::Idle => Err(Error::Invalid(OPEN_TX_MESSAGE.into())),
                    ClosePolicy::Refuse | ClosePolicy::Commit => Err(busy()),
                };
            }
        };
        disarm(&mut inner);
        if let Some(exec) = inner.exec.as_mut() {
            close_cursor_quietly(exec).await;
            let state = slot.state();
            if state != TxState::Idle {
                let refused = match policy {
                    ClosePolicy::Refuse => Some(Error::Invalid(OPEN_TX_MESSAGE.into())),
                    ClosePolicy::Commit if state == TxState::Failed => Some(Error::Invalid(TX_FAILED_MESSAGE.into())),
                    ClosePolicy::Commit => match exec.exec_internal("COMMIT").await {
                        Ok(_) => None,
                        Err(e) => {
                            slot.set_state(TxState::Idle);
                            resync(&slot, exec).await;
                            Some(e)
                        }
                    },
                    ClosePolicy::Rollback => {
                        if let Err(e) = exec.exec_internal("ROLLBACK").await {
                            tracing::debug!(code = %ErrorPayload::from(&e).code, "kapanışta ROLLBACK başarısız");
                        }
                        None
                    }
                };
                if let Some(e) = refused {
                    self.finish(&slot, &mut inner).await;
                    return Err(e);
                }
            }
        }
        inner.exec = None;
        slot.cancel.close();
        slot.set_state(TxState::Idle);
        self.map().remove(&session);
        tracing::debug!(session = %session, "sorgu oturumu kapatıldı");
        Ok(())
    }

    /// Sunucunun tüm oturumlarını kapatır: çalışan ifadeler iptal edilir, açık transaction'lar geri alınır
    /// (sunucu silinirken, bağlantı kesilirken ya da hedef/kilit değişince).
    pub async fn close_sessions_of(&self, server: Uuid) {
        let slots: Vec<Arc<SessionSlot>> = {
            let mut map = self.map();
            let ids: Vec<SessionId> = map.iter().filter(|(_, s)| s.server == server).map(|(id, _)| *id).collect();
            ids.into_iter().filter_map(|id| map.remove(&id)).collect()
        };
        for slot in slots {
            force_close(&slot).await;
        }
    }

    /// Uygulama kapanışı: tüm oturumları eşzamanlı kapatır (çalışanlar iptal edilir, açık transaction'lar geri
    /// alınır). Oturum başına en çok ~5 sn beklenir; oturumlar paralel olduğundan toplam da o kadardır.
    pub async fn close_all(&self) {
        let slots: Vec<Arc<SessionSlot>> = self.map().drain().map(|(_, s)| s).collect();
        let mut tasks = tokio::task::JoinSet::new();
        for slot in slots {
            tasks.spawn(async move { force_close(&slot).await });
        }
        while tasks.join_next().await.is_some() {}
    }

    /// `max_idle` boyunca kullanılmayan, transaction dışındaki oturumların bağlantısını kapatır ("uyku");
    /// sonraki işlem sessizce yeniden bağlanır. Açık transaction'lı ya da meşgul oturumlara dokunmaz.
    pub fn reap_idle(&self, max_idle: Duration) {
        let slots: Vec<Arc<SessionSlot>> = self.map().values().cloned().collect();
        for slot in slots {
            let Ok(mut inner) = slot.inner.try_lock() else { continue };
            if inner.exec.is_some() && slot.state() == TxState::Idle && inner.last_used.elapsed() >= max_idle {
                disarm(&mut inner);
                inner.exec = None;
                tracing::debug!(server = %slot.server, "boşta kalan sorgu oturumu uyutuldu");
            }
        }
    }

    /// Tamamlama önbelleğini boşaltır (DDL başarıyla bitince oturum katmanı kendisi çağırır).
    pub fn invalidate_completion_cache(&self, session: SessionId) {
        if let Ok(slot) = self.slot(session) {
            slot.invalidate_completions();
        }
    }

    /// Katalog tamamlama. Oturumun kendi bağlantısı **yalnızca boştayken** kullanılır; bir ifade çalışıyorsa ya da
    /// transaction `Failed` ise beklemeden boş liste döner. Sonuçlar oturum başına 60 sn önbelleğe alınır; hata
    /// olursa da boş liste döner (tamamlama kullanıcı akışını bozmamalı).
    pub async fn complete(&self, session: SessionId, req: CompletionRequest) -> Result<Vec<CompletionItem>> {
        let slot = self.slot(session)?;
        if slot.state() == TxState::Failed {
            return Ok(Vec::new());
        }
        if let Some((at, items)) = slot.completions().get(&req)
            && at.elapsed() < COMPLETION_TTL
        {
            return Ok(items.clone());
        }
        let Ok(mut inner) = slot.inner.try_lock() else { return Ok(Vec::new()) };
        if slot.cancel.closed() {
            return Err(session_gone());
        }
        if slot.state() == TxState::Failed || inner.exec.as_ref().is_some_and(Executor::is_closed) {
            // Kopan bağlantının bildirimi (kaybolan transaction) sonraki `run`a kalır.
            return Ok(Vec::new());
        }
        // Uykudaki oturum uyandırılmaz (bağlantı kurmak kilidi uzun tutar ve boşta uykuyu boşa çıkarır).
        let Some(exec) = inner.exec.as_mut() else { return Ok(Vec::new()) };
        let handle = exec.cancel_handle();
        // Transaction içindeyse (kullanıcının ya da ızgara imlecinin) arama bir SAVEPOINT'e sarılır: hata ya da
        // zaman aşımı transaction'ı bozmaz.
        let guarded = slot.state() != TxState::Idle || exec.cursor_open();
        if guarded && exec.exec_internal("SAVEPOINT pgt_complete").await.is_err() {
            return Ok(Vec::new());
        }
        let found = match tokio::time::timeout(COMPLETION_TIMEOUT, lookup(exec, &req)).await {
            Ok(r) => r,
            Err(_) => {
                cancel_quietly(&handle).await;
                Err(Error::Invalid("zaman aşımı".into()))
            }
        };
        if guarded {
            let end = if found.is_ok() {
                "RELEASE SAVEPOINT pgt_complete"
            } else {
                "ROLLBACK TO SAVEPOINT pgt_complete; RELEASE SAVEPOINT pgt_complete"
            };
            let _ = exec.exec_internal(end).await;
        }
        let items = match found {
            Ok(items) => items,
            Err(e) => {
                tracing::debug!(code = %ErrorPayload::from(&e).code, "tamamlama sorgusu başarısız");
                return Ok(Vec::new());
            }
        };
        inner.last_used = Instant::now();
        let mut cache = slot.completions();
        if cache.len() >= COMPLETION_CACHE_MAX {
            cache.clear();
        }
        cache.insert(req, (Instant::now(), items.clone()));
        Ok(items)
    }

    /// Satır döndüren tek bir `SELECT`/`VALUES`/`TABLE`/`WITH` sorgusunun sonucunu CSV'ye akıtır
    /// (bkz. `crate::export`). `progress(toplam_satır)` her parçadan sonra çağrılır; `false` dönerse dışa aktarma
    /// iptal edilir (`.tmp` silinir, hedefe dokunulmaz). Oturum dışa aktarma boyunca meşguldür; `cancel()` durdurur.
    /// Açık transaction'a (manuel mod) dokunulmaz; transaction yoksa geçici bir `BEGIN`/`COMMIT` kullanılır.
    pub async fn export_csv(
        &self,
        session: SessionId,
        sql: &str,
        path: &StdPath,
        opts: CsvOptions,
        progress: &mut (dyn FnMut(u64) -> bool + Send),
    ) -> Result<ExportSummary> {
        let slot = self.slot(session)?;
        let stmts = split(sql);
        let [stmt] = stmts.as_slice() else {
            return Err(Error::Invalid("Dışa aktarma tek bir SELECT ifadesi ister.".into()));
        };
        let text = &sql[stmt.start..stmt.end];
        // Kilit kapısı `run` ile aynı: kilitli oturumda yazma/şema ifadeleri ve salt okunur ayarını değiştirebilen
        // ifadeler (`select set_config(...)` bir SELECT'tir!) reddedilir. Onay kapısı uygulanmaz: dışa aktarma
        // yalnızca satır döndüren SELECT çalıştırır (tehlike/üretim yazması burada anlamsız).
        check_gates(&plan_script(sql, Scope::All, slot.guard)?, true)?;
        if classify(text).kind != StmtKind::Select || !cursor_capable(text, StmtKind::Select) {
            return Err(Error::Invalid("Yalnızca satır döndüren SELECT/VALUES/TABLE sorguları dışa aktarılabilir.".into()));
        }
        check_target(path)?;
        let mut inner = lock_open(&slot)?;
        step(slot.state(), TxEvent::Ok(None))?;
        disarm(&mut inner);
        let result = match self.wake(&slot, &mut inner).await {
            Err(e) => Err(e),
            Ok(()) => {
                let Some(exec) = inner.exec.as_mut() else { return Err(Error::NotConnected) };
                slot.cancel.begin();
                let handle = exec.cancel_handle();
                let result = cancellable(&slot.cancel, &handle, export_run(&slot, exec, text, path, opts, progress)).await;
                let requested = slot.cancel.requested();
                slot.cancel.end();
                if matches!(result, Err(Error::Query { .. })) && !exec.cursor_open() {
                    resync(&slot, exec).await;
                }
                let is_cancel = result.as_ref().err().is_some_and(is_cancel_error);
                result.map_err(|e| match e {
                    Error::Query { message, .. } if requested && is_cancel => Error::Query {
                        sqlstate: None,
                        message: CANCELLED_MESSAGE.into(),
                        detail: Some(message),
                        hint: None,
                    },
                    e => e,
                })
            }
        };
        self.finish(&slot, &mut inner).await;
        if let Ok(s) = &result {
            tracing::debug!(rows = s.rows, bytes = s.bytes, elapsed_ms = s.elapsed_ms, "CSV dışa aktarma bitti");
        }
        result
    }

    /// Test yardımcısı: oturum çalıştırıcısında bellekte aynı anda tutulmuş en çok satır.
    #[doc(hidden)]
    pub fn peak_buffered_rows(&self, session: SessionId) -> Result<usize> {
        let slot = self.slot(session)?;
        Ok(slot.inner.try_lock().ok().and_then(|i| i.exec.as_ref().map(Executor::peak_buffered_rows)).unwrap_or(0))
    }

    /// Test yardımcısı: oturum uykuda mı (bağlantısı kapatılmış).
    #[doc(hidden)]
    pub fn is_asleep(&self, session: SessionId) -> Result<bool> {
        let slot = self.slot(session)?;
        Ok(slot.inner.try_lock().map(|i| i.exec.is_none()).unwrap_or(false))
    }

    /// Test yardımcısı: bağlantısız (uykuda) bir oturum ekler.
    #[cfg(test)]
    fn insert_sleeping(&self, guard: Guard, state: TxState) -> SessionId {
        let id = Uuid::new_v4();
        let slot = Arc::new(SessionSlot {
            server: Uuid::new_v4(),
            target: Target::Maintenance,
            guard,
            state: StdMutex::new(state),
            inner: Mutex::new(Inner {
                exec: None,
                opts: SessionOptions::default(),
                applied_timeout: None,
                last_used: Instant::now(),
                cursor_timer: None,
                cursor_gen: 0,
            }),
            cancel: CancelToken::default(),
            completions: StdMutex::default(),
        });
        self.map().insert(id, slot);
        id
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SELECT: Option<TxOp> = None;

    #[test]
    fn transaction_state_machine_table() {
        use TxState::*;
        let ok = |op| TxEvent::Ok(Some(op));
        let cases: [(TxState, TxEvent, Option<TxState>); 12] = [
            (Idle, ok(TxOp::Begin), Some(InTransaction)),
            (InTransaction, ok(TxOp::Commit), Some(Idle)),
            (InTransaction, ok(TxOp::Rollback), Some(Idle)),
            (InTransaction, TxEvent::Error(SELECT), Some(Failed)),
            (Idle, TxEvent::Error(SELECT), Some(Idle)),
            (Failed, ok(TxOp::Rollback), Some(Idle)),
            (Failed, ok(TxOp::RollbackTo), Some(InTransaction)),
            (Failed, TxEvent::Ok(SELECT), None),
            (Failed, ok(TxOp::Commit), None),
            (InTransaction, ok(TxOp::Savepoint), Some(InTransaction)),
            (InTransaction, TxEvent::Error(Some(TxOp::Commit)), Some(Idle)),
            (Idle, TxEvent::Ok(SELECT), Some(Idle)),
        ];
        for (state, ev, want) in cases {
            match (step(state, ev), want) {
                (Ok(got), Some(want)) => assert_eq!(got, want, "{state:?} + {ev:?}"),
                (Err(Error::Invalid(m)), None) => assert_eq!(m, TX_FAILED_MESSAGE),
                (got, want) => panic!("{state:?} + {ev:?}: {got:?}, beklenen {want:?}"),
            }
        }
    }

    fn plans(sql: &str, guard: Guard) -> Vec<StmtPlan> {
        plan_script(sql, Scope::All, guard).unwrap().into_iter().map(|p| p.plan).collect()
    }

    fn gate(sql: &str, guard: Guard, confirmed: bool) -> Result<()> {
        check_gates(&plan_script(sql, Scope::All, guard).unwrap(), confirmed)
    }

    fn invalid(r: Result<()>) -> String {
        match r {
            Err(Error::Invalid(m)) => m,
            other => panic!("Invalid bekleniyordu: {other:?}"),
        }
    }

    const LOCKED: Guard = Guard { locked: true, production: false };
    const PROD: Guard = Guard { locked: false, production: true };
    const PLAIN: Guard = Guard { locked: false, production: false };

    #[test]
    fn locked_server_rejects_writes_and_allows_selects() {
        let p = plans("insert into t values (1)", LOCKED);
        assert!(p[0].locked_reject);
        assert_eq!(p[0].locked_reason, Some(LockedReason::Write));
        assert_eq!(plans("create table t(i int)", LOCKED)[0].locked_reason, Some(LockedReason::Ddl));
        assert!(!plans("select 1", LOCKED)[0].locked_reject);
        assert!(gate("select 1", LOCKED, false).is_ok());
        assert!(!plans("insert into t values (1)", PLAIN)[0].locked_reject);
        // Betikte tek bir yazma ifadesi tüm betiği engeller.
        assert_eq!(invalid(gate("select 1; insert into t values (1); select 2", LOCKED, true)), LOCKED_WRITE_MESSAGE);
    }

    #[test]
    fn locked_server_rejects_read_only_escapes() {
        for sql in [
            "set default_transaction_read_only = off",
            "begin read write",
            "select set_config('default_transaction_read_only','off',false)",
            "reset all",
        ] {
            let p = plans(sql, LOCKED);
            assert_eq!(p[0].locked_reason, Some(LockedReason::ReadOnlySetting), "{sql}");
            assert_eq!(invalid(gate(sql, LOCKED, true)), LOCKED_SETTING_MESSAGE, "{sql}");
            assert_eq!(invalid(gate(&format!("select 1; {sql}"), LOCKED, true)), LOCKED_SETTING_MESSAGE, "{sql}");
            assert!(!plans(sql, PLAIN)[0].locked_reject, "{sql}");
            assert!(gate(sql, PLAIN, false).is_ok(), "{sql}");
        }
        assert!(gate("begin read only", LOCKED, false).is_ok());
    }

    #[test]
    fn production_writes_and_dangerous_statements_need_confirmation() {
        let p = plans("update t set a = 1 where id = 1", PROD);
        assert!(p[0].production_write && p[0].danger.is_none());
        assert_eq!(invalid(gate("update t set a = 1 where id = 1", PROD, false)), CONFIRMATION_REQUIRED);
        assert!(gate("update t set a = 1 where id = 1", PROD, true).is_ok());
        assert!(gate("select 1", PROD, false).is_ok());

        let p = plans("update t set a = 1", PLAIN);
        assert_eq!((p[0].danger, p[0].production_write), (Some(Danger::UpdateWithoutWhere), false));
        assert_eq!(invalid(gate("select 1; update t set a = 1", PLAIN, false)), CONFIRMATION_REQUIRED);
        assert!(gate("update t set a = 1", PLAIN, true).is_ok());
        assert!(gate("update t set a = 1 where id = 1", PLAIN, false).is_ok());
        // Kilit onaydan önce gelir: onay kilidi aşmaz.
        assert_eq!(invalid(gate("delete from t", LOCKED, true)), LOCKED_WRITE_MESSAGE);
    }

    #[test]
    fn sensitive_statements_are_flagged_in_plan() {
        let p = plans("select 1; alter role app password 'x'; create user u with password 'y'", PLAIN);
        assert_eq!(p.iter().map(|p| p.sensitive).collect::<Vec<_>>(), [false, true, true]);
    }

    #[test]
    fn scopes_pick_statements_with_char_offsets() {
        let sql = "select 'ğü'; select 2;\n  select 3";
        let all = plan_script(sql, Scope::All, PLAIN).unwrap();
        assert_eq!(all.iter().map(|p| (p.plan.index, p.plan.char_offset)).collect::<Vec<_>>(), [(0, 0), (1, 13), (2, 25)]);
        assert_eq!(&sql[all[2].range.clone()], "select 3");

        // Seçim kendi başına bir betiktir: indeksler seçim içi, ofsetler editör metnine göre.
        let sel = plan_script(sql, Scope::Selection { start: 13, end: 34 }, PLAIN).unwrap();
        assert_eq!(sel.iter().map(|p| (p.plan.index, p.plan.char_offset)).collect::<Vec<_>>(), [(0, 13), (1, 25)]);
        assert_eq!(&sql[sel[0].range.clone()], "select 2");

        let at = plan_script(sql, Scope::AtCursor { offset: 16 }, PLAIN).unwrap();
        assert_eq!(at.len(), 1);
        assert_eq!((at[0].plan.index, at[0].plan.char_offset, &sql[at[0].range.clone()]), (0, 13, "select 2"));

        assert!(plan_script(sql, Scope::Selection { start: 5, end: 2 }, PLAIN).is_err());
        assert!(plan_script("", Scope::AtCursor { offset: 0 }, PLAIN).unwrap().is_empty());
        assert!(plan_script(sql, Scope::Selection { start: 3, end: 3 }, PLAIN).unwrap().is_empty());
    }

    #[test]
    fn plan_serializes_camel_case() {
        let p = plans("insert into t values (1)", LOCKED)[0];
        let v = serde_json::to_value(p).unwrap();
        assert_eq!(v["lockedReject"], true);
        assert_eq!(v["lockedReason"], "write");
        assert_eq!(v["productionWrite"], false);
        assert_eq!(serde_json::to_value(TxState::InTransaction).unwrap(), "inTransaction");
        let scope: Scope = serde_json::from_value(serde_json::json!({"type": "atCursor", "offset": 3})).unwrap();
        assert_eq!(scope, Scope::AtCursor { offset: 3 });
        let opts: SessionOptions =
            serde_json::from_value(serde_json::json!({"mode": "manual", "statementTimeoutMs": 5})).unwrap();
        assert_eq!(opts, SessionOptions { mode: TxMode::Manual, statement_timeout_ms: 5 });
    }

    fn manager() -> QueryManager {
        QueryManager::new(Arc::new(ConnectionManager::new()), Arc::new(RwLock::new(Registry::unavailable("servers.json"))))
    }

    struct Nop;
    impl EventSink for Nop {
        fn send(&mut self, _: QueryEvent) -> bool {
            true
        }
    }

    #[tokio::test]
    async fn open_requires_a_connected_server() {
        let qm = manager();
        let err = qm.open(Uuid::new_v4(), Target::Maintenance, SessionOptions::default()).await.unwrap_err();
        assert!(matches!(err, Error::NotConnected), "{err:?}");
    }

    /// Kapılar ağa gitmeden çalışır: geçen betik yeniden bağlanmaya çalışır (`NotConnected`), geçmeyen
    /// kapı hatasıyla döner.
    #[tokio::test]
    async fn run_gates_apply_before_any_connection() {
        let qm = manager();
        let plain = qm.insert_sleeping(PLAIN, TxState::Idle);
        let err = qm.run(plain, "update t set a = 1", Scope::All, 100, false, &mut Nop).await.unwrap_err();
        assert!(matches!(&err, Error::Invalid(m) if m == CONFIRMATION_REQUIRED), "{err:?}");
        let err = qm.run(plain, "update t set a = 1", Scope::All, 100, true, &mut Nop).await.unwrap_err();
        assert!(matches!(err, Error::NotConnected), "onay sonrası çalıştırılmaya çalışılır: {err:?}");

        let prod = qm.insert_sleeping(PROD, TxState::Idle);
        let err = qm.run(prod, "insert into t values (1)", Scope::All, 100, false, &mut Nop).await.unwrap_err();
        assert!(matches!(&err, Error::Invalid(m) if m == CONFIRMATION_REQUIRED), "{err:?}");

        let locked = qm.insert_sleeping(LOCKED, TxState::Idle);
        let err = qm.run(locked, "select 1; insert into t values (1)", Scope::All, 100, true, &mut Nop).await.unwrap_err();
        assert!(matches!(&err, Error::Invalid(m) if m == LOCKED_WRITE_MESSAGE), "{err:?}");
        let analyzed = qm.analyze(locked, "select 1; insert into t values (1)", Scope::All).unwrap();
        assert_eq!(analyzed.iter().map(|p| p.locked_reject).collect::<Vec<_>>(), [false, true]);

        let failed = qm.insert_sleeping(PLAIN, TxState::Failed);
        let err = qm.run(failed, "select 1", Scope::All, 100, false, &mut Nop).await.unwrap_err();
        assert!(matches!(&err, Error::Invalid(m) if m == TX_FAILED_MESSAGE), "{err:?}");
        let err = qm.tx(failed, TxAction::Commit).await.unwrap_err();
        assert!(matches!(&err, Error::Invalid(m) if m == TX_FAILED_MESSAGE), "{err:?}");
        let err = qm.run(failed, "rollback", Scope::All, 100, false, &mut Nop).await.unwrap_err();
        assert!(matches!(err, Error::NotConnected), "ROLLBACK kapıdan geçer: {err:?}");

        let err = qm.run(plain, "select 1", Scope::All, 0, false, &mut Nop).await.unwrap_err();
        assert!(matches!(err, Error::Invalid(_)), "{err:?}");
        assert!(matches!(qm.tx_state(Uuid::new_v4()), Err(Error::NotFound(_))));
    }

    #[tokio::test]
    async fn busy_session_rejects_a_second_operation_and_idle_cancel_is_ignored() {
        let qm = manager();
        let id = qm.insert_sleeping(PLAIN, TxState::Idle);
        let slot = qm.slot(id).unwrap();
        assert!(!slot.cancel.request(), "boştayken iptal yok sayılır");
        assert!(!slot.cancel.requested());
        let held = slot.inner.lock().await;
        let err = qm.run(id, "select 1", Scope::All, 100, false, &mut Nop).await.unwrap_err();
        assert!(matches!(&err, Error::Invalid(m) if m == BUSY_MESSAGE), "{err:?}");
        assert!(matches!(qm.tx(id, TxAction::Rollback).await, Err(Error::Invalid(m)) if m == BUSY_MESSAGE));
        assert_eq!(qm.tx_state(id).unwrap(), TxState::Idle, "durum kilitsiz okunur");
        drop(held);
        let err = qm.set_options(id, SessionOptions { mode: TxMode::Manual, statement_timeout_ms: u32::MAX }).await;
        assert!(matches!(err, Err(Error::Invalid(_))));
        qm.close(id, ClosePolicy::Refuse).await.unwrap();
        assert!(qm.tx_state(id).is_err());
    }

    /// Dışa aktarma kilit kapısını `run` ile paylaşır; red, oturum uyandırılmadan (ağa gitmeden) gelir.
    #[tokio::test]
    async fn export_applies_the_lock_gate_before_any_connection() {
        let qm = manager();
        let locked = qm.insert_sleeping(LOCKED, TxState::Idle);
        let path = std::env::temp_dir().join("pgt-export-gate-unit.csv");
        let export = |id, sql: &'static str| {
            let qm = &qm;
            let path = path.clone();
            async move { qm.export_csv(id, sql, &path, CsvOptions::default(), &mut |_| true).await }
        };
        for (sql, msg) in [
            ("select set_config('default_transaction_read_only','off',false)", LOCKED_SETTING_MESSAGE),
            ("select set_config('default_transaction_' || 'read_only','off',false)", LOCKED_SETTING_MESSAGE),
            ("insert into t values (1) returning *", LOCKED_WRITE_MESSAGE),
            ("with d as (delete from t returning *) select * from d", LOCKED_WRITE_MESSAGE),
        ] {
            let err = export(locked, sql).await.unwrap_err();
            assert!(matches!(&err, Error::Invalid(m) if m == msg), "{sql}: {err:?}");
        }
        // Kapıdan geçen sorgu uyandırmaya çalışır (bağlı sunucu yok).
        let err = export(locked, "select 1").await.unwrap_err();
        assert!(matches!(err, Error::NotConnected), "{err:?}");
        let plain = qm.insert_sleeping(PLAIN, TxState::Idle);
        let err = export(plain, "select set_config('a','b',false)").await.unwrap_err();
        assert!(matches!(err, Error::NotConnected), "kilitsiz oturumda kapı yok: {err:?}");
        assert!(!path.exists());
    }

    /// Her iptal isteği bir iptal gönderir (kaybolan ilk iptal ikinci tıklamayla yeniden denenir); çalışma
    /// yokken gelen istek ve `requested` olmadan kalan eski izin hiçbir şey göndermez.
    #[tokio::test]
    async fn every_cancel_request_sends_one_cancel() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        let token = CancelToken::default();
        let sent = AtomicUsize::new(0);
        assert!(!token.request(), "boştayken yok sayılır");
        token.notify.notify_one(); // eski izin, `requested` yok
        let (tx, rx) = tokio::sync::oneshot::channel::<()>();
        token.begin();
        let run = cancellable_with(
            &token,
            || {
                sent.fetch_add(1, Ordering::SeqCst);
                async {}
            },
            rx,
        );
        let driver = async {
            for _ in 0..10 {
                tokio::task::yield_now().await;
            }
            assert_eq!(sent.load(Ordering::SeqCst), 0, "eski izin iptal göndermez");
            assert!(token.request());
            for _ in 0..10 {
                tokio::task::yield_now().await;
            }
            assert_eq!(sent.load(Ordering::SeqCst), 1);
            // İlk iptal kayboldu (ifade sürüyor): ikinci istek yeniden gönderir.
            assert!(token.request());
            for _ in 0..10 {
                tokio::task::yield_now().await;
            }
            assert_eq!(sent.load(Ordering::SeqCst), 2);
            tx.send(()).unwrap();
        };
        let (out, ()) = tokio::join!(run, driver);
        out.unwrap();
        token.end();
        assert!(!token.request());
        assert_eq!(sent.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn auto_begin_is_skipped_only_for_transaction_boundaries() {
        for (sql, skip) in [
            ("begin", true), ("start transaction", true), ("commit", true), ("rollback", true), ("end", true),
            ("commit prepared 'x'", true), ("rollback prepared 'x'", true), ("prepare transaction 'x'", true),
            ("savepoint a", false), ("release a", false), ("rollback to a", false),
            ("set transaction isolation level serializable", false), ("select 1", false), ("insert into t values (1)", false),
        ] {
            assert_eq!(skips_auto_begin(sql, tx_op(sql)), skip, "{sql}");
        }
    }

    #[test]
    fn locked_server_flags_any_set_config_call() {
        let p = plans("select set_config('default_transaction_' || 'read_only','off',false)", LOCKED);
        assert_eq!(p[0].locked_reason, Some(LockedReason::ReadOnlySetting));
        assert!(!plans("select current_setting('x')", LOCKED)[0].locked_reject);
    }

    #[test]
    fn closed_token_keeps_cancel_and_cancelled_event_is_visible() {
        let token = CancelToken::default();
        token.close();
        token.begin();
        assert!(token.requested(), "kapatma iptali begin() ile sıfırlanmaz");
        token.end();
        assert!(token.closed());
        let QueryEvent::Failed { stmt_index, error, char_offset, .. } = cancelled_event(2, 17) else { unreachable!() };
        assert_eq!((stmt_index, char_offset), (2, 17));
        assert_eq!((error.code.as_str(), error.message.as_str()), ("query", CANCELLED_MESSAGE));
    }

    #[tokio::test]
    async fn busy_close_policies() {
        let qm = Arc::new(manager());
        let id = qm.insert_sleeping(PLAIN, TxState::InTransaction);
        let slot = qm.slot(id).unwrap();
        let held = slot.inner.lock().await;
        assert!(matches!(qm.close(id, ClosePolicy::Commit).await, Err(Error::Invalid(m)) if m == BUSY_MESSAGE));
        assert!(matches!(qm.close(id, ClosePolicy::Refuse).await, Err(Error::Invalid(m)) if m == OPEN_TX_MESSAGE));
        slot.set_state(TxState::Idle);
        assert!(matches!(qm.close(id, ClosePolicy::Refuse).await, Err(Error::Invalid(m)) if m == BUSY_MESSAGE));
        slot.set_state(TxState::InTransaction);

        let closer = {
            let qm = qm.clone();
            tokio::spawn(async move { qm.close(id, ClosePolicy::Rollback).await })
        };
        while !slot.cancel.closed() {
            tokio::task::yield_now().await;
        }
        assert!(matches!(qm.tx_state(id), Err(Error::NotFound(_))), "oturum hemen haritadan çıkar");
        drop(held);
        closer.await.unwrap().unwrap();
        assert_eq!(slot.state(), TxState::Idle);
        // Kaldırılmış oturumun eski bir tutamağı kilidi alsa da işlem yapamaz (yeniden bağlanmaz).
        assert!(matches!(lock_open(&slot), Err(Error::NotFound(_))));
    }

    #[test]
    fn cancel_rewrites_only_timeout_failures_of_a_requested_cancel() {
        struct Keep(Vec<QueryEvent>);
        impl EventSink for Keep {
            fn send(&mut self, ev: QueryEvent) -> bool {
                self.0.push(ev);
                true
            }
        }
        let token = CancelToken::default();
        let failed = |code: &str| QueryEvent::Failed {
            stmt_index: 0,
            error: ErrorPayload::from(&Error::Query {
                sqlstate: Some(code.into()),
                message: "canceling statement due to user request".into(),
                detail: None,
                hint: None,
            }),
            position: None,
            char_offset: 0,
            notices: Vec::new(),
        };
        let mut keep = Keep(Vec::new());
        {
            let mut g = GuardSink::new(&mut keep, &token);
            g.send(failed("57014")); // iptal istenmedi: zaman aşımı iletisi kalır
            token.begin();
            token.request();
            g.send(failed("57014"));
            g.send(failed("22012"));
            assert!(g.failed);
        }
        let msgs: Vec<_> = keep
            .0
            .iter()
            .map(|e| match e {
                QueryEvent::Failed { error, .. } => (error.message.clone(), error.detail.clone()),
                _ => unreachable!(),
            })
            .collect();
        assert_eq!(msgs[0].0, timeout_text());
        assert_eq!(msgs[1].0, CANCELLED_MESSAGE);
        assert_eq!(msgs[1].1.as_deref(), Some("canceling statement due to user request"));
        assert_eq!(msgs[2].0, "canceling statement due to user request");
        assert!(is_cancel_error(&Error::Query { sqlstate: Some("57014".into()), message: String::new(), detail: None, hint: None }));
    }
}
