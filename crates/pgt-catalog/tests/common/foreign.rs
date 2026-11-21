//! FDW / publication / subscription testlerinin ortak kurulumu (Plan 38): postgres_fdw, sunucular, user mapping'ler
//! (sır parolalı), foreign table, publication'lar ve `connect = false` abonelik; süper kullanıcı olmayan bir rol.

use pgt_catalog::node::{NodeId, Seg, TreeNode};
use pgt_core::testing::{TestServer, admin_client, drop_database};

use super::Fixture;

/// Ayırt edici sır: hiçbir çıktıda (düğüm, özellik, DDL, günlük) görünmemeli.
pub const SECRET: &str = "pgt_gizli_parola_7f3";
/// Süper kullanıcı olmayan rolün parolası (yalnız test kapsayıcısında).
pub const PLAIN_PASSWORD: &str = "pgt_plain_pw";

pub struct ForeignSetup {
    /// Nesnelerin bulunduğu veritabanı.
    pub db: String,
    /// Yalnız başka veritabanı aboneliği için.
    pub other_db: String,
    /// Süper kullanıcı olmayan rol.
    pub role: String,
    pub db_oid: u32,
    pub db_id: NodeId,
}

pub fn assert_no_secret(nodes: &[TreeNode]) {
    for n in nodes {
        let json = serde_json::to_string(n).unwrap();
        assert!(!json.contains(SECRET), "sır düğümde göründü: {}", n.label);
    }
}

/// Önceki yarım kalmış çalışmanın aboneliklerini (varsa) bırakır, sonra veritabanını siler.
pub async fn drop_db_with_subscriptions(ts: &TestServer, name: &str) {
    let admin = admin_client(ts, &ts.config.maintenance_db).await;
    let exists = admin.query_opt("SELECT 1 FROM pg_database WHERE datname = $1", &[&name]).await.unwrap().is_some();
    if exists {
        let db = admin_client(ts, name).await;
        let _ = db
            .batch_execute(
                "DO $$ DECLARE s text; BEGIN FOR s IN SELECT subname FROM pg_subscription WHERE subdbid = (SELECT oid FROM pg_database WHERE datname = current_database()) LOOP \
                 EXECUTE format('ALTER SUBSCRIPTION %I DISABLE', s); EXECUTE format('ALTER SUBSCRIPTION %I SET (slot_name = NONE)', s); EXECUTE format('DROP SUBSCRIPTION %I', s); END LOOP; END $$",
            )
            .await;
    }
    drop_database(&admin, name).await;
}

/// `prefix`'e özgü veritabanları ve rolle ortak nesneleri kurar (önceki çalışmanın kalıntısını önce temizler).
pub async fn setup(fx: &Fixture, prefix: &str) -> ForeignSetup {
    let ts = &fx.ts;
    let major = fx.major().await;
    let (db_name, other_db, role) = (prefix.to_string(), format!("{prefix}_other"), format!("{prefix}_plain"));
    for name in [&db_name, &other_db] {
        drop_db_with_subscriptions(ts, name).await;
    }
    let admin = admin_client(ts, &ts.config.maintenance_db).await;
    admin.batch_execute(&format!("DROP ROLE IF EXISTS {role}")).await.unwrap();
    admin.batch_execute(&format!("CREATE ROLE {role} LOGIN PASSWORD '{PLAIN_PASSWORD}'")).await.unwrap();
    for name in [&db_name, &other_db] {
        admin.batch_execute(&format!("CREATE DATABASE {name} TEMPLATE template0 ENCODING 'UTF8'")).await.unwrap();
    }
    let db = admin_client(ts, &db_name).await;
    let conninfo = format!("host=127.0.0.1 port=1 dbname=x user=u password={SECRET}");
    let mut sql = format!(
        "CREATE EXTENSION postgres_fdw;
         CREATE FOREIGN DATA WRAPPER pgt_plain_fdw;
         CREATE SERVER pgt_srv FOREIGN DATA WRAPPER postgres_fdw OPTIONS (host 'x', dbname 'y');
         CREATE SERVER pgt_srv_typed TYPE 'pg' VERSION '16' FOREIGN DATA WRAPPER postgres_fdw OPTIONS (host 'x', dbname 'y');
         CREATE USER MAPPING FOR CURRENT_USER SERVER pgt_srv OPTIONS (user 'u', password '{SECRET}');
         CREATE USER MAPPING FOR PUBLIC SERVER pgt_srv OPTIONS (user 'u', password '{SECRET}');
         CREATE TABLE public.pub_t (id int PRIMARY KEY);
         CREATE FOREIGN TABLE public.ft_remote (id int, label text) SERVER pgt_srv OPTIONS (table_name 'remote_t');
         CREATE PUBLICATION pgt_pub_tbl FOR TABLE public.pub_t;
         CREATE PUBLICATION pgt_pub_all FOR ALL TABLES;
         CREATE SUBSCRIPTION pgt_sub CONNECTION '{conninfo}' PUBLICATION pgt_pub_tbl WITH (connect = false, slot_name = NONE);"
    );
    if major >= 15 {
        sql.push_str("CREATE PUBLICATION pgt_pub_sch FOR TABLES IN SCHEMA public;");
    }
    db.batch_execute(&sql).await.unwrap();
    // Başka veritabanındaki abonelik bu veritabanının listesinde görünmemeli (subdbid süzgeci).
    admin_client(ts, &other_db)
        .await
        .batch_execute(&format!("CREATE SUBSCRIPTION pgt_sub_other CONNECTION '{conninfo}' PUBLICATION p WITH (connect = false, slot_name = NONE)"))
        .await
        .unwrap();
    let db_oid: u32 = admin.query_one("SELECT oid FROM pg_database WHERE datname = $1", &[&db_name]).await.unwrap().get(0);
    ForeignSetup { db_id: fx.server_id().child(Seg::Db(db_oid)), db: db_name, other_db, role, db_oid }
}

impl ForeignSetup {
    /// Süper kullanıcı olmayan rolle aynı sunucuya ikinci bir kayıt bağlar; kimliğini döndürür.
    pub async fn connect_plain(&self, fx: &Fixture) -> uuid::Uuid {
        let mut limited = fx.ts.config.clone();
        limited.id = uuid::Uuid::new_v4();
        limited.username = self.role.clone();
        fx.registry.write().unwrap().upsert(limited.clone()).unwrap();
        fx.mgr.connect(limited.clone(), Some(PLAIN_PASSWORD.into())).await.unwrap();
        limited.id
    }

    pub async fn teardown(&self, fx: &Fixture) {
        for name in [&self.db, &self.other_db] {
            drop_db_with_subscriptions(&fx.ts, name).await;
        }
        let admin = admin_client(&fx.ts, &fx.ts.config.maintenance_db).await;
        admin.batch_execute(&format!("DROP ROLE {}", self.role)).await.unwrap();
    }
}
