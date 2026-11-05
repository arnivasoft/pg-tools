//! Karşılaştırma kayıt defteri: istemcinin ürettiği `diffId` ile çalışan (iptal belirteci) ve hazır
//! (sonuç verisi) kayıtlar. Hazır kayıtlar `MAX_READY` ile sınırlıdır; en eskisi düşer.

use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use uuid::Uuid;

use crate::cancel::CancelToken;
use crate::run::DiffData;

/// Bellekte tutulan en çok hazır karşılaştırma sayısı.
pub const MAX_READY: usize = 8;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RegisterError {
    /// Aynı kimlikle çalışan ya da hazır kayıt var.
    DuplicateId,
}

enum Slot {
    /// `seq`: kaydı üreten `register` çağrısı (eski bir `RunGuard` yeni kaydı silmesin).
    Running { token: CancelToken, seq: u64 },
    Ready(Arc<DiffData>),
}

#[derive(Default)]
struct Inner {
    slots: HashMap<Uuid, Slot>,
    /// Hazır kayıtların tamamlanma sırası (en eski başta).
    ready_order: VecDeque<Uuid>,
    next_seq: u64,
}

impl Inner {
    fn remove(&mut self, id: Uuid) -> Option<Slot> {
        let slot = self.slots.remove(&id)?;
        if matches!(slot, Slot::Ready(_)) {
            self.ready_order.retain(|x| *x != id);
        }
        Some(slot)
    }
}

#[derive(Clone, Default)]
pub struct DiffRegistry(Arc<Mutex<Inner>>);

/// Düşünce, tamamlanmamış (hâlâ "çalışıyor") kaydı siler; hata ve iptal yollarını kapsar.
pub struct RunGuard {
    registry: DiffRegistry,
    id: Uuid,
    seq: u64,
}

impl Drop for RunGuard {
    fn drop(&mut self) {
        let mut g = self.registry.lock();
        if matches!(g.slots.get(&self.id), Some(Slot::Running { seq, .. }) if *seq == self.seq) {
            g.remove(self.id);
        }
    }
}

impl RunGuard {
    /// Kayıt hâlâ bu koşunun "çalışıyor" kaydıysa hazıra geçer; kapatılmış ya da yeniden kaydedilmişse
    /// veri düşer. En eski hazır kayıtlar `MAX_READY`'ye kırpılır.
    pub fn complete(&self, data: DiffData) {
        let mut g = self.registry.lock();
        if !matches!(g.slots.get(&self.id), Some(Slot::Running { seq, .. }) if *seq == self.seq) {
            return;
        }
        g.slots.insert(self.id, Slot::Ready(Arc::new(data)));
        g.ready_order.push_back(self.id);
        while g.ready_order.len() > MAX_READY {
            if let Some(old) = g.ready_order.pop_front() {
                g.slots.remove(&old);
            }
        }
    }
}

impl DiffRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    fn lock(&self) -> MutexGuard<'_, Inner> {
        self.0.lock().unwrap_or_else(PoisonError::into_inner)
    }

    pub fn register(&self, id: Uuid) -> Result<(CancelToken, RunGuard), RegisterError> {
        let mut g = self.lock();
        if g.slots.contains_key(&id) {
            return Err(RegisterError::DuplicateId);
        }
        let seq = g.next_seq;
        g.next_seq += 1;
        let token = CancelToken::new();
        g.slots.insert(
            id,
            Slot::Running {
                token: token.clone(),
                seq,
            },
        );
        drop(g);
        Ok((
            token,
            RunGuard {
                registry: self.clone(),
                id,
                seq,
            },
        ))
    }

    pub fn get(&self, id: Uuid) -> Option<Arc<DiffData>> {
        match self.lock().slots.get(&id)? {
            Slot::Ready(d) => Some(d.clone()),
            Slot::Running { .. } => None,
        }
    }

    /// Çalışan kaydı iptal eder (kayıt kalır; koruyucu düşünce silinir). Hazır kayda dokunmaz.
    pub fn cancel(&self, id: Uuid) {
        if let Some(Slot::Running { token, .. }) = self.lock().slots.get(&id) {
            token.cancel();
        }
    }

    /// İptal + kaldır.
    pub fn close(&self, id: Uuid) {
        if let Some(Slot::Running { token, .. }) = self.lock().remove(id) {
            token.cancel();
        }
    }

    /// Hepsini iptal eder ve kaldırır; kaldırılan kayıt sayısını döndürür.
    pub fn clear(&self) -> usize {
        let mut g = self.lock();
        let n = g.slots.len();
        for slot in g.slots.values() {
            if let Slot::Running { token, .. } = slot {
                token.cancel();
            }
        }
        g.slots.clear();
        g.ready_order.clear();
        n
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn data() -> DiffData {
        DiffData::empty_for_test()
    }

    #[test]
    fn duplicate_id_rejected() {
        let r = DiffRegistry::new();
        let id = Uuid::new_v4();
        let _a = r.register(id).unwrap();
        assert_eq!(r.register(id).err(), Some(RegisterError::DuplicateId));
    }

    #[test]
    fn complete_after_close_drops_data() {
        let r = DiffRegistry::new();
        let id = Uuid::new_v4();
        let (tok, g) = r.register(id).unwrap();
        r.close(id);
        assert!(tok.is_cancelled());
        g.complete(data());
        assert!(r.get(id).is_none());
    }

    #[test]
    fn complete_makes_ready_and_survives_guard_drop() {
        let r = DiffRegistry::new();
        let id = Uuid::new_v4();
        let (_t, g) = r.register(id).unwrap();
        assert!(r.get(id).is_none(), "çalışan kayıt hazır değil");
        g.complete(data());
        drop(g);
        assert!(r.get(id).is_some());
        assert_eq!(r.register(id).err(), Some(RegisterError::DuplicateId));
        r.close(id);
        assert!(r.get(id).is_none());
    }

    #[test]
    fn ninth_ready_evicts_oldest() {
        let r = DiffRegistry::new();
        let ids: Vec<Uuid> = (0..MAX_READY + 1).map(|_| Uuid::new_v4()).collect();
        for id in &ids {
            let (_t, g) = r.register(*id).unwrap();
            g.complete(data());
            drop(g);
        }
        assert!(r.get(ids[0]).is_none());
        assert_eq!(
            ids[1..].iter().filter(|i| r.get(**i).is_some()).count(),
            MAX_READY
        );
        // Düşen kimlik yeniden kaydedilebilir.
        assert!(r.register(ids[0]).is_ok());
    }

    #[test]
    fn clear_cancels_running_and_counts() {
        let r = DiffRegistry::new();
        let (a, g1) = r.register(Uuid::new_v4()).unwrap();
        let (b, g2) = r.register(Uuid::new_v4()).unwrap();
        let done = Uuid::new_v4();
        let (_t, g3) = r.register(done).unwrap();
        g3.complete(data());
        assert_eq!(r.clear(), 3);
        assert!(a.is_cancelled() && b.is_cancelled());
        assert!(r.get(done).is_none());
        drop((g1, g2, g3));
        assert_eq!(r.clear(), 0);
    }

    #[test]
    fn guard_drop_removes_unfinished_only() {
        let r = DiffRegistry::new();
        let id = Uuid::new_v4();
        let (_t, g) = r.register(id).unwrap();
        drop(g);
        assert!(r.register(id).is_ok(), "tamamlanmayan kayıt silinmiş olmalı");
    }

    #[test]
    fn stale_guard_does_not_remove_new_registration() {
        let r = DiffRegistry::new();
        let id = Uuid::new_v4();
        let (_t, old) = r.register(id).unwrap();
        r.close(id);
        let (_t2, _new) = r.register(id).unwrap();
        drop(old);
        assert_eq!(r.register(id).err(), Some(RegisterError::DuplicateId));
    }

    #[test]
    fn stale_complete_does_not_promote_new_registration() {
        let r = DiffRegistry::new();
        let id = Uuid::new_v4();
        let (_ta, a) = r.register(id).unwrap();
        r.close(id);
        let (tb, b) = r.register(id).unwrap();
        a.complete(data());
        assert!(r.get(id).is_none(), "eski koşunun verisi düşmeli");
        assert!(!tb.is_cancelled());
        r.cancel(id);
        assert!(tb.is_cancelled(), "yeni kaydın belirteci korunmuş olmalı");
        b.complete(data());
        assert!(r.get(id).is_some());
    }

    #[test]
    fn cancel_keeps_slot_until_guard_drops() {
        let r = DiffRegistry::new();
        let id = Uuid::new_v4();
        let (t, g) = r.register(id).unwrap();
        r.cancel(id);
        assert!(t.is_cancelled());
        drop(g);
        assert!(r.register(id).is_ok());
    }
}
