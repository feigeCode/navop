use std::sync::Arc;

use anyhow::{Result, anyhow};
use extension_host::{CancellationToken, RequestOptions};
use extension_plugin_adapter::ManagedUniversalPluginClient;
use extension_protocol::resource::{ResourceCloseParams, ResourceOpenParams, ResourceOpenResult};
use one_core::storage::ConnectionRepository;

use super::{
    ShellConnectionContext, session::ShellMountSession, ssh_tunnel::rewrite_config_with_tunnel,
};
use crate::universal_plugins::UniversalPluginService;

#[derive(Clone)]
pub(crate) struct ExtensionResourceLaunch {
    connection_id: i64,
    runtime_id: String,
    resource_type: String,
    config: serde_json::Value,
    /// SSH 隧道引用模式解析所需的连接仓库(无隧道配置时可为 None)
    repository: Option<Arc<ConnectionRepository>>,
}

pub(crate) struct OpenedExtensionResource {
    client: ManagedUniversalPluginClient,
    result: Option<ResourceOpenResult>,
    tokio: tokio::runtime::Handle,
    /// 打开资源时建立的 SSH 隧道守卫:随资源关闭/丢弃而释放
    tunnels: Vec<connection_tunnel::TunnelGuard>,
}

#[derive(Clone)]
pub(crate) struct ShellConnectionLaunch {
    name: String,
    contribution_id: String,
    alias: String,
    resource: ExtensionResourceLaunch,
}

pub(crate) struct PreparedShellConnection {
    launch: ShellConnectionLaunch,
    resource: OpenedExtensionResource,
}

impl ExtensionResourceLaunch {
    pub(crate) fn new(
        connection: &one_core::storage::StoredConnection,
        contribution: &extension_runtime::RegisteredResourceConnectionContribution,
        repository: Option<Arc<ConnectionRepository>>,
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
            repository,
        })
    }

    pub(crate) fn connection_id(&self) -> i64 {
        self.connection_id
    }

    pub(crate) async fn open(
        self,
        service: &UniversalPluginService,
        cancel: &CancellationToken,
    ) -> Result<OpenedExtensionResource> {
        let client = service.universal_plugin_client(&self.runtime_id)?;
        // SSH 隧道:启用时在宿主侧逐地址建隧道并改写 config 地址字段
        let config = match self.config {
            serde_json::Value::Object(config) => config,
            _ => return Err(anyhow!("extension connection config must be an object")),
        };
        let rewrite = rewrite_config_with_tunnel(config, self.repository.as_deref()).await?;
        let result = client
            .client()
            .open_resource_with_options(
                &ResourceOpenParams {
                    resource_type: self.resource_type,
                    config: serde_json::Value::Object(rewrite.config),
                    metadata: None,
                },
                RequestOptions::default().with_cancel(cancel.clone()),
            )
            .await?;
        let mut opened = OpenedExtensionResource::new(client, result, rewrite.guards);
        if cancel.is_cancelled() {
            opened.close().await;
            return Err(anyhow!("extension connection open cancelled"));
        }
        Ok(opened)
    }
}

impl OpenedExtensionResource {
    fn new(
        client: ManagedUniversalPluginClient,
        result: ResourceOpenResult,
        tunnels: Vec<connection_tunnel::TunnelGuard>,
    ) -> Self {
        Self {
            client,
            result: Some(result),
            tokio: tokio::runtime::Handle::current(),
            tunnels,
        }
    }

    pub(crate) fn capabilities(&self) -> &[String] {
        self.result
            .as_ref()
            .map(|result| result.capabilities.as_slice())
            .unwrap_or_default()
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

    fn take_result(&mut self) -> Result<ResourceOpenResult> {
        self.result
            .take()
            .ok_or_else(|| anyhow!("extension connection resource was already adopted"))
    }

    /// 取出隧道守卫(移交给会话等更长寿的持有者;未转移时随本资源 Drop 释放)。
    fn take_tunnels(&mut self) -> Vec<connection_tunnel::TunnelGuard> {
        std::mem::take(&mut self.tunnels)
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

impl ShellConnectionLaunch {
    pub(crate) fn new(
        connection: &one_core::storage::StoredConnection,
        contribution: &extension_runtime::RegisteredResourceConnectionContribution,
        view: &extension_runtime::RegisteredShellViewContribution,
        repository: Option<Arc<ConnectionRepository>>,
    ) -> Result<Self> {
        let resource = ExtensionResourceLaunch::new(connection, contribution, repository)?;
        let alias = view
            .backends
            .iter()
            .find_map(|(alias, runtime_id)| {
                (runtime_id == &contribution.runtime_id).then(|| alias.clone())
            })
            .ok_or_else(|| anyhow!("connection shell view does not expose its runtime"))?;
        Ok(Self {
            name: connection.name.clone(),
            contribution_id: contribution.id.clone(),
            alias,
            resource,
        })
    }

    pub(crate) fn connection_id(&self) -> i64 {
        self.resource.connection_id()
    }
}

impl PreparedShellConnection {
    pub(super) fn adopt(
        mut self,
        session: &Arc<ShellMountSession>,
    ) -> Result<ShellConnectionContext> {
        let result = self.resource.take_result()?;
        // 隧道守卫移交会话:随资源关闭/运行时停用(close_all 或会话 Drop)释放
        session.register_tunnels(self.resource.take_tunnels());
        let resource =
            session.register_resource(self.launch.alias, &self.resource.client, result)?;
        Ok(ShellConnectionContext {
            connection_id: self.launch.resource.connection_id,
            name: self.launch.name,
            contribution_id: self.launch.contribution_id,
            resource_type: self.launch.resource.resource_type,
            resource,
        })
    }
}

pub(crate) async fn open_connection_resource(
    service: &UniversalPluginService,
    launch: ShellConnectionLaunch,
    cancel: &CancellationToken,
) -> Result<PreparedShellConnection> {
    let resource = launch.resource.clone().open(service, cancel).await?;
    Ok(PreparedShellConnection { launch, resource })
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

#[cfg(test)]
mod tests;
