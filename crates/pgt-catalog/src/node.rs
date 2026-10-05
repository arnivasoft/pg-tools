use std::fmt;

use serde::Serialize;
use ts_rs::TS;
use uuid::Uuid;

use crate::{Error, Result};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Coll {
    Databases,
    Schemas,
    Tables,
    Columns,
    Views,
    MatViews,
    Functions,
    Procedures,
    TriggerFunctions,
    Sequences,
    Types,
    Domains,
    Constraints,
    Indexes,
    Triggers,
    Policies,
    Partitions,
    Extensions,
    Roles,
    Tablespaces,
}

const COLL_TAGS: [(Coll, &str); 20] = [
    (Coll::Databases, "dbs"),
    (Coll::Schemas, "schemas"),
    (Coll::Tables, "tables"),
    (Coll::Columns, "columns"),
    (Coll::Views, "views"),
    (Coll::MatViews, "matviews"),
    (Coll::Functions, "functions"),
    (Coll::Procedures, "procedures"),
    (Coll::TriggerFunctions, "trigfuncs"),
    (Coll::Sequences, "sequences"),
    (Coll::Types, "types"),
    (Coll::Domains, "domains"),
    (Coll::Constraints, "constraints"),
    (Coll::Indexes, "indexes"),
    (Coll::Triggers, "triggers"),
    (Coll::Policies, "policies"),
    (Coll::Partitions, "partitions"),
    (Coll::Extensions, "extensions"),
    (Coll::Roles, "roles"),
    (Coll::Tablespaces, "tablespaces"),
];

impl Coll {
    fn tag(self) -> &'static str {
        COLL_TAGS.iter().find(|(c, _)| *c == self).map(|(_, t)| *t).expect("her Coll'un etiketi var")
    }
    fn from_tag(tag: &str) -> Option<Self> {
        COLL_TAGS.iter().find(|(_, t)| *t == tag).map(|(c, _)| *c)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Seg {
    Coll(Coll),
    Db(u32),
    Schema(u32),
    Table(u32),
    Column(i16),
    Constraint(u32),
    Index(u32),
    Trigger(u32),
    Policy(u32),
    View(u32),
    MatView(u32),
    Function(u32),
    Procedure(u32),
    TriggerFunction(u32),
    Sequence(u32),
    Type(u32),
    Domain(u32),
    Extension(u32),
    Role(u32),
    Tablespace(u32),
}

impl Seg {
    /// OID taşıyan nesne parçaları için `(etiket, oid)`. `Column` (attnum, negatif olabilir) ve `Coll` hariç.
    fn oid_tag(self) -> Option<(&'static str, u32)> {
        Some(match self {
            Seg::Db(o) => ("db", o),
            Seg::Schema(o) => ("sch", o),
            Seg::Table(o) => ("tbl", o),
            Seg::Constraint(o) => ("con", o),
            Seg::Index(o) => ("idx", o),
            Seg::Trigger(o) => ("trg", o),
            Seg::Policy(o) => ("pol", o),
            Seg::View(o) => ("view", o),
            Seg::MatView(o) => ("mv", o),
            Seg::Function(o) => ("fn", o),
            Seg::Procedure(o) => ("proc", o),
            Seg::TriggerFunction(o) => ("tfn", o),
            Seg::Sequence(o) => ("seq", o),
            Seg::Type(o) => ("typ", o),
            Seg::Domain(o) => ("dom", o),
            Seg::Extension(o) => ("ext", o),
            Seg::Role(o) => ("role", o),
            Seg::Tablespace(o) => ("ts", o),
            Seg::Column(_) | Seg::Coll(_) => return None,
        })
    }

    fn from_oid_tag(tag: &str, oid: u32) -> Option<Seg> {
        Some(match tag {
            "db" => Seg::Db(oid),
            "sch" => Seg::Schema(oid),
            "tbl" => Seg::Table(oid),
            "con" => Seg::Constraint(oid),
            "idx" => Seg::Index(oid),
            "trg" => Seg::Trigger(oid),
            "pol" => Seg::Policy(oid),
            "view" => Seg::View(oid),
            "mv" => Seg::MatView(oid),
            "fn" => Seg::Function(oid),
            "proc" => Seg::Procedure(oid),
            "tfn" => Seg::TriggerFunction(oid),
            "seq" => Seg::Sequence(oid),
            "typ" => Seg::Type(oid),
            "dom" => Seg::Domain(oid),
            "ext" => Seg::Extension(oid),
            "role" => Seg::Role(oid),
            "ts" => Seg::Tablespace(oid),
            _ => return None,
        })
    }

    /// Nesne parçalarının OID'i (`Column` ve `Coll` için `None`).
    pub fn oid(self) -> Option<u32> {
        self.oid_tag().map(|(_, o)| o)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum NodeId {
    Root,
    Group(String),
    Node { server: Uuid, path: Vec<Seg> },
}

fn invalid(s: &str) -> Error {
    Error::Invalid(format!("geçersiz düğüm kimliği: {s}"))
}

impl NodeId {
    pub fn server(server: Uuid) -> Self {
        NodeId::Node { server, path: Vec::new() }
    }

    pub fn child(&self, seg: Seg) -> Self {
        match self {
            NodeId::Node { server, path } => {
                let mut path = path.clone();
                path.push(seg);
                NodeId::Node { server: *server, path }
            }
            _ => panic!("yalnızca sunucu altındaki düğümlerin çocuğu olabilir"),
        }
    }

    /// Son parçayı atar (koleksiyon düğümünün üst nesnesini bulmak için). Sunucu kimliğinde kendisini döndürür.
    pub fn parent(&self) -> Self {
        match self {
            NodeId::Node { server, path } if !path.is_empty() => {
                NodeId::Node { server: *server, path: path[..path.len() - 1].to_vec() }
            }
            other => other.clone(),
        }
    }

    pub fn parse(s: &str) -> Result<Self> {
        if s.is_empty() {
            return Ok(NodeId::Root);
        }
        if let Some(name) = s.strip_prefix("grp:") {
            return if name.is_empty() { Err(invalid(s)) } else { Ok(NodeId::Group(name.to_string())) };
        }
        let mut parts = s.split('/');
        let server = parts
            .next()
            .and_then(|p| p.strip_prefix("srv:"))
            .and_then(|u| Uuid::parse_str(u).ok())
            .ok_or_else(|| invalid(s))?;
        let mut path = Vec::new();
        for part in parts {
            let seg = match part.split_once(':') {
                Some(("col", n)) => Seg::Column(n.parse().map_err(|_| invalid(s))?),
                Some((tag, n)) => {
                    let oid: u32 = n.parse().map_err(|_| invalid(s))?;
                    Seg::from_oid_tag(tag, oid).ok_or_else(|| invalid(s))?
                }
                None => Seg::Coll(Coll::from_tag(part).ok_or_else(|| invalid(s))?),
            };
            path.push(seg);
        }
        Ok(NodeId::Node { server, path })
    }
}

/// "Query Tool'da aç" hedefi: düğümün sunucusu ve (varsa) yolundaki veritabanı OID'i. Arayüz düğüm kimliğini
/// ayrıştırmaz; `qt_target_of` bunu döndürür. `database_oid: None` = bakım veritabanı (ör. sunucu düğümü).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct QueryTarget {
    pub server_id: Uuid,
    pub database_oid: Option<u32>,
}

impl NodeId {
    /// Sunucu altındaki düğümler için hedef; kök ve grup düğümlerinde `None`.
    pub fn query_target(&self) -> Option<QueryTarget> {
        match self {
            NodeId::Node { server, path } => Some(QueryTarget {
                server_id: *server,
                database_oid: path.iter().find_map(|s| if let Seg::Db(oid) = s { Some(*oid) } else { None }),
            }),
            _ => None,
        }
    }
}

impl fmt::Display for NodeId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            NodeId::Root => Ok(()),
            NodeId::Group(name) => write!(f, "grp:{name}"),
            NodeId::Node { server, path } => {
                write!(f, "srv:{server}")?;
                for seg in path {
                    match seg {
                        Seg::Coll(c) => write!(f, "/{}", c.tag())?,
                        Seg::Column(n) => write!(f, "/col:{n}")?,
                        other => {
                            let (tag, oid) = other.oid_tag().expect("OID taşıyan parça");
                            write!(f, "/{tag}:{oid}")?
                        }
                    }
                }
                Ok(())
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum NodeKind {
    ServerGroup,
    Server,
    DatabaseCollection,
    Database,
    SchemaCollection,
    Schema,
    TableCollection,
    Table,
    ColumnCollection,
    Column,
    ConstraintCollection,
    Constraint,
    IndexCollection,
    Index,
    TriggerCollection,
    Trigger,
    PolicyCollection,
    Policy,
    PartitionCollection,
    ViewCollection,
    View,
    MaterializedViewCollection,
    MaterializedView,
    FunctionCollection,
    Function,
    ProcedureCollection,
    Procedure,
    TriggerFunctionCollection,
    TriggerFunction,
    SequenceCollection,
    Sequence,
    TypeCollection,
    Type,
    DomainCollection,
    Domain,
    ExtensionCollection,
    Extension,
    RoleCollection,
    Role,
    TablespaceCollection,
    Tablespace,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct NodeFlags {
    pub connected: bool,
    pub partitioned: bool,
    pub system: bool,
    pub primary_key: bool,
    pub not_null: bool,
    pub from_extension: bool,
    /// Yalnızca sunucu düğümlerinde: salt okunur kilit açık.
    pub locked: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct TreeNode {
    pub id: String,
    pub kind: NodeKind,
    pub label: String,
    pub detail: Option<String>,
    pub has_children: bool,
    pub server_id: Option<Uuid>,
    pub flags: NodeFlags,
    /// Yalnızca sunucu düğümlerinde: renk adı (`pgt_core::config::COLORS`) ve ortam etiketi.
    pub color: Option<String>,
    pub tag: Option<String>,
}

impl TreeNode {
    pub fn new(id: &NodeId, kind: NodeKind, label: impl Into<String>) -> Self {
        Self {
            id: id.to_string(),
            kind,
            label: label.into(),
            detail: None,
            has_children: true,
            server_id: None,
            flags: NodeFlags::default(),
            color: None,
            tag: None,
        }
    }
    pub fn with_detail(mut self, detail: Option<String>) -> Self {
        self.detail = detail;
        self
    }
    pub fn leaf(mut self) -> Self {
        self.has_children = false;
        self
    }
    pub fn with_flags(mut self, flags: NodeFlags) -> Self {
        self.flags = flags;
        self
    }
    pub fn with_marks(mut self, color: Option<String>, tag: Option<String>) -> Self {
        self.color = color;
        self.tag = tag;
        self
    }
    pub fn with_server(mut self, server: Uuid) -> Self {
        self.server_id = Some(server);
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const U: &str = "3f2a6d1e-8a4b-4c2e-9f00-0123456789ab";

    #[test]
    fn query_target_finds_server_and_database() {
        let u = Uuid::parse_str(U).unwrap();
        let t = |id: &str| NodeId::parse(id).unwrap().query_target();
        assert_eq!(t(&format!("srv:{U}")), Some(QueryTarget { server_id: u, database_oid: None }));
        assert_eq!(t(&format!("srv:{U}/dbs")), Some(QueryTarget { server_id: u, database_oid: None }));
        assert_eq!(t(&format!("srv:{U}/db:16384")), Some(QueryTarget { server_id: u, database_oid: Some(16384) }));
        assert_eq!(
            t(&format!("srv:{U}/db:16384/sch:2200/tables/tbl:24576")),
            Some(QueryTarget { server_id: u, database_oid: Some(16384) })
        );
        assert_eq!(t(&format!("srv:{U}/roles/role:10")), Some(QueryTarget { server_id: u, database_oid: None }));
        assert_eq!(t(""), None);
        assert_eq!(t("grp:Üretim"), None);
    }

    #[test]
    fn round_trips_all_shapes() {
        let cases = [
            "".to_string(),
            "grp:Üretim/Ankara".to_string(),
            format!("srv:{U}"),
            format!("srv:{U}/dbs"),
            format!("srv:{U}/db:16384"),
            format!("srv:{U}/db:16384/schemas"),
            format!("srv:{U}/db:16384/sch:2200"),
            format!("srv:{U}/db:16384/sch:2200/tables"),
            format!("srv:{U}/db:16384/sch:2200/tbl:24576"),
            format!("srv:{U}/db:16384/sch:2200/tbl:24576/columns"),
            format!("srv:{U}/db:16384/sch:2200/tbl:24576/col:3"),
        ];
        for s in cases {
            assert_eq!(NodeId::parse(&s).unwrap().to_string(), s);
        }
    }

    #[test]
    fn parses_into_typed_segments() {
        let id = NodeId::parse(&format!("srv:{U}/db:16384/sch:2200/tbl:24576/col:3")).unwrap();
        let NodeId::Node { server, path } = id else { panic!() };
        assert_eq!(server.to_string(), U);
        assert_eq!(path, vec![Seg::Db(16384), Seg::Schema(2200), Seg::Table(24576), Seg::Column(3)]);
        assert_eq!(NodeId::parse("grp:a/b").unwrap(), NodeId::Group("a/b".into()));
        assert_eq!(NodeId::parse("").unwrap(), NodeId::Root);
    }

    #[test]
    fn child_builds_paths() {
        let server = Uuid::parse_str(U).unwrap();
        let id = NodeId::server(server).child(Seg::Db(5)).child(Seg::Coll(Coll::Schemas));
        assert_eq!(id.to_string(), format!("srv:{U}/db:5/schemas"));
    }

    #[test]
    fn rejects_garbage() {
        for bad in ["x", "grp:", "srv:not-a-uuid", &format!("srv:{U}/db:abc"), &format!("srv:{U}/wat"), &format!("srv:{U}/col:99999")] {
            assert!(matches!(NodeId::parse(bad), Err(Error::Invalid(_))), "{bad}");
        }
    }

    #[test]
    fn tree_node_serializes_camel_case() {
        let server = Uuid::parse_str(U).unwrap();
        let n = TreeNode::new(&NodeId::server(server), NodeKind::Server, "prod").with_server(server);
        let v = serde_json::to_value(&n).unwrap();
        assert_eq!(v["kind"], "server");
        assert_eq!(v["hasChildren"], true);
        assert_eq!(v["serverId"], U);
        assert_eq!(v["flags"]["primaryKey"], false);
        assert_eq!(serde_json::to_value(NodeKind::TableCollection).unwrap(), "tableCollection");
    }
    #[test]
    fn round_trips_new_shapes() {
        let cases = [
            format!("srv:{U}/roles"),
            format!("srv:{U}/tablespaces"),
            format!("srv:{U}/role:10"),
            format!("srv:{U}/ts:1663"),
            format!("srv:{U}/db:16384/extensions"),
            format!("srv:{U}/db:16384/ext:9"),
            format!("srv:{U}/db:16384/sch:2200/views"),
            format!("srv:{U}/db:16384/sch:2200/matviews"),
            format!("srv:{U}/db:16384/sch:2200/functions"),
            format!("srv:{U}/db:16384/sch:2200/procedures"),
            format!("srv:{U}/db:16384/sch:2200/trigfuncs"),
            format!("srv:{U}/db:16384/sch:2200/sequences"),
            format!("srv:{U}/db:16384/sch:2200/types"),
            format!("srv:{U}/db:16384/sch:2200/domains"),
            format!("srv:{U}/db:16384/sch:2200/view:30"),
            format!("srv:{U}/db:16384/sch:2200/mv:31"),
            format!("srv:{U}/db:16384/sch:2200/fn:32"),
            format!("srv:{U}/db:16384/sch:2200/proc:33"),
            format!("srv:{U}/db:16384/sch:2200/tfn:34"),
            format!("srv:{U}/db:16384/sch:2200/seq:35"),
            format!("srv:{U}/db:16384/sch:2200/typ:36"),
            format!("srv:{U}/db:16384/sch:2200/dom:37"),
            format!("srv:{U}/db:16384/sch:2200/tbl:24576/constraints"),
            format!("srv:{U}/db:16384/sch:2200/tbl:24576/con:40"),
            format!("srv:{U}/db:16384/sch:2200/tbl:24576/idx:41"),
            format!("srv:{U}/db:16384/sch:2200/tbl:24576/trg:42"),
            format!("srv:{U}/db:16384/sch:2200/tbl:24576/pol:43"),
            format!("srv:{U}/db:16384/sch:2200/tbl:24576/partitions"),
            format!("srv:{U}/db:16384/sch:2200/tbl:24576/tbl:24580"),
            format!("srv:{U}/db:16384/sch:2200/view:30/col:-1"),
        ];
        for s in cases {
            assert_eq!(NodeId::parse(&s).unwrap().to_string(), s);
        }
    }

    #[test]
    fn typed_segments_for_new_kinds() {
        let id = NodeId::parse(&format!("srv:{U}/db:1/sch:2/tbl:3/con:4")).unwrap();
        let NodeId::Node { path, .. } = id else { panic!() };
        assert_eq!(path, vec![Seg::Db(1), Seg::Schema(2), Seg::Table(3), Seg::Constraint(4)]);
        assert_eq!(Seg::Constraint(4).oid(), Some(4));
        assert_eq!(Seg::Role(10).oid(), Some(10));
        assert_eq!(Seg::Column(3).oid(), None);
        assert_eq!(Seg::Coll(Coll::Views).oid(), None);
    }

    #[test]
    fn rejects_new_garbage() {
        for bad in [format!("srv:{U}/con:abc"), format!("srv:{U}/con:-5"), format!("srv:{U}/nope:1"), format!("srv:{U}/rolez")] {
            assert!(matches!(NodeId::parse(&bad), Err(Error::Invalid(_))), "{bad}");
        }
    }

    #[test]
    fn new_kinds_and_flag_serialize_camel_case() {
        assert_eq!(serde_json::to_value(NodeKind::MaterializedViewCollection).unwrap(), "materializedViewCollection");
        assert_eq!(serde_json::to_value(NodeKind::TriggerFunction).unwrap(), "triggerFunction");
        let f = serde_json::to_value(NodeFlags::default()).unwrap();
        assert_eq!(f["fromExtension"], false);
    }
}
