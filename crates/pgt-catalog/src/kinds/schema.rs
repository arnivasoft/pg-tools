use pgt_core::Result;
use pgt_core::connection::Target;
use uuid::Uuid;

use crate::Catalog;
use crate::db::{p, query};
use crate::node::{NodeFlags, NodeId, NodeKind, Seg, TreeNode};
use crate::version::VersionedSql;

static LIST: VersionedSql = VersionedSql(&[(10, include_str!("../sql/schemas.sql"))]);

pub(crate) async fn list(cat: &Catalog, server: Uuid, db: u32) -> Result<Vec<TreeNode>> {
    let rows = query(&cat.mgr, server, Target::Database(db), &LIST, p![]).await?;
    let show_system = cat.show_system();
    let base = NodeId::server(server).child(Seg::Db(db));
    Ok(rows
        .iter()
        .filter(|r| show_system || !r.get::<_, bool>("is_system"))
        .map(|r| {
            TreeNode::new(&base.child(Seg::Schema(r.get("oid"))), NodeKind::Schema, r.get::<_, String>("name"))
                .with_flags(NodeFlags { system: r.get("is_system"), ..NodeFlags::default() })
        })
        .collect())
}

static PROPS: VersionedSql = VersionedSql(&[(10, include_str!("../sql/schema_props.sql"))]);

pub(crate) async fn properties(cat: &Catalog, server: Uuid, db: u32, schema: u32) -> Result<Vec<crate::props::PropertySection>> {
    use crate::props::PropertySection;
    let r = crate::db::query_one(&cat.mgr, server, Target::Database(db), &PROPS, p![schema], "şema").await?;
    Ok(vec![
        PropertySection::new("Genel")
            .row("Ad", r.get::<_, String>("name"))
            .row("OID", r.get::<_, u32>("oid").to_string())
            .row("Sahip", r.get::<_, String>("owner"))
            .row_opt("Yorum", r.get::<_, Option<String>>("comment")),
        PropertySection::new("Güvenlik").row_opt("Yetkiler", r.get::<_, Option<String>>("acl")),
    ])
}

static DDL: VersionedSql = VersionedSql(&[(10, include_str!("../sql/schema_ddl.sql"))]);
static ACL: VersionedSql = VersionedSql(&[(10, include_str!("../sql/acl_namespace.sql"))]);

pub(crate) async fn ddl(cat: &Catalog, server: Uuid, db: u32, schema: u32) -> Result<String> {
    use crate::ddl::{comment, grants, header, join_blocks};
    let r = crate::db::query_one(&cat.mgr, server, Target::Database(db), &DDL, p![schema], "şema").await?;
    let q: String = r.get("qname");
    let create = format!("CREATE SCHEMA IF NOT EXISTS {q}\n    AUTHORIZATION {};\n", r.get::<_, String>("owner"));
    let acl = super::acl(cat, server, Target::Database(db), &ACL, schema).await?;
    Ok(header("SCHEMA", &q, &format!("DROP SCHEMA IF EXISTS {q};"))
        + &join_blocks(vec![create, comment("SCHEMA", &q, r.get::<_, Option<String>>("comment").as_deref()), grants("SCHEMA", &q, &acl)]))
}
