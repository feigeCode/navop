use crate::connection_visuals::{
    ConnectionVisualSize, connection_type_icon, database_type_icon,
    external_driver_icon_from_sources,
};
use db::ipc::IpcDriverRegistry;
use gpui_component::{Icon, IconName, Sizable};
use one_core::storage::{ConnectionType, DatabaseType};
use rust_i18n::t;
use std::path::PathBuf;

const BUILTIN_EXTERNAL_DRIVER_IDS: &[&str] = &["duckdb", "oracle-go"];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum NewConnectionCategory {
    All,
    Database,
    DomesticDatabase,
    TimeSeries,
    NoSql,
    Middleware,
    Terminal,
    Extensions,
}

impl NewConnectionCategory {
    pub(super) fn all() -> [Self; 8] {
        [
            Self::All,
            Self::Database,
            Self::DomesticDatabase,
            Self::TimeSeries,
            Self::NoSql,
            Self::Middleware,
            Self::Terminal,
            Self::Extensions,
        ]
    }

    pub(super) fn label(self) -> String {
        match self {
            Self::All => t!("NewConnection.category_all").to_string(),
            Self::Database => t!("NewConnection.category_database").to_string(),
            Self::DomesticDatabase => t!("NewConnection.category_domestic_database").to_string(),
            Self::TimeSeries => t!("NewConnection.category_time_series").to_string(),
            Self::NoSql => "NoSQL".to_string(),
            Self::Middleware => t!("NewConnection.category_middleware").to_string(),
            Self::Terminal => t!("NewConnection.category_terminal").to_string(),
            Self::Extensions => "Extensions".to_string(),
        }
    }

    pub(super) fn icon(self) -> IconName {
        match self {
            Self::All => IconName::LayoutDashboard,
            Self::Database | Self::DomesticDatabase => IconName::DatabaseLine,
            Self::TimeSeries => IconName::ChartPie,
            Self::NoSql => IconName::Server,
            Self::Middleware => IconName::Network,
            Self::Terminal => IconName::Terminal,
            Self::Extensions => IconName::ExtensionsLine,
        }
    }
}

#[derive(Clone, PartialEq, Eq)]
pub(super) enum NewConnectionKind {
    Ssh,
    Rdp,
    Vnc,
    Redis,
    MongoDB,
    Mqtt,
    Serial,
    Telnet,
    PortForwarding,
    MoreConnections,
    Database(DatabaseType),
    ExternalDatabase {
        driver_id: String,
        name: String,
        description: String,
        category: Option<String>,
        icon_asset_path: Option<String>,
        icon_file_path: Option<PathBuf>,
    },
    Extension(extension_runtime::RegisteredResourceConnectionContribution),
}

impl NewConnectionKind {
    pub(super) fn all_with_registry(registry: &IpcDriverRegistry) -> Vec<Self> {
        let mut items = vec![
            Self::Ssh,
            Self::Rdp,
            Self::Vnc,
            Self::Redis,
            Self::MongoDB,
            Self::Mqtt,
            Self::Serial,
            Self::Telnet,
            Self::PortForwarding,
        ];
        items.extend(
            DatabaseType::builtin_all()
                .iter()
                .cloned()
                .map(Self::Database),
        );
        items.extend(external_database_kinds(registry));
        items.push(Self::MoreConnections);
        items
    }

    pub(super) fn label(&self) -> String {
        match self {
            Self::Ssh => "SSH / SFTP".to_string(),
            Self::Rdp => "RDP".to_string(),
            Self::Vnc => "VNC".to_string(),
            Self::Redis => "Redis".to_string(),
            Self::MongoDB => "MongoDB".to_string(),
            Self::Mqtt => "MQTT".to_string(),
            Self::Serial => "Serial".to_string(),
            Self::Telnet => "Telnet".to_string(),
            Self::PortForwarding => t!("PortForwarding.new").to_string(),
            Self::MoreConnections => t!("NewConnection.more_connections").to_string(),
            Self::Database(db_type) => db_type.as_str().to_string(),
            Self::ExternalDatabase { name, .. } => name.clone(),
            Self::Extension(connection) => connection.label.clone(),
        }
    }

    pub(super) fn description(&self) -> String {
        match self {
            Self::Ssh => t!("NewConnection.description_ssh").to_string(),
            Self::Rdp => t!("NewConnection.description_rdp").to_string(),
            Self::Vnc => t!("NewConnection.description_vnc").to_string(),
            Self::Redis => t!("NewConnection.description_redis").to_string(),
            Self::MongoDB => t!("NewConnection.description_mongodb").to_string(),
            Self::Mqtt => t!("NewConnection.description_mqtt").to_string(),
            Self::Serial => t!("NewConnection.description_serial").to_string(),
            Self::Telnet => t!("NewConnection.description_telnet").to_string(),
            Self::PortForwarding => t!("NewConnection.description_port_forwarding").to_string(),
            Self::MoreConnections => t!("NewConnection.description_more_connections").to_string(),
            Self::Database(_) => t!("NewConnection.description_database").to_string(),
            Self::ExternalDatabase { description, .. } => description.clone(),
            Self::Extension(connection) => connection.description.clone().unwrap_or_default(),
        }
    }

    pub(super) fn category(&self) -> NewConnectionCategory {
        match self {
            Self::Ssh
            | Self::Rdp
            | Self::Vnc
            | Self::Serial
            | Self::Telnet
            | Self::PortForwarding => NewConnectionCategory::Terminal,
            Self::MoreConnections => NewConnectionCategory::All,
            Self::Redis | Self::MongoDB => NewConnectionCategory::NoSql,
            Self::Mqtt => NewConnectionCategory::Middleware,
            Self::Database(DatabaseType::TDengine) => NewConnectionCategory::TimeSeries,
            Self::Database(_) => NewConnectionCategory::Database,
            Self::ExternalDatabase { category, .. } => {
                if is_domestic_database_category(category.as_deref()) {
                    NewConnectionCategory::DomesticDatabase
                } else {
                    NewConnectionCategory::Database
                }
            }
            Self::Extension(_) => NewConnectionCategory::Extensions,
        }
    }

    pub(super) fn icon(&self) -> Icon {
        match self {
            Self::Ssh => connection_type_icon(ConnectionType::SshSftp, ConnectionVisualSize::Hero),
            Self::Rdp => connection_type_icon(ConnectionType::Rdp, ConnectionVisualSize::Hero),
            Self::Vnc => connection_type_icon(ConnectionType::Vnc, ConnectionVisualSize::Hero),
            Self::Redis => connection_type_icon(ConnectionType::Redis, ConnectionVisualSize::Hero),
            Self::MongoDB => {
                connection_type_icon(ConnectionType::MongoDB, ConnectionVisualSize::Hero)
            }
            Self::Mqtt => connection_type_icon(ConnectionType::Mqtt, ConnectionVisualSize::Hero),
            Self::Serial => {
                connection_type_icon(ConnectionType::Serial, ConnectionVisualSize::Hero)
            }
            Self::Telnet => {
                connection_type_icon(ConnectionType::Telnet, ConnectionVisualSize::Hero)
            }
            Self::PortForwarding => {
                connection_type_icon(ConnectionType::PortForwarding, ConnectionVisualSize::Hero)
            }
            Self::MoreConnections => IconName::Plus
                .mono()
                .with_size(ConnectionVisualSize::Hero.icon_size()),
            Self::Database(db_type) => database_type_icon(db_type, ConnectionVisualSize::Hero),
            Self::ExternalDatabase {
                icon_asset_path,
                icon_file_path,
                ..
            } => external_driver_icon_from_sources(
                icon_asset_path.as_deref(),
                icon_file_path.as_deref(),
                ConnectionVisualSize::Hero,
            )
            .unwrap_or_else(|| {
                connection_type_icon(ConnectionType::Database, ConnectionVisualSize::Hero)
            }),
            Self::Extension(connection) => external_driver_icon_from_sources(
                None,
                connection.icon_path.as_deref(),
                ConnectionVisualSize::Hero,
            )
            .unwrap_or_else(|| {
                connection_type_icon(ConnectionType::Extension, ConnectionVisualSize::Hero)
            }),
        }
    }
}

fn external_database_kinds(registry: &IpcDriverRegistry) -> Vec<NewConnectionKind> {
    registry
        .drivers()
        .iter()
        .filter(|driver| driver.ui.show_in_new_connection)
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
    fn external_database_kinds_skip_builtin_external_drivers() {
        let registry = IpcDriverRegistry::from_drivers(vec![
            manifest("duckdb", "DuckDB"),
            manifest("oracle-go", "Oracle Go"),
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
    fn external_database_kinds_respect_manifest_visibility() {
        let hidden: IpcDriverManifest = serde_json::from_value(serde_json::json!({
            "id": "redis",
            "name": "Redis",
            "api": "redis",
            "entry": { "command": "./redis-driver" },
            "transport": { "name": "redis.sock" },
            "ui": { "show_in_new_connection": false }
        }))
        .unwrap();
        let registry = IpcDriverRegistry::from_drivers(vec![hidden, manifest("custom", "Custom")]);

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
                NewConnectionCategory::TimeSeries,
                NewConnectionCategory::NoSql,
                NewConnectionCategory::Middleware,
                NewConnectionCategory::Terminal,
                NewConnectionCategory::Extensions,
            ]
        );
        assert_eq!(
            t!("NewConnection.category_domestic_database").to_string(),
            NewConnectionCategory::DomesticDatabase.label()
        );
    }

    #[test]
    fn tdengine_and_mqtt_kinds_map_to_their_categories() {
        let registry = IpcDriverRegistry::empty();
        let kinds = NewConnectionKind::all_with_registry(&registry);

        assert_eq!(
            NewConnectionKind::Database(DatabaseType::TDengine).category(),
            NewConnectionCategory::TimeSeries
        );
        assert_eq!(
            NewConnectionKind::Database(DatabaseType::MySQL).category(),
            NewConnectionCategory::Database
        );
        assert_eq!(
            NewConnectionKind::Mqtt.category(),
            NewConnectionCategory::Middleware
        );
        assert!(kinds.contains(&NewConnectionKind::Mqtt));
        assert!(kinds.contains(&NewConnectionKind::Database(DatabaseType::TDengine)));
        assert_eq!(
            t!("NewConnection.category_time_series").to_string(),
            NewConnectionCategory::TimeSeries.label()
        );
        assert_eq!(
            t!("NewConnection.category_middleware").to_string(),
            NewConnectionCategory::Middleware.label()
        );
    }

    #[test]
    fn remote_desktop_kinds_are_available_from_new_connection() {
        let registry = IpcDriverRegistry::empty();
        let kinds = NewConnectionKind::all_with_registry(&registry);
        assert!(kinds.contains(&NewConnectionKind::Rdp));
        assert!(kinds.contains(&NewConnectionKind::Vnc));
        assert_eq!(
            NewConnectionKind::Rdp.category(),
            NewConnectionCategory::Terminal
        );
        assert_eq!(
            NewConnectionKind::Vnc.category(),
            NewConnectionCategory::Terminal
        );
    }

    #[test]
    fn local_terminal_is_not_a_new_connection_kind() {
        let registry = IpcDriverRegistry::empty();
        let labels = NewConnectionKind::all_with_registry(&registry)
            .into_iter()
            .map(|kind| kind.label())
            .collect::<Vec<_>>();

        assert!(!labels.iter().any(|label| label == "Terminal"));
    }

    #[test]
    fn more_connections_kind_is_last_and_only_visible_in_all() {
        let registry = IpcDriverRegistry::empty();
        let kinds = NewConnectionKind::all_with_registry(&registry);

        assert!(matches!(
            kinds.last(),
            Some(NewConnectionKind::MoreConnections)
        ));
        assert_eq!(
            NewConnectionKind::MoreConnections.category(),
            NewConnectionCategory::All
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
            api: "database".into(),
            description: String::new(),
            version: String::new(),
            engines: Default::default(),
            compatibility: serde_json::Value::Null,
            entry: IpcDriverEntry {
                command: "./driver".to_string(),
                commands: Default::default(),
                args: Vec::new(),
                working_dir: None,
                env_from_config: Default::default(),
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
