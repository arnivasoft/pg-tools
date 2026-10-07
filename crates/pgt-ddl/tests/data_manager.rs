//! `DataManager` gerçek sunucuda (Docker; `PGT_TEST_URLS` yoksa atlanır): kapılar, tek transaction'lı uygulama,
//! `RETURNING` gidiş-dönüşü, bayat satır (`row_conflict`), kilit zaman aşımı, iptal ve tip gidiş-dönüşü.
//!
//! Her test kendi geçici veritabanını (`pgt_data_<ana sürüm>_<etiket>`) ve rolünü (`pgt_data_role_<etiket>`) kurar;
//! başarısız doğrulamadan sonra bile temizlenir. Okuma, Query Tool gibi metin protokolüyle (`simple_query`) yapılır.
//! Günlük gizliliği süreç genelinde abone gerektirdiği için `logging.rs`'tedir.

mod common;

use std::sync::{Arc, RwLock};
use std::time::{Duration, Instant};

use common::{Env, cleanup_in, guarded, servers, setup_in};
use pgt_core::config::ServerConfig;
use pgt_core::connection::{ConnectionManager, MAX_STATEMENT_TIMEOUT_MS};
use pgt_core::registry::Registry;
use pgt_core::testing::{TestServer, admin_client};
use pgt_core::{Error, ErrorPayload, Result};
use pgt_ddl::data::manager::DataManager;
use pgt_ddl::data::model::{
    CellEdit, CellValue, DataApplyOptions, DataChangeSet, DataOutcome, DataSource, RowChange, RowResult, SelectRequest,
};
use pgt_ddl::data::plan::SAME_ROW_WARNING;
use pgt_ddl::error_codes::{APPLY_ID_IN_USE, NO_CHANGES};
use pgt_ddl::{ObjectKind, ObjectRef, PlanView, Risk, ddl_payload, quote_ident, quote_literal};
use tempfile::TempDir;
use tokio_postgres::{Client, SimpleQueryMessage};
use uuid::Uuid;

const PFX: &str = "pgt_data";

macro_rules! docker_test {
    ($name:ident, $body:ident, $tag:literal) => {
        #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
        async fn $name() {
            for ts in servers() {
                guarded($body(ts.clone()), cleanup_in(&ts, PFX, $tag)).await;
            }
        }
    };
}

// ---- yardımcılar ----

type Rows = Vec<Vec<Option<String>>>;

struct Fx {
    dm: Arc<DataManager>,
    server: Uuid,
    db: u32,
    _dir: TempDir,
}

fn manager(mgr: Arc<ConnectionManager>, config: &ServerConfig, locked: bool, label: Option<&str>) -> (Arc<DataManager>, TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let mut registry = Registry::load(dir.path().join("servers.json")).unwrap().registry;
    let mut config = config.clone();
    config.read_only_lock = locked;
    config.label = label.map(str::to_string);
    registry.upsert(config).unwrap();
    (Arc::new(DataManager::new(mgr, Arc::new(RwLock::new(registry)))), dir)
}

fn fx(env: &Env, ts: &TestServer, locked: bool, label: Option<&str>) -> Fx {
    let (dm, dir) = manager(env.mgr.clone(), &ts.config, locked, label);
    Fx { dm, server: env.server, db: env.db_oid, _dir: dir }
}

fn opts(hash: &str) -> DataApplyOptions {
    DataApplyOptions { apply_id: Uuid::new_v4(), lock_timeout_ms: 5000, expected_hash: hash.into(), confirmed: true }
}

fn t(v: &str) -> CellValue {
    CellValue::Text { value: v.into() }
}

fn e(c: &str, v: CellValue) -> CellEdit {
    CellEdit { column: c.into(), value: v }
}

fn ins(values: Vec<CellEdit>) -> RowChange {
    RowChange::Insert { values }
}

fn upd(key: &[&str], version: &str, set: Vec<CellEdit>) -> RowChange {
    RowChange::Update { key: key.iter().map(|k| k.to_string()).collect(), version: version.into(), set }
}

fn del(key: &[&str], version: &str) -> RowChange {
    RowChange::Delete { key: key.iter().map(|k| k.to_string()).collect(), version: version.into() }
}

fn set_of(src: &DataSource, changes: Vec<RowChange>) -> DataChangeSet {
    DataChangeSet { object: src.object, fingerprint: src.fingerprint.clone(), changes }
}

fn code<T: std::fmt::Debug>(r: &Result<T>) -> String {
    match r {
        Err(e) => ddl_payload(e).code,
        Ok(o) => panic!("hata bekleniyordu: {o:?}"),
    }
}

fn applied(r: Result<DataOutcome>) -> Vec<RowResult> {
    match r {
        Ok(DataOutcome::Applied { statements, results, .. }) => {
            assert_eq!(statements as usize, results.len());
            results
        }
        other => panic!("uygulanmalıydı: {other:?}"),
    }
}

/// (failed_index, sqlstate, position, error, conflict)
fn failed(r: Result<DataOutcome>) -> (u32, Option<String>, Option<u32>, ErrorPayload, bool) {
    match r {
        Ok(DataOutcome::Failed { failed_index, sqlstate, position, error, conflict }) => {
            (failed_index, sqlstate, position, error, conflict)
        }
        other => panic!("Failed bekleniyordu: {other:?}"),
    }
}

/// Çakışma: `conflict = true`, kod `row_conflict`, SQLSTATE/konum yok; başarısız sıra döner.
fn conflict(r: Result<DataOutcome>) -> u32 {
    let (idx, sqlstate, position, error, conflict) = failed(r);
    assert!(conflict, "çakışma bekleniyordu: {error:?}");
    assert_eq!((sqlstate, position, error.code.as_str()), (None, None, "row_conflict"));
    idx
}

/// Metin protokolüyle okur (Query Tool gibi).
async fn simple_rows(c: &Client, sql: &str) -> Rows {
    c.simple_query(sql)
        .await
        .unwrap_or_else(|e| panic!("okuma başarısız: {}", common::db_message(&e)))
        .into_iter()
        .filter_map(|m| match m {
            SimpleQueryMessage::Row(r) => Some((0..r.len()).map(|i| r.get(i).map(str::to_string)).collect()),
            _ => None,
        })
        .collect()
}

fn s(v: &str) -> Option<String> {
    Some(v.to_string())
}

/// İlk kolonu `id` olan satır.
fn row<'a>(rows: &'a Rows, id: &str) -> &'a Vec<Option<String>> {
    rows.iter().find(|r| r[0].as_deref() == Some(id)).unwrap_or_else(|| panic!("satır {id} yok: {rows:?}"))
}

fn has(rows: &Rows, id: &str) -> bool {
    rows.iter().any(|r| r[0].as_deref() == Some(id))
}

fn ver(r: &[Option<String>]) -> String {
    r.last().unwrap().clone().expect("sürüm")
}

fn returned(r: &RowResult) -> &Vec<Option<String>> {
    r.returned.as_ref().expect("RETURNING satırı")
}

impl Fx {
    async fn object(env: &Env, name: &str) -> ObjectRef {
        ObjectRef { kind: ObjectKind::Table, oid: env.class_oid("public", name).await }
    }

    async fn load(&self, env: &Env, name: &str) -> DataSource {
        self.dm.load(self.server, self.db, Self::object(env, name).await).await.unwrap()
    }

    async fn preview(&self, set: &DataChangeSet) -> PlanView {
        self.dm.preview(self.server, self.db, set).await.unwrap()
    }

    async fn apply_with(&self, set: &DataChangeSet, edit: impl FnOnce(&mut DataApplyOptions)) -> Result<DataOutcome> {
        let view = self.preview(set).await;
        assert!(view.errors.is_empty(), "önizleme hatası: {:?}", view.errors);
        let mut o = opts(&view.hash);
        edit(&mut o);
        self.dm.apply(self.server, self.db, set, &o).await
    }

    async fn apply(&self, set: &DataChangeSet) -> Result<DataOutcome> {
        self.apply_with(set, |_| {}).await
    }

    /// Üretilen SELECT ile tüm satırlar (anahtar sıralı; son kolon sürüm).
    async fn rows(&self, env: &Env, src: &DataSource) -> Rows {
        let req = SelectRequest { order: vec![], filter: None };
        let sel = self.dm.select(self.server, self.db, src.object, &src.fingerprint, &req).await.unwrap().unwrap();
        assert_eq!(sel.version_index as usize, src.columns.len());
        simple_rows(&env.admin, &sel.sql).await
    }
}

async fn db_of(env: &Env) -> String {
    env.admin.query_one("SELECT current_database()::text", &[]).await.unwrap().get(0)
}

/// Veri uygulama oturumlarından kaçı kilit bekliyor.
async fn data_waiting(admin: &Client) -> i64 {
    let sql = "SELECT count(*) FROM pg_stat_activity WHERE application_name = 'pg-tools data' \
               AND wait_event_type = 'Lock' AND datname = current_database()";
    admin.query_one(sql, &[]).await.unwrap().get(0)
}

async fn wait_for_waiting(admin: &Client, n: i64) {
    let deadline = Instant::now() + Duration::from_secs(15);
    while data_waiting(admin).await != n {
        assert!(Instant::now() < deadline, "{n} bekleyen veri oturumu görülmedi");
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

/// Satırı `FOR UPDATE` ile kilitleyen ayrı oturum (transaction açık kalır).
async fn lock_row(ts: &TestServer, env: &Env, table: &str, id: &str) -> Client {
    let locker = admin_client(ts, &db_of(env).await).await;
    locker
        .batch_execute(&format!("BEGIN; SELECT * FROM public.{table} WHERE id = {} FOR UPDATE", quote_literal(id)))
        .await
        .unwrap();
    locker
}

fn major(env: &Env) -> u32 {
    env.version / 10_000
}

// ================================================================================================================
// Mutlu yol
// ================================================================================================================

docker_test!(data_apply_insert_update_delete_roundtrip, happy_body, "happy");

async fn happy_body(ts: TestServer) {
    let env = setup_in(&ts, PFX, "happy").await;
    let f = fx(&env, &ts, false, None);
    env.sql(
        "CREATE TABLE t_h (id serial PRIMARY KEY, name text NOT NULL, status text DEFAULT 'new', note text, n int DEFAULT 7);
         CREATE FUNCTION t_h_trg() RETURNS trigger LANGUAGE plpgsql AS $$
           BEGIN NEW.note := coalesce(NEW.note, '') || '+trg'; RETURN NEW; END $$;
         CREATE TRIGGER t_h_trg BEFORE INSERT OR UPDATE ON t_h FOR EACH ROW EXECUTE PROCEDURE t_h_trg();",
    )
    .await;
    let src = f.load(&env, "t_h").await;
    assert!(src.editable && !src.locked, "{src:?}");

    // INSERT: serial + varsayılan + tetikleyicinin `NEW` değişikliği → dönen satır = okunan satır (sürüm dahil).
    let r = applied(f.apply(&set_of(&src, vec![ins(vec![e("name", t("ayşe"))])])).await);
    assert_eq!(r.len(), 1);
    assert_eq!(r[0].index, 0);
    let got = returned(&r[0]).clone();
    assert_eq!(got.len(), src.columns.len() + 1);
    assert_eq!(&got[..5], &[s("1"), s("ayşe"), s("new"), s("+trg"), s("7")]);
    let rows = f.rows(&env, &src).await;
    assert_eq!(row(&rows, "1"), &got, "RETURNING satırı okunan satırla aynı olmalı (xmin dahil)");

    // UPDATE iki kolon: yeni değerler ve yeni sürüm.
    let old = ver(&got);
    let r = applied(f.apply(&set_of(&src, vec![upd(&["1"], &old, vec![e("status", t("done")), e("note", CellValue::Null)])])).await);
    let got = returned(&r[0]).clone();
    assert_eq!(&got[..5], &[s("1"), s("ayşe"), s("done"), s("+trg"), s("7")]);
    assert_ne!(ver(&got), old, "UPDATE yeni sürüm döndürmeli");
    let rows = f.rows(&env, &src).await;
    assert_eq!(row(&rows, "1"), &got);

    // UPDATE … SET c = DEFAULT.
    let r = applied(f.apply(&set_of(&src, vec![upd(&["1"], &ver(&got), vec![e("n", CellValue::Default)])])).await);
    assert_eq!(returned(&r[0])[4], s("7"));
    let v1 = ver(returned(&r[0]));

    // DELETE: satır yok; dönen satır yok.
    let r = applied(f.apply(&set_of(&src, vec![del(&["1"], &v1)])).await);
    assert_eq!(r[0].returned, None);
    assert!(!has(&f.rows(&env, &src).await, "1"));

    // Üç işlemlik tek uygulama: hepsi görünür, sonuçlar istemci sırasıyla.
    applied(f.apply(&set_of(&src, vec![ins(vec![e("id", t("10")), e("name", t("a"))]), ins(vec![e("id", t("11")), e("name", t("b"))])])).await);
    let rows = f.rows(&env, &src).await;
    let (v10, v11) = (ver(row(&rows, "10")), ver(row(&rows, "11")));
    let r = applied(
        f.apply(&set_of(
            &src,
            vec![
                ins(vec![e("id", t("12")), e("name", t("c")), e("status", CellValue::Null)]),
                upd(&["10"], &v10, vec![e("name", t("a2"))]),
                del(&["11"], &v11),
            ],
        ))
        .await,
    );
    assert_eq!(r.iter().map(|x| x.index).collect::<Vec<_>>(), [0, 1, 2]);
    assert_eq!((returned(&r[0])[2].clone(), returned(&r[1])[1].clone(), r[2].returned.clone()), (None, s("a2"), None));
    let rows = f.rows(&env, &src).await;
    assert_eq!(rows.iter().map(|r| r[0].clone().unwrap()).collect::<Vec<_>>(), ["10", "12"]);
    assert_eq!(row(&rows, "10"), returned(&r[1]));
    assert_eq!(row(&rows, "12"), returned(&r[0]));
}

// ================================================================================================================
// Atomiklik
// ================================================================================================================

docker_test!(data_apply_is_all_or_nothing, atomic_body, "atomic");

async fn atomic_body(ts: TestServer) {
    let env = setup_in(&ts, PFX, "atomic").await;
    let f = fx(&env, &ts, false, None);
    env.sql("CREATE TABLE t_a (id int PRIMARY KEY, u text UNIQUE); INSERT INTO t_a VALUES (1, 'x'), (2, 'y');").await;
    let src = f.load(&env, "t_a").await;
    let rows = f.rows(&env, &src).await;
    let (v1, v2) = (ver(row(&rows, "1")), ver(row(&rows, "2")));

    // 2. işlem UNIQUE ihlali → Failed{1, 23505}; 1. işlem de kalıcı değil.
    let (idx, sqlstate, _, error, is_conflict) = failed(
        f.apply(&set_of(
            &src,
            vec![ins(vec![e("id", t("10")), e("u", t("p"))]), upd(&["1"], &v1, vec![e("u", t("y"))]), ins(vec![e("id", t("11"))])],
        ))
        .await,
    );
    assert_eq!((idx, sqlstate.as_deref(), is_conflict), (1, Some("23505"), false));
    assert_eq!(error.code, "query");
    let rows = f.rows(&env, &src).await;
    assert!(!has(&rows, "10") && !has(&rows, "11"));
    assert_eq!(row(&rows, "1")[1], s("x"));

    // Ortada çakışma (başka oturum satırı güncelledi) → conflict; önceki işlem yok.
    env.sql("UPDATE t_a SET u = 'y2' WHERE id = 2").await;
    let idx = conflict(
        f.apply(&set_of(
            &src,
            vec![ins(vec![e("id", t("12"))]), upd(&["2"], &v2, vec![e("u", t("z"))]), ins(vec![e("id", t("13"))])],
        ))
        .await,
    );
    assert_eq!(idx, 1);
    let rows = f.rows(&env, &src).await;
    assert!(!has(&rows, "12") && !has(&rows, "13"));
    assert_eq!(row(&rows, "2")[1], s("y2"));

    // Ertelenmiş kısıt: hata COMMIT'te gelir → Failed (son ifade), SQLSTATE'li (sonuç belli; hiçbir şey yazılmadı).
    env.sql(
        "CREATE TABLE t_d (id int PRIMARY KEY, u int, CONSTRAINT t_d_u UNIQUE (u) DEFERRABLE INITIALLY DEFERRED);
         INSERT INTO t_d VALUES (1, 1);",
    )
    .await;
    let d = f.load(&env, "t_d").await;
    let (idx, sqlstate, _, error, is_conflict) =
        failed(f.apply(&set_of(&d, vec![ins(vec![e("id", t("2")), e("u", t("1"))]), ins(vec![e("id", t("3")), e("u", t("3"))])])).await);
    assert_eq!((idx, sqlstate.as_deref(), is_conflict), (1, Some("23505"), false));
    assert!(error.hint.as_deref().is_none_or(|h| !h.contains("belirsiz")), "{error:?}");
    assert_eq!(f.rows(&env, &d).await.len(), 1);
}

// ================================================================================================================
// Bayatlık
// ================================================================================================================

docker_test!(data_apply_detects_stale_rows, stale_body, "stale");

async fn stale_body(ts: TestServer) {
    let env = setup_in(&ts, PFX, "stale").await;
    let f = fx(&env, &ts, false, None);
    env.sql("CREATE TABLE t_s (id int PRIMARY KEY, v text); INSERT INTO t_s VALUES (1, 'a'), (2, 'b'), (3, 'c'), (4, 'd');").await;
    let src = f.load(&env, "t_s").await;
    let rows = f.rows(&env, &src).await;
    let v = |id: &str| ver(row(&rows, id));

    // Başka oturum UPDATE → eski sürümle Update/Delete çakışır.
    env.sql("UPDATE t_s SET v = 'a2' WHERE id = 1").await;
    assert_eq!(conflict(f.apply(&set_of(&src, vec![upd(&["1"], &v("1"), vec![e("v", t("x"))])])).await), 0);
    assert_eq!(conflict(f.apply(&set_of(&src, vec![del(&["1"], &v("1"))])).await), 0);
    // Başka oturum DELETE → Delete/Update çakışır.
    env.sql("DELETE FROM t_s WHERE id = 2").await;
    assert_eq!(conflict(f.apply(&set_of(&src, vec![del(&["2"], &v("2"))])).await), 0);
    assert_eq!(conflict(f.apply(&set_of(&src, vec![upd(&["2"], &v("2"), vec![e("v", t("x"))])])).await), 0);
    // Var olmayan anahtar da çakışmadır.
    assert_eq!(conflict(f.apply(&set_of(&src, vec![del(&["99"], &v("3"))])).await), 0);

    // Aynı satıra aynı sürümle iki Update: önizleme uyarır, ikincisi çakışır, ilki de geri alınır.
    let same = set_of(&src, vec![upd(&["3"], &v("3"), vec![e("v", t("c1"))]), upd(&["3"], &v("3"), vec![e("v", t("c2"))])]);
    assert!(f.preview(&same).await.warnings.iter().any(|w| w == SAME_ROW_WARNING));
    assert_eq!(conflict(f.apply(&same).await), 1);
    assert_eq!(row(&f.rows(&env, &src).await, "3")[1], s("c"));

    // UPDATE SET pk: yeni anahtar dönüşte; yeni anahtar + dönen sürümle sonraki Update çalışır.
    let r = applied(f.apply(&set_of(&src, vec![upd(&["4"], &v("4"), vec![e("id", t("40"))])])).await);
    let got = returned(&r[0]).clone();
    assert_eq!(got[0], s("40"));
    let r = applied(f.apply(&set_of(&src, vec![upd(&["40"], &ver(&got), vec![e("v", t("d2"))])])).await);
    assert_eq!(returned(&r[0])[..2], [s("40"), s("d2")]);
    // Eski anahtar artık çakışır.
    assert_eq!(conflict(f.apply(&set_of(&src, vec![del(&["4"], &ver(&got))])).await), 0);

    // BEFORE UPDATE tetikleyicisi RETURN NULL → 0 satır → çakışma.
    env.sql(
        "CREATE TABLE t_s2 (id int PRIMARY KEY, v text); INSERT INTO t_s2 VALUES (1, 'a');
         CREATE FUNCTION t_s2_skip() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN RETURN NULL; END $$;
         CREATE TRIGGER t_s2_skip BEFORE UPDATE OR DELETE ON t_s2 FOR EACH ROW EXECUTE PROCEDURE t_s2_skip();",
    )
    .await;
    let s2 = f.load(&env, "t_s2").await;
    let v1 = ver(row(&f.rows(&env, &s2).await, "1"));
    assert_eq!(conflict(f.apply(&set_of(&s2, vec![upd(&["1"], &v1, vec![e("v", t("x"))])])).await), 0);
    assert_eq!(conflict(f.apply(&set_of(&s2, vec![del(&["1"], &v1)])).await), 0);
    assert_eq!(row(&f.rows(&env, &s2).await, "1")[1], s("a"));
}

// ================================================================================================================
// Parmak izi
// ================================================================================================================

docker_test!(data_fingerprint_mismatch_is_plan_changed, fp_body, "fp");

async fn fp_body(ts: TestServer) {
    let env = setup_in(&ts, PFX, "fp").await;
    let f = fx(&env, &ts, false, None);
    env.sql("CREATE TABLE t_fp (id int PRIMARY KEY, v text); INSERT INTO t_fp VALUES (1, 'a');").await;
    let src = f.load(&env, "t_fp").await;
    let set = set_of(&src, vec![ins(vec![e("id", t("2"))])]);
    let view = f.preview(&set).await;
    let req = SelectRequest { order: vec![], filter: None };

    // COMMENT ON: etkilenmez.
    env.sql("COMMENT ON TABLE t_fp IS 'yorum'; COMMENT ON COLUMN t_fp.v IS 'k'").await;
    assert_eq!(f.preview(&set).await.hash, view.hash);
    assert!(f.dm.select(f.server, f.db, src.object, &src.fingerprint, &req).await.unwrap().is_ok());

    // ADD COLUMN: preview/apply/select → plan_changed; hiçbir şey yazılmaz.
    env.sql("ALTER TABLE t_fp ADD COLUMN w int").await;
    assert_eq!(code(&f.dm.preview(f.server, f.db, &set).await), "plan_changed");
    assert_eq!(code(&f.dm.apply(f.server, f.db, &set, &opts(&view.hash)).await), "plan_changed");
    assert_eq!(code(&f.dm.select(f.server, f.db, src.object, &src.fingerprint, &req).await), "plan_changed");
    let fresh = f.load(&env, "t_fp").await;
    assert_ne!(fresh.fingerprint, src.fingerprint);
    assert_eq!(f.rows(&env, &fresh).await.len(), 1);
    // Yeni parmak iziyle çalışır.
    applied(f.apply(&set_of(&fresh, vec![ins(vec![e("id", t("2"))])])).await);
}

// ================================================================================================================
// Kapılar
// ================================================================================================================

docker_test!(data_apply_gates, gates_body, "gates");

async fn gates_body(ts: TestServer) {
    let env = setup_in(&ts, PFX, "gates").await;
    env.sql("CREATE TABLE t_g (id serial PRIMARY KEY, v text); INSERT INTO t_g (v) VALUES ('a'); CREATE VIEW v_g AS SELECT * FROM t_g;").await;
    let f = fx(&env, &ts, false, None);
    let src = f.load(&env, "t_g").await;
    let v1 = ver(row(&f.rows(&env, &src).await, "1"));
    let insert = set_of(&src, vec![ins(vec![e("v", t("b"))])]);
    let delete = set_of(&src, vec![del(&["1"], &v1)]);

    // Kayıtsız sunucu.
    assert!(matches!(f.dm.apply(Uuid::new_v4(), f.db, &insert, &opts("x")).await, Err(Error::UnknownServer(_))));
    assert!(matches!(f.dm.load(Uuid::new_v4(), f.db, src.object).await, Err(Error::UnknownServer(_))));

    // Kilitli sunucu: apply → locked; load/select/preview çalışır.
    let locked = fx(&env, &ts, true, None);
    let ls = locked.load(&env, "t_g").await;
    assert!(ls.locked && ls.editable);
    assert_eq!(locked.rows(&env, &ls).await.len(), 1);
    let view = locked.preview(&insert).await;
    assert!(view.errors.is_empty() && !view.empty);
    assert_eq!(code(&locked.dm.apply(locked.server, locked.db, &insert, &opts(&view.hash)).await), "locked");

    // Risk: INSERT Safe, DELETE Destructive → onaysız confirmation_required, onaylı uygulanır.
    assert_eq!(view.risk, Risk::Safe);
    assert_eq!(f.preview(&delete).await.risk, Risk::Destructive);
    assert_eq!(code(&f.apply_with(&delete, |o| o.confirmed = false).await), "confirmation_required");
    assert_eq!(f.rows(&env, &src).await.len(), 1);
    applied(f.apply_with(&insert, |o| o.confirmed = false).await);

    // Üretim sunucusu: Safe plan da onay ister.
    let prod = fx(&env, &ts, false, Some("Üretim"));
    assert_eq!(code(&prod.apply_with(&insert, |o| o.confirmed = false).await), "confirmation_required");
    applied(prod.apply_with(&insert, |o| o.confirmed = true).await);
    assert_eq!(f.rows(&env, &src).await.len(), 3);

    // expected_hash oynanırsa plan_changed; boş plan Invalid(NO_CHANGES); lock_timeout sınırı.
    assert_eq!(code(&f.apply_with(&insert, |o| o.expected_hash = "0000000000000000".into()).await), "plan_changed");
    let empty = set_of(&src, vec![]);
    let ev = f.preview(&empty).await;
    assert!(ev.empty && ev.errors.is_empty());
    assert!(matches!(f.dm.apply(f.server, f.db, &empty, &opts(&ev.hash)).await, Err(Error::Invalid(m)) if m == NO_CHANGES));
    let r = f.apply_with(&insert, |o| o.lock_timeout_ms = MAX_STATEMENT_TIMEOUT_MS + 1).await;
    assert!(matches!(r, Err(Error::Invalid(_))), "{r:?}");

    // Plan doğrulama hataları PlanView.errors'ta aynen; apply → invalid_def.
    let bad = set_of(
        &src,
        vec![
            ins(vec![e("nope", t("x"))]),
            upd(&["1", "2"], "1", vec![e("v", t("x"))]),
            upd(&["1"], "x1", vec![e("v", t("x")), e("v", t("y"))]),
            del(&["1\0"], "1"),
        ],
    );
    let bv = f.preview(&bad).await;
    assert_eq!(
        bv.errors.iter().map(|e| e.path.as_str()).collect::<Vec<_>>(),
        ["changes[0].values[0]", "changes[1].key", "changes[2].version", "changes[2].set[1]", "changes[3].key[0]"]
    );
    assert!(bv.empty);
    assert_eq!(code(&f.dm.apply(f.server, f.db, &bad, &opts(&bv.hash)).await), "invalid_def");
    let too_many = set_of(&src, (0..2001).map(|_| ins(vec![])).collect());
    assert_eq!(f.preview(&too_many).await.errors.iter().map(|e| e.path.as_str()).collect::<Vec<_>>(), ["changes"]);

    // Salt okunur kaynak (görünüm): select çalışır; preview hatası `object`, apply invalid_def.
    let vobj = ObjectRef { kind: ObjectKind::View, oid: env.class_oid("public", "v_g").await };
    let vs = f.dm.load(f.server, f.db, vobj).await.unwrap();
    assert!(!vs.editable);
    let sel = f.dm.select(f.server, f.db, vobj, &vs.fingerprint, &SelectRequest { order: vec![], filter: None }).await.unwrap().unwrap();
    assert!(!sel.has_version);
    assert_eq!(simple_rows(&env.admin, &sel.sql).await.len(), 3);
    let vset = DataChangeSet { object: vobj, fingerprint: vs.fingerprint.clone(), changes: vec![ins(vec![e("v", t("x"))])] };
    let vv = f.preview(&vset).await;
    assert_eq!(vv.errors.iter().map(|e| e.path.as_str()).collect::<Vec<_>>(), ["object"]);
    assert_eq!(code(&f.dm.apply(f.server, f.db, &vset, &opts(&vv.hash)).await), "invalid_def");
    // Desteklenmeyen tür (şema) yüklenemez.
    let schema = ObjectRef { kind: ObjectKind::Schema, oid: env.schema_oid("public").await };
    assert!(matches!(f.dm.load(f.server, f.db, schema).await, Err(Error::Invalid(_))));

    // Yetkiler: giriş yapabilen test rolü. Tablo düzeyi SELECT/INSERT + kolon düzeyi UPDATE (v):
    // `id` güncellenemez, tablo düzeyi UPDATE/DELETE yok → alan hataları; sequence USAGE yok → PG 42501.
    let pw = format!("Pw{}", Uuid::new_v4().simple());
    let role = quote_ident(&env.role);
    env.sql(&format!(
        "ALTER ROLE {role} LOGIN PASSWORD {}; GRANT USAGE ON SCHEMA public TO {role};
         GRANT SELECT, INSERT ON t_g TO {role}; GRANT UPDATE (v) ON t_g TO {role};",
        quote_literal(&pw)
    ))
    .await;
    let mut config = ts.config.clone();
    config.id = Uuid::new_v4();
    config.username = env.role.clone();
    let mgr = Arc::new(ConnectionManager::new());
    mgr.connect(config.clone(), Some(pw)).await.expect("test rolüyle bağlanılamadı");
    let (dm, _dir) = manager(mgr.clone(), &config, false, None);
    let rf = Fx { dm, server: config.id, db: env.db_oid, _dir };
    let rs = rf.load(&env, "t_g").await;
    assert!(rs.editable && rs.can_insert && !rs.can_update && !rs.can_delete, "{rs:?}");
    let col = |n: &str| rs.columns.iter().find(|c| c.name == n).unwrap().clone();
    assert_eq!((col("id").updatable, col("v").updatable, col("id").insertable), (false, true, true));
    let rv = ver(row(&rf.rows(&env, &rs).await, "1"));
    let denied = set_of(&rs, vec![upd(&["1"], &rv, vec![e("v", t("x"))]), del(&["1"], &rv)]);
    assert_eq!(rf.preview(&denied).await.errors.iter().map(|e| e.path.as_str()).collect::<Vec<_>>(), ["changes[0]", "changes[1]"]);
    // serial varsayılanı sequence USAGE ister: sunucu reddeder (42501), hiçbir şey yazılmaz.
    let r_ins = set_of(&rs, vec![ins(vec![e("v", t("r"))])]);
    let (idx, sqlstate, _, error, is_conflict) = failed(rf.apply(&r_ins).await);
    assert_eq!((idx, sqlstate.as_deref(), is_conflict, error.code.as_str()), (0, Some("42501"), false, "query"));
    env.sql(&format!("GRANT USAGE ON SEQUENCE t_g_id_seq TO {role}")).await;
    let r = applied(rf.apply(&r_ins).await);
    assert_eq!(returned(&r[0])[1], s("r"));
    mgr.disconnect(config.id);
}

// ================================================================================================================
// Kilit zaman aşımı, iptal, eşzamanlı kimlik
// ================================================================================================================

docker_test!(data_apply_lock_timeout_and_cancel, cancel_body, "cancel");

async fn cancel_body(ts: TestServer) {
    let env = setup_in(&ts, PFX, "cancel").await;
    let f = fx(&env, &ts, false, None);
    env.sql("CREATE TABLE t_c (id int PRIMARY KEY, v text); INSERT INTO t_c VALUES (1, 'a');").await;
    let src = f.load(&env, "t_c").await;
    let v1 = ver(row(&f.rows(&env, &src).await, "1"));
    let set = set_of(&src, vec![ins(vec![e("id", t("2"))]), upd(&["1"], &v1, vec![e("v", t("x"))])]);

    // Başka oturum satırı kilitler: lock_timeout 300 ms → 55P03; ilk işlem de yazılmadı.
    let locker = lock_row(&ts, &env, "t_c", "1").await;
    let started = Instant::now();
    let (idx, sqlstate, _, error, is_conflict) = failed(f.apply_with(&set, |o| o.lock_timeout_ms = 300).await);
    assert_eq!((idx, sqlstate.as_deref(), is_conflict), (1, Some("55P03"), false));
    assert!(error.message.contains("lock_timeout"), "{error:?}");
    assert!(started.elapsed() < Duration::from_secs(10));
    assert!(!has(&f.rows(&env, &src).await, "2"));

    // lock_timeout 0 + cancel(apply_id) → 57014.
    let hash = f.preview(&set).await.hash;
    let o = DataApplyOptions { lock_timeout_ms: 0, ..opts(&hash) };
    let id = o.apply_id;
    let (dm, server, db, s2) = (f.dm.clone(), f.server, f.db, set.clone());
    let task = tokio::spawn(async move { dm.apply(server, db, &s2, &o).await });
    wait_for_waiting(&env.admin, 1).await;

    // Aynı apply_id ile ikinci uygulama: APPLY_ID_IN_USE (bağlantı açmadan).
    let dup = f.dm.apply(f.server, f.db, &set, &DataApplyOptions { apply_id: id, lock_timeout_ms: 0, ..opts(&hash) }).await;
    assert!(matches!(&dup, Err(Error::Invalid(m)) if m == APPLY_ID_IN_USE), "{dup:?}");

    f.dm.cancel(id).await.unwrap();
    let (idx, sqlstate, _, error, _) = failed(task.await.unwrap());
    assert_eq!((idx, sqlstate.as_deref()), (1, Some("57014")));
    assert!(error.message.contains("iptal"), "{error:?}");
    // Bitmiş ve bilinmeyen kimlik: Ok.
    f.dm.cancel(id).await.unwrap();
    f.dm.cancel(Uuid::new_v4()).await.unwrap();

    // cancel_all: Tokio bağlamı olmayan iş parçacığından.
    let o = DataApplyOptions { lock_timeout_ms: 0, ..opts(&hash) };
    let (dm, s2) = (f.dm.clone(), set.clone());
    let task = tokio::spawn(async move { dm.apply(server, db, &s2, &o).await });
    wait_for_waiting(&env.admin, 1).await;
    let dm = f.dm.clone();
    let n = std::thread::spawn(move || {
        assert!(tokio::runtime::Handle::try_current().is_err());
        dm.cancel_all()
    })
    .join()
    .unwrap();
    assert_eq!(n, 1);
    let (_, sqlstate, _, _, _) = failed(tokio::time::timeout(Duration::from_secs(10), task).await.expect("iptal ulaşmadı").unwrap());
    assert_eq!(sqlstate.as_deref(), Some("57014"));
    locker.batch_execute("COMMIT").await.unwrap();
    assert!(!has(&f.rows(&env, &src).await, "2"));

    // Kayıttan önce gelen iptal (mezar taşı): bağlantı açmadan iptal sonucu, hiçbir şey yazılmaz; tüketilir.
    let id = Uuid::new_v4();
    f.dm.cancel(id).await.unwrap();
    let (idx, sqlstate, _, _, _) = failed(f.apply_with(&set, |o| o.apply_id = id).await);
    assert_eq!((idx, sqlstate.as_deref()), (0, Some("57014")));
    assert!(!has(&f.rows(&env, &src).await, "2"));
    applied(f.apply_with(&set, |o| o.apply_id = id).await);
    assert_eq!(row(&f.rows(&env, &src).await, "1")[1], s("x"));
}

// ================================================================================================================
// Tip gidiş-dönüşü
// ================================================================================================================

docker_test!(data_types_roundtrip_through_text, types_body, "types");

async fn types_body(ts: TestServer) {
    let env = setup_in(&ts, PFX, "types").await;
    let f = fx(&env, &ts, false, None);
    let xml = env.try_sql("SELECT '<a/>'::xml").await.is_ok();
    let citext = env.has_extension("citext").await && env.try_sql("CREATE EXTENSION citext").await.is_ok();
    env.sql(
        "CREATE TYPE mood AS ENUM ('mutlu', 'üzgün');
         CREATE TABLE t_ty (id int PRIMARY KEY, b boolean, n numeric(12,3), f float8, by bytea, j json, jb jsonb,
           arr text[], ts timestamptz, u uuid, m mood, nul text, empty text);",
    )
    .await;
    if xml {
        env.sql("ALTER TABLE t_ty ADD COLUMN x xml").await;
    } else {
        eprintln!("PG {}: xml desteği yok; xml kolonu atlandı", major(&env));
    }
    if citext {
        env.sql("ALTER TABLE t_ty ADD COLUMN ci citext").await;
    } else {
        eprintln!("PG {}: citext yok; citext kolonu atlandı", major(&env));
    }
    let src = f.load(&env, "t_ty").await;
    let mut values = vec![
        ("id", t("1")),
        ("b", t("t")),
        ("n", t("12.500")),
        ("f", t("1.5")),
        ("by", t("\\x00ff10")),
        ("j", t("{\"b\": 1,  \"a\": [1, 2]}")),
        ("jb", t("{\"b\": 1, \"a\": [1, 2]}")),
        ("arr", t("{a,\"b c\",NULL,\"\"}")),
        ("ts", t("2024-02-29 12:34:56.789+00")),
        ("u", t("a0eebc99-9c0b-4ef8-bb6d-6bb9bd380a11")),
        ("m", t("üzgün")),
        ("nul", CellValue::Null),
        ("empty", t("")),
    ];
    if xml {
        values.push(("x", t("<a b=\"1\">ç</a>")));
    }
    if citext {
        values.push(("ci", t("MiXeD")));
    }
    let r = applied(f.apply(&set_of(&src, vec![ins(values.iter().map(|(c, v)| e(c, v.clone())).collect())])).await);
    let got = returned(&r[0]).clone();
    let rows = f.rows(&env, &src).await;
    let read = row(&rows, "1").clone();
    assert_eq!(read, got, "RETURNING = okunan satır");
    let idx = |c: &str| src.columns.iter().position(|k| k.name == c).unwrap();
    assert_eq!(read[idx("b")], s("t"));
    assert_eq!(read[idx("n")], s("12.500"));
    assert_eq!(read[idx("f")], s("1.5"));
    assert_eq!(read[idx("by")], s("\\x00ff10"));
    assert_eq!(read[idx("j")], s("{\"b\": 1,  \"a\": [1, 2]}"), "json metni aynen korunur");
    assert_eq!(read[idx("jb")], s("{\"a\": [1, 2], \"b\": 1}"));
    assert_eq!(read[idx("arr")], s("{a,\"b c\",NULL,\"\"}"));
    assert_eq!(read[idx("u")], s("a0eebc99-9c0b-4ef8-bb6d-6bb9bd380a11"));
    assert_eq!(read[idx("m")], s("üzgün"));
    assert_eq!((read[idx("nul")].clone(), read[idx("empty")].clone()), (None, s("")), "NULL ≠ ''");
    if citext {
        assert_eq!(read[idx("ci")], s("MiXeD"));
    }

    // Okunan değerlerle Update: sürüm değişir, değerler aynı kalır.
    let set: Vec<CellEdit> = src
        .columns
        .iter()
        .enumerate()
        .filter(|(_, c)| c.name != "id")
        .map(|(i, c)| e(&c.name, read[i].as_deref().map_or(CellValue::Null, t)))
        .collect();
    let r = applied(f.apply(&set_of(&src, vec![upd(&["1"], &ver(&read), set)])).await);
    let again = returned(&r[0]).clone();
    assert_ne!(ver(&again), ver(&read));
    assert_eq!(again[..src.columns.len()], read[..src.columns.len()]);
    assert_eq!(row(&f.rows(&env, &src).await, "1"), &again);
}

// ================================================================================================================
// Bölümlü tablo ve kalıtım
// ================================================================================================================

docker_test!(data_partitioned_and_inherited_tables, part_body, "part");

async fn part_body(ts: TestServer) {
    let env = setup_in(&ts, PFX, "part").await;
    let f = fx(&env, &ts, false, None);
    if major(&env) >= 11 {
        // Satır düzeyi tetikleyici alt transaction (EXCEPTION bloğu) açar; dönen sürüm yine satırın xmin'idir.
        env.sql(
            "CREATE TABLE t_p (id int, k int, v text, PRIMARY KEY (id, k)) PARTITION BY RANGE (k);
             CREATE TABLE t_p1 PARTITION OF t_p FOR VALUES FROM (0) TO (50);
             CREATE TABLE t_p2 PARTITION OF t_p FOR VALUES FROM (50) TO (100);
             CREATE FUNCTION t_p_trg() RETURNS trigger LANGUAGE plpgsql AS $$
               BEGIN BEGIN PERFORM 1 / 0; EXCEPTION WHEN division_by_zero THEN NULL; END; RETURN NEW; END $$;
             CREATE TRIGGER t_p_trg BEFORE INSERT ON t_p1 FOR EACH ROW EXECUTE PROCEDURE t_p_trg();",
        )
        .await;
        let src = f.load(&env, "t_p").await;
        assert!(src.editable, "{src:?}");
        let r = applied(f.apply(&set_of(&src, vec![ins(vec![e("id", t("1")), e("k", t("5")), e("v", t("a"))]), ins(vec![e("id", t("2")), e("k", t("70"))])])).await);
        let rows = f.rows(&env, &src).await;
        assert_eq!(row(&rows, "1"), returned(&r[0]), "bölümlü üst tabloya INSERT: dönen sürüm = okunan xmin");
        assert_eq!(row(&rows, "2"), returned(&r[1]));
        // Bölüm anahtarını değiştiren Update: satır bölümler arası taşınır.
        let r = applied(f.apply(&set_of(&src, vec![upd(&["1", "5"], &ver(returned(&r[0])), vec![e("k", t("60"))])])).await);
        let got = returned(&r[0]).clone();
        assert_eq!(&got[..3], &[s("1"), s("60"), s("a")]);
        let rows = f.rows(&env, &src).await;
        assert_eq!(row(&rows, "1"), &got);
        let in_p2: i64 = env.admin.query_one("SELECT count(*) FROM t_p2 WHERE id = 1", &[]).await.unwrap().get(0);
        assert_eq!(in_p2, 1);
        // Yeni anahtar + sürümle sonraki Update ve Delete.
        let r = applied(f.apply(&set_of(&src, vec![upd(&["1", "60"], &ver(&got), vec![e("v", t("b"))])])).await);
        applied(f.apply(&set_of(&src, vec![del(&["1", "60"], &ver(returned(&r[0])))])).await);
        assert!(!has(&f.rows(&env, &src).await, "1"));
        // Bölümün kendisi de düzenlenebilir (sıradan tablo).
        let p1 = f.load(&env, "t_p1").await;
        let r = applied(f.apply(&set_of(&p1, vec![ins(vec![e("id", t("3")), e("k", t("6"))])])).await);
        assert_eq!(row(&f.rows(&env, &p1).await, "3"), returned(&r[0]));
    } else {
        eprintln!("PG {}: bölümlü tabloda PK (PG 11+) yok; bölümlü adım atlandı", major(&env));
    }

    // Kalıtım: çocuk kendi PK'sıyla düzenlenir; üst tablodan okunan çocuk satırı üst tablo üzerinden güncellenir.
    env.sql(
        "CREATE TABLE t_par (id int PRIMARY KEY, v text);
         CREATE TABLE t_chi (extra int) INHERITS (t_par);
         ALTER TABLE t_chi ADD PRIMARY KEY (id);",
    )
    .await;
    let chi = f.load(&env, "t_chi").await;
    let r = applied(f.apply(&set_of(&chi, vec![ins(vec![e("id", t("1")), e("v", t("c")), e("extra", t("9"))])])).await);
    assert_eq!(row(&f.rows(&env, &chi).await, "1"), returned(&r[0]));
    let par = f.load(&env, "t_par").await;
    let rows = f.rows(&env, &par).await;
    let pr = row(&rows, "1").clone();
    let r = applied(f.apply(&set_of(&par, vec![upd(&["1"], &ver(&pr), vec![e("v", t("p"))])])).await);
    assert_eq!(&returned(&r[0])[..2], &[s("1"), s("p")]);
    assert_eq!(row(&f.rows(&env, &chi).await, "1")[..3], [s("1"), s("p"), s("9")]);
}
