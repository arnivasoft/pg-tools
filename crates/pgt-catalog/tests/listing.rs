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
async fn server_has_databases_roles_and_tablespaces() {
    for fx in fixtures().await {
        let children = fx.catalog.list_children(&fx.server_id()).await.unwrap();
        assert_eq!(
            children.iter().map(|c| (c.kind, c.label.as_str())).collect::<Vec<_>>(),
            [
                (NodeKind::DatabaseCollection, "Databases"),
                (NodeKind::RoleCollection, "Login/Group Roles"),
                (NodeKind::TablespaceCollection, "Tablespaces"),
            ]
        );
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
        admin.batch_execute("CREATE DATABASE pgt_noconn TEMPLATE template0 ENCODING 'UTF8'").await.unwrap();
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
async fn schema_collections_depend_on_version() {
    for fx in fixtures().await {
        let schema = fx.schema("app").await;
        let kids = fx.catalog.list_children(&schema).await.unwrap();
        let mut expected = vec!["Tables", "Views", "Materialized Views", "Functions"];
        if fx.major().await >= 11 {
            expected.push("Procedures");
        }
        expected.extend(["Trigger Functions", "Sequences", "Types", "Domains"]);
        assert_eq!(labels(&kids), expected);
    }
}

#[tokio::test]
async fn partitions_collection_only_for_partitioned_tables() {
    for fx in fixtures().await {
        let orders = fx.catalog.list_children(&fx.table("app", "orders").await).await.unwrap();
        assert_eq!(labels(&orders), ["Columns", "Constraints", "Indexes", "Triggers", "RLS Policies"]);
        let events = fx.catalog.list_children(&fx.table("app", "events").await).await.unwrap();
        assert_eq!(labels(&events), ["Columns", "Constraints", "Indexes", "Triggers", "RLS Policies", "Partitions"]);
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

#[tokio::test]
async fn roles_and_tablespaces_hide_system_by_default() {
    for fx in fixtures().await {
        let roles = fx.labels_of(&fx.server_id(), Coll::Roles).await;
        for expected in ["pgt_reader", "pgt_app"] {
            assert!(roles.iter().any(|r| r == expected), "{expected} yok: {roles:?}");
        }
        assert!(!roles.iter().any(|r| r.starts_with("pg_")), "yerleşik roller gizli olmalı: {roles:?}");
        let node = find(&fx.nodes_of(&fx.server_id(), Coll::Roles).await, "pgt_app").clone();
        assert_eq!(node.detail.as_deref(), Some("login"));
        assert!(!node.has_children);
        assert!(fx.labels_of(&fx.server_id(), Coll::Tablespaces).await.is_empty());
        fx.catalog.set_show_system(true);
        let all = fx.labels_of(&fx.server_id(), Coll::Tablespaces).await;
        assert_eq!(all, ["pg_default", "pg_global"]);
        fx.catalog.set_show_system(false);
    }
}

#[tokio::test]
async fn database_children_and_extensions() {
    for fx in fixtures().await {
        let kids = fx.catalog.list_children(&fx.db()).await.unwrap();
        assert_eq!(labels(&kids), ["Schemas", "Extensions"]);
        let ext = fx.labels_of(&fx.db(), Coll::Extensions).await;
        assert_eq!(ext, ["pg_trgm", "plpgsql"]);
        let nodes = fx.nodes_of(&fx.db(), Coll::Extensions).await;
        assert_eq!(find(&nodes, "pg_trgm").kind, NodeKind::Extension);
        assert!(find(&nodes, "pg_trgm").detail.is_some());
    }
}

#[tokio::test]
async fn table_sub_objects_are_listed() {
    for fx in fixtures().await {
        let customers = fx.table("app", "customers").await;
        let constraints = fx.nodes_of(&customers, Coll::Constraints).await;
        assert_eq!(labels(&constraints), ["customers_pkey", "customers_email_key", "customers_email_check"]);
        assert!(find(&constraints, "customers_pkey").flags.primary_key);
        assert_eq!(find(&constraints, "customers_email_check").detail.as_deref(), Some("check"));
        assert!(constraints.iter().all(|c| !c.has_children && c.kind == NodeKind::Constraint));

        let indexes = fx.nodes_of(&customers, Coll::Indexes).await;
        assert_eq!(labels(&indexes), ["customers_email_key", "customers_lower_name_idx", "customers_pkey"]);
        assert_eq!(find(&indexes, "customers_pkey").detail.as_deref(), Some("btree, primary key"));

        assert_eq!(fx.labels_of(&customers, Coll::Triggers).await, ["customers_touch"]);
        assert_eq!(fx.labels_of(&customers, Coll::Policies).await, ["customers_own", "customers_select"]);
        let policies = fx.nodes_of(&customers, Coll::Policies).await;
        assert_eq!(find(&policies, "customers_select").detail.as_deref(), Some("select"));
    }
}

#[tokio::test]
async fn partitions_are_tables_with_bounds() {
    for fx in fixtures().await {
        let events = fx.table("app", "events").await;
        let parts = fx.nodes_of(&events, Coll::Partitions).await;
        assert_eq!(labels(&parts), ["events_2026"]);
        assert_eq!(parts[0].kind, NodeKind::Table);
        assert!(parts[0].has_children);
        assert!(parts[0].detail.as_deref().unwrap().starts_with("FOR VALUES FROM"));
        // bir partition normal bir tablo gibi açılır
        let part_id = NodeId::parse(&parts[0].id).unwrap();
        let cols = fx.catalog.list_children(&part_id).await.unwrap();
        assert_eq!(labels(&cols)[0], "Columns");
    }
}

#[tokio::test]
async fn views_and_materialized_views_with_columns_and_indexes() {
    for fx in fixtures().await {
        let schema = fx.schema("app").await;
        let views = fx.nodes_of(&schema, Coll::Views).await;
        assert_eq!(labels(&views), ["Mixed View", "customer_orders"]);
        assert!(views.iter().all(|v| v.kind == NodeKind::View && v.has_children));

        let view = fx.child_of(&schema, Coll::Views, "customer_orders").await;
        assert_eq!(labels(&fx.catalog.list_children(&view).await.unwrap()), ["Columns"]);
        assert_eq!(fx.labels_of(&view, Coll::Columns).await, ["customer_id", "email", "order_id", "total"]);
        let mixed = fx.child_of(&schema, Coll::Views, "Mixed View").await;
        assert_eq!(fx.labels_of(&mixed, Coll::Columns).await, ["Çok Kolon"]);

        let mvs = fx.nodes_of(&schema, Coll::MatViews).await;
        assert_eq!(labels(&mvs), ["order_totals"]);
        let mv = fx.child_of(&schema, Coll::MatViews, "order_totals").await;
        assert_eq!(labels(&fx.catalog.list_children(&mv).await.unwrap()), ["Columns", "Indexes"]);
        assert_eq!(fx.labels_of(&mv, Coll::Indexes).await, ["order_totals_pk"]);
        assert_eq!(fx.labels_of(&mv, Coll::Columns).await, ["customer_id", "total"]);
    }
}

#[tokio::test]
async fn functions_are_listed_with_identity_arguments() {
    for fx in fixtures().await {
        let schema = fx.schema("app").await;
        let funcs = fx.labels_of(&schema, Coll::Functions).await;
        for expected in ["add_tax(amount numeric, rate numeric)", "fmt(v integer)", "fmt(v text)"] {
            assert!(funcs.iter().any(|f| f == expected), "{expected} yok: {funcs:?}");
        }
        assert!(!funcs.iter().any(|f| f.starts_with("touch_row")), "trigger fonksiyonu burada görünmemeli");
        assert!(!funcs.iter().any(|f| f.starts_with("int_span")), "range kurucuları gizli olmalı: {funcs:?}");
        assert_eq!(fx.labels_of(&schema, Coll::TriggerFunctions).await, ["touch_row()"]);
        if fx.major().await >= 11 {
            assert_eq!(fx.labels_of(&schema, Coll::Procedures).await, ["archive_orders(older_than date)"]);
        }
    }
}

#[tokio::test]
async fn sequences_types_and_domains() {
    for fx in fixtures().await {
        let schema = fx.schema("app").await;
        // kimlik (identity) sequence'leri listelenmez; bigserial'ın sequence'i listelenir
        assert_eq!(fx.labels_of(&schema, Coll::Sequences).await, ["invoice_seq", "orders_id_seq"]);
        let extra = fx.schema("extra").await;
        assert!(fx.labels_of(&extra, Coll::Sequences).await.is_empty());
        let types = fx.nodes_of(&schema, Coll::Types).await;
        assert_eq!(labels(&types), ["address", "int_span", "order_status"]);
        assert_eq!(find(&types, "order_status").detail.as_deref(), Some("enum"));
        assert_eq!(find(&types, "address").detail.as_deref(), Some("composite"));
        assert_eq!(find(&types, "int_span").detail.as_deref(), Some("range"));
        let domains = fx.nodes_of(&schema, Coll::Domains).await;
        assert_eq!(labels(&domains), ["positive_int"]);
        assert_eq!(domains[0].detail.as_deref(), Some("integer"));
    }
}

#[tokio::test]
async fn extension_members_are_flagged() {
    for fx in fixtures().await {
        let extra = fx.schema("extra").await;
        let funcs = fx.nodes_of(&extra, Coll::Functions).await;
        assert!(!funcs.is_empty(), "pg_trgm fonksiyonları extra şemasında olmalı");
        assert!(funcs.iter().all(|f| f.flags.from_extension));
        let app = fx.schema("app").await;
        assert!(fx.nodes_of(&app, Coll::Functions).await.iter().all(|f| !f.flags.from_extension));
    }
}

#[tokio::test]
async fn empty_collections_expand_to_nothing() {
    for fx in fixtures().await {
        let extra = fx.schema("extra").await;
        assert!(fx.nodes_of(&extra, Coll::Views).await.is_empty());
        assert!(fx.nodes_of(&extra, Coll::Domains).await.is_empty());
        let items = fx.table("app", "Order Items").await;
        assert!(fx.nodes_of(&items, Coll::Triggers).await.is_empty());
        assert!(fx.nodes_of(&items, Coll::Policies).await.is_empty());
    }
}

#[tokio::test]
async fn leaf_nodes_have_no_children_and_wrong_collections_are_invalid() {
    for fx in fixtures().await {
        let customers = fx.table("app", "customers").await;
        let pkey = fx.child_of(&customers, Coll::Constraints, "customers_pkey").await;
        assert!(fx.catalog.list_children(&pkey).await.unwrap().is_empty());
        // tabloda "Views" koleksiyonu yok
        let wrong = customers.child(Seg::Coll(Coll::Views));
        assert!(matches!(fx.catalog.list_children(&wrong).await, Err(Error::Invalid(_))));
        // sunucu altında "Tables" koleksiyonu yok
        let wrong = fx.server_id().child(Seg::Coll(Coll::Tables));
        assert!(matches!(fx.catalog.list_children(&wrong).await, Err(Error::Invalid(_))));
    }
}
