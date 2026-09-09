pub mod connection;
pub mod credential_vault;
pub mod demo_database;
pub mod manager;
pub mod master_key_rotation;
pub mod migration;
pub mod models;
pub mod query_directory;
pub mod quick_command;
pub mod rdp_settings;
pub mod repository;
pub mod row_mapping;
pub mod sftp_favorite_path;
pub mod sql_execution_history;
pub mod team_key_cache;
pub mod team_membership_cache;
pub mod terminal_command_history;
pub mod traits;

#[cfg(test)]
#[path = "team_key_cache_tests.rs"]
mod team_key_cache_tests;

#[cfg(test)]
#[path = "quick_command_defaults_tests.rs"]
mod quick_command_defaults_tests;

#[cfg(test)]
#[path = "rdp_settings_tests.rs"]
mod rdp_settings_tests;

#[cfg(test)]
#[path = "master_key_rotation_tests.rs"]
mod master_key_rotation_tests;

use anyhow::Result;
pub use credential_vault::*;
use gpui::App;
pub use manager::*;
pub use master_key_rotation::*;
pub use models::*;
pub use query_directory::*;
pub use quick_command::*;
pub use rdp_settings::*;
pub use repository::*;
pub use sftp_favorite_path::*;
pub use sql_execution_history::*;
pub use team_key_cache::*;
pub use team_membership_cache::*;
pub use terminal_command_history::*;

pub fn init(cx: &mut App) -> Result<()> {
    cx.set_global(ActiveConnections::new());
    manager::init(cx)?;
    repository::init(cx);

    // 首次启动时创建演示数据库
    let storage = cx.global::<GlobalStorageState>().storage.clone();
    if let Some(conn_repo) = storage.get::<ConnectionRepository>() {
        demo_database::try_init_demo(&conn_repo);
    }
    Ok(())
}
