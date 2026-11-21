//! FDW / publication / subscription özellikleri ve DDL'i (Plan 38 Task 2): Karar 3 satırları, Karar 4 DDL biçimi,
//! parola maskesi (sır hiçbir çıktıda ve günlükte yok), yetkisiz rol notları, maskesiz DDL'in çalıştırılabilirliği.

mod common;

use std::io::Write;
use std::sync::{Arc, Mutex, OnceLock};

use common::foreign::{SECRET, setup};
use common::{Fixture, find, fixtures};
use pgt_catalog::node::{Coll, NodeId};
use pgt_catalog::props::PropertySection;
use pgt_core::testing::admin_client;

const PREFIX: &str = "pgt_cat_fdw_obj";

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

/// Süreç genelinde günlük yakalama (tüm düzeyler): sır günlüğe de girmemeli.
fn capture() -> Buf {
    static BUF: OnceLock<Buf> = OnceLock::new();
    BUF.get_or_init(|| {
        let buf = Buf::default();
        let w = buf.clone();
        let subscriber = tracing_subscriber::fmt().with_writer(move || w.clone()).with_max_level(tracing::Level::TRACE).finish();
        tracing::subscriber::set_global_default(subscriber).expect("bu test ikilisinde başka global abone yok");
        buf
    })
    .clone()
}

macro_rules! snapshot {
    ($fx:expr, $name:expr, $ddl:expr) => {{
        let ddl_text = $fx.normalize(&$ddl);
        insta::assert_snapshot!($name, ddl_text);
    }};
    ($fx:expr, $name:expr, $ddl:expr, per_version) => {{
        let major = $fx.major().await;
        let ddl_text = $fx.normalize(&$ddl);
        insta::with_settings!({ snapshot_suffix => format!("pg{major}") }, {
            insta::assert_snapshot!($name, ddl_text);
        });
    }};
}

/// `(bölüm, etiket, değer)` üçlüleri; OID değeri `<oid>` olarak.
fn rows(sections: &[PropertySection]) -> Vec<(String, String, String)> {
    let mut out = Vec::new();
    for s in sections {
        for r in &s.rows {
            let value = if r.label == "OID" { "<oid>".to_string() } else { r.value.clone() };
            out.push((s.title.clone(), r.label.clone(), value));
        }
    }
    out
}

fn value<'a>(sections: &'a [PropertySection], label: &str) -> Option<&'a str> {
    sections.iter().flat_map(|s| &s.rows).find(|r| r.label == label).map(|r| r.value.as_str())
}

fn no_secret(what: &str, text: &str) {
    assert!(!text.contains(SECRET), "sır {what} çıktısında göründü:\n{text}");
}

async fn props(fx: &Fixture, id: &NodeId) -> Vec<PropertySection> {
    // Kutulu: katalog gelecekleri test geleceğine gömülmez (hata ayıklama derlemesinde yığın taşması).
    let p = Box::pin(fx.catalog.properties(id)).await.unwrap();
    no_secret("özellik", &serde_json::to_string(&p).unwrap());
    p
}

async fn ddl(fx: &Fixture, id: &NodeId) -> String {
    let d = Box::pin(fx.catalog.ddl(id)).await.unwrap();
    no_secret("DDL", &d);
    d
}

async fn nodes(fx: &Fixture, parent: &NodeId, coll: Coll) -> Vec<pgt_catalog::node::TreeNode> {
    Box::pin(fx.nodes_of(parent, coll)).await
}

async fn child(fx: &Fixture, parent: &NodeId, coll: Coll, label: &str) -> NodeId {
    Box::pin(fx.child_of(parent, coll, label)).await
}

fn id(node: &pgt_catalog::node::TreeNode) -> NodeId {
    NodeId::parse(&node.id).unwrap()
}

fn t(section: &str, label: &str, value: &str) -> (String, String, String) {
    (section.into(), label.into(), value.into())
}

#[tokio::test]
async fn foreign_and_replication_properties_and_ddl_mask_secrets() {
    let log = capture();
    for fx in fixtures().await {
        // Büyük test geleceği yığında değil yığın belleğinde (hata ayıklama derlemesinde yığın taşması).
        Box::pin(check(&fx)).await;
        // Sırayla (aynı test içinde): abonelik adları küme geneli `pg_stat_subscription`'da eşlendiği için paralel
        // kurulumların `pgt_sub`'ları PG 10–13 yetkisiz yolunu belirsizleştirirdi.
        Box::pin(check_same_name(&fx)).await;
    }
    let out = String::from_utf8_lossy(&log.0.lock().unwrap()).into_owned();
    no_secret("günlük", &out);
}

async fn check(fx: &Fixture) {
    let major = fx.major().await;
    let owner = fx.ts.config.username.clone();
    let st = Box::pin(setup(fx, PREFIX)).await;
    let role = st.role.clone();
    let db = admin_client(&fx.ts, &st.db).await;
    let mut extra = format!(
        "COMMENT ON FOREIGN DATA WRAPPER pgt_plain_fdw IS 'düz fdw';
         GRANT USAGE ON FOREIGN DATA WRAPPER pgt_plain_fdw TO {role};
         CREATE FOREIGN DATA WRAPPER pgt_opt_fdw OPTIONS (mode 'x', password '{SECRET}');
         CREATE SERVER pgt_srv_secret FOREIGN DATA WRAPPER pgt_opt_fdw OPTIONS (password '{SECRET}', host 'h');
         COMMENT ON SERVER pgt_srv IS 'uzak';
         CREATE SERVER pgt_srv_rt TYPE 'pg' VERSION '16' FOREIGN DATA WRAPPER postgres_fdw OPTIONS (host 'rt', port '5433');
         COMMENT ON SERVER pgt_srv_rt IS 'gidiş-dönüş';
         GRANT USAGE ON FOREIGN SERVER pgt_srv_rt TO {role} WITH GRANT OPTION;
         CREATE USER MAPPING FOR {role} SERVER pgt_srv_rt OPTIONS (user 'r');
         CREATE FOREIGN TABLE public.ft_rt (
             id int NOT NULL,
             label text OPTIONS (column_name 'lbl') COLLATE \"C\" DEFAULT 'x',
             CONSTRAINT ft_rt_id_check CHECK (id > 0)
         ) SERVER pgt_srv_rt OPTIONS (schema_name 'public', table_name 'rt');
         COMMENT ON FOREIGN TABLE public.ft_rt IS 'yabancı';
         COMMENT ON COLUMN public.ft_rt.label IS 'etiket';
         GRANT SELECT ON public.ft_rt TO {role};
         GRANT UPDATE (label) ON public.ft_rt TO {role};
         CREATE TABLE public.pub_t2 (id int PRIMARY KEY, v text, w text);
         CREATE PUBLICATION pgt_pub_empty;
         COMMENT ON PUBLICATION pgt_pub_tbl IS 'tablo yayını';
         COMMENT ON SUBSCRIPTION pgt_sub IS 'abonelik';
         CREATE SUBSCRIPTION pgt_sub_uri CONNECTION 'postgresql://u:{SECRET}@127.0.0.1:1/x?sslmode=disable' PUBLICATION a, b
             WITH (connect = false, slot_name = NONE, synchronous_commit = 'local');"
    );
    // Sürüme göre publication seçenekleri: kolon listesi + WHERE + TABLES IN SCHEMA (15+), via root (13+), gencols (18+).
    extra.push_str(&match major {
        // Kolon listesi ayrı yayında: PG, TABLES IN SCHEMA içeren yayında kolon listesine izin vermez.
        18.. => "CREATE SCHEMA pgt_s2; CREATE PUBLICATION pgt_pub_rt FOR TABLE public.pub_t2 WHERE (id > 10), TABLES IN SCHEMA pgt_s2 \
                 WITH (publish = 'insert, update', publish_via_partition_root = true, publish_generated_columns = stored);
                 CREATE PUBLICATION pgt_pub_cols FOR TABLE public.pub_t2 (id, v) WHERE (id > 10), public.pub_t;"
            .to_string(),
        15.. => "CREATE SCHEMA pgt_s2; CREATE PUBLICATION pgt_pub_rt FOR TABLE public.pub_t2 WHERE (id > 10), TABLES IN SCHEMA pgt_s2 \
                 WITH (publish = 'insert, update', publish_via_partition_root = true);
                 CREATE PUBLICATION pgt_pub_cols FOR TABLE public.pub_t2 (id, v) WHERE (id > 10), public.pub_t;"
            .to_string(),
        13.. => "CREATE PUBLICATION pgt_pub_rt FOR TABLE public.pub_t2 WITH (publish = 'insert, update', publish_via_partition_root = true);".to_string(),
        _ => "CREATE PUBLICATION pgt_pub_rt FOR TABLE public.pub_t2 WITH (publish = 'insert, update');".to_string(),
    });
    db.batch_execute(&extra).await.unwrap();
    let db_id = st.db_id.clone();

    // --- FDW ---
    let fdws = nodes(fx, &db_id, Coll::ForeignDataWrappers).await;
    let pg_fdw = id(find(&fdws, "postgres_fdw"));
    assert_eq!(
        rows(&props(fx, &pg_fdw).await),
        [
            t("Genel", "Ad", "postgres_fdw"),
            t("Genel", "OID", "<oid>"),
            t("Genel", "Sahip", &owner),
            t("Tanım", "Handler", "public.postgres_fdw_handler"),
            t("Tanım", "Validator", "public.postgres_fdw_validator"),
            t("Tanım", "Eklenti", "postgres_fdw"),
        ]
    );
    snapshot!(fx, "fdw_postgres_fdw", ddl(fx, &pg_fdw).await);
    let plain_fdw = id(find(&fdws, "pgt_plain_fdw"));
    assert_eq!(value(&props(fx, &plain_fdw).await, "Yorum"), Some("düz fdw"));
    snapshot!(fx, "fdw_plain", ddl(fx, &plain_fdw).await);
    let opt_fdw = id(find(&fdws, "pgt_opt_fdw"));
    assert_eq!(value(&props(fx, &opt_fdw).await, "Seçenekler"), Some("mode=x, password=********"));
    snapshot!(fx, "fdw_masked_options", ddl(fx, &opt_fdw).await);

    // --- Server ---
    let servers = nodes(fx, &pg_fdw, Coll::ForeignServers).await;
    let srv = id(find(&servers, "pgt_srv"));
    assert_eq!(
        rows(&props(fx, &srv).await),
        [
            t("Genel", "Ad", "pgt_srv"),
            t("Genel", "OID", "<oid>"),
            t("Genel", "Sahip", &owner),
            t("Genel", "Yorum", "uzak"),
            t("Tanım", "FDW", "postgres_fdw"),
            t("Tanım", "Seçenekler", "host=x, dbname=y"),
        ]
    );
    let srv_rt = id(find(&servers, "pgt_srv_rt"));
    let srv_rt_ddl = ddl(fx, &srv_rt).await;
    snapshot!(fx, "server_typed", srv_rt_ddl);
    let opt_servers = nodes(fx, &opt_fdw, Coll::ForeignServers).await;
    let srv_secret = id(find(&opt_servers, "pgt_srv_secret"));
    assert_eq!(value(&props(fx, &srv_secret).await, "Seçenekler"), Some("password=********, host=h"));
    let srv_secret_ddl = ddl(fx, &srv_secret).await;
    assert!(srv_secret_ddl.contains("OPTIONS (password '********', host 'h')"), "{srv_secret_ddl}");
    assert!(srv_secret_ddl.contains("-- Parola seçeneği gizlenmiştir."), "{srv_secret_ddl}");

    // --- User mapping ---
    let maps = nodes(fx, &srv, Coll::UserMappings).await;
    let um_owner = id(find(&maps, &owner));
    assert_eq!(
        rows(&props(fx, &um_owner).await),
        [t("Genel", "Rol", &owner), t("Genel", "OID", "<oid>"), t("Genel", "Sunucu", "pgt_srv"), t("Tanım", "Seçenekler", "user=u, password=********")]
    );
    snapshot!(fx, "user_mapping_owner", ddl(fx, &um_owner).await);
    let um_public = id(find(&maps, "PUBLIC"));
    let um_public_ddl = ddl(fx, &um_public).await;
    assert!(um_public_ddl.contains("CREATE USER MAPPING FOR PUBLIC SERVER pgt_srv\n    OPTIONS (\"user\" 'u', password '********');"), "{um_public_ddl}");

    // --- Foreign table ---
    let public = child(fx, &db_id, Coll::Schemas, "public").await;
    let ftables = nodes(fx, &public, Coll::ForeignTables).await;
    let ft = id(find(&ftables, "ft_rt"));
    assert_eq!(
        rows(&props(fx, &ft).await),
        [
            t("Genel", "Ad", "ft_rt"),
            t("Genel", "Şema", "public"),
            t("Genel", "OID", "<oid>"),
            t("Genel", "Sahip", &owner),
            t("Genel", "Yorum", "yabancı"),
            t("Tanım", "Sunucu", "pgt_srv_rt"),
            t("Tanım", "Seçenekler", "schema_name=public, table_name=rt"),
            t("Tanım", "Kolon sayısı", "2"),
        ]
    );
    let ft_ddl = ddl(fx, &ft).await;
    // Sürüme göre: sahibin açık ACL satırında PG 17+ `MAINTAIN` yetkisi.
    snapshot!(fx, "foreign_table", ft_ddl, per_version);
    // Kolonlar: özellikler ve `ALTER FOREIGN TABLE … ADD COLUMN` (kolon seçenekleriyle).
    let cols = nodes(fx, &ft, Coll::Columns).await;
    let label_col = id(find(&cols, "label"));
    let col_props = props(fx, &label_col).await;
    assert_eq!((value(&col_props, "Veri tipi"), value(&col_props, "Yorum")), (Some("text"), Some("etiket")));
    snapshot!(fx, "foreign_table_column", ddl(fx, &label_col).await);

    // --- Publication ---
    let pubs = nodes(fx, &db_id, Coll::Publications).await;
    let pub_all = id(find(&pubs, "pgt_pub_all"));
    let mut expected = vec![t("Genel", "Ad", "pgt_pub_all"), t("Genel", "OID", "<oid>"), t("Genel", "Sahip", &owner), t("Tanım", "Tüm tablolar", "Evet")];
    expected.push(t("Tanım", "Yayınlanan işlemler", if major >= 11 { "insert, update, delete, truncate" } else { "insert, update, delete" }));
    if major >= 13 {
        expected.push(t("Tanım", "publish_via_partition_root", "Hayır"));
    }
    if major >= 18 {
        expected.push(t("Tanım", "publish_generated_columns", "none"));
    }
    assert_eq!(rows(&props(fx, &pub_all).await), expected);
    let pub_rt = id(find(&pubs, "pgt_pub_rt"));
    let pub_rt_props = props(fx, &pub_rt).await;
    assert_eq!(value(&pub_rt_props, "Yayınlanan işlemler"), Some("insert, update"));
    assert_eq!(value(&pub_rt_props, "Tablo sayısı"), Some("1"));
    assert_eq!(value(&pub_rt_props, "publish_via_partition_root"), (major >= 13).then_some("Evet"));
    assert_eq!(value(&pub_rt_props, "Şemalar"), (major >= 15).then_some("pgt_s2"));
    assert_eq!(value(&pub_rt_props, "publish_generated_columns"), (major >= 18).then_some("stored"));
    let pub_rt_ddl = ddl(fx, &pub_rt).await;
    if major >= 15 {
        assert!(pub_rt_ddl.contains("FOR TABLE public.pub_t2 WHERE (id > 10), TABLES IN SCHEMA pgt_s2\n"), "{pub_rt_ddl}");
        let pub_cols = id(find(&pubs, "pgt_pub_cols"));
        assert_eq!(value(&props(fx, &pub_cols).await, "Tablo sayısı"), Some("2"));
        let pub_cols_ddl = ddl(fx, &pub_cols).await;
        assert!(pub_cols_ddl.contains("FOR TABLE public.pub_t, public.pub_t2 (id, v) WHERE (id > 10)\n"), "{pub_cols_ddl}");
        snapshot!(fx, "publication_columns", pub_cols_ddl, per_version);
    }
    if major >= 13 {
        assert!(pub_rt_ddl.contains("publish_via_partition_root = true"), "{pub_rt_ddl}");
    }
    snapshot!(fx, "publication_options", pub_rt_ddl, per_version);
    let pub_all_ddl = ddl(fx, &pub_all).await;
    snapshot!(fx, "publication_all_tables", pub_all_ddl, per_version);
    let pub_tbl = id(find(&pubs, "pgt_pub_tbl"));
    assert_eq!(value(&props(fx, &pub_tbl).await, "Yorum"), Some("tablo yayını"));
    let pub_empty = id(find(&pubs, "pgt_pub_empty"));
    let pub_empty_ddl = ddl(fx, &pub_empty).await;
    assert!(pub_empty_ddl.contains("CREATE PUBLICATION pgt_pub_empty\n    WITH (publish = "), "{pub_empty_ddl}");

    // --- Subscription ---
    let subs = nodes(fx, &db_id, Coll::Subscriptions).await;
    let sub = id(find(&subs, "pgt_sub"));
    let sub_props = props(fx, &sub).await;
    let mut expected = vec![
        t("Genel", "Ad", "pgt_sub"),
        t("Genel", "OID", "<oid>"),
        t("Genel", "Sahip", &owner),
        t("Genel", "Yorum", "abonelik"),
        t("Tanım", "Etkin", "Hayır"),
        t("Tanım", "Yayınlar", "pgt_pub_tbl"),
        t("Tanım", "Slot adı", "NONE"),
        t("Tanım", "synchronous_commit", "off"),
        t("Tanım", "Bağlantı", "host=127.0.0.1 port=1 dbname=x user=u password=********"),
    ];
    if major >= 14 {
        expected.push(t("Tanım", "binary", "Hayır"));
        expected.push(t("Tanım", "streaming", if major >= 18 { "parallel" } else { "off" }));
    }
    if major >= 15 {
        expected.push(t("Tanım", "two_phase", "devre dışı"));
        expected.push(t("Tanım", "disable_on_error", "Hayır"));
    }
    if major >= 16 {
        expected.push(t("Tanım", "password_required", "Evet"));
        expected.push(t("Tanım", "run_as_owner", "Hayır"));
    }
    if major >= 17 {
        expected.push(t("Tanım", "failover", "Hayır"));
    }
    if major >= 16 {
        expected.push(t("Tanım", "origin", "any"));
    }
    assert_eq!(rows(&sub_props), expected);
    let sub_ddl = ddl(fx, &sub).await;
    assert!(sub_ddl.contains("CONNECTION 'host=127.0.0.1 port=1 dbname=x user=u password=********'"), "{sub_ddl}");
    snapshot!(fx, "subscription", sub_ddl, per_version);
    let sub_uri = id(find(&subs, "pgt_sub_uri"));
    assert_eq!(value(&props(fx, &sub_uri).await, "Bağlantı"), Some("postgresql://u:********@127.0.0.1:1/x?sslmode=disable"));
    let sub_uri_ddl = ddl(fx, &sub_uri).await;
    assert!(sub_uri_ddl.contains("CONNECTION 'postgresql://u:********@127.0.0.1:1/x?sslmode=disable'\n    PUBLICATION a, b"), "{sub_uri_ddl}");
    assert!(sub_uri_ddl.contains("synchronous_commit = 'local'"), "{sub_uri_ddl}");

    // --- Süper kullanıcı olmayan rol: yetki notları ---
    let limited = st.connect_plain(fx).await;
    let relocate = |n: &NodeId| match n {
        NodeId::Node { path, .. } => NodeId::Node { server: limited, path: path.clone() },
        other => other.clone(),
    };
    let lum = relocate(&um_owner);
    assert_eq!(value(&props(fx, &lum).await, "Seçenekler"), Some("görüntülemek için yetki gerekir"));
    let lum_ddl = ddl(fx, &lum).await;
    assert!(lum_ddl.contains(&format!("CREATE USER MAPPING FOR {owner} SERVER pgt_srv;\n-- seçenekler okunamadı (yetki)\n")), "{lum_ddl}");
    assert!(!lum_ddl.contains("OPTIONS"), "{lum_ddl}");
    // Kendi eşlemesi (sunucuda USAGE yetkisi var): seçenekler okunur.
    let lum_own = nodes(fx, &relocate(&srv_rt), Coll::UserMappings).await;
    let lum_own_props = props(fx, &id(find(&lum_own, &role))).await;
    assert_eq!(value(&lum_own_props, "Seçenekler"), Some("user=r"));
    let lsub = relocate(&sub);
    let lsub_props = props(fx, &lsub).await;
    let lsub_ddl = ddl(fx, &lsub).await;
    // Bağlantı dizgesi okunamaz; diğer satırlar süper kullanıcınınkiyle aynı (PG 10–13'te `oid` de okunamaz:
    // abonelik `pg_stat_subscription`'daki adıyla bulunur).
    // Herkese açık sütun kümesi sürüme göre değişir (ör. PG 10'da `subsynccommit` kapalı): okunamayan sütunun satırı yok.
    let need = "görüntülemek için yetki gerekir";
    let admin = admin_client(&fx.ts, &fx.ts.config.maintenance_db).await;
    let mut hidden = Vec::new();
    for (label, col) in [
        ("synchronous_commit", "subsynccommit"),
        ("binary", "subbinary"),
        ("streaming", "substream"),
        ("two_phase", "subtwophasestate"),
        ("disable_on_error", "subdisableonerr"),
        ("password_required", "subpasswordrequired"),
        ("run_as_owner", "subrunasowner"),
        ("failover", "subfailover"),
        ("origin", "suborigin"),
    ] {
        let readable: Option<bool> = admin
            .query_one(
                "SELECT has_column_privilege($1, 'pg_catalog.pg_subscription'::regclass, a.attnum, 'SELECT') \
                 FROM (SELECT NULL) x LEFT JOIN pg_attribute a ON a.attrelid = 'pg_catalog.pg_subscription'::regclass AND a.attname = $2",
                &[&role, &col],
            )
            .await
            .unwrap()
            .get(0);
        if readable == Some(false) {
            hidden.push(label);
        }
    }
    let expected_limited: Vec<_> = rows(&sub_props)
        .into_iter()
        .filter(|(_, l, _)| !hidden.contains(&l.as_str()))
        .map(|(s, l, v)| if l == "Bağlantı" { (s, l, need.to_string()) } else { (s, l, v) })
        .collect();
    assert_eq!(rows(&lsub_props), expected_limited);
    assert!(
        lsub_ddl.contains("-- Bağlantı dizgesini görüntülemek için yetki gerekir.\nCREATE SUBSCRIPTION pgt_sub\n    CONNECTION '********'\n    PUBLICATION pgt_pub_tbl\n"),
        "{lsub_ddl}"
    );
    assert!(!lsub_ddl.contains("password=") && !lsub_ddl.contains("host="), "{lsub_ddl}");
    fx.mgr.disconnect(limited);

    // --- Maskesiz DDL çalıştırılabilir: DROP + yeniden oluştur, DDL aynı kalır ---
    for (drop, parent, coll, label) in [
        ("DROP FOREIGN TABLE public.ft_rt", &public, Coll::ForeignTables, "ft_rt"),
        ("DROP PUBLICATION pgt_pub_rt", &db_id, Coll::Publications, "pgt_pub_rt"),
        ("DROP PUBLICATION pgt_pub_empty", &db_id, Coll::Publications, "pgt_pub_empty"),
        ("DROP PUBLICATION pgt_pub_all", &db_id, Coll::Publications, "pgt_pub_all"),
        ("DROP PUBLICATION pgt_pub_cols", &db_id, Coll::Publications, "pgt_pub_cols"),
    ] {
        if label == "pgt_pub_cols" && major < 15 {
            continue;
        }
        let node = child(fx, parent, coll, label).await;
        let before = ddl(fx, &node).await;
        db.batch_execute(drop).await.unwrap();
        db.batch_execute(&before).await.unwrap_or_else(|e| panic!("DDL çalışmadı ({e}):\n{before}"));
        let again = child(fx, parent, coll, label).await;
        assert_ne!(again.to_string(), node.to_string(), "{label} yeniden oluşturulmalıydı");
        assert_eq!(ddl(fx, &again).await, before);
    }
    // Server: user mapping ve foreign table'ı CASCADE ile düşer; önce sunucu, sonra tablo yeniden kurulur.
    db.batch_execute("DROP SERVER pgt_srv_rt CASCADE").await.unwrap();
    db.batch_execute(&srv_rt_ddl).await.unwrap_or_else(|e| panic!("DDL çalışmadı ({e}):\n{srv_rt_ddl}"));
    db.batch_execute(&ft_ddl).await.unwrap_or_else(|e| panic!("DDL çalışmadı ({e}):\n{ft_ddl}"));
    let servers = nodes(fx, &pg_fdw, Coll::ForeignServers).await;
    assert_eq!(ddl(fx, &id(find(&servers, "pgt_srv_rt"))).await, srv_rt_ddl);
    let ftables = nodes(fx, &public, Coll::ForeignTables).await;
    assert_eq!(ddl(fx, &id(find(&ftables, "ft_rt"))).await, ft_ddl);

    drop(db);
    st.teardown(fx).await;
}

/// İki veritabanında aynı adlı abonelik: süper kullanıcı her sürümde bağlı veritabanınınkini oid yoluyla açar
/// (PG 10–11'de `oid` sistem sütunudur); öbür veritabanının aboneliği hiçbir zaman dönmez; yetkisiz rol PG 10–13'te
/// (oid herkese açık değil, ad eşlemesi belirsiz) aboneliği güvenli varsayılanla listelemez/bulamaz.
async fn check_same_name(fx: &Fixture) {
    let major = fx.major().await;
    let st = Box::pin(setup(fx, "pgt_cat_fdw_dup")).await;
    let other = admin_client(&fx.ts, &st.other_db).await;
    other
        .batch_execute(&format!(
            "CREATE SUBSCRIPTION pgt_sub CONNECTION 'host=127.0.0.1 port=1 dbname=other_db user=u password={SECRET}' \
             PUBLICATION other_pub WITH (connect = false, slot_name = NONE)"
        ))
        .await
        .unwrap();
    let admin = admin_client(&fx.ts, &fx.ts.config.maintenance_db).await;
    let sub_oid = |db: &str| {
        let (admin, db) = (&admin, db.to_string());
        async move {
            admin
                .query_one(
                    "SELECT s.oid FROM pg_subscription s JOIN pg_database d ON d.oid = s.subdbid WHERE s.subname = 'pgt_sub' AND d.datname = $1",
                    &[&db],
                )
                .await
                .unwrap()
                .get::<_, u32>(0)
        }
    };
    let (mine, theirs) = (sub_oid(&st.db).await, sub_oid(&st.other_db).await);
    assert_ne!(mine, theirs);
    let mine_id = st.db_id.child(pgt_catalog::node::Seg::Subscription(mine));
    let theirs_id = st.db_id.child(pgt_catalog::node::Seg::Subscription(theirs));

    // Süper kullanıcı: liste, özellik ve DDL bağlı veritabanının aboneliğidir.
    let subs = nodes(fx, &st.db_id, Coll::Subscriptions).await;
    assert_eq!(subs.iter().map(|n| n.id.as_str()).collect::<Vec<_>>(), [mine_id.to_string().as_str()]);
    let p = props(fx, &mine_id).await;
    assert_eq!(value(&p, "Yayınlar"), Some("pgt_pub_tbl"));
    assert_eq!(value(&p, "Bağlantı"), Some("host=127.0.0.1 port=1 dbname=x user=u password=********"));
    let d = ddl(fx, &mine_id).await;
    assert!(d.contains("dbname=x") && d.contains("PUBLICATION pgt_pub_tbl") && !d.contains("other"), "{d}");
    // Öbür veritabanının aboneliği bu veritabanından açılmaz.
    let not_found = |r: pgt_core::Result<_>, what: &str| assert!(matches!(r, Err(pgt_core::Error::NotFound(_))), "{what} bulunmamalıydı");
    not_found(Box::pin(fx.catalog.properties(&theirs_id)).await.map(|_| ()), "süper kullanıcı özellikleri");
    not_found(Box::pin(fx.catalog.ddl(&theirs_id)).await.map(|_| ()), "süper kullanıcı DDL'i");

    // Yetkisiz rol.
    let limited = st.connect_plain(fx).await;
    let relocate = |n: &NodeId| match n {
        NodeId::Node { path, .. } => NodeId::Node { server: limited, path: path.clone() },
        other => other.clone(),
    };
    let (lmine, ltheirs) = (relocate(&mine_id), relocate(&theirs_id));
    let lsubs = nodes(fx, &relocate(&st.db_id), Coll::Subscriptions).await;
    if major <= 13 {
        // `oid` yalnız `pg_stat_subscription`'dan ada göre: ad iki veritabanında var → belirsiz → listelenmez/bulunmaz.
        assert!(lsubs.is_empty(), "{:?}", lsubs.iter().map(|n| &n.id).collect::<Vec<_>>());
        not_found(Box::pin(fx.catalog.properties(&lmine)).await.map(|_| ()), "yetkisiz rol özellikleri (belirsiz ad)");
        not_found(Box::pin(fx.catalog.ddl(&lmine)).await.map(|_| ()), "yetkisiz rol DDL'i (belirsiz ad)");
    } else {
        assert_eq!(lsubs.iter().map(|n| n.id.as_str()).collect::<Vec<_>>(), [lmine.to_string().as_str()]);
        let lp = props(fx, &lmine).await;
        assert_eq!((value(&lp, "Yayınlar"), value(&lp, "Bağlantı")), (Some("pgt_pub_tbl"), Some("görüntülemek için yetki gerekir")));
        let ld = ddl(fx, &lmine).await;
        assert!(ld.contains("CONNECTION '********'\n    PUBLICATION pgt_pub_tbl\n"), "{ld}");
    }
    not_found(Box::pin(fx.catalog.properties(&ltheirs)).await.map(|_| ()), "yetkisiz rol özellikleri");
    not_found(Box::pin(fx.catalog.ddl(&ltheirs)).await.map(|_| ()), "yetkisiz rol DDL'i");
    fx.mgr.disconnect(limited);

    drop(other);
    st.teardown(fx).await;
}
