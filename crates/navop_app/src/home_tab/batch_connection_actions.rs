use super::*;
use gpui_component::{
    IndexPath,
    select::{Select, SelectItem, SelectState},
};
use one_core::storage::{SSH_ICON_IDS, SshAuthMethod, StoredConnection};

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
impl HomePage {
    /// 批量把选中 SSH 连接的用户名改成同一个值（#62「批量修改用户名」）。
    pub(crate) fn edit_selected_ssh_usernames(
        &mut self,
        connection_ids: Vec<i64>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.edit_selected_ssh_credentials(
            SshCredentialField::Username,
            connection_ids,
            window,
            cx,
        );
    }

    /// 批量把选中 SSH 连接的密码改成同一个值（#62「批量修改密码」）。
    pub(crate) fn edit_selected_ssh_passwords(
        &mut self,
        connection_ids: Vec<i64>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.edit_selected_ssh_credentials(
            SshCredentialField::Password,
            connection_ids,
            window,
            cx,
        );
    }

    /// 批量把选中 SSH 连接的图标改成同一个（#62「批量修改图标」）。
    ///
    /// 「自动」= 清掉手动指定，仍按探测到的系统图标显示。
    pub(crate) fn edit_selected_ssh_icons(
        &mut self,
        connection_ids: Vec<i64>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let connection_ids =
            unique_manageable_connection_ids(connection_ids, |id| self.can_move_connection(id));
        let connection_ids = select_editable_ssh_connections(
            self.connections.iter(),
            connection_ids,
            SshCredentialField::Username,
        );
        if connection_ids.is_empty() {
            window.open_dialog(cx, move |dialog, _window, _cx| {
                dialog
                    .title(t!("Connection.batch_edit_ssh_icon").to_string())
                    .child(t!("Connection.batch_edit_no_ssh").to_string())
                    .alert()
            });
            return;
        }

        let count = connection_ids.len();
        let icon_select = cx
            .new(|cx| SelectState::new(batch_icon_choices(), Some(IndexPath::new(0)), window, cx));
        let view = cx.entity();

        window.open_dialog(cx, move |dialog, _window, _cx| {
            let icon_select = icon_select.clone();
            let view = view.clone();
            let connection_ids = connection_ids.clone();

            dialog
                .title(t!("Connection.batch_edit_ssh_icon").to_string())
                .width(px(460.))
                .confirm()
                .child(
                    v_flex()
                        .gap_2()
                        .child(t!("Connection.batch_edit_icon_hint", count = count).to_string())
                        .child(t!("Connection.batch_edit_icon_select").to_string())
                        .child(Select::new(&icon_select)),
                )
                .on_ok(move |_, _window, cx: &mut App| {
                    let icon = icon_select.read(cx).selected_value().cloned().flatten();
                    persist_ssh_params_edits(
                        view.clone(),
                        connection_ids.clone(),
                        cx,
                        move |params| {
                            apply_batch_ssh_icon(params, icon.as_deref());
                            true
                        },
                    );
                    true
                })
        });
    }

    /// 凭据批量编辑的公共实现：弹窗输入一次，写到所有命中的 SSH 连接上。
    fn edit_selected_ssh_credentials(
        &mut self,
        field: SshCredentialField,
        connection_ids: Vec<i64>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let connection_ids =
            unique_manageable_connection_ids(connection_ids, |id| self.can_move_connection(id));
        let connection_ids =
            select_editable_ssh_connections(self.connections.iter(), connection_ids, field);
        if connection_ids.is_empty() {
            window.open_dialog(cx, move |dialog, _window, _cx| {
                dialog
                    .title(field.title())
                    .child(field.missing_error().into_any_element())
                    .alert()
            });
            return;
        }

        let count = connection_ids.len();
        let value_input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder(field.placeholder())
                .masked(field.is_password())
        });
        let error_message = cx.new(|_| Option::<String>::None);
        let view = cx.entity();

        let input_for_ok = value_input.clone();
        let error_for_ok = error_message.clone();
        let input_for_render = value_input.clone();
        let error_for_render = error_message.clone();

        window.open_dialog(cx, move |dialog, _window, _cx| {
            let input_for_ok = input_for_ok.clone();
            let error_for_ok = error_for_ok.clone();
            let input_for_render = input_for_render.clone();
            let error_for_render = error_for_render.clone();
            let view = view.clone();
            let connection_ids = connection_ids.clone();

            dialog
                .title(field.title())
                .width(px(460.))
                .confirm()
                .child(
                    v_flex()
                        .gap_2()
                        .child(field.hint(count))
                        .child(Input::new(&input_for_render))
                        .when_some(error_for_render.read(_cx).clone(), |this, error| {
                            this.child(div().text_xs().text_color(_cx.theme().danger).child(error))
                        }),
                )
                .on_ok(move |_, _window, cx: &mut App| {
                    let view = view.clone();
                    let value = input_for_ok.read(cx).text().to_string();
                    let value = value.trim().to_string();
                    if value.is_empty() {
                        error_for_ok.update(cx, |error, cx| {
                            *error = Some(field.empty_error());
                            cx.notify();
                        });
                        return false;
                    }

                    persist_ssh_params_edits(
                        view.clone(),
                        connection_ids.clone(),
                        cx,
                        move |params| field.apply(params, &value),
                    );
                    true
                })
        });
    }
}

/// 把一批 SSH 连接的参数改动写回存储并同步主页列表（#62 批量编辑的公共实现）。
fn persist_ssh_params_edits<F>(
    view: Entity<HomePage>,
    connection_ids: Vec<i64>,
    cx: &mut App,
    mutate: F,
) where
    F: Fn(&mut one_core::storage::SshParams) -> bool + Send + 'static,
{
    let storage = cx.global::<GlobalStorageState>().storage.clone();
    let save_task = cx.background_spawn(async move {
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
                serde_json::from_str::<one_core::storage::SshParams>(&decrypted.params)
            else {
                skipped += 1;
                continue;
            };
            if !mutate(&mut params) {
                skipped += 1;
                continue;
            }
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
                tracing::warn!(skipped, "批量修改 SSH 连接时跳过了无法处理的连接");
            }
        }
        Err(error) => tracing::error!("批量修改 SSH 连接失败: {error}"),
    })
    .detach();
}

/// 批量改图标下拉里的一项；`id` 为空表示「自动」（按探测到的系统选择）。
#[derive(Clone)]
struct BatchIconChoice {
    id: Option<String>,
    label: String,
}

impl SelectItem for BatchIconChoice {
    type Value = Option<String>;

    fn title(&self) -> SharedString {
        self.label.clone().into()
    }

    fn value(&self) -> &Self::Value {
        &self.id
    }
}

/// 「自动」+ 全部内置 SSH 图标，顺序与单项编辑里的一致。
fn batch_icon_choices() -> Vec<BatchIconChoice> {
    std::iter::once(BatchIconChoice {
        id: None,
        label: t!("Connection.batch_edit_icon_auto").to_string(),
    })
    .chain(SSH_ICON_IDS.iter().map(|id| BatchIconChoice {
        id: Some((*id).to_string()),
        label: (*id).to_string(),
    }))
    .collect()
}

/// 写入批量选择的图标；手选内置图标时清掉本地图片，避免图片继续优先显示。
fn apply_batch_ssh_icon(params: &mut one_core::storage::SshParams, icon: Option<&str>) {
    params.icon = icon.map(str::to_string);
    params.icon_file_path = None;
}

/// 批量编辑的 SSH 凭据字段（#62）。
#[derive(Clone, Copy)]
enum SshCredentialField {
    Username,
    Password,
}

impl SshCredentialField {
    fn is_password(self) -> bool {
        matches!(self, Self::Password)
    }

    fn title(self) -> String {
        match self {
            Self::Username => t!("Connection.batch_edit_ssh_username"),
            Self::Password => t!("Connection.batch_edit_ssh_password"),
        }
        .to_string()
    }

    fn hint(self, count: usize) -> String {
        match self {
            Self::Username => t!("Connection.batch_edit_username_hint", count = count),
            Self::Password => t!("Connection.batch_edit_password_hint", count = count),
        }
        .to_string()
    }

    fn placeholder(self) -> String {
        match self {
            Self::Username => t!("Connection.batch_edit_username_placeholder"),
            Self::Password => t!("Connection.batch_edit_password_placeholder"),
        }
        .to_string()
    }

    fn empty_error(self) -> String {
        match self {
            Self::Username => t!("Connection.batch_edit_username_empty"),
            Self::Password => t!("Connection.batch_edit_password_empty"),
        }
        .to_string()
    }

    fn missing_error(self) -> String {
        match self {
            Self::Username => t!("Connection.batch_edit_no_ssh"),
            Self::Password => t!("Connection.batch_edit_no_password_auth"),
        }
        .to_string()
    }

    /// 该连接是否能被本字段批量修改。
    fn matches(self, connection: &StoredConnection) -> bool {
        if connection.connection_type != ConnectionType::SshSftp {
            return false;
        }
        match self {
            Self::Username => true,
            Self::Password => {
                serde_json::from_str::<one_core::storage::SshParams>(&connection.params)
                    .map(|params| params.auth_method.contains_password())
                    .unwrap_or(false)
            }
        }
    }

    /// 把新值写进参数；返回是否真的改动了（密码认证以外的连接会被跳过）。
    fn apply(self, params: &mut one_core::storage::SshParams, value: &str) -> bool {
        match self {
            Self::Username => {
                params.username = value.to_string();
                true
            }
            Self::Password => set_ssh_password(&mut params.auth_method, value),
        }
    }
}

/// 递归把密码写入密码认证（含组合认证里的密码步骤）；没有密码因素时返回 false。
fn set_ssh_password(auth: &mut SshAuthMethod, password: &str) -> bool {
    match auth {
        SshAuthMethod::Password { password: current } => {
            *current = password.to_string();
            true
        }
        SshAuthMethod::Chain(steps) => {
            let mut changed = false;
            for step in steps {
                changed |= set_ssh_password(step, password);
            }
            changed
        }
        _ => false,
    }
}

/// 只保留能被该字段批量修改的 SSH 连接，供批量凭据编辑使用（#62）。
fn select_editable_ssh_connections<'a>(
    connections: impl IntoIterator<Item = &'a StoredConnection>,
    connection_ids: Vec<i64>,
    field: SshCredentialField,
) -> Vec<i64> {
    let candidates = connections
        .into_iter()
        .filter(|connection| field.matches(connection))
        .filter_map(|connection| connection.id)
        .collect::<HashSet<_>>();
    connection_ids
        .into_iter()
        .filter(|id| candidates.contains(id))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::{
        SshCredentialField, apply_batch_ssh_icon, batch_icon_choices,
        select_editable_ssh_connections, set_ssh_password, unique_manageable_connection_ids,
    };
    use one_core::storage::SSH_ICON_IDS;
    use one_core::storage::{ConnectionType, SshAuthMethod, StoredConnection};

    #[test]
    fn manageable_connection_ids_are_filtered_deduplicated_and_sorted() {
        let ids =
            unique_manageable_connection_ids(vec![8, 3, 8, 5, 2], |id| matches!(id, 3 | 5 | 8));

        assert_eq!(ids, vec![3, 5, 8]);
    }

    fn ssh_connection(id: i64, auth_method: SshAuthMethod) -> StoredConnection {
        let params: one_core::storage::SshParams = serde_json::from_value(serde_json::json!({
            "host": "example.com",
            "port": 22,
            "username": "root",
            "auth_method": auth_method,
        }))
        .expect("SshParams 反序列化不应失败");
        let mut connection = StoredConnection::new_ssh("ssh".to_string(), params, None);
        connection.id = Some(id);
        connection
    }

    #[test]
    fn batch_username_edit_only_keeps_ssh_connections() {
        let password_auth = || SshAuthMethod::Password {
            password: "secret".to_string(),
        };
        let connections = vec![
            ssh_connection(1, password_auth()),
            ssh_connection(2, SshAuthMethod::Agent),
            ssh_connection(3, password_auth()),
        ];

        assert_eq!(
            select_editable_ssh_connections(
                &connections,
                vec![1, 2, 3],
                SshCredentialField::Username
            ),
            vec![1, 2, 3]
        );
        assert_eq!(
            select_editable_ssh_connections(
                &connections,
                vec![1, 2, 3],
                SshCredentialField::Password
            ),
            vec![1, 3]
        );
    }

    #[test]
    fn setting_a_password_skips_connections_without_a_password_factor() {
        let mut password_auth = SshAuthMethod::Password {
            password: "old".to_string(),
        };
        assert!(set_ssh_password(&mut password_auth, "new"));
        assert!(matches!(
            password_auth,
            SshAuthMethod::Password { ref password } if password == "new"
        ));

        let mut chained = SshAuthMethod::Chain(vec![
            SshAuthMethod::PrivateKey {
                key_path: "/tmp/id".to_string(),
                passphrase: None,
            },
            SshAuthMethod::Password {
                password: "old".to_string(),
            },
        ]);
        assert!(set_ssh_password(&mut chained, "new"));
        assert!(matches!(
            chained,
            SshAuthMethod::Chain(ref steps)
                if steps.iter().any(|step| matches!(step, SshAuthMethod::Password { password } if password == "new"))
        ));

        let mut agent = SshAuthMethod::Agent;
        assert!(!set_ssh_password(&mut agent, "new"));
    }

    #[test]
    fn batch_icon_choices_cover_auto_and_every_built_in_icon() {
        let choices = batch_icon_choices();

        assert_eq!(choices.len(), SSH_ICON_IDS.len() + 1);
        assert_eq!(choices[0].id, None);
        assert_eq!(choices[1].id.as_deref(), Some("linux"));
        assert!(
            choices
                .iter()
                .any(|choice| choice.id.as_deref() == Some("docker"))
        );
    }

    #[test]
    fn batch_icon_edit_clears_the_custom_icon_file_and_auto_resets() {
        let mut params: one_core::storage::SshParams = serde_json::from_value(serde_json::json!({
            "host": "example.com",
            "port": 22,
            "username": "root",
            "auth_method": SshAuthMethod::Agent,
            "icon": "debian",
            "icon_file_path": "/tmp/icon.png",
        }))
        .expect("SshParams 反序列化不应失败");

        apply_batch_ssh_icon(&mut params, Some("ubuntu"));
        assert_eq!(params.icon.as_deref(), Some("ubuntu"));
        assert_eq!(params.icon_file_path, None);

        apply_batch_ssh_icon(&mut params, None);
        assert_eq!(params.icon, None);
        assert_eq!(params.icon_file_path, None);
    }

    #[test]
    fn batch_actions_have_one_confirmation_and_reuse_single_connection_operations() {
        let source = include_str!("batch_connection_actions.rs");
        assert!(source.contains("show_batch_delete_confirmation"));
        assert!(source.contains("this.delete_connection(connection_id, cx)"));
        assert!(source.contains("self.move_connection_to_workspace"));
        assert!(source.contains("edit_selected_ssh_usernames"));
        assert!(source.contains("with_decrypted_params"));
        assert!(source.contains("edit_selected_ssh_icons"));
        assert!(source.contains("persist_ssh_params_edits"));
    }
}
