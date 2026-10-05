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

static DDL: VersionedSql = VersionedSql(&[(10, include_str!("../sql/database_ddl.sql"))]);
static ACL: VersionedSql = VersionedSql(&[(10, include_str!("../sql/acl_database.sql"))]);

pub(crate) async fn ddl(cat: &Catalog, server: Uuid, db: u32) -> Result<String> {
    use crate::ddl::{comment, grants, header, join_blocks, literal};
    let r = crate::db::query_one(&cat.mgr, server, Target::Maintenance, &DDL, p![db], "veritabanı").await?;
    let q: String = r.get("qname");
    let create = format!(
        "CREATE DATABASE {q}\n    WITH\n    OWNER = {}\n    ENCODING = {}\n    LC_COLLATE = {}\n    LC_CTYPE = {}\n    TABLESPACE = {}\n    CONNECTION LIMIT = {}\n    IS_TEMPLATE = {};\n",
        r.get::<_, String>("owner"),
        literal(&r.get::<_, String>("encoding")),
        literal(&r.get::<_, String>("collate")),
        literal(&r.get::<_, String>("ctype")),
        r.get::<_, String>("tablespace"),
        r.get::<_, i32>("conn_limit"),
        if r.get::<_, bool>("is_template") { "True" } else { "False" },
    );
    let acl = super::acl(cat, server, Target::Maintenance, &ACL, db).await?;
    Ok(header("Database", &q, &format!("DROP DATABASE IF EXISTS {q};"))
        + &join_blocks(vec![create, comment("DATABASE", &q, r.get::<_, Option<String>>("comment").as_deref()), grants("DATABASE", &q, &acl)]))
}
