mod common;

use common::fixtures;
use pgt_catalog::node::{Coll, NodeId, Seg};

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

#[tokio::test]
async fn customers_ddl_has_rls_policies_indexes_triggers_and_column_grants() {
    for fx in fixtures().await {
        let ddl = fx.catalog.ddl(&fx.table("app", "customers").await).await.unwrap();
        for part in [
            "ALTER TABLE IF EXISTS app.customers ENABLE ROW LEVEL SECURITY;",
            "CREATE POLICY customers_own\n    ON app.customers\n    AS RESTRICTIVE\n    FOR UPDATE\n    TO PUBLIC\n    USING (id > 0)\n    WITH CHECK (id > 0);",
            "CREATE POLICY customers_select\n    ON app.customers\n    AS PERMISSIVE\n    FOR SELECT\n    TO pgt_reader\n    USING (true);",
            "CREATE UNIQUE INDEX customers_lower_name_idx ON app.customers USING btree (lower((name)::text));",
            "CREATE TRIGGER customers_touch BEFORE UPDATE ON app.customers FOR EACH ROW",
            "GRANT SELECT (email) ON TABLE app.customers TO pgt_app;",
        ] {
            assert!(ddl.contains(part), "'{part}' eksik:\n{ddl}");
        }
        // kısıta bağlı index'ler (customers_pkey, customers_email_key) ayrıca yazılmaz
        assert!(!ddl.contains("CREATE UNIQUE INDEX customers_pkey"), "{ddl}");
        assert!(!ddl.contains("CREATE UNIQUE INDEX customers_email_key"), "{ddl}");
    }
}

/// Gidiş-dönüş testinde bulundu: Table DDL'i index ve trigger yorumlarını yazmıyordu.
#[tokio::test]
async fn table_ddl_keeps_index_and_trigger_comments() {
    for fx in fixtures().await {
        let customers = fx.catalog.ddl(&fx.table("app", "customers").await).await.unwrap();
        let trigger_comment = "COMMENT ON TRIGGER customers_touch ON app.customers
    IS 'Dokunma';";
        assert!(customers.contains(trigger_comment), "{customers}");
        let orders = fx.catalog.ddl(&fx.table("app", "orders").await).await.unwrap();
        let index_comment = "COMMENT ON INDEX app.orders_customer_idx
    IS 'Müşteri dizini';";
        assert!(orders.contains(index_comment), "{orders}");
        for part in [
            "COMMENT ON CONSTRAINT customers_email_check ON app.customers\n    IS 'E-posta biçimi';",
            "    USING (true);\nCOMMENT ON POLICY customers_select ON app.customers\n    IS 'Okuma politikası';",
        ] {
            assert!(customers.contains(part), "'{part}' eksik:\n{customers}");
        }
    }
}

#[tokio::test]
async fn unlogged_forced_rls_and_disabled_trigger() {
    for fx in fixtures().await {
        let log = fx.catalog.ddl(&fx.table("extra", "audit_log").await).await.unwrap();
        assert!(log.contains("CREATE UNLOGGED TABLE IF NOT EXISTS extra.audit_log"), "{log}");
        let secrets = fx.catalog.ddl(&fx.table("extra", "secrets").await).await.unwrap();
        for part in [
            "ALTER TABLE IF EXISTS extra.secrets ENABLE ROW LEVEL SECURITY;",
            "ALTER TABLE IF EXISTS extra.secrets FORCE ROW LEVEL SECURITY;",
            // PG 10'da EXECUTE PROCEDURE, 11+ EXECUTE FUNCTION yazılır
            "CREATE TRIGGER secrets_touch BEFORE INSERT ON extra.secrets FOR EACH ROW EXECUTE",
            "app.touch_row();\nALTER TABLE IF EXISTS extra.secrets DISABLE TRIGGER secrets_touch;",
        ] {
            assert!(secrets.contains(part), "'{part}' eksik:\n{secrets}");
        }
    }
}

#[tokio::test]
async fn inherited_table_lists_only_local_columns() {
    for fx in fixtures().await {
        let ddl = fx.catalog.ddl(&fx.table("extra", "audit_child").await).await.unwrap();
        assert!(ddl.contains("extra_col integer"), "{ddl}");
        assert!(!ddl.contains("note text") && !ddl.contains("id bigint"), "devralınan kolonlar yazılmamalı:\n{ddl}");
        assert!(ddl.contains("INHERITS (extra.audit_base)"), "{ddl}");
    }
}

#[tokio::test]
async fn identity_options_are_kept() {
    for fx in fixtures().await {
        let ddl = fx.catalog.ddl(&fx.table("extra", "tickets").await).await.unwrap();
        assert!(
            ddl.contains("id integer NOT NULL GENERATED BY DEFAULT AS IDENTITY (START WITH 10 INCREMENT BY 2 MINVALUE 1 MAXVALUE 2147483647 CACHE 1)"),
            "{ddl}"
        );
        // varsayılan seçenekli identity kolon (customers.id) seçenek yazmaz
        let customers = fx.catalog.ddl(&fx.table("app", "customers").await).await.unwrap();
        assert!(customers.contains("id bigint NOT NULL GENERATED ALWAYS AS IDENTITY,"), "{customers}");
    }
}

#[tokio::test]
async fn partitions_use_partition_of_and_skip_inherited_indexes() {
    for fx in fixtures().await {
        let events = fx.table("app", "events").await;
        let parts = fx.nodes_of(&events, Coll::Partitions).await;
        let part = NodeId::parse(&parts[0].id).unwrap();
        let ddl = fx.catalog.ddl(&part).await.unwrap();
        assert!(
            ddl.contains("CREATE TABLE IF NOT EXISTS app.events_2026 PARTITION OF app.events (\n    CONSTRAINT events_2026_day_check CHECK (happened_on > '2025-12-31'::date)\n)\n    FOR VALUES FROM ('2026-01-01') TO ('2027-01-01');"),
            "{ddl}"
        );
        assert!(!ddl.contains("CREATE INDEX"), "üst tablodan gelen index'ler yazılmamalı:\n{ddl}");
        if fx.major().await >= 11 {
            let parent = fx.catalog.ddl(&events).await.unwrap();
            assert!(parent.contains("CREATE INDEX events_day_idx ON ONLY app.events"), "{parent}");
            let hits = fx.table("extra", "hits").await;
            let hash_parts = fx.nodes_of(&hits, Coll::Partitions).await;
            assert_eq!(hash_parts.len(), 2);
            let hash_ddl = fx.catalog.ddl(&NodeId::parse(&hash_parts[0].id).unwrap()).await.unwrap();
            assert!(hash_ddl.contains("PARTITION OF extra.hits\n    FOR VALUES WITH (modulus 2, remainder 0);"), "{hash_ddl}");
            // üst tablodaki satır trigger'ı partition'a klonlanır ama partition DDL'inde tekrar yazılmaz
            let hits_ddl = fx.catalog.ddl(&hits).await.unwrap();
            assert!(hits_ddl.contains("CREATE TRIGGER hits_audit AFTER INSERT ON extra.hits FOR EACH ROW"), "{hits_ddl}");
            assert!(!hash_ddl.contains("CREATE TRIGGER"), "klonlanmış trigger partition DDL'inde tekrarlanmamalı:\n{hash_ddl}");
        }
    }
}

#[tokio::test]
async fn generated_columns_on_pg12_and_newer() {
    for fx in fixtures().await {
        if fx.major().await < 12 {
            continue;
        }
        let ddl = fx.catalog.ddl(&fx.table("extra", "line_items").await).await.unwrap();
        // ifade biçimi (`(qty * price)` / `((qty)::numeric * price)`) sunucu sürümüne göre değişir
        assert!(ddl.contains("total numeric(12,2) GENERATED ALWAYS AS (") && ddl.contains("price)) STORED"), "{ddl}");
    }
}

#[tokio::test]
async fn full_table_ddl_snapshots_for_new_tables() {
    for fx in fixtures().await {
        for (schema, table) in [("extra", "audit_child"), ("extra", "tickets")] {
            let ddl = fx.catalog.ddl(&fx.table(schema, table).await).await.unwrap();
            snapshot!(fx, format!("table_{schema}_{table}"), ddl);
        }
    }
}
