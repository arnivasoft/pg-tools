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
}

impl Coll {
    fn tag(self) -> &'static str {
        match self {
            Coll::Databases => "dbs",
            Coll::Schemas => "schemas",
            Coll::Tables => "tables",
            Coll::Columns => "columns",
        }
    }
    fn from_tag(tag: &str) -> Option<Self> {
        Some(match tag {
            "dbs" => Coll::Databases,
            "schemas" => Coll::Schemas,
            "tables" => Coll::Tables,
            "columns" => Coll::Columns,
            _ => return None,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Seg {
    Coll(Coll),
    Db(u32),
    Schema(u32),
    Table(u32),
    Column(i16),
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
                Some(("db", n)) => Seg::Db(n.parse().map_err(|_| invalid(s))?),
                Some(("sch", n)) => Seg::Schema(n.parse().map_err(|_| invalid(s))?),
                Some(("tbl", n)) => Seg::Table(n.parse().map_err(|_| invalid(s))?),
                Some(("col", n)) => Seg::Column(n.parse().map_err(|_| invalid(s))?),
                Some(_) => return Err(invalid(s)),
                None => Seg::Coll(Coll::from_tag(part).ok_or_else(|| invalid(s))?),
            };
            path.push(seg);
        }
        Ok(NodeId::Node { server, path })
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
                        Seg::Db(o) => write!(f, "/db:{o}")?,
                        Seg::Schema(o) => write!(f, "/sch:{o}")?,
                        Seg::Table(o) => write!(f, "/tbl:{o}")?,
                        Seg::Column(n) => write!(f, "/col:{n}")?,
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
}
