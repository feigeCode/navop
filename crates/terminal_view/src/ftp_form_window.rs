use connection_form::credential::resolve_connection_for_runtime;
use connection_form::credential::{
    CredentialCapabilities, CredentialPickerConfig, CredentialPickerEvent,
    CredentialReferencePicker, create_credential_picker,
};
use connection_form::team::{
    TeamSelectItem, connection_sync_controls_visible_in, create_team_select, refresh_team_options,
    refresh_teams_tooltip, resolve_team_assignment, selected_team_id, team_label,
    team_management_enabled,
};
use ftp::FtpClient;
use gpui::prelude::FluentBuilder;
use gpui::{
    App, AppContext, AsyncApp, Context, Entity, FocusHandle, Focusable, InteractiveElement,
    IntoElement, ParentElement, Render, SharedString, StatefulInteractiveElement, Styled,
    WeakEntity, Window, div, px,
};
use gpui_component::{
    ActiveTheme, Disableable, IndexPath, Sizable,
    button::{Button, ButtonVariants as _},
    checkbox::Checkbox,
    h_flex,
    input::{Input, InputState, Textarea, TextareaState},
    select::{Select, SelectItem, SelectState},
    v_flex,
};
use one_assets::IconName;
use one_core::cloud_sync::TeamOption;
use one_core::connection_notifier::{ConnectionDataEvent, get_notifier};
use one_core::gpui_tokio::Tokio;
use one_core::storage::traits::Repository;
use one_core::storage::{FtpParams, StoredConnection, Workspace};
use rust_i18n::t;

pub struct FtpFormWindowConfig {
    pub editing_connection: Option<StoredConnection>,
    pub workspaces: Vec<Workspace>,
    pub teams: Vec<TeamOption>,
}

#[derive(Clone, Default, PartialEq)]
struct WorkspaceSelectItem {
    id: Option<i64>,
    name: String,
}

impl WorkspaceSelectItem {
    fn none() -> Self {
        Self {
            id: None,
            name: t!("Common.none").to_string(),
        }
    }

    fn from_workspace(ws: &Workspace) -> Self {
        Self {
            id: ws.id,
            name: ws.name.clone(),
        }
    }
}

impl SelectItem for WorkspaceSelectItem {
    type Value = Option<i64>;

    fn title(&self) -> SharedString {
        self.name.clone().into()
    }

    fn value(&self) -> &Self::Value {
        &self.id
    }
}

/// FTP 默认端口列表，供下拉选择。
#[derive(Clone, PartialEq)]
struct FtpPortItem {
    port: u16,
}

impl SelectItem for FtpPortItem {
    type Value = u16;

    fn title(&self) -> SharedString {
        self.port.to_string().into()
    }

    fn value(&self) -> &Self::Value {
        &self.port
    }
}

const FTP_DEFAULT_PORTS: &[u16] = &[21, 2121, 990, 21210];
const FTP_DEFAULT_PORT: u16 = 21;

pub struct FtpFormWindow {
    focus_handle: FocusHandle,
    is_editing: bool,
    editing_id: Option<i64>,
    editing_cloud_id: Option<String>,
    editing_last_synced_at: Option<i64>,
    editing_owner_id: Option<String>,

    // 基本信息
    name_input: Entity<InputState>,
    host_input: Entity<InputState>,
    port_input: Entity<InputState>,
    port_select: Entity<SelectState<Vec<FtpPortItem>>>,
    username_input: Entity<InputState>,
    password_input: Entity<InputState>,
    passive_mode: bool,
    use_tls: bool,
    workspace_select: Entity<SelectState<Vec<WorkspaceSelectItem>>>,
    team_select: Entity<SelectState<Vec<TeamSelectItem>>>,
    remark_input: Entity<TextareaState>,
    credential_picker: Entity<CredentialReferencePicker>,
    sync_enabled: bool,

    is_testing: bool,
    test_result: Option<Result<(), String>>,
}

impl FtpFormWindow {
    pub fn new(config: FtpFormWindowConfig, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let is_editing = config.editing_connection.is_some();
        let editing_id = config.editing_connection.as_ref().and_then(|c| c.id);
        let editing_cloud_id = config
            .editing_connection
            .as_ref()
            .and_then(|c| c.cloud_id.clone());
        let editing_last_synced_at = config
            .editing_connection
            .as_ref()
            .and_then(|c| c.last_synced_at);
        let editing_owner_id = config
            .editing_connection
            .as_ref()
            .and_then(|c| c.owner_id.clone());

        let name_input =
            cx.new(|cx| InputState::new(window, cx).placeholder(t!("Ftp.name_placeholder")));
        let host_input =
            cx.new(|cx| InputState::new(window, cx).placeholder(t!("Ftp.host_placeholder")));
        let port_input =
            cx.new(|cx| InputState::new(window, cx).placeholder(t!("Ftp.port_placeholder")));
        let username_input =
            cx.new(|cx| InputState::new(window, cx).placeholder(t!("Ftp.username_placeholder")));
        let password_input =
            cx.new(|cx| InputState::new(window, cx).placeholder(t!("Ftp.password_placeholder")));
        let remark_input = cx.new(|cx| {
            TextareaState::new(window, cx)
                .placeholder(t!("Ftp.remark_placeholder"))
                .auto_grow(3, 10)
        });

        // 端口选择，默认 21（索引 0）
        let port_items: Vec<FtpPortItem> = FTP_DEFAULT_PORTS
            .iter()
            .map(|&port| FtpPortItem { port })
            .collect();
        let port_select = cx
            .new(|cx| SelectState::new(port_items, Some(IndexPath::default().row(0)), window, cx));

        // 工作区选择
        let mut workspace_items = vec![WorkspaceSelectItem::none()];
        workspace_items.extend(
            config
                .workspaces
                .iter()
                .map(WorkspaceSelectItem::from_workspace),
        );
        let workspace_select =
            cx.new(|cx| SelectState::new(workspace_items, Some(Default::default()), window, cx));

        let team_select = create_team_select(&config.teams, None, window, cx);

        let mut sync_enabled = true;
        let mut workspace_id: Option<i64> = None;
        let mut team_id: Option<String> = None;
        let mut credential_reference = None;

        // 编辑模式：加载已有数据
        if let Some(ref conn) = config.editing_connection {
            sync_enabled = conn.sync_enabled;

            if let Ok(params) = conn.to_ftp_params() {
                name_input.update(cx, |s, cx| s.set_value(&conn.name, window, cx));
                host_input.update(cx, |s, cx| s.set_value(&params.host, window, cx));
                let port = params.port.to_string();
                port_input.update(cx, |s, cx| s.set_value(&port, window, cx));
                port_select.update(cx, |s, cx| {
                    s.set_selected_value(&params.port, window, cx);
                });
                username_input.update(cx, |s, cx| s.set_value(&params.username, window, cx));
                password_input.update(cx, |s, cx| s.set_value(&params.password, window, cx));
                credential_reference = params.credential_reference.clone();
            }
            workspace_id = conn.workspace_id;
            team_id = conn.team_id.clone();

            if let Some(ref remark) = conn.remark {
                remark_input.update(cx, |s, cx| s.set_value(remark, window, cx));
            }
        }

        let passive_mode = config
            .editing_connection
            .as_ref()
            .and_then(|conn| conn.to_ftp_params().ok())
            .map(|params| params.passive_mode)
            .unwrap_or(true);
        let use_tls = config
            .editing_connection
            .as_ref()
            .and_then(|conn| conn.to_ftp_params().ok())
            .map(|params| params.use_tls)
            .unwrap_or(false);

        if let Some(ws_id) = workspace_id {
            workspace_select.update(cx, |select, cx| {
                select.set_selected_value(&Some(ws_id), window, cx);
            });
        }

        if let Some(ref tid) = team_id {
            team_select.update(cx, |select, cx| {
                select.set_selected_value(&Some(tid.clone()), window, cx);
            });
        }

        let credential_picker = create_credential_picker(
            CredentialPickerConfig::new("ftp-credential", CredentialCapabilities::login())
                .reference(credential_reference),
            window,
            cx,
        );
        cx.subscribe(&credential_picker, |_, _, _: &CredentialPickerEvent, cx| {
            cx.notify()
        })
        .detach();

        Self {
            focus_handle: cx.focus_handle(),
            is_editing,
            editing_id,
            editing_cloud_id,
            editing_last_synced_at,
            editing_owner_id,
            name_input,
            host_input,
            port_input,
            port_select,
            username_input,
            password_input,
            passive_mode,
            use_tls,
            workspace_select,
            team_select,
            remark_input,
            credential_picker,
            sync_enabled,
            is_testing: false,
            test_result: None,
        }
    }

    fn get_workspace_id(&self, cx: &App) -> Option<i64> {
        self.workspace_select
            .read(cx)
            .selected_value()
            .cloned()
            .flatten()
    }

    fn get_team_id(&self, cx: &App) -> Option<String> {
        selected_team_id(&self.team_select, cx)
    }

    fn request_team_sync(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        refresh_team_options(&self.team_select, window, cx);
    }

    fn get_host(&self, cx: &App) -> String {
        self.host_input
            .read(cx)
            .text()
            .to_string()
            .trim()
            .to_string()
    }

    fn get_port(&self, cx: &App) -> Option<u16> {
        let manual = self.port_input.read(cx).text().to_string();
        let manual = manual.trim();
        if !manual.is_empty() {
            return manual.parse::<u16>().ok();
        }
        self.port_select
            .read(cx)
            .selected_value()
            .copied()
            .or(Some(FTP_DEFAULT_PORT))
    }

    fn build_ftp_params(&self, cx: &App) -> Option<FtpParams> {
        let host = self.get_host(cx);
        if host.is_empty() {
            return None;
        }
        let port = self.get_port(cx)?;
        if port == 0 {
            return None;
        }
        // 与 SSH 一致：选择了钥匙串凭据时，被引用的字段以钥匙串为准，
        // 手填值不落库。
        let credential_reference = self.credential_picker.read(cx).selected_reference();
        let credential_selected = credential_reference.is_some();
        Some(FtpParams {
            host,
            port,
            username: if credential_selected {
                String::new()
            } else {
                self.username_input.read(cx).text().to_string()
            },
            password: if credential_selected {
                String::new()
            } else {
                self.password_input.read(cx).text().to_string()
            },
            credential_reference,
            prompt_username: None,
            prompt_password: None,
            passive_mode: self.passive_mode,
            use_tls: self.use_tls,
            connect_timeout: Some(10),
        })
    }

    fn validation_error(&self, cx: &App) -> Option<String> {
        if self.get_host(cx).is_empty() || self.get_port(cx).is_none_or(|port| port == 0) {
            return Some(t!("Ftp.validation_error").to_string());
        }
        None
    }

    fn on_test(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(params) = self.build_ftp_params(cx) else {
            self.test_result = Some(Err(self
                .validation_error(cx)
                .unwrap_or_else(|| t!("Ftp.validation_error").to_string())));
            cx.notify();
            return;
        };
        let params = match resolve_ftp_test_params(params, cx) {
            Ok(params) => params,
            Err(error) => {
                self.test_result = Some(Err(error));
                cx.notify();
                return;
            }
        };

        self.is_testing = true;
        self.test_result = None;
        cx.notify();

        let window_handle = window.window_handle();
        cx.spawn(async move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            // 真实建连测试：TCP → 可选 AUTH TLS → 登录 → realpath
            let spawn_result = Tokio::spawn_result(cx, async move {
                let _client = FtpClient::connect(ftp::FtpConnectConfig {
                    host: params.host.clone(),
                    port: params.port,
                    username: params.username.clone(),
                    password: params.password.clone(),
                    passive_mode: params.passive_mode,
                    use_tls: params.use_tls,
                    connect_timeout: params.connect_timeout,
                })
                .await
                .map_err(|error| anyhow::anyhow!(error.to_string()))?;
                // connect 内已完成登录 + 二进制模式切换，能返回即视为测试成功
                Ok::<(), anyhow::Error>(())
            })
            .await;

            let _ = cx.update_window(window_handle, |_, _window, cx| {
                let _ = this.update(cx, |this, cx| {
                    this.is_testing = false;
                    this.test_result = Some(match spawn_result {
                        Ok(()) => Ok(()),
                        Err(error) => Err(format!("{error:#}")),
                    });
                    cx.notify();
                });
            });
        })
        .detach();
    }

    fn on_save(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(params) = self.build_ftp_params(cx) else {
            self.test_result = Some(Err(self
                .validation_error(cx)
                .unwrap_or_else(|| t!("Ftp.validation_error").to_string())));
            cx.notify();
            return;
        };

        let name = self.name_input.read(cx).text().to_string();
        let name = if name.is_empty() {
            format!("{}:{}", params.host, params.port)
        } else {
            name
        };

        let workspace_id = self.get_workspace_id(cx);
        let mut conn = StoredConnection::new_ftp(name, params, workspace_id);
        conn.sync_enabled = self.sync_enabled;
        let assignment = match resolve_team_assignment(
            self.get_team_id(cx),
            self.is_editing,
            self.editing_owner_id.clone(),
            cx,
        ) {
            Ok(assignment) => assignment,
            Err(error) => {
                self.test_result = Some(Err(error.to_string()));
                cx.notify();
                return;
            }
        };
        conn.team_id = assignment.team_id;
        conn.owner_id = assignment.owner_id;
        if self.is_editing {
            conn.id = self.editing_id;
            conn.cloud_id = self.editing_cloud_id.clone();
            conn.last_synced_at = self.editing_last_synced_at;
        }

        let remark = self.remark_input.read(cx).text().to_string();
        if !remark.is_empty() {
            conn.remark = Some(remark);
        }

        let storage = cx
            .global::<one_core::storage::GlobalStorageState>()
            .storage
            .clone();
        let is_editing = self.is_editing;

        let result: Result<StoredConnection, anyhow::Error> = (|| {
            let repo = storage
                .get::<one_core::storage::ConnectionRepository>()
                .ok_or_else(|| anyhow::anyhow!("ConnectionRepository not found"))?;

            if is_editing {
                repo.update(&mut conn)?;
            } else {
                repo.insert(&mut conn)?;
            }
            Ok(conn)
        })();

        match result {
            Ok(saved_conn) => {
                if let Some(notifier) = get_notifier(cx) {
                    let event = if is_editing {
                        ConnectionDataEvent::ConnectionUpdated {
                            connection: saved_conn,
                        }
                    } else {
                        ConnectionDataEvent::ConnectionCreated {
                            connection: saved_conn,
                        }
                    };
                    notifier.update(cx, |_, cx| {
                        cx.emit(event);
                    });
                }
                window.remove_window();
            }
            Err(error) => {
                let message = t!("Ftp.save_failed", error = error).to_string();
                tracing::error!("{}", message);
                self.test_result = Some(Err(message));
                cx.notify();
            }
        }
    }

    fn on_cancel(&mut self, window: &mut Window, _cx: &mut Context<Self>) {
        window.remove_window();
    }

    fn render_form_row(&self, label: &str, child: impl IntoElement) -> impl IntoElement {
        h_flex()
            .gap_3()
            .items_center()
            .child(
                div()
                    .w(px(100.0))
                    .text_sm()
                    .text_right()
                    .child(label.to_string()),
            )
            .child(div().flex_1().child(child))
    }
}

fn resolve_ftp_test_params(params: FtpParams, cx: &App) -> Result<FtpParams, String> {
    let connection = StoredConnection::new_ftp("FTP connection test".to_string(), params, None);
    resolve_connection_for_runtime(connection, cx).and_then(|connection| {
        connection
            .to_ftp_params()
            .map_err(|error| error.to_string())
    })
}

impl Focusable for FtpFormWindow {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for FtpFormWindow {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let is_testing = self.is_testing;
        // 与 SSH 表单一致：选择了钥匙串凭据时隐藏用户名/密码行。
        let credential_is_manual = self
            .credential_picker
            .read(cx)
            .selected_reference()
            .is_none();

        let test_result_element = match &self.test_result {
            Some(Ok(())) => Some(
                div()
                    .text_sm()
                    .text_color(cx.theme().success)
                    .child(t!("Ftp.test_success").to_string()),
            ),
            Some(Err(e)) => Some(
                div()
                    .text_sm()
                    .text_color(cx.theme().danger)
                    .child(e.clone()),
            ),
            None => None,
        };

        v_flex()
            .justify_center()
            .size_full()
            // 表单内容
            .child(
                div()
                    .id("ftp-form-content")
                    .flex_1()
                    .p_3()
                    .overflow_y_scroll()
                    .child(
                        v_flex()
                            .gap_2()
                            .child(
                                self.render_form_row(&t!("Ftp.name"), Input::new(&self.name_input)),
                            )
                            .child(
                                self.render_form_row(&t!("Ftp.host"), Input::new(&self.host_input)),
                            )
                            .child(
                                self.render_form_row(
                                    &t!("Ftp.port"),
                                    h_flex()
                                        .gap_2()
                                        .child(
                                            div()
                                                .flex_1()
                                                .child(Select::new(&self.port_select).w_full()),
                                        )
                                        .child(
                                            div().w(px(120.0)).child(Input::new(&self.port_input)),
                                        ),
                                ),
                            )
                            .child(self.render_form_row(
                                &t!("Ftp.keychain"),
                                self.credential_picker.clone(),
                            ))
                            .when(credential_is_manual, |form| {
                                form.child(self.render_form_row(
                                    &t!("Ftp.username"),
                                    Input::new(&self.username_input),
                                ))
                                .child(self.render_form_row(
                                    &t!("Ftp.password"),
                                    Input::new(&self.password_input),
                                ))
                            })
                            .child(
                                self.render_form_row(
                                    &t!("Ftp.passive_mode"),
                                    h_flex()
                                        .gap_2()
                                        .child(
                                            Checkbox::new("passive-mode")
                                                .checked(self.passive_mode)
                                                .on_click(cx.listener(|this, _, _, cx| {
                                                    this.passive_mode = !this.passive_mode;
                                                    cx.notify();
                                                })),
                                        )
                                        .child(
                                            div()
                                                .text_sm()
                                                .text_color(cx.theme().muted_foreground)
                                                .child(t!("Ftp.passive_mode_desc").to_string()),
                                        ),
                                ),
                            )
                            .child(
                                self.render_form_row(
                                    &t!("Ftp.use_tls"),
                                    h_flex()
                                        .gap_2()
                                        .child(
                                            Checkbox::new("use-tls")
                                                .checked(self.use_tls)
                                                .on_click(cx.listener(|this, _, _, cx| {
                                                    this.use_tls = !this.use_tls;
                                                    cx.notify();
                                                })),
                                        )
                                        .child(
                                            div()
                                                .text_sm()
                                                .text_color(cx.theme().muted_foreground)
                                                .child(t!("Ftp.use_tls_desc").to_string()),
                                        ),
                                ),
                            )
                            .child(self.render_form_row(
                                &t!("Ftp.workspace"),
                                Select::new(&self.workspace_select).w_full(),
                            ))
                            .when(
                                connection_sync_controls_visible_in(cx)
                                    && team_management_enabled(cx),
                                |form| {
                                    form.child(
                                        self.render_form_row(
                                            &team_label(),
                                            h_flex()
                                                .gap_2()
                                                .child(Select::new(&self.team_select).w_full())
                                                .child(
                                                    Button::new("sync-ftp-teams")
                                                        .icon(IconName::Refresh)
                                                        .ghost()
                                                        .tooltip(refresh_teams_tooltip())
                                                        .on_click(cx.listener(
                                                            |this, _, window, cx| {
                                                                this.request_team_sync(window, cx);
                                                            },
                                                        )),
                                                ),
                                        ),
                                    )
                                },
                            )
                            .when(connection_sync_controls_visible_in(cx), |form| {
                                form.child(
                                    self.render_form_row(
                                        &t!("ConnectionForm.cloud_sync"),
                                        h_flex()
                                            .gap_2()
                                            .child(
                                                Checkbox::new("sync-enabled")
                                                    .checked(self.sync_enabled)
                                                    .on_click(cx.listener(|this, _, _, cx| {
                                                        this.sync_enabled = !this.sync_enabled;
                                                        cx.notify();
                                                    })),
                                            )
                                            .child(
                                                div()
                                                    .text_sm()
                                                    .text_color(cx.theme().muted_foreground)
                                                    .child(
                                                        t!("ConnectionForm.cloud_sync_desc")
                                                            .to_string(),
                                                    ),
                                            ),
                                    ),
                                )
                            })
                            .child(self.render_form_row(
                                &t!("Ftp.remark"),
                                Textarea::new(&self.remark_input),
                            )),
                    ),
            )
            // 测试结果
            .when_some(test_result_element, |this, elem| {
                this.child(h_flex().justify_center().pb_2().child(elem))
            })
            // 底部按钮
            .child(
                h_flex()
                    .justify_end()
                    .gap_2()
                    .px_6()
                    .py_4()
                    .border_t_1()
                    .border_color(cx.theme().border)
                    .child(
                        Button::new("cancel")
                            .small()
                            .label(t!("Common.cancel").to_string())
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.on_cancel(window, cx);
                            })),
                    )
                    .child(
                        Button::new("test")
                            .small()
                            .outline()
                            .label(if is_testing {
                                t!("Connection.testing").to_string()
                            } else {
                                t!("Ftp.test").to_string()
                            })
                            .disabled(is_testing)
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.on_test(window, cx);
                            })),
                    )
                    .child(
                        Button::new("ok")
                            .small()
                            .primary()
                            .label(t!("Common.ok").to_string())
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.on_save(window, cx);
                            })),
                    ),
            )
    }
}
