//! Korumalı Parse (tamamlama ve hata denetimi ortak yardımcısı) ve arka plan işlemlerinin kullanıcı işlemine
//! yol vermesi.
//!
//! Korumalar (tasarım §4.2, bağlayıcı):
//! - **Hiçbir şey çalıştırılmaz:** yalnızca Parse/Describe (`prepare`); Bind/Execute yok.
//! - **Kullanıcı transaction'ına dokunulmaz:** oturum boştaysa Parse doğrudan (örtük transaction; kilitler Sync'te
//!   bırakılır); transaction içindeyse (ya da ızgara imlecinin transaction'ı açıksa) `SAVEPOINT pgt_parse` …
//!   `ROLLBACK TO` + `RELEASE` içinde (Parse'ın aldığı kilitler geri bırakılır); `Failed` ise hiçbir şey yapılmaz.
//! - **Kilit bekleme yok:** `lock_timeout` 200 ms (savepoint içinde `SET LOCAL`; doğrudan yolda önce `SHOW` ile
//!   okunan kullanıcı değeri sonra geri yüklenir).
//! - **Kullanıcıyı bekletmez:** toplam Parse süresi `PARSE_BUDGET`; bir kullanıcı işlemi (`run`, `more`, `tx`,
//!   `export`, `set_options`) başlarsa çalışan Parse iptal edilir ve kalanlar atlanır.
//!
//! SQL metni günlüğe yazılmaz.

use std::future::Future;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use pgt_core::connection::CancelHandle;
use pgt_core::{Error, ErrorPayload, Result};
use tokio::sync::Notify;
use tokio::time::Instant;

use crate::exec::{Executor, TextRows};
use crate::session::{TxState, cancel_quietly};

/// Bir Parse grubunun oturum kilidini tutabileceği en uzun süre.
pub(crate) const PARSE_BUDGET: Duration = Duration::from_secs(1);
/// Kurulum/temizlik komutlarının (savepoint, `lock_timeout`) en uzun süresi.
const STEP_WAIT: Duration = Duration::from_secs(2);
/// İptal edilen isteğin sonuçlanması için en uzun bekleme (bağlantı sırası tutarlı kalsın).
const DRAIN_WAIT: Duration = Duration::from_secs(1);

const SAVEPOINT: &str = "SAVEPOINT pgt_parse";
const LOCAL_LOCK_TIMEOUT: &str = "SET LOCAL lock_timeout = '200ms'";
const ROLLBACK_TO: &str = "ROLLBACK TO SAVEPOINT pgt_parse";
const CLEANUP: &str = "ROLLBACK TO SAVEPOINT pgt_parse; RELEASE SAVEPOINT pgt_parse";
const SWAP_LOCK_TIMEOUT: &str = "SHOW lock_timeout; SET lock_timeout = '200ms'";
const RESTORE_LOCK_TIMEOUT: &str = "SELECT pg_catalog.set_config('lock_timeout', $1::pg_catalog.text, false)";

// ---------------------------------------------------------------------------------------------------------------
// Arka plan işlemi
// ---------------------------------------------------------------------------------------------------------------

/// Oturum kilidini tutan arka plan işleminin (tamamlama, denetim) bayrakları. Kullanıcı işlemi kilidi alamazsa
/// `request_abort` ile arka plan işleminden bırakmasını ister.
#[derive(Default)]
pub(crate) struct Background {
    active: AtomicBool,
    abort: AtomicBool,
    notify: Notify,
}

impl Background {
    /// Arka plan işlemini başlatır; bekçi düşünce biter.
    pub(crate) fn begin(&self) -> BackgroundGuard<'_> {
        self.abort.store(false, Ordering::SeqCst);
        self.active.store(true, Ordering::SeqCst);
        BackgroundGuard(self)
    }

    /// Bir arka plan işlemi sürüyorsa bırakmasını ister (`true`).
    pub(crate) fn request_abort(&self) -> bool {
        if !self.active.load(Ordering::SeqCst) {
            return false;
        }
        self.abort.store(true, Ordering::SeqCst);
        self.notify.notify_one();
        true
    }

    pub(crate) fn aborted(&self) -> bool {
        self.abort.load(Ordering::SeqCst)
    }
}

pub(crate) struct BackgroundGuard<'a>(&'a Background);

impl Drop for BackgroundGuard<'_> {
    fn drop(&mut self) {
        self.0.active.store(false, Ordering::SeqCst);
        self.0.abort.store(false, Ordering::SeqCst);
    }
}

/// `fut`'u `deadline`'a kadar ya da arka plan işleminden vazgeçilene kadar çalıştırır. Süre dolarsa ya da vazgeçilirse
/// sunucuya iptal gönderilir, isteğin sonuçlanması en çok `DRAIN_WAIT` beklenir ve `None` döner. Vazgeçme isteği
/// `fut` hiç başlamadan gelmişse istek gönderilmez.
pub(crate) async fn bounded<F: Future>(bg: &Background, handle: &CancelHandle, deadline: Instant, fut: F) -> Option<F::Output> {
    if bg.aborted() || Instant::now() >= deadline {
        return None;
    }
    let mut fut = std::pin::pin!(fut);
    loop {
        tokio::select! {
            biased;
            out = &mut fut => return Some(out),
            () = tokio::time::sleep_until(deadline) => break,
            () = bg.notify.notified() => {
                if bg.aborted() {
                    break;
                }
            }
        }
    }
    cancel_quietly(handle).await;
    let _ = tokio::time::timeout(DRAIN_WAIT, fut).await;
    None
}

// ---------------------------------------------------------------------------------------------------------------
// Korumalı Parse
// ---------------------------------------------------------------------------------------------------------------

#[derive(Debug)]
pub(crate) enum Parsed {
    /// Sonuç sütunları (ad, tür adı); satır döndürmeyen ifadede boş.
    Columns(Vec<(String, String)>),
    /// Sunucu Parse'ı reddetti (sözdizimi, bilinmeyen ad, kilit zaman aşımı…).
    Failed(Error),
    /// Denenmedi (süre doldu, kullanıcı işlemi başladı, transaction hatalı, kurulum başarısız).
    Skipped,
}

#[derive(Debug)]
pub(crate) struct ParseBatch {
    /// `sqls` ile aynı sırada ve uzunlukta.
    pub results: Vec<Parsed>,
    /// Temizlik başarısız oldu: transaction durumu yoklanmalı (savepoint yolu) ya da `lock_timeout` geri
    /// yüklenemedi (doğrudan yol).
    pub broken: bool,
}

async fn step(exec: &mut Executor, sql: &str) -> Result<TextRows> {
    match tokio::time::timeout(STEP_WAIT, exec.exec_internal(sql)).await {
        Ok(r) => r,
        Err(_) => Err(Error::Invalid("zaman aşımı".into())),
    }
}

/// Temizlik bir kez yinelenir: geç gelen bir iptal ilk denemeyi vurmuş olabilir (`ROLLBACK TO` hatalı alt
/// transaction'ı da kurtarır).
async fn cleanup(exec: &mut Executor, sql: &str) -> bool {
    for _ in 0..2 {
        match step(exec, sql).await {
            Ok(_) => return true,
            Err(e) => tracing::debug!(code = %ErrorPayload::from(&e).code, "Parse temizliği başarısız"),
        }
    }
    false
}

async fn restore_lock_timeout(exec: &mut Executor, previous: &str) -> bool {
    for _ in 0..2 {
        let r = tokio::time::timeout(STEP_WAIT, exec.query_text(RESTORE_LOCK_TIMEOUT, &[&previous])).await;
        match r {
            Ok(Ok(_)) => return true,
            Ok(Err(e)) => tracing::debug!(code = %ErrorPayload::from(&e).code, "lock_timeout geri yüklenemedi"),
            Err(_) => tracing::debug!("lock_timeout geri yüklemesi zaman aşımına uğradı"),
        }
    }
    false
}

/// `sqls`'in her birini korumalarla Parse eder (bkz. modül belgesi). Çağıran oturum kilidini tutar.
pub(crate) async fn guarded_parse(exec: &mut Executor, state: TxState, bg: &Background, sqls: &[&str]) -> ParseBatch {
    let deadline = Instant::now() + PARSE_BUDGET;
    let mut results = Vec::with_capacity(sqls.len());
    let mut broken = false;
    if state != TxState::Failed && !sqls.is_empty() && !bg.aborted() {
        let handle = exec.cancel_handle();
        if state != TxState::Idle || exec.cursor_open() || exec.in_tx() {
            broken = in_savepoint(exec, bg, &handle, deadline, sqls, &mut results).await;
        } else {
            broken = direct(exec, bg, &handle, deadline, sqls, &mut results).await;
        }
    }
    results.resize_with(sqls.len(), || Parsed::Skipped);
    ParseBatch { results, broken }
}

/// Transaction içinde: tek savepoint; başarısız Parse alt transaction'ı bozduğundan `ROLLBACK TO` ile
/// kurtarılır (savepoint kalır, `SET LOCAL` yenilenir). Sonda `ROLLBACK TO` (kilitler bırakılır) + `RELEASE`.
async fn in_savepoint(
    exec: &mut Executor,
    bg: &Background,
    handle: &CancelHandle,
    deadline: Instant,
    sqls: &[&str],
    results: &mut Vec<Parsed>,
) -> bool {
    // SAVEPOINT tek başına gönderilir: başarısızsa kullanıcı transaction'ında hiçbir şey değişmemiştir.
    if step(exec, SAVEPOINT).await.is_err() {
        return false;
    }
    let mut ready = step(exec, LOCAL_LOCK_TIMEOUT).await.is_ok();
    for sql in sqls {
        if !ready {
            break;
        }
        match bounded(bg, handle, deadline, exec.prepare_described(sql)).await {
            Some(Ok(cols)) => results.push(Parsed::Columns(cols)),
            Some(Err(e)) => {
                results.push(Parsed::Failed(e));
                ready = step(exec, ROLLBACK_TO).await.is_ok() && step(exec, LOCAL_LOCK_TIMEOUT).await.is_ok();
            }
            None => break,
        }
    }
    !cleanup(exec, CLEANUP).await
}

/// Boşta: her Parse kendi örtük transaction'ında; `lock_timeout` geçici olarak 200 ms.
async fn direct(
    exec: &mut Executor,
    bg: &Background,
    handle: &CancelHandle,
    deadline: Instant,
    sqls: &[&str],
    results: &mut Vec<Parsed>,
) -> bool {
    // Tek gidiş-dönüş: SHOW başarılı, SET başarısızsa hiçbir şey değişmemiştir.
    let previous = match step(exec, SWAP_LOCK_TIMEOUT).await {
        Ok(rows) => match rows.into_iter().next().and_then(|r| r.into_iter().next().flatten()) {
            Some(v) => v,
            None => return false,
        },
        Err(_) => return false,
    };
    for sql in sqls {
        match bounded(bg, handle, deadline, exec.prepare_described(sql)).await {
            Some(Ok(cols)) => results.push(Parsed::Columns(cols)),
            Some(Err(e)) => results.push(Parsed::Failed(e)),
            None => break,
        }
    }
    let restored = restore_lock_timeout(exec, &previous).await;
    if !restored {
        tracing::warn!("lock_timeout geri yüklenemedi");
    }
    !restored
}
