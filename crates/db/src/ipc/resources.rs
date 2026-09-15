use crate::ipc::{IpcDriverManifest, IpcDriverRegistry};
use gpui::AssetSource;
use std::borrow::Cow;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tracing::{info, warn};

type RegistryReloader = dyn Fn() -> IpcDriverRegistry + Send + Sync;

/// 驱动包图标的资产命名空间：`driver-icons/{driver_id}/{resource}{ext}`。
///
/// 必须是无 scheme 的相对路径。gpui 的 `img()` 用
/// `url::Url::parse(..).is_ok()` 判断 URI，`driver://x/y` 会被判成合法 URL
/// 而走 `Resource::Uri` 发 HTTP 请求，图标永远加载不到。
pub const DRIVER_ICON_ASSET_PREFIX: &str = "driver-icons/";

/// 本地图标文件的资产命名空间：`local-icon/{path}`。
///
/// 同样不能直接交绝对路径：Windows 盘符（`C:\..`）会被 `url::Url::parse`
/// 判成 scheme = `c` 而走 HTTP 加载。加一层目录前缀后仍是相对路径，
/// 由应用 `AssetSource` 直接读盘，SVG 经 `img()` 以图像方式解码可保留原色。
pub const LOCAL_ICON_ASSET_PREFIX: &str = "local-icon/";

/// 路径是否落在磁盘图标资产命名空间内（驱动包图标 / 本地图标文件）。
pub fn is_icon_asset_path(path: &str) -> bool {
    path.starts_with(DRIVER_ICON_ASSET_PREFIX) || path.starts_with(LOCAL_ICON_ASSET_PREFIX)
}

/// 本地图标文件路径 → 无 scheme 的资产路径。
pub fn local_icon_asset_path(path: &Path) -> String {
    format!("{LOCAL_ICON_ASSET_PREFIX}{}", path.display())
}

/// 无 scheme 的资产路径 → 本地图标文件路径。
pub fn local_icon_file_path(asset_path: &str) -> Option<PathBuf> {
    asset_path
        .strip_prefix(LOCAL_ICON_ASSET_PREFIX)
        .filter(|path| !path.is_empty())
        .map(PathBuf::from)
}

pub struct DriverResourceLoader;

impl DriverResourceLoader {
    pub fn new() -> Self {
        Self
    }

    pub fn load_file(&self, path: &Path) -> std::io::Result<Vec<u8>> {
        std::fs::read(path)
    }
}

/// 磁盘图标资产源。
///
/// 服务两个命名空间，二者都是无 scheme 的路径串，以便 gpui 走
/// `Resource::Embedded` → `AssetSource::load` → `img()`，而不是被当成 URI：
///
/// - [`DRIVER_ICON_ASSET_PREFIX`]：经 [`IpcDriverRegistry`] 解析到驱动包内文件；
/// - [`LOCAL_ICON_ASSET_PREFIX`]：调用方给出的任意本地图标文件。
///
/// 之所以不再用 `Icon::data(bytes)` 提供位图内容：`Icon` 的 Color 模式下
/// Data 分支走 `svg()`，而 gpui 的 `svg()` 只绘制单色 alpha mask 且要求显式
/// text color，叠加后整块不绘制；只有 `img()` 的数字图像解码路径能保留 SVG 原色。
pub struct DriverAssetSource {
    loader: Arc<DriverResourceLoader>,
    registry: Arc<IpcDriverRegistry>,
    registry_reloader: Arc<RegistryReloader>,
}

impl DriverAssetSource {
    pub fn new(loader: Arc<DriverResourceLoader>, registry: Arc<IpcDriverRegistry>) -> Self {
        Self::with_registry_reloader(
            loader,
            registry,
            Arc::new(|| IpcDriverRegistry::load_default()),
        )
    }

    pub fn with_registry_reloader(
        loader: Arc<DriverResourceLoader>,
        registry: Arc<IpcDriverRegistry>,
        registry_reloader: Arc<RegistryReloader>,
    ) -> Self {
        Self {
            loader,
            registry,
            registry_reloader,
        }
    }

    fn parse_driver_path<'a>(&self, path: &'a str) -> Option<(&'a str, &'a str)> {
        let path = path.strip_prefix(DRIVER_ICON_ASSET_PREFIX)?;
        let mut parts = path.splitn(2, '/');
        let driver_id = parts.next()?;
        let resource = parts.next()?;
        if driver_id.is_empty() || resource.is_empty() {
            return None;
        }
        Some((driver_id, resource))
    }

    fn find_driver(&self, driver_id: &str) -> Option<IpcDriverManifest> {
        self.registry
            .find(driver_id)
            .or_else(|| (self.registry_reloader)().find(driver_id))
    }

    fn load_file_bytes(
        &self,
        asset_path: &str,
        file_path: &Path,
        driver_id: &str,
        resource: &str,
    ) -> Result<Cow<'static, [u8]>, anyhow::Error> {
        info!(
            target: "driver_icon",
            driver_id,
            resource,
            asset_path,
            file_path = %file_path.display(),
            exists = file_path.is_file(),
            "loading driver asset file"
        );

        match self.loader.load_file(file_path) {
            Ok(bytes) => {
                info!(
                    target: "driver_icon",
                    driver_id,
                    resource,
                    asset_path,
                    file_path = %file_path.display(),
                    bytes = bytes.len(),
                    "loaded driver asset file"
                );
                Ok(Cow::Owned(bytes))
            }
            Err(error) => {
                warn!(
                    target: "driver_icon",
                    driver_id,
                    resource,
                    asset_path,
                    file_path = %file_path.display(),
                    error = %error,
                    "failed to load driver asset file"
                );
                Err(anyhow::anyhow!(
                    "failed to load driver resource '{}': {}",
                    asset_path,
                    error
                ))
            }
        }
    }
}

impl AssetSource for DriverAssetSource {
    fn load(&self, path: &str) -> Result<Option<Cow<'static, [u8]>>, anyhow::Error> {
        // 本地图标文件：路径由调用方（SSH 自定义图标、扩展贡献图标）提供，
        // 不进驱动注册表，直接读盘。
        if let Some(file_path) = local_icon_file_path(path) {
            return self
                .load_file_bytes(path, &file_path, "", "local_icon")
                .map(Some);
        }

        if !path.starts_with(DRIVER_ICON_ASSET_PREFIX) {
            return Ok(None);
        }

        let (driver_id, resource) = self
            .parse_driver_path(path)
            .ok_or_else(|| anyhow::anyhow!("invalid driver path: {}", path))?;
        let driver = self
            .find_driver(driver_id)
            .ok_or_else(|| anyhow::anyhow!("driver not found: {}", driver_id))?;

        let file_path = match resource_kind(resource) {
            "icon" => driver
                .icon_path()
                .ok_or_else(|| anyhow::anyhow!("driver '{}' has no icon", driver_id))?,
            "icon_color" => driver
                .icon_color_path()
                .ok_or_else(|| anyhow::anyhow!("driver '{}' has no color icon", driver_id))?,
            _ => {
                return Err(anyhow::anyhow!(
                    "unknown resource type: {} (supported: icon, icon_color)",
                    resource
                ));
            }
        };

        self.load_file_bytes(path, &file_path, driver_id, resource)
            .map(Some)
    }

    fn list(&self, _path: &str) -> Result<Vec<gpui::SharedString>, anyhow::Error> {
        Ok(Vec::new())
    }
}

fn resource_kind(resource: &str) -> &str {
    resource.split_once('.').map_or(resource, |(kind, _)| kind)
}
