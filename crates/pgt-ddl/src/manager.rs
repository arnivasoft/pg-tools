//! DDL yöneticisi: önizleme, kapılı uygulama (kilit/geçerlilik/hash/onay), iptal, yükleme, varsayılanlar ve açılır listeler.
//!
//! Kapı sırası (`apply`, hiçbiri bağlantı açmaz): (1) kayıtsız sunucu, (2) `read_only_lock`, (3) plan üretimi
//! (geçersiz tanım / boş plan), (4) hash uyuşmazlığı, (5) yıkıcı plan onayı, (6) üretim sunucusunda veri kaybı
//! onayı, (7) `lock_timeout` sınırı. Plan istemciden alınmaz; her seferinde tanımdan yeniden üretilir.
//!
//! Günlük yalnızca işlem/nesne türü, ifade sayısı, süre ve sonuç kodunu içerir; SQL, ad ve değer asla.

use std::sync::{Arc, PoisonError, RwLock};
use std::time::Instant;

use pgt_core::connection::{ConnectionManager, MAX_STATEMENT_TIMEOUT_MS, Target};
use pgt_core::registry::Registry;
use pgt_core::{Error, ErrorPayload, Result};
use serde::{Deserialize, Serialize};
use ts_rs::TS;
use uuid::Uuid;

use crate::apply::{self, CancelMap, RunResult};
use crate::build::{self, BuildCtx};
use crate::error_codes::{
    CONFIRMATION_REQUIRED, INVALID_DEF, LOCKED_MESSAGE, NO_CHANGES, PLAN_CHANGED, PRODUCTION_CONFIRMATION_REQUIRED,
};
use crate::load::{self, ObjectRef};
use crate::lookup::{self, LookupItem, LookupRequest};
use crate::model::*;
use crate::plan::{PlanView, Risk, Statement};

/// Arayüzün istediği değişiklik; plan arka uçta bu istekten üretilir.
// Tek seferlik IPC yükü (saklanmaz); varyant boyut farkı önemsiz, `Box` JSON'u değiştirmeden yalnızca kullanımı zorlaştırır.
#[allow(clippy::large_enum_variant)]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(tag = "op", rename_all = "snake_case")]
#[ts(export)]
pub enum PlanRequest {
    Create { def: ObjectDef, concurrently: bool },
    Alter { old: ObjectDef, new: ObjectDef },
    Drop { def: ObjectDef, cascade: bool },
}

impl PlanRequest {
    fn op(&self) -> &'static str {
        match self {
            PlanRequest::Create { .. } => "create",
            PlanRequest::Alter { .. } => "alter",
            PlanRequest::Drop { .. } => "drop",
        }
    }

    fn kind(&self) -> ObjectKind {
        match self {
            PlanRequest::Create { def, .. } | PlanRequest::Drop { def, .. } => def.kind(),
            PlanRequest::Alter { new, .. } => new.kind(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ApplyOptions {
    /// İptal için istemci üretir.
    pub apply_id: Uuid,
    /// Kilit bekleme sınırı (ms; `0` = sınırsız).
    #[ts(type = "number")]
    pub lock_timeout_ms: u32,
    /// Önizlemedeki `PlanView.hash`.
    pub expected_hash: String,
    /// Yıkıcı/veri kaybı olan plan için kullanıcı onayı.
    pub confirmed: bool,
    /// Üretim sunucusunda veri kaybı olan plan için ek (ad yazarak) onay.
    pub confirmed_production: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[serde(tag = "outcome", rename_all = "snake_case", rename_all_fields = "camelCase")]
#[ts(export)]
pub enum ApplyOutcome {
    Applied {
        statements: u32,
        #[ts(type = "number")]
        elapsed_ms: u64,
    },
    /// `failed_index`: başarısız ifadenin 0 tabanlı sırası; `committed`: kalıcılaşmış (transaction dışı ya da önceki blok)
    /// ifade sayısı; `sqlstate`: PG hata kodu (ör. `57014` iptal, `55P03` kilit zaman aşımı; bağlantı hatasında yok).
    Failed { failed_index: u32, committed: u32, sqlstate: Option<String>, error: ErrorPayload },
}

/// "Yeni …" diyaloğunun açıldığı koleksiyon düğümü bağlamı.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct CreateContext {
    #[ts(type = "number | null")]
    pub schema_oid: Option<u32>,
    #[ts(type = "number | null")]
    pub table_oid: Option<u32>,
}

const SCHEMA_NAME_SQL: &str = "SELECT nspname::text AS name FROM pg_namespace WHERE oid = $1";
const TABLE_NAME_SQL: &str = "SELECT n.nspname::text AS schema, c.relname::text AS name \
     FROM pg_class c JOIN pg_namespace n ON n.oid = c.relnamespace WHERE c.oid = $1 AND c.relkind IN ('r', 'm', 'p')";

pub struct DdlManager {
    mgr: Arc<ConnectionManager>,
    registry: Arc<RwLock<Registry>>,
    cancels: CancelMap,
}

/// Çalıştırmanın günlük/iptal bilgileri.
struct Job {
    op: &'static str,
    kind: ObjectKind,
    lock_timeout_ms: u32,
    apply_id: Uuid,
}

/// Kayıttan okunan koruma bayrakları.
struct Guard {
    locked: bool,
    production: bool,
}

impl DdlManager {
    pub fn new(mgr: Arc<ConnectionManager>, registry: Arc<RwLock<Registry>>) -> Self {
        DdlManager { mgr, registry, cancels: CancelMap::default() }
    }

    fn guard(&self, server: Uuid) -> Result<Guard> {
        let registry = self.registry.read().unwrap_or_else(PoisonError::into_inner);
        let config = registry.get(server).ok_or(Error::UnknownServer(server))?;
        Ok(Guard { locked: config.read_only_lock, production: config.is_production() })
    }

    /// Önizleme: doğrulama hataları `PlanView.errors`'tadır (Err değil). Hiçbir şey yazmaz.
    pub async fn preview(&self, server: Uuid, _database_oid: u32, req: &PlanRequest) -> Result<PlanView> {
        let version_num = load::server_version(&self.mgr, server).await?;
        let view = plan_view(req, version_num);
        tracing::debug!(op = req.op(), kind = ?req.kind(), statements = view.statements.len(), errors = view.errors.len(), "DDL önizlemesi");
        Ok(view)
    }

    /// Kapılardan geçen planı ayrı bağlantıda uygular. Kapı reddi ve bağlantı kurulamaması `Err`; ifade
    /// hatası/iptali `Ok(ApplyOutcome::Failed)`.
    pub async fn apply(&self, server: Uuid, database_oid: u32, req: &PlanRequest, opts: ApplyOptions) -> Result<ApplyOutcome> {
        // (1) + (2)
        let guard = self.guard(server)?;
        if guard.locked {
            return Err(Error::Invalid(LOCKED_MESSAGE.into()));
        }
        // (3) — sürüm oturumun önbelleğinden okunur (bağlantı açılmaz).
        let view = plan_view(req, load::server_version(&self.mgr, server).await?);
        if !view.errors.is_empty() {
            return Err(Error::Invalid(INVALID_DEF.into()));
        }
        if view.empty {
            return Err(Error::Invalid(NO_CHANGES.into()));
        }
        // (4)
        if view.hash != opts.expected_hash {
            return Err(Error::Invalid(PLAN_CHANGED.into()));
        }
        // (5)
        if view.risk >= Risk::Destructive && !opts.confirmed {
            return Err(Error::Invalid(CONFIRMATION_REQUIRED.into()));
        }
        // (6)
        if view.risk == Risk::DataLoss && guard.production && !opts.confirmed_production {
            return Err(Error::Invalid(PRODUCTION_CONFIRMATION_REQUIRED.into()));
        }
        // (7)
        check_lock_timeout(opts.lock_timeout_ms)?;
        let job = Job { op: req.op(), kind: req.kind(), lock_timeout_ms: opts.lock_timeout_ms, apply_id: opts.apply_id };
        self.execute(server, database_oid, &view.statements, job).await
    }

    /// Bağlantı açar, ifadeleri blok blok çalıştırır, sonucu günlüğe (yalnızca tür/sayı/süre/sonuç) yazar.
    async fn execute(&self, server: Uuid, database_oid: u32, statements: &[Statement], job: Job) -> Result<ApplyOutcome> {
        let Job { op, kind, lock_timeout_ms, apply_id } = job;
        let started = Instant::now();
        // Yuva bağlantıdan önce açılır: bağlantı kurulurken gelen iptal de dikkate alınır.
        let reg = self.cancels.register(apply_id)?;
        let count = u32::try_from(statements.len()).unwrap_or(u32::MAX);
        let conn = match apply::open_apply_conn(&self.mgr, server, database_oid, lock_timeout_ms).await {
            Ok(c) => c,
            Err(e) => {
                tracing::warn!(op, ?kind, statements = count, outcome = apply::sqlstate(&e).unwrap_or("connect"), "DDL bağlantısı açılamadı");
                return Err(e);
            }
        };
        reg.set_handle(conn.cancel.clone());
        let result = apply::run_statements(&conn.client, statements, &reg).await;
        drop(reg);
        let elapsed_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
        Ok(match result {
            RunResult::Done => {
                tracing::info!(op, ?kind, statements = count, elapsed_ms, outcome = "applied", "DDL uygulandı");
                ApplyOutcome::Applied { statements: count, elapsed_ms }
            }
            RunResult::Failed { failed_index, committed, error } => {
                let sqlstate = apply::sqlstate(&error).map(str::to_string);
                tracing::info!(
                    op,
                    ?kind,
                    statements = count,
                    elapsed_ms,
                    outcome = sqlstate.as_deref().unwrap_or("error"),
                    "DDL uygulanamadı"
                );
                ApplyOutcome::Failed { failed_index, committed, sqlstate, error: apply::failure_payload(&error) }
            }
        })
    }

    /// Test yardımcısı: kapıları atlayıp verilen ifade listesini aynı blok çalıştırıcısıyla uygular
    /// (karışık transactional/transaction dışı planlar üreticilerle kurulamadığı için).
    #[cfg(test)]
    pub(crate) async fn apply_statements(
        &self,
        server: Uuid,
        database_oid: u32,
        statements: &[Statement],
        lock_timeout_ms: u32,
    ) -> Result<ApplyOutcome> {
        let job = Job { op: "test", kind: ObjectKind::Schema, lock_timeout_ms, apply_id: Uuid::new_v4() };
        self.execute(server, database_oid, statements, job).await
    }

    /// Süren uygulamayı iptal eder; bilinmeyen/bitmiş kimlik → `Ok` (idempotent).
    pub async fn cancel(&self, apply_id: Uuid) -> Result<()> {
        match self.cancels.request(apply_id) {
            Some(Some(handle)) => handle.cancel().await,
            _ => Ok(()),
        }
    }

    /// Tüm süren uygulamaları iptal eder (uygulama kapanışı); istekler arka planda gönderilir.
    pub fn cancel_all(&self) {
        let handles = self.cancels.request_all();
        let Ok(rt) = tokio::runtime::Handle::try_current() else { return };
        for h in handles {
            rt.spawn(async move {
                if h.cancel().await.is_err() {
                    tracing::debug!("DDL iptal isteği gönderilemedi");
                }
            });
        }
    }

    pub async fn load(&self, server: Uuid, database_oid: u32, obj: ObjectRef) -> Result<ObjectDef> {
        load::load(&self.mgr, server, database_oid, obj).await
    }

    pub async fn lookup(&self, server: Uuid, database_oid: u32, req: LookupRequest) -> Result<Vec<LookupItem>> {
        lookup::lookup(&self.mgr, server, database_oid, req).await
    }

    /// "Yeni …" diyaloğunun önceden doldurulmuş tanımı. Şema/tablo adları oid'den katalogdan çözülür.
    /// Geçersiz bağlam (sequence için şema yok, indeks için tablo yok, şema/eklenti için tablo) → `Invalid`.
    pub async fn defaults(&self, server: Uuid, database_oid: u32, kind: ObjectKind, ctx: CreateContext) -> Result<ObjectDef> {
        let invalid = |m: &str| Err(Error::Invalid(m.to_string()));
        Ok(match kind {
            ObjectKind::Schema | ObjectKind::Extension if ctx.table_oid.is_some() => {
                return invalid("Bu nesne tablo bağlamında oluşturulamaz.");
            }
            ObjectKind::Schema => ObjectDef::Schema(SchemaDef { name: String::new(), owner: None, comment: None }),
            ObjectKind::Extension => {
                ObjectDef::Extension(ExtensionDef { name: String::new(), schema: None, version: None, cascade: false })
            }
            ObjectKind::Sequence => {
                let (Some(schema_oid), None) = (ctx.schema_oid, ctx.table_oid) else {
                    return invalid("Sequence yalnızca bir şema altında oluşturulabilir.");
                };
                let schema = self.schema_name(server, database_oid, schema_oid).await?;
                ObjectDef::Sequence(SequenceDef {
                    schema,
                    name: String::new(),
                    owner: None,
                    data_type: SeqType::Bigint,
                    increment: 1,
                    min_value: None,
                    max_value: None,
                    start: 1,
                    cache: 1,
                    cycle: false,
                    owned_by: None,
                    comment: None,
                })
            }
            ObjectKind::Index => {
                let Some(table_oid) = ctx.table_oid else {
                    return invalid("İndeks yalnızca bir tablo altında oluşturulabilir.");
                };
                let (schema, table) = self.table_name(server, database_oid, table_oid).await?;
                ObjectDef::Index(IndexDef {
                    schema,
                    table,
                    name: String::new(),
                    method: "btree".into(),
                    unique: false,
                    columns: vec![],
                    include: vec![],
                    where_clause: None,
                    tablespace: None,
                    fillfactor: None,
                    comment: None,
                    constraint: None,
                })
            }
        })
    }

    async fn schema_name(&self, server: Uuid, database_oid: u32, oid: u32) -> Result<String> {
        let row = self
            .mgr
            .run(server, Target::Database(database_oid), move |c| async move { c.client.query_opt(SCHEMA_NAME_SQL, &[&oid]).await })
            .await?;
        row.map(|r| r.get("name")).ok_or_else(|| Error::NotFound("şema".into()))
    }

    async fn table_name(&self, server: Uuid, database_oid: u32, oid: u32) -> Result<(String, String)> {
        let row = self
            .mgr
            .run(server, Target::Database(database_oid), move |c| async move { c.client.query_opt(TABLE_NAME_SQL, &[&oid]).await })
            .await?;
        row.map(|r| (r.get("schema"), r.get("name"))).ok_or_else(|| Error::NotFound("tablo".into()))
    }
}

/// İstekten saf plan üretimi (önizleme ve uygulama aynı yolu kullanır; hash yalnızca ifadelerden).
pub(crate) fn plan_view(req: &PlanRequest, version_num: u32) -> PlanView {
    let ctx = |concurrently| BuildCtx { version_num, concurrently };
    PlanView::from_build(match req {
        PlanRequest::Create { def, concurrently } => build::create(def, &ctx(*concurrently)),
        PlanRequest::Alter { old, new } => build::alter(old, new, &ctx(false)),
        PlanRequest::Drop { def, cascade } => build::drop(def, *cascade, &ctx(false)),
    })
}

/// (7) `lock_timeout` sınırı (`SessionProfile::Ddl` doğrulamasıyla aynı; bağlantıdan önce).
fn check_lock_timeout(ms: u32) -> Result<()> {
    if ms > MAX_STATEMENT_TIMEOUT_MS {
        return Err(Error::Invalid(format!("lock_timeout en fazla {MAX_STATEMENT_TIMEOUT_MS} ms olabilir.")));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn schema(name: &str) -> ObjectDef {
        ObjectDef::Schema(SchemaDef { name: name.into(), owner: None, comment: None })
    }

    #[test]
    fn request_and_outcome_json_shapes() {
        let r = PlanRequest::Drop { def: schema("a"), cascade: true };
        let v = serde_json::to_value(&r).unwrap();
        assert_eq!(v["op"], "drop");
        assert_eq!(v["def"]["kind"], "schema");
        assert_eq!(serde_json::from_value::<PlanRequest>(v).unwrap(), r);
        let o = ApplyOptions {
            apply_id: Uuid::nil(),
            lock_timeout_ms: 5000,
            expected_hash: "h".into(),
            confirmed: true,
            confirmed_production: false,
        };
        let v = serde_json::to_value(&o).unwrap();
        assert_eq!(v["lockTimeoutMs"], 5000);
        assert_eq!(v["confirmedProduction"], false);
        assert_eq!(v["applyId"], Uuid::nil().to_string());
        let a = serde_json::to_value(ApplyOutcome::Applied { statements: 2, elapsed_ms: 7 }).unwrap();
        assert_eq!(a, json!({"outcome": "applied", "statements": 2, "elapsedMs": 7}));
        let f = ApplyOutcome::Failed {
            failed_index: 1,
            committed: 0,
            sqlstate: Some("42P07".into()),
            error: ErrorPayload { code: "query".into(), message: "m".into(), detail: None, hint: None },
        };
        let f = serde_json::to_value(f).unwrap();
        assert_eq!((f["outcome"].clone(), f["failedIndex"].clone(), f["sqlstate"].clone()), (json!("failed"), json!(1), json!("42P07")));
        let c: CreateContext = serde_json::from_value(json!({"schemaOid": 5, "tableOid": null})).unwrap();
        assert_eq!(c, CreateContext { schema_oid: Some(5), table_oid: None });
    }

    #[test]
    fn plan_view_matches_builders_and_hash() {
        let req = PlanRequest::Create { def: schema("a"), concurrently: false };
        let v = plan_view(&req, 160000);
        assert_eq!(v.statements.len(), 1);
        assert_eq!(v.hash, crate::plan_hash(&v.statements));
        let bad = plan_view(&PlanRequest::Create { def: schema(""), concurrently: false }, 160000);
        assert!(bad.empty && !bad.errors.is_empty());
        assert_eq!(bad.hash, crate::plan_hash(&[]));
        let noop = plan_view(&PlanRequest::Alter { old: schema("a"), new: schema("a") }, 160000);
        assert!(noop.empty && noop.errors.is_empty());
    }

    #[test]
    fn lock_timeout_bound() {
        assert!(check_lock_timeout(MAX_STATEMENT_TIMEOUT_MS).is_ok());
        assert!(matches!(check_lock_timeout(MAX_STATEMENT_TIMEOUT_MS + 1), Err(Error::Invalid(_))));
    }

    /// Docker testleri (`PGT_TEST_URLS` yoksa atlanır): crate içi yardımcıları (`apply_statements`,
    /// `open_apply_conn`, `prepare_session`) kullandıkları için birim testi olarak buradadır.
    mod docker {
        use super::*;
        use crate::apply::{open_apply_conn, prepare_session};
        use pgt_core::connection::SessionProfile;
        use pgt_core::testing::{TestServer, admin_client, drop_database, test_servers};

        fn st(sql: &str, transactional: bool) -> Statement {
            Statement { sql: sql.into(), risk: Risk::Safe, transactional, note: None }
        }

        async fn db_name(ts: &TestServer, tag: &str) -> String {
            let admin = admin_client(ts, &ts.config.maintenance_db).await;
            let v: i32 = admin.query_one("SELECT current_setting('server_version_num')::int4", &[]).await.unwrap().get(0);
            format!("pgt_ddl_mu_{}_{tag}", v / 10_000)
        }

        async fn cleanup(ts: &TestServer, tag: &str) {
            let admin = admin_client(ts, &ts.config.maintenance_db).await;
            drop_database(&admin, &db_name(ts, tag).await).await;
        }

        /// Veritabanını kurar; yönetici, `DdlManager`, veritabanı oid'i ve geçici dizini döndürür.
        async fn setup(ts: &TestServer, tag: &str) -> (tokio_postgres::Client, DdlManager, u32, tempfile::TempDir) {
            cleanup(ts, tag).await;
            let name = db_name(ts, tag).await;
            let maint = admin_client(ts, &ts.config.maintenance_db).await;
            maint.batch_execute(&format!("CREATE DATABASE {name} TEMPLATE template0")).await.unwrap();
            let oid: u32 = maint.query_one("SELECT oid FROM pg_database WHERE datname = $1", &[&name]).await.unwrap().get(0);
            let dir = tempfile::tempdir().unwrap();
            let mut registry = Registry::load(dir.path().join("servers.json")).unwrap().registry;
            registry.upsert(ts.config.clone()).unwrap();
            let mgr = Arc::new(ConnectionManager::new());
            mgr.connect(ts.config.clone(), ts.password.clone()).await.unwrap();
            (admin_client(ts, &name).await, DdlManager::new(mgr, Arc::new(RwLock::new(registry))), oid, dir)
        }

        async fn guarded(ts: &TestServer, tag: &str, body: impl std::future::Future<Output = ()> + Send + 'static) {
            let result = tokio::spawn(body).await;
            cleanup(ts, tag).await;
            if let Err(e) = result {
                std::panic::resume_unwind(e.into_panic());
            }
        }

        async fn exists(admin: &tokio_postgres::Client, sql: &str) -> bool {
            admin.query_one(&format!("SELECT ({sql}) IS NOT NULL"), &[]).await.unwrap().get(0)
        }

        async fn show(client: &tokio_postgres::Client, name: &str) -> String {
            client.query_one(&format!("SHOW {name}"), &[]).await.unwrap().get(0)
        }

        #[tokio::test]
        async fn mixed_plan_reports_partial_commit() {
            for ts in test_servers() {
                let tag = "mixed";
                let t = ts.clone();
                guarded(&ts, tag, async move {
                    let (admin, dm, db, _dir) = setup(&t, tag).await;
                    let plan = [
                        st("CREATE TABLE public.mx_a (id int)", true),
                        st("CREATE INDEX CONCURRENTLY mx_i ON public.mx_a (id)", false),
                        st("CREATE TABLE public.mx_b (id int)", true),
                        st("CREATE TABLE public.mx_a (id int)", true),
                    ];
                    let out = dm.apply_statements(t.config.id, db, &plan, 5000).await.unwrap();
                    let ApplyOutcome::Failed { failed_index, committed, sqlstate, .. } = out else { panic!("{out:?}") };
                    assert_eq!((failed_index, committed, sqlstate.as_deref()), (3, 2, Some("42P07")));
                    assert!(exists(&admin, "to_regclass('public.mx_a')").await);
                    assert!(exists(&admin, "to_regclass('public.mx_i')").await);
                    assert!(!exists(&admin, "to_regclass('public.mx_b')").await, "başarısız blok geri alınmalı");

                    // transactional blok → hata veren CONCURRENTLY (aynı ad): önceki blok kalıcı.
                    let plan = [
                        st("CREATE TABLE public.mx_c (id int)", true),
                        st("CREATE INDEX CONCURRENTLY mx_i ON public.mx_a (id)", false),
                    ];
                    let out = dm.apply_statements(t.config.id, db, &plan, 5000).await.unwrap();
                    let ApplyOutcome::Failed { failed_index, committed, .. } = out else { panic!("{out:?}") };
                    assert_eq!((failed_index, committed), (1, 1));
                    assert!(exists(&admin, "to_regclass('public.mx_c')").await);

                    // Başarılı karışık plan.
                    let plan = [st("CREATE TABLE public.mx_d (id int)", true), st("CREATE INDEX CONCURRENTLY mx_j ON public.mx_d (id)", false)];
                    let out = dm.apply_statements(t.config.id, db, &plan, 5000).await.unwrap();
                    assert!(matches!(out, ApplyOutcome::Applied { statements: 2, .. }), "{out:?}");
                    assert_eq!(dm.cancels.len(), 0);
                })
                .await;
            }
        }

        #[tokio::test]
        async fn applier_sets_session_explicitly() {
            for ts in test_servers() {
                let tag = "sets";
                let t = ts.clone();
                guarded(&ts, tag, async move {
                    let (_admin, dm, db, _dir) = setup(&t, tag).await;
                    // Uygulayıcının bağlantı yolu.
                    let conn = open_apply_conn(&dm.mgr, t.config.id, db, 4321).await.unwrap();
                    assert_eq!(show(&conn.client, "lock_timeout").await, "4321ms");
                    assert_eq!(show(&conn.client, "statement_timeout").await, "0");
                    assert_eq!(show(&conn.client, "default_transaction_read_only").await, "off");
                    assert_eq!(show(&conn.client, "standard_conforming_strings").await, "on");
                    assert_eq!(show(&conn.client, "application_name").await, "pg-tools ddl");

                    // Başlangıç ayarları yok sayılmış/farklı bir oturumda bile açık SET'ler değerleri kurar.
                    let other = dm
                        .mgr
                        .open_dedicated(t.config.id, Target::Database(db), SessionProfile::Query { read_only: true, statement_timeout_ms: 1234 })
                        .await
                        .unwrap();
                    other.client.batch_execute("SET standard_conforming_strings = off; SET lock_timeout = 77").await.unwrap();
                    prepare_session(&other.client, 0).await.unwrap();
                    assert_eq!(show(&other.client, "lock_timeout").await, "0");
                    assert_eq!(show(&other.client, "statement_timeout").await, "0");
                    assert_eq!(show(&other.client, "default_transaction_read_only").await, "off");
                    assert_eq!(show(&other.client, "standard_conforming_strings").await, "on");
                })
                .await;
            }
        }
    }
}
