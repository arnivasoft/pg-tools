use std::time::Duration;

use pgt_core::config::ServerConfig;
use pgt_core::connection::{ConnectionManager, Target};
use pgt_core::testing::{TestServer, admin_client, test_servers};
use pgt_core::{ConnectErrorKind, Error};

/// `PGT_TEST_URLS` içindeki her sunucu; test gövdesi her biri için ayrı çalışır (sürüm matrisi).
fn servers() -> Vec<TestServer> {
    let servers = test_servers();
    if servers.is_empty() {
        eprintln!("PGT_TEST_URLS tanımlı değil, test atlanıyor");
    }
    servers
}

#[tokio::test]
async fn unreachable_host_is_network_error() {
    let config = ServerConfig::new("kapalı", "127.0.0.1", 1, "postgres");
    let err = ConnectionManager::test(&config, None).await.unwrap_err();
    assert!(matches!(err, Error::Connect { kind: ConnectErrorKind::Network, .. }), "{err:?}");
}

#[tokio::test]
async fn connects_and_reports_version() {
    for ts in servers() {
        let mgr = ConnectionManager::new();
        let info = mgr.connect(ts.config.clone(), ts.password.clone()).await.unwrap();
        assert!(info.version_num >= 100000, "{info:?}");
        assert!(!info.version.contains(' '), "kısa sürüm bekleniyordu: {}", info.version);
        assert!(mgr.is_connected(ts.config.id));
        assert_eq!(mgr.info(ts.config.id).unwrap().version_num, info.version_num);
    }
}

#[tokio::test]
async fn session_settings_are_applied() {
    for ts in servers() {
        let mgr = ConnectionManager::new();
        mgr.connect(ts.config.clone(), ts.password.clone()).await.unwrap();
        let (read_only, app, timeout): (String, String, String) = mgr
            .run(ts.config.id, Target::Maintenance, |c| async move {
                let row = c
                    .client
                    .query_one(
                        "SELECT current_setting('default_transaction_read_only'), current_setting('application_name'), current_setting('statement_timeout')",
                        &[],
                    )
                    .await?;
                Ok((row.get(0), row.get(1), row.get(2)))
            })
            .await
            .unwrap();
        assert_eq!((read_only.as_str(), app.as_str(), timeout.as_str()), ("on", "pg-tools", "30s"));
    }
}

#[tokio::test]
async fn writes_are_rejected() {
    for ts in servers() {
        let mgr = ConnectionManager::new();
        mgr.connect(ts.config.clone(), ts.password.clone()).await.unwrap();
        let err = mgr
            .run(ts.config.id, Target::Maintenance, |c| async move { c.client.batch_execute("CREATE TEMP TABLE pgt_x(a int)").await })
            .await
            .unwrap_err();
        assert!(matches!(err, Error::Query { ref sqlstate, .. } if sqlstate.as_deref() == Some("25006")), "{err:?}");
    }
}

#[tokio::test]
async fn wrong_password_is_auth_error() {
    for ts in servers() {
        let err = ConnectionManager::test(&ts.config, Some("kesinlikle-yanlis-parola")).await.unwrap_err();
        assert!(matches!(err, Error::Connect { kind: ConnectErrorKind::Auth, .. }), "{err:?}");
    }
}

#[tokio::test]
async fn missing_password_is_password_required() {
    for ts in servers() {
        let err = ConnectionManager::test(&ts.config, None).await.unwrap_err();
        assert!(matches!(err, Error::PasswordRequired), "{err:?}");
    }
}

#[tokio::test]
async fn run_without_connect_is_not_connected() {
    let mgr = ConnectionManager::new();
    let err = mgr
        .run(uuid::Uuid::new_v4(), Target::Maintenance, |c| async move { c.client.simple_query("SELECT 1").await })
        .await
        .unwrap_err();
    assert!(matches!(err, Error::NotConnected));
}

#[tokio::test]
async fn database_target_resolves_by_oid() {
    for ts in servers() {
        let mgr = ConnectionManager::new();
        mgr.connect(ts.config.clone(), ts.password.clone()).await.unwrap();
        let oid: u32 = mgr
            .run(ts.config.id, Target::Maintenance, |c| async move {
                c.client.query_one("SELECT oid FROM pg_database WHERE datname = current_database()", &[]).await.map(|r| r.get(0))
            })
            .await
            .unwrap();
        let name: String = mgr
            .run(ts.config.id, Target::Database(oid), |c| async move {
                c.client.query_one("SELECT current_database()::text", &[]).await.map(|r| r.get(0))
            })
            .await
            .unwrap();
        assert_eq!(name, ts.config.maintenance_db);
        let missing = mgr
            .run(ts.config.id, Target::Database(4_000_000_000), |c| async move { c.client.simple_query("SELECT 1").await })
            .await
            .unwrap_err();
        assert!(matches!(missing, Error::NotFound(_)), "{missing:?}");
    }
}

#[tokio::test]
async fn reconnects_after_backend_terminated() {
    for ts in servers() {
        let mgr = ConnectionManager::new();
        mgr.connect(ts.config.clone(), ts.password.clone()).await.unwrap();
        let pid_query = |c: std::sync::Arc<pgt_core::connection::PgConn>| async move {
            c.client.query_one("SELECT pg_backend_pid()", &[]).await.map(|r| r.get::<_, i32>(0))
        };
        let pid = mgr.run(ts.config.id, Target::Maintenance, pid_query).await.unwrap();
        let admin = admin_client(&ts, &ts.config.maintenance_db).await;
        admin.execute("SELECT pg_terminate_backend($1)", &[&pid]).await.unwrap();
        tokio::time::sleep(Duration::from_millis(200)).await;
        let new_pid = mgr.run(ts.config.id, Target::Maintenance, pid_query).await.unwrap();
        assert_ne!(pid, new_pid);
        assert!(mgr.is_connected(ts.config.id));
    }
}

#[tokio::test]
async fn reap_idle_closes_and_run_reopens() {
    for ts in servers() {
        let mgr = ConnectionManager::new();
        mgr.connect(ts.config.clone(), ts.password.clone()).await.unwrap();
        mgr.reap_idle(Duration::ZERO).await;
        assert!(mgr.is_connected(ts.config.id), "oturum bağlı kalmalı");
        let one: i32 = mgr
            .run(ts.config.id, Target::Maintenance, |c| async move { c.client.query_one("SELECT 1", &[]).await.map(|r| r.get(0)) })
            .await
            .unwrap();
        assert_eq!(one, 1);
    }
}

#[tokio::test]
async fn disconnect_removes_session() {
    for ts in servers() {
        let mgr = ConnectionManager::new();
        mgr.connect(ts.config.clone(), ts.password.clone()).await.unwrap();
        mgr.disconnect(ts.config.id);
        assert!(!mgr.is_connected(ts.config.id));
    }
}

/// Sessiz yeniden bağlanma başarısız olunca oturum düşer ve hata `NotConnected`'tan ayırt edilebilir.
#[tokio::test]
async fn failed_silent_reconnect_reports_connection_lost() {
    for ts in servers() {
        let role = format!("pgt_lost_{}", std::process::id());
        let admin = admin_client(&ts, &ts.config.maintenance_db).await;
        admin.batch_execute(&format!("DROP ROLE IF EXISTS {role}; CREATE ROLE {role} LOGIN PASSWORD 'pgt-lost-pw'")).await.unwrap();
        let mut config = ts.config.clone();
        config.id = uuid::Uuid::new_v4();
        config.username = role.clone();
        let mgr = ConnectionManager::new();
        mgr.connect(config.clone(), Some("pgt-lost-pw".into())).await.unwrap();
        let pid_query = |c: std::sync::Arc<pgt_core::connection::PgConn>| async move {
            c.client.query_one("SELECT pg_backend_pid()", &[]).await.map(|r| r.get::<_, i32>(0))
        };
        let pid = mgr.run(config.id, Target::Maintenance, pid_query).await.unwrap();
        admin.batch_execute(&format!("ALTER ROLE {role} NOLOGIN")).await.unwrap();
        admin.execute("SELECT pg_terminate_backend($1)", &[&pid]).await.unwrap();
        tokio::time::sleep(Duration::from_millis(200)).await;
        let err = mgr.run(config.id, Target::Maintenance, pid_query).await.unwrap_err();
        admin.batch_execute(&format!("DROP ROLE IF EXISTS {role}")).await.unwrap();
        assert!(matches!(err, Error::ConnectionLost(_)), "{err:?}");
        assert!(!mgr.is_connected(config.id));
        let again = mgr.run(config.id, Target::Maintenance, pid_query).await.unwrap_err();
        assert!(matches!(again, Error::NotConnected), "{again:?}");
    }
}

/// Bağlı bir veritabanı silinince yalnızca o bağlantı düşer; sunucu oturumu bağlı kalır.
#[tokio::test]
async fn dropped_database_keeps_session_connected() {
    for ts in servers() {
        let db = format!("pgt_dropme_{}", std::process::id());
        let admin = admin_client(&ts, &ts.config.maintenance_db).await;
        // `DROP DATABASE ... WITH (FORCE)` PG 13+; eski sürümler için oturumlar önce sonlandırılır.
        let terminate = "SELECT pg_terminate_backend(pid) FROM pg_stat_activity WHERE datname = $1 AND pid <> pg_backend_pid()";
        admin.execute(terminate, &[&db]).await.unwrap();
        admin.batch_execute(&format!("DROP DATABASE IF EXISTS {db}")).await.unwrap();
        admin.batch_execute(&format!("CREATE DATABASE {db} TEMPLATE template0 ENCODING 'UTF8'")).await.unwrap();
        let oid: u32 = admin.query_one("SELECT oid FROM pg_database WHERE datname = $1", &[&db]).await.unwrap().get(0);
        let mgr = ConnectionManager::new();
        mgr.connect(ts.config.clone(), ts.password.clone()).await.unwrap();
        let one = |c: std::sync::Arc<pgt_core::connection::PgConn>| async move {
            c.client.query_one("SELECT 1", &[]).await.map(|r| r.get::<_, i32>(0))
        };
        assert_eq!(mgr.run(ts.config.id, Target::Database(oid), one).await.unwrap(), 1);
        admin.execute(terminate, &[&db]).await.unwrap();
        admin.batch_execute(&format!("DROP DATABASE {db}")).await.unwrap();
        tokio::time::sleep(Duration::from_millis(200)).await;
        let err = mgr.run(ts.config.id, Target::Database(oid), one).await.unwrap_err();
        assert!(!matches!(err, Error::ConnectionLost(_) | Error::NotConnected), "{err:?}");
        assert!(mgr.is_connected(ts.config.id));
        assert_eq!(mgr.run(ts.config.id, Target::Maintenance, one).await.unwrap(), 1);
    }
}

/// 28000 (ör. oturum açma izni olmayan rol) parola hatası değildir; sunucunun mesajıyla döner.
#[tokio::test]
async fn login_not_permitted_is_not_an_auth_error() {
    for ts in servers() {
        let role = format!("pgt_nologin_{}", std::process::id());
        let admin = admin_client(&ts, &ts.config.maintenance_db).await;
        admin.batch_execute(&format!("DROP ROLE IF EXISTS {role}; CREATE ROLE {role} NOLOGIN PASSWORD 'pgt-nologin-pw'")).await.unwrap();
        let mut config = ts.config.clone();
        config.username = role.clone();
        let err = ConnectionManager::test(&config, Some("pgt-nologin-pw")).await.unwrap_err();
        admin.batch_execute(&format!("DROP ROLE IF EXISTS {role}")).await.unwrap();
        assert!(
            matches!(err, Error::Connect { kind: ConnectErrorKind::Other, ref sqlstate, .. } if sqlstate.as_deref() == Some("28000")),
            "{err:?}"
        );
        let payload = pgt_core::ErrorPayload::from(&err);
        assert_eq!(payload.code, "connect");
        assert!(payload.message.contains("not permitted"), "{}", payload.message);
    }
}
