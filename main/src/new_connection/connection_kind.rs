use crate::connection_visuals::{
    ConnectionVisualSize, connection_type_icon, database_type_icon,
    external_driver_icon_from_sources,
};
use db::ipc::IpcDriverRegistry;
use gpui_component::{Icon, Sizable};
use one_assets::IconName;
use one_core::storage::{ConnectionType, DatabaseType};
use rust_i18n::t;
use std::path::PathBuf;

const BUILTIN_EXTERNAL_DRIVER_IDS: &[&str] = &["duckdb", "oracle-go"];

/// 中间件扩展贡献的统一 id 前缀(如 com.navop.middleware.mqtt)
const MIDDLEWARE_EXTENSION_ID_PREFIX: &str = "com.navop.middleware.";

/// TDengine 无原生实现,统一经此 driver_id 的 IPC 外部驱动连接
const TDENGINE_DRIVER_ID: &str = "tdengine";

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
    Serial,
    Telnet,
    PortForwarding,
    MoreConnections,
    /// 空类目的「+」安装入口:对应类目下没有任何成员时展示,点击跳转扩展管理页
    InstallCategoryExtensions(NewConnectionCategory),
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
            Self::Serial,
            Self::Telnet,
            Self::PortForwarding,
        ];
        items.extend(
            DatabaseType::builtin_all()
                .iter()
                // TDengine 无原生实现:仅当外部驱动已安装时才出现在新建入口清单
                .filter(|db_type| {
                    !matches!(db_type, DatabaseType::TDengine)
                        || registry.find(TDENGINE_DRIVER_ID).is_some()
                })
                .cloned()
                .map(Self::Database),
        );
        items.extend(external_database_kinds(registry));
        items.push(Self::MoreConnections);
        items
    }

    /// 为没有任何成员的类目追加「+」安装入口。
    ///
    /// 需在 Extension 贡献项全部追加之后调用(中间件类目依赖
    /// com.navop.middleware. 前缀的扩展贡献分流),且需在 MoreConnections
    /// 压回末尾之前调用,保证「+」项排在所属类目分组的末尾。
    pub(super) fn append_empty_category_install_entries(items: &mut Vec<Self>) {
        // 仅这两类依赖外部扩展/驱动填充,内置类目永远有成员,无需空态入口
        for category in [
            NewConnectionCategory::Middleware,
            NewConnectionCategory::TimeSeries,
        ] {
            let has_member = items.iter().any(|kind| kind.category() == category);
            if !has_member {
                items.push(Self::InstallCategoryExtensions(category));
            }
        }
    }

    /// 点击后是否直接跳转扩展管理页(MoreConnections 与空类目「+」安装入口)
    pub(super) fn opens_extensions_tab_on_click(&self) -> bool {
        matches!(
            self,
            Self::MoreConnections | Self::InstallCategoryExtensions(_)
        )
    }

    pub(super) fn label(&self) -> String {
        match self {
            Self::Ssh => "SSH / SFTP".to_string(),
            Self::Rdp => "RDP".to_string(),
            Self::Vnc => "VNC".to_string(),
            Self::Redis => "Redis".to_string(),
            Self::MongoDB => "MongoDB".to_string(),
            Self::Serial => "Serial".to_string(),
            Self::Telnet => "Telnet".to_string(),
            Self::PortForwarding => t!("PortForwarding.new").to_string(),
            Self::MoreConnections => t!("NewConnection.more_connections").to_string(),
            Self::InstallCategoryExtensions(NewConnectionCategory::Middleware) => {
                t!("NewConnection.more_middleware").to_string()
            }
            Self::InstallCategoryExtensions(NewConnectionCategory::TimeSeries) => {
                t!("NewConnection.more_time_series").to_string()
            }
            // 其余类目暂无专属文案,回退到通用的「更多连接」
            Self::InstallCategoryExtensions(_) => t!("NewConnection.more_connections").to_string(),
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
            Self::Serial => t!("NewConnection.description_serial").to_string(),
            Self::Telnet => t!("NewConnection.description_telnet").to_string(),
            Self::PortForwarding => t!("NewConnection.description_port_forwarding").to_string(),
            Self::MoreConnections => t!("NewConnection.description_more_connections").to_string(),
            // 「+」安装入口与「更多连接」语义一致:引导前往扩展市场
            Self::InstallCategoryExtensions(_) => {
                t!("NewConnection.description_more_connections").to_string()
            }
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
            // 「+」安装入口归属于其目标类目,侧栏选中该类目时可见
            Self::InstallCategoryExtensions(category) => *category,
            Self::Redis | Self::MongoDB => NewConnectionCategory::NoSql,
            Self::Database(DatabaseType::TDengine) => NewConnectionCategory::TimeSeries,
            Self::Database(_) => NewConnectionCategory::Database,
            Self::ExternalDatabase { category, .. } => {
                if is_domestic_database_category(category.as_deref()) {
                    NewConnectionCategory::DomesticDatabase
                } else {
                    // TODO: 驱动 manifest 的 category 目前仅约定 "domestic_database" 一种取值,
                    // 尚无时序类标识(如 "time_series");待驱动侧补充约定后再在此映射
                    // NewConnectionCategory::TimeSeries,避免臆造字段值。
                    NewConnectionCategory::Database
                }
            }
            Self::Extension(contribution) => {
                // 以 com.navop.middleware. 为前缀的扩展贡献归入中间件类目,其余留在扩展类目
                if contribution
                    .extension_id
                    .starts_with(MIDDLEWARE_EXTENSION_ID_PREFIX)
                {
                    NewConnectionCategory::Middleware
                } else {
                    NewConnectionCategory::Extensions
                }
            }
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
            // 「+」安装入口形态与「更多连接」一致:Plus 图标
            Self::InstallCategoryExtensions(_) => IconName::Plus
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
        assert_eq!(
            t!("NewConnection.category_middleware").to_string(),
            NewConnectionCategory::Middleware.label()
        );
    }

    #[test]
    fn tdengine_kind_maps_to_time_series_category() {
        // 类目映射本身与驱动是否安装无关
        assert_eq!(
            NewConnectionKind::Database(DatabaseType::TDengine).category(),
            NewConnectionCategory::TimeSeries
        );
        assert_eq!(
            NewConnectionKind::Database(DatabaseType::MySQL).category(),
            NewConnectionCategory::Database
        );
        assert_eq!(
            t!("NewConnection.category_time_series").to_string(),
            NewConnectionCategory::TimeSeries.label()
        );
    }

    #[test]
    fn tdengine_kind_hidden_without_driver_and_install_entry_shown() {
        // 未安装 tdengine 外部驱动时:新建入口不出现 TDengine,时序类目展示「+」安装入口
        let registry = IpcDriverRegistry::empty();
        let mut kinds = NewConnectionKind::all_with_registry(&registry);
        NewConnectionKind::append_empty_category_install_entries(&mut kinds);

        assert!(!kinds.contains(&NewConnectionKind::Database(DatabaseType::TDengine)));
        let install_entry =
            NewConnectionKind::InstallCategoryExtensions(NewConnectionCategory::TimeSeries);
        assert!(kinds.contains(&install_entry));
        assert_eq!(install_entry.category(), NewConnectionCategory::TimeSeries);
        assert!(install_entry.opens_extensions_tab_on_click());
        assert_eq!(
            t!("NewConnection.more_time_series").to_string(),
            install_entry.label()
        );
    }

    #[test]
    fn tdengine_kind_shown_with_driver_and_install_entry_hidden() {
        // 安装 tdengine 外部驱动后:新建入口出现 TDengine,时序类目不再展示「+」安装入口
        let registry =
            IpcDriverRegistry::from_drivers(vec![manifest(TDENGINE_DRIVER_ID, "TDengine")]);
        let mut kinds = NewConnectionKind::all_with_registry(&registry);
        NewConnectionKind::append_empty_category_install_entries(&mut kinds);

        assert!(kinds.contains(&NewConnectionKind::Database(DatabaseType::TDengine)));
        assert!(
            !kinds.contains(&NewConnectionKind::InstallCategoryExtensions(
                NewConnectionCategory::TimeSeries
            ))
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
    fn middleware_extension_contribution_routes_to_middleware_category() {
        // com.navop.middleware. 前缀的贡献归入中间件类目,其余归入扩展类目
        let mqtt = NewConnectionKind::Extension(contribution("com.navop.middleware.mqtt"));
        let rocketmq = NewConnectionKind::Extension(contribution("com.navop.middleware.rocketmq"));
        let other = NewConnectionKind::Extension(contribution("com.navop.other.tool"));

        assert_eq!(mqtt.category(), NewConnectionCategory::Middleware);
        assert_eq!(rocketmq.category(), NewConnectionCategory::Middleware);
        assert_eq!(other.category(), NewConnectionCategory::Extensions);
    }

    #[test]
    fn middleware_install_entry_follows_category_membership() {
        // 空中间件类目:展示「+」安装入口
        let mut kinds = NewConnectionKind::all_with_registry(&IpcDriverRegistry::empty());
        NewConnectionKind::append_empty_category_install_entries(&mut kinds);

        let install_entry =
            NewConnectionKind::InstallCategoryExtensions(NewConnectionCategory::Middleware);
        assert!(kinds.contains(&install_entry));
        assert_eq!(install_entry.category(), NewConnectionCategory::Middleware);
        assert!(install_entry.opens_extensions_tab_on_click());
        assert_eq!(
            t!("NewConnection.more_middleware").to_string(),
            install_entry.label()
        );

        // 有中间件扩展贡献时不补「+」;模拟真实调用序列:
        // MoreConnections 先 pop → 空类目补「+」 → MoreConnections 压回末尾
        let mut kinds_with_middleware = vec![NewConnectionKind::Extension(contribution(
            "com.navop.middleware.mqtt",
        ))];
        NewConnectionKind::append_empty_category_install_entries(&mut kinds_with_middleware);
        kinds_with_middleware.push(NewConnectionKind::MoreConnections);

        assert!(!kinds_with_middleware.contains(&install_entry));
        assert!(matches!(
            kinds_with_middleware.get(1),
            Some(NewConnectionKind::InstallCategoryExtensions(
                NewConnectionCategory::TimeSeries
            ))
        ));
        assert!(matches!(
            kinds_with_middleware.last(),
            Some(NewConnectionKind::MoreConnections)
        ));

        // 中间件与时序类目均有成员时,不补任何「+」入口
        let mut kinds_full = vec![
            NewConnectionKind::Extension(contribution("com.navop.middleware.mqtt")),
            NewConnectionKind::Database(DatabaseType::TDengine),
            NewConnectionKind::MoreConnections,
        ];
        NewConnectionKind::append_empty_category_install_entries(&mut kinds_full);

        assert_eq!(kinds_full.len(), 3);
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

    /// 构造扩展贡献项(仅 extension_id 参与类目分流,其余字段用空值)
    fn contribution(
        extension_id: &str,
    ) -> extension_runtime::RegisteredResourceConnectionContribution {
        extension_runtime::RegisteredResourceConnectionContribution {
            extension_id: extension_id.to_string(),
            extension_root: PathBuf::from("."),
            id: format!("{extension_id}:connection"),
            label: "Demo".to_string(),
            description: None,
            icon_path: None,
            runtime_id: String::new(),
            resource_type: String::new(),
            shell_view_id: None,
            form: Default::default(),
        }
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
