mod common;

use common::{Fixture, fixtures};
use pgt_catalog::Catalog;
use pgt_catalog::node::{NodeId, NodeKind};
use pgt_catalog::search::{SearchHit, SearchKind};
use pgt_core::Error;

/// Sonucun zincirini sunucu düğümünden başlayarak `list_children` ile yürür; her halka önceki halkanın
/// çocuğu olmalı, sonuç düğümü de son halkanın çocuğu olmalı.
async fn walk(cat: &Catalog, hit: &SearchHit, server_id: &str) {
    let mut parent = server_id.to_string();
    for (i, p) in hit.path.iter().enumerate() {
        let list = cat.list_children(&NodeId::parse(&parent).unwrap()).await.unwrap();
        assert!(list.iter().any(|n| &n.id == p), "{:?} sonucu: zincirin {i}. halkası ağaçta yok", hit.kind);
        parent = p.clone();
    }
    let list = cat.list_children(&NodeId::parse(&parent).unwrap()).await.unwrap();
    assert!(list.iter().any(|n| n.id == hit.id), "{:?} sonucu: düğümün kendisi son halkanın çocukları arasında yok", hit.kind);
}

async fn search(fx: &Fixture, q: &str, kinds: &[SearchKind]) -> Vec<SearchHit> {
    let r = fx.catalog.search(fx.server, fx.db_oid, q, kinds).await.unwrap();
    assert!(!r.truncated);
    r.hits
}

#[tokio::test]
async fn every_hit_is_reachable_in_tree() {
    for fx in fixtures().await {
        let r = fx.catalog.search(fx.server, fx.db_oid, "srch", &[]).await.unwrap();
        assert!(!r.truncated);
        let kinds: Vec<NodeKind> = r.hits.iter().map(|h| h.kind).collect();
        let mut want = vec![
            NodeKind::Schema, NodeKind::Table, NodeKind::View, NodeKind::MaterializedView, NodeKind::Function,
            NodeKind::TriggerFunction, NodeKind::Sequence, NodeKind::Type, NodeKind::Domain, NodeKind::Column,
            NodeKind::Index, NodeKind::Constraint, NodeKind::Trigger, NodeKind::Policy,
        ];
        if fx.major().await >= 11 {
            want.push(NodeKind::Procedure);
        }
        for k in want {
            assert!(kinds.contains(&k), "{k:?} türünde sonuç yok");
        }
        assert!(r.hits.iter().any(|h| h.name.starts_with("srch_part_leaf")));
        let sid = fx.server_id().to_string();
        for h in &r.hits {
            walk(&fx.catalog, h, &sid).await;
        }
    }
}

#[tokio::test]
async fn excludes_hidden_objects() {
    for fx in fixtures().await {
        assert!(search(&fx, "srch_ident_id_seq", &[]).await.is_empty(), "kimlik sequence'ı sonuçta olmamalı");
        assert!(search(&fx, "RI_ConstraintTrigger", &[]).await.is_empty(), "iç trigger sonuçta olmamalı");
        let tbl = search(&fx, "srch_tbl", &[]).await;
        assert!(tbl.iter().any(|h| h.kind == NodeKind::Table));
        assert!(!tbl.iter().any(|h| h.kind == NodeKind::Type), "tablo satır tipi sonuçta olmamalı");
        let en = search(&fx, "srch_enum", &[]).await;
        assert!(en.iter().any(|h| h.kind == NodeKind::Type && h.name == "srch_enum"));
        assert!(!en.iter().any(|h| h.name == "_srch_enum"), "dizi tipi sonuçta olmamalı");
    }
}

#[tokio::test]
async fn partition_chain_uses_root_schema() {
    for fx in fixtures().await {
        let schema_a = fx.schema("srch_a").await.to_string();
        let schema_b = fx.schema("srch_b").await.to_string();
        let hits = search(&fx, "srch_part_leaf", &[]).await;
        let leaf = hits.iter().find(|h| h.kind == NodeKind::Table).expect("yaprak bölüm bulunamadı");
        assert!(leaf.path.contains(&schema_a), "zincir kök şemadan geçmeli");
        assert!(!leaf.path.contains(&schema_b), "zincir bölümün kendi şemasından geçmemeli");
        walk(&fx.catalog, leaf, &fx.server_id().to_string()).await;
    }
}

#[tokio::test]
async fn wildcards_are_literal() {
    for fx in fixtures().await {
        let h = search(&fx, "srch%x", &[]).await;
        assert!(h.iter().any(|h| h.name == "srch%x" && h.kind == NodeKind::Table));
        assert!(!h.iter().any(|h| h.name == "srchzx"), "% joker olarak yorumlandı");
        let h = search(&fx, "SRCH_MIXED", &[]).await;
        assert!(h.iter().any(|h| h.kind == NodeKind::Column && h.name == "Srch_Mixed"));
        let h = search(&fx, "_under_", &[]).await;
        assert!(h.iter().any(|h| h.name == "srch_under_x"));
        assert!(!h.iter().any(|h| h.name == "srch_tbl"), "_ joker olarak yorumlandı");
    }
}

#[tokio::test]
async fn kinds_filter() {
    for fx in fixtures().await {
        let h = search(&fx, "srch", &[SearchKind::Column]).await;
        assert!(!h.is_empty());
        assert!(h.iter().all(|h| h.kind == NodeKind::Column));
    }
}

#[tokio::test]
async fn system_objects_follow_setting() {
    for fx in fixtures().await {
        fx.catalog.set_show_system(false);
        let h = search(&fx, "pg_class", &[]).await;
        assert!(!h.iter().any(|h| h.system), "sistem sonucu gizli olmalı");
        fx.catalog.set_show_system(true);
        let h = search(&fx, "pg_class", &[]).await;
        let sys = h.iter().find(|h| h.system && h.kind == NodeKind::Table && h.name == "pg_class" && h.schema.as_deref() == Some("pg_catalog"));
        let sys = sys.expect("pg_catalog.pg_class bulunamadı");
        walk(&fx.catalog, sys, &fx.server_id().to_string()).await;
        fx.catalog.set_show_system(false);
    }
}

#[tokio::test]
async fn ordering_exact_then_prefix() {
    for fx in fixtures().await {
        let h = search(&fx, "srch_tbl", &[]).await;
        assert_eq!(h[0].name, "srch_tbl");
        assert_eq!(h[0].kind, NodeKind::Table);
    }
}

#[tokio::test]
async fn blank_and_too_long() {
    for fx in fixtures().await {
        let r = fx.catalog.search(fx.server, fx.db_oid, "  ", &[]).await.unwrap();
        assert!(r.hits.is_empty() && !r.truncated);
        let err = fx.catalog.search(fx.server, fx.db_oid, &"x".repeat(201), &[]).await.unwrap_err();
        assert!(matches!(err, Error::Invalid(_)));
    }
}
