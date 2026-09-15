use anyhow::{Result, anyhow};
use extension_host::{CancellationToken, RequestOptions};
use extension_plugin_adapter::ManagedUniversalPluginClient;
use extension_protocol::resource::{ResourceCloseParams, ResourceOpenParams, ResourceOpenResult};

use crate::universal_plugins::UniversalPluginService;

/// 已保存扩展连接的 resource/open 启动参数。
/// 只携带 credential 引用，不携带明文 secret。
#[derive(Clone)]
pub(crate) struct ExtensionResourceLaunch {
    // 访问方都在 `shell-plugins` feature 路径下;关闭 feature 时是合法的休眠字段。
    #[cfg_attr(not(feature = "shell-plugins"), allow(dead_code))]
    connection_id: i64,
    runtime_id: String,
    resource_type: String,
    pub(crate) config: serde_json::Value,
}

pub(crate) struct OpenedExtensionResource {
    client: ManagedUniversalPluginClient,
    result: Option<ResourceOpenResult>,
    tokio: tokio::runtime::Handle,
}

impl ExtensionResourceLaunch {
    pub(crate) fn new(
        connection: &one_core::storage::StoredConnection,
        contribution: &extension_runtime::RegisteredResourceConnectionContribution,
    ) -> Result<Self> {
        let connection_id = connection
            .id
            .ok_or_else(|| anyhow!("extension connection must be saved before opening"))?;
        let params = connection.to_extension_params()?;
        let mut config = params.config;
        config.insert(
            "credential_refs".into(),
            serde_json::Value::Object(credential_refs(connection_id, params.secrets.keys())),
        );
        Ok(Self {
            connection_id,
            runtime_id: contribution.runtime_id.clone(),
            resource_type: contribution.resource_type.clone(),
            config: serde_json::Value::Object(config),
        })
    }

    #[cfg_attr(not(feature = "shell-plugins"), allow(dead_code))]
    pub(crate) fn connection_id(&self) -> i64 {
        self.connection_id
    }

    #[cfg_attr(not(feature = "shell-plugins"), allow(dead_code))]
    pub(crate) fn resource_type(&self) -> &str {
        &self.resource_type
    }

    pub(crate) async fn open(
        self,
        service: &UniversalPluginService,
        cancel: &CancellationToken,
    ) -> Result<OpenedExtensionResource> {
        let client = service.universal_plugin_client(&self.runtime_id)?;
        let result = client
            .client()
            .open_resource_with_options(
                &ResourceOpenParams {
                    resource_type: self.resource_type,
                    config: self.config,
                    metadata: None,
                },
                RequestOptions::default().with_cancel(cancel.clone()),
            )
            .await?;
        let mut opened = OpenedExtensionResource::new(client, result);
        if cancel.is_cancelled() {
            opened.close().await;
            return Err(anyhow!("extension connection open cancelled"));
        }
        Ok(opened)
    }
}

impl OpenedExtensionResource {
    fn new(client: ManagedUniversalPluginClient, result: ResourceOpenResult) -> Self {
        Self {
            client,
            result: Some(result),
            tokio: tokio::runtime::Handle::current(),
        }
    }

    /// 打开资源时观察到的 provider 进程 generation。
    pub(crate) fn generation(&self) -> u64 {
        self.client.runtime_generation()
    }

    #[cfg_attr(not(feature = "shell-plugins"), allow(dead_code))]
    pub(crate) fn client(&self) -> &ManagedUniversalPluginClient {
        &self.client
    }

    /// 把打开的资源移交给 ResourceSessionOwner;移交后本句柄不再负责关闭。
    /// client 是 Clone 句柄,clone 后本句柄的 Drop 变为 no-op(result 已被取走)。
    pub(crate) fn into_session(
        mut self,
        identity: extension_plugin_adapter::ResourceSessionIdentity,
    ) -> extension_plugin_adapter::ResourceSessionOwner {
        let result = self
            .take_result()
            .expect("opened resource was already adopted");
        extension_plugin_adapter::ResourceSessionOwner::new(identity, self.client.clone(), result)
    }

    pub(crate) async fn close(&mut self) {
        let Some(result) = self.result.take() else {
            return;
        };
        let _ = self
            .client
            .client()
            .close_resource(&ResourceCloseParams {
                resource_id: result.resource_id,
            })
            .await;
    }

    pub(crate) fn take_result(&mut self) -> Result<ResourceOpenResult> {
        self.result
            .take()
            .ok_or_else(|| anyhow!("extension connection resource was already adopted"))
    }
}

impl Drop for OpenedExtensionResource {
    fn drop(&mut self) {
        let Some(result) = self.result.take() else {
            return;
        };
        let client = self.client.clone();
        self.tokio.spawn(async move {
            let _ = client
                .client()
                .close_resource(&ResourceCloseParams {
                    resource_id: result.resource_id,
                })
                .await;
        });
    }
}

fn credential_refs<'a>(
    connection_id: i64,
    fields: impl Iterator<Item = &'a String>,
) -> serde_json::Map<String, serde_json::Value> {
    fields
        .map(|field| {
            (
                field.clone(),
                serde_json::Value::String(format!("secret://self/{connection_id}:{field}")),
            )
        })
        .collect()
}

/// 工作台/Shell 可见的连接上下文:宿主解析后注入 `NativeResourceWorkbench`。
///
/// 取 `ExtensionConnectionParams::config` 而不是整条 `StoredConnection`:
/// - `config` 与 `secrets` 按 `ExtensionConnectionParams::validate()` 不可能有
///   同名键 ⇒ 密码/token 天然不在其中,**不需要**靠"过滤敏感键"这种会漏的黑名单;
/// - `credential_refs`(provider 取密句柄)也不放进去 —— provider 侧的取密能力
///   不该顺着 UI 通道漏出去。
///
/// 解析失败(非扩展连接 / schema 版本不符)时返回 `Null`:连接上下文是可选
/// 增强,不该让工作台起不来。
pub(crate) fn ui_connection_context(
    connection: &one_core::storage::StoredConnection,
) -> serde_json::Value {
    connection
        .to_extension_params()
        .map(|params| serde_json::Value::Object(params.config))
        .unwrap_or(serde_json::Value::Null)
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use one_core::storage::{ExtensionConnectionParams, StoredConnection};

    use super::*;

    fn contribution() -> extension_runtime::RegisteredResourceConnectionContribution {
        extension_runtime::RegisteredResourceConnectionContribution {
            extension_id: "com.example.search".into(),
            extension_root: std::path::PathBuf::from("/tmp/com.example.search"),
            id: "search".into(),
            label: "Search".into(),
            description: None,
            icon_path: None,
            runtime_id: "com.example.search::main".into(),
            resource_type: "search".into(),
            shell_view_id: Some("explorer".into()),
            form: Default::default(),
        }
    }

    #[test]
    fn launch_materializes_secret_refs_without_secret_values() {
        let params = ExtensionConnectionParams::new(
            "com.example.search",
            "search",
            serde_json::Map::from_iter([("url".into(), "https://example.test".into())]),
            BTreeMap::from([("api_key".into(), "secret-value".into())]),
        )
        .unwrap();
        let mut connection = StoredConnection::new_extension("Search".into(), params, None);
        connection.id = Some(42);
        let launch = ExtensionResourceLaunch::new(&connection, &contribution()).unwrap();

        assert_eq!(
            Some("secret://self/42:api_key"),
            launch.config["credential_refs"]["api_key"].as_str()
        );
        assert!(!launch.config.to_string().contains("secret-value"));
        assert_eq!(42, launch.connection_id());
    }

    #[test]
    fn ui_connection_context_exposes_config_but_never_secrets() {
        // 回归:`source: connection` 绑定此前完全没有真实值来源。这里固定住
        // "能给 UI 的只有非敏感 config"——secrets 与取密句柄都不能出现在
        // 任何投递给工作台/Shell 的上下文里。
        let params = ExtensionConnectionParams::new(
            "com.example.search",
            "search",
            serde_json::Map::from_iter([
                ("url".into(), "https://example.test".into()),
                ("namespace".into(), "prod".into()),
            ]),
            BTreeMap::from([("api_key".into(), "secret-value".into())]),
        )
        .unwrap();
        let mut connection = StoredConnection::new_extension("Search".into(), params, None);
        connection.id = Some(42);

        let context = ui_connection_context(&connection);

        assert_eq!(Some("prod"), context["namespace"].as_str());
        assert_eq!(Some("https://example.test"), context["url"].as_str());
        let rendered = context.to_string();
        assert!(!rendered.contains("secret-value"), "context={rendered}");
        assert!(!rendered.contains("api_key"), "context={rendered}");
        assert!(!rendered.contains("secret://"), "context={rendered}");
    }

    #[test]
    fn ui_connection_context_is_null_for_non_extension_connections() {
        // 非扩展连接(内置 SSH/DB 等)的 params 不是 ExtensionConnectionParams;
        // 返回 Null 而不是报错 —— 连接上下文是可选增强,不该让工作台起不来。
        let params = ExtensionConnectionParams::new(
            "com.example.search",
            "search",
            serde_json::Map::new(),
            BTreeMap::new(),
        )
        .unwrap();
        let mut connection = StoredConnection::new_extension("Host".into(), params, None);
        connection.id = Some(7);
        connection.connection_type = one_core::storage::ConnectionType::SshSftp;

        assert_eq!(serde_json::Value::Null, ui_connection_context(&connection));
    }
}
