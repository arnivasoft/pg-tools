mod common;

use std::collections::BTreeSet;

use common::{Fixture, fixtures};
use pgt_catalog::node::{Coll, NodeId, TreeNode};
use pgt_core::testing::admin_client;

const SIGNATURE_SQL: &str = include_str!("fixtures/signature.sql");
const SCRATCH_DB: &str = "pgt_roundtrip";

fn parse(node: &TreeNode) -> NodeId {
    NodeId::parse(&node.id).unwrap()
}

/// `(başlık satırı, DDL)` çiftleri: fixture'daki her nesne için.
async fn collect_ddls(fx: &Fixture) -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = Vec::new();
    let mut push = |ddl: String| {
        let title = ddl.lines().next().unwrap_or("").to_string();
        out.push((title, ddl));
    };

    for node in fx.nodes_of(&fx.db(), Coll::Extensions).await {
        if node.label != "plpgsql" {
            push(fx.catalog.ddl(&parse(&node)).await.unwrap());
        }
    }
    for schema_name in ["app", "extra"] {
        let schema = fx.schema(schema_name).await;
        push(fx.catalog.ddl(&schema).await.unwrap());
        let mut colls = vec![Coll::Types, Coll::Domains, Coll::Sequences, Coll::Functions, Coll::TriggerFunctions, Coll::Views, Coll::MatViews];
        if fx.major().await >= 11 {
            colls.push(Coll::Procedures);
        }
        for coll in colls {
            for node in fx.nodes_of(&schema, coll).await {
                if !node.flags.from_extension {
                    push(fx.catalog.ddl(&parse(&node)).await.unwrap());
                }
            }
        }
        // tablolar ve (özyinelemeli) partition'ları
        let mut stack: Vec<NodeId> = fx.nodes_of(&schema, Coll::Tables).await.iter().map(parse).collect();
        while let Some(id) = stack.pop() {
            push(fx.catalog.ddl(&id).await.unwrap());
            let kids = fx.catalog.list_children(&id).await.unwrap();
            if kids.iter().any(|k| k.label == "Partitions") {
                stack.extend(fx.nodes_of(&id, Coll::Partitions).await.iter().map(parse));
            }
        }
    }
    out
}

/// Sequence DDL'inin `OWNED BY` bloklarını ayırır (tablo olmadan uygulanamaz).
fn split_owned_by(ddl: &str) -> (String, Vec<String>) {
    if !ddl.starts_with("-- Sequence:") {
        return (ddl.to_string(), Vec::new());
    }
    let (owned, rest): (Vec<&str>, Vec<&str>) = ddl.split("\n\n").partition(|b| b.starts_with("ALTER SEQUENCE") && b.contains("OWNED BY"));
    (rest.join("\n\n"), owned.into_iter().map(str::to_string).collect())
}

async fn signature(db: &tokio_postgres::Client) -> BTreeSet<(String, String, String)> {
    db.query(SIGNATURE_SQL, &[])
        .await
        .unwrap()
        .iter()
        .map(|r| (r.get::<_, String>(0), r.get::<_, String>(1), r.get::<_, String>(2)))
        .collect()
}

#[tokio::test]
async fn ddl_round_trip_reproduces_the_catalog() {
    for fx in fixtures().await {
        let ddls = collect_ddls(&fx).await;
        assert!(ddls.len() >= 25, "beklenenden az nesne toplandı: {}", ddls.len());

        let admin = admin_client(&fx.ts, &fx.ts.config.maintenance_db).await;
        admin
            .execute(
                "SELECT pg_terminate_backend(pid) FROM pg_stat_activity WHERE datname = $1 AND pid <> pg_backend_pid()",
                &[&SCRATCH_DB],
            )
            .await
            .unwrap();
        admin.batch_execute(&format!("DROP DATABASE IF EXISTS {SCRATCH_DB}")).await.unwrap();
        admin.batch_execute(&format!("CREATE DATABASE {SCRATCH_DB} TEMPLATE template0 ENCODING 'UTF8'")).await.unwrap();
        let scratch = admin_client(&fx.ts, SCRATCH_DB).await;

        let mut deferred: Vec<String> = Vec::new();
        let mut pending: Vec<(String, String)> = ddls
            .iter()
            .map(|(title, ddl)| {
                let (ddl, owned) = split_owned_by(ddl);
                deferred.extend(owned);
                (title.clone(), ddl)
            })
            .collect();

        for _pass in 0..12 {
            let before = pending.len();
            let mut next = Vec::new();
            for (title, ddl) in pending {
                if scratch.batch_execute(&ddl).await.is_err() {
                    next.push((title, ddl));
                }
            }
            pending = next;
            if pending.is_empty() || pending.len() == before {
                break;
            }
        }
        if !pending.is_empty() {
            let mut report = String::new();
            for (title, ddl) in &pending {
                let err = scratch.batch_execute(ddl).await.unwrap_err();
                let msg = err.as_db_error().map(|d| d.message().to_string()).unwrap_or_else(|| err.to_string());
                report.push_str(&format!("{title}\n  hata: {msg}\n"));
            }
            panic!("{} DDL uygulanamadı:\n{report}", pending.len());
        }
        for stmt in deferred {
            scratch.batch_execute(&stmt).await.unwrap_or_else(|e| panic!("OWNED BY uygulanamadı: {e}\n{stmt}"));
        }

        let source = admin_client(&fx.ts, common::FIXTURE_DB).await;
        let (want, got) = (signature(&source).await, signature(&scratch).await);
        let missing: Vec<_> = want.difference(&got).take(20).collect();
        let extra: Vec<_> = got.difference(&want).take(20).collect();
        assert!(
            missing.is_empty() && extra.is_empty(),
            "katalog imzaları farklı\nyalnızca fixture'da ({}):\n{missing:#?}\nyalnızca gidiş-dönüş veritabanında ({}):\n{extra:#?}",
            want.difference(&got).count(),
            got.difference(&want).count()
        );

        drop(scratch);
        admin
            .execute(
                "SELECT pg_terminate_backend(pid) FROM pg_stat_activity WHERE datname = $1 AND pid <> pg_backend_pid()",
                &[&SCRATCH_DB],
            )
            .await
            .unwrap();
        admin.batch_execute(&format!("DROP DATABASE {SCRATCH_DB}")).await.unwrap();
    }
}
