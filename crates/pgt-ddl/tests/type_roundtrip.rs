//! Trigger / Type / Domain üreticisi → gerçek sunucu → yükleyici gidiş-dönüş testleri (Docker; `PGT_TEST_URLS` yoksa
//! atlanır).
//!
//! Her test kendi geçici veritabanını (`pgt_ddl_ty_<ana sürüm>_<etiket>`) ve rolünü kurar; başarısız doğrulamadan sonra
//! bile temizlenir. Üreticinin SQL'i uygulayıcı gibi tek transaction'da çalıştırılır (`Env::exec_tx`); PG < 12 enum
//! `ADD VALUE` (transaction dışı) ifade ifade (`Env::exec`) ya da `DdlManager` ile. Sürüme bağlı kurallar
//! `server_version`'a göre dallanır; atlanan kısım yazdırılır.
//!
//! **Sabit nokta (her tür):** create → yükle → beklenen tanıma eşit; `alter(yüklü, yüklü)` boş plan; iki yükleme aynı;
//! yüklü tanımla (başka adla) yeniden oluşturulan nesnenin yüklü tanımı ad dışında aynı.

mod common;

use std::sync::{Arc, RwLock};

use common::{Env, cleanup_in, guarded, servers, setup_in};
use pgt_core::Error;
use pgt_core::registry::Registry;
use pgt_core::testing::TestServer;
use pgt_ddl::build;
use pgt_ddl::*;
use uuid::Uuid;

const PFX: &str = "pgt_ddl_ty";

macro_rules! docker_test {
    ($name:ident, $body:ident, $tag:literal) => {
        #[tokio::test]
        async fn $name() {
            for ts in servers() {
                guarded($body(ts.clone()), cleanup_in(&ts, PFX, $tag)).await;
            }
        }
    };
}

fn sqls(p: &Plan) -> Vec<&str> {
    p.statements.iter().map(|s| s.sql.as_str()).collect()
}

fn paths(e: &[FieldError]) -> Vec<&str> {
    e.iter().map(|e| e.path.as_str()).collect()
}

/// Sunucu iletisindeki SQLSTATE (`db_message` biçimi: "ileti (KOD)").
fn sqlstate(msg: &str) -> &str {
    msg.split_once('\n').map_or(msg, |(first, _)| first).rsplit_once('(').map_or("", |(_, c)| c.trim_end_matches(')'))
}

async fn try_load(env: &Env, kind: ObjectKind, oid: u32) -> pgt_core::Result<ObjectDef> {
    load(&env.mgr, env.server, env.db_oid, ObjectRef { kind, oid }).await
}

fn assert_noop(env: &Env, d: &ObjectDef) {
    let p = build::alter(d, d, &env.ctx(false)).unwrap_or_else(|e| panic!("noop reddedildi: {e:?}\n{d:?}"));
    assert!(p.statements.is_empty() && p.warnings.is_empty(), "boş plan bekleniyordu: {:?} {:?}\n{d:?}", sqls(&p), p.warnings);
}

fn balter(env: &Env, old: &ObjectDef, new: &ObjectDef) -> Plan {
    let p = build::alter(old, new, &env.ctx(false)).unwrap_or_else(|e| panic!("alter reddedildi: {e:?}\n  old: {old:?}\n  new: {new:?}"));
    for s in &p.statements {
        assert!(!s.sql.trim_end().ends_with(';'), "{}", s.sql);
    }
    p
}

/// Salt okunur tanım: değişiklik ve drop `object` hatası; `alter(l, l)` boş plan.
fn assert_read_only(env: &Env, l: &ObjectDef, changed: &ObjectDef) {
    let e = build::alter(l, changed, &env.ctx(false)).unwrap_err();
    assert_eq!(paths(&e), ["object"], "{l:?}");
    assert_noop(env, l);
    let e = build::drop(l, false, &env.ctx(false)).unwrap_err();
    assert_eq!(paths(&e), ["object"], "{l:?}");
}

// ================================================================================================================
// Trigger
// ================================================================================================================

fn ev() -> TriggerEvents {
    TriggerEvents { insert: false, update: false, update_columns: vec![], delete: false, truncate: false }
}

fn ins() -> TriggerEvents {
    TriggerEvents { insert: true, ..ev() }
}

fn upd(cols: &[&str]) -> TriggerEvents {
    TriggerEvents { update: true, update_columns: cols.iter().map(|c| (*c).to_string()).collect(), ..ev() }
}

fn trg(schema: &str, table: &str, name: &str, fschema: &str, fname: &str) -> TriggerDef {
    TriggerDef {
        schema: schema.into(),
        table: table.into(),
        name: name.into(),
        timing: TriggerTiming::Before,
        events: ins(),
        level: TriggerLevel::Row,
        when: None,
        function_schema: fschema.into(),
        function_name: fname.into(),
        args: vec![],
        old_table: None,
        new_table: None,
        constraint: false,
        deferrable: false,
        initially_deferred: false,
        enabled: TriggerEnabled::Origin,
        comment: None,
        editable: true,
        unsupported: vec![],
    }
}

async fn trigger_oid(env: &Env, schema: &str, table: &str, name: &str) -> u32 {
    let sql = "SELECT t.oid FROM pg_trigger t JOIN pg_class c ON c.oid = t.tgrelid JOIN pg_namespace n ON n.oid = c.relnamespace \
               WHERE n.nspname = $1::text AND c.relname = $2::text AND t.tgname = $3::text";
    env.admin
        .query_one(sql, &[&schema, &table, &name])
        .await
        .unwrap_or_else(|e| panic!("{schema}.{table}.{name} trigger bulunamadı: {e}"))
        .get(0)
}

async fn tload_oid(env: &Env, oid: u32) -> TriggerDef {
    match try_load(env, ObjectKind::Trigger, oid).await {
        Ok(ObjectDef::Trigger(d)) => d,
        other => panic!("trigger {oid} yüklenemedi: {other:?}"),
    }
}

async fn tload(env: &Env, d: &TriggerDef) -> (u32, TriggerDef) {
    let oid = trigger_oid(env, &d.schema, &d.table, &d.name).await;
    (oid, tload_oid(env, oid).await)
}

fn texpected(d: &TriggerDef) -> TriggerDef {
    TriggerDef { comment: d.comment.clone().filter(|c| !c.is_empty()), editable: true, unsupported: vec![], ..d.clone() }
}

async fn tcreate_and_check(env: &Env, def: &TriggerDef) -> (u32, TriggerDef) {
    let plan = build::create(&ObjectDef::Trigger(def.clone()), &env.ctx(false)).unwrap_or_else(|e| panic!("create reddedildi: {} {e:?}", def.name));
    if let Err(e) = env.try_exec_tx(&plan).await {
        panic!("create uygulanamadı: {e}");
    }
    let (oid, loaded) = tload(env, def).await;
    assert_eq!(loaded, texpected(def), "create → load ({})", def.name);
    let o = ObjectDef::Trigger(loaded.clone());
    assert_noop(env, &o);
    assert_eq!(tload_oid(env, oid).await, loaded, "iki yükleme aynı olmalı");
    // Sabit nokta: yüklü tanımla `<ad>_fp`.
    let fp = TriggerDef { name: format!("{}_fp", loaded.name), ..loaded.clone() };
    let plan = build::create(&ObjectDef::Trigger(fp.clone()), &env.ctx(false)).unwrap_or_else(|e| panic!("yüklü tanım create'te reddedildi: {e:?}"));
    env.exec_tx(&plan).await;
    let (_, l2) = tload(env, &fp).await;
    assert_eq!(l2, fp, "sabit nokta");
    env.exec_tx(&build::drop(&ObjectDef::Trigger(l2), false, &env.ctx(false)).unwrap()).await;
    (oid, loaded)
}

async fn talter_and_check(env: &Env, old: &TriggerDef, new: &TriggerDef) -> (u32, TriggerDef, Plan) {
    let plan = balter(env, &ObjectDef::Trigger(old.clone()), &ObjectDef::Trigger(new.clone()));
    if let Err(e) = env.try_exec_tx(&plan).await {
        panic!("alter uygulanamadı: {e}\n  plan: {:?}", sqls(&plan));
    }
    let (oid, loaded) = tload(env, new).await;
    assert_eq!(loaded, texpected(new), "alter → load; plan: {:?}", sqls(&plan));
    assert_noop(env, &ObjectDef::Trigger(loaded.clone()));
    (oid, loaded, plan)
}

const TRIGGER_SETUP: &str = r#"
CREATE SCHEMA "Tr Ş";
CREATE TABLE "Tr Ş".t (a int, b text, "Kolon C" int);
CREATE FUNCTION "Tr Ş".tf() RETURNS trigger LANGUAGE plpgsql AS 'BEGIN RETURN NEW; END';
"#;

docker_test!(trigger_create_then_load_matches, trigger_create_body, "tcreate");

async fn trigger_create_body(ts: TestServer) {
    let env = setup_in(&ts, PFX, "tcreate").await;
    let major = env.version / 10000;
    env.sql(TRIGGER_SETUP).await;
    let s = "Tr Ş";
    let t = |name: &str| trg(s, "t", name, s, "tf");
    let defs = vec![
        TriggerDef { events: upd(&["b", "a"]), when: Some("new.a > 0 AND old.b IS DISTINCT FROM new.b".into()), ..t("tr_upd") },
        TriggerDef { args: vec!["a".into(), "it's".into(), "c:\\yol".into(), "ş 日本".into(), String::new()], ..t("tr_args") },
        TriggerDef { timing: TriggerTiming::After, level: TriggerLevel::Statement, events: TriggerEvents { truncate: true, ..ev() }, ..t("tr_trunc") },
        TriggerDef {
            level: TriggerLevel::Statement,
            events: TriggerEvents { insert: true, update: true, update_columns: vec![], delete: true, truncate: true },
            ..t("tr_all")
        },
        TriggerDef {
            timing: TriggerTiming::After,
            level: TriggerLevel::Statement,
            events: upd(&[]),
            old_table: Some("eski".into()),
            new_table: Some("Yeni".into()),
            ..t("tr_ref_stmt")
        },
        TriggerDef { timing: TriggerTiming::After, new_table: Some("n".into()), ..t("tr_ref_row") },
        TriggerDef { timing: TriggerTiming::After, constraint: true, ..t("tr_c1") },
        TriggerDef { timing: TriggerTiming::After, constraint: true, deferrable: true, ..t("tr_c2") },
        TriggerDef {
            timing: TriggerTiming::After,
            constraint: true,
            deferrable: true,
            initially_deferred: true,
            events: TriggerEvents { delete: true, ..upd(&["a"]) },
            ..t("tr_c3")
        },
        TriggerDef { enabled: TriggerEnabled::Disabled, comment: Some("it's ; -- /* ğ".into()), ..t("tr_dis") },
        TriggerDef { enabled: TriggerEnabled::Replica, ..t("tr_rep") },
        TriggerDef { enabled: TriggerEnabled::Always, ..t("tr_alw") },
        TriggerDef { events: upd(&[]), ..trg(s, "t", "tr_sup", "pg_catalog", "suppress_redundant_updates_trigger") },
        TriggerDef { when: Some("new.\"Kolon C\" IS NOT NULL AND new.b <> 'x) WHEN ('::text".into()), ..t("Ad \"x\" WHEN (") },
    ];
    for d in &defs {
        tcreate_and_check(&env, d).await;
    }
    // Bölümlenmiş tabloda satır trigger'ı (PG 11+).
    if env.version >= 110000 {
        env.sql(r#"CREATE TABLE "Tr Ş".pt (a int) PARTITION BY RANGE (a); CREATE TABLE "Tr Ş".pt1 PARTITION OF "Tr Ş".pt FOR VALUES FROM (0) TO (10)"#).await;
        tcreate_and_check(&env, &TriggerDef { timing: TriggerTiming::After, ..trg(s, "pt", "tr_part", s, "tf") }).await;
    } else {
        eprintln!("PG {major}: bölümlenmiş tablo satır trigger'ı atlandı (PG 11+)");
    }
}

docker_test!(trigger_alter_rebuild_keeps_state, trigger_alter_body, "talter");

async fn trigger_alter_body(ts: TestServer) {
    let env = setup_in(&ts, PFX, "talter").await;
    let major = env.version / 10000;
    env.sql(TRIGGER_SETUP).await;
    let s = "Tr Ş";
    let d = TriggerDef { events: upd(&[]), enabled: TriggerEnabled::Disabled, comment: Some("c".into()), ..trg(s, "t", "tr", s, "tf") };
    let (oid, old) = tcreate_and_check(&env, &d).await;

    // VERIFY (ham PG davranışı, PG 14+): OR REPLACE etkinliği ORIGIN'e döndürür, yorumu korur.
    if env.version >= 140000 {
        env.sql(r#"CREATE TABLE "Tr Ş".v (a int); CREATE TRIGGER vr BEFORE INSERT ON "Tr Ş".v FOR EACH ROW EXECUTE FUNCTION "Tr Ş".tf();
                   ALTER TABLE "Tr Ş".v DISABLE TRIGGER vr; COMMENT ON TRIGGER vr ON "Tr Ş".v IS 'k';
                   CREATE OR REPLACE TRIGGER vr AFTER INSERT ON "Tr Ş".v FOR EACH ROW EXECUTE FUNCTION "Tr Ş".tf()"#)
            .await;
        let r = env
            .admin
            .query_one("SELECT tgenabled::text, obj_description(oid, 'pg_trigger') FROM pg_trigger WHERE tgname = 'vr'", &[])
            .await
            .unwrap();
        let (enabled, comment): (String, Option<String>) = (r.get(0), r.get(1));
        assert_eq!(enabled, "O", "OR REPLACE etkinliği ORIGIN'e döndürür");
        assert_eq!(comment.as_deref(), Some("k"), "OR REPLACE yorumu korur");
        eprintln!("PG {major}: OR REPLACE TRIGGER → tgenabled ORIGIN, yorum korunur");
    }

    // Ad + yapı değişimi; etkinlik (Disabled) ve yorum korunmalı.
    let new = TriggerDef {
        name: "tr2".into(),
        timing: TriggerTiming::After,
        when: Some("old.a IS DISTINCT FROM new.a".into()),
        args: vec!["x".into()],
        ..old.clone()
    };
    let (oid2, l, plan) = talter_and_check(&env, &old, &new).await;
    let got = sqls(&plan);
    assert!(got[0].starts_with("ALTER TRIGGER tr ON"), "{got:?}");
    if env.version >= 140000 {
        assert_eq!(got.len(), 3, "{got:?}");
        assert!(got[1].starts_with("CREATE OR REPLACE TRIGGER tr2"), "{got:?}");
        assert!(got[2].ends_with("DISABLE TRIGGER tr2"), "{got:?}");
        assert_eq!(oid2, oid, "OR REPLACE aynı satır");
    } else {
        assert_eq!(got.len(), 5, "{got:?}");
        assert!(got[1].starts_with("DROP TRIGGER tr2"), "{got:?}");
        assert!(got[2].starts_with("CREATE TRIGGER tr2"), "{got:?}");
        assert!(got[3].ends_with("DISABLE TRIGGER tr2"), "{got:?}");
        assert!(got[4].starts_with("COMMENT ON TRIGGER tr2"), "{got:?}");
        eprintln!("PG {major}: DROP + CREATE yeniden kurma; etkinlik ve yorum yeniden yazıldı");
    }
    assert_eq!((l.enabled, l.comment.as_deref()), (TriggerEnabled::Disabled, Some("c")));

    // Yalnız etkinlik.
    let (_, l, plan) = talter_and_check(&env, &l, &TriggerDef { enabled: TriggerEnabled::Always, ..l.clone() }).await;
    assert_eq!(plan.statements.len(), 1, "{:?}", sqls(&plan));
    assert!(plan.statements[0].sql.ends_with("ENABLE ALWAYS TRIGGER tr2"));
    // Yapı değişimi + Replica (ORIGIN dışı hedef yeniden yazılır) + yorum kaldırma.
    let new = TriggerDef { enabled: TriggerEnabled::Replica, comment: None, events: TriggerEvents { insert: true, ..upd(&["a"]) }, when: None, ..l.clone() };
    let (_, l, _) = talter_and_check(&env, &l, &new).await;
    assert_eq!((l.enabled, l.comment.as_deref()), (TriggerEnabled::Replica, None));
    // Yalnız yorum.
    let (_, _, plan) = talter_and_check(&env, &l, &TriggerDef { comment: Some("y".into()), ..l.clone() }).await;
    assert_eq!(plan.statements.len(), 1);

    // Constraint trigger: her sürümde DROP + CREATE CONSTRAINT (OR REPLACE kullanılamaz).
    let c = TriggerDef { timing: TriggerTiming::After, constraint: true, comment: Some("cc".into()), ..trg(s, "t", "ct", s, "tf") };
    let (_, cl) = tcreate_and_check(&env, &c).await;
    let new = TriggerDef { deferrable: true, initially_deferred: true, enabled: TriggerEnabled::Disabled, ..cl.clone() };
    let (_, cl2, plan) = talter_and_check(&env, &cl, &new).await;
    let got = sqls(&plan);
    assert!(got[0].starts_with("DROP TRIGGER ct") && got[1].starts_with("CREATE CONSTRAINT TRIGGER ct"), "{got:?}");
    assert_eq!(cl2.comment.as_deref(), Some("cc"));
}

docker_test!(trigger_internal_and_clones_are_read_only, trigger_readonly_body, "treadonly");

fn assert_trigger_read_only(env: &Env, l: &TriggerDef, want: &str) {
    assert!(!l.editable, "{}: {:?}", l.name, l.unsupported);
    assert!(l.unsupported.iter().any(|u| u.contains(want)), "{}: '{want}' yok: {:?}", l.name, l.unsupported);
    let o = ObjectDef::Trigger(l.clone());
    assert_read_only(env, &o, &ObjectDef::Trigger(TriggerDef { comment: Some("x".into()), ..l.clone() }));
}

async fn trigger_readonly_body(ts: TestServer) {
    let env = setup_in(&ts, PFX, "treadonly").await;
    let major = env.version / 10000;
    env.sql(
        "CREATE SCHEMA ro; CREATE TABLE ro.p (id int PRIMARY KEY); CREATE TABLE ro.c (id int REFERENCES ro.p);
         CREATE FUNCTION ro.tf() RETURNS trigger LANGUAGE plpgsql AS 'BEGIN RETURN NEW; END';
         CREATE CONSTRAINT TRIGGER cf AFTER INSERT ON ro.c FROM ro.p FOR EACH ROW EXECUTE PROCEDURE ro.tf();
         CREATE VIEW ro.v AS SELECT 1 AS x;
         CREATE TRIGGER vt INSTEAD OF INSERT ON ro.v FOR EACH ROW EXECUTE PROCEDURE ro.tf()",
    )
    .await;
    // FK iç trigger'ları (her iki tabloda).
    let rows = env
        .admin
        .query("SELECT oid FROM pg_trigger WHERE tgisinternal AND tgrelid IN ('ro.p'::regclass, 'ro.c'::regclass) ORDER BY oid", &[])
        .await
        .unwrap();
    assert_eq!(rows.len(), 4);
    for r in rows {
        let l = tload_oid(&env, r.get(0)).await;
        assert!(l.constraint);
        assert_trigger_read_only(&env, &l, "iç trigger");
    }
    // FROM başvurulan tablolu constraint trigger.
    let l = tload_oid(&env, trigger_oid(&env, "ro", "c", "cf").await).await;
    assert!(l.constraint);
    assert_trigger_read_only(&env, &l, "FROM ro.p");
    // View INSTEAD OF trigger'ı.
    let l = tload_oid(&env, trigger_oid(&env, "ro", "v", "vt").await).await;
    assert_eq!(l.timing, TriggerTiming::InsteadOf);
    assert_trigger_read_only(&env, &l, "görünüm");
    // Bölüm klonu (PG 11–12 iç, PG 13+ tgparentid).
    if env.version >= 110000 {
        env.sql(
            "CREATE TABLE ro.pt (a int) PARTITION BY RANGE (a); CREATE TABLE ro.pt1 PARTITION OF ro.pt FOR VALUES FROM (0) TO (10);
             CREATE TRIGGER rt AFTER INSERT ON ro.pt FOR EACH ROW EXECUTE PROCEDURE ro.tf()",
        )
        .await;
        let parent = tload_oid(&env, trigger_oid(&env, "ro", "pt", "rt").await).await;
        assert!(parent.editable && parent.unsupported.is_empty(), "{:?}", parent.unsupported);
        let clone = tload_oid(&env, trigger_oid(&env, "ro", "pt1", "rt").await).await;
        let want = if env.version >= 130000 { "klon" } else { "iç trigger" };
        assert_trigger_read_only(&env, &clone, want);
        eprintln!("PG {major}: bölüm klonu salt okunur ({:?})", clone.unsupported);
    }
    // Bulunamayan nesne; trigger olmayan oid.
    assert!(matches!(try_load(&env, ObjectKind::Trigger, 4_000_000_000).await, Err(Error::NotFound(_))));
}

// ================================================================================================================
// Type
// ================================================================================================================

fn tdef(schema: &str, name: &str, body: TypeBody) -> TypeDef {
    TypeDef { schema: schema.into(), name: name.into(), owner: None, comment: None, body, editable: true, unsupported: vec![] }
}

fn ev_vals(labels: &[&str]) -> TypeBody {
    TypeBody::Enum { values: labels.iter().map(|l| EnumValue { label: (*l).into(), original: None }).collect() }
}

fn attr(name: &str, t: &str, coll: Option<&str>) -> CompositeAttr {
    CompositeAttr { name: name.into(), type_name: t.into(), collation: coll.map(Into::into), original: None }
}

fn range(subtype: &str) -> TypeBody {
    rng(subtype, None, None, None, None)
}

fn rng(subtype: &str, opclass: Option<&str>, collation: Option<&str>, diff: Option<&str>, multirange: Option<&str>) -> TypeBody {
    TypeBody::Range {
        subtype: subtype.into(),
        subtype_opclass: opclass.map(Into::into),
        collation: collation.map(Into::into),
        canonical: None,
        subtype_diff: diff.map(Into::into),
        multirange_type_name: multirange.map(Into::into),
    }
}

async fn type_oid(env: &Env, schema: &str, name: &str) -> u32 {
    let sql = "SELECT t.oid FROM pg_type t JOIN pg_namespace n ON n.oid = t.typnamespace WHERE n.nspname = $1::text AND t.typname = $2::text";
    env.admin.query_one(sql, &[&schema, &name]).await.unwrap_or_else(|e| panic!("{schema}.{name} tipi bulunamadı: {e}")).get(0)
}

async fn yload_oid(env: &Env, oid: u32) -> TypeDef {
    match try_load(env, ObjectKind::Type, oid).await {
        Ok(ObjectDef::Type(d)) => d,
        other => panic!("type {oid} yüklenemedi: {other:?}"),
    }
}

async fn yload(env: &Env, schema: &str, name: &str) -> (u32, TypeDef) {
    let oid = type_oid(env, schema, name).await;
    (oid, yload_oid(env, oid).await)
}

/// Beklenen yüklü tanım: sahip yüklenenden, boş yorum `None`, enum/composite `original = Some(ad)`.
fn yexpected(d: &TypeDef, loaded: &TypeDef) -> TypeDef {
    let body = match &d.body {
        TypeBody::Enum { values } => TypeBody::Enum {
            values: values.iter().map(|v| EnumValue { label: v.label.clone(), original: Some(v.label.clone()) }).collect(),
        },
        TypeBody::Composite { attributes } => TypeBody::Composite {
            attributes: attributes.iter().map(|a| CompositeAttr { original: Some(a.name.clone()), ..a.clone() }).collect(),
        },
        r => r.clone(),
    };
    TypeDef {
        owner: d.owner.clone().or_else(|| loaded.owner.clone()),
        comment: d.comment.clone().filter(|c| !c.is_empty()),
        body,
        editable: true,
        unsupported: vec![],
        ..d.clone()
    }
}

async fn ycreate_and_check(env: &Env, def: &TypeDef) -> (u32, TypeDef) {
    let plan = build::create(&ObjectDef::Type(def.clone()), &env.ctx(false)).unwrap_or_else(|e| panic!("create reddedildi: {} {e:?}", def.name));
    if let Err(e) = env.try_exec_tx(&plan).await {
        panic!("create uygulanamadı: {e}");
    }
    let (oid, loaded) = yload(env, &def.schema, &def.name).await;
    assert_eq!(loaded, yexpected(def, &loaded), "create → load ({})", def.name);
    assert_noop(env, &ObjectDef::Type(loaded.clone()));
    assert_eq!(yload_oid(env, oid).await, loaded, "iki yükleme aynı olmalı");
    // Sabit nokta (multirange adı da değişir: tip adı alanında tekil olmalı).
    let body = match &loaded.body {
        TypeBody::Range { multirange_type_name: Some(m), .. } => {
            let TypeBody::Range { subtype, subtype_opclass, collation, canonical, subtype_diff, .. } = loaded.body.clone() else { unreachable!() };
            TypeBody::Range { subtype, subtype_opclass, collation, canonical, subtype_diff, multirange_type_name: Some(format!("{m}_fp")) }
        }
        b => b.clone(),
    };
    let fp = TypeDef { name: format!("{}_fp", loaded.name), owner: None, comment: None, body, ..loaded.clone() };
    let plan = build::create(&ObjectDef::Type(fp.clone()), &env.ctx(false)).unwrap_or_else(|e| panic!("yüklü tanım create'te reddedildi: {e:?}"));
    env.exec_tx(&plan).await;
    let (_, l2) = yload(env, &fp.schema, &fp.name).await;
    assert_eq!(l2, TypeDef { owner: l2.owner.clone(), ..fp }, "sabit nokta");
    env.exec_tx(&build::drop(&ObjectDef::Type(l2), false, &env.ctx(false)).unwrap()).await;
    (oid, loaded)
}

/// alter → tek transaction (ya da PG < 12 enum ADD VALUE varsa ifade ifade) → yükle → beklenen.
async fn yalter_and_check(env: &Env, old: &TypeDef, new: &TypeDef) -> (TypeDef, Plan) {
    let plan = balter(env, &ObjectDef::Type(old.clone()), &ObjectDef::Type(new.clone()));
    if plan.statements.iter().all(|s| s.transactional) {
        if let Err(e) = env.try_exec_tx(&plan).await {
            panic!("alter uygulanamadı: {e}\n  plan: {:?}", sqls(&plan));
        }
    } else {
        env.exec(&plan).await;
    }
    let (_, loaded) = yload(env, &new.schema, &new.name).await;
    assert_eq!(loaded, yexpected(new, &loaded), "alter → load; plan: {:?}", sqls(&plan));
    assert_noop(env, &ObjectDef::Type(loaded.clone()));
    (loaded, plan)
}

fn enum_labels(d: &TypeDef) -> Vec<String> {
    match &d.body {
        TypeBody::Enum { values } => values.iter().map(|v| v.label.clone()).collect(),
        b => panic!("enum bekleniyordu: {b:?}"),
    }
}

/// Yüklü enum değerleri + taslak sırasında yeni değerler (`None` = yeni etiket).
fn enum_draft(items: &[(&str, Option<&str>)]) -> TypeBody {
    TypeBody::Enum { values: items.iter().map(|(l, o)| EnumValue { label: (*l).into(), original: o.map(Into::into) }).collect() }
}

docker_test!(type_enum_round_trip_and_add_values, enum_body, "tenum");

async fn enum_body(ts: TestServer) {
    let env = setup_in(&ts, PFX, "tenum").await;
    let major = env.version / 10000;
    env.sql(r#"CREATE SCHEMA "Ty Ş""#).await;
    let s = "Ty Ş";
    ycreate_and_check(&env, &tdef(s, "e_empty", ev_vals(&[]))).await;
    let d = TypeDef {
        owner: Some(env.role.clone()),
        comment: Some("yorum 'x'".into()),
        ..tdef(s, "Renk e", ev_vals(&["kırmızı", "it's", "a b", "Z", "😀"]))
    };
    let (_, l) = ycreate_and_check(&env, &d).await;

    // VERIFY (PG 12+): aynı transaction'da eklenen değere AFTER/BEFORE ile başvurmak çalışır.
    if env.version >= 120000 {
        env.sql("CREATE TYPE chain AS ENUM ('a', 'b'); BEGIN; ALTER TYPE chain ADD VALUE 'x' AFTER 'a'; ALTER TYPE chain ADD VALUE 'y' AFTER 'x'; ALTER TYPE chain ADD VALUE 'w' BEFORE 'x'; COMMIT")
            .await;
        let labels: Vec<String> = env
            .admin
            .query("SELECT enumlabel::text FROM pg_enum WHERE enumtypid = 'chain'::regtype ORDER BY enumsortorder", &[])
            .await
            .unwrap()
            .iter()
            .map(|r| r.get(0))
            .collect();
        assert_eq!(labels, ["a", "w", "x", "y", "b"]);
        eprintln!("PG {major}: zincirleme ADD VALUE (aynı transaction) çalışıyor");
    }

    // Yeniden adlandırma + başa, araya (zincirleme iki değer) ve sona ekleme.
    let new = TypeDef {
        body: enum_draft(&[
            ("ilk", None),
            ("kırmızı", Some("kırmızı")),
            ("it is", Some("it's")),
            ("ara1", None),
            ("ara2", None),
            ("a b", Some("a b")),
            ("Z", Some("Z")),
            ("😀", Some("😀")),
            ("son", None),
        ]),
        ..l.clone()
    };
    let (l2, plan) = yalter_and_check(&env, &l, &new).await;
    assert_eq!(enum_labels(&l2), ["ilk", "kırmızı", "it is", "ara1", "ara2", "a b", "Z", "😀", "son"]);
    assert_eq!(plan.statements.len(), 5, "{:?}", sqls(&plan));
    assert!(plan.statements[0].sql.contains("RENAME VALUE"));
    assert_eq!(plan.statements.iter().filter(|s| !s.transactional).count(), if env.version < 120000 { 4 } else { 0 });
    // Boş enum'a ekleme (konumsuz) + zincir.
    let (_, e0) = yload(&env, s, "e_empty").await;
    let (e1, _) = yalter_and_check(&env, &e0, &TypeDef { body: enum_draft(&[("p", None), ("q", None)]), ..e0.clone() }).await;
    assert_eq!(enum_labels(&e1), ["p", "q"]);

    // Yerel ret: silme ve yeniden sıralama.
    let e = build::alter(&ObjectDef::Type(l2.clone()), &ObjectDef::Type(TypeDef { body: enum_draft(&[("ilk", Some("ilk"))]), ..l2.clone() }), &env.ctx(false)).unwrap_err();
    assert_eq!(paths(&e), ["body.values"]);
}

docker_test!(type_enum_partial_apply_reports_committed, enum_partial_body, "tenumpart");

/// PG < 12: `ADD VALUE` transaction dışıdır; ardından başarısız olan `OWNER TO` ile `committed = 1` ve değer kalıcı.
/// PG 12+: hepsi tek transaction; `committed = 0` ve değer eklenmez. `DdlManager` uygulama yolu üzerinden.
async fn enum_partial_body(ts: TestServer) {
    let env = setup_in(&ts, PFX, "tenumpart").await;
    let major = env.version / 10000;
    env.sql("CREATE SCHEMA tp; CREATE TYPE tp.e AS ENUM ('a')").await;
    let dir = tempfile::tempdir().unwrap();
    let mut registry = Registry::load(dir.path().join("servers.json")).unwrap().registry;
    registry.upsert(ts.config.clone()).unwrap();
    let dm = Arc::new(DdlManager::new(env.mgr.clone(), Arc::new(RwLock::new(registry))));
    let (oid, old) = yload(&env, "tp", "e").await;
    let new = TypeDef { owner: Some("pgt_yok_sahip".into()), body: enum_draft(&[("a", Some("a")), ("b", None)]), ..old.clone() };
    let object = ObjectRef { kind: ObjectKind::Type, oid };
    let req = PlanRequest::Alter { object, old: ObjectDef::Type(old.clone()), new: ObjectDef::Type(new) };
    let view = dm.preview(env.server, env.db_oid, &req).await.unwrap();
    let o = ApplyOptions {
        apply_id: Uuid::new_v4(),
        lock_timeout_ms: 5000,
        expected_hash: view.hash.clone(),
        confirmed: true,
        confirmed_production: false,
        password: None,
    };
    let out = dm.apply(env.server, env.db_oid, &req, o).await;
    let Ok(ApplyOutcome::Failed { failed_index, committed, sqlstate, .. }) = out else { panic!("{out:?}") };
    let (_, l) = yload(&env, "tp", "e").await;
    if env.version < 120000 {
        assert_eq!((failed_index, committed, sqlstate.as_deref()), (1, 1, Some("42704")));
        assert_eq!(enum_labels(&l), ["a", "b"], "ADD VALUE kalıcı olmalı (transaction dışı)");
        eprintln!("PG {major}: kısmi uygulama committed = 1");
    } else {
        assert_eq!((failed_index, committed, sqlstate.as_deref()), (1, 0, Some("42704")));
        assert_eq!(l, old, "hiçbir şey değişmemeli");
    }
}

docker_test!(type_composite_round_trip_and_ops, composite_body, "tcomp");

async fn composite_body(ts: TestServer) {
    let env = setup_in(&ts, PFX, "tcomp").await;
    env.sql(r#"CREATE SCHEMA tc; CREATE TYPE tc.inner_t AS ENUM ('x')"#).await;
    let d = TypeDef {
        comment: Some("c".into()),
        ..tdef(
            "tc",
            "Kişi",
            TypeBody::Composite {
                attributes: vec![
                    attr("id", "integer", None),
                    attr("Ad Soyad", "text", Some("C")),
                    attr("tutar", "numeric(5,2)", None),
                    attr("etiketler", "character varying(10)[]", None),
                    attr("iç", "tc.inner_t", None),
                    attr("zaman", "timestamp with time zone", None),
                ],
            },
        )
    };
    let (_, l) = ycreate_and_check(&env, &d).await;
    ycreate_and_check(&env, &tdef("tc", "bos", TypeBody::Composite { attributes: vec![] })).await;

    // RENAME + DROP + ALTER TYPE (+collation) + ADD (sırayla).
    let TypeBody::Composite { attributes: la } = &l.body else { unreachable!() };
    let new_attrs = vec![
        CompositeAttr { name: "kimlik".into(), ..la[0].clone() },
        CompositeAttr { collation: None, ..la[1].clone() },
        CompositeAttr { type_name: "numeric(10,2)".into(), ..la[2].clone() },
        la[4].clone(),
        la[5].clone(),
        attr("yeni", "text", Some("C")),
    ];
    let new = TypeDef { body: TypeBody::Composite { attributes: new_attrs }, ..l.clone() };
    let (l2, plan) = yalter_and_check(&env, &l, &new).await;
    let got = sqls(&plan);
    assert_eq!(got.len(), 5, "{got:?}");
    assert!(got[0].contains("RENAME ATTRIBUTE") && got[1].contains("DROP ATTRIBUTE") && got[4].contains("ADD ATTRIBUTE"), "{got:?}");

    // Typed table kullanırken öznitelik ekleme: PG hatası (CASCADE yazılmaz), hiçbir şey değişmez.
    env.sql(r#"CREATE TABLE tc.tt OF tc."Kişi""#).await;
    let mut attrs = match &l2.body {
        TypeBody::Composite { attributes } => attributes.clone(),
        _ => unreachable!(),
    };
    attrs.push(attr("z", "integer", None));
    let plan = balter(&env, &ObjectDef::Type(l2.clone()), &ObjectDef::Type(TypeDef { body: TypeBody::Composite { attributes: attrs }, ..l2.clone() }));
    let e = env.try_exec_tx(&plan).await.unwrap_err();
    assert_eq!(sqlstate(&e), "2BP01", "{e}");
    assert_eq!(yload(&env, "tc", "Kişi").await.1, l2, "hiçbir şey değişmemeli");

    // Tablo satır tipi salt okunur.
    let (_, row) = yload(&env, "tc", "tt").await;
    assert!(!row.editable && !row.unsupported.is_empty(), "{row:?}");
    assert_read_only(&env, &ObjectDef::Type(row.clone()), &ObjectDef::Type(TypeDef { comment: Some("x".into()), ..row.clone() }));
}

docker_test!(type_range_round_trip, range_body, "trange");

async fn range_body(ts: TestServer) {
    let env = setup_in(&ts, PFX, "trange").await;
    let major = env.version / 10000;
    env.sql("CREATE SCHEMA tr; CREATE FUNCTION tr.fdiff(float8, float8) RETURNS float8 LANGUAGE sql IMMUTABLE AS 'SELECT $1 - $2'").await;
    let mut defs = vec![
        tdef("tr", "f8r", rng("double precision", None, None, Some("tr.fdiff"), None)),
        tdef("tr", "Metin R", rng("text", None, Some("C"), None, None)),
        tdef("tr", "pat", rng("text", Some("text_pattern_ops"), None, None, None)),
        tdef("tr", "pg_diff", rng("double precision", None, None, Some("float8mi"), None)),
        TypeDef { comment: Some("t".into()), owner: Some(env.role.clone()), ..tdef("tr", "ts_r", range("timestamp with time zone")) },
    ];
    if env.version >= 140000 {
        defs.push(tdef("tr", "mr", rng("integer", None, None, None, Some("Çoklu"))));
    } else {
        eprintln!("PG {major}: MULTIRANGE_TYPE_NAME atlandı (PG 14+)");
    }
    for d in &defs {
        ycreate_and_check(&env, d).await;
    }
    // Ad + yorum + sahip değişir; gövde değişimi yerel ret.
    // PG 14+: range'in adı değişince otomatik multirange adı ESKİ adla kalır → yüklemede artık açık ad (`Some`).
    let (_, l) = yload(&env, "tr", "f8r").await;
    let new = TypeDef { name: "f8r2".into(), comment: Some("y".into()), owner: Some(env.role.clone()), ..l.clone() };
    let plan = balter(&env, &ObjectDef::Type(l.clone()), &ObjectDef::Type(new.clone()));
    assert_eq!(plan.statements.len(), 3);
    env.exec_tx(&plan).await;
    let (_, l2) = yload(&env, "tr", "f8r2").await;
    let want_body = match (&new.body, env.version >= 140000) {
        (TypeBody::Range { subtype, subtype_opclass, collation, canonical, subtype_diff, .. }, true) => TypeBody::Range {
            subtype: subtype.clone(),
            subtype_opclass: subtype_opclass.clone(),
            collation: collation.clone(),
            canonical: canonical.clone(),
            subtype_diff: subtype_diff.clone(),
            multirange_type_name: Some("f8r_multirange".into()),
        },
        (b, _) => b.clone(),
    };
    assert_eq!(l2, TypeDef { body: want_body, ..new.clone() });
    assert_noop(&env, &ObjectDef::Type(l2.clone()));
    let e = build::alter(&ObjectDef::Type(l2.clone()), &ObjectDef::Type(TypeDef { body: range("numeric"), ..l2.clone() }), &env.ctx(false)).unwrap_err();
    assert!(paths(&e).contains(&"body.subtype"), "{e:?}");

    // Sistem range'i salt okunur; canonical/subtype_diff regproc metni.
    let l = yload_oid(&env, type_oid(&env, "pg_catalog", "int4range").await).await;
    assert!(!l.editable && l.unsupported.iter().any(|u| u == "sistem şeması"), "{l:?}");
    let TypeBody::Range { canonical, subtype_diff, subtype, .. } = &l.body else { panic!("{l:?}") };
    assert_eq!((subtype.as_str(), canonical.as_deref(), subtype_diff.as_deref()), ("integer", Some("int4range_canonical"), Some("int4range_subdiff")));
    // Multirange tipi kendisi (PG 14+) ve base type salt okunur; domain Type olarak yüklenemez.
    if env.version >= 140000 {
        let m = yload_oid(&env, type_oid(&env, "tr", "Çoklu").await).await;
        assert!(!m.editable && m.unsupported.iter().any(|u| u.contains("MULTIRANGE")), "{m:?}");
        let (_, d) = yload(&env, "tr", "pat").await;
        let TypeBody::Range { multirange_type_name, .. } = &d.body else { unreachable!() };
        assert_eq!(multirange_type_name, &None, "varsayılan multirange adı None");
        let n: i64 = env.admin.query_one("SELECT count(*) FROM pg_type WHERE typname = 'patmultirange'", &[]).await.unwrap().get(0);
        let n2: i64 = env.admin.query_one("SELECT count(*) FROM pg_type WHERE typname = 'pat_multirange'", &[]).await.unwrap().get(0);
        assert_eq!(n + n2, 1);
    }
    let b = yload_oid(&env, type_oid(&env, "pg_catalog", "int4").await).await;
    assert!(!b.editable && b.unsupported.iter().any(|u| u.contains("BASE")), "{b:?}");
    env.sql("CREATE DOMAIN tr.dd AS integer").await;
    assert!(matches!(try_load(&env, ObjectKind::Type, type_oid(&env, "tr", "dd").await).await, Err(Error::Invalid(_))));
    // Eklentiye ait tip salt okunur.
    env.sql("CREATE TYPE tr.ext_e AS ENUM ('a'); ALTER EXTENSION plpgsql ADD TYPE tr.ext_e").await;
    let x = yload(&env, "tr", "ext_e").await.1;
    assert!(!x.editable && x.unsupported.iter().any(|u| u.contains("EXTENSION plpgsql")), "{x:?}");
    env.sql("ALTER EXTENSION plpgsql DROP TYPE tr.ext_e").await;
    assert!(matches!(try_load(&env, ObjectKind::Type, 4_000_000_000).await, Err(Error::NotFound(_))));
}

// ================================================================================================================
// Domain
// ================================================================================================================

fn ddef(schema: &str, name: &str, base: &str) -> DomainDef {
    DomainDef {
        schema: schema.into(),
        name: name.into(),
        owner: None,
        comment: None,
        base_type: base.into(),
        collation: None,
        default: None,
        not_null: false,
        constraints: vec![],
        editable: true,
        unsupported: vec![],
    }
}

fn chk(name: &str, expr: &str) -> DomainCheck {
    DomainCheck { name: Some(name.into()), expression: expr.into(), not_valid: false, validated: true, original: None, validate: false }
}

async fn dload_oid(env: &Env, oid: u32) -> DomainDef {
    match try_load(env, ObjectKind::Domain, oid).await {
        Ok(ObjectDef::Domain(d)) => d,
        other => panic!("domain {oid} yüklenemedi: {other:?}"),
    }
}

async fn dload(env: &Env, schema: &str, name: &str) -> (u32, DomainDef) {
    let oid = type_oid(env, schema, name).await;
    (oid, dload_oid(env, oid).await)
}

/// Beklenen: sahip yüklenenden, boş yorum `None`, kısıtlar ada göre sıralı, `original = Some(ad)`, `not_valid = false`,
/// `validated` = (yüklüde doğrulanmış ya da `validate` istenmiş) ve `NOT VALID` değil, `validate = false`.
fn dexpected(d: &DomainDef, loaded: &DomainDef) -> DomainDef {
    let mut constraints: Vec<DomainCheck> = d
        .constraints
        .iter()
        .map(|c| DomainCheck {
            original: c.name.clone(),
            not_valid: false,
            validated: (c.validated || c.validate) && !c.not_valid,
            validate: false,
            ..c.clone()
        })
        .collect();
    constraints.sort_by(|a, b| a.name.cmp(&b.name));
    DomainDef {
        owner: d.owner.clone().or_else(|| loaded.owner.clone()),
        comment: d.comment.clone().filter(|c| !c.is_empty()),
        constraints,
        editable: true,
        unsupported: vec![],
        ..d.clone()
    }
}

async fn dcreate_and_check(env: &Env, def: &DomainDef) -> (u32, DomainDef) {
    let plan = build::create(&ObjectDef::Domain(def.clone()), &env.ctx(false)).unwrap_or_else(|e| panic!("create reddedildi: {} {e:?}", def.name));
    if let Err(e) = env.try_exec_tx(&plan).await {
        panic!("create uygulanamadı: {e}");
    }
    let (oid, loaded) = dload(env, &def.schema, &def.name).await;
    assert_eq!(loaded, dexpected(def, &loaded), "create → load ({})", def.name);
    assert_noop(env, &ObjectDef::Domain(loaded.clone()));
    assert_eq!(dload_oid(env, oid).await, loaded, "iki yükleme aynı olmalı");
    let fp = DomainDef { name: format!("{}_fp", loaded.name), owner: None, comment: None, ..loaded.clone() };
    let plan = build::create(&ObjectDef::Domain(fp.clone()), &env.ctx(false)).unwrap_or_else(|e| panic!("yüklü tanım create'te reddedildi: {e:?}"));
    env.exec_tx(&plan).await;
    let (_, l2) = dload(env, &fp.schema, &fp.name).await;
    assert_eq!(l2, DomainDef { owner: l2.owner.clone(), ..fp }, "sabit nokta");
    env.exec_tx(&build::drop(&ObjectDef::Domain(l2), false, &env.ctx(false)).unwrap()).await;
    (oid, loaded)
}

async fn dalter_and_check(env: &Env, old: &DomainDef, new: &DomainDef) -> (DomainDef, Plan) {
    let plan = balter(env, &ObjectDef::Domain(old.clone()), &ObjectDef::Domain(new.clone()));
    if let Err(e) = env.try_exec_tx(&plan).await {
        panic!("alter uygulanamadı: {e}\n  plan: {:?}", sqls(&plan));
    }
    let (_, loaded) = dload(env, &new.schema, &new.name).await;
    assert_eq!(loaded, dexpected(new, &loaded), "alter → load; plan: {:?}", sqls(&plan));
    assert_noop(env, &ObjectDef::Domain(loaded.clone()));
    (loaded, plan)
}

docker_test!(domain_round_trip_and_constraint_ops, domain_body, "tdomain");

async fn domain_body(ts: TestServer) {
    let env = setup_in(&ts, PFX, "tdomain").await;
    let major = env.version / 10000;
    env.sql(r#"CREATE SCHEMA "Dm Ş""#).await;
    let s = "Dm Ş";
    let defs = vec![
        DomainDef {
            collation: Some("C".into()),
            default: Some("'x'::character varying".into()),
            not_null: true,
            constraints: vec![
                chk("c1", "VALUE::text <> 'y'::text"),
                chk("B iki", "length(VALUE::text) > 0 AND VALUE::text ~ '^a'::text"),
            ],
            comment: Some("yorum".into()),
            owner: Some(env.role.clone()),
            ..ddef(s, "kod", "character varying(10)")
        },
        DomainDef { default: Some("1 + 2".into()), ..ddef(s, "Sayı D", "integer") },
        ddef(s, "duz", "numeric(5,2)"),
        DomainDef { constraints: vec![chk("pos", "array_length(VALUE, 1) > 0")], ..ddef(s, "arr", "integer[]") },
    ];
    for d in &defs {
        dcreate_and_check(&env, d).await;
    }
    let (oid, l) = dload(&env, s, "kod").await;
    if env.version >= 170000 {
        let n: i64 = env.admin.query_one("SELECT count(*) FROM pg_constraint WHERE contypid = $1 AND contype = 'n'", &[&oid]).await.unwrap().get(0);
        assert_eq!(n, 1, "PG 17+ NOT NULL kısıt satırı");
        eprintln!("PG {major}: domain NOT NULL satırı (contype n) kısıt listesine girmedi");
    }

    // Kısıt işlemleri: RENAME, ifade değişimi (DROP + ADD), DROP, NOT VALID ekleme; varsayılan kaldırma; NOT NULL kaldırma.
    let c = |n: &str| l.constraints.iter().find(|c| c.name.as_deref() == Some(n)).unwrap().clone();
    let new = DomainDef {
        default: None,
        not_null: false,
        constraints: vec![
            DomainCheck { name: Some("c1 yeni".into()), ..c("c1") },
            DomainCheck { name: Some("nv".into()), expression: "VALUE::text > 'a'::text".into(), not_valid: true, validated: false, original: None, validate: false },
        ],
        ..l.clone()
    };
    let (l2, plan) = dalter_and_check(&env, &l, &new).await;
    assert_eq!(plan.statements.len(), 5, "{:?}", sqls(&plan));
    let nv = l2.constraints.iter().find(|c| c.name.as_deref() == Some("nv")).unwrap();
    assert!(!nv.validated && !nv.not_valid);
    // VALIDATE + ifade değişimi + yeni varsayılan.
    let new = DomainDef {
        default: Some("'b'::character varying".into()),
        constraints: l2
            .constraints
            .iter()
            .map(|c| match c.name.as_deref() {
                Some("nv") => DomainCheck { validate: true, ..c.clone() },
                _ => DomainCheck { expression: "VALUE::text <> 'z'::text".into(), ..c.clone() },
            })
            .collect(),
        ..l2.clone()
    };
    let (l3, _) = dalter_and_check(&env, &l2, &new).await;
    assert!(l3.constraints.iter().all(|c| c.validated));

    // SET NOT NULL, kullanan kolonda NULL varken başarısız olur; transaction geri alınır (yorum da yazılmaz).
    env.sql(&format!("CREATE TABLE \"Dm Ş\".t (v {}); INSERT INTO \"Dm Ş\".t VALUES (NULL)", qualified(s, "kod"))).await;
    let plan = balter(&env, &ObjectDef::Domain(l3.clone()), &ObjectDef::Domain(DomainDef { not_null: true, comment: Some("z".into()), ..l3.clone() }));
    let e = env.try_exec_tx(&plan).await.unwrap_err();
    assert_eq!(sqlstate(&e), "23502", "{e}");
    assert_eq!(dload(&env, s, "kod").await.1, l3, "hiçbir şey değişmemeli");

    // Salt okunur: sistem domain'i, eklentiye ait; Type oid'i Domain olarak yüklenemez.
    let sys = dload_oid(&env, type_oid(&env, "information_schema", "cardinal_number").await).await;
    assert!(!sys.editable && sys.unsupported.iter().any(|u| u == "sistem şeması"), "{sys:?}");
    env.sql(r#"CREATE DOMAIN "Dm Ş".ext_d AS integer; ALTER EXTENSION plpgsql ADD DOMAIN "Dm Ş".ext_d"#).await;
    let x = dload(&env, s, "ext_d").await.1;
    assert!(!x.editable && x.unsupported.iter().any(|u| u.contains("EXTENSION plpgsql")), "{x:?}");
    assert_read_only(&env, &ObjectDef::Domain(x.clone()), &ObjectDef::Domain(DomainDef { comment: Some("x".into()), ..x.clone() }));
    env.sql(r#"ALTER EXTENSION plpgsql DROP DOMAIN "Dm Ş".ext_d"#).await;
    env.sql(r#"CREATE TYPE "Dm Ş".en AS ENUM ('a')"#).await;
    assert!(matches!(try_load(&env, ObjectKind::Domain, type_oid(&env, s, "en").await).await, Err(Error::Invalid(_))));
    assert!(matches!(try_load(&env, ObjectKind::Domain, 4_000_000_000).await, Err(Error::NotFound(_))));
}
