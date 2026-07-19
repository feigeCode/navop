//! MongoDB domain contracts independent from GPUI and optional SDK backend.

#![allow(async_fn_in_trait)]

rust_i18n::i18n!("../mongodb_view/locales", fallback = "en");

use bson::{Bson, Document};
use std::path::PathBuf;

pub mod connection;
pub mod ipc;
pub mod types;

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
    IpcWithLegacy {
        modern: Box<extension_host::NativeDriverManifest>,
        legacy: Box<extension_host::NativeDriverManifest>,
    },
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
                let legacy_installed = registry.find("mongodb", LEGACY_MONGODB_DRIVER_ID).is_some();
                let selected = Self::select_from_registry(&registry)?;
                match selected.create_ipc(config).await {
                    Err(MongoError::ServerIncompatible(reason)) if !legacy_installed => {
                        Err(MongoError::NativeDriverRequired {
                            driver_id: LEGACY_MONGODB_DRIVER_ID.to_string(),
                            reason,
                        })
                    }
                    result => result,
                }
            }
            Self::Ipc(_) | Self::IpcWithLegacy { .. } => self.create_ipc(config).await,
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
    ) -> Result<Self, MongoError> {
        let modern = registry
            .find("mongodb", DEFAULT_MONGODB_MODERN_DRIVER_ID)
            .ok_or_else(|| MongoError::Internal("MongoDB native driver is not installed".into()))?;
        Ok(match registry.find("mongodb", LEGACY_MONGODB_DRIVER_ID) {
            Some(legacy) => Self::IpcWithLegacy {
                modern: Box::new(modern),
                legacy: Box::new(legacy),
            },
            None => Self::Ipc(Box::new(modern)),
        })
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
            Self::IpcWithLegacy { modern, legacy } => {
                let mut modern_connection =
                    IpcMongoConnection::with_manifest((**modern).clone(), config.clone());
                match modern_connection.connect().await {
                    Ok(()) => Ok(Box::new(modern_connection)),
                    Err(MongoError::ServerIncompatible(_)) => {
                        let mut legacy_connection =
                            IpcMongoConnection::with_manifest((**legacy).clone(), config);
                        legacy_connection.connect().await?;
                        Ok(Box::new(legacy_connection))
                    }
                    Err(error) => Err(error),
                }
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
    fn registry_selection_pairs_modern_and_legacy_manifests() {
        let root = tempfile::TempDir::new().unwrap();
        write_driver(root.path(), DEFAULT_MONGODB_MODERN_DRIVER_ID);
        write_driver(root.path(), LEGACY_MONGODB_DRIVER_ID);
        let registry = extension_host::NativeDriverRegistry::load_from_dir(root.path()).unwrap();

        let factory = MongoConnectionFactory::select_from_registry(&registry).unwrap();

        assert!(matches!(
            factory,
            MongoConnectionFactory::IpcWithLegacy { .. }
        ));
    }

    #[test]
    fn registry_selection_requires_modern_as_the_primary_driver() {
        let root = tempfile::TempDir::new().unwrap();
        write_driver(root.path(), LEGACY_MONGODB_DRIVER_ID);
        let registry = extension_host::NativeDriverRegistry::load_from_dir(root.path()).unwrap();

        let error = match MongoConnectionFactory::select_from_registry(&registry) {
            Ok(_) => panic!("legacy-only registry must not be selected"),
            Err(error) => error,
        };

        assert!(error.to_string().contains("not installed"));
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
