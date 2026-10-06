//! Kilit / ayar / hazır tx / pg_stat_statements sorguları entegrasyon testleri
//! (`PGT_TEST_URLS` yoksa atlanır; her sunucu için ayrı çalışır).

use std::future::Future;
use std::sync::Arc;
use std::time::Duration;

use pgt_core::connection::{ConnectionManager, DedicatedConn, SessionProfile, Target};
use pgt_core::testing::{TestServer, admin_client, drop_database, test_servers};
use pgt_dash::queries;
use pgt_dash::types::{Statements, StatementsOrder};

fn servers() -> Vec<TestServer> {
    let servers = test_servers();
    if servers.is_empty() {
        eprintln!("PGT_TEST_URLS tanımlı değil, test atlanıyor");
    }
    servers
}

async fn dash_conn_to(s: &TestServer, target: Target) -> DedicatedConn {
    let mgr = ConnectionManager::new();
    mgr.connect(s.config.clone(), s.password.clone()).await.unwrap();
    mgr.open_dedicated(s.config.id, target, SessionProfile::Dashboard).await.unwrap()
}

async fn db_oid(admin: &tokio_postgres::Client, name: &str) -> u32 {
    admin.query_one("SELECT oid::int8 FROM pg_database WHERE datname = $1", &[&name]).await.unwrap().get::<_, i64>(0) as u32
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
async fn blocking_chain_is_reported() {
    for s in servers() {
        let admin = Arc::new(admin_client(&s, &s.config.maintenance_db).await);
        let ver = admin.query_one("SHOW server_version_num", &[]).await.unwrap().get::<_, String>(0);
        let db = format!("pgt_dash_lock_{ver}");
        drop_database(&admin, &db).await;
        admin.batch_execute(&format!("CREATE DATABASE {db}")).await.unwrap();

        let (s2, admin2, db2) = (s.clone(), admin.clone(), db.clone());
        let body = async move {
            let (s, admin, db) = (s2, admin2, db2);
            let a = admin_client(&s, &db).await;
            a.batch_execute("CREATE TABLE t(id int)").await.unwrap();
            let pid_a: i32 = a.query_one("SELECT pg_backend_pid()", &[]).await.unwrap().get(0);
            a.batch_execute("BEGIN; LOCK TABLE t IN ACCESS EXCLUSIVE MODE").await.unwrap();
            let b = admin_client(&s, &db).await;
            let task_b = tokio::spawn(async move {
                let _ = b.batch_execute("SELECT * FROM t").await;
            });

            // Dash bağlantısı test veritabanına açılır (nesne adı yalnızca bağlı veritabanında çözülür).
            let c = dash_conn_to(&s, Target::Database(db_oid(&admin, &db).await)).await;
            let mut found = None;
            for _ in 0..50 {
                let r = queries::locks(&c.client, Some(&db)).await.unwrap();
                if r.sessions.iter().any(|x| !x.blocked_by.is_empty()) {
                    found = Some(r);
                    break;
                }
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
            let report = found.expect("5 sn içinde engellenen oturum görülmedi");
            let blocked = report.sessions.iter().find(|x| !x.blocked_by.is_empty()).unwrap();
            assert_eq!(blocked.blocked_by, vec![pid_a]);
            assert!(report.sessions.iter().any(|x| x.pid == pid_a && x.blocked_by.is_empty()), "kök engelleyici listede");
            assert!(report.locks.iter().any(|l| !l.granted && l.object.as_deref() == Some("t")));
            assert!(!report.truncated);
            let _ = a.batch_execute("ROLLBACK").await;
            let _ = task_b.await;
        };
        guarded(body, drop_database(&admin, &db)).await;
    }
}

#[tokio::test]
async fn settings_and_prepared_xacts() {
    for s in servers() {
        let admin = Arc::new(admin_client(&s, &s.config.maintenance_db).await);
        let (s2, admin2) = (s.clone(), admin.clone());
        let body = async move {
            let (s, admin) = (s2, admin2);
            let c = dash_conn_to(&s, Target::Maintenance).await;
            let all = queries::settings(&c.client).await.unwrap();
            assert!(all.len() > 200 && all.iter().any(|x| x.name == "max_connections"));
            let n: i64 = admin.query_one("SELECT current_setting('max_prepared_transactions')::int8", &[]).await.unwrap().get(0);
            if n > 0 {
                let _ = admin.batch_execute("ROLLBACK PREPARED 'pgt_dash_test'").await;
                admin.batch_execute("BEGIN; PREPARE TRANSACTION 'pgt_dash_test'").await.unwrap();
                let list = queries::prepared(&c.client).await.unwrap();
                let x = list.iter().find(|p| p.gid == "pgt_dash_test").expect("hazır tx listede olmalı");
                assert!(x.age_secs >= 0.0 && !x.owner.is_empty() && !x.database.is_empty());
            } else {
                eprintln!("max_prepared_transactions=0, hazır tx doğrulaması atlanıyor");
            }
            assert!(queries::prepared(&c.client).await.is_ok());
        };
        guarded(body, async {
            let _ = admin.batch_execute("ROLLBACK PREPARED 'pgt_dash_test'").await;
        })
        .await;
    }
}

#[tokio::test]
async fn statements_unavailable_without_extension() {
    for s in servers() {
        let c = dash_conn_to(&s, Target::Maintenance).await; // bakım DB'sinde eklenti yok
        match queries::statements(&c.client, c.info.version_num, None, StatementsOrder::TotalTime).await.unwrap() {
            Statements::Unavailable { reason } => assert!(!reason.is_empty()),
            Statements::Available { .. } => panic!("eklenti kurulu değil"),
        }
    }
}

/// Eklenti ayrı bir veritabanında kurulur (diğer testlerle çakışmaz). Sunucu `shared_preload_libraries`
/// ile başlatılmadıysa (Docker varsayılanı) sonuç "yüklenmemiş" nedenli `Unavailable`; başlatıldıysa
/// `Available` ve satırlar (elle kontrol için: `-c shared_preload_libraries=pg_stat_statements`).
#[tokio::test]
async fn statements_with_extension_in_own_database() {
    for s in servers() {
        let admin = Arc::new(admin_client(&s, &s.config.maintenance_db).await);
        let db = "pgt_dash_stmts";
        drop_database(&admin, db).await;
        admin.batch_execute(&format!("CREATE DATABASE {db}")).await.unwrap();

        let (s2, admin2) = (s.clone(), admin.clone());
        let body = async move {
            let (s, admin) = (s2, admin2);
            let preloaded: String = admin.query_one("SHOW shared_preload_libraries", &[]).await.unwrap().get(0);
            let preloaded = preloaded.contains("pg_stat_statements");
            let x = admin_client(&s, db).await;
            x.batch_execute("CREATE EXTENSION pg_stat_statements").await.unwrap();
            for _ in 0..3 {
                x.batch_execute("SELECT 1 AS pgt_dash_probe").await.unwrap();
            }
            let c = dash_conn_to(&s, Target::Database(db_oid(&admin, db).await)).await;
            let v = c.info.version_num;
            for order in [StatementsOrder::TotalTime, StatementsOrder::MeanTime, StatementsOrder::Calls] {
                match queries::statements(&c.client, v, Some(db), order).await.unwrap() {
                    Statements::Available { rows } => {
                        assert!(preloaded, "kitaplık yüklü değilken Available dönmemeli");
                        assert!(!rows.is_empty() && rows.len() <= 50);
                        if order == StatementsOrder::Calls {
                            assert!(rows.windows(2).all(|w| w[0].calls >= w[1].calls), "calls azalan sırada olmalı");
                        }
                        eprintln!("statements Available: PG {} {:?} {} satır", v / 10_000, order, rows.len());
                    }
                    Statements::Unavailable { reason } => {
                        assert!(!preloaded, "kitaplık yüklüyken Unavailable dönmemeli: {reason}");
                        assert!(reason.contains("yüklenmemiş"), "{reason}");
                    }
                }
            }
        };
        guarded(body, drop_database(&admin, db)).await;
    }
}
