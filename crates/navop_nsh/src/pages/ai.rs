use std::{
    collections::BTreeMap,
    sync::{
        Arc, Mutex,
        mpsc::{Receiver, Sender, channel},
    },
    time::Duration,
};

use agent_client_protocol::schema::v1::{ContentBlock, ImageContent, TextContent};
use agent_runtime::RuntimeEvent;
use ai_chat_view::{
    AcpAgentConfig, AcpClientProviders, AcpConnectOutcome, AcpConnection, AcpElicitationField,
    AcpElicitationFieldKind, AcpElicitationForm, AcpElicitationFuture, AcpElicitationMode,
    AcpElicitationOption, AcpElicitationOutcome, AcpElicitationProvider, AcpElicitationRequest,
    AcpPermissionFuture, AcpPermissionOption, AcpPermissionOutcome, AcpPermissionProvider,
    AcpPermissionRequest,
};
use base64::{Engine as _, engine::general_purpose::STANDARD};
use nsh_base::{input, switch, text_view, theme};
use nsh_core::prelude::*;
use serde_json::Value;

use crate::{services::NavopNshServices, ui};

#[derive(Clone)]
struct AcpSession(Arc<AcpConnection>);

impl PartialEq for AcpSession {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}

#[derive(Clone)]
struct PendingPermission {
    request_id: String,
    tool_name: String,
    summary: String,
    options: Vec<AcpPermissionOption>,
    resolver: Arc<Mutex<Option<tokio::sync::oneshot::Sender<AcpPermissionOutcome>>>>,
}

impl PendingPermission {
    fn allow_option_id(&self) -> Option<String> {
        self.options
            .iter()
            .find(|option| option.kind.starts_with("allow"))
            .or_else(|| self.options.first())
            .map(|option| option.option_id.clone())
    }
}

impl PartialEq for PendingPermission {
    fn eq(&self, other: &Self) -> bool {
        self.request_id == other.request_id
    }
}

#[derive(Clone)]
struct PendingElicitation {
    request: AcpElicitationRequest,
    resolver: Arc<Mutex<Option<tokio::sync::oneshot::Sender<AcpElicitationOutcome>>>>,
}

impl PartialEq for PendingElicitation {
    fn eq(&self, other: &Self) -> bool {
        self.request.request_id == other.request.request_id
    }
}

pub(crate) fn ai() -> impl IntoElement {
    let services = NavopNshServices::current();
    let agents = services
        .as_ref()
        .map(|services| services.acp_agents.clone())
        .unwrap_or_default();
    let selected_agent = signal(None::<String>);
    let session = signal(None::<AcpSession>);
    let generation = signal(0u64);
    let prompt = signal(String::new());
    let image_path = signal(String::new());
    let transcript = signal(String::from("Choose an ACP agent to start a conversation."));
    let status = signal(String::from("No agent connected"));
    let busy = signal(false);
    let permission = signal(None::<PendingPermission>);
    let (permission_provider, permission_requests) = permission_channel();
    install_permission_pump(permission_requests, permission);
    let elicitation = signal(None::<PendingElicitation>);
    let elicitation_values = signal(BTreeMap::<String, Value>::new());
    let (elicitation_provider, elicitation_requests) = elicitation_channel();
    install_elicitation_pump(elicitation_requests, elicitation, elicitation_values);

    let connect = {
        let services = services.clone();
        let agents = agents.clone();
        let permission_provider = permission_provider.clone();
        let elicitation_provider = elicitation_provider.clone();
        move |id: String| {
            let Some(connection) = agents.iter().find(|agent| agent.id.as_ref() == id) else {
                status.set("ACP agent was not found".to_owned());
                return;
            };
            let Some(services) = services.clone() else {
                status.set("AI runtime is unavailable".to_owned());
                return;
            };
            let config = connection.clone();
            let permission_provider = permission_provider.clone();
            let elicitation_provider = elicitation_provider.clone();
            let name = config.name.to_string();
            selected_agent.set(Some(id));
            session.set(None);
            generation.update(|generation| *generation += 1);
            transcript.set(format!("Connecting to {name}…"));
            busy.set(true);
            status.set(format!("Connecting to {name}…"));
            spawn(async move {
                let handle = services.tokio.clone();
                let workspace_root = services.workspace_root.clone();
                let connected = background(async move {
                    connect_acp(
                        config,
                        workspace_root,
                        handle,
                        permission_provider,
                        elicitation_provider,
                    )
                    .await
                })
                .await;
                busy.set(false);
                match connected {
                    Ok(Ok(connection)) => {
                        attach_connection(
                            connection,
                            services.tokio.clone(),
                            session,
                            generation,
                            transcript,
                            status,
                        );
                    }
                    Ok(Err(error)) => {
                        transcript.set(format!("AI connection failed:\n\n{error:#}"));
                        status.set("Connection failed".to_owned());
                    }
                    Err(error) => {
                        transcript.set(format!("AI connection task failed:\n\n{error}"));
                        status.set("Connection task failed".to_owned());
                    }
                }
            });
        }
    };
    let send = move || {
        let Some(session) = session.get_untracked() else {
            status.set("Connect an ACP agent first".to_owned());
            return;
        };
        let text = prompt.get_untracked();
        let text = text.trim();
        if text.is_empty() {
            return;
        }
        let blocks = match build_prompt_blocks(text, &image_path.get_untracked()) {
            Ok(blocks) => blocks,
            Err(error) => {
                status.set(error);
                return;
            }
        };
        match session.0.try_prompt(blocks) {
            Ok(turn_id) => {
                transcript.update(|transcript| {
                    if !transcript.is_empty() {
                        transcript.push_str("\n\n");
                    }
                    transcript.push_str("You: ");
                    transcript.push_str(text);
                    if !image_path.get_untracked().trim().is_empty() {
                        transcript.push_str("\n[image attached: ");
                        transcript.push_str(image_path.get_untracked().trim());
                        transcript.push(']');
                    }
                    transcript.push_str("\n\nAssistant: ");
                });
                prompt.set(String::new());
                image_path.set(String::new());
                status.set(format!("Running turn {}", turn_id.as_str()));
            }
            Err(error) => status.set(format!("Prompt failed: {error}")),
        }
    };
    let disconnect = move || {
        generation.update(|generation| *generation += 1);
        session.set(None);
        status.set("Disconnected".to_owned());
    };
    let stop = move || {
        if let Some(session) = session.get_untracked() {
            session.0.cancel();
            status.set("Cancelling turn…".to_owned());
        }
    };

    view()
        .size_full()
        .child(ui::page_header(
            "AI",
            "ACP conversation surface using the shared agent runtime",
        ))
        .child(
            view()
                .flex_none()
                .flex_row()
                .items_center()
                .gap_2()
                .px_4()
                .py_3()
                .border_b_1()
                .border_b_color(move || theme().border)
                .child(label("ACP AGENTS"))
                .when(agents.is_empty(), |row| {
                    row.child(
                        text("No enabled ACP agents configured")
                            .text_xs()
                            .text_color(move || theme().muted_foreground),
                    )
                })
                .children(agents.iter().cloned().map({
                    let connect = connect.clone();
                    move |agent| {
                        let id = agent.id.to_string();
                        let active_id = id.clone();
                        let active =
                            move || selected_agent.get().as_deref() == Some(active_id.as_str());
                        let name = agent.name.to_string();
                        view()
                            .px_3()
                            .py_1p5()
                            .rounded(move || theme().radius)
                            .role(AccessibilityRole::Button)
                            .label(name.clone())
                            .bg(move || {
                                if active() {
                                    theme().accent
                                } else {
                                    theme().muted
                                }
                            })
                            .on_press({
                                let connect = connect.clone();
                                let id = id.clone();
                                move || connect(id.clone())
                            })
                            .child(text(name).text_xs().text_color(move || theme().foreground))
                    }
                }))
                .child(view().flex_1())
                .child(
                    text(move || status.get())
                        .text_xs()
                        .text_color(move || theme().muted_foreground),
                )
                .child(
                    ui::secondary_button("Stop")
                        .disabled(move || session.get().is_none())
                        .on_press(stop),
                )
                .child(
                    ui::secondary_button("Disconnect")
                        .disabled(move || session.get().is_none())
                        .on_press(disconnect),
                ),
        )
        .child(
            view()
                .flex_1()
                .min_h_0()
                .p_4()
                .gap_3()
                .child(
                    scroll()
                        .flex_1()
                        .min_h_0()
                        .p_4()
                        .rounded(move || theme().radius)
                        .border_1()
                        .border_color(move || theme().border)
                        .bg(move || theme().card)
                        .child(
                            text_view()
                                .markdown(move || Some(SharedString::from(transcript.get())))
                                .text_color(move || theme().foreground),
                        ),
                )
                .child(dynamic(move || {
                    if let Some(prompt) = permission.get() {
                        permission_card(prompt, permission, status).into_element()
                    } else {
                        view().into_element()
                    }
                }))
                .child(dynamic(move || {
                    if let Some(prompt) = elicitation.get() {
                        elicitation_card(prompt, elicitation, elicitation_values, status)
                            .into_element()
                    } else {
                        view().into_element()
                    }
                }))
                .child(
                    input()
                        .value(move || image_path.get())
                        .on_change(move |value: &str| image_path.set(value.to_owned()))
                        .placeholder("Optional image attachment path (.png, .jpg, .webp, .gif)")
                        .h(px(34.0))
                        .px_3()
                        .rounded(move || theme().radius)
                        .border_1()
                        .border_color(move || theme().input)
                        .text_sm()
                        .text_color(move || theme().foreground),
                )
                .child(
                    view()
                        .flex_row()
                        .items_end()
                        .gap_2()
                        .child(
                            input()
                                .multiline(true)
                                .value(move || prompt.get())
                                .on_change(move |value: &str| prompt.set(value.to_owned()))
                                .on_submit(move |_: &str| send())
                                .placeholder("Ask the selected ACP agent…")
                                .h(px(92.0))
                                .flex_1()
                                .p_3()
                                .rounded(move || theme().radius)
                                .border_1()
                                .border_color(move || theme().input)
                                .text_color(move || theme().foreground),
                        )
                        .child(
                            ui::primary_button("Send")
                                .disabled(move || busy.get() || session.get().is_none())
                                .on_press(send),
                        ),
                ),
        )
}

async fn connect_acp(
    config: AcpAgentConfig,
    workspace_root: std::path::PathBuf,
    handle: tokio::runtime::Handle,
    permission_provider: AcpPermissionProvider,
    elicitation_provider: AcpElicitationProvider,
) -> anyhow::Result<AcpConnection> {
    let outcome = AcpConnection::connect_with_runtime_and_providers(
        &config,
        workspace_root,
        handle.clone(),
        AcpClientProviders {
            permission: Some(permission_provider),
            elicitation: Some(elicitation_provider),
        },
    )
    .await?;
    match outcome {
        AcpConnectOutcome::Ready(connection) => Ok(*connection),
        AcpConnectOutcome::AuthenticationRequired(pending) => {
            let methods = pending.methods();
            let method = methods.first().cloned().ok_or_else(|| {
                anyhow::anyhow!("ACP agent requires authentication, but advertises no methods")
            })?;
            let connection = pending.authenticate(method).await?;
            Ok(connection)
        }
    }
}

type PermissionRequest = (
    AcpPermissionRequest,
    tokio::sync::oneshot::Sender<AcpPermissionOutcome>,
);

fn permission_channel() -> (AcpPermissionProvider, Receiver<PermissionRequest>) {
    let (sender, receiver) = channel();
    let provider: AcpPermissionProvider = Arc::new(move |request| {
        let sender: Sender<PermissionRequest> = sender.clone();
        Box::pin(async move {
            let (response_tx, response_rx) = tokio::sync::oneshot::channel();
            if sender.send((request, response_tx)).is_err() {
                return AcpPermissionOutcome::Cancelled;
            }
            response_rx.await.unwrap_or(AcpPermissionOutcome::Cancelled)
        }) as AcpPermissionFuture
    });
    (provider, receiver)
}

fn install_permission_pump(
    requests: Receiver<PermissionRequest>,
    permission: Signal<Option<PendingPermission>>,
) {
    set_interval(Duration::from_millis(24), move || {
        while let Ok((request, response)) = requests.try_recv() {
            permission.set(Some(PendingPermission {
                request_id: request.request_id,
                tool_name: request.tool_name,
                summary: request.summary,
                options: request.options,
                resolver: Arc::new(Mutex::new(Some(response))),
            }));
        }
    });
}

type ElicitationRequest = (
    AcpElicitationRequest,
    tokio::sync::oneshot::Sender<AcpElicitationOutcome>,
);

fn elicitation_channel() -> (AcpElicitationProvider, Receiver<ElicitationRequest>) {
    let (sender, receiver) = channel();
    let provider: AcpElicitationProvider = Arc::new(move |request| {
        let sender: Sender<ElicitationRequest> = sender.clone();
        Box::pin(async move {
            let (response_tx, response_rx) = tokio::sync::oneshot::channel();
            if sender.send((request, response_tx)).is_err() {
                return AcpElicitationOutcome::Cancel;
            }
            response_rx.await.unwrap_or(AcpElicitationOutcome::Cancel)
        }) as AcpElicitationFuture
    });
    (provider, receiver)
}

fn install_elicitation_pump(
    requests: Receiver<ElicitationRequest>,
    elicitation: Signal<Option<PendingElicitation>>,
    values: Signal<BTreeMap<String, Value>>,
) {
    set_interval(Duration::from_millis(24), move || {
        while let Ok((request, response)) = requests.try_recv() {
            let defaults = elicitation_defaults(&request);
            values.set(defaults);
            elicitation.set(Some(PendingElicitation {
                request,
                resolver: Arc::new(Mutex::new(Some(response))),
            }));
        }
    });
}

fn elicitation_defaults(request: &AcpElicitationRequest) -> BTreeMap<String, Value> {
    let AcpElicitationMode::Form(form) = &request.mode else {
        return BTreeMap::new();
    };
    form.fields
        .iter()
        .filter_map(|field| {
            field
                .default
                .clone()
                .map(|default| (field.name.clone(), default))
                .or_else(|| match &field.kind {
                    AcpElicitationFieldKind::Boolean => {
                        Some((field.name.clone(), Value::Bool(false)))
                    }
                    AcpElicitationFieldKind::MultiSelect { .. } => {
                        Some((field.name.clone(), Value::Array(Vec::new())))
                    }
                    _ => None,
                })
        })
        .collect()
}

fn permission_card(
    prompt: PendingPermission,
    permission: Signal<Option<PendingPermission>>,
    status: Signal<String>,
) -> impl IntoElement {
    let allow = prompt.clone();
    let deny = prompt.clone();
    ui::card()
        .flex_none()
        .border_color(move || theme().ring)
        .gap_2()
        .child(
            text(format!("Tool permission · {}", prompt.tool_name))
                .text_sm()
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(move || theme().foreground),
        )
        .child(
            text(prompt.summary)
                .text_sm()
                .text_color(move || theme().muted_foreground),
        )
        .child(
            view()
                .flex_row()
                .gap_2()
                .child(ui::primary_button("Allow").on_press(move || {
                    let outcome = allow
                        .allow_option_id()
                        .map(|option_id| AcpPermissionOutcome::Selected { option_id })
                        .unwrap_or(AcpPermissionOutcome::Cancelled);
                    if let Some(response) =
                        allow.resolver.lock().ok().and_then(|mut slot| slot.take())
                    {
                        let _ = response.send(outcome);
                    }
                    permission.set(None);
                    status.set("Tool permission granted".to_owned());
                }))
                .child(ui::secondary_button("Deny").on_press(move || {
                    if let Some(response) =
                        deny.resolver.lock().ok().and_then(|mut slot| slot.take())
                    {
                        let _ = response.send(AcpPermissionOutcome::Cancelled);
                    }
                    permission.set(None);
                    status.set("Tool permission denied".to_owned());
                })),
        )
}

fn elicitation_card(
    prompt: PendingElicitation,
    elicitation: Signal<Option<PendingElicitation>>,
    values: Signal<BTreeMap<String, Value>>,
    status: Signal<String>,
) -> impl IntoElement {
    let accept = prompt.clone();
    let decline = prompt.clone();
    let cancel = prompt;
    let message = accept.request.message.clone();
    let body: AnyElement = match &accept.request.mode {
        AcpElicitationMode::Form(form) => elicitation_form(form, values).into_element(),
        AcpElicitationMode::Url { url } => view()
            .gap_1()
            .child(
                text("Complete this step in your browser:")
                    .text_sm()
                    .text_color(move || theme().muted_foreground),
            )
            .child(
                text(url.clone())
                    .text_sm()
                    .text_color(move || theme().primary),
            )
            .into_element(),
        AcpElicitationMode::Unsupported { mode } => text(format!(
            "The agent requested unsupported elicitation mode: {mode}"
        ))
        .text_sm()
        .text_color(move || theme().muted_foreground)
        .into_element(),
    };
    let answerable = accept.request.is_answerable();

    ui::card()
        .flex_none()
        .border_color(move || theme().ring)
        .gap_2()
        .child(
            text("Agent needs your input")
                .text_sm()
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(move || theme().foreground),
        )
        .child(
            text(message)
                .text_sm()
                .text_color(move || theme().muted_foreground),
        )
        .child(body)
        .child(
            view()
                .flex_row()
                .gap_2()
                .when(answerable, |row| {
                    row.child(ui::primary_button("Submit").on_press(move || {
                        match collect_elicitation_content(&accept.request, &values.get_untracked())
                        {
                            Ok(content) => {
                                resolve_elicitation(
                                    &accept,
                                    AcpElicitationOutcome::Accept(content),
                                    &status,
                                    "Elicitation response submitted",
                                );
                                elicitation.set(None);
                            }
                            Err(error) => status.set(error),
                        }
                    }))
                })
                .child(ui::secondary_button("Decline").on_press(move || {
                    resolve_elicitation(
                        &decline,
                        AcpElicitationOutcome::Decline,
                        &status,
                        "Elicitation request declined",
                    );
                    elicitation.set(None);
                }))
                .child(ui::secondary_button("Cancel").on_press(move || {
                    resolve_elicitation(
                        &cancel,
                        AcpElicitationOutcome::Cancel,
                        &status,
                        "Elicitation request cancelled",
                    );
                    elicitation.set(None);
                })),
        )
}

fn elicitation_form(
    form: &AcpElicitationForm,
    values: Signal<BTreeMap<String, Value>>,
) -> impl IntoElement {
    let title = form.title.clone();
    let description = form.description.clone();
    view()
        .gap_2()
        .when_some(title, |form, title| {
            form.child(
                text(title)
                    .text_sm()
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(move || theme().foreground),
            )
        })
        .when_some(description, |form, description| {
            form.child(
                text(description)
                    .text_xs()
                    .text_color(move || theme().muted_foreground),
            )
        })
        .children(
            form.fields
                .clone()
                .into_iter()
                .map(move |field| elicitation_field(field, values)),
        )
}

fn elicitation_field(
    field: AcpElicitationField,
    values: Signal<BTreeMap<String, Value>>,
) -> impl IntoElement {
    let name = field.name.clone();
    let title = if field.required {
        format!("{} *", field.title)
    } else {
        field.title.clone()
    };
    let description = field.description.clone();
    let value_for_input = values;
    let checked_field_name = name.clone();
    let checkbox_field_name = name.clone();
    let input_field_name = name.clone();
    let input_placeholder = match &field.kind {
        AcpElicitationFieldKind::Integer => "Integer",
        AcpElicitationFieldKind::Number => "Number",
        _ => "Value",
    };
    let control: AnyElement = match field.kind {
        AcpElicitationFieldKind::Boolean => switch()
            .checked(move || elicitation_bool(&values.get(), &checked_field_name))
            .track_class("w-9 h-5 rounded-full bg-muted checked:bg-primary")
            .thumb_class("size-4 rounded-full bg-background checked:translate-x-4")
            .accessibility_label(title.clone())
            .on_change(move |next: &bool| {
                values.update(|values| {
                    values.insert(checkbox_field_name.clone(), Value::Bool(*next));
                });
            })
            .into_element(),
        AcpElicitationFieldKind::SingleSelect { options } => {
            let field_name = name.clone();
            view()
                .flex_row()
                .flex_wrap()
                .gap_1()
                .children(options.into_iter().map(move |option| {
                    elicitation_option_button(option, field_name.clone(), "single", values)
                }))
                .into_element()
        }
        AcpElicitationFieldKind::MultiSelect { options } => {
            let field_name = name.clone();
            view()
                .flex_row()
                .flex_wrap()
                .gap_1()
                .children(options.into_iter().map(move |option| {
                    elicitation_option_button(option, field_name.clone(), "multi", values)
                }))
                .into_element()
        }
        AcpElicitationFieldKind::Text { .. }
        | AcpElicitationFieldKind::Integer
        | AcpElicitationFieldKind::Number => {
            let field_name = name.clone();
            input()
                .value(move || elicitation_text(&value_for_input.get(), &input_field_name))
                .on_change(move |next: &str| {
                    values.update(|values| {
                        values.insert(field_name.clone(), Value::String(next.to_owned()));
                    });
                })
                .placeholder(input_placeholder)
                .h(px(34.0))
                .px_3()
                .rounded(move || theme().radius)
                .border_1()
                .border_color(move || theme().input)
                .text_sm()
                .text_color(move || theme().foreground)
                .into_element()
        }
        AcpElicitationFieldKind::Unsupported { type_name } => {
            text(format!("Unsupported field type: {type_name}"))
                .text_xs()
                .text_color(move || theme().muted_foreground)
                .into_element()
        }
    };
    view()
        .gap_1()
        .child(
            text(title)
                .text_xs()
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(move || theme().foreground),
        )
        .when_some(description, |field, description| {
            field.child(
                text(description)
                    .text_xs()
                    .text_color(move || theme().muted_foreground),
            )
        })
        .child(control)
}

fn elicitation_option_button(
    option: AcpElicitationOption,
    field_name: String,
    mode: &'static str,
    values: Signal<BTreeMap<String, Value>>,
) -> impl IntoElement {
    let selected_value = option.value.clone();
    let selected_field = field_name.clone();
    let press_field = field_name;
    let press_value = option.value.clone();
    let border_field = selected_field.clone();
    let border_value = selected_value.clone();
    let background_field = selected_field;
    let background_value = selected_value;
    view()
        .px_2()
        .py_1p5()
        .rounded(move || theme().radius)
        .border_1()
        .border_color(move || {
            if elicitation_option_selected(&values.get(), &border_field, &border_value, mode) {
                theme().primary
            } else {
                theme().border
            }
        })
        .bg(move || {
            if elicitation_option_selected(
                &values.get(),
                &background_field,
                &background_value,
                mode,
            ) {
                theme().accent
            } else {
                theme().background
            }
        })
        .role(AccessibilityRole::Button)
        .on_press(move || {
            values.update(|values| {
                if mode == "multi" {
                    toggle_multi_value(values, &press_field, &press_value);
                } else {
                    values.insert(press_field.clone(), Value::String(press_value.clone()));
                }
            });
        })
        .child(
            text(option.title)
                .text_xs()
                .text_color(move || theme().foreground),
        )
}

fn elicitation_text(values: &BTreeMap<String, Value>, field: &str) -> String {
    match values.get(field) {
        Some(Value::String(value)) => value.clone(),
        Some(Value::Number(value)) => value.to_string(),
        Some(Value::Bool(value)) => value.to_string(),
        _ => String::new(),
    }
}

fn elicitation_bool(values: &BTreeMap<String, Value>, field: &str) -> bool {
    values.get(field).and_then(Value::as_bool).unwrap_or(false)
}

fn elicitation_option_selected(
    values: &BTreeMap<String, Value>,
    field: &str,
    option: &str,
    mode: &str,
) -> bool {
    match (mode, values.get(field)) {
        ("multi", Some(Value::Array(items))) => {
            items.iter().any(|item| item.as_str() == Some(option))
        }
        (_, Some(Value::String(value))) => value == option,
        _ => false,
    }
}

fn toggle_multi_value(values: &mut BTreeMap<String, Value>, field: &str, option: &str) {
    let entry = values
        .entry(field.to_owned())
        .or_insert_with(|| Value::Array(Vec::new()));
    let Value::Array(items) = entry else {
        *entry = Value::Array(vec![Value::String(option.to_owned())]);
        return;
    };
    if let Some(index) = items.iter().position(|item| item.as_str() == Some(option)) {
        items.remove(index);
    } else {
        items.push(Value::String(option.to_owned()));
    }
}

fn collect_elicitation_content(
    request: &AcpElicitationRequest,
    values: &BTreeMap<String, Value>,
) -> Result<BTreeMap<String, Value>, String> {
    let AcpElicitationMode::Form(form) = &request.mode else {
        return Ok(BTreeMap::new());
    };
    let mut content = BTreeMap::new();
    for field in &form.fields {
        let raw = values.get(&field.name).cloned();
        if field.required && elicitation_value_missing(raw.as_ref()) {
            return Err(format!("{} is required", field.title));
        }
        let Some(raw) = raw else {
            continue;
        };
        let value = match &field.kind {
            AcpElicitationFieldKind::Integer => {
                if let Some(number) = raw.as_i64() {
                    Value::Number(number.into())
                } else {
                    let text = raw.as_str().unwrap_or_default().trim();
                    if text.is_empty() {
                        continue;
                    }
                    Value::Number(
                        text.parse::<i64>()
                            .map_err(|_| format!("{} must be an integer", field.title))?
                            .into(),
                    )
                }
            }
            AcpElicitationFieldKind::Number => {
                if raw.is_number() {
                    raw
                } else {
                    let text = raw.as_str().unwrap_or_default().trim();
                    if text.is_empty() {
                        continue;
                    }
                    let number = text
                        .parse::<f64>()
                        .map_err(|_| format!("{} must be a number", field.title))?;
                    serde_json::Number::from_f64(number)
                        .map(Value::Number)
                        .ok_or_else(|| format!("{} must be a finite number", field.title))?
                }
            }
            AcpElicitationFieldKind::Unsupported { .. } => continue,
            _ => raw,
        };
        content.insert(field.name.clone(), value);
    }
    Ok(content)
}

fn elicitation_value_missing(value: Option<&Value>) -> bool {
    match value {
        None | Some(Value::Null) => true,
        Some(Value::String(value)) => value.trim().is_empty(),
        Some(Value::Array(values)) => values.is_empty(),
        _ => false,
    }
}

fn resolve_elicitation(
    prompt: &PendingElicitation,
    outcome: AcpElicitationOutcome,
    status: &Signal<String>,
    message: &str,
) {
    if let Some(response) = prompt.resolver.lock().ok().and_then(|mut slot| slot.take()) {
        let _ = response.send(outcome);
    }
    status.set(message.to_owned());
}

fn attach_connection(
    connection: AcpConnection,
    handle: tokio::runtime::Handle,
    session: Signal<Option<AcpSession>>,
    generation: Signal<u64>,
    transcript: Signal<String>,
    status: Signal<String>,
) {
    let mut events = connection.subscribe();
    let (event_tx, event_rx) = channel();
    handle.spawn(async move {
        loop {
            match events.recv().await {
                Ok(event) => {
                    if event_tx.send(event).is_err() {
                        break;
                    }
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            }
        }
    });
    generation.update(|generation| *generation += 1);
    let connection_generation = generation.get_untracked();
    session.set(Some(AcpSession(Arc::new(connection))));
    transcript.set(String::new());
    status.set("Connected".to_owned());
    install_event_pump(
        event_rx,
        generation,
        connection_generation,
        transcript,
        status,
    );
}

fn install_event_pump(
    events: Receiver<RuntimeEvent>,
    generation: Signal<u64>,
    connection_generation: u64,
    transcript: Signal<String>,
    status: Signal<String>,
) {
    set_interval(Duration::from_millis(24), move || {
        if generation.get_untracked() != connection_generation {
            return;
        }
        while let Ok(event) = events.try_recv() {
            apply_runtime_event(event, transcript, status);
        }
    });
}

fn apply_runtime_event(event: RuntimeEvent, transcript: Signal<String>, status: Signal<String>) {
    match event {
        RuntimeEvent::TurnStarted { .. } => status.set("Running".to_owned()),
        RuntimeEvent::AssistantMessageDelta { delta, .. } => {
            transcript.update(|transcript| transcript.push_str(&delta));
        }
        RuntimeEvent::AssistantMessage { text, .. } => {
            transcript.update(|transcript| append_if_missing(transcript, &text));
        }
        RuntimeEvent::ReasoningDelta { delta, .. } => {
            status.set(format!("Thinking: {}", compact_status(&delta)));
        }
        RuntimeEvent::ToolCallStarted { tool_name, .. } => {
            transcript.update(|transcript| {
                if !transcript.ends_with('\n') {
                    transcript.push('\n');
                }
                transcript.push_str(&format!("\n[tool] {tool_name:?}\n"));
            });
        }
        RuntimeEvent::NeedUserInput { question, .. } => {
            status.set(format!(
                "Tool approval/input required: {}",
                compact_status(&question)
            ));
        }
        RuntimeEvent::TurnCompleted { answer, .. } => {
            if let Some(answer) = answer {
                transcript.update(|transcript| append_if_missing(transcript, &answer));
            }
            status.set("Ready".to_owned());
        }
        RuntimeEvent::TurnCancelled { .. } => status.set("Turn cancelled".to_owned()),
        RuntimeEvent::TurnFailed { reason, .. } => {
            transcript.update(|transcript| {
                transcript.push_str("\n\nError: ");
                transcript.push_str(&reason);
            });
            status.set("Turn failed".to_owned());
        }
        _ => {}
    }
}

fn append_if_missing(buffer: &mut String, text: &str) {
    if text.is_empty() || buffer.ends_with(text) {
        return;
    }
    if !buffer.is_empty() && !buffer.chars().last().is_some_and(char::is_whitespace) {
        buffer.push_str("\n\n");
    }
    buffer.push_str(text);
}

fn compact_status(text: &str) -> String {
    let text = text.trim().replace('\n', " ");
    if text.chars().count() <= 120 {
        text
    } else {
        format!("{}…", text.chars().take(120).collect::<String>())
    }
}

fn build_prompt_blocks(text: &str, image_path: &str) -> Result<Vec<ContentBlock>, String> {
    let mut blocks = vec![ContentBlock::Text(TextContent::new(text.to_owned()))];
    let image_path = image_path.trim();
    if image_path.is_empty() {
        return Ok(blocks);
    }
    let path = std::path::Path::new(image_path);
    let mime_type = match path
        .extension()
        .and_then(|extension| extension.to_str())
        .map(str::to_ascii_lowercase)
        .as_deref()
    {
        Some("png") => "image/png",
        Some("jpg" | "jpeg") => "image/jpeg",
        Some("webp") => "image/webp",
        Some("gif") => "image/gif",
        _ => return Err("Image must be a PNG, JPEG, WebP or GIF file".to_owned()),
    };
    let metadata =
        std::fs::metadata(path).map_err(|error| format!("Failed to read image: {error}"))?;
    if metadata.len() > 20 * 1024 * 1024 {
        return Err("Image is larger than the 20 MiB attachment limit".to_owned());
    }
    let bytes = std::fs::read(path).map_err(|error| format!("Failed to read image: {error}"))?;
    blocks.push(ContentBlock::Image(ImageContent::new(
        STANDARD.encode(bytes),
        mime_type.to_owned(),
    )));
    Ok(blocks)
}

fn label(value: &'static str) -> impl IntoElement {
    text(value)
        .text_size(px(10.0))
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(move || theme().muted_foreground)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn final_assistant_text_is_not_duplicated_after_streaming() {
        let mut transcript = String::from("Assistant: hello");
        append_if_missing(&mut transcript, "hello");
        assert_eq!(transcript, "Assistant: hello");
    }

    #[test]
    fn compact_status_is_bounded() {
        assert!(compact_status(&"x".repeat(200)).ends_with('…'));
        assert_eq!(compact_status("ok\nnext"), "ok next");
    }

    #[test]
    fn permission_prefers_an_allow_option() {
        let prompt = PendingPermission {
            request_id: "request".to_owned(),
            tool_name: "shell".to_owned(),
            summary: "Run a command".to_owned(),
            options: vec![
                AcpPermissionOption {
                    option_id: "deny".to_owned(),
                    name: "Deny".to_owned(),
                    kind: "reject_once".to_owned(),
                },
                AcpPermissionOption {
                    option_id: "allow".to_owned(),
                    name: "Allow".to_owned(),
                    kind: "allow_once".to_owned(),
                },
            ],
            resolver: Arc::new(Mutex::new(None)),
        };
        assert_eq!(prompt.allow_option_id().as_deref(), Some("allow"));
    }

    #[test]
    fn elicitation_form_validates_required_fields_and_converts_numbers() {
        let request = AcpElicitationRequest {
            request_id: "request".to_owned(),
            session_id: "session".to_owned(),
            message: "Configure deployment".to_owned(),
            mode: AcpElicitationMode::Form(AcpElicitationForm {
                title: Some("Deploy".to_owned()),
                description: None,
                fields: vec![
                    AcpElicitationField {
                        name: "environment".to_owned(),
                        title: "Environment".to_owned(),
                        description: None,
                        required: true,
                        default: None,
                        kind: AcpElicitationFieldKind::Text { format: None },
                    },
                    AcpElicitationField {
                        name: "replicas".to_owned(),
                        title: "Replicas".to_owned(),
                        description: None,
                        required: true,
                        default: None,
                        kind: AcpElicitationFieldKind::Integer,
                    },
                ],
            }),
        };
        let mut values = BTreeMap::from([
            ("environment".to_owned(), Value::String("prod".to_owned())),
            ("replicas".to_owned(), Value::String("3".to_owned())),
        ]);
        assert_eq!(
            collect_elicitation_content(&request, &values)
                .unwrap()
                .get("replicas")
                .and_then(Value::as_i64),
            Some(3)
        );
        values.insert("replicas".to_owned(), Value::String("many".to_owned()));
        assert!(
            collect_elicitation_content(&request, &values)
                .unwrap_err()
                .contains("integer")
        );
        values.remove("environment");
        assert!(
            collect_elicitation_content(&request, &values)
                .unwrap_err()
                .contains("Environment")
        );
    }

    #[test]
    fn multi_select_values_toggle_cleanly() {
        let mut values = BTreeMap::new();
        toggle_multi_value(&mut values, "regions", "eu");
        toggle_multi_value(&mut values, "regions", "us");
        assert!(elicitation_option_selected(
            &values, "regions", "eu", "multi"
        ));
        toggle_multi_value(&mut values, "regions", "eu");
        assert!(!elicitation_option_selected(
            &values, "regions", "eu", "multi"
        ));
        assert!(elicitation_option_selected(
            &values, "regions", "us", "multi"
        ));
    }

    #[test]
    fn prompt_blocks_only_attach_supported_image_files() {
        let blocks = build_prompt_blocks("describe this", "").unwrap();
        assert_eq!(blocks.len(), 1);
        assert!(
            build_prompt_blocks("describe this", "/tmp/not-an-image.txt")
                .unwrap_err()
                .contains("PNG")
        );
    }
}
