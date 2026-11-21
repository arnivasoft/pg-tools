//! Nesne arama (spec 2026-11-17-object-search-design.md). SQL yalnız bu dosyadaki sabit parçalardan kurulur;
//! arama metni her zaman parametredir. Süzgeçler listeleme SQL'leriyle (`sql/list_*.sql`) birebir aynıdır:
//! sapma, sonucun ağaçta bulunamaması demektir.
use std::collections::HashMap;
use std::sync::Arc;

use pgt_core::connection::Target;
use pgt_core::pgtext::{lossy, lossy_opt};
use pgt_core::{Error, Result};
use serde::{Deserialize, Serialize};
use ts_rs::TS;
use uuid::Uuid;

use crate::Catalog;
use crate::db::{p, query, query_text};
use crate::node::{Coll, NodeId, NodeKind, Seg};
use crate::version::{PgVersion, VersionedSql};

pub const MAX_QUERY_CHARS: usize = 200;
pub const MAX_HITS: usize = 500;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum SearchKind {
    Schema, Table, Partition, View, Matview, Function, Procedure, TriggerFunction, Sequence, Type, Domain,
    Column, Index, Constraint, Trigger, Policy, Extension, ForeignTable, ForeignServer, Publication, Subscription,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct SearchHit {
    pub id: String,
    pub path: Vec<String>,
    pub kind: NodeKind,
    pub search_kind: SearchKind,
    pub name: String,
    pub schema: Option<String>,
    pub owner: Option<String>,
    pub system: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct SearchResult {
    pub hits: Vec<SearchHit>,
    pub truncated: bool,
}

const SYS: &str = "(n.nspname IN ('pg_catalog', 'information_schema') OR n.nspname LIKE 'pg\\_toast%' OR n.nspname LIKE 'pg\\_temp\\_%')";
const EXT: &str = "EXISTS (SELECT 1 FROM pg_depend i WHERE i.classid = 'pg_proc'::regclass AND i.objid = p.oid AND i.deptype = 'i')";

/// `(tür, en düşük ana sürüm, en yüksek ana sürüm, parça)`. `{SYS}` ve `{INT}` yer tutucuları `build_sql`'de
/// sabitlerle değiştirilir. Süzgeçler `sql/list_*.sql` ile aynıdır.
const FRAGMENTS: &[(SearchKind, u32, u32, &str)] = &[
    (SearchKind::Schema, 10, 99, "SELECT 'schema' AS kind, n.oid, 0::int2 AS attnum, n.nspname::text AS name, n.nspname::text AS match_name, n.oid AS schema_oid, n.nspname::text AS schema_name, 0::oid AS rel_oid, ''::text AS rel_kind, NULL::text AS owner, {SYS} AS is_system FROM pg_namespace n WHERE n.nspname ILIKE $1 ESCAPE '\\'"),
    (SearchKind::Table, 10, 99, "SELECT 'table', c.oid, 0::int2, c.relname::text, c.relname::text, n.oid, n.nspname::text, c.oid, c.relkind::text, NULL::text, {SYS} FROM pg_class c JOIN pg_namespace n ON n.oid = c.relnamespace WHERE c.relkind IN ('r', 'p') AND NOT c.relispartition AND c.relname ILIKE $1 ESCAPE '\\'"),
    // Not: bölümün is_system'ı bölümün kendi şemasından gelir (kök tablonunkinden değil); bilinen sınırlama.
    (SearchKind::Partition, 10, 99, "SELECT 'partition', c.oid, 0::int2, c.relname::text, c.relname::text, n.oid, n.nspname::text, c.oid, c.relkind::text, (SELECT pc.relname::text FROM pg_inherits i JOIN pg_class pc ON pc.oid = i.inhparent WHERE i.inhrelid = c.oid LIMIT 1), {SYS} FROM pg_class c JOIN pg_namespace n ON n.oid = c.relnamespace WHERE c.relispartition AND c.relname ILIKE $1 ESCAPE '\\'"),
    (SearchKind::View, 10, 99, "SELECT 'view', c.oid, 0::int2, c.relname::text, c.relname::text, n.oid, n.nspname::text, c.oid, 'v', NULL::text, {SYS} FROM pg_class c JOIN pg_namespace n ON n.oid = c.relnamespace WHERE c.relkind = 'v' AND c.relname ILIKE $1 ESCAPE '\\'"),
    (SearchKind::Matview, 10, 99, "SELECT 'matview', c.oid, 0::int2, c.relname::text, c.relname::text, n.oid, n.nspname::text, c.oid, 'm', NULL::text, {SYS} FROM pg_class c JOIN pg_namespace n ON n.oid = c.relnamespace WHERE c.relkind = 'm' AND c.relname ILIKE $1 ESCAPE '\\'"),
    (SearchKind::Function, 10, 10, "SELECT 'function', p.oid, 0::int2, p.proname::text || '(' || pg_get_function_identity_arguments(p.oid) || ')', p.proname::text, n.oid, n.nspname::text, 0::oid, '', NULL::text, {SYS} FROM pg_proc p JOIN pg_namespace n ON n.oid = p.pronamespace WHERE NOT p.proisagg AND p.prorettype <> 'trigger'::regtype AND NOT {INT} AND p.proname ILIKE $1 ESCAPE '\\'"),
    (SearchKind::Function, 11, 99, "SELECT 'function', p.oid, 0::int2, p.proname::text || '(' || pg_get_function_identity_arguments(p.oid) || ')', p.proname::text, n.oid, n.nspname::text, 0::oid, '', NULL::text, {SYS} FROM pg_proc p JOIN pg_namespace n ON n.oid = p.pronamespace WHERE p.prokind IN ('f', 'w') AND p.prorettype <> 'trigger'::regtype AND NOT {INT} AND p.proname ILIKE $1 ESCAPE '\\'"),
    (SearchKind::Procedure, 11, 99, "SELECT 'procedure', p.oid, 0::int2, p.proname::text || '(' || regexp_replace(pg_get_function_identity_arguments(p.oid), '(^|, )IN ', '\\1', 'g') || ')', p.proname::text, n.oid, n.nspname::text, 0::oid, '', NULL::text, {SYS} FROM pg_proc p JOIN pg_namespace n ON n.oid = p.pronamespace WHERE p.prokind = 'p' AND NOT {INT} AND p.proname ILIKE $1 ESCAPE '\\'"),
    (SearchKind::TriggerFunction, 10, 10, "SELECT 'triggerFunction', p.oid, 0::int2, p.proname::text || '(' || pg_get_function_identity_arguments(p.oid) || ')', p.proname::text, n.oid, n.nspname::text, 0::oid, '', NULL::text, {SYS} FROM pg_proc p JOIN pg_namespace n ON n.oid = p.pronamespace WHERE NOT p.proisagg AND p.prorettype = 'trigger'::regtype AND NOT {INT} AND p.proname ILIKE $1 ESCAPE '\\'"),
    (SearchKind::TriggerFunction, 11, 99, "SELECT 'triggerFunction', p.oid, 0::int2, p.proname::text || '(' || pg_get_function_identity_arguments(p.oid) || ')', p.proname::text, n.oid, n.nspname::text, 0::oid, '', NULL::text, {SYS} FROM pg_proc p JOIN pg_namespace n ON n.oid = p.pronamespace WHERE p.prokind IN ('f', 'w') AND p.prorettype = 'trigger'::regtype AND NOT {INT} AND p.proname ILIKE $1 ESCAPE '\\'"),
    (SearchKind::Sequence, 10, 99, "SELECT 'sequence', c.oid, 0::int2, c.relname::text, c.relname::text, n.oid, n.nspname::text, 0::oid, '', NULL::text, {SYS} FROM pg_class c JOIN pg_sequence s ON s.seqrelid = c.oid JOIN pg_namespace n ON n.oid = c.relnamespace WHERE c.relkind = 'S' AND NOT EXISTS (SELECT 1 FROM pg_depend d WHERE d.classid = 'pg_class'::regclass AND d.objid = c.oid AND d.deptype = 'i') AND c.relname ILIKE $1 ESCAPE '\\'"),
    (SearchKind::Type, 10, 99, "SELECT 'type', t.oid, 0::int2, t.typname::text, t.typname::text, n.oid, n.nspname::text, 0::oid, '', NULL::text, {SYS} FROM pg_type t JOIN pg_namespace n ON n.oid = t.typnamespace WHERE t.typtype IN ('e', 'c', 'r', 'b') AND (t.typtype <> 'c' OR EXISTS (SELECT 1 FROM pg_class c WHERE c.oid = t.typrelid AND c.relkind = 'c')) AND NOT EXISTS (SELECT 1 FROM pg_type el WHERE el.oid = t.typelem AND el.typarray = t.oid) AND t.typname ILIKE $1 ESCAPE '\\'"),
    (SearchKind::Domain, 10, 99, "SELECT 'domain', t.oid, 0::int2, t.typname::text, t.typname::text, n.oid, n.nspname::text, 0::oid, '', NULL::text, {SYS} FROM pg_type t JOIN pg_namespace n ON n.oid = t.typnamespace WHERE t.typtype = 'd' AND t.typname ILIKE $1 ESCAPE '\\'"),
    (SearchKind::Column, 10, 99, "SELECT 'column', 0::oid, a.attnum, a.attname::text, a.attname::text, n.oid, n.nspname::text, c.oid, c.relkind::text, c.relname::text, {SYS} FROM pg_attribute a JOIN pg_class c ON c.oid = a.attrelid JOIN pg_namespace n ON n.oid = c.relnamespace WHERE (c.relkind IN ('r', 'p', 'v', 'm') OR (c.relkind = 'f' AND c.relispartition)) AND a.attnum > 0 AND NOT a.attisdropped AND a.attname ILIKE $1 ESCAPE '\\'"),
    (SearchKind::Index, 10, 99, "SELECT 'index', x.indexrelid, 0::int2, ic.relname::text, ic.relname::text, n.oid, n.nspname::text, c.oid, c.relkind::text, c.relname::text, {SYS} FROM pg_index x JOIN pg_class ic ON ic.oid = x.indexrelid JOIN pg_am am ON am.oid = ic.relam JOIN pg_class c ON c.oid = x.indrelid JOIN pg_namespace n ON n.oid = c.relnamespace WHERE c.relkind IN ('r', 'p', 'm') AND ic.relname ILIKE $1 ESCAPE '\\'"),
    (SearchKind::Constraint, 10, 99, "SELECT 'constraint', k.oid, 0::int2, k.conname::text, k.conname::text, n.oid, n.nspname::text, c.oid, c.relkind::text, c.relname::text, {SYS} FROM pg_constraint k JOIN pg_class c ON c.oid = k.conrelid JOIN pg_namespace n ON n.oid = c.relnamespace WHERE c.relkind IN ('r', 'p') AND k.contype IN ('p', 'u', 'c', 'x', 'f') AND k.conname ILIKE $1 ESCAPE '\\'"),
    (SearchKind::Trigger, 10, 99, "SELECT 'trigger', g.oid, 0::int2, g.tgname::text, g.tgname::text, n.oid, n.nspname::text, c.oid, c.relkind::text, c.relname::text, {SYS} FROM pg_trigger g JOIN pg_class c ON c.oid = g.tgrelid JOIN pg_namespace n ON n.oid = c.relnamespace WHERE c.relkind IN ('r', 'p') AND NOT g.tgisinternal AND g.tgname ILIKE $1 ESCAPE '\\'"),
    (SearchKind::Policy, 10, 99, "SELECT 'policy', o.oid, 0::int2, o.polname::text, o.polname::text, n.oid, n.nspname::text, c.oid, c.relkind::text, c.relname::text, {SYS} FROM pg_policy o JOIN pg_class c ON c.oid = o.polrelid JOIN pg_namespace n ON n.oid = c.relnamespace WHERE c.relkind IN ('r', 'p') AND o.polname ILIKE $1 ESCAPE '\\'"),
    (SearchKind::Extension, 10, 99, "SELECT 'extension', e.oid, 0::int2, e.extname::text, e.extname::text, 0::oid, NULL::text, 0::oid, '', NULL::text, false FROM pg_extension e WHERE e.extname ILIKE $1 ESCAPE '\\'"),
    // Foreign table: `list_foreign_tables.sql` (yabancı bölümler bölüm parçasında; `rel_oid` 0 → zincir yüklenmez).
    (SearchKind::ForeignTable, 10, 99, "SELECT 'foreignTable', c.oid, 0::int2, c.relname::text, c.relname::text, n.oid, n.nspname::text, 0::oid, '', NULL::text, {SYS} FROM pg_class c JOIN pg_foreign_table ft ON ft.ftrelid = c.oid JOIN pg_foreign_server s ON s.oid = ft.ftserver JOIN pg_namespace n ON n.oid = c.relnamespace WHERE c.relkind = 'f' AND NOT c.relispartition AND c.relname ILIKE $1 ESCAPE '\\'"),
    // Foreign server: `list_foreign_servers.sql`; düğüm FDW'nin altında: FDW oid'i `rel_oid`'de, FDW adı `owner`'da.
    // Yalnız ad okunur (`srvoptions` okunmaz).
    (SearchKind::ForeignServer, 10, 99, "SELECT 'foreignServer', s.oid, 0::int2, s.srvname::text, s.srvname::text, 0::oid, NULL::text, s.srvfdw, '', w.fdwname::text, false FROM pg_foreign_server s JOIN pg_foreign_data_wrapper w ON w.oid = s.srvfdw WHERE s.srvname ILIKE $1 ESCAPE '\\'"),
    (SearchKind::Publication, 10, 99, "SELECT 'publication', p.oid, 0::int2, p.pubname::text, p.pubname::text, 0::oid, NULL::text, 0::oid, '', NULL::text, false FROM pg_publication p WHERE p.pubname ILIKE $1 ESCAPE '\\'"),
    // Subscription: `list_subscriptions.sql`; `subconninfo` okunmaz. Oid okunamıyorsa `SUB_BY_STATS` (bkz. `SubSource`).
    (SearchKind::Subscription, 10, 99, "SELECT 'subscription', s.oid, 0::int2, s.subname::text, s.subname::text, 0::oid, NULL::text, 0::oid, '', NULL::text, false FROM pg_subscription s WHERE s.subdbid = (SELECT d.oid FROM pg_database d WHERE d.datname = current_database()) AND s.subname ILIKE $1 ESCAPE '\\'"),
];

/// Subscription parçasının yedeği: `list_subscriptions_by_stats.sql` ile aynı eşleme (oid `pg_stat_subscription`'dan
/// ada göre; ad küme genelinde belirsizse abonelik listede olmadığı gibi sonuçta da olmaz). `subconninfo` okunmaz.
const SUB_BY_STATS: &str = "SELECT 'subscription', st.subid, 0::int2, s.subname::text, s.subname::text, 0::oid, NULL::text, 0::oid, '', NULL::text, false FROM pg_subscription s JOIN (SELECT x.subname, min(x.subid) AS subid FROM pg_stat_subscription x GROUP BY x.subname HAVING count(DISTINCT x.subid) = 1) st ON st.subname = s.subname WHERE s.subdbid = (SELECT d.oid FROM pg_database d WHERE d.datname = current_database()) AND s.subname ILIKE $1 ESCAPE '\\'";

/// Subscription oid'inin kaynağı. `pg_subscription.oid` PG 10–13'te süper kullanıcı olmayana açık değildir (42501):
/// liste gibi önce oid, sonra `pg_stat_subscription` ad eşlemesi denenir; o da reddedilirse tür atlanır.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SubSource {
    Oid,
    Stats,
    Skip,
}

pub(crate) fn escape_like(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for ch in s.chars() {
        if matches!(ch, '\\' | '%' | '_') {
            out.push('\\');
        }
        out.push(ch);
    }
    out
}

/// Kırpılmış metin; boşsa `None`; `MAX_QUERY_CHARS` üstü `Error::Invalid`.
pub(crate) fn normalize(query: &str) -> Result<Option<String>> {
    let q = query.trim();
    if q.chars().count() > MAX_QUERY_CHARS {
        return Err(Error::Invalid(format!("arama en fazla {MAX_QUERY_CHARS} karakter olabilir")));
    }
    Ok((!q.is_empty()).then(|| q.to_string()))
}

fn selected(kinds: &[SearchKind], major: u32, sub: SubSource) -> Vec<String> {
    FRAGMENTS
        .iter()
        .filter(|(k, min, max, _)| (kinds.is_empty() || kinds.contains(k)) && major >= *min && major <= *max)
        .filter_map(|(k, _, _, sql)| match (k, sub) {
            (SearchKind::Subscription, SubSource::Skip) => None,
            (SearchKind::Subscription, SubSource::Stats) => Some(SUB_BY_STATS),
            _ => Some(*sql),
        })
        .map(|sql| sql.replace("{SYS}", SYS).replace("{INT}", EXT))
        .collect()
}

/// Seçimde subscription parçası var mı (yetki reddinde yedeğe geçmeye değer mi).
fn has_subscription(kinds: &[SearchKind]) -> bool {
    kinds.is_empty() || kinds.contains(&SearchKind::Subscription)
}

/// Seçilen türlerin (boş = hepsi) sürüme uyan parçalarından tek sorgu. Sıralama: tam eşleşme, önek, ad, şema, tür.
#[cfg(test)]
pub(crate) fn build_sql(kinds: &[SearchKind], major: u32) -> String {
    assemble(&selected(kinds, major, SubSource::Oid))
}

/// `$2` ham (kırpılmış) metin, `$3` kaçırılmış ham metin + `%`; küçük harfe çevirme sunucuda (`lower`) yapılır.
fn assemble(parts: &[String]) -> String {
    format!(
        concat!(
        "SELECT * FROM ({}) h(kind, oid, attnum, name, match_name, schema_oid, schema_name, rel_oid, rel_kind, owner, is_system) WHERE ($4 OR NOT h.is_system) ",
        "ORDER BY CASE WHEN lower(h.match_name) = lower($2) THEN 0 WHEN lower(h.match_name) LIKE lower($3) ESCAPE '\\' THEN 1 ELSE 2 END, ",
        "h.name COLLATE \"C\", h.schema_name COLLATE \"C\", h.kind LIMIT {}"),
        parts.join(" UNION ALL "),
        MAX_HITS + 1
    )
}

#[derive(Debug, Clone)]
pub(crate) struct RawHit {
    pub kind: SearchKind,
    pub oid: u32,
    pub attnum: i16,
    pub name: String,
    pub schema_oid: u32,
    pub schema_name: Option<String>,
    pub rel_oid: u32,
    pub rel_kind: String,
    pub owner: Option<String>,
    pub system: bool,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct Chain {
    pub root_schema: u32,
    pub parents: Vec<u32>,
}

/// İlişkinin (tablo/bölüm/view/MV) düğümü ve o düğüme kadar olan zincir (ilişkinin kendisi dahil).
fn rel_node(schema: &NodeId, path: &mut Vec<NodeId>, rel_oid: u32, rel_kind: &str, chain: Option<&Chain>) -> NodeId {
    let (coll, seg) = match rel_kind {
        "v" => (Coll::Views, Seg::View(rel_oid)),
        "m" => (Coll::MatViews, Seg::MatView(rel_oid)),
        _ => (Coll::Tables, Seg::Table(rel_oid)),
    };
    path.push(schema.child(Seg::Coll(coll)));
    let mut cur = schema.clone();
    if let Some(ch) = chain {
        for parent in &ch.parents {
            cur = cur.child(Seg::Table(*parent));
            path.push(cur.clone());
            path.push(cur.child(Seg::Coll(Coll::Partitions)));
        }
    }
    let node = cur.child(seg);
    path.push(node.clone());
    node
}

pub(crate) fn to_hit(server: Uuid, db: u32, h: &RawHit, chains: &HashMap<u32, Chain>) -> SearchHit {
    let srv = NodeId::server(server);
    let db_id = srv.child(Seg::Db(db));
    let mut path = vec![srv.child(Seg::Coll(Coll::Databases)), db_id.clone()];
    let chain = chains.get(&h.rel_oid);
    // Bölümler kök tablonun şemasından geçer (bölümün kendi şeması farklı olabilir).
    let schema_oid = chain.map(|c| c.root_schema).unwrap_or(h.schema_oid);
    let schema_id = db_id.child(Seg::Schema(schema_oid));
    let in_schema = |path: &mut Vec<NodeId>| {
        path.push(db_id.child(Seg::Coll(Coll::Schemas)));
        path.push(schema_id.clone());
    };
    let (id, kind) = match h.kind {
        SearchKind::Schema => {
            path.push(db_id.child(Seg::Coll(Coll::Schemas)));
            (db_id.child(Seg::Schema(h.oid)), NodeKind::Schema)
        }
        SearchKind::Extension => {
            path.push(db_id.child(Seg::Coll(Coll::Extensions)));
            (db_id.child(Seg::Extension(h.oid)), NodeKind::Extension)
        }
        SearchKind::Publication => {
            path.push(db_id.child(Seg::Coll(Coll::Publications)));
            (db_id.child(Seg::Publication(h.oid)), NodeKind::Publication)
        }
        SearchKind::Subscription => {
            path.push(db_id.child(Seg::Coll(Coll::Subscriptions)));
            (db_id.child(Seg::Subscription(h.oid)), NodeKind::Subscription)
        }
        SearchKind::ForeignServer => {
            // Kimlik koleksiyon parçasını atlar (motor kuralı): `db:D/fdw:F/fsrv:S`.
            let fdw = db_id.child(Seg::ForeignDataWrapper(h.rel_oid));
            path.push(db_id.child(Seg::Coll(Coll::ForeignDataWrappers)));
            path.push(fdw.clone());
            path.push(fdw.child(Seg::Coll(Coll::ForeignServers)));
            (fdw.child(Seg::ForeignServer(h.oid)), NodeKind::ForeignServer)
        }
        SearchKind::Table | SearchKind::Partition | SearchKind::View | SearchKind::Matview => {
            in_schema(&mut path);
            let node = rel_node(&schema_id, &mut path, h.rel_oid, &h.rel_kind, chain);
            path.pop();
            let kind = match h.kind {
                SearchKind::View => NodeKind::View,
                SearchKind::Matview => NodeKind::MaterializedView,
                _ => NodeKind::Table,
            };
            (node, kind)
        }
        SearchKind::Column | SearchKind::Index | SearchKind::Constraint | SearchKind::Trigger | SearchKind::Policy => {
            in_schema(&mut path);
            let rel = rel_node(&schema_id, &mut path, h.rel_oid, &h.rel_kind, chain);
            let (coll, seg, kind) = match h.kind {
                SearchKind::Column => (Coll::Columns, Seg::Column(h.attnum), NodeKind::Column),
                SearchKind::Index => (Coll::Indexes, Seg::Index(h.oid), NodeKind::Index),
                SearchKind::Constraint => (Coll::Constraints, Seg::Constraint(h.oid), NodeKind::Constraint),
                SearchKind::Trigger => (Coll::Triggers, Seg::Trigger(h.oid), NodeKind::Trigger),
                _ => (Coll::Policies, Seg::Policy(h.oid), NodeKind::Policy),
            };
            path.push(rel.child(Seg::Coll(coll)));
            (rel.child(seg), kind)
        }
        k => {
            in_schema(&mut path);
            let (coll, seg, kind) = match k {
                SearchKind::Function => (Coll::Functions, Seg::Function(h.oid), NodeKind::Function),
                SearchKind::Procedure => (Coll::Procedures, Seg::Procedure(h.oid), NodeKind::Procedure),
                SearchKind::TriggerFunction => (Coll::TriggerFunctions, Seg::TriggerFunction(h.oid), NodeKind::TriggerFunction),
                SearchKind::Sequence => (Coll::Sequences, Seg::Sequence(h.oid), NodeKind::Sequence),
                SearchKind::Type => (Coll::Types, Seg::Type(h.oid), NodeKind::Type),
                SearchKind::ForeignTable => (Coll::ForeignTables, Seg::ForeignTable(h.oid), NodeKind::ForeignTable),
                _ => (Coll::Domains, Seg::Domain(h.oid), NodeKind::Domain),
            };
            path.push(schema_id.child(Seg::Coll(coll)));
            (schema_id.child(seg), kind)
        }
    };
    SearchHit {
        id: id.to_string(),
        path: path.iter().map(ToString::to_string).collect(),
        kind,
        search_kind: h.kind,
        name: h.name.clone(),
        schema: h.schema_name.clone(),
        owner: h.owner.clone(),
        system: h.system,
    }
}

impl RawHit {
    fn from_row(r: &tokio_postgres::Row) -> Result<Self> {
        let kind = match lossy(r, "kind").as_str() {
            "schema" => SearchKind::Schema,
            "table" => SearchKind::Table,
            "partition" => SearchKind::Partition,
            "view" => SearchKind::View,
            "matview" => SearchKind::Matview,
            "function" => SearchKind::Function,
            "procedure" => SearchKind::Procedure,
            "triggerFunction" => SearchKind::TriggerFunction,
            "sequence" => SearchKind::Sequence,
            "type" => SearchKind::Type,
            "domain" => SearchKind::Domain,
            "column" => SearchKind::Column,
            "index" => SearchKind::Index,
            "constraint" => SearchKind::Constraint,
            "trigger" => SearchKind::Trigger,
            "policy" => SearchKind::Policy,
            "extension" => SearchKind::Extension,
            "foreignTable" => SearchKind::ForeignTable,
            "foreignServer" => SearchKind::ForeignServer,
            "publication" => SearchKind::Publication,
            "subscription" => SearchKind::Subscription,
            other => return Err(Error::Invalid(format!("bilinmeyen arama türü: {other}"))),
        };
        Ok(Self {
            kind,
            oid: r.get::<_, u32>("oid"),
            attnum: r.get::<_, i16>("attnum"),
            name: lossy(r, "name"),
            schema_oid: r.get::<_, u32>("schema_oid"),
            schema_name: lossy_opt(r, "schema_name"),
            rel_oid: r.get::<_, u32>("rel_oid"),
            rel_kind: lossy(r, "rel_kind"),
            owner: lossy_opt(r, "owner"),
            system: r.get::<_, bool>("is_system"),
        })
    }
}

static CHAINS: VersionedSql = VersionedSql(&[(10, "WITH RECURSIVE up(child, parent, depth) AS (
  SELECT i.inhrelid, i.inhparent, 1 FROM pg_inherits i JOIN pg_class c ON c.oid = i.inhrelid
  WHERE i.inhrelid = ANY($1::oid[]) AND c.relispartition
  UNION ALL
  SELECT u.child, i.inhparent, u.depth + 1 FROM up u JOIN pg_inherits i ON i.inhrelid = u.parent
  JOIN pg_class c ON c.oid = i.inhrelid WHERE c.relispartition
)
SELECT u.child, array_agg(u.parent ORDER BY u.depth DESC) AS chain,
       (SELECT r.relnamespace FROM pg_class r WHERE r.oid = (array_agg(u.parent ORDER BY u.depth DESC))[1]) AS root_schema
FROM up u GROUP BY u.child")]);

async fn load_chains(cat: &Catalog, server: Uuid, db: u32, rels: Vec<u32>) -> Result<HashMap<u32, Chain>> {
    let rows = query(&cat.mgr, server, Target::Database(db), &CHAINS, p![rels]).await?;
    Ok(rows
        .iter()
        .map(|r| (r.get::<_, u32>("child"), Chain { root_schema: r.get::<_, u32>("root_schema"), parents: r.get::<_, Vec<u32>>("chain") }))
        .collect())
}

/// En çok `MAX_HITS` satır tutar; fazlası varsa `true` döner (sorgu `MAX_HITS + 1` satır ister).
pub(crate) fn cap<T>(mut rows: Vec<T>) -> (Vec<T>, bool) {
    let truncated = rows.len() > MAX_HITS;
    rows.truncate(MAX_HITS);
    (rows, truncated)
}

impl Catalog {
    pub async fn search(&self, server: Uuid, db: u32, query: &str, kinds: &[SearchKind]) -> Result<SearchResult> {
        let Some(q) = normalize(query)? else { return Ok(SearchResult { hits: Vec::new(), truncated: false }) };
        if !self.mgr.is_connected(server) {
            return Err(Error::NotConnected);
        }
        let major = PgVersion(self.mgr.info(server).ok_or(Error::NotConnected)?.version_num).major();
        let started = std::time::Instant::now();
        let denied = |e: &Error| matches!(e, Error::Query { sqlstate: Some(code), .. } if code == "42501");
        let mut rows = Vec::new();
        for sub in [SubSource::Oid, SubSource::Stats, SubSource::Skip] {
            let parts = selected(kinds, major, sub);
            if parts.is_empty() {
                return Ok(SearchResult { hits: Vec::new(), truncated: false });
            }
            let sql = Arc::new(assemble(&parts));
            let params = p![format!("%{}%", escape_like(&q)), q.clone(), format!("{}%", escape_like(&q)), self.show_system()];
            match query_text(&self.mgr, server, Target::Database(db), sql, params).await {
                // Yalnız subscription parçası yetki reddi verir (öbür kataloglar herkese açık): sıradaki kaynağa geç.
                Err(e) if denied(&e) && sub != SubSource::Skip && has_subscription(kinds) => {
                    tracing::debug!(?sub, "nesne araması: abonelik oid'i okunamıyor, yedek deneniyor");
                }
                r => {
                    rows = r?;
                    break;
                }
            }
        }
        let (rows, truncated) = cap(rows);
        let raw: Vec<RawHit> = rows.iter().map(RawHit::from_row).collect::<Result<_>>()?;
        let parts: Vec<u32> = raw.iter().filter(|h| h.rel_oid != 0 && (h.kind == SearchKind::Partition || matches!(h.rel_kind.as_str(), "r" | "p" | "f"))).map(|h| h.rel_oid).collect();
        let chains = if parts.is_empty() { HashMap::new() } else { load_chains(self, server, db, parts).await? };
        let hits = raw.iter().map(|h| to_hit(server, db, h, &chains)).collect();
        tracing::debug!(count = raw.len(), truncated, ms = started.elapsed().as_millis() as u64, "nesne araması");
        Ok(SearchResult { hits, truncated })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const U: &str = "3f2a6d1e-8a4b-4c2e-9f00-0123456789ab";
    fn srv() -> Uuid { Uuid::parse_str(U).unwrap() }

    #[test]
    fn escapes_like_wildcards() {
        assert_eq!(escape_like("a%b_c\\d"), "a\\%b\\_c\\\\d");
        assert_eq!(escape_like("Müşteri"), "Müşteri");
    }

    #[test]
    fn rejects_long_and_ignores_blank_queries() {
        assert!(matches!(normalize(&"x".repeat(201)), Err(Error::Invalid(_))));
        assert_eq!(normalize("   ").unwrap(), None);
        assert_eq!(normalize("  ad ").unwrap(), Some("ad".to_string()));
    }

    #[test]
    fn builds_sql_only_for_requested_kinds_and_version() {
        let sql = build_sql(&[SearchKind::Table], 16);
        assert!(sql.contains("'table'"));
        assert!(!sql.contains("'column'"));
        let all = build_sql(&[], 10);
        assert!(!all.contains("'procedure'"), "PG10'da procedure yok");
        assert!(all.contains("proisagg"), "PG10 fonksiyon süzgeci");
        let all16 = build_sql(&[], 16);
        assert!(all16.contains("'procedure'"));
        assert!(all16.contains("prokind"));
        assert!(!all16.contains("$q"), "kullanıcı metni SQL'e girmez; yalnız $1..$4");
    }

    fn raw(kind: SearchKind, rel_kind: &str) -> RawHit {
        RawHit {
            kind, oid: 50, attnum: 0, name: "n".into(), schema_oid: 2200, schema_name: Some("public".into()),
            rel_oid: 40, rel_kind: rel_kind.into(), owner: Some("t".into()), system: false,
        }
    }

    #[test]
    fn table_column_path_matches_tree_ids() {
        let mut h = raw(SearchKind::Column, "r");
        h.attnum = 3;
        let hit = to_hit(srv(), 16384, &h, &Default::default());
        let b = format!("srv:{U}/db:16384/sch:2200");
        assert_eq!(hit.id, format!("{b}/tbl:40/col:3"));
        assert_eq!(
            hit.path,
            vec![
                format!("srv:{U}/dbs"), format!("srv:{U}/db:16384"), format!("srv:{U}/db:16384/schemas"), b.clone(),
                format!("{b}/tables"), format!("{b}/tbl:40"), format!("{b}/tbl:40/columns"),
            ]
        );
        assert_eq!(hit.kind, NodeKind::Column);
    }

    #[test]
    fn cap_truncates_only_above_max_hits() {
        let (rows, truncated) = cap((0..MAX_HITS).collect::<Vec<_>>());
        assert_eq!((rows.len(), truncated), (MAX_HITS, false));
        let (rows, truncated) = cap((0..=MAX_HITS).collect::<Vec<_>>());
        assert_eq!((rows.len(), truncated), (MAX_HITS, true));
    }

    #[test]
    fn partition_chain_goes_through_root_schema() {
        // tbl:50 bölüm; zincir kök 30 (şema 2201) → 40 → 50.
        let mut h = raw(SearchKind::Partition, "r");
        h.oid = 50;
        h.rel_oid = 50;
        let chains = std::collections::HashMap::from([(50u32, Chain { root_schema: 2201, parents: vec![30, 40] })]);
        let hit = to_hit(srv(), 1, &h, &chains);
        let s = format!("srv:{U}/db:1/sch:2201");
        assert_eq!(hit.id, format!("{s}/tbl:30/tbl:40/tbl:50"));
        assert_eq!(hit.search_kind, SearchKind::Partition);
        assert_eq!(
            hit.path[3..].to_vec(),
            vec![s.clone(), format!("{s}/tables"), format!("{s}/tbl:30"), format!("{s}/tbl:30/partitions"),
                 format!("{s}/tbl:30/tbl:40"), format!("{s}/tbl:30/tbl:40/partitions")]
        );
    }

    #[test]
    fn derived_table_has_fixed_column_list() {
        let sql = build_sql(&[SearchKind::Table], 16);
        assert!(sql.contains("h(kind, oid, attnum, name, match_name, schema_oid, schema_name, rel_oid, rel_kind, owner, is_system)"));
        assert!(sql.contains("lower($2)"));
    }

    #[test]
    fn foreign_partition_column_goes_through_chain() {
        let mut h = raw(SearchKind::Column, "f");
        h.attnum = 2;
        h.rel_oid = 50;
        let chains = std::collections::HashMap::from([(50u32, Chain { root_schema: 2201, parents: vec![30] })]);
        let hit = to_hit(srv(), 1, &h, &chains);
        let s = format!("srv:{U}/db:1/sch:2201");
        assert_eq!(hit.id, format!("{s}/tbl:30/tbl:50/col:2"));
        assert_eq!(hit.path.last().unwrap(), &format!("{s}/tbl:30/tbl:50/columns"));
    }

    #[test]
    fn view_and_matview_owners() {
        let mut c = raw(SearchKind::Column, "v");
        c.attnum = 1;
        let col = to_hit(srv(), 1, &c, &Default::default());
        assert!(col.id.ends_with("/sch:2200/view:40/col:1"), "koleksiyon parçası nesne kimliğinde yok");
        assert!(col.path.iter().any(|p| p.ends_with("/sch:2200/views")));
        let i = raw(SearchKind::Index, "m");
        let hit = to_hit(srv(), 1, &i, &Default::default());
        assert!(hit.id.ends_with("/mv:40/idx:50"));
        assert!(hit.path.iter().any(|p| p.ends_with("/matviews")));
        assert!(hit.path.last().unwrap().ends_with("/mv:40/indexes"));
    }

    #[test]
    fn schema_level_and_db_level_kinds() {
        let f = to_hit(srv(), 1, &raw(SearchKind::TriggerFunction, ""), &Default::default());
        assert!(f.id.ends_with("/sch:2200/tfn:50"));
        assert!(f.path.last().unwrap().ends_with("/sch:2200/trigfuncs"));
        let mut e = raw(SearchKind::Extension, "");
        e.schema_name = None;
        let hit = to_hit(srv(), 1, &e, &Default::default());
        assert_eq!(hit.id, format!("srv:{U}/db:1/ext:50"));
        assert_eq!(hit.path.last().unwrap(), &format!("srv:{U}/db:1/extensions"));
        let s = to_hit(srv(), 1, &raw(SearchKind::Schema, ""), &Default::default());
        assert_eq!(s.id, format!("srv:{U}/db:1/sch:50"));
    }

    #[test]
    fn foreign_and_replication_ids_skip_collection_segment() {
        let d = format!("srv:{U}/db:1");
        let ft = to_hit(srv(), 1, &raw(SearchKind::ForeignTable, ""), &Default::default());
        assert_eq!((ft.id.as_str(), ft.kind), (format!("{d}/sch:2200/ftbl:50").as_str(), NodeKind::ForeignTable));
        assert_eq!(ft.path[2..].to_vec(), vec![format!("{d}/schemas"), format!("{d}/sch:2200"), format!("{d}/sch:2200/ftables")]);
        let mut s = raw(SearchKind::ForeignServer, "");
        (s.rel_oid, s.schema_name) = (7, None);
        let fs = to_hit(srv(), 1, &s, &Default::default());
        assert_eq!((fs.id.as_str(), fs.kind), (format!("{d}/fdw:7/fsrv:50").as_str(), NodeKind::ForeignServer));
        assert_eq!(fs.path[2..].to_vec(), vec![format!("{d}/fdws"), format!("{d}/fdw:7"), format!("{d}/fdw:7/fservers")]);
        let pubh = to_hit(srv(), 1, &raw(SearchKind::Publication, ""), &Default::default());
        assert_eq!((pubh.id.as_str(), pubh.kind), (format!("{d}/pub:50").as_str(), NodeKind::Publication));
        assert_eq!(pubh.path.last().unwrap(), &format!("{d}/pubs"));
        let sub = to_hit(srv(), 1, &raw(SearchKind::Subscription, ""), &Default::default());
        assert_eq!((sub.id.as_str(), sub.kind), (format!("{d}/sub:50").as_str(), NodeKind::Subscription));
        assert_eq!(sub.path.last().unwrap(), &format!("{d}/subs"));
    }

    #[test]
    fn subscription_fragment_never_reads_conninfo_and_falls_back() {
        for sub in [SubSource::Oid, SubSource::Stats] {
            let sql = assemble(&selected(&[], 13, sub));
            assert!(sql.contains("'subscription'"), "{sub:?}");
            assert!(!sql.contains("subconninfo") && !sql.contains("srvoptions") && !sql.contains("umoptions"), "{sub:?}");
            assert!(sql.contains("s.subdbid = (SELECT d.oid FROM pg_database d WHERE d.datname = current_database())"), "{sub:?}");
        }
        assert!(assemble(&selected(&[], 13, SubSource::Oid)).contains("SELECT 'subscription', s.oid,"));
        assert!(assemble(&selected(&[], 13, SubSource::Stats)).contains("pg_stat_subscription"));
        let skip = selected(&[], 13, SubSource::Skip);
        assert!(!assemble(&skip).contains("'subscription'"));
        assert!(assemble(&skip).contains("'publication'"), "yalnız subscription atlanır");
        assert!(selected(&[SearchKind::Subscription], 13, SubSource::Skip).is_empty());
        assert!(has_subscription(&[]) && has_subscription(&[SearchKind::Subscription]) && !has_subscription(&[SearchKind::Table]));
    }
}
