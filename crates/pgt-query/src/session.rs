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

use crate::check::{CheckDiag, CheckTarget};
use crate::complete::{CompletionItem, CompletionRequest, Lookup, lookup};
use crate::parse_guard::{Background, BackgroundGuard, Parsed, RESTORE_LOCK_TIMEOUT, bounded, guarded_parse, settled};
use crate::scoped::{Plan, Source};
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
/// Oturum durumu değiştirilmiş (SET, geçici tablo, advisory kilit…) bir oturumun bağlantısı koptu ve yeniden
/// kuruldu: bu durum kaybolmuş olabilir. Bir kez döner; yeni bağlantı hazırdır, tekrar çalıştırma çalışır.
pub const SESSION_RESET_MESSAGE: &str = "Bağlantı yeniden kuruldu; oturum durumu (SET ayarları, geçici tablolar, advisory kilitler) kaybolmuş olabilir. Sorguyu tekrar çalıştırın.";
/// Oturum bağlanırken sunucunun salt okunur kilidi ya da üretim etiketi değişti; bağlantı bırakıldı.
pub const SETTINGS_CHANGED_MESSAGE: &str =
    "Sunucunun salt okunur kilidi ya da üretim etiketi bağlanırken değişti; sekmeyi yeniden açın.";
/// Kullanıcının iptal ettiği ifadenin `Failed` iletisi (zaman aşımından ayırt edilir; sunucu iletisi `detail`'de).
pub const CANCELLED_MESSAGE: &str = "Sorgu kullanıcı tarafından iptal edildi.";
/// `more()` almayan açık imlecin kapatılma süresi (varsayılan).
pub const DEFAULT_CURSOR_IDLE: Duration = Duration::from_secs(5 * 60);

/// Tamamlama önbelleğinin tazelik süresi, bir aramanın en uzun süresi ve önbellek girdisi sınırı.
pub const COMPLETION_TTL: Duration = Duration::from_secs(60);
const COMPLETION_TIMEOUT: Duration = Duration::from_secs(5);
const COMPLETION_CACHE_MAX: usize = 256;
/// Kullanıcı işleminin, kilidi tutan arka plan işleminin (tamamlama) bırakmasını en çok bekleyeceği süre.
const BACKGROUND_YIELD_WAIT: Duration = Duration::from_secs(3);
/// Kilit meşgulken (arka plan bayrakları henüz kurulmamış olabilir) kullanıcı işleminin deneme sayısı (~2 ms arayla).
const BUSY_RETRIES: u32 = 4;

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
    /// İfade metninin karakter (Unicode scalar) uzunluğu: metin `[char_offset, char_offset + char_len)`.
    pub char_len: u32,
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
    /// `SELECT` ile oturum durumu oluşturan çağrı (oturum advisory kilidi, `set_config`): oturum durumlu sayılır.
    pub(crate) creates_session_state: bool,
}

/// İfade sunucu tarafı kod çalıştıran `DO`, `CALL` ya da hazırlanmış ifadeyi çalıştıran `EXECUTE` mı
/// (`PREPARE x AS DELETE …; EXECUTE x` gövdesi görünmez).
fn runs_server_code(sql: &str) -> bool {
    crate::lex::lex(sql)
        .into_iter()
        .find(|t| t.kind == crate::lex::Kind::Word)
        .is_some_and(|t| t.is_word(sql, "do") || t.is_word(sql, "call") || t.is_word(sql, "execute"))
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
        // DO/CALL gövdesi görünmez ama yazabilir: üretim sunucusunda onay ister (yanlış pozitif kabul edilir).
        // `select yazan_fonksiyon()` belgelenmiş sınırdır.
        let writes = matches!(a.kind, StmtKind::Write | StmtKind::Ddl) || runs_server_code(text);
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
                char_len: text.chars().count() as u32,
            },
            range,
            tx: tx_op(text),
            creates_session_state: a.creates_session_state,
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

pub(crate) async fn cancel_quietly(handle: &CancelHandle) {
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
    /// Bağlantıya bağlı oturum durumu olabilir: `Select` dışında bir ifade ya da oturum durumu oluşturan bir `SELECT`
    /// (oturum düzeyi advisory kilit, `set_config`) başarıyla tamamlandı (SET, geçici
    /// tablo, advisory kilit, DO/CALL, transaction komutu…). Böyle bir oturum boşta uyutulmaz; bağlantısı koparsa
    /// yeniden bağlanırken bir kez `SESSION_RESET_MESSAGE` döner.
    dirty: bool,
    /// Korumalı Parse `lock_timeout`'u geri yüklediğini doğrulayamadı: kullanıcının değeri; bir sonraki SQL'den önce
    /// uygulanır (`settle`). Yeni bağlantıda gereksizdir (sunucu varsayılanı).
    pending_lock_timeout: Option<String>,
    /// Açık transaction'da kullanıcı `lock_timeout`'u oturum düzeyinde değiştirdi (COMMIT'te bekleyen silinir).
    lock_timeout_set_in_tx: bool,
}

struct SessionSlot {
    server: Uuid,
    target: Target,
    guard: Guard,
    /// Kilitsiz okunur (`tx_state`); yalnızca `inner` kilidi tutulurken yazılır.
    state: StdMutex<TxState>,
    inner: Mutex<Inner>,
    cancel: CancelToken,
    /// Kilidi tutan arka plan işlemi (tamamlama); kullanıcı işlemi başlarsa ondan vazgeçilir.
    bg: Background,
    /// Tamamlama önbellekleri; 60 sn tazelik, DDL/SET/transaction sonrası boşaltılır.
    completions: StdMutex<Caches>,
}

type ItemsCache<K> = HashMap<K, (Instant, Vec<CompletionItem>)>;

#[derive(Default)]
struct Caches {
    /// İstemci isteği -> sonuç.
    requests: ItemsCache<CompletionRequest>,
    /// `to_regclass` ile çözülen ilişki kolonları: (şema, ad) -> sonuç.
    relations: ItemsCache<(Option<String>, String)>,
    /// Parse ile bulunan kolonlar: Parse edilen metnin karması -> sonuç.
    parsed: ItemsCache<u64>,
}

fn cache_get<K: std::hash::Hash + Eq>(map: &ItemsCache<K>, key: &K) -> Option<Vec<CompletionItem>> {
    map.get(key).filter(|(at, _)| at.elapsed() < COMPLETION_TTL).map(|(_, items)| items.clone())
}

fn cache_put<K: std::hash::Hash + Eq>(map: &mut ItemsCache<K>, key: K, items: Vec<CompletionItem>) {
    if map.len() >= COMPLETION_CACHE_MAX {
        map.clear();
    }
    map.insert(key, (Instant::now(), items));
}

impl Caches {
    fn clear(&mut self) {
        self.requests.clear();
        self.relations.clear();
        self.parsed.clear();
    }

    /// Kaynağın önbellekteki kolonları (`Fixed` ağ gerektirmez).
    fn source(&self, source: &Source) -> Option<Vec<CompletionItem>> {
        match source {
            Source::Fixed(names) => Some(crate::scoped::fixed_items(names)),
            Source::Catalog { schema, name } => cache_get(&self.relations, &(schema.clone(), name.clone())),
            Source::Parse(sql) => cache_get(&self.parsed, &crate::scoped::body_hash(sql)),
        }
    }
}

impl SessionSlot {
    fn state(&self) -> TxState {
        *self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn completions(&self) -> std::sync::MutexGuard<'_, Caches> {
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

/// Boşta oturum uyutulabilir mi: transaction yok, oturum durumu yok, süre doldu.
fn may_sleep(state: TxState, dirty: bool, idle: Duration, max_idle: Duration) -> bool {
    state == TxState::Idle && !dirty && idle >= max_idle
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

/// Oturum düzeyinde `lock_timeout`'u değiştirebilen kullanıcı ifadesi mi: `SET [SESSION] lock_timeout`,
/// `RESET lock_timeout`, `RESET ALL`, `DISCARD ALL`, `set_config('lock_timeout', …, false)` (`SET LOCAL` ve
/// `set_config(…, true)` hariç).
fn touches_lock_timeout(sql: &str) -> bool {
    use crate::lex::{Kind, lex};
    let toks: Vec<_> = lex(sql).into_iter().filter(|t| t.kind != Kind::Comment).collect();
    let word = |i: usize, w: &str| toks.get(i).is_some_and(|t| t.is_word(sql, w));
    let names_it = |t: &crate::lex::Token| {
        matches!(t.kind, Kind::Word | Kind::Ident | Kind::Str) && t.text(sql).to_ascii_lowercase().contains("lock_timeout")
    };
    if word(0, "discard") {
        return word(1, "all");
    }
    if (word(0, "set") && !word(1, "local")) || word(0, "reset") {
        return (word(0, "reset") && word(1, "all")) || toks.iter().skip(1).take(2).any(names_it);
    }
    // `set_config('lock_timeout', v, false)`: üçüncü argüman `true` ise transaction'a yereldir.
    let Some(f) = toks.iter().position(|t| t.is_word(sql, "set_config")) else { return false };
    let Some(open) = toks.get(f + 1).filter(|t| t.kind == Kind::Punct && t.text(sql) == "(") else { return false };
    let args: Vec<&crate::lex::Token> = toks[f + 2..]
        .iter()
        .take_while(|t| !(t.kind == Kind::Punct && t.text(sql) == ")" && t.depth == open.depth))
        .filter(|t| t.depth == open.depth + 1)
        .collect();
    let mut parts = args.split(|t| t.kind == Kind::Punct && t.text(sql) == ",");
    let first = parts.next().unwrap_or_default();
    let is_local = parts.nth(1).is_some_and(|third| third.iter().any(|t| t.is_word(sql, "true")));
    first.iter().any(|t| t.kind == Kind::Str && names_it(t)) && !is_local
}

/// Kullanıcının kendi `lock_timeout` değişikliği bekleyen eski geri yüklemeyi iptal eder — ama yalnızca kalıcı
/// olduğunda: transaction dışında (autocommit) hemen, transaction içinde ise COMMIT ile (ROLLBACK geri alır).
fn note_lock_timeout(pending: &mut Option<String>, set_in_tx: &mut bool, now: TxState, touched: bool, committed: bool) {
    if touched {
        if now == TxState::Idle {
            *pending = None;
        } else {
            *set_in_tx = true;
        }
    }
    if committed && *set_in_tx {
        *pending = None;
    }
    if now == TxState::Idle {
        *set_in_tx = false;
    }
}

/// Kullanıcı işlemi için oturum kilidi: kilidi bir arka plan işlemi (tamamlama) tutuyorsa ondan vazgeçmesi istenir
/// ve en çok `BACKGROUND_YIELD_WAIT` beklenir; başka bir işlem tutuyorsa (kısa bir yeniden denemeden sonra) `BUSY`.
/// Kilit alınınca bekleyen `lock_timeout` geri yüklemesi uygulanır.
async fn lock_user(slot: &SessionSlot) -> Result<tokio::sync::MutexGuard<'_, Inner>> {
    let mut inner = acquire_user(slot).await?;
    settle(slot, &mut inner).await;
    Ok(inner)
}

async fn acquire_user(slot: &SessionSlot) -> Result<tokio::sync::MutexGuard<'_, Inner>> {
    // Arka plan işlemi bayraklarını kilidi aldıktan hemen sonra kurar ve kilidi bırakmadan hemen önce düşürür; bu
    // pencerelerde `request_abort` `false` döner. Birkaç kısa yeniden deneme pencereyi kapatır.
    for attempt in 0..BUSY_RETRIES {
        match lock_open(slot) {
            Err(e) if matches!(e, Error::Invalid(_)) => {
                if slot.bg.request_abort() {
                    return match tokio::time::timeout(BACKGROUND_YIELD_WAIT, slot.inner.lock()).await {
                        Ok(_) if slot.cancel.closed() => Err(session_gone()),
                        Ok(inner) => Ok(inner),
                        Err(_) => Err(e),
                    };
                }
                if attempt + 1 == BUSY_RETRIES {
                    return Err(e);
                }
                tokio::time::sleep(Duration::from_millis(2)).await;
            }
            r => return r,
        }
    }
    Err(busy())
}

/// Bekleyen `lock_timeout` geri yüklemesini uygular. Başarısızsa ve oturum durumsuz/boştaysa bağlantı bırakılır
/// (sonraki işlem sessizce yeniden bağlanır, değer sunucu varsayılanına döner); aksi halde sonraki sefere kalır.
async fn settle(slot: &SessionSlot, inner: &mut Inner) {
    let Some(value) = inner.pending_lock_timeout.clone() else { return };
    let Some(exec) = inner.exec.as_mut().filter(|e| !e.is_closed()) else {
        inner.pending_lock_timeout = None; // yeniden bağlanınca sunucu varsayılanı geçerli
        return;
    };
    // Yalnızca transaction dışında (oturum düzeyinde) uygulanır: kullanıcı transaction'ında `set_config` geri
    // alınabilir ya da hata transaction'ı bozabilir. Aksi halde bekler.
    if slot.state() != TxState::Idle || exec.cursor_open() || exec.in_tx() {
        return;
    }
    let handle = exec.cancel_handle();
    if !crate::parse_guard::fail_restore()
        && matches!(settled(&handle, exec.query_text(RESTORE_LOCK_TIMEOUT, &[&value])).await, Some(Ok(_)))
    {
        inner.pending_lock_timeout = None;
        return;
    }
    if !inner.dirty && slot.state() == TxState::Idle && !exec.cursor_open() {
        tracing::warn!(server = %slot.server, "lock_timeout geri yüklenemedi; bağlantı bırakıldı (yeniden bağlanacak)");
        disarm(inner);
        inner.exec = None;
        inner.pending_lock_timeout = None;
    } else {
        tracing::warn!(server = %slot.server, "lock_timeout geri yüklenemedi; sonraki işlemde yeniden denenecek");
    }
}

/// Kilidi tutan arka plan işlemi: bekçi alanı önce düşer (bayraklar kilit bırakılmadan temizlenir).
struct BgLock<'a> {
    _bg: BackgroundGuard<'a>,
    inner: tokio::sync::MutexGuard<'a, Inner>,
}

/// Kilidi beklemeden alabilen arka plan işlemi; kilidi alamayan deneme bayraklara dokunmaz.
fn try_background(slot: &SessionSlot) -> Option<BgLock<'_>> {
    let inner = slot.inner.try_lock().ok()?;
    Some(BgLock { _bg: slot.bg.begin(), inner })
}

/// Oturumu zorla kapatır: kapalı işaretler (çalışan ifade iptal edilir), kilidi en çok `FORCE_CLOSE_WAIT`
/// bekler, açık transaction'ı geri alır ve bağlantıyı bırakır. Kilit zamanında alınamazsa çalışan işlem
/// bitince `finish` bağlantıyı bırakır (sunucu transaction'ı geri alır). Oturum haritadan önceden çıkarılmalıdır.
async fn force_close(slot: &SessionSlot) {
    slot.cancel.close();
    slot.bg.request_abort();
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
        let guard = self.current_guard(server)?;
        let profile = SessionProfile::Query { read_only: guard.locked, statement_timeout_ms: opts.statement_timeout_ms };
        let conn = self.mgr.open_dedicated(server, db, profile).await?;
        // Bağlanırken kilit/etiket değişmiş, sunucu silinmiş ya da bağlantısı kesilmiş (hedef değişikliği de
        // bağlantıyı keser) olabilir (`close_sessions_of` henüz haritada
        // olmayan bu oturumu göremez): eski korumayla oturum açılmaz, bağlantı bırakılır.
        let checked = self.guard_unchanged(server, guard).and_then(|()| {
            if self.mgr.is_connected(server) { Ok(()) } else { Err(Error::NotConnected) }
        });
        if let Err(e) = checked {
            drop(conn);
            return Err(e);
        }
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
                dirty: false,
                pending_lock_timeout: None,
                lock_timeout_set_in_tx: false,
            }),
            cancel: CancelToken::default(),
            bg: Background::default(),
            completions: StdMutex::default(),
        });
        self.map().insert(id, slot);
        tracing::debug!(%server, session = %id, locked = guard.locked, "sorgu oturumu açıldı");
        Ok(id)
    }

    fn current_guard(&self, server: Uuid) -> Result<Guard> {
        let registry = self.registry.read().unwrap_or_else(PoisonError::into_inner);
        let config = registry.get(server).ok_or(Error::UnknownServer(server))?;
        Ok(Guard { locked: config.read_only_lock, production: config.is_production() })
    }

    /// Kayıttaki koruma hâlâ `guard` mı; sunucu silindiyse `UnknownServer`, değiştiyse `SETTINGS_CHANGED_MESSAGE`.
    fn guard_unchanged(&self, server: Uuid, guard: Guard) -> Result<()> {
        if self.current_guard(server)? != guard {
            tracing::debug!(%server, "sunucu koruması bağlanırken değişti, oturum açılmadı");
            return Err(Error::Invalid(SETTINGS_CHANGED_MESSAGE.into()));
        }
        Ok(())
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
        let mut inner = lock_user(&slot).await?;
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
                    error: crate::error_codes::query_payload(&e),
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
            if ok && p.creates_session_state {
                inner.dirty = true;
            }
            let touched = ok && touches_lock_timeout(text);
            if ok && p.plan.kind != StmtKind::Select {
                // DDL, transaction komutları ve SET/DO/CALL… (arama yolu, katalog) önbelleği bayatlatabilir.
                slot.invalidate_completions();
                inner.dirty = true;
            }
            let event = if ok {
                Some(TxEvent::Ok(p.tx))
            } else if failed {
                Some(TxEvent::Error(p.tx))
            } else {
                None // alıcı gitti; ifadenin sonucu belirsiz, yoklama karar verir
            };
            let before = slot.state();
            if let Some(ev) = event {
                slot.set_state(step(before, ev).unwrap_or(before));
            }
            if !ok || p.plan.kind == StmtKind::Tx {
                resync(slot, exec).await;
            }
            let committed = ok && p.tx == Some(TxOp::Commit) && before == TxState::InTransaction;
            note_lock_timeout(&mut inner.pending_lock_timeout, &mut inner.lock_timeout_set_in_tx, slot.state(), touched, committed);
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
        let mut reset = false;
        if inner.exec.as_ref().is_some_and(Executor::is_closed) {
            inner.exec = None;
            let lost = slot.state() != TxState::Idle;
            reset = std::mem::take(&mut inner.dirty) && !lost;
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
            inner.pending_lock_timeout = None;
            inner.applied_timeout = Some(inner.opts.statement_timeout_ms);
            slot.set_state(TxState::Idle);
            tracing::debug!(server = %slot.server, "sorgu oturumu yeniden bağlandı");
        }
        if reset {
            tracing::warn!(server = %slot.server, "durumlu sorgu oturumunun bağlantısı koptu, yeniden kuruldu");
            return Err(Error::Invalid(SESSION_RESET_MESSAGE.into()));
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
        let mut inner = lock_user(&slot).await?;
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
        let mut inner = lock_user(&slot).await?;
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
        let before = slot.state();
        let result = exec.exec_internal(sql).await;
        // Başarılı ya da başarısız, COMMIT/ROLLBACK transaction'ı bitirir.
        slot.set_state(TxState::Idle);
        exec.set_in_tx(false);
        resync(slot, exec).await;
        let committed = result.is_ok() && op == TxAction::Commit && before == TxState::InTransaction;
        note_lock_timeout(&mut inner.pending_lock_timeout, &mut inner.lock_timeout_set_in_tx, slot.state(), false, committed);
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
        let mut inner = lock_user(&slot).await?;
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
        // Tamamlama gibi bir arka plan işlemi kilidi tutuyorsa ondan vazgeçilir (`lock_user`).
        let mut inner = match lock_user(&slot).await {
            Ok(inner) => inner,
            Err(e @ Error::NotFound(_)) => return Err(e),
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
    /// sonraki işlem sessizce yeniden bağlanır. Açık transaction'lı, durumlu (`dirty`) ya da meşgul oturumlara
    /// dokunmaz.
    pub fn reap_idle(&self, max_idle: Duration) {
        let slots: Vec<Arc<SessionSlot>> = self.map().values().cloned().collect();
        for slot in slots {
            let Ok(mut inner) = slot.inner.try_lock() else { continue };
            if inner.exec.is_some() && may_sleep(slot.state(), inner.dirty, inner.last_used.elapsed(), max_idle) {
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
    /// olursa da boş liste döner (tamamlama kullanıcı akışını bozmamalı). Tamamlama bir arka plan işlemidir:
    /// sürerken bir kullanıcı işlemi (`run`, `more`, `tx`, `export_csv`, `set_options`) başlarsa çalışan sorgu
    /// iptal edilir ve kilit bırakılır.
    ///
    /// `Scoped`: imleç kapsamı çözülür (`scope.rs`); ilişki → katalog kolonları (`to_regclass`, arama yoluna
    /// dokunmadan), CTE/alt sorgu → açık kolon listesi ya da gövdenin korumalı Parse'ı (çalıştırılmaz; bkz.
    /// `parse_guard`), fonksiyon → alias/kolon listesi; eşleşme yoksa `Columns { schema: None, relation: prefix }`.
    pub async fn complete(&self, session: SessionId, req: CompletionRequest) -> Result<Vec<CompletionItem>> {
        let slot = self.slot(session)?;
        if slot.state() == TxState::Failed {
            return Ok(Vec::new());
        }
        if let CompletionRequest::Scoped { sql, offset, prefix } = &req {
            return Self::complete_scoped(&slot, sql, *offset, prefix).await;
        }
        Self::complete_request(&slot, req).await
    }

    /// Arka plan işlemi için kilit: meşgulse, transaction hatalıysa, bağlantı kopmuşsa ya da oturum uykudaysa
    /// `None` (uykudaki oturum uyandırılmaz: bağlantı kurmak kilidi uzun tutar ve boşta uykuyu boşa çıkarır;
    /// kopan bağlantının bildirimi sonraki `run`a kalır). Bekleyen `lock_timeout` geri yüklemesi önce uygulanır.
    async fn lock_background(slot: &SessionSlot) -> Result<Option<BgLock<'_>>> {
        let Some(mut lock) = try_background(slot) else { return Ok(None) };
        if slot.cancel.closed() {
            return Err(session_gone());
        }
        settle(slot, &mut lock.inner).await;
        if slot.state() == TxState::Failed || lock.inner.exec.as_ref().is_none_or(Executor::is_closed) {
            return Ok(None);
        }
        Ok(Some(lock))
    }

    async fn complete_request(slot: &SessionSlot, req: CompletionRequest) -> Result<Vec<CompletionItem>> {
        if let Some(items) = cache_get(&slot.completions().requests, &req) {
            return Ok(items);
        }
        let Some(mut lock) = Self::lock_background(slot).await? else { return Ok(Vec::new()) };
        let inner = &mut *lock.inner;
        let Some(exec) = inner.exec.as_mut() else { return Ok(Vec::new()) };
        let deadline = Instant::now() + COMPLETION_TIMEOUT;
        let items = match Self::catalog_lookup(slot, exec, Lookup::Request(&req), deadline).await {
            Ok(items) => items,
            Err(e) => {
                tracing::debug!(code = %ErrorPayload::from(&e).code, "tamamlama sorgusu başarısız");
                return Ok(Vec::new());
            }
        };
        inner.last_used = Instant::now();
        cache_put(&mut slot.completions().requests, req, items.clone());
        Ok(items)
    }

    /// Katalog araması. Transaction içindeyse (kullanıcının ya da ızgara imlecinin) arama bir SAVEPOINT'e sarılır:
    /// hata, zaman aşımı ya da vazgeçme transaction'ı bozmaz.
    async fn catalog_lookup(
        slot: &SessionSlot,
        exec: &mut Executor,
        what: Lookup<'_>,
        deadline: Instant,
    ) -> Result<Vec<CompletionItem>> {
        const RELEASE: &str = "RELEASE SAVEPOINT pgt_complete";
        const ROLLBACK: &str = "ROLLBACK TO SAVEPOINT pgt_complete; RELEASE SAVEPOINT pgt_complete";
        let handle = exec.cancel_handle();
        let guarded = slot.state() != TxState::Idle || exec.cursor_open();
        if guarded {
            exec.exec_internal("SAVEPOINT pgt_complete").await?;
        }
        let found = bounded(&slot.bg, &handle, deadline, lookup(exec, what))
            .await
            .unwrap_or_else(|| Err(Error::Invalid("zaman aşımı".into())));
        if guarded {
            // Geç gelen bir iptal temizliği vurmuş olabilir: ROLLBACK TO hatalı alt transaction'ı da kurtarır.
            let released = found.is_ok() && exec.exec_internal(RELEASE).await.is_ok();
            if !released && exec.exec_internal(ROLLBACK).await.is_err() && exec.exec_internal(ROLLBACK).await.is_err() {
                resync(slot, exec).await;
            }
        }
        found
    }

    async fn complete_scoped(slot: &SessionSlot, sql: &str, offset: u32, prefix: &str) -> Result<Vec<CompletionItem>> {
        let targets = match crate::scoped::plan(sql, offset, prefix) {
            Plan::Fallback(relation) => {
                return Self::complete_request(slot, CompletionRequest::Columns { schema: None, relation }).await;
            }
            Plan::Targets(t) => t,
        };
        let mut bases: Vec<Option<Vec<CompletionItem>>> = {
            let cache = slot.completions();
            targets.iter().map(|t| cache.source(&t.source)).collect()
        };
        if bases.iter().any(Option::is_none) {
            Self::fill_sources(slot, &targets, &mut bases).await?;
        }
        Ok(crate::scoped::assemble(&targets, bases))
    }

    /// Önbellekte olmayan kaynakları oturum bağlantısında doldurur (katalog önce, sonra tek bir korumalı Parse
    /// grubu). Doldurulamayanlar `None` kalır.
    async fn fill_sources(
        slot: &SessionSlot,
        targets: &[crate::scoped::Target],
        bases: &mut [Option<Vec<CompletionItem>>],
    ) -> Result<()> {
        let Some(mut lock) = Self::lock_background(slot).await? else { return Ok(()) };
        let inner = &mut *lock.inner;
        let Some(exec) = inner.exec.as_mut() else { return Ok(()) };
        let deadline = Instant::now() + COMPLETION_TIMEOUT;
        for (t, base) in targets.iter().zip(bases.iter_mut()) {
            let Source::Catalog { schema, name } = &t.source else { continue };
            if base.is_some() || slot.bg.aborted() {
                continue;
            }
            let cached = slot.completions().source(&t.source);
            if let Some(items) = cached {
                *base = Some(items);
                continue;
            }
            let what = Lookup::RelationColumns { schema: schema.as_deref(), name };
            match Self::catalog_lookup(slot, exec, what, deadline).await {
                Ok(items) => {
                    cache_put(&mut slot.completions().relations, (schema.clone(), name.clone()), items.clone());
                    *base = Some(items);
                }
                Err(e) => tracing::debug!(code = %ErrorPayload::from(&e).code, "kolon araması başarısız"),
            }
        }
        let mut sqls: Vec<&str> = Vec::new();
        for (t, base) in targets.iter().zip(bases.iter()) {
            if let (None, Source::Parse(s)) = (base, &t.source)
                && !sqls.contains(&s.as_str())
            {
                sqls.push(s);
            }
        }
        if !sqls.is_empty() && !slot.bg.aborted() {
            let batch = guarded_parse(exec, slot.state(), &slot.bg, &sqls).await;
            if batch.broken {
                resync(slot, exec).await;
            }
            if let Some(v) = batch.restore_lock_timeout {
                inner.pending_lock_timeout = Some(v);
            }
            let mut cache = slot.completions();
            for (sql, r) in sqls.iter().zip(batch.results) {
                let items = match r {
                    Parsed::Columns(cols) => crate::scoped::parsed_items(&cols),
                    // Yalnızca metne bağlı kalıcı hatalar (42xxx sözdizimi/ad, 0A000 desteklenmiyor) 60 sn boş sonuç
                    // olarak saklanır ki her tuş vuruşunda yeniden denenmesin; kilit, iptal, bağlantı vb. geçicidir.
                    Parsed::Failed(Error::Query { sqlstate: Some(code), .. }, _)
                        if code.starts_with("42") || code == "0A000" =>
                    {
                        Vec::new()
                    }
                    Parsed::Failed(e, _) => {
                        tracing::debug!(code = %ErrorPayload::from(&e).code, "kolon Parse'ı başarısız");
                        continue;
                    }
                    Parsed::Skipped => continue,
                };
                cache_put(&mut cache.parsed, crate::scoped::body_hash(sql), items);
            }
            for (t, base) in targets.iter().zip(bases.iter_mut()) {
                if base.is_none() && matches!(t.source, Source::Parse(_)) {
                    *base = cache.source(&t.source);
                }
            }
        }
        inner.last_used = Instant::now();
        Ok(())
    }

    /// Parse tabanlı hata denetimi (tasarım §4): imlecin ifadesi ve görünür alanla kesişen ifadeler (`CheckTarget`;
    /// betiğin tamamı bölünür, bu sırayla en çok `CHECK_MAX_STMTS` / `CHECK_MAX_BYTES`)
    /// oturum bağlantısında yalnızca Parse edilir, **çalıştırılmaz** (korumalar: `parse_guard`; `Idle` doğrudan,
    /// transaction içinde savepoint, `lock_timeout` 200 ms, toplam ≤ 1 sn). Denetim bir arka plan işlemidir: oturum
    /// meşgulse, uykudaysa ya da transaction `Failed` ise beklemeden boş liste döner; sürerken bir kullanıcı işlemi
    /// başlarsa denetim iptal edilir. Kilit zaman aşımı/iptal edilen ifade için teşhis yoktur. İstek günlüğe ve
    /// geçmişe yazılmaz.
    pub async fn check(&self, session: SessionId, sql: &str, target: CheckTarget) -> Result<Vec<CheckDiag>> {
        let slot = self.slot(session)?;
        let stmts = crate::check::select(sql, target)?;
        if stmts.is_empty() || slot.state() == TxState::Failed {
            return Ok(Vec::new());
        }
        let Some(mut lock) = Self::lock_background(&slot).await? else { return Ok(Vec::new()) };
        let inner = &mut *lock.inner;
        let Some(exec) = inner.exec.as_mut() else { return Ok(Vec::new()) };
        let texts: Vec<&str> = stmts.iter().map(|s| &sql[s.range.clone()]).collect();
        let batch = guarded_parse(exec, slot.state(), &slot.bg, &texts).await;
        if batch.broken {
            resync(&slot, exec).await;
        }
        if let Some(v) = batch.restore_lock_timeout {
            inner.pending_lock_timeout = Some(v);
        }
        inner.last_used = Instant::now();
        let mut diags: Vec<CheckDiag> =
            stmts.iter().zip(batch.results).filter_map(|(s, r)| crate::check::diag(s, r)).collect();
        diags.sort_by_key(|d| d.stmt_index);
        Ok(diags)
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
        let mut inner = lock_user(&slot).await?;
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
        if result.is_ok() && classify(text).creates_session_state {
            inner.dirty = true;
        }
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
                dirty: false,
                pending_lock_timeout: None,
                lock_timeout_set_in_tx: false,
            }),
            cancel: CancelToken::default(),
            bg: Background::default(),
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
        // DO / CALL / EXECUTE gövdesi görünmez: üretimde onay ister, etiketsiz sunucuda istemez.
        for sql in [
            "do $$ begin delete from t; end $$", "call p()", "/* c */ DO $$ begin null; end $$",
            "EXECUTE x", "execute x(1, 2)",
        ] {
            assert!(plans(sql, PROD)[0].production_write, "{sql}");
            assert_eq!(invalid(gate(sql, PROD, false)), CONFIRMATION_REQUIRED, "{sql}");
            assert!(gate(sql, PROD, true).is_ok());
            assert!(!plans(sql, PLAIN)[0].production_write, "{sql}");
            assert!(gate(sql, PLAIN, false).is_ok(), "{sql}");
        }
        // Hazırlanmış ifade: PREPARE yazmaz ama EXECUTE onay ister.
        let p = plans("prepare x as delete from t; execute x", PROD);
        assert_eq!(p.iter().map(|p| p.production_write).collect::<Vec<_>>(), [false, true]);
        assert_eq!(invalid(gate("prepare x as delete from t; execute x", PROD, false)), CONFIRMATION_REQUIRED);
        assert!(!plans("select f_that_writes()", PROD)[0].production_write, "belgelenmiş sınır");

        let p = plans("update t set a = 1", PLAIN);
        assert_eq!((p[0].danger, p[0].production_write), (Some(Danger::UpdateWithoutWhere), false));
        assert_eq!(invalid(gate("select 1; update t set a = 1", PLAIN, false)), CONFIRMATION_REQUIRED);
        assert!(gate("update t set a = 1", PLAIN, true).is_ok());
        assert!(gate("update t set a = 1 where id = 1", PLAIN, false).is_ok());
        // Kilit onaydan önce gelir: onay kilidi aşmaz.
        assert_eq!(invalid(gate("delete from t", LOCKED, true)), LOCKED_WRITE_MESSAGE);
    }

    #[test]
    fn select_session_state_calls_are_flagged_in_plan() {
        let p = plan_script(
            "select 1; select pg_advisory_lock(1); select set_config('a', 'b', false); select pg_advisory_xact_lock(1)",
            Scope::All,
            PLAIN,
        )
        .unwrap();
        assert_eq!(p.iter().map(|p| p.creates_session_state).collect::<Vec<_>>(), [false, true, true, false]);
        assert!(p.iter().all(|p| p.plan.kind == StmtKind::Select));
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
        assert_eq!(all.iter().map(|p| p.plan.char_len).collect::<Vec<_>>(), [11, 8, 8], "ğü iki karakter");

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

    /// Kilidi bir arka plan işlemi (tamamlama) tutuyorsa kullanıcı işlemi ondan vazgeçmesini ister ve kilidi alır;
    /// arka plan dışı bir tutucu varsa beklemeden BUSY.
    #[tokio::test]
    async fn user_operation_makes_background_work_yield() {
        let qm = manager();
        let id = qm.insert_sleeping(PLAIN, TxState::Idle);
        let slot = qm.slot(id).unwrap();
        let (ready_tx, ready_rx) = tokio::sync::oneshot::channel();
        let worker = {
            let slot = slot.clone();
            tokio::spawn(async move {
                let held = try_background(&slot).expect("kilit boş");
                ready_tx.send(()).unwrap();
                while !slot.bg.aborted() {
                    tokio::time::sleep(Duration::from_millis(1)).await;
                }
                drop(held);
            })
        };
        ready_rx.await.unwrap();
        // Üst üste binen ikinci arka plan denemeleri (yazarken art arda tamamlama) kilidi alamaz ve ilk
        // işlemin bayraklarına dokunmaz.
        assert!(try_background(&slot).is_none());
        for _ in 0..3 {
            assert!(qm.complete(id, CompletionRequest::Schemas { include_system: false }).await.unwrap().is_empty());
        }
        let started = std::time::Instant::now();
        assert!(lock_user(&slot).await.is_ok(), "arka plan işlemi kilidi bırakmalı; çalıştırma BUSY almamalı");
        assert!(started.elapsed() < Duration::from_secs(1), "{:?}", started.elapsed());
        worker.await.unwrap();
        assert!(!slot.bg.request_abort(), "arka plan işlemi bitti");

        let held = slot.inner.lock().await;
        let started = std::time::Instant::now();
        assert!(matches!(lock_user(&slot).await, Err(Error::Invalid(m)) if m == BUSY_MESSAGE));
        assert!(started.elapsed() < Duration::from_millis(100));
        drop(held);
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

    /// `open()` bağlantıdan sonra korumayı yeniden doğrular: kilit/etiket değiştiyse ya da sunucu silindiyse
    /// eski korumayla oturum açılmaz.
    #[test]
    fn guard_is_revalidated_against_the_registry() {
        let dir = tempfile::tempdir().unwrap();
        let mut registry = Registry::load(dir.path().join("servers.json")).unwrap().registry;
        let config = registry.upsert(pgt_core::config::ServerConfig::new("s", "h", 5432, "u")).unwrap();
        let registry = Arc::new(RwLock::new(registry));
        let qm = QueryManager::new(Arc::new(ConnectionManager::new()), registry.clone());
        let before = qm.current_guard(config.id).unwrap();
        assert_eq!(before, PLAIN);
        assert!(qm.guard_unchanged(config.id, before).is_ok());

        type Change = fn(&mut pgt_core::config::ServerConfig);
        let changes: [Change; 2] = [|c| c.read_only_lock = true, |c| c.label = Some("Üretim".into())];
        for change in changes {
            let mut changed = config.clone();
            change(&mut changed);
            registry.write().unwrap().upsert(changed).unwrap();
            let err = qm.guard_unchanged(config.id, before).unwrap_err();
            assert!(matches!(&err, Error::Invalid(m) if m == SETTINGS_CHANGED_MESSAGE), "{err:?}");
            registry.write().unwrap().upsert(config.clone()).unwrap();
        }
        // Yalnızca ad/renk değişimi korumayı değiştirmez.
        let mut renamed = config.clone();
        renamed.name = "yeni".into();
        registry.write().unwrap().upsert(renamed).unwrap();
        assert!(qm.guard_unchanged(config.id, before).is_ok());
        registry.write().unwrap().remove(config.id).unwrap();
        assert!(matches!(qm.guard_unchanged(config.id, before), Err(Error::UnknownServer(_))));
    }

    #[test]
    fn only_clean_idle_sessions_may_sleep() {
        let (min, zero) = (Duration::from_secs(1800), Duration::ZERO);
        assert!(may_sleep(TxState::Idle, false, min, min));
        assert!(!may_sleep(TxState::Idle, false, zero, min), "süre dolmadı");
        assert!(!may_sleep(TxState::Idle, true, min, min), "durumlu oturum uyutulmaz");
        assert!(!may_sleep(TxState::InTransaction, false, min, min));
        assert!(!may_sleep(TxState::Failed, false, min, min));
    }

    #[test]
    fn lock_timeout_touching_statements() {
        for sql in [
            "set lock_timeout = '5s'",
            "SET SESSION lock_timeout TO 0",
            "reset lock_timeout",
            "RESET ALL",
            "select set_config('lock_timeout', '1s', false)",
            "select pg_catalog.set_config('lock_timeout', f(1, 2), false)",
            "discard all",
            "set \"lock_timeout\" = 1",
        ] {
            assert!(touches_lock_timeout(sql), "{sql}");
        }
        for sql in [
            "set local lock_timeout = 1",
            "set statement_timeout = 1",
            "select 'lock_timeout'",
            "reset role",
            "select set_config('lock_timeout', '1s', true)",
            "select set_config('statement_timeout', 'lock_timeout', false)",
            "discard plans",
        ] {
            assert!(!touches_lock_timeout(sql), "{sql}");
        }
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

/// Bekleyen `lock_timeout` geri yüklemesini gerçek sunucuda sınar (`PGT_TEST_URLS` yoksa atlanır).
#[cfg(test)]
mod db_tests {
    use std::sync::atomic::Ordering;

    use pgt_core::testing::test_servers;

    use super::*;
    use crate::parse_guard::{TEST_FAIL_RESTORE, TEST_SERIAL};

    struct Nop;
    impl EventSink for Nop {
        fn send(&mut self, _: QueryEvent) -> bool {
            true
        }
    }

    struct Rows(Vec<String>);
    impl EventSink for Rows {
        fn send(&mut self, ev: QueryEvent) -> bool {
            if let QueryEvent::Rows { rows, .. } = ev {
                self.0.extend(rows.into_iter().filter_map(|r| r.into_iter().next().flatten()));
            }
            true
        }
    }

    struct FailRestore;
    impl FailRestore {
        fn on() -> FailRestore {
            TEST_FAIL_RESTORE.store(true, Ordering::SeqCst);
            FailRestore
        }
    }
    impl Drop for FailRestore {
        fn drop(&mut self) {
            TEST_FAIL_RESTORE.store(false, Ordering::SeqCst);
        }
    }

    async fn show(qm: &QueryManager, id: SessionId) -> String {
        let mut rows = Rows(Vec::new());
        qm.run(id, "show lock_timeout", Scope::All, 10, true, &mut rows).await.unwrap();
        rows.0.pop().unwrap()
    }

    fn pending(qm: &QueryManager, id: SessionId) -> Option<String> {
        qm.slot(id).unwrap().inner.try_lock().unwrap().pending_lock_timeout.clone()
    }

    #[tokio::test]
    async fn unconfirmed_lock_timeout_restore_is_applied_before_the_next_user_sql() {
        let _serial = TEST_SERIAL.lock().await;
        for ts in test_servers() {
            let dir = tempfile::tempdir().unwrap();
            let mut registry = Registry::load(dir.path().join("servers.json")).unwrap().registry;
            registry.upsert(ts.config.clone()).unwrap();
            let mgr = Arc::new(ConnectionManager::new());
            mgr.connect(ts.config.clone(), ts.password.clone()).await.unwrap();
            let qm = QueryManager::new(mgr, Arc::new(RwLock::new(registry)));
            let opts = SessionOptions { mode: TxMode::Autocommit, statement_timeout_ms: 0 };
            let cte = |n: u32| CompletionRequest::Scoped {
                sql: format!("with c as (select {n} as v) select c. from c"),
                offset: 34 + n.to_string().len() as u32,
                prefix: "c".into(),
            };

            // Durumlu oturum (kullanıcı SET'i): bekleyen değer sonraki `run`dan önce uygulanır.
            let id = qm.open(ts.config.id, Target::Maintenance, opts).await.unwrap();
            qm.run(id, "set lock_timeout = '3s'", Scope::All, 10, true, &mut Nop).await.unwrap();
            {
                let _fail = FailRestore::on();
                let items = qm.complete(id, cte(1)).await.unwrap();
                assert_eq!(items.iter().map(|i| i.label.as_str()).collect::<Vec<_>>(), ["v"]);
            }
            assert_eq!(pending(&qm, id).as_deref(), Some("3s"));
            assert_eq!(show(&qm, id).await, "3s");
            assert_eq!(pending(&qm, id), None);

            // Durumsuz oturum: geri yükleme yine başarısızsa bağlantı bırakılır, sonraki işlem yeniden bağlanır.
            let id = qm.open(ts.config.id, Target::Maintenance, opts).await.unwrap();
            {
                let _fail = FailRestore::on();
                qm.complete(id, cte(2)).await.unwrap();
                assert_eq!(pending(&qm, id).as_deref(), Some("0"));
                qm.complete(id, cte(3)).await.unwrap();
                assert!(qm.is_asleep(id).unwrap(), "bağlantı bırakılmalı");
                assert_eq!(pending(&qm, id), None);
            }
            assert_eq!(show(&qm, id).await, "0");

            // Kullanıcının kendi SET'i bekleyen eski değeri iptal eder.
            let id = qm.open(ts.config.id, Target::Maintenance, opts).await.unwrap();
            qm.run(id, "set lock_timeout = '3s'", Scope::All, 10, true, &mut Nop).await.unwrap();
            {
                let _fail = FailRestore::on();
                qm.complete(id, cte(4)).await.unwrap();
                assert_eq!(pending(&qm, id).as_deref(), Some("3s"));
                // settle başarısız (durumlu oturum: bekler), sonra kullanıcının SET'i başarılı.
                qm.run(id, "set lock_timeout = '5s'", Scope::All, 10, true, &mut Nop).await.unwrap();
                assert_eq!(pending(&qm, id), None);
            }
            assert_eq!(show(&qm, id).await, "5s");

            // Kullanıcı transaction'ında bekleyen değer uygulanmaz; transaction bitince uygulanır.
            let manual = SessionOptions { mode: TxMode::Manual, statement_timeout_ms: 0 };
            let id = qm.open(ts.config.id, Target::Maintenance, manual).await.unwrap();
            qm.run(id, "set lock_timeout = '3s'", Scope::All, 10, true, &mut Nop).await.unwrap();
            qm.tx(id, TxAction::Commit).await.unwrap();
            {
                let _fail = FailRestore::on();
                qm.complete(id, cte(5)).await.unwrap();
                assert_eq!(pending(&qm, id).as_deref(), Some("3s"));
                qm.run(id, "select 1", Scope::All, 10, true, &mut Nop).await.unwrap(); // otomatik BEGIN
            }
            assert_eq!(qm.tx_state(id).unwrap(), TxState::InTransaction);
            qm.run(id, "select 2", Scope::All, 10, true, &mut Nop).await.unwrap();
            assert_eq!(pending(&qm, id).as_deref(), Some("3s"), "transaction içinde uygulanmamalı");
            qm.tx(id, TxAction::Rollback).await.unwrap();
            assert_eq!(pending(&qm, id).as_deref(), Some("3s"));
            let mut rows = Rows(Vec::new());
            qm.run(id, "show lock_timeout", Scope::All, 10, true, &mut rows).await.unwrap();
            assert_eq!(rows.0, ["3s"]);
            assert_eq!(pending(&qm, id), None);
            qm.tx(id, TxAction::Rollback).await.unwrap();

            // Transaction içindeki SET bekleyeni ancak COMMIT ile iptal eder; ROLLBACK etmez.
            {
                let _fail = FailRestore::on();
                qm.complete(id, cte(6)).await.unwrap();
                assert_eq!(pending(&qm, id).as_deref(), Some("3s"));
                qm.run(id, "set lock_timeout = '5s'", Scope::All, 10, true, &mut Nop).await.unwrap();
                assert_eq!(pending(&qm, id).as_deref(), Some("3s"), "transaction içinde silinmez");
                qm.tx(id, TxAction::Rollback).await.unwrap();
                assert_eq!(pending(&qm, id).as_deref(), Some("3s"), "ROLLBACK SET'i geri aldı");
                qm.run(id, "set lock_timeout = '7s'", Scope::All, 10, true, &mut Nop).await.unwrap();
                qm.tx(id, TxAction::Commit).await.unwrap();
                assert_eq!(pending(&qm, id), None, "COMMIT kullanıcı değerini kalıcı yaptı");
            }
            let mut rows = Rows(Vec::new());
            qm.run(id, "show lock_timeout", Scope::All, 10, true, &mut rows).await.unwrap();
            assert_eq!(rows.0, ["7s"]);
            qm.tx(id, TxAction::Rollback).await.unwrap();
        }
    }
}
