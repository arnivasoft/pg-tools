mod common;

use common::fixtures;
use pgt_catalog::node::{Coll, Seg};
use pgt_core::testing::admin_client;

macro_rules! snapshot {
    ($fx:expr, $name:expr, $ddl:expr) => {{
        let major = $fx.major().await;
        insta::with_settings!({ snapshot_suffix => format!("pg{major}") }, {
            insta::assert_snapshot!($name, $fx.normalize(&$ddl));
        });
    }};
}

#[tokio::test]
async fn table_ddl_snapshots() {
    for fx in fixtures().await {
        for name in ["customers", "orders", "events", "Order Items"] {
            let ddl = fx.catalog.ddl(&fx.table("app", name).await).await.unwrap();
            snapshot!(fx, format!("table_{}", name.replace(' ', "_")), ddl);
        }
        let scratch = fx.catalog.ddl(&fx.table("public", "scratch").await).await.unwrap();
        assert!(scratch.contains("CREATE UNLOGGED TABLE IF NOT EXISTS public.scratch"), "{scratch}");
    }
}

#[tokio::test]
async fn schema_and_column_ddl_snapshots() {
    for fx in fixtures().await {
        let schema = fx.catalog.ddl(&fx.schema("app").await).await.unwrap();
        snapshot!(fx, "schema_app", schema);
        let email = fx.table("app", "customers").await.child(Seg::Column(2));
        let column = fx.catalog.ddl(&email).await.unwrap();
        snapshot!(fx, "column_customers_email", column);
    }
}

#[tokio::test]
async fn database_ddl_has_core_clauses() {
    for fx in fixtures().await {
        let ddl = fx.catalog.ddl(&fx.db()).await.unwrap();
        for part in ["-- Database: pgt_fixture", "CREATE DATABASE pgt_fixture", "ENCODING = '", "LC_COLLATE = '", "CONNECTION LIMIT = -1", "IS_TEMPLATE = False;"] {
            assert!(ddl.contains(part), "'{part}' eksik:\n{ddl}");
        }
    }
}

#[tokio::test]
async fn non_object_nodes_have_empty_ddl() {
    for fx in fixtures().await {
        assert_eq!(fx.catalog.ddl(&fx.server_id()).await.unwrap(), "");
        assert_eq!(fx.catalog.ddl(&fx.db().child(Seg::Coll(Coll::Schemas))).await.unwrap(), "");
    }
}

/// Üretilen DDL boş bir veritabanında hatasız çalışmalı ve aynı kolonları üretmeli.
#[tokio::test]
async fn ddl_executes_in_scratch_database() {
    for fx in fixtures().await {
        let admin = admin_client(&fx.ts, &fx.ts.config.maintenance_db).await;
        admin.batch_execute("DROP DATABASE IF EXISTS pgt_scratch").await.unwrap();
        admin.batch_execute("CREATE DATABASE pgt_scratch").await.unwrap();
        let scratch = admin_client(&fx.ts, "pgt_scratch").await;

        let schema_ddl = fx.catalog.ddl(&fx.schema("app").await).await.unwrap();
        scratch.batch_execute(&schema_ddl).await.unwrap_or_else(|e| panic!("{e}\n{schema_ddl}"));
        // bigserial'ın sequence'i Plan 2'deki Sequence DDL'ine kadar elle oluşturulur.
        scratch.batch_execute("CREATE SEQUENCE app.orders_id_seq").await.unwrap();

        for name in ["customers", "orders", "Order Items", "events"] {
            let ddl = fx.catalog.ddl(&fx.table("app", name).await).await.unwrap();
            scratch.batch_execute(&ddl).await.unwrap_or_else(|e| panic!("{name}: {e}\n{ddl}"));
            let expected: Vec<(String, String)> = scratch
                .query(
                    "SELECT a.attname::text, format_type(a.atttypid, a.atttypmod) FROM pg_attribute a \
                     WHERE a.attrelid = $1::text::regclass AND a.attnum > 0 AND NOT a.attisdropped ORDER BY a.attnum",
                    &[&format!("app.\"{name}\"")],
                )
                .await
                .unwrap()
                .iter()
                .map(|r| (r.get(0), r.get(1)))
                .collect();
            let table = fx.table("app", name).await;
            let original = fx.catalog.list_children(&table.child(Seg::Coll(Coll::Columns))).await.unwrap();
            let original: Vec<(String, String)> = original.iter().map(|n| (n.label.clone(), n.detail.clone().unwrap())).collect();
            assert_eq!(expected, original, "{name}");
        }
        drop(scratch);
        admin
            .execute("SELECT pg_terminate_backend(pid) FROM pg_stat_activity WHERE datname = 'pgt_scratch' AND pid <> pg_backend_pid()", &[])
            .await
            .unwrap();
        admin.batch_execute("DROP DATABASE pgt_scratch").await.unwrap();
    }
}
