use db::ipc::{IpcDriverRegistry, driver_icon_from_asset_path, driver_icon_from_file_path};
use gpui_component::{Icon, Size};
use one_core::storage::DbConnectionConfig;
use std::path::Path;

pub(crate) fn external_driver_icon_for_config_with_registry(
    config: &DbConnectionConfig,
    size: impl Into<Size>,
    registry: &IpcDriverRegistry,
) -> Option<Icon> {
    let Some(display) = registry.display_for_config(config) else {
        return None;
    };

    let icon_asset_path = display.icon_asset_path;
    let icon_file_path = display.icon_file_path;
    if icon_asset_path.is_none() && icon_file_path.is_none() {
        return None;
    }
    Some(match icon_file_path {
        Some(path) => driver_icon_from_file_path(path, size),
        None => driver_icon_from_asset_path(icon_asset_path?, size),
    })
}

pub(crate) fn external_driver_icon_from_path(path: &str, size: impl Into<Size>) -> Icon {
    driver_icon_from_asset_path(path.to_string(), size)
}

pub(crate) fn external_driver_icon_from_file_path(path: &Path, size: impl Into<Size>) -> Icon {
    driver_icon_from_file_path(path.to_path_buf(), size)
}
