#![allow(dead_code)]
//! Docker entegrasyon testleri için ortak yardımcılar (`PGT_TEST_URLS` yoksa test atlanır).

use std::future::Future;
use std::sync::Arc;

use pgt_core::connection::ConnectionManager;
use pgt_core::testing::{TestServer, admin_client, drop_database, test_servers};
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
pub async fn guarded(
    body: impl Future<Output = ()> + Send + 'static,
    cleanup: impl Future<Output = ()>,
) {
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

    fn poll(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Self::Output> {
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
    e.as_db_error()
        .map(|d| format!("{} ({})", d.message(), d.code().code()))
        .unwrap_or_else(|| e.to_string())
}

pub async fn version_num(ts: &TestServer) -> u32 {
    let admin = admin_client(ts, &ts.config.maintenance_db).await;
    let v: i32 = admin
        .query_one("SELECT current_setting('server_version_num')::int4", &[])
        .await
        .unwrap()
        .get(0);
    v as u32
}

/// Önekli veritabanı adı: `<önek>_<ana sürüm>_<etiket>`.
pub async fn db_name_in(ts: &TestServer, prefix: &str, tag: &str) -> String {
    format!("{prefix}_{}_{tag}", version_num(ts).await / 10_000)
}

pub fn role_name_in(prefix: &str, tag: &str) -> String {
    format!("{prefix}_role_{tag}")
}

pub async fn cleanup_in(ts: &TestServer, prefix: &str, tag: &str) {
    let admin = admin_client(ts, &ts.config.maintenance_db).await;
    drop_database(&admin, &db_name_in(ts, prefix, tag).await).await;
    admin
        .batch_execute(&format!(
            "DROP ROLE IF EXISTS {}",
            role_name_in(prefix, tag)
        ))
        .await
        .unwrap();
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

/// `setup` ile aynı; veritabanı/rol adları `prefix` ile.
pub async fn setup_in(ts: &TestServer, prefix: &str, tag: &str) -> Env {
    cleanup_in(ts, prefix, tag).await;
    let name = db_name_in(ts, prefix, tag).await;
    let maint = admin_client(ts, &ts.config.maintenance_db).await;
    let role = role_name_in(prefix, tag);
    maint
        .batch_execute(&format!("CREATE ROLE {role}"))
        .await
        .unwrap();
    maint
        .batch_execute(&format!(
            "CREATE DATABASE {name} TEMPLATE template0 ENCODING 'UTF8'"
        ))
        .await
        .unwrap();
    let admin = admin_client(ts, &name).await;
    let mgr = Arc::new(ConnectionManager::new());
    mgr.connect(ts.config.clone(), ts.password.clone())
        .await
        .unwrap();
    let db_oid: u32 = maint
        .query_one("SELECT oid FROM pg_database WHERE datname = $1", &[&name])
        .await
        .unwrap()
        .get(0);
    let me: String = admin
        .query_one("SELECT current_user::text", &[])
        .await
        .unwrap()
        .get(0);
    Env {
        version: version_num(ts).await,
        admin,
        mgr,
        server: ts.config.id,
        db_oid,
        role,
        me,
    }
}

impl Env {
    pub async fn sql(&self, sql: &str) {
        if let Err(e) = self.admin.batch_execute(sql).await {
            panic!("ifade başarısız: {sql}\n  hata: {}", db_message(&e));
        }
    }

    pub async fn class_oid(&self, schema: &str, name: &str) -> u32 {
        let sql = "SELECT c.oid FROM pg_class c JOIN pg_namespace n ON n.oid = c.relnamespace WHERE n.nspname = $1::text AND c.relname = $2::text";
        self.admin
            .query_one(sql, &[&schema, &name])
            .await
            .unwrap_or_else(|e| panic!("{schema}.{name} bulunamadı: {e}"))
            .get(0)
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
        let v: Option<u32> = self
            .admin
            .query_one("SELECT to_regprocedure($1::text)::oid", &[&sig])
            .await
            .unwrap()
            .get(0);
        v.is_some()
    }

    /// Eklenti sunucuda kurulabilir mi.
    pub async fn has_extension(&self, name: &str) -> bool {
        !self
            .admin
            .query(
                "SELECT 1 FROM pg_available_extensions WHERE name = $1::text",
                &[&name],
            )
            .await
            .unwrap()
            .is_empty()
    }
}
