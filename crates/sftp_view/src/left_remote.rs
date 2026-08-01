use crate::endpoint::{LeftEndpointValue, load_connection};
use crate::left_remote_state::{LeftRemoteConnectionState, LeftRemoteEndpoint};
use crate::{FileItem, SftpView, disconnect_sftp_client, format_permissions};
use gpui::{AsyncApp, Context, WeakEntity, Window};
use gpui_component::{WindowExt, notification::Notification};
use one_core::gpui_tokio::Tokio;
use one_core::storage::ActiveConnections;
use rust_i18n::t;
use sftp::{RusshSftpClient, SftpClient};
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

    fn switch_left_to_local(&mut self, cx: &mut Context<Self>) {
        self.disconnect_left_remote(cx);
        self.local_panel.update(cx, |panel, cx| {
            panel.set_left_endpoint(false, cx);
            panel.set_current_path(self.local_current_path.to_string_lossy().to_string(), cx);
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
        let config = match crate::ssh_config::ssh_config_for(&connection) {
            Ok(config) => config,
            Err(error) => {
                window.push_notification(
                    Notification::error(t!("Endpoint.connection_invalid", error = error)),
                    cx,
                );
                return;
            }
        };

        self.disconnect_left_remote(cx);
        self.left_remote = Some(LeftRemoteEndpoint::connecting(connection, config));
        self.local_panel.update(cx, |panel, cx| {
            panel.set_left_endpoint(true, cx);
            panel.set_current_path(".".to_string(), cx);
            panel.set_items(Vec::new(), cx);
        });
        self.connect_left_remote(cx);
        cx.notify();
    }

    fn connect_left_remote(&mut self, cx: &mut Context<Self>) {
        if self.close_state.is_closing() {
            return;
        }
        let Some(endpoint) = self.left_remote.as_ref() else {
            return;
        };
        let config = endpoint.config.clone();
        let connection_id = endpoint.connection.id;
        let task = Tokio::spawn(cx, async move {
            let mut client = RusshSftpClient::connect(config).await?;
            let path = client
                .realpath(".")
                .await
                .unwrap_or_else(|_| ".".to_string());
            Ok::<_, anyhow::Error>((client, path))
        });

        cx.spawn(
            async move |this: WeakEntity<Self>, cx: &mut AsyncApp| match task.await {
                Ok(Ok((client, path))) => {
                    let client = Arc::new(Mutex::new(client));
                    let installed = this.update(cx, |this, cx| {
                        if this.close_state.is_closing() || this.left_remote_id() != connection_id {
                            return false;
                        }
                        let endpoint = this.left_remote.as_mut().expect("checked above");
                        endpoint.client = Some(client.clone());
                        endpoint.state = LeftRemoteConnectionState::Connected;
                        endpoint.current_path = path.clone();
                        endpoint.history = vec![path];
                        endpoint.history_index = 0;
                        this.set_left_connection_active(true, cx);
                        this.refresh_left_remote_dir(cx);
                        true
                    });
                    if !installed.unwrap_or(false) {
                        Tokio::spawn(cx, disconnect_sftp_client(client)).detach();
                    }
                }
                Ok(Err(error)) => {
                    let _ = this.update(cx, |this, cx| {
                        if this.close_state.is_closing() || this.left_remote_id() != connection_id {
                            return;
                        }
                        this.set_left_connection_error(error.to_string(), cx);
                    });
                }
                Err(error) => {
                    let _ = this.update(cx, |this, cx| {
                        if this.close_state.is_closing() || this.left_remote_id() != connection_id {
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
                        .map(|entry| FileItem {
                            name: entry.name,
                            size: entry.size,
                            modified: entry.modified,
                            is_dir: entry.is_dir,
                            permissions: format_permissions(entry.permissions, entry.is_dir),
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

    fn set_left_connection_error(&mut self, error: String, cx: &mut Context<Self>) {
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
    ) -> Option<Arc<Mutex<RusshSftpClient>>> {
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
