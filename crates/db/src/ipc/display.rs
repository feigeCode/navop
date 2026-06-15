use crate::ipc::{IpcDriverManifest, IpcDriverRegistry};
use gpui_component::{IconName, IconNamed};
use one_core::storage::DbConnectionConfig;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IpcDriverDisplay {
    pub driver_id: String,
    pub name: String,
    pub icon_asset_path: Option<String>,
}

impl IpcDriverManifest {
    pub fn icon_asset_path(&self) -> Option<String> {
        self.icon_asset_path_for(&self.ui.icon, "icon")
    }

    pub fn color_icon_asset_path(&self) -> Option<String> {
        let icon = self.ui.icon_color.as_deref()?;
        self.icon_asset_path_for(icon, "icon_color")
    }

    pub fn preferred_icon_asset_path(&self) -> Option<String> {
        self.color_icon_asset_path()
            .or_else(|| self.icon_asset_path())
    }

    fn icon_asset_path_for(&self, icon: &str, resource: &str) -> Option<String> {
        let icon = icon.trim();
        if icon.is_empty() {
            return None;
        }
        builtin_icon_asset_path(icon).or_else(|| Some(format!("driver://{}/{resource}", self.id)))
    }
}

impl IpcDriverRegistry {
    pub fn display_for_driver_id(&self, driver_id: &str) -> Option<IpcDriverDisplay> {
        let driver = self.find(driver_id)?;
        Some(IpcDriverDisplay {
            driver_id: driver.id.clone(),
            name: driver.name.clone(),
            icon_asset_path: driver.preferred_icon_asset_path(),
        })
    }

    pub fn display_for_config(&self, config: &DbConnectionConfig) -> Option<IpcDriverDisplay> {
        let driver_id = config.database_type.external_driver_id()?;
        self.display_for_driver_id(driver_id)
    }
}

fn builtin_icon_asset_path(icon: &str) -> Option<String> {
    let icon_name = match icon {
        "Database" => IconName::Database,
        "DuckDB" => IconName::DuckDB,
        "ClickHouse" | "ClickHouseColor" => IconName::ClickHouseColor,
        "MongoDB" => IconName::MongoDB,
        "MySQL" | "MySQLColor" => IconName::MySQLColor,
        "PostgreSQL" | "PostgreSQLColor" => IconName::PostgreSQLColor,
        "Redis" | "RedisColor" => IconName::RedisColor,
        "SQLite" | "SQLiteColor" => IconName::SQLiteColor,
        "Server" => IconName::Server,
        "Terminal" | "TerminalColor" => IconName::TerminalColor,
        _ => return None,
    };
    Some(icon_name.path().to_string())
}
