//! Klonlanabilir iptal belirteci (`pgt-backup`'taki ile aynı davranış; ona bağımlılık yok).

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use tokio::sync::Notify;

/// İptal kalıcıdır: sonradan bekleyen de hemen döner.
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[tokio::test]
    async fn cancel_is_sticky() {
        let t = CancelToken::new();
        assert!(!t.is_cancelled());
        t.cancel();
        assert!(t.is_cancelled());
        // Sonradan bekleyen hemen döner.
        tokio::time::timeout(Duration::from_millis(200), t.cancelled())
            .await
            .expect("hemen dönmeli");
        assert!(t.clone().is_cancelled(), "klon aynı bayrağı paylaşır");
    }

    #[tokio::test]
    async fn waiter_wakes_on_cancel() {
        let t = CancelToken::new();
        let t2 = t.clone();
        let h = tokio::spawn(async move { t2.cancelled().await });
        tokio::time::sleep(Duration::from_millis(20)).await;
        t.cancel();
        tokio::time::timeout(Duration::from_secs(2), h)
            .await
            .expect("uyanmalı")
            .unwrap();
    }
}
