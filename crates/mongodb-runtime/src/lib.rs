//! MongoDB domain contracts independent from GPUI and optional SDK backend.

#![allow(async_fn_in_trait)]

rust_i18n::i18n!("../mongodb_view/locales", fallback = "en");

use bson::{Bson, Document};
use std::path::PathBuf;

pub mod connection;
pub mod ipc;
pub mod types;
mod uri;

#[cfg(feature = "builtin-mongodb")]
mod builtin;

#[cfg(feature = "builtin-mongodb")]
pub use builtin::MongoConnectionImpl as BuiltinMongoConnection;
pub use connection::MongoConnection;
pub use ipc::IpcMongoConnection;
pub use types::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MongoBackendKind {
    Ipc,
    Builtin,
}

pub const DEFAULT_MONGODB_MODERN_DRIVER_ID: &str = "mongodb-modern";
pub const LEGACY_MONGODB_DRIVER_ID: &str = "mongodb-legacy";

#[derive(Clone)]
pub enum MongoConnectionFactory {
    Ipc(Box<extension_host::NativeDriverManifest>),
    InstalledRegistry(PathBuf),
    #[cfg(feature = "builtin-mongodb")]
    Builtin,
    Unavailable,
}

impl MongoConnectionFactory {
    pub fn from_installed_root(root: impl Into<PathBuf>) -> Self {
        Self::InstalledRegistry(root.into())
    }

    pub async fn create(
        &self,
        config: MongoConnectionConfig,
    ) -> Result<Box<dyn MongoConnection>, MongoError> {
        match self {
            Self::InstalledRegistry(root) => {
                let registry = extension_host::NativeDriverRegistry::load_from_dir(root)
                    .map_err(|error| MongoError::Internal(error.to_string()))?;
                let selected = Self::select_from_registry(&registry, &config.driver_id)?;
                selected.create_ipc(config).await
            }
            Self::Ipc(_) => self.create_ipc(config).await,
            #[cfg(feature = "builtin-mongodb")]
            Self::Builtin => {
                let mut connection = BuiltinMongoConnection::new(config);
                connection.connect().await?;
                Ok(Box::new(connection))
            }
            Self::Unavailable => Err(MongoError::Internal(
                "MongoDB native driver is not installed".into(),
            )),
        }
    }

    fn select_from_registry(
        registry: &extension_host::NativeDriverRegistry,
        driver_id: &str,
    ) -> Result<Self, MongoError> {
        let manifest = registry.find("mongodb", driver_id).ok_or_else(|| {
            MongoError::NativeDriverRequired {
                driver_id: driver_id.to_string(),
                reason: "selected MongoDB native driver is not installed".into(),
            }
        })?;
        Ok(Self::Ipc(Box::new(manifest)))
    }

    async fn create_ipc(
        &self,
        config: MongoConnectionConfig,
    ) -> Result<Box<dyn MongoConnection>, MongoError> {
        match self {
            Self::Ipc(manifest) => {
                let mut connection =
                    IpcMongoConnection::with_manifest((**manifest).clone(), config);
                connection.connect().await?;
                Ok(Box::new(connection))
            }
            _ => Err(MongoError::Internal(
                "MongoDB IPC driver selection is invalid".into(),
            )),
        }
    }
}

pub const fn default_backend_kind() -> MongoBackendKind {
    #[cfg(feature = "builtin-mongodb")]
    {
        MongoBackendKind::Builtin
    }
    #[cfg(not(feature = "builtin-mongodb"))]
    {
        MongoBackendKind::Ipc
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct MongoFindOptions {
    pub limit: Option<i64>,
    pub skip: Option<i64>,
    pub sort: Option<Document>,
    pub projection: Option<Document>,
}

pub fn bson_to_compact_json(value: &Bson) -> Result<String, MongoError> {
    serde_json::to_string(&value.clone().into_relaxed_extjson())
        .map_err(|error| MongoError::Serialization(error.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn default_feature_selects_ipc_backend() {
        #[cfg(not(feature = "builtin-mongodb"))]
        assert_eq!(MongoBackendKind::Ipc, default_backend_kind());
        #[cfg(feature = "builtin-mongodb")]
        assert_eq!(MongoBackendKind::Builtin, default_backend_kind());
    }
    #[test]
    fn find_options_keep_ui_paging_contract() {
        let options = MongoFindOptions {
            limit: Some(50),
            skip: Some(10),
            ..Default::default()
        };
        assert_eq!(Some(50), options.limit);
        assert_eq!(Some(10), options.skip);
    }

    #[test]
    fn registry_selection_uses_the_requested_driver() {
        let root = tempfile::TempDir::new().unwrap();
        write_driver(root.path(), LEGACY_MONGODB_DRIVER_ID);
        let registry = extension_host::NativeDriverRegistry::load_from_dir(root.path()).unwrap();

        let factory =
            MongoConnectionFactory::select_from_registry(&registry, LEGACY_MONGODB_DRIVER_ID)
                .unwrap();

        assert!(matches!(factory, MongoConnectionFactory::Ipc(_)));
    }

    #[test]
    fn registry_selection_reports_the_requested_missing_driver() {
        let root = tempfile::TempDir::new().unwrap();
        write_driver(root.path(), LEGACY_MONGODB_DRIVER_ID);
        let registry = extension_host::NativeDriverRegistry::load_from_dir(root.path()).unwrap();

        let error = match MongoConnectionFactory::select_from_registry(
            &registry,
            DEFAULT_MONGODB_MODERN_DRIVER_ID,
        ) {
            Ok(_) => panic!("missing requested driver must not be selected"),
            Err(error) => error,
        };

        assert!(matches!(
            error,
            MongoError::NativeDriverRequired { driver_id, .. }
                if driver_id == DEFAULT_MONGODB_MODERN_DRIVER_ID
        ));
    }

    fn write_driver(root: &std::path::Path, id: &str) {
        let dir = root.join(id);
        fs::create_dir_all(&dir).unwrap();
        fs::write(
            dir.join("driver.json"),
            serde_json::json!({
                "id": id,
                "name": id,
                "api": "mongodb",
                "entry": { "command": "driver" },
                "transport": { "name": format!("{id}.sock") }
            })
            .to_string(),
        )
        .unwrap();
    }
}
