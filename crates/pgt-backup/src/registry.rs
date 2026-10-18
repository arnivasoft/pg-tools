//! Çalışan işlerin kaydı (spec §1 "İptal"): iş başına iptal belirteci, aynı hedefe ikinci işin reddi ve
//! uygulama kapanırken/sayfa yenilenirken tüm işlerin iptali.

use std::collections::HashMap;
use std::fmt;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError};

use tokio::sync::Notify;
use uuid::Uuid;

use crate::args::PARTIAL_SUFFIX;

/// Klonlanabilir iptal belirteci. İptal kalıcıdır: sonradan bekleyen de hemen döner.
#[derive(Debug, Clone, Default)]
pub struct CancelToken(Arc<TokenInner>);

#[derive(Debug, Default)]
struct TokenInner {
    flag: AtomicBool,
    notify: Notify,
}

impl CancelToken {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn cancel(&self) {
        self.0.flag.store(true, Ordering::SeqCst);
        self.0.notify.notify_waiters();
    }

    pub fn is_cancelled(&self) -> bool {
        self.0.flag.load(Ordering::SeqCst)
    }

    /// İptal edilene kadar bekler (iptal edilmişse hemen döner).
    pub async fn cancelled(&self) {
        loop {
            let notified = self.0.notify.notified();
            let mut notified = std::pin::pin!(notified);
            // `enable` bayrak denetiminden önce: arada gelen `notify_waiters` kaçmaz.
            notified.as_mut().enable();
            if self.is_cancelled() {
                return;
            }
            notified.await;
        }
    }
}

/// Kayıt hatası.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RegisterError {
    /// Aynı hedefe (ya da onun geçici çıktısına) yazan bir iş sürüyor.
    SameTarget,
    /// Bu iş kimliği zaten kayıtlı.
    DuplicateId,
}

impl fmt::Display for RegisterError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            RegisterError::SameTarget => "Aynı hedefe yazan bir iş zaten çalışıyor.",
            RegisterError::DuplicateId => "Bu iş kimliği zaten kullanılıyor.",
        })
    }
}

impl std::error::Error for RegisterError {}

struct Entry {
    token: CancelToken,
    /// Hedef ve geçici çıktı yolunun karşılaştırma anahtarları.
    keys: Vec<String>,
}

type Jobs = Mutex<HashMap<Uuid, Entry>>;

/// Çalışan işler. Klonlar aynı kaydı paylaşır.
#[derive(Clone, Default)]
pub struct JobRegistry(Arc<Jobs>);

/// Yol karşılaştırma anahtarı: ayraçlar birleştirilir, sondaki ayraç atılır; Windows'ta büyük/küçük harf
/// duyarsızdır.
fn path_key(path: &str) -> String {
    let unified: String = path.trim().chars().map(|c| if c == '/' { '\\' } else { c }).collect();
    let trimmed = unified.trim_end_matches('\\');
    let base = if trimmed.is_empty() { unified.as_str() } else { trimmed };
    if cfg!(windows) { base.to_lowercase() } else { base.to_string() }
}

impl JobRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    fn jobs(&self) -> std::sync::MutexGuard<'_, HashMap<Uuid, Entry>> {
        self.0.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// İşi kaydeder. `output_path` (yedeklemenin hedefi; geri yüklemede `None`) çalışan bir işin hedefiyle ya da
    /// geçici çıktısıyla çakışırsa reddedilir. `JobGuard` düşünce kayıt silinir.
    pub fn register(&self, job_id: Uuid, output_path: Option<&str>) -> Result<(CancelToken, JobGuard), RegisterError> {
        let keys: Vec<String> = output_path
            .map(|p| vec![path_key(p), path_key(&format!("{}{PARTIAL_SUFFIX}", path_key(p)))])
            .unwrap_or_default();
        let mut jobs = self.jobs();
        if jobs.contains_key(&job_id) {
            return Err(RegisterError::DuplicateId);
        }
        if jobs.values().any(|e| e.keys.iter().any(|k| keys.contains(k))) {
            return Err(RegisterError::SameTarget);
        }
        let token = CancelToken::new();
        jobs.insert(job_id, Entry { token: token.clone(), keys });
        Ok((token, JobGuard { jobs: Arc::clone(&self.0), job_id }))
    }

    /// İşi iptal eder; iş bulunduysa `true`.
    pub fn cancel(&self, job_id: Uuid) -> bool {
        match self.jobs().get(&job_id) {
            Some(e) => {
                e.token.cancel();
                true
            }
            None => false,
        }
    }

    /// Tüm işleri iptal eder; iptal edilen iş sayısını döner. Kayıtlar işler bitince (guard düşünce) silinir.
    pub fn cancel_all(&self) -> usize {
        let jobs = self.jobs();
        for e in jobs.values() {
            e.token.cancel();
        }
        jobs.len()
    }

    /// Kayıtlı iş sayısı.
    pub fn len(&self) -> usize {
        self.jobs().len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// Düşünce işi kayıttan siler.
pub struct JobGuard {
    jobs: Arc<Jobs>,
    job_id: Uuid,
}

impl Drop for JobGuard {
    fn drop(&mut self) {
        self.jobs.lock().unwrap_or_else(PoisonError::into_inner).remove(&self.job_id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn second_job_to_same_target_is_rejected() {
        let reg = JobRegistry::new();
        let (a, b) = (Uuid::new_v4(), Uuid::new_v4());
        let (_t, guard) = reg.register(a, Some("C:/yedek/a.dump")).unwrap();
        // Ayraç, sondaki ayraç ve (Windows'ta) harf farkı aynı hedeftir.
        let same = if cfg!(windows) { "c:\\YEDEK\\a.dump\\" } else { "C:\\yedek\\a.dump\\" };
        assert_eq!(reg.register(b, Some(same)).err(), Some(RegisterError::SameTarget));
        // Başka bir işin geçici çıktısına yazmak da çakışmadır.
        assert_eq!(reg.register(b, Some("C:/yedek/a.dump.pgt-partial")).err(), Some(RegisterError::SameTarget));
        assert_eq!(reg.register(a, Some("C:/yedek/b.dump")).err(), Some(RegisterError::DuplicateId));
        // Çıktısız işler (geri yükleme) birbirini engellemez.
        let (_r1, _g1) = reg.register(Uuid::new_v4(), None).unwrap();
        let (_r2, _g2) = reg.register(Uuid::new_v4(), None).unwrap();
        drop(guard);
        assert!(reg.register(b, Some(same)).is_ok());
        assert!(!RegisterError::SameTarget.to_string().is_empty());
    }

    #[test]
    fn cancel_and_cancel_all() {
        let reg = JobRegistry::new();
        let (a, b, c) = (Uuid::new_v4(), Uuid::new_v4(), Uuid::new_v4());
        let (ta, _ga) = reg.register(a, Some("/x/a")).unwrap();
        let (tb, _gb) = reg.register(b, Some("/x/b")).unwrap();
        let (tc, gc) = reg.register(c, None).unwrap();
        assert!(reg.cancel(a));
        assert!(ta.is_cancelled() && !tb.is_cancelled());
        assert!(!reg.cancel(Uuid::new_v4()));
        drop(gc);
        assert_eq!(reg.len(), 2);
        assert_eq!(reg.cancel_all(), 2);
        assert!(tb.is_cancelled());
        // Kaydı silinmiş işin belirteci etkilenmez.
        assert!(!tc.is_cancelled());
    }

    #[tokio::test]
    async fn token_wakes_waiters_and_is_sticky() {
        let t = CancelToken::new();
        let waiter = tokio::spawn({
            let t = t.clone();
            async move { t.cancelled().await }
        });
        tokio::task::yield_now().await;
        t.cancel();
        tokio::time::timeout(std::time::Duration::from_secs(5), waiter).await.unwrap().unwrap();
        // İptal sonrası bekleme hemen döner.
        tokio::time::timeout(std::time::Duration::from_secs(1), t.cancelled()).await.unwrap();
    }
}
