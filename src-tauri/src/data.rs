//! Veri düzenleyici Tauri komutları (`data_*`). Hücre/anahtar/sürüm değerleri, SQL metni, ad ve URL asla günlüğe yazılmaz.
//!
//! `ddl.rs` kuralları aynen: her komutun geleceği `Box::pin` ile kutulanır (release'te 1 MB ana yığın taşmasın);
//! hata eşlemesi tek yerden `ddl_payload`; `tracing` çağrısı yok (yöneticide yalnızca işlem/sayı/süre/kod).
//! Kapı hataları (`locked`, `confirmation_required`, `plan_changed`, kayıtsız sunucu) `Err`; yürütme hataları
//! `Ok(DataOutcome::Failed{..})`; `data_select` filtre/sıralama alan hataları `Ok(DataSelectResult::Invalid)` (veri).
//! Argüman adları arayüzde camelCase görünür (`server_id` -> `serverId`, `database_oid` -> `databaseOid`).

use pgt_core::ErrorPayload;
use pgt_ddl::data::model::{DataApplyOptions, DataChangeSet, DataOutcome, DataSelectResult, DataSource, SelectRequest};
use pgt_ddl::{ObjectRef, PlanView, ddl_payload};
use tauri::State;
use uuid::Uuid;

use crate::state::AppState;

type CmdResult<T> = Result<T, ErrorPayload>;

#[tauri::command]
pub async fn data_load(state: State<'_, AppState>, server_id: Uuid, database_oid: u32, object: ObjectRef) -> CmdResult<DataSource> {
    Box::pin(state.data.load(server_id, database_oid, object)).await.map_err(|e| ddl_payload(&e))
}

#[tauri::command]
pub async fn data_select(
    state: State<'_, AppState>,
    server_id: Uuid,
    database_oid: u32,
    object: ObjectRef,
    fingerprint: String,
    request: SelectRequest,
) -> CmdResult<DataSelectResult> {
    Box::pin(state.data.select(server_id, database_oid, object, &fingerprint, &request))
        .await
        .map(DataSelectResult::from)
        .map_err(|e| ddl_payload(&e))
}

#[tauri::command]
pub async fn data_preview(state: State<'_, AppState>, server_id: Uuid, database_oid: u32, change_set: DataChangeSet) -> CmdResult<PlanView> {
    Box::pin(state.data.preview(server_id, database_oid, &change_set)).await.map_err(|e| ddl_payload(&e))
}

#[tauri::command]
pub async fn data_apply(
    state: State<'_, AppState>,
    server_id: Uuid,
    database_oid: u32,
    change_set: DataChangeSet,
    options: DataApplyOptions,
) -> CmdResult<DataOutcome> {
    Box::pin(state.data.apply(server_id, database_oid, &change_set, &options)).await.map_err(|e| ddl_payload(&e))
}

#[tauri::command]
pub async fn data_cancel(state: State<'_, AppState>, apply_id: Uuid) -> CmdResult<()> {
    Box::pin(state.data.cancel(apply_id)).await.map_err(|e| ddl_payload(&e))
}

#[cfg(test)]
mod tests {
    use super::*;

    use pgt_core::testing::admin_client;
    use pgt_ddl::ObjectKind;
    use pgt_ddl::data::model::{CellEdit, CellValue, RowChange};
    use pgt_query::events::QueryEvent;
    use pgt_query::session::TxMode;

    const TABLE: &str = "pgt_data_t6";

    /// `data_select` SQL'ini Query Tool oturumunda çalıştırır; (satırlar, sürüm kolonu indeksi).
    async fn run_select(fx: &crate::query::testkit::Fx, sql: &str) -> Vec<Vec<Option<String>>> {
        let id = fx.open(TxMode::Autocommit).await;
        let evs = fx.exec(id, sql).await;
        evs.into_iter().filter_map(|e| if let QueryEvent::Rows { rows, .. } = e { Some(rows) } else { None }).flatten().collect()
    }

    fn text(c: &str, v: &str) -> CellEdit {
        CellEdit { column: c.into(), value: CellValue::Text { value: v.into() } }
    }

    fn opts(hash: &str) -> DataApplyOptions {
        DataApplyOptions { apply_id: Uuid::new_v4(), lock_timeout_ms: 5000, expected_hash: hash.into(), confirmed: true }
    }

    /// Docker'lı sunucu gerekir (`PGT_TEST_URLS`); yoksa atlanır.
    #[tokio::test]
    async fn load_select_preview_apply_roundtrip_and_gates() {
        let Some(fx) = crate::query::testkit::fixture().await else { return };
        let sid = fx.ts.config.id;
        let admin = admin_client(&fx.ts, &fx.ts.config.maintenance_db).await;
        let db: u32 = admin.query_one("SELECT oid FROM pg_database WHERE datname = current_database()", &[]).await.unwrap().get(0);
        admin.batch_execute(&format!("DROP TABLE IF EXISTS public.{TABLE}")).await.unwrap();
        admin
            .batch_execute(&format!(
                "CREATE TABLE public.{TABLE} (id int PRIMARY KEY, name text); INSERT INTO public.{TABLE} VALUES (1,'a'),(2,'b'),(3,'c')"
            ))
            .await
            .unwrap();
        let oid: u32 = admin.query_one(&format!("SELECT 'public.{TABLE}'::regclass::oid"), &[]).await.unwrap().get(0);
        let object = ObjectRef { kind: ObjectKind::Table, oid };
        // Komutlar `State` ister (testte kurulamaz); komut gövdeleri yalnızca yöneticiyi sarar, yönetici doğrudan test edilir.
        let data = pgt_ddl::data::DataManager::new(fx.manager.clone(), fx.registry.clone());

        let body = async {
            let src = Box::pin(data.load(sid, db, object)).await.map_err(|e| ddl_payload(&e)).unwrap();
            assert!(src.editable && src.key.is_some());
            let fp = src.fingerprint.clone();

            let req = SelectRequest { order: vec![], filter: None };
            let sel = Box::pin(data.select(sid, db, object, &fp, &req)).await.map_err(|e| ddl_payload(&e)).unwrap();
            let sel = DataSelectResult::from(sel);
            let DataSelectResult::Ok { select } = sel else { panic!("{sel:?}") };
            let rows = run_select(&fx, &select.sql).await;
            assert_eq!(rows.len(), 3);
            let vi = select.version_index as usize;
            let ver = |id: &str| rows.iter().find(|r| r[0].as_deref() == Some(id)).unwrap()[vi].clone().unwrap();

            let set = DataChangeSet {
                object,
                fingerprint: fp.clone(),
                changes: vec![
                    RowChange::Insert { values: vec![text("id", "4"), text("name", "d")] },
                    RowChange::Update { key: vec!["1".into()], version: ver("1"), set: vec![text("name", "A")] },
                    RowChange::Delete { key: vec!["2".into()], version: ver("2") },
                ],
            };
            let view = Box::pin(data.preview(sid, db, &set)).await.map_err(|e| ddl_payload(&e)).unwrap();
            assert!(!view.empty && view.errors.is_empty());

            // Hatalı kaynak (olmayan kolon): alan hatası, kapı hatası değil.
            let mut bad = set.clone();
            bad.changes = vec![RowChange::Insert { values: vec![text("yok_kolon", "x")] }];
            let bv = Box::pin(data.preview(sid, db, &bad)).await.map_err(|e| ddl_payload(&e)).unwrap();
            assert!(!bv.errors.is_empty());

            // Kilitli sunucu: apply `locked`, select/preview serbest.
            let mut locked = fx.ts.config.clone();
            locked.read_only_lock = true;
            fx.registry.write().unwrap().upsert(locked).unwrap();
            let err = Box::pin(data.apply(sid, db, &set, &opts(&view.hash))).await.map_err(|e| ddl_payload(&e)).unwrap_err();
            assert_eq!(err.code, "locked");
            Box::pin(data.select(sid, db, object, &fp, &req)).await.unwrap().unwrap();
            fx.registry.write().unwrap().upsert(fx.ts.config.clone()).unwrap();

            let out = Box::pin(data.apply(sid, db, &set, &opts(&view.hash))).await.map_err(|e| ddl_payload(&e)).unwrap();
            let DataOutcome::Applied { statements, .. } = out else { panic!("{out:?}") };
            assert_eq!(statements, 3);

            let after = run_select(&fx, &select.sql).await;
            let mut got: Vec<(String, String)> = after.iter().map(|r| (r[0].clone().unwrap(), r[1].clone().unwrap())).collect();
            got.sort();
            let want: Vec<(String, String)> = [("1", "A"), ("3", "c"), ("4", "d")].iter().map(|(a, b)| (a.to_string(), b.to_string())).collect();
            assert_eq!(got, want);

            // Filtre hatası veri olarak döner.
            let badf = SelectRequest { order: vec![], filter: Some("1=1); DROP TABLE x; --".into()) };
            let r = Box::pin(data.select(sid, db, object, &fp, &badf)).await.map_err(|e| ddl_payload(&e)).unwrap();
            assert!(matches!(DataSelectResult::from(r), DataSelectResult::Invalid { ref errors } if !errors.is_empty()));

            // İptal: bilinmeyen kimlik idempotent Ok.
            Box::pin(data.cancel(Uuid::new_v4())).await.unwrap();
        };
        body.await;
        admin.batch_execute(&format!("DROP TABLE IF EXISTS public.{TABLE}")).await.unwrap();
    }

    /// Beş komut `main.rs` kayıt listesinde (derleme zamanı: işlevler adlandırılabilir olmalı).
    #[test]
    fn five_commands_exist() {
        let _ = (data_load, data_select, data_preview, data_apply, data_cancel);
    }
}
