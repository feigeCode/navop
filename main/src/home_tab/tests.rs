use super::*;
use one_core::storage::DbConnectionConfig;

use super::keybindings::{OPEN_LOCAL_TERMINAL_SHORTCUT_MACOS, OPEN_LOCAL_TERMINAL_SHORTCUT_OTHER};

mod rendering;
mod sync;
mod titles;

fn stored_external_connection(driver_id: &str) -> StoredConnection {
    StoredConnection::new_database(
        "demo".to_string(),
        DbConnectionConfig {
            id: String::new(),
            database_type: DatabaseType::external(driver_id),
            name: "demo".to_string(),
            host: "localhost".to_string(),
            port: 0,
            username: String::new(),
            password: String::new(),
            database: None,
            service_name: None,
            sid: None,
            workspace_id: None,
            proxy: None,
            extra_params: std::collections::HashMap::new(),
        },
        None,
    )
}
