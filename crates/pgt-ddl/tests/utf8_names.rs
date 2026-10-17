//! Adı geçersiz UTF-8 olan multirange tipi (PG 14+; `pgt_core::testing::INVALID_UTF8_TYPE_NAME_SQL`) bulunan
//! veritabanında yükleyiciler, aramalar ve veri kaynağı (Docker; `PGT_TEST_URLS` yoksa ya da PG < 14 ise atlanır).
//!
//! Beklenen: hiçbir yol çökmez. Kayıplı ad (U+FFFD) yalnızca gösterime girer: o adı taşıyan nesne salt okunur yüklenir
//! (`INVALID_UTF8_REASON`; değişiklik ve drop `object` hatası), aramalar SQL'e yazılacak değeri kayıplı döndürmez.

mod common;

use std::sync::{Arc, RwLock};

use common::{Env, cleanup_in, guarded, servers, setup_in};
use pgt_core::registry::Registry;
use pgt_core::testing::{INVALID_UTF8_TYPE_NAME_SQL, TestServer, invalid_utf8_range_name};
use pgt_ddl::data::manager::DataManager;
use pgt_ddl::*;

const PFX: &str = "pgt_ddl_u8";

macro_rules! docker_test {
    ($name:ident, $body:ident, $tag:literal) => {
        #[tokio::test]
        async fn $name() {
            for ts in servers() {
                guarded($body(ts.clone()), cleanup_in(&ts, PFX, $tag)).await;
            }
        }
    };
}

/// PG 14+ ise fixture'lı ortam.
async fn env(ts: &TestServer, tag: &str) -> Option<Env> {
    let env = setup_in(ts, PFX, tag).await;
    if env.version < 140000 {
        eprintln!("PG {}: multirange yok, atlanıyor", env.version / 10000);
        return None;
    }
    env.sql(INVALID_UTF8_TYPE_NAME_SQL).await;
    Some(env)
}

fn lossy(s: &str) -> bool {
    s.contains('\u{FFFD}')
}

async fn oid(env: &Env, sql: &str) -> u32 {
    env.admin.query_one(sql, &[]).await.unwrap().get(0)
}

async fn load_ok(env: &Env, kind: ObjectKind, oid: u32) -> ObjectDef {
    load(&env.mgr, env.server, env.db_oid, ObjectRef { kind, oid }).await.unwrap_or_else(|e| panic!("{kind:?} yüklenemedi: {e:?}"))
}

fn has_reason(unsupported: &[String]) -> bool {
    unsupported.iter().any(|u| u == INVALID_UTF8_REASON)
}

/// Salt okunur: değişiklik ve drop `object` hatası; `alter(l, l)` boş plan.
fn assert_read_only(env: &Env, l: &ObjectDef, changed: &ObjectDef) {
    let e = build::alter(l, changed, &env.ctx(false)).unwrap_err();
    assert_eq!(e.iter().map(|e| e.path.as_str()).collect::<Vec<_>>(), ["object"], "{e:?}");
    let p = build::alter(l, l, &env.ctx(false)).unwrap();
    assert!(p.statements.is_empty(), "{:?}", p.statements);
    let e = build::drop(l, false, &env.ctx(false)).unwrap_err();
    assert_eq!(e.iter().map(|e| e.path.as_str()).collect::<Vec<_>>(), ["object"], "{e:?}");
}

const MULTIRANGE_OID: &str = "SELECT r.rngmultitypid FROM pg_range r JOIN pg_type t ON t.oid = r.rngtypid WHERE t.typnamespace = 'u8'::regnamespace";

async fn table_body(ts: TestServer) {
    let Some(env) = env(&ts, "table").await else { return };
    let ObjectDef::Table(t) = load_ok(&env, ObjectKind::Table, env.class_oid("u8", "t").await).await else { unreachable!() };
    let m = t.columns.iter().find(|c| c.name == "m").unwrap();
    assert!(lossy(&m.data_type), "{m:?}");
    assert!(has_reason(&t.unsupported), "{:?}", t.unsupported);
    let l = ObjectDef::Table(t.clone());
    assert_read_only(&env, &l, &ObjectDef::Table(TableDef { comment: Some("x".into()), ..t.clone() }));
    // Görünüm kolonları da tipi taşır.
    let v = load_ok(&env, ObjectKind::View, env.class_oid("u8", "v").await).await;
    let ObjectDef::View(vd) = &v else { unreachable!() };
    assert!(has_reason(&vd.unsupported), "{:?}", vd.unsupported);
}
docker_test!(table_with_invalid_utf8_column_type_loads_read_only, table_body, "table");

async fn routine_domain_type_body(ts: TestServer) {
    let Some(env) = env(&ts, "rdt").await else { return };
    let ObjectDef::Domain(d) = load_ok(&env, ObjectKind::Domain, oid(&env, "SELECT 'u8.dm'::regtype::oid").await).await else { unreachable!() };
    assert!(lossy(&d.base_type) && !d.editable && has_reason(&d.unsupported), "{d:?}");
    assert_read_only(&env, &ObjectDef::Domain(d.clone()), &ObjectDef::Domain(DomainDef { comment: Some("x".into()), ..d.clone() }));

    let f_oid = oid(&env, "SELECT p.oid FROM pg_proc p WHERE p.pronamespace = 'u8'::regnamespace AND p.proname = 'f'").await;
    let ObjectDef::Function(f) = load_ok(&env, ObjectKind::Function, f_oid).await else { unreachable!() };
    assert!(lossy(&f.args[0].type_name) && !f.editable && has_reason(&f.unsupported), "{f:?}");
    assert_read_only(&env, &ObjectDef::Function(f.clone()), &ObjectDef::Function(FunctionDef { comment: Some("x".into()), ..f.clone() }));

    // Multirange tipinin kendisi ve dizisi: ad kayıplı, salt okunur.
    let m_oid = oid(&env, MULTIRANGE_OID).await;
    let a_oid = oid(&env, &format!("SELECT typarray FROM pg_type WHERE oid = {m_oid}")).await;
    for o in [m_oid, a_oid] {
        let ObjectDef::Type(t) = load_ok(&env, ObjectKind::Type, o).await else { unreachable!() };
        assert!(lossy(&t.name) && !t.editable && has_reason(&t.unsupported), "{t:?}");
    }
    // Range'in kendisi düzenlenebilir kalır (adı geçerli; multirange adı otomatik → None).
    let r_oid = oid(&env, "SELECT r.rngtypid FROM pg_range r JOIN pg_type t ON t.oid = r.rngtypid WHERE t.typnamespace = 'u8'::regnamespace").await;
    let ObjectDef::Type(r) = load_ok(&env, ObjectKind::Type, r_oid).await else { unreachable!() };
    assert_eq!(r.name, invalid_utf8_range_name());
    assert!(r.editable, "{:?}", r.unsupported);
}
docker_test!(domain_function_and_type_load_read_only, routine_domain_type_body, "rdt");

async fn lookup_body(ts: TestServer) {
    let Some(env) = env(&ts, "lookup").await else { return };
    let run = |req: LookupRequest| {
        let mgr = env.mgr.clone();
        let (server, db) = (env.server, env.db_oid);
        async move { lookup(&mgr, server, db, req).await.unwrap_or_else(|e| panic!("arama başarısız: {e:?}")) }
    };
    for q in [None, Some("multi".to_string()), Some("ş".to_string()), Some("u8".to_string())] {
        let items = run(LookupRequest::Types { query: q.clone() }).await;
        assert!(items.iter().all(|i| !lossy(&i.value)), "{q:?}: SQL'e yazılacak değer kayıplı olamaz");
        if q.as_deref() == Some("u8") {
            assert!(items.iter().any(|i| i.value.contains(&invalid_utf8_range_name())), "range listelenir");
        }
    }
    let cols = run(LookupRequest::TableColumns { schema: "u8".into(), table: "t".into() }).await;
    let m = cols.iter().find(|i| i.value == "m").expect("m kolonu");
    assert!(m.detail.as_deref().is_some_and(lossy), "{m:?}");
}
docker_test!(lookups_skip_or_display_invalid_utf8, lookup_body, "lookup");

async fn data_source_body(ts: TestServer) {
    let Some(env) = env(&ts, "data").await else { return };
    let dir = tempfile::tempdir().unwrap();
    let mut registry = Registry::load(dir.path().join("servers.json")).unwrap().registry;
    registry.upsert(ts.config.clone()).unwrap();
    let dm = DataManager::new(env.mgr.clone(), Arc::new(RwLock::new(registry)));
    let src = dm.load(env.server, env.db_oid, ObjectRef { kind: ObjectKind::Table, oid: env.class_oid("u8", "t").await }).await.unwrap();
    let m = src.columns.iter().find(|c| c.name == "m").unwrap();
    assert!(lossy(&m.type_display), "{m:?}");
}
docker_test!(data_source_displays_invalid_utf8_type_lossily, data_source_body, "data");
