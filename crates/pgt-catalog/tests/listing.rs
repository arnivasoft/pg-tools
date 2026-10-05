mod common;

use common::{find, fixtures, labels};
use pgt_catalog::node::{Coll, NodeId, NodeKind, Seg};
use pgt_core::Error;
use pgt_core::config::ServerConfig;

#[tokio::test]
async fn root_lists_groups_then_servers() {
    for fx in fixtures().await {
        {
            let mut reg = fx.registry.write().unwrap();
            let mut grouped = ServerConfig::new("zeta", "h", 5432, "u");
            grouped.group = Some("Üretim".into());
            reg.upsert(grouped).unwrap();
            reg.upsert(ServerConfig::new("alfa", "h", 5432, "u")).unwrap();
        }
        let root = fx.catalog.list_children(&NodeId::Root).await.unwrap();
        assert_eq!(root[0].kind, NodeKind::ServerGroup);
        assert_eq!(root[0].label, "Üretim");
        assert_eq!(root[0].id, "grp:Üretim");
        let servers: Vec<_> = root[1..].iter().map(|n| (n.label.as_str(), n.flags.connected)).collect();
        assert_eq!(servers[0], ("alfa", false));
        let test = root.iter().find(|n| n.server_id == Some(fx.server)).unwrap();
        assert!(test.flags.connected);
        assert!(test.detail.is_some(), "bağlı sunucuda sürüm gösterilmeli");
        let group = fx.catalog.list_children(&NodeId::Group("Üretim".into())).await.unwrap();
        assert_eq!(labels(&group), ["zeta"]);
    }
}

#[tokio::test]
async fn server_has_databases_collection() {
    for fx in fixtures().await {
        let children = fx.catalog.list_children(&fx.server_id()).await.unwrap();
        assert_eq!(children.len(), 1);
        assert_eq!((children[0].kind, children[0].label.as_str()), (NodeKind::DatabaseCollection, "Databases"));
    }
}

#[tokio::test]
async fn databases_exclude_templates() {
    for fx in fixtures().await {
        let dbs = fx.catalog.list_children(&fx.server_id().child(Seg::Coll(Coll::Databases))).await.unwrap();
        let names = labels(&dbs);
        assert!(names.contains(&"pgt_fixture"), "{names:?}");
        assert!(!names.contains(&"template0") && !names.contains(&"template1"));
        assert!(find(&dbs, "pgt_fixture").has_children);
    }
}

#[tokio::test]
async fn databases_without_connect_are_leaves() {
    for fx in fixtures().await {
        let admin = pgt_core::testing::admin_client(&fx.ts, &fx.ts.config.maintenance_db).await;
        // CREATE/DROP DATABASE çok ifadeli sorguda (örtük transaction) çalışmaz; ayrı çağrılar gerekir.
        admin.batch_execute("DROP DATABASE IF EXISTS pgt_noconn").await.unwrap();
        admin.batch_execute("CREATE DATABASE pgt_noconn").await.unwrap();
        admin.batch_execute("ALTER DATABASE pgt_noconn ALLOW_CONNECTIONS false").await.unwrap();
        let dbs = fx.catalog.list_children(&fx.server_id().child(Seg::Coll(Coll::Databases))).await.unwrap();
        assert!(!find(&dbs, "pgt_noconn").has_children);
        admin.batch_execute("DROP DATABASE pgt_noconn").await.unwrap();
    }
}

#[tokio::test]
async fn schemas_hide_system_by_default() {
    for fx in fixtures().await {
        let coll = fx.db().child(Seg::Coll(Coll::Schemas));
        let names: Vec<String> = labels(&fx.catalog.list_children(&coll).await.unwrap()).into_iter().map(String::from).collect();
        assert!(names.contains(&"app".to_string()) && names.contains(&"public".to_string()), "{names:?}");
        assert!(!names.iter().any(|n| n == "pg_catalog" || n == "information_schema" || n.starts_with("pg_toast")));
        fx.catalog.set_show_system(true);
        let all = fx.catalog.list_children(&coll).await.unwrap();
        assert!(find(&all, "pg_catalog").flags.system);
        assert!(!find(&all, "app").flags.system);
        fx.catalog.set_show_system(false);
    }
}

#[tokio::test]
async fn tables_exclude_partitions_and_flag_partitioned() {
    for fx in fixtures().await {
        let schema = fx.schema("app").await;
        let tables = fx.catalog.list_children(&schema.child(Seg::Coll(Coll::Tables))).await.unwrap();
        assert_eq!(labels(&tables), ["Order Items", "customers", "events", "orders"]);
        assert!(find(&tables, "events").flags.partitioned);
        assert!(!find(&tables, "orders").flags.partitioned);
        assert!(tables.iter().all(|t| t.kind == NodeKind::Table && t.has_children));
    }
}

#[tokio::test]
async fn columns_in_order_with_types_and_flags() {
    for fx in fixtures().await {
        let table = fx.table("app", "customers").await;
        let cols = fx.catalog.list_children(&table.child(Seg::Coll(Coll::Columns))).await.unwrap();
        assert_eq!(labels(&cols), ["id", "email", "name", "created_at"]);
        let id = find(&cols, "id");
        assert_eq!(id.detail.as_deref(), Some("bigint"));
        assert!(id.flags.primary_key && id.flags.not_null && !id.has_children);
        assert_eq!(find(&cols, "name").detail.as_deref(), Some("character varying(100)"));
        assert!(!find(&cols, "name").flags.not_null);
        let quoted = fx.table("app", "Order Items").await;
        let cols = fx.catalog.list_children(&quoted.child(Seg::Coll(Coll::Columns))).await.unwrap();
        assert_eq!(labels(&cols), ["Line No", "order_id", "Ürün Adı"]);
    }
}

#[tokio::test]
async fn collection_children_of_table_and_schema() {
    for fx in fixtures().await {
        let schema = fx.schema("app").await;
        let kids = fx.catalog.list_children(&schema).await.unwrap();
        assert_eq!(kids.iter().map(|k| (k.kind, k.label.as_str())).collect::<Vec<_>>(), [(NodeKind::TableCollection, "Tables")]);
        let table = fx.table("app", "orders").await;
        let kids = fx.catalog.list_children(&table).await.unwrap();
        assert_eq!(kids.iter().map(|k| (k.kind, k.label.as_str())).collect::<Vec<_>>(), [(NodeKind::ColumnCollection, "Columns")]);
    }
}

#[tokio::test]
async fn invalid_shapes_and_disconnected_servers_error() {
    for fx in fixtures().await {
        let bad = fx.server_id().child(Seg::Table(1));
        assert!(matches!(fx.catalog.list_children(&bad).await, Err(Error::Invalid(_))));
        let other = NodeId::server(uuid::Uuid::new_v4());
        assert!(matches!(fx.catalog.list_children(&other).await, Err(Error::NotConnected)));
    }
}
