//! Plan çalıştırıcısı: ayrı (havuzsuz) yazılabilir bağlantıda blok blok uygulama ve iptal haritası.
//!
//! - Ardışık `transactional` ifadeler tek `BEGIN … COMMIT` bloğunda çalışır (hata → `ROLLBACK`);
//!   `transactional = false` ifadeler (ör. `CREATE INDEX CONCURRENTLY`) tek başına, transaction dışında.
//! - İptal: `apply_id` → yuva (iptal bayrağı + `CancelHandle`). Std `Mutex` hiçbir `.await` boyunca tutulmaz;
//!   yuva `Registration` düşünce (başarı, hata, erken dönüş ya da panik) kaldırılır.
//! - SQL metni hiçbir hata iletisine/günlüğe girmez; hata yükü yalnızca PG'nin `message`/`detail`/`hint`'idir.

use std::collections::HashMap;
use std::sync::{Mutex as StdMutex, MutexGuard, PoisonError};

use pgt_core::connection::{CancelHandle, ConnectionManager, DedicatedConn, SessionProfile, Target};
use pgt_core::{Error, ErrorPayload, Result};
use tokio_postgres::Client;
use uuid::Uuid;

use crate::error_codes::APPLY_ID_IN_USE;
use crate::plan::Statement;

/// İptal edilen ifadenin SQLSTATE'i (`query_canceled`).
pub(crate) const CANCELED_SQLSTATE: &str = "57014";
/// Kilit beklerken `lock_timeout` doldu (`lock_not_available`).
pub(crate) const LOCK_TIMEOUT_SQLSTATE: &str = "55P03";
/// Geçersiz parametre değeri; sequence sınır/RESTART çakışmaları bu kodla gelir.
const INVALID_PARAMETER_SQLSTATE: &str = "22023";
const CANCELLED_MESSAGE: &str = "Uygulama iptal edildi.";
const LOCK_TIMEOUT_MESSAGE: &str = "Nesne kilidi beklenirken zaman aşımı (lock_timeout).";
const LOCK_TIMEOUT_HINT: &str = "Nesneyi kullanan oturumları kontrol edin ya da kilit zaman aşımını artırın.";
const BOUNDS_HINT: &str =
    "Sequence'in başlangıç/mevcut değeri yeni MINVALUE/MAXVALUE sınırlarının dışında; START/RESTART ya da sınır değerlerini gözden geçirin.";

/// Oturum ayarlarını açıkça kurar. `SessionProfile::Ddl` aynı değerleri `options` ile de gönderir, ancak
/// havuzlayıcılar başlangıç parametrelerini sessizce yok sayabilir (`lock_timeout` doğrulanamaz): uygulayıcı
/// hiçbir ifadeden önce bu dört ayarı kendisi `SET` eder.
pub(crate) async fn prepare_session(client: &Client, lock_timeout_ms: u32) -> Result<()> {
    let sets = format!(
        "SET lock_timeout = {lock_timeout_ms}; SET statement_timeout = 0; \
         SET default_transaction_read_only = off; SET standard_conforming_strings = on"
    );
    client.batch_execute(&sets).await.map_err(Error::from_query)
}

/// Uygulama bağlantısı: `Ddl` profiliyle ayrı bağlantı + açık `SET`'ler.
pub(crate) async fn open_apply_conn(mgr: &ConnectionManager, server: Uuid, database_oid: u32, lock_timeout_ms: u32) -> Result<DedicatedConn> {
    let conn = mgr.open_dedicated(server, Target::Database(database_oid), SessionProfile::Ddl { lock_timeout_ms }).await?;
    prepare_session(&conn.client, lock_timeout_ms).await?;
    Ok(conn)
}

#[derive(Default)]
struct Slot {
    handle: Option<CancelHandle>,
    cancelled: bool,
}

/// `apply_id` → iptal yuvası.
#[derive(Default)]
pub(crate) struct CancelMap {
    slots: StdMutex<HashMap<Uuid, Slot>>,
}

impl CancelMap {
    fn lock(&self) -> MutexGuard<'_, HashMap<Uuid, Slot>> {
        self.slots.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Yuva açar; aynı kimlikle süren uygulama varsa reddedilir (birinin düşmesi diğerinin yuvasını silmesin).
    pub(crate) fn register(&self, id: Uuid) -> Result<Registration<'_>> {
        let mut slots = self.lock();
        if slots.contains_key(&id) {
            return Err(Error::Invalid(APPLY_ID_IN_USE.into()));
        }
        slots.insert(id, Slot::default());
        Ok(Registration { map: self, id })
    }

    /// İptal bayrağını kurar; bağlantı açıldıysa tutamacı döndürür. Bilinmeyen kimlik → `None`.
    pub(crate) fn request(&self, id: Uuid) -> Option<Option<CancelHandle>> {
        let mut slots = self.lock();
        let slot = slots.get_mut(&id)?;
        slot.cancelled = true;
        Some(slot.handle.clone())
    }

    /// Tüm yuvalara iptal bayrağı kurar; açık bağlantıların tutamaçlarını döndürür.
    pub(crate) fn request_all(&self) -> Vec<CancelHandle> {
        let mut slots = self.lock();
        slots
            .values_mut()
            .filter_map(|s| {
                s.cancelled = true;
                s.handle.clone()
            })
            .collect()
    }

    #[cfg(test)]
    pub(crate) fn len(&self) -> usize {
        self.lock().len()
    }
}

/// Haritadaki yuvanın sahibi; düşünce yuva kaldırılır.
pub(crate) struct Registration<'a> {
    map: &'a CancelMap,
    id: Uuid,
}

impl Registration<'_> {
    pub(crate) fn set_handle(&self, handle: CancelHandle) {
        if let Some(slot) = self.map.lock().get_mut(&self.id) {
            slot.handle = Some(handle);
        }
    }

    pub(crate) fn cancelled(&self) -> bool {
        self.map.lock().get(&self.id).is_some_and(|s| s.cancelled)
    }
}

impl Drop for Registration<'_> {
    fn drop(&mut self) {
        self.map.lock().remove(&self.id);
    }
}

/// Çalıştırma sonucu (süre ve günlük çağıranda).
#[derive(Debug)]
pub(crate) enum RunResult {
    Done,
    Failed { failed_index: u32, committed: u32, error: Error },
}

pub(crate) fn cancelled_error() -> Error {
    Error::Query { sqlstate: Some(CANCELED_SQLSTATE.into()), message: CANCELLED_MESSAGE.into(), detail: None, hint: None }
}

fn idx(i: usize) -> u32 {
    u32::try_from(i).unwrap_or(u32::MAX)
}

/// Hata ifadenin çalışması sırasında geldiyse ve iptal istendiyse iptal hatasına çevirir.
fn statement_error(e: tokio_postgres::Error, reg: &Registration<'_>) -> Error {
    let e = Error::from_query(e);
    match &e {
        Error::Query { sqlstate: Some(s), .. } if s == CANCELED_SQLSTATE && reg.cancelled() => cancelled_error(),
        _ => e,
    }
}

/// Blok içi hatadan sonra transaction'ı geri alır; bağlantı kopmuşsa hata yok sayılır (sunucu zaten geri alır).
async fn rollback(client: &Client) {
    if let Err(e) = client.batch_execute("ROLLBACK").await {
        tracing::debug!(sqlstate = e.code().map(|c| c.code()), "ROLLBACK başarısız");
    }
}

/// İfadeleri blok blok çalıştırır. `committed`: hatadan önce kalıcılaşan ifade sayısı (önceki bloklar ve
/// transaction dışı ifadeler); hatalı blok tümüyle geri alınır.
pub(crate) async fn run_statements(client: &Client, statements: &[Statement], reg: &Registration<'_>) -> RunResult {
    let mut committed = 0u32;
    let mut i = 0;
    while i < statements.len() {
        if !statements[i].transactional {
            if reg.cancelled() {
                return RunResult::Failed { failed_index: idx(i), committed, error: cancelled_error() };
            }
            if let Err(e) = client.batch_execute(&statements[i].sql).await {
                return RunResult::Failed { failed_index: idx(i), committed, error: statement_error(e, reg) };
            }
            committed += 1;
            i += 1;
            continue;
        }
        let end = statements[i..].iter().position(|s| !s.transactional).map_or(statements.len(), |p| i + p);
        if reg.cancelled() {
            return RunResult::Failed { failed_index: idx(i), committed, error: cancelled_error() };
        }
        if let Err(e) = client.batch_execute("BEGIN").await {
            return RunResult::Failed { failed_index: idx(i), committed, error: statement_error(e, reg) };
        }
        for (k, s) in statements.iter().enumerate().take(end).skip(i) {
            if reg.cancelled() {
                rollback(client).await;
                return RunResult::Failed { failed_index: idx(k), committed, error: cancelled_error() };
            }
            if let Err(e) = client.batch_execute(&s.sql).await {
                let error = statement_error(e, reg);
                rollback(client).await;
                return RunResult::Failed { failed_index: idx(k), committed, error };
            }
        }
        // COMMIT hatası (ör. ertelenmiş kısıt): blok geri alınmıştır; hata bloğun son ifadesine yazılır.
        if let Err(e) = client.batch_execute("COMMIT").await {
            let error = statement_error(e, reg);
            rollback(client).await;
            return RunResult::Failed { failed_index: idx(end - 1), committed, error };
        }
        committed += idx(end - i);
        i = end;
    }
    RunResult::Done
}

/// SQLSTATE (varsa).
pub(crate) fn sqlstate(e: &Error) -> Option<&str> {
    match e {
        Error::Query { sqlstate, .. } | Error::Connect { sqlstate, .. } => sqlstate.as_deref(),
        _ => None,
    }
}

/// Başarısız ifadenin arayüz yükü. İptal ve kilit zaman aşımı kendi iletisini alır (çekirdeğin 57014 →
/// "statement_timeout" eşlemesi burada yanıltıcı olurdu: DDL oturumunda `statement_timeout = 0`); sequence
/// sınır çakışmasına (22023, MINVALUE/MAXVALUE) ipucu eklenir. SQL metni yüke girmez.
pub(crate) fn failure_payload(e: &Error) -> ErrorPayload {
    if let Error::Query { sqlstate: Some(code), message, detail, hint } = e {
        match code.as_str() {
            CANCELED_SQLSTATE => {
                return ErrorPayload { code: "query".into(), message: CANCELLED_MESSAGE.into(), detail: None, hint: None };
            }
            LOCK_TIMEOUT_SQLSTATE => {
                return ErrorPayload {
                    code: "query".into(),
                    message: LOCK_TIMEOUT_MESSAGE.into(),
                    detail: Some(message.clone()),
                    hint: Some(hint.clone().unwrap_or_else(|| LOCK_TIMEOUT_HINT.into())),
                };
            }
            INVALID_PARAMETER_SQLSTATE if hint.is_none() && (message.contains("MINVALUE") || message.contains("MAXVALUE")) => {
                return ErrorPayload {
                    code: "query".into(),
                    message: message.clone(),
                    detail: detail.clone(),
                    hint: Some(BOUNDS_HINT.into()),
                };
            }
            _ => {}
        }
    }
    ErrorPayload::from(e)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn q(code: &str, message: &str) -> Error {
        Error::Query { sqlstate: Some(code.into()), message: message.into(), detail: None, hint: None }
    }

    #[test]
    fn registration_is_removed_on_drop_and_ids_are_exclusive() {
        let map = CancelMap::default();
        let id = Uuid::new_v4();
        {
            let reg = map.register(id).unwrap();
            assert!(matches!(map.register(id), Err(Error::Invalid(m)) if m == APPLY_ID_IN_USE));
            assert!(!reg.cancelled());
            assert_eq!(map.request(id).map(|h| h.is_none()), Some(true));
            assert!(reg.cancelled());
            assert_eq!(map.len(), 1);
        }
        assert_eq!(map.len(), 0);
        assert!(map.request(id).is_none());
        // Panik sırasında da kaldırılır.
        let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _reg = map.register(id).unwrap();
            panic!("x");
        }));
        assert!(r.is_err());
        assert_eq!(map.len(), 0);
    }

    #[test]
    fn request_all_flags_every_slot() {
        let map = CancelMap::default();
        let (a, b) = (map.register(Uuid::new_v4()).unwrap(), map.register(Uuid::new_v4()).unwrap());
        assert!(map.request_all().is_empty());
        assert!(a.cancelled() && b.cancelled());
    }

    #[test]
    fn payload_mapping() {
        let p = failure_payload(&q("57014", "canceling statement due to user request"));
        assert_eq!((p.code.as_str(), p.message.as_str()), ("query", CANCELLED_MESSAGE));
        let p = failure_payload(&q("55P03", "canceling statement due to lock timeout"));
        assert_eq!(p.message, LOCK_TIMEOUT_MESSAGE);
        assert_eq!(p.detail.as_deref(), Some("canceling statement due to lock timeout"));
        assert!(p.hint.is_some());
        let p = failure_payload(&q("22023", "RESTART value (5) cannot be less than MINVALUE (10)"));
        assert_eq!(p.hint.as_deref(), Some(BOUNDS_HINT));
        assert!(p.message.starts_with("RESTART value"));
        let p = failure_payload(&q("22023", "invalid value for parameter"));
        assert_eq!(p.hint, None);
        let p = failure_payload(&q("42P07", "relation \"x\" already exists"));
        assert_eq!((p.code.as_str(), p.message.as_str()), ("query", "relation \"x\" already exists"));
        assert_eq!(sqlstate(&q("42P07", "")), Some("42P07"));
        assert_eq!(sqlstate(&Error::NotConnected), None);
    }
}
