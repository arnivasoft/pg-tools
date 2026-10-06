//! Tablo üreticisi → gerçek sunucu → tablo yükleyicisi gidiş-dönüş testleri (Docker; `PGT_TEST_URLS` yoksa atlanır).
//!
//! Her test kendi geçici veritabanını (`pgt_ddl_tb_<ana sürüm>_<etiket>`) ve rolünü kurar; başarısız doğrulamadan sonra
//! bile temizlenir. Üreticinin SQL'i `DdlManager` olmadan, uygulayıcı gibi tek transaction'da `admin_client` ile
//! çalıştırılır. Sürüme bağlı kurallar `server_version`'a göre dallanır; atlanan kısım açıkça yazdırılır.

mod common;

use std::collections::HashSet;

use common::{Env, cleanup_in, guarded, servers, setup_in};
use pgt_core::Error;
use pgt_core::testing::TestServer;
use pgt_ddl::build;
use pgt_ddl::build::type_canon::canonical_type;
use pgt_ddl::build::type_syntax::validate_type;
use pgt_ddl::*;

const PFX: &str = "pgt_ddl_tb";

// ---- tanım kurucuları ----

fn c(name: &str, ty: &str) -> ColumnDef {
    ColumnDef {
        attnum: None,
        name: name.into(),
        data_type: ty.into(),
        not_null: false,
        default: None,
        identity: None,
        generated: None,
        collation: None,
        comment: None,
        using: None,
    }
}

fn nn(col: ColumnDef) -> ColumnDef {
    ColumnDef { not_null: true, ..col }
}

fn dflt(col: ColumnDef, d: &str) -> ColumnDef {
    ColumnDef { default: Some(d.into()), ..col }
}

fn ident(col: ColumnDef, k: IdentityKind) -> ColumnDef {
    ColumnDef { identity: Some(k), not_null: true, ..col }
}

fn gen_col(col: ColumnDef, expr: &str, stored: bool) -> ColumnDef {
    ColumnDef { generated: Some(GeneratedDef { expr: expr.into(), stored }), ..col }
}

fn s(v: &[&str]) -> Vec<String> {
    v.iter().map(|x| x.to_string()).collect()
}

fn con(name: &str, kind: ConstraintKind) -> ConstraintDef {
    ConstraintDef { oid: None, name: Some(name.into()), kind, deferrable: false, initially_deferred: false, not_valid: false }
}

fn unnamed(kind: ConstraintKind) -> ConstraintDef {
    ConstraintDef { name: None, ..con("x", kind) }
}

fn pk(cols: &[&str]) -> ConstraintKind {
    ConstraintKind::PrimaryKey { columns: s(cols), include: vec![] }
}

fn uq(cols: &[&str]) -> ConstraintKind {
    ConstraintKind::Unique { columns: s(cols), include: vec![], nulls_not_distinct: false }
}

fn ck(expr: &str) -> ConstraintKind {
    ConstraintKind::Check { expr: expr.into(), no_inherit: false }
}

fn fk(cols: &[&str], schema: &str, table: &str, refs: &[&str]) -> ConstraintKind {
    ConstraintKind::ForeignKey {
        columns: s(cols),
        ref_schema: schema.into(),
        ref_table: table.into(),
        ref_columns: s(refs),
        match_type: FkMatch::Simple,
        on_update: FkAction::NoAction,
        on_delete: FkAction::NoAction,
    }
}

fn table(schema: &str, name: &str, columns: Vec<ColumnDef>, constraints: Vec<ConstraintDef>) -> TableDef {
    TableDef {
        schema: schema.into(),
        name: name.into(),
        owner: None,
        tablespace: None,
        persistence: Persistence::Permanent,
        columns,
        constraints,
        partition_by: None,
        fillfactor: None,
        comment: None,
        partition_text: None,
        is_partition: false,
        inherits: false,
        unsupported: vec![],
    }
}

fn parted(d: TableDef, strategy: PartitionStrategy, keys: Vec<PartitionKey>) -> TableDef {
    TableDef { partition_by: Some(PartitionByDef { strategy, keys }), ..d }
}

fn key(name: &str) -> PartitionKey {
    PartitionKey::Column { name: name.into() }
}

fn t(d: &TableDef) -> ObjectDef {
    ObjectDef::Table(d.clone())
}

fn with(d: &TableDef, f: impl FnOnce(&mut TableDef)) -> TableDef {
    let mut d = d.clone();
    f(&mut d);
    d
}

fn col_mut<'a>(d: &'a mut TableDef, name: &str) -> &'a mut ColumnDef {
    d.columns.iter_mut().find(|c| c.name == name).unwrap_or_else(|| panic!("kolon yok: {name}"))
}

fn con_mut<'a>(d: &'a mut TableDef, name: &str) -> &'a mut ConstraintDef {
    d.constraints.iter_mut().find(|c| c.name.as_deref() == Some(name)).unwrap_or_else(|| panic!("kısıt yok: {name}"))
}

fn sqls(p: &Plan) -> Vec<&str> {
    p.statements.iter().map(|s| s.sql.as_str()).collect()
}

// ---- yükle / karşılaştır ----

async fn tload(env: &Env, schema: &str, name: &str) -> TableDef {
    let oid = env.class_oid(schema, name).await;
    match load(&env.mgr, env.server, env.db_oid, ObjectRef { kind: ObjectKind::Table, oid }).await {
        Ok(ObjectDef::Table(d)) => d,
        other => panic!("{schema}.{name} yüklenemedi: {other:?}"),
    }
}

/// Beklenen tanımı yükleyicinin normalleştirmesine getirir: belirtilmeyen sahip yüklenenden; `pg_default` ve boş yorum
/// `None`; `partition_by` hiç yüklenmez (`partition_text` ayrıca doğrulanır); yeni kolon/kısıtın attnum/oid'i ve adsız
/// kısıtın adı yüklenenden; kanonik eşit tip metni `format_type` çıktısına; kısıtlar ad sırasıyla.
fn expected(def: &TableDef, loaded: &TableDef) -> TableDef {
    let mut e = def.clone();
    if e.owner.is_none() {
        e.owner = loaded.owner.clone();
    }
    if e.tablespace.as_deref() == Some("pg_default") {
        e.tablespace = None;
    }
    e.comment = e.comment.filter(|c| !c.is_empty());
    e.partition_by = None;
    e.partition_text = loaded.partition_text.clone();
    for col in &mut e.columns {
        let l = loaded.columns.iter().find(|l| l.name == col.name);
        if col.attnum.is_none() {
            col.attnum = l.and_then(|l| l.attnum);
        }
        if let Some(l) = l
            && canonical_type(&col.data_type) == canonical_type(&l.data_type)
        {
            col.data_type = l.data_type.clone();
        }
        col.comment = col.comment.take().filter(|x| !x.is_empty());
        col.using = None;
    }
    let mut used: HashSet<String> = e.constraints.iter().filter_map(|c| c.name.clone()).collect();
    for cn in &mut e.constraints {
        if cn.name.is_none()
            && let Some(l) = loaded.constraints.iter().find(|l| l.kind == cn.kind && !used.contains(l.name.as_deref().unwrap()))
        {
            cn.name = l.name.clone();
            used.insert(l.name.clone().unwrap());
        }
        // Yeni kısıtın ve DROP+ADD ile yeniden kurulan kısıtın oid'i sunucudan (adla) gelir.
        if !cn.oid.is_some_and(|o| loaded.constraints.iter().any(|l| l.oid == Some(o))) {
            cn.oid = loaded.constraints.iter().find(|l| l.name == cn.name).and_then(|l| l.oid);
        }
    }
    e.constraints.sort_by(|a, b| a.name.cmp(&b.name).then(a.oid.cmp(&b.oid)));
    e
}

/// İki tanımın farklı alanlarını okunur biçimde döker (uzun `assert_eq` çıktısı yerine).
fn diff(want: &TableDef, got: &TableDef) -> String {
    let mut out = vec![];
    let (mut w, mut g) = (want.clone(), got.clone());
    let n = w.columns.len().max(g.columns.len());
    for i in 0..n {
        let (a, b) = (w.columns.get(i), g.columns.get(i));
        if a != b {
            out.push(format!("columns[{i}]:
    beklenen {a:?}
    yüklenen {b:?}"));
        }
    }
    let n = w.constraints.len().max(g.constraints.len());
    for i in 0..n {
        let (a, b) = (w.constraints.get(i), g.constraints.get(i));
        if a != b {
            out.push(format!("constraints[{i}]:
    beklenen {a:?}
    yüklenen {b:?}"));
        }
    }
    w.columns.clear();
    g.columns.clear();
    w.constraints.clear();
    g.constraints.clear();
    if w != g {
        out.push(format!("tablo:
    beklenen {w:?}
    yüklenen {g:?}"));
    }
    out.join("
  ")
}

fn assert_same(want: &TableDef, got: &TableDef, what: &str) {
    assert!(want == got, "{what}
  {}", diff(want, got));
}

/// Yüklenen tanım kendisiyle karşılaştırılınca boş plan (ve uyarısız) üretir.
fn assert_noop(env: &Env, d: &TableDef) {
    let p = build::alter(&t(d), &t(d), &env.ctx(false)).unwrap_or_else(|e| panic!("noop reddedildi: {e:?}\n{d:?}"));
    assert!(p.statements.is_empty() && p.warnings.is_empty(), "{}.{}: boş plan bekleniyordu: {:?} {:?}", d.schema, d.name, sqls(&p), p.warnings);
}

/// create → tek transaction → yükle → beklenen tanıma eşit + noop + iki yükleme aynı.
async fn create_and_check(env: &Env, def: &TableDef) -> TableDef {
    let plan = build::create(&t(def), &env.ctx(false)).unwrap_or_else(|e| panic!("create reddedildi: {} {e:?}", def.name));
    env.exec_tx(&plan).await;
    let loaded = tload(env, &def.schema, &def.name).await;
    assert_same(&expected(def, &loaded), &loaded, &format!("create → load ({})", def.name));
    assert_noop(env, &loaded);
    assert_eq!(tload(env, &def.schema, &def.name).await, loaded, "iki yükleme aynı olmalı");
    loaded
}

fn alter_plan(env: &Env, old: &TableDef, new: &TableDef) -> Plan {
    build::alter(&t(old), &t(new), &env.ctx(false)).unwrap_or_else(|e| panic!("alter reddedildi: {e:?}\n  old: {old:?}\n  new: {new:?}"))
}

fn alter_errs(env: &Env, old: &TableDef, new: &TableDef) -> Vec<FieldError> {
    match build::alter(&t(old), &t(new), &env.ctx(false)) {
        Ok(p) => panic!("hata bekleniyordu: {:?}", sqls(&p)),
        Err(e) => e,
    }
}

/// alter(old, new) → tek transaction → yükle → beklenen yeni tanıma eşit + noop; yüklenen tanımı döndürür.
async fn alter_and_check(env: &Env, old: &TableDef, new: &TableDef) -> TableDef {
    let plan = alter_plan(env, old, new);
    if let Err(e) = env.try_exec_tx(&plan).await {
        panic!("alter uygulanamadı: {e}\n  plan: {:?}", sqls(&plan));
    }
    let loaded = tload(env, &new.schema, &new.name).await;
    assert_same(&expected(new, &loaded), &loaded, &format!("alter → load; plan: {:?}", sqls(&plan)));
    assert_noop(env, &loaded);
    loaded
}

async fn exists(env: &Env, sql: &str) -> bool {
    env.admin.query_one(&format!("SELECT EXISTS ({sql})"), &[]).await.unwrap().get(0)
}

// ================================================================================================================
// create → load
// ================================================================================================================

#[tokio::test]
async fn create_then_load_equals_def() {
    for ts in servers() {
        guarded(create_body(ts.clone()), cleanup_in(&ts, PFX, "create")).await;
    }
}

async fn create_body(ts: TestServer) {
    let env = setup_in(&ts, PFX, "create").await;
    let v = env.version;
    let role = env.role.clone();
    let sch = "tb s1";
    env.sql(r#"CREATE SCHEMA "tb s1""#).await;

    let refd = table(
        sch,
        "Ref T",
        vec![nn(c("id", "bigint")), nn(c("code", "text"))],
        vec![con("Ref PK", pk(&["id"])), con("Ref code", uq(&["code"])), con("Ref pair", uq(&["id", "code"]))],
    );
    create_and_check(&env, &refd).await;

    let mut cols = vec![
        ident(c("id", "bigint"), IdentityKind::Always),
        ident(c("seq", "integer"), IdentityKind::ByDefault),
        ColumnDef { collation: Some("C".into()), comment: Some("ad; -- 'x' /* y */".into()), ..nn(c("caption", "text")) },
        dflt(nn(c("qty", "integer")), "0"),
        dflt(c("price", "numeric(10,2)"), "1.5"),
        dflt(c("created", "timestamp with time zone"), "now()"),
        dflt(c("label", "text"), "'x'::text"),
    ];
    if v >= 120000 {
        cols.push(gen_col(c("doubled", "integer"), "qty * 2", true));
    } else {
        eprintln!("PG {}: STORED üretilmiş kolon atlandı (PG 12+)", v / 10000);
    }
    if v >= 180000 {
        cols.push(gen_col(c("plus1", "integer"), "qty + 1", false));
    } else {
        eprintln!("PG {}: VIRTUAL üretilmiş kolon atlandı (PG 18+)", v / 10000);
    }
    cols.extend([
        c("ref_id", "bigint"),
        c("ref_code", "text"),
        c("r2", "bigint"),
        c("r3", "bigint"),
        c("code", "character varying(20)"),
        c("flags", "integer[]"),
        ColumnDef { comment: Some("not".into()), ..c("note", "text") },
        c("Mixed Col", "integer"),
    ]);
    let fk_full = |cols: &[&str], refs: &[&str], m, u, d| {
        let ConstraintKind::ForeignKey { columns, ref_schema, ref_table, ref_columns, .. } = fk(cols, sch, "Ref T", refs) else { unreachable!() };
        ConstraintKind::ForeignKey { columns, ref_schema, ref_table, ref_columns, match_type: m, on_update: u, on_delete: d }
    };
    let main = TableDef {
        owner: Some(role.clone()),
        comment: Some("Ana tablo; it's".into()),
        ..table(
            sch,
            "Main T",
            cols,
            vec![
                con("Main PK", ConstraintKind::PrimaryKey { columns: s(&["id"]), include: if v >= 110000 { s(&["qty"]) } else { vec![] } }),
                con("uq_code", ConstraintKind::Unique { columns: s(&["code"]), include: vec![], nulls_not_distinct: v >= 150000 }),
                ConstraintDef { deferrable: true, initially_deferred: true, ..con("uq_def", uq(&["seq", "caption"])) },
                con("ck_qty", ck("qty >= 0")),
                con("ck_ni", ConstraintKind::Check { expr: "caption <> ''::text".into(), no_inherit: true }),
                con("fk_ref", fk(&["ref_id"], sch, "Ref T", &["id"])),
                con("fk_code", fk_full(&["ref_code"], &["code"], FkMatch::Full, FkAction::Cascade, FkAction::SetNull)),
                ConstraintDef {
                    deferrable: true,
                    initially_deferred: true,
                    ..con("fk_r2", fk_full(&["r2"], &["id"], FkMatch::Simple, FkAction::SetDefault, FkAction::Restrict))
                },
                ConstraintDef { deferrable: true, ..con("fk_r3", fk_full(&["r3"], &["id"], FkMatch::Simple, FkAction::NoAction, FkAction::Cascade)) },
                con("fk_pair", fk_full(&["ref_id", "ref_code"], &["id", "code"], FkMatch::Full, FkAction::Restrict, FkAction::NoAction)),
                unnamed(ck("qty < 1000000")),
                unnamed(uq(&["Mixed Col"])),
            ],
        )
    };
    let l = create_and_check(&env, &main).await;
    assert!(l.unsupported.is_empty(), "{:?}", l.unsupported);
    assert_eq!(l.owner.as_deref(), Some(role.as_str()));

    // UNLOGGED + fillfactor + pg_default (→ None) + sahip + yorum.
    let u1 = TableDef {
        persistence: Persistence::Unlogged,
        fillfactor: Some(70),
        tablespace: Some("pg_default".into()),
        owner: Some(role.clone()),
        comment: Some("it's".into()),
        ..table(sch, "u1", vec![c("a", "integer")], vec![])
    };
    let l = create_and_check(&env, &u1).await;
    assert_eq!((l.tablespace.clone(), l.fillfactor, l.persistence), (None, Some(70), Persistence::Unlogged));

    // Boş yorumlar `None` yüklenir; adsız PK sunucu adını alır.
    let c0 = TableDef {
        comment: Some(String::new()),
        ..table(sch, "c0", vec![ColumnDef { comment: Some(String::new()), ..nn(c("a", "integer")) }], vec![unnamed(pk(&["a"]))])
    };
    let l = create_and_check(&env, &c0).await;
    assert_eq!(l.constraints[0].name.as_deref(), Some("c0_pkey"));
    assert_eq!((l.comment.clone(), l.columns[0].comment.clone()), (None, None));

    // Bölümlenmiş tablolar: partition_text dolu, partition_by yüklenmez. Kimlik kolonu bölümlenmiş tabloda tüm
    // sürümlerde kabul edilir (gerçek sunucu; PG 17 öncesi için yerel kapı GEREKMEZ).
    let mut pcases: Vec<(TableDef, &str)> = vec![];
    let pcols = || vec![ident(c("id", "bigint"), IdentityKind::ByDefault), nn(c("k", "integer")), c("created", "timestamp with time zone"), c("v", "text")];
    let pk_k = |name: &str| if v >= 110000 { vec![con(name, pk(&["k", "id"]))] } else { vec![] };
    pcases.push((parted(table(sch, "pr", pcols(), pk_k("pr_pk")), PartitionStrategy::Range, vec![key("k")]), "RANGE (k)"));
    pcases.push((parted(table(sch, "pl", pcols(), pk_k("pl_pk")), PartitionStrategy::List, vec![key("k")]), "LIST (k)"));
    if v >= 110000 {
        pcases.push((parted(table(sch, "ph", pcols(), pk_k("ph_pk")), PartitionStrategy::Hash, vec![key("k")]), "HASH (k)"));
    } else {
        eprintln!("PG 10: PARTITION BY HASH ve bölümlenmiş tabloda PK atlandı (PG 11+)");
    }
    pcases.push((
        parted(
            table(sch, "pe", pcols(), vec![]),
            PartitionStrategy::Range,
            vec![PartitionKey::Expression { sql: "lower(v)".into() }, key("k")],
        ),
        "RANGE (lower(v), k)",
    ));
    if v < 180000 {
        // PG 10–17 UNLOGGED bölümlenmiş tabloyu kabul eder (PG 18 reddeder: yerel kapı, bkz. partitioned_table_server_rules).
        pcases.push((
            TableDef { persistence: Persistence::Unlogged, ..parted(table(sch, "pu", pcols(), vec![]), PartitionStrategy::List, vec![key("k")]) },
            "LIST (k)",
        ));
    }
    for (d, text) in pcases {
        let l = create_and_check(&env, &d).await;
        assert_eq!(l.partition_text.as_deref(), Some(text), "{}", d.name);
        assert!(l.partition_by.is_none() && !l.is_partition && !l.inherits);
        assert_eq!(l.columns[0].identity, Some(IdentityKind::ByDefault));
    }
}

// ================================================================================================================
// alter → load
// ================================================================================================================

#[tokio::test]
async fn alter_then_load_equals_new() {
    for ts in servers() {
        guarded(alter_body(ts.clone()), cleanup_in(&ts, PFX, "alter")).await;
    }
}

async fn alter_body(ts: TestServer) {
    let env = setup_in(&ts, PFX, "alter").await;
    let v = env.version;
    let role = env.role.clone();
    env.sql(r#"CREATE SCHEMA "tb a1"; CREATE SCHEMA "tb a2""#).await;
    let sch = "tb a1";
    let refd = table(sch, "ref", vec![nn(c("id", "integer")), c("code", "text")], vec![con("ref_pk", pk(&["id"])), con("ref_code", uq(&["code"]))]);
    create_and_check(&env, &refd).await;
    let base = table(
        sch,
        "w",
        vec![
            nn(c("id", "integer")),
            c("name", "text"),
            dflt(c("qty", "integer"), "1"),
            c("price", "numeric(10,2)"),
            c("txt", "text"),
            c("drop me", "text"),
            c("r", "integer"),
            dflt(c("dd", "integer"), "7"),
        ],
        vec![con("w_pk", pk(&["id"])), con("w_ck", ck("qty > 0"))],
    );
    let mut cur = create_and_check(&env, &base).await;
    env.sql(r#"INSERT INTO "tb a1".ref VALUES (1, 'a'), (2, 'b'); INSERT INTO "tb a1".w (id, name, qty, txt, r) VALUES (1, 'n', 3, '42', 1)"#).await;

    // Her adım: eski = son yüklenen; yeni = düzenlenmiş kopya; uygulanır ve yeniden yüklenir.
    type Step = Box<dyn Fn(&mut TableDef) + Send + Sync>;
    let mut steps: Vec<(&str, Step)> = vec![
        ("ADD COLUMN nullable", Box::new(|d| d.columns.push(c("added", "text")))),
        ("ADD COLUMN NOT NULL + default", Box::new(|d| d.columns.push(dflt(nn(c("flag", "boolean")), "false")))),
        ("ADD COLUMN identity", Box::new(|d| d.columns.push(ident(c("serialish", "bigint"), IdentityKind::ByDefault)))),
        ("ADD COLUMN collation + comment", Box::new(|d| d.columns.push(ColumnDef { collation: Some("C".into()), comment: Some("c".into()), ..c("coll", "text") }))),
        ("DROP COLUMN", Box::new(|d| d.columns.retain(|c| c.name != "drop me"))),
        ("RENAME COLUMN", Box::new(|d| col_mut(d, "name").name = "title".into())),
        ("TYPE alias", Box::new(|d| col_mut(d, "qty").data_type = "int8".into())),
        ("TYPE USING", Box::new(|d| {
            let x = col_mut(d, "txt");
            x.data_type = "integer".into();
            x.using = Some("txt::integer".into());
        })),
        ("TYPE varchar", Box::new(|d| col_mut(d, "title").data_type = "varchar(50)".into())),
        ("TYPE numeric(p)", Box::new(|d| col_mut(d, "price").data_type = "numeric(12)".into())),
        ("COLLATE değişimi", Box::new(|d| col_mut(d, "coll").collation = Some("POSIX".into()))),
        ("COLLATE kaldır", Box::new(|d| col_mut(d, "coll").collation = None)),
        ("SET NOT NULL", Box::new(|d| col_mut(d, "title").not_null = true)),
        ("DROP NOT NULL", Box::new(|d| col_mut(d, "title").not_null = false)),
        ("SET DEFAULT", Box::new(|d| col_mut(d, "added").default = Some("'z'::text".into()))),
        ("change DEFAULT", Box::new(|d| col_mut(d, "added").default = Some("upper('q'::text)".into()))),
        ("DROP DEFAULT", Box::new(|d| col_mut(d, "added").default = None)),
        ("TYPE with default", Box::new(|d| col_mut(d, "dd").data_type = "numeric".into())),
        ("ADD IDENTITY", Box::new(|d| {
            let x = col_mut(d, "r");
            x.identity = Some(IdentityKind::Always);
            x.not_null = true;
        })),
        ("SET GENERATED", Box::new(|d| col_mut(d, "r").identity = Some(IdentityKind::ByDefault))),
        ("DROP IDENTITY", Box::new(|d| col_mut(d, "serialish").identity = None)),
        ("column comments", Box::new(|d| {
            col_mut(d, "title").comment = Some("başlık; it's".into());
            col_mut(d, "coll").comment = None;
        })),
        ("ADD UNIQUE", Box::new(|d| d.constraints.push(con("w_title", uq(&["title"]))))),
        ("ADD CHECK unnamed", Box::new(|d| d.constraints.push(unnamed(ck("id < 1000"))))),
        ("ADD FK", Box::new(|d| d.constraints.push(con("w_fk", ConstraintKind::ForeignKey {
            columns: s(&["r"]),
            ref_schema: "tb a1".into(),
            ref_table: "ref".into(),
            ref_columns: s(&["id"]),
            match_type: FkMatch::Full,
            on_update: FkAction::Cascade,
            on_delete: FkAction::SetNull,
        })))),
        ("ADD CHECK NOT VALID", Box::new(|d| d.constraints.push(ConstraintDef { not_valid: true, ..con("w_nv", ck("txt > 0")) }))),
        ("VALIDATE", Box::new(|d| con_mut(d, "w_nv").not_valid = false)),
        ("RENAME CONSTRAINT", Box::new(|d| con_mut(d, "w_title").name = Some("w_title_u".into()))),
        ("structural CHECK", Box::new(|d| con_mut(d, "w_ck").kind = ck("qty >= 0"))),
        ("structural FK", Box::new(|d| {
            if let ConstraintKind::ForeignKey { on_delete, .. } = &mut con_mut(d, "w_fk").kind {
                *on_delete = FkAction::Cascade;
            }
            con_mut(d, "w_fk").deferrable = true;
        })),
        ("DROP CONSTRAINT", Box::new(|d| d.constraints.retain(|c| c.name.as_deref() != Some("w_title_u")))),
        ("PK yapısal", Box::new(|d| con_mut(d, "w_pk").kind = pk(&["id", "r"]))),
        ("table rename", Box::new(|d| d.name = "w2".into())),
        ("SET SCHEMA", Box::new(|d| d.schema = "tb a2".into())),
        ("owner", Box::new({
            let role = role.clone();
            move |d| d.owner = Some(role.clone())
        })),
        ("SET UNLOGGED", Box::new(|d| d.persistence = Persistence::Unlogged)),
        ("SET LOGGED", Box::new(|d| d.persistence = Persistence::Permanent)),
        ("fillfactor", Box::new(|d| d.fillfactor = Some(60))),
        ("fillfactor RESET", Box::new(|d| d.fillfactor = None)),
        ("table comment", Box::new(|d| d.comment = Some("yorum".into()))),
        ("table comment kaldır", Box::new(|d| d.comment = None)),
        ("SET TABLESPACE pg_default", Box::new(|d| d.tablespace = Some("pg_default".into()))),
        ("DROP COLUMN in constraint (with constraint)", Box::new(|d| {
            d.columns.retain(|c| c.name != "txt");
            d.constraints.retain(|c| c.name.as_deref() != Some("w_nv"));
        })),
    ];
    if v >= 110000 {
        steps.push(("UNIQUE INCLUDE", Box::new(|d| d.constraints.push(con("w_inc", ConstraintKind::Unique { columns: s(&["id"]), include: s(&["title"]), nulls_not_distinct: false })))));
    }
    if v >= 120000 {
        steps.push(("ADD COLUMN generated stored", Box::new(|d| d.columns.push(gen_col(c("g2", "integer"), "id * 2", true)))));
    }
    if v >= 130000 {
        steps.push(("DROP EXPRESSION", Box::new(|d| col_mut(d, "g2").generated = None)));
    }
    if v >= 150000 {
        steps.push(("UNIQUE NULLS NOT DISTINCT", Box::new(|d| d.constraints.push(con("w_nnd", ConstraintKind::Unique { columns: s(&["coll"]), include: vec![], nulls_not_distinct: true })))));
    }
    if v >= 170000 {
        steps.push(("ADD generated + SET EXPRESSION hazırlık", Box::new(|d| d.columns.push(gen_col(c("g3", "integer"), "id + 1", true)))));
        steps.push(("SET EXPRESSION", Box::new(|d| col_mut(d, "g3").generated.as_mut().unwrap().expr = "id + 2".into())));
    }
    if v >= 180000 {
        steps.push(("ADD virtual", Box::new(|d| d.columns.push(gen_col(c("g4", "integer"), "id * 3", false)))));
    }
    eprintln!("PG {}: {} alter adımı (sürüme bağlı adımlar: INCLUDE 11+, STORED 12+, DROP EXPRESSION 13+, NULLS NOT DISTINCT 15+, SET EXPRESSION 17+, VIRTUAL 18+)", v / 10000, steps.len());
    for (label, f) in &steps {
        let new = with(&cur, |d| f(d));
        let plan = alter_plan(&env, &cur, &new);
        assert!(!plan.statements.is_empty(), "{label}: boş plan");
        cur = alter_and_check(&env, &cur, &new).await;
    }
    if v >= 180000 {
        // PG 18: CHECK'li tabloda sanal kolonun SET EXPRESSION'ı sunucuda desteklenmez → yerel hata (w_ck var).
        let e = alter_errs(&env, &cur, &with(&cur, |d| col_mut(d, "g4").generated.as_mut().unwrap().expr = "id * 4".into()));
        assert!(e[0].path.ends_with(".generated") && e[0].message.contains("CHECK"), "{e:?}");
    }
    assert_eq!((cur.schema.as_str(), cur.name.as_str()), ("tb a2", "w2"));
    let n: i64 = env.admin.query_one(r#"SELECT count(*) FROM "tb a2".w2"#, &[]).await.unwrap().get(0);
    assert_eq!(n, 1, "satır korunmalı");
}

// ================================================================================================================
// değiştirmeden düzenle → boş plan (kullanıcının yazdığı biçimde kurulan tablolar) + yeniden kurma sabit noktası
// ================================================================================================================

#[tokio::test]
async fn unchanged_edit_is_an_empty_plan() {
    for ts in servers() {
        guarded(noop_body(ts.clone()), cleanup_in(&ts, PFX, "noop")).await;
    }
}

async fn noop_body(ts: TestServer) {
    let env = setup_in(&ts, PFX, "noop").await;
    let v = env.version;
    env.sql(
        r#"CREATE SCHEMA app;
           CREATE TABLE app.parent (id int PRIMARY KEY, code text UNIQUE, a int, b int, UNIQUE (a, b));
           CREATE SEQUENCE app.s;
           CREATE TABLE app.kinds (
             id serial PRIMARY KEY,
             big bigserial,
             i int DEFAULT 1 + 2,
             t text DEFAULT 'x' NOT NULL,
             n numeric(10, 2) DEFAULT -1.5,
             ts timestamptz DEFAULT now(),
             ct timestamp DEFAULT CURRENT_TIMESTAMP,
             d date DEFAULT ('now'::text)::date,
             arr int[] DEFAULT '{}',
             nv bigint DEFAULT nextval('app.s'),
             js jsonb DEFAULT '{"a": [1, 2]}',
             low text DEFAULT lower('A') COLLATE "C",
             e text DEFAULT 'it''s; -- not a comment',
             ref int REFERENCES app.parent ON DELETE CASCADE,
             rc text REFERENCES app.parent(code) MATCH FULL ON UPDATE SET NULL DEFERRABLE INITIALLY DEFERRED,
             ra int, rb int,
             CHECK (i > 0 AND t <> ''),
             CONSTRAINT "Odd ""Name""" CHECK ((i > 0) OR (i < -5)),
             CONSTRAINT ck_in CHECK (t IN ('x', 'y', 'it''s')),
             CONSTRAINT ck_between CHECK (i BETWEEN 1 AND 10) NO INHERIT,
             CONSTRAINT ck_fn CHECK (length(t) < 100 AND ts IS NOT NULL OR js ? 'a'),
             FOREIGN KEY (ra, rb) REFERENCES app.parent (a, b) ON UPDATE RESTRICT ON DELETE SET DEFAULT,
             UNIQUE (big, t)
           );
           CREATE TABLE app.ids (a int GENERATED ALWAYS AS IDENTITY PRIMARY KEY, b smallint GENERATED BY DEFAULT AS IDENTITY);
           CREATE UNLOGGED TABLE app.ul (x int) WITH (fillfactor = 55);
           COMMENT ON TABLE app.ul IS 'yorum';
           COMMENT ON COLUMN app.ul.x IS 'kolon';
           CREATE TABLE app.selfref (id int PRIMARY KEY, parent int REFERENCES app.selfref);
           CREATE TABLE app.empty_t ();"#,
    )
    .await;
    let mut names = vec!["parent", "kinds", "ids", "ul", "selfref", "empty_t"];
    if v >= 110000 {
        env.sql("CREATE TABLE app.inc (a int, b int, c int, CONSTRAINT inc_pk PRIMARY KEY (a) INCLUDE (b, c), UNIQUE (b) INCLUDE (a))").await;
        names.push("inc");
    }
    if v >= 120000 {
        env.sql("CREATE TABLE app.gen (a int, b int GENERATED ALWAYS AS (a * 2 + 1) STORED, c text GENERATED ALWAYS AS (('n' || a::text)) STORED)").await;
        names.push("gen");
    }
    if v >= 150000 {
        env.sql("CREATE TABLE app.nnd (a int, b int, UNIQUE NULLS NOT DISTINCT (a, b))").await;
        names.push("nnd");
    }
    if v >= 180000 {
        env.sql("CREATE TABLE app.virt (a int, b int GENERATED ALWAYS AS (a + 1) VIRTUAL, c int GENERATED ALWAYS AS (a * 2))").await;
        names.push("virt");
    }
    eprintln!("PG {}: noop tabloları {:?}", v / 10000, names);
    for name in &names {
        let l = tload(&env, "app", name).await;
        assert_noop(&env, &l);
        assert!(l.unsupported.is_empty(), "{name}: {:?}", l.unsupported);
        for col in &l.columns {
            validate_type(&col.data_type).unwrap_or_else(|e| panic!("{name}.{}: {} dilbilgisinden geçmedi: {e}", col.name, col.data_type));
        }
        // Yeniden kurma sabit noktası: yüklenen tanım (oid/attnum'suz, yeni adla) kurulur, yeniden yüklenir; DEFAULT/
        // CHECK/üretilmiş/FK metinleri ve tipler aynı kalmalı (pg_get_expr normalleşmesi kararlı).
        let copy_name = format!("{name}_copy");
        let mut copy = with(&l, |d| {
            d.name = copy_name.clone();
            for col in &mut d.columns {
                col.attnum = None;
            }
            for cn in &mut d.constraints {
                cn.oid = None;
                cn.name = cn.name.as_ref().map(|n| format!("{n}_c"));
                // Kendine başvuran FK kopyaya başvurmalı.
                if let ConstraintKind::ForeignKey { ref_table, .. } = &mut cn.kind
                    && ref_table == name
                {
                    *ref_table = copy_name.clone();
                }
            }
        });
        if l.columns.is_empty() {
            env.sql(&format!("CREATE TABLE app.{copy_name} ()")).await;
            let r = tload(&env, "app", &copy_name).await;
            assert!(r.columns.is_empty());
            continue;
        }
        let plan = build::create(&t(&copy), &env.ctx(false)).unwrap_or_else(|e| panic!("{name}: yüklenen tanım yeniden kurulamadı: {e:?}"));
        env.exec_tx(&plan).await;
        let r = tload(&env, "app", &copy_name).await;
        copy.constraints.sort_by(|a, b| a.name.cmp(&b.name));
        let strip = |d: &TableDef| {
            let mut d = d.clone();
            d.columns.iter_mut().for_each(|c| c.attnum = None);
            d.constraints.iter_mut().for_each(|c| c.oid = None);
            d
        };
        assert_same(&strip(&copy), &strip(&r), &format!("{name}: yeniden kurma sabit noktası"));
    }
}

// ================================================================================================================
// kolon yeniden adlandırma kısıtları korur
// ================================================================================================================

#[tokio::test]
async fn column_rename_keeps_constraints() {
    for ts in servers() {
        guarded(rename_body(ts.clone()), cleanup_in(&ts, PFX, "rename")).await;
    }
}

async fn rename_body(ts: TestServer) {
    let env = setup_in(&ts, PFX, "rename").await;
    let v = env.version;
    let inc = if v >= 110000 { s(&["c"]) } else { vec![] };
    let d = table(
        "public",
        "r",
        vec![nn(c("id", "integer")), c("a", "integer"), c("b", "integer"), c("c", "integer"), c("parent", "integer"), c("o", "integer")],
        vec![
            con("r_pk", pk(&["id"])),
            con("r_ab", ConstraintKind::Unique { columns: s(&["a", "b"]), include: inc.clone(), nulls_not_distinct: false }),
            con("r_ck", ck("a > 0")),
            con("r_self", fk(&["parent"], "public", "r", &["id"])),
        ],
    );
    let l = create_and_check(&env, &d).await;
    env.sql("CREATE TABLE other (x int REFERENCES r (id), y int, z int, FOREIGN KEY (y, z) REFERENCES r (a, b))").await;
    let other_before = tload(&env, "public", "other").await;
    let oids: Vec<_> = l.constraints.iter().map(|c| (c.name.clone(), c.oid)).collect();

    // Kolonlar yeniden adlandırılır; kısıt kolon listeleri istemcideki gibi güncellenir (CHECK metni sunucuda izlenir).
    let renames = [("id", "id2"), ("a", "a2"), ("b", "b2"), ("c", "c2"), ("parent", "parent2")];
    let new = with(&l, |d| {
        for (from, to) in renames {
            col_mut(d, from).name = to.into();
        }
        let map = |v: &mut Vec<String>| v.iter_mut().for_each(|x| if let Some((_, to)) = renames.iter().find(|(f, _)| f == x) { *x = to.to_string() });
        for cn in &mut d.constraints {
            match &mut cn.kind {
                ConstraintKind::PrimaryKey { columns, include } | ConstraintKind::Unique { columns, include, .. } => {
                    map(columns);
                    map(include);
                }
                ConstraintKind::ForeignKey { columns, ref_columns, .. } => {
                    map(columns);
                    map(ref_columns);
                }
                ConstraintKind::Check { .. } => {}
            }
        }
    });
    let plan = alter_plan(&env, &l, &new);
    assert!(plan.statements.iter().all(|s| s.sql.contains("RENAME COLUMN")), "yalnızca RENAME COLUMN: {:?}", sqls(&plan));
    assert_eq!(plan.statements.len(), renames.len());
    env.exec_tx(&plan).await;
    let r = tload(&env, "public", "r").await;
    assert_eq!(r.constraints.iter().map(|c| (c.name.clone(), c.oid)).collect::<Vec<_>>(), oids, "kısıt oid'leri aynı kalmalı");
    // CHECK metnini sunucu yeni adla yazar; geri kalanı istemcinin yazdığıyla aynı.
    let mut want = expected(&new, &r);
    con_mut(&mut want, "r_ck").kind = ck("a2 > 0");
    assert_same(&want, &r, "rename");
    assert_noop(&env, &r);
    // Diğer tablonun FK'si de oid ile izlendi: hedef kolon adları güncellendi, kısıtlar aynı.
    let other = tload(&env, "public", "other").await;
    assert_eq!(other.constraints.iter().map(|c| c.oid).collect::<Vec<_>>(), other_before.constraints.iter().map(|c| c.oid).collect::<Vec<_>>());
    assert!(other.constraints.iter().any(|c| matches!(&c.kind, ConstraintKind::ForeignKey { ref_columns, .. } if *ref_columns == s(&["a2", "b2"]))));

    // Yeniden adlandırma + kısıt listesini güncellememe → doğrulama hatası (sessiz DROP+ADD yok).
    let stale = with(&r, |d| col_mut(d, "a2").name = "a3".into());
    let e = alter_errs(&env, &r, &stale);
    assert!(e.iter().any(|x| x.path.starts_with("constraints[")), "{e:?}");
}

// ================================================================================================================
// bölüm ve kalıtım bayrakları ve koruması
// ================================================================================================================

#[tokio::test]
async fn partitions_and_inheritance_are_flagged() {
    for ts in servers() {
        guarded(part_body(ts.clone()), cleanup_in(&ts, PFX, "part")).await;
    }
}

async fn part_body(ts: TestServer) {
    let env = setup_in(&ts, PFX, "part").await;
    let v = env.version;
    let pk_clause = if v >= 110000 { ", PRIMARY KEY (k, id)" } else { "" };
    env.sql(&format!(
        "CREATE TABLE p (id int NOT NULL, k int NOT NULL, v text CHECK (v <> ''){pk_clause}) PARTITION BY LIST (k);
         CREATE TABLE p1 PARTITION OF p FOR VALUES IN (1);
         CREATE TABLE p2 PARTITION OF p FOR VALUES IN (2) PARTITION BY RANGE (id);
         CREATE TABLE p2a PARTITION OF p2 FOR VALUES FROM (0) TO (10);
         CREATE TABLE base (a int CHECK (a > 0), b text);
         CREATE TABLE kid (own int) INHERITS (base);
         CREATE TABLE base2 (z int);
         CREATE TABLE kid2 () INHERITS (base, base2);"
    ))
    .await;
    let p = tload(&env, "public", "p").await;
    assert_eq!((p.partition_text.as_deref(), p.is_partition, p.inherits), (Some("LIST (k)"), false, false));
    let p1 = tload(&env, "public", "p1").await;
    assert_eq!((p1.is_partition, p1.inherits, p1.partition_text.as_deref()), (true, false, None));
    assert_eq!(p1.columns.iter().map(|c| c.name.as_str()).collect::<Vec<_>>(), ["id", "k", "v"], "bölümün kolonları yüklenir");
    assert!(p1.constraints.iter().any(|c| matches!(c.kind, ConstraintKind::Check { .. })), "{:?}", p1.constraints);
    let p2 = tload(&env, "public", "p2").await;
    assert_eq!((p2.is_partition, p2.partition_text.as_deref()), (true, Some("RANGE (id)")), "alt bölümlenmiş bölüm");
    let kid = tload(&env, "public", "kid").await;
    assert_eq!((kid.is_partition, kid.inherits), (false, true));
    assert_eq!(kid.columns.iter().map(|c| c.name.as_str()).collect::<Vec<_>>(), ["a", "b", "own"]);
    assert!(tload(&env, "public", "kid2").await.inherits);
    assert!(!tload(&env, "public", "base").await.inherits, "ebeveyn kalıtım çocuğu değildir");
    for d in [&p, &p1, &p2, &kid] {
        assert_noop(&env, d);
    }

    for child in [&p1, &kid] {
        // Kolon ve kısıt farkı → doğrulama hatası; yüklemeden gelen bayrak değiştirilemez.
        let e = alter_errs(&env, child, &with(child, |d| d.columns.push(c("extra", "int"))));
        assert!(e.iter().any(|x| x.path == "columns"), "{e:?}");
        let e = alter_errs(&env, child, &with(child, |d| d.constraints.clear()));
        assert!(e.iter().any(|x| x.path == "constraints"), "{e:?}");
        let e = alter_errs(&env, child, &with(child, |d| {
            d.is_partition = !d.is_partition;
        }));
        assert!(e.iter().any(|x| x.path == "isPartition"), "{e:?}");
    }
    // Ad, yorum, sahip serbest: uygulanır ve yeniden yüklenir.
    let role = env.role.clone();
    let p1n = alter_and_check(&env, &p1, &with(&p1, |d| {
        d.name = "p1_renamed".into();
        d.comment = Some("bölüm".into());
        d.owner = Some(role.clone());
        col_mut(d, "v").comment = Some("kolon yorumu".into());
    }))
    .await;
    assert!(p1n.is_partition);
    let kidn = alter_and_check(&env, &kid, &with(&kid, |d| {
        d.name = "kid_renamed".into();
        d.comment = Some("çocuk".into());
    }))
    .await;
    assert!(kidn.inherits);

    // Ana bölümlenmiş tabloda kolon eklemek serbesttir ve bölümlere yayılır.
    let pn = alter_and_check(&env, &p, &with(&p, |d| d.columns.push(c("added", "text")))).await;
    assert_eq!(pn.columns.last().unwrap().name, "added");
    assert_eq!(tload(&env, "public", "p1_renamed").await.columns.last().unwrap().name, "added");

    // DROP: bölüm notu ve bölümlenmiş tablo notu (yüklenmiş tanımda partition_by yok, partition_text var).
    let dp = build::drop(&t(&p1n), false, &env.ctx(false)).unwrap();
    assert!(dp.statements[0].note.as_deref().unwrap().contains("bölümdür"), "{:?}", dp.statements[0].note);
    let dp = build::drop(&t(&pn), false, &env.ctx(false)).unwrap();
    assert!(dp.statements[0].note.as_deref().unwrap_or_default().contains("Bölümlenmiş tablo"), "{:?}", dp.statements[0].note);
    env.exec_tx(&build::drop(&t(&p1n), false, &env.ctx(false)).unwrap()).await;
    assert!(!exists(&env, "SELECT 1 FROM pg_class WHERE relname = 'p1_renamed'").await);
    env.exec_tx(&dp).await;
    assert!(!exists(&env, "SELECT 1 FROM pg_class WHERE relname IN ('p', 'p2', 'p2a')").await, "bölümler de silinir");
}

// ================================================================================================================
// desteklenmeyen kısıtlar korunur
// ================================================================================================================

#[tokio::test]
async fn unsupported_constraints_are_preserved() {
    for ts in servers() {
        guarded(unsup_body(ts.clone()), cleanup_in(&ts, PFX, "unsup")).await;
    }
}

async fn unsup_body(ts: TestServer) {
    let env = setup_in(&ts, PFX, "unsup").await;
    let v = env.version;
    env.sql(
        "CREATE FUNCTION trg() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN RETURN NEW; END $$;
         CREATE TABLE ref (id int PRIMARY KEY, a int, UNIQUE (id, a));
         CREATE TABLE ex (id int GENERATED BY DEFAULT AS IDENTITY (START WITH 100) PRIMARY KEY, r int4range, a int,
                          CONSTRAINT ex_no_overlap EXCLUDE USING gist (r WITH &&)) WITH (autovacuum_enabled = false);
         CREATE CONSTRAINT TRIGGER ex_ct AFTER INSERT ON ex DEFERRABLE FOR EACH ROW EXECUTE PROCEDURE trg();
         CREATE TYPE ct AS (x int, y text);
         CREATE TABLE typed OF ct;
         CREATE TABLE ix (a int, b int, c int, CONSTRAINT ix_pk PRIMARY KEY (a) WITH (fillfactor = 70),
                          CONSTRAINT ix_u UNIQUE (b) USING INDEX TABLESPACE pg_default, CONSTRAINT ix_c UNIQUE (c));",
    )
    .await;
    let mut expect_unsup = vec![
        "WITH (autovacuum_enabled=false)".to_string(),
        "kimlik dizisi seçenekleri (id)".to_string(),
        "EXCLUDE ex_no_overlap".to_string(),
        "CONSTRAINT TRIGGER ex_ct".to_string(),
    ];
    if v >= 150000 {
        env.sql("ALTER TABLE ex ADD CONSTRAINT ex_setnull FOREIGN KEY (id, a) REFERENCES ref (id, a) ON DELETE SET NULL (a)").await;
        expect_unsup.push("FOREIGN KEY ex_setnull (ON DELETE SET NULL/DEFAULT (kolon listesi))".into());
    } else {
        eprintln!("PG {}: ON DELETE SET NULL (kolonlar) atlandı (PG 15+)", v / 10000);
    }
    if v >= 180000 {
        env.sql(
            "ALTER TABLE ex ADD CONSTRAINT ex_ck_ne CHECK (a > 0) NOT ENFORCED;
             ALTER TABLE ex ADD CONSTRAINT ex_a_nn NOT NULL a NOT VALID;",
        )
        .await;
        expect_unsup.push("CHECK ex_ck_ne (NOT ENFORCED)".into());
        expect_unsup.push("NOT NULL ex_a_nn (NOT VALID)".into());
        if env.try_sql("CREATE EXTENSION btree_gist").await.is_ok() {
            env.sql("CREATE TABLE temporal (id int, valid daterange, CONSTRAINT temporal_pk PRIMARY KEY (id, valid WITHOUT OVERLAPS))").await;
            let tl = tload(&env, "public", "temporal").await;
            assert_eq!(tl.unsupported, ["PRIMARY KEY temporal_pk (WITHOUT OVERLAPS / PERIOD)"]);
            assert!(tl.constraints.is_empty());
            assert_noop(&env, &tl);
        } else {
            eprintln!("PG 18: btree_gist yok, WITHOUT OVERLAPS atlandı");
        }
    } else {
        eprintln!("PG {}: NOT ENFORCED / NOT NULL NOT VALID / WITHOUT OVERLAPS atlandı (PG 18+)", v / 10000);
    }
    let l = tload(&env, "public", "ex").await;
    let mut got = l.unsupported.clone();
    got.sort();
    expect_unsup.sort();
    assert_eq!(got, expect_unsup);
    assert_eq!(l.constraints.iter().map(|c| c.name.as_deref().unwrap()).collect::<Vec<_>>(), ["ex_pkey"], "yalnızca modellenen kısıtlar");
    assert_noop(&env, &l);
    // PK/UNIQUE indeks seçenekleri modellenmez: DROP+ADD bunları kaybederdi → desteklenmeyen listesine gider.
    // `USING INDEX TABLESPACE pg_default` varsayılandır (reltablespace 0) → modellenir.
    let ix = tload(&env, "public", "ix").await;
    assert_eq!(ix.unsupported, ["PRIMARY KEY ix_pk (WITH (fillfactor=70))"]);
    assert_eq!(ix.constraints.iter().map(|c| c.name.as_deref().unwrap()).collect::<Vec<_>>(), ["ix_c", "ix_u"]);
    assert_noop(&env, &ix);
    let typed = tload(&env, "public", "typed").await;
    // Katalog oturumu search_path = pg_catalog: kullanıcı tipleri şema nitelikli yazılır.
    assert_eq!(typed.unsupported, ["OF public.ct (tipli tablo)"]);

    // Başka bir değişiklik (kolon ekle + yeniden adlandır + yorum) desteklenmeyenlere dokunmaz.
    let new = with(&l, |d| {
        d.columns.push(c("added", "text"));
        d.name = "ex2".into();
        d.comment = Some("x".into());
    });
    let plan = alter_plan(&env, &l, &new);
    assert!(sqls(&plan).iter().all(|s| !s.contains("ex_no_overlap") && !s.contains("ex_ct") && !s.contains("DROP CONSTRAINT")), "{:?}", sqls(&plan));
    env.exec_tx(&plan).await;
    let l2 = tload(&env, "public", "ex2").await;
    let mut got2 = l2.unsupported.clone();
    got2.sort();
    assert_eq!(got2, expect_unsup, "desteklenmeyenler korunur");
    assert!(exists(&env, "SELECT 1 FROM pg_constraint WHERE conname = 'ex_no_overlap' AND contype = 'x'").await);

    // PG 18 NOT NULL kısıtları: düz olanlar görünmez; SET/DROP NOT NULL onlarla çalışır.
    env.sql("CREATE TABLE nn (a int NOT NULL, b int PRIMARY KEY, c int)").await;
    let nl = tload(&env, "public", "nn").await;
    assert!(nl.unsupported.is_empty(), "{:?}", nl.unsupported);
    assert_eq!(nl.constraints.len(), 1);
    let nl = alter_and_check(&env, &nl, &with(&nl, |d| {
        col_mut(d, "a").not_null = false;
        col_mut(d, "c").not_null = true;
    }))
    .await;
    assert!(nl.unsupported.is_empty());
    if v >= 180000 {
        let n: i64 = env.admin.query_one("SELECT count(*) FROM pg_constraint WHERE conrelid = 'nn'::regclass AND contype = 'n'", &[]).await.unwrap().get(0);
        assert_eq!(n, 2, "PG 18: b (PK) ve c için NOT NULL kısıtı");
    }
}

// ================================================================================================================
// yüklenen her tip dilbilgisinden geçer; canonical_type ↔ format_type
// ================================================================================================================

#[tokio::test]
async fn every_loaded_type_passes_the_grammar() {
    for ts in servers() {
        guarded(types_body(ts.clone()), cleanup_in(&ts, PFX, "types")).await;
    }
}

/// (kullanıcının yazdığı tip, sürüm alt sınırı).
const TYPE_CASES: &[(&str, u32)] = &[
    ("int", 0),
    ("int4", 0),
    ("integer", 0),
    ("INT", 0),
    ("int2", 0),
    ("smallint", 0),
    ("int8", 0),
    ("bigint", 0),
    ("varchar(20)", 0),
    ("character varying(20)", 0),
    ("varchar", 0),
    ("char varying(7)", 0),
    ("timestamptz(3)", 0),
    ("timestamp(3) with time zone", 0),
    ("timestamptz", 0),
    ("timestamp", 0),
    ("timestamp(0)", 0),
    ("timestamp without time zone", 0),
    ("float", 0),
    ("float(10)", 0),
    ("float(40)", 0),
    ("float4", 0),
    ("float8", 0),
    ("real", 0),
    ("double precision", 0),
    ("decimal(10,2)", 0),
    ("numeric(10, 2)", 0),
    ("numeric(5)", 0),
    ("decimal", 0),
    ("numeric", 0),
    ("char", 0),
    ("character", 0),
    ("char(5)", 0),
    ("nchar(3)", 0),
    ("national character varying(4)", 0),
    ("bpchar", 0),
    ("bool", 0),
    ("boolean", 0),
    ("bit", 0),
    ("bit(8)", 0),
    ("varbit", 0),
    ("varbit(5)", 0),
    ("bit varying(5)", 0),
    ("time", 0),
    ("time(2)", 0),
    ("timetz", 0),
    ("time(2) with time zone", 0),
    ("interval", 0),
    ("interval(2)", 0),
    ("interval year", 0),
    ("interval year to month", 0),
    ("interval day to second(3)", 0),
    ("interval hour to minute", 0),
    ("int8[]", 0),
    ("integer[][]", 0),
    ("int[3]", 0),
    ("varchar(5)[]", 0),
    ("timestamptz[]", 0),
    ("double precision[]", 0),
    ("\"char\"", 0),
    ("name", 0),
    ("text", 0),
    ("pg_catalog.int4", 0),
    ("pg_catalog.varchar(9)", 0),
    ("money", 0),
    ("tsvector", 0),
    ("oid", 0),
    ("regclass", 0),
    ("xml", 0),
    ("jsonb", 0),
    ("json", 0),
    ("uuid", 0),
    ("inet", 0),
    ("cidr", 0),
    ("macaddr8", 0),
    ("bytea", 0),
    ("point", 0),
    ("int4range", 0),
    ("tstzrange", 0),
    ("int2vector", 0),
    ("numeric(10,-2)", 150000),
    ("int4multirange", 140000),
    // Kullanıcı tipleri (gövdede kurulur). Katalog oturumu search_path = pg_catalog olduğundan yükleyici bunları
    // şema nitelikli yazar; niteliksiz yazım (`dm`) arama yoluna bağlıdır ve canonical_type ile eşit sayılmaz (bilinen sınır).
    ("public.dm", 0),
    ("public.en", 0),
    ("public.ct", 0),
    ("public.\"Mixed Type\"", 0),
    ("public.\"Mixed Type\"[]", 0),
    ("hidden.ht", 0),
    ("hidden.\"Hidden Dom\"", 0),
    ("public.fr", 0),
    ("public.en[]", 0),
];

async fn types_body(ts: TestServer) {
    let env = setup_in(&ts, PFX, "types").await;
    let v = env.version;
    env.sql(
        r#"CREATE DOMAIN dm AS int CHECK (VALUE > 0);
           CREATE TYPE en AS ENUM ('a', 'b');
           CREATE TYPE ct AS (x int, y text);
           CREATE TYPE "Mixed Type" AS (z int);
           CREATE SCHEMA hidden;
           CREATE TYPE hidden.ht AS ENUM ('q');
           CREATE DOMAIN hidden."Hidden Dom" AS text;
           CREATE TYPE fr AS RANGE (subtype = float8);"#,
    )
    .await;
    let mut cases: Vec<&str> = TYPE_CASES.iter().filter(|(_, min)| v >= *min).map(|(t, _)| *t).collect();
    let skipped: Vec<&str> = TYPE_CASES.iter().filter(|(_, min)| v < *min).map(|(t, _)| *t).collect();
    if !skipped.is_empty() {
        eprintln!("PG {}: sürüm gereği atlanan tipler {skipped:?}", v / 10000);
    }
    if env.try_sql("CREATE EXTENSION citext").await.is_ok() {
        cases.push("public.citext");
        cases.push("public.citext[]");
    } else {
        eprintln!("PG {}: citext yok, eklenti tipi atlandı", v / 10000);
    }
    let cols: Vec<ColumnDef> = cases.iter().enumerate().map(|(i, ty)| c(&format!("c{i}"), ty)).collect();
    let d = table("public", "ty", cols, vec![]);
    let plan = build::create(&t(&d), &env.ctx(false)).unwrap_or_else(|e| panic!("tipler: {e:?}"));
    env.exec_tx(&plan).await;
    let l = tload(&env, "public", "ty").await;
    let mut report = vec![];
    for (col, user) in l.columns.iter().zip(&cases) {
        validate_type(&col.data_type).unwrap_or_else(|e| panic!("{user} → {}: dilbilgisinden geçmedi: {e}", col.data_type));
        assert_eq!(canonical_type(user), canonical_type(&col.data_type), "canonical_type({user}) ≠ canonical_type({})", col.data_type);
        report.push(format!("{user} → {}", col.data_type));
    }
    eprintln!("PG {}: format_type eşlemesi: {}", v / 10000, report.join("; "));
    assert_same(&expected(&d, &l), &l, "tipler");
    // Kullanıcının takma ad metinleriyle düzenleme (yüklenen + takma adlar) boş plan üretir.
    let aliased = with(&l, |d| {
        for (col, user) in d.columns.iter_mut().zip(&cases) {
            col.data_type = user.to_string();
        }
    });
    let p = alter_plan(&env, &l, &aliased);
    assert!(p.statements.is_empty() && p.warnings.is_empty(), "{:?} {:?}", sqls(&p), p.warnings);
    assert_noop(&env, &l);
    // Yüklenen metinle tekrar kurulabilir (format_type çıktısı geçerli tip ifadesi).
    let again = with(&l, |d| {
        d.name = "ty2".into();
        d.columns.iter_mut().for_each(|c| c.attnum = None);
    });
    env.exec_tx(&build::create(&t(&again), &env.ctx(false)).unwrap()).await;
    let l2 = tload(&env, "public", "ty2").await;
    assert_eq!(l2.columns.iter().map(|c| &c.data_type).collect::<Vec<_>>(), l.columns.iter().map(|c| &c.data_type).collect::<Vec<_>>());
}

// ================================================================================================================
// tip ve collation açılır listeleri
// ================================================================================================================

#[tokio::test]
async fn type_lookup_and_collation_lookup() {
    for ts in servers() {
        guarded(lookup_body(ts.clone()), cleanup_in(&ts, PFX, "look")).await;
    }
}

async fn lookup_body(ts: TestServer) {
    let env = setup_in(&ts, PFX, "look").await;
    let v = env.version;
    env.sql(
        r#"CREATE TYPE my_enum AS ENUM ('a');
           CREATE DOMAIN my_dom AS int;
           CREATE TYPE my_comp AS (a int);
           CREATE TABLE my_table (a int);
           CREATE SCHEMA hidden;
           CREATE TYPE hidden.my_hidden AS ENUM ('b');
           CREATE COLLATION hidden.my_coll (locale = 'C');
           CREATE COLLATION public.dup_coll (locale = 'C');
           CREATE COLLATION hidden.dup_coll (locale = 'C');"#,
    )
    .await;
    let ask = |req| {
        let env = &env;
        async move { lookup(&env.mgr, env.server, env.db_oid, req).await.unwrap() }
    };
    let ints = ask(LookupRequest::Types { query: Some("int".into()) }).await;
    let vals: Vec<&str> = ints.iter().map(|i| i.value.as_str()).collect();
    for want in ["integer", "bigint", "smallint", "int4range", "interval", "integer[]"] {
        assert!(vals.contains(&want), "{want} yok: {vals:?}");
    }
    assert!(ints.len() <= 200);
    // Önek eşleşmesi önce: ilk sonuçlar "int" ile başlayan ad/değer.
    assert!(vals.iter().position(|v| *v == "integer").unwrap() < vals.iter().position(|v| *v == "point").unwrap_or(usize::MAX));
    // Sınır: 210 kullanıcı tipiyle toplam 200'ü aşar.
    let enums: Vec<String> = (0..210).map(|i| format!("CREATE TYPE bulk_{i:03} AS ENUM ('x')")).collect();
    env.sql(&enums.join(";")).await;
    let all = ask(LookupRequest::Types { query: None }).await;
    assert_eq!(all.len(), 200, "sınır 200");
    assert_eq!(ask(LookupRequest::Types { query: Some("bulk_".into()) }).await.len(), 200);
    assert_eq!(ask(LookupRequest::Types { query: Some("  ".into()) }).await.len(), 200, "boşluk = filtre yok");
    let wide = ask(LookupRequest::Types { query: Some("my_".into()) }).await;
    let wv: Vec<(&str, &str)> = wide.iter().map(|i| (i.value.as_str(), i.detail.as_deref().unwrap())).collect();
    // Katalog oturumu search_path = pg_catalog: pg_catalog dışındaki her tip şema nitelikli (yükleyiciyle aynı biçim).
    for want in [("public.my_enum", "enum"), ("public.my_dom", "domain"), ("public.my_comp", "composite"), ("hidden.my_hidden", "enum"), ("public.my_enum[]", "array")] {
        assert!(wv.contains(&want), "{want:?} yok: {wv:?}");
    }
    assert!(!wv.iter().any(|(v, _)| v.contains("my_table")), "tablo satır tipi listelenmez: {wv:?}");
    // Tümü: sözde/iç tip yok, `_x` dizi adı yok, her değer dilbilgisinden geçer.
    let mut everything = vec![];
    for q in ["", "a", "e", "i", "o", "u", "r", "t", "n", "s", "pg", "any", "record", "internal", "_int", "%", "'", "\""] {
        everything.extend(ask(LookupRequest::Types { query: Some(q.into()) }).await);
    }
    for it in &everything {
        validate_type(&it.value).unwrap_or_else(|e| panic!("{}: {e}", it.value));
        assert!(!it.value.starts_with('_'), "{}", it.value);
        for bad in ["internal", "record", "anyelement", "anyarray", "cstring", "trigger", "void", "pg_node_tree", "unknown", "event_trigger"] {
            assert_ne!(it.value, bad);
        }
        assert!(!it.value.starts_with("information_schema.") && !it.value.starts_with("pg_toast."), "{}", it.value);
    }
    assert!(ask(LookupRequest::Types { query: Some("%".into()) }).await.is_empty(), "% LIKE kalıbı değildir");
    if env.try_sql("CREATE EXTENSION citext").await.is_ok() {
        let ci = ask(LookupRequest::Types { query: Some("cit".into()) }).await;
        assert_eq!(ci.first().map(|i| i.value.as_str()), Some("public.citext"), "{ci:?}");
    } else {
        eprintln!("PG {}: citext yok, eklenti tipi lookup'ı atlandı", v / 10000);
    }
    let colls = ask(LookupRequest::Collations).await;
    let names: Vec<&str> = colls.iter().map(|c| c.value.as_str()).collect();
    for want in ["C", "POSIX", "default"] {
        assert!(names.contains(&want), "{want} yok");
    }
    // Kullanıcı şemasındaki collation listelenir (şema detail'de); birden fazla şemadaki ad (belirsiz) listelenmez.
    let mine = colls.iter().find(|c| c.value == "my_coll").expect("hidden.my_coll listede");
    assert!(mine.detail.as_deref().unwrap().contains("şema hidden"), "{mine:?}");
    assert!(!names.contains(&"dup_coll"), "belirsiz ad listelenmez");
    // Değerler niteliksiz ad: kullanıcı şemasındaki collation detail'de şemayı taşır, değerde değil.
    assert!(!colls.iter().any(|c| c.value.starts_with("hidden.") || c.value.starts_with("public.")));
    if env.try_sql("CREATE COLLATION public.tr_ci (provider = icu, locale = 'tr-TR')").await.is_ok() {
        let colls = ask(LookupRequest::Collations).await;
        let tr = colls.iter().find(|c| c.value == "tr_ci").expect("tr_ci listede");
        let d = tr.detail.as_deref().unwrap();
        assert!(d.starts_with("icu") && d.contains("şema public"), "{d}");
    } else {
        eprintln!("PG {}: ICU yok, tr_ci collation testi atlandı", v / 10000);
    }
    let mut sorted = names.clone();
    sorted.dedup();
    assert_eq!(sorted.len(), names.len(), "yinelenen ad yok");
    assert!(colls.iter().find(|c| c.value == "C").unwrap().detail.as_deref().unwrap().starts_with("libc"));
    eprintln!("PG {}: {} collation, int sorgusu {} tip", v / 10000, colls.len(), ints.len());
}

// ================================================================================================================
// DROP çeşitleri
// ================================================================================================================

#[tokio::test]
async fn drop_table_variants() {
    for ts in servers() {
        guarded(drop_body(ts.clone()), cleanup_in(&ts, PFX, "drop")).await;
    }
}

async fn drop_body(ts: TestServer) {
    let env = setup_in(&ts, PFX, "drop").await;
    env.sql(
        r#"CREATE TABLE plain (a int);
           CREATE TABLE "Odd ""T""" (a int);
           CREATE TABLE target (id int PRIMARY KEY);
           CREATE TABLE child (t int REFERENCES target);
           CREATE VIEW vw AS SELECT * FROM target;
           CREATE TABLE lp (k int) PARTITION BY LIST (k);
           CREATE TABLE lp1 PARTITION OF lp FOR VALUES IN (1);"#,
    )
    .await;
    let ctx = env.ctx(false);
    for name in ["plain", "Odd \"T\""] {
        let d = tload(&env, "public", name).await;
        let p = build::drop(&t(&d), false, &ctx).unwrap();
        assert_eq!(p.statements[0].risk, Risk::Destructive);
        env.exec_tx(&p).await;
        assert!(!exists(&env, &format!("SELECT 1 FROM pg_class WHERE relname = {}", quote_literal(name))).await);
    }
    let target = tload(&env, "public", "target").await;
    let p = build::drop(&t(&target), false, &ctx).unwrap();
    let err = env.try_exec_tx(&p).await.unwrap_err();
    assert!(err.contains("2BP01"), "bağımlı nesne varken CASCADE'siz DROP reddedilir: {err}");
    let p = build::drop(&t(&target), true, &ctx).unwrap();
    assert_eq!(p.statements[0].risk, Risk::DataLoss);
    assert!(!p.warnings.is_empty());
    env.exec_tx(&p).await;
    assert!(!exists(&env, "SELECT 1 FROM pg_class WHERE relname IN ('target', 'vw')").await);
    let child = tload(&env, "public", "child").await;
    assert!(child.constraints.is_empty(), "CASCADE bağımlı FK'yi düşürür");
    let lp1 = tload(&env, "public", "lp1").await;
    let p = build::drop(&t(&lp1), false, &ctx).unwrap();
    assert!(p.statements[0].note.as_deref().unwrap().contains("bölümdür"));
    env.exec_tx(&p).await;
    let lp = tload(&env, "public", "lp").await;
    let p = build::drop(&t(&lp), false, &ctx).unwrap();
    assert!(p.statements[0].note.as_deref().unwrap().contains("Bölümlenmiş"));
    env.exec_tx(&p).await;
    assert!(!exists(&env, "SELECT 1 FROM pg_class WHERE relname = 'lp'").await);
}

// ================================================================================================================
// yükleyici: sistem nesnesi, yanlış relkind, olmayan oid
// ================================================================================================================

#[tokio::test]
async fn table_loader_refuses_system_and_other_relkinds() {
    for ts in servers() {
        guarded(sys_body(ts.clone()), cleanup_in(&ts, PFX, "sys")).await;
    }
}

async fn sys_body(ts: TestServer) {
    let env = setup_in(&ts, PFX, "sys").await;
    env.sql("CREATE TABLE t (id int PRIMARY KEY); CREATE VIEW v AS SELECT 1 AS x; CREATE SEQUENCE sq; CREATE MATERIALIZED VIEW mv AS SELECT 1 AS x; CREATE TYPE cty AS (a int)").await;
    let try_load = |oid| {
        let env = &env;
        async move { load(&env.mgr, env.server, env.db_oid, ObjectRef { kind: ObjectKind::Table, oid }).await }
    };
    let pg_class = env.class_oid("pg_catalog", "pg_class").await;
    assert!(matches!(try_load(pg_class).await.unwrap_err(), Error::Invalid(m) if m.contains("sistem nesnesi")));
    let info = env.class_oid("information_schema", "sql_features").await;
    assert!(matches!(try_load(info).await.unwrap_err(), Error::Invalid(_)));
    for name in ["v", "sq", "mv", "t_pkey", "cty"] {
        let oid = env.class_oid("public", name).await;
        assert!(matches!(try_load(oid).await.unwrap_err(), Error::NotFound(_)), "{name}");
    }
    assert!(matches!(try_load(4_000_000_000).await.unwrap_err(), Error::NotFound(_)));
    assert!(try_load(env.class_oid("public", "t").await).await.is_ok());
}

// ================================================================================================================
// Ledger doğrulamaları: bölümlenmiş tablo sunucu kuralları
// ================================================================================================================

#[tokio::test]
async fn partitioned_table_server_rules() {
    for ts in servers() {
        guarded(prules_body(ts.clone()), async {
            cleanup_in(&ts, PFX, "prules").await;
            let admin = pgt_core::testing::admin_client(&ts, &ts.config.maintenance_db).await;
            let _ = admin.batch_execute("DROP TABLESPACE IF EXISTS pgt_ddl_tb_ts").await;
        })
        .await;
    }
}

async fn prules_body(ts: TestServer) {
    let env = setup_in(&ts, PFX, "prules").await;
    let v = env.version;
    let major = v / 10000;
    let ctx = env.ctx(false);
    env.sql("CREATE TABLE ref (id int PRIMARY KEY)").await;
    let base = || parted(table("public", "p", vec![nn(c("k", "integer")), c("r", "integer")], vec![]), PartitionStrategy::List, vec![key("k")]);

    // 1) Kimlik kolonu bölümlenmiş tabloda: CREATE her sürümde kabul edilir. Bölüm VARKEN kimlikli kolon EKLEMEK
    //    PG 10–16'da reddedilir ("cannot recursively add identity column…") → yerel kapı; PG 17+ kabul eder.
    //    Mevcut kolona ADD GENERATED, SET GENERATED ve DROP IDENTITY bölüm varken her sürümde geçerlidir.
    let pid = with(&base(), |d| {
        d.name = "pid".into();
        d.columns.push(ident(c("id", "bigint"), IdentityKind::Always));
        d.columns.push(nn(c("x", "bigint")));
    });
    let l = create_and_check(&env, &pid).await;
    env.sql("CREATE TABLE pid1 PARTITION OF pid FOR VALUES IN (1)").await;
    let add_ident = with(&l, |d| d.columns.push(ident(c("id2", "integer"), IdentityKind::ByDefault)));
    let raw = env.try_sql("ALTER TABLE pid ADD COLUMN raw_id bigint GENERATED BY DEFAULT AS IDENTITY").await;
    if v < 170000 {
        assert!(raw.unwrap_err().contains("cannot recursively add identity column"));
        assert_eq!(alter_errs(&env, &l, &add_ident)[0].path, "columns[4].identity");
        eprintln!("PG {major}: bölümü olan tabloya kimlikli kolon eklemek sunucuda reddedildi → yerel kapı doğrulandı");
    } else {
        raw.unwrap();
        env.sql("ALTER TABLE pid DROP COLUMN raw_id").await;
        let l2 = alter_and_check(&env, &tload(&env, "public", "pid").await, &with(&tload(&env, "public", "pid").await, |d| d.columns.push(ident(c("id2", "integer"), IdentityKind::ByDefault)))).await;
        assert_eq!(l2.columns.last().unwrap().identity, Some(IdentityKind::ByDefault));
        env.sql("ALTER TABLE pid DROP COLUMN id2").await;
        eprintln!("PG {major}: bölümü olan tabloya kimlikli kolon eklendi (PG 17+)");
    }
    let l = tload(&env, "public", "pid").await;
    let l = alter_and_check(&env, &l, &with(&l, |d| col_mut(d, "x").identity = Some(IdentityKind::Always))).await;
    let l = alter_and_check(&env, &l, &with(&l, |d| col_mut(d, "id").identity = Some(IdentityKind::ByDefault))).await;
    alter_and_check(&env, &l, &with(&l, |d| col_mut(d, "x").identity = None)).await;
    // Bölüme doğrudan ekleme: PG 10–16'da bölüm kimliği miras almaz (NULL ihlali), PG 17+ alır.
    let direct = env.try_sql("INSERT INTO pid1 (k, x) VALUES (1, 1)").await;
    assert_eq!(direct.is_ok(), v >= 170000, "{direct:?}");
    eprintln!("PG {major}: bölüme doğrudan INSERT kimlik değeri {}", if direct.is_ok() { "aldı" } else { "almadı (NULL ihlali)" });
    // Kalıtım ebeveyni (çocuğu varken) kimlikli kolon eklemeyi her sürümde reddeder; tanım çocukları bilmez → sunucu.
    env.sql("CREATE TABLE ip (k int); CREATE TABLE ip1 () INHERITS (ip)").await;
    let ip = tload(&env, "public", "ip").await;
    let p = alter_plan(&env, &ip, &with(&ip, |d| d.columns.push(ident(c("id", "bigint"), IdentityKind::Always))));
    let err = env.try_exec_tx(&p).await.unwrap_err();
    assert!(err.contains("cannot recursively add identity column"), "{err}");
    eprintln!("PG {major}: kalıtım ebeveynine kimlikli kolon eklemek sunucuda reddedildi (sunucuya bırakılır)");

    // 2) Bölümlenmiş tablodan FK: PG 10 yerel hata, PG 11+ sunucuda geçerli.
    let pf = with(&base(), |d| {
        d.name = "pf".into();
        d.constraints.push(con("pf_fk", fk(&["r"], "public", "ref", &["id"])));
    });
    if v < 110000 {
        let e = build::create(&t(&pf), &ctx).unwrap_err();
        assert_eq!(e[0].path, "constraints[0]");
        eprintln!("PG 10: bölümlenmiş tablodan FK yerel olarak reddedildi");
    } else {
        create_and_check(&env, &pf).await;
    }

    // 3) Bölümlenmiş tabloya başvuran FK: PG 11 sunucu reddeder (yerel kapı yok: hedefin türü yerelde bilinmez),
    //    PG 12+ geçerli ve yükleyici hedef bölümlerin alt kısıtlarını GİZLER.
    if v >= 110000 {
        env.sql("CREATE TABLE pref (id int PRIMARY KEY) PARTITION BY RANGE (id); CREATE TABLE pref1 PARTITION OF pref FOR VALUES FROM (0) TO (100); CREATE TABLE pref2 PARTITION OF pref FOR VALUES FROM (100) TO (200)").await;
        let tf = table("public", "tf", vec![c("r", "integer")], vec![con("tf_fk", fk(&["r"], "public", "pref", &["id"]))]);
        if v < 120000 {
            let p = build::create(&t(&tf), &ctx).unwrap();
            let err = env.try_exec_tx(&p).await.unwrap_err();
            assert!(err.contains("cannot reference partitioned table"), "{err}");
            eprintln!("PG 11: bölümlenmiş tabloya başvuran FK sunucuda reddedildi (beklenen)");
        } else {
            let l = create_and_check(&env, &tf).await;
            assert_eq!(l.constraints.len(), 1, "hedef bölümlerinin alt FK satırları gizlenir: {:?}", l.constraints);
            let n: i64 = env.admin.query_one("SELECT count(*) FROM pg_constraint WHERE conrelid = 'tf'::regclass", &[]).await.unwrap().get(0);
            assert!(n >= 3, "katalogda alt FK satırları var ({n})");
        }
    } else {
        eprintln!("PG 10: bölümlenmiş tabloya başvuran FK atlandı");
    }

    // 4) Bölümlenmiş tabloya NOT VALID FK: PG 11–17 sunucu reddeder → yerel kapı; PG 18 geçerli.
    if v >= 110000 {
        let l = create_and_check(&env, &with(&base(), |d| d.name = "pnv".into())).await;
        let new = with(&l, |d| d.constraints.push(ConstraintDef { not_valid: true, ..con("pnv_fk", fk(&["r"], "public", "ref", &["id"])) }));
        if v < 180000 {
            let e = alter_errs(&env, &l, &new);
            assert_eq!(e[0].path, "constraints[0].notValid", "{e:?}");
            let raw = env.try_sql("ALTER TABLE pnv ADD CONSTRAINT x FOREIGN KEY (r) REFERENCES ref (id) NOT VALID").await.unwrap_err();
            assert!(raw.contains("NOT VALID"), "{raw}");
            eprintln!("PG {major}: bölümlenmiş tabloya NOT VALID FK sunucuda reddedilir → yerel kapı doğrulandı");
        } else {
            let l2 = alter_and_check(&env, &l, &new).await;
            assert!(l2.constraints[0].not_valid);
            alter_and_check(&env, &l2, &with(&l2, |d| d.constraints[0].not_valid = false)).await;
        }
    }

    // 5) Kalıcılık: CREATE UNLOGGED bölümlenmiş PG 18 yerel hata (sunucu reddeder), PG 10–17 geçerli; alter'da
    //    kalıcılık değişimi her sürümde yerel hata (PG 10–17 sunucu SESSİZCE yoksayar, PG 18 reddeder).
    let pu = TableDef { persistence: Persistence::Unlogged, ..with(&base(), |d| d.name = "pu".into()) };
    if v >= 180000 {
        assert_eq!(build::create(&t(&pu), &ctx).unwrap_err()[0].path, "persistence");
        let raw = env.try_sql("CREATE UNLOGGED TABLE pu_raw (k int) PARTITION BY LIST (k)").await.unwrap_err();
        assert!(raw.contains("cannot be unlogged"), "{raw}");
    } else {
        let l = create_and_check(&env, &pu).await;
        assert_eq!(l.persistence, Persistence::Unlogged);
    }
    let pl = tload(&env, "public", "pid").await;
    let e = alter_errs(&env, &pl, &with(&pl, |d| d.persistence = Persistence::Unlogged));
    assert_eq!(e[0].path, "persistence");
    let raw = env.try_sql("ALTER TABLE pid SET UNLOGGED").await;
    let persisted: String = env.admin.query_one("SELECT relpersistence::text FROM pg_class WHERE relname = 'pid'", &[]).await.unwrap().get(0);
    if v >= 180000 {
        assert!(raw.is_err(), "PG 18 reddeder");
    } else {
        assert!(raw.is_ok() && persisted == "p", "PG {major}: SET UNLOGGED sessizce yoksayılır ({raw:?}, {persisted})");
    }
    eprintln!("PG {major}: bölümlenmiş SET UNLOGGED → sunucu {}", if raw.is_err() { "reddetti" } else { "sessizce yoksaydı" });

    // 6) fillfactor: bölümlenmiş tabloda depolama parametresi yok (yerel hata; sunucu da reddeder).
    let e = alter_errs(&env, &pl, &with(&pl, |d| d.fillfactor = Some(70)));
    assert_eq!(e[0].path, "fillfactor");
    assert!(env.try_sql("ALTER TABLE pid SET (fillfactor = 70)").await.is_err(), "sunucu bölümlenmiş tabloda fillfactor'ı reddeder");

    // 7) Tablespace: CREATE … TABLESPACE pg_default bölümlenmiş tabloda PG 12+ sunucu hatası, PG 10–11 sessizce
    //    yoksayılır → her sürümde yerel hata. SET TABLESPACE pg_default (geri dönüş) PG 12+ geçerli.
    let pts = TableDef { tablespace: Some("pg_default".into()), ..with(&base(), |d| d.name = "pts".into()) };
    assert_eq!(build::create(&t(&pts), &ctx).unwrap_err()[0].path, "tablespace");
    let raw = env.try_sql("CREATE TABLE pts_raw (k int) PARTITION BY LIST (k) TABLESPACE pg_default").await;
    eprintln!("PG {major}: bölümlenmiş CREATE … TABLESPACE pg_default → sunucu {}", if raw.is_err() { "reddetti" } else { "kabul etti (yoksayar)" });
    assert_eq!(raw.is_err(), v >= 120000);
    // Gerçek bir tablespace (sunucu dosya sistemine COPY TO PROGRAM ile dizin açılabiliyorsa).
    let made = env.try_sql("COPY (SELECT 1) TO PROGRAM 'mkdir -p /tmp/pgt_ddl_tb_ts'").await.is_ok()
        && env.try_sql("CREATE TABLESPACE pgt_ddl_tb_ts LOCATION '/tmp/pgt_ddl_tb_ts'").await.is_ok();
    if made {
        // Sıradan tablo: SET TABLESPACE ve geri dönüş.
        let r = create_and_check(&env, &table("public", "rts", vec![c("a", "integer")], vec![])).await;
        let r = alter_and_check(&env, &r, &with(&r, |d| d.tablespace = Some("pgt_ddl_tb_ts".into()))).await;
        assert_eq!(r.tablespace.as_deref(), Some("pgt_ddl_tb_ts"));
        let r = alter_and_check(&env, &r, &with(&r, |d| d.tablespace = Some("pg_default".into()))).await;
        assert_eq!(r.tablespace, None);
        // PK/UNIQUE indeksi varsayılan dışı tablespace'te: modellenmez → desteklenmeyen (DROP+ADD kaybederdi).
        env.sql("CREATE TABLE its (a int, CONSTRAINT its_u UNIQUE (a) USING INDEX TABLESPACE pgt_ddl_tb_ts)").await;
        let its = tload(&env, "public", "its").await;
        assert_eq!(its.unsupported, ["UNIQUE its_u (USING INDEX TABLESPACE pgt_ddl_tb_ts)"]);
        assert!(its.constraints.is_empty());
        env.sql("DROP TABLE its").await;
        let pdef = TableDef { tablespace: Some("pgt_ddl_tb_ts".into()), ..with(&base(), |d| d.name = "pts2".into()) };
        if v >= 120000 {
            let l = create_and_check(&env, &pdef).await;
            assert_eq!(l.tablespace.as_deref(), Some("pgt_ddl_tb_ts"));
            let l = alter_and_check(&env, &l, &with(&l, |d| d.tablespace = Some("pg_default".into()))).await;
            assert_eq!(l.tablespace, None);
            alter_and_check(&env, &l, &with(&l, |d| d.tablespace = Some("pgt_ddl_tb_ts".into()))).await;
            env.sql("DROP TABLE pts2").await;
        } else {
            assert_eq!(build::create(&t(&pdef), &ctx).unwrap_err()[0].path, "tablespace");
            let e = alter_errs(&env, &pl, &with(&pl, |d| d.tablespace = Some("pgt_ddl_tb_ts".into())));
            assert_eq!(e[0].path, "tablespace");
            env.sql("CREATE TABLE pts_old (k int) PARTITION BY LIST (k) TABLESPACE pgt_ddl_tb_ts; ALTER TABLE pid SET TABLESPACE pgt_ddl_tb_ts").await;
            assert_eq!(tload(&env, "public", "pts_old").await.tablespace, None, "PG {major}: bölümlenmiş TABLESPACE yoksayılır");
            assert_eq!(tload(&env, "public", "pid").await.tablespace, None, "PG {major}: SET TABLESPACE yoksayılır");
            env.sql("DROP TABLE pts_old").await;
        }
        env.sql("DROP TABLE rts").await;
        eprintln!("PG {major}: gerçek tablespace ile SET TABLESPACE doğrulandı");
    } else {
        eprintln!("PG {major}: tablespace dizini açılamadı (COPY TO PROGRAM yok), gerçek tablespace adımı atlandı");
    }
}

// ================================================================================================================
// Ledger doğrulamaları: üretilmiş kolon kuralları
// ================================================================================================================

#[tokio::test]
async fn generated_column_server_rules() {
    for ts in servers() {
        guarded(gen_body(ts.clone()), cleanup_in(&ts, PFX, "gen")).await;
    }
}

async fn gen_body(ts: TestServer) {
    let env = setup_in(&ts, PFX, "gen").await;
    let v = env.version;
    let major = v / 10000;
    if v < 120000 {
        let d = table("public", "g", vec![c("a", "integer"), gen_col(c("b", "integer"), "a * 2", true)], vec![]);
        assert_eq!(build::create(&t(&d), &env.ctx(false)).unwrap_err()[0].path, "columns[1].generated");
        eprintln!("PG {major}: üretilmiş kolon yok (PG 12+), yalnızca yerel kapı doğrulandı");
        return;
    }
    let d = table("public", "g", vec![c("a", "integer"), gen_col(c("b", "integer"), "a * 2", true), c("z", "integer")], vec![]);
    let l = create_and_check(&env, &d).await;
    // Üretilmiş kolonun kullandığı kolonun TYPE değişimi: sunucu reddeder; plan uyarısı bunu söyler.
    let retyped = with(&l, |d| col_mut(d, "a").data_type = "bigint".into());
    let p = alter_plan(&env, &l, &retyped);
    assert!(p.warnings.iter().any(|w| w.contains("üretilmiş 'b'")), "{:?}", p.warnings);
    let err = env.try_exec_tx(&p).await.unwrap_err();
    assert!(err.contains("generated column"), "{err}");
    // Kullanmayan kolonda uyarı yok ve uygulanır.
    let p = alter_plan(&env, &l, &with(&l, |d| col_mut(d, "z").data_type = "bigint".into()));
    assert!(!p.warnings.iter().any(|w| w.contains("üretilmiş")), "{:?}", p.warnings);
    let l = alter_and_check(&env, &l, &with(&l, |d| col_mut(d, "z").data_type = "bigint".into())).await;

    // DROP EXPRESSION (STORED): PG 13+.
    let dropped = with(&l, |d| col_mut(d, "b").generated = None);
    if v < 130000 {
        assert_eq!(alter_errs(&env, &l, &dropped)[0].path, "columns[1].generated");
        eprintln!("PG 12: DROP EXPRESSION yerel kapısı doğrulandı (PG 13+)");
    } else {
        let l2 = alter_and_check(&env, &l, &dropped).await;
        assert_eq!((l2.columns[1].generated.clone(), l2.columns[1].default.clone()), (None, None));
    }
    // SET EXPRESSION: PG 17+.
    env.sql("CREATE TABLE g2 (a int, b int GENERATED ALWAYS AS (a + 1) STORED)").await;
    let g2 = tload(&env, "public", "g2").await;
    let changed = with(&g2, |d| col_mut(d, "b").generated.as_mut().unwrap().expr = "a + 5".into());
    if v < 170000 {
        assert_eq!(alter_errs(&env, &g2, &changed)[0].path, "columns[1].generated");
        eprintln!("PG {major}: SET EXPRESSION yerel kapısı doğrulandı (PG 17+)");
    } else {
        alter_and_check(&env, &g2, &changed).await;
    }
    // PG 18 sanal kolon: DROP EXPRESSION sunucuda da reddedilir; STORED↔VIRTUAL değişimi yerel hata.
    if v >= 180000 {
        env.sql("CREATE TABLE gv (a int, b int GENERATED ALWAYS AS (a * 2) VIRTUAL)").await;
        let gv = tload(&env, "public", "gv").await;
        assert_eq!(gv.columns[1].generated, Some(GeneratedDef { expr: "a * 2".into(), stored: false }));
        assert_eq!(alter_errs(&env, &gv, &with(&gv, |d| col_mut(d, "b").generated = None))[0].path, "columns[1].generated");
        let raw = env.try_sql("ALTER TABLE gv ALTER COLUMN b DROP EXPRESSION").await.unwrap_err();
        assert!(raw.contains("virtual"), "{raw}");
        assert_eq!(alter_errs(&env, &gv, &with(&gv, |d| col_mut(d, "b").generated.as_mut().unwrap().stored = true))[0].path, "columns[1].generated");
        // CHECK'siz tabloda sanal SET EXPRESSION geçerli; CHECK eklenince sunucu reddeder (yerel kapı doğrulaması).
        let gv = alter_and_check(&env, &gv, &with(&gv, |d| col_mut(d, "b").generated.as_mut().unwrap().expr = "a * 3".into())).await;
        env.sql("ALTER TABLE gv ADD CONSTRAINT gv_ck CHECK (a > 0)").await;
        let raw = env.try_sql("ALTER TABLE gv ALTER COLUMN b SET EXPRESSION AS (a * 4)").await.unwrap_err();
        assert!(raw.contains("check constraints"), "{raw}");
        let gvc = tload(&env, "public", "gv").await;
        let e = alter_errs(&env, &gvc, &with(&gvc, |d| col_mut(d, "b").generated.as_mut().unwrap().expr = "a * 4".into()));
        assert_eq!(e[0].path, "columns[1].generated");
        // CHECK aynı düzenlemede kaldırılırsa (adım 3'te DROP, adım 8'de SET EXPRESSION) uygulanır.
        alter_and_check(&env, &gvc, &with(&gvc, |d| {
            d.constraints.clear();
            col_mut(d, "b").generated.as_mut().unwrap().expr = "a * 4".into();
        }))
        .await;
        let _ = gv;
        eprintln!("PG 18: sanal kolonda DROP EXPRESSION sunucuda reddedildi (yerel kural doğru)");
    } else {
        eprintln!("PG {major}: sanal kolon atlandı (PG 18+)");
    }
}

// ================================================================================================================
// Ledger doğrulamaları: kendine başvuran FK yeniden kurulumu (Probe A/B) gerçek sunucuda
// ================================================================================================================

#[tokio::test]
async fn self_fk_rebuild_probes_apply() {
    for ts in servers() {
        guarded(probe_body(ts.clone()), cleanup_in(&ts, PFX, "probe")).await;
    }
}

async fn probe_body(ts: TestServer) {
    let env = setup_in(&ts, PFX, "probe").await;
    let d = table(
        "public",
        "tree",
        vec![nn(c("a", "integer")), nn(c("b", "integer")), c("pa", "integer"), c("pb", "integer"), c("x", "integer")],
        vec![con("tree_ab", uq(&["a", "b"])), con("tree_self", fk(&["pa", "pb"], "public", "tree", &["a", "b"])), con("tree_x", uq(&["x"]))],
    );
    let l = create_and_check(&env, &d).await;
    env.sql("INSERT INTO tree VALUES (1, 1, NULL, NULL, 1), (2, 2, 1, 1, 2)").await;
    // Probe A: hedef UNIQUE yapısal değişir ((a,b) → (b,a)): FK önce düşer, anahtar yeniden kurulur, FK sonra eklenir.
    let new = with(&l, |d| con_mut(d, "tree_ab").kind = uq(&["b", "a"]));
    let p = alter_plan(&env, &l, &new);
    let kinds: Vec<String> = p.statements.iter().map(|s| s.sql.split(" CONSTRAINT ").nth(1).unwrap_or("").chars().take(30).collect()).collect();
    assert!(p.statements[0].sql.contains("DROP CONSTRAINT \"tree_self\"") || p.statements[0].sql.contains("DROP CONSTRAINT tree_self"), "{:?}", sqls(&p));
    eprintln!("Probe A planı: {kinds:?}");
    let l2 = alter_and_check(&env, &l, &new).await;
    // Probe A + tablo yeniden adlandırma: yeniden eklenen FK yeni adı kullanır.
    let new = with(&l2, |d| {
        d.name = "tree2".into();
        con_mut(d, "tree_ab").kind = uq(&["a", "b"]);
        // İstemci kendine başvuran FK'nin hedefini yeni ada yayar (yayılmamış biçim de "değişmemiş" sayılır).
        if let ConstraintKind::ForeignKey { ref_table, .. } = &mut con_mut(d, "tree_self").kind {
            *ref_table = "tree2".into();
        }
    });
    let l3 = alter_and_check(&env, &l2, &new).await;
    assert!(l3.constraints.iter().any(|c| matches!(&c.kind, ConstraintKind::ForeignKey { ref_table, .. } if ref_table == "tree2")));
    // Probe B: hedef kolon silinirken FK de kaldırılır → DROP FK, DROP UNIQUE, DROP COLUMN.
    let new = with(&l3, |d| {
        d.columns.retain(|c| c.name != "b");
        d.constraints.retain(|c| !matches!(c.name.as_deref(), Some("tree_self") | Some("tree_ab")));
    });
    let p = alter_plan(&env, &l3, &new);
    assert_eq!(p.statements.len(), 3, "{:?}", sqls(&p));
    alter_and_check(&env, &l3, &new).await;
    // Probe B (FK korunursa): yerel hata.
    // Yerel kolonu (pa) kalan, hedef kolonu (x) silinen kendine başvuran FK: hedef kolon yolunda hata.
    env.sql("ALTER TABLE tree2 ADD CONSTRAINT tree_self2 FOREIGN KEY (pa) REFERENCES tree2 (x)").await;
    let l4b = tload(&env, "public", "tree2").await;
    let new = with(&l4b, |d| {
        d.columns.retain(|c| c.name != "x");
        d.constraints.retain(|c| c.name.as_deref() != Some("tree_x"));
    });
    assert_eq!(new.constraints[0].name.as_deref(), Some("tree_self2"));
    let e = alter_errs(&env, &l4b, &new);
    assert_eq!(e.iter().map(|x| x.path.as_str()).collect::<Vec<_>>(), ["constraints[0].refColumns[0]"], "{e:?}");
}

// ================================================================================================================
// Ledger doğrulamaları: 11 adımlı birleşik plan gerçek sunucuda tek transaction'da uygulanır
// ================================================================================================================

#[tokio::test]
async fn full_combined_plan_applies() {
    for ts in servers() {
        guarded(full_body(ts.clone()), cleanup_in(&ts, PFX, "full")).await;
    }
}

async fn full_body(ts: TestServer) {
    let env = setup_in(&ts, PFX, "full").await;
    let role = env.role.clone();
    env.sql(r#"CREATE SCHEMA s1; CREATE SCHEMA "S 2"; CREATE TABLE s1.ref (id int PRIMARY KEY)"#).await;
    let d = table(
        "s1",
        "orders",
        vec![
            nn(c("id", "integer")),
            c("old_col", "text"),
            dflt(c("qty", "integer"), "1"),
            c("label", "text"),
            c("r", "integer"),
            c("note", "text"),
        ],
        vec![con("orders_pk", pk(&["id"])), con("orders_ck", ck("qty > 0")), con("orders_u", uq(&["label"])), con("orders_fk", fk(&["r"], "s1", "ref", &["id"]))],
    );
    let l = create_and_check(&env, &d).await;
    env.sql("INSERT INTO s1.ref VALUES (1); INSERT INTO s1.orders VALUES (1, 'x', 2, 'a', 1, 'n')").await;
    let new = with(&l, |d| {
        d.name = "Orders 2".into(); // (1)
        d.schema = "S 2".into(); // (2)
        d.constraints.retain(|c| c.name.as_deref() != Some("orders_fk")); // (3) DROP
        con_mut(d, "orders_ck").kind = ck("qty >= 0"); // (3)+(10) yapısal
        d.columns.retain(|c| c.name != "old_col"); // (4)
        col_mut(d, "qty").data_type = "bigint".into(); // (5) (+ DROP/SET DEFAULT)
        col_mut(d, "label").name = "title".into(); // (6)
        if let ConstraintKind::Unique { columns, .. } = &mut con_mut(d, "orders_u").kind {
            columns[0] = "title".into();
        }
        d.columns.push(dflt(nn(c("added", "boolean")), "true")); // (7)
        col_mut(d, "note").not_null = true; // (8)
        col_mut(d, "note").default = Some("'-'::text".into());
        con_mut(d, "orders_u").name = Some("orders_title".into()); // (9)
        d.constraints.push(con("orders_fk2", fk(&["r"], "s1", "ref", &["id"]))); // (10)
        d.constraints.push(ConstraintDef { not_valid: true, ..con("orders_nv", ck("id > 0")) });
        d.owner = Some(role.clone()); // (11)
        d.fillfactor = Some(80);
        d.comment = Some("birleşik".into());
        col_mut(d, "title").comment = Some("başlık".into());
    });
    let p = alter_plan(&env, &l, &new);
    eprintln!("birleşik plan ({} ifade):\n  {}", p.statements.len(), sqls(&p).join("\n  "));
    let l2 = alter_and_check(&env, &l, &new).await;
    assert_eq!((l2.schema.as_str(), l2.name.as_str()), ("S 2", "Orders 2"));
    assert_eq!(l2.constraints.iter().find(|c| c.name.as_deref() == Some("orders_title")).and_then(|c| c.oid), l.constraints.iter().find(|c| c.name.as_deref() == Some("orders_u")).and_then(|c| c.oid), "yeniden adlandırılan kısıt aynı oid");
    let n: i64 = env.admin.query_one(r#"SELECT count(*) FROM "S 2"."Orders 2" WHERE added AND qty = 2"#, &[]).await.unwrap().get(0);
    assert_eq!(n, 1);
}

// ================================================================================================================
// Ledger doğrulamaları: kolonsuz tablolar, serial reddi
// ================================================================================================================

#[tokio::test]
async fn zero_column_tables_and_serial() {
    for ts in servers() {
        guarded(zero_body(ts.clone()), cleanup_in(&ts, PFX, "zero")).await;
    }
}

async fn zero_body(ts: TestServer) {
    let env = setup_in(&ts, PFX, "zero").await;
    let ctx = env.ctx(false);
    env.sql("CREATE TABLE z ()").await;
    let z = tload(&env, "public", "z").await;
    assert!(z.columns.is_empty() && z.constraints.is_empty());
    assert_noop(&env, &z);
    assert_eq!(build::create(&t(&with(&z, |d| d.name = "z2".into())), &ctx).unwrap_err()[0].path, "columns", "create kolonsuz tabloyu reddeder");
    let z = alter_and_check(&env, &z, &with(&z, |d| d.columns.extend([c("a", "integer"), c("b", "text")]))).await;
    let z = alter_and_check(&env, &z, &with(&z, |d| d.columns.clear())).await;
    assert!(z.columns.is_empty());
    alter_and_check(&env, &z, &with(&z, |d| d.name = "z_renamed".into())).await;

    // serial: tip dilbilgisi reddeder (create, ADD COLUMN, TYPE); sunucuda serial ile kurulmuş tablo integer + nextval
    // varsayılanı + NOT NULL olarak yüklenir ve değiştirmeden düzenleme boş plandır.
    for ty in ["serial", "bigserial", "smallserial", "serial4", "pg_catalog.serial8", "SERIAL"] {
        let d = table("public", "s", vec![c("id", ty)], vec![]);
        assert_eq!(build::create(&t(&d), &ctx).unwrap_err()[0].path, "columns[0].dataType", "{ty}");
    }
    env.sql("CREATE TABLE ser (id serial PRIMARY KEY, b bigserial)").await;
    let ser = tload(&env, "public", "ser").await;
    assert_eq!(ser.columns[0].data_type, "integer");
    // Katalog oturumu search_path = pg_catalog: regclass şema nitelikli yazılır.
    assert_eq!(ser.columns[0].default.as_deref(), Some("nextval('public.ser_id_seq'::regclass)"));
    assert!(ser.columns[0].not_null && ser.unsupported.is_empty());
    assert_noop(&env, &ser);
    let e = alter_errs(&env, &ser, &with(&ser, |d| d.columns.push(c("x", "serial"))));
    assert_eq!(e[0].path, "columns[2].dataType");
    let e = alter_errs(&env, &ser, &with(&ser, |d| col_mut(d, "b").data_type = "serial".into()));
    assert_eq!(e[0].path, "columns[1].dataType");
    // Tablo yeniden adlandırma serial varsayılanını bozmaz (regclass oid ile izlenir).
    let ser2 = alter_and_check(&env, &ser, &with(&ser, |d| d.name = "ser2".into())).await;
    assert_eq!(ser2.columns[0].default.as_deref(), Some("nextval('public.ser_id_seq'::regclass)"));
}
