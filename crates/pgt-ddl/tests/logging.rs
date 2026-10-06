//! Günlük gizliliği: SQL metni, ad, değer ve parola `tracing` çıktısına girmez. Süreç genelinde (global) abone kurar;
//! bu yüzden `manager.rs` ile aynı test ikilisinde değil, kendi ikilisindedir (başka testlerin abonesiyle çakışmaz).

mod common;

use std::io::Write;
use std::sync::{Arc, Mutex, RwLock};

use common::{cleanup, guarded, servers, setup};
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
    ApplyOptions { apply_id: Uuid::new_v4(), lock_timeout_ms: 5000, expected_hash: hash.into(), confirmed: false, confirmed_production: false }
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
        guarded(logging_body(ts.clone()), cleanup(&ts, "m_log")).await;
        let out = String::from_utf8(buf.0.lock().unwrap().clone()).unwrap();
        assert!(out.contains("DDL uygulandı") && out.contains("DDL uygulanamadı"), "günlük yakalanmadı: {out}");
        assert!(out.contains("42P06"), "sonuç kodu günlükte olmalı: {out}");
        for secret in ["m_log_gizli_ad", "gizli yorum", "CREATE", "SCHEMA", "COMMENT", "already exists"] {
            assert!(!out.contains(secret), "günlükte {secret:?} var: {out}");
        }
        if let Some(p) = ts.password.as_deref().filter(|p| !p.is_empty()) {
            assert!(!out.contains(p), "günlükte parola var");
        }
    }
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
}
