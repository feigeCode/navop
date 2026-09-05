use super::*;

impl HomePage {
    pub(crate) fn show_connection_quick_open(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.ensure_master_key_ready_for_saved_connections(window, cx) {
            return;
        }

        let parent = cx.entity();
        let connections = self.connections.clone();
        let external_driver_registry = self.external_driver_registry.clone();
        let list = cx.new(|cx| {
            let mut delegate = ConnectionQuickOpenDelegate::new(parent, external_driver_registry);
            delegate.update_items(&connections);
            ListState::new(delegate, window, cx).searchable(true)
        });

        let list_for_focus = list.clone();
        window.open_dialog(cx, move |dialog, _window, _cx| {
            dialog
                .title(t!("Home.open_connection").to_string())
                .w(px(640.0))
                .margin_top(px(72.0))
                .close_button(false)
                .content({
                    let list = list.clone();
                    move |content, _window, _cx| {
                        content.p_0().child(
                            div().id("connection-quick-open-dialog").child(
                                List::new(&list)
                                    .search_placeholder(t!("Home.quick_open_placeholder").to_string())
                                    .with_size(Size::Large)
                                    .max_h(px(420.0)),
                            ),
                        )
                    }
                })
        });
        // 将焦点设置到 List 搜索框，使上下键和 Enter 键可用
        list_for_focus.update(cx, |state, cx| {
            state.focus(window, cx);
        });
    }

    pub(crate) fn show_new_connection_dialog(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.editing_connection_id = None;

        if !self.ensure_master_key_ready_for_new_connection(window, cx) {
            return;
        }

        let parent = cx.entity();
        let parent_window = window.window_handle();
        let external_driver_registry = self.external_driver_registry.clone();
        open_popup_window(
            PopupWindowOptions::new(t!("Home.new_connection").to_string()).size(1100.0, 700.0),
            move |window, cx| {
                cx.new(|cx| {
                    NewConnectionWindow::new(
                        parent,
                        parent_window,
                        external_driver_registry.clone(),
                        window,
                        cx,
                    )
                })
            },
            Some(window),
            cx,
        );
    }

    pub(crate) fn open_connection_from_quick(
        &mut self,
        connection: &StoredConnection,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.open_connection_from_quick_with_mode(connection, TabOpenMode::Activate, window, cx);
    }

    pub(crate) fn open_connection_from_quick_with_mode(
        &mut self,
        connection: &StoredConnection,
        open_mode: TabOpenMode,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.ensure_master_key_ready_for_saved_connections(window, cx) {
            return;
        }

        let Some(connection) = resolve_connection_credentials(connection, window, cx) else {
            return;
        };
        self.selected_connection_id = connection.id;
        self.touch_connection_last_used(connection.id, cx);
        let workspace = connection
            .workspace_id
            .and_then(|id| self.workspaces.iter().find(|w| w.id == Some(id)).cloned());
        let strategy = build_connection_open_strategy(connection, workspace);
        strategy.open(self, open_mode, window, cx);
        cx.notify();
    }

    pub(crate) fn open_remote_desktop_fullscreen_window(
        &mut self,
        connection: &StoredConnection,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        #[cfg(target_os = "windows")]
        {
            if connection.connection_type == ConnectionType::Rdp {
                if !self.ensure_master_key_ready_for_saved_connections(window, cx) {
                    return;
                }
                let Some(connection) = resolve_connection_credentials(connection, window, cx)
                else {
                    return;
                };
                let params = match connection.to_remote_desktop_params() {
                    Ok(params) => params,
                    Err(error) => {
                        tracing::warn!(
                            connection_id = ?connection.id,
                            ?error,
                            "无法解析远程桌面连接参数"
                        );
                        window.push_notification(
                            Notification::error(
                                t!(
                                    "Home.remote_desktop_parameters_invalid",
                                    error = format!("{error:#}")
                                )
                                .to_string(),
                            ),
                            cx,
                        );
                        return;
                    }
                };

                match crate::home::remote_desktop_window::launch_mstsc_fullscreen(&params) {
                    Ok(()) => {
                        self.selected_connection_id = connection.id;
                        self.touch_connection_last_used(connection.id, cx);
                        cx.notify();
                    }
                    Err(error) => {
                        tracing::warn!(
                            connection_id = ?connection.id,
                            ?error,
                            "无法启动 Windows 远程桌面客户端"
                        );
                        window.push_notification(
                            Notification::error(
                                t!(
                                    "Home.external_program_launch_failed",
                                    error = format!("{error:#}")
                                )
                                .to_string(),
                            ),
                            cx,
                        );
                    }
                }
                return;
            }
        }

        if !self.ensure_master_key_ready_for_saved_connections(window, cx) {
            return;
        }

        let protocol = match connection.connection_type {
            ConnectionType::Rdp => remote_desktop::RemoteDesktopProtocol::Rdp,
            ConnectionType::Vnc => remote_desktop::RemoteDesktopProtocol::Vnc,
            _ => return,
        };
        let Some(connection) = resolve_connection_credentials(connection, window, cx) else {
            return;
        };
        self.selected_connection_id = connection.id;
        self.touch_connection_last_used(connection.id, cx);
        extension_runtime::remote_desktop_provider_install::run_with_remote_desktop_provider_guard(
            self,
            connection.clone(),
            protocol,
            window,
            cx,
            move |_, _, cx| {
                let Some(options) =
                    crate::home::home_tabs::remote_desktop_options(&connection, protocol)
                else {
                    tracing::warn!(
                        connection_id = ?connection.id,
                        "无法解析远程桌面连接参数"
                    );
                    return;
                };
                crate::home::remote_desktop_window::open_remote_desktop_fullscreen_window(
                    options,
                    connection.name.clone(),
                    cx,
                );
            },
        );
        cx.notify();
    }

    pub(super) fn touch_connection_last_used(
        &mut self,
        connection_id: Option<i64>,
        cx: &mut Context<Self>,
    ) {
        let Some(connection_id) = connection_id else {
            return;
        };
        let storage = cx.global::<GlobalStorageState>().storage.clone();
        let result = storage
            .get::<ConnectionRepository>()
            .ok_or_else(|| anyhow::anyhow!("ConnectionRepository not found"))
            .and_then(|repo| repo.touch_last_used(connection_id));

        if let Err(err) = result {
            tracing::warn!("更新连接最近使用时间失败: {err}");
            return;
        }
        self.load_connections(cx);
    }

    /// 把连接从"最近使用"列表移除（仅清空最近使用时间，不删除连接本身）。
    pub(super) fn remove_recent_connection(
        &mut self,
        connection_id: Option<i64>,
        cx: &mut Context<Self>,
    ) {
        let Some(connection_id) = connection_id else {
            return;
        };
        let storage = cx.global::<GlobalStorageState>().storage.clone();
        let result = storage
            .get::<ConnectionRepository>()
            .ok_or_else(|| anyhow::anyhow!("ConnectionRepository not found"))
            .and_then(|repo| repo.clear_last_used(connection_id));

        if let Err(err) = result {
            tracing::warn!("清除连接最近使用时间失败: {err}");
            return;
        }
        self.load_connections(cx);
    }
}

pub(crate) fn resolve_connection_credentials(
    connection: &StoredConnection,
    window: &mut Window,
    cx: &mut Context<HomePage>,
) -> Option<StoredConnection> {
    let repository = cx
        .global::<GlobalStorageState>()
        .storage
        .get::<ConnectionRepository>();
    let result = repository
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("ConnectionRepository not found"))
        .and_then(|repository| repository.resolve_runtime_connection(connection));
    match result {
        Ok(connection) => Some(connection),
        Err(error) => {
            if let (Some(repository), Some(credential_id)) =
                (repository.as_ref(), missing_credential_id(&error))
            {
                match temporary_missing_credential_connection(repository, connection, credential_id)
                {
                    Ok(connection) => {
                        tracing::warn!(
                            connection_id = ?connection.id,
                            "当前设备缺少连接引用的钥匙串，改为仅本次连接输入凭据"
                        );
                        return Some(connection);
                    }
                    Err(fallback_error) => {
                        tracing::warn!(
                            connection_id = ?connection.id,
                            error = %fallback_error,
                            "无法创建缺失钥匙串的临时连接参数"
                        );
                    }
                }
            }
            tracing::warn!(
                connection_id = ?connection.id,
                error = %error,
                "无法解析连接引用的凭据"
            );
            window.push_notification(
                Notification::error(format!("无法解析连接凭据：{error:#}")).autohide(true),
                cx,
            );
            None
        }
    }
}

fn missing_credential_id(error: &anyhow::Error) -> Option<i64> {
    error.chain().find_map(|cause| {
        let CredentialResolutionError::MissingCredential(credential_id) =
            cause.downcast_ref::<CredentialResolutionError>()?
        else {
            return None;
        };
        Some(*credential_id)
    })
}

fn temporary_missing_credential_connection(
    repository: &ConnectionRepository,
    connection: &StoredConnection,
    missing_credential_id: i64,
) -> anyhow::Result<StoredConnection> {
    let mut runtime = connection.clone();
    runtime.params = match connection.connection_type {
        ConnectionType::SshSftp => {
            let mut params = connection.to_ssh_params()?;
            let Some(reference) = params.credential_reference.as_ref() else {
                anyhow::bail!("the missing credential is not the primary SSH credential");
            };
            if reference.credential_id != missing_credential_id {
                anyhow::bail!("the missing credential belongs to an SSH proxy or jump server");
            }
            params.credential_reference = None;
            params.username.clear();
            params.auth_method = SshAuthMethod::Password {
                password: String::new(),
            };
            params.prompt_username = Some(true);
            params.prompt_password = Some(true);
            let params = repository.credential_repository().resolve_ssh(params)?;
            serde_json::to_string(&params)?
        }
        ConnectionType::Telnet => {
            let mut params = connection.to_telnet_params()?;
            let Some(reference) = params.credential_reference.as_ref() else {
                anyhow::bail!("the missing credential is not the Telnet login credential");
            };
            if reference.credential_id != missing_credential_id {
                anyhow::bail!("the missing credential does not belong to this Telnet connection");
            }
            params.credential_reference = None;
            if params.login_script.is_empty() {
                params.login_script = vec![
                    TelnetLoginStep {
                        expect: r"(?i)(?:login|username|user\s*name|account)\s*[:>]\s*$"
                            .to_string(),
                        send: String::new(),
                    },
                    TelnetLoginStep {
                        expect: r"(?i)(?:password|passwd|passcode)\s*[:>]\s*$".to_string(),
                        send: String::new(),
                    },
                ];
            }
            let (prompt_username, prompt_password) = params.login_credential_prompt_fields();
            if !prompt_username && !prompt_password {
                anyhow::bail!(
                    "unable to identify username or password prompts in the Telnet expect script"
                );
            }
            params.prompt_username = prompt_username.then_some(true);
            params.prompt_password = prompt_password.then_some(true);
            serde_json::to_string(&params)?
        }
        _ => anyhow::bail!(
            "temporary credential input is unsupported for {:?}",
            connection.connection_type
        ),
    };
    Ok(runtime)
}
