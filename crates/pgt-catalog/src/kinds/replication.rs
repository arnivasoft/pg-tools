//! Publication ve subscription özellikleri/DDL'i (Plan 38 Karar 2–4).
//! Subscription bağlantı dizgesi (`subconninfo`) yalnız okuma yetkisi varsa okunur ve satır okunur okunmaz
//! maskelenir (`crate::mask::mask_conninfo`); ham değer bu modülün dışına çıkmaz.

use std::sync::Arc;

use pgt_core::connection::Target;
use pgt_core::pgtext::{lossy, lossy_opt, lossy_vec};
use pgt_core::{Error, Result};
use tokio_postgres::Row;
use uuid::Uuid;

use crate::Catalog;
use crate::db::{p, query, query_ddl, query_one, query_one_ddl, query_text};
use crate::ddl::{comment, header, join_blocks, literal, owner};
use crate::mask::{MASK, mask_conninfo};
use crate::props::{PropertySection, yes_no};
use crate::version::VersionedSql;

static PUBLICATION: VersionedSql = VersionedSql(&[(10, include_str!("../sql/publication.sql"))]);
/// Sürüme bağlı publication sütunları: `pubtruncate` 11+, `pubviaroot` 13+, `pubgencols` 18+ (`char`: `n`/`s`).
static PUB_FLAGS: VersionedSql = VersionedSql(&[
    (10, "SELECT NULL::bool AS trunc, NULL::bool AS via_root, NULL::text AS gencols FROM pg_publication p WHERE p.oid = $1"),
    (11, "SELECT p.pubtruncate AS trunc, NULL::bool AS via_root, NULL::text AS gencols FROM pg_publication p WHERE p.oid = $1"),
    (13, "SELECT p.pubtruncate AS trunc, p.pubviaroot AS via_root, NULL::text AS gencols FROM pg_publication p WHERE p.oid = $1"),
    (18, "SELECT p.pubtruncate AS trunc, p.pubviaroot AS via_root, p.pubgencols::text AS gencols FROM pg_publication p WHERE p.oid = $1"),
]);
static PUB_TABLES: VersionedSql = VersionedSql(&[
    (10, include_str!("../sql/publication_tables.pg10.sql")),
    (15, include_str!("../sql/publication_tables.sql")),
]);
static PUB_SCHEMAS: VersionedSql = VersionedSql(&[
    (10, "SELECT NULL::text AS name, NULL::text AS qname WHERE false AND $1::oid IS NOT NULL"),
    (15, include_str!("../sql/publication_schemas.sql")),
]);

struct Publication {
    head: Row,
    truncate: Option<bool>,
    via_root: Option<bool>,
    gencols: Option<String>,
    tables: Vec<Row>,
    /// PG 15 öncesinde `None`.
    schemas: Option<Vec<Row>>,
}

async fn load_publication(cat: &Catalog, server: Uuid, db: u32, oid: u32, for_ddl: bool) -> Result<Publication> {
    let target = Target::Database(db);
    let one = |sql: &'static VersionedSql| async move {
        if for_ddl {
            query_one_ddl(&cat.mgr, server, target, sql, p![oid], "publication").await
        } else {
            query_one(&cat.mgr, server, target, sql, p![oid], "publication").await
        }
    };
    let many = |sql: &'static VersionedSql| async move {
        if for_ddl { query_ddl(&cat.mgr, server, target, sql, p![oid]).await } else { query(&cat.mgr, server, target, sql, p![oid]).await }
    };
    let (head, flags, tables, schemas) = tokio::try_join!(one(&PUBLICATION), one(&PUB_FLAGS), many(&PUB_TABLES), many(&PUB_SCHEMAS))?;
    let major = cat.mgr.info(server).ok_or(Error::NotConnected)?.version_num / 10_000;
    Ok(Publication {
        truncate: flags.get("trunc"),
        via_root: flags.get("via_root"),
        gencols: lossy_opt(&flags, "gencols").map(|g| gencols_name(&g)),
        head,
        tables,
        schemas: (major >= 15).then_some(schemas),
    })
}

/// `pubgencols` (`n`/`s`) → seçenek değeri.
fn gencols_name(code: &str) -> String {
    match code {
        "n" => "none".into(),
        "s" => "stored".into(),
        other => other.into(),
    }
}

impl Publication {
    /// `publish` seçeneği: `insert, update, delete[, truncate]`.
    fn publish(&self) -> String {
        let h = &self.head;
        [("insert", Some(h.get::<_, bool>("ins"))), ("update", Some(h.get("upd"))), ("delete", Some(h.get("del"))), ("truncate", self.truncate)]
            .into_iter()
            .filter_map(|(name, on)| on.unwrap_or(false).then_some(name))
            .collect::<Vec<_>>()
            .join(", ")
    }
}

pub(crate) async fn publication_properties(cat: &Catalog, server: Uuid, db: u32, oid: u32) -> Result<Vec<PropertySection>> {
    let p = load_publication(cat, server, db, oid, false).await?;
    let h = &p.head;
    let all = h.get::<_, bool>("all_tables");
    let publish = p.publish();
    let mut def = PropertySection::new("Tanım")
        .row("Tüm tablolar", yes_no(all))
        .row("Yayınlanan işlemler", if publish.is_empty() { "—".to_string() } else { publish });
    if let Some(v) = p.via_root {
        def = def.row("publish_via_partition_root", yes_no(v));
    }
    if let Some(g) = &p.gencols {
        def = def.row("publish_generated_columns", g.clone());
    }
    if !all {
        def = def.row("Tablo sayısı", h.get::<_, i32>("table_count").to_string());
        if let Some(schemas) = &p.schemas {
            let names: Vec<String> = schemas.iter().map(|r| lossy(r, "name")).collect();
            def = def.row("Şemalar", if names.is_empty() { "—".to_string() } else { names.join(", ") });
        }
    }
    Ok(vec![
        PropertySection::new("Genel")
            .row("Ad", lossy(h, "name"))
            .row("OID", h.get::<_, u32>("oid").to_string())
            .row("Sahip", lossy(h, "owner"))
            .row_opt("Yorum", lossy_opt(h, "comment")),
        def,
    ])
}

pub(crate) async fn publication_ddl(cat: &Catalog, server: Uuid, db: u32, oid: u32) -> Result<String> {
    let p = load_publication(cat, server, db, oid, true).await?;
    let h = &p.head;
    let q = lossy(h, "qname");
    let mut create = format!("CREATE PUBLICATION {q}");
    if h.get::<_, bool>("all_tables") {
        create.push_str("\n    FOR ALL TABLES");
    } else {
        let tables: Vec<String> = p
            .tables
            .iter()
            .map(|r| {
                let mut t = lossy(r, "qname");
                if let Some(cols) = lossy_opt(r, "columns") {
                    t.push_str(&format!(" ({cols})"));
                }
                if let Some(qual) = lossy_opt(r, "qual") {
                    t.push_str(&format!(" WHERE ({qual})"));
                }
                t
            })
            .collect();
        let schemas: Vec<String> = p.schemas.iter().flatten().map(|r| lossy(r, "qname")).collect();
        // PG 15 sözdizimi: `FOR TABLE a, b, TABLES IN SCHEMA s1, s2` (devam öğeleri önceki türü alır).
        let mut parts = Vec::new();
        if !tables.is_empty() {
            parts.push(format!("TABLE {}", tables.join(", ")));
        }
        if !schemas.is_empty() {
            parts.push(format!("TABLES IN SCHEMA {}", schemas.join(", ")));
        }
        if !parts.is_empty() {
            create.push_str(&format!("\n    FOR {}", parts.join(", ")));
        }
    }
    let mut with = vec![format!("publish = {}", literal(&p.publish()))];
    if let Some(v) = p.via_root {
        with.push(format!("publish_via_partition_root = {v}"));
    }
    if let Some(g) = &p.gencols {
        with.push(format!("publish_generated_columns = {}", literal(g)));
    }
    create.push_str(&format!("\n    WITH ({});\n", with.join(", ")));
    let comment_lit = lossy_opt(h, "comment").map(|c| literal(&c));
    Ok(header("Publication", &q, &format!("DROP PUBLICATION IF EXISTS {q};"))
        + &join_blocks(vec![create, owner("PUBLICATION", &q, &lossy(h, "qowner")), comment("PUBLICATION", &q, comment_lit.as_deref())]))
}

static SUB_COLUMNS: VersionedSql = VersionedSql(&[(10, include_str!("../sql/subscription_columns.sql"))]);

/// `(pg_subscription sütunu, SELECT ifadeleri)`: sorgu yalnız sunucuda var olan ve okunabilen sütunlardan, bu sabit
/// parçalarla kurulur (sürüm farkı ve sütun yetkisi aynı yoldan çözülür: `subbinary`/`substream` 14+,
/// `subtwophasestate`/`subdisableonerr` 15+, `suborigin`/`subpasswordrequired`/`subrunasowner` 16+, `subfailover` 17+).
const SUB_EXPRS: &[(&str, &str)] = &[
    ("subname", "s.subname::text AS name, quote_ident(s.subname) AS qname"),
    ("subowner", "pg_get_userbyid(s.subowner)::text AS owner, quote_ident(pg_get_userbyid(s.subowner)) AS qowner"),
    ("subenabled", "s.subenabled AS enabled"),
    ("subpublications", "ARRAY(SELECT u.x FROM unnest(s.subpublications) WITH ORDINALITY u(x, n) ORDER BY u.n) AS publications, \
                         ARRAY(SELECT quote_ident(u.x) FROM unnest(s.subpublications) WITH ORDINALITY u(x, n) ORDER BY u.n) AS qpublications"),
    ("subslotname", "s.subslotname::text AS slot_name"),
    ("subsynccommit", "s.subsynccommit::text AS sync_commit"),
    ("subbinary", "s.subbinary AS bin"),
    ("substream", "s.substream::text AS streaming"),
    ("subtwophasestate", "s.subtwophasestate::text AS two_phase"),
    ("subdisableonerr", "s.subdisableonerr AS disable_on_error"),
    ("suborigin", "s.suborigin::text AS origin"),
    ("subpasswordrequired", "s.subpasswordrequired AS password_required"),
    ("subrunasowner", "s.subrunasowner AS run_as_owner"),
    ("subfailover", "s.subfailover AS failover"),
    ("subconninfo", "s.subconninfo AS conninfo"),
];

/// Okunmadan gösterilemeyen sütunlar: biri okunamıyorsa abonelik bütünüyle "yetki gerekir"dir.
const SUB_REQUIRED: [&str; 5] = ["subname", "subowner", "subenabled", "subpublications", "subslotname"];

const NEED_PRIVILEGE: &str = "görüntülemek için yetki gerekir";

/// Aboneliği oid'iyle seçen süzgeç (`pg_subscription.oid` okunabiliyorsa).
const SUB_BY_OID: &str = "s.oid = $1";
/// Yedek süzgeç (`list_subscriptions_by_stats.sql` ile aynı eşleme).
const SUB_BY_STATS: &str = "s.subname = (SELECT x.subname FROM pg_catalog.pg_stat_subscription x WHERE x.subid = $1 GROUP BY x.subname) \
     AND (SELECT count(DISTINCT y.subid) FROM pg_catalog.pg_stat_subscription y WHERE y.subname = s.subname) = 1";

enum Connection {
    /// Maskelenmiş bağlantı dizgesi.
    Masked(String),
    /// `subconninfo` okunamıyor.
    NoPrivilege,
}

struct Subscription {
    row: Row,
    connection: Connection,
}

impl Subscription {
    fn opt_bool(&self, col: &str) -> Option<bool> {
        self.row.try_get::<_, Option<bool>>(col).ok().flatten()
    }

    fn opt_text(&self, col: &str) -> Option<String> {
        self.row.columns().iter().any(|c| c.name() == col).then(|| lossy_opt(&self.row, col)).flatten()
    }

    /// `substream`: PG 14–15 `bool` (`true`/`false`), PG 16+ `char` (`f`/`t`/`p`).
    fn streaming(&self) -> Option<&'static str> {
        self.opt_text("streaming").map(|s| match s.as_str() {
            "t" | "true" => "on",
            "p" => "parallel",
            _ => "off",
        })
    }

    /// `subtwophasestate`: `d` devre dışı, `p` beklemede, `e` etkin.
    fn two_phase(&self) -> Option<(bool, &'static str)> {
        self.opt_text("two_phase").map(|s| match s.as_str() {
            "d" => (false, "devre dışı"),
            "p" => (true, "beklemede"),
            _ => (true, "etkin"),
        })
    }
}

/// Aboneliği okur; okuma yetkisi yoksa (42501 ya da zorunlu sütun okunamıyor) `None`.
async fn load_subscription(cat: &Catalog, server: Uuid, db: u32, oid: u32, for_ddl: bool) -> Result<Option<Subscription>> {
    let target = Target::Database(db);
    let readable: Vec<String> = query(&cat.mgr, server, target, &SUB_COLUMNS, p![]).await?.iter().map(|r| lossy(r, "col")).collect();
    if SUB_REQUIRED.iter().any(|c| !readable.iter().any(|r| r == c)) {
        return Ok(None);
    }
    let exprs: Vec<&str> = SUB_EXPRS.iter().filter(|(col, _)| readable.iter().any(|r| r == col)).map(|(_, e)| *e).collect();
    // `oid` okunamıyorsa (PG 10–13 sütun yetkisi) abonelik, listedeki gibi `pg_stat_subscription`'daki adıyla bulunur
    // (ad veritabanı içinde benzersiz; başka veritabanında aynı ad varsa belirsizdir → bulunamaz).
    // Yorum veritabanına özgü `pg_description`'dadır (COMMENT ON SUBSCRIPTION paylaşılan katalog kullanmaz).
    let filter = if readable.iter().any(|r| r == "oid") { SUB_BY_OID } else { SUB_BY_STATS };
    let sql = format!(
        "SELECT $1::oid AS oid, obj_description($1::oid, 'pg_subscription') AS comment, {} FROM pg_catalog.pg_subscription s \
         WHERE s.subdbid = (SELECT d.oid FROM pg_catalog.pg_database d WHERE d.datname = current_database()) AND {filter}",
        exprs.join(", ")
    );
    let rows = match query_text(&cat.mgr, server, target, Arc::new(sql), p![oid]).await {
        Err(Error::Query { sqlstate: Some(code), .. }) if code == "42501" => return Ok(None),
        other => other?,
    };
    if for_ddl {
        pgt_core::pgtext::ensure_utf8(&rows)?;
    }
    let row = rows.into_iter().next().ok_or_else(|| Error::NotFound("subscription".into()))?;
    let connection = if row.columns().iter().any(|c| c.name() == "conninfo") {
        // Ham değer yalnız bu ifadede: hemen maskelenir.
        Connection::Masked(mask_conninfo(&lossy(&row, "conninfo")))
    } else {
        Connection::NoPrivilege
    };
    Ok(Some(Subscription { row, connection }))
}

fn no_privilege_sections() -> Vec<PropertySection> {
    vec![PropertySection::new("Genel").row("Abonelik", NEED_PRIVILEGE)]
}

pub(crate) async fn subscription_properties(cat: &Catalog, server: Uuid, db: u32, oid: u32) -> Result<Vec<PropertySection>> {
    let Some(s) = load_subscription(cat, server, db, oid, false).await? else {
        return Ok(no_privilege_sections());
    };
    let r = &s.row;
    let slot = lossy_opt(r, "slot_name").unwrap_or_else(|| "NONE".into());
    let connection = match &s.connection {
        Connection::Masked(c) => c.clone(),
        Connection::NoPrivilege => NEED_PRIVILEGE.into(),
    };
    let mut def = PropertySection::new("Tanım")
        .row("Etkin", yes_no(r.get("enabled")))
        .row("Yayınlar", lossy_vec(r, "publications").join(", "))
        .row("Slot adı", slot)
        .row_opt("synchronous_commit", s.opt_text("sync_commit"))
        .row("Bağlantı", connection);
    if let Some(v) = s.opt_bool("bin") {
        def = def.row("binary", yes_no(v));
    }
    if let Some(v) = s.streaming() {
        def = def.row("streaming", v);
    }
    if let Some((_, label)) = s.two_phase() {
        def = def.row("two_phase", label);
    }
    for (label, col) in [("disable_on_error", "disable_on_error"), ("password_required", "password_required"), ("run_as_owner", "run_as_owner"), ("failover", "failover")] {
        if let Some(v) = s.opt_bool(col) {
            def = def.row(label, yes_no(v));
        }
    }
    if let Some(o) = s.opt_text("origin") {
        def = def.row("origin", o);
    }
    Ok(vec![
        PropertySection::new("Genel")
            .row("Ad", lossy(r, "name"))
            .row("OID", r.get::<_, u32>("oid").to_string())
            .row("Sahip", lossy(r, "owner"))
            .row_opt("Yorum", lossy_opt(r, "comment")),
        def,
    ])
}

pub(crate) async fn subscription_ddl(cat: &Catalog, server: Uuid, db: u32, oid: u32) -> Result<String> {
    let Some(s) = load_subscription(cat, server, db, oid, true).await? else {
        return Ok(format!("-- Subscription: {NEED_PRIVILEGE}\n"));
    };
    let r = &s.row;
    let q = lossy(r, "qname");
    let (note, conn) = match &s.connection {
        Connection::Masked(c) => ("-- Bağlantı dizgesindeki parola gizlenmiştir.\n", literal(c)),
        Connection::NoPrivilege => ("-- Bağlantı dizgesini görüntülemek için yetki gerekir.\n", literal(MASK)),
    };
    let mut with = vec![
        format!("enabled = {}", r.get::<_, bool>("enabled")),
        format!("slot_name = {}", lossy_opt(r, "slot_name").map_or_else(|| "NONE".to_string(), |n| literal(&n))),
    ];
    if let Some(v) = s.opt_text("sync_commit") {
        with.push(format!("synchronous_commit = {}", literal(&v)));
    }
    if let Some(v) = s.opt_bool("bin") {
        with.push(format!("binary = {v}"));
    }
    if let Some(v) = s.streaming() {
        with.push(format!("streaming = {}", match v { "on" => "true", "off" => "false", other => other }));
    }
    if let Some((on, _)) = s.two_phase() {
        with.push(format!("two_phase = {on}"));
    }
    if let Some(v) = s.opt_bool("disable_on_error") {
        with.push(format!("disable_on_error = {v}"));
    }
    if let Some(o) = s.opt_text("origin") {
        with.push(format!("origin = {}", literal(&o)));
    }
    for col in ["password_required", "run_as_owner", "failover"] {
        if let Some(v) = s.opt_bool(col) {
            with.push(format!("{col} = {v}"));
        }
    }
    let create = format!(
        "{note}CREATE SUBSCRIPTION {q}\n    CONNECTION {conn}\n    PUBLICATION {}\n    WITH ({});\n",
        lossy_vec(r, "qpublications").join(", "),
        with.join(", ")
    );
    let comment_lit = lossy_opt(r, "comment").map(|c| literal(&c));
    Ok(header("Subscription", &q, &format!("DROP SUBSCRIPTION IF EXISTS {q};"))
        + &join_blocks(vec![create, owner("SUBSCRIPTION", &q, &lossy(r, "qowner")), comment("SUBSCRIPTION", &q, comment_lit.as_deref())]))
}
