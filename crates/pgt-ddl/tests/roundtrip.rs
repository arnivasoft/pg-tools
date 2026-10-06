//! Üretici → sunucu → yükleyici gidiş-dönüş testleri (Docker; `PGT_TEST_URLS` yoksa atlanır).
//!
//! Her test kendi geçici veritabanını (`pgt_ddl_rt_<ana sürüm>_<etiket>`) ve rolünü kurar; başarısız
//! doğrulamadan sonra bile temizlenir, önceki çalıştırmaların artıkları başta silinir.

mod common;

use common::{Env, cleanup, guarded, servers, setup};
use pgt_core::Error;
use pgt_core::testing::TestServer;
use pgt_ddl::build;
use pgt_ddl::*;

// ---- tanım kurucuları ----

fn schema(name: &str) -> SchemaDef {
    SchemaDef { name: name.into(), owner: None, comment: None }
}

#[allow(clippy::too_many_arguments)]
fn seq(schema: &str, name: &str, t: SeqType, inc: i64, min: Option<i64>, max: Option<i64>, start: i64) -> SequenceDef {
    SequenceDef {
        schema: schema.into(),
        name: name.into(),
        owner: None,
        data_type: t,
        increment: inc,
        min_value: min,
        max_value: max,
        start,
        cache: 1,
        cycle: false,
        owned_by: None,
        comment: None,
    }
}

fn colref(schema: &str, table: &str, column: &str) -> ColumnRef {
    ColumnRef { schema: schema.into(), table: table.into(), column: column.into() }
}

fn col(name: &str) -> IndexColumn {
    IndexColumn { key: IndexKey::Column { name: name.into() }, opclass: None, collation: None, descending: false, nulls_first: None }
}

fn expr(sql: &str) -> IndexColumn {
    IndexColumn { key: IndexKey::Expression { sql: sql.into() }, ..col("x") }
}

fn idx(schema: &str, table: &str, name: &str, method: &str, columns: Vec<IndexColumn>) -> IndexDef {
    IndexDef {
        schema: schema.into(),
        table: table.into(),
        name: name.into(),
        method: method.into(),
        unique: false,
        columns,
        include: vec![],
        where_clause: None,
        tablespace: None,
        fillfactor: None,
        comment: None,
        constraint: None,
    }
}

// ---- oluştur / yükle / karşılaştır ----

async fn oid_of(env: &Env, def: &ObjectDef) -> u32 {
    match def {
        ObjectDef::Schema(d) => env.schema_oid(&d.name).await,
        ObjectDef::Sequence(d) => env.class_oid(&d.schema, &d.name).await,
        ObjectDef::Extension(d) => env.extension_oid(&d.name).await,
        ObjectDef::Index(d) => env.class_oid(&d.schema, &d.name).await,
    }
}

async fn load_named(env: &Env, def: &ObjectDef) -> ObjectDef {
    let oid = oid_of(env, def).await;
    load(&env.mgr, env.server, env.db_oid, ObjectRef { kind: def.kind(), oid }).await.unwrap()
}

/// Beklenen tanımı yükleyicinin normalleştirmesine getirir: belirtilmeyen sahip/şema/sürüm yüklenenden gelir,
/// varsayılan tablespace (`pg_default`) ve yönün varsayılanı olan `nulls_first` yüklemede `None`'dır.
fn expect(def: &ObjectDef, loaded: &ObjectDef) -> ObjectDef {
    let mut e = def.clone();
    match (&mut e, loaded) {
        (ObjectDef::Schema(d), ObjectDef::Schema(l)) => d.owner = d.owner.take().or_else(|| l.owner.clone()),
        (ObjectDef::Sequence(d), ObjectDef::Sequence(l)) => d.owner = d.owner.take().or_else(|| l.owner.clone()),
        (ObjectDef::Extension(d), ObjectDef::Extension(l)) => {
            d.schema = d.schema.take().or_else(|| l.schema.clone());
            d.version = d.version.take().or_else(|| l.version.clone());
        }
        (ObjectDef::Index(d), _) => {
            if d.tablespace.as_deref() == Some("pg_default") {
                d.tablespace = None;
            }
            for c in &mut d.columns {
                if c.nulls_first == Some(c.descending) {
                    c.nulls_first = None;
                }
            }
        }
        _ => panic!("tür uyuşmazlığı"),
    }
    e
}

/// `build::create` → ifadeleri çalıştır → yükle → beklenen tanıma eşit.
async fn create_and_check(env: &Env, def: ObjectDef) -> ObjectDef {
    let plan = build::create(&def, &env.ctx(false)).unwrap_or_else(|e| panic!("create reddedildi: {def:?} {e:?}"));
    env.exec(&plan).await;
    let loaded = load_named(env, &def).await;
    assert_eq!(expect(&def, &loaded), loaded, "create → load");
    loaded
}

/// `build::alter(old, new)` → çalıştır → yükle → beklenen yeni tanıma eşit; yüklenen tanımı döndürür.
async fn alter_and_check(env: &Env, old: &ObjectDef, new: ObjectDef, concurrently: bool) -> ObjectDef {
    let plan = build::alter(old, &new, &env.ctx(concurrently)).unwrap_or_else(|e| panic!("alter reddedildi: {old:?} → {new:?} {e:?}"));
    env.exec(&plan).await;
    let loaded = load_named(env, &new).await;
    assert_eq!(expect(&new, &loaded), loaded, "alter → load; plan: {:?}", plan.statements.iter().map(|s| &s.sql).collect::<Vec<_>>());
    loaded
}

/// Yüklenen tanımı kendisiyle karşılaştıran alter boş plan üretir (yükleyici/üretici normalleştirmesi tutarlı).
fn assert_noop(env: &Env, loaded: &ObjectDef) {
    let p = build::alter(loaded, loaded, &env.ctx(false)).unwrap();
    assert!(p.statements.is_empty(), "boş plan bekleniyordu: {:?}", p.statements.iter().map(|s| &s.sql).collect::<Vec<_>>());
}

fn as_seq(d: &ObjectDef) -> SequenceDef {
    match d {
        ObjectDef::Sequence(s) => s.clone(),
        _ => panic!("sequence bekleniyordu"),
    }
}
fn as_idx(d: &ObjectDef) -> IndexDef {
    match d {
        ObjectDef::Index(s) => s.clone(),
        _ => panic!("index bekleniyordu"),
    }
}
fn as_schema(d: &ObjectDef) -> SchemaDef {
    match d {
        ObjectDef::Schema(s) => s.clone(),
        _ => panic!("schema bekleniyordu"),
    }
}
fn as_ext(d: &ObjectDef) -> ExtensionDef {
    match d {
        ObjectDef::Extension(s) => s.clone(),
        _ => panic!("extension bekleniyordu"),
    }
}

// ---- create → load ----

#[tokio::test]
async fn create_then_load_equals_def() {
    for ts in servers() {
        guarded(create_body(ts.clone()), cleanup(&ts, "create")).await;
    }
}

async fn create_body(ts: TestServer) {
    let env = setup(&ts, "create").await;
    let role = env.role.clone();

    // --- Schema ---
    create_and_check(&env, ObjectDef::Schema(schema("rt plain"))).await;
    create_and_check(&env, ObjectDef::Schema(SchemaDef { owner: Some(role.clone()), comment: Some("it's a \\ test; -- /* x */".into()), ..schema("Rt \"Q\" s1") })).await;
    let loaded = load_named(&env, &ObjectDef::Schema(schema("rt plain"))).await;
    assert_eq!(as_schema(&loaded).owner.as_deref(), Some(env.me.as_str()), "sahip belirtilmezse oturum rolü");

    // --- Sequence ---
    env.sql(r#"CREATE SCHEMA "rt s1""#).await;
    env.sql(r#"CREATE TABLE "rt s1"."own me" (id int)"#).await;
    env.sql(r#"CREATE TABLE "rt s1"."own role" (id int, "Id2" int)"#).await;
    env.sql(&format!(r#"ALTER TABLE "rt s1"."own role" OWNER TO {role}"#)).await;
    let s = "rt s1";
    let seqs = vec![
        seq(s, "sm", SeqType::Smallint, 1, None, None, 1),
        seq(s, "int cycle", SeqType::Integer, 1, Some(10), Some(1000), 10),
        SequenceDef { cycle: true, cache: 5, ..seq(s, "Big Cycle", SeqType::Bigint, 3, None, None, 7) },
        seq(s, "desc", SeqType::Bigint, -2, None, None, -10),
        seq(s, "desc sm", SeqType::Smallint, -1, Some(-500), None, -5),
        SequenceDef { owned_by: Some(colref(s, "own me", "id")), ..seq(s, "owned", SeqType::Integer, 1, None, None, 1) },
        SequenceDef { comment: Some("it's".into()), owned_by: Some(colref(s, "own role", "Id2")), owner: Some(role.clone()), ..seq(s, "own+owner", SeqType::Bigint, 1, None, Some(99), 5) },
        SequenceDef { owner: Some(role.clone()), ..seq(s, "only owner", SeqType::Smallint, 2, Some(2), Some(30000), 2) },
    ];
    for d in seqs {
        let loaded = create_and_check(&env, ObjectDef::Sequence(d.clone())).await;
        // Varsayılan sınırlar sahiden `None` yüklenir (sunucunun yazdığı tür sınırı değil).
        let l = as_seq(&loaded);
        assert_eq!((l.min_value, l.max_value), (d.min_value, d.max_value), "{}", d.name);
    }

    // --- Extension (contrib; yoksa atla) ---
    for ext in ["citext", "pgcrypto"] {
        if !env.has_extension(ext).await {
            eprintln!("{ext} yok, eklenti testi atlandı");
            continue;
        }
        let ver: String = env.admin.query_one("SELECT default_version::text FROM pg_available_extensions WHERE name = $1::text", &[&ext]).await.unwrap().get(0);
        let def = ExtensionDef { name: ext.into(), schema: Some("rt s1".into()), version: Some(ver.clone()), cascade: false };
        let loaded = create_and_check(&env, ObjectDef::Extension(def.clone())).await;
        assert_eq!(as_ext(&loaded), def);
        env.sql(&format!("DROP EXTENSION {ext}")).await;
        // Şema/sürüm belirtilmeden: yükleyici sunucunun seçtiğini doldurur.
        let l = create_and_check(&env, ObjectDef::Extension(ExtensionDef { name: ext.into(), schema: None, version: None, cascade: false })).await;
        assert_eq!(as_ext(&l).schema.as_deref(), Some("public"));
        assert_eq!(as_ext(&l).version.as_deref(), Some(ver.as_str()));
    }

    // --- Index ---
    env.sql(r#"CREATE SCHEMA "rt s2""#).await;
    env.sql(r#"CREATE TABLE "rt s2"."Rt Tbl" (a text, b text, c int, d int, e text[], g jsonb, ts timestamptz, p point, "Weird Col" int, deleted timestamptz)"#).await;
    let (s, t) = ("rt s2", "Rt Tbl");
    let mut defs: Vec<IndexDef> = vec![
        idx(s, t, "bt simple", "btree", vec![col("a")]),
        IndexDef { unique: true, ..idx(s, t, "bt unique", "btree", vec![col("a"), col("c")]) },
        idx(s, t, "hs", "hash", vec![col("a")]),
        idx(s, t, "gin arr", "gin", vec![col("e")]),
        idx(s, t, "gin tsv", "gin", vec![expr("to_tsvector('simple'::regconfig, b)")]),
        idx(s, t, "gin jsonb", "gin", vec![IndexColumn { opclass: Some("jsonb_path_ops".into()), ..col("g") }]),
        idx(s, t, "gist pt", "gist", vec![col("p")]),
        idx(s, t, "brin ts", "brin", vec![col("ts")]),
        idx(s, t, "expr lower", "btree", vec![expr("lower(a)")]),
        idx(s, t, "expr arith", "btree", vec![expr("c + 1"), col("d")]),
        idx(s, t, "expr coll", "btree", vec![IndexColumn { collation: Some("C".into()), ..expr("lower(b)") }]),
        idx(s, t, "desc nl", "btree", vec![IndexColumn { descending: true, nulls_first: Some(false), ..col("c") }]),
        idx(s, t, "asc nf", "btree", vec![IndexColumn { nulls_first: Some(true), ..col("c") }]),
        // DESC NULLS FIRST yönün varsayılanıdır: yüklemede `None`'a normalleşir.
        idx(s, t, "desc nf", "btree", vec![IndexColumn { descending: true, nulls_first: Some(true), ..col("c") }]),
        idx(s, t, "weird col", "btree", vec![col("Weird Col"), col("d")]),
        IndexDef { where_clause: Some("deleted IS NULL".into()), ..idx(s, t, "partial", "btree", vec![col("a")]) },
        IndexDef { where_clause: Some("c > 10".into()), unique: true, ..idx(s, t, "partial u", "btree", vec![col("c")]) },
        IndexDef { fillfactor: Some(70), ..idx(s, t, "ff bt", "btree", vec![col("a")]) },
        IndexDef { fillfactor: Some(80), ..idx(s, t, "ff hs", "hash", vec![col("c")]) },
        IndexDef { fillfactor: Some(90), ..idx(s, t, "ff gist", "gist", vec![col("p")]) },
        idx(s, t, "opclass", "btree", vec![IndexColumn { opclass: Some("text_pattern_ops".into()), ..col("a") }]),
        idx(s, t, "collate c", "btree", vec![IndexColumn { collation: Some("C".into()), ..col("a") }]),
        idx(s, t, "coll+opc", "btree", vec![IndexColumn { collation: Some("C".into()), opclass: Some("text_pattern_ops".into()), descending: true, ..col("a") }]),
        IndexDef { tablespace: Some("pg_default".into()), ..idx(s, t, "ts default", "btree", vec![col("c")]) },
        IndexDef { comment: Some("it's; -- ok".into()), ..idx(s, t, "Cm \"Q\"", "btree", vec![col("c")]) },
        IndexDef {
            unique: true,
            where_clause: Some("d IS NOT NULL".into()),
            fillfactor: Some(75),
            comment: Some("all".into()),
            ..idx(s, t, "everything", "btree", vec![IndexColumn { descending: true, ..col("a") }, expr("lower(b)"), IndexColumn { nulls_first: Some(true), ..col("c") }])
        },
    ];
    if env.version >= 110000 {
        defs.push(IndexDef { include: vec!["d".into(), "Weird Col".into()], ..idx(s, t, "incl", "btree", vec![col("c")]) });
        defs.push(IndexDef { include: vec!["d".into()], unique: true, where_clause: Some("c > 0".into()), ..idx(s, t, "incl u", "btree", vec![col("a"), col("c")]) });
    } else {
        // PG 10: INCLUDE kapısı üreticide doğrulama hatasıdır.
        let d = IndexDef { include: vec!["d".into()], ..idx(s, t, "incl", "btree", vec![col("c")]) };
        assert!(build::create(&ObjectDef::Index(d), &env.ctx(false)).is_err());
    }
    if env.has_extension("pg_trgm").await {
        env.sql("CREATE EXTENSION pg_trgm SCHEMA public").await;
        defs.push(idx(s, t, "trgm", "gin", vec![IndexColumn { opclass: Some("gin_trgm_ops".into()), ..col("b") }]));
        defs.push(idx(s, t, "trgm gist", "gist", vec![IndexColumn { opclass: Some("gist_trgm_ops".into()), ..col("b") }]));
    } else {
        eprintln!("pg_trgm yok, trigram indeksleri atlandı");
    }
    if env.has_extension("btree_gist").await {
        env.sql("CREATE EXTENSION btree_gist SCHEMA public").await;
        defs.push(idx(s, t, "bgist", "gist", vec![col("c")]));
    } else {
        eprintln!("btree_gist yok, gist(int) indeksi atlandı");
    }
    for d in defs {
        create_and_check(&env, ObjectDef::Index(d)).await;
    }
}

// ---- alter old → new ----

#[tokio::test]
async fn alter_then_load_equals_new() {
    for ts in servers() {
        guarded(alter_body(ts.clone()), cleanup(&ts, "alter")).await;
    }
}

async fn alter_body(ts: TestServer) {
    let env = setup(&ts, "alter").await;
    let role = env.role.clone();
    let me = env.me.clone();

    // --- Schema: rename + owner + yorum, sonra yorumu kaldırma ---
    let old = create_and_check(&env, ObjectDef::Schema(schema("al s0"))).await;
    assert_noop(&env, &old);
    let mut n = as_schema(&old);
    n.name = "Al \"S\" 1".into();
    n.owner = Some(role.clone());
    n.comment = Some("it's".into());
    let loaded = alter_and_check(&env, &old, ObjectDef::Schema(n), false).await;
    let mut n = as_schema(&loaded);
    n.comment = None;
    let loaded = alter_and_check(&env, &loaded, ObjectDef::Schema(n), false).await;
    assert_eq!(as_schema(&loaded).comment, None);

    // --- Sequence ---
    for s in ["rt s1", "rt s2"] {
        env.sql(&format!(r#"CREATE SCHEMA "{s}""#)).await;
    }
    env.sql(r#"CREATE TABLE "rt s1".tb1 (id int)"#).await;
    env.sql(r#"CREATE TABLE "rt s2".tb2 (id int)"#).await;
    env.sql(r#"CREATE TABLE "rt s2".tb3 (id int)"#).await;
    env.sql(&format!(r#"ALTER TABLE "rt s2".tb3 OWNER TO {role}"#)).await;
    let first = SequenceDef { owned_by: Some(colref("rt s1", "tb1", "id")), ..seq("rt s1", "seq a", SeqType::Bigint, 1, None, None, 1) };
    let mut cur = create_and_check(&env, ObjectDef::Sequence(first)).await;
    assert_noop(&env, &cur);
    // 1) parametreler
    let mut n = as_seq(&cur);
    (n.increment, n.min_value, n.max_value, n.start, n.cache, n.cycle, n.comment) = (5, Some(-10), Some(5000), 10, 3, true, Some("c1".into()));
    cur = alter_and_check(&env, &cur, ObjectDef::Sequence(n), false).await;
    // 2) AS tür değişimi: smallint (sınırlar tür içinde), integer (varsayılan sınırlar), bigint
    let mut n = as_seq(&cur);
    n.data_type = SeqType::Smallint;
    cur = alter_and_check(&env, &cur, ObjectDef::Sequence(n), false).await;
    let mut n = as_seq(&cur);
    (n.data_type, n.min_value, n.max_value, n.increment, n.start) = (SeqType::Integer, None, None, 1, 1);
    cur = alter_and_check(&env, &cur, ObjectDef::Sequence(n), false).await;
    let mut n = as_seq(&cur);
    (n.data_type, n.max_value) = (SeqType::Bigint, Some(1_000_000_000_000));
    cur = alter_and_check(&env, &cur, ObjectDef::Sequence(n), false).await;
    // azalan yöne geçiş: artış işareti değişince üretici RESTART WITH ekler (sayaç yeni sınırların dışında kalamaz).
    let mut n = as_seq(&cur);
    (n.increment, n.min_value, n.max_value, n.start) = (-1, None, None, -1);
    cur = alter_and_check(&env, &cur, ObjectDef::Sequence(n), false).await;
    assert_noop(&env, &cur); // azalan, varsayılan sınırlı dizi: boş plan
    let mut n = as_seq(&cur);
    (n.increment, n.min_value, n.max_value, n.start) = (1, None, None, 1);
    cur = alter_and_check(&env, &cur, ObjectDef::Sequence(n), false).await;
    // 3) ad + şema + OWNED BY hedefi birlikte (NONE → rename → set schema → OWNED BY)
    let mut n = as_seq(&cur);
    (n.name, n.schema, n.owned_by) = ("seq b".into(), "rt s2".into(), Some(colref("rt s2", "tb2", "id")));
    cur = alter_and_check(&env, &cur, ObjectDef::Sequence(n), false).await;
    // 4) sahip + sahibi aynı olan tabloya bağlama
    let mut n = as_seq(&cur);
    (n.owner, n.owned_by) = (Some(role.clone()), Some(colref("rt s2", "tb3", "id")));
    cur = alter_and_check(&env, &cur, ObjectDef::Sequence(n), false).await;
    // 5) yalnız OWNED BY çözme + yorum kaldırma, sonra yeniden bağlama
    let mut n = as_seq(&cur);
    (n.owned_by, n.comment) = (None, None);
    cur = alter_and_check(&env, &cur, ObjectDef::Sequence(n), false).await;
    let mut n = as_seq(&cur);
    n.owned_by = Some(colref("rt s2", "tb3", "id"));
    cur = alter_and_check(&env, &cur, ObjectDef::Sequence(n), false).await;
    // 6) yalnız şema değişimi (bağlı), sahip değişimi yok
    let mut n = as_seq(&cur);
    env.sql(r#"CREATE TABLE "rt s1".tb4 (id int)"#).await;
    env.sql(&format!(r#"ALTER TABLE "rt s1".tb4 OWNER TO {role}"#)).await;
    (n.schema, n.owned_by) = ("rt s1".into(), Some(colref("rt s1", "tb4", "id")));
    cur = alter_and_check(&env, &cur, ObjectDef::Sequence(n), false).await;
    // 7) sahibi oturum rolüne geri alma (bağsız)
    let mut n = as_seq(&cur);
    (n.owner, n.owned_by) = (Some(me.clone()), None);
    cur = alter_and_check(&env, &cur, ObjectDef::Sequence(n), false).await;
    assert_eq!(as_seq(&cur).owner.as_deref(), Some(me.as_str()));

    // --- Extension: sürüm güncelleme + şema taşıma ---
    let mut schema_done = false;
    for ext in ["citext", "pgcrypto"] {
        if !env.has_extension(ext).await {
            eprintln!("{ext} yok, eklenti alter testi atlandı");
            continue;
        }
        if !schema_done {
            // Yalnız şema taşıma (sürüm yolundan bağımsız; PG 10'da da koşar) + boş plan.
            let def = ExtensionDef { name: ext.into(), schema: Some("public".into()), version: None, cascade: false };
            let old = create_and_check(&env, ObjectDef::Extension(def)).await;
            assert_noop(&env, &old);
            let mut n = as_ext(&old);
            n.schema = Some("rt s1".into());
            let moved = alter_and_check(&env, &old, ObjectDef::Extension(n), false).await;
            assert_eq!(as_ext(&moved).schema.as_deref(), Some("rt s1"));
            env.sql(&format!("DROP EXTENSION {ext}")).await;
            schema_done = true;
        }
        let row = env
            .admin
            .query_opt(
                "SELECT p.source::text, p.target::text FROM pg_extension_update_paths($1::name) p \
                 JOIN pg_available_extensions a ON a.name = $1::text AND a.default_version = p.target \
                 JOIN pg_available_extension_versions v ON v.name = $1::text AND v.version = p.source \
                 WHERE p.path IS NOT NULL ORDER BY p.source LIMIT 1",
                &[&ext],
            )
            .await
            .unwrap();
        let Some(row) = row else {
            eprintln!("{ext}: güncelleme yolu yok");
            continue;
        };
        let (source, target): (String, String) = (row.get(0), row.get(1));
        let def = ExtensionDef { name: ext.into(), schema: Some("public".into()), version: Some(source.clone()), cascade: false };
        let old = create_and_check(&env, ObjectDef::Extension(def)).await;
        let mut n = as_ext(&old);
        (n.version, n.schema) = (Some(target.clone()), Some("rt s1".into()));
        let loaded = alter_and_check(&env, &old, ObjectDef::Extension(n), false).await;
        assert_eq!(as_ext(&loaded).version.as_deref(), Some(target.as_str()));
        // Yalnız şema geri taşıma.
        let mut n = as_ext(&loaded);
        n.schema = Some("public".into());
        alter_and_check(&env, &loaded, ObjectDef::Extension(n), false).await;
        break;
    }

    // --- Index ---
    env.sql(r#"CREATE TABLE "rt s1"."idx t" (a text, b text, c int, d int)"#).await;
    env.sql(r#"CREATE TABLE "rt s1"."idx t2" (a text, c int, d int)"#).await;
    let t = "idx t";
    let base = IndexDef { comment: Some("c0".into()), ..idx("rt s1", t, "ix a", "btree", vec![col("a")]) };
    let mut cur = create_and_check(&env, ObjectDef::Index(base)).await;
    // yerinde: ad, tablespace, fillfactor (ekle/değiştir/sıfırla), yorum
    let mut n = as_idx(&cur);
    n.name = "ix b".into();
    cur = alter_and_check(&env, &cur, ObjectDef::Index(n), false).await;
    let mut n = as_idx(&cur);
    n.tablespace = Some("pg_default".into());
    n.fillfactor = Some(60);
    n.comment = None;
    cur = alter_and_check(&env, &cur, ObjectDef::Index(n), false).await;
    let mut n = as_idx(&cur);
    n.fillfactor = Some(80);
    n.comment = Some("it's".into());
    cur = alter_and_check(&env, &cur, ObjectDef::Index(n), false).await;
    let mut n = as_idx(&cur);
    (n.fillfactor, n.name) = (None, "Ix \"C\"".into());
    cur = alter_and_check(&env, &cur, ObjectDef::Index(n), false).await;
    assert_eq!(as_idx(&cur).fillfactor, None);
    // yapısal (DROP + CREATE): kolon, yön/null, benzersiz, ifade, WHERE
    let mut n = as_idx(&cur);
    n.columns = vec![col("a"), IndexColumn { descending: true, nulls_first: Some(false), ..col("c") }];
    cur = alter_and_check(&env, &cur, ObjectDef::Index(n), false).await;
    let mut n = as_idx(&cur);
    (n.unique, n.where_clause, n.fillfactor) = (true, Some("c > 10".into()), Some(70));
    cur = alter_and_check(&env, &cur, ObjectDef::Index(n), false).await;
    let mut n = as_idx(&cur);
    (n.columns, n.unique, n.where_clause) = (vec![expr("lower(a)")], false, None);
    cur = alter_and_check(&env, &cur, ObjectDef::Index(n), false).await;
    // yöntem değişimi + yeniden adlandırma birlikte
    let mut n = as_idx(&cur);
    (n.method, n.columns, n.fillfactor, n.name) = ("hash".into(), vec![col("a")], Some(80), "ix hash".into());
    cur = alter_and_check(&env, &cur, ObjectDef::Index(n), false).await;
    let mut n = as_idx(&cur);
    (n.method, n.fillfactor) = ("btree".into(), None);
    cur = alter_and_check(&env, &cur, ObjectDef::Index(n), false).await;
    // INCLUDE (PG 11+; PG 10'da üretici reddeder)
    if env.version >= 110000 {
        let mut n = as_idx(&cur);
        n.include = vec!["c".into(), "d".into()];
        cur = alter_and_check(&env, &cur, ObjectDef::Index(n), false).await;
        let mut n = as_idx(&cur);
        n.include = vec![];
        cur = alter_and_check(&env, &cur, ObjectDef::Index(n), false).await;
    } else {
        let mut n = as_idx(&cur);
        n.include = vec!["c".into()];
        assert!(build::alter(&cur, &ObjectDef::Index(n), &env.ctx(false)).is_err());
    }
    // tablo değişimi (başka tabloda yeniden oluşturma)
    let mut n = as_idx(&cur);
    (n.table, n.columns) = ("idx t2".into(), vec![col("a")]);
    cur = alter_and_check(&env, &cur, ObjectDef::Index(n), false).await;
    // CONCURRENTLY yeniden oluşturma (transaction dışı ifadeler)
    let mut n = as_idx(&cur);
    (n.columns, n.unique, n.name) = (vec![col("a"), col("c")], true, "ix conc".into());
    cur = alter_and_check(&env, &cur, ObjectDef::Index(n), true).await;
    let mut n = as_idx(&cur);
    n.where_clause = Some("d IS NOT NULL".into());
    cur = alter_and_check(&env, &cur, ObjectDef::Index(n), true).await;
    // değişiklik yok → boş plan
    let same = build::alter(&cur, &cur, &env.ctx(false)).unwrap();
    assert!(same.statements.is_empty());
}

// ---- kısıta bağlı indeks ----

#[tokio::test]
async fn constraint_backed_index_is_flagged() {
    for ts in servers() {
        guarded(constraint_body(ts.clone()), cleanup(&ts, "cons")).await;
    }
}

async fn constraint_body(ts: TestServer) {
    let env = setup(&ts, "cons").await;
    env.sql("CREATE TABLE parent (id int PRIMARY KEY, u int UNIQUE, v int)").await;
    env.sql("CREATE TABLE child (pid int REFERENCES parent(id), w int)").await;
    env.sql("CREATE INDEX plain_idx ON parent (v)").await;
    let load_idx = |name: &'static str| {
        let env = &env;
        async move {
            let oid = env.class_oid("public", name).await;
            as_idx(&load(&env.mgr, env.server, env.db_oid, ObjectRef { kind: ObjectKind::Index, oid }).await.unwrap())
        }
    };
    let pk = load_idx("parent_pkey").await;
    assert_eq!(pk.constraint.as_deref(), Some("parent_pkey"));
    assert!(pk.unique && pk.columns == vec![col("id")] && pk.method == "btree");
    let uq = load_idx("parent_u_key").await;
    assert_eq!(uq.constraint.as_deref(), Some("parent_u_key"));
    assert_eq!(load_idx("plain_idx").await.constraint, None);

    for def in [pk.clone(), uq.clone()] {
        let o = ObjectDef::Index(def.clone());
        let drop_err = build::drop(&o, false, &env.ctx(false)).unwrap_err();
        assert_eq!(drop_err[0].path, "constraint");
        let mut renamed = def.clone();
        renamed.name = "other".into();
        assert!(build::alter(&o, &ObjectDef::Index(renamed), &env.ctx(false)).is_err());
        let mut structural = def.clone();
        structural.columns = vec![col("v")];
        assert!(build::alter(&o, &ObjectDef::Index(structural), &env.ctx(false)).is_err());
    }
    // Kısıta bağlı indekste yerinde değişiklikler (fillfactor) serbest ve sunucuda geçerlidir.
    let mut ff = pk.clone();
    ff.fillfactor = Some(70);
    let plan = build::alter(&ObjectDef::Index(pk), &ObjectDef::Index(ff), &env.ctx(false)).unwrap();
    env.exec(&plan).await;
    let reloaded = load_idx("parent_pkey").await;
    assert_eq!((reloaded.fillfactor, reloaded.constraint.as_deref()), (Some(70), Some("parent_pkey")));
}

// ---- sistem nesneleri ----

#[tokio::test]
async fn system_objects_are_refused() {
    for ts in servers() {
        guarded(system_body(ts.clone()), cleanup(&ts, "sys")).await;
    }
}

async fn system_body(ts: TestServer) {
    let env = setup(&ts, "sys").await;
    let try_load = |kind, oid| {
        let env = &env;
        async move { load(&env.mgr, env.server, env.db_oid, ObjectRef { kind, oid }).await }
    };
    for name in ["pg_catalog", "information_schema", "pg_toast"] {
        let err = try_load(ObjectKind::Schema, env.schema_oid(name).await).await.unwrap_err();
        assert!(matches!(&err, Error::Invalid(m) if m.contains("sistem nesnesi")), "{name}: {err:?}");
    }
    let oid = env.class_oid("pg_catalog", "pg_class_oid_index").await;
    assert!(matches!(try_load(ObjectKind::Index, oid).await.unwrap_err(), Error::Invalid(_)));
    let oid = env.extension_oid("plpgsql").await;
    assert!(matches!(try_load(ObjectKind::Extension, oid).await.unwrap_err(), Error::Invalid(_)));
    // Kullanıcı nesnesi yüklenir; olmayan oid / yanlış tür NotFound.
    env.sql("CREATE TABLE t (id int); CREATE SEQUENCE sq").await;
    assert!(try_load(ObjectKind::Schema, env.schema_oid("public").await).await.is_ok());
    assert!(matches!(try_load(ObjectKind::Schema, 4_000_000_000).await.unwrap_err(), Error::NotFound(_)));
    assert!(matches!(try_load(ObjectKind::Sequence, 4_000_000_000).await.unwrap_err(), Error::NotFound(_)));
    assert!(matches!(try_load(ObjectKind::Extension, 4_000_000_000).await.unwrap_err(), Error::NotFound(_)));
    assert!(matches!(try_load(ObjectKind::Index, 4_000_000_000).await.unwrap_err(), Error::NotFound(_)));
    let table = env.class_oid("public", "t").await;
    assert!(matches!(try_load(ObjectKind::Index, table).await.unwrap_err(), Error::NotFound(_)));
    assert!(matches!(try_load(ObjectKind::Sequence, table).await.unwrap_err(), Error::NotFound(_)));
    assert!(try_load(ObjectKind::Sequence, env.class_oid("public", "sq").await).await.is_ok());
    // Kimlik (identity) dizisi tablo tanımından yönetilir; serial dizisi (OWNED BY) ise düzenlenebilir.
    env.sql("CREATE TABLE idt (id int GENERATED ALWAYS AS IDENTITY, s serial)").await;
    let err = try_load(ObjectKind::Sequence, env.class_oid("public", "idt_id_seq").await).await.unwrap_err();
    assert!(matches!(&err, Error::Invalid(m) if m.contains("identity")), "{err:?}");
    let serial = try_load(ObjectKind::Sequence, env.class_oid("public", "idt_s_seq").await).await.unwrap();
    assert!(matches!(&serial, ObjectDef::Sequence(s) if s.owned_by.is_some()));
}

// ---- açılır liste verileri ----

#[tokio::test]
async fn lookups_return_expected_shapes() {
    for ts in servers() {
        guarded(lookup_body(ts.clone()), cleanup(&ts, "look")).await;
    }
}

async fn lookup_body(ts: TestServer) {
    let env = setup(&ts, "look").await;
    env.sql(r#"CREATE SCHEMA "lk s"; CREATE TABLE "lk s"."Tbl" (id int, "Mixed Col" text, v numeric(10,2), a text[]); CREATE TABLE public.pt (x int)"#).await;
    let ask = |req| {
        let env = &env;
        async move { lookup(&env.mgr, env.server, env.db_oid, req).await.unwrap() }
    };
    assert_eq!(server_version(&env.mgr, env.server).await.unwrap(), env.version);
    assert!(matches!(server_version(&env.mgr, uuid::Uuid::new_v4()).await, Err(Error::NotConnected)));

    let roles = ask(LookupRequest::Roles).await;
    assert!(roles.iter().any(|r| r.value == env.role) && roles.iter().any(|r| r.value == env.me));
    assert!(roles.iter().all(|r| !r.value.starts_with("pg_")));

    let schemas = ask(LookupRequest::Schemas).await;
    let names: Vec<_> = schemas.iter().map(|s| s.value.as_str()).collect();
    assert!(names.contains(&"public") && names.contains(&"lk s"));
    assert!(names.iter().all(|n| !n.starts_with("pg_") && *n != "information_schema"), "{names:?}");

    let ts_items = ask(LookupRequest::Tablespaces).await;
    assert_eq!(ts_items[0].value, "pg_default");
    assert!(ts_items.iter().all(|t| t.value != "pg_global"));

    let exts = ask(LookupRequest::AvailableExtensions).await;
    let plpgsql = exts.iter().find(|e| e.value == "plpgsql").expect("plpgsql listede");
    let detail = plpgsql.detail.as_deref().unwrap();
    assert!(detail.starts_with("default ") && detail.contains(" · yüklü ") && !detail.contains("yüklü —"), "{detail}");
    let uninstalled = exts.iter().find(|e| e.value != "plpgsql").map(|e| e.detail.clone().unwrap());
    if let Some(d) = uninstalled {
        assert!(d.contains("yüklü —"), "{d}");
    }

    let methods: Vec<_> = ask(LookupRequest::IndexMethods).await.into_iter().map(|m| m.value).collect();
    assert_eq!(methods, ["btree", "hash", "gin", "gist", "brin", "spgist"]);

    let all_tables = ask(LookupRequest::Tables { schema: None }).await;
    assert!(all_tables.iter().any(|t| t.value == "Tbl" && t.detail.as_deref() == Some("lk s")));
    assert!(all_tables.iter().any(|t| t.value == "pt" && t.detail.as_deref() == Some("public")));
    assert!(all_tables.iter().all(|t| !t.detail.as_deref().unwrap().starts_with("pg_")));
    let only = ask(LookupRequest::Tables { schema: Some("lk s".into()) }).await;
    assert_eq!(only, vec![LookupItem { value: "Tbl".into(), detail: Some("lk s".into()) }]);

    let cols = ask(LookupRequest::TableColumns { schema: "lk s".into(), table: "Tbl".into() }).await;
    let got: Vec<_> = cols.iter().map(|c| (c.value.as_str(), c.detail.as_deref().unwrap())).collect();
    assert_eq!(got, [("id", "integer"), ("Mixed Col", "text"), ("v", "numeric(10,2)"), ("a", "text[]")]);
    assert!(ask(LookupRequest::TableColumns { schema: "lk s".into(), table: "yok".into() }).await.is_empty());
}
