use crate::endpoint::{LeftEndpointValue, load_connection};
use crate::host_key_prompt::{HostKeyPromptTarget, host_key_prompt_request};
use crate::left_remote_state::{LeftRemoteConnectionState, LeftRemoteEndpoint};
use crate::{FileItem, SftpView, disconnect_sftp_client, format_permissions};
use ftp::FtpClient;
use gpui::{AppContext, AsyncApp, Context, WeakEntity, Window};
use gpui_component::{WindowExt, notification::Notification};
use one_core::gpui_tokio::Tokio;
use one_core::storage::ActiveConnections;
use rust_i18n::t;
use sftp::{RemoteFileClient, RusshSftpClient, SftpClient, SharedRemoteFileClient};
use std::sync::Arc;
use tokio::sync::Mutex;

impl SftpView {
    pub(crate) fn switch_left_endpoint(
        &mut self,
        value: LeftEndpointValue,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.close_state.is_closing() {
            return;
        }
        match value {
            LeftEndpointValue::Local => self.switch_left_to_local(cx),
            LeftEndpointValue::Remote(id) => self.switch_left_to_remote(id, window, cx),
        }
    }

    pub(crate) fn switch_left_to_local(&mut self, cx: &mut Context<Self>) {
        self.disconnect_left_remote(cx);
        self.local_panel.update(cx, |panel, cx| {
            panel.set_left_endpoint(false, cx);
            panel.set_current_path(self.local_current_path.to_string_lossy().to_string(), cx);
        });
        self.remote_panel.update(cx, |panel, cx| {
            panel.set_opposite_endpoint_remote(false, cx);
        });
        self.refresh_local_dir(cx);
        cx.notify();
    }

    fn switch_left_to_remote(&mut self, id: i64, window: &mut Window, cx: &mut Context<Self>) {
        if self.close_state.is_closing() {
            return;
        }
        if self.left_remote_id() == Some(id) {
            return;
        }
        let Some(connection) = load_connection(id, cx) else {
            window.push_notification(Notification::error(t!("Endpoint.connection_missing")), cx);
            return;
        };
        let remote_file_ftp = crate::ssh_config::ftp_config_from_connection(&connection);
        // 右侧主连接与左侧端点共用同一套凭据提示策略：连接记录要求「连接时输入」
        // 且没有记住密码时，不直接拿空密码建连，改为弹窗人工录入。
        let (config, credential_prompt_policy) =
            match resolve_left_endpoint_config(&connection, &remote_file_ftp) {
                Ok(resolved) => resolved,
                Err(error) => {
                    window.push_notification(
                        Notification::error(t!("Endpoint.connection_invalid", error = error)),
                        cx,
                    );
                    return;
                }
            };

        self.disconnect_left_remote(cx);
        let sftp_initial_directory = crate::ssh_config::sftp_initial_directory_of(&connection);
        self.left_remote = Some(LeftRemoteEndpoint::connecting(
            connection,
            config,
            remote_file_ftp,
            sftp_initial_directory,
        ));
        self.local_panel.update(cx, |panel, cx| {
            panel.set_left_endpoint(true, cx);
            panel.set_current_path(".".to_string(), cx);
            panel.set_items(Vec::new(), cx);
        });
        self.remote_panel.update(cx, |panel, cx| {
            panel.set_opposite_endpoint_remote(true, cx);
        });
        if credential_prompt_policy.requires_prompt() {
            self.open_left_credential_prompt(id, credential_prompt_policy, window, cx);
        } else {
            self.left_credential_inputs = None;
            self.connect_left_remote(cx);
        }
        cx.notify();
    }

    pub(crate) fn connect_left_remote(&mut self, cx: &mut Context<Self>) {
        if self.close_state.is_closing() {
            return;
        }
        let generation = self.next_left_connection_generation();
        let Some(endpoint) = self.left_remote.as_mut() else {
            return;
        };
        endpoint.state = LeftRemoteConnectionState::Connecting;
        endpoint.loading = false;
        let config = endpoint.config.clone();
        let ftp_config = endpoint.remote_file_ftp.clone();
        let initial_directory = endpoint.sftp_initial_directory.clone();
        let connection_id = endpoint.connection.id;
        let window_handle = self.window_handle.clone();
        let task = Tokio::spawn(cx, async move {
            let mut client: Box<dyn RemoteFileClient> = match ftp_config {
                Some(ftp_config) => Box::new(FtpClient::connect(ftp_config).await?),
                None => Box::new(RusshSftpClient::connect(config).await?),
            };
            // 优先使用配置的初始目录，解析失败时回退到服务器登录目录
            let path = match initial_directory {
                Some(dir) => match client.realpath(&dir).await.ok() {
                    Some(resolved) => resolved,
                    None => {
                        tracing::warn!(
                            "Configured SFTP initial directory could not be resolved: {dir}"
                        );
                        client
                            .realpath(".")
                            .await
                            .unwrap_or_else(|_| ".".to_string())
                    }
                },
                None => client
                    .realpath(".")
                    .await
                    .unwrap_or_else(|_| ".".to_string()),
            };
            Ok::<_, anyhow::Error>((client, path))
        });

        cx.spawn(
            async move |this: WeakEntity<Self>, cx: &mut AsyncApp| match task.await {
                Ok(Ok((client, path))) => {
                    let client: SharedRemoteFileClient = Arc::new(Mutex::new(client));
                    let installed = this.update(cx, |this, cx| {
                        if this.close_state.is_closing()
                            || this.left_remote_id() != connection_id
                            || !this.is_current_left_connection_generation(generation)
                        {
                            return false;
                        }
                        let endpoint = this.left_remote.as_mut().expect("checked above");
                        endpoint.client = Some(client.clone());
                        endpoint.state = LeftRemoteConnectionState::Connected;
                        endpoint.current_path = path.clone();
                        endpoint.history = vec![path];
                        endpoint.history_index = 0;
                        this.left_credential_inputs = None;
                        this.set_left_connection_active(true, cx);
                        this.refresh_left_remote_dir(cx);
                        true
                    });
                    if !installed.unwrap_or(false) {
                        Tokio::spawn(cx, disconnect_sftp_client(client)).detach();
                    }
                }
                Ok(Err(error)) => {
                    let error_message = error.to_string();
                    if let Some(request) = host_key_prompt_request(&error) {
                        let should_prompt = this
                            .update(cx, |this, cx| {
                                if this.close_state.is_closing()
                                    || this.left_remote_id() != connection_id
                                    || !this.is_current_left_connection_generation(generation)
                                {
                                    return false;
                                }
                                this.set_left_connection_active(false, cx);
                                true
                            })
                            .unwrap_or(false);
                        if should_prompt {
                            let prompt_result = cx.update_window(window_handle, |_, window, cx| {
                                let _ = this.update(cx, |this, cx| {
                                    this.show_host_key_prompt(
                                        HostKeyPromptTarget::Left {
                                            connection_id,
                                            generation,
                                        },
                                        request,
                                        window,
                                        cx,
                                    );
                                });
                            });
                            if prompt_result.is_err() {
                                let _ = this.update(cx, |this, cx| {
                                    if this.close_state.is_closing()
                                        || this.left_remote_id() != connection_id
                                        || !this.is_current_left_connection_generation(generation)
                                    {
                                        return;
                                    }
                                    this.set_left_connection_error(error_message, cx);
                                });
                            }
                        }
                        return;
                    }
                    // 弹窗录入的凭据仍然连不上时，把原因回填到弹窗重试，
                    // 而不是让左侧直接断开、只能靠重新切换端点重来。
                    let reprompted = this
                        .update(cx, |this, cx| {
                            this.apply_left_credential_error(error_message.clone(), cx)
                        })
                        .unwrap_or(false);
                    if reprompted {
                        let prompt_result = cx.update_window(window_handle, |_, window, cx| {
                            let _ = this.update(cx, |this, cx| {
                                this.show_left_credential_prompt(window, cx);
                            });
                        });
                        if prompt_result.is_ok() {
                            return;
                        }
                    }
                    let _ = this.update(cx, |this, cx| {
                        if this.close_state.is_closing()
                            || this.left_remote_id() != connection_id
                            || !this.is_current_left_connection_generation(generation)
                        {
                            return;
                        }
                        this.left_credential_inputs = None;
                        this.set_left_connection_error(error_message, cx);
                    });
                }
                Err(error) => {
                    let _ = this.update(cx, |this, cx| {
                        if this.close_state.is_closing()
                            || this.left_remote_id() != connection_id
                            || !this.is_current_left_connection_generation(generation)
                        {
                            return;
                        }
                        this.set_left_connection_error(error.to_string(), cx);
                    });
                }
            },
        )
        .detach();
    }

    pub(crate) fn refresh_left_remote_dir(&mut self, cx: &mut Context<Self>) {
        if self.close_state.is_closing() {
            return;
        }
        let Some(endpoint) = self.left_remote.as_mut() else {
            return;
        };
        let Some(client) = endpoint.client.clone() else {
            return;
        };
        let path = endpoint.current_path.clone();
        let connection_id = endpoint.connection.id;
        endpoint.loading = true;
        self.local_panel.update(cx, |panel, cx| {
            panel.set_current_path(path.clone(), cx);
        });

        let task = Tokio::spawn(cx, async move {
            let mut client = client.lock().await;
            client.list_dir(&path).await
        });
        cx.spawn(async move |this: WeakEntity<Self>, cx| {
            let result = task.await;
            let _ = this.update(cx, |this, cx| {
                if this.close_state.is_closing() || this.left_remote_id() != connection_id {
                    return;
                }
                let endpoint = this.left_remote.as_mut().expect("checked above");
                endpoint.loading = false;
                if let Ok(Ok(entries)) = result {
                    let items = entries
                        .into_iter()
                        .map(|entry| {
                            let owner = entry.owner_display();
                            FileItem {
                                name: entry.name,
                                size: entry.size,
                                modified: entry.modified,
                                is_dir: entry.is_dir,
                                permissions: format_permissions(entry.permissions, entry.is_dir),
                                owner,
                                directory_size: crate::DirectorySizeState::Unknown,
                            }
                        })
                        .collect();
                    this.local_panel
                        .update(cx, |panel, cx| panel.set_items(items, cx));
                }
                cx.notify();
            });
        })
        .detach();
    }

    pub(crate) fn left_remote_id(&self) -> Option<i64> {
        self.left_remote.as_ref()?.connection.id
    }

    pub(crate) fn open_left_remote_terminal(&self, path: Option<String>, cx: &mut Context<Self>) {
        if self.close_state.is_closing() {
            return;
        }
        let Some(endpoint) = self.left_remote.as_ref() else {
            return;
        };
        cx.emit(crate::SftpViewEvent::OpenSshTerminal {
            connection: endpoint.connection.clone(),
            working_dir: path.unwrap_or_else(|| endpoint.current_path.clone()),
        });
    }

    pub(crate) fn on_left_remote_item_double_click(
        &mut self,
        name: String,
        is_dir: bool,
        cx: &mut Context<Self>,
    ) {
        if self.close_state.is_closing() || !is_dir {
            return;
        }
        let Some(endpoint) = self.left_remote.as_ref() else {
            return;
        };
        let next_path = if name == ".." {
            remote_parent(&endpoint.current_path)
        } else {
            crate::join_remote_path(&endpoint.current_path, &name)
        };
        self.navigate_left_remote_to(next_path, cx);
    }

    pub(crate) fn navigate_left_remote_to(&mut self, path: String, cx: &mut Context<Self>) {
        if self.close_state.is_closing() {
            return;
        }
        let Some(endpoint) = self.left_remote.as_mut() else {
            return;
        };
        if endpoint.current_path == path {
            return;
        }
        endpoint.current_path = path.clone();
        if endpoint.history_index + 1 < endpoint.history.len() {
            endpoint.history.truncate(endpoint.history_index + 1);
        }
        endpoint.history.push(path);
        endpoint.history_index = endpoint.history.len() - 1;
        self.refresh_left_remote_dir(cx);
    }

    pub(crate) fn can_go_back_left_remote(&self) -> bool {
        self.left_remote
            .as_ref()
            .is_some_and(|endpoint| endpoint.history_index > 0)
    }

    pub(crate) fn can_go_forward_left_remote(&self) -> bool {
        self.left_remote
            .as_ref()
            .is_some_and(|endpoint| endpoint.history_index + 1 < endpoint.history.len())
    }

    pub(crate) fn go_back_left_remote(&mut self, cx: &mut Context<Self>) {
        if self.close_state.is_closing() {
            return;
        }
        let Some(endpoint) = self.left_remote.as_mut() else {
            return;
        };
        if endpoint.history_index == 0 {
            return;
        }
        endpoint.history_index -= 1;
        endpoint.current_path = endpoint.history[endpoint.history_index].clone();
        self.refresh_left_remote_dir(cx);
    }

    pub(crate) fn go_forward_left_remote(&mut self, cx: &mut Context<Self>) {
        if self.close_state.is_closing() {
            return;
        }
        let Some(endpoint) = self.left_remote.as_mut() else {
            return;
        };
        if endpoint.history_index + 1 >= endpoint.history.len() {
            return;
        }
        endpoint.history_index += 1;
        endpoint.current_path = endpoint.history[endpoint.history_index].clone();
        self.refresh_left_remote_dir(cx);
    }

    pub(crate) fn set_left_connection_error(&mut self, error: String, cx: &mut Context<Self>) {
        if let Some(endpoint) = self.left_remote.as_mut() {
            endpoint.state = LeftRemoteConnectionState::Disconnected(error);
            endpoint.loading = false;
        }
        cx.notify();
    }

    pub(crate) fn disconnect_left_remote(&mut self, cx: &mut Context<Self>) {
        if let Some(client) = self.take_left_remote_client(cx) {
            Tokio::spawn(cx, disconnect_sftp_client(client)).detach();
        }
    }

    /// Remove the left endpoint and return its client to the caller.
    ///
    /// Close handling needs to decide whether disconnecting an endpoint should
    /// be awaited (the wait strategy) or detached (cancel/background).  Keep
    /// the ownership transfer synchronous here so that no disconnect task can
    /// be started before the close decision has been committed.
    pub(crate) fn take_left_remote_client(
        &mut self,
        cx: &mut Context<Self>,
    ) -> Option<SharedRemoteFileClient> {
        self.left_connection_generation.advance();
        self.left_credential_inputs = None;
        let mut endpoint = self.left_remote.take()?;
        self.set_left_connection_active_for(endpoint.connection.id, false, cx);
        endpoint.client.take()
    }

    fn set_left_connection_active(&self, active: bool, cx: &mut Context<Self>) {
        self.set_left_connection_active_for(self.left_remote_id(), active, cx);
    }

    fn set_left_connection_active_for(
        &self,
        connection_id: Option<i64>,
        active: bool,
        cx: &mut Context<Self>,
    ) {
        let Some(connection_id) = connection_id else {
            return;
        };
        let state = cx.global_mut::<ActiveConnections>();
        if active {
            state.add(connection_id);
        } else {
            state.remove(connection_id);
        }
    }
}

/// 解析左侧端点的建连配置与凭据提示策略。
///
/// FTP 模式下凭据策略来自嵌套 FTP 参数，SSH 模式下来自 SSH 参数；
/// 独立 FTP 连接没有 SSH 参数，用占位配置避免 panic。
fn resolve_left_endpoint_config(
    connection: &one_core::storage::StoredConnection,
    remote_file_ftp: &Option<ftp::FtpConnectConfig>,
) -> anyhow::Result<(
    ssh::SshConnectConfig,
    crate::ssh_config::SshCredentialPromptPolicy,
)> {
    if connection.connection_type == one_core::storage::ConnectionType::Ftp {
        return Ok((
            crate::ssh_config::unused_ssh_config_placeholder(),
            crate::ssh_config::ftp_credential_prompt_policy(connection),
        ));
    }
    let resolved = crate::ssh_config::resolve_ssh_connection(connection)?;
    let policy = if remote_file_ftp.is_some() {
        crate::ssh_config::ftp_credential_prompt_policy(connection)
    } else {
        resolved.credential_prompt_policy
    };
    Ok((resolved.config, policy))
}

fn remote_parent(path: &str) -> String {
    let trimmed = path.trim_end_matches('/');
    if trimmed.is_empty() || trimmed == "/" {
        return "/".to_string();
    }
    trimmed
        .rsplit_once('/')
        .map(|(parent, _)| if parent.is_empty() { "/" } else { parent })
        .unwrap_or(".")
        .to_string()
}
