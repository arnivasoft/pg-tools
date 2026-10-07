//! Günlük gizliliği: SQL metni, ad, değer ve parola `tracing` çıktısına girmez. Süreç genelinde (global) abone kurar;
//! bu yüzden `manager.rs` ile aynı test ikilisinde değil, kendi ikilisindedir (başka testlerin abonesiyle çakışmaz).

mod common;

use std::io::Write;
use std::sync::{Arc, Mutex, RwLock};

use common::{cleanup, cleanup_cluster, cluster_name, guarded, servers, setup};
use pgt_core::config::ServerConfig;
use pgt_core::registry::Registry;
use pgt_core::testing::TestServer;
use pgt_core::Result;
use pgt_ddl::*;
use uuid::Uuid;

fn create(def: ObjectDef) -> PlanRequest {
    PlanRequest::Create { def, concurrently: false }
}

fn opts(hash: &str) -> ApplyOptions {
    ApplyOptions { apply_id: Uuid::new_v4(), lock_timeout_ms: 5000, expected_hash: hash.into(), confirmed: false, confirmed_production: false, password: None }
}

fn manager(config: &ServerConfig, mgr: Arc<pgt_core::connection::ConnectionManager>) -> (DdlManager, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let mut registry = Registry::load(dir.path().join("servers.json")).unwrap().registry;
    registry.upsert(config.clone()).unwrap();
    (DdlManager::new(mgr, Arc::new(RwLock::new(registry))), dir)
}

async fn apply(dm: &DdlManager, server: Uuid, db: u32, req: &PlanRequest) -> Result<ApplyOutcome> {
    let hash = dm.preview(server, db, req).await.unwrap().hash;
    dm.apply(server, db, req, opts(&hash)).await
}

#[derive(Clone, Default)]
struct Buf(Arc<Mutex<Vec<u8>>>);

impl Write for Buf {
    fn write(&mut self, b: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(b);
        Ok(b.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

/// Süreç genelinde (global) abone: iş parçacığı yerel abone, paralel testlerin callsite önbelleğiyle yarışıp
/// olayları kaçırabiliyor. Diğer testlerin olayları da tampona düşer; denetimler yalnızca bu testin işaretlerine bakar.
#[tokio::test]
async fn password_free_logging() {
    let buf = Buf::default();
    let w = buf.clone();
    let subscriber = tracing_subscriber::fmt().with_writer(move || w.clone()).with_max_level(tracing::Level::TRACE).finish();
    tracing::subscriber::set_global_default(subscriber).expect("bu test ikilisinde başka global abone yok");
    for ts in servers() {
        let t = ts.clone();
        let clean = async move {
            cleanup(&t, "m_log").await;
            cleanup_cluster(&t, "mlog").await;
        };
        guarded(logging_body(ts.clone()), clean).await;
        let out = String::from_utf8(buf.0.lock().unwrap().clone()).unwrap();
        assert!(out.contains("DDL uygulandı") && out.contains("DDL uygulanamadı"), "günlük yakalanmadı: {out}");
        assert!(out.contains("42P06"), "sonuç kodu günlükte olmalı: {out}");
        let role = cluster_name(&ts, ObjectKind::Role, "mlog").await;
        assert!(out.contains("42710"), "rol tekrar kodu günlükte olmalı: {out}");
        for secret in [
            "m_log_gizli_ad",
            "gizli yorum",
            "CREATE",
            "SCHEMA",
            "COMMENT",
            "already exists",
            ROLE_PASSWORD,
            OTHER_PASSWORD,
            MASKED_PASSWORD,
            "SCRAM-SHA-256$",
            role.as_str(),
        ] {
            assert!(!out.contains(secret), "günlükte {secret:?} var: {out}");
        }
        if let Some(p) = ts.password.as_deref().filter(|p| !p.is_empty()) {
            assert!(!out.contains(p), "günlükte parola var");
        }
    }
}

/// Rol parolaları: günlükte, plan/sonuç JSON'unda ve `Debug` çıktısında hiç görünmemeli.
const ROLE_PASSWORD: &str = "gizli-parola-Q7";
const OTHER_PASSWORD: &str = "baska-parola-Z3";

fn secret(s: &str) -> Secret {
    serde_json::from_value(serde_json::json!(s)).unwrap()
}

async fn logging_body(ts: TestServer) {
    let env = setup(&ts, "m_log").await;
    let (dm, _dir) = manager(&ts.config, env.mgr.clone());
    let def = ObjectDef::Schema(SchemaDef { name: "m_log_gizli_ad".into(), owner: None, comment: Some("gizli yorum".into()) });
    assert!(matches!(apply(&dm, env.server, env.db_oid, &create(def.clone())).await, Ok(ApplyOutcome::Applied { .. })));
    // Aynı ad: 42P06 → Failed (hata iletisi adı içerir ama günlüğe yalnızca kod gider).
    let r = apply(&dm, env.server, env.db_oid, &create(def)).await;
    let Ok(ApplyOutcome::Failed { sqlstate, .. }) = r else { panic!("{r:?}") };
    assert_eq!(sqlstate.as_deref(), Some("42P06"));

    // Rol + parola: uygulanır; aynı adla ikinci uygulama 42710 ile düşer. Plan özeti parola değerinden bağımsızdır
    // (iki farklı parola aynı `expected_hash` ile kabul edilir).
    let name = cluster_name(&ts, ObjectKind::Role, "mlog").await;
    let def = RoleDef {
        name,
        login: true,
        superuser: false,
        createdb: false,
        createrole: false,
        inherit: true,
        replication: false,
        bypassrls: false,
        connection_limit: -1,
        valid_until: None,
        password: PasswordChange::Set,
        member_of: vec![],
        settings: vec![],
        comment: None,
        unsupported: vec![],
    };
    let req = create(ObjectDef::Role(def));
    let view = dm.preview(env.server, None, &req).await.unwrap();
    let view_json = serde_json::to_string(&view).unwrap();
    let mut o = opts(&view.hash);
    o.password = Some(secret(ROLE_PASSWORD));
    let dbg = format!("{o:?}");
    let first = dm.apply(env.server, env.db_oid, &req, o).await.unwrap();
    assert!(matches!(first, ApplyOutcome::Applied { .. }), "{first:?}");
    let mut o = opts(&view.hash);
    o.password = Some(secret(OTHER_PASSWORD));
    let second = dm.apply(env.server, None, &req, o).await.unwrap();
    let ApplyOutcome::Failed { sqlstate, position, .. } = &second else { panic!("{second:?}") };
    assert_eq!((sqlstate.as_deref(), *position), (Some("42710"), None));
    for text in [view_json, dbg, serde_json::to_string(&first).unwrap(), serde_json::to_string(&second).unwrap(), format!("{second:?}")] {
        for p in [ROLE_PASSWORD, OTHER_PASSWORD, "SCRAM-SHA-256$"] {
            assert!(!text.contains(p), "{p:?} sızdı: {text}");
        }
    }

    // G-11: GERÇEK (somutlaşmış) parola ifadesi sunucuda başarısız olur — CREATEROLE'lü süper kullanıcı olmayan rol bir
    // süper kullanıcının parolasını değiştirmeye çalışır → 42501, gizlenmiş ifadede; konum yok, yükte/Debug'da/günlükte
    // ne doğrulayıcı ne düz metin.
    role_password_statement_failure_is_masked_end_to_end(&ts, &env).await;
}

async fn role_password_statement_failure_is_masked_end_to_end(ts: &TestServer, env: &common::Env) {
    let base = cluster_name(ts, ObjectKind::Role, "mlog").await;
    let (creator, sup) = (format!("{base}_c"), format!("{base}_s"));
    let pw = format!("Pw{}", Uuid::new_v4().simple());
    env.admin
        .batch_execute(&format!(
            "CREATE ROLE {creator} LOGIN CREATEROLE PASSWORD {}; CREATE ROLE {sup} SUPERUSER",
            quote_literal(&pw)
        ))
        .await
        .unwrap();
    let mut config = ts.config.clone();
    config.id = Uuid::new_v4();
    config.username = creator.clone();
    let mgr = Arc::new(pgt_core::connection::ConnectionManager::new());
    if mgr.connect(config.clone(), Some(pw)).await.is_err() {
        panic!("CREATEROLE rolüyle bağlanılamadı");
    }
    let (dm, _dir) = manager(&config, mgr.clone());
    let oid: u32 = env.admin.query_one("SELECT oid FROM pg_roles WHERE rolname = $1::text", &[&sup]).await.unwrap().get(0);
    let object = ObjectRef { kind: ObjectKind::Role, oid };
    let ObjectDef::Role(loaded) = dm.load(config.id, None, object).await.unwrap() else { panic!() };
    let mut new = loaded.clone();
    new.password = PasswordChange::Set;
    let req = PlanRequest::Alter { object, old: ObjectDef::Role(loaded), new: ObjectDef::Role(new) };
    let view = dm.preview(config.id, None, &req).await.unwrap();
    assert!(view.statements.len() == 1 && view.statements[0].redacted, "{view:?}");
    let mut o = opts(&view.hash);
    o.password = Some(secret(MASKED_PASSWORD));
    let out = dm.apply(config.id, None, &req, o).await.unwrap();
    let ApplyOutcome::Failed { failed_index, sqlstate, position, error, .. } = &out else { panic!("{out:?}") };
    assert_eq!((*failed_index, sqlstate.as_deref(), *position), (0, Some("42501"), None));
    for text in [error.message.clone(), format!("{:?}", error.detail), format!("{:?}", error.hint), format!("{out:?}"), serde_json::to_string(&out).unwrap()] {
        for p in [MASKED_PASSWORD, "SCRAM-SHA-256$"] {
            assert!(!text.contains(p), "{p:?} sızdı: {text}");
        }
    }
    mgr.disconnect(config.id);
}

/// G-11 senaryosunun parolası (günlükte de aranır).
const MASKED_PASSWORD: &str = "maskeli-parola-K4";
