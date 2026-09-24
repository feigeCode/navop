use super::*;

const ORACLE_GO_DRIVER_ID: &str = "oracle-go";

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
            contribution,
            editing_connection: Some(connection.clone()),
            workspaces: self.workspaces.clone(),
            teams: get_cached_team_options(cx),
        };
        self.editing_connection_id = None;
        open_popup_window(
            PopupWindowOptions::new(format!("Edit {}", connection.name)).size(700.0, 650.0),
            move |window, cx| {
                cx.new(|cx| universal_plugins::ExtensionConnectionForm::new(config, window, cx))
            },
            Some(window),
            cx,
        );
    }

    pub(super) fn external_driver_name_for_title(
        driver_id: Option<&str>,
        registry: &IpcDriverRegistry,
    ) -> Option<String> {
        driver_id.and_then(|driver_id| registry.find(driver_id).map(|driver| driver.name))
    }

    pub(super) fn connection_title_for_locale(
        locale: &str,
        is_editing: bool,
        db_type: &DatabaseType,
        connection_name: Option<&str>,
        external_driver_name: Option<&str>,
    ) -> String {
        let db_type_label = connection_name
            .filter(|name| is_editing && !name.trim().is_empty())
            .or_else(|| external_driver_name.filter(|name| !name.trim().is_empty()))
            .unwrap_or_else(|| db_type.as_str());

        db::translate_connection_title_for_locale(locale, is_editing, db_type_label)
    }

    pub(super) fn editing_title_or_default(
        locale: &str,
        editing_connection: Option<&StoredConnection>,
        default_title: String,
    ) -> String {
        editing_connection
            .and_then(|connection| non_empty_name(&connection.name))
            .map(|name| db::translate_connection_title_for_locale(locale, true, name))
            .unwrap_or(default_title)
    }

    pub(super) fn typed_connection_title_for_locale(
        locale: &str,
        is_editing: bool,
        type_label: &str,
        editing_connection: Option<&StoredConnection>,
    ) -> String {
        let label = editing_connection
            .filter(|_| is_editing)
            .and_then(|connection| non_empty_name(&connection.name))
            .unwrap_or(type_label);
        db::translate_connection_title_for_locale(locale, is_editing, label)
    }
    pub(crate) fn show_connection_form(
        &mut self,
        db_type: DatabaseType,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.editing_connection_id.is_none() && !self.is_master_key_ready_for_new_connection() {
            return;
        }

        let editing_conn = self
            .editing_connection_id
            .and_then(|id| self.connections.iter().find(|c| c.id == Some(id)).cloned());
        let external_driver_id =
            external_driver_id_for_connection_form(&db_type, editing_conn.as_ref());
        let is_oracle_form = db_type == DatabaseType::Oracle
            || external_driver_id.as_deref() == Some(ORACLE_GO_DRIVER_ID);
        if let Some(driver_id) = external_driver_id.clone() {
            if driver_id != ORACLE_GO_DRIVER_ID
                && self.external_driver_registry.find(&driver_id).is_none()
            {
                let connection_name = editing_conn
                    .as_ref()
                    .map(|connection| connection.name.clone())
                    .unwrap_or_else(|| driver_id.clone());
                extension_runtime::database_driver_install::prompt_install_database_driver(
                    driver_id,
                    connection_name,
                    window,
                    cx,
                );
                return;
            }
        }
        let ssh_connections = self
            .connections
            .iter()
            .filter(|connection| connection.connection_type == ConnectionType::SshSftp)
            .cloned()
            .collect();

        let config = ConnectionFormWindowConfig {
            db_type: db_type.clone(),
            external_driver_id: None,
            external_driver_registry: self.external_driver_registry.clone(),
            editing_connection: editing_conn,
            initial_connection: None,
            on_saved: None,
            workspaces: self.workspaces.clone(),
            teams: get_cached_team_options(cx),
            ssh_connections,
        };

        self.editing_connection_id = None;
        let external_driver_name = Self::external_driver_name_for_title(
            external_driver_id.as_deref(),
            &config.external_driver_registry,
        );
        let title = Self::connection_title_for_locale(
            rust_i18n::locale().as_ref(),
            config.editing_connection.is_some(),
            &config.db_type,
            config
                .editing_connection
                .as_ref()
                .map(|connection| connection.name.as_str()),
            external_driver_name.as_deref(),
        );
        let popup_height = if is_oracle_form && config.editing_connection.is_some() {
            720.0
        } else {
            650.0
        };
        open_popup_window(
            PopupWindowOptions::new(title).size(700.0, popup_height),
            move |window, cx| cx.new(|cx| ConnectionFormWindow::new(config, window, cx)),
            Some(window),
            cx,
        );
    }

    /// 把临时连接保存为正式连接。
    ///
    /// 连接信息已由终端侧补全运行时用户名 / 密码，保存时会一并写入凭据库。
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

        open_popup_window(
            PopupWindowOptions::new(t!("Home.save_as_connection").to_string()).size(820.0, 750.0),
            move |window, cx| cx.new(|cx| SshFormWindow::new(config, window, cx)),
            Some(window),
            cx,
        );
    }

    pub(crate) fn show_ssh_form(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        if self.editing_connection_id.is_none() && !self.is_master_key_ready_for_new_connection() {
            return;
        }

        let editing_conn = self.editing_connection_id.and_then(|id| {
            self.connections
                .iter()
                .find(|c| c.id == Some(id) && c.connection_type == ConnectionType::SshSftp)
                .cloned()
        });

        let config = SshFormWindowConfig {
            editing_connection: editing_conn,
            initial_connection: None,
            on_saved: None,
            workspaces: self.workspaces.clone(),
            teams: get_cached_team_options(cx),
        };

        self.editing_connection_id = None;

        let title = Self::editing_title_or_default(
            rust_i18n::locale().as_ref(),
            config.editing_connection.as_ref(),
            if config.editing_connection.is_some() {
                t!("SSH.edit").to_string()
            } else {
                t!("SSH.new").to_string()
            },
        );
        open_popup_window(
            PopupWindowOptions::new(title).size(820.0, 750.0),
            move |window, cx| cx.new(|cx| SshFormWindow::new(config, window, cx)),
            Some(_window),
            cx,
        );
    }

    pub(crate) fn show_redis_form(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        if self.editing_connection_id.is_none() && !self.is_master_key_ready_for_new_connection() {
            return;
        }

        let editing_conn = self.editing_connection_id.and_then(|id| {
            self.connections
                .iter()
                .find(|c| c.id == Some(id) && c.connection_type == ConnectionType::Redis)
                .cloned()
        });

        let config = RedisFormWindowConfig {
            editing_connection: editing_conn,
            initial_connection: None,
            on_saved: None,
            workspaces: self.workspaces.clone(),
            teams: get_cached_team_options(cx),
            ssh_connections: self
                .connections
                .iter()
                .filter(|connection| connection.connection_type == ConnectionType::SshSftp)
                .cloned()
                .collect(),
        };

        self.editing_connection_id = None;

        let title = Self::typed_connection_title_for_locale(
            rust_i18n::locale().as_ref(),
            config.editing_connection.is_some(),
            "Redis",
            config.editing_connection.as_ref(),
        );
        open_popup_window(
            PopupWindowOptions::new(title).size(700.0, 650.0),
            move |window, cx| cx.new(|cx| RedisFormWindow::new(config, window, cx)),
            Some(_window),
            cx,
        );
    }

    pub(crate) fn show_ftp_form(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        if self.editing_connection_id.is_none() && !self.is_master_key_ready_for_new_connection() {
            return;
        }

        let editing_conn = self.editing_connection_id.and_then(|id| {
            self.connections
                .iter()
                .find(|c| c.id == Some(id) && c.connection_type == ConnectionType::Ftp)
                .cloned()
        });

        let config = FtpFormWindowConfig {
            editing_connection: editing_conn,
            workspaces: self.workspaces.clone(),
            teams: get_cached_team_options(cx),
        };

        self.editing_connection_id = None;

        let title = Self::editing_title_or_default(
            rust_i18n::locale().as_ref(),
            config.editing_connection.as_ref(),
            if config.editing_connection.is_some() {
                t!("Ftp.edit").to_string()
            } else {
                t!("Ftp.new").to_string()
            },
        );
        open_popup_window(
            PopupWindowOptions::new(title).size(700.0, 650.0),
            move |window, cx| cx.new(|cx| FtpFormWindow::new(config, window, cx)),
            Some(_window),
            cx,
        );
    }

    pub(crate) fn show_mongodb_form(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        if self.editing_connection_id.is_none() && !self.is_master_key_ready_for_new_connection() {
            return;
        }

        let editing_conn = self.editing_connection_id.and_then(|id| {
            self.connections
                .iter()
                .find(|c| c.id == Some(id) && c.connection_type == ConnectionType::MongoDB)
                .cloned()
        });

        let config = MongoFormWindowConfig {
            editing_connection: editing_conn,
            initial_connection: None,
            on_saved: None,
            workspaces: self.workspaces.clone(),
            teams: get_cached_team_options(cx),
            ssh_connections: self.connections.clone(),
        };

        self.editing_connection_id = None;

        let title = Self::typed_connection_title_for_locale(
            rust_i18n::locale().as_ref(),
            config.editing_connection.is_some(),
            "MongoDB",
            config.editing_connection.as_ref(),
        );
        open_popup_window(
            PopupWindowOptions::new(title).size(700.0, 650.0),
            move |window, cx| cx.new(|cx| MongoFormWindow::new(config, window, cx)),
            Some(_window),
            cx,
        );
    }

    pub(crate) fn show_serial_form(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        if self.editing_connection_id.is_none() && !self.is_master_key_ready_for_new_connection() {
            return;
        }

        let editing_conn = self.editing_connection_id.and_then(|id| {
            self.connections
                .iter()
                .find(|c| c.id == Some(id) && c.connection_type == ConnectionType::Serial)
                .cloned()
        });

        let config = SerialFormWindowConfig {
            editing_connection: editing_conn,
            workspaces: self.workspaces.clone(),
            teams: get_cached_team_options(cx),
        };

        self.editing_connection_id = None;

        let title = Self::editing_title_or_default(
            rust_i18n::locale().as_ref(),
            config.editing_connection.as_ref(),
            if config.editing_connection.is_some() {
                t!("Serial.edit").to_string()
            } else {
                t!("Serial.new").to_string()
            },
        );
        open_popup_window(
            PopupWindowOptions::new(title).size(700.0, 600.0),
            move |window, cx| cx.new(|cx| SerialFormWindow::new(config, window, cx)),
            Some(_window),
            cx,
        );
    }

    pub(crate) fn show_telnet_form(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        if self.editing_connection_id.is_none() && !self.is_master_key_ready_for_new_connection() {
            return;
        }

        let editing_conn = self.editing_connection_id.and_then(|id| {
            self.connections
                .iter()
                .find(|c| c.id == Some(id) && c.connection_type == ConnectionType::Telnet)
                .cloned()
        });

        let config = TelnetFormWindowConfig {
            editing_connection: editing_conn,
            workspaces: self.workspaces.clone(),
            teams: get_cached_team_options(cx),
        };

        self.editing_connection_id = None;

        let title = Self::editing_title_or_default(
            rust_i18n::locale().as_ref(),
            config.editing_connection.as_ref(),
            if config.editing_connection.is_some() {
                t!("Telnet.edit").to_string()
            } else {
                t!("Telnet.new").to_string()
            },
        );
        open_popup_window(
            PopupWindowOptions::new(title).size(700.0, 600.0),
            move |window, cx| cx.new(|cx| TelnetFormWindow::new(config, window, cx)),
            Some(_window),
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
