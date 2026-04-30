use std::collections::HashMap;

use gpui::{Pixels, Point};

use crate::{NodeId, PortId};

#[derive(Debug, Clone)]
pub struct PortLayoutCache {
    pub map: HashMap<NodeId, HashMap<PortId, Point<Pixels>>>,
}

impl PortLayoutCache {
    pub fn new() -> Self {
        Self {
            map: HashMap::new(),
        }
    }

    pub fn clear_node(&mut self, node_id: &NodeId) {
        self.map.remove(node_id);
    }

    pub fn claer_all(&mut self) {
        self.map.clear();
    }
}
