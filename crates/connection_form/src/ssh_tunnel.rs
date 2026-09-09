use gpui::prelude::FluentBuilder;
use gpui::{
    App, AppContext, Axis, Context, Entity, IntoElement, ParentElement, Render, SharedString,
    Styled, Window, px,
};
use gpui_component::{
    checkbox::Checkbox,
    form::{field, v_form},
    h_flex,
    input::{Input, InputState, Textarea, TextareaState},
    radio::Radio,
    select::{Select, SelectItem, SelectState},
};
use one_core::storage::{ConnectionType, StoredConnection};
use rust_i18n::t;

use crate::{SshAuthOption, normalize_ssh_auth_type};

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SshTunnelFormValue {
    pub enabled: bool,
    pub connection_id: Option<i64>,
    pub host: String,
    pub port: u16,
    pub username: String,
    pub auth_type: String,
    pub password: Option<String>,
    pub private_key_path: Option<String>,
    pub private_key_content: Option<String>,
    pub private_key_passphrase: Option<String>,
    pub target_host: Option<String>,
    pub target_port: Option<u16>,
    pub timeout: Option<u64>,
}

#[derive(Clone, Debug)]
pub struct SshTunnelFormConfig {
    pub id_prefix: SharedString,
    pub target_host_placeholder: SharedString,
    pub target_port_placeholder: SharedString,
    pub timeout_label: SharedString,
    pub timeout_placeholder: SharedString,
}

impl SshTunnelFormConfig {
    pub fn new(
        id_prefix: impl Into<SharedString>,
        target_host_placeholder: impl Into<SharedString>,
        target_port_placeholder: impl Into<SharedString>,
        timeout_label: impl Into<SharedString>,
        timeout_placeholder: impl Into<SharedString>,
    ) -> Self {
        Self {
            id_prefix: id_prefix.into(),
            target_host_placeholder: target_host_placeholder.into(),
            target_port_placeholder: target_port_placeholder.into(),
            timeout_label: timeout_label.into(),
            timeout_placeholder: timeout_placeholder.into(),
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SshConnectionSelectItem {
    pub id: Option<i64>,
    pub name: String,
}

impl SshConnectionSelectItem {
    pub fn none() -> Self {
        Self {
            id: None,
            name: t!("ConnectionForm.ssh_connection_manual").to_string(),
        }
    }

    pub fn from_connection(connection: &StoredConnection) -> Self {
        let host = connection.to_ssh_params().ok().map(|params| params.host);
        let name = match host.as_deref().filter(|host| !host.trim().is_empty()) {
            Some(host) => format!("{} ({})", connection.name, host),
            None => connection.name.clone(),
        };

        Self {
            id: connection.id,
            name,
        }
    }
}

impl SelectItem for SshConnectionSelectItem {
    type Value = Option<i64>;

    fn title(&self) -> SharedString {
        self.name.clone().into()
    }

    fn value(&self) -> &Self::Value {
        &self.id
    }
}

pub struct SshTunnelForm {
    config: SshTunnelFormConfig,
    connections: Vec<StoredConnection>,
    connection_select: Entity<SelectState<Vec<SshConnectionSelectItem>>>,
    enabled: bool,
    host_input: Entity<InputState>,
    port_input: Entity<InputState>,
    username_input: Entity<InputState>,
    auth_type: String,
    password_input: Entity<InputState>,
    private_key_path_input: Entity<InputState>,
    private_key_content_input: Entity<TextareaState>,
    private_key_passphrase_input: Entity<InputState>,
    target_host_input: Entity<InputState>,
    target_port_input: Entity<InputState>,
    timeout_input: Entity<InputState>,
}

impl SshTunnelForm {
    pub fn new(
        config: SshTunnelFormConfig,
        connections: Vec<StoredConnection>,
        initial: Option<SshTunnelFormValue>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let initial = initial.unwrap_or_else(|| SshTunnelFormValue {
            port: 22,
            auth_type: "password".to_string(),
            ..Default::default()
        });
        let connections = connections
            .into_iter()
            .filter(|connection| connection.connection_type == ConnectionType::SshSftp)
            .collect::<Vec<_>>();

        let mut items = vec![SshConnectionSelectItem::none()];
        items.extend(
            connections
                .iter()
                .map(SshConnectionSelectItem::from_connection),
        );
        let connection_select = cx.new(|cx| {
            let mut state = SelectState::new(items, Some(Default::default()), window, cx);
            if let Some(id) = initial.connection_id {
                state.set_selected_value(&Some(id), window, cx);
            }
            state
        });

        let host_input = input(window, cx, "jump.example.com", initial.host);
        let port_input = input(window, cx, "22", initial.port.to_string());
        let username_input = input(window, cx, "root", initial.username);
        let password_input = masked_input(
            window,
            cx,
            t!("ConnectionForm.enter_password"),
            initial.password,
        );
        let private_key_path_input = input(
            window,
            cx,
            "~/.ssh/id_rsa",
            initial.private_key_path.unwrap_or_default(),
        );
        let private_key_content_input = cx.new(|cx| {
            let mut state = TextareaState::new(window, cx)
                .placeholder(t!("ConnectionForm.ssh_private_key_content_placeholder"))
                .auto_grow(5, 14);
            if let Some(value) = initial.private_key_content.clone() {
                state.set_value(value, window, cx);
            }
            state
        });
        let private_key_passphrase_input = masked_input(
            window,
            cx,
            t!("ConnectionForm.enter_passphrase"),
            initial.private_key_passphrase,
        );
        let target_host_input = input(
            window,
            cx,
            config.target_host_placeholder.clone(),
            initial.target_host.unwrap_or_default(),
        );
        let target_port_input = input(
            window,
            cx,
            config.target_port_placeholder.clone(),
            initial
                .target_port
                .map(|port| port.to_string())
                .unwrap_or_default(),
        );
        let timeout_input = input(
            window,
            cx,
            config.timeout_placeholder.clone(),
            initial
                .timeout
                .map(|timeout| timeout.to_string())
                .unwrap_or_default(),
        );

        Self {
            config,
            connections,
            connection_select,
            enabled: initial.enabled,
            host_input,
            port_input,
            username_input,
            auth_type: normalize_ssh_auth_type(&initial.auth_type).to_string(),
            password_input,
            private_key_path_input,
            private_key_content_input,
            private_key_passphrase_input,
            target_host_input,
            target_port_input,
            timeout_input,
        }
    }

    pub fn value(&self, cx: &App) -> SshTunnelFormValue {
        SshTunnelFormValue {
            enabled: self.enabled,
            connection_id: self.selected_connection_id(cx),
            host: text(&self.host_input, cx),
            port: optional_u16(&self.port_input, cx).unwrap_or(22),
            username: text(&self.username_input, cx),
            auth_type: self.auth_type.clone(),
            password: optional_text(&self.password_input, cx),
            private_key_path: optional_text(&self.private_key_path_input, cx),
            private_key_content: optional_textarea_text(&self.private_key_content_input, cx),
            private_key_passphrase: optional_text(&self.private_key_passphrase_input, cx),
            target_host: optional_text(&self.target_host_input, cx),
            target_port: optional_u16(&self.target_port_input, cx),
            timeout: optional_u64(&self.timeout_input, cx),
        }
    }

    pub fn selected_connection_id(&self, cx: &App) -> Option<i64> {
        self.connection_select
            .read(cx)
            .selected_value()
            .cloned()
            .flatten()
    }

    pub fn selected_connection(&self, cx: &App) -> Option<StoredConnection> {
        let selected_id = self.selected_connection_id(cx)?;
        self.connections
            .iter()
            .find(|connection| connection.id == Some(selected_id))
            .cloned()
    }

    pub fn set_connections(
        &mut self,
        connections: Vec<StoredConnection>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.connections = connections
            .into_iter()
            .filter(|connection| connection.connection_type == ConnectionType::SshSftp)
            .collect();
        let mut items = vec![SshConnectionSelectItem::none()];
        items.extend(
            self.connections
                .iter()
                .map(SshConnectionSelectItem::from_connection),
        );
        let selected = self.selected_connection_id(cx);
        self.connection_select.update(cx, |select, cx| {
            select.set_items(items, window, cx);
            select.set_selected_value(&selected, window, cx);
        });
    }

    fn render_row(
        &self,
        label: impl Into<SharedString>,
        child: impl IntoElement,
    ) -> gpui_component::form::Field {
        field()
            .label(label.into())
            .items_center()
            .justify_end()
            .child(h_flex().w_full().gap_2().child(child))
    }
}

impl Render for SshTunnelForm {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let using_ssh_reference = self.selected_connection_id(cx).is_some();
        let auth_type = normalize_ssh_auth_type(&self.auth_type);
        let prefix = self.config.id_prefix.clone();

        v_form()
            .layout(Axis::Horizontal)
            .columns(1)
            .label_width(px(100.))
            .child(
                self.render_row(
                    t!("ConnectionForm.ssh_tunnel_enabled"),
                    Checkbox::new(format!("{prefix}-enabled"))
                        .checked(self.enabled)
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.enabled = !this.enabled;
                            cx.notify();
                        })),
                ),
            )
            .when(self.enabled, |form| {
                form.child(
                    self.render_row(
                        t!("ConnectionForm.ssh_connection_id"),
                        Select::new(&self.connection_select)
                            .placeholder(t!("ConnectionForm.ssh_connection_manual"))
                            .w_full(),
                    ),
                )
                .when(!using_ssh_reference, |form| {
                    form.child(
                        self.render_row(
                            t!("ConnectionForm.ssh_host"),
                            Input::new(&self.host_input),
                        ),
                    )
                    .child(
                        self.render_row(
                            t!("ConnectionForm.ssh_port"),
                            Input::new(&self.port_input),
                        ),
                    )
                    .child(self.render_row(
                        t!("ConnectionForm.ssh_username"),
                        Input::new(&self.username_input),
                    ))
                    .child(self.render_row(
                        t!("ConnectionForm.ssh_auth_type"),
                        h_flex().w_full().flex_wrap().gap_4().children(
                            SshAuthOption::ALL.iter().copied().map(|option| {
                                Radio::new(format!("{prefix}-auth-{}", option.value()))
                                    .label(option.label())
                                    .checked(auth_type == option.value())
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        this.auth_type = option.value().to_string();
                                        cx.notify();
                                    }))
                            }),
                        ),
                    ))
                    .when(SshAuthOption::Password.value() == auth_type, |form| {
                        form.child(self.render_row(
                            t!("ConnectionForm.ssh_password"),
                            Input::new(&self.password_input).mask_toggle(),
                        ))
                    })
                    .when(SshAuthOption::PrivateKey.value() == auth_type, |form| {
                        form.child(self.render_row(
                            t!("ConnectionForm.ssh_private_key_path"),
                            Input::new(&self.private_key_path_input),
                        ))
                        .child(self.render_row(
                            t!("ConnectionForm.ssh_private_key_passphrase"),
                            Input::new(&self.private_key_passphrase_input).mask_toggle(),
                        ))
                    })
                    .when(
                        SshAuthOption::PrivateKeyContent.value() == auth_type,
                        |form| {
                            form.child(self.render_row(
                                t!("ConnectionForm.ssh_private_key_content"),
                                Textarea::new(&self.private_key_content_input),
                            ))
                            .child(self.render_row(
                                t!("ConnectionForm.ssh_private_key_passphrase"),
                                Input::new(&self.private_key_passphrase_input).mask_toggle(),
                            ))
                        },
                    )
                })
                .child(self.render_row(
                    t!("ConnectionForm.ssh_target_host"),
                    Input::new(&self.target_host_input),
                ))
                .child(self.render_row(
                    t!("ConnectionForm.ssh_target_port"),
                    Input::new(&self.target_port_input),
                ))
                .child(self.render_row(
                    self.config.timeout_label.clone(),
                    Input::new(&self.timeout_input),
                ))
            })
    }
}

fn input(
    window: &mut Window,
    cx: &mut Context<SshTunnelForm>,
    placeholder: impl Into<SharedString>,
    value: String,
) -> Entity<InputState> {
    cx.new(|cx| {
        let mut state = InputState::new(window, cx).placeholder(placeholder);
        if !value.is_empty() {
            state.set_value(value, window, cx);
        }
        state
    })
}

fn masked_input(
    window: &mut Window,
    cx: &mut Context<SshTunnelForm>,
    placeholder: impl Into<SharedString>,
    value: Option<String>,
) -> Entity<InputState> {
    cx.new(|cx| {
        let mut state = InputState::new(window, cx)
            .placeholder(placeholder)
            .masked(true);
        if let Some(value) = value {
            state.set_value(value, window, cx);
        }
        state
    })
}

fn text(input: &Entity<InputState>, cx: &App) -> String {
    input.read(cx).text().to_string().trim().to_string()
}

fn optional_text(input: &Entity<InputState>, cx: &App) -> Option<String> {
    let value = text(input, cx);
    (!value.is_empty()).then_some(value)
}

fn optional_textarea_text(input: &Entity<TextareaState>, cx: &App) -> Option<String> {
    let value = input.read(cx).text().to_string().trim().to_string();
    (!value.is_empty()).then_some(value)
}

fn optional_u16(input: &Entity<InputState>, cx: &App) -> Option<u16> {
    text(input, cx).parse().ok()
}

fn optional_u64(input: &Entity<InputState>, cx: &App) -> Option<u64> {
    text(input, cx).parse().ok()
}
