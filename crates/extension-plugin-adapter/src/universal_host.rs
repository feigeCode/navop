//! Host-authoritative APIs used by native universal resource providers.

use std::{collections::BTreeMap, sync::Arc};

use extension_host::{HostApiProvider, HostError, HostResult};
use extension_protocol::host::{self, ResolveSecretParams, ResolveSecretResult};
use extension_protocol::host_blob::{
    HostBlobAbortParams, HostBlobBeginParams, HostBlobBeginResult, HostBlobFinishParams,
    HostBlobFinishResult, HostBlobWriteParams, HostBlobWriteResult,
};

use crate::blob_store::{BlobOwner, BlobStore};
use crate::provider_permissions::{
    ProviderPermissionError, ProviderPermissionSet, SecretReference,
};

/// Application-owned secret lookup. Values never enter logs or provider UI state.
#[async_trait::async_trait]
pub trait SecretResolver: Send + Sync {
    async fn resolve(&self, namespace: &str, key: &str) -> HostResult<Vec<u8>>;
}

/// Simple in-memory resolver suitable for tests and integration harnesses.
#[derive(Clone, Default)]
pub struct MapSecretResolver(BTreeMap<String, Vec<u8>>);

impl MapSecretResolver {
    pub fn insert(mut self, namespace: &str, key: &str, value: impl Into<Vec<u8>>) -> Self {
        self.0.insert(reference_key(namespace, key), value.into());
        self
    }
}

#[async_trait::async_trait]
impl SecretResolver for MapSecretResolver {
    async fn resolve(&self, namespace: &str, key: &str) -> HostResult<Vec<u8>> {
        self.0
            .get(&reference_key(namespace, key))
            .cloned()
            .ok_or_else(|| HostError::protocol(secret_not_found()))
    }
}

pub struct UniversalProviderHost {
    permissions: ProviderPermissionSet,
    secrets: Arc<dyn SecretResolver>,
    blobs: Option<BlobCapability>,
}

#[derive(Clone)]
struct BlobCapability {
    store: BlobStore,
    owner: BlobOwner,
}

impl UniversalProviderHost {
    pub fn new<I, P>(permissions: I, secrets: Arc<dyn SecretResolver>) -> Self
    where
        I: IntoIterator<Item = P>,
        P: AsRef<str>,
    {
        Self {
            permissions: ProviderPermissionSet::new(
                permissions
                    .into_iter()
                    .map(|value| value.as_ref().to_owned()),
            ),
            secrets,
            blobs: None,
        }
    }

    pub fn with_blob_store(mut self, store: BlobStore, owner: BlobOwner) -> Self {
        self.blobs = Some(BlobCapability { store, owner });
        self
    }

    fn blob_capability(&self) -> HostResult<&BlobCapability> {
        self.blobs
            .as_ref()
            .ok_or_else(|| HostError::NotImplemented("host blob uploads are not configured".into()))
    }

    async fn resolve_secret(&self, params: ResolveSecretParams) -> HostResult<ResolveSecretResult> {
        let reference = SecretReference::parse(&params.secret_ref)
            .map_err(|error| HostError::protocol(error.into()))?;
        if !self.permissions.allows_secret(&reference) {
            return Err(HostError::protocol(
                ProviderPermissionError::SecretDenied.into(),
            ));
        }
        let value = self
            .secrets
            .resolve(&reference.namespace, &reference.key)
            .await?;
        Ok(ResolveSecretResult { value })
    }
}

#[async_trait::async_trait]
impl HostApiProvider for UniversalProviderHost {
    async fn request_credential(
        &self,
        _params: host::RequestCredentialParams,
    ) -> HostResult<host::RequestCredentialResult> {
        // Providers should receive explicit secret references from connection
        // configuration; interactive credential acquisition remains host-owned.
        Err(HostError::NotImplemented(
            "interactive credential requests are not available for this extension".into(),
        ))
    }

    async fn resolve_secret(&self, params: ResolveSecretParams) -> HostResult<ResolveSecretResult> {
        self.resolve_secret(params).await
    }

    async fn notify(&self, _params: host::NotifyParams) -> HostResult<host::NotifyResult> {
        Ok(host::NotifyResult { clicked: None })
    }

    async fn storage_get(
        &self,
        _params: host::StorageGetParams,
    ) -> HostResult<host::StorageGetResult> {
        Ok(host::StorageGetResult { value: None })
    }

    async fn storage_set(&self, _params: host::StorageSetParams) -> HostResult<()> {
        Ok(())
    }

    async fn log(&self, _params: host::LogParams) -> HostResult<()> {
        Ok(())
    }

    async fn host_blob_begin(
        &self,
        params: HostBlobBeginParams,
    ) -> HostResult<HostBlobBeginResult> {
        let capability = self.blob_capability()?;
        capability
            .store
            .begin_upload(&capability.owner, params)
            .map_err(|error| HostError::protocol(error.into()))
    }

    async fn host_blob_write(
        &self,
        params: HostBlobWriteParams,
    ) -> HostResult<HostBlobWriteResult> {
        let capability = self.blob_capability()?;
        capability
            .store
            .write_upload(&capability.owner, params)
            .map_err(|error| HostError::protocol(error.into()))
    }

    async fn host_blob_finish(
        &self,
        params: HostBlobFinishParams,
    ) -> HostResult<HostBlobFinishResult> {
        let capability = self.blob_capability()?;
        capability
            .store
            .finish_upload(&capability.owner, params)
            .map_err(|error| HostError::protocol(error.into()))
    }

    async fn host_blob_abort(&self, params: HostBlobAbortParams) -> HostResult<()> {
        let capability = self.blob_capability()?;
        capability
            .store
            .abort_upload(&capability.owner, &params.upload_id)
            .map_err(|error| HostError::protocol(error.into()))
    }
}

fn reference_key(namespace: &str, key: &str) -> String {
    format!("{namespace}\u{1}{key}")
}

fn secret_not_found() -> extension_protocol::ProtocolError {
    extension_protocol::ProtocolError::new(
        extension_protocol::error_codes::SECRET_NOT_FOUND,
        "requested secret was not found",
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use extension_protocol::conn::SecretRef;

    fn provider() -> UniversalProviderHost {
        UniversalProviderHost::new(
            ["secrets:read:elasticsearch.*".to_owned()],
            Arc::new(MapSecretResolver::default().insert(
                "elasticsearch",
                "api_key",
                b"token-value",
            )),
        )
    }

    #[tokio::test]
    async fn secret_resolution_is_permission_checked() {
        let result = provider()
            .resolve_secret(ResolveSecretParams {
                secret_ref: SecretRef::new("secret://elasticsearch/api_key"),
            })
            .await
            .unwrap();
        assert_eq!(b"token-value".to_vec(), result.value);

        let error = provider()
            .resolve_secret(ResolveSecretParams {
                secret_ref: SecretRef::new("secret://other/api_key"),
            })
            .await
            .unwrap_err();
        assert!(
            matches!(error, HostError::Protocol(error) if error.code == extension_protocol::error_codes::PERMISSION_DENIED)
        );
    }
}
