//! MongoDB domain contracts independent from GPUI and optional SDK backend.

#![allow(async_fn_in_trait)]

rust_i18n::i18n!("../mongodb_view/locales", fallback = "en");

use bson::{Bson, Document};

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
    #[cfg(feature = "builtin-mongodb")]
    Builtin,
    Unavailable,
}

impl MongoConnectionFactory {
    pub async fn create(
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
}
