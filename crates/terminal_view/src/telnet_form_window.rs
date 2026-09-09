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
use gpui::prelude::FluentBuilder;
use gpui::{
    App, AppContext, AsyncApp, Context, Entity, FocusHandle, Focusable, InteractiveElement,
    IntoElement, ParentElement, Render, SharedString, StatefulInteractiveElement, Styled,
    WeakEntity, Window, div, px,
};
use gpui_component::{
    ActiveTheme, Disableable, IconName, IndexPath, Sizable,
    button::{Button, ButtonVariants as _},
    checkbox::Checkbox,
    h_flex,
    input::{Input, InputState, Textarea, TextareaState},
    select::{Select, SelectItem, SelectState},
    v_flex,
};
use one_core::cloud_sync::TeamOption;
use one_core::connection_notifier::{ConnectionDataEvent, get_notifier};
use one_core::gpui_tokio::Tokio;
use one_core::storage::traits::Repository;
use one_core::storage::{
    StoredConnection, TelnetBackspaceCode, TelnetLoginStep, TelnetParams, Workspace,
};
use rust_i18n::t;

pub struct TelnetFormWindowConfig {
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

/// Telnet 默认端口列表，供下拉选择。
#[derive(Clone, PartialEq)]
struct TelnetPortItem {
    port: u16,
}

impl SelectItem for TelnetPortItem {
    type Value = u16;

    fn title(&self) -> SharedString {
        self.port.to_string().into()
    }

    fn value(&self) -> &Self::Value {
        &self.port
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
struct TelnetBackspaceSelectItem {
    code: TelnetBackspaceCode,
}

impl SelectItem for TelnetBackspaceSelectItem {
    type Value = TelnetBackspaceCode;

    fn title(&self) -> SharedString {
        self.code.label().into()
    }

    fn value(&self) -> &Self::Value {
        &self.code
    }
}

#[derive(Clone)]
struct TelnetLoginStepInput {
    expect_input: Entity<InputState>,
    send_input: Entity<InputState>,
}

pub struct TelnetFormWindow {
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
    port_select: Entity<SelectState<Vec<TelnetPortItem>>>,
    backspace_code_select: Entity<SelectState<Vec<TelnetBackspaceSelectItem>>>,
    workspace_select: Entity<SelectState<Vec<WorkspaceSelectItem>>>,
    team_select: Entity<SelectState<Vec<TeamSelectItem>>>,
    remark_input: Entity<TextareaState>,
    credential_picker: Entity<CredentialReferencePicker>,
    login_script_rows: Vec<TelnetLoginStepInput>,
    sync_enabled: bool,

    is_testing: bool,
    test_result: Option<Result<(), String>>,
}

const TELNET_DEFAULT_PORTS: &[u16] = &[23, 2323, 8000, 8023];
const TELNET_DEFAULT_PORT: u16 = 23;

fn collect_login_script_steps(
    rows: impl IntoIterator<Item = (String, String)>,
) -> Option<Vec<TelnetLoginStep>> {
    let mut steps = Vec::new();
    for (expect, send) in rows {
        if expect.trim().is_empty() {
            if send.trim().is_empty() {
                continue;
            }
            return None;
        }
        let Ok(expect_regex) = regex::bytes::Regex::new(&expect) else {
            return None;
        };
        if expect_regex.is_match(b"") {
            return None;
        }
        steps.push(TelnetLoginStep { expect, send });
    }
    Some(steps)
}

impl TelnetFormWindow {
    pub fn new(
        config: TelnetFormWindowConfig,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
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
            cx.new(|cx| InputState::new(window, cx).placeholder(t!("Telnet.name_placeholder")));
        let host_input =
            cx.new(|cx| InputState::new(window, cx).placeholder(t!("Telnet.host_placeholder")));
        let port_input =
            cx.new(|cx| InputState::new(window, cx).placeholder(t!("Telnet.port_placeholder")));
        let remark_input = cx.new(|cx| {
            TextareaState::new(window, cx)
                .placeholder(t!("Telnet.remark_placeholder"))
                .auto_grow(3, 10)
        });

        // 端口选择，默认 23（索引 0）
        let port_items: Vec<TelnetPortItem> = TELNET_DEFAULT_PORTS
            .iter()
            .map(|&port| TelnetPortItem { port })
            .collect();
        let port_select = cx
            .new(|cx| SelectState::new(port_items, Some(IndexPath::default().row(0)), window, cx));
        let backspace_code_items = TelnetBackspaceCode::all()
            .iter()
            .copied()
            .map(|code| TelnetBackspaceSelectItem { code })
            .collect::<Vec<_>>();
        let backspace_code_select = cx.new(|cx| {
            let mut state = SelectState::new(backspace_code_items, None, window, cx);
            state.set_selected_value(&TelnetBackspaceCode::default(), window, cx);
            state
        });

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
        let mut login_script_steps = Vec::new();

        // 编辑模式：加载已有数据
        if let Some(ref conn) = config.editing_connection {
            sync_enabled = conn.sync_enabled;

            if let Ok(params) = conn.to_telnet_params() {
                name_input.update(cx, |s, cx| s.set_value(&conn.name, window, cx));
                host_input.update(cx, |s, cx| s.set_value(&params.host, window, cx));
                let port = params.port.to_string();
                port_input.update(cx, |s, cx| s.set_value(&port, window, cx));
                port_select.update(cx, |s, cx| {
                    s.set_selected_value(&params.port, window, cx);
                });
                backspace_code_select.update(cx, |select, cx| {
                    select.set_selected_value(&params.backspace_code, window, cx);
                });
                credential_reference = params.credential_reference.clone();
                login_script_steps = params.login_script;
            }
            workspace_id = conn.workspace_id;
            team_id = conn.team_id.clone();

            if let Some(ref remark) = conn.remark {
                remark_input.update(cx, |s, cx| s.set_value(remark, window, cx));
            }
        }

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
            CredentialPickerConfig::new("telnet-credential", CredentialCapabilities::login())
                .reference(credential_reference),
            window,
            cx,
        );
        cx.subscribe(&credential_picker, |_, _, _: &CredentialPickerEvent, cx| {
            cx.notify()
        })
        .detach();

        let login_script_rows = login_script_steps
            .into_iter()
            .map(|step| {
                let row = Self::new_login_script_row(window, cx);
                row.expect_input
                    .update(cx, |input, cx| input.set_value(&step.expect, window, cx));
                row.send_input
                    .update(cx, |input, cx| input.set_value(&step.send, window, cx));
                row
            })
            .collect();

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
            backspace_code_select,
            workspace_select,
            team_select,
            remark_input,
            credential_picker,
            login_script_rows,
            sync_enabled,
            is_testing: false,
            test_result: None,
        }
    }

    fn new_login_script_row(window: &mut Window, cx: &mut Context<Self>) -> TelnetLoginStepInput {
        TelnetLoginStepInput {
            expect_input: cx.new(|cx| {
                InputState::new(window, cx)
                    .placeholder(t!("Telnet.login_script_expect_placeholder"))
            }),
            send_input: cx.new(|cx| {
                InputState::new(window, cx).placeholder(t!("Telnet.login_script_send_placeholder"))
            }),
        }
    }

    fn add_login_script_row(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.login_script_rows
            .push(Self::new_login_script_row(window, cx));
        cx.notify();
    }

    fn remove_login_script_row(&mut self, index: usize, cx: &mut Context<Self>) {
        if index < self.login_script_rows.len() {
            self.login_script_rows.remove(index);
            cx.notify();
        }
    }

    fn collect_login_script(&self, cx: &App) -> Option<Vec<TelnetLoginStep>> {
        collect_login_script_steps(self.login_script_rows.iter().map(|row| {
            (
                row.expect_input.read(cx).text().to_string(),
                row.send_input.read(cx).text().to_string(),
            )
        }))
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
            .or(Some(TELNET_DEFAULT_PORT))
    }

    fn build_telnet_params(&self, cx: &App) -> Option<TelnetParams> {
        let host = self.get_host(cx);
        if host.is_empty() {
            return None;
        }
        let port = self.get_port(cx)?;
        if port == 0 {
            return None;
        }
        let login_script = self.collect_login_script(cx)?;
        Some(TelnetParams {
            host,
            port,
            credential_reference: self.credential_picker.read(cx).selected_reference(),
            prompt_username: None,
            prompt_password: None,
            backspace_code: self
                .backspace_code_select
                .read(cx)
                .selected_value()
                .copied()
                .unwrap_or_default(),
            login_script,
        })
    }

    fn validation_error(&self, cx: &App) -> Option<String> {
        if self.get_host(cx).is_empty() || self.get_port(cx).is_none_or(|port| port == 0) {
            return Some(t!("Telnet.validation_error").to_string());
        }
        if self.collect_login_script(cx).is_none() {
            return Some(t!("Telnet.login_script_validation_error").to_string());
        }
        None
    }

    fn on_test(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(params) = self.build_telnet_params(cx) else {
            self.test_result = Some(Err(self
                .validation_error(cx)
                .unwrap_or_else(|| t!("Telnet.validation_error").to_string())));
            cx.notify();
            return;
        };
        let params = match resolve_telnet_test_params(params, cx) {
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
            let spawn_result = Tokio::spawn_result(cx, async move {
                tokio::time::timeout(
                    std::time::Duration::from_secs(5),
                    tokio::net::TcpStream::connect((params.host.as_str(), params.port)),
                )
                .await
                .map_err(|_| anyhow::anyhow!(t!("Telnet.test_timeout").to_string()))?
                .map_err(|error| anyhow::anyhow!(error.to_string()))?;
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
        let Some(params) = self.build_telnet_params(cx) else {
            self.test_result = Some(Err(self
                .validation_error(cx)
                .unwrap_or_else(|| t!("Telnet.validation_error").to_string())));
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
        let mut conn = StoredConnection::new_telnet(name, params, workspace_id);
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
                let message = t!("Telnet.save_failed", error = error).to_string();
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

fn resolve_telnet_test_params(params: TelnetParams, cx: &App) -> Result<TelnetParams, String> {
    let connection =
        StoredConnection::new_telnet("Telnet connection test".to_string(), params, None);
    resolve_connection_for_runtime(connection, cx).and_then(|connection| {
        connection
            .to_telnet_params()
            .map_err(|error| error.to_string())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn collect_login_script_steps_skips_blank_rows() {
        let steps = collect_login_script_steps(vec![
            ("".to_string(), "".to_string()),
            ("  ".to_string(), "\t".to_string()),
            ("login:".to_string(), "admin".to_string()),
        ]);

        assert_eq!(
            steps,
            Some(vec![TelnetLoginStep {
                expect: "login:".to_string(),
                send: "admin".to_string(),
            }])
        );
    }

    #[test]
    fn collect_login_script_steps_rejects_send_without_expect() {
        assert_eq!(
            collect_login_script_steps(vec![("".to_string(), "admin".to_string())]),
            None
        );
        assert_eq!(
            collect_login_script_steps(vec![("   ".to_string(), "admin".to_string())]),
            None
        );
    }

    #[test]
    fn collect_login_script_steps_preserves_non_blank_text() {
        assert_eq!(
            collect_login_script_steps(vec![("login:  ".to_string(), "  admin\r".to_string(),)]),
            Some(vec![TelnetLoginStep {
                expect: "login:  ".to_string(),
                send: "  admin\r".to_string(),
            }])
        );
    }

    #[test]
    fn collect_login_script_steps_rejects_invalid_regex() {
        assert_eq!(
            collect_login_script_steps(vec![("(?i)(login".to_string(), "admin".to_string())]),
            None
        );
    }

    #[test]
    fn collect_login_script_steps_rejects_regex_that_matches_empty_input() {
        assert_eq!(
            collect_login_script_steps(vec![("a*".to_string(), "admin".to_string())]),
            None
        );
    }

    #[test]
    fn connection_test_resolves_keychain_reference_before_opening_tcp_socket() {
        let source = include_str!("telnet_form_window.rs");
        let on_test = source
            .split("fn on_test(")
            .nth(1)
            .and_then(|source| source.split("fn on_save(").next())
            .expect("Telnet on_test source");

        let resolve = on_test
            .find("resolve_telnet_test_params")
            .expect("runtime credential resolution");
        let connect = on_test
            .find("TcpStream::connect")
            .expect("Telnet TCP connection");
        assert!(resolve < connect);
        assert!(source.contains("resolve_connection_for_runtime(connection, cx)"));
        assert!(source.contains(".to_telnet_params()"));
    }
}

impl Focusable for TelnetFormWindow {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for TelnetFormWindow {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let is_testing = self.is_testing;

        let test_result_element = match &self.test_result {
            Some(Ok(())) => Some(
                div()
                    .text_sm()
                    .text_color(cx.theme().success)
                    .child(t!("Telnet.test_tcp_success").to_string()),
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
                    .id("telnet-form-content")
                    .flex_1()
                    .p_3()
                    .overflow_y_scroll()
                    .child(
                        v_flex()
                            .gap_2()
                            .child(
                                self.render_form_row(
                                    &t!("Telnet.name"),
                                    Input::new(&self.name_input),
                                ),
                            )
                            .child(
                                self.render_form_row(
                                    &t!("Telnet.host"),
                                    Input::new(&self.host_input),
                                ),
                            )
                            .child(
                                self.render_form_row(
                                    &t!("Telnet.port"),
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
                                &t!("Telnet.backspace_code"),
                                Select::new(&self.backspace_code_select).w_full(),
                            ))
                            .child(self.render_form_row(
                                &t!("Telnet.keychain"),
                                self.credential_picker.clone(),
                            ))
                            .child(self.render_form_row(
                                &t!("Telnet.workspace"),
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
                                                    Button::new("sync-telnet-teams")
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
                            .child(
                                v_flex()
                                    .gap_2()
                                    .child(
                                        div()
                                            .text_sm()
                                            .text_color(cx.theme().muted_foreground)
                                            .child(t!("Telnet.login_script").to_string()),
                                    )
                                    .child(
                                        div()
                                            .text_xs()
                                            .text_color(cx.theme().muted_foreground)
                                            .child(t!("Telnet.login_script_hint").to_string()),
                                    )
                                    .child(div().text_xs().text_color(cx.theme().warning).child(
                                        t!("Telnet.login_script_credential_hint").to_string(),
                                    ))
                                    .child(
                                        h_flex()
                                            .gap_2()
                                            .child(div().w(px(28.0)))
                                            .child(
                                                div()
                                                    .flex_1()
                                                    .text_xs()
                                                    .text_color(cx.theme().muted_foreground)
                                                    .child(
                                                        t!("Telnet.login_script_expect")
                                                            .to_string(),
                                                    ),
                                            )
                                            .child(
                                                div()
                                                    .flex_1()
                                                    .text_xs()
                                                    .text_color(cx.theme().muted_foreground)
                                                    .child(
                                                        t!("Telnet.login_script_send").to_string(),
                                                    ),
                                            )
                                            .child(div().w(px(28.0))),
                                    )
                                    .children(self.login_script_rows.iter().enumerate().map(
                                        |(index, row)| {
                                            h_flex()
                                                .gap_2()
                                                .items_center()
                                                .child(
                                                    div()
                                                        .w(px(28.0))
                                                        .text_sm()
                                                        .text_color(cx.theme().muted_foreground)
                                                        .child(format!("{}", index + 1)),
                                                )
                                                .child(
                                                    div().flex_1().child(
                                                        Input::new(&row.expect_input).small(),
                                                    ),
                                                )
                                                .child(
                                                    div()
                                                        .flex_1()
                                                        .child(Input::new(&row.send_input).small()),
                                                )
                                                .child(
                                                    Button::new((
                                                        "remove-telnet-login-step",
                                                        index,
                                                    ))
                                                    .small()
                                                    .ghost()
                                                    .icon(IconName::Close)
                                                    .tooltip(
                                                        t!("Telnet.login_script_remove")
                                                            .to_string(),
                                                    )
                                                    .on_click(cx.listener(
                                                        move |this, _, _window, cx| {
                                                            this.remove_login_script_row(index, cx);
                                                        },
                                                    )),
                                                )
                                        },
                                    ))
                                    .child(
                                        Button::new("add-telnet-login-step")
                                            .small()
                                            .outline()
                                            .icon(IconName::Plus)
                                            .label(t!("Telnet.login_script_add").to_string())
                                            .on_click(cx.listener(|this, _, window, cx| {
                                                this.add_login_script_row(window, cx);
                                            })),
                                    ),
                            )
                            .child(self.render_form_row(
                                &t!("Telnet.remark"),
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
                                t!("Telnet.test_tcp").to_string()
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
