use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ActionContext {
    pub extension_id: String,
    pub command_id: String,
    pub node_id: String,
    pub node_name: String,
    pub node_type: String,
    pub database_type: String,
    pub connection_id: String,
}
