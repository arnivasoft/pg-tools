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
    let info = cat.mgr.info(s.id);
    TreeNode::new(&id, NodeKind::Server, s.name.clone())
        .with_detail(info.as_ref().map(|i| i.version.clone()))
        .with_server(s.id)
        .with_flags(NodeFlags { connected: info.is_some(), locked: s.read_only_lock, ..NodeFlags::default() })
        .with_marks(s.color.clone(), s.label.clone())
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

pub(crate) async fn properties(cat: &Catalog, server: uuid::Uuid) -> Result<Vec<crate::props::PropertySection>> {
    use crate::props::PropertySection;
    let config = cat
        .registry
        .read()
        .unwrap()
        .get(server)
        .cloned()
        .ok_or(pgt_core::Error::UnknownServer(server))?;
    let ssl_mode = serde_json::to_value(config.ssl.mode).ok().and_then(|v| v.as_str().map(str::to_string)).unwrap_or_default();
    let mut out = vec![
        PropertySection::new("Genel")
            .row("Ad", config.name.clone())
            .row_opt("Grup", config.group_name())
            .row("Host", config.host.clone())
            .row("Port", config.port.to_string())
            .row("Kullanıcı", config.username.clone())
            .row("Bakım veritabanı", config.maintenance_db.clone())
            .row("SSL modu", ssl_mode),
    ];
    if let Some(info) = cat.mgr.info(server) {
        out.push(PropertySection::new("Sunucu").row("Sürüm", info.version).row("Sürüm numarası", info.version_num.to_string()));
    }
    Ok(out)
}
