//! Veri düzenleyici kaynak yükleyicisi gerçek sunucuda (Docker; `PGT_TEST_URLS` yoksa atlanır): kolonlar, satır kimliği
//! anahtarı, yetkiler, yazılabilirlik ve parmak izi.
//!
//! Her test kendi geçici veritabanını (`pgt_data_<ana sürüm>_<etiket>`) ve rolünü (`pgt_data_role_<etiket>`) kurar;
//! başarısız doğrulamadan sonra bile temizlenir. Sürüme bağlı adımlar (INCLUDE/bölümlü PK PG 11+, üretilmiş kolon
//! PG 12+, sanal üretilmiş kolon PG 18+) ana sürüme göre dallanır; atlanan kısım açıkça yazdırılır.

mod common;

use common::{Env, cleanup_in, guarded, servers, setup_in};
use pgt_core::Error;
use pgt_core::testing::TestServer;
use pgt_ddl::data::model::{DataColumn, DataSource, Identity, KeySource, ReadOnlyReason, RelKind};
use pgt_ddl::data::source::{NOT_VIEWABLE, load_source};
use pgt_ddl::{ObjectKind, ObjectRef, quote_ident};

const PFX: &str = "pgt_data";

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

fn major(env: &Env) -> u32 {
    env.version / 10_000
}

async fn try_src(env: &Env, kind: ObjectKind, oid: u32) -> pgt_core::Result<DataSource> {
    load_source(&env.admin, ObjectRef { kind, oid }, major(env)).await
}

async fn src_kind(env: &Env, kind: ObjectKind, schema: &str, name: &str) -> DataSource {
    let oid = env.class_oid(schema, name).await;
    let s = try_src(env, kind, oid).await.unwrap_or_else(|e| panic!("{schema}.{name} yüklenemedi: {e:?}"));
    // Değişmezler: oid/tür korunur, `locked` yükleyicide hep false, anahtar kolonları kaynak kolonlarındandır,
    // `editable` ⇔ neden yok, parmak izi 16 hane hex.
    assert_eq!(s.object, ObjectRef { kind, oid });
    assert_eq!((s.schema.as_str(), s.name.as_str()), (schema, name));
    assert!(!s.locked);
    assert_eq!(s.editable, s.read_only_reason.is_none(), "{name}");
    assert_eq!(s.fingerprint.len(), 16, "{name}");
    if let Some(k) = &s.key {
        for c in &k.columns {
            assert!(s.columns.iter().any(|col| &col.name == c), "{name}: anahtar kolonu {c} kaynakta yok");
        }
    }
    let attnums: Vec<i16> = s.columns.iter().map(|c| c.attnum).collect();
    assert!(attnums.windows(2).all(|w| w[0] < w[1]), "{name}: attnum sırası {attnums:?}");
    s
}

async fn src(env: &Env, name: &str) -> DataSource {
    src_kind(env, ObjectKind::Table, "public", name).await
}

fn key_cols(s: &DataSource) -> Option<(KeySource, String, Vec<String>)> {
    s.key.as_ref().map(|k| (k.source, k.name.clone(), k.columns.clone()))
}

fn k(source: KeySource, name: &str, cols: &[&str]) -> Option<(KeySource, String, Vec<String>)> {
    Some((source, name.to_string(), cols.iter().map(|c| c.to_string()).collect()))
}

fn col<'a>(s: &'a DataSource, name: &str) -> &'a DataColumn {
    s.columns.iter().find(|c| c.name == name).unwrap_or_else(|| panic!("{} kolonu yok", name))
}

fn names(s: &DataSource) -> Vec<&str> {
    s.columns.iter().map(|c| c.name.as_str()).collect()
}

// ================================================================================================================
// Anahtar seçimi
// ================================================================================================================

docker_test!(data_source_keys_follow_spec_rule, keys_body, "keys");

async fn keys_body(ts: TestServer) {
    let env = setup_in(&ts, PFX, "keys").await;
    let major = major(&env);
    use KeySource::{PrimaryKey as Pk, UniqueIndex as Uq};

    // PK'lı: daha küçük/önce gelen benzersiz indeks olsa da PK.
    env.sql("CREATE TABLE t_pk (a int NOT NULL, id int, b int NOT NULL, CONSTRAINT t_pk_pkey PRIMARY KEY (id, b));
             CREATE UNIQUE INDEX aa_small ON t_pk (a)")
        .await;
    let s = src(&env, "t_pk").await;
    assert_eq!(key_cols(&s), k(Pk, "t_pk_pkey", &["id", "b"]));
    assert_eq!((s.relkind, s.editable, s.read_only_reason), (RelKind::Table, true, None));
    assert!(s.can_select && s.can_insert && s.can_update && s.can_delete);

    // Çok kolonlu PK: kısıt sırası (tablo sırası değil).
    env.sql(r#"CREATE TABLE "t Multi" (a int, b int, c text, PRIMARY KEY (c, a))"#).await;
    let s = src(&env, "t Multi").await;
    assert_eq!(key_cols(&s), k(Pk, "t Multi_pkey", &["c", "a"]));

    // PK yok: kısmi/ifadeli/NULL'lanabilir/geçersiz benzersiz indeksler atlanır; en az kolonlu geçerli NOT NULL indeks.
    env.sql(
        "CREATE TABLE t_uq (a int NOT NULL, b int NOT NULL, c int NOT NULL, x int, d int NOT NULL);
         CREATE UNIQUE INDEX aa_partial ON t_uq (a) WHERE a > 0;
         CREATE UNIQUE INDEX ab_expr ON t_uq ((a + 1));
         CREATE UNIQUE INDEX ac_nullable ON t_uq (x);
         CREATE UNIQUE INDEX ad_wide ON t_uq (a, b);
         CREATE UNIQUE INDEX zz_narrow ON t_uq (c);
         CREATE UNIQUE INDEX zy_narrow ON t_uq (b);
         INSERT INTO t_uq VALUES (1, 1, 1, NULL, 7), (2, 2, 2, NULL, 7);",
    )
    .await;
    // Geçersiz indeks: yinelenen değerlerde başarısız CONCURRENTLY (transaction dışında) geride geçersiz indeks bırakır.
    let failed = env.try_sql("CREATE UNIQUE INDEX ab_invalid ON t_uq (d)").await;
    assert!(failed.is_err(), "yinelenen değerle benzersiz indeks kurulmamalıydı");
    let failed = env.try_sql("CREATE UNIQUE INDEX CONCURRENTLY ab_invalid ON t_uq (d)").await;
    assert!(failed.is_err());
    let invalid: bool = env
        .admin
        .query_one("SELECT NOT indisvalid FROM pg_index WHERE indexrelid = 'ab_invalid'::regclass", &[])
        .await
        .unwrap()
        .get(0);
    assert!(invalid, "ab_invalid geçersiz kalmalıydı");
    let s = src(&env, "t_uq").await;
    // b ve c tek kolonlu; eşitlikte ad sırası: "zy_narrow" < "zz_narrow".
    assert_eq!(key_cols(&s), k(Uq, "zy_narrow", &["b"]));
    assert_eq!((s.editable, s.read_only_reason), (true, None));

    // Yalnız UNIQUE (nullable) → anahtarsız.
    env.sql("CREATE TABLE t_nullable (x int UNIQUE, y text)").await;
    let s = src(&env, "t_nullable").await;
    assert_eq!(s.key, None);
    assert_eq!((s.editable, s.read_only_reason), (false, Some(ReadOnlyReason::NoKey)));

    // UNIQUE NOT NULL kolon (kısıt olarak) → anahtar indeks adıyla.
    env.sql("CREATE TABLE t_uqnn (x int NOT NULL UNIQUE, y text)").await;
    let s = src(&env, "t_uqnn").await;
    assert_eq!(key_cols(&s), k(Uq, "t_uqnn_x_key", &["x"]));

    // INCLUDE: anahtara girmez (PG 11+). NULL'lanabilir INCLUDE kolonu indeksi geçersiz kılmaz.
    if major >= 11 {
        env.sql(
            "CREATE TABLE t_incl (a int, b int, c text, PRIMARY KEY (a) INCLUDE (b, c));
             CREATE TABLE t_uincl (a int NOT NULL, b int, c text);
             CREATE UNIQUE INDEX t_uincl_idx ON t_uincl (a) INCLUDE (b, c);",
        )
        .await;
        assert_eq!(key_cols(&src(&env, "t_incl").await), k(Pk, "t_incl_pkey", &["a"]));
        assert_eq!(key_cols(&src(&env, "t_uincl").await), k(Uq, "t_uincl_idx", &["a"]));
    } else {
        eprintln!("PG {major}: INCLUDE (PG 11+) adımı atlandı");
    }

    // Bölümlü tablo: PG 11+ PK; PG 10'da bölümlü tabloda PK/benzersiz indeks yok → anahtarsız.
    if major >= 11 {
        env.sql(
            "CREATE TABLE t_part (id int, k int, v text, PRIMARY KEY (id, k)) PARTITION BY RANGE (k);
             CREATE TABLE t_part_1 PARTITION OF t_part FOR VALUES FROM (0) TO (100);",
        )
        .await;
        let s = src(&env, "t_part").await;
        assert_eq!(s.relkind, RelKind::Partitioned);
        assert_eq!(key_cols(&s), k(Pk, "t_part_pkey", &["id", "k"]));
        assert_eq!((s.editable, s.read_only_reason), (true, None));
        let s = src(&env, "t_part_1").await;
        assert_eq!(s.relkind, RelKind::Table);
        assert_eq!(s.key.as_ref().map(|k| (k.source, k.columns.clone())), Some((Pk, vec!["id".into(), "k".into()])));
        assert_eq!(names(&s), ["id", "k", "v"]);
    } else {
        env.sql(
            "CREATE TABLE t_part (id int, k int, v text) PARTITION BY RANGE (k);
             CREATE TABLE t_part_1 PARTITION OF t_part FOR VALUES FROM (0) TO (100);",
        )
        .await;
        let s = src(&env, "t_part").await;
        assert_eq!(s.relkind, RelKind::Partitioned);
        assert_eq!((s.key.clone(), s.read_only_reason), (None, Some(ReadOnlyReason::NoKey)));
        eprintln!("PG {major}: bölümlü tabloda PK (PG 11+) adımı atlandı; anahtarsız bölümlü tablo denetlendi");
    }

    // Kalıtım çocuğu: PK kalıtılmaz → anahtarsız; ebeveyn kolonları önce.
    env.sql(
        "CREATE TABLE t_parent (id int PRIMARY KEY, v text);
         CREATE TABLE t_child (extra int) INHERITS (t_parent);",
    )
    .await;
    let s = src(&env, "t_parent").await;
    assert_eq!(key_cols(&s), k(Pk, "t_parent_pkey", &["id"]));
    let s = src(&env, "t_child").await;
    assert_eq!(s.relkind, RelKind::Table);
    assert_eq!(names(&s), ["id", "v", "extra"]);
    assert_eq!((s.key.clone(), s.read_only_reason), (None, Some(ReadOnlyReason::NoKey)));
    // Çocuğa kendi PK'sı eklenince anahtar gelir.
    env.sql("ALTER TABLE t_child ADD PRIMARY KEY (id)").await;
    assert_eq!(key_cols(&src(&env, "t_child").await), k(Pk, "t_child_pkey", &["id"]));
}

// ================================================================================================================
// Kolonlar: tipler, kimlik, üretilmiş, yazılabilirlik
// ================================================================================================================

docker_test!(data_source_columns_types_identity_generated, cols_body, "cols");

async fn cols_body(ts: TestServer) {
    let env = setup_in(&ts, PFX, "cols").await;
    let major = major(&env);
    env.sql(
        r#"CREATE TABLE t_cols (
             id int GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
             d bigint GENERATED BY DEFAULT AS IDENTITY,
             n numeric(10,2) NOT NULL DEFAULT 0,
             arr text[],
             "Weird ""Col""" varchar(20),
             dropped int,
             plain text
           );
           ALTER TABLE t_cols DROP COLUMN dropped;"#,
    )
    .await;
    if major >= 12 {
        env.sql("ALTER TABLE t_cols ADD COLUMN g bigint GENERATED ALWAYS AS (d * 2) STORED").await;
    } else {
        eprintln!("PG {major}: üretilmiş kolon (PG 12+) adımı atlandı");
    }
    if major >= 18 {
        env.sql("ALTER TABLE t_cols ADD COLUMN vg bigint GENERATED ALWAYS AS (d * 3) VIRTUAL").await;
    } else {
        eprintln!("PG {major}: sanal üretilmiş kolon (PG 18+) adımı atlandı");
    }
    let s = src(&env, "t_cols").await;
    let mut want = vec!["id", "d", "n", "arr", "Weird \"Col\"", "plain"];
    if major >= 12 {
        want.push("g");
    }
    if major >= 18 {
        want.push("vg");
    }
    assert_eq!(names(&s), want, "silinen kolon görünmemeli, attnum sırası");

    let id = col(&s, "id");
    assert_eq!((id.type_oid, id.type_display.as_str()), (23, "integer"));
    assert_eq!((id.identity, id.generated, id.not_null, id.has_default), (Identity::Always, false, true, false));
    assert_eq!((id.insertable, id.updatable), (true, false), "GENERATED ALWAYS kimlik UPDATE edilemez");

    let d = col(&s, "d");
    assert_eq!((d.type_oid, d.type_display.as_str()), (20, "bigint"));
    assert_eq!((d.identity, d.insertable, d.updatable), (Identity::ByDefault, true, true));

    let n = col(&s, "n");
    assert_eq!((n.type_oid, n.type_display.as_str()), (1700, "numeric(10,2)"));
    assert_eq!((n.not_null, n.has_default, n.identity), (true, true, Identity::None));

    let arr = col(&s, "arr");
    assert_eq!((arr.type_oid, arr.type_display.as_str()), (1009, "text[]"));
    assert_eq!((arr.not_null, arr.has_default), (false, false));

    let w = col(&s, "Weird \"Col\"");
    assert_eq!((w.type_oid, w.type_display.as_str()), (1043, "character varying(20)"));

    if major >= 12 {
        let g = col(&s, "g");
        assert_eq!((g.generated, g.insertable, g.updatable, g.has_default), (true, false, false, true));
    }
    if major >= 18 {
        let vg = col(&s, "vg");
        assert_eq!((vg.generated, vg.insertable, vg.updatable), (true, false, false));
    }
    for c in &s.columns {
        if !c.generated {
            assert!(c.insertable, "{} yazılabilir olmalı", c.name);
        }
    }
    assert_eq!((s.editable, s.read_only_reason), (true, None));
}

// ================================================================================================================
// Türler: görünüm, MV, sistem tablosu, desteklenmeyen nesneler
// ================================================================================================================

docker_test!(data_source_kinds_and_system_objects, kinds_body, "kinds");

async fn kinds_body(ts: TestServer) {
    let env = setup_in(&ts, PFX, "kinds").await;
    env.sql(
        "CREATE TABLE t (id int PRIMARY KEY, v text);
         CREATE VIEW v_t AS SELECT id, v FROM t;
         CREATE MATERIALIZED VIEW mv_t AS SELECT id, v FROM t;
         CREATE UNIQUE INDEX mv_t_id ON mv_t (id);
         CREATE SEQUENCE seq_t;",
    )
    .await;

    let s = src_kind(&env, ObjectKind::View, "public", "v_t").await;
    assert_eq!(s.relkind, RelKind::View);
    assert_eq!(names(&s), ["id", "v"]);
    assert_eq!(s.key, None);
    assert_eq!((s.editable, s.read_only_reason), (false, Some(ReadOnlyReason::UnsupportedKind)));
    assert!(s.can_select);

    let s = src_kind(&env, ObjectKind::MaterializedView, "public", "mv_t").await;
    assert_eq!(s.relkind, RelKind::MatView);
    assert_eq!(names(&s), ["id", "v"]);
    // MV kolonları hiç `attnotnull` değildir → benzersiz indeksi de anahtar olamaz; tür önceliği yine UnsupportedKind.
    assert_eq!(s.key, None);
    assert_eq!((s.editable, s.read_only_reason), (false, Some(ReadOnlyReason::UnsupportedKind)));

    // Sistem tablosu: okunur, düzenlenemez.
    let s = src_kind(&env, ObjectKind::Table, "pg_catalog", "pg_class").await;
    assert_eq!(s.relkind, RelKind::Table);
    assert!(names(&s).contains(&"relname"));
    assert_eq!((s.editable, s.read_only_reason), (false, Some(ReadOnlyReason::SystemObject)));

    // Desteklenmeyen tür ve relkind.
    let seq = env.class_oid("public", "seq_t").await;
    let idx = env.class_oid("public", "t_pkey").await;
    for (kind, oid) in [(ObjectKind::Sequence, seq), (ObjectKind::Table, seq), (ObjectKind::Table, idx), (ObjectKind::Index, idx)] {
        match try_src(&env, kind, oid).await {
            Err(Error::Invalid(m)) => assert_eq!(m, NOT_VIEWABLE),
            other => panic!("{kind:?}: Invalid bekleniyordu: {other:?}"),
        }
    }
    let schema = env.schema_oid("public").await;
    assert!(matches!(try_src(&env, ObjectKind::Schema, schema).await, Err(Error::Invalid(_))));
    // Var olmayan oid.
    let missing: u32 = env.admin.query_one("SELECT max(oid)::oid FROM pg_class", &[]).await.unwrap().get::<_, u32>(0) + 1_000_000;
    assert!(matches!(try_src(&env, ObjectKind::Table, missing).await, Err(Error::NotFound(_))));
}

// ================================================================================================================
// Yetkiler
// ================================================================================================================

docker_test!(data_source_privileges_of_current_role, privs_body, "privs");

async fn privs_body(ts: TestServer) {
    let env = setup_in(&ts, PFX, "privs").await;
    let role = quote_ident(&env.role);
    env.sql(&format!(
        "CREATE TABLE t_priv (id int PRIMARY KEY, v text);
         GRANT USAGE ON SCHEMA public TO {role};
         GRANT SELECT ON t_priv TO {role};"
    ))
    .await;

    // Yalnız SELECT: hiçbir yazma yetkisi yok.
    let s = as_role(&env, &role).await;
    assert!(s.can_select);
    assert_eq!((s.can_insert, s.can_update, s.can_delete), (false, false, false));
    for c in &s.columns {
        assert_eq!((c.insertable, c.updatable), (false, false), "{}", c.name);
    }
    assert!(s.key.is_some());
    assert_eq!((s.editable, s.read_only_reason), (false, Some(ReadOnlyReason::NoPrivilege)));
    let fp_select_only = s.fingerprint.clone();

    // Kolon düzeyi UPDATE (v): kolon yazılabilir, tablo düzeyi `can_update` false kalır (has_table_privilege).
    env.sql(&format!("GRANT UPDATE (v) ON t_priv TO {role}")).await;
    let s = as_role(&env, &role).await;
    assert!(!s.can_update);
    assert_eq!((col(&s, "id").updatable, col(&s, "v").updatable), (false, true));
    assert_eq!(s.read_only_reason, Some(ReadOnlyReason::NoPrivilege));

    // Tablo düzeyi INSERT: tüm kolonlar insertable, kaynak yazılabilir.
    env.sql(&format!("GRANT INSERT ON t_priv TO {role}")).await;
    let s = as_role(&env, &role).await;
    assert_eq!((s.can_insert, s.can_update, s.can_delete), (true, false, false));
    assert!(s.columns.iter().all(|c| c.insertable));
    assert_eq!((s.editable, s.read_only_reason), (true, None));
    // Yetki değişimi parmak izini bozmaz.
    assert_eq!(s.fingerprint, fp_select_only);

    // Süper kullanıcı (yönetici bağlantısı) hepsini görür.
    let s = src(&env, "t_priv").await;
    assert!(s.can_insert && s.can_update && s.can_delete);
    assert_eq!(s.fingerprint, fp_select_only);
}

/// `SET ROLE` altında yükler (has_*_privilege geçerli kullanıcıya bakar), sonra `RESET ROLE`.
async fn as_role(env: &Env, role: &str) -> DataSource {
    env.sql(&format!("SET ROLE {role}")).await;
    let oid = env.class_oid("public", "t_priv").await;
    let s = try_src(env, ObjectKind::Table, oid).await;
    env.sql("RESET ROLE").await;
    s.unwrap_or_else(|e| panic!("rol altında yüklenemedi: {e:?}"))
}

// ================================================================================================================
// Parmak izi
// ================================================================================================================

docker_test!(data_source_fingerprint_tracks_definition, fp_body, "fp");

async fn fp_body(ts: TestServer) {
    let env = setup_in(&ts, PFX, "fp").await;
    env.sql("CREATE TABLE t_fp (id int PRIMARY KEY, v text)").await;
    let base = src(&env, "t_fp").await.fingerprint;
    assert_eq!(src(&env, "t_fp").await.fingerprint, base, "kararlı");

    env.sql("COMMENT ON TABLE t_fp IS 'yorum'; COMMENT ON COLUMN t_fp.v IS 'kolon yorumu'").await;
    assert_eq!(src(&env, "t_fp").await.fingerprint, base, "COMMENT parmak izini değiştirmez");
    env.sql("ALTER TABLE t_fp ALTER COLUMN v SET DEFAULT 'x'").await;
    assert_eq!(src(&env, "t_fp").await.fingerprint, base, "varsayılan parmak izine girmez");

    env.sql("ALTER TABLE t_fp ADD COLUMN w int").await;
    let added = src(&env, "t_fp").await.fingerprint;
    assert_ne!(added, base, "ADD COLUMN parmak izini değiştirir");

    env.sql("ALTER TABLE t_fp ALTER COLUMN w TYPE bigint").await;
    let retyped = src(&env, "t_fp").await.fingerprint;
    assert_ne!(retyped, added, "tip değişimi");

    env.sql("ALTER TABLE t_fp DROP CONSTRAINT t_fp_pkey").await;
    let s = src(&env, "t_fp").await;
    assert_eq!(s.key, None);
    assert_ne!(s.fingerprint, retyped, "anahtar düştü");

    env.sql("ALTER TABLE t_fp RENAME TO t_fp2").await;
    let renamed = src(&env, "t_fp2").await.fingerprint;
    assert_ne!(renamed, s.fingerprint, "ad değişimi");
}
