use pgt_core::Result;
use pgt_core::connection::Target;
use uuid::Uuid;

use crate::Catalog;
use crate::db::{p, query};
use crate::node::{NodeFlags, NodeId, NodeKind, Seg, TreeNode};
use crate::props::approx_rows;
use crate::version::VersionedSql;

static LIST: VersionedSql = VersionedSql(&[(10, include_str!("../sql/tables.sql"))]);

pub(crate) async fn list(cat: &Catalog, server: Uuid, db: u32, schema: u32) -> Result<Vec<TreeNode>> {
    let rows = query(&cat.mgr, server, Target::Database(db), &LIST, p![schema]).await?;
    let base = NodeId::server(server).child(Seg::Db(db)).child(Seg::Schema(schema));
    Ok(rows
        .iter()
        .map(|r| {
            TreeNode::new(&base.child(Seg::Table(r.get("oid"))), NodeKind::Table, r.get::<_, String>("name"))
                .with_detail(approx_rows(r.get("reltuples")))
                .with_flags(NodeFlags { partitioned: r.get("partitioned"), ..NodeFlags::default() })
        })
        .collect())
}

static PROPS: VersionedSql = VersionedSql(&[(10, include_str!("../sql/table_props.sql"))]);

pub(crate) async fn properties(cat: &Catalog, server: Uuid, db: u32, table: u32) -> Result<Vec<crate::props::PropertySection>> {
    use crate::props::PropertySection;
    let r = crate::db::query_one(&cat.mgr, server, Target::Database(db), &PROPS, p![table], "tablo").await?;
    let text = |c: &str| r.get::<_, Option<String>>(c);
    Ok(vec![
        PropertySection::new("Genel")
            .row("Ad", r.get::<_, String>("name"))
            .row("Şema", r.get::<_, String>("schema"))
            .row("OID", r.get::<_, u32>("oid").to_string())
            .row("Sahip", r.get::<_, String>("owner"))
            .row_opt("Yorum", text("comment")),
        PropertySection::new("Tanım")
            .row("Tür", r.get::<_, String>("kind"))
            .row("Kalıcılık", r.get::<_, String>("persistence"))
            .row("Tablespace", r.get::<_, String>("tablespace"))
            .row_opt("Seçenekler", text("options")),
        PropertySection::new("İstatistik")
            .row_opt("Tahmini satır", approx_rows(r.get("reltuples")))
            .row_opt("Toplam boyut", text("total_size"))
            .row_opt("Tablo boyutu", text("table_size"))
            .row_opt("İndeks boyutu", text("indexes_size"))
            .row_opt("Son vacuum", text("last_vacuum"))
            .row_opt("Son autovacuum", text("last_autovacuum"))
            .row_opt("Son analyze", text("last_analyze"))
            .row_opt("Son autoanalyze", text("last_autoanalyze")),
    ])
}

static HEAD: VersionedSql = VersionedSql(&[(10, include_str!("../sql/table_ddl_head.sql"))]);
pub(crate) static COLUMNS: VersionedSql = VersionedSql(&[
    (10, include_str!("../sql/table_ddl_columns.pg10.sql")),
    (12, include_str!("../sql/table_ddl_columns.sql")),
]);
static CONSTRAINTS: VersionedSql = VersionedSql(&[(10, include_str!("../sql/table_ddl_constraints.sql"))]);
static ACL: VersionedSql = VersionedSql(&[(10, include_str!("../sql/acl_relation.sql"))]);

pub(crate) struct TableHead {
    pub qname: String,
    pub owner: String,
    pub partkey: Option<String>,
    pub unlogged: bool,
    pub options: Option<String>,
    pub tablespace: Option<String>,
    pub comment: Option<String>,
}

pub(crate) async fn head(cat: &Catalog, server: Uuid, db: u32, table: u32) -> Result<TableHead> {
    let r = crate::db::query_one(&cat.mgr, server, Target::Database(db), &HEAD, p![table], "tablo").await?;
    Ok(TableHead {
        qname: r.get("qname"),
        owner: r.get("owner"),
        partkey: r.get("partkey"),
        unlogged: r.get("unlogged"),
        options: r.get("options"),
        tablespace: r.get("tablespace"),
        comment: r.get("comment"),
    })
}

pub(crate) async fn ddl(cat: &Catalog, server: Uuid, db: u32, table: u32) -> Result<String> {
    use crate::ddl::{ColumnDef, column_line, comment, grants, header, join_blocks, owner};
    let h = head(cat, server, db, table).await?;
    let target = Target::Database(db);
    let cols: Vec<ColumnDef> = crate::db::query(&cat.mgr, server, target, &COLUMNS, p![table, None::<i16>])
        .await?
        .iter()
        .map(ColumnDef::from_row)
        .collect();
    let constraints = crate::db::query(&cat.mgr, server, target, &CONSTRAINTS, p![table]).await?;
    let acl = super::acl(cat, server, target, &ACL, table).await?;

    let mut lines: Vec<String> = cols.iter().map(column_line).collect();
    lines.extend(constraints.iter().map(|k| format!("CONSTRAINT {} {}", k.get::<_, String>("name"), k.get::<_, String>("def"))));
    let body = lines.iter().map(|l| format!("    {l}")).collect::<Vec<_>>().join(",\n");
    let mut create = format!(
        "CREATE {}TABLE IF NOT EXISTS {}\n(\n{body}\n)",
        if h.unlogged { "UNLOGGED " } else { "" },
        h.qname
    );
    if let Some(key) = &h.partkey {
        create.push_str(&format!("\nPARTITION BY {key}"));
    }
    if let Some(options) = h.options.as_deref().filter(|o| !o.is_empty()) {
        create.push_str(&format!("\nWITH ({options})"));
    }
    if let Some(ts) = &h.tablespace {
        create.push_str(&format!("\nTABLESPACE {ts}"));
    }
    create.push_str(";\n");

    let mut blocks = vec![create, owner("TABLE IF EXISTS", &h.qname, &h.owner), comment("TABLE", &h.qname, h.comment.as_deref())];
    for c in &cols {
        blocks.push(comment("COLUMN", &format!("{}.{}", h.qname, c.name), c.comment.as_deref()));
    }
    blocks.push(grants("TABLE", &h.qname, &acl));
    Ok(header("Table", &h.qname, &format!("DROP TABLE IF EXISTS {};", h.qname)) + &join_blocks(blocks))
}
