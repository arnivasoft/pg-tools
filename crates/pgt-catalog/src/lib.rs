mod db;
mod ddl;
mod kinds;
mod quote;
pub mod node;
pub mod props;
pub mod version;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, RwLock};

use pgt_core::connection::ConnectionManager;
use pgt_core::registry::Registry;
pub use pgt_core::{Error, Result};

use node::{NodeId, TreeNode};

pub struct Catalog {
    pub(crate) mgr: Arc<ConnectionManager>,
    pub(crate) registry: Arc<RwLock<Registry>>,
    show_system: AtomicBool,
}

impl Catalog {
    pub fn new(mgr: Arc<ConnectionManager>, registry: Arc<RwLock<Registry>>) -> Self {
        Self { mgr, registry, show_system: AtomicBool::new(false) }
    }

    pub fn set_show_system(&self, value: bool) {
        self.show_system.store(value, Ordering::Relaxed);
    }

    pub(crate) fn show_system(&self) -> bool {
        self.show_system.load(Ordering::Relaxed)
    }

    pub async fn properties(&self, id: &NodeId) -> Result<Vec<props::PropertySection>> {
        kinds::properties(self, id).await
    }

    pub async fn ddl(&self, id: &NodeId) -> Result<String> {
        kinds::ddl(self, id).await
    }

    pub async fn list_children(&self, id: &NodeId) -> Result<Vec<TreeNode>> {
        kinds::list_children(self, id).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn show_system_flag_defaults_off_and_follows_the_setter() {
        let catalog = Catalog::new(Arc::new(ConnectionManager::new()), Arc::new(RwLock::new(Registry::unavailable("servers.json"))));
        assert!(!catalog.show_system(), "sistem nesneleri varsayılan olarak gizli");
        catalog.set_show_system(true);
        assert!(catalog.show_system());
        catalog.set_show_system(false);
        assert!(!catalog.show_system());
    }
}
