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

/// 首页列表里一条连接应该落在哪个分组桶里。
///
/// 返回 `None` 表示当前分组过滤把它隐藏了；返回 `Some(None)` 表示「未分组」桶。
/// 分组已被删除（或本地暂时缺失）的连接一律并进「未分组」桶：它们既不属于任何
/// 现存分组，又不会出现在任何分组标题下，否则会从首页彻底消失。
pub(crate) fn connection_bucket(
    workspace_id: Option<i64>,
    known_workspace_ids: &HashSet<i64>,
    filtered_workspace_ids: &HashSet<i64>,
) -> Option<Option<i64>> {
    match workspace_id {
        Some(workspace_id) if !known_workspace_ids.contains(&workspace_id) => Some(None),
        Some(workspace_id)
            if filtered_workspace_ids.is_empty()
                || filtered_workspace_ids.contains(&workspace_id) =>
        {
            Some(Some(workspace_id))
        }
        Some(_) => None,
        None => Some(None),
    }
}

#[cfg(test)]
mod connection_bucket_tests {
    use super::*;

    fn ids(values: &[i64]) -> HashSet<i64> {
        values.iter().copied().collect()
    }

    #[test]
    fn ungrouped_connections_stay_visible_under_a_group_filter() {
        // 过滤只挑分组，不能把「未分组」一起过滤掉：否则在过滤状态下新建的连接
        // 会从首页消失（侧栏却还在显示）。
        assert_eq!(
            Some(None),
            connection_bucket(None, &ids(&[1, 2]), &ids(&[1]))
        );
    }

    #[test]
    fn connections_of_filtered_out_groups_are_hidden() {
        assert_eq!(None, connection_bucket(Some(2), &ids(&[1, 2]), &ids(&[1])));
        assert_eq!(
            Some(Some(1)),
            connection_bucket(Some(1), &ids(&[1, 2]), &ids(&[1]))
        );
    }

    #[test]
    fn connections_of_deleted_groups_fall_back_to_unassigned() {
        assert_eq!(
            Some(None),
            connection_bucket(Some(9), &ids(&[1, 2]), &ids(&[]))
        );
        assert_eq!(
            Some(None),
            connection_bucket(Some(9), &ids(&[1, 2]), &ids(&[1]))
        );
    }
}
