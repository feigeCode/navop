//! 连接视觉信息的领域侧解析。
//!
//! 公共壳只负责把图标画到界面上，具体「这个数据库连接用哪个外部驱动图标」
//! 由领域包决定：数据库领域解析驱动清单，终端领域永远返回 `None`。

use std::path::PathBuf;
use std::sync::{Arc, OnceLock};

use one_core::storage::DbConnectionConfig;

/// 外部驱动图标的来源：随包资产或磁盘文件。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DriverIconSource {
    Asset(String),
    File(PathBuf),
}

/// 领域提供的连接视觉信息。
pub trait ConnectionVisuals: Send + Sync {
    /// 数据库连接的外部驱动图标；`None` 表示回退到内置类型图标。
    fn driver_icon(&self, _config: &DbConnectionConfig) -> Option<DriverIconSource> {
        None
    }

    /// 该资产路径是否属于驱动资产（仅用于诊断日志与资产路由）。
    fn is_driver_asset(&self, _path: &str) -> bool {
        false
    }

    /// 读取驱动资产字节；不属于驱动资产时返回 `None`。
    fn load_driver_asset(&self, _path: &str) -> Option<Vec<u8>> {
        None
    }
}

static VISUALS: OnceLock<Arc<dyn ConnectionVisuals>> = OnceLock::new();

/// 注册领域提供的连接视觉解析（首次注册生效）。
pub fn set_connection_visuals(visuals: Arc<dyn ConnectionVisuals>) {
    let _ = VISUALS.set(visuals);
}

/// 当前产品注册的连接视觉解析。
pub fn connection_visuals() -> Option<&'static Arc<dyn ConnectionVisuals>> {
    VISUALS.get()
}

/// 随包驱动图标资产的路径前缀。
pub const DRIVER_ICON_ASSET_PREFIX: &str = "driver-icon/";
/// 本地图标文件的路径前缀（调用方给出的任意文件）。
pub const LOCAL_ICON_ASSET_PREFIX: &str = "local-icon/";

/// 本地图标文件 → 资产路径。
pub fn local_icon_asset_path(path: &std::path::Path) -> String {
    format!("{LOCAL_ICON_ASSET_PREFIX}{}", path.display())
}

/// 资产路径 → 本地图标文件。
pub fn local_icon_file_path(asset_path: &str) -> Option<PathBuf> {
    asset_path
        .strip_prefix(LOCAL_ICON_ASSET_PREFIX)
        .filter(|path| !path.is_empty())
        .map(PathBuf::from)
}

/// 资产路径是否落在图标命名空间内（驱动图标或本地图标文件）。
pub fn is_icon_asset_path(path: &str) -> bool {
    path.starts_with(DRIVER_ICON_ASSET_PREFIX) || path.starts_with(LOCAL_ICON_ASSET_PREFIX)
}

/// 以品牌原色渲染一个随包图标资产。
pub fn icon_from_asset_path(
    path: impl Into<String>,
    size: impl Into<gpui_component::Size>,
) -> gpui_component::Icon {
    use gpui_component::Sizable as _;
    gpui_component::Icon::default()
        .path(path.into())
        .color()
        .with_size(size)
}

/// 以品牌原色渲染一个本地图标文件。
pub fn icon_from_file_path(
    path: impl Into<PathBuf>,
    size: impl Into<gpui_component::Size>,
) -> gpui_component::Icon {
    use gpui_component::Sizable as _;
    let asset_path = local_icon_asset_path(&path.into());
    gpui_component::Icon::default()
        .path(asset_path)
        .color()
        .with_size(size)
}
