mod common;

use common::foreign::{SECRET, capture, setup};
use common::{Fixture, fixtures};
use pgt_catalog::Catalog;
use pgt_catalog::node::{NodeId, NodeKind};
use pgt_catalog::search::{SearchHit, SearchKind};
use pgt_core::Error;
use pgt_core::testing::admin_client;
use uuid::Uuid;

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

const NEW_KINDS: [SearchKind; 4] = [SearchKind::ForeignTable, SearchKind::ForeignServer, SearchKind::Publication, SearchKind::Subscription];

/// Yeni dört türün `(tür, ad)` çiftleri, sıralı.
fn new_kind_hits(hits: &[SearchHit]) -> Vec<(SearchKind, String)> {
    let mut v: Vec<_> = hits.iter().filter(|h| NEW_KINDS.contains(&h.search_kind)).map(|h| (h.search_kind, h.name.clone())).collect();
    v.sort_by(|a, b| format!("{a:?}").cmp(&format!("{b:?}")));
    v
}

fn assert_no_secret(hits: &[SearchHit]) {
    let json = serde_json::to_string(hits).unwrap();
    assert!(!json.contains(SECRET), "sır arama sonucunda göründü");
}

async fn search_on(fx: &Fixture, server: Uuid, db: u32, q: &str, kinds: &[SearchKind]) -> Vec<SearchHit> {
    let r = fx.catalog.search(server, db, q, kinds).await.unwrap();
    assert!(!r.truncated);
    assert_no_secret(&r.hits);
    r.hits
}

/// Foreign table/server, publication ve subscription (Plan 38 Task 3): ad parçasıyla bulunur, kimlik ağaçta adım adım
/// açılır, başka veritabanının aboneliği gelmez, yetkisiz rolde hata yok, sır sonuçta ve günlükte yok. Abonelik
/// kuran tüm denetimler bu tek testte sırayla (`pg_stat_subscription` küme genelidir).
#[tokio::test]
async fn foreign_and_replication_kinds_are_reachable_and_secret_free() {
    let log = capture();
    for fx in fixtures().await {
        Box::pin(check_foreign(&fx)).await;
    }
    assert!(!log.text().contains(SECRET), "sır günlükte göründü");
}

async fn check_foreign(fx: &Fixture) {
    let major = fx.major().await;
    let st = Box::pin(setup(fx, "pgt_cat_fdw_srch")).await;
    let sid = fx.server_id().to_string();
    let s = |k: SearchKind, n: &str| (k, n.to_string());

    // Süper kullanıcı: ad parçasıyla; FDW ve user mapping aranmaz, başka veritabanının aboneliği gelmez.
    let hits = search_on(fx, fx.server, st.db_oid, "pgt_", &[]).await;
    let mut want = vec![
        s(SearchKind::ForeignServer, "pgt_srv"), s(SearchKind::ForeignServer, "pgt_srv_typed"),
        s(SearchKind::Publication, "pgt_pub_all"), s(SearchKind::Publication, "pgt_pub_tbl"),
        s(SearchKind::Subscription, "pgt_sub"),
    ];
    if major >= 15 {
        want.push(s(SearchKind::Publication, "pgt_pub_sch"));
    }
    want.sort_by(|a, b| format!("{a:?}").cmp(&format!("{b:?}")));
    assert_eq!(new_kind_hits(&hits), want);
    assert!(!hits.iter().any(|h| h.name == "pgt_plain_fdw" || h.name == "pgt_sub_other"), "FDW aranmaz; öbür veritabanının aboneliği gelmez");
    let ft = search_on(fx, fx.server, st.db_oid, "t_remo", &[SearchKind::ForeignTable]).await;
    assert_eq!(ft.iter().map(|h| (h.kind, h.name.as_str(), h.schema.as_deref())).collect::<Vec<_>>(), [(NodeKind::ForeignTable, "ft_remote", Some("public"))]);
    assert!(!search_on(fx, fx.server, st.db_oid, "ft_remote", &[SearchKind::Table]).await.iter().any(|h| h.name == "ft_remote"), "foreign table tablo değil");
    let srv = hits.iter().find(|h| h.name == "pgt_srv").unwrap();
    assert_eq!((srv.kind, srv.owner.as_deref(), srv.schema.as_deref()), (NodeKind::ForeignServer, Some("postgres_fdw"), None));
    assert!(srv.id.contains("/fdw:") && !srv.id.contains("/fdws/") && !srv.id.contains("/fservers/"), "{}", srv.id);
    let sub = hits.iter().find(|h| h.search_kind == SearchKind::Subscription).unwrap().clone();
    let mut reached: Vec<&SearchHit> = hits.iter().filter(|h| NEW_KINDS.contains(&h.search_kind)).collect();
    reached.extend(&ft);
    for h in reached {
        walk(&fx.catalog, h, &sid).await;
    }
    // Tek tür süzgeci yalnız subscription'ı sorgular.
    assert_eq!(search_on(fx, fx.server, st.db_oid, "sub", &[SearchKind::Subscription]).await.iter().map(|h| h.id.as_str()).collect::<Vec<_>>(), [sub.id.as_str()]);

    // Yetkisiz rol: hata yok; PG 10–13'te oid `pg_stat_subscription` ad eşlemesinden (liste ile aynı kimlik).
    let limited = st.connect_plain(fx).await;
    let lsid = NodeId::server(limited).to_string();
    let relocate = |id: &str| id.replacen(&sid, &lsid, 1);
    let lhits = search_on(fx, limited, st.db_oid, "pgt_", &[]).await;
    assert_eq!(new_kind_hits(&lhits), want);
    let lsub = lhits.iter().find(|h| h.search_kind == SearchKind::Subscription).unwrap();
    assert_eq!(lsub.id, relocate(&sub.id));
    for h in lhits.iter().filter(|h| NEW_KINDS.contains(&h.search_kind)) {
        walk(&fx.catalog, h, &lsid).await;
    }
    let lonly = search_on(fx, limited, st.db_oid, "pgt_sub", &[SearchKind::Subscription]).await;
    assert_eq!(lonly.iter().map(|h| h.id.clone()).collect::<Vec<_>>(), [relocate(&sub.id)]);

    // Öbür veritabanında aynı adlı abonelik: süper kullanıcı her sürümde kendi aboneliğini (oid yolu) bulur; yetkisiz
    // rol PG 10–13'te ad belirsiz olduğundan (liste gibi) aboneliği göstermez ama hata da almaz.
    let other = admin_client(&fx.ts, &st.other_db).await;
    other
        .batch_execute(&format!(
            "CREATE SUBSCRIPTION pgt_sub CONNECTION 'host=127.0.0.1 port=1 dbname=o user=u password={SECRET}' \
             PUBLICATION other_pub WITH (connect = false, slot_name = NONE)"
        ))
        .await
        .unwrap();
    let dup = search_on(fx, fx.server, st.db_oid, "pgt_sub", &[]).await;
    assert_eq!(dup.iter().filter(|h| h.search_kind == SearchKind::Subscription).map(|h| h.id.as_str()).collect::<Vec<_>>(), [sub.id.as_str()]);
    let ldup = search_on(fx, limited, st.db_oid, "pgt_", &[]).await;
    let lsubs: Vec<&str> = ldup.iter().filter(|h| h.search_kind == SearchKind::Subscription).map(|h| h.id.as_str()).collect();
    if major <= 13 {
        assert!(lsubs.is_empty(), "{lsubs:?}");
    } else {
        assert_eq!(lsubs, [relocate(&sub.id).as_str()]);
    }
    assert!(ldup.iter().any(|h| h.search_kind == SearchKind::Publication), "öbür türler yedekte de aranır");
    fx.mgr.disconnect(limited);

    drop(other);
    st.teardown(fx).await;
}
