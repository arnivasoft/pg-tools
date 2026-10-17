#![allow(dead_code)]
//! Docker entegrasyon testleri için ortak yardımcılar (`PGT_TEST_URLS` yoksa test atlanır).

use std::future::Future;
use std::sync::Arc;

use pgt_core::connection::ConnectionManager;
use pgt_core::testing::{TestServer, admin_client, drop_database, test_servers};
use pgt_ddl::Plan;
use tokio_postgres::Client;
use uuid::Uuid;

pub fn servers() -> Vec<TestServer> {
    let servers = test_servers();
    if servers.is_empty() {
        eprintln!("PGT_TEST_URLS tanımlı değil, test atlanıyor");
    }
    servers
}

/// Gövdeyi ayrı görevde çalıştırır; bir doğrulama başarısız olsa bile `cleanup` çalışır, sonra panik yeniden fırlatılır.
/// Temizlik de paniklerse gövdenin paniği öncelikli (asıl hata gizlenmez); temizlik paniği yalnızca yazdırılır.
pub async fn guarded(body: impl Future<Output = ()> + Send + 'static, cleanup: impl Future<Output = ()>) {
    let result = tokio::spawn(body).await;
    let cleaned = CatchUnwind(Box::pin(cleanup)).await;
    match (result, cleaned) {
        (Err(e), cleaned) => {
            if cleaned.is_err() {
                eprintln!("temizlik de başarısız oldu (gövde hatası öncelikli)");
            }
            std::panic::resume_unwind(e.into_panic());
        }
        (Ok(()), Err(c)) => std::panic::resume_unwind(c),
        (Ok(()), Ok(())) => {}
    }
}

/// Ödünç alan (spawn edilemeyen) geleceğin paniğini yakalar (ek bağımlılık olmadan `FutureExt::catch_unwind`).
struct CatchUnwind<F>(std::pin::Pin<Box<F>>);

impl<F: Future> Future for CatchUnwind<F> {
    type Output = std::thread::Result<F::Output>;

    fn poll(mut self: std::pin::Pin<&mut Self>, cx: &mut std::task::Context<'_>) -> std::task::Poll<Self::Output> {
        let inner = &mut self.0;
        match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| inner.as_mut().poll(cx))) {
            Ok(std::task::Poll::Pending) => std::task::Poll::Pending,
            Ok(std::task::Poll::Ready(v)) => std::task::Poll::Ready(Ok(v)),
            Err(e) => std::task::Poll::Ready(Err(e)),
        }
    }
}

/// `tokio_postgres::Error` yalnızca "db error" yazar; sunucu iletisini çıkarır.
pub fn db_message(e: &tokio_postgres::Error) -> String {
    e.as_db_error().map(|d| format!("{} ({})", d.message(), d.code().code())).unwrap_or_else(|| e.to_string())
}

pub async fn version_num(ts: &TestServer) -> u32 {
    let admin = admin_client(ts, &ts.config.maintenance_db).await;
    let v: i32 = admin.query_one("SELECT current_setting('server_version_num')::int4", &[]).await.unwrap().get(0);
    v as u32
}

/// Nesne testlerinin varsayılan ad öneki (`tests/roundtrip.rs`); tablo testleri `pgt_ddl_tb` kullanır.
pub const RT_PREFIX: &str = "pgt_ddl_rt";

/// Geçici veritabanı adı: `pgt_ddl_rt_<ana sürüm>_<etiket>` (testler paralel koşar; her test kendi veritabanını alır).
pub async fn db_name(ts: &TestServer, tag: &str) -> String {
    db_name_in(ts, RT_PREFIX, tag).await
}

/// Önekli veritabanı adı: `<önek>_<ana sürüm>_<etiket>`.
pub async fn db_name_in(ts: &TestServer, prefix: &str, tag: &str) -> String {
    format!("{prefix}_{}_{tag}", version_num(ts).await / 10_000)
}

pub fn role_name(tag: &str) -> String {
    role_name_in(RT_PREFIX, tag)
}

pub fn role_name_in(prefix: &str, tag: &str) -> String {
    format!("{prefix}_role_{tag}")
}

/// Veritabanını ve test rolünü siler (önce veritabanı: rol nesne sahibi olabilir).
pub async fn cleanup(ts: &TestServer, tag: &str) {
    cleanup_in(ts, RT_PREFIX, tag).await
}

pub async fn cleanup_in(ts: &TestServer, prefix: &str, tag: &str) {
    let admin = admin_client(ts, &ts.config.maintenance_db).await;
    drop_database(&admin, &db_name_in(ts, prefix, tag).await).await;
    admin.batch_execute(&format!("DROP ROLE IF EXISTS {}", role_name_in(prefix, tag))).await.unwrap();
}

// ---- küme (sunucu kapsamlı) nesneleri: veritabanı ve rol ----

/// Küme nesnesi adı: veritabanı `pgt_ddl_dbk_<ana sürüm>_<etiket>`, rol `pgt_ddl_rk_<etiket>` (her sürüm ayrı
/// kapsayıcıdır). Testler paralel koşar: etiketler benzersizdir; temizlik bu adı ve `<ad>_…` türevlerini siler.
pub async fn cluster_name(ts: &TestServer, kind: pgt_ddl::ObjectKind, tag: &str) -> String {
    // `_` içeren etiket başka etiketin `<ad>_…` türevi gibi görünür ve onun temizliğine girer.
    assert!(!tag.contains('_'), "küme etiketi '_' içeremez: {tag}");
    match kind {
        pgt_ddl::ObjectKind::Database => db_name_in(ts, "pgt_ddl_dbk", tag).await,
        pgt_ddl::ObjectKind::Role => format!("pgt_ddl_rk_{tag}"),
        other => panic!("küme nesnesi değil: {other:?}"),
    }
}

/// `base` ya da `base_…` adlı nesneler (LIKE değil: `_` joker sayılmasın).
const BY_PREFIX: &str = "$1 = $2 OR left($1, length($2) + 1) = $2 || '_'";

/// Veritabanını (varsa) siler: şablonsa önce şablonluktan çıkarır, oturumlarını sonlandırıp bekler, 55006'da yeniden
/// dener. Ad `quote_ident` ile yazılır (test adları tırnak gerektirebilir).
pub async fn drop_test_database(admin: &Client, name: &str) {
    let q = pgt_ddl::quote_ident(name);
    let exists = !admin.query("SELECT 1 FROM pg_database WHERE datname = $1::text", &[&name]).await.unwrap().is_empty();
    if !exists {
        return;
    }
    admin.batch_execute(&format!("ALTER DATABASE {q} IS_TEMPLATE false")).await.unwrap();
    const TERMINATE: &str =
        "SELECT pg_terminate_backend(pid) FROM pg_stat_activity WHERE datname = $1::text AND pid <> pg_backend_pid()";
    const REMAINING: &str = "SELECT count(*) FROM pg_stat_activity WHERE datname = $1::text AND pid <> pg_backend_pid()";
    for attempt in 0..5 {
        admin.execute(TERMINATE, &[&name]).await.unwrap();
        for _ in 0..50 {
            let left: i64 = admin.query_one(REMAINING, &[&name]).await.unwrap().get(0);
            if left == 0 {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        }
        match admin.batch_execute(&format!("DROP DATABASE IF EXISTS {q}")).await {
            Ok(()) => return,
            Err(e) if attempt < 4 && e.code() == Some(&tokio_postgres::error::SqlState::OBJECT_IN_USE) => {
                tokio::time::sleep(std::time::Duration::from_millis(200)).await;
            }
            Err(e) => panic!("{name} silinemedi: {}", db_message(&e)),
        }
    }
}

/// Rolü (varsa) siler: önce bakım veritabanındaki sahiplik/yetkilerini (`DROP OWNED BY`) kaldırır. Rolün başka
/// veritabanlarındaki nesneleri o veritabanlarıyla birlikte silinmiş olmalıdır (temizlik önce veritabanlarını siler).
pub async fn drop_test_role(admin: &Client, name: &str) {
    if let Err(e) = try_drop_test_role(admin, name).await {
        panic!("{name} silinemedi: {e}");
    }
}

async fn try_drop_test_role(admin: &Client, name: &str) -> Result<(), String> {
    let exists = !admin.query("SELECT 1 FROM pg_roles WHERE rolname = $1::text", &[&name]).await.unwrap().is_empty();
    if !exists {
        return Ok(());
    }
    let q = pgt_ddl::quote_ident(name);
    admin.batch_execute(&format!("DROP OWNED BY {q}")).await.map_err(|e| db_message(&e))?;
    admin.batch_execute(&format!("DROP ROLE {q}")).await.map_err(|e| db_message(&e))
}

/// Etiketin tüm küme nesnelerini siler: önce veritabanları (roller nesne sahibi olabilir), sonra roller.
pub async fn cleanup_cluster(ts: &TestServer, tag: &str) {
    let admin = admin_client(ts, &ts.config.maintenance_db).await;
    let db = cluster_name(ts, pgt_ddl::ObjectKind::Database, tag).await;
    let sql = format!("SELECT datname::text FROM pg_database WHERE {}", BY_PREFIX.replace("$1", "datname::text").replace("$2", "$1::text"));
    for r in admin.query(&sql, &[&db]).await.unwrap() {
        drop_test_database(&admin, r.get(0)).await;
    }
    let role = cluster_name(ts, pgt_ddl::ObjectKind::Role, tag).await;
    let sql = format!("SELECT rolname::text FROM pg_roles WHERE {}", BY_PREFIX.replace("$1", "rolname::text").replace("$2", "$1::text"));
    let mut left: Vec<String> = admin.query(&sql, &[&role]).await.unwrap().iter().map(|r| r.get(0)).collect();
    // PG 16+: başka üyelikleri vermiş (grantor) rol, o üyelikler durdukça silinemeyebilir → birkaç tur.
    for _ in 0..3 {
        let mut failed = vec![];
        for name in left {
            if try_drop_test_role(&admin, &name).await.is_err() {
                failed.push(name);
            }
        }
        if failed.is_empty() {
            return;
        }
        left = failed;
    }
    for name in &left {
        drop_test_role(&admin, name).await;
    }
}

// ---- yetki testlerinin küme rolleri ----

/// Yetki testi rolü: `pgt_acl_rt_<etiket>_<n>` (`n` tırnak gerektiren ad olabilir). Roller küme geneldir: etiket
/// test başına benzersizdir ve `_` içermez (başka etiketin rollerini temizliğe katmasın).
pub fn acl_role(tag: &str, n: &str) -> String {
    assert!(!tag.contains('_'), "yetki etiketi '_' içeremez: {tag}");
    format!("pgt_acl_rt_{tag}_{n}")
}

/// Etiketin yetki test rollerini siler. Rollerin test veritabanındaki yetkileri/nesneleri veritabanıyla birlikte
/// gider (önce veritabanı silinmelidir); bakım veritabanındakiler `DROP OWNED BY` ile; PG 16+ veren bağımlılıkları
/// için birkaç tur.
pub async fn drop_acl_roles(ts: &TestServer, tag: &str) {
    let admin = admin_client(ts, &ts.config.maintenance_db).await;
    let prefix = acl_role(tag, "");
    let sql = "SELECT rolname::text FROM pg_roles WHERE left(rolname::text, length($1::text)) = $1::text";
    let mut left: Vec<String> = admin.query(sql, &[&prefix]).await.unwrap().iter().map(|r| r.get(0)).collect();
    for _ in 0..3 {
        let mut failed = vec![];
        for name in left {
            if try_drop_test_role(&admin, &name).await.is_err() {
                failed.push(name);
            }
        }
        if failed.is_empty() {
            return;
        }
        left = failed;
    }
    for name in &left {
        drop_test_role(&admin, name).await;
    }
}

/// Yetki testi temizliği: önce veritabanı (`cleanup_in`), sonra etiketin rolleri.
pub async fn cleanup_acl(ts: &TestServer, prefix: &str, tag: &str) {
    cleanup_in(ts, prefix, tag).await;
    drop_acl_roles(ts, tag).await;
}

pub struct Env {
    pub admin: Client,
    pub mgr: Arc<ConnectionManager>,
    pub server: Uuid,
    pub db_oid: u32,
    pub version: u32,
    pub role: String,
    pub me: String,
}

/// Önceki çalıştırmanın artıklarını temizler, veritabanı ve rolü kurar, salt okunur katalog bağlantısını açar.
pub async fn setup(ts: &TestServer, tag: &str) -> Env {
    setup_in(ts, RT_PREFIX, tag).await
}

/// `setup` ile aynı; veritabanı/rol adları `prefix` ile.
pub async fn setup_in(ts: &TestServer, prefix: &str, tag: &str) -> Env {
    cleanup_in(ts, prefix, tag).await;
    let name = db_name_in(ts, prefix, tag).await;
    let maint = admin_client(ts, &ts.config.maintenance_db).await;
    let role = role_name_in(prefix, tag);
    maint.batch_execute(&format!("CREATE ROLE {role}")).await.unwrap();
    maint.batch_execute(&format!("CREATE DATABASE {name} TEMPLATE template0 ENCODING 'UTF8'")).await.unwrap();
    let admin = admin_client(ts, &name).await;
    let mgr = Arc::new(ConnectionManager::new());
    mgr.connect(ts.config.clone(), ts.password.clone()).await.unwrap();
    let db_oid: u32 = maint.query_one("SELECT oid FROM pg_database WHERE datname = $1", &[&name]).await.unwrap().get(0);
    let me: String = admin.query_one("SELECT current_user::text", &[]).await.unwrap().get(0);
    Env { version: version_num(ts).await, admin, mgr, server: ts.config.id, db_oid, role, me }
}

impl Env {
    pub fn ctx(&self, concurrently: bool) -> pgt_ddl::BuildCtx {
        pgt_ddl::BuildCtx { version_num: self.version, concurrently }
    }

    /// Planın ifadelerini sırayla çalıştırır (apply henüz yok; her ifade kendi başına gönderilir).
    pub async fn exec(&self, plan: &Plan) {
        for s in &plan.statements {
            if let Err(e) = self.admin.batch_execute(&s.sql).await {
                panic!("ifade başarısız: {}\n  hata: {}", s.sql, db_message(&e));
            }
        }
    }

    /// Planı uygulayıcı gibi TEK transaction'da çalıştırır; ilk hatada geri alır ve sunucu iletisini döndürür.
    pub async fn try_exec_tx(&self, plan: &Plan) -> Result<(), String> {
        self.admin.batch_execute("BEGIN").await.unwrap();
        for s in &plan.statements {
            if let Err(e) = self.admin.batch_execute(&s.sql).await {
                self.admin.batch_execute("ROLLBACK").await.unwrap();
                return Err(format!("{}\n  ifade: {}", db_message(&e), s.sql));
            }
        }
        self.admin.batch_execute("COMMIT").await.map_err(|e| db_message(&e))
    }

    /// `try_exec_tx`; hata panik.
    pub async fn exec_tx(&self, plan: &Plan) {
        if let Err(e) = self.try_exec_tx(plan).await {
            panic!("plan başarısız: {e}");
        }
    }

    /// Ham SQL; sunucu hatası iletisiyle döner (beklenen ret denemeleri için).
    pub async fn try_sql(&self, sql: &str) -> Result<(), String> {
        self.admin.batch_execute(sql).await.map_err(|e| db_message(&e))
    }

    pub async fn sql(&self, sql: &str) {
        if let Err(e) = self.admin.batch_execute(sql).await {
            panic!("ifade başarısız: {sql}\n  hata: {}", db_message(&e));
        }
    }

    pub async fn class_oid(&self, schema: &str, name: &str) -> u32 {
        let sql = "SELECT c.oid FROM pg_class c JOIN pg_namespace n ON n.oid = c.relnamespace WHERE n.nspname = $1::text AND c.relname = $2::text";
        self.admin.query_one(sql, &[&schema, &name]).await.unwrap_or_else(|e| panic!("{schema}.{name} bulunamadı: {e}")).get(0)
    }

    /// Fonksiyon/procedure oid'i: `şema.ad(giriş tipleri)` (`regprocedure`; `args` = virgüllü tip listesi, OUT hariç).
    pub async fn proc_oid(&self, schema: &str, name: &str, args: &str) -> u32 {
        let sig = format!("{}({args})", pgt_ddl::qualified(schema, name));
        self.admin
            .query_one("SELECT $1::text::regprocedure::oid", &[&sig])
            .await
            .unwrap_or_else(|e| panic!("{sig} bulunamadı: {}", db_message(&e)))
            .get(0)
    }

    /// `şema.ad(giriş tipleri)` var mı (`to_regprocedure`).
    pub async fn proc_exists(&self, schema: &str, name: &str, args: &str) -> bool {
        let sig = format!("{}({args})", pgt_ddl::qualified(schema, name));
        let v: Option<u32> = self.admin.query_one("SELECT to_regprocedure($1::text)::oid", &[&sig]).await.unwrap().get(0);
        v.is_some()
    }

    /// Tip/domain oid'i (`şema.ad`; `regtype` değil: dizi adları `_x` biçiminde de bulunsun).
    pub async fn type_oid(&self, schema: &str, name: &str) -> u32 {
        let sql = "SELECT t.oid FROM pg_type t JOIN pg_namespace n ON n.oid = t.typnamespace WHERE n.nspname = $1::text AND t.typname = $2::text";
        self.admin.query_one(sql, &[&schema, &name]).await.unwrap_or_else(|e| panic!("{schema}.{name} bulunamadı: {e}")).get(0)
    }

    pub async fn schema_oid(&self, name: &str) -> u32 {
        self.admin.query_one("SELECT oid FROM pg_namespace WHERE nspname = $1::text", &[&name]).await.unwrap().get(0)
    }

    pub async fn extension_oid(&self, name: &str) -> u32 {
        self.admin.query_one("SELECT oid FROM pg_extension WHERE extname = $1::text", &[&name]).await.unwrap().get(0)
    }

    /// Eklenti sunucuda kurulabilir mi.
    pub async fn has_extension(&self, name: &str) -> bool {
        !self.admin.query("SELECT 1 FROM pg_available_extensions WHERE name = $1::text", &[&name]).await.unwrap().is_empty()
    }
}
