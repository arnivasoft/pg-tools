//! Database / Role (sunucu kapsamlı) üreticisi → gerçek sunucu → yükleyici gidiş-dönüş testleri (Docker;
//! `PGT_TEST_URLS` yoksa atlanır).
//!
//! Her test benzersiz adlı küme nesneleri kurar (`pgt_ddl_dbk_<ana sürüm>_<etiket>[_…]`, `pgt_ddl_rk_<etiket>[_…]`) ve
//! başarısız doğrulamadan sonra bile `cleanup_cluster` ile siler. İfadeler bakım bağlantısında (`admin_client(ts,
//! maintenance_db)`, süper kullanıcı) çalışır: Database planları transaction dışında tek tek, Role planları uygulayıcı
//! gibi tek transaction'da. Yükleyiciler `ConnectionManager` üzerinden bakım bağlantısını kullanır (`database_oid` yok
//! sayılır). Sürüme bağlı kurallar sürüm numarasına göre dallanır; atlanan kısım yazdırılır.
//!
//! Normalleştirme sözleşmesi: `alter(load, load)` boş; `create(def)` → `load` = `def`, şu farklarla: Database
//! `template → None`, create'te `None` bırakılan create-only alanlar sunucu değerini alır, `tablespace None →
//! Some("pg_default")`, kodlama kanonik yazım; Role `password → keep`, PG 16+ üyelik seçenekleri dolar. Ayar değerleri
//! kurallı biçimde verilir (`search_path` = `"$user", public`).
//!
//! Parolalar yalnızca süreç içinde rastgele üretilir; hiçbir yere yazdırılmaz (parolalı ifadeler `secret_sql` ile
//! çalışır: hata iletisinde SQL yer almaz).

mod common;

use std::sync::Arc;

use common::{cleanup_cluster, cluster_name, guarded, servers};
use pgt_core::Error;
use pgt_core::connection::ConnectionManager;
use pgt_core::testing::{TestServer, admin_client};
use pgt_ddl::build::{self, DropOptions};
use pgt_ddl::*;
use tokio_postgres::Client;
use uuid::Uuid;

// ---- ortam ----

struct Cx {
    ts: TestServer,
    /// Bakım veritabanına süper kullanıcı bağlantısı.
    admin: Client,
    mgr: Arc<ConnectionManager>,
    server: Uuid,
    version: u32,
    maint: String,
    maint_oid: u32,
    me: String,
    db_base: String,
    role_base: String,
}

/// Sunucu hatası (ifade sırası, SQLSTATE, ileti, ayrıntı).
#[derive(Debug)]
struct DbErr {
    index: usize,
    code: String,
    message: String,
    detail: Option<String>,
}

fn db_err(index: usize, e: &tokio_postgres::Error) -> DbErr {
    match e.as_db_error() {
        Some(d) => DbErr {
            index,
            code: d.code().code().to_string(),
            message: d.message().to_string(),
            detail: d.detail().map(str::to_string),
        },
        None => DbErr { index, code: String::new(), message: e.to_string(), detail: None },
    }
}

fn sqls(p: &Plan) -> Vec<&str> {
    p.statements.iter().map(|s| s.sql.as_str()).collect()
}

/// Test parolası: süreç içinde rastgele; yazdırılmaz.
fn random_password() -> String {
    format!("Pw{}", Uuid::new_v4().simple())
}

impl Cx {
    async fn new(ts: &TestServer, tag: &str) -> Cx {
        cleanup_cluster(ts, tag).await;
        let maint = ts.config.maintenance_db.clone();
        let admin = admin_client(ts, &maint).await;
        let mgr = Arc::new(ConnectionManager::new());
        mgr.connect(ts.config.clone(), ts.password.clone()).await.unwrap();
        let v: i32 = admin.query_one("SELECT current_setting('server_version_num')::int4", &[]).await.unwrap().get(0);
        let maint_oid: u32 =
            admin.query_one("SELECT oid FROM pg_database WHERE datname = $1::text", &[&maint]).await.unwrap().get(0);
        let me: String = admin.query_one("SELECT current_user::text", &[]).await.unwrap().get(0);
        Cx {
            db_base: cluster_name(ts, ObjectKind::Database, tag).await,
            role_base: cluster_name(ts, ObjectKind::Role, tag).await,
            ts: ts.clone(),
            admin,
            mgr,
            server: ts.config.id,
            version: v as u32,
            maint,
            maint_oid,
            me,
        }
    }

    fn major(&self) -> u32 {
        self.version / 10_000
    }

    fn ctx(&self) -> BuildCtx {
        BuildCtx { version_num: self.version, concurrently: false }
    }

    /// `<veritabanı tabanı>_<sonek>` (boş sonek = taban).
    fn db(&self, suffix: &str) -> String {
        if suffix.is_empty() { self.db_base.clone() } else { format!("{}_{suffix}", self.db_base) }
    }

    fn role(&self, suffix: &str) -> String {
        if suffix.is_empty() { self.role_base.clone() } else { format!("{}_{suffix}", self.role_base) }
    }

    async fn sql(&self, sql: &str) {
        if let Err(e) = self.admin.batch_execute(sql).await {
            panic!("ifade başarısız: {sql}\n  hata: {:?}", db_err(0, &e));
        }
    }

    /// Gizli değer içeren ifade: hata iletisinde SQL yer almaz.
    async fn secret_sql(&self, sql: &str) {
        if let Err(e) = self.admin.batch_execute(sql).await {
            let code = e.as_db_error().map(|d| d.code().code().to_string()).unwrap_or_default();
            panic!("parolalı ifade başarısız ({code})");
        }
    }

    async fn db_oid(&self, name: &str) -> Option<u32> {
        self.admin
            .query_opt("SELECT oid FROM pg_database WHERE datname = $1::text", &[&name])
            .await
            .unwrap()
            .map(|r| r.get(0))
    }

    async fn role_oid(&self, name: &str) -> Option<u32> {
        self.admin.query_opt("SELECT oid FROM pg_roles WHERE rolname = $1::text", &[&name]).await.unwrap().map(|r| r.get(0))
    }

    /// Veritabanı oid'i hiçbir zaman kullanılmaz (sunucu kapsamlı): bilerek anlamsız bir değer verilir.
    async fn try_load(&self, kind: ObjectKind, oid: u32) -> pgt_core::Result<ObjectDef> {
        load(&self.mgr, self.server, u32::MAX, ObjectRef { kind, oid }).await
    }

    async fn load_db(&self, name: &str) -> DatabaseDef {
        let oid = self.db_oid(name).await.unwrap_or_else(|| panic!("{name} yok"));
        match self.try_load(ObjectKind::Database, oid).await {
            Ok(ObjectDef::Database(d)) => d,
            other => panic!("{name} yüklenemedi: {other:?}"),
        }
    }

    async fn load_role(&self, name: &str) -> RoleDef {
        let oid = self.role_oid(name).await.unwrap_or_else(|| panic!("{name} yok"));
        match self.try_load(ObjectKind::Role, oid).await {
            Ok(ObjectDef::Role(d)) => d,
            other => panic!("{name} yüklenemedi: {other:?}"),
        }
    }

    /// Database planı: her ifade transaction dışı (`transactional = false`) ve tek tek; ilk hatada durur.
    async fn try_run_nt(&self, plan: &Plan) -> Result<(), DbErr> {
        for (i, s) in plan.statements.iter().enumerate() {
            assert!(!s.transactional && !s.redacted, "Database ifadesi transaction dışı olmalı: {}", s.sql);
            self.admin.batch_execute(&s.sql).await.map_err(|e| db_err(i, &e))?;
        }
        Ok(())
    }

    async fn run_nt(&self, plan: &Plan) {
        if let Err(e) = self.try_run_nt(plan).await {
            panic!("plan başarısız: {e:?}\n  plan: {:?}", sqls(plan));
        }
    }

    /// Role planı: uygulayıcı gibi tek transaction; ilk hatada geri alır.
    async fn try_run_tx(&self, plan: &Plan) -> Result<(), DbErr> {
        self.admin.batch_execute("BEGIN").await.unwrap();
        for (i, s) in plan.statements.iter().enumerate() {
            assert!(s.transactional && !s.redacted, "Role ifadesi transaction içinde, gizlenmemiş olmalı: {}", s.sql);
            if let Err(e) = self.admin.batch_execute(&s.sql).await {
                self.admin.batch_execute("ROLLBACK").await.unwrap();
                return Err(db_err(i, &e));
            }
        }
        self.admin.batch_execute("COMMIT").await.map_err(|e| db_err(plan.statements.len(), &e))
    }

    async fn run_tx(&self, plan: &Plan) {
        if let Err(e) = self.try_run_tx(plan).await {
            panic!("plan başarısız: {e:?}\n  plan: {:?}", sqls(plan));
        }
    }

    /// Ayrı süper kullanıcı oturumunda `SET ROLE <as_role>` ile ifade (grantor testi); oturum kapatılır.
    async fn sql_as(&self, as_role: &str, sql: &str) {
        let c = admin_client(&self.ts, &self.maint).await;
        c.batch_execute(&format!("SET ROLE {}", quote_ident(as_role))).await.unwrap();
        if let Err(e) = c.batch_execute(sql).await {
            panic!("{as_role} olarak başarısız: {sql}\n  hata: {:?}", db_err(0, &e));
        }
    }

    async fn has_icu(&self) -> bool {
        let n: i64 =
            self.admin.query_one("SELECT count(*) FROM pg_collation WHERE collprovider = 'i'", &[]).await.unwrap().get(0);
        n > 0
    }
}

macro_rules! docker_test {
    ($name:ident, $body:ident, $tag:literal) => {
        #[tokio::test]
        async fn $name() {
            for ts in servers() {
                guarded($body(ts.clone(), $tag), cleanup_cluster(&ts, $tag)).await;
            }
        }
    };
}

// ================================================================================================================
// Database
// ================================================================================================================

fn database(name: &str) -> DatabaseDef {
    DatabaseDef {
        name: name.into(),
        owner: None,
        template: None,
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

fn od(d: &DatabaseDef) -> ObjectDef {
    ObjectDef::Database(d.clone())
}

fn setting(name: &str, value: &str) -> ConfigSetting {
    ConfigSetting { name: name.into(), value: value.into() }
}

/// Beklenen veritabanı (normalleştirme sözleşmesi).
fn db_expected(def: &DatabaseDef, loaded: &DatabaseDef) -> DatabaseDef {
    DatabaseDef {
        owner: def.owner.clone().or_else(|| loaded.owner.clone()),
        template: None,
        encoding: def.encoding.as_ref().map(|e| e.to_ascii_uppercase()).or_else(|| loaded.encoding.clone()),
        locale_provider: def.locale_provider.or(loaded.locale_provider),
        lc_collate: def.lc_collate.clone().or_else(|| loaded.lc_collate.clone()),
        lc_ctype: def.lc_ctype.clone().or_else(|| loaded.lc_ctype.clone()),
        provider_locale: def.provider_locale.clone().or_else(|| loaded.provider_locale.clone()),
        tablespace: Some(def.tablespace.clone().unwrap_or_else(|| "pg_default".into())),
        comment: def.comment.clone().filter(|c| !c.is_empty()),
        ..def.clone()
    }
}

fn assert_db_noop(cx: &Cx, d: &DatabaseDef) {
    let p = build::alter(&od(d), &od(d), &cx.ctx()).unwrap_or_else(|e| panic!("noop reddedildi: {e:?}\n{d:?}"));
    assert!(p.statements.is_empty() && p.warnings.is_empty(), "{}: boş plan bekleniyordu: {:?} {:?}", d.name, sqls(&p), p.warnings);
}

/// create → (transaction dışı) → yükle → beklenen + noop + iki yükleme aynı.
async fn db_create_and_check(cx: &Cx, def: &DatabaseDef) -> DatabaseDef {
    let plan = build::create(&od(def), &cx.ctx()).unwrap_or_else(|e| panic!("create reddedildi: {} {e:?}", def.name));
    cx.run_nt(&plan).await;
    let loaded = cx.load_db(&def.name).await;
    assert_eq!(loaded, db_expected(def, &loaded), "create → load ({})", def.name);
    assert!(loaded.owner.is_some() && loaded.tablespace.is_some() && loaded.template.is_none());
    if cx.version >= 150000 {
        assert!(loaded.locale_provider.is_some(), "PG 15+: sağlayıcı dolu olmalı");
    } else {
        assert!(loaded.locale_provider.is_none() && loaded.provider_locale.is_none());
    }
    assert_db_noop(cx, &loaded);
    assert_eq!(cx.load_db(&def.name).await, loaded, "iki yükleme aynı olmalı");
    loaded
}

async fn db_alter_and_check(cx: &Cx, old: &DatabaseDef, new: &DatabaseDef) -> DatabaseDef {
    let plan = build::alter(&od(old), &od(new), &cx.ctx()).unwrap_or_else(|e| panic!("alter reddedildi: {e:?}\n  old: {old:?}\n  new: {new:?}"));
    assert!(!plan.statements.is_empty(), "boş olmayan plan bekleniyordu: {new:?}");
    cx.run_nt(&plan).await;
    let loaded = cx.load_db(&new.name).await;
    assert_eq!(&loaded, new, "alter → load; plan: {:?}", sqls(&plan));
    assert_db_noop(cx, &loaded);
    loaded
}

docker_test!(database_create_then_load_matches, db_create_body, "dbcreate");

async fn db_create_body(ts: TestServer, tag: &str) {
    let cx = Cx::new(&ts, tag).await;
    let owner = cx.role("own");
    cx.sql(&format!("CREATE ROLE {owner}")).await;
    let mut defs = vec![
        DatabaseDef {
            owner: Some(owner.clone()),
            template: Some("template0".into()),
            encoding: Some("UTF8".into()),
            lc_collate: Some("C".into()),
            lc_ctype: Some("C".into()),
            connection_limit: 7,
            settings: vec![setting("search_path", "\"$user\", public"), setting("work_mem", "64MB")],
            comment: Some("yorum; -- 'x' /* y */ \\".into()),
            ..database(&cx.db("full"))
        },
        database(&cx.db("min")),
        // Küçük harf kodlama kanonik yazılır; PG 10'da da CREATE DATABASE … ALLOW_CONNECTIONS / IS_TEMPLATE.
        DatabaseDef {
            template: Some("template0".into()),
            encoding: Some("utf8".into()),
            is_template: true,
            allow_connections: false,
            ..database(&cx.db("tpl"))
        },
        DatabaseDef { tablespace: Some("pg_default".into()), comment: Some(String::new()), ..database(&cx.db("ts")) },
        // Tırnak gerektiren ad (boşluk, çift/tek tırnak, `;`, `--`).
        DatabaseDef { comment: Some("c".into()), ..database(&format!("{}_Q \"x\"; --'", cx.db_base)) },
    ];
    if cx.version >= 150000 {
        // PG 15+: LOCALE_PROVIDER = libc + LC_* (ICU_LOCALE yok) kabul edilir.
        defs.push(DatabaseDef {
            template: Some("template0".into()),
            encoding: Some("UTF8".into()),
            locale_provider: Some(LocaleProvider::Libc),
            lc_collate: Some("C".into()),
            lc_ctype: Some("C".into()),
            ..database(&cx.db("libc"))
        });
    } else {
        eprintln!("PG {}: LOCALE_PROVIDER = libc atlandı (PG 15+)", cx.major());
    }
    for d in &defs {
        let plan = build::create(&od(d), &cx.ctx()).unwrap();
        if d.is_template {
            assert!(plan.warnings.iter().any(|w| w == build::database::IS_TEMPLATE_WARNING), "{:?}", plan.warnings);
        }
        let l = db_create_and_check(&cx, d).await;
        if d.locale_provider == Some(LocaleProvider::Libc) {
            assert_eq!(l.provider_locale, None);
        }
    }
    // 'POSIX' yerel ayarı bazı sürümlerde sunucuda kanonik 'C' olarak saklanır (setlocale'in döndürdüğü ad): create →
    // load eşitliği bu değerle kurulamaz, ama yüklü değerle alter boş kalır (sabit nokta).
    let posix = DatabaseDef {
        template: Some("template0".into()),
        lc_collate: Some("POSIX".into()),
        lc_ctype: Some("POSIX".into()),
        ..database(&cx.db("posix"))
    };
    cx.run_nt(&build::create(&od(&posix), &cx.ctx()).unwrap()).await;
    let l = cx.load_db(&posix.name).await;
    let stored = (l.lc_collate.clone().unwrap(), l.lc_ctype.clone().unwrap());
    assert!(["C", "POSIX"].contains(&stored.0.as_str()) && ["C", "POSIX"].contains(&stored.1.as_str()), "{stored:?}");
    assert_db_noop(&cx, &l);
    eprintln!("PG {}: LC_COLLATE/LC_CTYPE 'POSIX' saklanan değer {stored:?}", cx.major());
    let full = cx.load_db(&cx.db("full")).await;
    assert_eq!(full.encoding.as_deref(), Some("UTF8"));
    assert_eq!(cx.load_db(&cx.db("tpl")).await.encoding.as_deref(), Some("UTF8"), "kanonik kodlama");
    // Varsayılanlar: sahip bağlı rol, kodlama/yerel ayar şablondan (template1).
    let min = cx.load_db(&cx.db("min")).await;
    assert_eq!(min.owner.as_deref(), Some(cx.me.as_str()));
    let maint = cx.load_db(&cx.maint).await;
    assert_eq!((min.encoding.clone(), min.lc_collate.clone()), (maint.encoding.clone(), maint.lc_collate.clone()));
    eprintln!("PG {}: CREATE DATABASE … ALLOW_CONNECTIONS/CONNECTION LIMIT/IS_TEMPLATE kabul edildi", cx.major());
}

docker_test!(database_icu_and_builtin_providers, db_providers_body, "dbprov");

async fn db_providers_body(ts: TestServer, tag: &str) {
    let cx = Cx::new(&ts, tag).await;
    let base = |name: &str| DatabaseDef { template: Some("template0".into()), encoding: Some("UTF8".into()), ..database(name) };
    let icu = DatabaseDef { locale_provider: Some(LocaleProvider::Icu), provider_locale: Some("en-US".into()), ..base(&cx.db("icu")) };
    let builtin =
        DatabaseDef { locale_provider: Some(LocaleProvider::Builtin), provider_locale: Some("C.UTF-8".into()), ..base(&cx.db("bi")) };
    if cx.version < 150000 {
        // Sürüm kapısı yerel hatadır (sunucuya gitmez).
        assert_eq!(build::create(&od(&icu), &cx.ctx()).unwrap_err()[0].path, "localeProvider");
        eprintln!("PG {}: ICU/builtin veritabanı sağlayıcısı atlandı (PG 15+)", cx.major());
        return;
    }
    if cx.has_icu().await {
        let l = db_create_and_check(&cx, &icu).await;
        assert_eq!((l.locale_provider, l.provider_locale.as_deref()), (Some(LocaleProvider::Icu), Some("en-US")));
        eprintln!("PG {}: LOCALE_PROVIDER icu ICU_LOCALE 'en-US' gidiş-dönüş tamam", cx.major());
    } else {
        eprintln!("PG {}: sunucuda ICU collation yok, icu atlandı", cx.major());
    }
    if cx.version < 170000 {
        assert_eq!(build::create(&od(&builtin), &cx.ctx()).unwrap_err()[0].path, "localeProvider");
        eprintln!("PG {}: builtin sağlayıcı atlandı (PG 17+)", cx.major());
        return;
    }
    let has_c_utf8 = !cx
        .admin
        .query("SELECT 1 FROM pg_collation WHERE collprovider = 'b' AND colllocale = 'C.UTF-8'", &[])
        .await
        .unwrap()
        .is_empty();
    if !has_c_utf8 {
        eprintln!("PG {}: imajda builtin C.UTF-8 yok, atlandı", cx.major());
        return;
    }
    let l = db_create_and_check(&cx, &builtin).await;
    assert_eq!((l.locale_provider, l.provider_locale.as_deref()), (Some(LocaleProvider::Builtin), Some("C.UTF-8")));
    eprintln!("PG {}: LOCALE_PROVIDER builtin BUILTIN_LOCALE 'C.UTF-8' gidiş-dönüş tamam", cx.major());
}

docker_test!(database_alter_round_trips, db_alter_body, "dbalter");

async fn db_alter_body(ts: TestServer, tag: &str) {
    let cx = Cx::new(&ts, tag).await;
    let owner = cx.role("own");
    cx.sql(&format!("CREATE ROLE {owner}")).await;
    let def = DatabaseDef {
        template: Some("template0".into()),
        encoding: Some("UTF8".into()),
        settings: vec![setting("statement_timeout", "1min"), setting("work_mem", "4MB")],
        ..database(&cx.db(""))
    };
    let l0 = db_create_and_check(&cx, &def).await;
    // Bakım bağlantısından başka bir veritabanında RENAME / OWNER / WITH / SET / RESET / COMMENT.
    let l1 = db_alter_and_check(&cx, &l0, &DatabaseDef { name: cx.db("ren"), ..l0.clone() }).await;
    let l2 = db_alter_and_check(&cx, &l1, &DatabaseDef { owner: Some(owner.clone()), ..l1.clone() }).await;
    let new = DatabaseDef { connection_limit: 3, allow_connections: false, is_template: true, ..l2.clone() };
    let plan = build::alter(&od(&l2), &od(&new), &cx.ctx()).unwrap();
    assert_eq!(plan.statements.len(), 1, "WITH tek ifade: {:?}", sqls(&plan));
    let l3 = db_alter_and_check(&cx, &l2, &new).await;
    let l4 = db_alter_and_check(
        &cx,
        &l3,
        &DatabaseDef { connection_limit: -1, allow_connections: true, is_template: false, ..l3.clone() },
    )
    .await;
    // Ayarlar: work_mem kaldırılır, statement_timeout değişir, search_path eklenir.
    let l5 = db_alter_and_check(
        &cx,
        &l4,
        &DatabaseDef {
            settings: vec![setting("search_path", "\"$user\", public"), setting("statement_timeout", "2min")],
            ..l4.clone()
        },
    )
    .await;
    let l6 = db_alter_and_check(&cx, &l5, &DatabaseDef { comment: Some("ilk 'yorum'".into()), ..l5.clone() }).await;
    let l7 = db_alter_and_check(&cx, &l6, &DatabaseDef { comment: Some("ikinci".into()), settings: vec![], ..l6.clone() }).await;
    // Hepsi birden (rename + WITH + ayar + yorum + sahip; OWNER TO en sonda).
    let all = DatabaseDef {
        name: cx.db("ren2"),
        owner: Some(cx.me.clone()),
        connection_limit: 9,
        settings: vec![setting("work_mem", "8MB")],
        comment: None,
        ..l7.clone()
    };
    let plan = build::alter(&od(&l7), &od(&all), &cx.ctx()).unwrap();
    assert!(plan.statements.last().unwrap().sql.contains(" OWNER TO "), "{:?}", sqls(&plan));
    db_alter_and_check(&cx, &l7, &all).await;
    eprintln!("PG {}: bakım bağlantısından başka veritabanında COMMENT/OWNER/SET tamam", cx.major());
}

docker_test!(database_unchanged_edit_is_empty_plan, db_noop_body, "dbnoop");

async fn db_noop_body(ts: TestServer, tag: &str) {
    let cx = Cx::new(&ts, tag).await;
    let name = cx.db("");
    let q = quote_ident(&name);
    cx.sql(&format!("CREATE DATABASE {q} TEMPLATE template0 ENCODING 'UTF8' LC_COLLATE 'C' LC_CTYPE 'C' CONNECTION LIMIT 4")).await;
    // Sunucu kanonik büyük/küçük harfli adları saklar (TimeZone, DateStyle); yükleyici küçük harf döndürür.
    cx.sql(&format!(
        "ALTER DATABASE {q} SET \"TimeZone\" = 'UTC'; ALTER DATABASE {q} SET DateStyle = 'ISO, DMY'; \
         ALTER DATABASE {q} SET search_path = '$user', 'public'; ALTER DATABASE {q} SET work_mem = '64MB'; \
         COMMENT ON DATABASE {q} IS 'n'"
    ))
    .await;
    let l = cx.load_db(&name).await;
    let names: Vec<&str> = l.settings.iter().map(|s| s.name.as_str()).collect();
    assert_eq!(names, ["datestyle", "search_path", "timezone", "work_mem"]);
    assert_eq!(l.settings[1].value, "\"$user\", public");
    assert_db_noop(&cx, &l);
    // Yalnızca yorum değişince plan yalnızca COMMENT; küçük harf adla RESET çalışır.
    let p = build::alter(&od(&l), &od(&DatabaseDef { comment: Some("m".into()), ..l.clone() }), &cx.ctx()).unwrap();
    assert_eq!(p.statements.len(), 1, "{:?}", sqls(&p));
    let new = DatabaseDef {
        settings: l.settings.iter().filter(|s| s.name != "timezone" && s.name != "datestyle").cloned().collect(),
        ..l.clone()
    };
    db_alter_and_check(&cx, &l, &new).await;
}

docker_test!(database_rename_blocked_by_session_reports_55006, db_busy_body, "dbbusy");

async fn db_busy_body(ts: TestServer, tag: &str) {
    let cx = Cx::new(&ts, tag).await;
    let l = db_create_and_check(&cx, &database(&cx.db(""))).await;
    let session = admin_client(&cx.ts, &l.name).await;
    session.query_one("SELECT 1", &[]).await.unwrap();
    let plan = build::alter(&od(&l), &od(&DatabaseDef { name: cx.db("ren"), ..l.clone() }), &cx.ctx()).unwrap();
    let e = cx.try_run_nt(&plan).await.expect_err("bağlı oturum varken RENAME başarısız olmalı");
    assert_eq!((e.index, e.code.as_str()), (0, "55006"), "{e:?}");
    assert!(cx.db_oid(&l.name).await.is_some() && cx.db_oid(&cx.db("ren")).await.is_none(), "ad aynı kalmalı");
    // Bağlı oturum varken DROP da 55006 (FORCE'suz).
    let drop_plan = build::drop(&od(&l), false, &cx.ctx()).unwrap();
    assert_eq!(cx.try_run_nt(&drop_plan).await.unwrap_err().code, "55006");
    drop_session(session).await;
}

async fn drop_session(c: Client) {
    drop(c);
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;
}

docker_test!(database_drop_flows, db_drop_body, "dbdrop");

async fn db_drop_body(ts: TestServer, tag: &str) {
    let cx = Cx::new(&ts, tag).await;
    let l = db_create_and_check(&cx, &database(&cx.db("plain"))).await;
    let plan = build::drop(&od(&l), false, &cx.ctx()).unwrap();
    assert_eq!(plan.statements[0].risk, Risk::DataLoss);
    cx.run_nt(&plan).await;
    assert!(cx.db_oid(&l.name).await.is_none());
    // Şablon veritabanı: yerel hata (sunucuya gitmez).
    let t = db_create_and_check(&cx, &DatabaseDef { is_template: true, ..database(&cx.db("tpl")) }).await;
    assert_eq!(build::drop(&od(&t), false, &cx.ctx()).unwrap_err()[0].path, "isTemplate");
    // WITH (FORCE): PG 13+ bağlı oturumu sonlandırır; öncesinde sürüm hatası.
    let f = db_create_and_check(&cx, &database(&cx.db("force"))).await;
    let force = build::drop_with(&od(&f), DropOptions { cascade: false, force: true }, &cx.ctx());
    if cx.version < 130000 {
        assert_eq!(force.unwrap_err()[0].path, "force");
        eprintln!("PG {}: DROP DATABASE … WITH (FORCE) atlandı (PG 13+)", cx.major());
        return;
    }
    let session = admin_client(&cx.ts, &f.name).await;
    session.query_one("SELECT 1", &[]).await.unwrap();
    cx.run_nt(&force.unwrap()).await;
    assert!(cx.db_oid(&f.name).await.is_none());
    assert!(session.query_one("SELECT 1", &[]).await.is_err(), "FORCE oturumu sonlandırmalı");
}

// ================================================================================================================
// Role
// ================================================================================================================

fn role(name: &str) -> RoleDef {
    RoleDef {
        name: name.into(),
        login: false,
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

fn member(role: &str, admin: bool) -> RoleMembership {
    RoleMembership { role: role.into(), admin, inherit_option: None, set_option: None }
}

fn or(d: &RoleDef) -> ObjectDef {
    ObjectDef::Role(d.clone())
}

/// Beklenen rol: parola `keep`, üyelikler ad sırasıyla; PG 16+ seçenekleri yüklenenden (ayrıca `assert_options`).
fn role_expected(def: &RoleDef, loaded: &RoleDef) -> RoleDef {
    let mut member_of: Vec<RoleMembership> = def
        .member_of
        .iter()
        .map(|m| {
            let l = loaded.member_of.iter().find(|x| x.role == m.role);
            RoleMembership {
                inherit_option: l.and_then(|l| l.inherit_option),
                set_option: l.and_then(|l| l.set_option),
                ..m.clone()
            }
        })
        .collect();
    member_of.sort_by(|a, b| a.role.cmp(&b.role));
    RoleDef {
        password: PasswordChange::Keep,
        member_of,
        comment: def.comment.clone().filter(|c| !c.is_empty()),
        ..def.clone()
    }
}

/// PG 16+: tüm üyeliklerde seçenekler dolu; öncesinde boş.
fn assert_options(cx: &Cx, r: &RoleDef) {
    for m in &r.member_of {
        if cx.version >= 160000 {
            assert!(m.inherit_option.is_some() && m.set_option.is_some(), "PG 16+ seçenekler dolu olmalı: {m:?}");
        } else {
            assert!(m.inherit_option.is_none() && m.set_option.is_none(), "PG < 16 seçenek yok: {m:?}");
        }
    }
}

fn assert_role_noop(cx: &Cx, r: &RoleDef) {
    let p = build::alter(&or(r), &or(r), &cx.ctx()).unwrap_or_else(|e| panic!("noop reddedildi: {e:?}\n{r:?}"));
    assert!(p.statements.is_empty() && p.warnings.is_empty(), "{}: boş plan bekleniyordu: {:?} {:?}", r.name, sqls(&p), p.warnings);
}

async fn role_create_and_check(cx: &Cx, def: &RoleDef) -> RoleDef {
    let plan = build::create(&or(def), &cx.ctx()).unwrap_or_else(|e| panic!("create reddedildi: {} {e:?}", def.name));
    cx.run_tx(&plan).await;
    let loaded = cx.load_role(&def.name).await;
    assert_eq!(loaded, role_expected(def, &loaded), "create → load ({})", def.name);
    assert_options(cx, &loaded);
    assert_role_noop(cx, &loaded);
    assert_eq!(cx.load_role(&def.name).await, loaded, "iki yükleme aynı olmalı");
    loaded
}

async fn role_alter_and_check(cx: &Cx, old: &RoleDef, new: &RoleDef) -> RoleDef {
    let plan = build::alter(&or(old), &or(new), &cx.ctx()).unwrap_or_else(|e| panic!("alter reddedildi: {e:?}\n  old: {old:?}\n  new: {new:?}"));
    assert!(!plan.statements.is_empty(), "boş olmayan plan bekleniyordu: {new:?}");
    cx.run_tx(&plan).await;
    let loaded = cx.load_role(&new.name).await;
    assert_eq!(loaded, role_expected(new, &loaded), "alter → load; plan: {:?}", sqls(&plan));
    assert_options(cx, &loaded);
    assert_role_noop(cx, &loaded);
    loaded
}

docker_test!(role_create_then_load_matches, role_create_body, "rcreate");

async fn role_create_body(ts: TestServer, tag: &str) {
    let cx = Cx::new(&ts, tag).await;
    let (g1, g2) = (cx.role("g1"), cx.role("g2"));
    cx.sql(&format!("CREATE ROLE {g1}; CREATE ROLE {g2}")).await;
    let defs = vec![
        role(&cx.role("plain")),
        // Tüm öznitelikler açık (PG 10'da BYPASSRLS dahil; test bağlantısı süper kullanıcı).
        RoleDef {
            login: true,
            superuser: true,
            createdb: true,
            createrole: true,
            inherit: false,
            replication: true,
            bypassrls: true,
            connection_limit: 5,
            valid_until: Some("2030-01-02T03:04:05Z".into()),
            ..role(&cx.role("all"))
        },
        RoleDef { login: true, valid_until: Some("2031-06-30T23:59:59.123400Z".into()), ..role(&cx.role("frac")) },
        RoleDef { valid_until: Some("infinity".into()), connection_limit: 0, ..role(&cx.role("inf")) },
        RoleDef {
            member_of: vec![member(&g2, false), member(&g1, true), member("pg_read_all_stats", false)],
            settings: vec![setting("search_path", "\"$user\", public"), setting("work_mem", "64MB")],
            comment: Some("yorum; -- 'x' /* y */ \\".into()),
            ..role(&cx.role("members"))
        },
        RoleDef { inherit: false, member_of: vec![member(&g1, false)], ..role(&cx.role("noinh")) },
        RoleDef { comment: Some("q".into()), ..role(&format!("{}_Q \"x\"; --'", cx.role_base)) },
    ];
    for d in &defs {
        let l = role_create_and_check(&cx, d).await;
        assert_eq!(l.password, PasswordChange::Keep);
        assert!(l.unsupported.is_empty(), "{:?}", l.unsupported);
        if cx.version >= 160000 {
            // PG 16+ varsayılanları: INHERIT seçeneği üyenin rolinherit'i, SET true.
            for m in &l.member_of {
                assert_eq!((m.inherit_option, m.set_option), (Some(d.inherit), Some(true)), "{m:?}");
            }
        }
    }
    eprintln!("PG {}: VALID UNTIL 'infinity' ve BYPASSRLS gidiş-dönüş tamam", cx.major());
    // Saat dilimli giriş UTC'ye normalleşir; yüklü biçim üretici dilbilgisini karşılar.
    let tz = RoleDef { valid_until: Some("2031-06-30 12:00:00+03".into()), ..role(&cx.role("tz")) };
    cx.run_tx(&build::create(&or(&tz), &cx.ctx()).unwrap()).await;
    let l = cx.load_role(&tz.name).await;
    assert_eq!(l.valid_until.as_deref(), Some("2031-06-30T09:00:00Z"));
    assert!(build::role::valid_until_ok(l.valid_until.as_deref().unwrap()));
    assert_role_noop(&cx, &l);
}

docker_test!(role_alter_round_trips, role_alter_body, "ralter");

async fn role_alter_body(ts: TestServer, tag: &str) {
    let cx = Cx::new(&ts, tag).await;
    let (g1, g2, g3) = (cx.role("g1"), cx.role("g2"), cx.role("g3"));
    cx.sql(&format!("CREATE ROLE {g1}; CREATE ROLE {g2}; CREATE ROLE {g3}")).await;
    let mut cur = role_create_and_check(&cx, &role(&cx.role(""))).await;
    // Her öznitelik tek tek açılır, sonra hepsi birden kapanır.
    type Flag = fn(&mut RoleDef) -> &mut bool;
    let flags: [Flag; 7] = [
        |r| &mut r.login,
        |r| &mut r.superuser,
        |r| &mut r.createdb,
        |r| &mut r.createrole,
        |r| &mut r.inherit,
        |r| &mut r.replication,
        |r| &mut r.bypassrls,
    ];
    for f in flags {
        let mut n = cur.clone();
        let v = f(&mut n);
        *v = !*v;
        cur = role_alter_and_check(&cx, &cur, &n).await;
    }
    let mut n = cur.clone();
    for f in flags {
        *f(&mut n) = false;
    }
    cur = role_alter_and_check(&cx, &cur, &n).await;
    cur = role_alter_and_check(&cx, &cur, &RoleDef { connection_limit: 10, valid_until: Some("2030-01-01T00:00:00Z".into()), ..cur.clone() }).await;
    cur = role_alter_and_check(&cx, &cur, &RoleDef { valid_until: Some("infinity".into()), ..cur.clone() }).await;
    cur = role_alter_and_check(&cx, &cur, &RoleDef { valid_until: Some("2029-12-31T23:00:00.500000Z".into()), ..cur.clone() }).await;
    cur = role_alter_and_check(&cx, &cur, &RoleDef { name: cx.role("ren"), ..cur.clone() }).await;
    // Üyelikler: ekle, çıkar, admin iki yön.
    cur = role_alter_and_check(&cx, &cur, &RoleDef { member_of: vec![member(&g1, false), member(&g2, true)], ..cur.clone() }).await;
    cur = role_alter_and_check(&cx, &cur, &RoleDef { member_of: vec![member(&g2, false), member(&g3, true)], ..cur.clone() }).await;
    cur = role_alter_and_check(&cx, &cur, &RoleDef { member_of: vec![member(&g2, true), member(&g3, true)], ..cur.clone() }).await;
    // Ayarlar ve yorum.
    cur = role_alter_and_check(&cx, &cur, &RoleDef { settings: vec![setting("work_mem", "8MB")], comment: Some("a".into()), ..cur.clone() }).await;
    cur = role_alter_and_check(
        &cx,
        &cur,
        &RoleDef {
            settings: vec![setting("search_path", "\"$user\", public"), setting("work_mem", "16MB")],
            comment: Some("b 'c'".into()),
            ..cur.clone()
        },
    )
    .await;
    cur = role_alter_and_check(&cx, &cur, &RoleDef { settings: vec![setting("search_path", "\"$user\", public")], comment: None, ..cur.clone() }).await;
    // Hepsi birden.
    let all = RoleDef {
        name: cx.role("ren2"),
        login: true,
        connection_limit: 2,
        member_of: vec![member(&g1, true)],
        settings: vec![],
        comment: Some("son".into()),
        ..cur.clone()
    };
    role_alter_and_check(&cx, &cur, &all).await;
}

docker_test!(role_unchanged_edit_is_empty_plan, role_noop_body, "rnoop");

async fn role_noop_body(ts: TestServer, tag: &str) {
    let cx = Cx::new(&ts, tag).await;
    let (g1, g2) = (cx.role("g1"), cx.role("g2"));
    let r = cx.role("");
    let q = quote_ident(&r);
    cx.sql(&format!(
        "CREATE ROLE {g1}; CREATE ROLE {g2}; \
         CREATE ROLE {q} LOGIN NOINHERIT CONNECTION LIMIT 3 VALID UNTIL '2033-03-03 03:03:03.000007+00'; \
         GRANT {g1} TO {q} WITH ADMIN OPTION; GRANT {g2} TO {q}; GRANT pg_monitor TO {q}; \
         ALTER ROLE {q} SET \"TimeZone\" = 'UTC'; ALTER ROLE {q} SET DateStyle = 'ISO, DMY'; \
         ALTER ROLE {q} SET search_path = '$user', 'public'; COMMENT ON ROLE {q} IS 'n'"
    ))
    .await;
    let l = cx.load_role(&r).await;
    assert_eq!(l.valid_until.as_deref(), Some("2033-03-03T03:03:03.000007Z"));
    let names: Vec<&str> = l.settings.iter().map(|s| s.name.as_str()).collect();
    assert_eq!(names, ["datestyle", "search_path", "timezone"]);
    let members: Vec<(&str, bool)> = l.member_of.iter().map(|m| (m.role.as_str(), m.admin)).collect();
    assert_eq!(members, [("pg_monitor", false), (g1.as_str(), true), (g2.as_str(), false)]);
    assert_options(&cx, &l);
    assert_role_noop(&cx, &l);
    // Küçük harf adla RESET çalışır.
    let new = RoleDef { settings: l.settings.iter().filter(|s| s.name == "search_path").cloned().collect(), ..l.clone() };
    role_alter_and_check(&cx, &l, &new).await;
}

docker_test!(role_pg16_grant_options_are_read_only_and_preserved, role_pg16_body, "rpg16");

async fn role_pg16_body(ts: TestServer, tag: &str) {
    let cx = Cx::new(&ts, tag).await;
    let (g, m, gr, m2) = (cx.role("g"), cx.role("m"), cx.role("gr"), cx.role("m2"));
    cx.sql(&format!("CREATE ROLE {g}; CREATE ROLE {m}; CREATE ROLE {gr}; CREATE ROLE {m2}; GRANT {g} TO {gr} WITH ADMIN OPTION"))
        .await;
    if cx.version < 160000 {
        eprintln!("PG {}: üyelik başına INHERIT/SET seçenekleri atlandı (PG 16+)", cx.major());
    } else {
        cx.sql(&format!("GRANT {g} TO {m} WITH INHERIT FALSE, SET FALSE")).await;
        let l = cx.load_role(&m).await;
        assert_eq!(l.member_of, [RoleMembership { role: g.clone(), admin: false, inherit_option: Some(false), set_option: Some(false) }]);
        assert_role_noop(&cx, &l);
        // ADMIN değişimi diğer seçeneklere dokunmaz (iki yön).
        let mut n = l.clone();
        n.member_of[0].admin = true;
        let l2 = role_alter_and_check(&cx, &l, &n).await;
        assert_eq!((l2.member_of[0].inherit_option, l2.member_of[0].set_option), (Some(false), Some(false)));
        let mut n = l2.clone();
        n.member_of[0].admin = false;
        let l3 = role_alter_and_check(&cx, &l2, &n).await;
        assert_eq!(l3.member_of, l.member_of);
    }
    // Başka grantor'ın verdiği üyeliği süper kullanıcı REVOKE ederse (GRANTED BY'sız):
    cx.sql_as(&gr, &format!("GRANT {g} TO {m2}")).await;
    let l = cx.load_role(&m2).await;
    assert_eq!(l.member_of.len(), 1, "{l:?}");
    let plan = build::alter(&or(&l), &or(&RoleDef { member_of: vec![], ..l.clone() }), &cx.ctx()).unwrap();
    cx.run_tx(&plan).await;
    let after = cx.load_role(&m2).await;
    if cx.version >= 160000 {
        // PG 16+: REVOKE yalnızca bağlı rolün (süper kullanıcıda önyükleme süper kullanıcısının) verdiği satırı kaldırır;
        // başka grantor'ın satırı kalır — sunucu WARNING verir, hata yok (plan "başarılı" görünür).
        assert_eq!(after.member_of.len(), 1, "PG 16+: başka grantor'ın üyeliği kalmalı: {after:?}");
        eprintln!("PG {}: başka grantor'ın üyeliğinde REVOKE = WARNING + etkisiz (üyelik duruyor)", cx.major());
    } else {
        assert!(after.member_of.is_empty(), "PG < 16: REVOKE grantor'dan bağımsız kaldırır: {after:?}");
        eprintln!("PG {}: başka grantor'ın üyeliğinde REVOKE üyeliği kaldırdı", cx.major());
    }
}

docker_test!(role_unsupported_settings_and_multi_grantor, role_unsupported_body, "runsup");

async fn role_unsupported_body(ts: TestServer, tag: &str) {
    let cx = Cx::new(&ts, tag).await;
    let (r, g, gr) = (cx.role(""), cx.role("g"), cx.role("gr"));
    let maint = quote_ident(&cx.maint);
    cx.sql(&format!(
        "CREATE ROLE {r}; CREATE ROLE {g}; CREATE ROLE {gr}; GRANT {g} TO {gr} WITH ADMIN OPTION; \
         ALTER ROLE {r} IN DATABASE {maint} SET work_mem = '8MB'; ALTER ROLE {r} SET work_mem = '4MB'"
    ))
    .await;
    let l = cx.load_role(&r).await;
    let in_db = format!("IN DATABASE {}: work_mem=8MB", cx.maint);
    assert_eq!(l.settings, [setting("work_mem", "4MB")]);
    assert_eq!(l.unsupported, std::slice::from_ref(&in_db));
    assert_role_noop(&cx, &l);
    // Başka düzenlemeler desteklenmeyen ayara dokunmaz.
    let l2 = role_alter_and_check(&cx, &l, &RoleDef { comment: Some("x".into()), ..l.clone() }).await;
    assert_eq!(l2.unsupported, std::slice::from_ref(&in_db));
    let l3 = role_alter_and_check(&cx, &l2, &RoleDef { settings: vec![], ..l2.clone() }).await;
    assert_eq!(l3.unsupported, std::slice::from_ref(&in_db));
    if cx.version < 160000 {
        eprintln!("PG {}: birden çok grantor atlandı (PG 16+)", cx.major());
        return;
    }
    // PG 16+: aynı (rol, üye) için iki grantor → MEMBER … unsupported, düzenlenmez.
    cx.sql(&format!("GRANT {g} TO {r}")).await;
    cx.sql_as(&gr, &format!("GRANT {g} TO {r}")).await;
    let l4 = cx.load_role(&r).await;
    assert!(l4.member_of.is_empty(), "{l4:?}");
    assert_eq!(l4.unsupported, [format!("MEMBER {g} (birden çok grantor)"), in_db.clone()]);
    assert_role_noop(&cx, &l4);
    let l5 = role_alter_and_check(&cx, &l4, &RoleDef { comment: Some("y".into()), ..l4.clone() }).await;
    assert_eq!(l5.unsupported, l4.unsupported);
}

docker_test!(role_drop_with_owned_objects_fails_cleanly, role_drop_body, "rdrop");

async fn role_drop_body(ts: TestServer, tag: &str) {
    let cx = Cx::new(&ts, tag).await;
    let (r, free) = (cx.role(""), cx.role("free"));
    let db = cx.db("");
    cx.sql(&format!("CREATE ROLE {r}; CREATE ROLE {free}")).await;
    // CREATE DATABASE çok ifadeli dizgede (örtük transaction) çalışamaz: ayrı gönderilir.
    cx.sql(&format!("CREATE DATABASE {db}")).await;
    let in_db = admin_client(&cx.ts, &db).await;
    in_db.batch_execute(&format!("CREATE TABLE owned_t (id int); ALTER TABLE owned_t OWNER TO {r}")).await.unwrap();
    drop_session(in_db).await;
    let l = cx.load_role(&r).await;
    let plan = build::drop(&or(&l), false, &cx.ctx()).unwrap();
    assert_eq!(plan.statements[0].risk, Risk::Destructive);
    let e = cx.try_run_tx(&plan).await.expect_err("sahip olunan nesne varken DROP ROLE başarısız olmalı");
    assert_eq!(e.code, "2BP01", "{e:?}");
    assert!(e.message.contains(&r), "ileti rolü anmalı: {e:?}");
    assert!(e.detail.as_deref().is_some_and(|d| d.contains(&db)), "ayrıntı veritabanını anmalı: {e:?}");
    assert!(cx.role_oid(&r).await.is_some(), "rol durmalı");
    // CASCADE yok (yerel hata).
    assert_eq!(build::drop(&or(&l), true, &cx.ctx()).unwrap_err()[0].path, "cascade");
    // Bağımlılığı olmayan rol silinir; yükleme NotFound.
    let f = cx.load_role(&free).await;
    let oid = cx.role_oid(&free).await.unwrap();
    cx.run_tx(&build::drop(&or(&f), false, &cx.ctx()).unwrap()).await;
    assert!(matches!(cx.try_load(ObjectKind::Role, oid).await, Err(Error::NotFound(_))));
}

docker_test!(role_rename_clears_md5_only, role_md5_body, "rmd5");

async fn role_md5_body(ts: TestServer, tag: &str) {
    let cx = Cx::new(&ts, tag).await;
    let (md5, scram) = (cx.role("md5"), cx.role("scram"));
    cx.sql(&format!("CREATE ROLE {md5} LOGIN; CREATE ROLE {scram} LOGIN")).await;
    cx.secret_sql(&format!(
        "SET password_encryption = 'md5'; ALTER ROLE {md5} PASSWORD {}; \
         SET password_encryption = 'scram-sha-256'; ALTER ROLE {scram} PASSWORD {}; RESET password_encryption",
        quote_literal(&random_password()),
        quote_literal(&random_password())
    ))
    .await;
    // Yalnızca "boş mu" okunur (süper kullanıcı test bağlantısı); değer hiç alınmaz.
    let has_password = |name: String| {
        let c = &cx.admin;
        async move {
            c.query_one("SELECT rolpassword IS NOT NULL FROM pg_authid WHERE rolname = $1::text", &[&name])
                .await
                .unwrap()
                .get::<_, bool>(0)
        }
    };
    assert!(has_password(md5.clone()).await && has_password(scram.clone()).await);
    for (old, new) in [(&md5, cx.role("md5_r")), (&scram, cx.role("scram_r"))] {
        let l = cx.load_role(old).await;
        let plan = build::alter(&or(&l), &or(&RoleDef { name: new.clone(), ..l.clone() }), &cx.ctx()).unwrap();
        assert!(plan.warnings.iter().any(|w| w == build::role::MD5_RENAME_WARNING));
        role_alter_and_check(&cx, &l, &RoleDef { name: new, ..l.clone() }).await;
    }
    assert!(!has_password(cx.role("md5_r")).await, "MD5 parolası ad değişince temizlenmeli");
    assert!(has_password(cx.role("scram_r")).await, "SCRAM parolası korunmalı");
    eprintln!("PG {}: RENAME yalnızca MD5 parolasını temizledi", cx.major());
}

docker_test!(loaders_work_without_superuser, nonsuper_body, "rnosu");

async fn nonsuper_body(ts: TestServer, tag: &str) {
    let cx = Cx::new(&ts, tag).await;
    let (u, g, other) = (cx.role("u"), cx.role("g"), cx.role("other"));
    let db = cx.db("");
    let password = random_password();
    cx.sql(&format!(
        "CREATE ROLE {g}; CREATE ROLE {u} LOGIN NOSUPERUSER; \
         CREATE ROLE {other} VALID UNTIL 'infinity' CONNECTION LIMIT 2; GRANT {g} TO {other} WITH ADMIN OPTION; \
         ALTER ROLE {other} SET work_mem = '8MB'; COMMENT ON ROLE {other} IS 'o'"
    ))
    .await;
    cx.sql(&format!("CREATE DATABASE {db} CONNECTION LIMIT 5")).await;
    cx.sql(&format!("ALTER DATABASE {db} SET work_mem = '2MB'; COMMENT ON DATABASE {db} IS 'd'")).await;
    cx.secret_sql(&format!("SET password_encryption = 'scram-sha-256'; ALTER ROLE {u} PASSWORD {}; RESET password_encryption", quote_literal(&password)))
        .await;
    let mut config = cx.ts.config.clone();
    config.username = u.clone();
    let mgr = ConnectionManager::new();
    if mgr.connect(config.clone(), Some(password)).await.is_err() {
        panic!("süper kullanıcı olmayan rolle bağlanılamadı");
    }
    for (kind, oid) in [
        (ObjectKind::Role, cx.role_oid(&other).await.unwrap()),
        (ObjectKind::Role, cx.role_oid(&u).await.unwrap()),
        (ObjectKind::Database, cx.db_oid(&db).await.unwrap()),
        (ObjectKind::Database, cx.maint_oid),
    ] {
        let as_user = load(&mgr, config.id, 0, ObjectRef { kind, oid }).await.unwrap_or_else(|e| panic!("{kind:?} yüklenemedi: {e:?}"));
        let as_super = cx.try_load(kind, oid).await.unwrap();
        assert_eq!(as_user, as_super, "süper kullanıcı olmayan yükleme aynı olmalı");
    }
    mgr.disconnect(config.id);
}

docker_test!(system_role_is_refused, system_role_body, "rsys");

async fn system_role_body(ts: TestServer, tag: &str) {
    let cx = Cx::new(&ts, tag).await;
    let oid = cx.role_oid("pg_monitor").await.unwrap();
    assert!(matches!(cx.try_load(ObjectKind::Role, oid).await, Err(Error::Invalid(m)) if m.contains("sistem rolü")));
    assert!(matches!(cx.try_load(ObjectKind::Role, 0).await, Err(Error::NotFound(_))));
    assert!(matches!(cx.try_load(ObjectKind::Database, 0).await, Err(Error::NotFound(_))));
}

// ================================================================================================================
// Açılır listeler
// ================================================================================================================

docker_test!(new_lookups_return_expected_shapes, lookups_body, "lookups");

async fn lookups_body(ts: TestServer, tag: &str) {
    let cx = Cx::new(&ts, tag).await;
    let get = |req: LookupRequest| lookup(&cx.mgr, cx.server, cx.maint_oid, req);
    let detail = |items: &[LookupItem], v: &str| items.iter().find(|i| i.value == v).map(|i| i.detail.clone());

    let roles = get(LookupRequest::GrantableRoles).await.unwrap();
    assert_eq!(detail(&roles, "pg_monitor"), Some(Some("önceden tanımlı".into())));
    assert_eq!(detail(&roles, &cx.me), Some(Some("superuser".into())));

    let enc = get(LookupRequest::Encodings).await.unwrap();
    assert!(detail(&enc, "UTF8").is_some() && detail(&enc, "SJIS").is_none());
    assert_eq!(enc.len(), build::database::SERVER_ENCODINGS.len());

    let tpl = get(LookupRequest::TemplateDatabases).await.unwrap();
    assert_eq!(detail(&tpl, "template1"), Some(Some("şablon".into())));
    assert_eq!(detail(&tpl, "template0"), Some(Some("şablon".into())));
    assert_eq!(detail(&tpl, &cx.maint), Some(None));

    let libc = get(LookupRequest::Locales { provider: LocaleProvider::Libc }).await.unwrap();
    assert!(detail(&libc, "C").is_some() && detail(&libc, "POSIX").is_some(), "{libc:?}");
    assert!(libc.iter().all(|i| !i.value.is_empty() && !i.value.chars().any(char::is_control)));
    let icu = get(LookupRequest::Locales { provider: LocaleProvider::Icu }).await.unwrap();
    if cx.version < 150000 {
        assert!(icu.is_empty(), "PG < 15: ICU veritabanı sağlayıcısı yok");
    } else if cx.has_icu().await {
        assert!(!icu.is_empty());
    } else {
        eprintln!("PG {}: ICU yok, Locales{{icu}} denetimi atlandı", cx.major());
    }
    let builtin = get(LookupRequest::Locales { provider: LocaleProvider::Builtin }).await.unwrap();
    if cx.version < 170000 {
        assert!(builtin.is_empty(), "PG < 17: builtin yok");
    } else {
        assert!(detail(&builtin, "C").is_some(), "{builtin:?}");
    }

    let settings = get(LookupRequest::Settings).await.unwrap();
    assert!(detail(&settings, "work_mem").is_some_and(|d| d.is_some()));
    assert!(detail(&settings, "shared_buffers").is_none());
    assert!(detail(&settings, "timezone").is_some(), "adlar küçük harf");
    assert!(settings.iter().all(|s| s.value == s.value.to_lowercase()));
    eprintln!(
        "PG {}: lookup boyutları roller {} kodlama {} şablon {} libc {} icu {} builtin {} ayar {}",
        cx.major(),
        roles.len(),
        enc.len(),
        tpl.len(),
        libc.len(),
        icu.len(),
        builtin.len(),
        settings.len()
    );
}
