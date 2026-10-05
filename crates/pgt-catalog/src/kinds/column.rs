use pgt_core::Result;
use pgt_core::connection::Target;
use uuid::Uuid;

use crate::Catalog;
use crate::db::{p, query};
use crate::node::{NodeFlags, NodeId, NodeKind, Seg, TreeNode};
use crate::version::VersionedSql;

static LIST: VersionedSql = VersionedSql(&[(10, include_str!("../sql/columns.sql"))]);

pub(crate) async fn list(cat: &Catalog, server: Uuid, db: u32, schema: u32, table: u32) -> Result<Vec<TreeNode>> {
    let rows = query(&cat.mgr, server, Target::Database(db), &LIST, p![table]).await?;
    let base = NodeId::server(server).child(Seg::Db(db)).child(Seg::Schema(schema)).child(Seg::Table(table));
    Ok(rows
        .iter()
        .map(|r| {
            TreeNode::new(&base.child(Seg::Column(r.get("attnum"))), NodeKind::Column, r.get::<_, String>("name"))
                .with_detail(Some(r.get("type_name")))
                .with_flags(NodeFlags { primary_key: r.get("is_pk"), not_null: r.get("not_null"), ..NodeFlags::default() })
                .leaf()
        })
        .collect())
}

static PROPS: VersionedSql = VersionedSql(&[
    (10, include_str!("../sql/column_props.pg10.sql")),
    (12, include_str!("../sql/column_props.sql")),
]);

pub(crate) async fn properties(cat: &Catalog, server: Uuid, db: u32, table: u32, attnum: i16) -> Result<Vec<crate::props::PropertySection>> {
    use crate::props::{PropertySection, yes_no};
    let r = crate::db::query_one(&cat.mgr, server, Target::Database(db), &PROPS, p![table, attnum], "kolon").await?;
    let text = |c: &str| r.get::<_, Option<String>>(c);
    let generated = text("generated");
    let (default_label, default_value) = match &generated {
        Some(kind) => (format!("Üretilen ifade ({kind})"), text("default_expr")),
        None => ("Varsayılan".to_string(), text("default_expr")),
    };
    Ok(vec![
        PropertySection::new("Genel")
            .row("Ad", r.get::<_, String>("name"))
            .row("Sıra", r.get::<_, i16>("attnum").to_string())
            .row("Veri tipi", r.get::<_, String>("type_name"))
            .row("Boş olamaz", yes_no(r.get("not_null")))
            .row_opt("Yorum", text("comment")),
        PropertySection::new("Tanım")
            .row_opt(default_label, default_value)
            .row_opt("Identity", text("identity"))
            .row_opt("Collation", text("collation")),
    ])
}

pub(crate) async fn ddl(cat: &Catalog, server: Uuid, db: u32, table: u32, attnum: i16) -> Result<String> {
    use crate::ddl::{ColumnDef, column_line, comment, header, join_blocks};
    let h = super::table::head(cat, server, db, table).await?;
    let row = crate::db::query_one(&cat.mgr, server, Target::Database(db), &super::table::COLUMNS, p![table, Some(attnum)], "kolon").await?;
    let c = ColumnDef::from_row(&row);
    let target = format!("{}.{}", h.qname, c.name);
    let add = format!("ALTER TABLE IF EXISTS {}\n    ADD COLUMN {};\n", h.qname, column_line(&c));
    Ok(header("Column", &target, &format!("ALTER TABLE IF EXISTS {} DROP COLUMN IF EXISTS {};", h.qname, c.name))
        + &join_blocks(vec![add, comment("COLUMN", &target, c.comment.as_deref())]))
}
