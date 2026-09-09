//! Host API 反向调用处理器。
//!
//! 扩展可以通过 JSON-RPC 请求调用宿主提供的能力：
//! - `host/request_credential`: 请求凭证
//! - `host/notify`: 发送通知
//! - `host/storage/*`: 键值存储
//! - `host/blob/*`: 上传宿主管理的 blob

use std::sync::Arc;

use extension_protocol::error::{ErrorData, ProtocolError, error_codes};
use extension_protocol::host;
use extension_protocol::host_blob::{
    HostBlobAbortParams, HostBlobBeginParams, HostBlobBeginResult, HostBlobFinishParams,
    HostBlobFinishResult, HostBlobWriteParams, HostBlobWriteResult,
};
use serde_json::Value;

use crate::error::{HostError, HostResult};

/// Host API 处理器。
///
/// 上层应用提供具体实现（连接凭证存储、通知系统、UI 路由等）。
#[async_trait::async_trait]
pub trait HostApiProvider: Send + Sync {
    /// 请求凭证（密码、私钥等）。
    ///
    /// 扩展通过 `SecretRef` 引用凭证，宿主负责解析并返回引用。
    async fn request_credential(
        &self,
        params: host::RequestCredentialParams,
    ) -> HostResult<host::RequestCredentialResult>;

    /// Resolve a stored secret into a one-shot value.
    ///
    /// Implementations must authenticate and authorize the extension before
    /// returning this value, and must never log or persist the resolved bytes.
    async fn resolve_secret(
        &self,
        params: host::ResolveSecretParams,
    ) -> HostResult<host::ResolveSecretResult>;

    /// 发送通知给用户，返回用户点击的 action id。
    async fn notify(&self, params: host::NotifyParams) -> HostResult<host::NotifyResult>;

    /// 从键值存储读取。
    async fn storage_get(
        &self,
        params: host::StorageGetParams,
    ) -> HostResult<host::StorageGetResult>;

    /// 写入键值存储。
    async fn storage_set(&self, params: host::StorageSetParams) -> HostResult<()>;

    /// 记录日志（扩展发送的日志）。
    async fn log(&self, params: host::LogParams) -> HostResult<()>;

    /// Starts a provider upload into host-authoritative blob storage.
    async fn host_blob_begin(
        &self,
        _params: HostBlobBeginParams,
    ) -> HostResult<HostBlobBeginResult> {
        Err(HostError::NotImplemented(
            "host blob uploads are not configured".into(),
        ))
    }

    /// Appends one strictly ordered base64 chunk to a pending host blob.
    async fn host_blob_write(
        &self,
        _params: HostBlobWriteParams,
    ) -> HostResult<HostBlobWriteResult> {
        Err(HostError::NotImplemented(
            "host blob uploads are not configured".into(),
        ))
    }

    /// Seals a pending upload and publishes an opaque readable blob id.
    async fn host_blob_finish(
        &self,
        _params: HostBlobFinishParams,
    ) -> HostResult<HostBlobFinishResult> {
        Err(HostError::NotImplemented(
            "host blob uploads are not configured".into(),
        ))
    }

    /// Aborts a pending upload. Implementations must make this idempotent.
    async fn host_blob_abort(&self, _params: HostBlobAbortParams) -> HostResult<()> {
        Err(HostError::NotImplemented(
            "host blob uploads are not configured".into(),
        ))
    }
}

/// Host API 处理器。
///
/// 持有 `HostApiProvider` 实现，路由方法调用到对应的处理函数。
pub struct HostApiHandler {
    provider: Arc<dyn HostApiProvider>,
}

impl std::fmt::Debug for HostApiHandler {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HostApiHandler").finish_non_exhaustive()
    }
}

impl HostApiHandler {
    pub fn new(provider: Arc<dyn HostApiProvider>) -> Self {
        Self { provider }
    }

    /// 处理扩展发起的 Host API 请求。
    pub async fn handle(&self, method: &str, params: Value) -> HostResult<Value> {
        match method {
            extension_protocol::method::HOST_REQUEST_CREDENTIAL => {
                let params: host::RequestCredentialParams =
                    serde_json::from_value(params).map_err(|e| HostError::Serde(e))?;
                let result = self.provider.request_credential(params).await?;
                Ok(serde_json::to_value(result).expect("credential value must serialize"))
            }
            extension_protocol::method::HOST_RESOLVE_SECRET => {
                let params: host::ResolveSecretParams =
                    serde_json::from_value(params).map_err(HostError::Serde)?;
                let result = self.provider.resolve_secret(params).await?;
                Ok(serde_json::to_value(result).expect("secret value must serialize"))
            }
            extension_protocol::method::HOST_NOTIFY => {
                let params: host::NotifyParams =
                    serde_json::from_value(params).map_err(|e| HostError::Serde(e))?;
                let result = self.provider.notify(params).await?;
                Ok(serde_json::to_value(result).expect("notify result must serialize"))
            }
            extension_protocol::method::HOST_STORAGE_GET => {
                let params: host::StorageGetParams =
                    serde_json::from_value(params).map_err(|e| HostError::Serde(e))?;
                let result = self.provider.storage_get(params).await?;
                Ok(serde_json::to_value(result).expect("storage get result must serialize"))
            }
            extension_protocol::method::HOST_STORAGE_SET => {
                let params: host::StorageSetParams =
                    serde_json::from_value(params).map_err(|e| HostError::Serde(e))?;
                self.provider.storage_set(params).await?;
                Ok(Value::Null)
            }
            extension_protocol::method::HOST_LOG => {
                let params: host::LogParams =
                    serde_json::from_value(params).map_err(|e| HostError::Serde(e))?;
                self.provider.log(params).await?;
                Ok(Value::Null)
            }
            extension_protocol::method::HOST_BLOB_BEGIN => {
                let params = serde_json::from_value(params).map_err(HostError::Serde)?;
                let result = self.provider.host_blob_begin(params).await?;
                Ok(serde_json::to_value(result).expect("host blob begin result must serialize"))
            }
            extension_protocol::method::HOST_BLOB_WRITE => {
                let params = serde_json::from_value(params).map_err(HostError::Serde)?;
                let result = self.provider.host_blob_write(params).await?;
                Ok(serde_json::to_value(result).expect("host blob write result must serialize"))
            }
            extension_protocol::method::HOST_BLOB_FINISH => {
                let params = serde_json::from_value(params).map_err(HostError::Serde)?;
                let result = self.provider.host_blob_finish(params).await?;
                Ok(serde_json::to_value(result).expect("host blob finish result must serialize"))
            }
            extension_protocol::method::HOST_BLOB_ABORT => {
                let params = serde_json::from_value(params).map_err(HostError::Serde)?;
                self.provider.host_blob_abort(params).await?;
                Ok(Value::Null)
            }
            _ => {
                let error = ProtocolError::new(
                    error_codes::METHOD_NOT_FOUND,
                    format!("unknown host API method: {method}"),
                )
                .with_data(ErrorData::default());
                Err(HostError::Protocol(Box::new(error)))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct MockProvider;

    #[async_trait::async_trait]
    impl HostApiProvider for MockProvider {
        async fn request_credential(
            &self,
            _params: host::RequestCredentialParams,
        ) -> HostResult<host::RequestCredentialResult> {
            use extension_protocol::conn::SecretRef;
            Ok(host::RequestCredentialResult {
                secret_ref: SecretRef {
                    secret_ref: "test:password".into(),
                },
                remembered: false,
            })
        }

        async fn resolve_secret(
            &self,
            _params: host::ResolveSecretParams,
        ) -> HostResult<host::ResolveSecretResult> {
            Ok(host::ResolveSecretResult {
                value: b"token-value".to_vec(),
            })
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
            _params: HostBlobBeginParams,
        ) -> HostResult<HostBlobBeginResult> {
            Ok(HostBlobBeginResult {
                upload_id: "upload-1".into(),
                max_bytes: 1024,
            })
        }

        async fn host_blob_write(
            &self,
            params: HostBlobWriteParams,
        ) -> HostResult<HostBlobWriteResult> {
            Ok(HostBlobWriteResult {
                total_bytes: params.bytes_written.into(),
            })
        }

        async fn host_blob_finish(
            &self,
            _params: HostBlobFinishParams,
        ) -> HostResult<HostBlobFinishResult> {
            Ok(HostBlobFinishResult {
                blob_id: "host-blob-1".into(),
                total_bytes: 3,
                content_type: None,
            })
        }

        async fn host_blob_abort(&self, _params: HostBlobAbortParams) -> HostResult<()> {
            Ok(())
        }
    }

    #[tokio::test]
    async fn handler_routes_to_provider() {
        let provider = Arc::new(MockProvider);
        let handler = HostApiHandler::new(provider);

        let result = handler
            .handle(
                extension_protocol::method::HOST_NOTIFY,
                serde_json::json!({
                    "level": "info",
                    "title": "Test",
                    "message": "Hello"
                }),
            )
            .await;

        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn handler_returns_method_not_found_for_unknown_method() {
        let provider = Arc::new(MockProvider);
        let handler = HostApiHandler::new(provider);

        let result = handler.handle("unknown/method", Value::Null).await;

        assert!(
            matches!(result, Err(HostError::Protocol(e)) if e.code == error_codes::METHOD_NOT_FOUND)
        );
    }

    #[tokio::test]
    async fn handler_routes_host_blob_upload_lifecycle() {
        let handler = HostApiHandler::new(Arc::new(MockProvider));

        let begin = handler
            .handle(
                extension_protocol::method::HOST_BLOB_BEGIN,
                serde_json::json!({"expected_bytes": 3}),
            )
            .await
            .unwrap();
        assert_eq!("upload-1", begin["upload_id"]);

        let write = handler
            .handle(
                extension_protocol::method::HOST_BLOB_WRITE,
                serde_json::json!({
                    "upload_id": "upload-1",
                    "sequence": 0,
                    "data": "YWJj",
                    "bytes_written": 3
                }),
            )
            .await
            .unwrap();
        assert_eq!(3, write["total_bytes"]);

        let finish = handler
            .handle(
                extension_protocol::method::HOST_BLOB_FINISH,
                serde_json::json!({"upload_id": "upload-1"}),
            )
            .await
            .unwrap();
        assert_eq!("host-blob-1", finish["blob_id"]);

        let abort = handler
            .handle(
                extension_protocol::method::HOST_BLOB_ABORT,
                serde_json::json!({"upload_id": "missing"}),
            )
            .await
            .unwrap();
        assert_eq!(Value::Null, abort);
    }
}
