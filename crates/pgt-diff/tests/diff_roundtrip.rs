//! Şema karşılaştırma uçtan uca (Docker; `PGT_TEST_URLS` yoksa atlanır): `run_diff` -> betik -> hedefe uygulama -> yeniden
//! karşılaştırma, ayrıca sahiplik/yetki, bölümleme, çapraz sürüm, uzantı, iptal, eşzamanlılık, günlük gizliliği ve performans.
//!
//! Veritabanları `pgt_diff_<ana sürüm>_<etiket>`: `rta` (aynı veritabanı), `rtbs`/`rtbt` (iki veritabanı), `ord`, `own`,
//! `rec`, `part`, `xv`, `ext`, `cx`, `log`, `perf`.

mod common;

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Instant;

use common::{
    Env, assert_no_idle_in_tx_for, cleanup_in, db_message, db_name_in, guarded, rt_fixture,
    servers, setup_in,
};
use pgt_core::connection::{ConnectionManager, Target};
use pgt_core::testing::TestServer;
use pgt_ddl::ObjectKind;
use pgt_diff::{
    CancelToken, DiffData, DiffEvent, DiffOptions, DiffSide, DiffSink, ItemProblem, ItemStatus,
    RunError, ScriptError, SideTag, run_diff,
};

const PFX: &str = "pgt_diff";
const STAMP: &str = "2026-11-05 00:00:00 UTC";

// --- günlük yakalama (süreç geneli abone; `pgt-ddl/tests/logging.rs` deseni) ---

#[derive(Clone, Default)]
struct Buf(Arc<Mutex<Vec<u8>>>);

impl std::io::Write for Buf {
    fn write(&mut self, b: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(b);
        Ok(b.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

/// Aynı tampon bu ikilideki bütün testlerce paylaşılır; her test yalnız kendi işaretlerine bakar. Her test başında çağrılır
/// (abone ilk çağrıda kurulur; sonradan kurulursa önceki çağrı yerleri olayları kaçırır).
fn capture() -> Buf {
    static BUF: OnceLock<Buf> = OnceLock::new();
    BUF.get_or_init(|| {
        let buf = Buf::default();
        let w = buf.clone();
        let subscriber = tracing_subscriber::fmt()
            .with_writer(move || w.clone())
            .with_max_level(tracing::Level::TRACE)
            .finish();
        tracing::subscriber::set_global_default(subscriber)
            .expect("bu test ikilisinde başka global abone yok");
        buf
    })
    .clone()
}

// --- yardımcılar ---

/// Olayları toplar; istenirse ilk ilerleme olayında işi iptal eder.
#[derive(Default)]
struct Events {
    all: Vec<DiffEvent>,
    cancel_on_first: Option<CancelToken>,
}

impl DiffSink for Events {
    fn send(&mut self, e: DiffEvent) -> bool {
        self.all.push(e);
        if let Some(t) = &self.cancel_on_first {
            t.cancel();
        }
        true
    }
}

fn side(env: &Env, schema: &str) -> DiffSide {
    DiffSide {
        server_id: env.server,
        database_oid: env.db_oid,
        schema: schema.to_string(),
    }
}

async fn run(
    mgr: &ConnectionManager,
    src: &DiffSide,
    tgt: &DiffSide,
    options: DiffOptions,
) -> (Result<DiffData, RunError>, Vec<DiffEvent>) {
    let mut sink = Events::default();
    let r = Box::pin(run_diff(
        mgr,
        src,
        tgt,
        options,
        &mut sink,
        &CancelToken::new(),
    ))
    .await;
    (r, sink.all)
}

async fn run_ok(
    mgr: &ConnectionManager,
    src: &DiffSide,
    tgt: &DiffSide,
    options: DiffOptions,
) -> (DiffData, Vec<DiffEvent>) {
    let (r, events) = run(mgr, src, tgt, options).await;
    match r {
        Ok(d) => (d, events),
        Err(e) => panic!("karşılaştırma başarısız: {e:?}"),
    }
}

/// Her tarafta ilerleme olayları gelmiş, sayaçlar azalmıyor ve son olayda `loaded == total`.
fn assert_progress(events: &[DiffEvent]) {
    for want in [SideTag::Source, SideTag::Target] {
        let v: Vec<(u32, u32)> = events
            .iter()
            .filter_map(|DiffEvent::Progress { side, loaded, total }| {
                (*side == want).then_some((*loaded, *total))
            })
            .collect();
        assert!(!v.is_empty(), "{want:?} için ilerleme olayı yok");
        let (loaded, total) = *v.last().unwrap();
        assert_eq!(loaded, total, "{want:?}: son olay tamamlanmış olmalı");
        assert!(
            v.windows(2).all(|w| w[0].0 <= w[1].0),
            "{want:?}: ilerleme geri gitmemeli"
        );
    }
}

type Desc = (String, Option<String>, String, Option<String>);

fn desc(kind: ObjectKind, table: Option<&str>, name: &str, sig: Option<&str>) -> Desc {
    (
        format!("{kind:?}"),
        table.map(str::to_string),
        name.to_string(),
        sig.map(str::to_string),
    )
}

/// Öğe tanımı -> (durum, sorun).
fn states(d: &DiffData) -> BTreeMap<Desc, (ItemStatus, Option<ItemProblem>)> {
    d.result()
        .items
        .iter()
        .map(|i| {
            (
                desc(i.kind, i.table.as_deref(), &i.name, i.signature.as_deref()),
                (i.status, i.problem),
            )
        })
        .collect()
}

/// `Same` olmayan öğeler tam olarak beklenenler; geri kalan hepsi `Same`.
fn assert_states(d: &DiffData, want: &[(Desc, ItemStatus, Option<ItemProblem>)]) {
    let got = states(d);
    let non_same: BTreeMap<_, _> = got
        .iter()
        .filter(|(_, (s, _))| *s != ItemStatus::Same)
        .map(|(k, v)| (k.clone(), *v))
        .collect();
    let expected: BTreeMap<_, _> = want
        .iter()
        .map(|(k, s, p)| (k.clone(), (*s, *p)))
        .collect();
    assert_eq!(non_same, expected, "Same dışındaki öğeler beklenenle eşleşmeli");
}

fn selectable_keys(d: &DiffData) -> Vec<String> {
    d.result()
        .items
        .iter()
        .filter(|i| i.selectable)
        .map(|i| i.key.clone())
        .collect()
}

/// Betiği hedefe uygular. SQL metni yalnız `PGT_DIFF_SHOW_SQL` tanımlıysa hata iletisine eklenir.
async fn apply(env: &Env, sql: &str) {
    if let Err(e) = env.admin.batch_execute(sql).await {
        panic!("betik uygulanamadı: {}{}", db_message(&e), show(sql));
    }
}

/// SQL metni yalnız `PGT_DIFF_SHOW_SQL` tanımlıysa (iletiye eklenecek biçimde) döner.
fn show(sql: &str) -> String {
    if std::env::var_os("PGT_DIFF_SHOW_SQL").is_some() {
        format!("\n{sql}")
    } else {
        String::new()
    }
}

fn pos(sql: &str, needle: &str) -> usize {
    sql.find(needle)
        .unwrap_or_else(|| panic!("betikte {needle:?} yok{}", show(sql)))
}

fn opts() -> DiffOptions {
    DiffOptions::default()
}

fn s(v: &str) -> Option<&str> {
    Some(v)
}

// --- gidiş-dönüş ---

/// Fikstürün beklenen farkları (kaynak -> hedef). Sürüme bağlı: enum değeri farkı yalnız PG 12+.
fn expected_states(ver: u32) -> Vec<(Desc, ItemStatus, Option<ItemProblem>)> {
    use ItemStatus::*;
    use ObjectKind::*;
    let mut v = vec![
        (desc(Domain, None, "posint", None), Different, None),
        (desc(Table, None, "orders", None), Different, None),
        (desc(Table, None, "products", None), Different, None),
        (desc(Table, None, "audit_log", None), OnlySource, None),
        (desc(Sequence, None, "audit_log_id_seq", None), OnlySource, None),
        (desc(Table, None, "legacy_stuff", None), OnlyTarget, None),
        (desc(Sequence, None, "legacy_stuff_id_seq", None), OnlyTarget, None),
        (
            desc(Index, s("orders"), "orders_total_partial_idx", None),
            OnlySource,
            None,
        ),
        (desc(Trigger, s("orders"), "orders_touch", None), OnlySource, None),
        (desc(Function, None, "add_one", s("integer")), Different, None),
        (desc(Function, None, "fmt", s("integer")), Different, None),
        (desc(View, None, "v2", None), Different, None),
        // Sorgusu değişen MV yeniden kurulur; indeksi (`mv_id_idx`, Same) MV öğesiyle yeniden oluşturulur.
        (desc(MaterializedView, None, "mv", None), Different, None),
        // Satır tipine bağlı rutinler: tablodan / görünümden sonra oluşturulur.
        (desc(Table, None, "rt_items", None), OnlySource, None),
        (desc(View, None, "rt_v", None), OnlySource, None),
        (desc(Function, None, "list_items", s("")), OnlySource, None),
        (desc(Function, None, "v_label", s("rt_v")), OnlySource, None),
    ];
    if ver >= 120_000 {
        v.push((desc(Type, None, "mood", None), Different, None));
    }
    v
}

/// Karşılaştır -> beklenen durumlar -> hepsini seç -> betik -> hedefe uygula -> yeniden karşılaştır (hepsi `Same`).
async fn roundtrip(mgr: &ConnectionManager, src: DiffSide, tgt: DiffSide, tgt_env: &Env, ver: u32) {
    let (d, events) = run_ok(mgr, &src, &tgt, opts()).await;
    assert_progress(&events);
    assert_states(&d, &expected_states(ver));
    assert!(
        d.result().items.iter().any(|i| i.status == ItemStatus::Same),
        "değişmeyen öğeler de listelenmeli"
    );
    assert!(d.result().items.iter().all(|i| i.problem.is_none()));
    // Varsayılan seçim: yalnız hedefte olanlar hariç.
    for i in &d.result().items {
        assert_eq!(
            i.default_selected,
            i.selectable && i.status != ItemStatus::OnlyTarget && i.status != ItemStatus::Same,
            "varsayılan seçim kuralı"
        );
    }
    assert_eq!(
        d.result().warnings.is_empty(),
        src.schema == tgt.schema,
        "şema adı uyarısı yalnız adlar farklıysa: {:?}",
        d.result().warnings
    );

    // Satır farkı: farklı görünümün iki yanı ve satır farkı dolu.
    let v2 = d.result().items.iter().find(|i| i.name == "v2").unwrap();
    let det = d.detail(&v2.key).unwrap();
    assert!(!det.source_text.is_empty() && !det.target_text.is_empty());
    assert!(det.lines.as_ref().is_some_and(|l| !l.is_empty()));

    let script = d
        .script(&selectable_keys(&d), STAMP)
        .expect("betik üretilmeli");
    assert!(script.transactional, "bu fikstürde betik transaction'lıdır");
    assert!(script.statements > 0);
    apply(tgt_env, &script.sql).await;

    let (again, _) = run_ok(mgr, &src, &tgt, opts()).await;
    let left: Vec<_> = again
        .result()
        .items
        .iter()
        .filter(|i| i.status != ItemStatus::Same)
        .map(|i| (i.kind, i.name.clone(), i.status, i.problem))
        .collect();
    assert!(left.is_empty(), "yeniden karşılaştırma temiz olmalı: {left:?}{}", show(&script.sql));
}

async fn roundtrip_same_db(ts: TestServer) {
    capture();
    let env = setup_in(&ts, PFX, "rta").await;
    let (a, b) = ("pgt_diff_src", "pgt_diff_tgt");
    env.sql(&rt_fixture(a, env.version, true)).await;
    env.sql(&rt_fixture(b, env.version, false)).await;
    roundtrip(&env.mgr, side(&env, a), side(&env, b), &env, env.version).await;
    // Havuz bağlantılarının search_path'i değişmedi.
    let sp: String = env
        .mgr
        .run(env.server, Target::Database(env.db_oid), |c| async move {
            c.client
                .query_one("SHOW search_path", &[])
                .await
                .map(|r| r.get::<_, String>(0))
        })
        .await
        .unwrap();
    assert_eq!(sp, "pg_catalog");
    assert_no_idle_in_tx_for(&env).await;
}

async fn roundtrip_two_dbs(ts: TestServer) {
    capture();
    let src_env = setup_in(&ts, PFX, "rtbs").await;
    let tgt_env = setup_in(&ts, PFX, "rtbt").await;
    // Tek yönetici yeter: iki veritabanı aynı sunucuda.
    let ver = src_env.version;
    src_env.sql(&rt_fixture("app", ver, true)).await;
    tgt_env.sql(&rt_fixture("app", ver, false)).await;
    roundtrip(
        &src_env.mgr,
        side(&src_env, "app"),
        side(&tgt_env, "app"),
        &tgt_env,
        ver,
    )
    .await;
    assert_no_idle_in_tx_for(&src_env).await;
    assert_no_idle_in_tx_for(&tgt_env).await;
}

#[tokio::test]
async fn roundtrip_same_database() {
    for ts in servers() {
        guarded(roundtrip_same_db(ts.clone()), cleanup_in(&ts, PFX, "rta")).await;
    }
}

#[tokio::test]
async fn roundtrip_two_databases_same_schema_name() {
    for ts in servers() {
        let t = ts.clone();
        let clean = async move {
            cleanup_in(&t, PFX, "rtbs").await;
            cleanup_in(&t, PFX, "rtbt").await;
        };
        guarded(roundtrip_two_dbs(ts.clone()), clean).await;
    }
}

// --- betik sıralaması (bağımlılık sırası), aynı şema reddi ---

async fn ordering(ts: TestServer) {
    capture();
    let env = setup_in(&ts, PFX, "ord").await;
    env.sql(
        "CREATE SCHEMA o_src;
         CREATE SCHEMA o_tgt;
         CREATE TYPE o_src.mood AS ENUM ('a','b');
         CREATE TABLE o_src.parent_t (id int PRIMARY KEY, m o_src.mood);
         CREATE TABLE o_src.child_t (id int PRIMARY KEY, p int REFERENCES o_src.parent_t (id));
         CREATE VIEW o_src.base_v AS SELECT id FROM o_src.parent_t;
         CREATE VIEW o_src.mid_v AS SELECT id FROM o_src.base_v;
         CREATE MATERIALIZED VIEW o_src.top_mv AS SELECT id FROM o_src.mid_v;
         CREATE INDEX child_idx ON o_src.child_t (p);",
    )
    .await;
    let (src, tgt) = (side(&env, "o_src"), side(&env, "o_tgt"));

    // Aynı şema reddi: iş başlamadan, olay yok.
    let (r, events) = run(&env.mgr, &src, &src, opts()).await;
    assert!(matches!(r, Err(RunError::SameSchema)), "{:?}", r.map(|_| ()));
    assert!(events.is_empty());

    let (d, _) = run_ok(&env.mgr, &src, &tgt, opts()).await;
    assert!(d.result().items.iter().all(|i| i.status == ItemStatus::OnlySource));
    // Anahtarlar bilerek ters sırada verilir: sıra seçime değil bağımlılığa göre olmalı.
    let mut keys = selectable_keys(&d);
    keys.reverse();
    assert_eq!(keys.len(), 7);
    let script = d.script(&keys, STAMP).unwrap();
    let sql = &script.sql;
    let at = |what: &str| pos(sql, what);
    assert!(at("CREATE TYPE") < at("CREATE TABLE"), "tip tablodan önce{}", show(sql));
    let (p, c) = (at("parent_t ("), at("child_t ("));
    let fk = at("FOREIGN KEY");
    assert!(p < fk && c < fk, "FK iki tablo oluştuktan sonra{}", show(sql));
    let (b, m, t) = (at("VIEW o_tgt.base_v"), at("VIEW o_tgt.mid_v"), at("VIEW o_tgt.top_mv"));
    assert!(b < m && m < t, "görünüm kendi tabanından sonra{}", show(sql));
    assert!(fk < b, "görünümler FK'lardan sonra{}", show(sql));
    assert!(at("INDEX child_idx") > t, "indeks görünümlerden sonra{}", show(sql));
    apply(&env, sql).await;
    let (again, _) = run_ok(&env.mgr, &src, &tgt, opts()).await;
    assert!(
        again.result().items.iter().all(|i| i.status == ItemStatus::Same),
        "yeniden karşılaştırma temiz olmalı{}",
        show(sql)
    );

    // Hedefte bulunan ama kaynakta olmayan görünüm zinciri: silmeler bağımlılığın tersine (üstteki önce).
    env.sql(
        "DROP MATERIALIZED VIEW o_src.top_mv; DROP VIEW o_src.mid_v; DROP VIEW o_src.base_v;",
    )
    .await;
    let (d, _) = run_ok(&env.mgr, &src, &tgt, opts()).await;
    let drops: Vec<String> = d
        .result()
        .items
        .iter()
        .filter(|i| i.status == ItemStatus::OnlyTarget)
        .map(|i| i.key.clone())
        .collect();
    assert_eq!(drops.len(), 3);
    let script = d.script(&drops, STAMP).unwrap();
    let sql = &script.sql;
    let (b, m, t) = (pos(sql, "base_v"), pos(sql, "mid_v"), pos(sql, "top_mv"));
    assert!(
        t < m && m < b,
        "silmede bağımlı önce{}",
        show(sql)
    );
    apply(&env, sql).await;
    assert_no_idle_in_tx_for(&env).await;
}

#[tokio::test]
async fn script_is_dependency_ordered() {
    for ts in servers() {
        guarded(ordering(ts.clone()), cleanup_in(&ts, PFX, "ord")).await;
    }
}

// --- sahiplik / yetki seçeneği ---

async fn ownership(ts: TestServer) {
    capture();
    let env = setup_in(&ts, PFX, "own").await;
    let (ra, rb) = (env.role.clone(), format!("{PFX}_rk_own"));
    env.sql(&format!("DROP ROLE IF EXISTS {rb}; CREATE ROLE {rb};"))
        .await;
    let body = |s: &str, owner: &str, other: &str, source: bool| {
        let grants = if source {
            format!(
                "GRANT SELECT ON {s}.t1 TO {other};
                 REVOKE EXECUTE ON FUNCTION {s}.f1() FROM PUBLIC;"
            )
        } else {
            format!("GRANT INSERT ON {s}.t1 TO {owner};")
        };
        format!(
            "CREATE SCHEMA {s};
             CREATE TABLE {s}.t1 (id int PRIMARY KEY);
             CREATE VIEW {s}.vw AS SELECT id FROM {s}.t1;
             CREATE FUNCTION {s}.f1() RETURNS int LANGUAGE sql AS 'SELECT 1';
             ALTER TABLE {s}.t1 OWNER TO {t1_owner};
             ALTER VIEW {s}.vw OWNER TO {t1_owner};
             {grants}",
            t1_owner = if source { owner } else { other },
        )
    };
    env.sql(&body("k_src", &ra, &rb, true)).await;
    env.sql(&body("k_tgt", &ra, &rb, false)).await;
    let (src, tgt) = (side(&env, "k_src"), side(&env, "k_tgt"));

    // Seçenek kapalıyken sahiplik ve yetki farkı görünmez.
    let (d, _) = run_ok(&env.mgr, &src, &tgt, opts()).await;
    assert!(
        d.result().items.iter().all(|i| i.status == ItemStatus::Same),
        "sahiplik kapalıyken fark yok"
    );

    let on = DiffOptions {
        ownership: true,
        ..opts()
    };
    let (d, _) = run_ok(&env.mgr, &src, &tgt, on).await;
    let diff: Vec<&str> = d
        .result()
        .items
        .iter()
        .filter(|i| i.status == ItemStatus::Different)
        .map(|i| i.name.as_str())
        .collect();
    for n in ["t1", "vw", "f1"] {
        assert!(diff.contains(&n), "{n} farklı olmalı: {diff:?}");
    }
    let script = d.script(&selectable_keys(&d), STAMP).unwrap();
    assert!(script.sql.contains("OWNER TO"), "{}", show(&script.sql));
    assert!(
        script.sql.contains("GRANT") || script.sql.contains("REVOKE"),
        "{}",
        show(&script.sql)
    );
    apply(&env, &script.sql).await;
    let (again, _) = run_ok(&env.mgr, &src, &tgt, on).await;
    let left: Vec<_> = again
        .result()
        .items
        .iter()
        .filter(|i| i.status != ItemStatus::Same)
        .map(|i| (i.name.clone(), i.status, i.problem))
        .collect();
    assert!(left.is_empty(), "sahiplik gidiş-dönüşü temiz olmalı: {left:?}{}", show(&script.sql));
    assert_no_idle_in_tx_for(&env).await;
}

#[tokio::test]
async fn ownership_and_grants_roundtrip() {
    for ts in servers() {
        let t = ts.clone();
        let clean = async move {
            cleanup_in(&t, PFX, "own").await;
            let admin = pgt_core::testing::admin_client(&t, &t.config.maintenance_db).await;
            admin
                .batch_execute(&format!("DROP ROLE IF EXISTS {PFX}_rk_own"))
                .await
                .unwrap();
        };
        guarded(ownership(ts.clone()), clean).await;
    }
}

// --- yeniden kurma: hedef sahibi korunur, kaybettirecek görünüm elle ---

async fn recreate_state(ts: TestServer) {
    capture();
    let env = setup_in(&ts, PFX, "rec").await;
    let rb = format!("{PFX}_rk_rec");
    env.sql(&format!("DROP ROLE IF EXISTS {rb}; CREATE ROLE {rb};"))
        .await;
    let body = |s: &str, source: bool| {
        format!(
            "CREATE SCHEMA {s};
             CREATE FUNCTION {s}.sd() RETURNS {ret} LANGUAGE sql SECURITY DEFINER AS $$SELECT {val}$$;
             CREATE TABLE {s}.vt_base (id int);
             CREATE VIEW {s}.vt AS SELECT id FROM {s}.vt_base{filter};
             CREATE FUNCTION {s}.vt_ins() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN RETURN NEW; END $$;
             CREATE TRIGGER vt_ins INSTEAD OF INSERT ON {s}.vt FOR EACH ROW EXECUTE PROCEDURE {s}.vt_ins();",
            ret = if source { "text" } else { "integer" },
            val = if source { "'1'::text" } else { "1" },
            filter = if source { " WHERE id > 0" } else { "" },
        )
    };
    env.sql(&body("r_src", true)).await;
    env.sql(&body("r_tgt", false)).await;
    // Hedefte SECURITY DEFINER fonksiyonun sahibi bağlı rol değil.
    env.sql(&format!("ALTER FUNCTION r_tgt.sd() OWNER TO {rb};"))
        .await;
    let (src, tgt) = (side(&env, "r_src"), side(&env, "r_tgt"));
    use ItemProblem::Manual;
    use ItemStatus::Different;
    let (d, _) = run_ok(&env.mgr, &src, &tgt, opts()).await;
    assert_states(
        &d,
        &[
            (desc(ObjectKind::Function, None, "sd", s("")), Different, None),
            (desc(ObjectKind::View, None, "vt", None), Different, Some(Manual)),
        ],
    );
    let vt = d.result().items.iter().find(|i| i.name == "vt").unwrap();
    assert!(!vt.selectable);
    assert!(
        vt.notes.iter().any(|n| n.contains("TRIGGER vt_ins")),
        "kaybolacak trigger notta olmalı: {:?}",
        vt.notes
    );
    let script = d.script(&selectable_keys(&d), STAMP).unwrap();
    let sql = &script.sql;
    assert!(
        !sql.contains("DROP VIEW") && !sql.contains("vt_base"),
        "elle öğe betikte olmamalı{}",
        show(sql)
    );
    assert!(sql.contains("DROP FUNCTION"), "dönüş tipi değişimi yeniden kurar{}", show(sql));
    apply(&env, sql).await;
    let row = env
        .admin
        .query_one(
            "SELECT pg_catalog.pg_get_userbyid(p.proowner)::pg_catalog.text, p.prosecdef
             FROM pg_catalog.pg_proc p JOIN pg_catalog.pg_namespace n ON n.oid = p.pronamespace
             WHERE n.nspname = 'r_tgt' AND p.proname = 'sd'",
            &[],
        )
        .await
        .unwrap();
    assert_eq!(
        row.get::<_, String>(0),
        rb,
        "sahiplik kapalıyken hedef sahibi korunmalı{}",
        show(sql)
    );
    assert!(row.get::<_, bool>(1), "SECURITY DEFINER korunmalı");
    let (again, _) = run_ok(&env.mgr, &src, &tgt, opts()).await;
    assert_states(
        &again,
        &[(desc(ObjectKind::View, None, "vt", None), Different, Some(Manual))],
    );
    assert_no_idle_in_tx_for(&env).await;
}

#[tokio::test]
async fn recreate_keeps_target_owner_and_lossy_view_is_manual() {
    for ts in servers() {
        let t = ts.clone();
        let clean = async move {
            cleanup_in(&t, PFX, "rec").await;
            let admin = pgt_core::testing::admin_client(&t, &t.config.maintenance_db).await;
            admin
                .batch_execute(&format!("DROP ROLE IF EXISTS {PFX}_rk_rec"))
                .await
                .unwrap();
        };
        guarded(recreate_state(ts.clone()), clean).await;
    }
}

// --- bölümleme ---

async fn partitions(ts: TestServer) {
    capture();
    let env = setup_in(&ts, PFX, "part").await;
    env.sql(
        "CREATE SCHEMA p_src;
         CREATE SCHEMA p_tgt;
         CREATE TABLE p_src.parent (id int, v int) PARTITION BY RANGE (id);
         CREATE TABLE p_src.parent_1 PARTITION OF p_src.parent FOR VALUES FROM (0) TO (10);
         CREATE TABLE p_src.plain (id int PRIMARY KEY);",
    )
    .await;
    let (src, tgt) = (side(&env, "p_src"), side(&env, "p_tgt"));
    let (d, _) = run_ok(&env.mgr, &src, &tgt, opts()).await;
    use ItemStatus::OnlySource;
    use ItemProblem::Manual;
    assert_states(
        &d,
        &[
            (desc(ObjectKind::Table, None, "parent", None), OnlySource, Some(Manual)),
            (desc(ObjectKind::Table, None, "parent_1", None), OnlySource, Some(Manual)),
            (desc(ObjectKind::Table, None, "plain", None), OnlySource, None),
        ],
    );
    let key = |n: &str| d.result().items.iter().find(|i| i.name == n).unwrap().clone();
    for n in ["parent", "parent_1"] {
        let it = key(n);
        assert!(!it.selectable && !it.default_selected, "{n} seçilemez");
        assert_eq!(
            d.script(std::slice::from_ref(&it.key), STAMP).unwrap_err(),
            ScriptError::NotSelectable
        );
    }
    let script = d.script(&selectable_keys(&d), STAMP).unwrap();
    assert!(!script.sql.contains("parent"), "elle gereken öğe betikte olmamalı");
    assert!(script.sql.contains("plain"));
    apply(&env, &script.sql).await;
    assert_no_idle_in_tx_for(&env).await;
}

#[tokio::test]
async fn partitioned_tables_are_manual() {
    for ts in servers() {
        guarded(partitions(ts.clone()), cleanup_in(&ts, PFX, "part")).await;
    }
}

// --- çapraz sürüm (kaynak 16 -> hedef 13) ---

async fn cross_version(src_ts: TestServer, tgt_ts: TestServer) {
    capture();
    let src_env = setup_in(&src_ts, PFX, "xv").await;
    let tgt_env = setup_in(&tgt_ts, PFX, "xv").await;
    // Tek yönetici iki sunucuya da bağlı olmalı.
    src_env
        .mgr
        .connect(tgt_ts.config.clone(), tgt_ts.password.clone())
        .await
        .unwrap();
    src_env
        .sql(
            "CREATE SCHEMA xs;
             CREATE TABLE xs.u (id int, code text, CONSTRAINT u_code_key UNIQUE NULLS NOT DISTINCT (code));
             CREATE TABLE xs.fine (id int PRIMARY KEY);",
        )
        .await;
    tgt_env.sql("CREATE SCHEMA xs;").await;
    let src = side(&src_env, "xs");
    let tgt = side(&tgt_env, "xs");
    let (d, _) = run_ok(&src_env.mgr, &src, &tgt, opts()).await;
    assert_states(
        &d,
        &[
            (
                desc(ObjectKind::Table, None, "u", None),
                ItemStatus::OnlySource,
                Some(ItemProblem::BuildFailed),
            ),
            (
                desc(ObjectKind::Table, None, "fine", None),
                ItemStatus::OnlySource,
                None,
            ),
        ],
    );
    let u = d.result().items.iter().find(|i| i.name == "u").unwrap();
    assert!(!u.selectable);
    assert!(
        u.errors.iter().any(|e| e.contains("PG 15")),
        "sürüm iletisi bekleniyordu: {:?}",
        u.errors
    );
    assert!(
        d.result().warnings.iter().any(|w| w.contains("13")),
        "sürüm uyarısı: {:?}",
        d.result().warnings
    );
    // Seçilebilen öğe hedefe (PG 13) uygulanır.
    let script = d.script(&selectable_keys(&d), STAMP).unwrap();
    apply(&tgt_env, &script.sql).await;
}

#[tokio::test]
async fn cross_version_pg16_to_pg13_reports_build_failure() {
    let all = servers();
    let mut majors = vec![];
    for ts in &all {
        majors.push((ts.clone(), common::version_num(ts).await / 10_000));
    }
    let src = majors.iter().find(|(_, m)| *m == 16).map(|(t, _)| t.clone());
    let tgt = majors.iter().find(|(_, m)| *m == 13).map(|(t, _)| t.clone());
    let (Some(src), Some(tgt)) = (src, tgt) else {
        eprintln!("PG 16 ve PG 13 birlikte yok, çapraz sürüm testi atlanıyor");
        return;
    };
    let (a, b) = (src.clone(), tgt.clone());
    let clean = async move {
        cleanup_in(&a, PFX, "xv").await;
        cleanup_in(&b, PFX, "xv").await;
    };
    guarded(cross_version(src, tgt), clean).await;
}

// --- uzantı nesneleri ---

async fn extension_members(ts: TestServer) {
    capture();
    let env = setup_in(&ts, PFX, "ext").await;
    if !env.has_extension("citext").await {
        eprintln!("citext eklentisi yok, uzantı testi atlanıyor");
        return;
    }
    env.sql(
        "CREATE SCHEMA es;
         CREATE SCHEMA et;
         CREATE EXTENSION citext SCHEMA es;
         CREATE TABLE es.holder (c es.citext);",
    )
    .await;
    let (d, _) = run_ok(&env.mgr, &side(&env, "es"), &side(&env, "et"), opts()).await;
    let names: Vec<(ObjectKind, &str)> = d
        .result()
        .items
        .iter()
        .map(|i| (i.kind, i.name.as_str()))
        .collect();
    assert_eq!(names, vec![(ObjectKind::Table, "holder")], "uzantı üyeleri listelenmemeli");
}

#[tokio::test]
async fn extension_objects_are_not_listed() {
    for ts in servers() {
        guarded(extension_members(ts.clone()), cleanup_in(&ts, PFX, "ext")).await;
    }
}

// --- iptal, hata kodları, eşzamanlılık ---

async fn cancel_and_errors(ts: TestServer) {
    capture();
    let env = setup_in(&ts, PFX, "cx").await;
    env.sql(
        "CREATE SCHEMA big;
         CREATE SCHEMA small;
         CREATE TABLE small.one (id int);
         DO $$ BEGIN
           FOR i IN 1..400 LOOP
             EXECUTE format('CREATE TABLE big.t%s (id int PRIMARY KEY, a text)', i);
           END LOOP;
         END $$;",
    )
    .await;
    let (big, small) = (side(&env, "big"), side(&env, "small"));

    // Yükleme sürerken iptal (ilk ilerleme olayında): Cancelled, oturum kalmaz.
    let token = CancelToken::new();
    let mut sink = Events {
        cancel_on_first: Some(token.clone()),
        ..Events::default()
    };
    let r = Box::pin(run_diff(&env.mgr, &big, &small, opts(), &mut sink, &token)).await;
    assert!(matches!(r, Err(RunError::Cancelled)), "{:?}", r.map(|_| ()));
    assert!(!sink.all.is_empty(), "iptal iş başladıktan sonra gelmeli");
    assert_no_idle_in_tx_for(&env).await;

    // Alıcı gittiğinde (send false) de iptal.
    struct Gone;
    impl DiffSink for Gone {
        fn send(&mut self, _: DiffEvent) -> bool {
            false
        }
    }
    let r = Box::pin(run_diff(&env.mgr, &big, &small, opts(), &mut Gone, &CancelToken::new())).await;
    assert!(matches!(r, Err(RunError::Cancelled)), "{:?}", r.map(|_| ()));
    assert_no_idle_in_tx_for(&env).await;

    // Aynı şema reddi; eksik şema hedef tarafı gösterir.
    let (r, events) = run(&env.mgr, &big, &big, opts()).await;
    assert!(matches!(r, Err(RunError::SameSchema)));
    assert!(events.is_empty());
    let nope = side(&env, "nope");
    let (r, _) = run(&env.mgr, &big, &nope, opts()).await;
    assert!(
        matches!(
            r,
            Err(RunError::Schema {
                side: SideTag::Target,
                check: pgt_diff::SchemaCheck::Missing
            })
        ),
        "{:?}",
        r.map(|_| ())
    );
    assert_no_idle_in_tx_for(&env).await;

    // İş sürerken havuz sorgusu başarılı olur: sorgu, işin ilk ilerleme olayından sonra ve iş bitmeden başlar.
    struct FirstEvent(Option<tokio::sync::oneshot::Sender<()>>);
    impl DiffSink for FirstEvent {
        fn send(&mut self, _: DiffEvent) -> bool {
            if let Some(tx) = self.0.take() {
                let _ = tx.send(());
            }
            true
        }
    }
    let (started_tx, started_rx) = tokio::sync::oneshot::channel();
    let (mgr, b, sm) = (env.mgr.clone(), big.clone(), small.clone());
    let job = tokio::spawn(async move {
        let mut sink = FirstEvent(Some(started_tx));
        Box::pin(run_diff(&mgr, &b, &sm, opts(), &mut sink, &CancelToken::new()))
            .await
            .map(|d| d.result().items.len())
    });
    started_rx.await.expect("iş ilerleme olayı göndermeli");
    assert!(!job.is_finished(), "havuz sorgusu iş sürerken çalışmalı");
    let one: i32 = env
        .mgr
        .run(env.server, Target::Database(env.db_oid), |c| async move {
            c.client.query_one("SELECT 1", &[]).await.map(|r| r.get(0))
        })
        .await
        .unwrap();
    assert_eq!(one, 1);
    let n = job.await.unwrap().expect("karşılaştırma tamamlanmalı");
    assert_eq!(n, 401, "400 + 1 tablo; PK indeksleri listelenmez");
    assert_no_idle_in_tx_for(&env).await;
}

#[tokio::test]
async fn cancel_during_load_leaves_no_session() {
    for ts in servers() {
        guarded(cancel_and_errors(ts.clone()), cleanup_in(&ts, PFX, "cx")).await;
    }
}

// --- günlük gizliliği ---

#[tokio::test]
async fn logs_carry_no_names_or_sql() {
    let buf = capture();
    for ts in servers() {
        guarded(roundtrip_same_db_for_log(ts.clone()), cleanup_in(&ts, PFX, "log")).await;
        let all = String::from_utf8(buf.0.lock().unwrap().clone()).unwrap();
        // Yalnız bu crate'in olayları (`pgt_diff::…` hedefi); bağımlılıkların TRACE satırları kapsam dışı.
        let out: String = all
            .lines()
            .filter(|l| l.contains("pgt_diff::"))
            .map(|l| format!("{l}\n"))
            .collect();
        assert!(out.contains("schema_diff"), "günlük yakalanmadı: {all}");
        let db = db_name_in(&ts, PFX, "log").await;
        for secret in [
            db.as_str(),
            "pgt_diff_src",
            "pgt_diff_tgt",
            "customers",
            "audit_log",
            "legacy_stuff",
            "products",
            "customer_id",
            "CREATE",
            "ALTER",
            "SELECT",
        ] {
            assert!(!out.contains(secret), "günlükte {secret:?} var: {out}");
        }
        if let Some(p) = ts.password.as_deref().filter(|p| !p.is_empty()) {
            assert!(!out.contains(p), "günlükte parola var");
        }
    }
}

async fn roundtrip_same_db_for_log(ts: TestServer) {
    let env = setup_in(&ts, PFX, "log").await;
    let (a, b) = ("pgt_diff_src", "pgt_diff_tgt");
    env.sql(&rt_fixture(a, env.version, true)).await;
    env.sql(&rt_fixture(b, env.version, false)).await;
    roundtrip(&env.mgr, side(&env, a), side(&env, b), &env, env.version).await;
}

// --- performans (elle; spec §4: süre yazdırılır, ölçülmez) ---

async fn perf(ts: TestServer) {
    let env = setup_in(&ts, PFX, "perf").await;
    for s in ["perf_src", "perf_tgt"] {
        env.sql(&format!(
            "CREATE SCHEMA {s};
             DO $$ BEGIN
               FOR i IN 1..500 LOOP
                 EXECUTE format('CREATE TABLE {s}.t%s (id int PRIMARY KEY, a text, b int)', i);
                 EXECUTE format('CREATE INDEX t%s_a_idx ON {s}.t%s (a)', i, i);
                 EXECUTE format('CREATE VIEW {s}.v%s AS SELECT id FROM {s}.t%s', i, i);
                 EXECUTE format('CREATE FUNCTION {s}.f%s(int) RETURNS int LANGUAGE sql AS $f$ SELECT 1 $f$', i);
               END LOOP;
             END $$;"
        ))
        .await;
    }
    let t = Instant::now();
    let (d, _) = run_ok(&env.mgr, &side(&env, "perf_src"), &side(&env, "perf_tgt"), opts()).await;
    println!(
        "diff_perf_500_tables: {} nesne, {} ms (hedef < 10000 ms; ölçülmez)",
        d.result().items.len(),
        t.elapsed().as_millis()
    );
}

#[tokio::test]
#[ignore = "elle: cargo test -p pgt-diff --test diff_roundtrip -- --ignored --nocapture"]
async fn diff_perf_500_tables() {
    for ts in servers() {
        guarded(perf(ts.clone()), cleanup_in(&ts, PFX, "perf")).await;
    }
}
