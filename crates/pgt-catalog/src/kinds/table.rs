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
static INDEXES: VersionedSql = VersionedSql(&[(10, include_str!("../sql/table_ddl_indexes.sql"))]);
static TRIGGERS: VersionedSql = VersionedSql(&[
    (10, include_str!("../sql/table_ddl_triggers.pg10.sql")),
    (13, include_str!("../sql/table_ddl_triggers.sql")),
]);
static POLICIES: VersionedSql = VersionedSql(&[(10, include_str!("../sql/table_ddl_policies.sql"))]);
static COLUMN_ACL: VersionedSql = VersionedSql(&[(10, include_str!("../sql/table_ddl_column_acl.sql"))]);

pub(crate) struct TableHead {
    pub qname: String,
    pub owner: String,
    pub partkey: Option<String>,
    pub unlogged: bool,
    pub options: Option<String>,
    pub tablespace: Option<String>,
    pub comment: Option<String>,
    pub partition_of: Option<String>,
    pub partition_bound: Option<String>,
    pub inherits: Option<String>,
    pub rls_enabled: bool,
    pub rls_forced: bool,
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
        partition_of: r.get("partition_of"),
        partition_bound: r.get("partition_bound"),
        inherits: r.get("inherits"),
        rls_enabled: r.get("rls_enabled"),
        rls_forced: r.get("rls_forced"),
    })
}

pub(crate) async fn ddl(cat: &Catalog, server: Uuid, db: u32, table: u32) -> Result<String> {
    use crate::ddl::{ColumnDef, column_line, comment, grants, header, join_blocks, owner};
    let target = Target::Database(db);
    let q = |sql: &'static VersionedSql, params: crate::db::Params| query(&cat.mgr, server, target, sql, params);
    // Aynı bağlantıda eş zamanlı (pipeline) çalışır: tek tur gidiş-dönüşe yakın süre.
    let (h, col_rows, constraints, acl, indexes, triggers, policies, col_acl) = tokio::try_join!(
        head(cat, server, db, table),
        q(&COLUMNS, p![table, None::<i16>]),
        q(&CONSTRAINTS, p![table]),
        super::acl(cat, server, target, &ACL, table),
        q(&INDEXES, p![table]),
        q(&TRIGGERS, p![table]),
        q(&POLICIES, p![table]),
        q(&COLUMN_ACL, p![table]),
    )?;
    let cols: Vec<ColumnDef> = col_rows.iter().map(ColumnDef::from_row).collect();

    let unlogged = if h.unlogged { "UNLOGGED " } else { "" };
    let mut create = match (&h.partition_of, &h.partition_bound) {
        (Some(parent), Some(bound)) => {
            format!("CREATE {unlogged}TABLE IF NOT EXISTS {} PARTITION OF {parent}\n    {bound}", h.qname)
        }
        _ => {
            let mut lines: Vec<String> = cols.iter().map(column_line).collect();
            lines.extend(constraints.iter().map(|k| format!("CONSTRAINT {} {}", k.get::<_, String>("name"), k.get::<_, String>("def"))));
            let body = lines.iter().map(|l| format!("    {l}")).collect::<Vec<_>>().join(",\n");
            let mut s = format!("CREATE {unlogged}TABLE IF NOT EXISTS {}\n(\n{body}\n)", h.qname);
            if let Some(parents) = &h.inherits {
                s.push_str(&format!("\nINHERITS ({parents})"));
            }
            s
        }
    };
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
    blocks.push(col_acl.iter().map(|r| format!("{}\n", r.get::<_, String>("stmt"))).collect());
    if h.rls_enabled {
        blocks.push(format!("ALTER TABLE IF EXISTS {} ENABLE ROW LEVEL SECURITY;\n", h.qname));
    }
    if h.rls_forced {
        blocks.push(format!("ALTER TABLE IF EXISTS {} FORCE ROW LEVEL SECURITY;\n", h.qname));
    }
    blocks.extend(policies.iter().map(|r| format!("{}\n", r.get::<_, String>("def"))));
    blocks.push(indexes.iter().map(|r| format!("{}\n", r.get::<_, String>("def"))).collect());
    blocks.push(
        triggers
            .iter()
            .map(|r| {
                let mut s = format!("{}\n", r.get::<_, String>("def"));
                if let Some(state) = r.get::<_, Option<String>>("state") {
                    s.push_str(&format!("ALTER TABLE IF EXISTS {} {state} TRIGGER {};\n", h.qname, r.get::<_, String>("name")));
                }
                s
            })
            .collect(),
    );
    Ok(header("Table", &h.qname, &format!("DROP TABLE IF EXISTS {};", h.qname)) + &join_blocks(blocks))
}
