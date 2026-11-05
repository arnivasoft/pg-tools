//! Asistan katalog araçlarının gerçek PostgreSQL'de (PG 10+) sınaması. `PGT_TEST_URLS` yoksa atlanır.
//! `pgt_ai_tools` veritabanını oluşturur/yeniden kurar.

use pgt_ai::tools::run;
use pgt_core::testing::{TestServer, admin_client, drop_database, test_servers};
use serde_json::json;

const DB: &str = "pgt_ai_tools";

const FIXTURE: &str = r#"
CREATE SCHEMA shop;
CREATE SCHEMA "Şube";
CREATE TABLE shop.customers (
    id int PRIMARY KEY,
    email text NOT NULL UNIQUE,
    created_at timestamptz DEFAULT now()
);
COMMENT ON TABLE shop.customers IS 'Müşteriler';
COMMENT ON COLUMN shop.customers.email IS 'İletişim adresi';
CREATE TABLE shop.orders (
    id bigint PRIMARY KEY,
    customer_id int NOT NULL REFERENCES shop.customers (id),
    total numeric(12, 2) CHECK (total >= 0)
);
CREATE INDEX orders_customer_idx ON shop.orders (customer_id);
CREATE TABLE shop.events (id int, at date NOT NULL) PARTITION BY RANGE (at);
CREATE TABLE shop.events_2026 PARTITION OF shop.events FOR VALUES FROM ('2026-01-01') TO ('2027-01-01');
CREATE VIEW shop.big_orders AS SELECT id, total FROM shop.orders WHERE total > 1000;
CREATE MATERIALIZED VIEW shop.order_counts AS SELECT customer_id, count(*) AS n FROM shop.orders GROUP BY customer_id;
CREATE TABLE "Şube"."İlçe" (kod int PRIMARY KEY, "Ad" text);
CREATE TABLE public.orders (id int PRIMARY KEY, customer_email text);
INSERT INTO shop.customers VALUES (1, 'gizli@ornek.com');
"#;

async fn tools_roundtrip(ts: TestServer) {
    let admin = admin_client(&ts, &ts.config.maintenance_db).await;
    drop_database(&admin, DB).await;
    admin.batch_execute(&format!("CREATE DATABASE {DB}")).await.unwrap();
    let c = admin_client(&ts, DB).await;
    c.batch_execute(FIXTURE).await.unwrap();

    // list_relations: bölüm (events_2026) listelenmez; şema süzgeci ve ad parçası.
    let out = run(&c, "list_relations", &json!({"schema": "shop", "name_contains": null})).await.unwrap().unwrap();
    for name in ["customers", "orders", "events", "big_orders", "order_counts"] {
        assert!(out.contains(&format!("shop\t{name}\t")), "{name} eksik:\n{out}");
    }
    assert!(!out.contains("events_2026"), "bölüm listelenmemeli:\n{out}");
    assert!(out.contains("partitioned table") && out.contains("materialized view") && out.contains("\tview\t"));
    assert!(out.contains("Müşteriler"));
    let out = run(&c, "list_relations", &json!({"schema": null, "name_contains": "ORD"})).await.unwrap().unwrap();
    assert!(out.contains("public\torders\t") && out.contains("shop\torders\t") && out.contains("shop\torder_counts\t"));
    assert!(!out.contains("customers"));
    let out = run(&c, "list_relations", &json!({"schema": "yok", "name_contains": null})).await.unwrap().unwrap();
    assert_eq!(out, "No matching relations.");

    // describe_relation: kolonlar, kısıtlar, gelen FK, indeksler; satır verisi asla yok.
    let out = run(&c, "describe_relation", &json!({"schema": "shop", "name": "customers"})).await.unwrap().unwrap();
    assert!(out.starts_with("shop.customers (table)"), "{out}");
    assert!(out.contains("email\ttext\tnot null\t\tİletişim adresi"), "{out}");
    assert!(out.contains("created_at\ttimestamp with time zone\tnull\tnow()"), "{out}");
    assert!(out.contains("PRIMARY KEY (id)") && out.contains("UNIQUE (email)"), "{out}");
    assert!(out.contains("referenced by:\nshop.orders (orders_customer_id_fkey): FOREIGN KEY (customer_id) REFERENCES shop.customers(id)"), "{out}");
    assert!(out.contains("indexes:"), "{out}");
    assert!(!out.contains("gizli@ornek.com"), "satır verisi sızdı:\n{out}");
    let out = run(&c, "describe_relation", &json!({"schema": "shop", "name": "orders"})).await.unwrap().unwrap();
    assert!(out.contains("CHECK (total >= 0") && out.contains("orders_customer_idx"), "{out}");
    let out = run(&c, "describe_relation", &json!({"schema": null, "name": "big_orders"})).await.unwrap().unwrap();
    assert!(out.contains("(view)") && out.contains("definition:") && out.contains("1000"), "{out}");
    let out = run(&c, "describe_relation", &json!({"schema": null, "name": "İlçe"})).await.unwrap().unwrap();
    assert!(out.starts_with("Şube.İlçe (table)") && out.contains("Ad\ttext"), "{out}");

    // Belirsiz ad ve bulunamayan ad: modele hata (çalıştırma hatası değil).
    let err = run(&c, "describe_relation", &json!({"schema": null, "name": "orders"})).await.unwrap().unwrap_err();
    assert!(err.contains("several schemas") && err.contains("public") && err.contains("shop"), "{err}");
    let err = run(&c, "describe_relation", &json!({"schema": null, "name": "yok"})).await.unwrap().unwrap_err();
    assert!(err.contains("not found"), "{err}");

    // search_columns: harf duyarsız, bölüm hariç.
    let out = run(&c, "search_columns", &json!({"name_contains": "EMAIL"})).await.unwrap().unwrap();
    assert!(out.contains("shop\tcustomers\temail\ttext") && out.contains("public\torders\tcustomer_email\ttext"), "{out}");
    let out = run(&c, "search_columns", &json!({"name_contains": "at"})).await.unwrap().unwrap();
    assert!(out.contains("shop\tevents\tat\tdate") && !out.contains("events_2026"), "{out}");

    // Geçersiz girdi sorgu çalıştırmadan hata döner.
    let err = run(&c, "search_columns", &json!({"name_contains": ""})).await.unwrap().unwrap_err();
    assert!(err.contains("empty"));

    drop(c);
    drop_database(&admin, DB).await;
}

#[tokio::test]
async fn catalog_tools_on_every_test_server() {
    let servers = test_servers();
    if servers.is_empty() {
        eprintln!("PGT_TEST_URLS yok; atlandı");
        return;
    }
    for ts in servers {
        tools_roundtrip(ts).await;
    }
}
