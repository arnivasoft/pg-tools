//! DashManager entegrasyon testleri (`PGT_TEST_URLS` yoksa atlanır; her sunucu için ayrı çalışır).

use std::future::Future;
use std::sync::Arc;
use std::time::Duration;

use pgt_core::Error;
use pgt_core::config::ServerConfig;
use pgt_core::connection::ConnectionManager;
use pgt_core::testing::{TestServer, admin_client, drop_database, test_servers};
use pgt_dash::{DashManager, SignalKind, SignalOutcome};
use uuid::Uuid;

fn servers() -> Vec<TestServer> {
    let servers = test_servers();
    if servers.is_empty() {
        eprintln!("PGT_TEST_URLS tanımlı değil, test atlanıyor");
    }
    servers
}

async fn connected(s: &TestServer) -> (Arc<ConnectionManager>, Uuid) {
    let cm = Arc::new(ConnectionManager::new());
    cm.connect(s.config.clone(), s.password.clone()).await.unwrap();
    (cm, s.config.id)
}

/// Gövdeyi ayrı görevde çalıştırır; bir doğrulama başarısız olsa bile `cleanup` çalışır, sonra panik yeniden fırlatılır.
async fn guarded(body: impl Future<Output = ()> + Send + 'static, cleanup: impl Future<Output = ()>) {
    let result = tokio::spawn(body).await;
    cleanup.await;
    if let Err(e) = result {
        std::panic::resume_unwind(e.into_panic());
    }
}

#[tokio::test]
async fn open_query_close_roundtrip() {
    for s in servers() {
        let (cm, id) = connected(&s).await;
        let dm = DashManager::new(cm.clone());
        let info = dm.open(id, None).await.unwrap();
        assert!(info.self_pid > 0 && info.database.is_none());
        assert!(dm.sample(info.id).await.is_ok());
        assert!(dm.activity(info.id).await.unwrap().rows.iter().any(|r| r.is_self));
        dm.close(info.id);
        assert!(matches!(dm.sample(info.id).await, Err(Error::NotFound(_))), "kapalı kimlik: NotFound");
        // close_server / close_all de oturumları düşürür.
        let a = dm.open(id, None).await.unwrap();
        let b = dm.open(id, None).await.unwrap();
        dm.close_server(id);
        assert!(dm.sample(a.id).await.is_err() && dm.sample(b.id).await.is_err());
        let c = dm.open(id, None).await.unwrap();
        dm.close_all();
        assert!(dm.sample(c.id).await.is_err());
    }
}

#[tokio::test]
async fn database_scope_filters_activity() {
    for s in servers() {
        let admin = Arc::new(admin_client(&s, &s.config.maintenance_db).await);
        let ver = admin.query_one("SHOW server_version_num", &[]).await.unwrap().get::<_, String>(0);
        let db = format!("pgt_dash_scope_{ver}");
        drop_database(&admin, &db).await;
        admin.batch_execute(&format!("CREATE DATABASE {db}")).await.unwrap();

        let (s2, admin2, db2) = (s.clone(), admin.clone(), db.clone());
        let body = async move {
            let (s, admin, db) = (s2, admin2, db2);
            let oid = admin
                .query_one("SELECT oid::int8 FROM pg_database WHERE datname = $1", &[&db])
                .await
                .unwrap()
                .get::<_, i64>(0) as u32;
            let (cm, id) = connected(&s).await;
            let dm = DashManager::new(cm);
            let info = dm.open(id, Some(oid)).await.unwrap();
            assert_eq!(info.database.as_deref(), Some(db.as_str()));
            let rows = dm.activity(info.id).await.unwrap().rows;
            assert!(rows.iter().any(|r| r.is_self));
            assert!(rows.iter().all(|r| r.database.as_deref() == Some(db.as_str())), "yalnızca bu veritabanının oturumları");
            dm.close(info.id);
        };
        guarded(body, drop_database(&admin, &db)).await;
    }
}

#[tokio::test]
async fn cancel_and_terminate_real_sessions() {
    for s in servers() {
        let (cm, id) = connected(&s).await;
        let dm = Arc::new(DashManager::new(cm));
        let info = dm.open(id, None).await.unwrap();

        let (s2, dm2) = (s.clone(), dm.clone());
        let body = async move {
            let (s, dm) = (s2, dm2);
            // A: uzun sorgu iptal edilir, oturum yaşar.
            let a = Arc::new(admin_client(&s, &s.config.maintenance_db).await);
            let pid_a: i32 = a.query_one("SELECT pg_backend_pid()", &[]).await.unwrap().get(0);
            let probe = admin_client(&s, &s.config.maintenance_db).await;
            let a2 = a.clone();
            let sleeper = tokio::spawn(async move { a2.batch_execute("SELECT pg_sleep(30)").await });
            let mut seen = false;
            for _ in 0..50 {
                let n: i64 = probe
                    .query_one("SELECT count(*) FROM pg_stat_activity WHERE pid = $1 AND state = 'active'", &[&pid_a])
                    .await
                    .unwrap()
                    .get(0);
                if n == 1 {
                    seen = true;
                    break;
                }
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
            assert!(seen, "uyuyan sorgu 5 sn içinde active görülmedi");
            assert_eq!(dm.signal(info.id, pid_a, SignalKind::Cancel).await.unwrap(), SignalOutcome::Sent);
            let err = sleeper.await.unwrap().unwrap_err();
            assert_eq!(err.code().map(|c| c.code()), Some("57014"));
            assert!(a.query_one("SELECT 1", &[]).await.is_ok(), "iptal oturumu öldürmez");

            // B: sonlandırılır.
            let b = admin_client(&s, &s.config.maintenance_db).await;
            let pid_b: i32 = b.query_one("SELECT pg_backend_pid()", &[]).await.unwrap().get(0);
            assert_eq!(dm.signal(info.id, pid_b, SignalKind::Terminate).await.unwrap(), SignalOutcome::Sent);
            let mut closed = false;
            for _ in 0..50 {
                if b.is_closed() {
                    closed = true;
                    break;
                }
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
            assert!(closed && b.query_one("SELECT 1", &[]).await.is_err());

            assert_eq!(dm.signal(info.id, 2_000_000_000, SignalKind::Cancel).await.unwrap(), SignalOutcome::NotFound);
            assert!(matches!(dm.signal(info.id, info.self_pid, SignalKind::Terminate).await, Err(Error::Invalid(_))));
            assert!(dm.sample(info.id).await.is_ok(), "reddedilen sinyal bağlantıyı bozmaz");
        };
        guarded(body, async {
            dm.close(info.id);
        })
        .await;
    }
}

#[tokio::test]
async fn signal_without_privilege_is_an_error_not_silence() {
    for s in servers() {
        let admin = Arc::new(admin_client(&s, &s.config.maintenance_db).await);
        let ver = admin.query_one("SHOW server_version_num", &[]).await.unwrap().get::<_, String>(0);
        let role = format!("pgt_dash_np_{ver}");
        let pw = Uuid::new_v4().simple().to_string();
        admin.batch_execute(&format!("DROP ROLE IF EXISTS {role}")).await.unwrap();
        admin.batch_execute(&format!("CREATE ROLE {role} LOGIN PASSWORD '{pw}'")).await.unwrap();

        let (s2, admin2, role2) = (s.clone(), admin.clone(), role.clone());
        let body = async move {
            let (s, admin, role) = (s2, admin2, role2);
            let mut cfg = ServerConfig::new("yetkisiz", s.config.host.clone(), s.config.port, role);
            cfg.maintenance_db = s.config.maintenance_db.clone();
            cfg.ssl = s.config.ssl.clone();
            let cm = Arc::new(ConnectionManager::new());
            cm.connect(cfg.clone(), Some(pw)).await.unwrap();
            let dm = DashManager::new(cm);
            let info = dm.open(cfg.id, None).await.unwrap();
            // Başka bir rolün (yönetici) oturumu.
            let other: i32 = admin.query_one("SELECT pg_backend_pid()", &[]).await.unwrap().get(0);
            let err = dm.signal(info.id, other, SignalKind::Cancel).await.unwrap_err();
            assert!(matches!(&err, Error::Query { sqlstate: Some(c), .. } if c == "42501"), "{err:?}");
            assert!(dm.sample(info.id).await.is_ok(), "başarısız sinyal bağlantıyı bozmaz");
            dm.close(info.id);
        };
        guarded(body, async {
            // Dash bağlantıları düşmüş olmalı; rol silinmeden önce oturumları kapanmasını kısaca bekle.
            for _ in 0..30 {
                if admin.batch_execute(&format!("DROP ROLE IF EXISTS {role}")).await.is_ok() {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
        })
        .await;
    }
}

#[tokio::test]
async fn reconnects_after_the_dash_connection_is_killed() {
    for s in servers() {
        let admin = admin_client(&s, &s.config.maintenance_db).await;
        let (cm, id) = connected(&s).await;
        let dm = DashManager::new(cm);
        let info = dm.open(id, None).await.unwrap();
        admin.execute("SELECT pg_terminate_backend($1)", &[&info.self_pid]).await.unwrap();
        let mut ok = false;
        for _ in 0..30 {
            if dm.sample(info.id).await.is_ok() {
                ok = true;
                break;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        assert!(ok, "3 sn içinde yeniden bağlanıp sorgu çalışmalıydı");
        let rows = dm.activity(info.id).await.unwrap().rows;
        let me = rows.iter().find(|r| r.is_self).expect("kendi oturumu");
        assert_ne!(me.pid, info.self_pid, "yeni bağlantı, yeni pid");
        dm.close(info.id);
    }
}
