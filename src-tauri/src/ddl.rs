//! Nesne diyalogları Tauri komutları (`ddl_*`). SQL metni/tanım değerleri/parola/URL asla günlüğe yazılmaz.
//!
//! Her komutun gelecekleri `Box::pin` ile kutulanır: büyük gelecekler invoke işleyicisinin ana iş parçacığı
//! yığın çerçevesine gömülürse release derlemesinde 1 MB'lık ana yığın taşar (bkz. commands.rs).
//! Hata eşlemesi tek yerden: `ddl_payload` (kapı hataları kodlu: locked, confirmation_required,
//! production_confirmation_required, plan_changed, invalid_def). Yürütme hataları `Ok(ApplyOutcome::Failed{..})` döner.
//! Tauri'de argüman adları arayüzde camelCase görünür (`server_id` -> `serverId`, `database_oid` -> `databaseOid`).
//! `databaseOid` `null` olabilir: sunucu kapsamlı türler (Database/Role) ve sunucu genelindeki açılır listeler bakım
//! veritabanında çalışır; veritabanı kapsamlı türde `null` → `invalid`.
//! Parola yalnızca `ddl_apply`'ın `options.password` alanında gelir (`Secret`: `Debug`/`Serialize` maskeli); komut
//! katmanı argümanları günlüğe yazmaz (`tauri::command` argüman kaydı yapmaz, burada `tracing` çağrısı yok).

use pgt_core::ErrorPayload;
use pgt_ddl::{
    AclDef, ApplyOptions, ApplyOutcome, CreateContext, CreatePreset, DdlTarget, DefaultAclDef, DefaultAclScope, LookupItem, LookupRequest, ObjectDef, ObjectKind, ObjectRef, PlanRequest, PlanView,
    ddl_payload,
};
use tauri::State;
use uuid::Uuid;

use crate::state::AppState;

type CmdResult<T> = Result<T, ErrorPayload>;

/// Düğüm kimliğinden diyalog hedefi (arayüz kimliği ayrıştırmaz).
#[tauri::command]
pub async fn ddl_target_of(_state: State<'_, AppState>, node_id: String) -> CmdResult<DdlTarget> {
    pgt_ddl::target_of(&node_id).map_err(|e| ddl_payload(&e))
}

#[tauri::command]
pub async fn ddl_defaults(
    state: State<'_, AppState>,
    server_id: Uuid,
    database_oid: Option<u32>,
    kind: ObjectKind,
    ctx: CreateContext,
    // `DdlTarget.create.preset` (Plan 18: `trigger_function`); arayüz göndermezse `None`.
    preset: Option<CreatePreset>,
) -> CmdResult<ObjectDef> {
    Box::pin(state.ddl.defaults(server_id, database_oid, kind, ctx, preset)).await.map_err(|e| ddl_payload(&e))
}

#[tauri::command]
pub async fn ddl_load(state: State<'_, AppState>, server_id: Uuid, database_oid: Option<u32>, object: ObjectRef) -> CmdResult<ObjectDef> {
    Box::pin(state.ddl.load(server_id, database_oid, object)).await.map_err(|e| ddl_payload(&e))
}

/// Nesnenin etkin yetkileri (Plan 20). Database için `databaseOid` `null` olabilir (bakım veritabanı).
#[tauri::command]
pub async fn ddl_load_acl(state: State<'_, AppState>, server_id: Uuid, database_oid: Option<u32>, object: ObjectRef) -> CmdResult<AclDef> {
    Box::pin(state.ddl.load_acl(server_id, database_oid, object)).await.map_err(|e| ddl_payload(&e))
}

/// Bir rolün varsayılan yetkileri (Plan 20; veritabanı kapsamlı).
#[tauri::command]
pub async fn ddl_load_default_acl(
    state: State<'_, AppState>,
    server_id: Uuid,
    database_oid: u32,
    scope: DefaultAclScope,
) -> CmdResult<DefaultAclDef> {
    Box::pin(state.ddl.load_default_acl(server_id, database_oid, scope)).await.map_err(|e| ddl_payload(&e))
}

#[tauri::command]
pub async fn ddl_lookup(
    state: State<'_, AppState>,
    server_id: Uuid,
    database_oid: Option<u32>,
    request: LookupRequest,
) -> CmdResult<Vec<LookupItem>> {
    Box::pin(state.ddl.lookup(server_id, database_oid, request)).await.map_err(|e| ddl_payload(&e))
}

#[tauri::command]
pub async fn ddl_preview(state: State<'_, AppState>, server_id: Uuid, database_oid: Option<u32>, request: PlanRequest) -> CmdResult<PlanView> {
    Box::pin(state.ddl.preview(server_id, database_oid, &request)).await.map_err(|e| ddl_payload(&e))
}

#[tauri::command]
pub async fn ddl_apply(
    state: State<'_, AppState>,
    server_id: Uuid,
    database_oid: Option<u32>,
    request: PlanRequest,
    options: ApplyOptions,
) -> CmdResult<ApplyOutcome> {
    Box::pin(state.ddl.apply(server_id, database_oid, &request, options)).await.map_err(|e| ddl_payload(&e))
}

#[tauri::command]
pub async fn ddl_cancel(state: State<'_, AppState>, apply_id: Uuid) -> CmdResult<()> {
    Box::pin(state.ddl.cancel(apply_id)).await.map_err(|e| ddl_payload(&e))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    use pgt_core::connection::ConnectionManager;
    use pgt_core::testing::admin_client;
    use pgt_ddl::{DdlManager, SchemaDef};

    const SCHEMA: &str = "pgt_t7_schema";

    fn def() -> ObjectDef {
        ObjectDef::Schema(SchemaDef { name: SCHEMA.into(), owner: None, comment: Some("t7".into()) })
    }

    fn opts(hash: &str) -> ApplyOptions {
        ApplyOptions { apply_id: Uuid::new_v4(), lock_timeout_ms: 5000, expected_hash: hash.into(), confirmed: false, confirmed_production: false, password: None }
    }

    #[test]
    fn bad_node_id_maps_to_invalid_code() {
        let err = pgt_ddl::target_of("srv:nope").map_err(|e| ddl_payload(&e)).unwrap_err();
        assert_eq!(err.code, "invalid");
    }

    /// Docker'lı sunucu gerekir (`PGT_TEST_URLS`); yoksa atlanır.
    #[tokio::test]
    async fn preview_apply_load_roundtrip_and_gates() {
        let Some(fx) = crate::query::testkit::fixture().await else { return };
        let sid = fx.ts.config.id;
        let admin = admin_client(&fx.ts, &fx.ts.config.maintenance_db).await;
        let db: u32 = admin.query_one("SELECT oid FROM pg_database WHERE datname = current_database()", &[]).await.unwrap().get(0);
        admin.batch_execute(&format!("DROP SCHEMA IF EXISTS {SCHEMA} CASCADE")).await.unwrap();
        let ddl = DdlManager::new(fx.manager.clone(), fx.registry.clone());

        let body = async {
            let req = PlanRequest::Create { def: def(), concurrently: false };
            let view = Box::pin(ddl.preview(sid, Some(db), &req)).await.map_err(|e| ddl_payload(&e)).unwrap();
            assert!(!view.empty && view.errors.is_empty());

            // Kilitli kayıt: apply `locked` koduyla reddedilir, preview serbest.
            let mut locked = fx.ts.config.clone();
            locked.read_only_lock = true;
            fx.registry.write().unwrap().upsert(locked).unwrap();
            let err = Box::pin(ddl.apply(sid, Some(db), &req, opts(&view.hash))).await.map_err(|e| ddl_payload(&e)).unwrap_err();
            assert_eq!(err.code, "locked");
            Box::pin(ddl.preview(sid, Some(db), &req)).await.unwrap();
            fx.registry.write().unwrap().upsert(fx.ts.config.clone()).unwrap();

            let out = Box::pin(ddl.apply(sid, Some(db), &req, opts(&view.hash))).await.map_err(|e| ddl_payload(&e)).unwrap();
            let ApplyOutcome::Applied { created, .. } = out else { panic!("{out:?}") };
            assert_eq!(created.map(|c| c.kind), Some(ObjectKind::Schema));

            let oid: u32 = admin.query_one("SELECT oid FROM pg_namespace WHERE nspname = $1", &[&SCHEMA]).await.unwrap().get(0);
            let loaded = Box::pin(ddl.load(sid, Some(db), ObjectRef { kind: ObjectKind::Schema, oid })).await.map_err(|e| ddl_payload(&e)).unwrap();
            let (ObjectDef::Schema(l), ObjectDef::Schema(d)) = (&loaded, &def()) else { panic!() };
            assert_eq!((&l.name, &l.comment), (&d.name, &d.comment));

            let roles = Box::pin(ddl.lookup(sid, Some(db), LookupRequest::Roles)).await.map_err(|e| ddl_payload(&e)).unwrap();
            assert!(!roles.is_empty());

            // Bağlı olmayan sunucu: `not_connected`.
            let idle = DdlManager::new(Arc::new(ConnectionManager::new()), fx.registry.clone());
            let err = Box::pin(idle.apply(sid, Some(db), &req, opts(&view.hash))).await.map_err(|e| ddl_payload(&e)).unwrap_err();
            assert_eq!(err.code, "not_connected");

            // Yürütme hatası `Ok(Failed)`, kapı reddi `Err(kod)`: aynı şemayı yeniden oluşturmak sunucuda 42P06 ile
            // düşer; yanlış hash ise bağlantı açılmadan `plan_changed` olur.
            let hash = Box::pin(ddl.preview(sid, Some(db), &req)).await.map_err(|e| ddl_payload(&e)).unwrap().hash;
            let out = Box::pin(ddl.apply(sid, Some(db), &req, opts(&hash))).await.map_err(|e| ddl_payload(&e)).unwrap();
            let ApplyOutcome::Failed { sqlstate, committed, .. } = out else { panic!("{out:?}") };
            assert_eq!((sqlstate.as_deref(), committed), (Some("42P06"), 0));
            let err = Box::pin(ddl.apply(sid, Some(db), &req, opts("yanlis"))).await.map_err(|e| ddl_payload(&e)).unwrap_err();
            assert_eq!(err.code, "plan_changed");

            // Sunucu istemcinin tanımına güvenmez: tahrifli `def` ile silme `plan_changed` olur, şema yerinde kalır.
            let ObjectDef::Schema(mut tampered) = loaded.clone() else { panic!() };
            tampered.comment = Some("sahte".into());
            let drop = PlanRequest::Drop { object: ObjectRef { kind: ObjectKind::Schema, oid }, def: ObjectDef::Schema(tampered), cascade: false, force: false };
            let hash = Box::pin(ddl.preview(sid, Some(db), &drop)).await.map_err(|e| ddl_payload(&e)).unwrap().hash;
            let mut o = opts(&hash);
            o.confirmed = true;
            let err = Box::pin(ddl.apply(sid, Some(db), &drop, o)).await.map_err(|e| ddl_payload(&e)).unwrap_err();
            assert_eq!(err.code, "plan_changed");
        };
        body.await;
        admin.batch_execute(&format!("DROP SCHEMA IF EXISTS {SCHEMA} CASCADE")).await.unwrap();
    }
}
