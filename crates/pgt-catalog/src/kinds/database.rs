use pgt_core::Result;
use pgt_core::connection::Target;
use uuid::Uuid;

use crate::Catalog;
use crate::db::{p, query};
use crate::node::{NodeId, NodeKind, Seg, TreeNode};
use crate::version::VersionedSql;

static LIST: VersionedSql = VersionedSql(&[(10, include_str!("../sql/databases.sql"))]);

pub(crate) async fn list(cat: &Catalog, server: Uuid) -> Result<Vec<TreeNode>> {
    let rows = query(&cat.mgr, server, Target::Maintenance, &LIST, p![]).await?;
    Ok(rows
        .iter()
        .map(|r| {
            let oid: u32 = r.get("oid");
            let node = TreeNode::new(&NodeId::server(server).child(Seg::Db(oid)), NodeKind::Database, r.get::<_, String>("name"));
            if r.get::<_, bool>("allow_conn") { node } else { node.leaf() }
        })
        .collect())
}

static PROPS: VersionedSql = VersionedSql(&[(10, include_str!("../sql/database_props.sql"))]);

pub(crate) async fn properties(cat: &Catalog, server: Uuid, db: u32) -> Result<Vec<crate::props::PropertySection>> {
    use crate::props::{PropertySection, yes_no};
    let r = crate::db::query_one(&cat.mgr, server, Target::Maintenance, &PROPS, p![db], "veritabanı").await?;
    Ok(vec![
        PropertySection::new("Genel")
            .row("Ad", r.get::<_, String>("name"))
            .row("OID", r.get::<_, u32>("oid").to_string())
            .row("Sahip", r.get::<_, String>("owner"))
            .row_opt("Yorum", r.get::<_, Option<String>>("comment")),
        PropertySection::new("Tanım")
            .row("Encoding", r.get::<_, String>("encoding"))
            .row("Collation", r.get::<_, String>("collate"))
            .row("Character type", r.get::<_, String>("ctype"))
            .row("Tablespace", r.get::<_, String>("tablespace"))
            .row("Bağlantı sınırı", r.get::<_, i32>("conn_limit").to_string())
            .row("Bağlantıya izin", yes_no(r.get("allow_conn"))),
        PropertySection::new("İstatistik").row_opt("Boyut", r.get::<_, Option<String>>("size")),
    ])
}
