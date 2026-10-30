use super::*;

impl HomePage {
    pub(crate) fn show_connection_quick_open(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.show_connection_quick_open_with_query(String::new(), window, cx);
    }

    /// 打开快速连接对话框；`query` 预填搜索词（主页搜索框回车 ssh 命令时传入，
    /// 复用 quick open 的临时连接解析与确认逻辑）。
    pub(crate) fn show_connection_quick_open_with_query(
        &mut self,
        query: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.ensure_master_key_ready_for_saved_connections(window, cx) {
            return;
        }

        let parent = cx.entity();
        let connections = self.connections.clone();
        let list = cx.new(|cx| {
            let mut delegate = ConnectionQuickOpenDelegate::new(parent);
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
                                    .search_placeholder(t!("Home.search_placeholder").to_string())
                                    .with_size(Size::Large)
                                    .max_h(px(420.0)),
                            ),
                        )
                    }
                })
        });
        // 将焦点设置到 List 搜索框，使上下键和 Enter 键可用；预填搜索词
        // 会触发 delegate 的临时 ssh 解析，直接回车即可连接。
        list_for_focus.update(cx, |state, cx| {
            state.focus(window, cx);
            if !query.is_empty() {
                state.set_query(&query, window, cx);
            }
        });
    }

    /// 打开「新建连接」对话框。
    ///
    /// 连接类型与表单都由领域注册表提供：全量产品看到全部类型，独立产品只看到
    /// 自己领域注册的类型。这里不再维护产品内的向导页面。
    pub(crate) fn show_new_connection_dialog(
        &mut self,
        target_workspace: Option<i64>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        app_kit::connection_actions::open_new_connection_in_workspace(target_workspace, window, cx);
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
        if app_kit::registry().open_connection(&connection, open_mode, self, window, cx) {
            cx.notify();
            return;
        }
        let strategy = build_connection_open_strategy(connection, workspace);
        strategy.open(self, open_mode, window, cx);
        cx.notify();
    }

    #[cfg(feature = "shell")]
    pub(crate) fn open_remote_desktop_fullscreen_window(
        &mut self,
        connection: &StoredConnection,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(extras) = app_kit::registry().shell_extras() else {
            return;
        };
        // Windows 的 RDP 优先调用系统客户端（原生全屏体验）。
        #[cfg(target_os = "windows")]
        if connection.connection_type == ConnectionType::Rdp {
            if !self.ensure_master_key_ready_for_saved_connections(window, cx) {
                return;
            }
            let Some(connection) = resolve_connection_credentials(connection, window, cx) else {
                return;
            };
            match extras.launch_remote_desktop_external(&connection) {
                Ok(()) => {
                    self.selected_connection_id = connection.id;
                    self.touch_connection_last_used(connection.id, cx);
                    cx.notify();
                }
                Err(error) => {
                    tracing::warn!(
                        connection_id = ?connection.id,
                        "无法启动 Windows 远程桌面客户端: {error}"
                    );
                    window.push_notification(
                        Notification::error(
                            t!("Home.external_program_launch_failed", error = error).to_string(),
                        ),
                        cx,
                    );
                }
            }
            return;
        }

        if !self.ensure_master_key_ready_for_saved_connections(window, cx) {
            return;
        }
        let protocol = match connection.connection_type {
            ConnectionType::Rdp => domain_api::RemoteProtocol::Rdp,
            ConnectionType::Vnc => domain_api::RemoteProtocol::Vnc,
            _ => return,
        };
        let provider_protocol = match protocol {
            domain_api::RemoteProtocol::Rdp => remote_desktop::RemoteDesktopProtocol::Rdp,
            domain_api::RemoteProtocol::Vnc => remote_desktop::RemoteDesktopProtocol::Vnc,
        };
        let Some(connection) = resolve_connection_credentials(connection, window, cx) else {
            return;
        };
        self.selected_connection_id = connection.id;
        self.touch_connection_last_used(connection.id, cx);
        let title = connection.name.clone();
        extension_runtime::remote_desktop_provider_install::run_with_remote_desktop_provider_guard(
            self,
            connection.clone(),
            provider_protocol,
            window,
            cx,
            move |_, _, cx| {
                extras.open_remote_desktop_fullscreen(&connection, protocol, title, cx);
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
}

pub(crate) fn resolve_connection_credentials(
    connection: &StoredConnection,
    window: &mut Window,
    cx: &mut App,
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
                Notification::error(
                    t!(
                        "Connection.credential_resolve_failed",
                        error = format!("{error:#}")
                    )
                    .to_string(),
                )
                .autohide(true),
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
