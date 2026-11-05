//! `load_in`/`load_acl_in` (istemci alan yükleyiciler) ↔ `load`/`load_acl` eşdeğerliği ve anlık görüntü öncülü
//! (Docker; `PGT_TEST_URLS` yoksa atlanır).
//!
//! Fikstür veritabanı `pgt_ddl_li_<ana sürüm>_all` (ikinci test `_snap`; paralel koşar), şema `s`. Her nesne için havuzlu yükleme ile ayrılmış bağlantıdaki
//! yükleme aynı sonucu vermelidir. İkinci test spec §2.1 öncülünü sınar: ayrılmış bağlantıda `search_path` yerel
//! olarak `pg_catalog, s` yapılınca normalleştirilmiş çıktı niteliksiz gelir; havuz bağlantısı etkilenmez.

mod common;

use common::{Env, cleanup_in, guarded, servers, setup_in};
use pgt_core::Error;
use pgt_core::connection::{SessionProfile, Target};
use pgt_core::testing::TestServer;
use pgt_ddl::*;

const PFX: &str = "pgt_ddl_li";
const TAG: &str = "all";
const SNAP_TAG: &str = "snap";

async fn fixture(ts: &TestServer, tag: &str) -> Env {
    let env = setup_in(ts, PFX, tag).await;
    env.sql(
        "CREATE SCHEMA s;
         CREATE TYPE s.mood AS ENUM ('a','b');
         CREATE TYPE s.pair AS (x int, y s.mood);
         CREATE TYPE s.r AS RANGE (subtype = int4);
         CREATE DOMAIN s.pos AS int CHECK (VALUE > 0);
         CREATE TABLE s.p (id serial PRIMARY KEY, m s.mood DEFAULT 'a', d s.pos);
         CREATE TABLE s.c (id int PRIMARY KEY, p_id int REFERENCES s.p(id), u text UNIQUE, CHECK (id > 0));
         CREATE INDEX c_u ON s.c (lower(u));
         CREATE FUNCTION s.f(a s.mood) RETURNS int LANGUAGE sql AS 'SELECT 1';
         CREATE FUNCTION s.tf() RETURNS trigger LANGUAGE plpgsql AS 'BEGIN RETURN NEW; END';
         CREATE TRIGGER tg BEFORE INSERT ON s.c FOR EACH ROW EXECUTE PROCEDURE s.tf();
         CREATE VIEW s.v AS SELECT * FROM s.c;
         CREATE MATERIALIZED VIEW s.mv AS SELECT id FROM s.p;
         GRANT SELECT ON s.c TO PUBLIC;",
    )
    .await;
    if env.version >= 110000 {
        env.sql("CREATE PROCEDURE s.pr() LANGUAGE sql AS ''").await;
    }
    env
}

async fn trigger_oid(env: &Env) -> u32 {
    env.admin.query_one("SELECT oid FROM pg_trigger WHERE tgname = 'tg'", &[]).await.unwrap().get(0)
}

/// Fikstürdeki tüm yüklenebilir nesneler (tür, oid).
async fn objects(env: &Env) -> Vec<ObjectRef> {
    let r = |kind, oid| ObjectRef { kind, oid };
    let mut v = vec![
        r(ObjectKind::Schema, env.schema_oid("s").await),
        r(ObjectKind::Sequence, env.class_oid("s", "p_id_seq").await),
        r(ObjectKind::Index, env.class_oid("s", "c_u").await),
        r(ObjectKind::Index, env.class_oid("s", "p_pkey").await),
        r(ObjectKind::Table, env.class_oid("s", "p").await),
        r(ObjectKind::Table, env.class_oid("s", "c").await),
        r(ObjectKind::View, env.class_oid("s", "v").await),
        r(ObjectKind::MaterializedView, env.class_oid("s", "mv").await),
        r(ObjectKind::Function, env.proc_oid("s", "f", "s.mood").await),
        r(ObjectKind::Function, env.proc_oid("s", "tf", "").await),
        r(ObjectKind::Trigger, trigger_oid(env).await),
        r(ObjectKind::Type, env.type_oid("s", "mood").await),
        r(ObjectKind::Type, env.type_oid("s", "pair").await),
        r(ObjectKind::Type, env.type_oid("s", "r").await),
        r(ObjectKind::Domain, env.type_oid("s", "pos").await),
    ];
    if env.version >= 110000 {
        v.push(r(ObjectKind::Procedure, env.proc_oid("s", "pr", "").await));
    } else {
        eprintln!("procedure adımı PG 11+ gerektirir, atlanıyor");
    }
    if env.version >= 140000 {
        v.push(r(ObjectKind::Type, env.type_oid("s", "r_multirange").await));
    } else {
        eprintln!("multirange adımı PG 14+ gerektirir, atlanıyor");
    }
    v
}

async fn equivalence(ts: TestServer) {
    let env = fixture(&ts, TAG).await;
    let dedicated = env.mgr.open_dedicated(env.server, Target::Database(env.db_oid), SessionProfile::Catalog).await.unwrap();
    for obj in objects(&env).await {
        let pooled = load(&env.mgr, env.server, env.db_oid, obj).await.unwrap_or_else(|e| panic!("{obj:?} yüklenemedi: {e:?}"));
        let inner = load_in(&dedicated.client, env.version, obj).await.unwrap_or_else(|e| panic!("{obj:?} load_in: {e:?}"));
        assert_eq!(pooled, inner, "{:?}", obj.kind);
        if acl_supported(obj.kind) {
            let pooled = load_acl(&env.mgr, env.server, env.db_oid, obj).await.unwrap();
            let inner = load_acl_in(&dedicated.client, env.version, obj).await.unwrap();
            assert_eq!(pooled, inner, "acl {:?}", obj.kind);
        }
    }
    // Yok olan nesne: iki yol da aynı hatayı verir.
    let missing = ObjectRef { kind: ObjectKind::Table, oid: 1 };
    let a = load(&env.mgr, env.server, env.db_oid, missing).await.unwrap_err();
    let b = load_in(&dedicated.client, env.version, missing).await.unwrap_err();
    assert!(matches!((&a, &b), (Error::NotFound(x), Error::NotFound(y)) if x == y), "{a:?} / {b:?}");
    // Sunucu kapsamlı türler istemci alan yolda reddedilir.
    for kind in [ObjectKind::Database, ObjectKind::Role] {
        let err = load_in(&dedicated.client, env.version, ObjectRef { kind, oid: 1 }).await.unwrap_err();
        assert!(matches!(&err, Error::Invalid(m) if m == SERVER_SCOPED_MSG), "{err:?}");
    }
    let db = ObjectRef { kind: ObjectKind::Database, oid: env.db_oid };
    let err = load_acl_in(&dedicated.client, env.version, db).await.unwrap_err();
    assert!(matches!(&err, Error::Invalid(m) if m == SERVER_SCOPED_MSG), "{err:?}");
    let role = ObjectRef { kind: ObjectKind::Role, oid: 1 };
    let err = load_acl_in(&dedicated.client, env.version, role).await.unwrap_err();
    assert!(matches!(&err, Error::Invalid(m) if m == load_acl::UNSUPPORTED_KIND_MSG), "{err:?}");
}

#[tokio::test]
async fn load_in_matches_load() {
    for ts in servers() {
        guarded(equivalence(ts.clone()), cleanup_in(&ts, PFX, TAG)).await;
    }
}

async fn pool_search_path(env: &Env) -> String {
    env.mgr
        .run(env.server, Target::Database(env.db_oid), |c| async move {
            c.client.query_one("SHOW search_path", &[]).await.map(|r| r.get::<_, String>(0))
        })
        .await
        .unwrap()
}

fn column(t: &TableDef, name: &str) -> ColumnDef {
    t.columns.iter().find(|c| c.name == name).unwrap().clone()
}

async fn snapshot(ts: TestServer) {
    let env = fixture(&ts, SNAP_TAG).await;
    let dedicated = env.mgr.open_dedicated(env.server, Target::Database(env.db_oid), SessionProfile::Catalog).await.unwrap();
    let c = &dedicated.client;
    c.batch_execute("BEGIN ISOLATION LEVEL REPEATABLE READ READ ONLY").await.unwrap();
    c.batch_execute("SELECT pg_catalog.set_config('search_path', 'pg_catalog, s', true)").await.unwrap();
    let r = |kind, oid| ObjectRef { kind, oid };
    let v_ref = r(ObjectKind::View, env.class_oid("s", "v").await);
    let p_ref = r(ObjectKind::Table, env.class_oid("s", "p").await);
    let f_ref = r(ObjectKind::Function, env.proc_oid("s", "f", "s.mood").await);
    let pair_ref = r(ObjectKind::Type, env.type_oid("s", "pair").await);

    // search_path = pg_catalog, s → niteliksiz.
    let ObjectDef::View(v) = load_in(c, env.version, v_ref).await.unwrap() else { panic!("view bekleniyordu") };
    assert!(v.query.contains("FROM c") && !v.query.contains("FROM s.c"), "{}", v.query);
    let ObjectDef::Table(p) = load_in(c, env.version, p_ref).await.unwrap() else { panic!("table bekleniyordu") };
    assert_eq!(column(&p, "m").data_type, "mood");
    assert_eq!(column(&p, "m").default.as_deref(), Some("'a'::mood"));
    assert_eq!(column(&p, "id").default.as_deref(), Some("nextval('p_id_seq'::regclass)"));
    let ObjectDef::Function(f) = load_in(c, env.version, f_ref).await.unwrap() else { panic!("function bekleniyordu") };
    assert_eq!(f.args[0].type_name, "mood");
    let ObjectDef::Type(t) = load_in(c, env.version, pair_ref).await.unwrap() else { panic!("type bekleniyordu") };
    let TypeBody::Composite { attributes } = &t.body else { panic!("composite bekleniyordu") };
    assert_eq!(attributes.iter().find(|a| a.name == "y").unwrap().type_name, "mood");
    c.batch_execute("ROLLBACK").await.unwrap();

    // Havuz (search_path = pg_catalog) → s. nitelemeli.
    let ObjectDef::View(v) = load(&env.mgr, env.server, env.db_oid, v_ref).await.unwrap() else { panic!() };
    assert!(v.query.contains("FROM s.c"), "{}", v.query);
    let ObjectDef::Table(p) = load(&env.mgr, env.server, env.db_oid, p_ref).await.unwrap() else { panic!() };
    assert_eq!(column(&p, "m").data_type, "s.mood");
    assert_eq!(column(&p, "m").default.as_deref(), Some("'a'::s.mood"));
    assert_eq!(column(&p, "id").default.as_deref(), Some("nextval('s.p_id_seq'::regclass)"));
    let ObjectDef::Function(f) = load(&env.mgr, env.server, env.db_oid, f_ref).await.unwrap() else { panic!() };
    assert_eq!(f.args[0].type_name, "s.mood");
    let ObjectDef::Type(t) = load(&env.mgr, env.server, env.db_oid, pair_ref).await.unwrap() else { panic!() };
    let TypeBody::Composite { attributes } = &t.body else { panic!() };
    assert_eq!(attributes.iter().find(|a| a.name == "y").unwrap().type_name, "s.mood");

    // Sızıntı yok.
    assert_eq!(pool_search_path(&env).await, "pg_catalog");
}

#[tokio::test]
async fn snapshot_search_path_premise() {
    for ts in servers() {
        guarded(snapshot(ts.clone()), cleanup_in(&ts, PFX, SNAP_TAG)).await;
    }
}
