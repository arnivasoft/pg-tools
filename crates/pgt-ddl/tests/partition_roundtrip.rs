//! Partition üreticisi → gerçek sunucu → yükleyici gidiş-dönüş testleri (Docker; `PGT_TEST_URLS` yoksa atlanır).
//!
//! Her test kendi geçici veritabanını (`pgt_ddl_pt_<ana sürüm>_<etiket>`) ve rolünü kurar; başarısız doğrulamadan sonra
//! bile temizlenir (`guarded`). Normal planlar uygulayıcı gibi tek transaction'da, `stmt_nt` ifadeleri tek tek
//! (transaction dışı) çalıştırılır.
//!
//! **Sabit nokta:** yüklenen her tanımda `alter(yüklü, yüklü)` boş plandır; test girdileri PG'nin yazdığı
//! (kanonik) biçimdedir (`'2024-01-01'`, `'a,b'`, `1`…), böylece `create → load` ve `alter → load` girdiyle eşittir.
//! PG'nin ürettiği SQLSTATE'ler (ATTACH uyumsuz satır, çakışan sınır, DEFAULT bölümlü CONCURRENTLY) sabitlenir.

mod common;

use common::{Env, db_message, guarded, servers, setup_in};
use pgt_core::connection::ConnectionManager;
use pgt_core::testing::TestServer;
use pgt_ddl::build::{self, DropOptions};
use pgt_ddl::*;
use std::sync::Arc;

const PFX: &str = "pgt_ddl_pt";

async fn cleanup(ts: &TestServer, tag: &str) {
    common::cleanup_in(ts, PFX, tag).await;
}

macro_rules! docker_test {
    ($name:ident, $body:ident, $tag:literal) => {
        #[tokio::test]
        async fn $name() {
            for ts in servers() {
                guarded($body(ts.clone()), cleanup(&ts, $tag)).await;
            }
        }
    };
}

fn sqls(p: &Plan) -> Vec<&str> {
    p.statements.iter().map(|x| x.sql.as_str()).collect()
}

fn bv(sql: &str) -> BoundValue {
    BoundValue::Value { sql: sql.into() }
}

fn range(from: Vec<BoundValue>, to: Vec<BoundValue>) -> PartitionBound {
    PartitionBound::Range { from, to }
}

fn range1(a: &str, b: &str) -> PartitionBound {
    range(vec![bv(a)], vec![bv(b)])
}

fn list(v: &[&str]) -> PartitionBound {
    PartitionBound::List { values: v.iter().map(|x| x.to_string()).collect() }
}

/// Bölüm tanımı (`public` şeması, `create` modu); `strategy`/`key_count` üst tablodan gelir.
fn pdef(parent: &str, name: &str, strategy: PartitionStrategy, key_count: u32, bound: PartitionBound) -> PartitionDef {
    PartitionDef {
        parent_schema: "public".into(),
        parent_table: parent.into(),
        strategy,
        key_count,
        mode: PartitionMode::Create,
        schema: "public".into(),
        name: name.into(),
        bound,
        partition_by: None,
        tablespace: None,
        owner: None,
        comment: None,
        detach_pending: false,
        editable: true,
        unsupported: vec![],
    }
}

fn rdef(parent: &str, name: &str, bound: PartitionBound) -> PartitionDef {
    pdef(parent, name, PartitionStrategy::Range, 1, bound)
}

fn pd(d: &PartitionDef) -> ObjectDef {
    ObjectDef::Partition(d.clone())
}

async fn pload(env: &Env, schema: &str, name: &str) -> PartitionDef {
    let oid = env.class_oid(schema, name).await;
    match load(&env.mgr, env.server, env.db_oid, ObjectRef { kind: ObjectKind::Partition, oid }).await {
        Ok(ObjectDef::Partition(d)) => {
            // `load_in` (verilen istemcide) aynı tanımı verir.
            match load_in(&env.admin, env.version, ObjectRef { kind: ObjectKind::Partition, oid }).await {
                Ok(ObjectDef::Partition(d2)) => assert_eq!(d, d2, "load ≠ load_in ({name})"),
                other => panic!("load_in başarısız: {other:?}"),
            }
            d
        }
        other => panic!("bölüm yüklenemedi: {other:?}"),
    }
}

/// Yükleyicinin yazmadığı alanları (sahip/yorum/tablespace/partition_by/mode) yükleyicinin yazdığı değerlere çeker.
fn expected(def: &PartitionDef) -> PartitionDef {
    PartitionDef {
        mode: PartitionMode::Create,
        partition_by: None,
        tablespace: None,
        owner: None,
        comment: None,
        detach_pending: false,
        editable: true,
        unsupported: vec![],
        ..def.clone()
    }
}

fn assert_noop(env: &Env, d: &PartitionDef) {
    let pl = build::alter(&pd(d), &pd(d), &env.ctx(false)).unwrap_or_else(|e| panic!("noop reddedildi: {e:?}\n{d:?}"));
    assert!(pl.statements.is_empty() && pl.warnings.is_empty(), "{}: boş plan bekleniyordu: {:?}", d.name, sqls(&pl));
}

/// create → tek transaction → yükle → beklenen tanıma eşit + noop.
async fn create_and_check(env: &Env, def: &PartitionDef) -> PartitionDef {
    let plan = build::create(&pd(def), &env.ctx(false)).unwrap_or_else(|e| panic!("create reddedildi: {} {e:?}", def.name));
    env.exec_tx(&plan).await;
    let loaded = pload(env, &def.schema, &def.name).await;
    assert_eq!(expected(def), loaded, "create → load ({}); plan: {:?}", def.name, sqls(&plan));
    assert_noop(env, &loaded);
    loaded
}

/// Planı BEGIN…COMMIT içinde çalıştırır; ilk hatada ROLLBACK ve PG'nin SQLSTATE'i.
async fn tx_code(env: &Env, plan: &Plan) -> Option<String> {
    env.admin.batch_execute("BEGIN").await.unwrap();
    for s in &plan.statements {
        if let Err(e) = env.admin.batch_execute(&s.sql).await {
            env.admin.batch_execute("ROLLBACK").await.unwrap();
            return Some(e.code().map(|c| c.code().to_string()).unwrap_or_else(|| db_message(&e)));
        }
    }
    env.admin.batch_execute("COMMIT").await.unwrap();
    None
}

/// Tek ifadeyi (transaction dışı) çalıştırır; hata → SQLSTATE.
async fn stmt_code(env: &Env, sql: &str) -> Option<String> {
    env.admin.batch_execute(sql).await.err().map(|e| e.code().map(|c| c.code().to_string()).unwrap_or_else(|| db_message(&e)))
}

async fn is_partition(env: &Env, schema: &str, name: &str) -> bool {
    let oid = env.class_oid(schema, name).await;
    env.admin.query_one("SELECT relispartition FROM pg_class WHERE oid = $1", &[&oid]).await.unwrap().get(0)
}

async fn count(env: &Env, table: &str) -> i64 {
    env.admin.query_one(&format!("SELECT count(*) FROM {table}"), &[]).await.unwrap().get(0)
}

// ---- create → load ----

docker_test!(partition_create_then_load_equals_def, create_body, "create");

async fn create_body(ts: TestServer) {
    let env = setup_in(&ts, PFX, "create").await;
    // Oturum saat dilimi sabit: veritabanı varsayılanı UTC; yeni bir yönetici (yeni oturumlar) kullanılır.
    let dbn = common::db_name_in(&ts, PFX, "create").await;
    env.sql(&format!("ALTER DATABASE {dbn} SET timezone = 'UTC'")).await;
    let mgr = Arc::new(ConnectionManager::new());
    mgr.connect(ts.config.clone(), ts.password.clone()).await.unwrap();
    let env = Env { mgr, ..env };
    let hash = env.version >= 110000;

    // date, tek anahtar; sahip/yorum/tablespace yazılır ama yüklenmez.
    env.sql("CREATE TABLE r1 (d date, v text) PARTITION BY RANGE (d)").await;
    let mut a = rdef("r1", "r1a", range1("'2024-01-01'", "'2024-02-01'"));
    a.owner = Some(env.role.clone());
    a.comment = Some("açıklama".into());
    let la = create_and_check(&env, &a).await;
    assert!(la.owner.is_none() && la.comment.is_none() && la.tablespace.is_none() && la.partition_by.is_none());
    let owner: String = env
        .admin
        .query_one("SELECT pg_get_userbyid(relowner)::text FROM pg_class WHERE oid = $1", &[&env.class_oid("public", "r1a").await])
        .await
        .unwrap()
        .get(0);
    assert_eq!(owner, env.role, "OWNER TO uygulanmalı");
    let cm: Option<String> = env.admin.query_one("SELECT obj_description($1::oid, 'pg_class')", &[&env.class_oid("public", "r1a").await]).await.unwrap().get(0);
    assert_eq!(cm.as_deref(), Some("açıklama"));
    create_and_check(&env, &rdef("r1", "r1b", range1("'2024-02-01'", "'2024-03-01'"))).await;

    // timestamptz (oturum saat dilimi UTC).
    env.sql("CREATE TABLE r2 (ts timestamptz) PARTITION BY RANGE (ts)").await;
    create_and_check(&env, &rdef("r2", "r2a", range1("'2024-01-01 00:00:00+00'", "'2024-02-01 00:00:00+00'"))).await;

    // text: virgül, tırnak, parantez içeren değerler.
    env.sql("CREATE TABLE r3 (t text) PARTITION BY RANGE (t)").await;
    create_and_check(&env, &rdef("r3", "r3a", range1("'a,b'", "'it''s'"))).await;
    create_and_check(&env, &rdef("r3", "r3b", range1("'it''s'", "'(x)'"))).await;

    // Çok kolonlu range: MINVALUE / MAXVALUE.
    env.sql("CREATE TABLE r4 (a int, b int) PARTITION BY RANGE (a, b)").await;
    let r4 = |n: &str, f: Vec<BoundValue>, t: Vec<BoundValue>| pdef("r4", n, PartitionStrategy::Range, 2, range(f, t));
    create_and_check(&env, &r4("r4a", vec![BoundValue::Minvalue, BoundValue::Minvalue], vec![bv("10"), BoundValue::Maxvalue])).await;
    create_and_check(&env, &r4("r4b", vec![bv("10"), BoundValue::Maxvalue], vec![BoundValue::Maxvalue, BoundValue::Maxvalue])).await;
    env.sql("CREATE TABLE r5 (a int, b int) PARTITION BY RANGE (a, b)").await;
    create_and_check(&env, &pdef("r5", "r5a", PartitionStrategy::Range, 2, range(vec![bv("1"), bv("2")], vec![bv("3"), bv("4")]))).await;

    // list: NULL dahil, virgüllü/tırnaklı/parantezli metin; sayısal list.
    env.sql("CREATE TABLE l1 (k text) PARTITION BY LIST (k)").await;
    let l = |n: &str, v: &[&str]| pdef("l1", n, PartitionStrategy::List, 1, list(v));
    create_and_check(&env, &l("l1a", &["'a'", "'b,c'", "'(x)'", "'it''s'", "NULL"])).await;
    env.sql("CREATE TABLE l2 (k int) PARTITION BY LIST (k)").await;
    create_and_check(&env, &pdef("l2", "l2a", PartitionStrategy::List, 1, list(&["1", "2", "3"]))).await;

    // hash ve default (PG 11+).
    if hash {
        env.sql("CREATE TABLE h1 (id int) PARTITION BY HASH (id)").await;
        let h = |n: &str, r: u32| pdef("h1", n, PartitionStrategy::Hash, 1, PartitionBound::Hash { modulus: 4, remainder: r });
        create_and_check(&env, &h("h1a", 0)).await;
        create_and_check(&env, &h("h1b", 1)).await;
        create_and_check(&env, &pdef("r1", "r1d", PartitionStrategy::Range, 1, PartitionBound::Default)).await;
        create_and_check(&env, &pdef("l2", "l2d", PartitionStrategy::List, 1, PartitionBound::Default)).await;
    }

    // Alt bölümlü bölüm (tüm sürümler): create'te partition_by yazılır, yüklenince None; torun altında kurulur.
    env.sql("CREATE TABLE s1 (id int, n int) PARTITION BY RANGE (id)").await;
    let mut mid = rdef("s1", "s1a", range1("0", "100"));
    mid.partition_by = Some(PartitionByDef { strategy: PartitionStrategy::Range, keys: vec![PartitionKey::Column { name: "n".into() }] });
    create_and_check(&env, &mid).await;
    let relkind: String =
        env.admin.query_one("SELECT relkind::text FROM pg_class WHERE oid = $1", &[&env.class_oid("public", "s1a").await]).await.unwrap().get(0);
    assert_eq!(relkind, "p", "alt bölümlü bölüm 'p' olmalı");
    create_and_check(&env, &rdef("s1a", "s1a1", range1("0", "50"))).await;
}

// ---- attach ----

docker_test!(partition_attach_and_alter_bound, attach_body, "attach");

async fn attach_body(ts: TestServer) {
    let env = setup_in(&ts, PFX, "attach").await;
    env.sql("CREATE TABLE a1 (id int) PARTITION BY RANGE (id); CREATE TABLE c_ok (id int); INSERT INTO c_ok VALUES (5)").await;

    // Uyumlu satırla bağlanır; yüklenen tanım create modunda gelir (mode attach → create).
    let mut ok = rdef("a1", "c_ok", range1("0", "10"));
    ok.mode = PartitionMode::Attach;
    let plan = build::create(&pd(&ok), &env.ctx(false)).unwrap();
    assert_eq!(plan.statements.len(), 1);
    assert_eq!(plan.statements[0].risk, Risk::Destructive);
    env.exec_tx(&plan).await;
    let loaded = pload(&env, "public", "c_ok").await;
    assert_eq!(expected(&ok), loaded);
    assert!(is_partition(&env, "public", "c_ok").await);
    assert_noop(&env, &loaded);

    // Uyumsuz satır: ATTACH başarısız, tablo bağlanmaz.
    env.sql("CREATE TABLE c_bad (id int); INSERT INTO c_bad VALUES (50)").await;
    let mut bad = rdef("a1", "c_bad", range1("10", "20"));
    bad.mode = PartitionMode::Attach;
    let code = tx_code(&env, &build::create(&pd(&bad), &env.ctx(false)).unwrap()).await;
    assert_eq!(code.as_deref(), Some("23514"), "ATTACH uyumsuz satır SQLSTATE");
    assert!(!is_partition(&env, "public", "c_bad").await, "başarısız ATTACH tabloyu bağlamamalı");

    // Çakışan sınırla ATTACH (c_ok 0..10).
    env.sql("CREATE TABLE c_ovl (id int)").await;
    let mut ovl = rdef("a1", "c_ovl", range1("5", "15"));
    ovl.mode = PartitionMode::Attach;
    let code = tx_code(&env, &build::create(&pd(&ovl), &env.ctx(false)).unwrap()).await;
    assert_eq!(code.as_deref(), Some("42P17"), "ATTACH çakışan sınır SQLSTATE");
    assert!(!is_partition(&env, "public", "c_ovl").await);

    // Sınır değiştirme: 0..10 → 0..20 (DETACH + ATTACH, aynı transaction).
    let wider = PartitionDef { bound: range1("0", "20"), ..loaded.clone() };
    let plan = build::alter(&pd(&loaded), &pd(&wider), &env.ctx(false)).unwrap();
    assert_eq!(plan.statements.len(), 2, "{:?}", sqls(&plan));
    assert!(plan.statements.iter().all(|s| s.risk == Risk::Destructive));
    env.exec_tx(&plan).await;
    let reloaded = pload(&env, "public", "c_ok").await;
    assert_eq!(reloaded.bound, wider.bound);
    assert!(is_partition(&env, "public", "c_ok").await);
    assert_noop(&env, &reloaded);
    assert_eq!(count(&env, "c_ok").await, 1, "satırlar korunmalı");
    // Aynı sınır → boş plan.
    assert!(build::alter(&pd(&reloaded), &pd(&reloaded), &env.ctx(false)).unwrap().statements.is_empty());

    // Çakışan yeni sınır: hata, eski sınır korunur (aynı transaction → DETACH geri alınır).
    env.sql("CREATE TABLE c2 (id int)").await;
    let mut c2 = rdef("a1", "c2", range1("30", "40"));
    c2.mode = PartitionMode::Attach;
    env.exec_tx(&build::create(&pd(&c2), &env.ctx(false)).unwrap()).await;
    let c2l = pload(&env, "public", "c2").await;
    let overlapping = PartitionDef { bound: range1("10", "25"), ..c2l.clone() };
    let plan = build::alter(&pd(&c2l), &pd(&overlapping), &env.ctx(false)).unwrap();
    assert!(env.try_exec_tx(&plan).await.is_err());
    let code = tx_code(&env, &plan).await;
    assert_eq!(code.as_deref(), Some("42P17"), "yeniden bağlamada çakışan sınır SQLSTATE");
    let after = pload(&env, "public", "c2").await;
    assert_eq!(after.bound, c2l.bound, "başarısız sınır değişiminde eski sınır korunmalı");
    assert!(is_partition(&env, "public", "c2").await);

    // Mevcut satırı dışarıda bırakan sınır: hata, eski sınır korunur.
    env.sql("INSERT INTO c2 VALUES (35)").await;
    let shrunk = PartitionDef { bound: range1("30", "33"), ..after.clone() };
    let plan = build::alter(&pd(&after), &pd(&shrunk), &env.ctx(false)).unwrap();
    let code = tx_code(&env, &plan).await;
    assert_eq!(code.as_deref(), Some("23514"), "yeniden bağlamada uyumsuz satır SQLSTATE");
    assert_eq!(pload(&env, "public", "c2").await.bound, after.bound);
}

// ---- detach ----

docker_test!(partition_detach_forms, detach_body, "detach");

async fn detach_body(ts: TestServer) {
    let env = setup_in(&ts, PFX, "detach").await;
    let ctx = env.ctx(false);
    env.sql("CREATE TABLE d1 (id int) PARTITION BY RANGE (id); CREATE TABLE d1a PARTITION OF d1 FOR VALUES FROM (0) TO (10); INSERT INTO d1 VALUES (1), (2)").await;
    let d1a = pload(&env, "public", "d1a").await;

    // cascade / force reddi.
    let e = build::drop_with(&pd(&d1a), DropOptions { cascade: true, force: false, concurrently: false }, &ctx).unwrap_err();
    assert_eq!(e[0].path, "cascade");

    // Normal ayırma: tablo durur, satırlar korunur, artık bölüm değil.
    let plan = build::drop_with(&pd(&d1a), DropOptions::default(), &ctx).unwrap();
    assert_eq!(plan.statements.len(), 1);
    assert!(plan.statements[0].transactional && plan.statements[0].risk == Risk::Destructive, "{:?}", sqls(&plan));
    env.exec_tx(&plan).await;
    assert!(!is_partition(&env, "public", "d1a").await);
    assert_eq!(count(&env, "d1a").await, 2, "ayırma satırları korumalı");
    assert_eq!(count(&env, "d1").await, 0);
    let oid = env.class_oid("public", "d1a").await;
    let r = load(&env.mgr, env.server, env.db_oid, ObjectRef { kind: ObjectKind::Partition, oid }).await;
    assert!(matches!(r, Err(pgt_core::Error::NotFound(_))), "bölüm olmayan tablo NotFound: {r:?}");
    // Bölümlü üst tablo da bölüm değildir.
    let poid = env.class_oid("public", "d1").await;
    let r = load(&env.mgr, env.server, env.db_oid, ObjectRef { kind: ObjectKind::Partition, oid: poid }).await;
    assert!(matches!(r, Err(pgt_core::Error::NotFound(_))), "{r:?}");

    let conc = DropOptions { cascade: false, force: false, concurrently: true };
    if env.version < 140000 {
        env.sql("ALTER TABLE d1 ATTACH PARTITION d1a FOR VALUES FROM (0) TO (10)").await;
        let d = pload(&env, "public", "d1a").await;
        let e = build::drop_with(&pd(&d), conc, &ctx).unwrap_err();
        assert_eq!(e[0].path, "concurrently");
        return;
    }

    // PG 14+: CONCURRENTLY (DEFAULT bölüm yok) başarılı; `stmt_nt`.
    env.sql("ALTER TABLE d1 ATTACH PARTITION d1a FOR VALUES FROM (0) TO (10)").await;
    let d = pload(&env, "public", "d1a").await;
    assert!(!d.detach_pending, "Docker'da yalnız detach_pending = false doğrulanır");
    let plan = build::drop_with(&pd(&d), conc, &ctx).unwrap();
    assert!(!plan.statements[0].transactional && plan.statements[0].sql.ends_with("CONCURRENTLY"), "{:?}", sqls(&plan));
    env.exec(&plan).await;
    assert!(!is_partition(&env, "public", "d1a").await);
    assert_eq!(count(&env, "d1a").await, 2);

    // DEFAULT bölüm varken CONCURRENTLY → PG hatası; bölüm bağlı kalır.
    env.sql("ALTER TABLE d1 ATTACH PARTITION d1a FOR VALUES FROM (0) TO (10); CREATE TABLE d1d PARTITION OF d1 DEFAULT").await;
    let d = pload(&env, "public", "d1a").await;
    let plan = build::drop_with(&pd(&d), conc, &ctx).unwrap();
    let code = stmt_code(&env, &plan.statements[0].sql).await;
    assert_eq!(code.as_deref(), Some("55000"), "DETACH CONCURRENTLY + DEFAULT bölüm SQLSTATE");
    assert!(is_partition(&env, "public", "d1a").await);

    // Yarım kalmış ayırma: sunucuda doğal üretmek zor → katalog bayrağı elle kurulur (süper kullanıcı, atılabilir
    // veritabanı); yükleyici `detach_pending`'i okur, ayırma yalnız FINALIZE yazar ve gerçek sunucuda tamamlanır.
    env.sql("UPDATE pg_inherits SET inhdetachpending = true WHERE inhrelid = 'd1a'::regclass").await;
    let p = pload(&env, "public", "d1a").await;
    assert!(p.detach_pending, "inhdetachpending yüklenmeli");
    let plan = build::drop_with(&pd(&p), conc, &ctx).unwrap();
    assert_eq!(plan.statements.len(), 1);
    assert!(plan.statements[0].sql.ends_with("FINALIZE") && !plan.statements[0].transactional, "{:?}", sqls(&plan));
    let e = build::alter(&pd(&p), &pd(&PartitionDef { bound: range1("0", "11"), ..p.clone() }), &ctx).unwrap_err();
    assert_eq!(e[0].path, "object");
    env.exec(&plan).await;
    assert!(!is_partition(&env, "public", "d1a").await);
}

// ---- attachable_tables ----

docker_test!(partition_attachable_tables_lookup, lookup_body, "lookup");

async fn lookup_body(ts: TestServer) {
    let env = setup_in(&ts, PFX, "lookup").await;
    env.sql(
        "CREATE SCHEMA other; \
         CREATE TABLE top (id int, n int) PARTITION BY RANGE (id); \
         CREATE TABLE mid PARTITION OF top FOR VALUES FROM (0) TO (100) PARTITION BY RANGE (n); \
         CREATE TABLE leaf PARTITION OF mid FOR VALUES FROM (0) TO (10); \
         CREATE TABLE plain1 (id int); \
         CREATE TABLE empty_parted (id int) PARTITION BY LIST (id); \
         CREATE TABLE inh_parent (id int); CREATE TABLE inh_child () INHERITS (inh_parent); \
         CREATE TABLE other.o1 (id int); \
         CREATE TABLE sib (id int) PARTITION BY RANGE (id)",
    )
    .await;
    let list = |oid: u32| {
        let env = &env;
        async move { lookup(&env.mgr, env.server, env.db_oid, LookupRequest::AttachableTables { parent_oid: oid }).await.unwrap() }
    };
    let has = |items: &[LookupItem], schema: &str, name: &str| items.iter().any(|i| i.value == name && i.detail.as_deref() == Some(schema));

    // Alt bölümlü `mid` için: kendisi, atası (top), bölümü (leaf), kalıtım üyeleri listede yok.
    let mid = env.class_oid("public", "mid").await;
    let items = list(mid).await;
    for (s, n) in [("public", "plain1"), ("public", "empty_parted"), ("other", "o1"), ("public", "sib")] {
        assert!(has(&items, s, n), "{s}.{n} listelenmeli: {items:?}");
    }
    for n in ["top", "mid", "leaf", "inh_parent", "inh_child"] {
        assert!(!items.iter().any(|i| i.value == n), "{n} listelenmemeli: {items:?}");
    }
    assert!(items.iter().all(|i| {
        let d = i.detail.as_deref().unwrap_or("");
        !d.starts_with("pg_") && d != "information_schema"
    }), "sistem şeması tabloları listelenmemeli");
    assert!(!items.iter().any(|i| i.detail.as_deref() == Some("pg_catalog")));
    // Sıra: şema, sonra ad (COLLATE "C").
    let keys: Vec<_> = items.iter().map(|i| (i.detail.clone().unwrap_or_default(), i.value.clone())).collect();
    let mut sorted = keys.clone();
    sorted.sort();
    assert_eq!(keys, sorted);

    // Üst tablo kendisi de listelenmez (üst düzey bölümlü tablo için de).
    let top = env.class_oid("public", "top").await;
    let items = list(top).await;
    assert!(!items.iter().any(|i| i.value == "top") && has(&items, "public", "plain1"), "{items:?}");

    // Bölümü olmayan bölümlü tablo için kendisi listelenmez.
    let ep = env.class_oid("public", "empty_parted").await;
    let items = list(ep).await;
    assert!(!items.iter().any(|i| i.value == "empty_parted") && has(&items, "public", "sib"), "{items:?}");

    // Hedef çözümü: veritabanı kapsamlı.
    assert!(lookup(&env.mgr, env.server, None, LookupRequest::AttachableTables { parent_oid: mid }).await.is_err());
}

// ---- NotFound ----

docker_test!(partition_load_rejects_non_partitions, notfound_body, "notfound");

async fn notfound_body(ts: TestServer) {
    let env = setup_in(&ts, PFX, "notfound").await;
    env.sql("CREATE TABLE plain1 (id int); CREATE TABLE np (id int) PARTITION BY RANGE (id)").await;
    for t in ["plain1", "np"] {
        let oid = env.class_oid("public", t).await;
        let r = load(&env.mgr, env.server, env.db_oid, ObjectRef { kind: ObjectKind::Partition, oid }).await;
        assert!(matches!(r, Err(pgt_core::Error::NotFound(_))), "{t}: {r:?}");
        let r = load_in(&env.admin, env.version, ObjectRef { kind: ObjectKind::Partition, oid }).await;
        assert!(matches!(r, Err(pgt_core::Error::NotFound(_))), "{t}: {r:?}");
    }
    let r = load(&env.mgr, env.server, env.db_oid, ObjectRef { kind: ObjectKind::Partition, oid: 4_000_000_000 }).await;
    assert!(matches!(r, Err(pgt_core::Error::NotFound(_))), "{r:?}");
}
