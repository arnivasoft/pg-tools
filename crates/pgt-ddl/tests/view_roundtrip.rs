//! View / Materialized View üreticisi → gerçek sunucu → yükleyici gidiş-dönüş testleri (Docker; `PGT_TEST_URLS` yoksa
//! atlanır).
//!
//! Her test kendi geçici veritabanını (`pgt_ddl_vw_<ana sürüm>_<etiket>`) ve rolünü kurar; başarısız doğrulamadan sonra
//! bile temizlenir. Üreticinin SQL'i `DdlManager` olmadan, uygulayıcı gibi tek transaction'da `admin_client` ile
//! çalıştırılır (`Env::exec_tx`). Sürüme bağlı kurallar `server_version`'a göre dallanır; atlanan kısım yazdırılır.
//!
//! Sorgu metni `pg_get_viewdef` çıktısıdır (sürüme göre biçim farkı olabilir): beklenen tanımın `query`'si yüklenenden
//! alınır ve **sabit nokta** ayrıca sınanır — yüklü sorguyla yeniden oluşturulan nesnenin yüklü sorgusu aynıdır.

mod common;

use common::{Env, cleanup_in, guarded, servers, setup_in};
use pgt_core::Error;
use pgt_core::testing::{TestServer, admin_client};
use pgt_ddl::build;
use pgt_ddl::*;

const PFX: &str = "pgt_ddl_vw";

// ---- tanım kurucuları ----

fn view(schema: &str, name: &str, query: &str) -> ViewDef {
    ViewDef {
        schema: schema.into(),
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

fn matview(schema: &str, name: &str, query: &str) -> MatViewDef {
    MatViewDef {
        schema: schema.into(),
        name: name.into(),
        owner: None,
        query: query.into(),
        tablespace: None,
        fillfactor: None,
        with_data: true,
        comment: None,
        refresh: None,
        columns: vec![],
        unsupported: vec![],
    }
}

fn ov(d: &ViewDef) -> ObjectDef {
    ObjectDef::View(d.clone())
}

fn om(d: &MatViewDef) -> ObjectDef {
    ObjectDef::MaterializedView(d.clone())
}

fn sqls(p: &Plan) -> Vec<&str> {
    p.statements.iter().map(|s| s.sql.as_str()).collect()
}

fn col_names(c: &[ViewColumn]) -> Vec<&str> {
    c.iter().map(|c| c.name.as_str()).collect()
}

/// Temel tablo: her test kendi şemasında.
async fn base_table(env: &Env, schema_sql: &str) {
    env.sql(&format!("CREATE SCHEMA {schema_sql}")).await;
    env.sql(&format!(
        "CREATE TABLE {schema_sql}.t (id int PRIMARY KEY, a text, b int); \
         INSERT INTO {schema_sql}.t VALUES (1, 'x', 1), (2, 'y', 2), (3, 'z', 0)"
    ))
    .await;
}

// ---- yükle / karşılaştır ----

async fn try_load(env: &Env, kind: ObjectKind, oid: u32) -> pgt_core::Result<ObjectDef> {
    load(&env.mgr, env.server, env.db_oid, ObjectRef { kind, oid }).await
}

async fn vload(env: &Env, schema: &str, name: &str) -> ViewDef {
    let oid = env.class_oid(schema, name).await;
    match try_load(env, ObjectKind::View, oid).await {
        Ok(ObjectDef::View(d)) => d,
        other => panic!("{schema}.{name} yüklenemedi: {other:?}"),
    }
}

async fn mload(env: &Env, schema: &str, name: &str) -> MatViewDef {
    let oid = env.class_oid(schema, name).await;
    match try_load(env, ObjectKind::MaterializedView, oid).await {
        Ok(ObjectDef::MaterializedView(d)) => d,
        other => panic!("{schema}.{name} yüklenemedi: {other:?}"),
    }
}

/// Beklenen görünüm: belirtilmeyen sahip yüklenenden, boş yorum `None`, sorgu ve kolonlar yüklenenden, `recreate` yok.
fn vexpected(def: &ViewDef, loaded: &ViewDef) -> ViewDef {
    ViewDef {
        owner: def.owner.clone().or_else(|| loaded.owner.clone()),
        comment: def.comment.clone().filter(|c| !c.is_empty()),
        query: loaded.query.clone(),
        columns: loaded.columns.clone(),
        recreate: false,
        ..def.clone()
    }
}

/// Beklenen MV: View gibi + `pg_default` → `None`, `refresh` yok.
fn mexpected(def: &MatViewDef, loaded: &MatViewDef) -> MatViewDef {
    MatViewDef {
        owner: def.owner.clone().or_else(|| loaded.owner.clone()),
        comment: def.comment.clone().filter(|c| !c.is_empty()),
        tablespace: def.tablespace.clone().filter(|t| t != "pg_default"),
        query: loaded.query.clone(),
        columns: loaded.columns.clone(),
        refresh: None,
        ..def.clone()
    }
}

fn assert_vnoop(env: &Env, d: &ViewDef) {
    let p = build::alter(&ov(d), &ov(d), &env.ctx(false)).unwrap_or_else(|e| panic!("noop reddedildi: {e:?}\n{d:?}"));
    assert!(p.statements.is_empty() && p.warnings.is_empty(), "{}: boş plan bekleniyordu: {:?} {:?}", d.name, sqls(&p), p.warnings);
}

fn assert_mnoop(env: &Env, d: &MatViewDef) {
    let p = build::alter(&om(d), &om(d), &env.ctx(false)).unwrap_or_else(|e| panic!("noop reddedildi: {e:?}\n{d:?}"));
    assert!(p.statements.is_empty() && p.warnings.is_empty(), "{}: boş plan bekleniyordu: {:?} {:?}", d.name, sqls(&p), p.warnings);
}

/// Sabit nokta: yüklü sorguyla (aynı seçeneklerle) `<ad>_fp` oluşturulur; yüklü sorgusu ve kolonları aynıdır; silinir.
async fn view_fixed_point(env: &Env, loaded: &ViewDef) {
    let fp = ViewDef { name: format!("{}_fp", loaded.name), owner: None, comment: None, unsupported: vec![], ..loaded.clone() };
    let plan = build::create(&ov(&fp), &env.ctx(false)).unwrap_or_else(|e| panic!("yüklü sorgu create'te reddedildi: {e:?}\n{:?}", loaded.query));
    env.exec_tx(&plan).await;
    let l2 = vload(env, &fp.schema, &fp.name).await;
    assert_eq!(l2.query, loaded.query, "sabit nokta (sorgu)");
    assert_eq!(l2.columns, loaded.columns, "sabit nokta (kolonlar)");
    env.exec_tx(&build::drop(&ov(&l2), false, &env.ctx(false)).unwrap()).await;
}

async fn matview_fixed_point(env: &Env, loaded: &MatViewDef) {
    let fp = MatViewDef {
        name: format!("{}_fp", loaded.name),
        owner: None,
        comment: None,
        with_data: false,
        unsupported: vec![],
        ..loaded.clone()
    };
    let plan = build::create(&om(&fp), &env.ctx(false)).unwrap_or_else(|e| panic!("yüklü sorgu create'te reddedildi: {e:?}\n{:?}", loaded.query));
    env.exec_tx(&plan).await;
    let l2 = mload(env, &fp.schema, &fp.name).await;
    assert_eq!(l2.query, loaded.query, "sabit nokta (sorgu)");
    assert_eq!(l2.columns, loaded.columns, "sabit nokta (kolonlar)");
    env.exec_tx(&build::drop(&om(&l2), false, &env.ctx(false)).unwrap()).await;
}

/// create → tek transaction → yükle → beklenen tanıma eşit + noop + iki yükleme aynı + sabit nokta.
async fn vcreate_and_check(env: &Env, def: &ViewDef) -> ViewDef {
    let plan = build::create(&ov(def), &env.ctx(false)).unwrap_or_else(|e| panic!("create reddedildi: {} {e:?}", def.name));
    env.exec_tx(&plan).await;
    let loaded = vload(env, &def.schema, &def.name).await;
    assert_eq!(loaded, vexpected(def, &loaded), "create → load ({})", def.name);
    assert!(!loaded.columns.is_empty() && loaded.owner.is_some());
    assert!(!loaded.query.ends_with(';') && loaded.query.trim() == loaded.query, "{:?}", loaded.query);
    assert_vnoop(env, &loaded);
    assert_eq!(vload(env, &def.schema, &def.name).await, loaded, "iki yükleme aynı olmalı");
    view_fixed_point(env, &loaded).await;
    loaded
}

async fn mcreate_and_check(env: &Env, def: &MatViewDef) -> MatViewDef {
    let plan = build::create(&om(def), &env.ctx(false)).unwrap_or_else(|e| panic!("create reddedildi: {} {e:?}", def.name));
    env.exec_tx(&plan).await;
    let loaded = mload(env, &def.schema, &def.name).await;
    assert_eq!(loaded, mexpected(def, &loaded), "create → load ({})", def.name);
    assert!(!loaded.columns.is_empty() && loaded.owner.is_some() && loaded.refresh.is_none());
    assert_mnoop(env, &loaded);
    assert_eq!(mload(env, &def.schema, &def.name).await, loaded, "iki yükleme aynı olmalı");
    matview_fixed_point(env, &loaded).await;
    loaded
}

fn valter(env: &Env, old: &ViewDef, new: &ViewDef) -> Plan {
    build::alter(&ov(old), &ov(new), &env.ctx(false)).unwrap_or_else(|e| panic!("alter reddedildi: {e:?}\n  old: {old:?}\n  new: {new:?}"))
}

fn malter(env: &Env, old: &MatViewDef, new: &MatViewDef) -> Plan {
    build::alter(&om(old), &om(new), &env.ctx(false)).unwrap_or_else(|e| panic!("alter reddedildi: {e:?}\n  old: {old:?}\n  new: {new:?}"))
}

/// alter(old, new) → tek transaction → yükle → beklenen yeni tanıma eşit + noop. `new.unsupported` beklenen listedir.
async fn valter_and_check(env: &Env, old: &ViewDef, new: &ViewDef) -> ViewDef {
    let plan = valter(env, old, new);
    if let Err(e) = env.try_exec_tx(&plan).await {
        panic!("alter uygulanamadı: {e}\n  plan: {:?}", sqls(&plan));
    }
    let loaded = vload(env, &new.schema, &new.name).await;
    assert_eq!(loaded, vexpected(new, &loaded), "alter → load; plan: {:?}", sqls(&plan));
    assert_vnoop(env, &loaded);
    loaded
}

async fn malter_and_check(env: &Env, old: &MatViewDef, new: &MatViewDef) -> MatViewDef {
    let plan = malter(env, old, new);
    if let Err(e) = env.try_exec_tx(&plan).await {
        panic!("alter uygulanamadı: {e}\n  plan: {:?}", sqls(&plan));
    }
    let loaded = mload(env, &new.schema, &new.name).await;
    assert_eq!(loaded, mexpected(new, &loaded), "alter → load; plan: {:?}", sqls(&plan));
    assert_mnoop(env, &loaded);
    loaded
}

async fn can_select(env: &Env, role: &str, rel: &str) -> bool {
    env.admin.query_one("SELECT has_table_privilege($1::text, $2::text, 'SELECT')", &[&role, &rel]).await.unwrap().get(0)
}

async fn count(env: &Env, sql: &str) -> i64 {
    env.admin.query_one(sql, &[]).await.unwrap().get(0)
}

/// Sunucu iletisindeki SQLSTATE (`db_message` biçimi: "ileti (KOD)").
fn sqlstate(msg: &str) -> &str {
    msg.split_once('\n').map_or(msg, |(first, _)| first).rsplit_once('(').map_or("", |(_, c)| c.trim_end_matches(')'))
}

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

// ================================================================================================================
// View
// ================================================================================================================

docker_test!(view_create_then_load_matches, vcreate_body, "vcreate");

async fn vcreate_body(ts: TestServer) {
    let env = setup_in(&ts, PFX, "vcreate").await;
    let major = env.version / 10000;
    base_table(&env, r#""Vw S""#).await;
    let q = r#"SELECT id, a FROM "Vw S".t WHERE b > 0"#;
    let mut defs = vec![
        view("Vw S", "plain", q),
        ViewDef { check_option: Some(CheckOption::Local), ..view("Vw S", "chk local", q) },
        ViewDef { check_option: Some(CheckOption::Cascaded), ..view("Vw S", "chk_casc", q) },
        ViewDef { security_barrier: true, ..view("Vw S", "barrier", q) },
        ViewDef {
            owner: Some(env.role.clone()),
            comment: Some("yorum; -- 'x' /* y */ \\".into()),
            check_option: Some(CheckOption::Local),
            security_barrier: true,
            ..view("Vw S", "Owned", q)
        },
    ];
    if env.version >= 150000 {
        defs.push(ViewDef { security_invoker: true, ..view("Vw S", "invoker", q) });
        defs.push(ViewDef {
            security_invoker: true,
            security_barrier: true,
            check_option: Some(CheckOption::Cascaded),
            ..view("Vw S", "all_opts", q)
        });
    } else {
        eprintln!("PG {major}: security_invoker atlandı (PG 15+)");
        // Sürüm kapısı yerel hatadır (sunucuya gitmez).
        let d = ViewDef { security_invoker: true, ..view("Vw S", "invoker", q) };
        assert_eq!(build::create(&ov(&d), &env.ctx(false)).unwrap_err()[0].path, "securityInvoker");
    }
    for d in &defs {
        let l = vcreate_and_check(&env, d).await;
        assert_eq!(col_names(&l.columns), ["id", "a"]);
        assert_eq!(l.columns[0].data_type, "integer");
        assert!(l.unsupported.is_empty(), "{:?}", l.unsupported);
    }
    // check_option tırnaksız (`= local`) tüm sürümlerde kabul edildi; sunucu kuralı gerçekten etkin.
    let e = env.try_sql(r#"INSERT INTO "Vw S"."chk local" VALUES (9, 'q')"#).await;
    assert!(e.is_err_and(|m| m.contains("44000")), "WITH CHECK OPTION ihlali beklenirdi");
    eprintln!("PG {major}: WITH (check_option = local|cascaded) tırnaksız kabul edildi");
}

docker_test!(view_unchanged_edit_is_empty_plan, vnoop_body, "vnoop");

async fn vnoop_body(ts: TestServer) {
    let env = setup_in(&ts, PFX, "vnoop").await;
    base_table(&env, "vw").await;
    let queries = [
        "SELECT id, a FROM vw.t WHERE b > 0",
        "SELECT id, a FROM vw.t WHERE b > 0 ORDER BY id DESC NULLS LAST LIMIT 5 OFFSET 1",
        "SELECT * FROM vw.t",
        "WITH x AS (SELECT id, b FROM vw.t) SELECT id FROM x WHERE b IN (1, 2)",
        "SELECT 1 AS one UNION ALL SELECT 2",
        "VALUES (1, 'x'), (2, 'y')",
        "SELECT t.id, u.id AS uid FROM vw.t t LEFT JOIN vw.t u ON u.id = t.b",
        "SELECT CASE WHEN b > 1 THEN 'big' ELSE 'small' END AS size, count(*) OVER (PARTITION BY a) AS n FROM vw.t",
        "SELECT a || ';' || '--' AS s, $$x$$::text AS d, 'it''s' AS q FROM vw.t",
        "SELECT id::text AS s, '2020-01-01'::date + 1 AS d, ARRAY[1, 2] AS arr, b::numeric(10,2) AS n FROM vw.t",
        "SELECT a, sum(b) AS total FROM vw.t GROUP BY a HAVING sum(b) > 0",
        "SELECT id FROM vw.t WHERE a LIKE 'z%' AND b BETWEEN 1 AND 3 AND EXISTS (SELECT 1 FROM vw.t x WHERE x.id = t.b)",
        "SELECT 1 AS x -- yorum\n",
        "SELECT lower(a) AS l, coalesce(b, 0) AS c, now() - interval '1 day' AS y FROM vw.t",
        "(SELECT id FROM vw.t) EXCEPT (SELECT b FROM vw.t)",
    ];
    for (i, q) in queries.iter().enumerate() {
        let mut d = view("vw", &format!("v{i}"), q);
        d.comment = (i % 2 == 0).then(|| format!("c{i}"));
        if i == 0 {
            d.check_option = Some(CheckOption::Cascaded);
            d.security_barrier = true;
        }
        if i == 1 && env.version >= 150000 {
            d.security_invoker = true;
        }
        let l = vcreate_and_check(&env, &d).await;
        assert!(l.unsupported.is_empty());
    }
}

docker_test!(view_replace_keeps_grants_and_options_follow_new_def, vreplace_body, "vreplace");

async fn vreplace_body(ts: TestServer) {
    let env = setup_in(&ts, PFX, "vreplace").await;
    base_table(&env, "vw").await;
    let old = vcreate_and_check(
        &env,
        &ViewDef {
            check_option: Some(CheckOption::Local),
            security_barrier: true,
            comment: Some("k".into()),
            ..view("vw", "v", "SELECT id, a FROM vw.t")
        },
    )
    .await;
    env.sql(&format!("GRANT SELECT ON vw.v TO {}", env.role)).await;
    assert!(can_select(&env, &env.role, "vw.v").await);
    // Sona kolon ekle + seçenek değişimi (check_option cascaded, security_barrier true → false) → tek C-O-R.
    let new = ViewDef {
        query: "SELECT id, a, b FROM vw.t".into(),
        check_option: Some(CheckOption::Cascaded),
        security_barrier: false,
        ..old.clone()
    };
    let plan = valter(&env, &old, &new);
    assert_eq!(plan.statements.len(), 1);
    assert!(plan.statements[0].sql.starts_with("CREATE OR REPLACE VIEW vw.v WITH (check_option = cascaded) AS ("));
    let l = valter_and_check(&env, &old, &new).await;
    assert_eq!(col_names(&l.columns), ["id", "a", "b"]);
    assert!(!l.security_barrier, "C-O-R seçenek listesini tümüyle değiştirmeli");
    assert!(can_select(&env, &env.role, "vw.v").await, "C-O-R GRANT'i korumalı");
    // Seçeneklerin tamamı kalkar (boş liste): WITH yazılmaz, reloptions boşalır.
    let bare = ViewDef { query: "SELECT id, a, b, b + 1 AS c FROM vw.t".into(), check_option: None, ..l.clone() };
    let l = valter_and_check(&env, &l, &bare).await;
    assert_eq!((l.check_option, l.security_barrier), (None, false));
    view_fixed_point(&env, &l).await;
}

docker_test!(view_incompatible_replace_fails_and_recreate_succeeds, vincompat_body, "vincompat");

async fn vincompat_body(ts: TestServer) {
    let env = setup_in(&ts, PFX, "vincompat").await;
    let major = env.version / 10000;
    base_table(&env, "vw").await;
    let old = vcreate_and_check(&env, &ViewDef { comment: Some("k".into()), ..view("vw", "v", "SELECT id, a FROM vw.t") }).await;
    env.sql(&format!("GRANT SELECT ON vw.v TO {}", env.role)).await;
    // Kolon silme, tip değişimi, ad değişimi: C-O-R sunucuda reddedilir, transaction geri alınır.
    for (what, q) in [
        ("kolon silme", "SELECT id FROM vw.t"),
        ("tip değişimi", "SELECT id, b AS a FROM vw.t"),
        ("ad değişimi", "SELECT id, a AS z FROM vw.t"),
    ] {
        let new = ViewDef { query: q.into(), ..old.clone() };
        let plan = valter(&env, &old, &new);
        let e = env.try_exec_tx(&plan).await.expect_err("uyumsuz C-O-R reddedilmeliydi");
        let code = sqlstate(&e);
        eprintln!("PG {major}: uyumsuz CREATE OR REPLACE VIEW ({what}) → SQLSTATE {code}");
        assert!(code == "42P16" || code == "42804", "{what}: {e}");
        assert_eq!(vload(&env, "vw", "v").await, old, "görünüm değişmemiş olmalı");
    }
    // Açık recreate: DROP + CREATE + sahip + yorum; kolonlar farklı; GRANT kaybolur.
    let new = ViewDef { query: "SELECT id FROM vw.t".into(), recreate: true, ..old.clone() };
    let plan = valter(&env, &old, &new);
    assert_eq!(plan.risk(), Risk::DataLoss);
    let l = valter_and_check(&env, &old, &new).await;
    assert_eq!(col_names(&l.columns), ["id"]);
    assert_eq!(l.comment.as_deref(), Some("k"));
    assert!(!can_select(&env, &env.role, "vw.v").await, "recreate GRANT'i kaybettirir");
}

docker_test!(view_recreate_with_dependent_fails_cleanly, vdep_body, "vdep");

async fn vdep_body(ts: TestServer) {
    let env = setup_in(&ts, PFX, "vdep").await;
    let major = env.version / 10000;
    base_table(&env, "vw").await;
    let v1 = vcreate_and_check(&env, &view("vw", "v1", "SELECT id, a FROM vw.t")).await;
    let v2 = vcreate_and_check(&env, &view("vw", "v2", "SELECT id FROM vw.v1")).await;
    let new = ViewDef { query: "SELECT id FROM vw.t".into(), recreate: true, ..v1.clone() };
    let e = env.try_exec_tx(&valter(&env, &v1, &new)).await.expect_err("bağımlı görünüm varken DROP reddedilmeli");
    eprintln!("PG {major}: bağımlılı recreate → SQLSTATE {}", sqlstate(&e));
    assert_eq!(sqlstate(&e), "2BP01", "{e}");
    assert_eq!(vload(&env, "vw", "v1").await, v1);
    assert_eq!(vload(&env, "vw", "v2").await, v2);
}

docker_test!(view_rename_schema_owner_comment_combined, vcombo_body, "vcombo");

async fn vcombo_body(ts: TestServer) {
    let env = setup_in(&ts, PFX, "vcombo").await;
    base_table(&env, "vw").await;
    env.sql(r#"CREATE SCHEMA "Vw 2""#).await;
    let old = vcreate_and_check(&env, &ViewDef { comment: Some("eski".into()), ..view("vw", "v", "SELECT id, a FROM vw.t") }).await;
    // Ad + şema + sahip + yorum + seçenek (sorgu aynı → ALTER VIEW SET).
    let new = ViewDef {
        schema: "Vw 2".into(),
        name: "Yeni V".into(),
        owner: Some(env.role.clone()),
        comment: Some("yeni 'yorum'".into()),
        security_barrier: true,
        ..old.clone()
    };
    let l = valter_and_check(&env, &old, &new).await;
    // Ad + şema + sorgu + sahip geri + yorum kaldırma (C-O-R yeni ad/şemada).
    let back = ViewDef {
        schema: "vw".into(),
        name: "select".into(),
        owner: Some(env.me.clone()),
        comment: None,
        query: "SELECT id, a, b FROM vw.t".into(),
        ..l.clone()
    };
    let plan = valter(&env, &l, &back);
    assert_eq!(plan.statements.len(), 5, "{:?}", sqls(&plan));
    let l = valter_and_check(&env, &l, &back).await;
    assert_eq!((l.security_barrier, l.comment.as_deref()), (true, None));
    view_fixed_point(&env, &l).await;
}

docker_test!(view_unsupported_lists_trigger_rule_default_comment, vunsup_body, "vunsup");

async fn vunsup_body(ts: TestServer) {
    let env = setup_in(&ts, PFX, "vunsup").await;
    base_table(&env, "vw").await;
    vcreate_and_check(&env, &view("vw", "v", "SELECT id, a FROM vw.t")).await;
    // PG 10: EXECUTE FUNCTION yok (PG 11+); EXECUTE PROCEDURE tüm sürümlerde geçerli.
    env.sql(
        "CREATE FUNCTION vw.trg_fn() RETURNS trigger LANGUAGE plpgsql AS $$BEGIN RETURN NEW; END$$; \
         CREATE TRIGGER trg_ins INSTEAD OF INSERT ON vw.v FOR EACH ROW EXECUTE PROCEDURE vw.trg_fn(); \
         CREATE RULE r_upd AS ON UPDATE TO vw.v DO INSTEAD NOTHING; \
         ALTER VIEW vw.v ALTER COLUMN a SET DEFAULT 'd'; \
         COMMENT ON COLUMN vw.v.id IS 'kolon yorumu'",
    )
    .await;
    let l = vload(&env, "vw", "v").await;
    let want = ["TRIGGER trg_ins", "RULE r_upd", "DEFAULT a", "COMMENT id"];
    assert_eq!(l.unsupported, want);
    assert_vnoop(&env, &l);
    // Sorguya dokunmayan başka bir düzenleme (yorum) ekleri korur.
    let new = ViewDef { comment: Some("k".into()), unsupported: want.iter().map(|s| s.to_string()).collect(), ..l.clone() };
    let l = valter_and_check(&env, &l, &new).await;
    assert_eq!(l.unsupported, want);
    // Yeniden kurma uyarısı kaybolacakları listeler.
    let r = ViewDef { query: "SELECT id, a, b FROM vw.t".into(), recreate: true, ..l.clone() };
    let p = valter(&env, &l, &r);
    assert!(p.warnings.iter().any(|w| w.contains("TRIGGER trg_ins") && w.contains("COMMENT id")), "{:?}", p.warnings);
}

docker_test!(view_body_with_backslash_literal_is_still_renamable, vbs_body, "vbs");

async fn vbs_body(ts: TestServer) {
    let env = setup_in(&ts, PFX, "vbs").await;
    env.sql("CREATE SCHEMA vw").await;
    env.sql(r"CREATE VIEW vw.bs AS SELECT E'a\\b' AS x").await;
    let l = vload(&env, "vw", "bs").await;
    // standard_conforming_strings = on: pg_get_viewdef sıradan '…' yazar.
    assert!(l.query.contains(r"'a\b'"), "{:?}", l.query);
    assert_vnoop(&env, &l);
    let renamed = ViewDef { name: "bs2".into(), comment: Some("k".into()), ..l.clone() };
    let l2 = valter_and_check(&env, &l, &renamed).await;
    assert_eq!(l2.query, l.query);
    let changed = ViewDef { query: format!("{}, 1 AS y", l2.query), ..l2.clone() };
    let e = build::alter(&ov(&l2), &ov(&changed), &env.ctx(false)).unwrap_err();
    assert_eq!(e[0].path, "query");
    assert!(e[0].message.contains("E'"), "{e:?}");
}

// ================================================================================================================
// Materialized View
// ================================================================================================================

docker_test!(matview_create_with_and_without_data, mcreate_body, "mcreate");

async fn mcreate_body(ts: TestServer) {
    let env = setup_in(&ts, PFX, "mcreate").await;
    base_table(&env, r#""Mv S""#).await;
    let q = r#"SELECT id, a, b * 2 AS b2 FROM "Mv S".t WHERE b > 0"#;
    let full = MatViewDef {
        owner: Some(env.role.clone()),
        comment: Some("yorum 'x'".into()),
        fillfactor: Some(70),
        tablespace: Some("pg_default".into()),
        ..matview("Mv S", "Full MV", q)
    };
    let l = mcreate_and_check(&env, &full).await;
    assert_eq!((l.with_data, l.fillfactor, l.tablespace.clone()), (true, Some(70), None));
    assert_eq!(col_names(&l.columns), ["id", "a", "b2"]);
    assert!(l.unsupported.is_empty(), "{:?}", l.unsupported);
    assert_eq!(count(&env, r#"SELECT count(*) FROM "Mv S"."Full MV""#).await, 2);
    let empty = mcreate_and_check(&env, &MatViewDef { with_data: false, ..matview("Mv S", "empty", q) }).await;
    assert!(!empty.with_data);
    let e = env.try_sql(r#"SELECT * FROM "Mv S".empty"#).await.unwrap_err();
    assert_eq!(sqlstate(&e), "55000", "{e}");
    // Fillfactor dışı depolama seçeneği ve kolon yorumu: desteklenmeyen listesinde; noop.
    env.sql(r#"ALTER MATERIALIZED VIEW "Mv S".empty SET (autovacuum_enabled = false); COMMENT ON COLUMN "Mv S".empty.a IS 'c'"#).await;
    let l = mload(&env, "Mv S", "empty").await;
    assert_eq!(l.unsupported, ["WITH (autovacuum_enabled=false)", "COMMENT a"]);
    assert_mnoop(&env, &l);
}

docker_test!(matview_refresh_transitions, mrefresh_body, "mrefresh");

async fn mrefresh_body(ts: TestServer) {
    let env = setup_in(&ts, PFX, "mrefresh").await;
    let major = env.version / 10000;
    base_table(&env, "vw").await;
    let l = mcreate_and_check(&env, &matview("vw", "mv", "SELECT id, a FROM vw.t")).await;
    // true → false: REFRESH … WITH NO DATA.
    let l = malter_and_check(&env, &l, &MatViewDef { with_data: false, ..l.clone() }).await;
    assert!(!l.with_data);
    // false → true: REFRESH.
    env.sql("INSERT INTO vw.t VALUES (4, 'w', 5)").await;
    let l = malter_and_check(&env, &l, &MatViewDef { with_data: true, ..l.clone() }).await;
    assert!(l.with_data);
    assert_eq!(count(&env, "SELECT count(*) FROM vw.mv").await, 4);
    // true → true normal.
    env.sql("INSERT INTO vw.t VALUES (5, 'v', 6)").await;
    let l = malter_and_check(&env, &l, &MatViewDef { refresh: Some(MatViewRefresh::Normal), ..l.clone() }).await;
    assert_eq!(count(&env, "SELECT count(*) FROM vw.mv").await, 5);
    // CONCURRENTLY: UNIQUE indeks yokken 55000; varken transaction içinde başarı.
    let conc = MatViewDef { refresh: Some(MatViewRefresh::Concurrently), ..l.clone() };
    let plan = malter(&env, &l, &conc);
    assert_eq!(sqls(&plan), ["REFRESH MATERIALIZED VIEW CONCURRENTLY vw.mv"]);
    assert!(plan.statements[0].transactional);
    let e = env.try_exec_tx(&plan).await.expect_err("UNIQUE indeks yokken CONCURRENTLY reddedilmeli");
    eprintln!("PG {major}: indekssiz REFRESH CONCURRENTLY → SQLSTATE {}", sqlstate(&e));
    assert_eq!(sqlstate(&e), "55000", "{e}");
    env.sql("CREATE UNIQUE INDEX mv_uq ON vw.mv (id)").await;
    let l = mload(&env, "vw", "mv").await;
    assert_eq!(l.unsupported, ["INDEX mv_uq"]);
    env.sql("INSERT INTO vw.t VALUES (6, 'u', 7)").await;
    let conc = MatViewDef { refresh: Some(MatViewRefresh::Concurrently), ..l.clone() };
    let l = malter_and_check(&env, &l, &conc).await;
    assert_eq!(count(&env, "SELECT count(*) FROM vw.mv").await, 6);
    assert_eq!(l.unsupported, ["INDEX mv_uq"]);
    eprintln!("PG {major}: REFRESH MATERIALIZED VIEW CONCURRENTLY uygulama transaction'ında çalıştı");
}

docker_test!(matview_redefine_drops_indexes_and_data, mredef_body, "mredef");

async fn mredef_body(ts: TestServer) {
    let env = setup_in(&ts, PFX, "mredef").await;
    base_table(&env, "vw").await;
    mcreate_and_check(
        &env,
        &MatViewDef { comment: Some("k".into()), fillfactor: Some(80), ..matview("vw", "mv", "SELECT id, a FROM vw.t") },
    )
    .await;
    env.sql(&format!("CREATE INDEX mv_a ON vw.mv (a); CREATE UNIQUE INDEX mv_id ON vw.mv (id); GRANT SELECT ON vw.mv TO {}", env.role)).await;
    let l = mload(&env, "vw", "mv").await;
    assert_eq!(l.unsupported, ["INDEX mv_a", "INDEX mv_id"]);
    let new = MatViewDef {
        query: "SELECT id, a, b FROM vw.t WHERE b > 0".into(),
        refresh: Some(MatViewRefresh::Normal),
        unsupported: vec![],
        ..l.clone()
    };
    let plan = malter(&env, &l, &new);
    assert_eq!(plan.risk(), Risk::DataLoss);
    assert!(plan.warnings.iter().any(|w| w == "İndeksler yeniden oluşturulmaz: mv_a, mv_id"), "{:?}", plan.warnings);
    let l2 = malter_and_check(&env, &l, &new).await;
    assert!(l2.unsupported.is_empty());
    assert_eq!(col_names(&l2.columns), ["id", "a", "b"]);
    assert_eq!((l2.fillfactor, l2.comment.as_deref(), l2.with_data), (Some(80), Some("k"), true));
    assert_eq!(count(&env, "SELECT count(*) FROM pg_index WHERE indrelid = 'vw.mv'::regclass").await, 0);
    assert_eq!(count(&env, "SELECT count(*) FROM vw.mv").await, 2);
    assert!(!can_select(&env, &env.role, "vw.mv").await, "yeniden kurma GRANT'i kaybettirir");
    // Veri olmadan yeniden kurma.
    let l3 = malter_and_check(&env, &l2, &MatViewDef { query: "SELECT id FROM vw.t".into(), with_data: false, ..l2.clone() }).await;
    assert!(!l3.with_data);
    matview_fixed_point(&env, &l3).await;
}

docker_test!(matview_alter_tablespace_fillfactor_owner_comment_rename_schema, malter_body, "malter");

const TS_NAME: &str = "pgt_ddl_vw_ts";

async fn malter_body(ts: TestServer) {
    let env = setup_in(&ts, PFX, "malter").await;
    let major = env.version / 10000;
    base_table(&env, "vw").await;
    env.sql(r#"CREATE SCHEMA "Mv 2""#).await;
    let l = mcreate_and_check(&env, &matview("vw", "mv", "SELECT id, a FROM vw.t")).await;
    // Ad + şema + fillfactor + sahip + yorum.
    let new = MatViewDef {
        schema: "Mv 2".into(),
        name: "Yeni MV".into(),
        fillfactor: Some(60),
        owner: Some(env.role.clone()),
        comment: Some("y".into()),
        ..l.clone()
    };
    let l = malter_and_check(&env, &l, &new).await;
    // fillfactor RESET, yorum kaldırma, sahip geri.
    let l = malter_and_check(&env, &l, &MatViewDef { fillfactor: None, comment: None, owner: Some(env.me.clone()), ..l.clone() }).await;
    assert_eq!((l.fillfactor, l.comment.clone()), (None, None));
    // Gerçek bir tablespace (sunucu dosya sistemine COPY TO PROGRAM ile dizin açılabiliyorsa).
    let _ = env.try_sql(&format!("DROP TABLESPACE IF EXISTS {TS_NAME}")).await;
    let made = env.try_sql(&format!("COPY (SELECT 1) TO PROGRAM 'mkdir -p /tmp/{TS_NAME}'")).await.is_ok()
        && env.try_sql(&format!("CREATE TABLESPACE {TS_NAME} LOCATION '/tmp/{TS_NAME}'")).await.is_ok();
    if made {
        let l = malter_and_check(&env, &l, &MatViewDef { tablespace: Some(TS_NAME.into()), ..l.clone() }).await;
        assert_eq!(l.tablespace.as_deref(), Some(TS_NAME));
        // Yeniden kurma tablespace'i korur.
        let l = malter_and_check(&env, &l, &MatViewDef { query: "SELECT id FROM vw.t".into(), ..l.clone() }).await;
        assert_eq!(l.tablespace.as_deref(), Some(TS_NAME));
        let l = malter_and_check(&env, &l, &MatViewDef { tablespace: Some("pg_default".into()), ..l.clone() }).await;
        assert_eq!(l.tablespace, None);
        // Tablespace temizlenemez (ALTER ile None): yerel alan hatası.
        let x = MatViewDef { tablespace: Some(TS_NAME.into()), ..l.clone() };
        let e = build::alter(&om(&x), &om(&l), &env.ctx(false)).unwrap_err();
        assert_eq!(e[0].path, "tablespace");
        env.sql(&format!("DROP TABLESPACE {TS_NAME}")).await;
    } else {
        eprintln!("PG {major}: tablespace oluşturulamadı, SET TABLESPACE atlandı");
    }
}

// ================================================================================================================
// PG 16+: INHERIT'siz (SET yetkili) role sahip devri ve sonraki ifadeler (Task 3 inceleme N-1)
// ================================================================================================================

docker_test!(owner_change_to_noinherit_role_order, owner_order_body, "oorder");

async fn owner_order_body(ts: TestServer) {
    let env = setup_in(&ts, PFX, "oorder").await;
    let major = env.version / 10000;
    if env.version < 160000 {
        eprintln!("PG {major}: üyelik başına INHERIT/SET yok (PG 16+), sahip sırası vakası atlandı");
        return;
    }
    let maint = admin_client(&ts, &ts.config.maintenance_db).await;
    let (a, b) = (format!("{}_a", env.role), format!("{}_b", env.role));
    maint.batch_execute(&format!("DROP ROLE IF EXISTS {a}; DROP ROLE IF EXISTS {b}; CREATE ROLE {a}; CREATE ROLE {b}")).await.unwrap();
    maint.batch_execute(&format!("GRANT {b} TO {a} WITH INHERIT FALSE, SET TRUE")).await.unwrap();
    base_table(&env, "vw").await;
    env.sql(&format!("GRANT USAGE, CREATE ON SCHEMA vw TO {a}, {b}; GRANT SELECT ON vw.t TO {a}, {b}")).await;
    env.sql(&format!("SET ROLE {a}")).await;
    let mut outcomes = vec![];
    // Bağlı rol `a` (süper kullanıcı değil) görünümü/MV'yi `b`'ye devrederken yorum/yenileme de değişiyor.
    let vplan = build::create(&ov(&view("vw", "v", "SELECT id FROM vw.t")), &env.ctx(false)).unwrap();
    env.exec_tx(&vplan).await;
    let mplan = build::create(&om(&matview("vw", "mv", "SELECT id FROM vw.t")), &env.ctx(false)).unwrap();
    env.exec_tx(&mplan).await;
    env.sql("RESET ROLE").await;
    let vl = vload(&env, "vw", "v").await;
    let ml = mload(&env, "vw", "mv").await;
    env.sql(&format!("SET ROLE {a}")).await;
    let vnew = ViewDef { owner: Some(b.clone()), comment: Some("k".into()), ..vl.clone() };
    outcomes.push(("view OWNER+COMMENT", env.try_exec_tx(&valter(&env, &vl, &vnew)).await));
    let mnew = MatViewDef { owner: Some(b.clone()), comment: Some("k".into()), refresh: Some(MatViewRefresh::Normal), ..ml.clone() };
    outcomes.push(("matview OWNER+COMMENT+REFRESH", env.try_exec_tx(&malter(&env, &ml, &mnew)).await));
    // Yalnızca sahip devri tek başına başarılı.
    let vonly = ViewDef { owner: Some(b.clone()), ..vl.clone() };
    outcomes.push(("view yalnız OWNER", env.try_exec_tx(&valter(&env, &vl, &vonly)).await));
    env.sql("RESET ROLE").await;
    for (what, r) in &outcomes {
        match r {
            Ok(()) => eprintln!("PG {major}: {what} → başarılı"),
            Err(e) => eprintln!("PG {major}: {what} → SQLSTATE {}", sqlstate(e)),
        }
    }
    assert!(outcomes[0].1.as_ref().is_err_and(|e| sqlstate(e) == "42501"), "{:?}", outcomes[0].1);
    assert!(outcomes[1].1.as_ref().is_err_and(|e| sqlstate(e) == "42501"), "{:?}", outcomes[1].1);
    assert!(outcomes[2].1.is_ok(), "{:?}", outcomes[2].1);
    // Temizlik: nesneleri sil (veritabanı silinince de gider), rolleri kaldır.
    env.sql("DROP SCHEMA vw CASCADE").await;
    maint.batch_execute(&format!("DROP ROLE {a}; DROP ROLE {b}")).await.unwrap();
}

// ================================================================================================================
// Reddedilen nesneler
// ================================================================================================================

docker_test!(system_and_temp_views_are_refused, refuse_body, "refuse");

async fn refuse_body(ts: TestServer) {
    let env = setup_in(&ts, PFX, "refuse").await;
    base_table(&env, "vw").await;
    let sys = env.class_oid("pg_catalog", "pg_tables").await;
    assert!(matches!(try_load(&env, ObjectKind::View, sys).await, Err(Error::Invalid(m)) if m == "sistem nesnesi düzenlenemez"));
    env.sql("CREATE TEMP VIEW tv AS SELECT 1 AS x").await;
    let tv: u32 = env.admin.query_one("SELECT 'tv'::regclass::oid", &[]).await.unwrap().get(0);
    assert!(matches!(try_load(&env, ObjectKind::View, tv).await, Err(Error::Invalid(m)) if m == "geçici görünüm düzenlenemez"));
    let t = env.class_oid("vw", "t").await;
    assert!(matches!(try_load(&env, ObjectKind::View, t).await, Err(Error::NotFound(_))));
    assert!(matches!(try_load(&env, ObjectKind::MaterializedView, t).await, Err(Error::NotFound(_))));
    // Tür uyuşmazlığı: görünüm oid'i MV olarak, MV oid'i görünüm olarak.
    env.sql("CREATE VIEW vw.v AS SELECT 1 AS x; CREATE MATERIALIZED VIEW vw.mv AS SELECT 1 AS x").await;
    let (v, mv) = (env.class_oid("vw", "v").await, env.class_oid("vw", "mv").await);
    assert!(matches!(try_load(&env, ObjectKind::MaterializedView, v).await, Err(Error::NotFound(_))));
    assert!(matches!(try_load(&env, ObjectKind::View, mv).await, Err(Error::NotFound(_))));
    assert!(matches!(try_load(&env, ObjectKind::View, 0).await, Err(Error::NotFound(_))));
}
