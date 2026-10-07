use std::time::{Duration, Instant};

use pgt_core::config::ServerConfig;
use pgt_core::connection::{
    ConnectionManager, DedicatedConn, SessionProfile, Target, apply_session_sets, connect_dedicated_once, connect_once,
};
use pgt_core::testing::{TestServer, admin_client, drop_database, test_servers};
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
        let (read_only, app, timeout, search_path): (String, String, String, String) = mgr
            .run(ts.config.id, Target::Maintenance, |c| async move {
                let row = c
                    .client
                    .query_one(
                        "SELECT current_setting('default_transaction_read_only'), current_setting('application_name'), current_setting('statement_timeout'), current_setting('search_path')",
                        &[],
                    )
                    .await?;
                Ok((row.get(0), row.get(1), row.get(2), row.get(3)))
            })
            .await
            .unwrap();
        assert_eq!(
            (read_only.as_str(), app.as_str(), timeout.as_str(), search_path.as_str()),
            ("on", "pg-tools", "30s", "pg_catalog")
        );
    }
}

/// `options` gönderilmeyen oturum (yedek yol; `options`'ı sessizce yok sayan havuzlayıcıyla aynı durum):
/// sürüm sorgusundaki ayar denetimi ayarların etkin olmadığını görür ve `SESSION_SETS`'i uygular.
#[tokio::test]
async fn session_settings_are_applied_without_options_parameter() {
    for ts in servers() {
        let conn = connect_once(&ts.config, &ts.config.maintenance_db, ts.password.as_deref(), false).await.unwrap();
        let row = conn
            .client
            .query_one(
                "SELECT current_setting('default_transaction_read_only'), current_setting('application_name'), current_setting('statement_timeout'), current_setting('search_path')",
                &[],
            )
            .await
            .unwrap();
        let got: (String, String, String, String) = (row.get(0), row.get(1), row.get(2), row.get(3));
        assert_eq!((got.0.as_str(), got.1.as_str(), got.2.as_str(), got.3.as_str()), ("on", "pg-tools", "30s", "pg_catalog"));
        let err = conn.client.batch_execute("CREATE TEMP TABLE pgt_x(a int)").await.unwrap_err();
        assert_eq!(err.code().map(|c| c.code()), Some("25006"), "{err:?}");
    }
}

#[tokio::test]
async fn failing_session_set_is_a_connect_error() {
    for ts in servers() {
        let conn = connect_once(&ts.config, &ts.config.maintenance_db, ts.password.as_deref(), true).await.unwrap();
        let err = apply_session_sets(&conn.client, &["SET pgt_no_such = 1"]).await.unwrap_err();
        assert!(
            matches!(&err, Error::Connect { kind: ConnectErrorKind::Other, sqlstate: Some(code), .. } if code == "42704"),
            "{err:?}"
        );
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
        drop_database(&admin, &db).await;
        admin.batch_execute(&format!("CREATE DATABASE {db} TEMPLATE template0 ENCODING 'UTF8'")).await.unwrap();
        let oid: u32 = admin.query_one("SELECT oid FROM pg_database WHERE datname = $1", &[&db]).await.unwrap().get(0);
        let mgr = ConnectionManager::new();
        mgr.connect(ts.config.clone(), ts.password.clone()).await.unwrap();
        let one = |c: std::sync::Arc<pgt_core::connection::PgConn>| async move {
            c.client.query_one("SELECT 1", &[]).await.map(|r| r.get::<_, i32>(0))
        };
        assert_eq!(mgr.run(ts.config.id, Target::Database(oid), one).await.unwrap(), 1);
        drop_database(&admin, &db).await;
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

const QUERY_RW: SessionProfile = SessionProfile::Query { read_only: false, statement_timeout_ms: 0 };

async fn show(conn: &DedicatedConn, setting: &str) -> String {
    conn.client.query_one(&format!("SELECT current_setting('{setting}')"), &[]).await.unwrap().get(0)
}

async fn maintenance_oid(mgr: &ConnectionManager, server: uuid::Uuid) -> u32 {
    mgr.run(server, Target::Maintenance, |c| async move {
        c.client.query_one("SELECT oid FROM pg_database WHERE datname = current_database()", &[]).await.map(|r| r.get(0))
    })
    .await
    .unwrap()
}

/// Yazılabilir sorgu oturumu: yazma serbest, `search_path` kullanıcınınki, süre sınırı yok.
#[tokio::test]
async fn dedicated_query_session_is_writable_with_user_search_path() {
    for ts in servers() {
        let mgr = ConnectionManager::new();
        mgr.connect(ts.config.clone(), ts.password.clone()).await.unwrap();
        let conn = mgr.open_dedicated(ts.config.id, Target::Maintenance, QUERY_RW).await.unwrap();
        assert_eq!(conn.info, mgr.info(ts.config.id).unwrap());
        conn.client.batch_execute("CREATE TEMP TABLE t(i int)").await.unwrap();
        assert_eq!(show(&conn, "default_transaction_read_only").await, "off");
        assert_eq!(show(&conn, "statement_timeout").await, "0");
        assert_eq!(show(&conn, "search_path").await, "\"$user\", public");
        assert_eq!(show(&conn, "application_name").await, "pg-tools query");
    }
}

#[tokio::test]
async fn dedicated_read_only_query_session_rejects_writes() {
    for ts in servers() {
        let mgr = ConnectionManager::new();
        mgr.connect(ts.config.clone(), ts.password.clone()).await.unwrap();
        let profile = SessionProfile::Query { read_only: true, statement_timeout_ms: 300_000 };
        let conn = mgr.open_dedicated(ts.config.id, Target::Maintenance, profile).await.unwrap();
        let err = conn.client.batch_execute("CREATE TEMP TABLE t(i int)").await.unwrap_err();
        assert_eq!(err.code().map(|c| c.code()), Some("25006"), "{err:?}");
        assert_eq!(show(&conn, "statement_timeout").await, "5min");
        assert_eq!(show(&conn, "search_path").await, "\"$user\", public");
    }
}

/// `options` gönderilmeyen sorgu oturumu (yedek yol): ayarlar `SET` ile uygulanır, `search_path`'e dokunulmaz.
#[tokio::test]
async fn dedicated_session_settings_are_applied_without_options_parameter() {
    for ts in servers() {
        let profile = SessionProfile::Query { read_only: true, statement_timeout_ms: 12_345 };
        let conn = connect_dedicated_once(&ts.config, &ts.config.maintenance_db, ts.password.as_deref(), profile, false)
            .await
            .unwrap();
        assert_eq!(show(&conn, "default_transaction_read_only").await, "on");
        assert_eq!(show(&conn, "statement_timeout").await, "12345ms");
        assert_eq!(show(&conn, "search_path").await, "\"$user\", public");
        assert_eq!(show(&conn, "application_name").await, "pg-tools query");
    }
}

#[tokio::test]
async fn dedicated_session_forwards_notices() {
    for ts in servers() {
        let mgr = ConnectionManager::new();
        mgr.connect(ts.config.clone(), ts.password.clone()).await.unwrap();
        let mut conn = mgr.open_dedicated(ts.config.id, Target::Maintenance, QUERY_RW).await.unwrap();
        conn.client.batch_execute("DO $$ BEGIN RAISE NOTICE 'merhaba'; END $$").await.unwrap();
        let notice = tokio::time::timeout(Duration::from_secs(5), conn.notices.recv()).await.unwrap().unwrap();
        assert_eq!(notice.message, "merhaba");
        assert_eq!(notice.code, "00000");
        assert_eq!(notice.severity, "NOTICE");
    }
}

#[tokio::test]
async fn dedicated_session_query_can_be_cancelled() {
    for ts in servers() {
        let mgr = ConnectionManager::new();
        mgr.connect(ts.config.clone(), ts.password.clone()).await.unwrap();
        let conn = mgr.open_dedicated(ts.config.id, Target::Maintenance, QUERY_RW).await.unwrap();
        let started = Instant::now();
        let query = conn.client.batch_execute("SELECT pg_sleep(30)");
        let cancel = async {
            tokio::time::sleep(Duration::from_millis(300)).await;
            conn.cancel.cancel().await
        };
        let (result, cancelled) = tokio::join!(query, cancel);
        cancelled.unwrap();
        let err = result.unwrap_err();
        assert_eq!(err.code().map(|c| c.code()), Some("57014"), "{err:?}");
        assert!(started.elapsed() < Duration::from_secs(2), "{:?}", started.elapsed());
        // Oturum iptalden sonra kullanılabilir kalır.
        let one: i32 = conn.client.query_one("SELECT 1", &[]).await.unwrap().get(0);
        assert_eq!(one, 1);
    }
}

#[tokio::test]
async fn open_dedicated_without_connect_is_not_connected() {
    let mgr = ConnectionManager::new();
    let err = mgr.open_dedicated(uuid::Uuid::new_v4(), Target::Maintenance, QUERY_RW).await.unwrap_err();
    assert!(matches!(err, Error::NotConnected), "{err:?}");
}

/// Sorgu oturumu havuza girmez; `run` yolu etkilenmez. Veritabanı hedefi oid ile çözülür.
#[tokio::test]
async fn dedicated_session_is_outside_the_pool_and_resolves_database_oid() {
    for ts in servers() {
        let mgr = ConnectionManager::new();
        mgr.connect(ts.config.clone(), ts.password.clone()).await.unwrap();
        let oid = maintenance_oid(&mgr, ts.config.id).await;
        assert_eq!(mgr.pooled_connections(ts.config.id).await, 1);
        let conn = mgr.open_dedicated(ts.config.id, Target::Database(oid), QUERY_RW).await.unwrap();
        let db: String = conn.client.query_one("SELECT current_database()::text", &[]).await.unwrap().get(0);
        assert_eq!(db, ts.config.maintenance_db);
        let other = mgr.open_dedicated(ts.config.id, Target::Maintenance, QUERY_RW).await.unwrap();
        assert_eq!(mgr.pooled_connections(ts.config.id).await, 1, "sorgu oturumları havuz bağlantısı açmamalı");
        let pids: (i32, i32) = (
            conn.client.query_one("SELECT pg_backend_pid()", &[]).await.unwrap().get(0),
            other.client.query_one("SELECT pg_backend_pid()", &[]).await.unwrap().get(0),
        );
        let pool_pid: i32 = mgr
            .run(ts.config.id, Target::Maintenance, |c| async move {
                c.client.query_one("SELECT pg_backend_pid()", &[]).await.map(|r| r.get(0))
            })
            .await
            .unwrap();
        assert!(pids.0 != pool_pid && pids.1 != pool_pid && pids.0 != pids.1, "{pids:?} {pool_pid}");
        let missing = mgr.open_dedicated(ts.config.id, Target::Database(4_000_000_000), QUERY_RW).await.unwrap_err();
        assert!(matches!(missing, Error::NotFound(_)), "{missing:?}");
        assert!(mgr.is_connected(ts.config.id));
    }
}

/// İstemci düşünce sunucu oturumu kapanır ve sürücü görevi biter (NOTICE kanalı kapanır).
#[tokio::test]
async fn dropping_dedicated_client_closes_backend_and_driver() {
    for ts in servers() {
        let mgr = ConnectionManager::new();
        mgr.connect(ts.config.clone(), ts.password.clone()).await.unwrap();
        let DedicatedConn { client, mut notices, .. } =
            mgr.open_dedicated(ts.config.id, Target::Maintenance, QUERY_RW).await.unwrap();
        let pid: i32 = client.query_one("SELECT pg_backend_pid()", &[]).await.unwrap().get(0);
        drop(client);
        let closed = tokio::time::timeout(Duration::from_secs(5), notices.recv()).await.expect("sürücü görevi bitmedi");
        assert!(closed.is_none());
        let admin = admin_client(&ts, &ts.config.maintenance_db).await;
        let mut alive = true;
        for _ in 0..50 {
            let n: i64 = admin.query_one("SELECT count(*) FROM pg_stat_activity WHERE pid = $1", &[&pid]).await.unwrap().get(0);
            if n == 0 {
                alive = false;
                break;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        assert!(!alive, "sunucu oturumu kapanmadı");
    }
}

/// Silinen veritabanında sorgu oturumu 3D000 ile başarısız olur ve ad önbelleği düşer: sonraki deneme adı
/// yeniden çözer ve `NotFound` döner. Sunucu oturumu bağlı kalır.
#[tokio::test]
async fn dedicated_session_on_dropped_database_re_resolves_name() {
    for ts in servers() {
        let db = format!("pgt_qdrop_{}", std::process::id());
        let admin = admin_client(&ts, &ts.config.maintenance_db).await;
        drop_database(&admin, &db).await;
        admin.batch_execute(&format!("CREATE DATABASE {db} TEMPLATE template0 ENCODING 'UTF8'")).await.unwrap();
        let oid: u32 = admin.query_one("SELECT oid FROM pg_database WHERE datname = $1", &[&db]).await.unwrap().get(0);
        let mgr = ConnectionManager::new();
        mgr.connect(ts.config.clone(), ts.password.clone()).await.unwrap();
        drop(mgr.open_dedicated(ts.config.id, Target::Database(oid), QUERY_RW).await.unwrap());
        drop_database(&admin, &db).await;
        let err = mgr.open_dedicated(ts.config.id, Target::Database(oid), QUERY_RW).await.unwrap_err();
        assert!(matches!(&err, Error::Connect { sqlstate: Some(c), .. } if c == "3D000"), "{err:?}");
        let again = mgr.open_dedicated(ts.config.id, Target::Database(oid), QUERY_RW).await.unwrap_err();
        assert!(matches!(again, Error::NotFound(_)), "{again:?}");
        assert!(mgr.is_connected(ts.config.id));
    }
}

async fn current_db(mgr: &ConnectionManager, server: uuid::Uuid, oid: u32) -> String {
    mgr.run(server, Target::Database(oid), |c| async move {
        c.client.query_one("SELECT current_database()::text", &[]).await.map(|r| r.get::<_, String>(0))
    })
    .await
    .unwrap()
}

/// `release_database` havuzdaki veritabanı bağlantısını kapatır ve ad önbelleğini siler: sunucu oturumu kalmaz,
/// yeniden adlandırma 55006 vermez, sonraki `run` yeni adı çözer. Bağlı değilken çağrı sessizdir.
#[tokio::test]
async fn release_database_closes_the_pooled_connection_and_forgets_the_name() {
    for ts in servers() {
        let db = format!("pgt_qrel_{}", std::process::id());
        let renamed = format!("{db}_yeni");
        let admin = admin_client(&ts, &ts.config.maintenance_db).await;
        drop_database(&admin, &db).await;
        drop_database(&admin, &renamed).await;
        admin.batch_execute(&format!("CREATE DATABASE {db} TEMPLATE template0 ENCODING 'UTF8'")).await.unwrap();
        let oid: u32 = admin.query_one("SELECT oid FROM pg_database WHERE datname = $1", &[&db]).await.unwrap().get(0);
        let mgr = ConnectionManager::new();
        mgr.release_database(ts.config.id, oid).await;
        mgr.connect(ts.config.clone(), ts.password.clone()).await.unwrap();
        assert_eq!(current_db(&mgr, ts.config.id, oid).await, db);
        assert_eq!(mgr.pooled_connections(ts.config.id).await, 2);
        mgr.release_database(ts.config.id, oid).await;
        assert_eq!(mgr.pooled_connections(ts.config.id).await, 1, "bakım bağlantısı kalmalı");
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            let left: i64 = admin.query_one("SELECT count(*) FROM pg_stat_activity WHERE datname = $1", &[&db]).await.unwrap().get(0);
            if left == 0 {
                break;
            }
            assert!(Instant::now() < deadline, "havuz bağlantısı sunucuda kapanmadı");
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        admin.batch_execute(&format!("ALTER DATABASE {db} RENAME TO {renamed}")).await.unwrap();
        assert_eq!(current_db(&mgr, ts.config.id, oid).await, renamed, "ad önbelleği yeniden çözülmeli");
        assert!(mgr.is_connected(ts.config.id));
        mgr.release_database(ts.config.id, oid).await;
        drop(mgr);
        drop_database(&admin, &renamed).await;
    }
}
