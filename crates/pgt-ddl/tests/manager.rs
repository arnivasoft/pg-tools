//! `DdlManager` testleri: kapılar (sıra dahil), blok çalıştırma, lock_timeout, iptal ve günlük gizliliği.
//! Docker'lı testler `PGT_TEST_URLS` yoksa atlanır; her test kendi veritabanını (`pgt_ddl_rt_<sürüm>_m_<etiket>`) kurar.

mod common;

use std::io::Write;
use std::sync::{Arc, Mutex, RwLock};
use std::time::{Duration, Instant};

use common::{Env, cleanup, guarded, servers, setup};
use pgt_core::config::ServerConfig;
use pgt_core::connection::ConnectionManager;
use pgt_core::registry::Registry;
use pgt_core::testing::{TestServer, admin_client, drop_database};
use pgt_core::{Error, ErrorPayload, Result};
use pgt_ddl::*;
use tempfile::TempDir;
use uuid::Uuid;

// ---- yardımcılar ----

struct Fx {
    dm: Arc<DdlManager>,
    server: Uuid,
    db: u32,
    _dir: TempDir,
}

fn manager(mgr: Arc<ConnectionManager>, config: &ServerConfig, locked: bool, label: Option<&str>) -> (Arc<DdlManager>, TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let mut registry = Registry::load(dir.path().join("servers.json")).unwrap().registry;
    let mut config = config.clone();
    config.read_only_lock = locked;
    config.label = label.map(str::to_string);
    registry.upsert(config).unwrap();
    (Arc::new(DdlManager::new(mgr, Arc::new(RwLock::new(registry)))), dir)
}

fn fx(env: &Env, ts: &TestServer, locked: bool, label: Option<&str>) -> Fx {
    let (dm, dir) = manager(env.mgr.clone(), &ts.config, locked, label);
    Fx { dm, server: env.server, db: env.db_oid, _dir: dir }
}

fn opts(hash: &str) -> ApplyOptions {
    ApplyOptions { apply_id: Uuid::new_v4(), lock_timeout_ms: 5000, expected_hash: hash.into(), confirmed: false, confirmed_production: false }
}

impl Fx {
    async fn preview(&self, req: &PlanRequest) -> PlanView {
        self.dm.preview(self.server, self.db, req).await.unwrap()
    }

    /// Önizleme hash'iyle uygular; `edit` seçenekleri değiştirir.
    async fn apply_with(&self, req: &PlanRequest, edit: impl FnOnce(&mut ApplyOptions)) -> Result<ApplyOutcome> {
        let mut o = opts(&self.preview(req).await.hash);
        edit(&mut o);
        self.dm.apply(self.server, self.db, req, o).await
    }

    async fn apply(&self, req: &PlanRequest) -> Result<ApplyOutcome> {
        self.apply_with(req, |_| {}).await
    }
}

fn code(r: &Result<ApplyOutcome>) -> String {
    match r {
        Err(e) => ddl_payload(e).code,
        Ok(o) => panic!("hata bekleniyordu: {o:?}"),
    }
}

fn applied(r: Result<ApplyOutcome>) {
    match r {
        Ok(ApplyOutcome::Applied { .. }) => {}
        other => panic!("uygulanmalıydı: {other:?}"),
    }
}

fn failed(r: Result<ApplyOutcome>) -> (u32, u32, Option<String>, ErrorPayload) {
    match r {
        Ok(ApplyOutcome::Failed { failed_index, committed, sqlstate, error }) => (failed_index, committed, sqlstate, error),
        other => panic!("Failed bekleniyordu: {other:?}"),
    }
}

fn schema(name: &str) -> ObjectDef {
    ObjectDef::Schema(SchemaDef { name: name.into(), owner: None, comment: None })
}

fn create(def: ObjectDef) -> PlanRequest {
    PlanRequest::Create { def, concurrently: false }
}

fn index(table: &str, name: &str) -> ObjectDef {
    ObjectDef::Index(IndexDef {
        schema: "public".into(),
        table: table.into(),
        name: name.into(),
        method: "btree".into(),
        unique: false,
        columns: vec![IndexColumn { key: IndexKey::Column { name: "id".into() }, opclass: None, collation: None, descending: false, nulls_first: None }],
        include: vec![],
        where_clause: None,
        tablespace: None,
        fillfactor: None,
        comment: None,
        constraint: None,
    })
}

/// `şema` ya da `şema.ilişki` var mı (PG 10'da `to_regclass` `cstring` aldığı için katalogdan).
async fn exists(env: &Env, name: &str) -> bool {
    let sql = "SELECT EXISTS (SELECT 1 FROM pg_namespace WHERE nspname = $1::text)                OR EXISTS (SELECT 1 FROM pg_class c JOIN pg_namespace n ON n.oid = c.relnamespace WHERE n.nspname || '.' || c.relname = $1::text)";
    env.admin.query_one(sql, &[&name]).await.unwrap().get(0)
}

/// Uygulama oturumlarından kaçı kilit bekliyor.
async fn ddl_waiting(admin: &tokio_postgres::Client) -> i64 {
    let sql = "SELECT count(*) FROM pg_stat_activity WHERE application_name = 'pg-tools ddl' AND wait_event_type = 'Lock' AND datname = current_database()";
    admin.query_one(sql, &[]).await.unwrap().get(0)
}

async fn wait_for_waiting(admin: &tokio_postgres::Client, n: i64) {
    let deadline = Instant::now() + Duration::from_secs(15);
    while ddl_waiting(admin).await != n {
        assert!(Instant::now() < deadline, "{n} bekleyen DDL oturumu görülmedi");
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

/// Tabloyu ACCESS EXCLUSIVE kilitleyen ayrı oturum (transaction açık kalır).
async fn lock_table(ts: &TestServer, env: &Env, table: &str) -> tokio_postgres::Client {
    let name: String = env.admin.query_one("SELECT current_database()::text", &[]).await.unwrap().get(0);
    let locker = admin_client(ts, &name).await;
    locker.batch_execute(&format!("BEGIN; LOCK TABLE public.{table} IN ACCESS EXCLUSIVE MODE")).await.unwrap();
    locker
}

// ---- bağlantısız kapılar ----

/// Kapılar bağlantı açmadan önce çalışır: bağlı olmayan sunucuda kilit/kayıt kapıları yine de kendi kodunu döndürür.
#[tokio::test]
async fn gates_run_before_any_connection() {
    let mgr = Arc::new(ConnectionManager::new());
    let config = ServerConfig::new("x", "127.0.0.1", 1, "u");
    let req = create(schema("a"));
    let (locked, _d1) = manager(mgr.clone(), &config, true, None);
    let r = locked.apply(config.id, 1, &req, opts("x")).await;
    assert_eq!(code(&r), "locked");
    let (open, _d2) = manager(mgr.clone(), &config, false, None);
    assert!(matches!(open.apply(config.id, 1, &req, opts("x")).await, Err(Error::NotConnected)));
    assert!(matches!(open.apply(Uuid::new_v4(), 1, &req, opts("x")).await, Err(Error::UnknownServer(_))));
    // İptal: bilinmeyen kimlik Ok.
    open.cancel(Uuid::new_v4()).await.unwrap();
    open.cancel_all();
}

// ---- önizleme / kilit ----

#[tokio::test]
async fn preview_reports_validation_errors_without_error() {
    for ts in servers() {
        guarded(preview_body(ts.clone()), cleanup(&ts, "m_preview")).await;
    }
}

async fn preview_body(ts: TestServer) {
    let env = setup(&ts, "m_preview").await;
    let f = fx(&env, &ts, false, None);
    let v = f.preview(&create(schema(""))).await;
    assert!(!v.errors.is_empty() && v.statements.is_empty() && v.empty);
    assert_eq!(v.hash, plan_hash(&[]));
    // Geçerli önizleme hiçbir şey yazmaz.
    let v = f.preview(&create(schema("m_pv"))).await;
    assert_eq!((v.statements.len(), v.errors.len(), v.risk), (1, 0, Risk::Safe));
    assert_eq!(v.hash, plan_hash(&v.statements));
    assert!(!exists(&env, "m_pv").await);
}

#[tokio::test]
async fn locked_server_refuses_apply_but_allows_preview() {
    for ts in servers() {
        guarded(locked_body(ts.clone()), cleanup(&ts, "m_locked")).await;
    }
}

async fn locked_body(ts: TestServer) {
    let env = setup(&ts, "m_locked").await;
    let f = fx(&env, &ts, true, None);
    let req = create(schema("m_lk"));
    assert_eq!(code(&f.apply(&req).await), "locked");
    assert!(!exists(&env, "m_lk").await);
    // Kilit, geçersiz tanımdan önce gelir.
    assert_eq!(code(&f.apply(&create(schema(""))).await), "locked");
    // Önizleme, yükleme ve açılır listeler serbest.
    assert_eq!(f.preview(&req).await.statements.len(), 1);
    let oid = env.schema_oid("public").await;
    assert!(matches!(f.dm.load(f.server, f.db, ObjectRef { kind: ObjectKind::Schema, oid }).await.unwrap(), ObjectDef::Schema(_)));
    let items = f.dm.lookup(f.server, f.db, LookupRequest::Schemas).await.unwrap();
    assert!(items.iter().any(|i| i.value == "public"));
}

// ---- onay / üretim / hash / kapı sırası ----

#[tokio::test]
async fn destructive_plan_needs_confirmation() {
    for ts in servers() {
        guarded(destructive_body(ts.clone()), cleanup(&ts, "m_destr")).await;
    }
}

async fn destructive_body(ts: TestServer) {
    let env = setup(&ts, "m_destr").await;
    let f = fx(&env, &ts, false, None);
    applied(f.apply(&create(schema("m_d"))).await);
    assert!(exists(&env, "m_d").await);
    let drop = PlanRequest::Drop { def: schema("m_d"), cascade: false };
    assert_eq!(f.preview(&drop).await.risk, Risk::Destructive);
    assert_eq!(code(&f.apply(&drop).await), "confirmation_required");
    assert!(exists(&env, "m_d").await);
    applied(f.apply_with(&drop, |o| o.confirmed = true).await);
    assert!(!exists(&env, "m_d").await);
}

#[tokio::test]
async fn production_dataloss_needs_name_confirmation() {
    for ts in servers() {
        guarded(production_body(ts.clone()), cleanup(&ts, "m_prod")).await;
    }
}

async fn production_body(ts: TestServer) {
    let env = setup(&ts, "m_prod").await;
    let f = fx(&env, &ts, false, Some("Üretim"));
    env.sql("CREATE SCHEMA m_p1; CREATE TABLE m_p1.t (id int); CREATE SCHEMA m_p2; CREATE SCHEMA m_p3").await;
    let cascade = |n: &str| PlanRequest::Drop { def: schema(n), cascade: true };
    assert_eq!(f.preview(&cascade("m_p1")).await.risk, Risk::DataLoss);
    // Önce genel onay (5), sonra üretim onayı (6).
    assert_eq!(code(&f.apply(&cascade("m_p1")).await), "confirmation_required");
    assert_eq!(code(&f.apply_with(&cascade("m_p1"), |o| o.confirmed = true).await), "production_confirmation_required");
    assert!(exists(&env, "m_p1.t").await);
    applied(f.apply_with(&cascade("m_p1"), |o| (o.confirmed, o.confirmed_production) = (true, true)).await);
    assert!(!exists(&env, "m_p1").await);
    // Yalnızca yıkıcı (veri kaybı yok) plan üretimde ek onay istemez.
    applied(f.apply_with(&PlanRequest::Drop { def: schema("m_p2"), cascade: false }, |o| o.confirmed = true).await);
    // Üretim etiketi olmayan sunucuda veri kaybı için genel onay yeter.
    let plain = fx(&env, &ts, false, Some("test"));
    applied(plain.apply_with(&cascade("m_p3"), |o| o.confirmed = true).await);
    assert!(!exists(&env, "m_p3").await);
}

#[tokio::test]
async fn stale_hash_is_refused_and_gate_order_holds() {
    for ts in servers() {
        guarded(stale_body(ts.clone()), cleanup(&ts, "m_stale")).await;
    }
}

async fn stale_body(ts: TestServer) {
    let env = setup(&ts, "m_stale").await;
    let f = fx(&env, &ts, false, None);
    let a = create(schema("m_sa"));
    let b = create(schema("m_sb"));
    let hash_a = f.preview(&a).await.hash;
    assert_eq!(code(&f.dm.apply(f.server, f.db, &b, opts(&hash_a)).await), "plan_changed");
    assert!(!exists(&env, "m_sb").await && !exists(&env, "m_sa").await);
    // Geçersiz tanım hash uyuşmazlığından önce gelir.
    assert_eq!(code(&f.dm.apply(f.server, f.db, &create(schema("")), opts("bogus")).await), "invalid_def");
    // Hash uyuşmazlığı onaydan önce gelir.
    env.sql("CREATE SCHEMA m_sc").await;
    let drop = PlanRequest::Drop { def: schema("m_sc"), cascade: false };
    assert_eq!(code(&f.dm.apply(f.server, f.db, &drop, opts("bogus")).await), "plan_changed");
    // Boş plan: kodsuz `invalid`.
    let noop = PlanRequest::Alter { old: schema("m_sc"), new: schema("m_sc") };
    let r = f.apply(&noop).await;
    assert_eq!(code(&r), "invalid");
    assert!(matches!(r, Err(Error::Invalid(m)) if m == "Değişiklik yok"));
    // (7) lock_timeout sınırı en sonda; hiçbir şey yazılmaz.
    let r = f.apply_with(&a, |o| o.lock_timeout_ms = u32::MAX).await;
    assert_eq!(code(&r), "invalid");
    assert!(!exists(&env, "m_sa").await);
}

// ---- çalıştırma ----

#[tokio::test]
async fn all_transactional_failure_leaves_nothing() {
    for ts in servers() {
        guarded(all_tx_body(ts.clone()), cleanup(&ts, "m_alltx")).await;
    }
}

async fn all_tx_body(ts: TestServer) {
    let env = setup(&ts, "m_alltx").await;
    let f = fx(&env, &ts, false, None);
    env.sql("CREATE SCHEMA m_ta").await;
    // RENAME (başarılı) + OWNER TO olmayan rol (hata) → ikisi de geri alınır.
    let new = ObjectDef::Schema(SchemaDef { name: "m_tb".into(), owner: Some("m_no_such_role".into()), comment: None });
    let req = PlanRequest::Alter { old: schema("m_ta"), new };
    assert_eq!(f.preview(&req).await.statements.len(), 2);
    let (idx, committed, sqlstate, error) = failed(f.apply(&req).await);
    assert_eq!((idx, committed, sqlstate.as_deref()), (1, 0, Some("42704")));
    assert_eq!(error.code, "query");
    assert!(!error.message.contains("ALTER"), "SQL metni hataya girmemeli: {}", error.message);
    assert!(exists(&env, "m_ta").await && !exists(&env, "m_tb").await);
}

#[tokio::test]
async fn applied_outcome_is_verified_by_load_and_defaults_are_prefilled() {
    for ts in servers() {
        guarded(applied_body(ts.clone()), cleanup(&ts, "m_applied")).await;
    }
}

async fn applied_body(ts: TestServer) {
    let env = setup(&ts, "m_applied").await;
    let f = fx(&env, &ts, false, None);
    let public = env.schema_oid("public").await;
    let ctx = |schema_oid, table_oid| CreateContext { schema_oid, table_oid };

    // Sequence varsayılanı şema bağlamından.
    let ObjectDef::Sequence(mut s) = f.dm.defaults(f.server, f.db, ObjectKind::Sequence, ctx(Some(public), None)).await.unwrap() else {
        panic!("sequence bekleniyordu")
    };
    assert_eq!((s.schema.as_str(), s.name.as_str(), s.data_type, s.increment, s.start, s.cache), ("public", "", SeqType::Bigint, 1, 1, 1));
    assert_eq!((s.min_value, s.max_value, s.cycle), (None, None, false));
    s.name = "m_seq".into();
    s.increment = 2;
    s.start = 5;
    s.comment = Some("yorum".into());
    let def = ObjectDef::Sequence(s.clone());
    let r = f.apply(&create(def.clone())).await.unwrap();
    let ApplyOutcome::Applied { statements, .. } = r else { panic!("{r:?}") };
    assert_eq!(statements as usize, f.preview(&create(def.clone())).await.statements.len());
    let oid = env.class_oid("public", "m_seq").await;
    let loaded = f.dm.load(f.server, f.db, ObjectRef { kind: ObjectKind::Sequence, oid }).await.unwrap();
    s.owner = Some(env.me.clone());
    assert_eq!(loaded, ObjectDef::Sequence(s));

    // İndeks varsayılanı tablo bağlamından.
    env.sql(r#"CREATE TABLE public."M t" (id int)"#).await;
    let t = env.class_oid("public", "M t").await;
    let ObjectDef::Index(i) = f.dm.defaults(f.server, f.db, ObjectKind::Index, ctx(None, Some(t))).await.unwrap() else {
        panic!("indeks bekleniyordu")
    };
    assert_eq!((i.schema.as_str(), i.table.as_str(), i.method.as_str(), i.name.as_str()), ("public", "M t", "btree", ""));
    assert!(i.columns.is_empty() && !i.unique);
    let ObjectDef::Schema(sd) = f.dm.defaults(f.server, f.db, ObjectKind::Schema, ctx(None, None)).await.unwrap() else { panic!() };
    assert!(sd.name.is_empty());
    let ObjectDef::Extension(e) = f.dm.defaults(f.server, f.db, ObjectKind::Extension, ctx(Some(public), None)).await.unwrap() else {
        panic!()
    };
    assert!(e.name.is_empty() && e.schema.is_none());

    // Geçersiz bağlamlar.
    let invalid = |r: Result<ObjectDef>| assert!(matches!(r, Err(Error::Invalid(_))), "{r:?}");
    invalid(f.dm.defaults(f.server, f.db, ObjectKind::Sequence, ctx(None, None)).await);
    invalid(f.dm.defaults(f.server, f.db, ObjectKind::Sequence, ctx(Some(public), Some(t))).await);
    invalid(f.dm.defaults(f.server, f.db, ObjectKind::Index, ctx(Some(public), None)).await);
    invalid(f.dm.defaults(f.server, f.db, ObjectKind::Schema, ctx(None, Some(t))).await);
    invalid(f.dm.defaults(f.server, f.db, ObjectKind::Extension, ctx(None, Some(t))).await);
    let missing = f.dm.defaults(f.server, f.db, ObjectKind::Index, ctx(None, Some(u32::MAX - 7))).await;
    assert!(matches!(missing, Err(Error::NotFound(_))), "{missing:?}");
    let missing = f.dm.defaults(f.server, f.db, ObjectKind::Sequence, ctx(Some(u32::MAX - 7), None)).await;
    assert!(matches!(missing, Err(Error::NotFound(_))), "{missing:?}");
}

#[tokio::test]
async fn non_transactional_statement_runs_outside_tx() {
    for ts in servers() {
        guarded(concurrently_body(ts.clone()), cleanup(&ts, "m_conc")).await;
    }
}

async fn concurrently_body(ts: TestServer) {
    let env = setup(&ts, "m_conc").await;
    let f = fx(&env, &ts, false, None);
    env.sql("CREATE TABLE public.ct (id int)").await;
    let req = PlanRequest::Create { def: index("ct", "ct_idx"), concurrently: true };
    let v = f.preview(&req).await;
    assert!(!v.statements[0].transactional, "CONCURRENTLY transaction dışı olmalı");
    applied(f.apply(&req).await);
    let valid: bool = env.admin.query_one("SELECT indisvalid FROM pg_index WHERE indexrelid = 'public.ct_idx'::regclass", &[]).await.unwrap().get(0);
    assert!(valid);
}

#[tokio::test]
async fn lock_timeout_aborts_waiting_apply() {
    for ts in servers() {
        guarded(lock_timeout_body(ts.clone()), cleanup(&ts, "m_lockto")).await;
    }
}

async fn lock_timeout_body(ts: TestServer) {
    let env = setup(&ts, "m_lockto").await;
    let f = fx(&env, &ts, false, None);
    env.sql("CREATE TABLE public.lt (id int)").await;
    let locker = lock_table(&ts, &env, "lt").await;
    let req = create(index("lt", "lt_idx"));
    let hash = f.preview(&req).await.hash;
    let started = Instant::now();
    let r = f.dm.apply(f.server, f.db, &req, ApplyOptions { lock_timeout_ms: 500, ..opts(&hash) }).await;
    let elapsed = started.elapsed();
    let (idx, committed, sqlstate, error) = failed(r);
    assert_eq!((idx, committed, sqlstate.as_deref()), (0, 0, Some("55P03")));
    assert!(error.hint.is_some(), "{error:?}");
    assert!(elapsed < Duration::from_secs(2), "lock_timeout beklenenden uzun: {elapsed:?}");
    assert!(!exists(&env, "public.lt_idx").await);
    assert_eq!(ddl_waiting(&env.admin).await, 0, "arkada bekleyen DDL oturumu kalmamalı");
    locker.batch_execute("COMMIT").await.unwrap();
    // Kilit bırakılınca aynı plan (aynı hash) uygulanır.
    applied(f.dm.apply(f.server, f.db, &req, ApplyOptions { lock_timeout_ms: 500, ..opts(&hash) }).await);
    assert!(exists(&env, "public.lt_idx").await);
}

// ---- iptal ----

#[tokio::test]
async fn cancel_stops_a_running_apply() {
    for ts in servers() {
        guarded(cancel_body(ts.clone()), cleanup(&ts, "m_cancel")).await;
    }
}

async fn cancel_body(ts: TestServer) {
    let env = setup(&ts, "m_cancel").await;
    let f = fx(&env, &ts, false, None);
    env.sql("CREATE TABLE public.cn (id int)").await;
    let locker = lock_table(&ts, &env, "cn").await;
    let req = create(index("cn", "cn_idx"));
    let o = ApplyOptions { lock_timeout_ms: 0, ..opts(&f.preview(&req).await.hash) };
    let id = o.apply_id;
    let (dm, server, db) = (f.dm.clone(), f.server, f.db);
    let task = tokio::spawn(async move { dm.apply(server, db, &req, o).await });
    wait_for_waiting(&env.admin, 1).await;
    f.dm.cancel(id).await.unwrap();
    let (idx, committed, sqlstate, error) = failed(task.await.unwrap());
    assert_eq!((idx, committed, sqlstate.as_deref()), (0, 0, Some("57014")));
    assert!(error.message.contains("iptal"), "{error:?}");
    // Bitmiş ve bilinmeyen kimlik: Ok.
    f.dm.cancel(id).await.unwrap();
    f.dm.cancel(Uuid::new_v4()).await.unwrap();
    locker.batch_execute("COMMIT").await.unwrap();
    assert!(!exists(&env, "public.cn_idx").await);
}

#[tokio::test]
async fn concurrent_apply_ids_are_independent() {
    for ts in servers() {
        guarded(concurrent_body(ts.clone()), cleanup(&ts, "m_concid")).await;
    }
}

async fn concurrent_body(ts: TestServer) {
    let env = setup(&ts, "m_concid").await;
    let f = fx(&env, &ts, false, None);
    env.sql("CREATE TABLE public.cc (id int)").await;
    let locker = lock_table(&ts, &env, "cc").await;
    let spawn = |name: &str, o_edit: Option<Uuid>| {
        let req = create(index("cc", name));
        let (dm, server, db) = (f.dm.clone(), f.server, f.db);
        async move {
            let hash = dm.preview(server, db, &req).await.unwrap().hash;
            let mut o = ApplyOptions { lock_timeout_ms: 0, ..opts(&hash) };
            if let Some(id) = o_edit {
                o.apply_id = id;
            }
            let id = o.apply_id;
            (id, tokio::spawn(async move { dm.apply(server, db, &req, o).await }))
        }
    };
    let (a, ta) = spawn("cc_a", None).await;
    let (b, tb) = spawn("cc_b", None).await;
    wait_for_waiting(&env.admin, 2).await;
    // Süren kimlikle ikinci uygulama reddedilir (bağlantı açmadan, kodsuz invalid).
    let (_, dup) = spawn("cc_c", Some(b)).await;
    assert!(matches!(dup.await.unwrap(), Err(Error::Invalid(_))));
    f.dm.cancel(a).await.unwrap();
    let (_, _, sqlstate, _) = failed(ta.await.unwrap());
    assert_eq!(sqlstate.as_deref(), Some("57014"));
    // B hâlâ bekliyor; kilit bırakılınca uygulanır.
    wait_for_waiting(&env.admin, 1).await;
    assert!(!tb.is_finished());
    locker.batch_execute("COMMIT").await.unwrap();
    applied(tb.await.unwrap());
    assert!(exists(&env, "public.cc_b").await && !exists(&env, "public.cc_a").await && !exists(&env, "public.cc_c").await);
}

// ---- hata yolları ----

#[tokio::test]
async fn apply_on_dropped_database_is_an_error_not_a_panic() {
    for ts in servers() {
        guarded(dropped_body(ts.clone()), cleanup(&ts, "m_dropped")).await;
    }
}

async fn dropped_body(ts: TestServer) {
    let env = setup(&ts, "m_dropped").await;
    let f = fx(&env, &ts, false, None);
    let maint = admin_client(&ts, &ts.config.maintenance_db).await;
    let name = format!("pgt_ddl_rt_{}_m_gone", env.version / 10_000);
    drop_database(&maint, &name).await;
    maint.batch_execute(&format!("CREATE DATABASE {name}")).await.unwrap();
    let oid: u32 = maint.query_one("SELECT oid FROM pg_database WHERE datname = $1", &[&name]).await.unwrap().get(0);
    drop_database(&maint, &name).await;
    let req = create(schema("m_x"));
    let hash = f.preview(&req).await.hash;
    let r = f.dm.apply(f.server, oid, &req, opts(&hash)).await;
    assert!(r.is_err(), "{r:?}");
}

// ---- günlük ----

#[derive(Clone, Default)]
struct Buf(Arc<Mutex<Vec<u8>>>);

impl Write for Buf {
    fn write(&mut self, b: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(b);
        Ok(b.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

/// Süreç genelinde (global) abone: iş parçacığı yerel abone, paralel testlerin callsite önbelleğiyle yarışıp
/// olayları kaçırabiliyor. Diğer testlerin olayları da tampona düşer; denetimler yalnızca bu testin işaretlerine bakar.
#[tokio::test]
async fn password_free_logging() {
    let buf = Buf::default();
    let w = buf.clone();
    let subscriber = tracing_subscriber::fmt().with_writer(move || w.clone()).with_max_level(tracing::Level::TRACE).finish();
    tracing::subscriber::set_global_default(subscriber).expect("bu test ikilisinde başka global abone yok");
    for ts in servers() {
        guarded(logging_body(ts.clone()), cleanup(&ts, "m_log")).await;
        let out = String::from_utf8(buf.0.lock().unwrap().clone()).unwrap();
        assert!(out.contains("DDL uygulandı") && out.contains("DDL uygulanamadı"), "günlük yakalanmadı: {out}");
        assert!(out.contains("42P06"), "sonuç kodu günlükte olmalı: {out}");
        for secret in ["m_log_gizli_ad", "gizli yorum", "CREATE", "SCHEMA", "COMMENT", "already exists"] {
            assert!(!out.contains(secret), "günlükte {secret:?} var: {out}");
        }
        if let Some(p) = ts.password.as_deref().filter(|p| !p.is_empty()) {
            assert!(!out.contains(p), "günlükte parola var");
        }
    }
}

async fn logging_body(ts: TestServer) {
    let env = setup(&ts, "m_log").await;
    let f = fx(&env, &ts, false, None);
    let def = ObjectDef::Schema(SchemaDef { name: "m_log_gizli_ad".into(), owner: None, comment: Some("gizli yorum".into()) });
    applied(f.apply(&create(def.clone())).await);
    // Aynı ad: 42P06 → Failed (hata iletisi adı içerir ama günlüğe yalnızca kod gider).
    let (_, _, sqlstate, _) = failed(f.apply(&create(def)).await);
    assert_eq!(sqlstate.as_deref(), Some("42P06"));
}
