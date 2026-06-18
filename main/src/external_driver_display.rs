use db::ipc::{IpcDriverRegistry, driver_icon_from_asset_path, driver_icon_from_file_path};
use gpui_component::{Icon, Size};
use one_core::storage::DbConnectionConfig;
use std::path::Path;
use tracing::info;

pub(crate) fn external_driver_icon_for_config(
    config: &DbConnectionConfig,
    size: impl Into<Size>,
) -> Option<Icon> {
    external_driver_icon_for_config_with_registry(config, size, &IpcDriverRegistry::load_default())
}

fn external_driver_icon_for_config_with_registry(
    config: &DbConnectionConfig,
    size: impl Into<Size>,
    registry: &IpcDriverRegistry,
) -> Option<Icon> {
    let driver_id = config.database_type.external_driver_id()?;
    let Some(display) = registry.display_for_config(config) else {
        info!(
            target: "driver_icon",
            driver_id,
            connection_id = %config.id,
            connection_name = %config.name,
            "home connection has no external driver icon; falling back"
        );
        return None;
    };

    let icon_asset_path = display.icon_asset_path;
    let icon_file_path = display.icon_file_path;
    if icon_asset_path.is_none() && icon_file_path.is_none() {
        info!(
            target: "driver_icon",
            driver_id,
            connection_id = %config.id,
            connection_name = %config.name,
            "home connection external driver has no icon path; falling back"
        );
        return None;
    }

    info!(
        target: "driver_icon",
        driver_id,
        connection_id = %config.id,
        connection_name = %config.name,
        asset_path = ?icon_asset_path,
        file_path = ?icon_file_path.as_ref().map(|path| path.display().to_string()),
        "home connection selected external driver icon"
    );
    Some(match icon_file_path {
        Some(path) => driver_icon_from_file_path(path, size),
        None => driver_icon_from_asset_path(icon_asset_path?, size),
    })
}

pub(crate) fn external_driver_icon_from_path(path: &str, size: impl Into<Size>) -> Icon {
    info!(
        target: "driver_icon",
        asset_path = path,
        "new connection selected external driver icon"
    );
    driver_icon_from_asset_path(path.to_string(), size)
}

pub(crate) fn external_driver_icon_from_file_path(path: &Path, size: impl Into<Size>) -> Icon {
    info!(
        target: "driver_icon",
        file_path = %path.display(),
        exists = path.is_file(),
        "new connection selected external driver icon file"
    );
    driver_icon_from_file_path(path.to_path_buf(), size)
}
