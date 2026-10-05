use pgt_core::Result;
use pgt_core::config::ServerConfig;

use crate::Catalog;
use crate::node::{NodeFlags, NodeId, NodeKind, TreeNode};

fn sorted(mut servers: Vec<ServerConfig>) -> Vec<ServerConfig> {
    servers.sort_by_key(|s| s.name.to_lowercase());
    servers
}

async fn server_node(cat: &Catalog, s: &ServerConfig) -> TreeNode {
    let id = NodeId::server(s.id);
    let info = cat.mgr.info(s.id).await;
    TreeNode::new(&id, NodeKind::Server, s.name.clone())
        .with_detail(info.as_ref().map(|i| i.version.clone()))
        .with_server(s.id)
        .with_flags(NodeFlags { connected: info.is_some(), ..NodeFlags::default() })
}

pub(crate) async fn root_children(cat: &Catalog) -> Result<Vec<TreeNode>> {
    let servers = cat.registry.read().unwrap().servers().to_vec();
    let mut groups: Vec<String> = servers.iter().filter_map(|s| s.group_name().map(str::to_string)).collect();
    groups.sort_by_key(|g| g.to_lowercase());
    groups.dedup();
    let mut out: Vec<TreeNode> = groups
        .into_iter()
        .map(|g| TreeNode::new(&NodeId::Group(g.clone()), NodeKind::ServerGroup, g))
        .collect();
    for s in sorted(servers.into_iter().filter(|s| s.group_name().is_none()).collect()) {
        out.push(server_node(cat, &s).await);
    }
    Ok(out)
}

pub(crate) async fn group_children(cat: &Catalog, group: &str) -> Result<Vec<TreeNode>> {
    let servers = cat.registry.read().unwrap().servers().to_vec();
    let mut out = Vec::new();
    for s in sorted(servers.into_iter().filter(|s| s.group_name() == Some(group)).collect()) {
        out.push(server_node(cat, &s).await);
    }
    Ok(out)
}
