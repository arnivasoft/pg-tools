//! `DdlManager` testleri: kapılar (sıra dahil), blok çalıştırma, lock_timeout, iptal. Günlük gizliliği süreç genelinde abone gerektirdiği için ayrı ikilide (`logging.rs`).
//! Docker'lı testler `PGT_TEST_URLS` yoksa atlanır; her test kendi veritabanını (`pgt_ddl_rt_<sürüm>_m_<etiket>`) kurar.

mod common;

use std::sync::{Arc, RwLock};
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
    ApplyOptions { apply_id: Uuid::new_v4(), lock_timeout_ms: 5000, expected_hash: hash.into(), confirmed: false, confirmed_production: false, password: None }
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

    /// Şemayı sunucudan yükler (kimlik + `ddl_load` çıktısı): arayüzün geri göndereceği biçim.
    async fn load_schema(&self, env: &Env, name: &str) -> (ObjectRef, SchemaDef) {
        let object = ObjectRef { kind: ObjectKind::Schema, oid: env.schema_oid(name).await };
        let ObjectDef::Schema(d) = self.dm.load(self.server, self.db, object).await.unwrap() else { panic!("şema bekleniyordu") };
        (object, d)
    }

    async fn drop_schema(&self, env: &Env, name: &str, cascade: bool) -> PlanRequest {
        let (object, d) = self.load_schema(env, name).await;
        PlanRequest::Drop { object, def: ObjectDef::Schema(d), cascade, force: false }
    }

    async fn alter_schema(&self, env: &Env, name: &str, edit: impl FnOnce(&mut SchemaDef)) -> PlanRequest {
        let (object, old) = self.load_schema(env, name).await;
        let mut new = old.clone();
        edit(&mut new);
        PlanRequest::Alter { object, old: ObjectDef::Schema(old), new: ObjectDef::Schema(new) }
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
        Ok(ApplyOutcome::Failed { failed_index, committed, sqlstate, error, .. }) => (failed_index, committed, sqlstate, error),
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
        valid: true,
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
    let drop = f.drop_schema(&env, "m_d", false).await;
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
    let (c1, c3) = (f.drop_schema(&env, "m_p1", true).await, f.drop_schema(&env, "m_p3", true).await);
    let cascade = |n: &str| if n == "m_p1" { c1.clone() } else { c3.clone() };
    assert_eq!(f.preview(&cascade("m_p1")).await.risk, Risk::DataLoss);
    // Önce genel onay (5), sonra üretim onayı (6).
    assert_eq!(code(&f.apply(&cascade("m_p1")).await), "confirmation_required");
    assert_eq!(code(&f.apply_with(&cascade("m_p1"), |o| o.confirmed = true).await), "production_confirmation_required");
    assert!(exists(&env, "m_p1.t").await);
    applied(f.apply_with(&cascade("m_p1"), |o| (o.confirmed, o.confirmed_production) = (true, true)).await);
    assert!(!exists(&env, "m_p1").await);
    // Yalnızca yıkıcı (veri kaybı yok) plan üretimde ek onay istemez.
    let p2 = f.drop_schema(&env, "m_p2", false).await;
    applied(f.apply_with(&p2, |o| o.confirmed = true).await);
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
    let drop = f.drop_schema(&env, "m_sc", false).await;
    assert_eq!(code(&f.dm.apply(f.server, f.db, &drop, opts("bogus")).await), "plan_changed");
    // Boş plan: kodsuz `invalid`.
    let noop = f.alter_schema(&env, "m_sc", |_| {}).await;
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
    let req = f.alter_schema(&env, "m_ta", |n| (n.name, n.owner) = ("m_tb".into(), Some("m_no_such_role".into()))).await;
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
    assert!(elapsed < Duration::from_secs(5), "lock_timeout beklenenden uzun: {elapsed:?}");
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

// ---- iptal: Tokio bağlamsız iş parçacığı, gelecek düşmesi, kayıttan önce gelen iptal ----

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cancel_all_works_from_a_plain_thread() {
    for ts in servers() {
        guarded(cancel_all_body(ts.clone()), cleanup(&ts, "m_cancelall")).await;
    }
}

async fn cancel_all_body(ts: TestServer) {
    let env = setup(&ts, "m_cancelall").await;
    let f = fx(&env, &ts, false, None);
    env.sql("CREATE TABLE public.ca (id int)").await;
    let locker = lock_table(&ts, &env, "ca").await;
    let req = create(index("ca", "ca_idx"));
    let o = ApplyOptions { lock_timeout_ms: 0, ..opts(&f.preview(&req).await.hash) };
    let (dm, server, db) = (f.dm.clone(), f.server, f.db);
    let task = tokio::spawn(async move { dm.apply(server, db, &req, o).await });
    wait_for_waiting(&env.admin, 1).await;
    // Tauri ana iş parçacığı gibi: ortam Tokio bağlamı olmayan düz iş parçacığı.
    let dm = f.dm.clone();
    std::thread::spawn(move || {
        assert!(tokio::runtime::Handle::try_current().is_err());
        dm.cancel_all();
    })
    .join()
    .unwrap();
    let (idx, committed, sqlstate, _) = failed(tokio::time::timeout(Duration::from_secs(10), task).await.expect("iptal ulaşmadı").unwrap());
    assert_eq!((idx, committed, sqlstate.as_deref()), (0, 0, Some("57014")));
    locker.batch_execute("COMMIT").await.unwrap();
    assert!(!exists(&env, "public.ca_idx").await);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn dropping_the_apply_future_cancels_the_server_statement() {
    for ts in servers() {
        guarded(drop_future_body(ts.clone()), cleanup(&ts, "m_dropfut")).await;
    }
}

async fn drop_future_body(ts: TestServer) {
    let env = setup(&ts, "m_dropfut").await;
    let f = fx(&env, &ts, false, None);
    env.sql("CREATE TABLE public.df (id int)").await;
    let locker = lock_table(&ts, &env, "df").await;
    let req = create(index("df", "df_idx"));
    let o = ApplyOptions { lock_timeout_ms: 0, ..opts(&f.preview(&req).await.hash) };
    let (dm, server, db) = (f.dm.clone(), f.server, f.db);
    let task = tokio::spawn(async move { dm.apply(server, db, &req, o).await });
    wait_for_waiting(&env.admin, 1).await;
    task.abort();
    assert!(task.await.unwrap_err().is_cancelled());
    // Kilit hâlâ tutuluyor: yalnızca iptal (bağlantı kapanışı değil) bekleyen ifadeyi bitirir.
    wait_for_waiting(&env.admin, 0).await;
    locker.batch_execute("COMMIT").await.unwrap();
    assert!(!exists(&env, "public.df_idx").await);
}

#[tokio::test]
async fn cancel_before_apply_registers_still_cancels() {
    for ts in servers() {
        guarded(early_cancel_body(ts.clone()), cleanup(&ts, "m_earlycn")).await;
    }
}

async fn early_cancel_body(ts: TestServer) {
    let env = setup(&ts, "m_earlycn").await;
    let f = fx(&env, &ts, false, None);
    let id = Uuid::new_v4();
    f.dm.cancel(id).await.unwrap(); // henüz kayıtlı değil: mezar taşı
    let r = f.apply_with(&create(schema("m_early")), |o| o.apply_id = id).await;
    let (idx, committed, sqlstate, _) = failed(r);
    assert_eq!((idx, committed, sqlstate.as_deref()), (0, 0, Some("57014")));
    assert!(!exists(&env, "m_early").await);
    // Mezar taşı tüketildi: aynı kimlik yeniden kullanılırsa normal uygulanır.
    applied(f.apply_with(&create(schema("m_early")), |o| o.apply_id = id).await);
    assert!(exists(&env, "m_early").await);
}

// ---- Applied.created ----

#[tokio::test]
async fn applied_reports_the_created_object() {
    for ts in servers() {
        guarded(created_body(ts.clone()), cleanup(&ts, "m_created")).await;
    }
}

async fn created_body(ts: TestServer) {
    let env = setup(&ts, "m_created").await;
    let f = fx(&env, &ts, false, None);
    let created = |r: Result<ApplyOutcome>| match r {
        Ok(ApplyOutcome::Applied { created, .. }) => created,
        other => panic!("uygulanmalıydı: {other:?}"),
    };

    let c = created(f.apply(&create(schema("m_cr_s"))).await);
    assert_eq!(c, Some(ObjectRef { kind: ObjectKind::Schema, oid: env.schema_oid("m_cr_s").await }));

    let seq = ObjectDef::Sequence(SequenceDef {
        schema: "m_cr_s".into(),
        name: "m_cr_seq".into(),
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
    });
    let c = created(f.apply(&create(seq)).await);
    assert_eq!(c, Some(ObjectRef { kind: ObjectKind::Sequence, oid: env.class_oid("m_cr_s", "m_cr_seq").await }));

    env.sql("CREATE TABLE public.cr_t (id int)").await;
    let c = created(f.apply(&create(index("cr_t", "cr_idx"))).await);
    assert_eq!(c, Some(ObjectRef { kind: ObjectKind::Index, oid: env.class_oid("public", "cr_idx").await }));

    // Eklenti: contrib kurulu değilse atlanır.
    let ext: Option<String> = env
        .admin
        .query_opt("SELECT name::text FROM pg_available_extensions WHERE name IN ('citext', 'pgcrypto') ORDER BY name LIMIT 1", &[])
        .await
        .unwrap()
        .map(|r| r.get(0));
    if let Some(name) = ext {
        let def = ObjectDef::Extension(ExtensionDef { name: name.clone(), schema: None, version: None, cascade: false });
        let c = created(f.apply(&create(def)).await);
        let oid: u32 = env.admin.query_one("SELECT oid FROM pg_extension WHERE extname = $1", &[&name]).await.unwrap().get(0);
        assert_eq!(c, Some(ObjectRef { kind: ObjectKind::Extension, oid }));
    }

    // Drop: created yok.
    let drop = f.drop_schema(&env, "m_cr_s", true).await;
    let r = f.apply_with(&drop, |o| o.confirmed = true).await;
    assert_eq!(created(r), None);
}

// ---- hata konumu ----

#[tokio::test]
async fn failed_apply_reports_the_error_position() {
    for ts in servers() {
        guarded(position_body(ts.clone()), cleanup(&ts, "m_position")).await;
    }
}

async fn position_body(ts: TestServer) {
    let env = setup(&ts, "m_position").await;
    let f = fx(&env, &ts, false, None);
    env.sql("CREATE TABLE public.ps (id int)").await;
    let mut d = index("ps", "ps_idx");
    let ObjectDef::Index(i) = &mut d else { unreachable!() };
    i.columns = vec![IndexColumn { key: IndexKey::Expression { sql: "nosuch_col + 1".into() }, opclass: None, collation: None, descending: false, nulls_first: None }];
    let req = create(d);
    let sql = f.preview(&req).await.statements[0].sql.clone();
    let r = f.apply(&req).await;
    let ApplyOutcome::Failed { sqlstate, position, .. } = r.unwrap() else { panic!("Failed bekleniyordu") };
    assert_eq!(sqlstate.as_deref(), Some("42703"));
    let pos = position.expect("hata konumu") as usize;
    // Konum, çalıştırılan ifade metnindeki 1 tabanlı karakter sapmasıdır ve hatalı sütun adını gösterir.
    assert!(sql.chars().skip(pos - 1).collect::<String>().starts_with("nosuch_col"), "{pos}: {sql}");
}

// ---- geçersiz indeks bayrağı ----

#[tokio::test]
async fn load_reports_index_validity() {
    for ts in servers() {
        guarded(valid_body(ts.clone()), cleanup(&ts, "m_valid")).await;
    }
}

async fn valid_body(ts: TestServer) {
    let env = setup(&ts, "m_valid").await;
    let f = fx(&env, &ts, false, None);
    env.sql("CREATE TABLE public.vv (id int)").await;
    applied(f.apply(&create(index("vv", "vv_idx"))).await);
    let oid = env.class_oid("public", "vv_idx").await;
    let load = || async { f.dm.load(f.server, f.db, ObjectRef { kind: ObjectKind::Index, oid }).await.unwrap() };
    let ObjectDef::Index(l) = load().await else { panic!() };
    assert!(l.valid);
    // Başarısız CONCURRENTLY inşasının bıraktığı durumu taklit eder.
    env.sql("UPDATE pg_index SET indisvalid = false WHERE indexrelid = 'public.vv_idx'::regclass").await;
    let ObjectDef::Index(l) = load().await else { panic!() };
    assert!(!l.valid);
    // `valid` alter farkına girmez: yalnızca geçerlilik bayrağı farklı iki tanım arasında plan boş.
    let req = PlanRequest::Alter {
        object: ObjectRef { kind: ObjectKind::Index, oid },
        old: ObjectDef::Index(l.clone()),
        new: ObjectDef::Index(IndexDef { valid: true, ..l }),
    };
    let r = f.preview(&req).await;
    assert!(r.empty && r.errors.is_empty());
}

// ---- nesne denetimi: sunucu istemcinin old/def alanlarına güvenmez ----

#[tokio::test]
async fn apply_verifies_the_client_definition_against_the_server() {
    for ts in servers() {
        guarded(object_check_body(ts.clone()), cleanup(&ts, "m_objchk")).await;
    }
}

async fn object_check_body(ts: TestServer) {
    let env = setup(&ts, "m_objchk").await;
    let f = fx(&env, &ts, false, None);
    env.sql("CREATE SCHEMA m_oc; COMMENT ON SCHEMA m_oc IS 'gercek'").await;

    let honest = f.alter_schema(&env, "m_oc", |n| (n.name, n.comment) = ("m_oc2".into(), Some("yeni".into()))).await;
    // Tahrif: `old` farklı yorum taşıyor -> plan_changed; hiçbir şey yazılmaz.
    let PlanRequest::Alter { object, old, new } = honest.clone() else { unreachable!() };
    let ObjectDef::Schema(mut tampered) = old.clone() else { unreachable!() };
    tampered.comment = Some("sahte".into());
    let bad = PlanRequest::Alter { object, old: ObjectDef::Schema(tampered), new: new.clone() };
    assert_eq!(code(&f.apply(&bad).await), "plan_changed");
    assert!(exists(&env, "m_oc").await && !exists(&env, "m_oc2").await);
    // Tür uyuşmazlığı (object = indeks, tanım = şema): geçersiz tanım.
    let wrong_kind = PlanRequest::Alter { object: ObjectRef { kind: ObjectKind::Index, oid: object.oid }, old: old.clone(), new: new.clone() };
    assert_eq!(code(&f.apply(&wrong_kind).await), "invalid_def");
    // Önizleme nesneye bakmaz (tahrifli istekte de plan üretir).
    assert_eq!(f.preview(&bad).await.statements.len(), 2);
    // Bayat yeniden adlandırma: nesne yüklendikten sonra başkası adını değiştirdi -> plan_changed.
    env.sql("ALTER SCHEMA m_oc RENAME TO m_oc_renamed").await;
    assert_eq!(code(&f.apply(&honest).await), "plan_changed");
    assert!(exists(&env, "m_oc_renamed").await && !exists(&env, "m_oc2").await);
    // Yok olan nesne: yükleme hatası olduğu gibi geçer.
    env.sql("DROP SCHEMA m_oc_renamed").await;
    assert_eq!(code(&f.apply(&honest).await), "not_found");

    // Dürüst akış: yeniden yükle -> uygula -> yeni kimlik.
    env.sql("CREATE SCHEMA m_oc3; COMMENT ON SCHEMA m_oc3 IS 'a'").await;
    let honest = f.alter_schema(&env, "m_oc3", |n| n.name = "m_oc4".into()).await;
    let ApplyOutcome::Applied { created, .. } = f.apply(&honest).await.unwrap() else { panic!() };
    assert_eq!(created, Some(ObjectRef { kind: ObjectKind::Schema, oid: env.schema_oid("m_oc4").await }));

    // Silme: tahrifli `def` reddedilir, dürüst olan çalışır.
    env.sql("CREATE SCHEMA m_oc5").await;
    let PlanRequest::Drop { object, def, cascade, .. } = f.drop_schema(&env, "m_oc5", false).await else { unreachable!() };
    let ObjectDef::Schema(mut sd) = def.clone() else { unreachable!() };
    sd.comment = Some("sahte".into());
    let bad = PlanRequest::Drop { object, def: ObjectDef::Schema(sd), cascade, force: false };
    assert_eq!(code(&f.apply_with(&bad, |o| o.confirmed = true).await), "plan_changed");
    assert!(exists(&env, "m_oc5").await);
    applied(f.apply_with(&PlanRequest::Drop { object, def, cascade, force: false }, |o| o.confirmed = true).await);
    assert!(!exists(&env, "m_oc5").await);
}

#[tokio::test]
async fn system_objects_and_constraint_indexes_are_refused() {
    for ts in servers() {
        guarded(system_body(ts.clone()), cleanup(&ts, "m_sysobj")).await;
    }
}

async fn system_body(ts: TestServer) {
    let env = setup(&ts, "m_sysobj").await;
    let f = fx(&env, &ts, false, None);
    // plpgsql eklentisi sistem nesnesidir: yükleme reddeder, istemci tanımı ne derse desin silinemez.
    let object = ObjectRef { kind: ObjectKind::Extension, oid: env.extension_oid("plpgsql").await };
    let def = ObjectDef::Extension(ExtensionDef { name: "plpgsql".into(), schema: None, version: None, cascade: false });
    let req = PlanRequest::Drop { object, def, cascade: false, force: false };
    let r = f.apply_with(&req, |o| o.confirmed = true).await;
    assert_eq!(code(&r), "invalid");
    let still: bool = env.admin.query_one("SELECT EXISTS (SELECT 1 FROM pg_extension WHERE extname = 'plpgsql')", &[]).await.unwrap().get(0);
    assert!(still);

    // PK indeksi: yüklenen tanım `constraint` taşır; istemci `constraint: null` gönderirse reddedilir.
    env.sql("CREATE TABLE public.pk_t (id int PRIMARY KEY)").await;
    let oid = env.class_oid("public", "pk_t_pkey").await;
    let object = ObjectRef { kind: ObjectKind::Index, oid };
    let ObjectDef::Index(loaded) = f.dm.load(f.server, f.db, object).await.unwrap() else { panic!() };
    assert_eq!(loaded.constraint.as_deref(), Some("pk_t_pkey"));
    let liar = IndexDef { constraint: None, ..loaded.clone() };
    let req = PlanRequest::Drop { object, def: ObjectDef::Index(liar), cascade: false, force: false };
    assert_eq!(code(&f.apply_with(&req, |o| o.confirmed = true).await), "plan_changed");
    // Dürüst tanım: üretici kısıt korumasıyla reddeder (invalid_def).
    let req = PlanRequest::Drop { object, def: ObjectDef::Index(loaded), cascade: false, force: false };
    assert_eq!(code(&f.apply_with(&req, |o| o.confirmed = true).await), "invalid_def");
    assert!(exists(&env, "public.pk_t_pkey").await);
}

// ================================================================================================================
// Tablo: yönetici entegrasyonu (defaults, created, nesne denetimi, kapılar, lock_timeout, koruma)
// ================================================================================================================

fn col(name: &str, ty: &str) -> ColumnDef {
    ColumnDef {
        attnum: None,
        name: name.into(),
        data_type: ty.into(),
        not_null: false,
        default: None,
        identity: None,
        generated: None,
        collation: None,
        comment: None,
        using: None,
    }
}

fn names(v: &[&str]) -> Vec<String> {
    v.iter().map(|x| x.to_string()).collect()
}

fn named(name: &str, kind: ConstraintKind) -> ConstraintDef {
    ConstraintDef { oid: None, name: Some(name.into()), kind, deferrable: false, initially_deferred: false, not_valid: false }
}

fn unique(cols: &[&str]) -> ConstraintKind {
    ConstraintKind::Unique { columns: names(cols), include: vec![], nulls_not_distinct: false }
}

fn check(expr: &str) -> ConstraintKind {
    ConstraintKind::Check { expr: expr.into(), no_inherit: false }
}

fn tbl(d: &TableDef) -> ObjectDef {
    ObjectDef::Table(d.clone())
}

fn col_mut<'a>(d: &'a mut TableDef, name: &str) -> &'a mut ColumnDef {
    d.columns.iter_mut().find(|c| c.name == name).unwrap_or_else(|| panic!("kolon yok: {name}"))
}

/// Uygulanan tanımın sunucudaki karşılığı: yeni kolon/kısıtın attnum/oid'i ve belirtilmeyen sahip yüklenenden;
/// kısıtlar ad sırasıyla (yükleyicinin sırası).
fn settled(want: &TableDef, loaded: &TableDef) -> TableDef {
    let mut e = want.clone();
    if e.owner.is_none() {
        e.owner = loaded.owner.clone();
    }
    e.partition_by = None;
    for c in &mut e.columns {
        if c.attnum.is_none() {
            c.attnum = loaded.columns.iter().find(|l| l.name == c.name).and_then(|l| l.attnum);
        }
        c.using = None;
    }
    for k in &mut e.constraints {
        if k.oid.is_none() {
            k.oid = loaded.constraints.iter().find(|l| l.name == k.name).and_then(|l| l.oid);
        }
        k.not_valid = false;
    }
    e.constraints.sort_by(|a, b| a.name.cmp(&b.name));
    e
}

impl Fx {
    async fn load_table(&self, env: &Env, schema: &str, name: &str) -> (ObjectRef, TableDef) {
        let object = ObjectRef { kind: ObjectKind::Table, oid: env.class_oid(schema, name).await };
        (object, self.reload(object).await)
    }

    async fn reload(&self, object: ObjectRef) -> TableDef {
        match self.dm.load(self.server, self.db, object).await {
            Ok(ObjectDef::Table(d)) => d,
            other => panic!("tablo yüklenemedi: {other:?}"),
        }
    }

    /// `public.<name>` tablosunu yükler, `edit` ile değiştirir: (istek, nesne, eski, yeni).
    async fn alter_table(&self, env: &Env, name: &str, edit: impl FnOnce(&mut TableDef)) -> (PlanRequest, ObjectRef, TableDef, TableDef) {
        let (object, old) = self.load_table(env, "public", name).await;
        let mut new = old.clone();
        edit(&mut new);
        (PlanRequest::Alter { object, old: tbl(&old), new: tbl(&new) }, object, old, new)
    }
}

fn created_of(r: Result<ApplyOutcome>) -> Option<ObjectRef> {
    match r {
        Ok(ApplyOutcome::Applied { created, .. }) => created,
        other => panic!("uygulanmalıydı: {other:?}"),
    }
}

#[tokio::test]
async fn table_defaults_create_and_drop_through_the_manager() {
    for ts in servers() {
        guarded(table_create_body(ts.clone()), cleanup(&ts, "m_tbcreate")).await;
    }
}

async fn table_create_body(ts: TestServer) {
    let env = setup(&ts, "m_tbcreate").await;
    let f = fx(&env, &ts, false, None);
    let public = env.schema_oid("public").await;
    let ctx = |schema_oid, table_oid| CreateContext { schema_oid, table_oid };

    // Varsayılan: şema bağlamından, boş ad/kolon/kısıt, kalıcı; yükleme alanları boş.
    let ObjectDef::Table(d) = f.dm.defaults(f.server, f.db, ObjectKind::Table, ctx(Some(public), None)).await.unwrap() else {
        panic!("tablo bekleniyordu")
    };
    assert_eq!((d.schema.as_str(), d.name.as_str(), d.persistence), ("public", "", Persistence::Permanent));
    assert!(d.columns.is_empty() && d.constraints.is_empty() && d.unsupported.is_empty());
    assert_eq!((&d.owner, &d.tablespace, &d.partition_by, d.fillfactor, &d.comment), (&None, &None, &None, None, &None));
    assert_eq!((&d.partition_text, d.is_partition, d.inherits), (&None, false, false));
    // Geçersiz bağlamlar.
    env.sql("CREATE TABLE public.other (id int)").await;
    let other = env.class_oid("public", "other").await;
    for c in [ctx(None, None), ctx(Some(public), Some(other)), ctx(None, Some(other))] {
        let r = f.dm.defaults(f.server, f.db, ObjectKind::Table, c).await;
        assert!(matches!(r, Err(Error::Invalid(_))), "{c:?}: {r:?}");
    }
    let missing = f.dm.defaults(f.server, f.db, ObjectKind::Table, ctx(Some(u32::MAX - 7), None)).await;
    assert!(matches!(missing, Err(Error::NotFound(_))), "{missing:?}");

    // create → apply → created dolu → load eşit.
    let mut def = d;
    def.name = "Tb \"c\"".into();
    def.comment = Some("tablo".into());
    def.columns = vec![
        ColumnDef { identity: Some(IdentityKind::Always), not_null: true, ..col("id", "bigint") },
        ColumnDef { default: Some("0".into()), not_null: true, ..col("qty", "integer") },
        ColumnDef { comment: Some("fiyat".into()), ..col("price", "numeric(10,2)") },
    ];
    def.constraints = vec![
        named("tb_pkey", ConstraintKind::PrimaryKey { columns: names(&["id"]), include: vec![] }),
        named("tb_qty_check", check("qty >= 0")),
    ];
    let req = create(tbl(&def));
    assert_eq!(f.preview(&req).await.risk, Risk::Safe);
    let c = created_of(f.apply(&req).await);
    let object = ObjectRef { kind: ObjectKind::Table, oid: env.class_oid("public", "Tb \"c\"").await };
    assert_eq!(c, Some(object));
    let loaded = f.reload(object).await;
    let mut want = settled(&def, &loaded);
    // CHECK ifadesi sunucunun normalleştirmesiyle (`pg_get_constraintdef`) yüklenir.
    let ck = loaded.constraints.iter().find(|k| k.name.as_deref() == Some("tb_qty_check")).unwrap();
    want.constraints.iter_mut().find(|k| k.name.as_deref() == Some("tb_qty_check")).unwrap().kind = ck.kind.clone();
    assert_eq!(loaded, want);
    // Aynı create ikinci kez: yürütme hatası (42P07), kapı değil.
    let (_, committed, sqlstate, _) = failed(f.apply(&req).await);
    assert_eq!((committed, sqlstate.as_deref()), (0, Some("42P07")));

    // Drop: onaysız reddedilir; onaylı uygulanır, created yok.
    let drop = PlanRequest::Drop { object, def: tbl(&loaded), cascade: false, force: false };
    assert_eq!(f.preview(&drop).await.risk, Risk::DataLoss);
    assert_eq!(code(&f.apply(&drop).await), "confirmation_required");
    assert!(exists(&env, "public.Tb \"c\"").await);
    assert_eq!(created_of(f.apply_with(&drop, |o| o.confirmed = true).await), None);
    assert!(!exists(&env, "public.Tb \"c\"").await);
    // Silinmiş tabloyu yeniden silme: yükleme hatası olduğu gibi (not_found).
    assert_eq!(code(&f.apply_with(&drop, |o| o.confirmed = true).await), "not_found");

    // CASCADE: bağımlı FK düşer (DataLoss); CASCADE'siz silme sunucuda 2BP01.
    env.sql("CREATE TABLE public.par (id int PRIMARY KEY); CREATE TABLE public.chi (p int CONSTRAINT chi_fk REFERENCES public.par)").await;
    let (object, par) = f.load_table(&env, "public", "par").await;
    let plain = PlanRequest::Drop { object, def: tbl(&par), cascade: false, force: false };
    let (_, committed, sqlstate, _) = failed(f.apply_with(&plain, |o| o.confirmed = true).await);
    assert_eq!((committed, sqlstate.as_deref()), (0, Some("2BP01")));
    let cascade = PlanRequest::Drop { object, def: tbl(&par), cascade: true, force: false };
    assert_eq!(f.preview(&cascade).await.risk, Risk::DataLoss);
    applied(f.apply_with(&cascade, |o| o.confirmed = true).await);
    assert!(!exists(&env, "public.par").await);
    let (_, chi) = f.load_table(&env, "public", "chi").await;
    assert!(chi.constraints.is_empty(), "CASCADE bağımlı FK'yi düşürür: {:?}", chi.constraints);
}

#[tokio::test]
async fn table_alter_applies_and_reloads_equal() {
    for ts in servers() {
        guarded(table_alter_body(ts.clone()), cleanup(&ts, "m_tbalter")).await;
    }
}

async fn table_alter_body(ts: TestServer) {
    let env = setup(&ts, "m_tbalter").await;
    let f = fx(&env, &ts, false, None);
    env.sql("CREATE TABLE public.ta (id int PRIMARY KEY, b text, CONSTRAINT ta_b_key UNIQUE (b))").await;
    let (_, before) = f.load_table(&env, "public", "ta").await;
    let uq_oid = before.constraints.iter().find(|k| k.name.as_deref() == Some("ta_b_key")).unwrap().oid;
    assert!(uq_oid.is_some());

    // Kolon ekle + kısıt ekle + kolon yeniden adlandır (kısıt kolon listesi eşlemeyle izlenir).
    let (req, object, _, new) = f
        .alter_table(&env, "ta", |n| {
            col_mut(n, "b").name = "b2".into();
            for k in &mut n.constraints {
                if let ConstraintKind::Unique { columns, .. } = &mut k.kind {
                    *columns = names(&["b2"]);
                }
            }
            n.columns.push(col("c", "text"));
            n.constraints.push(named("ta_c_key", unique(&["c"])));
        })
        .await;
    let v = f.preview(&req).await;
    assert_eq!((v.risk, v.errors.len()), (Risk::Safe, 0));
    assert!(!v.statements.iter().any(|s| s.sql.contains("DROP CONSTRAINT")), "eşlenen kısıt yeniden kurulmamalı: {:?}", v.statements);
    // Kimlik (şema/ad) değişmedi: created yok.
    assert_eq!(created_of(f.apply(&req).await), None);
    let loaded = f.reload(object).await;
    assert_eq!(loaded, settled(&new, &loaded));
    let uq = loaded.constraints.iter().find(|k| k.name.as_deref() == Some("ta_b_key")).unwrap();
    assert_eq!((uq.oid, &uq.kind), (uq_oid, &unique(&["b2"])), "kısıt oid'i kolon yeniden adlandırmada korunur");

    // Tablo yeniden adlandırma + yorum: created = aynı oid (tablo kimliği yeni adla bulunur).
    let (req, object, _, new) = f.alter_table(&env, "ta", |n| (n.name, n.comment) = ("ta_new".into(), Some("y".into()))).await;
    assert_eq!(created_of(f.apply(&req).await), Some(object));
    assert_eq!(f.reload(object).await, new);

    // Kısıtlı kolonu sil + kısıtları kaldır aynı düzenlemede: plan sırası gerçek sunucuda geçerli.
    env.sql(
        "CREATE TABLE public.tx (id int, x int, CONSTRAINT tx_x_check CHECK (x > 0), CONSTRAINT tx_x_key UNIQUE (x), \
         CONSTRAINT tx_idx_key UNIQUE (id, x))",
    )
    .await;
    let (req, object, _, new) = f
        .alter_table(&env, "tx", |n| {
            n.columns.retain(|c| c.name != "x");
            n.constraints.clear();
        })
        .await;
    let v = f.preview(&req).await;
    assert_eq!(v.risk, Risk::DataLoss);
    let pos = |needle: &str| v.statements.iter().position(|s| s.sql.contains(needle)).unwrap_or_else(|| panic!("{needle}: {:?}", v.statements));
    assert!(pos("DROP CONSTRAINT") < pos("DROP COLUMN"));
    applied(f.apply_with(&req, |o| o.confirmed = true).await);
    let loaded = f.reload(object).await;
    assert_eq!(loaded, new);
    assert!(loaded.constraints.is_empty() && loaded.columns.len() == 1);
}

#[tokio::test]
async fn table_object_check_rejects_stale_and_tampered_definitions() {
    for ts in servers() {
        guarded(table_stale_body(ts.clone()), cleanup(&ts, "m_tbstale")).await;
    }
}

async fn table_stale_body(ts: TestServer) {
    let env = setup(&ts, "m_tbstale").await;
    let f = fx(&env, &ts, false, None);
    env.sql("CREATE TABLE public.ts1 (id int PRIMARY KEY, a int, b int, CONSTRAINT ts1_a_check CHECK (a > 0))").await;
    let (honest, object, old, new) = f.alter_table(&env, "ts1", |n| n.columns.push(col("c", "text"))).await;

    // Tahrifler: her biri plan üretiminden geçer ama `old` sunucudaki tanıma eşit değildir → plan_changed.
    let tampered = |edit: &dyn Fn(&mut TableDef)| {
        let (mut o, mut n) = (old.clone(), new.clone());
        edit(&mut o);
        edit(&mut n);
        PlanRequest::Alter { object, old: tbl(&o), new: tbl(&n) }
    };
    let swap_attnums = |d: &mut TableDef| {
        let (x, y) = (col_mut(d, "a").attnum, col_mut(d, "b").attnum);
        col_mut(d, "a").attnum = y;
        col_mut(d, "b").attnum = x;
    };
    let mut inherited = old.clone();
    inherited.inherits = true;
    let cases: Vec<(&str, PlanRequest)> = vec![
        ("fazladan desteklenmeyen öğe", tampered(&|d| d.unsupported.push("EXCLUDE kısıtı sahte".into()))),
        ("kısıt listesinden çıkarılmış", tampered(&|d| d.constraints.retain(|k| k.name.as_deref() != Some("ts1_a_check")))),
        ("değiştirilmiş attnum", tampered(&swap_attnums)),
        ("değiştirilmiş kısıt oid'i", tampered(&|d| d.constraints[0].oid = d.constraints[0].oid.map(|o| o + 1))),
        (
            "sahte kalıtım bayrağı",
            PlanRequest::Alter { object, old: tbl(&inherited), new: tbl(&TableDef { comment: Some("x".into()), ..inherited.clone() }) },
        ),
    ];
    for (what, req) in &cases {
        assert!(f.preview(req).await.errors.is_empty(), "{what}: plan üretilebilmeli");
        assert_eq!(code(&f.apply_with(req, |o| o.confirmed = true).await), "plan_changed", "{what}");
    }
    assert_eq!(f.reload(object).await, old, "tahrifli istekler hiçbir şey yazmamalı");

    // Bayat `old`: başka oturum kolon ekledi → plan_changed; istek uygulanmaz.
    env.sql("ALTER TABLE public.ts1 ADD COLUMN z int").await;
    assert_eq!(code(&f.apply(&honest).await), "plan_changed");
    let now = f.reload(object).await;
    assert!(now.columns.iter().any(|c| c.name == "z") && !now.columns.iter().any(|c| c.name == "c"));
    // Yeniden yükleyip aynı düzenleme: uygulanır.
    let (req, object, _, new) = f.alter_table(&env, "ts1", |n| n.columns.push(col("c", "text"))).await;
    applied(f.apply(&req).await);
    let loaded = f.reload(object).await;
    assert_eq!(loaded, settled(&new, &loaded));
}

#[tokio::test]
async fn table_gates_confirmation_production_and_lock() {
    for ts in servers() {
        guarded(table_gates_body(ts.clone()), cleanup(&ts, "m_tbgates")).await;
    }
}

async fn table_gates_body(ts: TestServer) {
    let env = setup(&ts, "m_tbgates").await;
    let prod = fx(&env, &ts, false, Some("Üretim"));
    env.sql("CREATE TABLE public.tg (id int, gone text, nn int)").await;

    // DROP COLUMN: DataLoss → genel onay, sonra üretim onayı.
    let (req, object, _, new) = prod.alter_table(&env, "tg", |n| n.columns.retain(|c| c.name != "gone")).await;
    assert_eq!(prod.preview(&req).await.risk, Risk::DataLoss);
    assert_eq!(code(&prod.apply(&req).await), "confirmation_required");
    assert_eq!(code(&prod.apply_with(&req, |o| o.confirmed = true).await), "production_confirmation_required");
    assert!(prod.reload(object).await.columns.iter().any(|c| c.name == "gone"));
    applied(prod.apply_with(&req, |o| (o.confirmed, o.confirmed_production) = (true, true)).await);
    assert_eq!(prod.reload(object).await, new);

    // SET NOT NULL: yalnızca yıkıcı → üretimde de genel onay yeter.
    let (req, object, _, new) = prod.alter_table(&env, "tg", |n| col_mut(n, "nn").not_null = true).await;
    assert_eq!(prod.preview(&req).await.risk, Risk::Destructive);
    assert_eq!(code(&prod.apply(&req).await), "confirmation_required");
    applied(prod.apply_with(&req, |o| o.confirmed = true).await);
    assert_eq!(prod.reload(object).await, new);

    // Kilitli sunucu: tablo create/alter `locked`; önizleme serbest.
    let locked = fx(&env, &ts, true, None);
    let mut def = prod.reload(object).await;
    def.name = "tg_new".into();
    def.columns = vec![col("id", "int")];
    def.constraints.clear();
    let c = create(tbl(&def));
    assert!(locked.preview(&c).await.errors.is_empty());
    assert_eq!(code(&locked.apply(&c).await), "locked");
    assert!(!exists(&env, "public.tg_new").await);
    let (req, ..) = locked.alter_table(&env, "tg", |n| n.comment = Some("x".into())).await;
    assert_eq!(code(&locked.apply(&req).await), "locked");
}

#[tokio::test]
async fn table_alter_respects_lock_timeout() {
    for ts in servers() {
        guarded(table_lockto_body(ts.clone()), cleanup(&ts, "m_tblockto")).await;
    }
}

async fn table_lockto_body(ts: TestServer) {
    let env = setup(&ts, "m_tblockto").await;
    let f = fx(&env, &ts, false, None);
    env.sql("CREATE TABLE public.tl (id int)").await;
    let (req, object, old, new) = f.alter_table(&env, "tl", |n| n.columns.push(col("c", "text"))).await;
    let locker = lock_table(&ts, &env, "tl").await;
    let hash = f.preview(&req).await.hash;
    // Nesne denetimi katalogdan okur (kilit beklemez); ALTER TABLE kilidi bekler ve zaman aşımına uğrar.
    let started = Instant::now();
    let r = f.dm.apply(f.server, f.db, &req, ApplyOptions { lock_timeout_ms: 500, ..opts(&hash) }).await;
    let (idx, committed, sqlstate, _) = failed(r);
    assert_eq!((idx, committed, sqlstate.as_deref()), (0, 0, Some("55P03")));
    assert!(started.elapsed() < Duration::from_secs(5), "lock_timeout beklenenden uzun");
    assert_eq!(ddl_waiting(&env.admin).await, 0);
    locker.batch_execute("COMMIT").await.unwrap();
    assert_eq!(f.reload(object).await, old, "zaman aşımında değişiklik olmamalı");
    applied(f.dm.apply(f.server, f.db, &req, ApplyOptions { lock_timeout_ms: 500, ..opts(&hash) }).await);
    let loaded = f.reload(object).await;
    assert_eq!(loaded, settled(&new, &loaded));
}

#[tokio::test]
async fn system_and_partition_tables_are_protected() {
    for ts in servers() {
        guarded(table_protect_body(ts.clone()), cleanup(&ts, "m_tbprot")).await;
    }
}

async fn table_protect_body(ts: TestServer) {
    let env = setup(&ts, "m_tbprot").await;
    let f = fx(&env, &ts, false, None);
    // Sistem tablosu: yükleme `Invalid`; istemci tanımı ne derse desin silinemez/değiştirilemez.
    let object = ObjectRef { kind: ObjectKind::Table, oid: env.class_oid("pg_catalog", "pg_class").await };
    assert!(matches!(f.dm.load(f.server, f.db, object).await, Err(Error::Invalid(_))));
    let fake = TableDef {
        schema: "public".into(),
        name: "pg_class".into(),
        owner: None,
        tablespace: None,
        persistence: Persistence::Permanent,
        columns: vec![],
        constraints: vec![],
        partition_by: None,
        fillfactor: None,
        comment: None,
        partition_text: None,
        is_partition: false,
        inherits: false,
        unsupported: vec![],
    };
    let r = f.apply_with(&PlanRequest::Drop { object, def: tbl(&fake), cascade: true, force: false }, |o| (o.confirmed, o.confirmed_production) = (true, true)).await;
    assert_eq!(code(&r), "invalid");
    let alter = PlanRequest::Alter { object, old: tbl(&fake), new: tbl(&TableDef { comment: Some("x".into()), ..fake.clone() }) };
    assert_eq!(code(&f.apply(&alter).await), "invalid");
    assert!(exists(&env, "pg_catalog.pg_class").await);

    // Bölüm: kolon/kısıt değişikliği `invalid_def`; yeniden adlandırma serbest (created = aynı oid).
    env.sql("CREATE TABLE public.pp (k int, v text) PARTITION BY LIST (k); CREATE TABLE public.pp1 PARTITION OF public.pp FOR VALUES IN (1)").await;
    let (req, object, old, _) = f.alter_table(&env, "pp1", |n| n.columns.push(col("extra", "int"))).await;
    assert!(old.is_partition);
    assert_eq!(code(&f.apply(&req).await), "invalid_def");
    let (req, ..) = f.alter_table(&env, "pp1", |n| col_mut(n, "v").not_null = true).await;
    assert_eq!(code(&f.apply_with(&req, |o| o.confirmed = true).await), "invalid_def");
    let (req, ..) = f.alter_table(&env, "pp1", |n| n.constraints.push(named("pp1_ck", check("k > 0")))).await;
    assert_eq!(code(&f.apply(&req).await), "invalid_def");
    assert_eq!(f.reload(object).await, old);
    let (req, object, _, new) = f.alter_table(&env, "pp1", |n| n.name = "pp_one".into()).await;
    assert_eq!(created_of(f.apply(&req).await), Some(object));
    assert_eq!(f.reload(object).await, new);
}

// ================================================================================================================
// Plan 14: View / MatView / Database / Role yönetici entegrasyonu (kapsam, parola kapısı, bakım veritabanı)
// ================================================================================================================

use common::{cleanup_cluster, cluster_name};
use pgt_ddl::manager::{DATABASE_IN_USE_HINT, MEMBERSHIP_WARNING, PARTIAL_APPLY_HINT, PRIVILEGE_HINT, VIEW_COLUMNS_HINT};

/// Bu bölümün testleri birbirini bekler: her biri birkaç bağlantı açar (bakım/hedef veritabanı, havuz, ayrı oturum);
/// paralel test ikilisinde sunucunun `max_connections` (100) sınırı aşılmasın.
static SERIAL: std::sync::LazyLock<tokio::sync::Mutex<()>> = std::sync::LazyLock::new(|| tokio::sync::Mutex::new(()));

/// Test parolası: süreç içinde rastgele; hiçbir yere yazdırılmaz.
fn random_password() -> String {
    format!("Pw{}", Uuid::new_v4().simple())
}

/// `Secret` yalnızca IPC'den (JSON dizge) gelir; testte de aynı yol.
fn secret(s: &str) -> Secret {
    serde_json::from_value(serde_json::json!(s)).unwrap()
}

fn invalid_message(r: &Result<ApplyOutcome>) -> String {
    match r {
        Err(Error::Invalid(m)) => m.clone(),
        other => panic!("Invalid bekleniyordu: {other:?}"),
    }
}

fn view_def(name: &str, query: &str) -> ViewDef {
    ViewDef {
        schema: "public".into(),
        name: name.into(),
        owner: None,
        query: query.into(),
        check_option: None,
        security_barrier: false,
        security_invoker: false,
        comment: None,
        recreate: false,
        columns: vec![],
        unsupported: vec![],
    }
}

fn database_def(name: &str) -> DatabaseDef {
    DatabaseDef {
        name: name.into(),
        owner: None,
        template: Some("template0".into()),
        encoding: None,
        locale_provider: None,
        lc_collate: None,
        lc_ctype: None,
        provider_locale: None,
        tablespace: None,
        connection_limit: -1,
        is_template: false,
        allow_connections: true,
        settings: vec![],
        comment: None,
    }
}

fn role_def(name: &str) -> RoleDef {
    RoleDef {
        name: name.into(),
        login: true,
        superuser: false,
        createdb: false,
        createrole: false,
        inherit: true,
        replication: false,
        bypassrls: false,
        connection_limit: -1,
        valid_until: None,
        password: PasswordChange::Keep,
        member_of: vec![],
        settings: vec![],
        comment: None,
        unsupported: vec![],
    }
}

impl Fx {
    async fn load_def(&self, object: ObjectRef) -> ObjectDef {
        self.dm.load(self.server, self.db, object).await.unwrap_or_else(|e| panic!("{:?} yüklenemedi: {e:?}", object.kind))
    }

    async fn load_view(&self, object: ObjectRef) -> ViewDef {
        let ObjectDef::View(v) = self.load_def(object).await else { panic!("görünüm bekleniyordu") };
        v
    }

    /// Sunucu kapsamlı: veritabanı oid'i verilmeden yüklenir.
    async fn load_database(&self, oid: u32) -> DatabaseDef {
        match self.dm.load(self.server, None, ObjectRef { kind: ObjectKind::Database, oid }).await {
            Ok(ObjectDef::Database(d)) => d,
            other => panic!("veritabanı yüklenemedi: {other:?}"),
        }
    }

    async fn load_role(&self, oid: u32) -> RoleDef {
        match self.dm.load(self.server, None, ObjectRef { kind: ObjectKind::Role, oid }).await {
            Ok(ObjectDef::Role(r)) => r,
            other => panic!("rol yüklenemedi: {other:?}"),
        }
    }
}

async fn database_oid(admin: &tokio_postgres::Client, name: &str) -> Option<u32> {
    admin.query_opt("SELECT oid FROM pg_database WHERE datname = $1::text", &[&name]).await.unwrap().map(|r| r.get(0))
}

async fn role_oid(admin: &tokio_postgres::Client, name: &str) -> Option<u32> {
    admin.query_opt("SELECT oid FROM pg_roles WHERE rolname = $1::text", &[&name]).await.unwrap().map(|r| r.get(0))
}

/// Ortak temizlik: test veritabanı + etiketin küme nesneleri.
async fn cleanup_both(ts: TestServer, tag: &'static str, cluster_tag: &'static str) {
    cleanup(&ts, tag).await;
    cleanup_cluster(&ts, cluster_tag).await;
}

// ---- View / MatView ----

#[tokio::test]
async fn view_lifecycle_through_manager() {
    let _serial = SERIAL.lock().await;
    for ts in servers() {
        guarded(view_life_body(ts.clone()), cleanup(&ts, "m_view")).await;
    }
}

async fn view_life_body(ts: TestServer) {
    let env = setup(&ts, "m_view").await;
    let f = fx(&env, &ts, false, None);
    let prod = fx(&env, &ts, false, Some("Üretim"));
    env.sql("CREATE TABLE public.vt (a int, b text)").await;
    let public = env.schema_oid("public").await;
    // Varsayılanlar: şema bağlamı zorunlu, tablo bağlamı yasak, veritabanı gerekli.
    let ctx = CreateContext { schema_oid: Some(public), table_oid: None };
    let ObjectDef::View(mut def) = f.dm.defaults(f.server, f.db, ObjectKind::View, ctx).await.unwrap() else { panic!() };
    assert_eq!((def.schema.as_str(), def.query.as_str(), def.check_option, def.security_barrier), ("public", "", None, false));
    let bad = CreateContext { schema_oid: Some(public), table_oid: Some(1) };
    assert!(matches!(f.dm.defaults(f.server, f.db, ObjectKind::View, bad).await, Err(Error::Invalid(_))));
    assert!(matches!(f.dm.defaults(f.server, None, ObjectKind::View, ctx).await, Err(Error::Invalid(_))));
    def.name = "mv_v".into();
    def.query = "SELECT a, b FROM public.vt".into();
    let object = created_of(f.apply(&create(ObjectDef::View(def.clone()))).await).expect("created");
    assert_eq!((object.kind, object.oid), (ObjectKind::View, env.class_oid("public", "mv_v").await));
    let loaded = f.load_view(object).await;
    assert_eq!((loaded.name.as_str(), loaded.owner.as_deref(), loaded.columns.len()), ("mv_v", Some(env.me.as_str()), 2));
    let noop = PlanRequest::Alter { object, old: ObjectDef::View(loaded.clone()), new: ObjectDef::View(loaded.clone()) };
    assert!(f.preview(&noop).await.empty, "yüklenen tanım sabit nokta olmalı");

    // CREATE OR REPLACE: sona kolon eklemek serbest, oid korunur.
    let mut wider = loaded.clone();
    wider.query = "SELECT a, b, 1 AS c FROM public.vt".into();
    let alter = PlanRequest::Alter { object, old: ObjectDef::View(loaded.clone()), new: ObjectDef::View(wider) };
    assert_eq!(f.preview(&alter).await.risk, Risk::Safe);
    assert_eq!(created_of(f.apply(&alter).await), None, "C-O-R kimliği değiştirmez");
    let loaded = f.load_view(object).await;
    assert_eq!(loaded.columns.len(), 3);

    // Uyumsuz C-O-R (kolon düşürme): 42P16 + Karar 1 ipucu, değişiklik yok.
    let mut narrow = loaded.clone();
    narrow.query = "SELECT a FROM public.vt".into();
    let alter = PlanRequest::Alter { object, old: ObjectDef::View(loaded.clone()), new: ObjectDef::View(narrow.clone()) };
    let (_, committed, sqlstate, error) = failed(f.apply(&alter).await);
    assert_eq!((committed, sqlstate.as_deref()), (0, Some("42P16")));
    assert_eq!(error.hint.as_deref(), Some(VIEW_COLUMNS_HINT));
    assert_eq!(f.load_view(object).await, loaded);

    // Açık yeniden kurma: DataLoss → onay, üretimde ad onayı; yeni oid.
    narrow.recreate = true;
    let recreate = PlanRequest::Alter { object, old: ObjectDef::View(loaded.clone()), new: ObjectDef::View(narrow) };
    assert_eq!(f.preview(&recreate).await.risk, Risk::DataLoss);
    assert_eq!(code(&f.apply(&recreate).await), "confirmation_required");
    assert_eq!(code(&prod.apply_with(&recreate, |o| o.confirmed = true).await), "production_confirmation_required");
    assert_eq!(f.load_view(object).await, loaded, "reddedilen plan hiçbir şey yazmamalı");
    let rebuilt = created_of(prod.apply_with(&recreate, |o| (o.confirmed, o.confirmed_production) = (true, true)).await).expect("created");
    assert_eq!(rebuilt.kind, ObjectKind::View);
    assert_ne!(rebuilt.oid, object.oid, "yeniden kurma yeni oid verir");
    let rebuilt_def = f.load_view(rebuilt).await;
    assert_eq!(rebuilt_def.columns.len(), 1);

    // Silme Destructive: onaysız red.
    let drop = PlanRequest::Drop { object: rebuilt, def: ObjectDef::View(rebuilt_def), cascade: false, force: false };
    assert_eq!(f.preview(&drop).await.risk, Risk::Destructive);
    assert_eq!(code(&f.apply(&drop).await), "confirmation_required");
    assert!(exists(&env, "public.mv_v").await);
    applied(f.apply_with(&drop, |o| o.confirmed = true).await);
    assert!(!exists(&env, "public.mv_v").await);
}

#[tokio::test]
async fn matview_refresh_needs_confirmation() {
    let _serial = SERIAL.lock().await;
    for ts in servers() {
        guarded(matview_refresh_body(ts.clone()), cleanup(&ts, "m_mvref")).await;
    }
}

async fn matview_refresh_body(ts: TestServer) {
    let env = setup(&ts, "m_mvref").await;
    let f = fx(&env, &ts, false, None);
    env.sql("CREATE TABLE public.mt (a int); INSERT INTO public.mt VALUES (1); CREATE MATERIALIZED VIEW public.mm AS SELECT a FROM public.mt")
        .await;
    let object = ObjectRef { kind: ObjectKind::MaterializedView, oid: env.class_oid("public", "mm").await };
    let ObjectDef::MaterializedView(loaded) = f.load_def(object).await else { panic!() };
    assert_eq!(loaded.refresh, None);
    let refresh = |mode| {
        let mut new = loaded.clone();
        new.refresh = Some(mode);
        PlanRequest::Alter { object, old: ObjectDef::MaterializedView(loaded.clone()), new: ObjectDef::MaterializedView(new) }
    };
    let normal = refresh(MatViewRefresh::Normal);
    assert_eq!(f.preview(&normal).await.risk, Risk::Destructive);
    assert_eq!(code(&f.apply(&normal).await), "confirmation_required");
    env.sql("INSERT INTO public.mt VALUES (2)").await;
    applied(f.apply_with(&normal, |o| o.confirmed = true).await);
    let n: i64 = env.admin.query_one("SELECT count(*) FROM public.mm", &[]).await.unwrap().get(0);
    assert_eq!(n, 2, "REFRESH çalışmalı");
    // CONCURRENTLY unique indeks ister: 55000, ipucu (PG'nin ya da bizim) dolu.
    let (_, _, sqlstate, error) = failed(f.apply_with(&refresh(MatViewRefresh::Concurrently), |o| o.confirmed = true).await);
    assert_eq!(sqlstate.as_deref(), Some("55000"));
    assert!(error.hint.is_some(), "{error:?}");
}

// ---- Database ----

#[tokio::test]
async fn database_lifecycle_on_maintenance() {
    let _serial = SERIAL.lock().await;
    for ts in servers() {
        guarded(database_life_body(ts.clone()), cleanup_both(ts.clone(), "m_dblife", "mdblife")).await;
    }
}

async fn database_life_body(ts: TestServer) {
    let env = setup(&ts, "m_dblife").await;
    let f = fx(&env, &ts, false, None);
    let prod = fx(&env, &ts, false, Some("Üretim"));
    let maint = &env.admin;
    let base = cluster_name(&ts, ObjectKind::Database, "mdblife").await;
    // Varsayılanlar: bağlam boş olmalı; veritabanı oid'i gerekmez.
    let ObjectDef::Database(mut def) = f.dm.defaults(f.server, None, ObjectKind::Database, CreateContext::default()).await.unwrap() else {
        panic!()
    };
    assert_eq!((def.connection_limit, def.allow_connections, def.is_template, def.tablespace.clone()), (-1, true, false, None));
    let schema_ctx = CreateContext { schema_oid: Some(1), table_oid: None };
    assert!(matches!(f.dm.defaults(f.server, None, ObjectKind::Database, schema_ctx).await, Err(Error::Invalid(_))));
    def.name = base.clone();
    def.template = Some("template0".into());
    def.comment = Some("dblife".into());
    // `database_oid` = başka bir veritabanı (Fx.db) verilse de CREATE bakım veritabanında çalışır.
    let created = created_of(f.apply(&create(ObjectDef::Database(def.clone()))).await).expect("created");
    assert_eq!((created.kind, Some(created.oid)), (ObjectKind::Database, database_oid(maint, &base).await));
    let loaded = f.load_database(created.oid).await;
    assert_eq!(
        (loaded.name.as_str(), loaded.owner.as_deref(), loaded.comment.as_deref()),
        (base.as_str(), Some(env.me.as_str()), Some("dblife"))
    );
    assert_eq!(loaded.tablespace.as_deref(), Some("pg_default"));
    assert_eq!(f.dm.load(f.server, Some(env.db_oid), created).await.unwrap(), ObjectDef::Database(loaded.clone()), "database_oid yok sayılır");
    let noop = PlanRequest::Alter { object: created, old: ObjectDef::Database(loaded.clone()), new: ObjectDef::Database(loaded.clone()) };
    assert!(f.preview(&noop).await.empty);

    // Yöneticinin havuzu hedef veritabanına bağlıyken (açılır liste çalıştı) RENAME başarılı: havuz bağlantısı bırakılır.
    let schemas = f.dm.lookup(f.server, Some(created.oid), LookupRequest::Schemas).await.unwrap();
    assert!(schemas.iter().any(|s| s.value == "public"));
    let renamed = format!("{base}_r");
    let mut new = loaded.clone();
    new.name = renamed.clone();
    let rename = PlanRequest::Alter { object: created, old: ObjectDef::Database(loaded.clone()), new: ObjectDef::Database(new) };
    let r = created_of(f.apply(&rename).await).expect("created");
    assert_eq!(r, created, "yeniden adlandırma oid'i korur");
    assert_eq!(database_oid(maint, &renamed).await, Some(created.oid));
    // Yeni adla havuz yeniden bağlanır (ad önbelleği temizlendi).
    assert!(!f.dm.lookup(f.server, Some(created.oid), LookupRequest::Schemas).await.unwrap().is_empty());
    let loaded = f.load_database(created.oid).await;

    // Başka bir oturum bağlıyken DROP → 55006 + Karar 8 ipucu; DataLoss üretim kapısı.
    let other = admin_client(&ts, &renamed).await;
    let drop = PlanRequest::Drop { object: created, def: ObjectDef::Database(loaded.clone()), cascade: false, force: false };
    assert_eq!(f.preview(&drop).await.risk, Risk::DataLoss);
    assert_eq!(code(&f.apply(&drop).await), "confirmation_required");
    assert_eq!(code(&prod.apply_with(&drop, |o| o.confirmed = true).await), "production_confirmation_required");
    let (idx, committed, sqlstate, error) = failed(f.apply_with(&drop, |o| o.confirmed = true).await);
    assert_eq!((idx, committed, sqlstate.as_deref()), (0, 0, Some("55006")));
    assert_eq!(error.hint.as_deref(), Some(DATABASE_IN_USE_HINT));
    assert!(database_oid(maint, &renamed).await.is_some());
    if env.version >= 130000 {
        let force = PlanRequest::Drop { object: created, def: ObjectDef::Database(loaded), cascade: false, force: true };
        applied(prod.apply_with(&force, |o| (o.confirmed, o.confirmed_production) = (true, true)).await);
        assert!(database_oid(maint, &renamed).await.is_none());
        assert!(other.simple_query("SELECT 1").await.is_err(), "FORCE öteki oturumu sonlandırmalı");
    } else {
        eprintln!("PG {}: WITH (FORCE) yok, atlandı", env.version / 10_000);
    }
}

#[tokio::test]
async fn maintenance_database_is_protected() {
    let _serial = SERIAL.lock().await;
    for ts in servers() {
        let maint = admin_client(&ts, &ts.config.maintenance_db).await;
        let q = quote_ident(&ts.config.maintenance_db);
        let original: Option<String> = maint
            .query_one("SELECT shobj_description(oid, 'pg_database') FROM pg_database WHERE datname = $1::text", &[&ts.config.maintenance_db])
            .await
            .unwrap()
            .get(0);
        let restore = format!("COMMENT ON DATABASE {q} IS {}", original.as_deref().map_or("NULL".to_string(), quote_literal));
        drop(maint);
        let t = ts.clone();
        let cleanup_all = async move {
            admin_client(&t, &t.config.maintenance_db).await.batch_execute(&restore).await.unwrap();
            cleanup(&t, "m_maint").await;
        };
        guarded(maintenance_body(ts.clone()), cleanup_all).await;
    }
}

async fn maintenance_body(ts: TestServer) {
    let env = setup(&ts, "m_maint").await;
    let f = fx(&env, &ts, false, None);
    let maint = &env.admin;
    let oid = database_oid(maint, &ts.config.maintenance_db).await.unwrap();
    let object = ObjectRef { kind: ObjectKind::Database, oid };
    let loaded = f.load_database(oid).await;
    let alter = |edit: &dyn Fn(&mut DatabaseDef)| {
        let mut new = loaded.clone();
        edit(&mut new);
        PlanRequest::Alter { object, old: ObjectDef::Database(loaded.clone()), new: ObjectDef::Database(new) }
    };
    let refused = [
        alter(&|n| n.name = format!("{}_x", n.name)),
        alter(&|n| n.tablespace = Some("pgt_yok_ts".into())),
        alter(&|n| n.allow_connections = false),
        PlanRequest::Drop { object, def: ObjectDef::Database(loaded.clone()), cascade: false, force: false },
    ];
    for req in &refused {
        let r = f.apply_with(req, |o| (o.confirmed, o.confirmed_production) = (true, true)).await;
        assert_eq!(invalid_message(&r), error_codes::MAINTENANCE_DATABASE, "{req:?}");
    }
    assert_eq!(database_oid(maint, &ts.config.maintenance_db).await, Some(oid));
    // Yorum değişikliği serbest (temizlik özgün yorumu geri yazar).
    let comment = alter(&|n| n.comment = Some("pgt-ddl bakım testi".into()));
    applied(f.apply(&comment).await);
    assert_eq!(f.load_database(oid).await.comment.as_deref(), Some("pgt-ddl bakım testi"));
}

#[tokio::test]
async fn database_partial_apply_reports_committed() {
    let _serial = SERIAL.lock().await;
    for ts in servers() {
        guarded(database_partial_body(ts.clone()), cleanup_both(ts.clone(), "m_dbpart", "mdbpart")).await;
    }
}

async fn database_partial_body(ts: TestServer) {
    let env = setup(&ts, "m_dbpart").await;
    let f = fx(&env, &ts, false, None);
    let maint = &env.admin;
    let base = cluster_name(&ts, ObjectKind::Database, "mdbpart").await;
    // Kısmi create: CREATE DATABASE kalıcı, sonraki SET (tanınmayan parametre) başarısız.
    let mut def = database_def(&base);
    def.settings = vec![ConfigSetting { name: "pgt_no_such_param".into(), value: "x".into() }];
    let out = f.apply(&create(ObjectDef::Database(def))).await;
    let Ok(ApplyOutcome::Failed { failed_index, committed, sqlstate, error, created, .. }) = out else { panic!("{out:?}") };
    assert_eq!((failed_index, committed), (1, 1), "{error:?}");
    assert!(sqlstate.is_some(), "{error:?}");
    let oid = database_oid(maint, &base).await.expect("CREATE DATABASE kalıcı olmalı");
    assert_eq!(created, Some(ObjectRef { kind: ObjectKind::Database, oid }), "kısmi create yine de bulunur");
    assert!(error.hint.is_some(), "kısmi uygulama ipucu (ya da PG ipucu) bekleniyordu: {error:?}");
    // Rename + var olmayan sahip: RENAME kalıcı (transaction dışı), OWNER TO (en sonda) başarısız.
    let loaded = f.load_database(oid).await;
    let mut new = loaded.clone();
    new.name = format!("{base}_r");
    new.owner = Some("pgt_yok_sahip".into());
    let object = ObjectRef { kind: ObjectKind::Database, oid };
    let req = PlanRequest::Alter { object, old: ObjectDef::Database(loaded), new: ObjectDef::Database(new) };
    let out = f.apply(&req).await;
    let Ok(ApplyOutcome::Failed { failed_index, committed, sqlstate, error, created, .. }) = out else { panic!("{out:?}") };
    assert_eq!((failed_index, committed, sqlstate.as_deref()), (1, 1, Some("42704")));
    assert_eq!(error.hint.as_deref(), Some(PARTIAL_APPLY_HINT));
    assert_eq!(created, Some(object));
    assert_eq!(database_oid(maint, &format!("{base}_r")).await, Some(oid), "ad değişmiş olmalı (transaction dışı)");
}

// ---- Role ----

#[tokio::test]
async fn role_lifecycle_and_password() {
    let _serial = SERIAL.lock().await;
    for ts in servers() {
        guarded(role_life_body(ts.clone()), cleanup_both(ts.clone(), "m_rolelife", "mrolelife")).await;
    }
}

async fn role_life_body(ts: TestServer) {
    let env = setup(&ts, "m_rolelife").await;
    let f = fx(&env, &ts, false, None);
    let maint = &env.admin;
    let name = cluster_name(&ts, ObjectKind::Role, "mrolelife").await;
    let ObjectDef::Role(mut def) = f.dm.defaults(f.server, None, ObjectKind::Role, CreateContext::default()).await.unwrap() else { panic!() };
    assert_eq!((def.login, def.inherit, def.superuser, def.connection_limit, def.password), (false, true, false, -1, PasswordChange::Keep));
    def.name = name.clone();
    def.login = true;
    def.password = PasswordChange::Set;
    let req = create(ObjectDef::Role(def.clone()));
    // Parolasız: kodlu red, hiçbir şey yazılmaz.
    assert_eq!(code(&f.apply(&req).await), "role_password_missing");
    assert_eq!(role_oid(maint, &name).await, None);
    // Boş parola: kabul kuralı (değer yankılanmaz).
    let r = f.apply_with(&req, |o| o.password = Some(secret(""))).await;
    assert_eq!(invalid_message(&r), build::role::SECRET_EMPTY_MSG);
    assert_eq!(role_oid(maint, &name).await, None);
    let pw = random_password();
    let created = created_of(f.apply_with(&req, |o| o.password = Some(secret(&pw))).await).expect("created");
    assert_eq!((created.kind, Some(created.oid)), (ObjectKind::Role, role_oid(maint, &name).await));
    let mut cfg = ts.config.clone();
    cfg.id = Uuid::new_v4();
    cfg.username = name.clone();
    assert!(ConnectionManager::test(&cfg, Some(&pw)).await.is_ok(), "doğru parola ile giriş");
    assert!(ConnectionManager::test(&cfg, Some("yanlis-parola")).await.is_err(), "yanlış parola reddedilmeli");
    let stored: String = maint.query_one("SELECT rolpassword FROM pg_authid WHERE rolname = $1::text", &[&name]).await.unwrap().get(0);
    assert!(stored.starts_with("SCRAM-SHA-256$") && !stored.contains(&pw), "istemci tarafı SCRAM doğrulayıcısı saklanmalı");

    // keep planına parola → PASSWORD_NOT_USED.
    let loaded = f.load_role(created.oid).await;
    assert_eq!(loaded.password, PasswordChange::Keep);
    let mut commented = loaded.clone();
    commented.comment = Some("c".into());
    let keep = PlanRequest::Alter { object: created, old: ObjectDef::Role(loaded.clone()), new: ObjectDef::Role(commented) };
    let r = f.apply_with(&keep, |o| o.password = Some(secret(&pw))).await;
    assert_eq!(invalid_message(&r), error_codes::PASSWORD_NOT_USED);
    applied(f.apply(&keep).await);
    let loaded = f.load_role(created.oid).await;

    // clear → giriş başarısız.
    let mut cleared = loaded.clone();
    cleared.password = PasswordChange::Clear;
    applied(f.apply(&PlanRequest::Alter { object: created, old: ObjectDef::Role(loaded), new: ObjectDef::Role(cleared) }).await);
    assert!(ConnectionManager::test(&cfg, Some(&pw)).await.is_err(), "parola temizlenince giriş başarısız");

    // REVOKE Destructive → onay; süper kullanıcının REVOKE'u etkili (uyarı yok).
    let group = format!("{name}_g");
    maint.batch_execute(&format!("CREATE ROLE {group}; GRANT {group} TO {name}")).await.unwrap();
    let loaded = f.load_role(created.oid).await;
    assert_eq!(loaded.member_of.len(), 1);
    let mut revoked = loaded.clone();
    revoked.member_of.clear();
    let revoke = PlanRequest::Alter { object: created, old: ObjectDef::Role(loaded), new: ObjectDef::Role(revoked) };
    assert_eq!(f.preview(&revoke).await.risk, Risk::Destructive);
    assert_eq!(code(&f.apply(&revoke).await), "confirmation_required");
    let out = f.apply_with(&revoke, |o| o.confirmed = true).await;
    let Ok(ApplyOutcome::Applied { warnings, .. }) = out else { panic!("{out:?}") };
    assert_eq!(warnings, None);
    assert!(f.load_role(created.oid).await.member_of.is_empty());
}

#[tokio::test]
async fn role_insufficient_privilege_is_reported() {
    let _serial = SERIAL.lock().await;
    for ts in servers() {
        guarded(role_priv_body(ts.clone()), cleanup_both(ts.clone(), "m_rolepriv", "mrolepriv")).await;
    }
}

async fn role_priv_body(ts: TestServer) {
    let env = setup(&ts, "m_rolepriv").await;
    let maint = &env.admin;
    let base = cluster_name(&ts, ObjectKind::Role, "mrolepriv").await;
    let (creator, group, member) = (format!("{base}_c"), format!("{base}_g"), format!("{base}_m"));
    let pw = random_password();
    maint
        .batch_execute(&format!(
            "CREATE ROLE {creator} LOGIN CREATEROLE PASSWORD {}; CREATE ROLE {group}; CREATE ROLE {member}; \
             GRANT {group} TO {member}; GRANT {group} TO {creator} WITH ADMIN OPTION",
            quote_literal(&pw)
        ))
        .await
        .unwrap();
    let mut config = ts.config.clone();
    config.id = Uuid::new_v4();
    config.username = creator.clone();
    let mgr = Arc::new(ConnectionManager::new());
    if mgr.connect(config.clone(), Some(pw)).await.is_err() {
        panic!("CREATEROLE rolüyle bağlanılamadı");
    }
    let (dm, dir) = manager(mgr.clone(), &config, false, None);
    let f = Fx { dm, server: config.id, db: env.db_oid, _dir: dir };
    // SUPERUSER yalnızca süper kullanıcı verebilir: 42501 + Karar 9 ipucu.
    let mut sup = role_def(&format!("{base}_s"));
    sup.superuser = true;
    let (_, committed, sqlstate, error) = failed(f.apply(&create(ObjectDef::Role(sup))).await);
    assert_eq!((committed, sqlstate.as_deref()), (0, Some("42501")));
    assert_eq!(error.hint.as_deref(), Some(PRIVILEGE_HINT));
    assert_eq!(role_oid(maint, &format!("{base}_s")).await, None);
    // REVOKE (GRANTED BY'sız) süper kullanıcının verdiği üyeliği PG 16+'da kaldırmayabilir (WARNING, etkisiz):
    // yönetici yeniden yükleyip farkı uyarı olarak bildirir. Uyarı ancak ve ancak üyelik gerçekten kaldıysa vardır.
    let member_oid = role_oid(maint, &member).await.unwrap();
    let loaded = f.load_role(member_oid).await;
    assert_eq!(loaded.member_of.len(), 1, "{loaded:?}");
    let mut revoked = loaded.clone();
    revoked.member_of.clear();
    let object = ObjectRef { kind: ObjectKind::Role, oid: member_oid };
    let req = PlanRequest::Alter { object, old: ObjectDef::Role(loaded), new: ObjectDef::Role(revoked) };
    let out = f.apply_with(&req, |o| o.confirmed = true).await;
    let Ok(ApplyOutcome::Applied { warnings, .. }) = out else { panic!("{out:?}") };
    let still: bool = maint
        .query_one(
            "SELECT EXISTS (SELECT 1 FROM pg_auth_members m JOIN pg_roles r ON r.oid = m.roleid JOIN pg_roles u ON u.oid = m.member \
             WHERE r.rolname = $1::text AND u.rolname = $2::text)",
            &[&group, &member],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(warnings.is_some(), still, "uyarı = üyelik kaldı (PG {}): {warnings:?}", env.version / 10_000);
    if let Some(w) = warnings {
        assert_eq!(w, [MEMBERSHIP_WARNING]);
        eprintln!("PG {}: GRANTED BY'sız REVOKE etkisiz kaldı, uyarı verildi", env.version / 10_000);
    }
    mgr.disconnect(config.id);
}

// ---- tahrifat / kapsam / kilit ----

#[tokio::test]
async fn tampered_requests_are_refused() {
    let _serial = SERIAL.lock().await;
    for ts in servers() {
        guarded(tamper_body(ts.clone()), cleanup_both(ts.clone(), "m_tamper", "mtamper")).await;
    }
}

async fn tamper_body(ts: TestServer) {
    let env = setup(&ts, "m_tamper").await;
    let f = fx(&env, &ts, false, None);
    let maint = &env.admin;
    let role = cluster_name(&ts, ObjectKind::Role, "mtamper").await;
    let db = cluster_name(&ts, ObjectKind::Database, "mtamper").await;
    maint.batch_execute(&format!("CREATE ROLE {role}")).await.unwrap();
    maint.batch_execute(&format!("CREATE DATABASE {db} TEMPLATE template0")).await.unwrap();
    let robj = ObjectRef { kind: ObjectKind::Role, oid: role_oid(maint, &role).await.unwrap() };
    let dobj = ObjectRef { kind: ObjectKind::Database, oid: database_oid(maint, &db).await.unwrap() };
    let loaded = f.load_role(robj.oid).await;

    // Eski tarafta parola niyeti `keep` dışında: yüklenen tanımla eşleşmez → plan_changed (parola gönderilse bile).
    let mut forged_old = loaded.clone();
    forged_old.password = PasswordChange::Set;
    let mut new = loaded.clone();
    new.comment = Some("x".into());
    let req = PlanRequest::Alter { object: robj, old: ObjectDef::Role(forged_old), new: ObjectDef::Role(new.clone()) };
    assert_eq!(code(&f.apply(&req).await), "plan_changed");
    assert_eq!(code(&f.apply_with(&req, |o| o.password = Some(secret("p"))).await), "plan_changed");
    // Bayat `old`: başka oturum yorumu değiştirdi.
    let stale = PlanRequest::Alter { object: robj, old: ObjectDef::Role(loaded.clone()), new: ObjectDef::Role(new.clone()) };
    let hash = f.preview(&stale).await.hash;
    maint.batch_execute(&format!("COMMENT ON ROLE {role} IS 'başka oturum'")).await.unwrap();
    assert_eq!(code(&f.dm.apply(f.server, None, &stale, opts(&hash)).await), "plan_changed");
    let loaded = f.load_role(robj.oid).await;
    // Parola niyeti sahte hash'le: keep planının hash'i set planına verilemez.
    let mut set = loaded.clone();
    set.password = PasswordChange::Set;
    let keep_req = PlanRequest::Alter { object: robj, old: ObjectDef::Role(loaded.clone()), new: ObjectDef::Role(new) };
    let set_req = PlanRequest::Alter { object: robj, old: ObjectDef::Role(loaded.clone()), new: ObjectDef::Role(set) };
    let keep_hash = f.preview(&keep_req).await.hash;
    let mut o = opts(&keep_hash);
    o.password = Some(secret(&random_password()));
    assert_eq!(code(&f.dm.apply(f.server, None, &set_req, o).await), "plan_changed");
    // Tür uyuşmazlığı: nesne rol, tanım veritabanı → invalid_def.
    let dloaded = f.load_database(dobj.oid).await;
    let mixed = PlanRequest::Drop { object: robj, def: ObjectDef::Database(dloaded.clone()), cascade: false, force: false };
    assert_eq!(code(&f.apply_with(&mixed, |o| o.confirmed = true).await), "invalid_def");
    // Database drop: def.name ≠ yüklenen → plan_changed; veritabanı yerinde.
    let mut renamed = dloaded.clone();
    renamed.name = format!("{db}_x");
    let drop = PlanRequest::Drop { object: dobj, def: ObjectDef::Database(renamed), cascade: false, force: false };
    assert_eq!(code(&f.apply_with(&drop, |o| o.confirmed = true).await), "plan_changed");
    assert!(database_oid(maint, &db).await.is_some());
    // View: oynanmış hash → plan_changed; veritabanısız → NEEDS_DATABASE.
    let view = create(ObjectDef::View(view_def("mt_v", "SELECT 1 AS x")));
    assert_eq!(code(&f.dm.apply(f.server, f.db, &view, opts("bogus")).await), "plan_changed");
    let hash = f.preview(&view).await.hash;
    let r = f.dm.apply(f.server, None, &view, opts(&hash)).await;
    assert_eq!(invalid_message(&r), error_codes::NEEDS_DATABASE);
    assert!(!exists(&env, "public.mt_v").await);
    // Kapsam: veritabanı kapsamlı yükleme/açılır liste `None` ile reddedilir; sunucu geneli serbest.
    let schema = ObjectRef { kind: ObjectKind::Schema, oid: env.schema_oid("public").await };
    assert!(matches!(f.dm.load(f.server, None, schema).await, Err(Error::Invalid(m)) if m == error_codes::NEEDS_DATABASE));
    assert!(matches!(f.dm.lookup(f.server, None, LookupRequest::Schemas).await, Err(Error::Invalid(_))));
    let roles = f.dm.lookup(f.server, None, LookupRequest::GrantableRoles).await.unwrap();
    assert!(roles.iter().any(|r| r.value == role));
}

#[tokio::test]
async fn locked_server_refuses_all_new_kinds() {
    let _serial = SERIAL.lock().await;
    for ts in servers() {
        guarded(locked_new_body(ts.clone()), cleanup_both(ts.clone(), "m_lockednew", "mlockednew")).await;
    }
}

async fn locked_new_body(ts: TestServer) {
    let env = setup(&ts, "m_lockednew").await;
    let f = fx(&env, &ts, true, None);
    let maint = &env.admin;
    let role = cluster_name(&ts, ObjectKind::Role, "mlockednew").await;
    let db = cluster_name(&ts, ObjectKind::Database, "mlockednew").await;
    let mv = MatViewDef {
        schema: "public".into(),
        name: "ml_m".into(),
        owner: None,
        query: "SELECT 1 AS x".into(),
        tablespace: None,
        fillfactor: None,
        with_data: true,
        comment: None,
        refresh: None,
        columns: vec![],
        unsupported: vec![],
    };
    let mut r = role_def(&role);
    r.password = PasswordChange::Set;
    for def in [
        ObjectDef::View(view_def("ml_v", "SELECT 1 AS x")),
        ObjectDef::MaterializedView(mv),
        ObjectDef::Database(database_def(&db)),
        ObjectDef::Role(r),
    ] {
        let req = create(def.clone());
        let r = f
            .apply_with(&req, |o| {
                (o.confirmed, o.confirmed_production) = (true, true);
                o.password = Some(secret("p"));
            })
            .await;
        assert_eq!(code(&r), "locked", "{:?}", def.kind());
        assert!(!f.preview(&req).await.empty, "önizleme serbest");
    }
    assert!(!exists(&env, "public.ml_v").await && !exists(&env, "public.ml_m").await);
    assert_eq!((role_oid(maint, &role).await, database_oid(maint, &db).await), (None, None));
    // Yükleme ve açılır listeler serbest.
    let moid = database_oid(maint, &ts.config.maintenance_db).await.unwrap();
    assert_eq!(f.load_database(moid).await.name, ts.config.maintenance_db);
    assert!(!f.dm.lookup(f.server, None, LookupRequest::Settings).await.unwrap().is_empty());
    assert!(!f.dm.lookup(f.server, f.db, LookupRequest::Schemas).await.unwrap().is_empty());
}
