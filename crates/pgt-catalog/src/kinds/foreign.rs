//! Foreign data wrapper, foreign server, user mapping ve foreign table özellikleri/DDL'i (Plan 38 Karar 3–4).
//! Seçenek dizilerindeki `password` değerleri satır okunur okunmaz maskelenir (`crate::mask`).

use pgt_core::Result;
use pgt_core::connection::Target;
use pgt_core::pgtext::{lossy, lossy_opt, lossy_vec};
use tokio_postgres::Row;
use uuid::Uuid;

use crate::Catalog;
use crate::db::{p, query_one, query_one_ddl};
use crate::ddl::{ColumnDef, column_line, comment, grants, header, join_blocks, literal, owner};
use crate::mask::{is_secret_key, mask_options};
use crate::props::PropertySection;
use crate::version::VersionedSql;

static FDW: VersionedSql = VersionedSql(&[(10, include_str!("../sql/fdw.sql"))]);
static SERVER: VersionedSql = VersionedSql(&[(10, include_str!("../sql/foreign_server.sql"))]);
static USER_MAPPING: VersionedSql = VersionedSql(&[(10, include_str!("../sql/user_mapping.sql"))]);
static FOREIGN_TABLE: VersionedSql = VersionedSql(&[(10, include_str!("../sql/foreign_table.sql"))]);
static COLUMN_OPTIONS: VersionedSql = VersionedSql(&[(10, include_str!("../sql/foreign_table_column_options.sql"))]);
static CONSTRAINTS: VersionedSql = VersionedSql(&[(10, include_str!("../sql/table_ddl_constraints.sql"))]);
static COLUMN_ACL: VersionedSql = VersionedSql(&[(10, include_str!("../sql/table_ddl_column_acl.sql"))]);
static ACL_FDW: VersionedSql = VersionedSql(&[(10, include_str!("../sql/acl_foreign_data_wrapper.sql"))]);
static ACL_SERVER: VersionedSql = VersionedSql(&[(10, include_str!("../sql/acl_foreign_server.sql"))]);
static ACL_RELATION: VersionedSql = VersionedSql(&[(10, include_str!("../sql/acl_relation.sql"))]);

/// Maskelenmiş seçenekler: Inspector gösterimi (`k=v, …`) ve DDL `OPTIONS (k 'v', …)` gövdesi.
pub(crate) struct Options {
    items: Vec<String>,
    keys: Vec<String>,
    /// En az bir gizli değer maskelendi.
    pub masked: bool,
}

impl Options {
    /// `options`: ham `anahtar=değer` dizisi, `option_keys`: sunucuda `quote_ident` uygulanmış anahtarlar (aynı sıra).
    /// Ham dizi yalnız bu işlevin içinde yaşar.
    pub(crate) fn from_row(r: &Row) -> Self {
        let raw = lossy_vec(r, "options");
        let masked = raw.iter().any(|o| o.split_once('=').is_some_and(|(k, _)| is_secret_key(k)));
        Self { items: mask_options(&raw), keys: lossy_vec(r, "option_keys"), masked }
    }

    pub(crate) fn display(&self) -> Option<String> {
        (!self.items.is_empty()).then(|| self.items.join(", "))
    }

    /// `OPTIONS (…)`; seçenek yoksa `None`.
    pub(crate) fn clause(&self) -> Option<String> {
        if self.items.is_empty() {
            return None;
        }
        let pairs: Vec<String> = self
            .keys
            .iter()
            .zip(&self.items)
            .map(|(k, item)| format!("{k} {}", literal(item.split_once('=').map_or("", |(_, v)| v))))
            .collect();
        Some(format!("OPTIONS ({})", pairs.join(", ")))
    }
}

/// Gizli değer maskelendiyse DDL'in başına yazılan not.
fn masked_note(o: &Options) -> String {
    if o.masked { "-- Parola seçeneği gizlenmiştir.\n".to_string() } else { String::new() }
}

fn general(r: &Row) -> PropertySection {
    PropertySection::new("Genel")
        .row("Ad", lossy(r, "name"))
        .row("OID", r.get::<_, u32>("oid").to_string())
        .row("Sahip", lossy(r, "owner"))
        .row_opt("Yorum", lossy_opt(r, "comment"))
}

pub(crate) async fn fdw_properties(cat: &Catalog, server: Uuid, db: u32, oid: u32) -> Result<Vec<PropertySection>> {
    let r = query_one(&cat.mgr, server, Target::Database(db), &FDW, p![oid], "foreign data wrapper").await?;
    let opts = Options::from_row(&r);
    Ok(vec![
        general(&r),
        PropertySection::new("Tanım")
            .row_opt("Handler", lossy_opt(&r, "handler"))
            .row_opt("Validator", lossy_opt(&r, "validator"))
            .row_opt("Seçenekler", opts.display())
            .row_opt("Eklenti", lossy_opt(&r, "extension")),
    ])
}

pub(crate) async fn fdw_ddl(cat: &Catalog, server: Uuid, db: u32, oid: u32) -> Result<String> {
    let target = Target::Database(db);
    let (r, acl) = tokio::try_join!(
        query_one_ddl(&cat.mgr, server, target, &FDW, p![oid], "foreign data wrapper"),
        super::acl(cat, server, target, &ACL_FDW, oid),
    )?;
    let q = lossy(&r, "qname");
    let opts = Options::from_row(&r);
    let mut create = format!("CREATE FOREIGN DATA WRAPPER {q}");
    for (kw, col) in [("HANDLER", "handler"), ("VALIDATOR", "validator")] {
        if let Some(f) = lossy_opt(&r, col) {
            create.push_str(&format!("\n    {kw} {f}"));
        }
    }
    if let Some(o) = opts.clause() {
        create.push_str(&format!("\n    {o}"));
    }
    create.push_str(";\n");
    let comment_lit = lossy_opt(&r, "comment").map(|c| literal(&c));
    Ok(header("Foreign Data Wrapper", &q, &format!("DROP FOREIGN DATA WRAPPER IF EXISTS {q};"))
        + &join_blocks(vec![
            masked_note(&opts) + &create,
            owner("FOREIGN DATA WRAPPER", &q, &lossy(&r, "qowner")),
            comment("FOREIGN DATA WRAPPER", &q, comment_lit.as_deref()),
            grants("FOREIGN DATA WRAPPER", &q, &acl),
        ]))
}

pub(crate) async fn server_properties(cat: &Catalog, server: Uuid, db: u32, oid: u32) -> Result<Vec<PropertySection>> {
    let r = query_one(&cat.mgr, server, Target::Database(db), &SERVER, p![oid], "foreign server").await?;
    let opts = Options::from_row(&r);
    Ok(vec![
        general(&r),
        PropertySection::new("Tanım")
            .row("FDW", lossy(&r, "fdw"))
            .row_opt("Tür", lossy_opt(&r, "type"))
            .row_opt("Sürüm", lossy_opt(&r, "version"))
            .row_opt("Seçenekler", opts.display())
            .row_opt("Eklenti", lossy_opt(&r, "extension")),
    ])
}

pub(crate) async fn server_ddl(cat: &Catalog, server: Uuid, db: u32, oid: u32) -> Result<String> {
    let target = Target::Database(db);
    let (r, acl) = tokio::try_join!(
        query_one_ddl(&cat.mgr, server, target, &SERVER, p![oid], "foreign server"),
        super::acl(cat, server, target, &ACL_SERVER, oid),
    )?;
    let q = lossy(&r, "qname");
    let opts = Options::from_row(&r);
    let mut create = format!("CREATE SERVER {q}");
    if let Some(t) = lossy_opt(&r, "type") {
        create.push_str(&format!("\n    TYPE {}", literal(&t)));
    }
    if let Some(v) = lossy_opt(&r, "version") {
        create.push_str(&format!("\n    VERSION {}", literal(&v)));
    }
    create.push_str(&format!("\n    FOREIGN DATA WRAPPER {}", lossy(&r, "qfdw")));
    if let Some(o) = opts.clause() {
        create.push_str(&format!("\n    {o}"));
    }
    create.push_str(";\n");
    let comment_lit = lossy_opt(&r, "comment").map(|c| literal(&c));
    Ok(header("Foreign Server", &q, &format!("DROP SERVER IF EXISTS {q};"))
        + &join_blocks(vec![
            masked_note(&opts) + &create,
            owner("SERVER", &q, &lossy(&r, "qowner")),
            comment("SERVER", &q, comment_lit.as_deref()),
            grants("FOREIGN SERVER", &q, &acl),
        ]))
}

/// Yetkisiz kullanıcıya `pg_user_mappings.umoptions` NULL gelir.
const OPTIONS_NEED_PRIVILEGE: &str = "görüntülemek için yetki gerekir";

pub(crate) async fn user_mapping_properties(cat: &Catalog, server: Uuid, db: u32, oid: u32) -> Result<Vec<PropertySection>> {
    let r = query_one(&cat.mgr, server, Target::Database(db), &USER_MAPPING, p![oid], "user mapping").await?;
    let options = if r.get::<_, bool>("options_readable") {
        Options::from_row(&r).display().unwrap_or_else(|| "—".into())
    } else {
        OPTIONS_NEED_PRIVILEGE.into()
    };
    Ok(vec![
        PropertySection::new("Genel")
            .row("Rol", lossy(&r, "role"))
            .row("OID", r.get::<_, u32>("oid").to_string())
            .row("Sunucu", lossy(&r, "server")),
        PropertySection::new("Tanım").row("Seçenekler", options),
    ])
}

pub(crate) async fn user_mapping_ddl(cat: &Catalog, server: Uuid, db: u32, oid: u32) -> Result<String> {
    let r = query_one_ddl(&cat.mgr, server, Target::Database(db), &USER_MAPPING, p![oid], "user mapping").await?;
    let target = format!("FOR {} SERVER {}", lossy(&r, "qrole"), lossy(&r, "qserver"));
    let mut create = format!("CREATE USER MAPPING {target}");
    let mut note = String::new();
    if r.get::<_, bool>("options_readable") {
        let opts = Options::from_row(&r);
        note = masked_note(&opts);
        if let Some(o) = opts.clause() {
            create.push_str(&format!("\n    {o}"));
        }
        create.push_str(";\n");
    } else {
        create.push_str(";\n-- seçenekler okunamadı (yetki)\n");
    }
    Ok(header("User Mapping", &target, &format!("DROP USER MAPPING IF EXISTS {target};")) + &note + &create)
}

pub(crate) async fn foreign_table_properties(cat: &Catalog, server: Uuid, db: u32, oid: u32) -> Result<Vec<PropertySection>> {
    let r = query_one(&cat.mgr, server, Target::Database(db), &FOREIGN_TABLE, p![oid], "foreign table").await?;
    let opts = Options::from_row(&r);
    Ok(vec![
        PropertySection::new("Genel")
            .row("Ad", lossy(&r, "name"))
            .row("Şema", lossy(&r, "schema"))
            .row("OID", r.get::<_, u32>("oid").to_string())
            .row("Sahip", lossy(&r, "owner"))
            .row_opt("Yorum", lossy_opt(&r, "comment")),
        PropertySection::new("Tanım")
            .row("Sunucu", lossy(&r, "server"))
            .row_opt("Seçenekler", opts.display())
            .row_opt("Miras", lossy_opt(&r, "inherits"))
            .row("Kolon sayısı", r.get::<_, i32>("column_count").to_string()),
    ])
}

/// `ad tip [OPTIONS (…)] [COLLATE …] [NOT NULL] [DEFAULT …]`: foreign table kolon sözdiziminde OPTIONS tipten hemen sonra gelir.
fn foreign_column_line(c: &ColumnDef, opts: Option<&Options>) -> (String, bool) {
    let line = column_line(c);
    let head = format!("{} {}", c.name, c.type_name);
    let rest = line.strip_prefix(&head).unwrap_or_default();
    match opts.and_then(|o| o.clause().map(|clause| (clause, o.masked))) {
        Some((clause, masked)) => (format!("{head} {clause}{rest}"), masked),
        None => (line, false),
    }
}

async fn column_options(cat: &Catalog, server: Uuid, db: u32, table: u32) -> Result<Vec<(String, Options)>> {
    let rows = crate::db::query_ddl(&cat.mgr, server, Target::Database(db), &COLUMN_OPTIONS, p![table]).await?;
    Ok(rows.iter().map(|r| (lossy(r, "name"), Options::from_row(r))).collect())
}

pub(crate) async fn foreign_table_ddl(cat: &Catalog, server: Uuid, db: u32, oid: u32) -> Result<String> {
    let target = Target::Database(db);
    let q = |sql: &'static VersionedSql, params: crate::db::Params| crate::db::query_ddl(&cat.mgr, server, target, sql, params);
    let (r, col_rows, col_opts, constraints, acl, col_acl) = tokio::try_join!(
        query_one_ddl(&cat.mgr, server, target, &FOREIGN_TABLE, p![oid], "foreign table"),
        q(&super::table::COLUMNS, p![oid, None::<i16>]),
        column_options(cat, server, db, oid),
        q(&CONSTRAINTS, p![oid]),
        super::acl(cat, server, target, &ACL_RELATION, oid),
        q(&COLUMN_ACL, p![oid]),
    )?;
    let qname = lossy(&r, "qname");
    let opts = Options::from_row(&r);
    let cols: Vec<ColumnDef> = col_rows.iter().map(ColumnDef::from_row).collect();
    let mut masked = opts.masked;
    let mut lines: Vec<String> = cols
        .iter()
        .map(|c| {
            let (line, m) = foreign_column_line(c, col_opts.iter().find(|(n, _)| *n == c.name).map(|(_, o)| o));
            masked |= m;
            line
        })
        .collect();
    lines.extend(constraints.iter().map(|k| format!("CONSTRAINT {} {}", k.get::<_, String>("name"), k.get::<_, String>("def"))));
    let body = lines.iter().map(|l| format!("    {l}")).collect::<Vec<_>>().join(",\n");
    let mut create = format!("CREATE FOREIGN TABLE IF NOT EXISTS {qname}\n(\n{body}\n)");
    if let Some(parents) = lossy_opt(&r, "inherits") {
        create.push_str(&format!("\nINHERITS ({parents})"));
    }
    create.push_str(&format!("\n    SERVER {}", lossy(&r, "qserver")));
    if let Some(o) = opts.clause() {
        create.push_str(&format!("\n    {o}"));
    }
    create.push_str(";\n");
    let note = if masked { "-- Parola seçeneği gizlenmiştir.\n" } else { "" };
    let comment_lit = lossy_opt(&r, "comment").map(|c| literal(&c));
    let mut blocks = vec![
        format!("{note}{create}"),
        owner("FOREIGN TABLE IF EXISTS", &qname, &lossy(&r, "qowner")),
        comment("FOREIGN TABLE", &qname, comment_lit.as_deref()),
    ];
    for c in &cols {
        blocks.push(comment("COLUMN", &format!("{qname}.{}", c.name), c.comment.as_deref()));
    }
    for k in &constraints {
        let target = format!("{} ON {qname}", k.get::<_, String>("name"));
        blocks.push(comment("CONSTRAINT", &target, k.get::<_, Option<String>>("comment").as_deref()));
    }
    blocks.push(grants("TABLE", &qname, &acl));
    blocks.push(col_acl.iter().map(|r| format!("{}\n", r.get::<_, String>("stmt"))).collect());
    Ok(header("Foreign Table", &qname, &format!("DROP FOREIGN TABLE IF EXISTS {qname};")) + &join_blocks(blocks))
}

/// Foreign table kolonu: `ALTER FOREIGN TABLE … ADD COLUMN` (kolon seçenekleri maskeli).
pub(crate) async fn column_ddl(cat: &Catalog, server: Uuid, db: u32, table: u32, attnum: i16) -> Result<String> {
    let target = Target::Database(db);
    let (r, row, col_opts) = tokio::try_join!(
        query_one_ddl(&cat.mgr, server, target, &FOREIGN_TABLE, p![table], "foreign table"),
        query_one_ddl(&cat.mgr, server, target, &super::table::COLUMNS, p![table, Some(attnum)], "kolon"),
        column_options(cat, server, db, table),
    )?;
    let qname = lossy(&r, "qname");
    let c = ColumnDef::from_row(&row);
    let (line, masked) = foreign_column_line(&c, col_opts.iter().find(|(n, _)| *n == c.name).map(|(_, o)| o));
    let col = format!("{qname}.{}", c.name);
    let note = if masked { "-- Parola seçeneği gizlenmiştir.\n" } else { "" };
    let add = format!("{note}ALTER FOREIGN TABLE IF EXISTS {qname}\n    ADD COLUMN {line};\n");
    Ok(header("Column", &col, &format!("ALTER FOREIGN TABLE IF EXISTS {qname} DROP COLUMN IF EXISTS {};", c.name))
        + &join_blocks(vec![add, comment("COLUMN", &col, c.comment.as_deref())]))
}

