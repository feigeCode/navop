use super::*;

pub(crate) fn can_manage_connection_with_permissions(
    connections: &[StoredConnection],
    connection_id: i64,
    permissions: &TeamPermissionSnapshot,
) -> bool {
    connections
        .iter()
        .find(|connection| connection.id == Some(connection_id))
        .is_some_and(|connection| permissions.can_edit_connection(connection.team_id.as_deref()))
}

impl HomePage {
    pub(crate) fn can_move_connection(&self, connection_id: i64) -> bool {
        self.can_manage_connection(connection_id)
    }

    pub(crate) fn can_export_connection_credentials(&self, connection_id: i64) -> bool {
        self.connection_credential_export_identity(connection_id)
            .is_some()
    }

    pub(crate) fn connection_credential_export_identity(
        &self,
        connection_id: i64,
    ) -> Option<ConnectionCredentialExportIdentity> {
        let connection = self
            .connections
            .iter()
            .find(|connection| connection.id == Some(connection_id))?;
        self.team_permissions
            .can_edit_connection(connection.team_id.as_deref())
            .then(|| ConnectionCredentialExportIdentity::from_connection(connection))
    }

    fn can_manage_connection(&self, connection_id: i64) -> bool {
        can_manage_connection_with_permissions(
            &self.connections,
            connection_id,
            &self.team_permissions,
        )
    }

    pub(crate) fn cached_team_options(&self) -> &[TeamOption] {
        self.team_permissions.teams()
    }

    pub(crate) fn move_connection_to_workspace(
        &mut self,
        connection_id: i64,
        workspace_id: Option<i64>,
        cx: &mut Context<Self>,
    ) {
        if !self.valid_workspace_move(connection_id, workspace_id) {
            return;
        }
        let Some(mut connection) = self
            .connections
            .iter()
            .find(|connection| connection.id == Some(connection_id))
            .cloned()
        else {
            return;
        };
        connection.workspace_id = workspace_id;
        let storage = cx.global::<GlobalStorageState>().storage.clone();
        let save_task = cx.background_spawn(async move {
            let repo = storage
                .get::<ConnectionRepository>()
                .ok_or_else(|| anyhow::anyhow!("ConnectionRepository not found"))?;
            let connection_id = connection
                .id
                .ok_or_else(|| anyhow::anyhow!("Connection ID not found"))?;
            connection.updated_at =
                Some(repo.update_workspace(connection_id, connection.workspace_id)?);
            Ok::<_, anyhow::Error>(connection)
        });
        cx.spawn(async move |this, cx| match save_task.await {
            Ok(connection) => {
                _ = this.update(cx, |this, cx| {
                    if let Some(item) = this
                        .connections
                        .iter_mut()
                        .find(|item| item.id == connection.id)
                    {
                        *item = connection.clone();
                    }
                    emit_connection_event(
                        ConnectionDataEvent::ConnectionUpdated { connection },
                        cx,
                    );
                    cx.notify();
                });
            }
            Err(error) => tracing::error!("移动连接分组失败: {error}"),
        })
        .detach();
    }

    fn valid_workspace_move(&self, connection_id: i64, workspace_id: Option<i64>) -> bool {
        let target_exists = workspace_id.is_none()
            || self
                .workspaces
                .iter()
                .any(|workspace| workspace.id == workspace_id);
        target_exists
            && self.can_move_connection(connection_id)
            && self.connections.iter().any(|connection| {
                connection.id == Some(connection_id) && connection.workspace_id != workspace_id
            })
    }
}
