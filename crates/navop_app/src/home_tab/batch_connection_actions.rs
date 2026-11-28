use super::*;

fn unique_manageable_connection_ids(
    connection_ids: Vec<i64>,
    mut can_manage: impl FnMut(i64) -> bool,
) -> Vec<i64> {
    let mut ids = connection_ids
        .into_iter()
        .filter(|id| can_manage(*id))
        .collect::<HashSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();
    ids.sort_unstable();
    ids
}

impl HomePage {
    pub(crate) fn move_connections_to_workspace(
        &mut self,
        connection_ids: Vec<i64>,
        workspace_id: Option<i64>,
        cx: &mut Context<Self>,
    ) {
        let connection_ids =
            unique_manageable_connection_ids(connection_ids, |id| self.can_move_connection(id));
        for connection_id in connection_ids {
            self.move_connection_to_workspace(connection_id, workspace_id, cx);
        }
    }

    pub(crate) fn confirm_delete_connections(
        &mut self,
        connection_ids: Vec<i64>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let connection_ids =
            unique_manageable_connection_ids(connection_ids, |id| self.can_move_connection(id));
        if connection_ids.is_empty() {
            return;
        }
        let active_count = connection_ids
            .iter()
            .filter(|id| cx.global::<ActiveConnections>().is_active(**id))
            .count();
        if active_count > 0 {
            show_batch_delete_in_use_alert(active_count, window, cx);
            return;
        }
        show_batch_delete_confirmation(connection_ids, window, cx);
    }
}

fn show_batch_delete_in_use_alert(
    active_count: usize,
    window: &mut Window,
    cx: &mut Context<HomePage>,
) {
    window.open_dialog(cx, move |dialog, _window, _cx| {
        dialog
            .title(t!("Connection.in_use_title").to_string().into_any_element())
            .child(
                t!("Connection.batch_delete_in_use", count = active_count)
                    .to_string()
                    .into_any_element(),
            )
            .alert()
    });
}

fn show_batch_delete_confirmation(
    connection_ids: Vec<i64>,
    window: &mut Window,
    cx: &mut Context<HomePage>,
) {
    let count = connection_ids.len();
    let view = cx.entity();
    window.open_dialog(cx, move |dialog, _window, _cx| {
        let view = view.clone();
        let connection_ids = connection_ids.clone();
        dialog
            .title(t!("Common.delete").to_string().into_any_element())
            .child(
                t!("Connection.batch_delete_confirm", count = count)
                    .to_string()
                    .into_any_element(),
            )
            .confirm()
            .on_ok(move |_, _, cx: &mut App| {
                _ = view.update(cx, |this, cx| {
                    for connection_id in connection_ids.iter().copied() {
                        this.delete_connection(connection_id, cx);
                    }
                });
                true
            })
    });
}

/// 只保留确实是 SSH 的连接，供批量修改用户名使用（#62）。
fn editable_ssh_connection_ids(
    connections: impl IntoIterator<Item = (Option<i64>, ConnectionType)>,
    connection_ids: Vec<i64>,
) -> Vec<i64> {
    let connections = connections.into_iter().collect::<Vec<_>>();
    connection_ids
        .into_iter()
        .filter(|id| {
            connections.iter().any(|(connection_id, kind)| {
                connection_id == &Some(*id) && *kind == ConnectionType::SshSftp
            })
        })
        .collect()
}

impl HomePage {
    /// 批量把选中 SSH 连接的用户名改成同一个值（#62「批量修改用户名」）。
    pub(crate) fn edit_selected_ssh_usernames(
        &mut self,
        connection_ids: Vec<i64>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let connection_ids =
            unique_manageable_connection_ids(connection_ids, |id| self.can_move_connection(id));
        let connection_ids = editable_ssh_connection_ids(
            self.connections
                .iter()
                .map(|connection| (connection.id, connection.connection_type)),
            connection_ids,
        );
        if connection_ids.is_empty() {
            window.open_dialog(cx, |dialog, _window, _cx| {
                dialog
                    .title(t!("Connection.batch_edit_ssh_username").to_string())
                    .child(
                        t!("Connection.batch_edit_no_ssh")
                            .to_string()
                            .into_any_element(),
                    )
                    .alert()
            });
            return;
        }

        let count = connection_ids.len();
        let username_input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder(t!("Connection.batch_edit_username_placeholder"))
        });
        let error_message = cx.new(|_| Option::<String>::None);
        let view = cx.entity();

        let username_for_ok = username_input.clone();
        let error_for_ok = error_message.clone();
        let username_for_render = username_input.clone();
        let error_for_render = error_message.clone();

        window.open_dialog(cx, move |dialog, _window, _cx| {
            let username_for_ok = username_for_ok.clone();
            let error_for_ok = error_for_ok.clone();
            let username_for_render = username_for_render.clone();
            let error_for_render = error_for_render.clone();
            let view = view.clone();
            let connection_ids = connection_ids.clone();

            dialog
                .title(t!("Connection.batch_edit_ssh_username").to_string())
                .width(px(460.))
                .confirm()
                .child(
                    v_flex()
                        .gap_2()
                        .child(t!("Connection.batch_edit_username_hint", count = count).to_string())
                        .child(Input::new(&username_for_render))
                        .when_some(error_for_render.read(_cx).clone(), |this, error| {
                            this.child(div().text_xs().text_color(_cx.theme().danger).child(error))
                        }),
                )
                .on_ok(move |_, _window, cx: &mut App| {
                    let view = view.clone();
                    let username = username_for_ok.read(cx).text().to_string();
                    let username = username.trim().to_string();
                    if username.is_empty() {
                        error_for_ok.update(cx, |error, cx| {
                            *error = Some(t!("Connection.batch_edit_username_empty").to_string());
                            cx.notify();
                        });
                        return false;
                    }

                    let storage = cx.global::<GlobalStorageState>().storage.clone();
                    let connection_ids = connection_ids.clone();
                    let save_task =
                        cx.background_spawn(async move {
                            let repo = storage
                                .get::<ConnectionRepository>()
                                .ok_or_else(|| anyhow::anyhow!("ConnectionRepository not found"))?;
                            let mut updated_connections = Vec::new();
                            let mut skipped = 0usize;
                            for connection_id in connection_ids {
                                let Some(connection) = repo.get(connection_id)? else {
                                    skipped += 1;
                                    continue;
                                };
                                let decrypted = connection.with_decrypted_params();
                                let Ok(mut params) =
                                    serde_json::from_str::<one_core::storage::SshParams>(
                                        &decrypted.params,
                                    )
                                else {
                                    skipped += 1;
                                    continue;
                                };
                                params.username = username.clone();
                                let mut updated = decrypted;
                                updated.params = serde_json::to_string(&params)?;
                                repo.update(&updated)?;
                                updated_connections.push(updated);
                            }
                            Ok::<_, anyhow::Error>((updated_connections, skipped))
                        });

                    cx.spawn(async move |cx| match save_task.await {
                        Ok((updated_connections, skipped)) => {
                            _ = view.update(cx, |home, cx| {
                                for connection in updated_connections {
                                    if let Some(item) = home
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
                                }
                                cx.notify();
                            });
                            if skipped > 0 {
                                tracing::warn!(
                                    skipped,
                                    "批量修改 SSH 用户名时跳过了无法解析的连接"
                                );
                            }
                        }
                        Err(error) => tracing::error!("批量修改 SSH 用户名失败: {error}"),
                    })
                    .detach();
                    true
                })
        });
    }
}

#[cfg(test)]
mod tests {
    use super::{editable_ssh_connection_ids, unique_manageable_connection_ids};
    use one_core::storage::ConnectionType;

    #[test]
    fn manageable_connection_ids_are_filtered_deduplicated_and_sorted() {
        let ids =
            unique_manageable_connection_ids(vec![8, 3, 8, 5, 2], |id| matches!(id, 3 | 5 | 8));

        assert_eq!(ids, vec![3, 5, 8]);
    }

    #[test]
    fn batch_username_edit_only_keeps_ssh_connections() {
        let connections = vec![
            (Some(1), ConnectionType::SshSftp),
            (Some(2), ConnectionType::Database),
            (Some(3), ConnectionType::SshSftp),
            (Some(4), ConnectionType::Rdp),
            (None, ConnectionType::SshSftp),
        ];

        assert_eq!(
            editable_ssh_connection_ids(connections, vec![1, 2, 3, 4]),
            vec![1, 3]
        );
    }

    #[test]
    fn batch_actions_have_one_confirmation_and_reuse_single_connection_operations() {
        let source = include_str!("batch_connection_actions.rs");
        assert!(source.contains("show_batch_delete_confirmation"));
        assert!(source.contains("this.delete_connection(connection_id, cx)"));
        assert!(source.contains("self.move_connection_to_workspace"));
        assert!(source.contains("edit_selected_ssh_usernames"));
        assert!(source.contains("with_decrypted_params"));
    }
}
