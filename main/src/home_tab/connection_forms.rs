use super::*;

/// 扩展连接表单覆盖的连接类型:原生扩展连接,以及可迁移的历史内置中间件连接。
fn is_extension_form_type(connection_type: &ConnectionType) -> bool {
    matches!(
        connection_type,
        ConnectionType::Extension | ConnectionType::Mqtt
    )
}

/// 把连接归一为可编辑的扩展连接形态(历史内置 MQTT 按需迁移)。
///
/// 非扩展表单覆盖的类型、或迁移/参数解析失败时返回 `None`。
fn editable_extension_connection(connection: &StoredConnection) -> Option<StoredConnection> {
    if !is_extension_form_type(&connection.connection_type) {
        return None;
    }
    let mut connection = connection.clone();
    connection.try_migrate_legacy_middleware_connection();
    connection.to_extension_params().ok().map(|_| connection)
}

/// 连接表单弹窗的复用键。
///
/// 表单窗口关闭时只隐藏、不销毁（带 Touch Bar 的 Mac 上关闭即闪退的修法），同一个键再次打开时
/// 重新显示同一个原生窗口，所以键要带上「哪一类表单 + 哪一个连接」：「新建」只占一个槽位
/// （同一时刻只需要一个新建表单），编辑表单按连接 id 各占一个 —— 否则给 B 打开编辑表单
/// 会把 A 那个（连同 A 里没保存的修改）顶掉。
pub(super) fn connection_form_reuse_key(kind: &str, editing: Option<&StoredConnection>) -> String {
    match editing.and_then(|connection| connection.id) {
        Some(id) => format!("connection-form:{kind}:{id}"),
        None => format!("connection-form:{kind}:new"),
    }
}

impl HomePage {
    pub(crate) fn show_extension_form(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let editing = self
            .editing_connection_id
            .and_then(|id| {
                self.connections
                    .iter()
                    .find(|connection| connection.id == Some(id))
            })
            .filter(|connection| is_extension_form_type(&connection.connection_type))
            .cloned();
        let Some(editing) = editing else {
            return;
        };
        let Some(connection) = editable_extension_connection(&editing) else {
            window.push_notification("Extension connection data is invalid", cx);
            return;
        };
        let Ok(params) = connection.to_extension_params() else {
            window.push_notification("Extension connection data is invalid", cx);
            return;
        };
        let Some(contribution) = cx
            .try_global::<extension_runtime::GlobalExtensionRuntimeCatalog>()
            .and_then(|catalog| catalog.get())
            .and_then(|catalog| {
                catalog
                    .resource_connection(&params.extension_id, &params.contribution_id)
                    .cloned()
            })
        else {
            window.push_notification(
                format!(
                    "Extension {} is missing or no longer provides connection {}",
                    params.extension_id, params.contribution_id
                ),
                cx,
            );
            return;
        };
        let config = universal_plugins::ExtensionConnectionFormConfig {
            initial_workspace_id: None,
            contribution,
            editing_connection: Some(connection.clone()),
            workspaces: self.workspaces.clone(),
            teams: get_cached_team_options(cx),
        };
        self.editing_connection_id = None;
        open_reusable_popup_window(
            PopupWindowOptions::new(format!("Edit {}", connection.name)).size(700.0, 650.0),
            connection_form_reuse_key("extension", config.editing_connection.as_ref()),
            move |window, cx| {
                cx.new(|cx| {
                    universal_plugins::ExtensionConnectionForm::new(config.clone(), window, cx)
                })
            },
            Some(window),
            cx,
        );
    }

    pub(crate) fn show_save_temporary_connection_form(
        &mut self,
        connection: StoredConnection,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.is_master_key_ready_for_new_connection() {
            return;
        }

        let config = SshFormWindowConfig {
            initial_workspace_id: None,
            editing_connection: None,
            initial_connection: Some(connection),
            on_saved: Some(Arc::new(|saved, _action, window, cx| {
                window.push_notification(
                    t!("Home.save_as_connection_done", name = saved.name.clone()).to_string(),
                    cx,
                );
            })),
            workspaces: self.workspaces.clone(),
            teams: get_cached_team_options(cx),
        };

        open_reusable_popup_window(
            PopupWindowOptions::new(t!("Home.save_as_connection").to_string()).size(820.0, 750.0),
            connection_form_reuse_key("ssh-save-temporary", None),
            move |window, cx| cx.new(|cx| SshFormWindow::new(config.clone(), window, cx)),
            Some(window),
            cx,
        );
    }
}

#[cfg(test)]
mod extension_form_tests {
    use one_core::storage::{
        ConnectionType, ExtensionConnectionParams, MQTT_EXTENSION_ID, MqttParams, StoredConnection,
    };

    use super::{editable_extension_connection, is_extension_form_type};

    #[test]
    fn extension_form_covers_extension_and_legacy_mqtt_only() {
        assert!(is_extension_form_type(&ConnectionType::Extension));
        assert!(is_extension_form_type(&ConnectionType::Mqtt));
        assert!(!is_extension_form_type(&ConnectionType::Redis));
    }

    #[test]
    fn legacy_mqtt_connection_is_editable_through_the_extension_form() {
        let legacy = StoredConnection::new_mqtt("旧 MQTT".to_string(), MqttParams::default(), None);

        let migrated = editable_extension_connection(&legacy)
            .expect("旧内置 MQTT 连接应迁移为可编辑的扩展连接");

        assert_eq!(ConnectionType::Extension, migrated.connection_type);
        let params: ExtensionConnectionParams = migrated
            .to_extension_params()
            .expect("迁移后应产出扩展参数");
        assert_eq!(MQTT_EXTENSION_ID, params.extension_id);
        assert_eq!("mqtt", params.contribution_id);
        // 保留原行 id:保存时更新原连接而不是新建。
        assert_eq!(legacy.id, migrated.id);
    }

    #[test]
    fn non_extension_connection_is_not_editable_through_the_extension_form() {
        let mut connection =
            StoredConnection::new_mqtt("x".to_string(), MqttParams::default(), None);
        connection.connection_type = ConnectionType::Redis;

        assert!(editable_extension_connection(&connection).is_none());
    }

    #[test]
    fn unparsable_legacy_mqtt_is_rejected_instead_of_reported_as_editable() {
        let mut legacy = StoredConnection::new_mqtt("bad".to_string(), MqttParams::default(), None);
        legacy.params = "{not json".to_string();

        assert!(editable_extension_connection(&legacy).is_none());
    }
}
