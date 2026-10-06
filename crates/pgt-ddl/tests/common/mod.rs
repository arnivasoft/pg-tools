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
pub async fn guarded(body: impl Future<Output = ()> + Send + 'static, cleanup: impl Future<Output = ()>) {
    let result = tokio::spawn(body).await;
    cleanup.await;
    if let Err(e) = result {
        std::panic::resume_unwind(e.into_panic());
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
