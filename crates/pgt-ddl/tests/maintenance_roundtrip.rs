//! Bakım işlemleri (Plan 24) Docker testleri: hedef yükleyicisi (`load_maintenance_target`) ve üreticinin
//! (`build::maintenance`) ürettiği ifadenin sunucuda düz çalıştırılması (`PlanRequest::Maintenance` Task 4'te).
//! `PGT_TEST_URLS` yoksa atlanır.
//!
//! Her test kendi geçici veritabanını (`pgt_ddl_rt_<ana sürüm>_<etiket>`) ve `pgt_ddl_rt_<etiket>` şemasını kurar;
//! başarısız doğrulamadan sonra bile veritabanı silinir (`guarded` + `cleanup`). Sürüme bağlı kısımlar (bölümlenmiş
//! indeks PG 11+) atlandığında yazdırılır.

mod common;

use std::time::Duration;

use common::{Env, cleanup, db_name, guarded, servers, setup};
use pgt_core::Error;
use pgt_core::connection::{SessionProfile, Target};
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
