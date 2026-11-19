//! Policy / Tablespace / tablo RLS üreticisi → gerçek sunucu → yükleyici gidiş-dönüş testleri (Docker;
//! `PGT_TEST_URLS` yoksa atlanır; tablespace testleri ayrıca `PGT_TEST_TS_ROOT` ister, yoksa atlanır).
//!
//! Her test kendi geçici veritabanını (`pgt_ddl_pol_<ana sürüm>_<etiket>`) ve rolünü kurar; tablespace testleri küme
//! geneli `pgt_ddl_ts_<etiket>` adlarını ve kendi `tsN` alt dizinini kullanır; başarısız doğrulamadan sonra bile
//! temizlenir (`guarded`). Policy ve RLS ifadeleri uygulayıcı gibi tek transaction'da, tablespace ifadeleri tek tek
//! (transaction dışı) çalıştırılır.
//!
//! **Sabit nokta:** yüklenen her tanımda `alter(yüklü, yüklü)` boş plandır (uyarısız) ve yüklü tanımla (başka adla)
//! yeniden oluşturulan nesnenin yüklü tanımı aynıdır (`pg_get_expr` metni, rol sırası, `PUBLIC`, tablespace
//! seçenek metni, `location`).

mod common;

use common::{Env, db_message, drop_test_tablespaces, guarded, servers, setup_in, ts_name, ts_root};
use pgt_core::testing::TestServer;
use pgt_ddl::build;
use pgt_ddl::*;

const PFX: &str = "pgt_ddl_pol";

/// Veritabanı + rol + (varsa) tablespace temizliği: önce veritabanı (tablespace içindeki nesneler), sonra rol/tablespace.
async fn cleanup(ts: &TestServer, tag: &str) {
    let admin = pgt_core::testing::admin_client(ts, &ts.config.maintenance_db).await;
    common::drop_test_database(&admin, &common::db_name_in(ts, PFX, tag).await).await;
    drop_test_tablespaces(ts, tag).await;
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

fn s(v: &[&str]) -> Vec<String> {
    v.iter().map(|x| x.to_string()).collect()
}

fn sqls(p: &Plan) -> Vec<&str> {
    p.statements.iter().map(|x| x.sql.as_str()).collect()
}

// ---- Policy ----

fn pol(table: &str, name: &str) -> PolicyDef {
    PolicyDef {
        schema: "public".into(),
        table: table.into(),
        name: name.into(),
        command: PolicyCommand::All,
        permissive: true,
        roles: vec![],
        using: None,
        with_check: None,
        comment: None,
        editable: true,
        unsupported: vec![],
        table_rls: None,
    }
}

fn p(d: &PolicyDef) -> ObjectDef {
    ObjectDef::Policy(d.clone())
}

fn pwith(d: &PolicyDef, f: impl FnOnce(&mut PolicyDef)) -> PolicyDef {
    let mut d = d.clone();
    f(&mut d);
    d
}

async fn pload(env: &Env, table: &str, name: &str) -> PolicyDef {
    let sql = "SELECT p.oid FROM pg_policy p JOIN pg_class c ON c.oid = p.polrelid \
               WHERE c.relname = $1::text AND p.polname = $2::text AND c.relnamespace = 'public'::regnamespace";
    let oid: u32 = env.admin.query_one(sql, &[&table, &name]).await.unwrap_or_else(|e| panic!("politika bulunamadı: {e}")).get(0);
    match load(&env.mgr, env.server, env.db_oid, ObjectRef { kind: ObjectKind::Policy, oid }).await {
        Ok(ObjectDef::Policy(d)) => d,
        other => panic!("politika yüklenemedi: {other:?}"),
    }
}

/// Beklenen tanım: roller bayt sırasında (`COLLATE "C"`), boş yorum `None`, `table_rls` yüklenenden. İfade metni PG'nin
/// yeniden yazdırdığı biçimde döner (parantez/dönüşüm eklenir): metin eşitliği yerine varlık denetlenir.
fn pexpected(def: &PolicyDef, loaded: &PolicyDef) -> PolicyDef {
    let mut e = def.clone();
    e.roles.sort();
    e.comment = e.comment.filter(|c| !c.is_empty());
    e.table_rls = loaded.table_rls;
    e.using = def.using.as_ref().and(loaded.using.clone());
    e.with_check = def.with_check.as_ref().and(loaded.with_check.clone());
    e
}

fn assert_pnoop(env: &Env, d: &PolicyDef) {
    let pl = build::alter(&p(d), &p(d), &env.ctx(false)).unwrap_or_else(|e| panic!("noop reddedildi: {e:?}\n{d:?}"));
    assert!(pl.statements.is_empty() && pl.warnings.is_empty(), "{}: boş plan bekleniyordu: {:?} {:?}", d.name, sqls(&pl), pl.warnings);
}

/// create → tek transaction → yükle → beklenen tanıma eşit + noop + yüklü tanımla başka adla yeniden kurulan aynı.
async fn pcreate_and_check(env: &Env, def: &PolicyDef) -> PolicyDef {
    let plan = build::create(&p(def), &env.ctx(false)).unwrap_or_else(|e| panic!("create reddedildi: {} {e:?}", def.name));
    env.exec_tx(&plan).await;
    let loaded = pload(env, &def.table, &def.name).await;
    assert_eq!(pexpected(def, &loaded), loaded, "create → load ({})", def.name);
    assert_pnoop(env, &loaded);
    // Sabit nokta: yüklü metinle başka adla kurulan politikanın yüklü tanımı aynıdır.
    let dup = pwith(&loaded, |d| d.name = format!("{}_dup", d.name));
    env.exec_tx(&build::create(&p(&dup), &env.ctx(false)).unwrap()).await;
    let again = pload(env, &def.table, &dup.name).await;
    assert_eq!(PolicyDef { name: loaded.name.clone(), ..again }, loaded, "yeniden kurma sabit nokta ({})", def.name);
    env.sql(&format!("DROP POLICY {} ON {}", quote_ident(&dup.name), qualified(&dup.schema, &dup.table))).await;
    loaded
}

async fn palter_and_check(env: &Env, old: &PolicyDef, new: &PolicyDef) -> (Plan, PolicyDef) {
    let plan = build::alter(&p(old), &p(new), &env.ctx(false)).unwrap_or_else(|e| panic!("alter reddedildi: {e:?}\n old: {old:?}\n new: {new:?}"));
    if let Err(e) = env.try_exec_tx(&plan).await {
        panic!("alter uygulanamadı: {e}\n  plan: {:?}", sqls(&plan));
    }
    let loaded = pload(env, &new.table, &new.name).await;
    assert_eq!(pexpected(new, &loaded), loaded, "alter → load; plan: {:?}", sqls(&plan));
    assert_pnoop(env, &loaded);
    (plan, loaded)
}

const CPLX_USING: &str = "owner_name = current_user AND n::bigint > (SELECT count(*) FROM t) AND id IN (SELECT 1)";
const CPLX_CHECK: &str = "(n)::numeric >= 0 AND owner_name <> 'x'::text";

docker_test!(policy_create_then_load_equals_def, create_body, "create");

async fn create_body(ts: TestServer) {
    let env = setup_in(&ts, PFX, "create").await;
    env.sql("CREATE TABLE t (id int, owner_name text, n int); ALTER TABLE t ENABLE ROW LEVEL SECURITY").await;
    let me = env.me.clone();

    // PUBLIC, karmaşık ifadeler.
    let mut a = pol("t", "a");
    a.using = Some(CPLX_USING.into());
    a.with_check = Some(CPLX_CHECK.into());
    a.comment = Some("açıklama".into());
    let la = pcreate_and_check(&env, &a).await;
    assert!(la.roles.is_empty(), "PUBLIC boş liste yüklenir: {:?}", la.roles);
    assert_eq!(la.comment.as_deref(), Some("açıklama"));
    assert_eq!(la.table_rls, Some(RowSecurity { enabled: true, forced: false }));

    // İki rol (ters sırada verilir → bayt sırasıyla yüklenir), RESTRICTIVE.
    let mut b = pol("t", "b");
    b.command = PolicyCommand::Select;
    b.permissive = false;
    b.roles = vec![me.clone(), env.role.clone()];
    b.using = Some("n > 0".into());
    let lb = pcreate_and_check(&env, &b).await;
    let mut sorted = vec![me.clone(), env.role.clone()];
    sorted.sort();
    assert_eq!(lb.roles, sorted);
    assert!(!lb.permissive);

    // Komutlar ve ifadesiz politika.
    let mut i = pol("t", "ins");
    i.command = PolicyCommand::Insert;
    i.with_check = Some("n >= 0".into());
    pcreate_and_check(&env, &i).await;
    let mut u = pol("t", "upd");
    u.command = PolicyCommand::Update;
    u.using = Some("true".into());
    u.with_check = Some("n < 100".into());
    pcreate_and_check(&env, &u).await;
    let mut d = pol("t", "del");
    d.command = PolicyCommand::Delete;
    d.using = Some("n = 0".into());
    pcreate_and_check(&env, &d).await;
    pcreate_and_check(&env, &pol("t", "bare")).await;

    // Bölümlenmiş üst tablo ve bölümünde politika (PG 10'da bölümlenmiş tablo politikası denenmez).
    env.sql("CREATE TABLE pp (id int, n int) PARTITION BY RANGE (id); CREATE TABLE pp1 PARTITION OF pp FOR VALUES FROM (0) TO (10)").await;
    for tbl in ["pp", "pp1"] {
        if env.version < 110000 && tbl == "pp" {
            eprintln!("PG {}: bölümlenmiş tabloda politika denenmiyor", env.version / 10_000);
            continue;
        }
        let mut x = pol(tbl, "px");
        x.using = Some("n >= 0".into());
        pcreate_and_check(&env, &x).await;
    }
}

docker_test!(policy_alter_rebuild_rename_and_in_place, alter_body, "alter");

async fn alter_body(ts: TestServer) {
    let env = setup_in(&ts, PFX, "alter").await;
    env.sql("CREATE TABLE t (id int, owner_name text, n int); ALTER TABLE t ENABLE ROW LEVEL SECURITY").await;
    let mut base = pol("t", "p1");
    base.using = Some("n > 0".into());
    base.with_check = Some("n < 10".into());
    base.comment = Some("yorum".into());
    base.roles = vec![env.role.clone()];
    let old = pcreate_and_check(&env, &base).await;

    // Rol sırası farkı değişiklik sayılmaz.
    let two = pwith(&old, |d| d.roles = vec![env.me.clone(), env.role.clone()]);
    let (_, old) = palter_and_check(&env, &old, &two).await;
    let reversed = pwith(&old, |d| d.roles.reverse());
    assert!(build::alter(&p(&old), &p(&reversed), &env.ctx(false)).unwrap().statements.is_empty());

    // Yalnız ad değişimi: tek RENAME.
    let renamed = pwith(&old, |d| d.name = "p2".into());
    let (plan, old) = palter_and_check(&env, &old, &renamed).await;
    assert_eq!(plan.statements.len(), 1, "{:?}", sqls(&plan));
    assert!(plan.statements[0].sql.contains("RENAME TO"), "{:?}", sqls(&plan));

    // Yerinde: yalnız USING metni → tek ALTER POLICY (Safe).
    let using = pwith(&old, |d| d.using = Some("n > 5".into()));
    let (plan, old) = palter_and_check(&env, &old, &using).await;
    assert_eq!(plan.statements.len(), 1, "{:?}", sqls(&plan));
    assert!(plan.statements[0].sql.starts_with("ALTER POLICY") && plan.statements[0].risk == Risk::Safe, "{:?}", sqls(&plan));

    // Komut değişimi: yeniden kurma (DROP + CREATE, Destructive); yorum korunur.
    let cmd = pwith(&old, |d| d.command = PolicyCommand::Update);
    let (plan, old) = palter_and_check(&env, &old, &cmd).await;
    assert!(plan.statements[0].sql.starts_with("DROP POLICY") && plan.statements[0].risk == Risk::Destructive, "{:?}", sqls(&plan));
    assert_eq!(old.comment.as_deref(), Some("yorum"));
    assert_eq!(old.command, PolicyCommand::Update);

    // PERMISSIVE → RESTRICTIVE: yeniden kurma.
    let restr = pwith(&old, |d| d.permissive = false);
    let (_, old) = palter_and_check(&env, &old, &restr).await;
    assert!(!old.permissive);

    // USING kaldırma: ALTER POLICY ifadeyi kaldıramaz → yeniden kurma; WITH CHECK korunur.
    let no_using = pwith(&old, |d| d.using = None);
    let (plan, old) = palter_and_check(&env, &old, &no_using).await;
    assert!(plan.statements.len() >= 2 && plan.statements[0].sql.starts_with("DROP POLICY"), "{:?}", sqls(&plan));
    assert!(old.using.is_none() && old.with_check.is_some());

    // Yorum kaldırma ve rolleri PUBLIC'e döndürme (TO PUBLIC).
    let cleared = pwith(&old, |d| {
        d.comment = None;
        d.roles = vec![];
    });
    let (_, old) = palter_and_check(&env, &old, &cleared).await;
    assert!(old.roles.is_empty() && old.comment.is_none());
}

docker_test!(policy_errors_read_only_and_drop, errors_body, "errors");

async fn errors_body(ts: TestServer) {
    let env = setup_in(&ts, PFX, "errors").await;
    env.sql("CREATE TABLE t (id int, n int); ALTER TABLE t ENABLE ROW LEVEL SECURITY").await;
    let mut a = pol("t", "a");
    a.using = Some("n > 0".into());
    let old = pcreate_and_check(&env, &a).await;
    // Tablo değişimi alan hatasıdır.
    let moved = pwith(&old, |d| d.table = "other".into());
    let e = build::alter(&p(&old), &p(&moved), &env.ctx(false)).unwrap_err();
    assert_eq!(e[0].path, "table");
    // Salt okunur (editable = false) tanımda değişiklik reddedilir, değişiklik yoksa boş plan.
    let ro = pwith(&old, |d| d.editable = false);
    assert!(build::alter(&p(&ro), &p(&ro), &env.ctx(false)).unwrap().statements.is_empty());
    let e = build::alter(&p(&ro), &p(&pwith(&ro, |d| d.comment = Some("x".into()))), &env.ctx(false)).unwrap_err();
    assert_eq!(e[0].path, "object");
    // Drop.
    let plan = build::drop(&p(&old), false, &env.ctx(false)).unwrap();
    env.exec_tx(&plan).await;
    let n: i64 = env.admin.query_one("SELECT count(*) FROM pg_policy WHERE polname = 'a'", &[]).await.unwrap().get(0);
    assert_eq!(n, 0);
}

docker_test!(table_rls_is_effective_for_non_owner, rls_body, "rls");

async fn rls_body(ts: TestServer) {
    let env = setup_in(&ts, PFX, "rls").await;
    env.sql(&format!(
        "CREATE TABLE t (id int, n int); INSERT INTO t VALUES (1, 1), (2, 2), (3, 3); GRANT SELECT ON t TO {}",
        quote_ident(&env.role)
    ))
    .await;
    async fn count_as_role(env: &Env) -> i64 {
        env.sql(&format!("SET ROLE {}", quote_ident(&env.role))).await;
        let n: i64 = env.admin.query_one("SELECT count(*) FROM public.t", &[]).await.unwrap().get(0);
        env.sql("RESET ROLE").await;
        n
    }
    assert_eq!(count_as_role(&env).await, 3, "RLS kapalıyken tüm satırlar");

    // Alter ile ENABLE → Destructive; politikasız tabloda sahip olmayan hiçbir satır görmez.
    let oid = env.class_oid("public", "t").await;
    let ObjectDef::Table(t0) = load(&env.mgr, env.server, env.db_oid, ObjectRef { kind: ObjectKind::Table, oid }).await.unwrap() else { panic!() };
    assert_eq!(t0.row_security, RowSecurity { enabled: false, forced: false });
    let t1 = TableDef { row_security: RowSecurity { enabled: true, forced: false }, ..t0.clone() };
    let plan = build::alter(&ObjectDef::Table(t0.clone()), &ObjectDef::Table(t1.clone()), &env.ctx(false)).unwrap();
    assert!(plan.statements.iter().any(|s| s.sql.contains("ENABLE ROW LEVEL SECURITY") && s.risk == Risk::Destructive), "{:?}", sqls(&plan));
    env.exec_tx(&plan).await;
    assert_eq!(count_as_role(&env).await, 0, "ENABLE sonrası sahip olmayan hiç satır görmez");

    // Politika eklenince görünür; politika rolü etkiler.
    let mut allow = pol("t", "allow");
    allow.command = PolicyCommand::Select;
    allow.roles = vec![env.role.clone()];
    allow.using = Some("n >= 2".into());
    env.exec_tx(&build::create(&p(&allow), &env.ctx(false)).unwrap()).await;
    assert_eq!(count_as_role(&env).await, 2);
    let l = pload(&env, "t", "allow").await;
    assert_eq!(l.table_rls, Some(RowSecurity { enabled: true, forced: false }));

    // DISABLE → Safe + uyarı; tüm satırlar yeniden görünür.
    let plan = build::alter(&ObjectDef::Table(t1.clone()), &ObjectDef::Table(t0.clone()), &env.ctx(false)).unwrap();
    assert!(plan.statements.iter().all(|s| s.risk == Risk::Safe) && !plan.warnings.is_empty(), "{:?} {:?}", sqls(&plan), plan.warnings);
    env.exec_tx(&plan).await;
    assert_eq!(count_as_role(&env).await, 3);
    // Bayrak kapalıyken politika yüklenir ve değişiklik planı uyarı verir.
    let l = pload(&env, "t", "allow").await;
    assert_eq!(l.table_rls, Some(RowSecurity::default()));
    let plan = build::alter(&p(&l), &p(&pwith(&l, |d| d.comment = Some("c".into()))), &env.ctx(false)).unwrap();
    assert!(!plan.warnings.is_empty(), "RLS kapalıyken uyarı bekleniyordu");
}

// ---- Tablespace ----

async fn ts_oid(env: &Env, name: &str) -> u32 {
    env.admin.query_one("SELECT oid FROM pg_tablespace WHERE spcname = $1::text", &[&name]).await.unwrap_or_else(|e| panic!("tablespace yok: {e}")).get(0)
}

async fn ts_exists(env: &Env, name: &str) -> bool {
    !env.admin.query("SELECT 1 FROM pg_tablespace WHERE spcname = $1::text", &[&name]).await.unwrap().is_empty()
}

async fn tsload(env: &Env, name: &str) -> TablespaceDef {
    let oid = ts_oid(env, name).await;
    match load(&env.mgr, env.server, None, ObjectRef { kind: ObjectKind::Tablespace, oid }).await {
        Ok(ObjectDef::Tablespace(d)) => d,
        other => panic!("tablespace yüklenemedi: {other:?}"),
    }
}

fn tsdef(name: &str, loc: &str) -> TablespaceDef {
    TablespaceDef {
        name: name.into(),
        owner: None,
        location: loc.into(),
        options: TablespaceOptions::default(),
        comment: None,
        editable: true,
        unsupported: vec![],
    }
}

fn t(d: &TablespaceDef) -> ObjectDef {
    ObjectDef::Tablespace(d.clone())
}

fn assert_tsnoop(env: &Env, d: &TablespaceDef) {
    let pl = build::alter(&t(d), &t(d), &env.ctx(false)).unwrap_or_else(|e| panic!("noop reddedildi: {e:?}"));
    assert!(pl.statements.is_empty() && pl.warnings.is_empty(), "boş plan bekleniyordu: {:?} {:?}", sqls(&pl), pl.warnings);
}

/// Planı transaction dışında (tablespace ifadeleri transaction dışıdır) tek tek çalıştırır.
async fn exec_nt(env: &Env, plan: &Plan) {
    assert!(plan.statements.iter().all(|s| !s.transactional), "tüm tablespace ifadeleri transaction dışıdır: {:?}", sqls(plan));
    env.exec(plan).await;
}

fn opts(env: &Env) -> TablespaceOptions {
    TablespaceOptions {
        seq_page_cost: Some("1.5".into()),
        random_page_cost: Some("2.25".into()),
        effective_io_concurrency: Some("4".into()),
        maintenance_io_concurrency: (env.version >= 130000).then(|| "8".into()),
    }
}

docker_test!(tablespace_create_then_load_equals_def, ts_create_body, "tscreate");

async fn ts_create_body(ts: TestServer) {
    let Some(root) = ts_root() else { return };
    let env = setup_in(&ts, PFX, "tscreate").await;
    let mut d = tsdef(&ts_name("tscreate"), &format!("{root}/ts1"));
    d.options = opts(&env);
    d.comment = Some("açıklama".into());
    d.owner = Some(env.role.clone());
    exec_nt(&env, &build::create(&t(&d), &env.ctx(false)).unwrap()).await;
    let loaded = tsload(&env, &d.name).await;
    assert_eq!(loaded, d, "create → load");
    assert_tsnoop(&env, &loaded);
    assert_eq!(tsload(&env, &d.name).await, loaded, "iki yükleme aynı");
    // Seçeneksiz, yorumsuz, sahipsiz: sahip yüklenenden (süper kullanıcı).
    let d2 = tsdef(&format!("{}_b", ts_name("tscreate")), &format!("{root}/ts2"));
    exec_nt(&env, &build::create(&t(&d2), &env.ctx(false)).unwrap()).await;
    let l2 = tsload(&env, &d2.name).await;
    assert_eq!(l2.owner.as_deref(), Some(env.me.as_str()));
    assert_eq!((l2.options.clone(), l2.comment.clone(), l2.editable), (TablespaceOptions::default(), None, true));
    assert_tsnoop(&env, &l2);
}

docker_test!(tablespace_alter_rename_set_reset_owner, ts_alter_body, "tsalter");

async fn ts_alter_body(ts: TestServer) {
    let Some(root) = ts_root() else { return };
    let env = setup_in(&ts, PFX, "tsalter").await;
    let mut d = tsdef(&ts_name("tsalter"), &format!("{root}/ts3"));
    d.options = TablespaceOptions { seq_page_cost: Some("1.1".into()), random_page_cost: Some("3".into()), ..Default::default() };
    d.comment = Some("ilk".into());
    exec_nt(&env, &build::create(&t(&d), &env.ctx(false)).unwrap()).await;
    let old = tsload(&env, &d.name).await;

    let mut new = old.clone();
    new.name = format!("{}_r", ts_name("tsalter"));
    new.options.seq_page_cost = Some("0.5".into());
    new.options.random_page_cost = None;
    new.options.effective_io_concurrency = Some("2".into());
    new.comment = Some("ikinci".into());
    new.owner = Some(env.role.clone());
    let plan = build::alter(&t(&old), &t(&new), &env.ctx(false)).unwrap();
    assert!(plan.statements[0].sql.contains("RENAME TO"), "{:?}", sqls(&plan));
    assert!(plan.statements.last().unwrap().sql.contains("OWNER TO"), "OWNER TO en sonda: {:?}", sqls(&plan));
    exec_nt(&env, &plan).await;
    let loaded = tsload(&env, &new.name).await;
    assert_eq!(loaded, new, "alter → load; plan: {:?}", sqls(&plan));
    assert!(!ts_exists(&env, &old.name).await);
    assert_tsnoop(&env, &loaded);

    // Tüm seçenekleri kaldır (RESET) ve yorumu sil.
    let mut cleared = loaded.clone();
    cleared.options = TablespaceOptions::default();
    cleared.comment = None;
    let plan = build::alter(&t(&loaded), &t(&cleared), &env.ctx(false)).unwrap();
    exec_nt(&env, &plan).await;
    assert_eq!(tsload(&env, &new.name).await, cleared);
    // `location` değişimi alan hatasıdır.
    let mut moved = cleared.clone();
    moved.location = format!("{root}/ts4");
    assert_eq!(build::alter(&t(&cleared), &t(&moved), &env.ctx(false)).unwrap_err()[0].path, "location");
}

docker_test!(tablespace_drop_refuses_nonempty_then_drops_empty, ts_drop_body, "tsdrop");

async fn ts_drop_body(ts: TestServer) {
    let Some(root) = ts_root() else { return };
    let env = setup_in(&ts, PFX, "tsdrop").await;
    let d = tsdef(&ts_name("tsdrop"), &format!("{root}/ts5"));
    exec_nt(&env, &build::create(&t(&d), &env.ctx(false)).unwrap()).await;
    let loaded = tsload(&env, &d.name).await;
    env.sql(&format!("CREATE TABLE inside (id int) TABLESPACE {}", quote_ident(&d.name))).await;

    let plan = build::drop(&t(&loaded), false, &env.ctx(false)).unwrap();
    assert_eq!(plan.statements.len(), 1);
    assert!(plan.statements[0].risk == Risk::Destructive && !plan.statements[0].transactional, "{:?}", sqls(&plan));
    let err = env.admin.batch_execute(&plan.statements[0].sql).await.unwrap_err();
    assert_eq!(err.code().map(|c| c.code()), Some("55000"), "{}", db_message(&err));
    assert!(ts_exists(&env, &d.name).await, "başarısız drop hiçbir şeyi değiştirmez");
    assert_eq!(tsload(&env, &d.name).await, loaded);

    env.sql("DROP TABLE inside").await;
    exec_nt(&env, &plan).await;
    assert!(!ts_exists(&env, &d.name).await);
}

// Sistem tablespace'leri (dizin gerekmez; `PGT_TEST_TS_ROOT` olmadan da çalışır).
docker_test!(system_tablespaces_load_read_only, ts_system_body, "tssys");

async fn ts_system_body(ts: TestServer) {
    let env = setup_in(&ts, PFX, "tssys").await;
    for name in ["pg_default", "pg_global"] {
        let l = tsload(&env, name).await;
        assert_eq!(l.location, "", "{name}: sistem tablespace'inde konum boş");
        assert_eq!(l.unsupported, s(&["Sistem tablespace'i"]));
        assert!(!l.editable);
        assert_tsnoop(&env, &l);
        let changed = TablespaceDef { comment: Some("x".into()), ..l.clone() };
        let e = build::alter(&t(&l), &t(&changed), &env.ctx(false)).unwrap_err();
        assert_eq!(e.iter().map(|e| e.path.as_str()).collect::<Vec<_>>(), ["object"], "{name}");
        let e = build::drop(&t(&l), false, &env.ctx(false)).unwrap_err();
        assert_eq!(e.iter().map(|e| e.path.as_str()).collect::<Vec<_>>(), ["object"], "{name}");
    }
}
