use std::path::Path;

use db::ipc::{IpcDriverRegistry, driver_icon_from_asset_path, driver_icon_from_file_path};
use gpui_component::{Icon, IconName, IconSize, Sizable};
use one_core::storage::{ConnectionType, DatabaseType, DbConnectionConfig, StoredConnection};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ExternalDriverIconSource<'a> {
    File(&'a Path),
    Asset(&'a str),
}

/// Semantic icon sizes for connection identity surfaces.
///
/// The visual size is intentionally independent from the surrounding control
/// hit target or container safe area.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ConnectionVisualSize {
    Tree,
    Inline,
    List,
    Card,
    Hero,
    Rail,
}

impl ConnectionVisualSize {
    pub(crate) const fn icon_size(self) -> IconSize {
        match self {
            Self::Tree => IconSize::Default,
            Self::Inline | Self::Rail => IconSize::Medium,
            Self::List => IconSize::Large,
            Self::Card => IconSize::Display,
            Self::Hero => IconSize::Hero,
        }
    }
}

const fn connection_type_icon_name(kind: ConnectionType) -> IconName {
    match kind {
        ConnectionType::All => IconName::Server,
        ConnectionType::Database => IconName::Database,
        ConnectionType::SshSftp => IconName::TerminalColor,
        ConnectionType::Redis => IconName::Redis,
        ConnectionType::MongoDB => IconName::MongoDB,
        ConnectionType::Serial => IconName::SerialPort,
        ConnectionType::Telnet => IconName::SquareTerminalColor,
        ConnectionType::PortForwarding => IconName::PortForwardingColor,
        ConnectionType::Rdp => IconName::Rdp,
        ConnectionType::Vnc => IconName::Vnc,
    }
}

const fn connection_type_navigation_icon_name(kind: ConnectionType) -> IconName {
    match kind {
        ConnectionType::All => IconName::ServerLine,
        ConnectionType::Database => IconName::DatabaseLine,
        ConnectionType::SshSftp => IconName::TerminalLine,
        ConnectionType::Redis => IconName::RedisLine,
        ConnectionType::MongoDB => IconName::MongoDBLine,
        ConnectionType::Serial => IconName::SerialLine,
        ConnectionType::Telnet => IconName::SquareTerminal,
        ConnectionType::PortForwarding => IconName::PortForwardingLine,
        ConnectionType::Rdp => IconName::RdpLine,
        ConnectionType::Vnc => IconName::VncLine,
    }
}

/// Monochrome line icon used by navigation and filtering surfaces.
pub(crate) fn connection_type_navigation_icon(
    kind: ConnectionType,
    size: ConnectionVisualSize,
) -> Icon {
    connection_type_navigation_icon_name(kind)
        .mono()
        .with_size(size.icon_size())
}

/// Monochrome navigation-rail icon with the shared rail glyph size.
pub(crate) fn connection_type_rail_icon(kind: ConnectionType) -> Icon {
    connection_type_navigation_icon(kind, ConnectionVisualSize::Rail)
}

/// Original-color protocol identity icon used by cards, lists, and connection pickers.
pub(crate) fn connection_type_icon(kind: ConnectionType, size: ConnectionVisualSize) -> Icon {
    connection_type_icon_name(kind)
        .color()
        .with_size(size.icon_size())
}

pub(crate) fn database_type_icon(kind: &DatabaseType, size: ConnectionVisualSize) -> Icon {
    let name = match kind {
        DatabaseType::MySQL => IconName::MySQLColor,
        DatabaseType::PostgreSQL => IconName::PostgreSQLColor,
        DatabaseType::SQLite => IconName::SQLiteColor,
        DatabaseType::DuckDB => IconName::DuckDB,
        DatabaseType::MSSQL => IconName::MSSQLColor,
        DatabaseType::Oracle => IconName::OracleColor,
        DatabaseType::ClickHouse => IconName::ClickHouseColor,
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

pub(crate) fn stored_connection_icon(
    connection: &StoredConnection,
    size: ConnectionVisualSize,
    registry: &IpcDriverRegistry,
) -> Icon {
    match connection.connection_type {
        ConnectionType::Database => connection
            .to_db_connection()
            .map(|config| database_config_icon(&config, size, registry))
            .unwrap_or_else(|_| generic_database_icon(size)),
        ConnectionType::SshSftp => connection
            .to_ssh_params()
            .map(|params| color_icon(params.os_icon(), size))
            .unwrap_or_else(|_| color_icon(IconName::LinuxPenguinColor, size)),
        kind => connection_type_icon(kind, size),
    }
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

    #[test]
    fn semantic_connection_sizes_map_to_the_shared_icon_scale() {
        assert_eq!(ConnectionVisualSize::Tree.icon_size(), IconSize::Default);
        assert_eq!(ConnectionVisualSize::Inline.icon_size(), IconSize::Medium);
        assert_eq!(ConnectionVisualSize::List.icon_size(), IconSize::Large);
        assert_eq!(ConnectionVisualSize::Card.icon_size(), IconSize::Display);
        assert_eq!(ConnectionVisualSize::Hero.icon_size(), IconSize::Hero);
        assert_eq!(ConnectionVisualSize::Rail.icon_size(), IconSize::Medium);
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
            (ConnectionType::All, IconName::ServerLine),
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
}
