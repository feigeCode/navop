use crate::ipc::{IpcDriverManifest, IpcDriverRegistry};
use gpui_component::{Icon, IconName, IconNamed, Sizable, Size};
use one_core::storage::DbConnectionConfig;
use std::path::{Path, PathBuf};
use tracing::info;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IpcDriverDisplay {
    pub driver_id: String,
    pub name: String,
    pub icon_asset_path: Option<String>,
    pub icon_file_path: Option<PathBuf>,
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

    pub fn icon_file_path(&self) -> Option<PathBuf> {
        self.icon_file_path_for(&self.ui.icon, "icon")
    }

    pub fn color_icon_file_path(&self) -> Option<PathBuf> {
        let icon = self.ui.icon_color.as_deref()?;
        self.icon_file_path_for(icon, "icon_color")
    }

    pub fn preferred_icon_file_path(&self) -> Option<PathBuf> {
        self.color_icon_file_path()
            .or_else(|| self.icon_file_path())
    }

    fn icon_asset_path_for(&self, icon: &str, resource: &str) -> Option<String> {
        let icon = icon.trim();
        if icon.is_empty() {
            info!(
                target: "driver_icon",
                driver_id = %self.id,
                resource,
                "driver icon manifest value is empty"
            );
            return None;
        }
        let asset_path = builtin_icon_asset_path(icon)
            .unwrap_or_else(|| format!("driver://{}/{resource}{}", self.id, icon_extension(icon)));
        info!(
            target: "driver_icon",
            driver_id = %self.id,
            resource,
            manifest_icon = icon,
            asset_path = %asset_path,
            manifest_dir = %self.manifest_dir.display(),
            "resolved driver icon asset path"
        );
        Some(asset_path)
    }

    fn icon_file_path_for(&self, icon: &str, resource: &str) -> Option<PathBuf> {
        let icon = icon.trim();
        if icon.is_empty() || builtin_icon_asset_path(icon).is_some() {
            return None;
        }
        let file_path = self.manifest_dir.join(icon);
        info!(
            target: "driver_icon",
            driver_id = %self.id,
            resource,
            manifest_icon = icon,
            file_path = %file_path.display(),
            exists = file_path.is_file(),
            "resolved driver icon file path"
        );
        Some(file_path)
    }
}

pub fn driver_icon_from_asset_path(path: impl Into<String>, size: impl Into<Size>) -> Icon {
    Icon::default().path(path.into()).color().with_size(size)
}

pub fn driver_icon_from_file_path(path: impl Into<PathBuf>, size: impl Into<Size>) -> Icon {
    Icon::default()
        .file_path(path.into())
        .color()
        .with_size(size)
}

impl IpcDriverRegistry {
    pub fn display_for_driver_id(&self, driver_id: &str) -> Option<IpcDriverDisplay> {
        let driver = match self.find(driver_id) {
            Some(driver) => driver,
            None => {
                info!(
                    target: "driver_icon",
                    driver_id,
                    "external driver display lookup missed registry"
                );
                return None;
            }
        };
        let display = IpcDriverDisplay {
            driver_id: driver.id.clone(),
            name: driver.name.clone(),
            icon_asset_path: driver.preferred_icon_asset_path(),
            icon_file_path: driver.preferred_icon_file_path(),
        };
        let display_driver_id = display.driver_id.as_str();
        let display_name = display.name.as_str();
        let display_icon_asset_path = display.icon_asset_path.as_deref();
        let display_icon_file_path = display.icon_file_path.as_ref().map(|path| path.display());
        info!(
            target: "driver_icon",
            driver_id = %display_driver_id,
            name = %display_name,
            icon_asset_path = ?display_icon_asset_path,
            icon_file_path = ?display_icon_file_path,
            ui_icon = %driver.ui.icon,
            ui_icon_color = ?driver.ui.icon_color,
            "resolved external driver display"
        );
        Some(display)
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

fn icon_extension(icon: &str) -> String {
    Path::new(icon)
        .extension()
        .and_then(|extension| extension.to_str())
        .filter(|extension| !extension.trim().is_empty())
        .map(|extension| format!(".{extension}"))
        .unwrap_or_default()
}
