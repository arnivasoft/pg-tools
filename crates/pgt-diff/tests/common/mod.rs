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

/// Bu veritabanında açık transaction'da bekleyen oturum kalmadı (bağlantı düşüşü asenkron: kısa süre beklenir).
pub async fn assert_no_idle_in_tx_for(env: &Env) {
    let sql = "SELECT count(*) FROM pg_stat_activity WHERE datname = current_database() AND state LIKE 'idle in transaction%'";
    for _ in 0..30 {
        let n: i64 = env.admin.query_one(sql, &[]).await.unwrap().get(0);
        if n == 0 {
            return;
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    panic!("idle in transaction oturum kaldı");
}

/// Gidiş-dönüş fikstürü: şema `s` için kaynak (`source`) ya da hedef tarafın DDL'i. Ortak kısım iki tarafta aynıdır;
/// farklar sabit bir tabloya karşılık gelir (bkz. `diff_roundtrip.rs` içindeki beklenen durumlar).
///
/// Bilinen sınırlara dokunulmaz: yeniden kurulan nesnenin bağımlısı yok (görünüm farkı yaprak `v2`'de; dönüş tipi farkı
/// bağımlısız `fmt`'te), kolon tipi farkı görünüm/MV'nin bağlı olmadığı `products`'ta, enum'a değer ekleme yalnız PG 12+
/// (eski sürümde betik transaction'sız olurdu ve tek çok-ifadeli sorgu olarak gönderilemezdi).
pub fn rt_fixture(s: &str, ver: u32, source: bool) -> String {
    let mood = if source || ver < 120_000 {
        "'sad','ok','happy'"
    } else {
        "'sad','ok'"
    };
    let posint = if source { "VALUE > 0" } else { "VALUE >= 0" };
    let mut q = format!(
        "CREATE SCHEMA {s};
         CREATE TYPE {s}.mood AS ENUM ({mood});
         CREATE TYPE {s}.pair AS (m {s}.mood, n int);
         CREATE TYPE {s}.span AS RANGE (subtype = int4);
         CREATE DOMAIN {s}.posint AS int CHECK ({posint});
         CREATE TABLE {s}.customers (
             id serial PRIMARY KEY,
             email text NOT NULL,
             mood {s}.mood,
             qty {s}.posint,
             CONSTRAINT customers_email_key UNIQUE (email),
             CONSTRAINT customers_qty_chk CHECK (qty < 1000));
         COMMENT ON TABLE {s}.customers IS 'müşteri tablosu';
         COMMENT ON COLUMN {s}.customers.email IS 'e-posta';
         CREATE TABLE {s}.orders (
             id serial PRIMARY KEY,
             customer_id int NOT NULL REFERENCES {s}.customers (id),
             note text,
             total numeric(10,2),
             pr {s}.pair);
         CREATE INDEX orders_note_lower_idx ON {s}.orders (lower(note));
         CREATE FUNCTION {s}.touch() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN RETURN NEW; END $$;
         CREATE FUNCTION {s}.add_one(a integer) RETURNS integer LANGUAGE sql AS $$SELECT a + {inc}$$;
         CREATE FUNCTION {s}.add_one(a text) RETURNS text LANGUAGE sql AS $$SELECT a || '1'$$;
         COMMENT ON FUNCTION {s}.add_one(text) IS 'metin ekler';
         CREATE VIEW {s}.v1 AS SELECT id, email FROM {s}.customers;
         CREATE VIEW {s}.v2 AS SELECT {v2cols} FROM {s}.v1;
         CREATE MATERIALIZED VIEW {s}.mv AS SELECT {mvcols} FROM {s}.orders;
         CREATE INDEX mv_id_idx ON {s}.mv (id);
         CREATE TABLE {s}.seq_owner (v int);
         CREATE SEQUENCE {s}.standalone_seq;
         ALTER SEQUENCE {s}.standalone_seq OWNED BY {s}.seq_owner.v;
         CREATE TABLE {s}.ordered {ordered};",
        inc = if source { 1 } else { 2 },
        v2cols = if source { "id" } else { "id, email" },
        mvcols = if source { "id, total" } else { "id, total, note" },
        ordered = if source {
            "(a int, b text, c int)"
        } else {
            "(c int, a int, b text)"
        },
    );
    if source {
        q.push_str(&format!(
            "COMMENT ON TABLE {s}.orders IS 'sipariş tablosu';
             CREATE INDEX orders_total_partial_idx ON {s}.orders (total) WHERE total > 100;
             CREATE TRIGGER orders_touch BEFORE INSERT ON {s}.orders FOR EACH ROW EXECUTE PROCEDURE {s}.touch();
             CREATE FUNCTION {s}.fmt(a integer) RETURNS text LANGUAGE sql AS $$SELECT a::text$$;
             CREATE TABLE {s}.products (id int PRIMARY KEY, name text, price numeric(10,2), sku text);
             CREATE TABLE {s}.audit_log (
                 id bigserial PRIMARY KEY,
                 order_id int REFERENCES {s}.orders (id),
                 msg text);
             CREATE TABLE {s}.rt_items (id int, label text);
             CREATE FUNCTION {s}.list_items() RETURNS SETOF {s}.rt_items LANGUAGE sql AS $$SELECT 1, 'x'::text$$;
             CREATE VIEW {s}.rt_v AS SELECT id, label FROM {s}.rt_items;
             CREATE FUNCTION {s}.v_label(r {s}.rt_v) RETURNS text LANGUAGE sql AS $$SELECT ($1).label$$;"
        ));
    } else {
        q.push_str(&format!(
            "CREATE FUNCTION {s}.fmt(a integer) RETURNS integer LANGUAGE sql AS $$SELECT a$$;
             CREATE TABLE {s}.products (id int PRIMARY KEY, title text, price numeric(12,4), legacy int);
             CREATE TABLE {s}.legacy_stuff (id serial PRIMARY KEY, x int);"
        ));
    }
    if ver >= 110_000 {
        q.push_str(&format!(
            "CREATE PROCEDURE {s}.noop() LANGUAGE sql AS '';"
        ));
    }
    q
}
