//! Veri düzenleyici yöneticisi: kaynak yükleme, SELECT üretimi, önizleme, kapılı uygulama ve iptal (`DdlManager`
//! kalıbı).
//!
//! `apply` kapı sırası (Plan 16 Global Constraints "Yazma kapısı sırası"): (1) kayıtsız sunucu, (2) `read_only_lock`
//! → `locked`, (3) kaynak yükleme (havuzdaki katalog bağlantısı; `fingerprint` farkı → `plan_changed`; salt okunur
//! kaynak (4)'te `object` alan hatası olur), (4) plan doğrulama/üretimi (alan hataları → `invalid_def`; boş plan →
//! `Invalid(NO_CHANGES)`), (5) `expected_hash` → `plan_changed`, (6) onay: plan riski ≥ Destructive (DELETE) ya da
//! üretim sunucusu (boş olmayan her plan; Query Tool `production_write` ile aynı `ServerConfig::is_production`) →
//! `confirmed` yoksa `confirmation_required`, (7) `lock_timeout` sınırı, (8) yürütme: `apply_id` kaydı (kullanımdaysa
//! `Invalid(APPLY_ID_IN_USE)`), `SessionProfile::Data` ile **ayrı** uygulama bağlantısı, tek transaction.
//! Plan istemciden alınmaz; değişiklik listesinden her seferinde yeniden üretilir. `preview`/`load`/`select` kilit
//! kapısına takılmaz.
//!
//! Günlük yalnızca işlem (`data_load|data_select|data_preview|data_apply`), değişiklik/ifade sayısı, süre ve sonuç
//! kodunu içerir; değer, anahtar, sürüm, ad, SQL ve PG hata iletisi (ör. 23505 `DETAIL`'i satır değeri içerir) asla.

use std::sync::{Arc, Mutex, PoisonError, RwLock};
use std::time::Instant;

use pgt_core::connection::{ConnectionManager, DedicatedConn, SessionProfile, Target};
use pgt_core::registry::Registry;
use pgt_core::{Error, Result};
use uuid::Uuid;

use super::exec::{self, ExecResult};
use super::model::{DataApplyOptions, DataChangeSet, DataOutcome, DataSelect, DataSource, SelectRequest};
use super::plan::plan_changes;
use super::select::select_sql;
use super::source::load_source;
use crate::apply::{self, CancelMap};
use crate::error_codes::{CONFIRMATION_REQUIRED, INVALID_DEF, LOCKED_MESSAGE, NO_CHANGES, PLAN_CHANGED, ROW_CONFLICT_MESSAGE, ddl_payload};
use crate::load::ObjectRef;
use crate::manager::{CANCEL_RESEND_AFTER, CANCEL_RESENDS, CancelOnDrop, check_lock_timeout};
use crate::model::FieldError;
use crate::plan::{PlanView, Risk, Statement};

pub struct DataManager {
    mgr: Arc<ConnectionManager>,
    registry: Arc<RwLock<Registry>>,
    cancels: CancelMap,
    /// Son `apply`'ın çalışma zamanı: `cancel_all` Tokio bağlamı olmayan iş parçacığından çağrılabilir.
    rt: Mutex<Option<tokio::runtime::Handle>>,
}

/// Kayıttan okunan koruma bayrakları.
struct Guard {
    locked: bool,
    production: bool,
}

fn elapsed_ms(started: Instant) -> u64 {
    u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX)
}

/// Günlükteki sonuç kodu: sabit kod kümesi (`ErrorPayload.code`) ya da SQLSTATE; metin asla.
fn err_code(e: &Error) -> String {
    ddl_payload(e).code
}

impl DataManager {
    pub fn new(mgr: Arc<ConnectionManager>, registry: Arc<RwLock<Registry>>) -> Self {
        DataManager { mgr, registry, cancels: CancelMap::default(), rt: Mutex::new(None) }
    }

    fn guard(&self, server: Uuid) -> Result<Guard> {
        let registry = self.registry.read().unwrap_or_else(PoisonError::into_inner);
        let config = registry.get(server).ok_or(Error::UnknownServer(server))?;
        Ok(Guard { locked: config.read_only_lock, production: config.is_production() })
    }

    /// Kaynak tanımı havuzdaki katalog bağlantısında (`load_source`); `locked` registry'den.
    async fn source(&self, server: Uuid, db_oid: u32, object: ObjectRef, guard: &Guard) -> Result<DataSource> {
        let mut src = self
            .mgr
            .run(server, Target::Database(db_oid), move |c| async move {
                Ok(load_source(&c.client, object, c.version_num() / 10_000).await)
            })
            .await??;
        src.locked = guard.locked;
        Ok(src)
    }

    /// Kaynağı yükler ve istemcinin parmak iziyle karşılaştırır (fark → `plan_changed`).
    async fn current(&self, server: Uuid, db_oid: u32, object: ObjectRef, fingerprint: &str, guard: &Guard) -> Result<DataSource> {
        let src = self.source(server, db_oid, object, guard).await?;
        if src.fingerprint != fingerprint {
            return Err(Error::Invalid(PLAN_CHANGED.into()));
        }
        Ok(src)
    }

    /// Kaynak tanımı (katalog havuzu bağlantısı); `locked` registry'den.
    pub async fn load(&self, server: Uuid, db_oid: u32, object: ObjectRef) -> Result<DataSource> {
        let started = Instant::now();
        let r = async { self.source(server, db_oid, object, &self.guard(server)?).await }.await;
        let outcome = r.as_ref().map_or_else(err_code, |_| "ok".into());
        tracing::debug!(op = "data_load", elapsed_ms = elapsed_ms(started), outcome = outcome.as_str(), "veri kaynağı");
        r
    }

    /// SELECT üretimi: kaynağı yükler, parmak izini denetler; doğrulama hataları iç `Err`'de.
    pub async fn select(
        &self,
        server: Uuid,
        db_oid: u32,
        object: ObjectRef,
        fingerprint: &str,
        req: &SelectRequest,
    ) -> Result<std::result::Result<DataSelect, Vec<FieldError>>> {
        let started = Instant::now();
        let r = async {
            let src = self.current(server, db_oid, object, fingerprint, &self.guard(server)?).await?;
            Ok(select_sql(&src, req))
        }
        .await;
        let outcome = match &r {
            Ok(Ok(_)) => "ok".to_string(),
            Ok(Err(_)) => "invalid_filter".to_string(),
            Err(e) => err_code(e),
        };
        tracing::debug!(op = "data_select", elapsed_ms = elapsed_ms(started), outcome = outcome.as_str(), "veri SELECT'i");
        r
    }

    /// Önizleme: yükler, parmak izini denetler, `plan_changes` → `PlanView` (doğrulama hataları `errors`'ta).
    /// Kilit kapısı ve yazma yok.
    pub async fn preview(&self, server: Uuid, db_oid: u32, set: &DataChangeSet) -> Result<PlanView> {
        let started = Instant::now();
        let r = async {
            let src = self.current(server, db_oid, set.object, &set.fingerprint, &self.guard(server)?).await?;
            Ok(PlanView::from_build(plan_changes(&src, set)))
        }
        .await;
        let (statements, errors, outcome) = match &r {
            Ok(v) => (v.statements.len(), v.errors.len(), if v.errors.is_empty() { "ok".into() } else { "invalid_def".into() }),
            Err(e) => (0, 0, err_code(e)),
        };
        tracing::debug!(
            op = "data_preview",
            changes = set.changes.len(),
            statements,
            errors,
            elapsed_ms = elapsed_ms(started),
            outcome = outcome.as_str(),
            "veri önizlemesi"
        );
        r
    }

    /// Kapılardan geçen planı ayrı bağlantıda tek transaction'da uygular. Kapı reddi ve bağlantı kurulamaması `Err`;
    /// ifade hatası, çakışma ve iptal `Ok(DataOutcome::Failed)` (hiçbir ifade kalıcı değildir; tek istisna
    /// SQLSTATE'siz `COMMIT` hatası → `outcome_unknown = true`, kalıcılaşmış olabilir).
    pub async fn apply(&self, server: Uuid, db_oid: u32, set: &DataChangeSet, opts: &DataApplyOptions) -> Result<DataOutcome> {
        let started = Instant::now();
        let mut statements = 0usize;
        let r = self.apply_gated(server, db_oid, set, opts, &mut statements).await;
        let outcome = match &r {
            Ok(DataOutcome::Applied { .. }) => "applied".to_string(),
            Ok(DataOutcome::Failed { conflict: true, .. }) => "conflict".to_string(),
            Ok(DataOutcome::Failed { sqlstate, .. }) => sqlstate.clone().unwrap_or_else(|| "error".into()),
            Err(e) => err_code(e),
        };
        tracing::info!(
            op = "data_apply",
            changes = set.changes.len(),
            statements,
            elapsed_ms = elapsed_ms(started),
            outcome = outcome.as_str(),
            "veri uygulaması"
        );
        r
    }

    async fn apply_gated(
        &self,
        server: Uuid,
        db_oid: u32,
        set: &DataChangeSet,
        opts: &DataApplyOptions,
        statements: &mut usize,
    ) -> Result<DataOutcome> {
        // (1) + (2)
        let guard = self.guard(server)?;
        if guard.locked {
            return Err(Error::Invalid(LOCKED_MESSAGE.into()));
        }
        // (3)
        let src = self.current(server, db_oid, set.object, &set.fingerprint, &guard).await?;
        // (4)
        let view = PlanView::from_build(plan_changes(&src, set));
        if !view.errors.is_empty() {
            return Err(Error::Invalid(INVALID_DEF.into()));
        }
        if view.empty {
            return Err(Error::Invalid(NO_CHANGES.into()));
        }
        *statements = view.statements.len();
        // (5)
        if view.hash != opts.expected_hash {
            return Err(Error::Invalid(PLAN_CHANGED.into()));
        }
        // (6)
        if (view.risk >= Risk::Destructive || guard.production) && !opts.confirmed {
            return Err(Error::Invalid(CONFIRMATION_REQUIRED.into()));
        }
        // (7)
        check_lock_timeout(opts.lock_timeout_ms)?;
        // (8)
        self.execute(server, db_oid, &view.statements, opts).await
    }

    /// Kayıt + bağlantı + yürütme. Bağlantı her uygulamada yeniden açılır (Query Tool oturumu paylaşılmaz).
    async fn execute(&self, server: Uuid, db_oid: u32, statements: &[Statement], opts: &DataApplyOptions) -> Result<DataOutcome> {
        let rt = tokio::runtime::Handle::current();
        *self.rt.lock().unwrap_or_else(PoisonError::into_inner) = Some(rt.clone());
        let started = Instant::now();
        // Yuva bağlantıdan önce açılır: bağlantı kurulurken gelen iptal de dikkate alınır.
        let reg = self.cancels.register(opts.apply_id)?;
        // Kayıttan önce gelen iptal (mezar taşı): bağlantı açmadan, hiçbir ifade çalışmadan iptal sonucu.
        if reg.cancelled() {
            return Ok(failed_outcome(0, apply::cancelled_error(), None, false));
        }
        let profile = SessionProfile::Data { lock_timeout_ms: opts.lock_timeout_ms };
        let conn = apply::open_apply_conn_as(&self.mgr, server, Target::Database(db_oid), profile).await?;
        // `notices` alıcısı hemen düşer (pgt-dash kalıbı): okunmayan kanal uygulama boyunca büyümesin; sürücü
        // alıcısız kanala gönderim hatasını yok sayar. Veri düzenleyici sunucu iletisi göstermez.
        let DedicatedConn { client, notices, cancel, .. } = conn;
        drop(notices);
        reg.set_handle(cancel.clone());
        // Çağıran gelecek çalışırken düşerse sunucudaki ifade iptal edilir.
        let mut on_drop = CancelOnDrop { handle: cancel, rt, armed: true };
        let result = exec::run_changes(&client, statements, &reg).await;
        on_drop.armed = false;
        drop(reg);
        drop(client);
        Ok(match result {
            ExecResult::Done(results) => DataOutcome::Applied {
                statements: apply::idx(statements.len()),
                elapsed_ms: elapsed_ms(started),
                results,
            },
            ExecResult::Conflict { index } => conflict_outcome(index),
            ExecResult::Failed { index, error, position, outcome_unknown } => failed_outcome(index, error, position, outcome_unknown),
        })
    }

    /// Süren uygulamayı iptal eder; bilinmeyen/bitmiş kimlik → `Ok` (idempotent; kayıttan önce gelen iptal mezar
    /// taşı bırakır). İstek ifade başlamadan önce ulaşıp yutulmuş olabilir: yuva duruyorsa en çok 3 kez yeniden gönderilir.
    pub async fn cancel(&self, apply_id: Uuid) -> Result<()> {
        let Some(Some(handle)) = self.cancels.request(apply_id) else { return Ok(()) };
        let first = handle.cancel().await;
        for _ in 0..CANCEL_RESENDS {
            tokio::time::sleep(CANCEL_RESEND_AFTER).await;
            match self.cancels.pending_cancel(apply_id) {
                Some(h) => {
                    let _ = h.cancel().await;
                }
                None => break,
            }
        }
        first
    }

    /// Tüm süren uygulamaları iptal eder (uygulama kapanışı/yeniden yükleme); Tokio bağlamı yoksa son `apply`'ın
    /// çalışma zamanı kullanılır. Gönderilmek üzere sıraya alınan iptal sayısını döndürür.
    pub fn cancel_all(&self) -> usize {
        let handles = self.cancels.request_all();
        if handles.is_empty() {
            return 0;
        }
        let rt = tokio::runtime::Handle::try_current()
            .ok()
            .or_else(|| self.rt.lock().unwrap_or_else(PoisonError::into_inner).clone());
        let Some(rt) = rt else { return 0 };
        let n = handles.len();
        for h in handles {
            rt.spawn(async move {
                if h.cancel().await.is_err() {
                    tracing::debug!(op = "data_cancel", "veri iptal isteği gönderilemedi");
                }
            });
        }
        n
    }
}

/// Çakışma (`index`. ifade ≠ 1 satır): geri alındı, sonuç bellidir.
fn conflict_outcome(index: u32) -> DataOutcome {
    DataOutcome::Failed {
        failed_index: index,
        sqlstate: None,
        position: None,
        error: ddl_payload(&Error::Invalid(ROW_CONFLICT_MESSAGE.into())),
        conflict: true,
        outcome_unknown: false,
    }
}

/// PG hatası/iptal → `Failed` (çakışma değil). Yük `failure_payload` (PG iletisi yalnızca kullanıcıya); `COMMIT`
/// yanıtsızsa (`outcome_unknown`: yalnızca SQLSTATE'siz COMMIT hatası) belirsiz sonuç ipucu ve bayrağı.
fn failed_outcome(index: u32, error: Error, position: Option<u32>, outcome_unknown: bool) -> DataOutcome {
    let payload = if outcome_unknown { apply::unknown_outcome_payload(&error) } else { apply::failure_payload(&error) };
    DataOutcome::Failed {
        failed_index: index,
        sqlstate: apply::sqlstate(&error).map(str::to_string),
        position,
        error: payload,
        conflict: false,
        outcome_unknown,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn query(sqlstate: Option<&str>) -> Error {
        Error::Query { sqlstate: sqlstate.map(str::to_string), message: "m".into(), detail: None, hint: None }
    }

    #[test]
    fn failed_outcome_flags_only_unknown_commit_outcome() {
        // SQLSTATE'siz COMMIT hatası: kalıcılaşmış olabilir → `outcome_unknown` + belirsizlik ipucu.
        let DataOutcome::Failed { failed_index, sqlstate, conflict, outcome_unknown, error, .. } =
            failed_outcome(2, query(None), None, true)
        else {
            panic!("Failed bekleniyordu")
        };
        assert_eq!((failed_index, sqlstate, conflict, outcome_unknown), (2, None, false, true));
        assert_eq!(error.hint, apply::unknown_outcome_payload(&query(None)).hint);

        // Sıradan ifade hatası ve iptal: sonuç belli (hiçbir şey yazılmadı).
        for (e, pos) in [(query(Some("23505")), Some(4)), (apply::cancelled_error(), None)] {
            let DataOutcome::Failed { outcome_unknown, conflict, position, .. } = failed_outcome(0, e, pos, false) else {
                panic!("Failed bekleniyordu")
            };
            assert_eq!((outcome_unknown, conflict, position), (false, false, pos));
        }
    }

    #[test]
    fn conflict_outcome_is_known_and_coded() {
        let DataOutcome::Failed { failed_index, sqlstate, position, error, conflict, outcome_unknown } = conflict_outcome(3)
        else {
            panic!("Failed bekleniyordu")
        };
        assert_eq!((failed_index, sqlstate, position, conflict, outcome_unknown), (3, None, None, true, false));
        assert_eq!(error.code, "row_conflict");
    }
}
