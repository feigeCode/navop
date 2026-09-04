use db::ipc::IpcDriverRegistry;
use db_view::connection_form_window::{ConnectionFormWindow, ConnectionFormWindowConfig};
use gpui::{AnyView, AnyWindowHandle, AppContext, Context, Entity, Window};
use mongodb_view::{MongoFormWindow, MongoFormWindowConfig};
use mqtt_view::{MqttFormConfig, MqttFormWindow};
use one_core::cloud_sync::get_cached_team_options;
use one_core::storage::{ConnectionType, DatabaseType, RemoteDesktopProtocol};
use port_forwarding_view::{PortForwardingFormWindow, PortForwardingFormWindowConfig};
use redis_view::{RedisFormWindow, RedisFormWindowConfig};
use terminal_view::{
    SerialFormWindow, SerialFormWindowConfig, SshFormWindow, SshFormWindowConfig, TelnetFormWindow,
    TelnetFormWindowConfig,
};

#[cfg(feature = "shell-plugins")]
use crate::extension_connection_form::{ExtensionConnectionForm, ExtensionConnectionFormConfig};
use crate::home_tab::HomePage;
use crate::new_connection::NewConnectionWindow;
use crate::new_connection::connection_kind::NewConnectionKind;
use remote_desktop_view::remote_desktop_form::{
    RemoteDesktopFormWindow, RemoteDesktopFormWindowConfig,
};

pub(crate) enum NewConnectionFormResult {
    Form(AnyView),
    Done,
    Blocked,
}

pub(crate) trait NewConnectionFormPage {
    fn build_form_view(
        self,
        parent: Entity<HomePage>,
        parent_window: AnyWindowHandle,
        external_driver_registry: &IpcDriverRegistry,
        window: &mut Window,
        cx: &mut Context<NewConnectionWindow>,
    ) -> NewConnectionFormResult;
}

impl NewConnectionFormPage for NewConnectionKind {
    fn build_form_view(
        self,
        parent: Entity<HomePage>,
        parent_window: AnyWindowHandle,
        external_driver_registry: &IpcDriverRegistry,
        window: &mut Window,
        cx: &mut Context<NewConnectionWindow>,
    ) -> NewConnectionFormResult {
        match self {
            Self::Ssh => build_ssh_form(parent, window, cx),
            Self::Rdp => build_remote_desktop_form(parent, RemoteDesktopProtocol::Rdp, window, cx),
            Self::Vnc => build_remote_desktop_form(parent, RemoteDesktopProtocol::Vnc, window, cx),
            Self::Redis => build_redis_form(parent, window, cx),
            Self::MongoDB => build_mongo_form(parent, window, cx),
            Self::Mqtt => build_mqtt_form(parent, window, cx),
            Self::Serial => build_serial_form(parent, window, cx),
            Self::Telnet => build_telnet_form(parent, window, cx),
            Self::PortForwarding => build_port_forwarding_form(parent, window, cx),
            Self::MoreConnections => open_extensions_tab(parent, parent_window, cx),
            Self::Database(db_type) => {
                build_database_form(parent, db_type, None, external_driver_registry, window, cx)
            }
            Self::ExternalDatabase { driver_id, .. } => {
                let db_type = DatabaseType::external(driver_id.clone());
                build_database_form(
                    parent,
                    db_type,
                    Some(driver_id),
                    external_driver_registry,
                    window,
                    cx,
                )
            }
            #[cfg(feature = "shell-plugins")]
            Self::Extension(contribution) => build_extension_form(parent, contribution, window, cx),
            #[cfg(not(feature = "shell-plugins"))]
            Self::Extension(_) => NewConnectionFormResult::Blocked,
        }
    }
}

#[cfg(feature = "shell-plugins")]
fn build_extension_form(
    parent: Entity<HomePage>,
    contribution: extension_runtime::RegisteredResourceConnectionContribution,
    window: &mut Window,
    cx: &mut Context<NewConnectionWindow>,
) -> NewConnectionFormResult {
    let teams = get_cached_team_options(cx);
    let Some(config) = parent.update(cx, |home, _| {
        if home.editing_connection_id.is_none() && !home.is_master_key_ready_for_new_connection() {
            return None;
        }
        let editing_connection = home.editing_connection_id.and_then(|id| {
            home.connections
                .iter()
                .find(|connection| {
                    connection.id == Some(id)
                        && connection.connection_type == ConnectionType::Extension
                })
                .cloned()
        });
        home.editing_connection_id = None;
        Some(ExtensionConnectionFormConfig {
            contribution,
            editing_connection,
            workspaces: home.workspaces.clone(),
            teams,
        })
    }) else {
        return NewConnectionFormResult::Blocked;
    };
    NewConnectionFormResult::Form(
        cx.new(|cx| ExtensionConnectionForm::new(config, window, cx))
            .into(),
    )
}

fn build_port_forwarding_form(
    parent: Entity<HomePage>,
    window: &mut Window,
    cx: &mut Context<NewConnectionWindow>,
) -> NewConnectionFormResult {
    let Some(config) = parent.update(cx, |home, cx| {
        if !home.is_master_key_ready_for_new_connection() {
            return None;
        }

        let editing_connection = home.editing_connection_id.and_then(|id| {
            home.connections
                .iter()
                .find(|c| c.id == Some(id) && c.connection_type == ConnectionType::PortForwarding)
                .cloned()
        });
        let ssh_connections = home
            .connections
            .iter()
            .filter(|connection| connection.connection_type == ConnectionType::SshSftp)
            .cloned()
            .collect();
        home.editing_connection_id = None;
        Some(PortForwardingFormWindowConfig {
            editing_connection,
            ssh_connections,
            workspaces: home.workspaces.clone(),
            teams: get_cached_team_options(cx),
        })
    }) else {
        return NewConnectionFormResult::Blocked;
    };

    NewConnectionFormResult::Form(
        cx.new(|cx| PortForwardingFormWindow::new(config, window, cx))
            .into(),
    )
}

fn build_remote_desktop_form(
    parent: Entity<HomePage>,
    protocol: RemoteDesktopProtocol,
    window: &mut Window,
    cx: &mut Context<NewConnectionWindow>,
) -> NewConnectionFormResult {
    let Some(config) = parent.update(cx, |home, cx| {
        if !home.is_master_key_ready_for_new_connection() {
            return None;
        }
        let connection_type = protocol.connection_type();
        let editing_connection = home.editing_connection_id.and_then(|id| {
            home.connections
                .iter()
                .find(|c| c.id == Some(id) && c.connection_type == connection_type)
                .cloned()
        });
        home.editing_connection_id = None;
        Some(RemoteDesktopFormWindowConfig {
            protocol,
            editing_connection,
            workspaces: home.workspaces.clone(),
            teams: get_cached_team_options(cx),
        })
    }) else {
        return NewConnectionFormResult::Blocked;
    };

    NewConnectionFormResult::Form(
        cx.new(|cx| RemoteDesktopFormWindow::new(config, window, cx))
            .into(),
    )
}

fn open_extensions_tab(
    parent: Entity<HomePage>,
    parent_window: AnyWindowHandle,
    cx: &mut Context<NewConnectionWindow>,
) -> NewConnectionFormResult {
    let _ = parent_window.update(cx, |_, window, cx| {
        let _ = parent.update(cx, |home, cx| {
            home.add_extensions_tab(window, cx);
        });
    });
    NewConnectionFormResult::Done
}

fn build_database_form(
    parent: Entity<HomePage>,
    db_type: DatabaseType,
    external_driver_id: Option<String>,
    external_driver_registry: &IpcDriverRegistry,
    window: &mut Window,
    cx: &mut Context<NewConnectionWindow>,
) -> NewConnectionFormResult {
    if let Some(driver_id) = external_driver_id.as_deref() {
        if external_driver_registry.find(driver_id).is_none() {
            extension_runtime::database_driver_install::prompt_install_database_driver(
                driver_id.to_string(),
                driver_id.to_string(),
                window,
                cx,
            );
            return NewConnectionFormResult::Blocked;
        }
    }

    let Some(config) = parent.update(cx, |home, cx| {
        if !home.is_master_key_ready_for_new_connection() {
            return None;
        }

        let editing_connection = home
            .editing_connection_id
            .and_then(|id| home.connections.iter().find(|c| c.id == Some(id)).cloned());
        let ssh_connections = home
            .connections
            .iter()
            .filter(|connection| connection.connection_type == ConnectionType::SshSftp)
            .cloned()
            .collect();
        home.editing_connection_id = None;
        Some(ConnectionFormWindowConfig {
            db_type,
            external_driver_id: external_driver_id.clone(),
            external_driver_registry: external_driver_registry.clone(),
            editing_connection,
            initial_connection: None,
            on_saved: None,
            workspaces: home.workspaces.clone(),
            teams: get_cached_team_options(cx),
            ssh_connections,
        })
    }) else {
        return NewConnectionFormResult::Blocked;
    };

    NewConnectionFormResult::Form(
        cx.new(|cx| ConnectionFormWindow::new(config, window, cx))
            .into(),
    )
}

fn build_ssh_form(
    parent: Entity<HomePage>,
    window: &mut Window,
    cx: &mut Context<NewConnectionWindow>,
) -> NewConnectionFormResult {
    let Some(config) = parent.update(cx, |home, cx| {
        if !home.is_master_key_ready_for_new_connection() {
            return None;
        }

        let editing_connection = home.editing_connection_id.and_then(|id| {
            home.connections
                .iter()
                .find(|c| c.id == Some(id) && c.connection_type == ConnectionType::SshSftp)
                .cloned()
        });
        home.editing_connection_id = None;
        Some(SshFormWindowConfig {
            editing_connection,
            initial_connection: None,
            on_saved: None,
            workspaces: home.workspaces.clone(),
            teams: get_cached_team_options(cx),
        })
    }) else {
        return NewConnectionFormResult::Blocked;
    };

    NewConnectionFormResult::Form(cx.new(|cx| SshFormWindow::new(config, window, cx)).into())
}

fn build_redis_form(
    parent: Entity<HomePage>,
    window: &mut Window,
    cx: &mut Context<NewConnectionWindow>,
) -> NewConnectionFormResult {
    let Some(config) = parent.update(cx, |home, cx| {
        if !home.is_master_key_ready_for_new_connection() {
            return None;
        }

        let editing_connection = home.editing_connection_id.and_then(|id| {
            home.connections
                .iter()
                .find(|c| c.id == Some(id) && c.connection_type == ConnectionType::Redis)
                .cloned()
        });
        let ssh_connections = home
            .connections
            .iter()
            .filter(|connection| connection.connection_type == ConnectionType::SshSftp)
            .cloned()
            .collect();
        home.editing_connection_id = None;
        Some(RedisFormWindowConfig {
            editing_connection,
            initial_connection: None,
            on_saved: None,
            workspaces: home.workspaces.clone(),
            teams: get_cached_team_options(cx),
            ssh_connections,
        })
    }) else {
        return NewConnectionFormResult::Blocked;
    };

    NewConnectionFormResult::Form(cx.new(|cx| RedisFormWindow::new(config, window, cx)).into())
}

fn build_mongo_form(
    parent: Entity<HomePage>,
    window: &mut Window,
    cx: &mut Context<NewConnectionWindow>,
) -> NewConnectionFormResult {
    let Some(config) = parent.update(cx, |home, cx| {
        if !home.is_master_key_ready_for_new_connection() {
            return None;
        }

        let editing_connection = home.editing_connection_id.and_then(|id| {
            home.connections
                .iter()
                .find(|c| c.id == Some(id) && c.connection_type == ConnectionType::MongoDB)
                .cloned()
        });
        home.editing_connection_id = None;
        let ssh_connections = home.connections.clone();
        Some(MongoFormWindowConfig {
            editing_connection,
            initial_connection: None,
            on_saved: None,
            workspaces: home.workspaces.clone(),
            teams: get_cached_team_options(cx),
            ssh_connections,
        })
    }) else {
        return NewConnectionFormResult::Blocked;
    };

    NewConnectionFormResult::Form(cx.new(|cx| MongoFormWindow::new(config, window, cx)).into())
}

fn build_mqtt_form(
    parent: Entity<HomePage>,
    window: &mut Window,
    cx: &mut Context<NewConnectionWindow>,
) -> NewConnectionFormResult {
    let Some(config) = parent.update(cx, |home, cx| {
        if !home.is_master_key_ready_for_new_connection() {
            return None;
        }

        let editing_connection = home.editing_connection_id.and_then(|id| {
            home.connections
                .iter()
                .find(|c| c.id == Some(id) && c.connection_type == ConnectionType::Mqtt)
                .cloned()
        });
        home.editing_connection_id = None;
        let ssh_connections = home.connections.clone();
        Some(MqttFormConfig {
            editing_connection,
            initial_connection: None,
            workspaces: home.workspaces.clone(),
            teams: get_cached_team_options(cx),
            ssh_connections,
            on_saved: None,
        })
    }) else {
        return NewConnectionFormResult::Blocked;
    };

    NewConnectionFormResult::Form(
        cx.new(|cx| MqttFormWindow::new(config.into_window_config(), window, cx))
            .into(),
    )
}

fn build_serial_form(
    parent: Entity<HomePage>,
    window: &mut Window,
    cx: &mut Context<NewConnectionWindow>,
) -> NewConnectionFormResult {
    let Some(config) = parent.update(cx, |home, cx| {
        if !home.is_master_key_ready_for_new_connection() {
            return None;
        }

        let editing_connection = home.editing_connection_id.and_then(|id| {
            home.connections
                .iter()
                .find(|c| c.id == Some(id) && c.connection_type == ConnectionType::Serial)
                .cloned()
        });
        home.editing_connection_id = None;
        Some(SerialFormWindowConfig {
            editing_connection,
            workspaces: home.workspaces.clone(),
            teams: get_cached_team_options(cx),
        })
    }) else {
        return NewConnectionFormResult::Blocked;
    };

    NewConnectionFormResult::Form(
        cx.new(|cx| SerialFormWindow::new(config, window, cx))
            .into(),
    )
}

fn build_telnet_form(
    parent: Entity<HomePage>,
    window: &mut Window,
    cx: &mut Context<NewConnectionWindow>,
) -> NewConnectionFormResult {
    let Some(config) = parent.update(cx, |home, cx| {
        if !home.is_master_key_ready_for_new_connection() {
            return None;
        }

        let editing_connection = home.editing_connection_id.and_then(|id| {
            home.connections
                .iter()
                .find(|c| c.id == Some(id) && c.connection_type == ConnectionType::Telnet)
                .cloned()
        });
        home.editing_connection_id = None;
        Some(TelnetFormWindowConfig {
            editing_connection,
            workspaces: home.workspaces.clone(),
            teams: get_cached_team_options(cx),
        })
    }) else {
        return NewConnectionFormResult::Blocked;
    };

    NewConnectionFormResult::Form(
        cx.new(|cx| TelnetFormWindow::new(config, window, cx))
            .into(),
    )
}
