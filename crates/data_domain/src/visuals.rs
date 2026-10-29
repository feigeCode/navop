//! 数据领域的连接视觉：外部数据库驱动的图标与随包资产。

use std::sync::{Arc, OnceLock};

use app_kit::visuals::{ConnectionVisuals, DriverIconSource};
use db::ipc::{DRIVER_ICON_ASSET_PREFIX, DriverAssetSource, DriverResourceLoader, IpcDriverRegistry};
use one_core::storage::DbConnectionConfig;

/// 由数据库领域提供的连接视觉解析。
#[derive(Default)]
pub struct DataConnectionVisuals {
    registry: OnceLock<Arc<IpcDriverRegistry>>,
    assets: OnceLock<DriverAssetSource>,
}

impl DataConnectionVisuals {
    pub fn new() -> Self {
        Self::default()
    }

    fn registry(&self) -> &Arc<IpcDriverRegistry> {
        self.registry
            .get_or_init(|| Arc::new(IpcDriverRegistry::load_default()))
    }

    fn assets(&self) -> &DriverAssetSource {
        self.assets.get_or_init(|| {
            DriverAssetSource::new(
                Arc::new(DriverResourceLoader::new()),
                self.registry().clone(),
            )
        })
    }
}

impl ConnectionVisuals for DataConnectionVisuals {
    fn driver_icon(&self, config: &DbConnectionConfig) -> Option<DriverIconSource> {
        let display = self.registry().display_for_config(config)?;
        if let Some(path) = display.icon_file_path {
            return Some(DriverIconSource::File(path));
        }
        display.icon_asset_path.map(DriverIconSource::Asset)
    }

    fn is_driver_asset(&self, path: &str) -> bool {
        path.starts_with(DRIVER_ICON_ASSET_PREFIX)
    }

    fn load_driver_asset(&self, path: &str) -> Option<Vec<u8>> {
        use gpui::AssetSource as _;
        self.assets()
            .load(path)
            .ok()
            .flatten()
            .map(|bytes| bytes.into_owned())
    }
}
