//! Host API 反向调用处理器。
//!
//! 扩展可以通过 JSON-RPC 请求调用宿主提供的能力：
//! - `host/request_credential`: 请求凭证
//! - `host/notify`: 发送通知
//! - `host/quick_pick`: 显示快速选择对话框
//! - `host/open_view`: 打开视图
//! - `host/storage/*`: 键值存储

use std::sync::Arc;

use extension_protocol::error::{ErrorData, ProtocolError, error_codes};
use extension_protocol::host;
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

    /// 发送通知给用户，返回用户点击的 action id。
    async fn notify(&self, params: host::NotifyParams) -> HostResult<host::NotifyResult>;

    /// 显示快速选择对话框。
    async fn quick_pick(&self, params: host::QuickPickParams) -> HostResult<host::QuickPickResult>;

    /// 打开视图（对话框、面板等）。
    async fn open_view(&self, params: host::OpenViewParams) -> HostResult<()>;

    /// 从键值存储读取。
    async fn storage_get(
        &self,
        params: host::StorageGetParams,
    ) -> HostResult<host::StorageGetResult>;

    /// 写入键值存储。
    async fn storage_set(&self, params: host::StorageSetParams) -> HostResult<()>;

    /// 记录日志（扩展发送的日志）。
    async fn log(&self, params: host::LogParams) -> HostResult<()>;
}

/// Host API 处理器。
///
/// 持有 `HostApiProvider` 实现，路由方法调用到对应的处理函数。
pub struct HostApiHandler {
    provider: Arc<dyn HostApiProvider>,
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
            extension_protocol::method::HOST_NOTIFY => {
                let params: host::NotifyParams =
                    serde_json::from_value(params).map_err(|e| HostError::Serde(e))?;
                let result = self.provider.notify(params).await?;
                Ok(serde_json::to_value(result).expect("notify result must serialize"))
            }
            extension_protocol::method::HOST_QUICK_PICK => {
                let params: host::QuickPickParams =
                    serde_json::from_value(params).map_err(|e| HostError::Serde(e))?;
                let result = self.provider.quick_pick(params).await?;
                Ok(serde_json::to_value(result).expect("quick pick result must serialize"))
            }
            extension_protocol::method::HOST_OPEN_VIEW => {
                let params: host::OpenViewParams =
                    serde_json::from_value(params).map_err(|e| HostError::Serde(e))?;
                self.provider.open_view(params).await?;
                Ok(Value::Null)
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

        async fn notify(&self, _params: host::NotifyParams) -> HostResult<host::NotifyResult> {
            Ok(host::NotifyResult { clicked: None })
        }

        async fn quick_pick(
            &self,
            _params: host::QuickPickParams,
        ) -> HostResult<host::QuickPickResult> {
            Ok(host::QuickPickResult {
                selected: vec!["option1".into()],
                cancelled: false,
            })
        }

        async fn open_view(&self, _params: host::OpenViewParams) -> HostResult<()> {
            Ok(())
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
}
