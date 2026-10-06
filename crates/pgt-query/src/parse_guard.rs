//! Korumalı Parse (tamamlama ve hata denetimi ortak yardımcısı) ve arka plan işlemlerinin kullanıcı işlemine
//! yol vermesi.
//!
//! Korumalar (tasarım §4.2, bağlayıcı):
//! - **Hiçbir şey çalıştırılmaz:** kullanıcı metni için yalnızca Parse/Describe (`prepare`); Bind/Execute yok.
//!   Not: tokio-postgres, sonuç sütunlarında tanımadığı (yerleşik olmayan) bir tür görürse tür bilgisini kendi sabit
//!   katalog sorgusuyla (`pg_catalog.pg_type` üzerinde Bind/Execute) çözer; bu kullanıcı SQL'i değildir ve aynı
//!   korumaların (savepoint / örtük transaction) içinde çalışır.
//! - **Kullanıcı transaction'ına dokunulmaz:** oturum boştaysa Parse doğrudan (örtük transaction; kilitler Sync'te
//!   bırakılır); transaction içindeyse (ya da ızgara imlecinin transaction'ı açıksa) her çağrıya özgü
//!   `SAVEPOINT pgt_parse_<n>` … `ROLLBACK TO` + `RELEASE` içinde (Parse'ın aldığı kilitler geri bırakılır);
//!   `Failed` ise hiçbir şey yapılmaz.
//! - **Kilit bekleme yok:** `lock_timeout` 200 ms (savepoint içinde `SET LOCAL`; doğrudan yolda önce `SHOW` ile
//!   okunan kullanıcı değeri sonra geri yüklenir; geri yükleme doğrulanamazsa değer `ParseBatch::restore_lock_timeout`
//!   ile çağırana bırakılır ve oturum bir sonraki SQL'den önce uygular).
//! - **Kullanıcıyı bekletmez:** toplam Parse süresi `PARSE_BUDGET`; bir kullanıcı işlemi (`run`, `more`, `tx`,
//!   `export`, `set_options`, `close`) başlarsa çalışan Parse iptal edilir ve kalanlar atlanır.
//! - **Zaman aşımı "olmadı" demek değildir:** süresi dolan bir komut düşürülmez; sunucuya iptal gönderilir ve sonucu
//!   (`DRAIN_WAIT` boyunca) beklenir. Sonuç yine bilinmiyorsa durum "bozuk" sayılır (çağıran yoklar).
//!
//! SQL metni günlüğe yazılmaz.

use std::future::Future;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::Duration;

use pgt_core::connection::CancelHandle;
use pgt_core::{Error, ErrorPayload, Result};
use tokio::sync::Notify;
use tokio::time::Instant;

use crate::exec::{Executor, TextRows};
use crate::session::{TxState, cancel_quietly};

/// Bir Parse grubunun oturum kilidini tutabileceği en uzun süre.
pub(crate) const PARSE_BUDGET: Duration = Duration::from_secs(1);
/// Kurulum/temizlik komutlarının (savepoint, `lock_timeout`) iptalden önceki en uzun süresi.
const STEP_WAIT: Duration = Duration::from_secs(2);
/// İptal edilen isteğin sonuçlanması için en uzun bekleme (bağlantı sırası tutarlı kalsın).
const DRAIN_WAIT: Duration = Duration::from_secs(1);

const LOCAL_LOCK_TIMEOUT: &str = "SET LOCAL lock_timeout = '200ms'";
const SHOW_LOCK_TIMEOUT: &str = "SHOW lock_timeout";
const SET_LOCK_TIMEOUT: &str = "SET lock_timeout = '200ms'";
pub(crate) const RESTORE_LOCK_TIMEOUT: &str =
    "SELECT pg_catalog.set_config('lock_timeout', $1::pg_catalog.text, false)";

/// Savepoint adları çağrı başına benzersizdir (kullanıcının aynı adlı savepoint'ine dokunulmasın).
static SAVEPOINT_SEQ: AtomicU64 = AtomicU64::new(1);

/// Test kancaları: adım süresini kısaltır (zaman aşımı yolunu sınamak için) ve geri yüklemeyi başarısız sayar.
#[cfg(test)]
pub(crate) static TEST_STEP_WAIT_MS: AtomicU64 = AtomicU64::new(u64::MAX);
#[cfg(test)]
pub(crate) static TEST_FAIL_RESTORE: AtomicBool = AtomicBool::new(false);
/// Bu öneklerle başlayan iç komutlar çalıştırılır ama sonuçları "bilinmiyor" (zaman aşımı) sayılır.
#[cfg(test)]
pub(crate) static TEST_UNKNOWN_STEPS: std::sync::Mutex<Vec<&'static str>> = std::sync::Mutex::new(Vec::new());
/// Kancaları kullanan testler sırayla çalışır.
#[cfg(test)]
pub(crate) static TEST_SERIAL: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

fn step_wait() -> Duration {
    #[cfg(test)]
    {
        let ms = TEST_STEP_WAIT_MS.load(Ordering::SeqCst);
        if ms != u64::MAX {
            return Duration::from_millis(ms);
        }
    }
    STEP_WAIT
}

pub(crate) fn fail_restore() -> bool {
    #[cfg(test)]
    {
        if TEST_FAIL_RESTORE.load(Ordering::SeqCst) {
            return true;
        }
    }
    false
}

// ---------------------------------------------------------------------------------------------------------------
// Arka plan işlemi
// ---------------------------------------------------------------------------------------------------------------

/// Oturum kilidini tutan arka plan işleminin (tamamlama, denetim) bayrakları. Bayraklar yalnızca kilidi **alan**
/// arka plan işlemi tarafından kurulur (`begin`, kilit alındıktan hemen sonra) ve bekçisi kilit bırakılmadan önce
/// düşürülür; kilidi alamayan bir deneme bayraklara dokunmaz. Kullanıcı işlemi kilidi alamazsa `request_abort` ile
/// arka plan işleminden bırakmasını ister.
#[derive(Default)]
pub(crate) struct Background {
    active: AtomicBool,
    abort: AtomicBool,
    notify: Notify,
}

impl Background {
    /// Kilidi almış arka plan işlemini işaretler; bekçi düşünce biter. Bekçi kilitten önce düşmelidir.
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

/// Kısa iç komut: `step_wait()` içinde bitmezse iptal gönderilir ve sonucu `DRAIN_WAIT` beklenir. `None`: sonuç
/// bilinmiyor (komut sunucuda yine de çalışmış olabilir).
pub(crate) async fn settled<T, F: Future<Output = Result<T>>>(handle: &CancelHandle, fut: F) -> Option<Result<T>> {
    let mut fut = std::pin::pin!(fut);
    if let Ok(r) = tokio::time::timeout(step_wait(), &mut fut).await {
        return Some(r);
    }
    cancel_quietly(handle).await;
    tokio::time::timeout(DRAIN_WAIT, fut).await.ok()
}

async fn step(exec: &mut Executor, handle: &CancelHandle, sql: &str) -> Option<Result<TextRows>> {
    #[cfg(test)]
    {
        let unknown = TEST_UNKNOWN_STEPS.lock().unwrap().iter().any(|p| sql.starts_with(p));
        if unknown {
            let _ = settled(handle, exec.exec_internal(sql)).await;
            return None;
        }
    }
    settled(handle, exec.exec_internal(sql)).await
}

fn ok(r: &Option<Result<TextRows>>) -> bool {
    matches!(r, Some(Ok(_)))
}

// ---------------------------------------------------------------------------------------------------------------
// Korumalı Parse
// ---------------------------------------------------------------------------------------------------------------

#[derive(Debug)]
pub(crate) enum Parsed {
    /// Sonuç sütunları (ad, tür adı); satır döndürmeyen ifadede boş.
    Columns(Vec<(String, String)>),
    /// Sunucu Parse'ı reddetti (sözdizimi, bilinmeyen ad, kilit zaman aşımı…); konum: metin içinde 1 tabanlı
    /// karakter (Unicode scalar) konumu.
    Failed(Error, Option<u32>),
    /// Denenmedi (süre doldu, kullanıcı işlemi başladı, transaction hatalı, kurulum başarısız).
    Skipped,
}

#[derive(Debug, Default)]
pub(crate) struct ParseBatch {
    /// `sqls` ile aynı sırada ve uzunlukta.
    pub results: Vec<Parsed>,
    /// Savepoint yolu temizlenemedi ya da sonucu bilinmiyor: transaction durumu yoklanmalı.
    pub broken: bool,
    /// Doğrudan yol: `lock_timeout` 200 ms'ye çekilmiş olabilir ve geri yükleme doğrulanamadı; oturum bu değeri
    /// (kullanıcının önceki değeri) bir sonraki SQL'den önce uygulamalıdır.
    pub restore_lock_timeout: Option<String>,
}

/// `sqls`'in her birini korumalarla Parse eder (bkz. modül belgesi). Çağıran oturum kilidini tutar.
pub(crate) async fn guarded_parse(exec: &mut Executor, state: TxState, bg: &Background, sqls: &[&str]) -> ParseBatch {
    let deadline = Instant::now() + PARSE_BUDGET;
    let mut batch = ParseBatch { results: Vec::with_capacity(sqls.len()), ..ParseBatch::default() };
    if state != TxState::Failed && !sqls.is_empty() && !bg.aborted() {
        let handle = exec.cancel_handle();
        if state != TxState::Idle || exec.cursor_open() || exec.in_tx() {
            in_savepoint(exec, bg, &handle, deadline, sqls, &mut batch).await;
        } else {
            direct(exec, bg, &handle, deadline, sqls, &mut batch).await;
        }
    }
    batch.results.resize_with(sqls.len(), || Parsed::Skipped);
    batch
}

/// Transaction içinde: çağrıya özgü tek savepoint; başarısız Parse alt transaction'ı bozduğundan `ROLLBACK TO` ile
/// kurtarılır (savepoint kalır, `SET LOCAL` yenilenir). Sonda `ROLLBACK TO` (kilitler bırakılır) + `RELEASE`.
async fn in_savepoint(
    exec: &mut Executor,
    bg: &Background,
    handle: &CancelHandle,
    deadline: Instant,
    sqls: &[&str],
    batch: &mut ParseBatch,
) {
    let name = format!("pgt_parse_{}", SAVEPOINT_SEQ.fetch_add(1, Ordering::Relaxed));
    // SAVEPOINT başarısızsa (ör. geç bir iptal) kullanıcı transaction'ı bozulmuş olabilir; sonucu bilinmiyorsa
    // savepoint açık kalmış olabilir. Her iki durumda da temizlik denenmez (yanlış savepoint'e dönülmesin), yoklanır.
    if !ok(&step(exec, handle, &format!("SAVEPOINT {name}")).await) {
        batch.broken = true;
        return;
    }
    let rollback_to = format!("ROLLBACK TO SAVEPOINT {name}");
    let mut ready = ok(&step(exec, handle, LOCAL_LOCK_TIMEOUT).await);
    for sql in sqls {
        if !ready {
            break;
        }
        match bounded(bg, handle, deadline, exec.prepare_described(sql)).await {
            Some(Ok(cols)) => batch.results.push(Parsed::Columns(cols)),
            Some(Err((e, pos))) => {
                batch.results.push(Parsed::Failed(e, pos));
                ready = ok(&step(exec, handle, &rollback_to).await) && ok(&step(exec, handle, LOCAL_LOCK_TIMEOUT).await);
            }
            None => break,
        }
    }
    let cleanup = format!("{rollback_to}; RELEASE SAVEPOINT {name}");
    // Yalnızca hata alan temizlik yinelenir (geç iptal; ROLLBACK TO hatalı alt transaction'ı da kurtarır).
    // Sonucu bilinmeyen temizlik yinelenmez: çalışmış olabilir, savepoint gitmiş olabilir.
    let done = match step(exec, handle, &cleanup).await {
        Some(Ok(_)) => true,
        Some(Err(e)) => {
            tracing::debug!(code = %ErrorPayload::from(&e).code, "Parse temizliği başarısız, yineleniyor");
            ok(&step(exec, handle, &cleanup).await)
        }
        None => false,
    };
    batch.broken = !done;
}

/// Boşta: her Parse kendi örtük transaction'ında; `lock_timeout` geçici olarak 200 ms.
async fn direct(
    exec: &mut Executor,
    bg: &Background,
    handle: &CancelHandle,
    deadline: Instant,
    sqls: &[&str],
    batch: &mut ParseBatch,
) {
    // SHOW yan etkisizdir: başarısızsa hiçbir şey değişmemiştir.
    let previous = match step(exec, handle, SHOW_LOCK_TIMEOUT).await {
        Some(Ok(rows)) => match rows.into_iter().next().and_then(|r| r.into_iter().next().flatten()) {
            Some(v) => v,
            None => return,
        },
        _ => return,
    };
    match step(exec, handle, SET_LOCK_TIMEOUT).await {
        Some(Ok(_)) => {}
        Some(Err(_)) => return, // tek ifade başarısız: değer değişmedi
        None => {
            // Sonucu bilinmiyor: değer 200 ms olmuş olabilir.
            batch.restore_lock_timeout = Some(previous);
            return;
        }
    }
    for sql in sqls {
        match bounded(bg, handle, deadline, exec.prepare_described(sql)).await {
            Some(Ok(cols)) => batch.results.push(Parsed::Columns(cols)),
            Some(Err((e, pos))) => batch.results.push(Parsed::Failed(e, pos)),
            None => break,
        }
    }
    if !restore_lock_timeout(exec, handle, &previous).await {
        tracing::warn!("lock_timeout geri yüklenemedi; sonraki SQL'den önce yeniden denenecek");
        batch.restore_lock_timeout = Some(previous);
    }
}

/// `set_config` eşgüçlüdür: başarısızlıkta (sonucu bilinmese de) bir kez yinelenir.
async fn restore_lock_timeout(exec: &mut Executor, handle: &CancelHandle, previous: &str) -> bool {
    if fail_restore() {
        return false;
    }
    for _ in 0..2 {
        match settled(handle, exec.query_text(RESTORE_LOCK_TIMEOUT, &[&previous])).await {
            Some(Ok(_)) => return true,
            Some(Err(e)) => tracing::debug!(code = %ErrorPayload::from(&e).code, "lock_timeout geri yüklenemedi"),
            None => tracing::debug!("lock_timeout geri yüklemesinin sonucu bilinmiyor"),
        }
    }
    false
}

/// Korumaların zaman aşımı/bilinmeyen sonuç yollarını gerçek sunucuda sınar (`PGT_TEST_URLS` yoksa atlanır).
#[cfg(test)]
mod db_tests {
    use pgt_core::connection::{ConnectionManager, SessionProfile, Target};
    use pgt_core::testing::{TestServer, test_servers};

    use super::*;

    async fn executor(ts: &TestServer) -> (ConnectionManager, Executor) {
        let mgr = ConnectionManager::new();
        mgr.connect(ts.config.clone(), ts.password.clone()).await.unwrap();
        let profile = SessionProfile::Query { read_only: false, statement_timeout_ms: 0 };
        let conn = mgr.open_dedicated(ts.config.id, Target::Maintenance, profile).await.unwrap();
        (mgr, Executor::new(conn))
    }

    async fn cell(exec: &mut Executor, sql: &str) -> String {
        exec.exec_internal(sql).await.unwrap()[0][0].clone().unwrap()
    }

    struct Hooks;

    impl Hooks {
        fn unknown(prefixes: &[&'static str]) -> Hooks {
            *TEST_UNKNOWN_STEPS.lock().unwrap() = prefixes.to_vec();
            Hooks
        }
    }

    impl Drop for Hooks {
        fn drop(&mut self) {
            TEST_UNKNOWN_STEPS.lock().unwrap().clear();
            TEST_STEP_WAIT_MS.store(u64::MAX, Ordering::SeqCst);
            TEST_FAIL_RESTORE.store(false, Ordering::SeqCst);
        }
    }

    /// Süresi dolan komut düşürülmez: iptal edilir ve gerçek sonucu (57014) beklenir; bağlantı kullanılabilir kalır.
    #[tokio::test]
    async fn timed_out_step_is_cancelled_and_drained() {
        let _serial = TEST_SERIAL.lock().await;
        for ts in test_servers() {
            let (_mgr, mut exec) = executor(&ts).await;
            let _hooks = Hooks::unknown(&[]);
            TEST_STEP_WAIT_MS.store(50, Ordering::SeqCst);
            let handle = exec.cancel_handle();
            let r = settled(&handle, exec.exec_internal("select pg_sleep(5)")).await;
            assert!(matches!(&r, Some(Err(Error::Query { sqlstate: Some(c), .. })) if c == "57014"), "{r:?}");
            assert_eq!(cell(&mut exec, "select 1").await, "1");
        }
    }

    /// Sonucu bilinmeyen temizlik yinelenmez (yineleme savepoint gittiği için transaction'ı bozardı); kullanıcının
    /// `pgt_parse` adlı savepoint'ine dokunulmaz; durum "bozuk" bildirilir.
    #[tokio::test]
    async fn unknown_cleanup_is_not_retried_and_user_savepoint_survives() {
        let _serial = TEST_SERIAL.lock().await;
        for ts in test_servers() {
            let (_mgr, mut exec) = executor(&ts).await;
            exec.exec_internal("begin; savepoint pgt_parse; create temp table after_sp(i int)").await.unwrap();
            let _hooks = Hooks::unknown(&["ROLLBACK TO SAVEPOINT pgt_parse_"]);
            let bg = Background::default();
            let batch = guarded_parse(&mut exec, TxState::InTransaction, &bg, &["select 1 as a"]).await;
            assert!(batch.broken);
            assert!(matches!(&batch.results[..], [Parsed::Columns(c)] if c[0].0 == "a"), "{batch:?}");
            assert_eq!(cell(&mut exec, "select 1").await, "1", "transaction bozulmamalı");
            exec.exec_internal("rollback to savepoint pgt_parse").await.unwrap();
            let n = cell(&mut exec, "select count(*) from pg_class where relname = 'after_sp' and relpersistence = 't'").await;
            assert_eq!(n, "0", "kullanıcının savepoint'i yerinde");
            exec.exec_internal("rollback").await.unwrap();
        }
    }

    /// Başarısız SAVEPOINT "bozuk" sayılır (çağıran yoklar) ve hiçbir şey Parse edilmez.
    #[tokio::test]
    async fn savepoint_failure_is_broken() {
        let _serial = TEST_SERIAL.lock().await;
        for ts in test_servers() {
            let (_mgr, mut exec) = executor(&ts).await;
            let bg = Background::default();
            // Durum makinesi transaction sanıyor ama yok: SAVEPOINT 25P01 ile reddedilir.
            let batch = guarded_parse(&mut exec, TxState::InTransaction, &bg, &["select 1"]).await;
            assert!(batch.broken);
            assert!(matches!(&batch.results[..], [Parsed::Skipped]));
        }
    }

    /// Doğrudan yol: SET'in sonucu bilinmiyorsa Parse yapılmaz ve kullanıcı değeri geri yükleme için döner.
    #[tokio::test]
    async fn unknown_set_leaves_a_pending_restore() {
        let _serial = TEST_SERIAL.lock().await;
        for ts in test_servers() {
            let (_mgr, mut exec) = executor(&ts).await;
            exec.exec_internal("set lock_timeout = '3s'").await.unwrap();
            let _hooks = Hooks::unknown(&["SET lock_timeout"]);
            let bg = Background::default();
            let batch = guarded_parse(&mut exec, TxState::Idle, &bg, &["select 1"]).await;
            assert!(!batch.broken);
            assert!(matches!(&batch.results[..], [Parsed::Skipped]));
            assert_eq!(batch.restore_lock_timeout.as_deref(), Some("3s"));
            assert_eq!(cell(&mut exec, "show lock_timeout").await, "200ms", "SET sunucuda çalıştı");

            // Geri yükleme doğrulanamazsa da bekleyen değer döner.
            drop(_hooks);
            TEST_FAIL_RESTORE.store(true, Ordering::SeqCst);
            let _hooks = Hooks;
            exec.exec_internal("set lock_timeout = '3s'").await.unwrap();
            let batch = guarded_parse(&mut exec, TxState::Idle, &bg, &["select 1 as x"]).await;
            assert!(matches!(&batch.results[..], [Parsed::Columns(_)]));
            assert_eq!(batch.restore_lock_timeout.as_deref(), Some("3s"));
        }
    }
}
