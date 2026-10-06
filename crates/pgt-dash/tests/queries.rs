//! Dashboard sorguları entegrasyon testleri (`PGT_TEST_URLS` yoksa atlanır; her sunucu için ayrı çalışır).

use pgt_core::connection::{ConnectionManager, DedicatedConn, SessionProfile, Target};
use pgt_core::testing::{TestServer, admin_client, test_servers};
use pgt_dash::queries;

fn servers() -> Vec<TestServer> {
    let servers = test_servers();
    if servers.is_empty() {
        eprintln!("PGT_TEST_URLS tanımlı değil, test atlanıyor");
    }
    servers
}

async fn dash_conn(s: &TestServer) -> DedicatedConn {
    let mgr = ConnectionManager::new();
    mgr.connect(s.config.clone(), s.password.clone()).await.unwrap();
    mgr.open_dedicated(s.config.id, Target::Maintenance, SessionProfile::Dashboard).await.unwrap()
}

#[tokio::test]
async fn sample_and_overview_run_on_every_version() {
    for s in servers() {
        let c = dash_conn(&s).await;
        let v = c.info.version_num;
        let a = queries::sample(&c.client, None).await.unwrap();
        assert!(a.at > 1.0e9 && a.sessions_active >= 1, "kendi sorgumuz etkin sayılmalı (v{v})");
        let o = queries::overview(&c.client, None).await.unwrap();
        assert!(o.max_connections >= 1 && o.uptime_secs > 0.0 && o.version.starts_with("PostgreSQL"));
        let b = queries::sample(&c.client, None).await.unwrap();
        assert!(b.xact_commit >= a.xact_commit && b.at >= a.at); // sayaçlar monoton
    }
}

#[tokio::test]
async fn activity_marks_self_and_filters_by_database() {
    for s in servers() {
        let c = dash_conn(&s).await;
        let v = c.info.version_num;
        eprintln!("activity varyantı: PG {} ({})", v / 10_000, if v / 10_000 >= 13 { "pg13+" } else { "pg10-12" });
        let rows = queries::activity(&c.client, v, None).await.unwrap();
        let me: Vec<_> = rows.iter().filter(|r| r.is_self).collect();
        assert_eq!(me.len(), 1);
        assert_eq!(me[0].application, "pg-tools dash");
        assert_eq!(me[0].state.as_deref(), Some("active"));
        let none = queries::activity(&c.client, v, Some("yok_boyle_bir_db")).await.unwrap();
        assert!(none.is_empty());
    }
}

#[tokio::test]
async fn activity_hides_other_users_queries_without_privilege() {
    for s in servers() {
        let admin = admin_client(&s, &s.config.maintenance_db).await;
        let role = "pgt_dash_lowpriv";
        let pw = uuid::Uuid::new_v4().simple().to_string(); // yazdırılmaz
        admin.batch_execute(&format!("DROP ROLE IF EXISTS {role}")).await.unwrap();
        admin.batch_execute(&format!("CREATE ROLE {role} LOGIN PASSWORD '{pw}'")).await.unwrap();

        // Yönetici oturumunda uzun bir sorgu (başka rol); ayrı görevde çalışır.
        let sleeper = admin_client(&s, &s.config.maintenance_db).await;
        let sleeper_pid: i32 = sleeper.query_one("SELECT pg_backend_pid()", &[]).await.unwrap().get(0);
        let task = tokio::spawn(async move {
            let _ = sleeper.batch_execute("SELECT pg_sleep(30)").await;
        });
        // pg_sleep'in başladığını bekle.
        for _ in 0..50 {
            let n: i64 = admin
                .query_one(
                    "SELECT count(*) FROM pg_stat_activity WHERE pid = $1 AND state = 'active' AND query LIKE '%pg_sleep%'",
                    &[&sleeper_pid],
                )
                .await
                .unwrap()
                .get(0);
            if n == 1 {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        }

        let mut low = s.clone();
        low.config.username = role.to_string();
        low.password = Some(pw);
        let c = dash_conn(&low).await;
        let rows = queries::activity(&c.client, c.info.version_num, None).await.unwrap();
        let other = rows.iter().find(|r| r.pid == sleeper_pid).expect("uyuyan oturum listede olmalı");
        assert_eq!(other.application, "pg-tools-test");
        assert_eq!(other.query, None);
        let me = rows.iter().find(|r| r.is_self).unwrap();
        assert!(me.query.is_some());

        drop(c);
        admin.execute("SELECT pg_terminate_backend($1)", &[&sleeper_pid]).await.unwrap();
        let _ = task.await;
        // Düşük yetkili oturum kapanana dek DROP ROLE yeniden denenir.
        for _ in 0..20 {
            if admin.batch_execute(&format!("DROP ROLE IF EXISTS {role}")).await.is_ok() {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(200)).await;
        }
    }
}
