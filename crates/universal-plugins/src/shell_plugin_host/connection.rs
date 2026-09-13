use std::sync::Arc;

use anyhow::{Result, anyhow};
use extension_host::CancellationToken;

use super::ShellConnectionContext;
use crate::extension_resource::{ExtensionResourceLaunch, OpenedExtensionResource};
use crate::universal_plugins::UniversalPluginService;

#[derive(Clone)]
pub(crate) struct ShellConnectionLaunch {
    name: String,
    contribution_id: String,
    alias: String,
    pub(crate) resource: ExtensionResourceLaunch,
}

pub(crate) struct PreparedShellConnection {
    launch: ShellConnectionLaunch,
    resource: OpenedExtensionResource,
}

impl ShellConnectionLaunch {
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
        session: &Arc<super::session::ShellMountSession>,
    ) -> Result<ShellConnectionContext> {
        let result = self.resource.take_result()?;
        let resource =
            session.register_resource(self.launch.alias, self.resource.client(), result)?;
        Ok(ShellConnectionContext {
            connection_id: self.launch.resource.connection_id(),
            name: self.launch.name,
            contribution_id: self.launch.contribution_id,
            resource_type: self.launch.resource.resource_type().to_string(),
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
