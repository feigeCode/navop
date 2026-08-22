use std::path::Path;

use one_core::storage::{DatabaseType, DbConnectionConfig};

pub fn file_config(id: &str, database_type: DatabaseType, path: &Path) -> DbConnectionConfig {
    DbConnectionConfig {
        id: id.to_string(),
        database_type,
        name: id.to_string(),
        host: path.to_string_lossy().to_string(),
        port: 0,
        username: String::new(),
        password: String::new(),
        credential_reference: None,
        database: None,
        service_name: None,
        sid: None,
        workspace_id: None,
        proxy: None,
        extra_params: std::collections::HashMap::new(),
    }
}
