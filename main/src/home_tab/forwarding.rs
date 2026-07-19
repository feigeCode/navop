use super::*;

impl HomePage {
    pub(crate) fn show_port_forwarding_form(
        &mut self,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.editing_connection_id.is_none() && !self.is_master_key_ready_for_new_connection() {
            return;
        }

        let editing_connection = self.editing_connection_id.and_then(|id| {
            self.connections
                .iter()
                .find(|c| c.id == Some(id) && c.connection_type == ConnectionType::PortForwarding)
                .cloned()
        });
        let ssh_connections = self
            .connections
            .iter()
            .filter(|connection| connection.connection_type == ConnectionType::SshSftp)
            .cloned()
            .collect();

        let config = PortForwardingFormWindowConfig {
            editing_connection,
            ssh_connections,
            workspaces: self.workspaces.clone(),
            teams: get_cached_team_options(cx),
        };

        self.editing_connection_id = None;

        let title = Self::editing_title_or_default(
            rust_i18n::locale().as_ref(),
            config.editing_connection.as_ref(),
            if config.editing_connection.is_some() {
                t!("PortForwarding.edit").to_string()
            } else {
                t!("PortForwarding.new").to_string()
            },
        );
        open_popup_window(
            PopupWindowOptions::new(title).size(700.0, 520.0),
            move |window, cx| cx.new(|cx| PortForwardingFormWindow::new(config, window, cx)),
            cx,
        );
    }

    pub(crate) fn open_port_forwarding_tab(
        &mut self,
        connection: StoredConnection,
        mode: TabOpenMode,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let connection_name = connection.name.clone();
        let (tab_id, config) = match self.port_forwarding_tab_config(connection) {
            Ok(result) => result,
            Err(error) => {
                let message = t!(
                    "Home.port_forwarding_failed",
                    name = connection_name,
                    error = error.to_string()
                );
                window.push_notification(message.to_string(), cx);
                return;
            }
        };
        self.tab_container.update(cx, |container, cx| {
            let item_id = tab_id.clone();
            container.activate_or_add_tab_lazy_with_mode(
                tab_id,
                mode,
                move |_window, cx| {
                    let tab = cx.new(|cx| PortForwardingTab::new(config, cx));
                    TabItem::new(item_id, "home", tab)
                },
                window,
                cx,
            );
        });
    }

    pub(super) fn port_forwarding_tab_config(
        &self,
        connection: StoredConnection,
    ) -> anyhow::Result<(String, PortForwardingTabConfig)> {
        let connection_id = connection
            .id
            .ok_or_else(|| anyhow::anyhow!("missing connection id"))?;
        let params = connection.to_port_forwarding_params()?;
        let ssh_connection = self
            .connections
            .iter()
            .find(|conn| conn.id == Some(params.ssh_connection_id))
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("referenced SSH connection is missing"))?;
        let config = PortForwardingTabConfig::new(
            connection,
            ssh_connection,
            Arc::clone(&self.port_forwarding_runtime),
        )?;
        Ok((format!("port-forwarding-{connection_id}"), config))
    }

    pub(crate) fn show_remote_desktop_form(
        &mut self,
        protocol: StoredRemoteDesktopProtocol,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.editing_connection_id.is_none() && !self.is_master_key_ready_for_new_connection() {
            return;
        }

        let connection_type = protocol.connection_type();
        let editing_conn = self.editing_connection_id.and_then(|id| {
            self.connections
                .iter()
                .find(|c| c.id == Some(id) && c.connection_type == connection_type)
                .cloned()
        });

        let config = RemoteDesktopFormWindowConfig {
            protocol,
            editing_connection: editing_conn,
            workspaces: self.workspaces.clone(),
            teams: get_cached_team_options(cx),
        };

        self.editing_connection_id = None;

        let title = Self::editing_title_or_default(
            rust_i18n::locale().as_ref(),
            config.editing_connection.as_ref(),
            if config.editing_connection.is_some() {
                t!("RemoteDesktopForm.title_edit", protocol = protocol.label()).to_string()
            } else {
                t!("RemoteDesktopForm.title_new", protocol = protocol.label()).to_string()
            },
        );
        open_popup_window(
            PopupWindowOptions::new(title).size(700.0, 560.0),
            move |window, cx| cx.new(|cx| RemoteDesktopFormWindow::new(config, window, cx)),
            cx,
        );
    }
}
