use crate::ipc::{IpcDriverManifest, IpcDriverRegistry};
use gpui::AssetSource;
use std::borrow::Cow;
use std::path::Path;
use std::sync::Arc;
use tracing::{info, warn};

type RegistryReloader = dyn Fn() -> IpcDriverRegistry + Send + Sync;

pub struct DriverResourceLoader;

impl DriverResourceLoader {
    pub fn new() -> Self {
        Self
    }

    pub fn load_file(&self, path: &Path) -> std::io::Result<Vec<u8>> {
        std::fs::read(path)
    }
}

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
        let path = path.strip_prefix("driver://")?;
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
}

impl AssetSource for DriverAssetSource {
    fn load(&self, path: &str) -> Result<Option<Cow<'static, [u8]>>, anyhow::Error> {
        if !path.starts_with("driver://") {
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

        info!(
            target: "driver_icon",
            driver_id,
            resource,
            asset_path = path,
            file_path = %file_path.display(),
            exists = file_path.is_file(),
            "loading driver asset file"
        );

        match self.loader.load_file(&file_path) {
            Ok(bytes) => {
                info!(
                    target: "driver_icon",
                    driver_id,
                    resource,
                    asset_path = path,
                    file_path = %file_path.display(),
                    bytes = bytes.len(),
                    "loaded driver asset file"
                );
                Ok(Some(Cow::Owned(bytes)))
            }
            Err(error) => {
                warn!(
                    target: "driver_icon",
                    driver_id,
                    resource,
                    asset_path = path,
                    file_path = %file_path.display(),
                    error = %error,
                    "failed to load driver asset file"
                );
                Err(anyhow::anyhow!(
                    "failed to load driver resource '{}': {}",
                    path,
                    error
                ))
            }
        }
    }

    fn list(&self, _path: &str) -> Result<Vec<gpui::SharedString>, anyhow::Error> {
        Ok(Vec::new())
    }
}

fn resource_kind(resource: &str) -> &str {
    resource.split_once('.').map_or(resource, |(kind, _)| kind)
}
