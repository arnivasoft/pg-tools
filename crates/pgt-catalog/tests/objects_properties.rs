mod common;

use common::fixtures;
use pgt_catalog::node::{Coll, Seg};
use pgt_catalog::props::PropertySection;
use pgt_core::Error;

fn prop<'a>(sections: &'a [PropertySection], title: &str, label: &str) -> Option<&'a str> {
    sections.iter().find(|s| s.title == title)?.rows.iter().find(|r| r.label == label).map(|r| r.value.as_str())
}

#[tokio::test]
async fn constraint_properties() {
    for fx in fixtures().await {
        let customers = fx.table("app", "customers").await;
        let pk = fx.child_of(&customers, Coll::Constraints, "customers_pkey").await;
        let s = fx.catalog.properties(&pk).await.unwrap();
        assert_eq!(prop(&s, "Genel", "Ad"), Some("customers_pkey"));
        assert_eq!(prop(&s, "Genel", "Tablo"), Some("app.customers"));
        assert_eq!(prop(&s, "Tanım", "Tür"), Some("primary key"));
        assert_eq!(prop(&s, "Tanım", "Tanım"), Some("PRIMARY KEY (id)"));
        assert_eq!(prop(&s, "Tanım", "Ertelenebilir"), Some("hayır"));
        assert_eq!(prop(&s, "Tanım", "Doğrulanmış"), Some("evet"));
        assert_eq!(prop(&s, "Genel", "Yorum"), None, "boş değerli satır gösterilmez");
    }
}

#[tokio::test]
async fn index_trigger_and_policy_properties() {
    for fx in fixtures().await {
        let customers = fx.table("app", "customers").await;

        let idx = fx.child_of(&customers, Coll::Indexes, "customers_lower_name_idx").await;
        let s = fx.catalog.properties(&idx).await.unwrap();
        assert_eq!(prop(&s, "Genel", "Tablo"), Some("customers"));
        assert_eq!(prop(&s, "Tanım", "Erişim yöntemi"), Some("btree"));
        assert!(prop(&s, "Tanım", "Tanım").unwrap().contains("CREATE UNIQUE INDEX customers_lower_name_idx ON app.customers"));
        assert_eq!(prop(&s, "Tanım", "Benzersiz"), Some("evet"));
        assert_eq!(prop(&s, "Tanım", "Birincil anahtar"), Some("hayır"));
        assert!(prop(&s, "İstatistik", "Boyut").is_some());

        let trg = fx.child_of(&customers, Coll::Triggers, "customers_touch").await;
        let s = fx.catalog.properties(&trg).await.unwrap();
        assert_eq!(prop(&s, "Genel", "Yorum"), Some("Dokunma"));
        assert_eq!(prop(&s, "Tanım", "Durum"), Some("etkin"));
        assert_eq!(prop(&s, "Tanım", "Fonksiyon"), Some("app.touch_row"));
        assert!(prop(&s, "Tanım", "Tanım").unwrap().contains("BEFORE UPDATE ON app.customers"));

        let pol = fx.child_of(&customers, Coll::Policies, "customers_own").await;
        let s = fx.catalog.properties(&pol).await.unwrap();
        assert_eq!(prop(&s, "Tanım", "Komut"), Some("UPDATE"));
        assert_eq!(prop(&s, "Tanım", "Tür"), Some("restrictive"));
        assert_eq!(prop(&s, "Tanım", "Roller"), Some("PUBLIC"));
        assert_eq!(prop(&s, "Tanım", "USING"), Some("id > 0"));
        assert_eq!(prop(&s, "Tanım", "WITH CHECK"), Some("id > 0"));
    }
}

#[tokio::test]
async fn view_and_materialized_view_properties() {
    for fx in fixtures().await {
        let schema = fx.schema("app").await;
        let view = fx.child_of(&schema, Coll::Views, "customer_orders").await;
        let s = fx.catalog.properties(&view).await.unwrap();
        assert_eq!(prop(&s, "Genel", "Yorum"), Some("Müşteri siparişleri"));
        assert_eq!(prop(&s, "Tanım", "Tür"), Some("View"));
        assert_eq!(prop(&s, "Tanım", "Seçenekler"), Some("security_barrier=true"));
        assert_eq!(prop(&s, "Tanım", "Doldurulmuş"), None);

        let mv = fx.child_of(&schema, Coll::MatViews, "order_totals").await;
        let s = fx.catalog.properties(&mv).await.unwrap();
        assert_eq!(prop(&s, "Tanım", "Tür"), Some("Materialized view"));
        assert_eq!(prop(&s, "Tanım", "Doldurulmuş"), Some("evet"));
        assert!(prop(&s, "İstatistik", "Boyut").is_some());
    }
}

#[tokio::test]
async fn sequence_properties() {
    for fx in fixtures().await {
        let schema = fx.schema("app").await;
        let seq = fx.child_of(&schema, Coll::Sequences, "invoice_seq").await;
        let s = fx.catalog.properties(&seq).await.unwrap();
        assert_eq!(prop(&s, "Genel", "Yorum"), Some("Fatura numarası"));
        assert_eq!(prop(&s, "Tanım", "Veri tipi"), Some("bigint"));
        assert_eq!(prop(&s, "Tanım", "Başlangıç"), Some("100"));
        assert_eq!(prop(&s, "Tanım", "Artış"), Some("5"));
        assert_eq!(prop(&s, "Tanım", "En küçük"), Some("100"));
        assert_eq!(prop(&s, "Tanım", "En büyük"), Some("100000"));
        assert_eq!(prop(&s, "Tanım", "Önbellek"), Some("10"));
        assert_eq!(prop(&s, "Tanım", "Döngü"), Some("evet"));
        let owned = fx.child_of(&schema, Coll::Sequences, "orders_id_seq").await;
        let s = fx.catalog.properties(&owned).await.unwrap();
        assert_eq!(prop(&s, "Tanım", "Sahibi olan kolon"), Some("app.orders.id"));
    }
}

#[tokio::test]
async fn function_properties_resolve_overloads_by_oid() {
    for fx in fixtures().await {
        let schema = fx.schema("app").await;
        let tax = fx.child_of(&schema, Coll::Functions, "add_tax(amount numeric, rate numeric)").await;
        let s = fx.catalog.properties(&tax).await.unwrap();
        assert_eq!(prop(&s, "Genel", "Ad"), Some("add_tax"));
        assert_eq!(prop(&s, "Genel", "Yorum"), Some("KDV ekler"));
        assert_eq!(prop(&s, "Tanım", "Argümanlar"), Some("amount numeric, rate numeric DEFAULT 0.18"));
        assert_eq!(prop(&s, "Tanım", "Dönüş tipi"), Some("numeric"));
        assert_eq!(prop(&s, "Tanım", "Dil"), Some("sql"));
        assert_eq!(prop(&s, "Tanım", "Değişkenlik"), Some("immutable"));

        let int_fmt = fx.child_of(&schema, Coll::Functions, "fmt(v integer)").await;
        let text_fmt = fx.child_of(&schema, Coll::Functions, "fmt(v text)").await;
        assert_ne!(int_fmt, text_fmt);
        assert_eq!(prop(&fx.catalog.properties(&int_fmt).await.unwrap(), "Tanım", "Argümanlar"), Some("v integer"));
        assert_eq!(prop(&fx.catalog.properties(&text_fmt).await.unwrap(), "Tanım", "Argümanlar"), Some("v text"));

        let trg_fn = fx.child_of(&schema, Coll::TriggerFunctions, "touch_row()").await;
        assert_eq!(prop(&fx.catalog.properties(&trg_fn).await.unwrap(), "Tanım", "Dönüş tipi"), Some("trigger"));
        if fx.major().await >= 11 {
            let proc = fx.child_of(&schema, Coll::Procedures, "archive_orders(older_than date)").await;
            let s = fx.catalog.properties(&proc).await.unwrap();
            assert_eq!(prop(&s, "Genel", "Yorum"), Some("Eski siparişleri siler"));
            assert_eq!(prop(&s, "Tanım", "Argümanlar"), Some("older_than date"));
        }
    }
}

#[tokio::test]
async fn type_and_domain_properties() {
    for fx in fixtures().await {
        let schema = fx.schema("app").await;
        let en = fx.child_of(&schema, Coll::Types, "order_status").await;
        let s = fx.catalog.properties(&en).await.unwrap();
        assert_eq!(prop(&s, "Tanım", "Tür"), Some("enum"));
        assert_eq!(prop(&s, "Tanım", "Etiketler"), Some("'new', 'paid', 'shipped'"));
        assert_eq!(prop(&s, "Genel", "Yorum"), Some("Sipariş durumu"));

        let comp = fx.child_of(&schema, Coll::Types, "address").await;
        let s = fx.catalog.properties(&comp).await.unwrap();
        assert_eq!(prop(&s, "Tanım", "Alanlar"), Some("street text, city text, zip character varying(10)"));

        let range = fx.child_of(&schema, Coll::Types, "int_span").await;
        assert_eq!(prop(&fx.catalog.properties(&range).await.unwrap(), "Tanım", "Alt tip"), Some("integer"));

        let dom = fx.child_of(&schema, Coll::Domains, "positive_int").await;
        let s = fx.catalog.properties(&dom).await.unwrap();
        assert_eq!(prop(&s, "Tanım", "Tür"), Some("domain"));
        assert_eq!(prop(&s, "Tanım", "Taban tip"), Some("integer"));
        assert_eq!(prop(&s, "Tanım", "Boş olamaz"), Some("evet"));
        assert_eq!(prop(&s, "Tanım", "Varsayılan"), Some("1"));
    }
}

#[tokio::test]
async fn extension_role_and_tablespace_properties() {
    for fx in fixtures().await {
        let ext = fx.child_of(&fx.db(), Coll::Extensions, "pg_trgm").await;
        let s = fx.catalog.properties(&ext).await.unwrap();
        assert_eq!(prop(&s, "Tanım", "Şema"), Some("extra"));
        assert!(prop(&s, "Tanım", "Sürüm").is_some());

        let role = fx.child_of(&fx.server_id(), Coll::Roles, "pgt_app").await;
        let s = fx.catalog.properties(&role).await.unwrap();
        assert_eq!(prop(&s, "Yetkiler", "Oturum açabilir"), Some("evet"));
        assert_eq!(prop(&s, "Yetkiler", "Superuser"), Some("hayır"));
        assert_eq!(prop(&s, "Tanım", "Bağlantı sınırı"), Some("5"));
        assert_eq!(prop(&s, "Tanım", "Üye olduğu roller"), Some("pgt_reader"));

        fx.catalog.set_show_system(true);
        let ts = fx.child_of(&fx.server_id(), Coll::Tablespaces, "pg_default").await;
        fx.catalog.set_show_system(false);
        let s = fx.catalog.properties(&ts).await.unwrap();
        assert_eq!(prop(&s, "Genel", "Ad"), Some("pg_default"));
        assert!(prop(&s, "İstatistik", "Boyut").is_some());
    }
}

#[tokio::test]
async fn missing_oid_is_not_found_for_every_kind() {
    for fx in fixtures().await {
        let ghost = 4_000_000_000u32;
        let schema = fx.schema("app").await;
        let customers = fx.table("app", "customers").await;
        let ids = [
            customers.child(Seg::Constraint(ghost)),
            customers.child(Seg::Index(ghost)),
            customers.child(Seg::Trigger(ghost)),
            customers.child(Seg::Policy(ghost)),
            schema.child(Seg::View(ghost)),
            schema.child(Seg::MatView(ghost)),
            schema.child(Seg::Sequence(ghost)),
            schema.child(Seg::Function(ghost)),
            schema.child(Seg::Procedure(ghost)),
            schema.child(Seg::TriggerFunction(ghost)),
            schema.child(Seg::Type(ghost)),
            schema.child(Seg::Domain(ghost)),
            fx.db().child(Seg::Extension(ghost)),
            fx.server_id().child(Seg::Role(ghost)),
            fx.server_id().child(Seg::Tablespace(ghost)),
        ];
        for id in ids {
            assert!(matches!(fx.catalog.properties(&id).await, Err(Error::NotFound(_))), "{id}");
        }
        // koleksiyonlar için Properties boş
        assert!(fx.catalog.properties(&customers.child(Seg::Coll(Coll::Indexes))).await.unwrap().is_empty());
    }
}

#[tokio::test]
async fn view_and_matview_columns_have_properties_but_no_ddl() {
    for fx in fixtures().await {
        let schema = fx.schema("app").await;
        for (coll, rel, col) in [(Coll::Views, "customer_orders", "email"), (Coll::MatViews, "order_totals", "total")] {
            let rel_id = fx.child_of(&schema, coll, rel).await;
            let col_id = fx.child_of(&rel_id, Coll::Columns, col).await;
            assert_eq!(fx.catalog.ddl(&col_id).await.unwrap(), "", "{col_id}");
            assert!(!fx.catalog.properties(&col_id).await.unwrap().is_empty(), "{col_id}");
        }
    }
}

#[tokio::test]
async fn role_valid_until_infinity_is_valid_ddl() {
    for fx in fixtures().await {
        let admin = pgt_core::testing::admin_client(&fx.ts, &fx.ts.config.maintenance_db).await;
        admin.batch_execute("DROP ROLE IF EXISTS pgt_forever; CREATE ROLE pgt_forever NOLOGIN VALID UNTIL 'infinity'").await.unwrap();
        let role = fx.child_of(&fx.server_id(), Coll::Roles, "pgt_forever").await;
        let ddl = fx.catalog.ddl(&role).await.unwrap();
        assert!(ddl.contains("VALID UNTIL 'infinity'"), "{ddl}");
        let s = fx.catalog.properties(&role).await.unwrap();
        assert_eq!(prop(&s, "Tanım", "Geçerlilik sonu"), Some("infinity"));
        admin.batch_execute("DROP ROLE pgt_forever").await.unwrap();
    }
}
