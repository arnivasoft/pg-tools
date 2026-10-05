mod common;

use common::fixtures;
use pgt_catalog::node::{Coll, NodeId, Seg};
use pgt_catalog::props::PropertySection;
use pgt_core::Error;

fn prop<'a>(sections: &'a [PropertySection], title: &str, label: &str) -> Option<&'a str> {
    sections.iter().find(|s| s.title == title)?.rows.iter().find(|r| r.label == label).map(|r| r.value.as_str())
}

#[tokio::test]
async fn server_properties_without_query() {
    for fx in fixtures().await {
        let s = fx.catalog.properties(&fx.server_id()).await.unwrap();
        assert_eq!(prop(&s, "Genel", "Host"), Some(fx.ts.config.host.as_str()));
        assert_eq!(prop(&s, "Genel", "SSL modu"), Some("prefer"));
        assert!(prop(&s, "Sunucu", "Sürüm").is_some());
    }
}

#[tokio::test]
async fn database_properties() {
    for fx in fixtures().await {
        let s = fx.catalog.properties(&fx.db()).await.unwrap();
        assert_eq!(prop(&s, "Genel", "Ad"), Some("pgt_fixture"));
        assert_eq!(prop(&s, "Genel", "OID"), Some(fx.db_oid.to_string().as_str()));
        assert!(prop(&s, "Tanım", "Encoding").is_some());
        assert!(prop(&s, "İstatistik", "Boyut").is_some());
    }
}

#[tokio::test]
async fn database_properties_without_connect_privilege() {
    for fx in fixtures().await {
        let admin = pgt_core::testing::admin_client(&fx.ts, &fx.ts.config.maintenance_db).await;
        admin.batch_execute("DROP DATABASE IF EXISTS pgt_private").await.unwrap();
        admin.batch_execute("CREATE DATABASE pgt_private").await.unwrap();
        admin.batch_execute("REVOKE CONNECT ON DATABASE pgt_private FROM PUBLIC").await.unwrap();
        admin.batch_execute("DO $$ BEGIN IF NOT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'pgt_limited') THEN CREATE ROLE pgt_limited LOGIN PASSWORD 'pgt_limited'; END IF; END $$").await.unwrap();
        let oid: u32 = admin.query_one("SELECT oid FROM pg_database WHERE datname = 'pgt_private'", &[]).await.unwrap().get(0);

        let mut limited = fx.ts.config.clone();
        limited.id = uuid::Uuid::new_v4();
        limited.username = "pgt_limited".into();
        fx.registry.write().unwrap().upsert(limited.clone()).unwrap();
        fx.mgr.connect(limited.clone(), Some("pgt_limited".into())).await.unwrap();

        let id = NodeId::server(limited.id).child(Seg::Db(oid));
        let s = fx.catalog.properties(&id).await.unwrap();
        assert_eq!(prop(&s, "Genel", "Ad"), Some("pgt_private"));
        assert_eq!(prop(&s, "İstatistik", "Boyut"), None, "yetki yoksa boyut gösterilmez");
        fx.mgr.disconnect(limited.id);
        admin.batch_execute("DROP DATABASE pgt_private").await.unwrap();
    }
}

#[tokio::test]
async fn schema_properties() {
    for fx in fixtures().await {
        let s = fx.catalog.properties(&fx.schema("app").await).await.unwrap();
        assert_eq!(prop(&s, "Genel", "Ad"), Some("app"));
        assert_eq!(prop(&s, "Genel", "Yorum"), Some("Uygulama şeması"));
        assert!(prop(&s, "Güvenlik", "Yetkiler").unwrap().contains("pgt_reader=U"));
    }
}

#[tokio::test]
async fn table_properties() {
    for fx in fixtures().await {
        let s = fx.catalog.properties(&fx.table("app", "customers").await).await.unwrap();
        assert_eq!(prop(&s, "Genel", "Ad"), Some("customers"));
        assert_eq!(prop(&s, "Genel", "Şema"), Some("app"));
        assert_eq!(prop(&s, "Genel", "Yorum"), Some("Müşteriler"));
        assert_eq!(prop(&s, "Tanım", "Tür"), Some("Table"));
        assert_eq!(prop(&s, "Tanım", "Kalıcılık"), Some("permanent"));
        assert!(prop(&s, "İstatistik", "Toplam boyut").is_some());
        let orders = fx.catalog.properties(&fx.table("app", "orders").await).await.unwrap();
        assert_eq!(prop(&orders, "Tanım", "Seçenekler"), Some("fillfactor=80"));
        let events = fx.catalog.properties(&fx.table("app", "events").await).await.unwrap();
        assert_eq!(prop(&events, "Tanım", "Tür"), Some("Partitioned table"));
        let scratch = fx.catalog.properties(&fx.table("public", "scratch").await).await.unwrap();
        assert_eq!(prop(&scratch, "Tanım", "Kalıcılık"), Some("unlogged"));
    }
}

#[tokio::test]
async fn column_properties() {
    for fx in fixtures().await {
        let table = fx.table("app", "customers").await;
        let col = |n: i16| table.child(Seg::Column(n));
        let id = fx.catalog.properties(&col(1)).await.unwrap();
        assert_eq!(prop(&id, "Genel", "Veri tipi"), Some("bigint"));
        assert_eq!(prop(&id, "Tanım", "Identity"), Some("ALWAYS"));
        let email = fx.catalog.properties(&col(2)).await.unwrap();
        assert_eq!(prop(&email, "Genel", "Boş olamaz"), Some("Evet"));
        assert_eq!(prop(&email, "Genel", "Yorum"), Some("E-posta"));
        let name = fx.catalog.properties(&col(3)).await.unwrap();
        assert_eq!(prop(&name, "Tanım", "Collation"), Some("\"C\""));
        let created = fx.catalog.properties(&col(4)).await.unwrap();
        assert_eq!(prop(&created, "Tanım", "Varsayılan"), Some("now()"));
    }
}

#[tokio::test]
async fn collections_have_no_properties_and_missing_oid_is_not_found() {
    for fx in fixtures().await {
        let coll = fx.db().child(Seg::Coll(Coll::Schemas));
        assert!(fx.catalog.properties(&coll).await.unwrap().is_empty());
        assert!(fx.catalog.properties(&NodeId::Root).await.unwrap().is_empty());
        let schema = fx.schema("app").await;
        let ghost = schema.child(Seg::Table(4_000_000_000));
        assert!(matches!(fx.catalog.properties(&ghost).await, Err(Error::NotFound(_))));
    }
}
