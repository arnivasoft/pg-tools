//! Nesne yetkileri yükleyicisi (`load_acl`) ↔ fark üreticisi (`build::acl::privileges`) gidiş-dönüş testleri (Docker;
//! `PGT_TEST_URLS` yoksa atlanır).
//!
//! Her test kendi geçici veritabanını (`pgt_ddl_acl_<ana sürüm>_<etiket>`), şemasını (`pgt_ddl_rt_<etiket>…`) ve küme
//! rollerini (`pgt_acl_rt_<etiket>_<n>`) kurar; başarısız doğrulamadan sonra bile önce veritabanı, sonra roller silinir
//! (`cleanup_acl`). Yönetici `Privileges` isteğini henüz uygulamadığından (geçici kapı) planın ifadeleri uygulayıcı gibi
//! tek transaction'da düz istemciyle çalıştırılır (`Env::exec_tx`). Sürüme bağlı kısımlar (procedure PG 11+, MAINTAIN
//! PG 17+, multirange PG 14+) atlandığında yazdırılır.
//!
//! Varsayılan yetkiler (`load_default_acl` ↔ `default_privileges`) aynı kalıpla sınanır (`apply_def`); `FOR ROLE`
//! yetkisizliği `SET ROLE` ile süper kullanıcı olmayan role geçilerek denenir; PG 18 large object notu PG 18+.
//!
//! **Sabit nokta:** yüklenen her tanımda `privileges(yüklü, yüklü)` boş plandır; düzenlenmiş kopya uygulanıp yeniden
//! yüklendiğinde `grants` istenenle (kanonik sırada, boş satırlar düşmüş) aynıdır.

mod common;

use common::{Env, acl_role, cleanup_acl, guarded, servers, setup_in};
use pgt_core::Error;
use pgt_core::testing::TestServer;
use pgt_ddl::build::acl::privileges;
use pgt_ddl::build::default_acl::default_privileges;
use pgt_ddl::load_acl::{LARGE_OBJECTS_REASON, SYSTEM_SCHEMA_REASON};
use pgt_ddl::*;

const PFX: &str = "pgt_ddl_acl";

macro_rules! docker_test {
    ($name:ident, $body:ident, $tag:literal) => {
        #[tokio::test]
        async fn $name() {
            for ts in servers() {
                guarded($body(ts.clone(), $tag), cleanup_acl(&ts, PFX, $tag)).await;
            }
        }
    };
}

// ---- kurulum ----

/// Önceki çalıştırmanın artıklarını (veritabanı, sonra roller) siler; veritabanını kurar; `roles` adlı test rollerini
/// (`acl_role(tag, n)`) oluşturur.
async fn setup(ts: &TestServer, tag: &str, roles: &[&str]) -> Env {
    cleanup_acl(ts, PFX, tag).await;
    let env = setup_in(ts, PFX, tag).await;
    for n in roles {
        env.sql(&format!("CREATE ROLE {}", quote_ident(&acl_role(tag, n)))).await;
    }
    env
}

fn role(tag: &str, n: &str) -> String {
    acl_role(tag, n)
}

// ---- yükle / kur / karşılaştır ----

async fn try_load(env: &Env, kind: ObjectKind, oid: u32) -> pgt_core::Result<AclDef> {
    load_acl(&env.mgr, env.server, env.db_oid, ObjectRef { kind, oid }).await
}

async fn ld(env: &Env, kind: ObjectKind, oid: u32) -> AclDef {
    try_load(env, kind, oid).await.unwrap_or_else(|e| panic!("{kind:?} {oid} yetkileri yüklenemedi: {e:?}"))
}

fn pg(privilege: Privilege, grant_option: bool) -> PrivilegeGrant {
    PrivilegeGrant { privilege, grant_option }
}

/// GRANT OPTION'sız satır.
fn row(who: Option<&str>, ps: &[Privilege]) -> AclGrant {
    AclGrant { grantee: who.map(Into::into), privileges: ps.iter().map(|p| pg(*p, false)).collect() }
}

fn row_go(who: Option<&str>, ps: &[(Privilege, bool)]) -> AclGrant {
    AclGrant { grantee: who.map(Into::into), privileges: ps.iter().map(|(p, g)| pg(*p, *g)).collect() }
}

/// Sahibin varsayılan satırı: türün (sürüme göre) tüm yetkileri, GRANT OPTION'sız (`acldefault`).
fn owner_all(env: &Env, kind: ObjectKind) -> AclGrant {
    row(Some(&env.me), privileges_for(kind, env.version))
}

fn canonical(kind: ObjectKind, mut v: Vec<AclGrant>) -> Vec<AclGrant> {
    v.retain(|g| !g.privileges.is_empty());
    sort_grants(&mut v, privileges_for(kind, 170000));
    v
}

fn sqls(p: &Plan) -> Vec<&str> {
    p.statements.iter().map(|s| s.sql.as_str()).collect()
}

fn paths(e: &[FieldError]) -> Vec<&str> {
    e.iter().map(|e| e.path.as_str()).collect()
}

/// Sunucu iletisindeki SQLSTATE (`db_message` biçimi: "ileti (KOD)").
fn sqlstate(msg: &str) -> &str {
    msg.split_once('\n').map_or(msg, |(first, _)| first).rsplit_once('(').map_or("", |(_, c)| c.trim_end_matches(')'))
}

/// `privileges(d, d)` boş plan.
fn assert_fixed(env: &Env, d: &AclDef) {
    let p = privileges(&env.ctx(false), d, d, false).unwrap_or_else(|e| panic!("sabit nokta reddedildi: {e:?}"));
    assert!(p.statements.is_empty(), "sabit nokta boş değil: {:?}", sqls(&p));
}

/// Yüklü tanımın ortak alanları ve varsayılan (NULL ACL) durumu.
fn assert_default(env: &Env, d: &AclDef, kind: ObjectKind, want: Vec<AclGrant>) {
    assert_eq!(d.kind, kind);
    assert_eq!((d.owner.as_str(), d.viewer.as_str()), (env.me.as_str(), env.me.as_str()));
    assert!(!d.explicit, "varsayılan ACL NULL olmalı: {d:?}");
    assert_eq!(d.grants, canonical(kind, want), "{kind:?}");
    assert!(d.foreign.is_empty() && d.columns.is_empty(), "{d:?}");
    assert!(d.editable && d.unsupported.is_empty(), "{d:?}");
    assert_fixed(env, d);
}

/// `loaded`'ın `grants`'ını `grants` ile değiştirip planlar, tek transaction'da uygular, yeniden yükler; yüklenen
/// `grants` istenenin kanonik biçimi olmalı, `explicit = true` ve sabit nokta.
async fn apply(env: &Env, loaded: &AclDef, oid: u32, grants: Vec<AclGrant>, cascade: bool) -> AclDef {
    let new = AclDef { grants, ..loaded.clone() };
    let plan = privileges(&env.ctx(false), loaded, &new, cascade).unwrap_or_else(|e| panic!("plan reddedildi: {e:?}"));
    assert!(!plan.statements.is_empty(), "değişiklik planı boş");
    env.exec_tx(&plan).await;
    let reloaded = ld(env, loaded.kind, oid).await;
    assert_eq!(reloaded.grants, canonical(loaded.kind, new.grants.clone()), "plan: {:?}", sqls(&plan));
    assert!(reloaded.explicit, "uygulama sonrası ACL açık olmalı");
    assert_eq!((&reloaded.schema, &reloaded.name, &reloaded.signature), (&loaded.schema, &loaded.name, &loaded.signature));
    assert_fixed(env, &reloaded);
    reloaded
}

// ================================================================================================================
// varsayılan (NULL) ACL'nin etkin gösterimi
// ================================================================================================================

docker_test!(acl_defaults_show_effective_acl, defaults_body, "defaults");

async fn defaults_body(ts: TestServer, tag: &'static str) {
    let env = setup(&ts, tag, &[]).await;
    let major = env.version / 10000;
    let s = format!("pgt_ddl_rt_{tag}");
    env.sql(&format!(
        "CREATE SCHEMA {s};
         CREATE TABLE {s}.t (a int);
         CREATE VIEW {s}.v AS SELECT a FROM {s}.t;
         CREATE MATERIALIZED VIEW {s}.mv AS SELECT a FROM {s}.t;
         CREATE SEQUENCE {s}.sq;
         CREATE FUNCTION {s}.f(a integer) RETURNS integer LANGUAGE sql AS 'SELECT a';
         CREATE TYPE {s}.e AS ENUM ('x');
         CREATE TYPE {s}.c AS (a int);
         CREATE DOMAIN {s}.d AS integer"
    ))
    .await;
    let me = Some(env.me.as_str());
    use ObjectKind as K;
    use Privilege::*;

    for (kind, name) in [(K::Table, "t"), (K::View, "v"), (K::MaterializedView, "mv"), (K::Sequence, "sq")] {
        let d = ld(&env, kind, env.class_oid(&s, name).await).await;
        assert_eq!((d.schema.as_deref(), d.name.as_str(), d.signature.as_deref()), (Some(s.as_str()), name, None));
        assert_default(&env, &d, kind, vec![owner_all(&env, kind)]);
    }
    let t = ld(&env, K::Table, env.class_oid(&s, "t").await).await;
    assert_eq!(t.grants[0].privileges.iter().any(|p| p.privilege == Maintain), major >= 17, "MAINTAIN yalnız PG 17+");

    let f = ld(&env, K::Function, env.proc_oid(&s, "f", "integer").await).await;
    assert_eq!(f.signature.as_deref(), Some("a integer"));
    assert_default(&env, &f, K::Function, vec![row(None, &[Execute]), row(me, &[Execute])]);

    if major >= 11 {
        env.sql(&format!("CREATE PROCEDURE {s}.p(a integer) LANGUAGE plpgsql AS 'BEGIN END'")).await;
        let p = ld(&env, K::Procedure, env.proc_oid(&s, "p", "integer").await).await;
        assert_default(&env, &p, K::Procedure, vec![row(None, &[Execute]), row(me, &[Execute])]);
    } else {
        eprintln!("PG {major}: procedure atlandı (PG 11+)");
    }

    let sch = ld(&env, K::Schema, env.schema_oid(&s).await).await;
    assert_eq!((sch.schema.as_deref(), sch.name.as_str()), (None, s.as_str()));
    assert_default(&env, &sch, K::Schema, vec![row(me, &[Usage, Create])]);

    let db = load_acl(&env.mgr, env.server, None, ObjectRef { kind: K::Database, oid: env.db_oid }).await.unwrap();
    assert_eq!(db.schema, None);
    assert_default(&env, &db, K::Database, vec![row(None, &[Temporary, Connect]), row(me, &[Create, Temporary, Connect])]);

    for (kind, name) in [(K::Type, "e"), (K::Type, "c"), (K::Domain, "d")] {
        let d = ld(&env, kind, env.type_oid(&s, name).await).await;
        assert_default(&env, &d, kind, vec![row(None, &[Usage]), row(me, &[Usage])]);
    }
}

// ================================================================================================================
// tablo: GRANT, GRANT OPTION ekleme/kaldırma, tümünü kaldırma
// ================================================================================================================

docker_test!(acl_table_grant_and_grant_option_roundtrip, table_body, "table");

async fn table_body(ts: TestServer, tag: &'static str) {
    let env = setup(&ts, tag, &["1", "2"]).await;
    let (r1, r2) = (role(tag, "1"), role(tag, "2"));
    let s = format!("pgt_ddl_rt_{tag}");
    env.sql(&format!("CREATE SCHEMA {s}; CREATE TABLE {s}.t (a int)")).await;
    let oid = env.class_oid(&s, "t").await;
    let l0 = ld(&env, ObjectKind::Table, oid).await;
    use Privilege::*;
    let owner = owner_all(&env, ObjectKind::Table);

    // İlk değişiklik: NULL ACL → açık ACL; sahip satırı korunur.
    let l1 = apply(
        &env,
        &l0,
        oid,
        vec![owner.clone(), row(Some(&r1), &[Select, Insert]), row_go(Some(&r2), &[(Select, true), (Update, false)])],
        false,
    )
    .await;
    assert_eq!(l1.grants.iter().find(|g| g.grantee.as_deref() == Some(env.me.as_str())), Some(&owner));

    // GRANT OPTION kaldırma (r2 SELECT) ve ekleme (r1 INSERT false→true).
    let p = privileges(
        &env.ctx(false),
        &l1,
        &AclDef {
            grants: vec![owner.clone(), row_go(Some(&r1), &[(Select, false), (Insert, true)]), row(Some(&r2), &[Select, Update])],
            ..l1.clone()
        },
        false,
    )
    .unwrap();
    let q = |r: &str| quote_ident(r);
    assert_eq!(
        sqls(&p),
        [
            format!("REVOKE GRANT OPTION FOR SELECT ON TABLE {s}.t FROM {}", q(&r2)),
            format!("GRANT INSERT ON TABLE {s}.t TO {} WITH GRANT OPTION", q(&r1)),
        ]
    );
    let l2 = apply(
        &env,
        &l1,
        oid,
        vec![owner.clone(), row_go(Some(&r1), &[(Select, false), (Insert, true)]), row(Some(&r2), &[Select, Update])],
        false,
    )
    .await;

    // Alıcının tüm yetkilerini kaldırma (boş satır) → alıcı listeden düşer.
    let l3 = apply(&env, &l2, oid, vec![owner.clone(), row(Some(&r1), &[]), row(Some(&r2), &[Update])], false).await;
    assert_eq!(l3.grants.len(), 2);
}

// ================================================================================================================
// fonksiyon: PUBLIC EXECUTE kaldırma, aşırı yükleme, procedure
// ================================================================================================================

docker_test!(acl_function_public_execute_and_overloads, function_body, "function");

async fn function_body(ts: TestServer, tag: &'static str) {
    let env = setup(&ts, tag, &["1"]).await;
    let major = env.version / 10000;
    let r1 = role(tag, "1");
    let s = format!("pgt_ddl_rt_{tag}");
    env.sql(&format!(
        "CREATE SCHEMA {s};
         CREATE FUNCTION {s}.f(a integer) RETURNS text LANGUAGE sql AS 'SELECT ''int''::text';
         CREATE FUNCTION {s}.f(a text) RETURNS text LANGUAGE sql AS 'SELECT a'"
    ))
    .await;
    use Privilege::*;
    let me = Some(env.me.as_str());
    let (fi, ft) = (env.proc_oid(&s, "f", "integer").await, env.proc_oid(&s, "f", "text").await);
    let li = ld(&env, ObjectKind::Function, fi).await;
    let lt0 = ld(&env, ObjectKind::Function, ft).await;

    // NULL ACL'de PUBLIC EXECUTE kaldırma → tek REVOKE; sonra explicit = true, sahip satırı korunur.
    let new = AclDef { grants: vec![row(me, &[Execute])], ..li.clone() };
    let p = privileges(&env.ctx(false), &li, &new, false).unwrap();
    assert_eq!(sqls(&p), [format!("REVOKE EXECUTE ON FUNCTION {s}.f(a integer) FROM PUBLIC")]);
    let li2 = apply(&env, &li, fi, vec![row(me, &[Execute])], false).await;
    assert_eq!(li2.grants, [row(me, &[Execute])]);

    // Diğer aşırı yükleme değişmedi.
    assert_eq!(ld(&env, ObjectKind::Function, ft).await, lt0);

    // GRANT OPTION'lı yetki.
    apply(&env, &li2, fi, vec![row(me, &[Execute]), row_go(Some(&r1), &[(Execute, true)])], false).await;
    assert_eq!(ld(&env, ObjectKind::Function, ft).await, lt0);

    if major >= 11 {
        env.sql(&format!("CREATE PROCEDURE {s}.p(a integer, b text) LANGUAGE plpgsql AS 'BEGIN END'")).await;
        let po = env.proc_oid(&s, "p", "integer, text").await;
        let lp = ld(&env, ObjectKind::Procedure, po).await;
        // PG 14+ procedure imzasında giriş modunu (`IN`) da yazar (OUT argümanlı procedure); imza olduğu gibi kullanılır.
        let sig = lp.signature.clone().unwrap();
        assert_eq!(sig, if major >= 14 { "IN a integer, IN b text" } else { "a integer, b text" });
        let p = privileges(&env.ctx(false), &lp, &AclDef { grants: vec![row(me, &[Execute]), row(Some(&r1), &[Execute])], ..lp.clone() }, false)
            .unwrap();
        assert_eq!(
            sqls(&p),
            [
                format!("REVOKE EXECUTE ON PROCEDURE {s}.p({sig}) FROM PUBLIC"),
                format!("GRANT EXECUTE ON PROCEDURE {s}.p({sig}) TO {}", quote_ident(&r1)),
            ]
        );
        apply(&env, &lp, po, vec![row(me, &[Execute]), row(Some(&r1), &[Execute])], false).await;
        // Procedure oid'i Function olarak (ya da tersi) bulunamaz.
        assert!(matches!(try_load(&env, ObjectKind::Function, po).await, Err(Error::NotFound(_))));
        assert!(matches!(try_load(&env, ObjectKind::Procedure, fi).await, Err(Error::NotFound(_))));
    } else {
        eprintln!("PG {major}: procedure atlandı (PG 11+)");
    }
}

// ================================================================================================================
// veren ayrımı (foreign), RESTRICT 2BP01, CASCADE
// ================================================================================================================

// ================================================================================================================
// boş ACL ('{}'): sahip dahil herkesten her yetki kaldırılınca
// ================================================================================================================

docker_test!(acl_empty_acl_loads_without_rows, empty_body, "empty");

async fn empty_body(ts: TestServer, tag: &'static str) {
    let env = setup(&ts, tag, &[]).await;
    let s = format!("pgt_ddl_rt_{tag}");
    env.sql(&format!("CREATE SCHEMA {s}; CREATE TABLE {s}.t (a int); CREATE FUNCTION {s}.f() RETURNS int LANGUAGE sql AS 'SELECT 1'")).await;
    let me = Some(env.me.as_str());
    for (kind, oid) in [(ObjectKind::Table, env.class_oid(&s, "t").await), (ObjectKind::Function, env.proc_oid(&s, "f", "").await)] {
        let l = ld(&env, kind, oid).await;
        // Sahip satırı boş + PUBLIC yok → `aclexplode('{}')` hiç satır vermez.
        let e = apply(&env, &l, oid, vec![row(me, &[])], false).await;
        let raw: String = env
            .admin
            .query_one("SELECT coalesce((SELECT relacl::text FROM pg_class WHERE oid = $1), (SELECT proacl::text FROM pg_proc WHERE oid = $1))", &[&oid])
            .await
            .unwrap()
            .get(0);
        assert_eq!(raw, "{}", "{kind:?}");
        assert!(e.grants.is_empty() && e.foreign.is_empty() && e.explicit && e.editable, "{kind:?}: {e:?}");
        assert_fixed(&env, &e);
        // Geri verme de çalışır.
        apply(&env, &e, oid, vec![owner_all(&env, kind)], false).await;
    }
}

// ================================================================================================================
// aggregate salt okunur (R4a); pencere fonksiyonu düzenlenebilir
// ================================================================================================================

docker_test!(acl_aggregates_are_read_only, aggregate_body, "aggregate");

async fn aggregate_body(ts: TestServer, tag: &'static str) {
    let env = setup(&ts, tag, &[]).await;
    let s = format!("pgt_ddl_rt_{tag}");
    env.sql(&format!("CREATE SCHEMA {s}; CREATE AGGREGATE {s}.ag(integer) (sfunc = int4pl, stype = integer)")).await;
    let oid = env.proc_oid(&s, "ag", "integer").await;
    let d = ld(&env, ObjectKind::Function, oid).await;
    assert!(!d.editable && d.unsupported.len() == 1 && d.unsupported[0].starts_with("AGGREGATE"), "{d:?}");
    assert_fixed(&env, &d);
    let e = privileges(&env.ctx(false), &d, &AclDef { grants: vec![], ..d.clone() }, false).unwrap_err();
    assert_eq!(paths(&e), ["object"]);
    // Sıralı küme aggregate'i (imzası ORDER BY içerir) ve sistem pencere fonksiyonu.
    let sys = |name: &'static str| {
        let env = &env;
        async move {
            let sql = "SELECT p.oid FROM pg_proc p JOIN pg_namespace n ON n.oid = p.pronamespace WHERE n.nspname = 'pg_catalog' AND p.proname = $1::text ORDER BY p.oid LIMIT 1";
            env.admin.query_one(sql, &[&name]).await.unwrap().get::<_, u32>(0)
        }
    };
    let pd = ld(&env, ObjectKind::Function, sys("percentile_disc").await).await;
    assert!(pd.unsupported.iter().any(|u| u.starts_with("AGGREGATE")), "{pd:?}");
    assert!(pd.signature.as_deref().unwrap_or_default().contains("ORDER BY"), "{pd:?}");
    let rn = ld(&env, ObjectKind::Function, sys("row_number").await).await;
    assert_eq!(rn.unsupported, ["sistem şeması"], "pencere fonksiyonu yalnız sistem şeması nedeniyle salt okunur");
}

docker_test!(acl_foreign_grants_restrict_and_cascade, foreign_body, "foreign");

async fn foreign_body(ts: TestServer, tag: &'static str) {
    let env = setup(&ts, tag, &["2", "3"]).await;
    let (r2, r3) = (role(tag, "2"), role(tag, "3"));
    let (q2, q3) = (quote_ident(&r2), quote_ident(&r3));
    let s = format!("pgt_ddl_rt_{tag}");
    env.sql(&format!("CREATE SCHEMA {s}; CREATE TABLE {s}.t (a int)")).await;
    let oid = env.class_oid(&s, "t").await;
    use Privilege::*;
    let owner = owner_all(&env, ObjectKind::Table);
    let l0 = ld(&env, ObjectKind::Table, oid).await;
    let l1 = apply(&env, &l0, oid, vec![owner.clone(), row_go(Some(&r2), &[(Select, true)])], false).await;

    // r2, GRANT OPTION'la r3'e verir (SET ROLE: süper kullanıcı üyeliksiz geçebilir).
    env.sql(&format!("GRANT USAGE ON SCHEMA {s} TO {q2}; SET ROLE {q2}; GRANT SELECT ON {s}.t TO {q3}; RESET ROLE")).await;
    let l2 = ld(&env, ObjectKind::Table, oid).await;
    let fg = ForeignGrant { grantor: r2.clone(), grantee: Some(r3.clone()), privilege: Select, grant_option: false };
    assert_eq!(l2.foreign, std::slice::from_ref(&fg));
    assert_eq!(l2.grants, l1.grants, "başka verenin girdisi grants'a girmez");
    assert_fixed(&env, &l2);
    let e = privileges(&env.ctx(false), &l2, &AclDef { foreign: vec![], ..l2.clone() }, false).unwrap_err();
    assert_eq!(paths(&e), ["foreign"]);

    // Bağımlı yetki varken RESTRICT: REVOKE ve REVOKE GRANT OPTION FOR → 2BP01, ACL değişmez.
    for grants in [vec![owner.clone()], vec![owner.clone(), row(Some(&r2), &[Select])]] {
        let p = privileges(&env.ctx(false), &l2, &AclDef { grants, ..l2.clone() }, false).unwrap();
        let err = env.try_exec_tx(&p).await.expect_err("bağımlı yetki varken RESTRICT başarılı oldu");
        assert_eq!(sqlstate(&err), "2BP01", "{err}");
        assert_eq!(ld(&env, ObjectKind::Table, oid).await, l2, "başarısız plan ACL'yi değiştirdi");
    }

    // Aynı alıcı (r3) sahipten de alır: sahip girdisi grants'ta, r2'ninki foreign'da kalır.
    let l3 = apply(&env, &l2, oid, vec![owner.clone(), row_go(Some(&r2), &[(Select, true)]), row(Some(&r3), &[Select])], false).await;
    assert_eq!(l3.foreign, std::slice::from_ref(&fg));
    // Sahip girdisini kaldırmak yalnız onu kaldırır.
    let l4 = apply(&env, &l3, oid, vec![owner.clone(), row_go(Some(&r2), &[(Select, true)])], false).await;
    assert_eq!(l4.foreign, [fg]);

    // CASCADE: r2'nin yetkisi ve r2'nin verdiği bağımlı yetki birlikte gider.
    let p = privileges(&env.ctx(false), &l4, &AclDef { grants: vec![owner.clone()], ..l4.clone() }, true).unwrap();
    assert_eq!(sqls(&p), [format!("REVOKE SELECT ON TABLE {s}.t FROM {q2} CASCADE")]);
    assert!(p.statements.iter().all(|st| st.risk == Risk::DataLoss));
    let l5 = apply(&env, &l4, oid, vec![owner], true).await;
    assert!(l5.foreign.is_empty(), "{:?}", l5.foreign);
}

// ================================================================================================================
// kolon yetkileri (salt okunur)
// ================================================================================================================

docker_test!(acl_column_grants_are_listed, columns_body, "columns");

async fn columns_body(ts: TestServer, tag: &'static str) {
    let env = setup(&ts, tag, &["1"]).await;
    let r1 = role(tag, "1");
    let q1 = quote_ident(&r1);
    let s = format!("pgt_ddl_rt_{tag}");
    env.sql(&format!(
        r#"CREATE SCHEMA {s}; CREATE TABLE {s}.t (a int, "B c" text, z int);
           GRANT INSERT ("B c") ON {s}.t TO {q1}; GRANT SELECT (a), UPDATE (a) ON {s}.t TO {q1}; GRANT SELECT (z) ON {s}.t TO PUBLIC"#
    ))
    .await;
    let oid = env.class_oid(&s, "t").await;
    let d = ld(&env, ObjectKind::Table, oid).await;
    use Privilege::*;
    let c = |column: &str, grantee: Option<&str>, privilege| ColumnGrant {
        column: column.into(),
        grantor: env.me.clone(),
        grantee: grantee.map(Into::into),
        privilege,
        grant_option: false,
    };
    // Kolon sırası (attnum), sonra alıcı, veren, kanonik yetki sırası.
    assert_eq!(
        d.columns,
        [c("a", Some(&r1), Select), c("a", Some(&r1), Update), c("B c", Some(&r1), Insert), c("z", None, Select)]
    );
    // Kolon yetkileri tablo ACL'sini değiştirmez.
    assert!(!d.explicit && d.foreign.is_empty() && d.editable);
    assert_eq!(d.grants, [owner_all(&env, ObjectKind::Table)]);
    assert_fixed(&env, &d);
    let e = privileges(&env.ctx(false), &d, &AclDef { columns: vec![], ..d.clone() }, false).unwrap_err();
    assert_eq!(paths(&e), ["columns"]);
    // Tablo düzeyi değişiklik kolon yetkilerini korur.
    let d2 = apply(&env, &d, oid, vec![owner_all(&env, ObjectKind::Table), row(Some(&r1), &[Select])], false).await;
    assert_eq!(d2.columns, d.columns);
}

// ================================================================================================================
// hostile adlar: rol, şema, tablo, fonksiyon imzası
// ================================================================================================================

docker_test!(acl_hostile_names_roundtrip, hostile_body, "hostile");

async fn hostile_body(ts: TestServer, tag: &'static str) {
    let env = setup(&ts, tag, &[r#"Ş "q" Büyük x"#]).await;
    let hr = role(tag, r#"Ş "q" Büyük x"#);
    let s = format!(r#"pgt_ddl_rt_{tag} "Ş" X"#);
    let qs = quote_ident(&s);
    env.sql(&format!(
        r#"CREATE SCHEMA {qs};
           CREATE TABLE {qs}."T ""x"" ş" (a int);
           CREATE TYPE {qs}."Tip ""q""" AS ENUM ('a');
           CREATE FUNCTION {qs}."F ""x"""(a {qs}."Tip ""q""", VARIADIC b integer[]) RETURNS int LANGUAGE sql AS 'SELECT 1';
           CREATE FUNCTION {qs}."F ""x"""(a integer) RETURNS int LANGUAGE sql AS 'SELECT 2'"#
    ))
    .await;
    use Privilege::*;
    let me = Some(env.me.as_str());

    let toid = env.class_oid(&s, r#"T "x" ş"#).await;
    let t = ld(&env, ObjectKind::Table, toid).await;
    assert_eq!((t.schema.as_deref(), t.name.as_str()), (Some(s.as_str()), r#"T "x" ş"#));
    let t2 = apply(&env, &t, toid, vec![owner_all(&env, ObjectKind::Table), row_go(Some(&hr), &[(Select, true), (Delete, false)])], false).await;
    assert_eq!(t2.grants.iter().filter_map(|g| g.grantee.as_deref()).collect::<Vec<_>>(), [hr.as_str(), env.me.as_str()]);

    let soid = env.schema_oid(&s).await;
    let sch = ld(&env, ObjectKind::Schema, soid).await;
    apply(&env, &sch, soid, vec![row(me, &[Usage, Create]), row(Some(&hr), &[Usage])], false).await;

    let foid = env.proc_oid(&s, r#"F "x""#, &format!(r#"{qs}."Tip ""q""", integer[]"#)).await;
    let other = env.proc_oid(&s, r#"F "x""#, "integer").await;
    let lother = ld(&env, ObjectKind::Function, other).await;
    let f = ld(&env, ObjectKind::Function, foid).await;
    let sig = f.signature.clone().unwrap();
    assert!(sig.contains(r#""Tip ""q""""#) && sig.contains("VARIADIC b integer[]"), "{sig}");
    let f2 = apply(&env, &f, foid, vec![row(me, &[Execute]), row(Some(&hr), &[Execute])], false).await;
    assert_eq!(f2.grants.len(), 2);
    assert_eq!(ld(&env, ObjectKind::Function, other).await, lother, "aşırı yüklenmiş diğer fonksiyon değişti");
    // Sunucu tarafında doğru nesne: hedeflenen imzada PUBLIC kaldırıldı, hostile rol eklendi; diğerinin ACL'si NULL.
    let acl_sql = "SELECT proacl IS NULL, has_function_privilege($1::text, oid, 'EXECUTE') FROM pg_proc WHERE oid = $2::oid";
    let (null_acl, can): (bool, bool) = {
        let r = env.admin.query_one(acl_sql, &[&hr, &foid]).await.unwrap();
        (r.get(0), r.get(1))
    };
    assert!(!null_acl && can);
    assert!(!lother.explicit);
    // Hostile rolün tablo yetkisi sunucuda da doğru nesnede.
    let can_select: bool = env
        .admin
        .query_one("SELECT has_table_privilege($1::text, $2::oid, 'SELECT WITH GRANT OPTION')", &[&hr, &toid])
        .await
        .unwrap()
        .get(0);
    assert!(can_select);
}

// ================================================================================================================
// MAINTAIN (PG 17+)
// ================================================================================================================

docker_test!(acl_maintain_roundtrip_pg17, maintain_body, "maintain");

async fn maintain_body(ts: TestServer, tag: &'static str) {
    let env = setup(&ts, tag, &["1"]).await;
    let major = env.version / 10000;
    let r1 = role(tag, "1");
    let s = format!("pgt_ddl_rt_{tag}");
    env.sql(&format!("CREATE SCHEMA {s}; CREATE TABLE {s}.t (a int)")).await;
    let oid = env.class_oid(&s, "t").await;
    let l = ld(&env, ObjectKind::Table, oid).await;
    let grants = vec![owner_all(&env, ObjectKind::Table), row(Some(&r1), &[Privilege::Maintain])];
    if major >= 17 {
        let p = privileges(&env.ctx(false), &l, &AclDef { grants: grants.clone(), ..l.clone() }, false).unwrap();
        assert_eq!(sqls(&p), [format!("GRANT MAINTAIN ON TABLE {s}.t TO {}", quote_ident(&r1))]);
        apply(&env, &l, oid, grants, false).await;
    } else {
        eprintln!("PG {major}: MAINTAIN gidiş-dönüşü atlandı (PG 17+)");
        let e = privileges(&env.ctx(false), &l, &AclDef { grants, ..l.clone() }, false).unwrap_err();
        assert_eq!(paths(&e), ["grants[1].privileges[0].privilege"]);
    }
}

// ================================================================================================================
// diğer türler: sequence, görünüm, MV, şema, veritabanı, tip, domain
// ================================================================================================================

docker_test!(acl_other_kinds_roundtrip, kinds_body, "kinds");

async fn kinds_body(ts: TestServer, tag: &'static str) {
    let env = setup(&ts, tag, &["1", "2"]).await;
    let (r1, r2) = (role(tag, "1"), role(tag, "2"));
    let s = format!("pgt_ddl_rt_{tag}");
    env.sql(&format!(
        "CREATE SCHEMA {s};
         CREATE TABLE {s}.t (a int);
         CREATE VIEW {s}.v AS SELECT a FROM {s}.t;
         CREATE MATERIALIZED VIEW {s}.mv AS SELECT a FROM {s}.t;
         CREATE SEQUENCE {s}.sq;
         CREATE TYPE {s}.e AS ENUM ('x');
         CREATE DOMAIN {s}.d AS integer"
    ))
    .await;
    use ObjectKind as K;
    use Privilege::*;
    let me = Some(env.me.as_str());

    let sq = env.class_oid(&s, "sq").await;
    let l = ld(&env, K::Sequence, sq).await;
    apply(&env, &l, sq, vec![owner_all(&env, K::Sequence), row_go(Some(&r1), &[(Usage, false), (Select, true)])], false).await;

    for (kind, name) in [(K::View, "v"), (K::MaterializedView, "mv")] {
        let oid = env.class_oid(&s, name).await;
        let l = ld(&env, kind, oid).await;
        apply(&env, &l, oid, vec![owner_all(&env, kind), row(Some(&r1), &[Select]), row(None, &[Select])], false).await;
    }

    let so = env.schema_oid(&s).await;
    let l = ld(&env, K::Schema, so).await;
    let l2 = apply(&env, &l, so, vec![row(me, &[Usage, Create]), row(Some(&r1), &[Usage]), row_go(Some(&r2), &[(Create, true)])], false).await;
    apply(&env, &l2, so, vec![row(me, &[Usage, Create]), row(Some(&r1), &[Usage]), row(Some(&r2), &[Create])], false).await;

    let dbref = ObjectRef { kind: K::Database, oid: env.db_oid };
    let l = load_acl(&env.mgr, env.server, None, dbref).await.unwrap();
    let new = vec![row(None, &[Connect]), row(me, &[Create, Temporary, Connect]), row(Some(&r1), &[Connect])];
    let p = privileges(&env.ctx(false), &l, &AclDef { grants: new.clone(), ..l.clone() }, false).unwrap();
    env.exec_tx(&p).await;
    // Veritabanı bakım bağlantısından okunur; `database_oid` yok sayılır.
    let l2 = load_acl(&env.mgr, env.server, 12345u32, dbref).await.unwrap();
    assert_eq!(l2.grants, canonical(K::Database, new));
    assert!(l2.explicit);
    assert_fixed(&env, &l2);

    for (kind, name) in [(K::Type, "e"), (K::Domain, "d")] {
        let oid = env.type_oid(&s, name).await;
        let l = ld(&env, kind, oid).await;
        let p = privileges(&env.ctx(false), &l, &AclDef { grants: vec![row(me, &[Usage]), row_go(Some(&r1), &[(Usage, true)])], ..l.clone() }, false)
            .unwrap();
        let what = if kind == K::Type { "TYPE" } else { "DOMAIN" };
        assert_eq!(
            sqls(&p),
            [format!("REVOKE USAGE ON {what} {s}.{name} FROM PUBLIC"), format!("GRANT USAGE ON {what} {s}.{name} TO {} WITH GRANT OPTION", quote_ident(&r1))]
        );
        apply(&env, &l, oid, vec![row(me, &[Usage]), row_go(Some(&r1), &[(Usage, true)])], false).await;
    }
}

// ================================================================================================================
// salt okunur: sistem şeması, dizi/multirange/satır tipleri; tür uyuşmazlığı ve hedef hataları
// ================================================================================================================

docker_test!(acl_read_only_and_kind_mismatch, readonly_body, "readonly");

async fn readonly_body(ts: TestServer, tag: &'static str) {
    let env = setup(&ts, tag, &[]).await;
    let major = env.version / 10000;
    let s = format!("pgt_ddl_rt_{tag}");
    env.sql(&format!(
        "CREATE SCHEMA {s};
         CREATE TABLE {s}.t (a int);
         CREATE SEQUENCE {s}.sq;
         CREATE TYPE {s}.e AS ENUM ('x');
         CREATE TYPE {s}.c AS (a int);
         CREATE TYPE {s}.r AS RANGE (subtype = integer);
         CREATE DOMAIN {s}.d AS integer;
         CREATE INDEX i ON {s}.t (a)"
    ))
    .await;
    use ObjectKind as K;

    // Sistem şeması nesnesi ve şemanın kendisi.
    for (kind, oid) in [(K::Table, env.class_oid("pg_catalog", "pg_class").await), (K::Schema, env.schema_oid("pg_catalog").await)] {
        let d = ld(&env, kind, oid).await;
        assert!(!d.editable && d.unsupported == ["sistem şeması"], "{d:?}");
        assert_fixed(&env, &d);
        // Herhangi bir değişiklik reddedilir (ilk satırı düşür ya da PUBLIC'e ekle).
        let mut grants = d.grants.clone();
        if grants.is_empty() {
            grants.push(row(None, &[privileges_for(kind, env.version)[0]]));
        } else {
            grants.remove(0);
        }
        let e = privileges(&env.ctx(false), &d, &AclDef { grants, ..d.clone() }, false).unwrap_err();
        assert_eq!(paths(&e), ["object"], "{kind:?}");
    }
    let info = ld(&env, K::Schema, env.schema_oid("information_schema").await).await;
    assert!(!info.editable, "{info:?}");

    // Dizi tipi, tablo satır tipi, multirange (PG 14+) salt okunur; bağımsız composite ve enum düzenlenebilir.
    let arr = ld(&env, K::Type, env.type_oid(&s, "_e").await).await;
    assert!(!arr.editable && arr.unsupported.iter().any(|u| u.contains("dizi")), "{arr:?}");
    assert_fixed(&env, &arr);
    let rowtype = ld(&env, K::Type, env.type_oid(&s, "t").await).await;
    assert!(!rowtype.editable && rowtype.unsupported.iter().any(|u| u.contains("satır tipi")), "{rowtype:?}");
    for name in ["c", "e", "r"] {
        let d = ld(&env, K::Type, env.type_oid(&s, name).await).await;
        assert!(d.editable, "{name}: {d:?}");
    }
    if major >= 14 {
        let mr = ld(&env, K::Type, env.type_oid(&s, "r_multirange").await).await;
        assert!(!mr.editable && mr.unsupported.iter().any(|u| u.contains("MULTIRANGE")), "{mr:?}");
    } else {
        eprintln!("PG {major}: multirange salt okunurluğu atlandı (PG 14+)");
    }

    // Tür uyuşmazlığı → NotFound.
    let (t, sq) = (env.class_oid(&s, "t").await, env.class_oid(&s, "sq").await);
    let (e, d) = (env.type_oid(&s, "e").await, env.type_oid(&s, "d").await);
    for (kind, oid) in [
        (K::Table, sq),
        (K::Sequence, t),
        (K::View, t),
        (K::MaterializedView, t),
        (K::Table, env.class_oid(&s, "i").await),
        (K::Type, d),
        (K::Domain, e),
        (K::Table, 0),
        (K::Schema, 0),
        (K::Function, 0),
    ] {
        assert!(matches!(try_load(&env, kind, oid).await, Err(Error::NotFound(_))), "{kind:?} {oid}");
    }
    assert!(matches!(
        load_acl(&env.mgr, env.server, None, ObjectRef { kind: K::Database, oid: 0 }).await,
        Err(Error::NotFound(_))
    ));
    // Veritabanı gerektiren tür + `database_oid` yok → NEEDS_DATABASE; yetkisi olmayan tür → Invalid.
    match load_acl(&env.mgr, env.server, None, ObjectRef { kind: K::Table, oid: t }).await {
        Err(Error::Invalid(m)) => assert_eq!(m, error_codes::NEEDS_DATABASE),
        other => panic!("{other:?}"),
    }
    for kind in [K::Index, K::Extension, K::Role, K::Trigger] {
        assert!(matches!(try_load(&env, kind, t).await, Err(Error::Invalid(_))), "{kind:?}");
    }
}

// ================================================================================================================
// varsayılan yetkiler (`load_default_acl` ↔ `build::default_acl::default_privileges`)
// ================================================================================================================

async fn try_load_def(env: &Env, role: Option<&str>, schema_oid: Option<u32>) -> pgt_core::Result<DefaultAclDef> {
    load_default_acl(&env.mgr, env.server, env.db_oid, DefaultAclScope { role: role.map(Into::into), schema_oid }).await
}

async fn ldd(env: &Env, role: Option<&str>, schema_oid: Option<u32>) -> DefaultAclDef {
    try_load_def(env, role, schema_oid).await.unwrap_or_else(|e| panic!("varsayılan yetkiler yüklenemedi: {e:?}"))
}

fn section(d: &DefaultAclDef, t: DefaultAclObjectType) -> &DefaultAclSection {
    d.sections.iter().find(|s| s.object_type == t).unwrap_or_else(|| panic!("{t:?} bölümü yok"))
}

fn section_mut(d: &mut DefaultAclDef, t: DefaultAclObjectType) -> &mut DefaultAclSection {
    d.sections.iter_mut().find(|s| s.object_type == t).unwrap_or_else(|| panic!("{t:?} bölümü yok"))
}

fn def_canonical(t: DefaultAclObjectType, mut v: Vec<AclGrant>) -> Vec<AclGrant> {
    v.retain(|g| !g.privileges.is_empty());
    sort_grants(&mut v, default_privileges_for(t, 170000));
    v
}

/// Rolün `acldefault` satırı: bölüm türünün (sürüme göre) tüm yetkileri.
fn def_owner_all(env: &Env, who: &str, t: DefaultAclObjectType) -> AclGrant {
    row(Some(who), default_privileges_for(t, env.version))
}

/// `default_privileges(d, d)` boş plan.
fn assert_def_fixed(env: &Env, d: &DefaultAclDef) {
    let p = default_privileges(&env.ctx(false), d, d).unwrap_or_else(|e| panic!("sabit nokta reddedildi: {e:?}"));
    assert!(p.statements.is_empty() && p.warnings.is_empty(), "sabit nokta boş değil: {:?}", sqls(&p));
}

/// `loaded`'ı `edit` ile değiştirip planlar, tek transaction'da uygular, aynı kapsamla yeniden yükler; bölümlerin
/// `grants`'ı istenenin kanonik biçimi, `baseline` değişmemiş ve sabit nokta olmalı.
async fn apply_def(env: &Env, loaded: &DefaultAclDef, scope: (Option<&str>, Option<u32>), edit: impl FnOnce(&mut DefaultAclDef)) -> (Plan, DefaultAclDef) {
    let mut new = loaded.clone();
    edit(&mut new);
    let plan = default_privileges(&env.ctx(false), loaded, &new).unwrap_or_else(|e| panic!("plan reddedildi: {e:?}"));
    assert!(!plan.statements.is_empty(), "değişiklik planı boş");
    env.exec_tx(&plan).await;
    let reloaded = ldd(env, scope.0, scope.1).await;
    for (got, want) in reloaded.sections.iter().zip(&new.sections) {
        assert_eq!(got.grants, def_canonical(want.object_type, want.grants.clone()), "{:?} plan: {:?}", want.object_type, sqls(&plan));
        assert_eq!(got.baseline, want.baseline, "{:?}", want.object_type);
    }
    assert_eq!((&reloaded.role, &reloaded.schema, &reloaded.viewer), (&loaded.role, &loaded.schema, &loaded.viewer));
    assert_def_fixed(env, &reloaded);
    (plan, reloaded)
}

async fn flag(env: &Env, sql: &str, params: &[&(dyn tokio_postgres::types::ToSql + Sync)]) -> bool {
    env.admin.query_one(sql, params).await.unwrap_or_else(|e| panic!("sorgu başarısız: {e}")).get(0)
}

/// Fonksiyonun etkin ACL'sinde PUBLIC girdisi var mı.
const PROC_PUBLIC_SQL: &str =
    "SELECT EXISTS (SELECT 1 FROM pg_proc p, aclexplode(coalesce(p.proacl, acldefault('f', p.proowner))) a WHERE p.oid = $1::oid AND a.grantee = 0)";

// ---- boş durum: acldefault değerleri, şema kapsamı, rol/şema bulunamadı, sistem şeması ----

docker_test!(default_acl_effective_defaults, def_defaults_body, "ddefaults");

async fn def_defaults_body(ts: TestServer, tag: &'static str) {
    let env = setup(&ts, tag, &["1"]).await;
    let major = env.version / 10000;
    let r1 = role(tag, "1");
    let s = format!("pgt_ddl_rt_{tag}");
    env.sql(&format!("CREATE SCHEMA {s}")).await;
    let soid = env.schema_oid(&s).await;
    use DefaultAclObjectType::*;
    use Privilege::*;
    let me = env.me.clone();

    let d = ldd(&env, None, None).await;
    assert_eq!((d.role.as_str(), d.viewer.as_str(), d.schema.as_deref()), (me.as_str(), me.as_str(), None));
    assert!(d.editable && d.unsupported.is_empty(), "{d:?}");
    assert_eq!(d.sections.iter().map(|x| x.object_type).collect::<Vec<_>>(), [Tables, Sequences, Functions, Types, Schemas]);
    assert_eq!(section(&d, Tables).grants, [def_owner_all(&env, &me, Tables)]);
    assert_eq!(section(&d, Tables).grants[0].privileges.iter().any(|p| p.privilege == Maintain), major >= 17, "MAINTAIN yalnız PG 17+");
    assert_eq!(section(&d, Sequences).grants, [row(Some(&me), &[Usage, Select, Update])]);
    assert_eq!(section(&d, Functions).grants, [row(None, &[Execute]), row(Some(&me), &[Execute])]);
    assert_eq!(section(&d, Types).grants, [row(None, &[Usage]), row(Some(&me), &[Usage])]);
    assert_eq!(section(&d, Schemas).grants, [row(Some(&me), &[Usage, Create])]);
    assert!(d.sections.iter().all(|x| x.baseline.is_empty()));
    assert_def_fixed(&env, &d);

    // Şema kapsamı: girdi yok → boş grants; baseline genel etkin değer; Schemas bölümü yok.
    let ds = ldd(&env, None, Some(soid)).await;
    assert_eq!((ds.role.as_str(), ds.schema.as_deref()), (me.as_str(), Some(s.as_str())));
    assert!(ds.editable && ds.unsupported.is_empty(), "{ds:?}");
    assert_eq!(ds.sections.iter().map(|x| x.object_type).collect::<Vec<_>>(), [Tables, Sequences, Functions, Types]);
    for x in &ds.sections {
        assert!(x.grants.is_empty(), "{x:?}");
        assert_eq!(x.baseline, section(&d, x.object_type).grants, "{:?}", x.object_type);
    }
    assert_def_fixed(&env, &ds);

    // Başka rol: kendi acldefault'u; bağlı rol değişmez.
    let d1 = ldd(&env, Some(&r1), None).await;
    assert_eq!((d1.role.as_str(), d1.viewer.as_str()), (r1.as_str(), me.as_str()));
    assert_eq!(section(&d1, Tables).grants, [def_owner_all(&env, &r1, Tables)]);
    assert_eq!(section(&d1, Functions).grants, [row(None, &[Execute]), row(Some(&r1), &[Execute])]);

    // Bulunamadı.
    let missing = role(tag, "yok");
    match try_load_def(&env, Some(&missing), None).await {
        Err(Error::NotFound(w)) => assert_eq!(w, "rol"),
        other => panic!("{other:?}"),
    }
    match try_load_def(&env, None, Some(0)).await {
        Err(Error::NotFound(w)) => assert_eq!(w, "şema"),
        other => panic!("{other:?}"),
    }

    // Sistem şeması: salt okunur, değişiklik reddedilir.
    let sys = ldd(&env, None, Some(env.schema_oid("pg_catalog").await)).await;
    assert!(!sys.editable && sys.unsupported == [SYSTEM_SCHEMA_REASON], "{sys:?}");
    assert_def_fixed(&env, &sys);
    let mut new = sys.clone();
    section_mut(&mut new, Tables).grants.push(row(Some(&r1), &[Select]));
    assert_eq!(paths(&default_privileges(&env.ctx(false), &sys, &new).unwrap_err()), ["object"]);
}

// ---- genel kapsam: GRANT/REVOKE yeni nesnelere uygulanır; boş ACL; varsayılana dönüş ----

docker_test!(default_acl_general_scope_applies_to_new_objects, def_general_body, "dgeneral");

async fn def_general_body(ts: TestServer, tag: &'static str) {
    let env = setup(&ts, tag, &["1", "2"]).await;
    let (r1, r2) = (role(tag, "1"), role(tag, "2"));
    let (q1, q2) = (quote_ident(&r1), quote_ident(&r2));
    let s = format!("pgt_ddl_rt_{tag}");
    env.sql(&format!("CREATE SCHEMA {s}; CREATE TABLE {s}.before (a int)")).await;
    use DefaultAclObjectType::*;
    use Privilege::*;
    let me = env.me.clone();
    let qme = quote_ident(&me);
    let d0 = ldd(&env, None, None).await;

    let (p, d1) = apply_def(&env, &d0, (None, None), |n| {
        section_mut(n, Tables).grants.push(row(Some(&r1), &[Select]));
        section_mut(n, Sequences).grants.push(row_go(Some(&r2), &[(Usage, true)]));
        section_mut(n, Functions).grants.retain(|g| g.grantee.is_some());
    })
    .await;
    assert_eq!(
        sqls(&p),
        [
            format!("ALTER DEFAULT PRIVILEGES FOR ROLE {qme} GRANT SELECT ON TABLES TO {q1}"),
            format!("ALTER DEFAULT PRIVILEGES FOR ROLE {qme} GRANT USAGE ON SEQUENCES TO {q2} WITH GRANT OPTION"),
            format!("ALTER DEFAULT PRIVILEGES FOR ROLE {qme} REVOKE EXECUTE ON FUNCTIONS FROM PUBLIC"),
        ]
    );
    assert!(p.statements.iter().all(|st| st.risk == Risk::Safe));
    assert_eq!(section(&d1, Types).grants, section(&d0, Types).grants);

    // Yeni nesneler varsayılanı alır; önceden var olan tablo değişmez.
    env.sql(&format!("CREATE TABLE {s}.t (a int); CREATE SEQUENCE {s}.sq; CREATE FUNCTION {s}.f() RETURNS int LANGUAGE sql AS 'SELECT 1'"))
        .await;
    let (t, before, sq) = (env.class_oid(&s, "t").await, env.class_oid(&s, "before").await, env.class_oid(&s, "sq").await);
    let f = env.proc_oid(&s, "f", "").await;
    assert!(flag(&env, "SELECT has_table_privilege($1::text, $2::oid, 'SELECT')", &[&r1, &t]).await);
    assert!(!flag(&env, "SELECT has_table_privilege($1::text, $2::oid, 'SELECT')", &[&r1, &before]).await);
    assert!(flag(&env, "SELECT has_sequence_privilege($1::text, $2::oid, 'USAGE WITH GRANT OPTION')", &[&r2, &sq]).await);
    assert!(!flag(&env, PROC_PUBLIC_SQL, &[&f]).await, "yeni fonksiyonda PUBLIC EXECUTE olmamalı");
    // Yeni tablonun nesne ACL'si de varsayılanı gösterir.
    let lt = ld(&env, ObjectKind::Table, t).await;
    assert!(lt.explicit && lt.grants.contains(&row(Some(&r1), &[Select])), "{lt:?}");

    // Boş ACL ('{}'): rol dahil herkesten tablo yetkileri kaldırılır.
    let (_, d2) = apply_def(&env, &d1, (None, None), |n| section_mut(n, Tables).grants.clear()).await;
    assert!(section(&d2, Tables).grants.is_empty());
    let raw: String = env
        .admin
        .query_one(
            "SELECT d.defaclacl::text FROM pg_default_acl d JOIN pg_roles r ON r.oid = d.defaclrole WHERE d.defaclnamespace = 0 AND d.defaclobjtype = 'r' AND r.rolname = $1::text",
            &[&me],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(raw, "{}");

    // acldefault'a dönüş: PG girdiyi siler; yükleme ilk durumla aynı.
    let (_, d3) = apply_def(&env, &d2, (None, None), |n| {
        for x in &mut n.sections {
            x.grants = section(&d0, x.object_type).grants.clone();
        }
    })
    .await;
    assert_eq!(d3, d0);
    let left: i64 = env.admin.query_one("SELECT count(*) FROM pg_default_acl", &[]).await.unwrap().get(0);
    assert_eq!(left, 0, "varsayılana dönen girdiler silinmeli");
}

// ---- şema kapsamı: yalnız eklenen yetkiler; baseline salt okunur ----

docker_test!(default_acl_schema_scope_roundtrip, def_schema_body, "dschema");

async fn def_schema_body(ts: TestServer, tag: &'static str) {
    let env = setup(&ts, tag, &["1", "2"]).await;
    let (r1, r2) = (role(tag, "1"), role(tag, "2"));
    let q2 = quote_ident(&r2);
    let s = format!("pgt_ddl_rt_{tag}");
    env.sql(&format!("CREATE SCHEMA {s}")).await;
    let soid = env.schema_oid(&s).await;
    use DefaultAclObjectType::*;
    use Privilege::*;
    let qme = quote_ident(&env.me);

    // Genel kapsamda r1 SELECT → şema kapsamının baseline'ında görünür.
    let g0 = ldd(&env, None, None).await;
    let (_, g1) = apply_def(&env, &g0, (None, None), |n| section_mut(n, Tables).grants.push(row(Some(&r1), &[Select]))).await;
    let l0 = ldd(&env, None, Some(soid)).await;
    assert_eq!(section(&l0, Tables).baseline, section(&g1, Tables).grants);
    assert!(l0.sections.iter().all(|x| x.grants.is_empty()));

    let (p, l1) = apply_def(&env, &l0, (None, Some(soid)), |n| {
        section_mut(n, Tables).grants.push(row(Some(&r2), &[Select, Insert]));
        section_mut(n, Types).grants.push(row_go(Some(&r2), &[(Usage, true)]));
    })
    .await;
    assert_eq!(
        sqls(&p),
        [
            format!("ALTER DEFAULT PRIVILEGES FOR ROLE {qme} IN SCHEMA {s} GRANT SELECT, INSERT ON TABLES TO {q2}"),
            format!("ALTER DEFAULT PRIVILEGES FOR ROLE {qme} IN SCHEMA {s} GRANT USAGE ON TYPES TO {q2} WITH GRANT OPTION"),
        ]
    );
    assert_eq!(section(&l1, Tables).grants, [row(Some(&r2), &[Select, Insert])]);
    // Genel kapsam değişmedi.
    assert_eq!(ldd(&env, None, None).await, g1);

    // baseline salt okunur.
    let mut bad = l1.clone();
    section_mut(&mut bad, Tables).baseline.clear();
    assert_eq!(paths(&default_privileges(&env.ctx(false), &l1, &bad).unwrap_err()), ["sections[0].baseline"]);

    // Şemadaki yeni tablo: genel (r1 SELECT) + şema (r2 SELECT, INSERT); başka şemadaki tablo yalnız genel.
    env.sql(&format!("CREATE TABLE {s}.t (a int); CREATE TABLE public.{s}_t (a int)")).await;
    let (t, other) = (env.class_oid(&s, "t").await, env.class_oid("public", &format!("{s}_t")).await);
    let has = "SELECT has_table_privilege($1::text, $2::oid, $3::text)";
    assert!(flag(&env, has, &[&r1, &t, &"SELECT"]).await);
    assert!(flag(&env, has, &[&r2, &t, &"INSERT"]).await);
    assert!(!flag(&env, has, &[&r2, &other, &"INSERT"]).await);
    assert!(flag(&env, has, &[&r1, &other, &"SELECT"]).await);

    // Şema girdilerini geri alma → girdiler kalkar (grants boş), baseline aynı.
    let (p, l2) = apply_def(&env, &l1, (None, Some(soid)), |n| {
        for x in &mut n.sections {
            x.grants.clear();
        }
    })
    .await;
    assert_eq!(
        sqls(&p),
        [
            format!("ALTER DEFAULT PRIVILEGES FOR ROLE {qme} IN SCHEMA {s} REVOKE SELECT, INSERT ON TABLES FROM {q2}"),
            format!("ALTER DEFAULT PRIVILEGES FOR ROLE {qme} IN SCHEMA {s} REVOKE USAGE ON TYPES FROM {q2}"),
        ]
    );
    assert_eq!(l2, l0);
}

// ---- FOR ROLE: başka rol için üyelik gerekir (42501) ----

docker_test!(default_acl_for_other_role_requires_membership, def_for_role_body, "dforrole");

async fn def_for_role_body(ts: TestServer, tag: &'static str) {
    let env = setup(&ts, tag, &["own", "other"]).await;
    let (own, other) = (role(tag, "own"), role(tag, "other"));
    let (qown, qother) = (quote_ident(&own), quote_ident(&other));
    let s = format!("pgt_ddl_rt_{tag}");
    env.sql(&format!("CREATE SCHEMA {s}; GRANT CREATE, USAGE ON SCHEMA {s} TO {qown}")).await;
    use DefaultAclObjectType::*;
    use Privilege::*;

    // Süper kullanıcı başka rol için ayarlayabilir.
    let l0 = ldd(&env, Some(&own), None).await;
    assert_eq!(section(&l0, Tables).grants, [def_owner_all(&env, &own, Tables)]);
    let (p, l1) = apply_def(&env, &l0, (Some(&own), None), |n| section_mut(n, Tables).grants.push(row(Some(&other), &[Select]))).await;
    assert_eq!(sqls(&p), [format!("ALTER DEFAULT PRIVILEGES FOR ROLE {qown} GRANT SELECT ON TABLES TO {qother}")]);

    // Rolün oluşturduğu tablo varsayılanı alır.
    env.sql(&format!("SET ROLE {qown}; CREATE TABLE {s}.t (a int); RESET ROLE")).await;
    let t = env.class_oid(&s, "t").await;
    assert!(flag(&env, "SELECT has_table_privilege($1::text, $2::oid, 'SELECT')", &[&other, &t]).await);

    // Üye olmayan (süper kullanıcı olmayan) rol: 42501, hiçbir şey değişmez.
    let mut new = l1.clone();
    section_mut(&mut new, Sequences).grants.push(row(Some(&other), &[Usage]));
    let p = default_privileges(&env.ctx(false), &l1, &new).unwrap();
    env.sql(&format!("SET ROLE {qother}")).await;
    let res = env.try_exec_tx(&p).await;
    env.sql("RESET ROLE").await;
    let err = res.expect_err("üye olmayan rol başka rol için varsayılan yetki değiştirebildi");
    assert_eq!(sqlstate(&err), "42501", "{err}");
    assert_eq!(ldd(&env, Some(&own), None).await, l1);
}

// ---- hostile adlar: rol, şema, alıcı ----

docker_test!(default_acl_hostile_names_roundtrip, def_hostile_body, "dhostile");

async fn def_hostile_body(ts: TestServer, tag: &'static str) {
    let env = setup(&ts, tag, &[r#"Ş "q" Büyük x"#, "a b'c"]).await;
    let (hr, grantee) = (role(tag, r#"Ş "q" Büyük x"#), role(tag, "a b'c"));
    let qhr = quote_ident(&hr);
    let s = format!(r#"pgt_ddl_rt_{tag} "Ş" X"#);
    let qs = quote_ident(&s);
    env.sql(&format!("CREATE SCHEMA {qs}; GRANT CREATE, USAGE ON SCHEMA {qs} TO {qhr}")).await;
    let soid = env.schema_oid(&s).await;
    use DefaultAclObjectType::*;
    use Privilege::*;

    for schema in [None, Some(soid)] {
        let l = ldd(&env, Some(&hr), schema).await;
        assert_eq!((l.role.as_str(), l.schema.as_deref()), (hr.as_str(), schema.map(|_| s.as_str())));
        let (p, _) = apply_def(&env, &l, (Some(&hr), schema), |n| {
            section_mut(n, Tables).grants.push(row_go(Some(&grantee), &[(Select, true), (Delete, false)]));
            if schema.is_none() {
                section_mut(n, Functions).grants.retain(|g| g.grantee.is_some());
            }
        })
        .await;
        assert!(p.statements.iter().all(|st| st.sql.contains(&qhr)), "{:?}", sqls(&p));
    }
    // Hostile rolün hostile şemada oluşturduğu tablo ve fonksiyon her iki kapsamdan da yetki alır.
    env.sql(&format!(r#"SET ROLE {qhr}; CREATE TABLE {qs}."T ""x""" (a int); CREATE FUNCTION {qs}.f() RETURNS int LANGUAGE sql AS 'SELECT 1'; RESET ROLE"#))
        .await;
    let t = env.class_oid(&s, r#"T "x""#).await;
    assert!(flag(&env, "SELECT has_table_privilege($1::text, $2::oid, 'SELECT WITH GRANT OPTION, DELETE')", &[&grantee, &t]).await);
    assert!(!flag(&env, PROC_PUBLIC_SQL, &[&env.proc_oid(&s, "f", "").await]).await);
}

// ---- PG 18 large object varsayılanları: bilgi notu, düzenlenebilir ----

docker_test!(default_acl_large_objects_note_pg18, def_large_body, "dlarge");

async fn def_large_body(ts: TestServer, tag: &'static str) {
    let env = setup(&ts, tag, &["1"]).await;
    let major = env.version / 10000;
    let r1 = role(tag, "1");
    let s = format!("pgt_ddl_rt_{tag}");
    env.sql(&format!("CREATE SCHEMA {s}")).await;
    if major < 18 {
        eprintln!("PG {major}: LARGE OBJECTS varsayılan yetkileri atlandı (PG 18+)");
        return;
    }
    env.sql(&format!("ALTER DEFAULT PRIVILEGES GRANT SELECT ON LARGE OBJECTS TO {}", quote_ident(&r1))).await;
    let d = ldd(&env, None, None).await;
    assert_eq!(d.unsupported, [LARGE_OBJECTS_REASON]);
    assert!(d.editable, "{d:?}");
    assert_def_fixed(&env, &d);
    // Diğer bölümler düzenlenebilir; large object girdisine dokunulmaz.
    let (_, d2) = apply_def(&env, &d, (None, None), |n| {
        section_mut(n, DefaultAclObjectType::Types).grants.push(row(Some(&r1), &[Privilege::Usage]));
    })
    .await;
    assert_eq!(d2.unsupported, [LARGE_OBJECTS_REASON]);
    let lo: i64 = env.admin.query_one("SELECT count(*) FROM pg_default_acl WHERE defaclobjtype = 'L'", &[]).await.unwrap().get(0);
    assert_eq!(lo, 1);
    // Şema kapsamında genel large object girdisi görünmez.
    let ds = ldd(&env, None, Some(env.schema_oid(&s).await)).await;
    assert!(ds.unsupported.is_empty() && ds.editable, "{ds:?}");
}
