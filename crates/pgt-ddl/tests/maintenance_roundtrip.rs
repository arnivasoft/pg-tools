//! Bakım işlemleri (Plan 24) Docker testleri: hedef yükleyicisi (`load_maintenance_target`), üreticinin
//! (`build::maintenance`) ürettiği ifadenin sunucuda düz çalıştırılması ve `PlanRequest::Maintenance`'ın
//! `DdlManager` kapılarından uygulanması (iletiler, lock_timeout, iptal, ilerleme). `PGT_TEST_URLS` yoksa atlanır.
//!
//! Her test kendi geçici veritabanını (`pgt_ddl_rt_<ana sürüm>_<etiket>`) ve `pgt_ddl_rt_<etiket>` şemasını kurar;
//! başarısız doğrulamadan sonra bile veritabanı silinir (`guarded` + `cleanup`). Sürüme bağlı kısımlar (bölümlenmiş
//! indeks PG 11+) atlandığında yazdırılır.

mod common;

use std::sync::{Arc, RwLock};
use std::time::{Duration, Instant};

use common::{Env, cleanup, db_name, guarded, servers, setup};
use pgt_core::Error;
use pgt_core::connection::{SessionProfile, Target};
use pgt_core::registry::Registry;
use pgt_core::testing::{TestServer, admin_client};
use pgt_ddl::build::maintenance::maintenance;
use pgt_ddl::load_maintenance::{NO_MAINTENANCE_MSG, PARTITIONED_INDEX_REASON, TEMP_SCHEMA_REASON, WRONG_DATABASE_MSG};
use pgt_ddl::*;

macro_rules! docker_test {
    ($name:ident, $body:ident, $tag:literal) => {
        #[tokio::test]
        async fn $name() {
            for ts in servers() {
                guarded($body(ts.clone(), $tag), async move { cleanup(&ts, $tag).await }).await;
            }
        }
    };
}

const S: &str = "pgt_ddl_rt_mt";

async fn try_load(env: &Env, kind: ObjectKind, oid: u32) -> pgt_core::Result<MaintenanceTarget> {
    load_maintenance_target(&env.mgr, env.server, env.db_oid, ObjectRef { kind, oid }).await
}

async fn ld(env: &Env, kind: ObjectKind, oid: u32) -> MaintenanceTarget {
    try_load(env, kind, oid).await.unwrap_or_else(|e| panic!("{kind:?} {oid} yüklenemedi: {e:?}"))
}

fn base(kind: MaintenanceTargetKind, schema: Option<&str>, name: &str) -> MaintenanceTarget {
    MaintenanceTarget {
        kind,
        schema: schema.map(Into::into),
        name: name.into(),
        table: None,
        partitioned: false,
        system: false,
        cluster_index: None,
        indexes: vec![],
        editable: true,
        unsupported: vec![],
    }
}

fn strs(v: &[&str]) -> Vec<String> {
    v.iter().map(|s| s.to_string()).collect()
}

/// Ortak fikstür: kümelenmiş tablo (kümelenebilir/kümelenemez indeksler), MV, bölümlenmiş tablo (PG 11+ bölümlenmiş
/// indeksiyle), görünüm ve sequence.
async fn fixture(env: &Env) {
    env.sql(&format!(
        "CREATE SCHEMA {S};
         CREATE TABLE {S}.t (id int PRIMARY KEY, a int, b int);
         CREATE INDEX t_b ON {S}.t (b);
         CREATE INDEX t_a_part ON {S}.t (a) WHERE a > 0;
         CREATE INDEX t_a_hash ON {S}.t USING hash (a);
         ALTER TABLE {S}.t CLUSTER ON t_b;
         CREATE TABLE {S}.\"Ğ x\"\"y\" (id int);
         CREATE MATERIALIZED VIEW {S}.mv AS SELECT id FROM {S}.t;
         CREATE INDEX mv_id ON {S}.mv (id);
         CREATE TABLE {S}.p (id int, k int) PARTITION BY RANGE (k);
         CREATE TABLE {S}.p1 PARTITION OF {S}.p FOR VALUES FROM (0) TO (100);
         CREATE VIEW {S}.v AS SELECT 1 AS x;
         CREATE SEQUENCE {S}.s;"
    ))
    .await;
    if env.version >= 110000 {
        env.sql(&format!("CREATE INDEX p_k ON {S}.p (k)")).await;
    }
}

// ================================================================================================================
// yükleme alanları
// ================================================================================================================

docker_test!(loads_each_kind, loads_each_kind_body, "mtload");
async fn loads_each_kind_body(ts: TestServer, tag: &'static str) {
    let env = setup(&ts, tag).await;
    fixture(&env).await;
    use MaintenanceTargetKind as K;

    // Tablo: kümelenebilir indeksler ad sırasıyla (kısmi ve hash indeks CLUSTER'da kullanılamaz), kümeleme indeksi.
    let t = ld(&env, ObjectKind::Table, env.class_oid(S, "t").await).await;
    let mut want = base(K::Table, Some(S), "t");
    want.indexes = strs(&["t_b", "t_pkey"]);
    want.cluster_index = Some("t_b".into());
    assert_eq!(t, want);

    // Ham ad (tırnaklama üreticide).
    let odd = ld(&env, ObjectKind::Table, env.class_oid(S, "Ğ x\"y").await).await;
    assert_eq!(odd, base(K::Table, Some(S), "Ğ x\"y"));

    let mv = ld(&env, ObjectKind::MaterializedView, env.class_oid(S, "mv").await).await;
    let mut want = base(K::MaterializedView, Some(S), "mv");
    want.indexes = strs(&["mv_id"]);
    assert_eq!(mv, want);

    let i = ld(&env, ObjectKind::Index, env.class_oid(S, "t_b").await).await;
    let mut want = base(K::Index, Some(S), "t_b");
    want.table = Some("t".into());
    assert_eq!(i, want);

    let sch = ld(&env, ObjectKind::Schema, env.schema_oid(S).await).await;
    assert_eq!(sch, base(K::Schema, None, S));

    let db = ld(&env, ObjectKind::Database, env.db_oid).await;
    assert_eq!(db, base(K::Database, None, &db_name(&ts, tag).await));

    // Bölüm sıradan tablodur.
    let p1 = ld(&env, ObjectKind::Table, env.class_oid(S, "p1").await).await;
    assert!(!p1.partitioned && p1.editable, "{p1:?}");
}

docker_test!(partitioned_table_and_index, partitioned_body, "mtpart");
async fn partitioned_body(ts: TestServer, tag: &'static str) {
    let env = setup(&ts, tag).await;
    fixture(&env).await;
    let p = ld(&env, ObjectKind::Table, env.class_oid(S, "p").await).await;
    let mut want = base(MaintenanceTargetKind::Table, Some(S), "p");
    want.partitioned = true;
    if env.version >= 110000 {
        want.indexes = strs(&["p_k"]);
    }
    assert_eq!(p, want);
    if env.version < 110000 {
        eprintln!("PG {}: bölümlenmiş indeks yok (PG 11+), atlandı", env.version);
        return;
    }
    // Bölümlenmiş indeks salt okunur; bölümün kendi indeksi sıradan indekstir.
    let pk = ld(&env, ObjectKind::Index, env.class_oid(S, "p_k").await).await;
    let mut want = base(MaintenanceTargetKind::Index, Some(S), "p_k");
    want.table = Some("p".into());
    want.partitioned = true;
    want.editable = false;
    want.unsupported = strs(&[PARTITIONED_INDEX_REASON]);
    assert_eq!(pk, want);
    let child: String = env
        .admin
        .query_one(
            &format!("SELECT c.relname::text FROM pg_index x JOIN pg_class c ON c.oid = x.indexrelid WHERE x.indrelid = '{S}.p1'::regclass"),
            &[],
        )
        .await
        .unwrap()
        .get(0);
    let ci = ld(&env, ObjectKind::Index, env.class_oid(S, &child).await).await;
    assert!(!ci.partitioned && ci.editable && ci.table.as_deref() == Some("p1"), "{ci:?}");
}

docker_test!(system_objects_are_flagged, system_body, "mtsys");
async fn system_body(ts: TestServer, tag: &'static str) {
    let env = setup(&ts, tag).await;
    let c = ld(&env, ObjectKind::Table, env.class_oid("pg_catalog", "pg_class").await).await;
    assert_eq!((c.kind, c.schema.as_deref(), c.name.as_str()), (MaintenanceTargetKind::Table, Some("pg_catalog"), "pg_class"));
    assert!(c.system && c.editable && c.unsupported.is_empty(), "{c:?}");
    assert!(c.indexes.contains(&"pg_class_oid_index".to_string()), "{:?}", c.indexes);
    let i = ld(&env, ObjectKind::Index, env.class_oid("pg_catalog", "pg_class_oid_index").await).await;
    assert!(i.system && i.table.as_deref() == Some("pg_class"), "{i:?}");
    for n in ["pg_catalog", "information_schema", "pg_toast"] {
        let s = ld(&env, ObjectKind::Schema, env.schema_oid(n).await).await;
        assert!(s.system && s.editable, "{n}: {s:?}");
    }
    let public = ld(&env, ObjectKind::Schema, env.schema_oid("public").await).await;
    assert!(!public.system, "{public:?}");
    // Sistem nesnesinde VACUUM üretilir, CLUSTER reddedilir (üretici kuralı).
    let ctx = BuildCtx { version_num: env.version, concurrently: false };
    let vac = VacuumOptions {
        full: false,
        freeze: false,
        analyze: false,
        verbose: false,
        disable_page_skipping: false,
        skip_locked: false,
        index_cleanup: None,
        parallel: None,
    };
    let p = maintenance(&ctx, &c, &MaintenanceOp::Vacuum(vac)).expect("sistem tablosunda VACUUM");
    env.sql(&p.statements[0].sql).await;
    let clu = ClusterOptions { verbose: false, using_index: Some("pg_class_oid_index".into()) };
    assert!(maintenance(&ctx, &c, &MaintenanceOp::Cluster(clu)).is_err());
}

docker_test!(temp_objects_are_read_only, temp_body, "mttemp");
async fn temp_body(ts: TestServer, tag: &'static str) {
    let env = setup(&ts, tag).await;
    // Geçici tablo test oturumunundur; yükleyici (ve uygulayıcı) başka bağlantıdır.
    env.sql("CREATE TEMP TABLE tt (id int PRIMARY KEY)").await;
    let schema: String = env
        .admin
        .query_one("SELECT n.nspname::text FROM pg_class c JOIN pg_namespace n ON n.oid = c.relnamespace WHERE c.oid = 'tt'::regclass", &[])
        .await
        .unwrap()
        .get(0);
    let t = ld(&env, ObjectKind::Table, env.class_oid(&schema, "tt").await).await;
    assert_eq!((t.editable, t.unsupported.clone()), (false, strs(&[TEMP_SCHEMA_REASON])), "{t:?}");
    let i = ld(&env, ObjectKind::Index, env.class_oid(&schema, "tt_pkey").await).await;
    assert!(!i.editable, "{i:?}");
    let s = ld(&env, ObjectKind::Schema, env.schema_oid(&schema).await).await;
    assert_eq!((s.editable, s.unsupported.clone()), (false, strs(&[TEMP_SCHEMA_REASON])), "{s:?}");
}

docker_test!(missing_and_mismatched_objects, missing_body, "mtmiss");
async fn missing_body(ts: TestServer, tag: &'static str) {
    let env = setup(&ts, tag).await;
    fixture(&env).await;
    let t = env.class_oid(S, "t").await;
    let v = env.class_oid(S, "v").await;
    let s = env.class_oid(S, "s").await;
    let i = env.class_oid(S, "t_b").await;
    let mv = env.class_oid(S, "mv").await;
    // Tür uyuşmazlığı → NotFound.
    for (kind, oid) in [
        (ObjectKind::Table, v),
        (ObjectKind::Table, s),
        (ObjectKind::Table, i),
        (ObjectKind::Table, mv),
        (ObjectKind::Index, t),
        (ObjectKind::MaterializedView, t),
        (ObjectKind::Schema, t),
    ] {
        assert!(matches!(try_load(&env, kind, oid).await, Err(Error::NotFound(_))), "{kind:?}");
    }
    // Bakımı olmayan türler → Invalid (katalog okunmadan).
    for kind in [ObjectKind::View, ObjectKind::Sequence, ObjectKind::Function, ObjectKind::Role, ObjectKind::Extension] {
        match try_load(&env, kind, v).await {
            Err(Error::Invalid(m)) => assert_eq!(m, NO_MAINTENANCE_MSG),
            other => panic!("{kind:?}: {other:?}"),
        }
    }
    // Silinmiş oid → NotFound.
    env.sql(&format!("DROP TABLE {S}.t CASCADE; DROP SCHEMA {S} CASCADE")).await;
    assert!(matches!(try_load(&env, ObjectKind::Table, t).await, Err(Error::NotFound(_))));
    assert!(matches!(try_load(&env, ObjectKind::Index, i).await, Err(Error::NotFound(_))));
}

docker_test!(database_target_must_be_the_connected_database, database_body, "mtdb");
async fn database_body(ts: TestServer, tag: &'static str) {
    let env = setup(&ts, tag).await;
    let maint = admin_client(&ts, &ts.config.maintenance_db).await;
    let other: u32 = maint.query_one("SELECT oid FROM pg_database WHERE datname = current_database()", &[]).await.unwrap().get(0);
    assert_ne!(other, env.db_oid);
    match try_load(&env, ObjectKind::Database, other).await {
        Err(Error::Invalid(m)) => assert_eq!(m, WRONG_DATABASE_MSG),
        r => panic!("başka veritabanı reddedilmeli: {r:?}"),
    }
    let missing: u32 = maint.query_one("SELECT max(oid)::int8 + 1000 FROM pg_database", &[]).await.unwrap().get::<_, i64>(0) as u32;
    assert!(matches!(try_load(&env, ObjectKind::Database, missing).await, Err(Error::NotFound(_))));
}

// ================================================================================================================
// geçersiz UTF-8 ad (SQL_ASCII veritabanı)
// ================================================================================================================
//
// UTF8 veritabanında ilişki/şema/veritabanı adı geçersiz UTF-8 olamaz (PG `truncate_identifier` çok baytlı sınırda
// keser; kesik ad yalnız PG 14+ multirange *tip* adlarında görülür). SQL_ASCII veritabanında ham bayt saklanır ama
// sunucu UTF8 istemciye gönderirken doğrular (22021): yükleyici çökmeden sorgu hatası döner, kayıplı ad üretmez.
// Kayıplı okuma + `INVALID_UTF8_REASON` deseni yine de savunma olarak uygulanır (Plan 18/20 kalıbı).

#[tokio::test]
async fn invalid_utf8_names_fail_cleanly() {
    const TAG: &str = "mtu8";
    for ts in servers() {
        let ts2 = ts.clone();
        guarded(invalid_utf8_body(ts.clone()), async move {
            let maint = admin_client(&ts2, &ts2.config.maintenance_db).await;
            common::drop_test_database(&maint, &format!("{}_a", db_name(&ts2, TAG).await)).await;
            cleanup(&ts2, TAG).await;
        })
        .await;
    }
}

async fn invalid_utf8_body(ts: TestServer) {
    let env = setup(&ts, "mtu8").await;
    let name = format!("{}_a", db_name(&ts, "mtu8").await);
    let maint = admin_client(&ts, &ts.config.maintenance_db).await;
    common::drop_test_database(&maint, &name).await;
    maint
        .batch_execute(&format!("CREATE DATABASE {name} TEMPLATE template0 ENCODING 'SQL_ASCII' LC_COLLATE 'C' LC_CTYPE 'C'"))
        .await
        .unwrap();
    let db_oid: u32 = maint.query_one("SELECT oid FROM pg_database WHERE datname = $1::text", &[&name]).await.unwrap().get(0);
    let a = admin_client(&ts, &name).await;
    // 0xFF geçerli UTF-8 değildir; ad sunucu tarafında kurulur (istemci metni UTF-8 olmak zorunda).
    a.batch_execute(
        "CREATE SCHEMA s;
         DO $$ DECLARE n text := convert_from('\\x61ff'::bytea, 'SQL_ASCII'); BEGIN
           EXECUTE format('CREATE TABLE s.%I (id int)', n);
           EXECUTE format('CREATE INDEX ON s.%I (id)', n);
           EXECUTE format('CREATE SCHEMA %I', n);
         END $$;
         CREATE TABLE s.ok (id int);",
    )
    .await
    .unwrap();
    let oid_of = |sql: &'static str| {
        let a = &a;
        async move { a.query_one(sql, &[]).await.unwrap().get::<_, u32>(0) }
    };
    let load = |kind, oid| load_maintenance_target(&env.mgr, env.server, db_oid, ObjectRef { kind, oid });
    let t_oid = oid_of("SELECT oid FROM pg_class WHERE relkind = 'r' AND relnamespace = 's'::regnamespace AND relname <> 'ok'").await;
    let i_oid = oid_of("SELECT indexrelid FROM pg_index WHERE indrelid IN (SELECT oid FROM pg_class WHERE relnamespace = 's'::regnamespace AND relname <> 'ok')").await;
    let n_oid = oid_of("SELECT oid FROM pg_namespace WHERE nspname NOT IN ('s', 'public') AND nspname NOT LIKE 'pg\\_%' AND nspname <> 'information_schema'").await;
    let ok_oid = oid_of("SELECT oid FROM pg_class WHERE relname = 'ok'").await;
    for (kind, oid) in [(ObjectKind::Table, t_oid), (ObjectKind::Index, i_oid), (ObjectKind::Schema, n_oid)] {
        match load(kind, oid).await {
            Err(Error::Query { sqlstate, .. }) => assert_eq!(sqlstate.as_deref(), Some("22021"), "{kind:?}"),
            r => panic!("{kind:?}: sorgu hatası beklenir: {r:?}"),
        }
    }
    // Geçerli adlı tablo aynı veritabanında sorunsuz yüklenir (bağlantı hatadan sonra da kullanılabilir).
    let ok = load(ObjectKind::Table, ok_oid).await.unwrap();
    assert!(ok.editable && ok.name == "ok", "{ok:?}");
    drop(a);
}

// ================================================================================================================
// araştırmalar: sürüme göre sunucu davranışı (üreticinin kuralları bunlara dayanır)
// ================================================================================================================

/// Üretilen ifadeyi ayrı (uygulayıcı benzeri) bağlantıda transaction dışı çalıştırır; sunucu iletilerini döndürür.
async fn run_collect(env: &Env, sql: &str) -> Result<Vec<String>, String> {
    let mut conn = env
        .mgr
        .open_dedicated(env.server, Target::Database(env.db_oid), SessionProfile::Ddl { lock_timeout_ms: 0 })
        .await
        .unwrap();
    let r = conn.client.batch_execute(sql).await.map_err(|e| common::db_message(&e));
    drop(conn.client);
    let mut msgs = vec![];
    while let Some(n) = conn.notices.recv().await {
        msgs.push(format!("{}: {}", n.severity, n.message));
    }
    r.map(|()| msgs)
}

/// `pg_stat_user_tables` sütunu dolana kadar bekler (istatistikler gecikmeli yazılır; en çok ~10 sn).
async fn stat_set(env: &Env, rel: &str, col: &str) -> bool {
    let sql = format!("SELECT {col} IS NOT NULL FROM pg_stat_user_tables WHERE relid = '{rel}'::regclass");
    for _ in 0..100 {
        env.sql("SELECT pg_stat_clear_snapshot()").await;
        if env.admin.query_one(&sql, &[]).await.unwrap().get::<_, bool>(0) {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    false
}

fn vac() -> VacuumOptions {
    VacuumOptions {
        full: false,
        freeze: false,
        analyze: false,
        verbose: false,
        disable_page_skipping: false,
        skip_locked: false,
        index_cleanup: None,
        parallel: None,
    }
}

docker_test!(partitioned_vacuum_and_analyze_reach_partitions, partitioned_ops_body, "mtpops");
async fn partitioned_ops_body(ts: TestServer, tag: &'static str) {
    let env = setup(&ts, tag).await;
    fixture(&env).await;
    env.sql(&format!("INSERT INTO {S}.p SELECT g, g % 100 FROM generate_series(1, 1000) g")).await;
    let p = ld(&env, ObjectKind::Table, env.class_oid(S, "p").await).await;
    let ctx = BuildCtx { version_num: env.version, concurrently: false };
    let rel = format!("{S}.p1");
    let vacuum = maintenance(&ctx, &p, &MaintenanceOp::Vacuum(vac())).unwrap();
    let analyze = maintenance(&ctx, &p, &MaintenanceOp::Analyze(AnalyzeOptions { verbose: false, skip_locked: false })).unwrap();
    let vm = run_collect(&env, &vacuum.statements[0].sql).await.unwrap_or_else(|e| panic!("VACUUM: {e}"));
    let vacuumed = stat_set(&env, &rel, "last_vacuum").await;
    let am = run_collect(&env, &analyze.statements[0].sql).await.unwrap_or_else(|e| panic!("ANALYZE: {e}"));
    let analyzed = stat_set(&env, &rel, "last_analyze").await;
    eprintln!("PG {}: bölümlenmiş VACUUM → bölüm last_vacuum={vacuumed} iletiler={vm:?}; ANALYZE → bölüm last_analyze={analyzed} iletiler={am:?}", env.version);
    assert!(vacuumed && analyzed, "PG {}: bölüm işlenmedi (VACUUM {vm:?}, ANALYZE {am:?})", env.version);
    assert!(vm.iter().chain(&am).all(|m| !m.starts_with("WARNING")), "{vm:?} {am:?}");
}

docker_test!(full_with_disable_page_skipping_is_rejected_by_server, full_dps_body, "mtfdps");
async fn full_dps_body(ts: TestServer, tag: &'static str) {
    let env = setup(&ts, tag).await;
    fixture(&env).await;
    let r = run_collect(&env, &format!("VACUUM (FULL, DISABLE_PAGE_SKIPPING) {S}.t")).await;
    eprintln!("PG {}: VACUUM (FULL, DISABLE_PAGE_SKIPPING) → {r:?}", env.version);
    let e = r.expect_err("sunucu reddetmeli (üretici de reddeder)");
    assert!(e.contains("DISABLE_PAGE_SKIPPING"), "{e}");
    // Üretici aynı birleşimi doğrulamada reddeder.
    let ctx = BuildCtx { version_num: env.version, concurrently: false };
    let t = ld(&env, ObjectKind::Table, env.class_oid(S, "t").await).await;
    let o = VacuumOptions { full: true, disable_page_skipping: true, ..vac() };
    assert!(maintenance(&ctx, &t, &MaintenanceOp::Vacuum(o)).is_err());
}

// ================================================================================================================
// uygulama: `PlanRequest::Maintenance` → `DdlManager` kapıları
// ================================================================================================================

/// Kayıtlı (kilitli ya da değil) sunucuyla yönetici.
fn manager(env: &Env, ts: &TestServer, locked: bool) -> (Arc<DdlManager>, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let mut registry = Registry::load(dir.path().join("servers.json")).unwrap().registry;
    let mut config = ts.config.clone();
    config.read_only_lock = locked;
    registry.upsert(config).unwrap();
    (Arc::new(DdlManager::new(env.mgr.clone(), Arc::new(RwLock::new(registry)))), dir)
}

fn opts(hash: &str, confirmed: bool) -> ApplyOptions {
    ApplyOptions { apply_id: uuid::Uuid::new_v4(), lock_timeout_ms: 5000, expected_hash: hash.into(), confirmed, confirmed_production: false, password: None }
}

/// Hedefi yöneticiden yükler (arayüzün `ddl_load_maintenance_target` → istek akışı).
async fn request(dm: &DdlManager, env: &Env, kind: ObjectKind, oid: u32, op: MaintenanceOp) -> PlanRequest {
    let object = ObjectRef { kind, oid };
    let target = dm.load_maintenance_target(env.server, env.db_oid, object).await.unwrap_or_else(|e| panic!("{kind:?}: {e:?}"));
    PlanRequest::Maintenance { object, target, op }
}

/// Önizleme hash'iyle, onaylı uygular.
async fn apply_req(dm: &DdlManager, env: &Env, req: &PlanRequest, edit: impl FnOnce(&mut ApplyOptions)) -> pgt_core::Result<ApplyOutcome> {
    let view = dm.preview(env.server, env.db_oid, req).await.unwrap();
    assert!(view.errors.is_empty(), "{:?}", view.errors);
    let mut o = opts(&view.hash, true);
    edit(&mut o);
    dm.apply(env.server, env.db_oid, req, o).await
}

async fn applied(dm: &DdlManager, env: &Env, req: &PlanRequest) -> Vec<ServerMessage> {
    match apply_req(dm, env, req, |_| {}).await {
        Ok(ApplyOutcome::Applied { statements: 1, created: None, warnings: None, messages, .. }) => messages,
        other => panic!("uygulanmalıydı: {other:?}"),
    }
}

fn code(r: &pgt_core::Result<ApplyOutcome>) -> String {
    match r {
        Err(e) => ddl_payload(e).code,
        Ok(o) => panic!("kapı reddi bekleniyordu: {o:?}"),
    }
}

async fn filenode(env: &Env, rel: &str) -> u32 {
    env.admin.query_one(&format!("SELECT pg_relation_filenode('{rel}'::regclass)::oid"), &[]).await.unwrap().get(0)
}

fn reindex(concurrently: bool, verbose: bool) -> MaintenanceOp {
    MaintenanceOp::Reindex(ReindexOptions { concurrently, verbose })
}

docker_test!(maintenance_applies_through_the_manager, apply_body, "mtapply");
async fn apply_body(ts: TestServer, tag: &'static str) {
    let env = setup(&ts, tag).await;
    fixture(&env).await;
    env.sql(&format!("INSERT INTO {S}.t SELECT g, g, g FROM generate_series(1, 2000) g")).await;
    let (dm, _dir) = manager(&env, &ts, false);
    let t = env.class_oid(S, "t").await;
    let rel = format!("{S}.t");

    // VACUUM (Safe) ve ANALYZE: istatistik sütunları dolar.
    let vacuum = request(&dm, &env, ObjectKind::Table, t, MaintenanceOp::Vacuum(vac())).await;
    assert_eq!(dm.preview(env.server, env.db_oid, &vacuum).await.unwrap().risk, Risk::Safe);
    applied(&dm, &env, &vacuum).await;
    assert!(stat_set(&env, &rel, "last_vacuum").await, "last_vacuum dolmalı");
    let analyze = request(&dm, &env, ObjectKind::Table, t, MaintenanceOp::Analyze(AnalyzeOptions { verbose: false, skip_locked: false })).await;
    applied(&dm, &env, &analyze).await;
    assert!(stat_set(&env, &rel, "last_analyze").await, "last_analyze dolmalı");

    // VERBOSE: sunucu INFO iletileri sonuca düşer.
    let verbose = request(&dm, &env, ObjectKind::Table, t, MaintenanceOp::Vacuum(VacuumOptions { verbose: true, ..vac() })).await;
    let m = applied(&dm, &env, &verbose).await;
    assert!(!m.is_empty() && m.iter().any(|m| m.severity == "INFO"), "{m:?}");

    // REINDEX TABLE / INDEX / SCHEMA / DATABASE: indeks dosyası yenilenir (Destructive, onaylı).
    let idx = format!("{S}.t_b");
    let i = env.class_oid(S, "t_b").await;
    let s = env.schema_oid(S).await;
    for (kind, oid) in [(ObjectKind::Table, t), (ObjectKind::Index, i), (ObjectKind::Schema, s), (ObjectKind::Database, env.db_oid)] {
        let before = filenode(&env, &idx).await;
        let req = request(&dm, &env, kind, oid, reindex(false, kind == ObjectKind::Index)).await;
        assert_eq!(dm.preview(env.server, env.db_oid, &req).await.unwrap().risk, Risk::Destructive);
        let m = applied(&dm, &env, &req).await;
        assert_ne!(filenode(&env, &idx).await, before, "{kind:?}: indeks yeniden kurulmalı");
        if kind == ObjectKind::Index {
            assert!(!m.is_empty(), "REINDEX (VERBOSE) iletisi bekleniyordu");
        }
    }
    // REINDEX CONCURRENTLY (PG 12+; Safe).
    if env.version >= 120000 {
        let before = filenode(&env, &idx).await;
        let req = request(&dm, &env, ObjectKind::Index, i, reindex(true, false)).await;
        assert_eq!(dm.preview(env.server, env.db_oid, &req).await.unwrap().risk, Risk::Safe);
        applied(&dm, &env, &req).await;
        assert_ne!(filenode(&env, &idx).await, before);
        let req = request(&dm, &env, ObjectKind::Table, t, reindex(true, false)).await;
        applied(&dm, &env, &req).await;
    } else {
        eprintln!("PG {}: REINDEX CONCURRENTLY yok (PG 12+), atlandı", env.version);
    }

    // CLUSTER USING: kümeleme indeksi değişir; USING'siz CLUSTER kayıtlı kümeleme indeksini kullanır.
    let clustered = |name: &'static str| {
        let sql = format!("SELECT indisclustered FROM pg_index WHERE indexrelid = '{S}.{name}'::regclass");
        let admin = &env.admin;
        async move { admin.query_one(&sql, &[]).await.unwrap().get::<_, bool>(0) }
    };
    let using = MaintenanceOp::Cluster(ClusterOptions { verbose: true, using_index: Some("t_pkey".into()) });
    let using = request(&dm, &env, ObjectKind::Table, t, using).await;
    let m = applied(&dm, &env, &using).await;
    assert!(clustered("t_pkey").await && !clustered("t_b").await);
    assert!(!m.is_empty(), "CLUSTER VERBOSE iletisi bekleniyordu");
    let plain = request(&dm, &env, ObjectKind::Table, t, MaintenanceOp::Cluster(ClusterOptions { verbose: false, using_index: None })).await;
    applied(&dm, &env, &plain).await;
    assert!(clustered("t_pkey").await);

    // MV ve veritabanı hedefi (bağlı veritabanı; bakım veritabanı değil).
    let mv_op = MaintenanceOp::Vacuum(VacuumOptions { analyze: true, ..vac() });
    let mv = request(&dm, &env, ObjectKind::MaterializedView, env.class_oid(S, "mv").await, mv_op).await;
    applied(&dm, &env, &mv).await;
    let db_op = MaintenanceOp::Analyze(AnalyzeOptions { verbose: false, skip_locked: false });
    let db = request(&dm, &env, ObjectKind::Database, env.db_oid, db_op).await;
    applied(&dm, &env, &db).await;
}

docker_test!(maintenance_gates_hold, gates_body, "mtgates");
async fn gates_body(ts: TestServer, tag: &'static str) {
    let env = setup(&ts, tag).await;
    fixture(&env).await;
    let (dm, _dir) = manager(&env, &ts, false);
    let t = env.class_oid(S, "t").await;
    let full = request(&dm, &env, ObjectKind::Table, t, MaintenanceOp::Vacuum(VacuumOptions { full: true, ..vac() })).await;
    let hash = dm.preview(env.server, env.db_oid, &full).await.unwrap().hash;

    // Kilitli sunucu: önizleme serbest, uygulama `locked`.
    let (locked, _d) = manager(&env, &ts, true);
    assert_eq!(code(&locked.apply(env.server, env.db_oid, &full, opts(&hash, true)).await), "locked");
    assert_eq!(locked.preview(env.server, env.db_oid, &full).await.unwrap().statements.len(), 1);
    // Destructive (VACUUM FULL) onaysız → `confirmation_required`.
    assert_eq!(code(&dm.apply(env.server, env.db_oid, &full, opts(&hash, false)).await), "confirmation_required");
    // Veritabanı gerekir (Database türünde de).
    let dbreq = request(&dm, &env, ObjectKind::Database, env.db_oid, MaintenanceOp::Vacuum(vac())).await;
    let dbhash = dm.preview(env.server, env.db_oid, &dbreq).await.unwrap().hash;
    match dm.apply(env.server, None, &dbreq, opts(&dbhash, true)).await {
        Err(Error::Invalid(m)) => assert_eq!(m, pgt_ddl::error_codes::NEEDS_DATABASE),
        other => panic!("{other:?}"),
    }
    // Sahte hedef (başka ad/şema): yeniden yükleme eşitliği yakalar.
    let PlanRequest::Maintenance { object, target, op } = full.clone() else { unreachable!() };
    for spoof in [MaintenanceTarget { name: "mv".into(), ..target.clone() }, MaintenanceTarget { schema: Some("public".into()), ..target.clone() }] {
        let req = PlanRequest::Maintenance { object, target: spoof, op: op.clone() };
        let h = dm.preview(env.server, env.db_oid, &req).await.unwrap().hash;
        assert_eq!(code(&dm.apply(env.server, env.db_oid, &req, opts(&h, true)).await), "plan_changed");
    }
    // Bayat hash.
    let other = request(&dm, &env, ObjectKind::Table, t, MaintenanceOp::Vacuum(vac())).await;
    assert_eq!(code(&dm.apply(env.server, env.db_oid, &other, opts(&hash, true)).await), "plan_changed");
    // Bayat hedef: yükleme sonrası yeniden adlandırma → `plan_changed`; silme → `not_found`.
    env.sql(&format!("ALTER TABLE {S}.t RENAME TO t2")).await;
    assert_eq!(code(&dm.apply(env.server, env.db_oid, &full, opts(&hash, true)).await), "plan_changed");
    env.sql(&format!("DROP TABLE {S}.t2 CASCADE")).await;
    assert_eq!(code(&dm.apply(env.server, env.db_oid, &full, opts(&hash, true)).await), "not_found");
    // Nesne türü ↔ hedef türü uyuşmazlığı ve salt okunur hedef bağlantıdan önce reddedilir.
    let mismatched = PlanRequest::Maintenance { object: ObjectRef { kind: ObjectKind::Index, oid: object.oid }, target: target.clone(), op: op.clone() };
    assert_eq!(code(&dm.apply(env.server, env.db_oid, &mismatched, opts(&hash, true)).await), "invalid_def");
    let ro = PlanRequest::Maintenance { object, target: MaintenanceTarget { editable: false, ..target }, op };
    assert!(matches!(dm.apply(env.server, env.db_oid, &ro, opts(&hash, true)).await, Err(Error::Invalid(_))));
}

/// Uygulama oturumlarından kaçı kilit bekliyor.
async fn ddl_waiting(admin: &tokio_postgres::Client) -> i64 {
    let sql = "SELECT count(*) FROM pg_stat_activity WHERE application_name = 'pg-tools ddl' AND wait_event_type = 'Lock' \
               AND datname = current_database()";
    admin.query_one(sql, &[]).await.unwrap().get(0)
}

/// Tabloyu ACCESS EXCLUSIVE kilitleyen ayrı oturum (transaction açık kalır).
async fn lock_table(ts: &TestServer, tag: &str, rel: &str) -> tokio_postgres::Client {
    let locker = admin_client(ts, &db_name(ts, tag).await).await;
    locker.batch_execute(&format!("BEGIN; LOCK TABLE {rel} IN ACCESS EXCLUSIVE MODE")).await.unwrap();
    locker
}

docker_test!(maintenance_lock_timeout_and_cancel, lock_body, "mtlock");
async fn lock_body(ts: TestServer, tag: &'static str) {
    let env = setup(&ts, tag).await;
    fixture(&env).await;
    let (dm, _dir) = manager(&env, &ts, false);
    let t = env.class_oid(S, "t").await;
    let full_op = MaintenanceOp::Vacuum(VacuumOptions { full: true, verbose: true, ..vac() });
    let full = request(&dm, &env, ObjectKind::Table, t, full_op).await;
    let locker = lock_table(&ts, tag, &format!("{S}.t")).await;

    // lock_timeout aşımı: 55P03 + bakım kilit ipucu.
    let started = Instant::now();
    let r = apply_req(&dm, &env, &full, |o| o.lock_timeout_ms = 300).await;
    let Ok(ApplyOutcome::Failed { failed_index: 0, committed: 0, sqlstate, error, .. }) = r else { panic!("{r:?}") };
    assert_eq!(sqlstate.as_deref(), Some("55P03"));
    assert_eq!(error.hint.as_deref(), Some(pgt_ddl::manager::MAINTENANCE_LOCK_HINT));
    assert!(started.elapsed() < Duration::from_secs(5));

    // İptal: kilit beklerken `cancel(apply_id)` → 57014, ipucusuz; ilerleme sorgusu bekleme sırasında hatasız.
    let hash = dm.preview(env.server, env.db_oid, &full).await.unwrap().hash;
    let o = ApplyOptions { lock_timeout_ms: 0, ..opts(&hash, true) };
    let id = o.apply_id;
    let (dm2, server, db, req) = (dm.clone(), env.server, env.db_oid, full.clone());
    let task = tokio::spawn(async move { dm2.apply(server, db, &req, o).await });
    let deadline = Instant::now() + Duration::from_secs(15);
    while ddl_waiting(&env.admin).await != 1 {
        assert!(Instant::now() < deadline, "bekleyen bakım oturumu görülmedi");
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    // Kilit beklerken ilerleme satırı olmayabilir (VACUUM FULL kilidi aldıktan sonra raporlar); sorgu hatasız döner.
    let p = dm.maintenance_progress(env.server, env.db_oid, id).await.unwrap();
    eprintln!("PG {}: kilit beklerken ilerleme = {p:?}", env.version);
    dm.cancel(id).await.unwrap();
    let r = tokio::time::timeout(Duration::from_secs(15), task).await.expect("iptal 15 sn içinde sonuçlanmalı").unwrap();
    let Ok(ApplyOutcome::Failed { sqlstate, error, .. }) = r else { panic!("{r:?}") };
    assert_eq!(sqlstate.as_deref(), Some("57014"));
    assert_eq!(error.hint, None, "{error:?}");
    // Bitmiş kimlik: ilerleme `None`, iptal Ok.
    assert_eq!(dm.maintenance_progress(env.server, env.db_oid, id).await.unwrap(), None);
    dm.cancel(id).await.unwrap();
    locker.batch_execute("COMMIT").await.unwrap();
    assert_eq!(ddl_waiting(&env.admin).await, 0);
}

// İlerleme (en iyi çaba): büyük tabloda VACUUM ve VACUUM FULL sürerken yoklanır. Satır gelirse alanlar tutarlı
// olmalı; gelmezse (kısa sürdü) `None` kabul. Her sürümde sorgu hatasızdır (PG 10–11'de yalnız vacuum görünümü).
docker_test!(maintenance_progress_is_read_while_running, progress_body, "mtprog");
async fn progress_body(ts: TestServer, tag: &'static str) {
    let env = setup(&ts, tag).await;
    env.sql(&format!(
        "CREATE SCHEMA {S};
         CREATE TABLE {S}.big (id int, pad text);
         INSERT INTO {S}.big SELECT g, repeat('x', 100) FROM generate_series(1, 200000) g;
         DELETE FROM {S}.big WHERE id % 2 = 0;"
    ))
    .await;
    let (dm, _dir) = manager(&env, &ts, false);
    let big = env.class_oid(S, "big").await;
    for op in [MaintenanceOp::Vacuum(vac()), MaintenanceOp::Vacuum(VacuumOptions { full: true, ..vac() })] {
        let req = request(&dm, &env, ObjectKind::Table, big, op.clone()).await;
        let hash = dm.preview(env.server, env.db_oid, &req).await.unwrap().hash;
        let o = opts(&hash, true);
        let id = o.apply_id;
        let (dm2, server, db) = (dm.clone(), env.server, env.db_oid);
        let task = tokio::spawn(async move { dm2.apply(server, db, &req, o).await });
        let mut seen = vec![];
        while !task.is_finished() {
            let p = dm.maintenance_progress(env.server, env.db_oid, id).await.expect("ilerleme sorgusu hatasız olmalı");
            seen.extend(p);
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        let r = task.await.unwrap();
        assert!(matches!(r, Ok(ApplyOutcome::Applied { .. })), "{r:?}");
        eprintln!("PG {}: {op:?} ilerleme satırları = {}, örnek = {:?}", env.version, seen.len(), seen.first());
        for p in &seen {
            assert!(!p.phase.is_empty(), "{p:?}");
            if let (Some(d), Some(t)) = (p.done, p.total) {
                assert!(d <= t, "{p:?}");
            }
            if p.total.is_some() {
                assert!(p.done.is_some() && p.unit.is_some(), "{p:?}");
            }
            assert!(p.unit.as_deref().is_none_or(|u| ["blocks", "tuples", "indexes"].contains(&u)), "{p:?}");
        }
        assert_eq!(dm.maintenance_progress(env.server, env.db_oid, id).await.unwrap(), None);
    }
}
