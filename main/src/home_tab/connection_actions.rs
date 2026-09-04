use super::*;

fn sensitive_export_copy_text(
    authorized_identity: Option<&ConnectionCredentialExportIdentity>,
    result: anyhow::Result<StoredConnection>,
) -> Option<String> {
    let authorized_identity = authorized_identity?;
    let connection = result.ok()?;
    if !authorized_identity.matches(&connection) {
        return None;
    }
    crate::persistent_connection_sidebar::connection_full_info_text(&connection)
}

impl HomePage {
    pub(crate) fn edit_connection(
        &mut self,
        connection: StoredConnection,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(connection_id) = connection.id else {
            return;
        };
        match connection.connection_type {
            ConnectionType::Database => {
                let database_type = connection
                    .to_db_connection()
                    .ok()
                    .map(|params| params.database_type);
                self.confirm_edit_connection(
                    connection_id,
                    connection.name,
                    database_type,
                    window,
                    cx,
                );
            }
            ConnectionType::SshSftp => {
                self.editing_connection_id = Some(connection_id);
                self.show_ssh_form(window, cx);
            }
            ConnectionType::Redis => {
                self.editing_connection_id = Some(connection_id);
                self.show_redis_form(window, cx);
            }
            ConnectionType::MongoDB => {
                self.editing_connection_id = Some(connection_id);
                self.show_mongodb_form(window, cx);
            }
            ConnectionType::Mqtt => {
                self.editing_connection_id = Some(connection_id);
                self.show_mqtt_form(window, cx);
            }
            ConnectionType::Serial => {
                self.editing_connection_id = Some(connection_id);
                self.show_serial_form(window, cx);
            }
            ConnectionType::Telnet => {
                self.editing_connection_id = Some(connection_id);
                self.show_telnet_form(window, cx);
            }
            ConnectionType::PortForwarding => {
                self.editing_connection_id = Some(connection_id);
                self.show_port_forwarding_form(window, cx);
            }
            ConnectionType::Rdp | ConnectionType::Vnc => {
                let protocol = if connection.connection_type == ConnectionType::Rdp {
                    StoredRemoteDesktopProtocol::Rdp
                } else {
                    StoredRemoteDesktopProtocol::Vnc
                };
                self.editing_connection_id = Some(connection_id);
                self.show_remote_desktop_form(protocol, window, cx);
            }
            ConnectionType::Extension => {
                #[cfg(feature = "shell-plugins")]
                if cx.global::<ActiveConnections>().is_active(connection_id) {
                    let name = connection.name;
                    window.open_dialog(cx, move |dialog, _window, _cx| {
                        dialog
                            .title(t!("Connection.in_use_title").to_string().into_any_element())
                            .child(
                                t!("Connection.in_use_cannot_edit", conn_name = name)
                                    .to_string()
                                    .into_any_element(),
                            )
                            .alert()
                    });
                } else {
                    self.editing_connection_id = Some(connection_id);
                    self.show_extension_form(window, cx);
                }
                #[cfg(not(feature = "shell-plugins"))]
                {
                    let _ = (connection_id, &connection);
                }
            }
            _ => {}
        }
    }

    pub(super) fn confirm_edit_connection(
        &mut self,
        conn_id: i64,
        conn_name: String,
        db_type: Option<DatabaseType>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let is_active = cx.global::<ActiveConnections>().is_active(conn_id);

        if is_active {
            window.open_dialog(cx, move |dialog, _window, _cx| {
                dialog
                    .title(t!("Connection.in_use_title").to_string().into_any_element())
                    .child(
                        t!("Connection.in_use_cannot_edit", conn_name = conn_name)
                            .to_string()
                            .into_any_element(),
                    )
                    .alert()
            });
        } else if let Some(db_type) = db_type {
            self.editing_connection_id = Some(conn_id);
            self.show_connection_form(db_type, window, cx);
        }
    }

    /// 复制连接，创建一个副本
    pub(crate) fn duplicate_connection(
        &mut self,
        conn: StoredConnection,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let storage = cx.global::<GlobalStorageState>().storage.clone();
        let current_user = self.current_user.clone();

        cx.spawn(async move |this, cx: &mut AsyncApp| {
            let result: anyhow::Result<StoredConnection> = (|| {
                let repo = storage
                    .get::<ConnectionRepository>()
                    .ok_or_else(|| anyhow::anyhow!("ConnectionRepository not found"))?;

                // 获取现有连接名称列表，用于生成唯一名称
                let existing_names: HashSet<String> = repo
                    .list()
                    .unwrap_or_default()
                    .iter()
                    .map(|c| c.name.clone())
                    .collect();

                // 生成新的唯一名称
                let new_name = generate_duplicate_name(&conn.name, &existing_names);

                // 克隆连接，清除 id 和云同步相关字段
                let mut new_conn = conn.clone();
                new_conn.id = None;
                new_conn.cloud_id = None;
                new_conn.last_synced_at = None;
                new_conn.name = new_name;
                new_conn.owner_id = current_user.map(|u| u.id);

                // 保存新连接
                repo.insert(&mut new_conn)?;
                Ok(new_conn)
            })();

            match result {
                Ok(saved_conn) => {
                    // 发出 ConnectionCreated 事件，首页自动刷新
                    _ = this.update(cx, |_this, cx| {
                        if let Some(notifier) = get_notifier(cx) {
                            notifier.update(cx, |_, cx| {
                                cx.emit(ConnectionDataEvent::ConnectionCreated {
                                    connection: saved_conn,
                                });
                            });
                        }
                    });
                }
                Err(e) => {
                    tracing::error!("复制连接失败: {}", e);
                }
            }
        })
        .detach();
    }

    pub(crate) fn confirm_delete_connection(
        &mut self,
        conn_id: i64,
        conn_name: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let is_active = cx.global::<ActiveConnections>().is_active(conn_id);
        let view = cx.entity().clone();

        if is_active {
            window.open_dialog(cx, move |dialog, _window, _cx| {
                dialog
                    .title(t!("Connection.in_use_title").to_string().into_any_element())
                    .child(
                        t!("Connection.in_use_cannot_delete", conn_name = conn_name)
                            .to_string()
                            .into_any_element(),
                    )
                    .alert()
            });
        } else {
            window.open_dialog(cx, move |dialog, _window, _cx| {
                let view_clone = view.clone();
                dialog
                    .title(t!("Common.delete").to_string().into_any_element())
                    .child(
                        t!("Connection.delete_confirm", conn_name = conn_name)
                            .to_string()
                            .into_any_element(),
                    )
                    .confirm()
                    .on_ok(move |_, _, cx: &mut App| {
                        let _ = view_clone.update(cx, |this, cx| {
                            this.delete_connection(conn_id, cx);
                        });
                        true
                    })
            });
        }
    }

    pub(crate) fn confirm_copy_full_connection_info(
        &mut self,
        connection_id: i64,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.can_export_connection_credentials(connection_id) {
            window.push_notification(
                Notification::error(t!("Connection.copy_sensitive_unavailable").to_string())
                    .autohide(true),
                cx,
            );
            return;
        }

        let view = cx.entity();
        window.open_dialog(cx, move |dialog, _window, _cx| {
            let view = view.clone();
            dialog
                .title(
                    t!("Connection.copy_full_info_confirm_title")
                        .to_string()
                        .into_any_element(),
                )
                .child(
                    t!("Connection.copy_full_info_confirm_message")
                        .to_string()
                        .into_any_element(),
                )
                .confirm()
                .button_props(
                    DialogButtonProps::default()
                        .ok_text(t!("Connection.copy_full_info_confirm_action").to_string())
                        .cancel_text(t!("Common.cancel").to_string()),
                )
                .on_ok(move |_, window, cx: &mut App| {
                    let _ = view.update(cx, |this, cx| {
                        this.copy_full_connection_info(connection_id, window, cx);
                    });
                    true
                })
        });
    }

    fn copy_full_connection_info(
        &mut self,
        connection_id: i64,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(export_identity) = self.connection_credential_export_identity(connection_id)
        else {
            window.push_notification(
                Notification::error(t!("Connection.copy_sensitive_unavailable").to_string())
                    .autohide(true),
                cx,
            );
            return;
        };

        let storage = cx.global::<GlobalStorageState>().storage.clone();
        let home = cx.entity();
        let window_handle = window.window_handle();
        let expected_team_id = export_identity.team_id;
        let expected_owner_id = export_identity.owner_id;
        let load_task = cx.background_spawn(async move {
            let repo = storage
                .get::<ConnectionRepository>()
                .ok_or_else(|| anyhow::anyhow!("ConnectionRepository not found"))?;
            repo.get_for_sensitive_export(
                connection_id,
                expected_team_id.as_deref(),
                expected_owner_id.as_deref(),
            )?
            .ok_or_else(|| anyhow::anyhow!("Connection not found"))
        });

        cx.spawn(async move |_, cx: &mut AsyncApp| {
            let result = load_task.await;
            let _ = cx.update_window(window_handle, |_, window, cx| {
                let authorized_identity = home
                    .read(cx)
                    .connection_credential_export_identity(connection_id);
                let text = sensitive_export_copy_text(authorized_identity.as_ref(), result);
                match text {
                    Some(text) => {
                        cx.write_to_clipboard(ClipboardItem::new_string(text));
                        window.push_notification(
                            Notification::success(
                                t!("Connection.copy_sensitive_success").to_string(),
                            )
                            .autohide(true),
                            cx,
                        );
                    }
                    None => {
                        window.push_notification(
                            Notification::error(
                                t!("Connection.copy_sensitive_unavailable").to_string(),
                            )
                            .autohide(true),
                            cx,
                        );
                    }
                }
                window.refresh();
            });
        })
        .detach();
    }

    pub(super) fn delete_connection(&mut self, conn_id: i64, cx: &mut Context<Self>) {
        let storage = cx.global::<GlobalStorageState>().storage.clone();

        // 获取连接的 cloud_id，用于删除云端数据
        let cloud_id = self
            .connections
            .iter()
            .find(|c| c.id == Some(conn_id))
            .and_then(|c| c.cloud_id.clone());

        // 如果用户已登录且连接有 cloud_id，需要同时删除云端
        let cloud_client = if cloud_id.is_some() && self.current_user.is_some() {
            Some(self.auth_service.cloud_client())
        } else {
            None
        };

        cx.spawn(async move |this, cx: &mut AsyncApp| {
            // 1. 先删除云端连接（如果有）
            if let (Some(cloud_id), Some(client)) = (&cloud_id, cloud_client) {
                match client.delete_sync_data(cloud_id).await {
                    Ok(_) => {
                        tracing::info!("[删除] 云端连接删除成功: {}", cloud_id);
                    }
                    Err(e) => {
                        // 云端删除失败，记录到待删除表，下次同步时重试
                        tracing::warn!(
                            "[删除] 云端连接删除失败: {} - {}（记录到待删除列表）",
                            cloud_id,
                            e
                        );
                        if let Some(pending_repo) = storage.get::<PendingCloudDeletionRepository>()
                        {
                            if let Err(e) = pending_repo.add(cloud_id, "connection") {
                                tracing::error!("[删除] 记录待删除失败: {}", e);
                            }
                        }
                    }
                }
            } else if let Some(cloud_id) = &cloud_id {
                // 用户未登录但连接有 cloud_id，也记录到待删除表
                tracing::info!("[删除] 用户离线，记录到待删除列表: {}", cloud_id);
                if let Some(pending_repo) = storage.get::<PendingCloudDeletionRepository>() {
                    if let Err(e) = pending_repo.add(cloud_id, "connection") {
                        tracing::error!("[删除] 记录待删除失败: {}", e);
                    }
                }
            }

            // 2. 删除本地连接
            let result = (|| {
                let repo = storage
                    .get::<ConnectionRepository>()
                    .ok_or_else(|| anyhow::anyhow!("ConnectionRepository not found"))?;
                repo.delete(conn_id)
            })();

            match result {
                Ok(_) => {
                    _ = this.update(cx, |this, cx| {
                        this.connections.retain(|c| c.id != Some(conn_id));
                        if this.selected_connection_id == Some(conn_id) {
                            this.selected_connection_id = None;
                        }
                        emit_connection_event(
                            ConnectionDataEvent::ConnectionDeleted {
                                connection_id: conn_id,
                                cloud_id: cloud_id.clone(),
                            },
                            cx,
                        );
                        cx.notify();
                    });
                }
                Err(e) => {
                    tracing::error!("Failed to delete connection: {}", e);
                }
            }
        })
        .detach();
    }
}

#[cfg(test)]
mod sensitive_copy_tests {
    use one_core::storage::{SshAuthMethod, SshParams};

    use super::*;

    fn sensitive_ssh_connection() -> StoredConnection {
        StoredConnection::new_ssh(
            "Sensitive SSH".to_string(),
            SshParams {
                sftp_default_directory: None,
                disabled_jump_server: None,
                sftp_account: None,
                host: "ssh.example.test".to_string(),
                port: 22,
                username: "alice".to_string(),
                auth_method: SshAuthMethod::Password {
                    password: "clipboard-secret".to_string(),
                },
                credential_reference: None,
                prompt_username: None,
                prompt_password: None,
                keyboard_interactive: None,
                terminal_encoding: Default::default(),
                terminal_type: Default::default(),
                connect_timeout: None,
                keepalive_interval: None,
                keepalive_max: None,
                default_directory: None,
                init_script: None,
                disable_shell_integration: None,
                x11_forwarding: None,
                allow_legacy_algorithms: None,
                jump_server: None,
                proxy: None,
                os_id: None,
                icon: None,
                icon_file_path: None,
                account_expect: Default::default(),
            },
            None,
        )
    }

    #[test]
    fn sensitive_export_text_requires_the_current_authorized_identity() {
        let mut connection = sensitive_ssh_connection();
        connection.team_id = Some("team-a".to_string());
        connection.owner_id = Some("owner-a".to_string());
        let identity = ConnectionCredentialExportIdentity::from_connection(&connection);

        let allowed = sensitive_export_copy_text(Some(&identity), Ok(connection.clone()))
            .expect("copyable text");
        assert!(allowed.contains("clipboard-secret"));
        assert_eq!(
            None,
            sensitive_export_copy_text(None, Ok(connection.clone()))
        );

        connection.team_id = Some("team-b".to_string());
        assert_eq!(
            None,
            sensitive_export_copy_text(Some(&identity), Ok(connection))
        );
    }

    #[test]
    fn sensitive_export_text_fails_closed_for_load_or_format_errors() {
        let connection = sensitive_ssh_connection();
        let identity = ConnectionCredentialExportIdentity::from_connection(&connection);
        let load_error = sensitive_export_copy_text(
            Some(&identity),
            Err(anyhow::anyhow!("private load failure")),
        );
        assert_eq!(None, load_error);

        let mut malformed = connection;
        malformed.params = "{malformed".to_string();
        assert_eq!(
            None,
            sensitive_export_copy_text(Some(&identity), Ok(malformed))
        );
    }
}
