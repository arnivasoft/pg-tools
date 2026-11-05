//! Liste sorgusu, şema denetimi ve taraf yükleyicisi (Docker; `PGT_TEST_URLS` yoksa atlanır).
//!
//! Fikstür veritabanları `pgt_diff_<ana sürüm>_ls` (liste) ve `_side` (yükleyici; testler paralel koşar),
//! şema `s` (+ karşılaştırma için `s2`).

mod common;

use std::sync::Mutex;
use std::time::{Duration, Instant};

use common::{Env, cleanup_in, guarded, servers, setup_in};
use pgt_core::connection::{SessionProfile, Target};
use pgt_core::testing::TestServer;
use pgt_ddl::{ObjectDef, ObjectKind};
use pgt_diff::snapshot::load_objects;
use pgt_diff::{
    CancelToken, DiffSide, Listed, SchemaCheck, SnapshotError, list_objects, load_side,
    schema_check,
};

const PFX: &str = "pgt_diff";
const LIST_TAG: &str = "ls";
const SIDE_TAG: &str = "side";

async fn fixture(ts: &TestServer, tag: &str) -> Env {
    let env = setup_in(ts, PFX, tag).await;
    env.sql(
        "CREATE SCHEMA s;
         CREATE SCHEMA s2;
         CREATE TYPE s.mood AS ENUM ('a','b');
         CREATE TYPE s.pair AS (x int, y s.mood);
         CREATE TYPE s.r AS RANGE (subtype = int4);
         CREATE DOMAIN s.pos AS int CHECK (VALUE > 0);
         CREATE TABLE s.ser (id serial PRIMARY KEY, u text UNIQUE, m s.mood, d s.pos);
         CREATE TABLE s.idt (id int GENERATED ALWAYS AS IDENTITY, n int);
         CREATE TABLE s.ch (id int PRIMARY KEY, p int REFERENCES s.ser(id));
         CREATE INDEX ser_n ON s.ser (lower(u));
         CREATE FUNCTION s.tf() RETURNS trigger LANGUAGE plpgsql AS 'BEGIN RETURN NEW; END';
         CREATE TRIGGER ch_tg BEFORE INSERT ON s.ch FOR EACH ROW EXECUTE PROCEDURE s.tf();
         CREATE FUNCTION s.f(a integer) RETURNS int LANGUAGE sql AS 'SELECT 1';
         CREATE FUNCTION s.f(a text) RETURNS int LANGUAGE sql AS 'SELECT 2';
         CREATE FUNCTION s.g(a integer) RETURNS int LANGUAGE sql AS 'SELECT 1';
         CREATE FUNCTION s2.g(b integer) RETURNS int LANGUAGE sql AS 'SELECT 1';
         CREATE AGGREGATE s.agg(int) (sfunc = int4pl, stype = int);
         CREATE VIEW s.v1 AS SELECT id FROM s.ser;
         CREATE VIEW s.v2 AS SELECT * FROM s.v1;
         CREATE MATERIALIZED VIEW s.mv AS SELECT id FROM s.ser;
         CREATE TABLE s.pt (id int, v int) PARTITION BY RANGE (id);
         CREATE TABLE s.pt_1 PARTITION OF s.pt FOR VALUES FROM (0) TO (10);
         GRANT SELECT ON s.ser TO PUBLIC;",
    )
    .await;
    if env.has_extension("citext").await {
        env.sql("CREATE EXTENSION IF NOT EXISTS citext SCHEMA s")
            .await;
    } else {
        eprintln!("citext eklentisi yok, uzantı üyesi adımı atlanıyor");
    }
    if env.version >= 110000 {
        env.sql(
            "CREATE PROCEDURE s.pr() LANGUAGE sql AS '';
             CREATE INDEX pt_v ON s.pt (v);
             CREATE TRIGGER pt_tg AFTER INSERT ON s.pt FOR EACH ROW EXECUTE PROCEDURE s.tf();",
        )
        .await;
    } else {
        eprintln!("procedure/bölüm indeksi/bölüm trigger adımları PG 11+ gerektirir, atlanıyor");
    }
    env
}

type Row = (String, Option<String>, String, Option<String>);

fn row(kind: ObjectKind, table: Option<&str>, name: &str, sig: Option<&str>) -> Row {
    (
        format!("{kind:?}"),
        table.map(str::to_string),
        name.to_string(),
        sig.map(str::to_string),
    )
}

fn rows_of(l: &[Listed]) -> Vec<Row> {
    let mut v: Vec<Row> = l
        .iter()
        .map(|x| {
            (
                format!("{:?}", x.kind),
                x.table.clone(),
                x.name.clone(),
                x.signature.clone(),
            )
        })
        .collect();
    v.sort();
    v
}

async fn oid_of(env: &Env, schema: &str, name: &str) -> u32 {
    env.class_oid(schema, name).await
}

async fn list_test(ts: TestServer) {
    use ObjectKind::*;
    let env = fixture(&ts, LIST_TAG).await;
    let listed = list_objects(&env.admin, env.version, "s").await.unwrap();

    let mut want = vec![
        row(Table, None, "ser", None),
        row(Table, None, "idt", None),
        row(Table, None, "ch", None),
        row(Table, None, "pt", None),
        row(Table, None, "pt_1", None),
        // Kimlik dizisi (idt) yok; serial dizisi var.
        row(Sequence, None, "ser_id_seq", None),
        row(View, None, "v1", None),
        row(View, None, "v2", None),
        row(MaterializedView, None, "mv", None),
        // PK/UNIQUE indeksleri yok; düz indeks var.
        row(Index, Some("ser"), "ser_n", None),
        // FK'nın iç trigger'ları yok.
        row(Trigger, Some("ch"), "ch_tg", None),
        row(Function, None, "f", Some("integer")),
        row(Function, None, "f", Some("text")),
        row(Function, None, "g", Some("integer")),
        row(Function, None, "tf", Some("")),
        // Tablo satır tipi, dizi tipleri, multirange, range kurucu işlevleri ve aggregate yok.
        row(Type, None, "mood", None),
        row(Type, None, "pair", None),
        row(Type, None, "r", None),
        row(Domain, None, "pos", None),
    ];
    if env.version >= 110000 {
        want.push(row(Procedure, None, "pr", Some("")));
        // Bölümdeki indeks/trigger klonları yok; üsttekiler var.
        want.push(row(Index, Some("pt"), "pt_v", None));
        want.push(row(Trigger, Some("pt"), "pt_tg", None));
    }
    want.sort();
    assert_eq!(rows_of(&listed), want);

    // Sıra: kind, table_name, name (C).
    let sql_kind = |k: ObjectKind| match k {
        Table => "table",
        View => "view",
        MaterializedView => "materialized_view",
        Sequence => "sequence",
        Index => "index",
        Trigger => "trigger",
        Function => "function",
        Procedure => "procedure",
        Type => "type",
        Domain => "domain",
        other => panic!("beklenmeyen tür: {other:?}"),
    };
    // Bayt sırası ("C"); boş tablo adı (NULL) sona.
    let order_key = |x: &Listed| {
        (
            sql_kind(x.kind),
            x.table.is_none(),
            x.table.clone(),
            x.name.clone(),
        )
    };
    let order: Vec<_> = listed.iter().map(order_key).collect();
    let mut sorted = order.clone();
    sorted.sort();
    assert_eq!(
        order, sorted,
        "liste kind, table_name, name sırasında olmalı"
    );
    assert_eq!(order.len(), want.len());

    // Bölüm bayrağı.
    let by_name = |n: &str| {
        listed
            .iter()
            .find(|x| x.name == n && x.kind == Table)
            .unwrap()
    };
    assert!(!by_name("pt").is_partition);
    assert!(by_name("pt_1").is_partition);
    assert!(!by_name("ser").is_partition);

    // Görünüm bağımlılıkları.
    let v1 = oid_of(&env, "s", "v1").await;
    let ser = oid_of(&env, "s", "ser").await;
    let v2 = listed.iter().find(|x| x.name == "v2").unwrap();
    assert!(v2.deps.contains(&v1), "{:?}", v2.deps);
    let mv = listed.iter().find(|x| x.name == "mv").unwrap();
    assert!(mv.deps.contains(&ser), "{:?}", mv.deps);
    assert!(by_name("ser").deps.is_empty());

    // Aynı imza, farklı argüman adı (iki şemada).
    let s2 = list_objects(&env.admin, env.version, "s2").await.unwrap();
    assert_eq!(
        rows_of(&s2),
        vec![row(Function, None, "g", Some("integer"))]
    );

    // Şema denetimi.
    assert_eq!(
        schema_check(&env.admin, "nope").await.unwrap(),
        SchemaCheck::Missing
    );
    assert_eq!(
        schema_check(&env.admin, "pg_catalog").await.unwrap(),
        SchemaCheck::System
    );
    assert_eq!(
        schema_check(&env.admin, "information_schema")
            .await
            .unwrap(),
        SchemaCheck::System
    );
    assert_eq!(
        schema_check(&env.admin, "s").await.unwrap(),
        SchemaCheck::Ok
    );
}

#[tokio::test]
async fn list_objects_fixture() {
    for ts in servers() {
        guarded(list_test(ts.clone()), cleanup_in(&ts, PFX, LIST_TAG)).await;
    }
}

fn side(env: &Env, schema: &str) -> DiffSide {
    DiffSide {
        server_id: env.server,
        database_oid: env.db_oid,
        schema: schema.to_string(),
    }
}

async fn pool_search_path(env: &Env) -> String {
    env.mgr
        .run(env.server, Target::Database(env.db_oid), |c| async move {
            c.client
                .query_one("SHOW search_path", &[])
                .await
                .map(|r| r.get::<_, String>(0))
        })
        .await
        .unwrap()
}

/// Bu veritabanında açık transaction'da bekleyen oturum kalmadı (bağlantı düşüşü asenkron: kısa süre beklenir).
async fn assert_no_idle_in_tx(env: &Env) {
    let sql = "SELECT count(*) FROM pg_stat_activity WHERE datname = current_database() AND state LIKE 'idle in transaction%'";
    for _ in 0..30 {
        let n: i64 = env.admin.query_one(sql, &[]).await.unwrap().get(0);
        if n == 0 {
            return;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    panic!("idle in transaction oturum kaldı");
}

async fn side_test(ts: TestServer) {
    let env = fixture(&ts, SIDE_TAG).await;
    let never = CancelToken::new();

    // Tam yükleme: ilerleme, kimlik bilgileri, niteliksiz görünüm sorgusu, yetki.
    let calls: Mutex<Vec<(u32, u32)>> = Mutex::new(vec![]);
    let report = |done, total| {
        calls.lock().unwrap().push((done, total));
        true
    };
    let snap = load_side(&env.mgr, &side(&env, "s"), true, &report, &never)
        .await
        .unwrap();
    let n = snap.objects.len() as u32;
    assert!(n > 0);
    assert_eq!(calls.lock().unwrap().last().copied(), Some((n, n)));
    assert_eq!(snap.version_num, env.version);
    assert!(!snap.database.is_empty() && !snap.viewer.is_empty() && !snap.version.is_empty());
    assert_eq!(snap.database, common::db_name_in(&ts, PFX, SIDE_TAG).await);
    assert!(
        snap.objects.iter().all(|l| l.def.is_ok()),
        "{:?}",
        snap.objects
            .iter()
            .filter(|l| l.def.is_err())
            .map(|l| &l.def)
            .collect::<Vec<_>>()
    );
    let v2 = snap.objects.iter().find(|l| l.listed.name == "v2").unwrap();
    let Ok(ObjectDef::View(v)) = &v2.def else {
        panic!("view bekleniyordu")
    };
    assert!(
        v.query.contains("FROM v1") && !v.query.contains("s.v1"),
        "{}",
        v.query
    );
    let ser = snap
        .objects
        .iter()
        .find(|l| l.listed.name == "ser" && l.listed.kind == ObjectKind::Table)
        .unwrap();
    assert!(matches!(ser.acl, Some(Ok(_))), "{:?}", ser.acl);
    let idx = snap
        .objects
        .iter()
        .find(|l| l.listed.kind == ObjectKind::Index)
        .unwrap();
    assert!(idx.acl.is_none());
    // Yetki kapalıyken hiç ACL yok.
    let snap = load_side(&env.mgr, &side(&env, "s"), false, &|_, _| true, &never)
        .await
        .unwrap();
    assert!(snap.objects.iter().all(|l| l.acl.is_none()));
    assert_no_idle_in_tx(&env).await;

    // Şema denetimi hataları.
    for (schema, want) in [
        ("nope", SchemaCheck::Missing),
        ("pg_catalog", SchemaCheck::System),
    ] {
        match load_side(&env.mgr, &side(&env, schema), false, &|_, _| true, &never).await {
            Err(SnapshotError::Schema(got)) => assert_eq!(got, want),
            other => panic!(
                "şema hatası bekleniyordu: {:?}",
                other.map(|s| s.objects.len())
            ),
        }
    }
    assert_no_idle_in_tx(&env).await;

    // Başlamadan iptal.
    let t = Instant::now();
    let c = CancelToken::new();
    c.cancel();
    assert!(matches!(
        load_side(&env.mgr, &side(&env, "s"), false, &|_, _| true, &c).await,
        Err(SnapshotError::Cancelled)
    ));
    assert!(t.elapsed() < Duration::from_secs(2));

    // report ilk çağrıda false.
    let r = load_side(&env.mgr, &side(&env, "s"), false, &|_, _| false, &never).await;
    assert!(
        matches!(r, Err(SnapshotError::Cancelled)),
        "{:?}",
        r.map(|s| s.objects.len())
    );
    assert_no_idle_in_tx(&env).await;

    // Yükleme sürerken havuz sorgusu çalışır; havuz search_path'i değişmez.
    let (mgr, sd) = (env.mgr.clone(), side(&env, "s"));
    let job = tokio::spawn(async move {
        load_side(&mgr, &sd, true, &|_, _| true, &CancelToken::new())
            .await
            .map(|s| s.objects.len())
    });
    let one: i32 = env
        .mgr
        .run(env.server, Target::Database(env.db_oid), |c| async move {
            c.client.query_one("SELECT 1", &[]).await.map(|r| r.get(0))
        })
        .await
        .unwrap();
    assert_eq!(one, 1);
    assert!(job.await.unwrap().is_ok());
    assert_eq!(pool_search_path(&env).await, "pg_catalog");
    assert_no_idle_in_tx(&env).await;

    // Boş şema: tam bir (0, 0) bildirimi.
    env.sql("CREATE SCHEMA empty_s").await;
    let calls: Mutex<Vec<(u32, u32)>> = Mutex::new(vec![]);
    let report = |done, total| {
        calls.lock().unwrap().push((done, total));
        true
    };
    let snap = load_side(&env.mgr, &side(&env, "empty_s"), false, &report, &never)
        .await
        .unwrap();
    assert!(snap.objects.is_empty());
    assert_eq!(*calls.lock().unwrap(), vec![(0, 0)]);

    item_error_path(&env).await;
}

/// Nesne hatası (sistem nesnesi) yalnız o nesneyi `Err` yapar; iş sürer, transaction bozulmaz.
async fn item_error_path(env: &Env) {
    let conn = env
        .mgr
        .open_dedicated(
            env.server,
            Target::Database(env.db_oid),
            SessionProfile::Catalog,
        )
        .await
        .unwrap();
    let c = &conn.client;
    c.batch_execute("BEGIN ISOLATION LEVEL REPEATABLE READ READ ONLY")
        .await
        .unwrap();
    c.batch_execute("SELECT pg_catalog.set_config('search_path', 'pg_catalog, s', true)")
        .await
        .unwrap();
    let listed = list_objects(c, env.version, "s").await.unwrap();
    let pick = |n: &str| {
        listed
            .iter()
            .find(|l| l.name == n && l.kind == ObjectKind::Table)
            .unwrap()
            .clone()
    };
    let sys = Listed {
        kind: ObjectKind::Table,
        oid: env.class_oid("pg_catalog", "pg_class").await,
        name: "pg_class".into(),
        table: None,
        signature: None,
        is_partition: false,
        deps: vec![],
    };
    let objs = vec![sys, pick("ser"), pick("ch")];
    let loaded = load_objects(
        c,
        env.version,
        objs,
        false,
        &|_, _| true,
        &CancelToken::new(),
    )
    .await
    .unwrap();
    assert_eq!(loaded.len(), 3);
    assert!(loaded[0].def.is_err(), "sistem nesnesi hata olmalı");
    assert!(!loaded[0].def.as_ref().unwrap_err().is_empty());
    assert!(loaded[1].def.is_ok() && loaded[2].def.is_ok());
    // Transaction hâlâ sağlam.
    c.query_one("SELECT 1", &[]).await.unwrap();
    c.batch_execute("ROLLBACK").await.unwrap();
}

#[tokio::test]
async fn load_side_behaviour() {
    for ts in servers() {
        guarded(side_test(ts.clone()), cleanup_in(&ts, PFX, SIDE_TAG)).await;
    }
}
