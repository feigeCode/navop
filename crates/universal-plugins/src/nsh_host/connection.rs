//! 扩展连接（connection）的页面装载。
//!
//! 连接型扩展（远程桌面、ACP agent…）打开时做两件事：激活 provider，然后
//! **把连接资源本身打开**（`resource.open`，配置来自保存的连接），再把打开
//! 得到的句柄借给页面。页面拿到的 `navop.context.current().connection` 就是
//! 这份上下文，`resourceHandle` 可以直接 invoke。
//!
//! 与嵌入工作台的"借来的会话"不同，这里的资源是**本页拥有的**：tab 关掉
//! 就该把它关掉 —— 连接是页面开起来的。

use std::sync::Arc;

use anyhow::{Result, anyhow};
use extension_host::CancellationToken;
use serde_json::{Value, json};

use crate::extension_resource::{
    ExtensionResourceLaunch, OpenedExtensionResource, ui_connection_context,
};
use crate::universal_plugins::UniversalPluginService;

/// 一次连接页面的启动参数（借用前）。
#[derive(Clone)]
pub(crate) struct NshConnectionLaunch {
    name: String,
    contribution_id: String,
    alias: String,
    /// 连接的非敏感配置（页面用它显示"这个页面连着谁"）。
    config: Value,
    pub(crate) resource: ExtensionResourceLaunch,
}

impl NshConnectionLaunch {
    pub(crate) fn new(
        connection: &one_core::storage::StoredConnection,
        contribution: &extension_runtime::RegisteredResourceConnectionContribution,
        view: &extension_runtime::RegisteredShellViewContribution,
    ) -> Result<Self> {
        let resource = ExtensionResourceLaunch::new(connection, contribution)?;
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
            config: ui_connection_context(connection),
            resource,
        })
    }
}

/// 连接资源已经打开、等着交给页面会话。
pub(crate) struct PreparedNshConnection {
    launch: NshConnectionLaunch,
    opened: OpenedExtensionResource,
}

impl PreparedNshConnection {
    /// 把句柄登记进会话，返回页面要看到的连接上下文。
    pub(super) fn adopt(mut self, session: &Arc<super::session::NshSession>) -> Result<Value> {
        let result = self.opened.take_result()?;
        let handle = session
            .register_connection_resource(self.opened.client(), result)
            .map_err(|error| anyhow!("{}", error.message))?;
        Ok(json!({
            "connectionId": self.launch.resource.connection_id(),
            "name": self.launch.name,
            "contributionId": self.launch.contribution_id,
            "resourceType": self.launch.resource.resource_type(),
            "resourceHandle": handle,
            "alias": self.launch.alias,
            "config": self.launch.config,
        }))
    }
}

/// 打开连接资源（provider 侧 `resource.open`）。
pub(crate) async fn open_connection_resource(
    service: &UniversalPluginService,
    launch: NshConnectionLaunch,
    cancel: &CancellationToken,
) -> Result<PreparedNshConnection> {
    let opened = launch.resource.clone().open(service, cancel).await?;
    Ok(PreparedNshConnection { launch, opened })
}
