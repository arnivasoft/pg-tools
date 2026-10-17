//! Veri sözleşmeli SQL dosyalarını `TreeNode` / `PropertySection` / DDL metnine çeviren genel motor.

use pgt_core::connection::Target;
use pgt_core::pgtext::{PgText, lossy, lossy_opt};
use pgt_core::{Error, Result};
use uuid::Uuid;

use crate::Catalog;
use crate::db::{p, query, query_ddl};
use crate::node::{Coll, NodeFlags, NodeId, NodeKind, Seg, TreeNode};
use crate::props::PropertySection;
use super::registry::{DdlDef, PropsDef};
use crate::ddl::{grants, header, join_blocks};
use crate::version::VersionedSql;

/// Sorgunun hangi bağlantıda çalışacağı.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Level {
    /// Sunucu geneli kataloglar (roller, tablespace'ler): bakım veritabanı bağlantısı.
    Server,
    /// Veritabanına özel kataloglar.
    Database,
}

pub(crate) fn target(level: Level, db: Option<u32>) -> Result<Target> {
    match (level, db) {
        (Level::Server, _) => Ok(Target::Maintenance),
        (Level::Database, Some(d)) => Ok(Target::Database(d)),
        (Level::Database, None) => Err(Error::Invalid("veritabanı kimliği yok".into())),
    }
}

/// Bir koleksiyonun (ve çocuklarının) tanımı.
pub(crate) struct CollDef {
    pub coll: Coll,
    pub label: &'static str,
    pub coll_kind: NodeKind,
    pub child_kind: NodeKind,
    pub seg: fn(u32) -> Seg,
    pub list: &'static VersionedSql,
    pub level: Level,
    /// Liste sorgusu `$1` olarak üst nesnenin OID'ini alır mı.
    pub takes_parent: bool,
    /// Koleksiyonun gösterildiği en düşük PostgreSQL ana sürümü.
    pub min_version: u32,
}

/// `parent` altında koleksiyon düğümünü üretir.
pub(crate) fn collection_node(parent: &NodeId, def: &CollDef) -> TreeNode {
    TreeNode::new(&parent.child(Seg::Coll(def.coll)), def.coll_kind, def.label)
}

/// Koleksiyonun çocuklarını listeler. Çocuk kimlikleri koleksiyon parçasını atlar: `parent.child(seg(oid))`.
pub(crate) async fn list(
    cat: &Catalog,
    server: Uuid,
    db: Option<u32>,
    def: &CollDef,
    parent: &NodeId,
    parent_oid: Option<u32>,
) -> Result<Vec<TreeNode>> {
    let params = match (def.takes_parent, parent_oid) {
        (true, Some(oid)) => p![oid],
        (true, None) => return Err(Error::Invalid(format!("{} için üst nesne yok", def.label))),
        (false, _) => p![],
    };
    let rows = query(&cat.mgr, server, target(def.level, db)?, def.list, params).await?;
    let show_system = cat.show_system();
    let mut out = Vec::with_capacity(rows.len());
    for r in &rows {
        let flag = |c: &str| r.try_get::<_, bool>(c).unwrap_or(false);
        if flag("system") && !show_system {
            continue;
        }
        let oid: u32 = r.get("oid");
        let mut node = TreeNode::new(&parent.child((def.seg)(oid)), def.child_kind, lossy(r, "name"))
            .with_detail(r.try_get::<_, Option<PgText>>("detail").ok().flatten().map(PgText::into_lossy))
            .with_flags(NodeFlags {
                partitioned: flag("partitioned"),
                system: flag("system"),
                primary_key: flag("primary_key"),
                not_null: flag("not_null"),
                from_extension: flag("from_extension"),
                ..NodeFlags::default()
            });
        if r.try_get::<_, bool>("leaf").unwrap_or(true) {
            node = node.leaf();
        }
        out.push(node);
    }
    Ok(out)
}

/// Properties SQL'inin `(section, label, value)` satırlarını bölümlere çevirir.
pub(crate) async fn properties(
    cat: &Catalog,
    server: Uuid,
    db: Option<u32>,
    def: &PropsDef,
    oid: u32,
) -> Result<Vec<PropertySection>> {
    let rows = query(&cat.mgr, server, target(def.level, db)?, def.sql, p![oid]).await?;
    if rows.is_empty() {
        return Err(Error::NotFound(def.what.to_string()));
    }
    let mut sections: Vec<PropertySection> = Vec::new();
    for r in &rows {
        let title: String = r.get("section");
        let label: String = r.get("label");
        let value = lossy_opt(r, "value");
        if sections.last().is_none_or(|s| s.title != title) {
            sections.push(PropertySection::new(title));
        }
        let last = sections.pop().expect("az önce eklendi");
        sections.push(last.row_opt(label, value));
    }
    Ok(sections.into_iter().filter(|s| !s.rows.is_empty()).collect())
}

static ACL_RELATION: VersionedSql = VersionedSql(&[(10, include_str!("../sql/acl_relation.sql"))]);
static ACL_PROC: VersionedSql = VersionedSql(&[(10, include_str!("../sql/acl_proc.sql"))]);
static ACL_TYPE: VersionedSql = VersionedSql(&[(10, include_str!("../sql/acl_type.sql"))]);
static ACL_TABLESPACE: VersionedSql = VersionedSql(&[(10, include_str!("../sql/acl_tablespace.sql"))]);

fn acl_sql(catalog: &str) -> Result<&'static VersionedSql> {
    match catalog {
        "relation" => Ok(&ACL_RELATION),
        "proc" => Ok(&ACL_PROC),
        "type" => Ok(&ACL_TYPE),
        "tablespace" => Ok(&ACL_TABLESPACE),
        other => Err(Error::Invalid(format!("bilinmeyen ACL kataloğu: {other}"))),
    }
}

/// DDL SQL'inin tek satırlık sonucunu pgAdmin biçimli metne çevirir.
pub(crate) async fn ddl(cat: &Catalog, server: Uuid, db: Option<u32>, def: &DdlDef, oid: u32) -> Result<String> {
    let target = target(def.level, db)?;
    let rows = query_ddl(&cat.mgr, server, target, def.sql, p![oid]).await?;
    let r = rows.first().ok_or_else(|| Error::NotFound(def.what.to_string()))?;
    let text = |c: &str| r.get::<_, Option<String>>(c);
    let acl = match text("acl_catalog") {
        Some(name) => super::acl(cat, server, target, acl_sql(&name)?, oid).await?,
        None => Vec::new(),
    };
    let grants_block = match (text("acl_kw"), text("acl_target")) {
        (Some(kw), Some(target_name)) => grants(&kw, &target_name, &acl),
        _ => String::new(),
    };
    let blocks = vec![
        r.get::<_, String>("body"),
        // OWNER TO, OWNED BY'dan önce gelmeli: dizi tabloyla aynı sahibe sahip olmadan bağlanamaz.
        text("owner_stmt").unwrap_or_default(),
        text("extra_stmt").unwrap_or_default(),
        text("comment_stmt").unwrap_or_default(),
        grants_block,
    ];
    Ok(header(&r.get::<_, String>("title"), &r.get::<_, String>("qname"), &r.get::<_, String>("drop_stmt")) + &join_blocks(blocks))
}
