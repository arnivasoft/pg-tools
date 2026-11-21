mod common;

use common::foreign::{assert_no_secret, setup};
use common::{find, fixtures, labels};
use pgt_catalog::node::{Coll, NodeId, NodeKind, Seg};

#[tokio::test]
async fn foreign_and_replication_nodes_list_with_labels_details_and_no_secret() {
    for fx in fixtures().await {
        let major = fx.major().await;
        let owner = &fx.ts.config.username;
        let st = setup(&fx, "pgt_cat_fdw_list").await;
        let (db_oid, db_id) = (st.db_oid, st.db_id.clone());

        // Veritabanı çocuk sırası.
        let kids = fx.catalog.list_children(&db_id).await.unwrap();
        assert_eq!(
            labels(&kids),
            ["Schemas", "Extensions", "Foreign Data Wrappers", "Publications", "Subscriptions"]
        );
        assert_eq!(kids[2].kind, NodeKind::ForeignDataWrapperCollection);
        assert_eq!(kids[3].kind, NodeKind::PublicationCollection);
        assert_eq!(kids[4].kind, NodeKind::SubscriptionCollection);
        // Çocuk kimlikleri koleksiyon parçasını atlar (motor kuralı): `db:D/fdw:N`, `…/fdw:N/fsrv:S`, `…/fsrv:S/um:U`.
        assert_eq!(kids[2].id, format!("{db_id}/fdws"));
        assert!(pg_fdw_id_ok(&find(&fx.nodes_of(&db_id, Coll::ForeignDataWrappers).await, "postgres_fdw").id, &db_id));

        // FDW: handler ayrıntısı, eklenti bayrağı, alt koleksiyon.
        let fdws = fx.nodes_of(&db_id, Coll::ForeignDataWrappers).await;
        assert_eq!(labels(&fdws), ["pgt_plain_fdw", "postgres_fdw"]);
        let plain = find(&fdws, "pgt_plain_fdw");
        assert_eq!((plain.kind, plain.detail.as_deref(), plain.flags.from_extension, plain.has_children), (NodeKind::ForeignDataWrapper, None, false, true));
        let pg_fdw = find(&fdws, "postgres_fdw");
        assert_eq!(pg_fdw.detail.as_deref(), Some("postgres_fdw_handler"));
        assert!(pg_fdw.flags.from_extension && pg_fdw.has_children);
        let fdw_id = NodeId::parse(&pg_fdw.id).unwrap();
        assert_eq!(fdw_id.to_string(), pg_fdw.id);
        let fdw_kids = fx.catalog.list_children(&fdw_id).await.unwrap();
        assert_eq!(labels(&fdw_kids), ["Foreign Servers"]);
        assert_eq!(fdw_kids[0].kind, NodeKind::ForeignServerCollection);
        assert_eq!(fdw_kids[0].id, format!("{}/fservers", pg_fdw.id));

        // Server: tür/sürüm ayrıntısı, alt koleksiyon.
        let servers = fx.nodes_of(&fdw_id, Coll::ForeignServers).await;
        assert_eq!(labels(&servers), ["pgt_srv", "pgt_srv_typed"]);
        assert_eq!(find(&servers, "pgt_srv").detail, None);
        assert_eq!(find(&servers, "pgt_srv_typed").detail.as_deref(), Some("pg · 16"));
        let srv = find(&servers, "pgt_srv");
        assert_eq!((srv.kind, srv.has_children), (NodeKind::ForeignServer, true));
        assert!(srv.id.starts_with(&format!("{}/fsrv:", pg_fdw.id)));
        let srv_id = NodeId::parse(&srv.id).unwrap();
        let srv_kids = fx.catalog.list_children(&srv_id).await.unwrap();
        assert_eq!(labels(&srv_kids), ["User Mappings"]);

        // User mapping: PUBLIC önce, sonra rol; yaprak; ayrıntı yok.
        let maps = fx.nodes_of(&srv_id, Coll::UserMappings).await;
        assert_eq!(labels(&maps), ["PUBLIC", owner.as_str()]);
        for m in &maps {
            assert_eq!((m.kind, m.has_children, m.detail.as_deref()), (NodeKind::UserMapping, false, None));
            assert!(m.id.starts_with(&format!("{}/um:", srv.id)), "{}", m.id);
        }
        assert!(fx.nodes_of(&NodeId::parse(&servers[1].id).unwrap(), Coll::UserMappings).await.is_empty());

        // Foreign table: Tables'ta yok, ftables'ta var, kolonları listelenir.
        let public = fx.child_of(&db_id, Coll::Schemas, "public").await;
        let schema_kids = fx.catalog.list_children(&public).await.unwrap();
        let mut expected = vec!["Tables", "Views", "Materialized Views", "Foreign Tables", "Functions"];
        if major >= 11 {
            expected.push("Procedures");
        }
        expected.extend(["Trigger Functions", "Sequences", "Types", "Domains"]);
        assert_eq!(labels(&schema_kids), expected);
        let tables = fx.nodes_of(&public, Coll::Tables).await;
        assert_eq!(labels(&tables), ["pub_t"]);
        let ftables = fx.nodes_of(&public, Coll::ForeignTables).await;
        assert_eq!(labels(&ftables), ["ft_remote"]);
        let ft = &ftables[0];
        assert_eq!((ft.kind, ft.detail.as_deref(), ft.has_children), (NodeKind::ForeignTable, Some("pgt_srv"), true));
        assert!(ft.id.contains("/ftbl:"), "{}", ft.id);
        let ft_id = NodeId::parse(&ft.id).unwrap();
        let ft_kids = fx.catalog.list_children(&ft_id).await.unwrap();
        assert_eq!(labels(&ft_kids), ["Columns"]);
        let cols = fx.nodes_of(&ft_id, Coll::Columns).await;
        assert_eq!(labels(&cols), ["id", "label"]);
        assert_eq!(cols[1].detail.as_deref(), Some("text"));

        // Publication: ayrıntı ve yaprak.
        let pubs = fx.nodes_of(&db_id, Coll::Publications).await;
        let mut expected_pubs = vec!["pgt_pub_all", "pgt_pub_tbl"];
        if major >= 15 {
            expected_pubs.insert(1, "pgt_pub_sch");
        }
        assert_eq!(labels(&pubs), expected_pubs);
        assert_eq!(find(&pubs, "pgt_pub_all").detail.as_deref(), Some("all tables"));
        assert_eq!(find(&pubs, "pgt_pub_tbl").detail.as_deref(), Some("1 tablo"));
        if major >= 15 {
            assert_eq!(find(&pubs, "pgt_pub_sch").detail.as_deref(), Some("0 tablo, 1 şema"));
        }
        assert!(pubs.iter().all(|p| p.kind == NodeKind::Publication && !p.has_children && p.id.starts_with(&format!("{db_id}/pub:"))));

        // Subscription: yalnız bu veritabanının abonelikleri, ayrıntı = etkin/devre dışı.
        let subs = fx.nodes_of(&db_id, Coll::Subscriptions).await;
        assert_eq!(labels(&subs), ["pgt_sub"]);
        assert_eq!((subs[0].kind, subs[0].detail.as_deref(), subs[0].has_children), (NodeKind::Subscription, Some("devre dışı"), false));
        assert!(subs[0].id.starts_with(&format!("{db_id}/sub:")));

        // Ham sır hiçbir düğümde yok.
        let mut all = Vec::new();
        for list in [&fdws, &servers, &maps, &ftables, &pubs, &subs, &kids, &cols] {
            all.extend(list.iter().cloned());
        }
        assert_no_secret(&all);

        // Süper kullanıcı olmayan rolle: liste hatasız (conninfo okunmaz; PG 10–13'te oid `pg_stat_subscription`'dan).
        let limited = st.connect_plain(&fx).await;
        let ldb = NodeId::server(limited).child(Seg::Db(db_oid));
        let lsubs = fx.catalog.list_children(&ldb.child(Seg::Coll(Coll::Subscriptions))).await.unwrap();
        assert_eq!(labels(&lsubs), ["pgt_sub"]);
        assert_eq!(lsubs[0].id, subs[0].id.replacen(&db_id.to_string(), &ldb.to_string(), 1));
        assert_eq!((lsubs[0].kind, lsubs[0].detail.as_deref(), lsubs[0].has_children), (NodeKind::Subscription, Some("devre dışı"), false));
        assert_no_secret(&lsubs);
        let lpubs = fx.catalog.list_children(&ldb.child(Seg::Coll(Coll::Publications))).await.unwrap();
        assert_eq!(lpubs.len(), expected_pubs.len());
        let lfdws = fx.catalog.list_children(&ldb.child(Seg::Coll(Coll::ForeignDataWrappers))).await.unwrap();
        assert_eq!(lfdws.len(), 2);
        let lmaps = fx
            .catalog
            .list_children(&ldb.child(Seg::ForeignDataWrapper(fdw_oid(&pg_fdw.id))).child(Seg::ForeignServer(srv_oid(&srv.id))).child(Seg::Coll(Coll::UserMappings)))
            .await
            .unwrap();
        // Yetkisiz rol de eşlemeleri görür (`pg_user_mappings` herkese açık; yalnız seçenekler gizli): aynı kimlikler.
        assert_eq!(labels(&lmaps), ["PUBLIC", owner.as_str()]);
        for (l, m) in lmaps.iter().zip(&maps) {
            assert_eq!(l.id, m.id.replacen(&db_id.to_string(), &ldb.to_string(), 1));
            assert_eq!((l.kind, l.has_children, l.detail.as_deref()), (NodeKind::UserMapping, false, None));
        }
        assert_no_secret(&lmaps);
        fx.mgr.disconnect(limited);

        st.teardown(&fx).await;
    }
}

fn pg_fdw_id_ok(id: &str, db_id: &NodeId) -> bool {
    id.starts_with(&format!("{db_id}/fdw:"))
}

fn fdw_oid(id: &str) -> u32 {
    let NodeId::Node { path, .. } = NodeId::parse(id).unwrap() else { panic!() };
    path.iter().find_map(|s| if let Seg::ForeignDataWrapper(o) = s { Some(*o) } else { None }).unwrap()
}

fn srv_oid(id: &str) -> u32 {
    let NodeId::Node { path, .. } = NodeId::parse(id).unwrap() else { panic!() };
    path.iter().find_map(|s| if let Seg::ForeignServer(o) = s { Some(*o) } else { None }).unwrap()
}
