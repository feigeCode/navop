use std::path::Path;

use db::ipc::{IpcDriverRegistry, driver_icon_from_asset_path, driver_icon_from_file_path};
use gpui_component::{BrandIcon, Icon, IconName, IconSize, ObjectIcon, Sizable};
use one_core::storage::{ConnectionType, DatabaseType, DbConnectionConfig, StoredConnection};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ConnectionIdentityIcon {
    Brand(IconName),
    ColorObject(IconName),
    Object(IconName),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ConnectionVisual {
    navigation_icon: IconName,
    identity_icon: ConnectionIdentityIcon,
    accessible_label: &'static str,
}

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

const fn connection_visual(kind: ConnectionType) -> ConnectionVisual {
    use ConnectionIdentityIcon::{Brand, ColorObject, Object};

    match kind {
        ConnectionType::All => ConnectionVisual {
            navigation_icon: IconName::ServerLine,
            identity_icon: Object(IconName::Server),
            accessible_label: "All",
        },
        ConnectionType::Database => ConnectionVisual {
            navigation_icon: IconName::DatabaseLine,
            identity_icon: Object(IconName::Database),
            accessible_label: "Database",
        },
        ConnectionType::SshSftp => ConnectionVisual {
            navigation_icon: IconName::TerminalLine,
            identity_icon: ColorObject(IconName::TerminalColor),
            accessible_label: "SSH/SFTP",
        },
        ConnectionType::Redis => ConnectionVisual {
            navigation_icon: IconName::RedisLine,
            identity_icon: Brand(IconName::Redis),
            accessible_label: "Redis",
        },
        ConnectionType::MongoDB => ConnectionVisual {
            navigation_icon: IconName::MongoDBLine,
            identity_icon: Brand(IconName::MongoDB),
            accessible_label: "MongoDB",
        },
        ConnectionType::Serial => ConnectionVisual {
            navigation_icon: IconName::SerialLine,
            identity_icon: ColorObject(IconName::SerialPort),
            accessible_label: "Serial",
        },
        ConnectionType::PortForwarding => ConnectionVisual {
            navigation_icon: IconName::PortForwardingLine,
            identity_icon: ColorObject(IconName::PortForwardingColor),
            accessible_label: "Port Forwarding",
        },
        ConnectionType::Rdp => ConnectionVisual {
            navigation_icon: IconName::RdpLine,
            identity_icon: ColorObject(IconName::Rdp),
            accessible_label: "RDP",
        },
        ConnectionType::Vnc => ConnectionVisual {
            navigation_icon: IconName::VncLine,
            identity_icon: ColorObject(IconName::Vnc),
            accessible_label: "VNC",
        },
    }
}

/// Monochrome line icons used by navigation and filtering surfaces.
///
/// Protocol identity colors belong to content surfaces, not navigation state.
pub(crate) const fn connection_type_line_icon(kind: ConnectionType) -> IconName {
    connection_visual(kind).navigation_icon
}

/// Monochrome icon for navigation, filtering, and other structural surfaces.
pub(crate) fn connection_type_navigation_icon(
    kind: ConnectionType,
    size: ConnectionVisualSize,
) -> Icon {
    ObjectIcon::new(connection_type_line_icon(kind))
        .with_size(size.icon_size())
        .into_icon()
}

/// Navigation-rail icon with the shared rail glyph size and monochrome policy.
pub(crate) fn connection_type_rail_icon(kind: ConnectionType) -> Icon {
    connection_type_navigation_icon(kind, ConnectionVisualSize::Rail)
}

/// Rich protocol identity icon used by cards, lists, and connection pickers.
pub(crate) fn connection_type_icon(kind: ConnectionType, size: ConnectionVisualSize) -> Icon {
    match connection_visual(kind).identity_icon {
        ConnectionIdentityIcon::Brand(name) => {
            BrandIcon::new(name).with_size(size.icon_size()).into_icon()
        }
        ConnectionIdentityIcon::ColorObject(name) => {
            Icon::new(name).color().with_size(size.icon_size())
        }
        ConnectionIdentityIcon::Object(name) => ObjectIcon::new(name)
            .with_size(size.icon_size())
            .into_icon(),
    }
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
    BrandIcon::new(name).with_size(size.icon_size()).into_icon()
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
            .map(|params| brand_icon(params.os_icon(), size))
            .unwrap_or_else(|_| brand_icon(IconName::LinuxPenguinColor, size)),
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
    ObjectIcon::new(IconName::Database)
        .with_size(size.icon_size())
        .into_icon()
}

fn brand_icon(name: IconName, size: ConnectionVisualSize) -> Icon {
    BrandIcon::new(name).with_size(size.icon_size()).into_icon()
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
    use gpui_component::IconKind;

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
    fn every_connection_type_has_a_complete_visual_definition() {
        for connection_type in ConnectionType::all() {
            let visual = connection_visual(connection_type);
            assert_eq!(visual.accessible_label, connection_type.label());
            assert_eq!(visual.navigation_icon.kind(), IconKind::ObjectGlyph);
        }
    }

    #[test]
    fn navigation_icons_never_use_brand_color() {
        for connection_type in ConnectionType::all() {
            assert_ne!(
                connection_type_line_icon(connection_type).kind(),
                IconKind::BrandColor
            );
        }
    }

    #[test]
    fn remote_desktop_navigation_and_identity_icons_have_distinct_roles() {
        let rdp = connection_visual(ConnectionType::Rdp);
        assert_eq!(rdp.navigation_icon, IconName::RdpLine);
        assert!(matches!(
            rdp.identity_icon,
            ConnectionIdentityIcon::ColorObject(IconName::Rdp)
        ));

        let vnc = connection_visual(ConnectionType::Vnc);
        assert_eq!(vnc.navigation_icon, IconName::VncLine);
        assert!(matches!(
            vnc.identity_icon,
            ConnectionIdentityIcon::ColorObject(IconName::Vnc)
        ));
    }

    #[test]
    fn identity_icons_preserve_color_for_brands_and_protocol_color_assets() {
        assert!(matches!(
            connection_visual(ConnectionType::Redis).identity_icon,
            ConnectionIdentityIcon::Brand(IconName::Redis)
        ));
        assert!(matches!(
            connection_visual(ConnectionType::MongoDB).identity_icon,
            ConnectionIdentityIcon::Brand(IconName::MongoDB)
        ));

        for connection_type in [ConnectionType::All, ConnectionType::Database] {
            assert!(matches!(
                connection_visual(connection_type).identity_icon,
                ConnectionIdentityIcon::Object(_)
            ));
        }

        assert!(matches!(
            connection_visual(ConnectionType::SshSftp).identity_icon,
            ConnectionIdentityIcon::ColorObject(IconName::TerminalColor)
        ));
        assert!(matches!(
            connection_visual(ConnectionType::Serial).identity_icon,
            ConnectionIdentityIcon::ColorObject(IconName::SerialPort)
        ));
        assert!(matches!(
            connection_visual(ConnectionType::PortForwarding).identity_icon,
            ConnectionIdentityIcon::ColorObject(IconName::PortForwardingColor)
        ));
        assert!(matches!(
            connection_visual(ConnectionType::Rdp).identity_icon,
            ConnectionIdentityIcon::ColorObject(IconName::Rdp)
        ));
        assert!(matches!(
            connection_visual(ConnectionType::Vnc).identity_icon,
            ConnectionIdentityIcon::ColorObject(IconName::Vnc)
        ));
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
