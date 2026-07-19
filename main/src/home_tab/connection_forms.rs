use super::*;

impl HomePage {
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
        if let Some(driver_id) =
            external_driver_id_for_connection_form(&db_type, editing_conn.as_ref())
        {
            if self.external_driver_registry.find(&driver_id).is_none() {
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
        let external_driver_id = external_driver_id_for_connection_form(
            &config.db_type,
            config.editing_connection.as_ref(),
        );
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
        open_popup_window(
            PopupWindowOptions::new(title).size(700.0, 650.0),
            move |window, cx| cx.new(|cx| ConnectionFormWindow::new(config, window, cx)),
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
            PopupWindowOptions::new(title).size(700.0, 650.0),
            move |window, cx| cx.new(|cx| SshFormWindow::new(config, window, cx)),
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
            cx,
        );
    }
}
