use std::{
    cell::RefCell,
    rc::Rc,
    sync::{
        Arc,
        mpsc::{Receiver, Sender, channel},
    },
    time::Duration,
};

use nsh_base::{Terminal, TerminalApi, TerminalEvent, terminal as terminal_element, theme};
use nsh_core::prelude::*;
use one_core::storage::{ConnectionType, StoredConnection};
use ssh::{ChannelEvent, SshChannel, SshSessionManager};
use terminal::{LocalConfig, SshBackend};

use crate::{
    pty::{PtyCommand, PtyMessage},
    services::NavopNshServices,
    ui,
};

#[derive(Clone)]
struct TerminalChoice {
    id: Option<i64>,
    label: String,
    connection: Option<StoredConnection>,
    native: bool,
}

pub(crate) fn terminal() -> impl IntoElement {
    let services = NavopNshServices::current();
    let local_config = services
        .as_ref()
        .and_then(|services| services.local_terminal.clone());
    let resolved_connections = services
        .as_ref()
        .map(|services| services.resolved_connections.clone())
        .unwrap_or_default();
    let tokio = services.as_ref().map(|services| services.tokio.clone());
    let choices = terminal_choices(local_config.as_ref(), &resolved_connections);
    let requested_connection = crate::route::take_connection_request(crate::route::Route::Terminal);
    let initial_choice = requested_connection
        .and_then(|id| choices.iter().find(|choice| choice.id == Some(id)).cloned())
        .or_else(|| choices.first().cloned());
    let selected = signal(initial_choice.as_ref().and_then(|choice| choice.id));
    let active_name = signal(initial_choice.as_ref().map_or_else(
        || "No terminal target".to_owned(),
        |choice| choice.label.clone(),
    ));
    let native_target = signal(
        initial_choice
            .as_ref()
            .and_then(|choice| choice.native.then_some(choice.id).flatten()),
    );
    let status = signal(String::from("Starting terminal…"));
    let terminal_ref = component_ref::<Terminal>();
    let (output_tx, output_rx) = channel::<PtyMessage>();
    let current_commands = Rc::new(RefCell::new(None::<Sender<PtyCommand>>));

    install_output_pump(terminal_ref, output_rx, status);

    let start = {
        let current_commands = current_commands.clone();
        let output_tx = output_tx.clone();
        let local_config = local_config.clone();
        let tokio = tokio.clone();
        move |choice: TerminalChoice| {
            if let Some(commands) = current_commands.borrow_mut().take() {
                let _ = commands.send(PtyCommand::Shutdown);
            }
            if let Some(api) = terminal_ref.api::<TerminalApi>() {
                api.reset();
            }
            status.set(format!("Starting {}…", choice.label));
            if choice.native {
                native_target.set(choice.id);
                status.set(format!("Attached to {}", choice.label));
                return;
            }
            native_target.set(None);
            let result = if let Some(connection) = choice.connection {
                let Some(tokio) = tokio.clone() else {
                    status.set("Tokio runtime is unavailable".to_owned());
                    return;
                };
                spawn_ssh_worker(connection, output_tx.clone(), tokio, status)
            } else if let Some(config) = local_config.clone() {
                Ok(start_local_worker(config, output_tx.clone()))
            } else {
                Err("Local terminal profile is unavailable".to_owned())
            };
            match result {
                Ok(commands) => *current_commands.borrow_mut() = Some(commands),
                Err(error) => status.set(error),
            }
        }
    };

    if let Some(initial) = initial_choice {
        start(initial);
    } else {
        status.set("No local terminal or SSH connection is available".to_owned());
    }

    cleanup({
        let current_commands = current_commands.clone();
        move || {
            if let Some(commands) = current_commands.borrow_mut().take() {
                let _ = commands.send(PtyCommand::Shutdown);
            }
        }
    });

    view()
        .size_full()
        .child(ui::page_header(
            "Terminal",
            "nsh-base terminal emulator with local PTY and SSH byte-stream backends",
        ))
        .child(
            view()
                .flex_1()
                .min_h_0()
                .p_3()
                .gap_2()
                .child(
                    view()
                        .flex_row()
                        .items_center()
                        .flex_wrap()
                        .gap_2()
                        .child(
                            text("SESSION")
                                .text_size(px(10.0))
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(move || theme().muted_foreground),
                        )
                        .children(choices.into_iter().map({
                            let start = start.clone();
                            move |choice| {
                                terminal_choice_button(choice, selected, active_name, start.clone())
                            }
                        }))
                        .child(
                            text(move || status.get())
                                .flex_1()
                                .text_xs()
                                .text_color(move || theme().muted_foreground),
                        ),
                )
                .child(
                    text(move || format!("Attached to {}", active_name.get()))
                        .text_xs()
                        .text_color(move || theme().muted_foreground),
                )
                .when_some(native_target.get(), |content, id| {
                    content.child(
                        native("navop.terminal")
                            .prop("connection_id", serde_json::Value::from(id))
                            .flex_1()
                            .min_h_0(),
                    )
                })
                .when(native_target.get().is_none(), |content| {
                    content.child(
                        terminal_element()
                            .component_ref(&terminal_ref)
                            .scrollback(5000)
                            .font_family("monospace")
                            .font_size(px(13.0))
                            .line_height(px(19.0))
                            .accessibility_label("Terminal")
                            .flex_1()
                            .min_h_0()
                            .p_2()
                            .rounded(move || theme().radius)
                            .bg(move || theme().background)
                            .on_data({
                                let current_commands = current_commands.clone();
                                move |data: &SharedString| {
                                    if let Some(commands) = current_commands.borrow().as_ref() {
                                        let _ = commands
                                            .send(PtyCommand::Write(data.as_bytes().to_vec()));
                                    }
                                }
                            })
                            .on_resize({
                                let current_commands = current_commands.clone();
                                move |event: &TerminalEvent| {
                                    if let TerminalEvent::Resize(cols, rows) = event
                                        && let Some(commands) = current_commands.borrow().as_ref()
                                    {
                                        let _ = commands.send(PtyCommand::Resize {
                                            cols: (*cols).min(u16::MAX as u32) as u16,
                                            rows: (*rows).min(u16::MAX as u32) as u16,
                                        });
                                    }
                                }
                            }),
                    )
                }),
        )
}

fn terminal_choices(
    local: Option<&LocalConfig>,
    resolved_connections: &[StoredConnection],
) -> Vec<TerminalChoice> {
    let mut choices = Vec::new();
    if local.is_some() {
        choices.push(TerminalChoice {
            id: None,
            label: "Local shell".to_owned(),
            connection: None,
            native: false,
        });
    }
    choices.extend(
        resolved_connections
            .iter()
            .filter(|connection| connection.connection_type == ConnectionType::SshSftp)
            .filter(|connection| connection.id.is_some())
            .cloned()
            .map(|connection| TerminalChoice {
                id: connection.id,
                label: connection.name.clone(),
                connection: Some(connection),
                native: false,
            }),
    );
    choices.extend(
        resolved_connections
            .iter()
            .filter(|connection| {
                matches!(
                    connection.connection_type,
                    ConnectionType::Serial | ConnectionType::Telnet
                )
            })
            .filter(|connection| connection.id.is_some())
            .cloned()
            .map(|connection| TerminalChoice {
                id: connection.id,
                label: connection.name.clone(),
                connection: Some(connection),
                native: true,
            }),
    );
    choices
}

fn terminal_choice_button(
    choice: TerminalChoice,
    selected: Signal<Option<i64>>,
    active_name: Signal<String>,
    start: impl Fn(TerminalChoice) + Clone + 'static,
) -> impl IntoElement {
    let id = choice.id;
    let label = choice.label.clone();
    let press_choice = choice.clone();
    view()
        .px_3()
        .py_1p5()
        .rounded(move || theme().radius)
        .border_1()
        .border_color(move || {
            if selected.get() == id {
                theme().primary
            } else {
                theme().border
            }
        })
        .bg(move || {
            if selected.get() == id {
                theme().accent
            } else {
                theme().background
            }
        })
        .role(AccessibilityRole::Button)
        .label(label.clone())
        .accessibility_state(move || AccessibilityState {
            selected: selected.get() == id,
            ..AccessibilityState::default()
        })
        .on_press(move || {
            selected.set(id);
            active_name.set(press_choice.label.clone());
            start(press_choice.clone());
        })
        .child(text(label).text_xs().text_color(move || theme().foreground))
}

fn start_local_worker(config: LocalConfig, output: Sender<PtyMessage>) -> Sender<PtyCommand> {
    let (command_tx, command_rx) = channel();
    crate::pty::spawn(config, command_rx, output);
    command_tx
}

fn spawn_ssh_worker(
    connection: StoredConnection,
    output: Sender<PtyMessage>,
    tokio: tokio::runtime::Handle,
    status: Signal<String>,
) -> Result<Sender<PtyCommand>, String> {
    let params = connection
        .to_ssh_params()
        .map_err(|error| format!("Invalid SSH connection: {error}"))?;
    if params.prompts_for_username()
        || (params.prompts_for_password() && params.auth_method.contains_password())
    {
        return Err(
            "This SSH connection requires interactive credentials; unlock them before connecting"
                .to_owned(),
        );
    }
    let config = terminal::terminal::ssh_terminal_config_from_connection(&connection)
        .map_err(|error| format!("Failed to prepare SSH connection: {error:#}"))?;
    let connection_id = connection.id;
    let (command_tx, command_rx) = channel();
    status.set("Connecting to SSH host…".to_owned());
    tokio.spawn(async move {
        let manager = Arc::new(SshSessionManager::new(config.ssh_config));
        let established = SshBackend::establish_channel_for_test(
            &manager,
            &config.pty_config,
            connection_id,
            true,
        )
        .await;
        let (_client, mut channel, _) = match established {
            Ok(value) => value,
            Err(error) => {
                let _ = output.send(PtyMessage::Status(format!(
                    "SSH connection failed: {error:#}"
                )));
                let _ = manager.disconnect().await;
                return;
            }
        };
        let _ = output.send(PtyMessage::Status("SSH connected".to_owned()));
        run_ssh_channel(&mut channel, command_rx, output.clone()).await;
        let _ = channel.eof().await;
        let _ = channel.close().await;
        let _ = manager.disconnect().await;
    });
    Ok(command_tx)
}

async fn run_ssh_channel<C: SshChannel + ?Sized>(
    channel: &mut C,
    command_rx: Receiver<PtyCommand>,
    output: Sender<PtyMessage>,
) {
    loop {
        match tokio::time::timeout(Duration::from_millis(10), channel.recv()).await {
            Ok(Some(ChannelEvent::Data(data)))
            | Ok(Some(ChannelEvent::ExtendedData { data, .. })) => {
                if output.send(PtyMessage::Data(data)).is_err() {
                    break;
                }
            }
            Ok(Some(ChannelEvent::Eof)) | Ok(Some(ChannelEvent::Close)) | Ok(None) => {
                let _ = output.send(PtyMessage::Status("SSH disconnected".to_owned()));
                break;
            }
            Ok(Some(ChannelEvent::ExitStatus(code))) => {
                let _ = output.send(PtyMessage::Status(format!("SSH shell exited ({code})")));
                break;
            }
            Ok(Some(ChannelEvent::ExitSignal {
                signal_name,
                error_message,
            })) => {
                let _ = output.send(PtyMessage::Status(format!(
                    "SSH shell terminated ({signal_name}: {error_message})"
                )));
                break;
            }
            Err(_) => {}
        }

        let mut shutdown = false;
        while let Ok(command) = command_rx.try_recv() {
            match command {
                PtyCommand::Write(data) => {
                    if let Err(error) = channel.send_data(&data).await {
                        let _ =
                            output.send(PtyMessage::Status(format!("SSH write failed: {error:#}")));
                        shutdown = true;
                        break;
                    }
                }
                PtyCommand::Resize { cols, rows } => {
                    if let Err(error) = channel.resize_pty(u32::from(cols), u32::from(rows)).await {
                        let _ = output
                            .send(PtyMessage::Status(format!("SSH resize failed: {error:#}")));
                    }
                }
                PtyCommand::Shutdown => {
                    shutdown = true;
                    break;
                }
            }
        }
        if shutdown {
            break;
        }
    }
}

fn install_output_pump(
    terminal_ref: ComponentRef<Terminal>,
    output: Receiver<PtyMessage>,
    status: Signal<String>,
) {
    let pending = Rc::new(RefCell::new(Vec::<u8>::new()));
    set_interval(Duration::from_millis(16), move || {
        while let Ok(message) = output.try_recv() {
            match message {
                PtyMessage::Data(bytes) => {
                    let mut pending = pending.borrow_mut();
                    pending.extend(bytes);
                    flush_utf8(&mut pending, terminal_ref);
                }
                PtyMessage::Status(message) => status.set(message),
            }
        }
    });
}

fn flush_utf8(pending: &mut Vec<u8>, terminal_ref: ComponentRef<Terminal>) {
    match std::str::from_utf8(pending) {
        Ok(text) => {
            write_terminal(terminal_ref, text);
            pending.clear();
        }
        Err(error) if error.error_len().is_some() => {
            let text = String::from_utf8_lossy(pending);
            write_terminal(terminal_ref, &text);
            pending.clear();
        }
        Err(error) => {
            let valid = error.valid_up_to();
            if valid > 0 {
                let text = String::from_utf8_lossy(&pending[..valid]);
                write_terminal(terminal_ref, &text);
                pending.drain(..valid);
            }
        }
    }
}

fn write_terminal(terminal_ref: ComponentRef<Terminal>, text: &str) {
    if !text.is_empty()
        && let Some(api) = terminal_ref.api::<TerminalApi>()
    {
        api.write(text.to_owned());
    }
}

#[cfg(test)]
mod tests {
    use std::{collections::VecDeque, sync::Mutex};

    use super::*;

    #[test]
    fn utf8_flush_keeps_incomplete_multibyte_tail() {
        let mut pending = "terminal 中".as_bytes().to_vec();
        let split = pending.len() - 1;
        let tail = pending.split_off(split);
        assert!(tail.len() < 3);
        assert!(std::str::from_utf8(&pending).is_err());
        assert!(std::str::from_utf8(&tail).is_err());
    }

    #[test]
    fn terminal_choices_list_local_then_saved_ssh_hosts() {
        let local = LocalConfig {
            shell: Some("/bin/sh".to_owned()),
            args: Vec::new(),
            working_dir: None,
            env: Vec::new(),
        };
        let mut ssh = StoredConnection::new_ssh("Server".to_owned(), default_ssh_params(), None);
        ssh.id = Some(7);
        let choices = terminal_choices(Some(&local), &[ssh]);
        assert_eq!(choices.len(), 2);
        assert_eq!(choices[0].label, "Local shell");
        assert_eq!(choices[1].id, Some(7));
    }

    #[test]
    fn serial_and_telnet_choices_use_the_advanced_terminal_backend() {
        let mut serial = StoredConnection::new_serial(
            "Console".to_owned(),
            one_core::storage::SerialParams::default(),
            None,
        );
        serial.id = Some(8);
        let mut telnet = StoredConnection::new_telnet(
            "Switch".to_owned(),
            one_core::storage::TelnetParams::default(),
            None,
        );
        telnet.id = Some(9);
        let choices = terminal_choices(None, &[serial, telnet]);
        assert_eq!(choices.len(), 2);
        assert!(choices.iter().all(|choice| choice.native));
    }

    #[derive(Clone, Default)]
    struct FakeSshChannel {
        events: Arc<Mutex<VecDeque<ChannelEvent>>>,
        writes: Arc<Mutex<Vec<Vec<u8>>>>,
        resizes: Arc<Mutex<Vec<(u32, u32)>>>,
    }

    #[async_trait::async_trait]
    impl SshChannel for FakeSshChannel {
        async fn request_pty(&mut self, _config: &ssh::PtyConfig) -> anyhow::Result<()> {
            Ok(())
        }

        async fn exec(&mut self, _command: &str) -> anyhow::Result<()> {
            Ok(())
        }

        async fn request_shell(&mut self) -> anyhow::Result<()> {
            Ok(())
        }

        async fn set_env(&mut self, _name: &str, _value: &str) -> anyhow::Result<()> {
            Ok(())
        }

        async fn send_data(&mut self, data: &[u8]) -> anyhow::Result<()> {
            self.writes.lock().unwrap().push(data.to_vec());
            Ok(())
        }

        async fn resize_pty(&mut self, width: u32, height: u32) -> anyhow::Result<()> {
            self.resizes.lock().unwrap().push((width, height));
            Ok(())
        }

        async fn recv(&mut self) -> Option<ChannelEvent> {
            self.events.lock().unwrap().pop_front()
        }

        async fn eof(&mut self) -> anyhow::Result<()> {
            Ok(())
        }

        async fn close(&mut self) -> anyhow::Result<()> {
            Ok(())
        }
    }

    #[tokio::test]
    async fn ssh_channel_pump_forwards_output_input_resize_and_close() {
        let mut fake = FakeSshChannel::default();
        fake.events
            .lock()
            .unwrap()
            .push_back(ChannelEvent::Data(b"hello".to_vec()));
        fake.events.lock().unwrap().push_back(ChannelEvent::Close);
        let (command_tx, command_rx) = channel::<PtyCommand>();
        command_tx
            .send(PtyCommand::Write(b"ls\r".to_vec()))
            .unwrap();
        command_tx
            .send(PtyCommand::Resize {
                cols: 120,
                rows: 40,
            })
            .unwrap();
        let (output_tx, output_rx) = channel::<PtyMessage>();

        run_ssh_channel(&mut fake, command_rx, output_tx).await;

        assert_eq!(*fake.writes.lock().unwrap(), vec![b"ls\r".to_vec()]);
        assert_eq!(*fake.resizes.lock().unwrap(), vec![(120, 40)]);
        let messages = output_rx.try_iter().collect::<Vec<_>>();
        assert!(matches!(
            messages.first(),
            Some(PtyMessage::Data(data)) if data == b"hello"
        ));
        assert!(matches!(
            messages.last(),
            Some(PtyMessage::Status(status)) if status == "SSH disconnected"
        ));
    }

    fn default_ssh_params() -> one_core::storage::SshParams {
        one_core::storage::SshParams {
            host: "example.com".to_owned(),
            port: 22,
            username: "root".to_owned(),
            auth_method: one_core::storage::SshAuthMethod::Agent,
            sftp_account: None,
            sftp_default_directory: None,
            credential_reference: None,
            prompt_username: None,
            prompt_password: None,
            keyboard_interactive: None,
            otp: None,
            terminal_encoding: Default::default(),
            terminal_type: Default::default(),
            account_expect: Default::default(),
            connect_timeout: None,
            keepalive_interval: None,
            keepalive_max: None,
            default_directory: None,
            init_script: None,
            disable_shell_integration: None,
            x11_forwarding: None,
            allow_legacy_algorithms: None,
            forward_agent: None,
            agent_forward_key: None,
            jump_server: None,
            disabled_jump_server: None,
            proxy: None,
            os_id: None,
            icon: None,
            icon_file_path: None,
            remote_file: None,
        }
    }
}
