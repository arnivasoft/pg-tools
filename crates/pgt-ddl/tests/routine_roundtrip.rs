//! Function / Procedure üreticisi → gerçek sunucu → yükleyici gidiş-dönüş testleri (Docker; `PGT_TEST_URLS` yoksa
//! atlanır).
//!
//! Her test kendi geçici veritabanını (`pgt_ddl_fn_<ana sürüm>_<etiket>`) ve rolünü kurar; başarısız doğrulamadan sonra
//! bile temizlenir. Üreticinin SQL'i `DdlManager` olmadan, uygulayıcı gibi tek transaction'da çalıştırılır
//! (`Env::exec_tx`). Sürüme bağlı kurallar `server_version`'a göre dallanır; atlanan kısım yazdırılır.
//!
//! **Sabit nokta:** yüklenen her tanımda `alter(yüklü, yüklü)` boş plandır ve yüklü tanımla (başka adla) yeniden
//! oluşturulan nesnenin yüklü tanımı aynıdır (ad hariç). Beklenen tanım, create girdisinden yükleyici kurallarıyla
//! çıkarılır (`expected`): sahip yüklenenden, COST fonksiyonda her zaman dolu (varsayılan `100`), ROWS küme dönende
//! dolu (varsayılan `1000`), ayarlar ada göre sıralı.

mod common;

use common::{Env, cleanup_in, guarded, servers, setup_in};
use pgt_core::Error;
use pgt_core::testing::TestServer;
use pgt_ddl::build;
use pgt_ddl::*;

const PFX: &str = "pgt_ddl_fn";

// ---- tanım kurucuları ----

fn func(schema: &str, name: &str, language: &str, returns: FunctionReturn, body: &str) -> FunctionDef {
    FunctionDef {
        schema: schema.into(),
        name: name.into(),
        owner: None,
        comment: None,
        language: language.into(),
        args: vec![],
        returns: Some(returns),
        body: body.into(),
        volatility: Volatility::Volatile,
        strict: false,
        security_definer: false,
        leakproof: false,
        parallel: Parallel::Unsafe,
        cost: None,
        rows: None,
        settings: vec![],
        recreate: false,
        editable: true,
        unsupported: vec![],
    }
}

fn proc_def(schema: &str, name: &str, body: &str) -> FunctionDef {
    FunctionDef { returns: None, ..func(schema, name, "plpgsql", FunctionReturn::Void, body) }
}

fn ty(t: &str) -> FunctionReturn {
    FunctionReturn::Type { type_name: t.into(), setof: false }
}

fn setof(t: &str) -> FunctionReturn {
    FunctionReturn::Type { type_name: t.into(), setof: true }
}

fn arg(mode: ArgMode, name: Option<&str>, t: &str, default: Option<&str>) -> FunctionArg {
    FunctionArg { mode, name: name.map(Into::into), type_name: t.into(), default: default.map(Into::into) }
}

fn a_in(name: &str, t: &str) -> FunctionArg {
    arg(ArgMode::In, Some(name), t, None)
}

fn set(n: &str, v: &str) -> ConfigSetting {
    ConfigSetting { name: n.into(), value: v.into() }
}

fn wrap(kind: ObjectKind, d: &FunctionDef) -> ObjectDef {
    match kind {
        ObjectKind::Procedure => ObjectDef::Procedure(d.clone()),
        _ => ObjectDef::Function(d.clone()),
    }
}

fn sqls(p: &Plan) -> Vec<&str> {
    p.statements.iter().map(|s| s.sql.as_str()).collect()
}

fn paths(e: &[FieldError]) -> Vec<&str> {
    e.iter().map(|e| e.path.as_str()).collect()
}

/// `regprocedure` giriş tipi listesi (OUT ve TABLE hariç).
fn input_types(d: &FunctionDef) -> String {
    d.args.iter().filter(|a| a.mode != ArgMode::Out).map(|a| a.type_name.as_str()).collect::<Vec<_>>().join(", ")
}

/// Sunucu iletisindeki SQLSTATE (`db_message` biçimi: "ileti (KOD)").
fn sqlstate(msg: &str) -> &str {
    msg.split_once('\n').map_or(msg, |(first, _)| first).rsplit_once('(').map_or("", |(_, c)| c.trim_end_matches(')'))
}

// ---- yükle / karşılaştır ----

async fn try_load(env: &Env, kind: ObjectKind, oid: u32) -> pgt_core::Result<ObjectDef> {
    load(&env.mgr, env.server, env.db_oid, ObjectRef { kind, oid }).await
}

async fn load_oid(env: &Env, kind: ObjectKind, oid: u32) -> FunctionDef {
    match try_load(env, kind, oid).await {
        Ok(ObjectDef::Function(d)) if kind == ObjectKind::Function => d,
        Ok(ObjectDef::Procedure(d)) if kind == ObjectKind::Procedure => d,
        other => panic!("{kind:?} {oid} yüklenemedi: {other:?}"),
    }
}

/// `d`'nin şema/ad/giriş tipleriyle yükler.
async fn fload(env: &Env, kind: ObjectKind, d: &FunctionDef) -> (u32, FunctionDef) {
    let oid = env.proc_oid(&d.schema, &d.name, &input_types(d)).await;
    (oid, load_oid(env, kind, oid).await)
}

/// Beklenen yüklü tanım: belirtilmeyen sahip yüklenenden, boş yorum `None`, COST fonksiyonda `100` (belirtilmemişse),
/// ROWS küme dönende `1000` (belirtilmemişse), procedure'de ikisi `None`; ayarlar ada göre sıralı; `recreate` yok.
fn expected(kind: ObjectKind, def: &FunctionDef, loaded: &FunctionDef) -> FunctionDef {
    let function = kind == ObjectKind::Function;
    let set_returning = matches!(def.returns, Some(FunctionReturn::Type { setof: true, .. }) | Some(FunctionReturn::Table { .. }));
    let mut settings = def.settings.clone();
    settings.sort_by(|a, b| a.name.cmp(&b.name));
    FunctionDef {
        owner: def.owner.clone().or_else(|| loaded.owner.clone()),
        comment: def.comment.clone().filter(|c| !c.is_empty()),
        cost: if function { def.cost.clone().or(Some("100".into())) } else { None },
        rows: if function && set_returning { def.rows.clone().or(Some("1000".into())) } else { None },
        settings,
        recreate: false,
        editable: true,
        unsupported: vec![],
        ..def.clone()
    }
}

fn assert_noop(env: &Env, kind: ObjectKind, d: &FunctionDef) {
    let o = wrap(kind, d);
    let p = build::alter(&o, &o, &env.ctx(false)).unwrap_or_else(|e| panic!("noop reddedildi: {e:?}\n{d:?}"));
    assert!(p.statements.is_empty() && p.warnings.is_empty(), "{}: boş plan bekleniyordu: {:?} {:?}", d.name, sqls(&p), p.warnings);
}

/// Sabit nokta: yüklü tanımla `<ad>_fp` oluşturulur; yüklü tanımı ad dışında aynıdır; silinir.
async fn fixed_point(env: &Env, kind: ObjectKind, loaded: &FunctionDef) {
    let fp = FunctionDef { name: format!("{}_fp", loaded.name), owner: None, comment: None, ..loaded.clone() };
    let plan = build::create(&wrap(kind, &fp), &env.ctx(false))
        .unwrap_or_else(|e| panic!("yüklü tanım create'te reddedildi: {e:?}\n{loaded:?}"));
    env.exec_tx(&plan).await;
    let (_, l2) = fload(env, kind, &fp).await;
    let want = FunctionDef { name: fp.name.clone(), owner: l2.owner.clone(), comment: None, ..loaded.clone() };
    assert_eq!(l2, want, "sabit nokta");
    env.exec_tx(&build::drop(&wrap(kind, &l2), false, &env.ctx(false)).unwrap()).await;
}

/// create → tek transaction → yükle → beklenen tanıma eşit + noop + iki yükleme aynı + sabit nokta.
async fn create_and_check(env: &Env, kind: ObjectKind, def: &FunctionDef) -> (u32, FunctionDef) {
    let plan = build::create(&wrap(kind, def), &env.ctx(false)).unwrap_or_else(|e| panic!("create reddedildi: {} {e:?}", def.name));
    if let Err(e) = env.try_exec_tx(&plan).await {
        panic!("create uygulanamadı: {e}");
    }
    let (oid, loaded) = fload(env, kind, def).await;
    assert_eq!(loaded, expected(kind, def, &loaded), "create → load ({})", def.name);
    assert!(loaded.owner.is_some() && loaded.editable && loaded.unsupported.is_empty());
    assert_noop(env, kind, &loaded);
    assert_eq!(load_oid(env, kind, oid).await, loaded, "iki yükleme aynı olmalı");
    fixed_point(env, kind, &loaded).await;
    (oid, loaded)
}

fn falter(env: &Env, kind: ObjectKind, old: &FunctionDef, new: &FunctionDef) -> Plan {
    let p = build::alter(&wrap(kind, old), &wrap(kind, new), &env.ctx(false))
        .unwrap_or_else(|e| panic!("alter reddedildi: {e:?}\n  old: {old:?}\n  new: {new:?}"));
    for s in &p.statements {
        assert!(!s.sql.trim_end().ends_with(';'), "{}", s.sql);
    }
    p
}

/// alter(old, new) → tek transaction → yükle (`new` imzasıyla) → beklenen yeni tanıma eşit + noop.
async fn alter_and_check(env: &Env, kind: ObjectKind, old: &FunctionDef, new: &FunctionDef) -> (u32, FunctionDef, Plan) {
    let plan = falter(env, kind, old, new);
    if let Err(e) = env.try_exec_tx(&plan).await {
        panic!("alter uygulanamadı: {e}\n  plan: {:?}", sqls(&plan));
    }
    let (oid, loaded) = fload(env, kind, new).await;
    assert_eq!(loaded, expected(kind, new, &loaded), "alter → load; plan: {:?}", sqls(&plan));
    assert_noop(env, kind, &loaded);
    (oid, loaded, plan)
}

async fn can_execute(env: &Env, role: &str, oid: u32) -> bool {
    env.admin.query_one("SELECT has_function_privilege($1::text, $2::oid, 'EXECUTE')", &[&role, &oid]).await.unwrap().get(0)
}

async fn acl_is_default(env: &Env, oid: u32) -> bool {
    env.admin.query_one("SELECT proacl IS NULL FROM pg_proc WHERE oid = $1", &[&oid]).await.unwrap().get(0)
}

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

// ================================================================================================================
// create → load
// ================================================================================================================

docker_test!(routine_create_then_load_matches, create_body, "fcreate");

async fn create_body(ts: TestServer) {
    let env = setup_in(&ts, PFX, "fcreate").await;
    let major = env.version / 10000;
    let s = "Fn Ş";
    env.sql(r#"CREATE SCHEMA "Fn Ş""#).await;
    let f = ObjectKind::Function;
    let defs = vec![
        // Skaler sql, DEFAULT'lu, öznitelikli.
        FunctionDef {
            args: vec![a_in("a", "integer"), arg(ArgMode::In, Some("b"), "integer", Some("2"))],
            volatility: Volatility::Immutable,
            strict: true,
            parallel: Parallel::Safe,
            cost: Some("5".into()),
            ..func(s, "f_sql", "sql", ty("integer"), "SELECT a + b")
        },
        // plpgsql küme dönen, ROWS + STABLE.
        FunctionDef {
            args: vec![a_in("n", "integer")],
            volatility: Volatility::Stable,
            rows: Some("50".into()),
            cost: Some("0.5".into()),
            ..func(s, "f_set", "plpgsql", setof("integer"), "BEGIN FOR i IN 1..n LOOP RETURN NEXT i; END LOOP; END")
        },
        // TABLE dönen (hostile kolon adı), varsayılan ROWS.
        FunctionDef {
            args: vec![a_in("a", "integer")],
            ..func(
                s,
                "f_tbl",
                "sql",
                FunctionReturn::Table {
                    columns: vec![
                        ReturnColumn { name: "x".into(), type_name: "integer".into() },
                        ReturnColumn { name: "Y z\"".into(), type_name: "text".into() },
                    ],
                },
                "SELECT a, 'q'::text",
            )
        },
        // Trigger fonksiyonu.
        func(s, "f_trg", "plpgsql", FunctionReturn::Trigger, "BEGIN RETURN NEW; END"),
        // OUT/INOUT/VARIADIC (çok çıkış → record).
        FunctionDef {
            args: vec![
                a_in("a", "integer"),
                arg(ArgMode::Inout, Some("b"), "text", None),
                arg(ArgMode::Out, Some("c"), "numeric", None),
                arg(ArgMode::Variadic, Some("v"), "integer[]", None),
            ],
            ..func(s, "f_modes", "plpgsql", ty("record"), "BEGIN b := b || a::text; c := a; END")
        },
        // Tek OUT → dönüş tipi o OUT'un tipi; adsız giriş.
        FunctionDef {
            args: vec![arg(ArgMode::In, None, "integer", None), arg(ArgMode::Out, Some("r"), "integer", None)],
            ..func(s, "f_one", "sql", ty("integer"), "SELECT $1")
        },
        // Varsayılanlar (`pg_get_function_arg_default` biçiminde).
        FunctionDef {
            args: vec![
                arg(ArgMode::In, Some("a"), "integer", Some("1")),
                arg(ArgMode::In, Some("b"), "text", Some("'x'::text")),
                arg(ArgMode::In, Some("c"), "integer[]", Some("ARRAY[1, 2]")),
                arg(ArgMode::In, Some("d"), "timestamp with time zone", Some("now()")),
            ],
            ..func(s, "f_def", "plpgsql", FunctionReturn::Void, "BEGIN NULL; END")
        },
        // SET listesi (liste ayarı, kanonik ad TimeZone, noktalı özel ad).
        FunctionDef {
            settings: vec![
                set("work_mem", "64MB"),
                set("search_path", "\"$user\", public"),
                set("timezone", "UTC"),
                set("App.Mode", "it's"),
            ],
            ..func(s, "f_set_list", "sql", ty("text"), "SELECT current_setting('work_mem')")
        },
        // SECURITY DEFINER + LEAKPROOF (süper kullanıcı) + sahip + yorum + Unicode ad + büyük COST.
        FunctionDef {
            owner: Some(env.role.clone()),
            comment: Some("yorum; -- 'x' /* y */ \\ ğ".into()),
            args: vec![a_in("a", "integer")],
            security_definer: true,
            leakproof: true,
            volatility: Volatility::Immutable,
            cost: Some("10000000000".into()),
            settings: vec![set("search_path", "pg_catalog")],
            ..func(s, "Çağrı f", "sql", ty("boolean"), "SELECT a > 0")
        },
        // void sql (PG 10–13: son SELECT void dönmeli).
        func(s, "f_void", "sql", FunctionReturn::Void, "SELECT pg_sleep(0)"),
    ];
    for d in &defs {
        let (_, l) = create_and_check(&env, f, d).await;
        assert_eq!(l.language, d.language);
    }
    // Procedure (PG 11+); OUT argümanlı procedure PG 14+.
    let p = ObjectKind::Procedure;
    let base = FunctionDef {
        owner: Some(env.role.clone()),
        comment: Some("p".into()),
        args: vec![a_in("a", "integer"), arg(ArgMode::Inout, Some("b"), "text", None)],
        security_definer: true,
        settings: vec![set("work_mem", "1MB")],
        ..proc_def(s, "p_main", "BEGIN b := b || a::text; END")
    };
    if env.version >= 110000 {
        create_and_check(&env, p, &base).await;
        create_and_check(&env, p, &proc_def(s, "p_empty", "BEGIN NULL; END")).await;
        if env.version >= 140000 {
            let out = FunctionDef {
                args: vec![a_in("a", "integer"), arg(ArgMode::Out, Some("b"), "text", None)],
                ..proc_def(s, "p_out", "BEGIN b := a::text; END")
            };
            create_and_check(&env, p, &out).await;
        } else {
            eprintln!("PG {major}: procedure OUT atlandı (PG 14+)");
        }
    } else {
        eprintln!("PG {major}: procedure atlandı (PG 11+)");
        // Sürüm kapısı yerel hatadır (sunucuya gitmez).
        let e = build::create(&wrap(p, &base), &env.ctx(false)).unwrap_err();
        assert!(paths(&e).contains(&"kind"), "{e:?}");
    }
}

// ================================================================================================================
// Gövde baytı baytına
// ================================================================================================================

docker_test!(routine_body_round_trips_byte_for_byte, body_body, "fbody");

async fn body_body(ts: TestServer) {
    let env = setup_in(&ts, PFX, "fbody").await;
    env.sql("CREATE SCHEMA fb").await;
    let f = ObjectKind::Function;
    let bodies = [
        "SELECT 1\r\n-- satır sonu CRLF\r\n",
        "SELECT '$pgt$ $pgt1$ $$ $_$'::text",
        "SELECT 'ğüş 😀 ; -- /* x */ '' \\ '::text",
        "\n\n  SELECT 'tabs\tand\nnewlines'::text  \n\n",
        "SELECT 'x $pgt'::text -- sonu etiket önekiyle biten gövde $pgt",
    ];
    for (i, b) in bodies.iter().enumerate() {
        let d = func("fb", &format!("b{i}"), "sql", ty("text"), b);
        let d = if i == 0 { FunctionDef { returns: Some(ty("integer")), ..d } } else { d };
        let (_, l) = create_and_check(&env, f, &d).await;
        assert_eq!(l.body.as_bytes(), b.as_bytes(), "gövde baytı baytına ({i})");
    }
    // plpgsql CRLF gövde, OR REPLACE ile başka bir CRLF/etiketli gövdeye.
    let d = func("fb", "pl", "plpgsql", ty("text"), "BEGIN\r\n  RETURN 'a';\r\nEND");
    let (oid, old) = create_and_check(&env, f, &d).await;
    let new = FunctionDef { body: "BEGIN\r\n  RETURN '$pgt$ ğ $$';\r\nEND\r\n".into(), ..old.clone() };
    let (oid2, l, plan) = alter_and_check(&env, f, &old, &new).await;
    assert_eq!((oid2, plan.statements.len()), (oid, 1));
    assert!(plan.statements[0].sql.starts_with("CREATE OR REPLACE FUNCTION fb.pl()"), "{:?}", sqls(&plan));
    assert_eq!(l.body.as_bytes(), new.body.as_bytes());
    let got: String = env.admin.query_one("SELECT fb.pl()", &[]).await.unwrap().get(0);
    assert_eq!(got, "$pgt$ ğ $$");
}

// ================================================================================================================
// OR REPLACE: yetkiler korunur
// ================================================================================================================

docker_test!(routine_replace_keeps_grants, grant_body, "fgrant");

async fn grant_body(ts: TestServer) {
    let env = setup_in(&ts, PFX, "fgrant").await;
    env.sql("CREATE SCHEMA fg").await;
    let f = ObjectKind::Function;
    let d = FunctionDef {
        args: vec![a_in("a", "integer"), arg(ArgMode::In, Some("b"), "integer", Some("1"))],
        ..func("fg", "g", "sql", ty("integer"), "SELECT a")
    };
    let (oid, old) = create_and_check(&env, f, &d).await;
    env.sql(&format!("REVOKE EXECUTE ON FUNCTION fg.g(integer, integer) FROM PUBLIC; GRANT EXECUTE ON FUNCTION fg.g(integer, integer) TO {}", env.role)).await;
    assert!(can_execute(&env, &env.role, oid).await);
    // Gövde + öznitelik + ayar + varsayılan değişimi + yeni varsayılanlı argüman YOK (imza aynı) → tek OR REPLACE.
    let new = FunctionDef {
        body: "SELECT a + b".into(),
        volatility: Volatility::Stable,
        strict: true,
        security_definer: true,
        parallel: Parallel::Restricted,
        cost: Some("7".into()),
        settings: vec![set("work_mem", "2MB")],
        args: vec![a_in("a", "integer"), arg(ArgMode::In, Some("b"), "integer", Some("5"))],
        ..old.clone()
    };
    let (oid2, l, plan) = alter_and_check(&env, f, &old, &new).await;
    assert_eq!(plan.statements.len(), 1, "{:?}", sqls(&plan));
    assert!(plan.statements[0].sql.starts_with("CREATE OR REPLACE FUNCTION fg.g("));
    assert_eq!(plan.statements[0].risk, Risk::Safe);
    assert!(!plan.warnings.is_empty(), "SECURITY DEFINER uyarısı");
    assert_eq!(oid2, oid, "OR REPLACE aynı nesne");
    assert!(can_execute(&env, &env.role, oid).await, "GRANT korunmalı");
    assert!(!acl_is_default(&env, oid).await, "PUBLIC yetkisi geri gelmemeli");
    let v: i32 = env.admin.query_one("SELECT fg.g(1)", &[]).await.unwrap().get(0);
    assert_eq!(v, 6);
    // Varsayılan EKLEME OR REPLACE ile yapılır (kaldırma yeniden kurma ister — bkz. recreate testi).
    let new2 = FunctionDef { args: vec![arg(ArgMode::In, Some("a"), "integer", Some("0")), l.args[1].clone()], ..l.clone() };
    let (_, l2, plan) = alter_and_check(&env, f, &l, &new2).await;
    assert_eq!(plan.statements.len(), 1);
    assert!(can_execute(&env, &env.role, oid).await);
    // Ayar kaldırma + COST varsayılana dönüş (yüklü metin yazılır).
    let new3 = FunctionDef { settings: vec![], cost: Some("100".into()), ..l2.clone() };
    alter_and_check(&env, f, &l2, &new3).await;
    assert!(can_execute(&env, &env.role, oid).await);
}

// ================================================================================================================
// Bilinen sınır: sıradan '…' içinde '\' olan yüklü varsayılan
// ================================================================================================================

docker_test!(routine_backslash_default_blocks_definition_edits, backslash_body, "fbslash");

/// `pg_get_function_arg_default` `E'x\\y'` varsayılanını `'x\y'::text` olarak verir (standard_conforming_strings
/// açık); serbest SQL kuralı sıradan dizgede `\`'yi reddeder ve `OR REPLACE` tüm varsayılanları yeniden yazdığı için
/// gövde/öznitelik düzenlemesi alan hatası alır. Ad/şema/sahip/yorum düzenlemeleri etkilenmez. (Belgelenmiş sınır.)
async fn backslash_body(ts: TestServer) {
    let env = setup_in(&ts, PFX, "fbslash").await;
    env.sql(r"CREATE SCHEMA bs; CREATE FUNCTION bs.f(a text DEFAULT E'x\\y') RETURNS text LANGUAGE sql AS 'SELECT a'").await;
    let f = ObjectKind::Function;
    let oid = env.proc_oid("bs", "f", "text").await;
    let l = load_oid(&env, f, oid).await;
    assert_eq!(l.args[0].default.as_deref(), Some(r"'x\y'::text"));
    assert!(l.editable && l.unsupported.is_empty());
    assert_noop(&env, f, &l);
    // Yorum düzenlemesi çalışır.
    let new = FunctionDef { comment: Some("c".into()), ..l.clone() };
    let (_, l, _) = alter_and_check(&env, f, &l, &new).await;
    // Gövde düzenlemesi PG'ye gitmeden varsayılan alanında reddedilir.
    let new = FunctionDef { body: "SELECT a || a".into(), ..l.clone() };
    let e = build::alter(&wrap(f, &l), &wrap(f, &new), &env.ctx(false)).unwrap_err();
    assert_eq!(paths(&e), ["args[0].default"], "{e:?}");
    // `E'…'` biçimine çevrilen varsayılanla düzenleme yapılabilir (kullanıcı yolu).
    let new = FunctionDef { args: vec![arg(ArgMode::In, Some("a"), "text", Some(r"E'x\\y'"))], ..new };
    let plan = falter(&env, f, &l, &new);
    env.exec_tx(&plan).await;
    let l2 = load_oid(&env, f, oid).await;
    assert_eq!((l2.body.as_str(), l2.args[0].default.as_deref()), ("SELECT a || a", Some(r"'x\y'::text")));
}

// ================================================================================================================
// Ad + şema + sahip + yorum birlikte
// ================================================================================================================

docker_test!(routine_rename_move_owner_comment_together, move_body, "fmove");

async fn move_body(ts: TestServer) {
    let env = setup_in(&ts, PFX, "fmove").await;
    env.sql(r#"CREATE SCHEMA fm; CREATE SCHEMA "Fm 2""#).await;
    let f = ObjectKind::Function;
    let d = FunctionDef { args: vec![a_in("a", "integer")], comment: Some("eski".into()), ..func("fm", "m", "sql", ty("integer"), "SELECT a") };
    let (oid, old) = create_and_check(&env, f, &d).await;
    let new = FunctionDef {
        schema: "Fm 2".into(),
        name: "Yeni ad".into(),
        owner: Some(env.role.clone()),
        comment: Some("yeni".into()),
        body: "SELECT a * 2".into(),
        ..old.clone()
    };
    let (oid2, moved, plan) = alter_and_check(&env, f, &old, &new).await;
    assert_eq!(oid2, oid);
    let prefixes = [
        "CREATE OR REPLACE FUNCTION fm.m(",
        "ALTER FUNCTION fm.m(integer) RENAME TO \"Yeni ad\"",
        "ALTER FUNCTION fm.\"Yeni ad\"(integer) SET SCHEMA \"Fm 2\"",
        "COMMENT ON FUNCTION \"Fm 2\".\"Yeni ad\"(integer) IS 'yeni'",
        &format!("ALTER FUNCTION \"Fm 2\".\"Yeni ad\"(integer) OWNER TO {}", env.role),
    ];
    assert_eq!(plan.statements.len(), prefixes.len(), "{:?}", sqls(&plan));
    for (s, p) in plan.statements.iter().zip(prefixes) {
        assert!(s.sql.starts_with(p), "{} ↛ {p}", s.sql);
    }
    // Yorum kaldırma (IS NULL).
    let (_, l, plan) = alter_and_check(&env, f, &moved, &FunctionDef { comment: None, ..moved.clone() }).await;
    assert!(l.comment.is_none());
    assert_eq!(sqls(&plan), ["COMMENT ON FUNCTION \"Fm 2\".\"Yeni ad\"(integer) IS NULL"]);

    // Procedure (PG 11+): aynı sıra, PROCEDURE ve açık modlu kimlikle (PG 14+'da OUT dahil).
    if env.version >= 110000 {
        let p = ObjectKind::Procedure;
        let mut args = vec![a_in("a", "integer"), arg(ArgMode::Inout, Some("b"), "text", None)];
        if env.version >= 140000 {
            args.push(arg(ArgMode::Out, Some("c"), "integer", None));
        }
        let d = FunctionDef { args, ..proc_def("fm", "pm", "BEGIN NULL; END") };
        let (oid, old) = create_and_check(&env, p, &d).await;
        let new = FunctionDef {
            schema: "Fm 2".into(),
            name: "pm2".into(),
            owner: Some(env.role.clone()),
            comment: Some("pc".into()),
            body: "BEGIN b := b || 'x'; END".into(),
            ..old.clone()
        };
        let (oid2, _, plan) = alter_and_check(&env, p, &old, &new).await;
        assert_eq!(oid2, oid);
        assert_eq!(plan.statements.len(), 5, "{:?}", sqls(&plan));
        assert!(plan.statements[1].sql.starts_with("ALTER PROCEDURE fm.pm(IN integer, INOUT text"), "{}", plan.statements[1].sql);
    }
}

// ================================================================================================================
// OR REPLACE sınırları: yerel ret, yeniden kurma, bağımlı nesne
// ================================================================================================================

docker_test!(routine_recreate_rules, recreate_body, "frecreate");

async fn recreate_body(ts: TestServer) {
    let env = setup_in(&ts, PFX, "frecreate").await;
    env.sql("CREATE SCHEMA fr").await;
    let f = ObjectKind::Function;
    let d = FunctionDef {
        owner: Some(env.role.clone()),
        comment: Some("c".into()),
        args: vec![a_in("a", "integer"), arg(ArgMode::In, Some("b"), "text", Some("'x'::text")), arg(ArgMode::Out, Some("o"), "integer", None)],
        ..func("fr", "r", "sql", ty("integer"), "SELECT a")
    };
    let (oid, old) = create_and_check(&env, f, &d).await;
    env.sql(&format!("GRANT EXECUTE ON FUNCTION fr.r(integer, text) TO {}", env.role)).await;
    assert!(!acl_is_default(&env, oid).await);

    // `recreate` olmadan OR REPLACE'in yapamadığı değişiklikler PG'ye gitmeden alan hatası.
    let with_args = |args: Vec<FunctionArg>| FunctionDef { args, ..old.clone() };
    let cases: Vec<(FunctionDef, &str)> = vec![
        (with_args(vec![a_in("a", "bigint"), old.args[1].clone(), old.args[2].clone()]), "args[0].typeName"),
        (with_args(vec![a_in("z", "integer"), old.args[1].clone(), old.args[2].clone()]), "args[0].name"),
        (with_args(vec![old.args[0].clone(), arg(ArgMode::In, Some("b"), "text", None), old.args[2].clone()]), "args[1].default"),
        (with_args(vec![old.args[0].clone(), old.args[1].clone(), arg(ArgMode::Out, Some("o2"), "integer", None)]), "args"),
        (with_args(vec![old.args[0].clone(), old.args[1].clone()]), "args"),
    ];
    for (new, want) in &cases {
        let e = build::alter(&wrap(f, &old), &wrap(f, new), &env.ctx(false)).unwrap_err();
        assert!(paths(&e).contains(want), "{want}: {e:?}");
    }
    let ret = FunctionDef { args: vec![old.args[0].clone(), old.args[1].clone(), arg(ArgMode::Out, Some("o"), "bigint", None)], returns: Some(ty("bigint")), ..old.clone() };
    let e = build::alter(&wrap(f, &old), &wrap(f, &ret), &env.ctx(false)).unwrap_err();
    assert!(paths(&e).contains(&"returns") || paths(&e).contains(&"args"), "{e:?}");
    assert_eq!(load_oid(&env, f, oid).await, old, "hiçbir şey değişmemeli");

    // `recreate`: yeni imza (bigint), DROP (DataLoss) + CREATE + COMMENT + OWNER; GRANT'lar kaybolur.
    let new = FunctionDef {
        args: vec![a_in("a", "bigint"), arg(ArgMode::In, Some("b"), "text", None)],
        returns: Some(ty("bigint")),
        recreate: true,
        ..old.clone()
    };
    let (oid2, l, plan) = alter_and_check(&env, f, &old, &new).await;
    assert_eq!(plan.statements[0].sql, "DROP FUNCTION fr.r(integer, text)");
    assert_eq!(plan.statements[0].risk, Risk::DataLoss);
    assert_eq!(plan.statements.len(), 4, "{:?}", sqls(&plan));
    assert_ne!(oid2, oid);
    assert!(!env.proc_exists("fr", "r", "integer, text").await, "eski imza silinmeli");
    assert!(acl_is_default(&env, oid2).await, "yeniden kurma yetkileri sıfırlar");
    assert_eq!((l.owner.as_deref(), l.comment.as_deref()), (Some(env.role.as_str()), Some("c")));

    // Bağımlı görünüm varken yeniden kurma → 2BP01, hiçbir şey değişmez.
    env.sql("CREATE VIEW fr.v AS SELECT fr.r(1::bigint, 'q') AS x").await;
    let new2 = FunctionDef { args: vec![a_in("a", "numeric"), l.args[1].clone()], returns: Some(ty("numeric")), recreate: true, ..l.clone() };
    let plan = falter(&env, f, &l, &new2);
    let e = env.try_exec_tx(&plan).await.unwrap_err();
    assert_eq!(sqlstate(&e), "2BP01", "{e}");
    assert_eq!(load_oid(&env, f, oid2).await, l, "hiçbir şey değişmemeli");

    // Bağımlı trigger varken trigger fonksiyonunu yeniden kurma → 2BP01.
    let tf = func("fr", "tf", "plpgsql", FunctionReturn::Trigger, "BEGIN RETURN NEW; END");
    let (toid, tl) = create_and_check(&env, f, &tf).await;
    env.sql("CREATE TABLE fr.t (a int); CREATE TRIGGER tg BEFORE INSERT ON fr.t FOR EACH ROW EXECUTE PROCEDURE fr.tf()").await;
    let tnew = FunctionDef { returns: Some(FunctionReturn::Void), body: "BEGIN NULL; END".into(), recreate: true, ..tl.clone() };
    let e = env.try_exec_tx(&falter(&env, f, &tl, &tnew)).await.unwrap_err();
    assert_eq!(sqlstate(&e), "2BP01", "{e}");
    assert_eq!(load_oid(&env, f, toid).await, tl);
    let n: i64 = env.admin.query_one("SELECT count(*) FROM pg_trigger WHERE tgname = 'tg'", &[]).await.unwrap().get(0);
    assert_eq!(n, 1);
}

// ================================================================================================================
// Aşırı yükleme: doğru nesne
// ================================================================================================================

docker_test!(routine_overloads_change_the_right_one, overload_body, "foverload");

async fn overload_body(ts: TestServer) {
    let env = setup_in(&ts, PFX, "foverload").await;
    env.sql("CREATE SCHEMA fo").await;
    let f = ObjectKind::Function;
    let (oi, li) = create_and_check(&env, f, &FunctionDef { args: vec![a_in("a", "integer")], ..func("fo", "o", "sql", ty("text"), "SELECT 'int'::text") }).await;
    let (ot, lt) = create_and_check(&env, f, &FunctionDef { args: vec![a_in("a", "text")], ..func("fo", "o", "sql", ty("text"), "SELECT 'text'::text") }).await;
    // text sürümünün gövdesi + yorumu + adı değişir; integer sürümü aynı kalır.
    let new = FunctionDef { body: "SELECT 'text2'::text".into(), comment: Some("t".into()), name: "o2".into(), ..lt.clone() };
    let (ot2, _, _) = alter_and_check(&env, f, &lt, &new).await;
    assert_eq!(ot2, ot);
    assert_eq!(load_oid(&env, f, oi).await, li, "integer sürümü değişmemeli");
    // integer sürümünü sil: text sürümü (o2) kalır.
    env.exec_tx(&build::drop(&wrap(f, &li), false, &env.ctx(false)).unwrap()).await;
    assert!(!env.proc_exists("fo", "o", "integer").await && env.proc_exists("fo", "o2", "text").await);

    // PG 14+: `q(int, text)` ve `q(int, OUT text)` procedure'leri; modsuz kimlik belirsiz olurdu (açık mod düzeltmesi).
    if env.version >= 140000 {
        let p = ObjectKind::Procedure;
        let (qi, li) = create_and_check(&env, p, &FunctionDef { args: vec![a_in("a", "integer"), a_in("b", "text")], ..proc_def("fo", "q", "BEGIN NULL; END") }).await;
        let out = FunctionDef { args: vec![a_in("a", "integer"), arg(ArgMode::Out, Some("b"), "text", None)], ..proc_def("fo", "q", "BEGIN b := 'o'; END") };
        // create_and_check sabit noktası adı değiştirir; OUT sürümü ayrı imzayla (`q(integer)`) bulunur.
        let (qo, lo) = create_and_check(&env, p, &out).await;
        assert_ne!(qi, qo);
        let (_, li2, plan) = alter_and_check(&env, p, &li, &FunctionDef { comment: Some("in".into()), ..li.clone() }).await;
        assert_eq!(sqls(&plan), ["COMMENT ON PROCEDURE fo.q(IN integer, IN text) IS 'in'"]);
        assert_eq!(load_oid(&env, p, qo).await, lo, "OUT sürümü değişmemeli");
        let plan = falter(&env, p, &lo, &FunctionDef { comment: Some("out".into()), body: "BEGIN b := 'o2'; END".into(), ..lo.clone() });
        env.exec_tx(&plan).await;
        assert_eq!(load_oid(&env, p, qo).await.comment.as_deref(), Some("out"));
        assert_eq!(load_oid(&env, p, qi).await, li2, "IN sürümü değişmemeli");
        env.exec_tx(&build::drop(&wrap(p, &li2), false, &env.ctx(false)).unwrap()).await;
        assert!(try_load(&env, p, qo).await.is_ok());
        eprintln!("PG {}: procedure açık modlu kimlik aşırı yüklemede doğru nesneyi seçti", env.version / 10000);
    }
}

// ================================================================================================================
// Salt okunur nesneler
// ================================================================================================================

docker_test!(routine_unsupported_objects_are_read_only, readonly_body, "freadonly");

/// Salt okunur tanım: `editable = false`, yorum değişimi ve drop `object` hatası; `alter(l, l)` boş plan. Sistem
/// şemasındaki nesnede eski taraf kimlik denetimi (`old.schema`) her alter'ı zaten reddeder: noop da hata.
fn assert_read_only(env: &Env, kind: ObjectKind, l: &FunctionDef) {
    assert!(!l.editable && !l.unsupported.is_empty(), "{}: {:?}", l.name, l.unsupported);
    let system = l.unsupported.iter().any(|u| u == "sistem şeması");
    let new = FunctionDef { comment: Some("x".into()), ..l.clone() };
    let e = build::alter(&wrap(kind, l), &wrap(kind, &new), &env.ctx(false)).unwrap_err();
    if system {
        assert_eq!(paths(&e), ["old.schema"], "{}", l.name);
        let e = build::alter(&wrap(kind, l), &wrap(kind, l), &env.ctx(false)).unwrap_err();
        assert_eq!(paths(&e), ["old.schema"], "{}", l.name);
    } else {
        assert_eq!(paths(&e), ["object"], "{}", l.name);
        assert_noop(env, kind, l);
    }
    let e = build::drop(&wrap(kind, l), false, &env.ctx(false)).unwrap_err();
    assert!(!e.is_empty(), "{}", l.name);
    if !system {
        assert_eq!(paths(&e), ["object"], "{}", l.name);
    }
}

async fn readonly_body(ts: TestServer) {
    let env = setup_in(&ts, PFX, "freadonly").await;
    let major = env.version / 10000;
    let f = ObjectKind::Function;
    // Sistem şeması (iç/C dili, eklenti, pencere birlikte).
    let system: [(&str, &str, &[&str]); 3] = [
        ("lower", "text", &["LANGUAGE internal", "sistem şeması"]),
        ("plpgsql_call_handler", "", &["LANGUAGE c", "EXTENSION plpgsql (eklentiye ait)", "sistem şeması"]),
        ("row_number", "", &["LANGUAGE internal", "WINDOW", "sistem şeması"]),
    ];
    for (name, args, want) in system {
        let oid = env.proc_oid("pg_catalog", name, args).await;
        let l = load_oid(&env, f, oid).await;
        for w in want {
            assert!(l.unsupported.iter().any(|u| u.as_str() == *w), "{name}: {w} yok: {:?}", l.unsupported);
        }
        assert_read_only(&env, f, &l);
    }

    env.sql("CREATE SCHEMA ro").await;
    let mut cases: Vec<(&str, &str, &str, Vec<&str>)> = vec![
        (
            "CREATE FUNCTION ro.i(integer) RETURNS integer LANGUAGE internal IMMUTABLE STRICT AS 'int4abs'",
            "i",
            "integer",
            vec!["LANGUAGE internal"],
        ),
        (
            "CREATE FUNCTION ro.c() RETURNS language_handler LANGUAGE c AS '$libdir/plpgsql', 'plpgsql_call_handler'",
            "c",
            "",
            vec!["LANGUAGE c"],
        ),
        ("CREATE AGGREGATE ro.agg(integer) (sfunc = int4pl, stype = integer)", "agg", "integer", vec!["LANGUAGE internal", "AGGREGATE"]),
        (
            "CREATE FUNCTION ro.e() RETURNS integer LANGUAGE sql AS 'SELECT 1'; ALTER EXTENSION plpgsql ADD FUNCTION ro.e()",
            "e",
            "",
            vec!["EXTENSION plpgsql (eklentiye ait)"],
        ),
        (
            "CREATE FUNCTION ro.fs(internal) RETURNS internal LANGUAGE internal IMMUTABLE STRICT AS 'int4in'; \
             CREATE FUNCTION ro.ts(internal) RETURNS integer LANGUAGE internal IMMUTABLE STRICT AS 'int4in'; \
             CREATE TRANSFORM FOR integer LANGUAGE plpgsql (FROM SQL WITH FUNCTION ro.fs(internal), TO SQL WITH FUNCTION ro.ts(internal)); \
             CREATE FUNCTION ro.tr(a integer) RETURNS integer LANGUAGE plpgsql TRANSFORM FOR TYPE integer AS 'BEGIN RETURN a; END'",
            "tr",
            "integer",
            vec!["TRANSFORM FOR TYPE"],
        ),
    ];
    if env.version >= 120000 {
        cases.push((
            "CREATE FUNCTION ro.sup(a text) RETURNS boolean LANGUAGE sql SUPPORT textlike_support AS 'SELECT true'",
            "sup",
            "text",
            vec!["SUPPORT textlike_support"],
        ));
    } else {
        eprintln!("PG {major}: SUPPORT atlandı (PG 12+)");
    }
    if env.version >= 140000 {
        cases.push((
            "CREATE FUNCTION ro.atom(a integer) RETURNS integer LANGUAGE sql BEGIN ATOMIC SELECT a + 1; END",
            "atom",
            "integer",
            vec!["BEGIN ATOMIC (SQL standart gövde)"],
        ));
    } else {
        eprintln!("PG {major}: BEGIN ATOMIC atlandı (PG 14+)");
    }
    for (sql, name, args, want) in &cases {
        env.sql(sql).await;
        let oid = env.proc_oid("ro", name, args).await;
        let l = load_oid(&env, f, oid).await;
        assert_eq!(&l.unsupported, want, "{name}");
        assert_read_only(&env, f, &l);
    }

    // Tür uyuşmazlığı ve bulunamayan nesne.
    let foid = env.proc_oid("ro", "e", "").await;
    assert!(matches!(try_load(&env, ObjectKind::Procedure, foid).await, Err(Error::Invalid(_))));
    if env.version >= 110000 {
        env.sql("CREATE PROCEDURE ro.p() LANGUAGE sql AS 'SELECT 1'").await;
        let poid = env.proc_oid("ro", "p", "").await;
        assert!(matches!(try_load(&env, ObjectKind::Function, poid).await, Err(Error::Invalid(_))));
        assert!(load_oid(&env, ObjectKind::Procedure, poid).await.editable);
    }
    assert!(matches!(try_load(&env, f, 4_000_000_000).await, Err(Error::NotFound(_))));
}
