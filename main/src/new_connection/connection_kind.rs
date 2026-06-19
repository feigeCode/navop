use crate::external_driver_display::{
    external_driver_icon_from_file_path, external_driver_icon_from_path,
};
use db::ipc::IpcDriverRegistry;
use gpui::{Styled, px};
use gpui_component::{Icon, IconName, Sizable};
use one_core::storage::DatabaseType;
use rust_i18n::t;
use std::path::PathBuf;

const BUILTIN_EXTERNAL_DRIVER_IDS: &[&str] = &["duckdb"];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum NewConnectionCategory {
    All,
    Database,
    DomesticDatabase,
    NoSql,
    Terminal,
}

impl NewConnectionCategory {
    pub(super) fn all() -> [Self; 5] {
        [
            Self::All,
            Self::Database,
            Self::DomesticDatabase,
            Self::NoSql,
            Self::Terminal,
        ]
    }

    pub(super) fn label(self) -> String {
        match self {
            Self::All => t!("NewConnection.category_all").to_string(),
            Self::Database => t!("NewConnection.category_database").to_string(),
            Self::DomesticDatabase => t!("NewConnection.category_domestic_database").to_string(),
            Self::NoSql => "NoSQL".to_string(),
            Self::Terminal => t!("NewConnection.category_terminal").to_string(),
        }
    }

    pub(super) fn icon(self) -> IconName {
        match self {
            Self::All => IconName::AppsColor,
            Self::Database => IconName::Database,
            Self::DomesticDatabase => IconName::Database,
            Self::NoSql => IconName::Server,
            Self::Terminal => IconName::Terminal,
        }
    }
}

#[derive(Clone, PartialEq, Eq)]
pub(super) enum NewConnectionKind {
    Ssh,
    Terminal,
    Redis,
    MongoDB,
    Serial,
    Database(DatabaseType),
    ExternalDatabase {
        driver_id: String,
        name: String,
        description: String,
        category: Option<String>,
        icon_asset_path: Option<String>,
        icon_file_path: Option<PathBuf>,
    },
}

impl NewConnectionKind {
    pub(super) fn all() -> Vec<Self> {
        let mut items = vec![
            Self::Ssh,
            Self::Terminal,
            Self::Redis,
            Self::MongoDB,
            Self::Serial,
        ];
        items.extend(
            DatabaseType::builtin_all()
                .iter()
                .cloned()
                .map(Self::Database),
        );
        items.extend(external_database_kinds(&IpcDriverRegistry::load_default()));
        items
    }

    pub(super) fn label(&self) -> String {
        match self {
            Self::Ssh => "SSH / SFTP".to_string(),
            Self::Terminal => "Terminal".to_string(),
            Self::Redis => "Redis".to_string(),
            Self::MongoDB => "MongoDB".to_string(),
            Self::Serial => t!("Serial.new").to_string(),
            Self::Database(db_type) => db_type.as_str().to_string(),
            Self::ExternalDatabase { name, .. } => name.clone(),
        }
    }

    pub(super) fn description(&self) -> String {
        match self {
            Self::Ssh => t!("NewConnection.description_ssh").to_string(),
            Self::Terminal => t!("NewConnection.description_terminal").to_string(),
            Self::Redis => t!("NewConnection.description_redis").to_string(),
            Self::MongoDB => t!("NewConnection.description_mongodb").to_string(),
            Self::Serial => t!("NewConnection.description_serial").to_string(),
            Self::Database(_) => t!("NewConnection.description_database").to_string(),
            Self::ExternalDatabase { description, .. } => description.clone(),
        }
    }

    pub(super) fn category(&self) -> NewConnectionCategory {
        match self {
            Self::Ssh | Self::Terminal | Self::Serial => NewConnectionCategory::Terminal,
            Self::Redis | Self::MongoDB => NewConnectionCategory::NoSql,
            Self::Database(_) => NewConnectionCategory::Database,
            Self::ExternalDatabase { category, .. } => {
                if is_domestic_database_category(category.as_deref()) {
                    NewConnectionCategory::DomesticDatabase
                } else {
                    NewConnectionCategory::Database
                }
            }
        }
    }

    pub(super) fn icon(&self) -> Icon {
        match self {
            Self::Ssh => IconName::TerminalColor.color().with_size(px(40.0)),
            Self::Terminal => IconName::Terminal
                .mono()
                .text_color(gpui::rgb(0x8b5cf6))
                .with_size(px(40.0)),
            Self::Redis => IconName::Redis.color().with_size(px(40.0)),
            Self::MongoDB => IconName::MongoDB.color().with_size(px(40.0)),
            Self::Serial => IconName::SerialPort.color().with_size(px(40.0)),
            Self::Database(db_type) => db_type.as_icon().with_size(px(40.0)),
            Self::ExternalDatabase {
                icon_asset_path,
                icon_file_path,
                ..
            } => {
                if let Some(path) = icon_file_path {
                    return external_driver_icon_from_file_path(path, px(40.0));
                }
                icon_asset_path
                    .as_deref()
                    .map(|path| external_driver_icon_from_path(path, px(40.0)))
                    .unwrap_or_else(|| IconName::Database.color().with_size(px(40.0)))
            }
        }
    }
}

fn external_database_kinds(registry: &IpcDriverRegistry) -> Vec<NewConnectionKind> {
    registry
        .drivers()
        .iter()
        .filter(|driver| !is_builtin_external_driver(&driver.id))
        .map(|driver| {
            let icon_asset_path = driver.preferred_icon_asset_path();
            let icon_file_path = driver.preferred_icon_file_path();
            NewConnectionKind::ExternalDatabase {
                driver_id: driver.id.clone(),
                name: driver.name.clone(),
                description: driver.description.clone(),
                category: driver.category.clone(),
                icon_asset_path,
                icon_file_path,
            }
        })
        .collect()
}

fn is_builtin_external_driver(driver_id: &str) -> bool {
    BUILTIN_EXTERNAL_DRIVER_IDS.contains(&driver_id)
}

fn is_domestic_database_category(category: Option<&str>) -> bool {
    category == Some("domestic_database")
}

#[cfg(test)]
mod tests {
    use super::*;
    use db::ipc::{IpcDriverEntry, IpcDriverManifest, IpcDriverRegistry, IpcDriverTransport};
    use std::path::PathBuf;

    #[test]
    fn external_database_kinds_skip_builtin_duckdb_external_driver() {
        let registry = IpcDriverRegistry::from_drivers(vec![
            manifest("duckdb", "DuckDB"),
            manifest("custom", "Custom"),
        ]);

        let ids: Vec<String> = external_database_kinds(&registry)
            .into_iter()
            .filter_map(|kind| match kind {
                NewConnectionKind::ExternalDatabase { driver_id, .. } => Some(driver_id),
                _ => None,
            })
            .collect();

        assert_eq!(ids, vec!["custom"]);
    }

    #[test]
    fn connection_categories_include_domestic_database() {
        assert_eq!(
            NewConnectionCategory::all(),
            [
                NewConnectionCategory::All,
                NewConnectionCategory::Database,
                NewConnectionCategory::DomesticDatabase,
                NewConnectionCategory::NoSql,
                NewConnectionCategory::Terminal,
            ]
        );
        assert_eq!(
            t!("NewConnection.category_domestic_database").to_string(),
            NewConnectionCategory::DomesticDatabase.label()
        );
    }

    #[test]
    fn ipc_database_driver_uses_manifest_category_for_domestic_database() {
        let registry = IpcDriverRegistry::from_drivers(vec![
            manifest("dm", "Dameng DM"),
            manifest_with_category("kingbase", "KingbaseES", "domestic_database"),
            manifest_with_category("gbase8s", "GBase 8s", "domestic_database"),
            manifest("iotdb", "Apache IoTDB"),
        ]);

        let mut categories: Vec<(String, NewConnectionCategory)> =
            external_database_kinds(&registry)
                .into_iter()
                .filter_map(|kind| match kind {
                    NewConnectionKind::ExternalDatabase { ref driver_id, .. } => {
                        Some((driver_id.clone(), kind.category()))
                    }
                    _ => None,
                })
                .collect();
        categories.sort_by(|left, right| left.0.cmp(&right.0));

        assert_eq!(
            categories,
            vec![
                ("dm".to_string(), NewConnectionCategory::Database),
                (
                    "gbase8s".to_string(),
                    NewConnectionCategory::DomesticDatabase
                ),
                ("iotdb".to_string(), NewConnectionCategory::Database),
                (
                    "kingbase".to_string(),
                    NewConnectionCategory::DomesticDatabase
                ),
            ]
        );
    }

    #[test]
    fn external_database_kind_uses_manifest_icon() {
        let mut driver = manifest("custom", "Custom");
        driver.ui.icon = "icons/custom.svg".to_string();
        let registry = IpcDriverRegistry::from_drivers(vec![driver]);

        let icon_paths =
            external_database_kinds(&registry)
                .into_iter()
                .find_map(|kind| match kind {
                    NewConnectionKind::ExternalDatabase {
                        icon_asset_path,
                        icon_file_path,
                        ..
                    } => Some((icon_asset_path, icon_file_path)),
                    _ => None,
                });

        assert_eq!(
            Some((
                Some("driver://custom/icon.svg".to_string()),
                Some(PathBuf::from("./icons/custom.svg"))
            )),
            icon_paths
        );
    }

    fn manifest(id: &str, name: &str) -> IpcDriverManifest {
        manifest_with_optional_category(id, name, None)
    }

    fn manifest_with_category(id: &str, name: &str, category: &str) -> IpcDriverManifest {
        manifest_with_optional_category(id, name, Some(category.to_string()))
    }

    fn manifest_with_optional_category(
        id: &str,
        name: &str,
        category: Option<String>,
    ) -> IpcDriverManifest {
        IpcDriverManifest {
            id: id.to_string(),
            name: name.to_string(),
            description: String::new(),
            version: String::new(),
            entry: IpcDriverEntry {
                command: "./driver".to_string(),
                args: Vec::new(),
                working_dir: None,
            },
            transport: IpcDriverTransport::local_socket(format!("{id}.sock")),
            dialect: Default::default(),
            capabilities: None,
            connection: Default::default(),
            methods: Vec::new(),
            ui: Default::default(),
            category,
            manifest_dir: PathBuf::from("."),
        }
    }
}
