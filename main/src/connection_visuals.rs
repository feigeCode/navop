use std::path::Path;

use gpui::App;

use db::ipc::{IpcDriverRegistry, driver_icon_from_asset_path, driver_icon_from_file_path};
use gpui_component::{Icon, Sizable};
use one_assets::IconName;
use one_core::storage::{
    ConnectionType, DatabaseType, DbConnectionConfig, SshParams, StoredConnection,
};
use one_ui::IconSize;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ExternalDriverIconSource<'a> {
    File(&'a Path),
    Asset(&'a str),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SshIconSource<'a> {
    File(&'a Path),
    BuiltIn(IconName),
}

/// Semantic icon sizes for connection identity surfaces.
///
/// The visual size is intentionally independent from the surrounding control
/// hit target or container safe area.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ConnectionVisualSize {
    Tree,
    List,
    Card,
    Hero,
}

impl ConnectionVisualSize {
    pub(crate) const fn icon_size(self) -> IconSize {
        match self {
            Self::Tree => IconSize::Default,
            Self::List => IconSize::Large,
            Self::Card => IconSize::Large,
            Self::Hero => IconSize::Hero,
        }
    }
}

const fn connection_type_icon_name(kind: ConnectionType) -> IconName {
    match kind {
        ConnectionType::All => IconName::Server,
        ConnectionType::Database => IconName::Database,
        ConnectionType::SshSftp => IconName::TerminalColor,
        ConnectionType::Ftp => IconName::FtpColor,
        ConnectionType::Redis => IconName::Redis,
        ConnectionType::MongoDB => IconName::MongoDB,
        // 品牌 SVG 图标,见 connection_type_icon 的特判分支
        ConnectionType::Mqtt => IconName::Network,
        ConnectionType::Serial => IconName::SerialPort,
        ConnectionType::Telnet => IconName::SquareTerminalColor,
        ConnectionType::PortForwarding => IconName::PortForwardingColor,
        ConnectionType::Rdp => IconName::Rdp,
        ConnectionType::Vnc => IconName::Vnc,
        ConnectionType::Extension => IconName::ExtensionsColor,
    }
}

const fn connection_type_navigation_icon_name(kind: ConnectionType) -> IconName {
    match kind {
        ConnectionType::All => IconName::Asterisk,
        ConnectionType::Database => IconName::DatabaseLine,
        ConnectionType::SshSftp => IconName::TerminalLine,
        ConnectionType::Ftp => IconName::Network,
        ConnectionType::Redis => IconName::RedisLine,
        ConnectionType::MongoDB => IconName::MongoDBLine,
        // 品牌 SVG 图标,见 connection_type_navigation_icon 的特判分支
        ConnectionType::Mqtt => IconName::Network,
        ConnectionType::Serial => IconName::SerialLine,
        ConnectionType::Telnet => IconName::SquareTerminal,
        ConnectionType::PortForwarding => IconName::PortForwardingLine,
        ConnectionType::Rdp => IconName::RdpLine,
        ConnectionType::Vnc => IconName::VncLine,
        ConnectionType::Extension => IconName::ExtensionsLine,
    }
}

/// Monochrome line icon used by navigation and filtering surfaces.
pub(crate) fn connection_type_navigation_icon(
    kind: ConnectionType,
    size: ConnectionVisualSize,
) -> Icon {
    // MQTT 品牌线条图标经应用 AssetSource 提供
    if kind == ConnectionType::Mqtt {
        return Icon::default()
            .path(one_core::storage::NAVOP_MQTT_LINE_ICON)
            .mono()
            .with_size(size.icon_size());
    }
    connection_type_navigation_icon_name(kind)
        .mono()
        .with_size(size.icon_size())
}

/// English connection-type label，与「连接类型筛选」菜单一致（core 的 `label()` 为准）。
pub(crate) fn connection_type_label(kind: ConnectionType) -> String {
    kind.label().to_string()
}

/// 连接类型显示名：扩展连接展开为其扩展贡献显示名（真实类型），其余用内置英文名。
/// 与「连接类型筛选」一致——筛选菜单也按扩展贡献逐项列出真实类型。
pub(crate) fn connection_type_display_label(connection: &StoredConnection, cx: &App) -> String {
    if connection.connection_type == ConnectionType::Extension {
        if let Some(label) = extension_connection_label(connection, cx) {
            return label;
        }
    }
    connection_type_label(connection.connection_type)
}

/// 从全局扩展目录读取 (extension_id, contribution_id) 对应的扩展贡献显示名。
fn extension_connection_label(connection: &StoredConnection, cx: &App) -> Option<String> {
    let params = connection.to_extension_params().ok()?;
    let catalog = extension_catalog_from_cx(cx)?;
    catalog
        .resource_connection(&params.extension_id, &params.contribution_id)
        .map(|contribution| contribution.label.clone())
}

/// Original-color protocol identity icon used by cards, lists, and connection pickers.
pub(crate) fn connection_type_icon(kind: ConnectionType, size: ConnectionVisualSize) -> Icon {
    // MQTT 品牌图标经应用 AssetSource 提供(外部 IconName 无此变体)
    if kind == ConnectionType::Mqtt {
        return Icon::default()
            .path(one_core::storage::NAVOP_MQTT_COLOR_ICON)
            .color()
            .with_size(size.icon_size());
    }
    connection_type_icon_name(kind)
        .color()
        .with_size(size.icon_size())
}

pub(crate) fn database_type_icon(kind: &DatabaseType, size: ConnectionVisualSize) -> Icon {
    // TDengine 品牌图标经应用 AssetSource 提供(外部 IconName 无此变体)
    if matches!(kind, DatabaseType::TDengine) {
        return Icon::default()
            .path(one_core::storage::NAVOP_TDENGINE_COLOR_ICON)
            .color()
            .with_size(size.icon_size());
    }
    let name = match kind {
        DatabaseType::MySQL => IconName::MySQLColor,
        DatabaseType::PostgreSQL => IconName::PostgreSQLColor,
        DatabaseType::SQLite => IconName::SQLiteColor,
        DatabaseType::DuckDB => IconName::DuckDB,
        DatabaseType::MSSQL => IconName::MSSQLColor,
        DatabaseType::Oracle => IconName::OracleColor,
        DatabaseType::ClickHouse => IconName::ClickHouseColor,
        // 上方提前返回,此处仅为穷尽匹配
        DatabaseType::TDengine => return generic_database_icon(size),
        DatabaseType::External { .. } => return generic_database_icon(size),
    };
    name.color().with_size(size.icon_size())
}

pub(crate) fn database_config_icon(
    config: &DbConnectionConfig,
    size: ConnectionVisualSize,
    registry: &IpcDriverRegistry,
) -> Icon {
    external_driver_icon_for_config_with_registry(config, size, registry)
        .unwrap_or_else(|| database_type_icon(&config.database_type, size))
}

/// Resolve the icon for a stored connection. Extension connections
/// (`ConnectionType::Extension`) additionally resolve
/// `contributes.connections[].icon` through the extension runtime catalog when
/// one is available; other kinds use built-in or driver-provided icons.
pub(crate) fn stored_connection_icon_with_catalog(
    connection: &StoredConnection,
    size: ConnectionVisualSize,
    registry: &IpcDriverRegistry,
    extension_catalog: Option<&extension_runtime::ExtensionRuntimeCatalog>,
) -> Icon {
    match connection.connection_type {
        ConnectionType::Database => connection
            .to_db_connection()
            .map(|config| database_config_icon(&config, size, registry))
            .unwrap_or_else(|_| generic_database_icon(size)),
        ConnectionType::SshSftp => connection
            .to_ssh_params()
            .map(|params| ssh_icon(&params, size))
            .unwrap_or_else(|_| color_icon(IconName::LinuxPenguinColor, size)),
        ConnectionType::Extension => extension_connection_icon(connection, size, extension_catalog)
            .unwrap_or_else(|| connection_type_icon(ConnectionType::Extension, size)),
        kind => connection_type_icon(kind, size),
    }
}

/// 扩展连接图标：按 (extension_id, contribution_id) 从扩展目录取
/// `contributes.connections[].icon` 指向的 SVG 文件渲染。任一环节缺失
/// （非扩展连接、目录不可用、贡献不存在、未声明图标）都返回 None，
/// 由调用方回退到通用扩展图标。
fn extension_connection_icon(
    connection: &StoredConnection,
    size: ConnectionVisualSize,
    extension_catalog: Option<&extension_runtime::ExtensionRuntimeCatalog>,
) -> Option<Icon> {
    let params = connection.to_extension_params().ok()?;
    let contribution = extension_catalog?.resource_connection(
        &params.extension_id,
        &params.contribution_id,
    )?;
    let icon_path = contribution.icon_path.as_ref()?;
    Some(driver_icon_from_file_path(icon_path.clone(), size.icon_size()))
}

/// 便捷读取全局扩展运行时目录（与 connection_forms / connection_type_menu 的
/// 取用方式保持一致）。
pub(crate) fn extension_catalog_from_cx(
    cx: &App,
) -> Option<std::sync::Arc<extension_runtime::ExtensionRuntimeCatalog>> {
    cx.try_global::<extension_runtime::GlobalExtensionRuntimeCatalog>()
        .and_then(|global| global.get())
}

fn ssh_icon(params: &SshParams, size: ConnectionVisualSize) -> Icon {
    match ssh_icon_source(params) {
        SshIconSource::File(path) => {
            driver_icon_from_file_path(path.to_path_buf(), size.icon_size())
        }
        SshIconSource::BuiltIn(name) => color_icon(name, size),
    }
}

fn ssh_icon_source(params: &SshParams) -> SshIconSource<'_> {
    params
        .icon_file_path
        .as_deref()
        .map(Path::new)
        .filter(|path| path.is_file())
        .map(SshIconSource::File)
        .unwrap_or_else(|| SshIconSource::BuiltIn(params.os_icon()))
}

pub(crate) fn external_driver_icon_for_config_with_registry(
    config: &DbConnectionConfig,
    size: ConnectionVisualSize,
    registry: &IpcDriverRegistry,
) -> Option<Icon> {
    let display = registry.display_for_config(config)?;
    external_driver_icon_from_sources(
        display.icon_asset_path.as_deref(),
        display.icon_file_path.as_deref(),
        size,
    )
}

/// Resolves external driver visuals with the host-authoritative precedence:
/// filesystem path, bundled asset path, then caller-provided fallback.
pub(crate) fn external_driver_icon_from_sources(
    icon_asset_path: Option<&str>,
    icon_file_path: Option<&Path>,
    size: ConnectionVisualSize,
) -> Option<Icon> {
    match external_driver_icon_source(icon_asset_path, icon_file_path)? {
        ExternalDriverIconSource::File(path) => Some(driver_icon_from_file_path(
            path.to_path_buf(),
            size.icon_size(),
        )),
        ExternalDriverIconSource::Asset(path) => Some(driver_icon_from_asset_path(
            path.to_string(),
            size.icon_size(),
        )),
    }
}

fn generic_database_icon(size: ConnectionVisualSize) -> Icon {
    IconName::Database.color().with_size(size.icon_size())
}

fn color_icon(name: IconName, size: ConnectionVisualSize) -> Icon {
    name.color().with_size(size.icon_size())
}

fn external_driver_icon_source<'a>(
    icon_asset_path: Option<&'a str>,
    icon_file_path: Option<&'a Path>,
) -> Option<ExternalDriverIconSource<'a>> {
    icon_file_path
        .map(ExternalDriverIconSource::File)
        .or_else(|| icon_asset_path.map(ExternalDriverIconSource::Asset))
}

#[cfg(test)]
mod tests {
    use super::*;
    use one_core::storage::{SshAccountExpect, SshAuthMethod};
    use tempfile::tempdir;

    #[test]
    fn semantic_connection_sizes_map_to_the_shared_icon_scale() {
        assert_eq!(ConnectionVisualSize::Tree.icon_size(), IconSize::Default);
        assert_eq!(ConnectionVisualSize::List.icon_size(), IconSize::Large);
        assert_eq!(ConnectionVisualSize::Card.icon_size(), IconSize::Large);
        assert_eq!(ConnectionVisualSize::Hero.icon_size(), IconSize::Hero);
    }

    #[test]
    fn connection_types_map_directly_to_original_color_assets() {
        let expected = [
            (ConnectionType::All, IconName::Server),
            (ConnectionType::Database, IconName::Database),
            (ConnectionType::SshSftp, IconName::TerminalColor),
            (ConnectionType::Redis, IconName::Redis),
            (ConnectionType::MongoDB, IconName::MongoDB),
            (ConnectionType::Serial, IconName::SerialPort),
            (ConnectionType::Telnet, IconName::SquareTerminalColor),
            (
                ConnectionType::PortForwarding,
                IconName::PortForwardingColor,
            ),
            (ConnectionType::Rdp, IconName::Rdp),
            (ConnectionType::Vnc, IconName::Vnc),
        ];

        for (connection_type, icon_name) in expected {
            assert_eq!(connection_type_icon_name(connection_type), icon_name);
        }
    }

    #[test]
    fn connection_navigation_icons_map_to_monochrome_line_assets() {
        let expected = [
            (ConnectionType::All, IconName::Asterisk),
            (ConnectionType::Database, IconName::DatabaseLine),
            (ConnectionType::SshSftp, IconName::TerminalLine),
            (ConnectionType::Redis, IconName::RedisLine),
            (ConnectionType::MongoDB, IconName::MongoDBLine),
            (ConnectionType::Serial, IconName::SerialLine),
            (ConnectionType::Telnet, IconName::SquareTerminal),
            (ConnectionType::PortForwarding, IconName::PortForwardingLine),
            (ConnectionType::Rdp, IconName::RdpLine),
            (ConnectionType::Vnc, IconName::VncLine),
        ];

        for (connection_type, icon_name) in expected {
            assert_eq!(
                connection_type_navigation_icon_name(connection_type),
                icon_name
            );
        }
    }

    #[test]
    fn external_driver_file_icon_takes_precedence_over_asset_icon() {
        let file = Path::new("/tmp/navop-driver-icon.svg");
        assert_eq!(
            external_driver_icon_source(Some("icons/driver.svg"), Some(file)),
            Some(ExternalDriverIconSource::File(file))
        );
        assert_eq!(
            external_driver_icon_source(Some("icons/driver.svg"), None),
            Some(ExternalDriverIconSource::Asset("icons/driver.svg"))
        );
        assert_eq!(external_driver_icon_source(None, None), None);
    }

    #[test]
    fn ssh_file_icon_takes_precedence_and_missing_file_falls_back() {
        let directory = tempdir().expect("temporary directory should be created");
        let icon_path = directory.path().join("custom.svg");
        std::fs::write(&icon_path, "<svg/>").expect("temporary icon should be written");
        let mut params = ssh_params();
        params.icon = Some("ubuntu".to_string());
        params.icon_file_path = Some(icon_path.to_string_lossy().into_owned());

        assert_eq!(
            ssh_icon_source(&params),
            SshIconSource::File(icon_path.as_path())
        );

        params.icon_file_path = Some(directory.path().join("missing.svg").display().to_string());
        assert_eq!(
            ssh_icon_source(&params),
            SshIconSource::BuiltIn(IconName::UbuntuColor)
        );
    }

    fn ssh_params() -> SshParams {
        SshParams {
                remote_file: None,
            sftp_default_directory: None,
            disabled_jump_server: None,
            sftp_account: None,
            host: "example.com".to_string(),
            port: 22,
            username: "root".to_string(),
            auth_method: SshAuthMethod::Agent,
            credential_reference: None,
            prompt_username: None,
            prompt_password: None,
            keyboard_interactive: None,
            terminal_encoding: Default::default(),
            terminal_type: Default::default(),
            connect_timeout: None,
            keepalive_interval: None,
            keepalive_max: None,
            default_directory: None,
            init_script: None,
            disable_shell_integration: None,
            x11_forwarding: None,
            allow_legacy_algorithms: None,
            jump_server: None,
            proxy: None,
            os_id: None,
            icon: None,
            icon_file_path: None,
            account_expect: SshAccountExpect::default(),
        }
    }

    fn extension_connection() -> StoredConnection {
        let params = one_core::storage::ExtensionConnectionParams::new(
            "com.example.demo",
            "demo",
            serde_json::Map::new(),
            Default::default(),
        )
        .expect("extension params should validate");
        StoredConnection::new_extension("Demo".to_string(), params, None)
    }

    #[test]
    fn extension_connection_without_catalog_falls_back() {
        let connection = extension_connection();
        // 目录不可用时不 panic，交由调用方回退通用扩展图标
        assert!(extension_connection_icon(&connection, ConnectionVisualSize::List, None).is_none());
    }

    #[test]
    fn extension_connection_resolves_icon_from_catalog() {
        let directory = tempdir().expect("temporary directory should be created");
        let icons = directory.path().join("icons");
        std::fs::create_dir_all(&icons).expect("icons directory should be created");
        std::fs::write(
            icons.join("demo-color.svg"),
            "<svg xmlns=\"http://www.w3.org/2000/svg\"/>",
        )
        .expect("temporary icon should be written");
        std::fs::write(
            directory.path().join("extension.json"),
            r#"{
                "schema_version": 1,
                "id": "com.example.demo",
                "name": "Demo",
                "version": "0.1.0",
                "engines": { "onetcli": ">=0.4.0" },
                "permissions": ["shell:exec", "spawn:./bin/provider"],
                "runtime": {
                    "ipc": [{
                        "id": "main",
                        "entry": { "command": "./bin/provider" },
                        "transport": { "kind": "local_socket" }
                    }]
                },
                "contributes": {
                    "connections": [{
                        "id": "demo",
                        "label": "Demo",
                        "icon": "icons/demo-color.svg",
                        "runtimeId": "main",
                        "resourceType": "demo"
                    }]
                }
            }"#,
        )
        .expect("temporary manifest should be written");

        let manifest = extension_runtime::extension::manifest::load_from_dir(directory.path())
            .expect("manifest should parse");
        let catalog = extension_runtime::ExtensionRuntimeCatalog::from_manifests(vec![manifest])
            .expect("catalog should build");

        let connection = extension_connection();
        // 目录中存在 (extension_id, contribution_id) 且声明了 icon → 解析成功
        assert!(extension_connection_icon(
            &connection,
            ConnectionVisualSize::List,
            Some(&catalog)
        )
        .is_some());
    }
}
