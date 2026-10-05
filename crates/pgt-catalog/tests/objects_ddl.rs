mod common;

use common::fixtures;
use pgt_catalog::node::{Coll, Seg};
use pgt_core::Error;

macro_rules! snapshot {
    ($fx:expr, $name:expr, $ddl:expr) => {{
        let major = $fx.major().await;
        let ddl_text = $fx.normalize(&$ddl);
        insta::with_settings!({ snapshot_suffix => format!("pg{major}") }, {
            insta::assert_snapshot!($name, ddl_text);
        });
    }};
}

#[tokio::test]
async fn table_sub_object_ddl_snapshots() {
    for fx in fixtures().await {
        let customers = fx.table("app", "customers").await;
        for (coll, label, name) in [
            (Coll::Constraints, "customers_pkey", "constraint_pk"),
            (Coll::Constraints, "customers_email_check", "constraint_check"),
            (Coll::Indexes, "customers_lower_name_idx", "index_expr"),
            (Coll::Triggers, "customers_touch", "trigger"),
            (Coll::Policies, "customers_own", "policy_restrictive"),
            (Coll::Policies, "customers_select", "policy_select"),
        ] {
            let id = fx.child_of(&customers, coll, label).await;
            let ddl = fx.catalog.ddl(&id).await.unwrap();
            snapshot!(fx, format!("objects_{name}"), ddl);
        }
        let orders = fx.table("app", "orders").await;
        let idx = fx.child_of(&orders, Coll::Indexes, "orders_customer_idx").await;
        snapshot!(fx, "objects_index_partial", fx.catalog.ddl(&idx).await.unwrap());
    }
}

#[tokio::test]
async fn schema_object_ddl_snapshots() {
    for fx in fixtures().await {
        let schema = fx.schema("app").await;
        for (coll, label, name) in [
            (Coll::Views, "customer_orders", "view"),
            (Coll::Views, "Mixed View", "view_quoted"),
            (Coll::MatViews, "order_totals", "matview"),
            (Coll::Sequences, "invoice_seq", "sequence_custom"),
            (Coll::Sequences, "orders_id_seq", "sequence_owned"),
            (Coll::Functions, "add_tax(amount numeric, rate numeric)", "function"),
            (Coll::TriggerFunctions, "touch_row()", "trigger_function"),
            (Coll::Types, "order_status", "type_enum"),
            (Coll::Types, "address", "type_composite"),
            (Coll::Types, "int_span", "type_range"),
            (Coll::Domains, "positive_int", "domain"),
        ] {
            let id = fx.child_of(&schema, coll, label).await;
            let ddl = fx.catalog.ddl(&id).await.unwrap();
            snapshot!(fx, format!("objects_{name}"), ddl);
        }
    }
}

#[tokio::test]
async fn database_and_server_object_ddl_snapshots() {
    for fx in fixtures().await {
        let ext = fx.child_of(&fx.db(), Coll::Extensions, "pg_trgm").await;
        snapshot!(fx, "objects_extension", fx.catalog.ddl(&ext).await.unwrap());
        let role = fx.child_of(&fx.server_id(), Coll::Roles, "pgt_app").await;
        let role_ddl = fx.catalog.ddl(&role).await.unwrap();
        assert!(!role_ddl.to_lowercase().contains("password"), "rol DDL'inde parola olmamalı:\n{role_ddl}");
        snapshot!(fx, "objects_role", role_ddl);
        fx.catalog.set_show_system(true);
        let ts = fx.child_of(&fx.server_id(), Coll::Tablespaces, "pg_default").await;
        fx.catalog.set_show_system(false);
        snapshot!(fx, "objects_tablespace", fx.catalog.ddl(&ts).await.unwrap());
    }
}

#[tokio::test]
async fn quoted_names_survive_in_ddl() {
    for fx in fixtures().await {
        let schema = fx.schema("app").await;
        let view = fx.child_of(&schema, Coll::Views, "Mixed View").await;
        let ddl = fx.catalog.ddl(&view).await.unwrap();
        assert!(ddl.contains("-- View: app.\"Mixed View\""), "{ddl}");
        assert!(ddl.contains("CREATE OR REPLACE VIEW app.\"Mixed View\""), "{ddl}");
        assert!(ddl.contains("\"Çok Kolon\""), "{ddl}");
        assert!(ddl.contains("DROP VIEW IF EXISTS app.\"Mixed View\";"), "{ddl}");
    }
}

#[tokio::test]
async fn overloaded_functions_resolve_by_oid() {
    for fx in fixtures().await {
        let schema = fx.schema("app").await;
        let int_fmt = fx.child_of(&schema, Coll::Functions, "fmt(v integer)").await;
        let text_fmt = fx.child_of(&schema, Coll::Functions, "fmt(v text)").await;
        let a = fx.catalog.ddl(&int_fmt).await.unwrap();
        let b = fx.catalog.ddl(&text_fmt).await.unwrap();
        assert!(a.contains("app.fmt(v integer)") && !a.contains("v text"), "{a}");
        assert!(b.contains("app.fmt(v text)") && !b.contains("v integer"), "{b}");
        assert!(a.contains("DROP FUNCTION IF EXISTS app.fmt(v integer);"), "{a}");
    }
}

#[tokio::test]
async fn procedure_ddl_on_pg11_and_newer() {
    for fx in fixtures().await {
        if fx.major().await < 11 {
            continue;
        }
        let schema = fx.schema("app").await;
        let proc = fx.child_of(&schema, Coll::Procedures, "archive_orders(older_than date)").await;
        let ddl = fx.catalog.ddl(&proc).await.unwrap();
        assert!(ddl.contains("-- Procedure: app.archive_orders(older_than date)"), "{ddl}");
        // PG 14+ `pg_get_functiondef` yazarken `IN` modunu açıkça ekler; ikisi de geçerli söz dizimidir.
        assert!(
            ddl.contains("CREATE OR REPLACE PROCEDURE app.archive_orders(older_than date)")
                || ddl.contains("CREATE OR REPLACE PROCEDURE app.archive_orders(IN older_than date)"),
            "{ddl}"
        );
        assert!(ddl.contains("COMMENT ON PROCEDURE app.archive_orders"), "{ddl}");
    }
}

#[tokio::test]
async fn function_grants_are_listed() {
    for fx in fixtures().await {
        let schema = fx.schema("app").await;
        let f = fx.child_of(&schema, Coll::Functions, "add_tax(amount numeric, rate numeric)").await;
        let ddl = fx.catalog.ddl(&f).await.unwrap();
        assert!(ddl.contains("GRANT EXECUTE ON FUNCTION app.add_tax(amount numeric, rate numeric) TO pgt_reader;"), "{ddl}");
    }
}

#[tokio::test]
async fn missing_oid_ddl_is_not_found_and_collections_are_empty() {
    for fx in fixtures().await {
        let ghost = 4_000_000_000u32;
        let schema = fx.schema("app").await;
        let customers = fx.table("app", "customers").await;
        for id in [
            customers.child(Seg::Constraint(ghost)),
            customers.child(Seg::Index(ghost)),
            customers.child(Seg::Trigger(ghost)),
            customers.child(Seg::Policy(ghost)),
            schema.child(Seg::View(ghost)),
            schema.child(Seg::Sequence(ghost)),
            schema.child(Seg::Function(ghost)),
            schema.child(Seg::Type(ghost)),
            schema.child(Seg::Domain(ghost)),
            fx.db().child(Seg::Extension(ghost)),
            fx.server_id().child(Seg::Role(ghost)),
            fx.server_id().child(Seg::Tablespace(ghost)),
        ] {
            assert!(matches!(fx.catalog.ddl(&id).await, Err(Error::NotFound(_))), "{id}");
        }
        assert_eq!(fx.catalog.ddl(&customers.child(Seg::Coll(Coll::Indexes))).await.unwrap(), "");
    }
}
