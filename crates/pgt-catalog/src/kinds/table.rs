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
