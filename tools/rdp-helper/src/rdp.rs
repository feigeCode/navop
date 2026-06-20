use std::sync::mpsc as std_mpsc;

use anyhow::Context as _;
use ironrdp::connector::{self, Credentials};
use ironrdp::input::{Database, MouseButton, MousePosition, Operation, Scancode, WheelRotations};
use ironrdp::pdu::rdp::capability_sets::{MajorPlatformType, client_codecs_capabilities};
use ironrdp::pdu::rdp::client_info::{CompressionType, PerformanceFlags, TimezoneInfo};
use ironrdp_client::config::{ClipboardType, Config, Destination};
use ironrdp_client::rdp::{DvcPipeProxyFactory, RdpClient, RdpInputEvent, RdpOutputEvent};
use smallvec::SmallVec;
use tokio::sync::mpsc;

use crate::clipboard::{TextClipboardController, text_clipboard};
use crate::pixels::rdp_u32_pixels_to_rgba;
use crate::protocol::{ConnectRequest, HelperEvent, HelperMouseButton, HelperRequest};

pub struct RdpRuntime {
    pub input_tx: mpsc::UnboundedSender<RdpInputEvent>,
    pub output_rx: std_mpsc::Receiver<HelperEvent>,
    pub clipboard: TextClipboardController,
}

pub fn start(connect: ConnectRequest) -> anyhow::Result<RdpRuntime> {
    let config = build_config(connect)?;
    let (input_tx, input_rx) = RdpInputEvent::create_channel();
    let (output_tx, output_rx) = mpsc::channel::<RdpOutputEvent>(64);
    let (helper_output_tx, helper_output_rx) = std_mpsc::channel::<HelperEvent>();
    let (clipboard, cliprdr_factory) = text_clipboard(input_tx.clone(), helper_output_tx.clone());
    let dvc_pipe_proxy_factory = DvcPipeProxyFactory::new(input_tx.clone());
    let client = RdpClient {
        config,
        output_event_sender: output_tx,
        input_event_receiver: input_rx,
        cliprdr_factory: Some(cliprdr_factory),
        dvc_pipe_proxy_factory,
    };

    spawn_client_thread(client, output_rx, helper_output_tx)?;
    Ok(RdpRuntime {
        input_tx,
        output_rx: helper_output_rx,
        clipboard,
    })
}

pub fn apply_input_request(
    request: HelperRequest,
    input_tx: &mpsc::UnboundedSender<RdpInputEvent>,
    input_database: &mut Database,
    clipboard: &TextClipboardController,
) -> anyhow::Result<bool> {
    match request {
        HelperRequest::Resize { width, height } => input_tx
            .send(RdpInputEvent::Resize {
                width,
                height,
                scale_factor: 100,
                physical_size: None,
            })
            .map_err(|_| anyhow::anyhow!("RDP input channel closed"))?,
        HelperRequest::MouseMove { x, y } => {
            send_operations(
                input_database,
                input_tx,
                [Operation::MouseMove(MousePosition { x, y })],
            );
        }
        HelperRequest::MouseButton { button, pressed } => {
            send_operations(
                input_database,
                input_tx,
                [mouse_button_operation(button, pressed)],
            );
        }
        HelperRequest::Wheel { vertical, units } => {
            send_operations(
                input_database,
                input_tx,
                [Operation::WheelRotations(WheelRotations {
                    is_vertical: vertical,
                    rotation_units: units,
                })],
            );
        }
        HelperRequest::Key {
            code,
            extended,
            pressed,
        } => send_operations(
            input_database,
            input_tx,
            [key_operation(code, extended, pressed)?],
        ),
        HelperRequest::Text { text } => send_text(input_database, input_tx, &text),
        HelperRequest::ClipboardText { text } => clipboard.set_local_text(text)?,
        HelperRequest::Close => {
            input_tx
                .send(RdpInputEvent::Close)
                .map_err(|_| anyhow::anyhow!("RDP input channel closed"))?;
            return Ok(false);
        }
        HelperRequest::Connect { .. } => {
            anyhow::bail!("Connect request is only valid as the first message")
        }
    }
    Ok(true)
}

fn spawn_client_thread(
    client: RdpClient,
    mut output_rx: mpsc::Receiver<RdpOutputEvent>,
    helper_output_tx: std_mpsc::Sender<HelperEvent>,
) -> anyhow::Result<()> {
    std::thread::Builder::new()
        .name("onetcli-rdp-helper-runtime".to_string())
        .spawn(move || {
            let runtime = tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .build();
            let Ok(runtime) = runtime else {
                let _ = helper_output_tx.send(HelperEvent::ConnectionFailure {
                    message: "failed to create RDP tokio runtime".to_string(),
                });
                return;
            };

            let output_sender = helper_output_tx.clone();
            runtime.spawn(async move {
                while let Some(event) = output_rx.recv().await {
                    if output_sender.send(output_event_to_helper(event)).is_err() {
                        break;
                    }
                }
            });
            runtime.block_on(client.run());
        })
        .context("spawn RDP client thread")?;
    Ok(())
}

fn build_config(connect: ConnectRequest) -> anyhow::Result<Config> {
    let codecs = client_codecs_capabilities(&[])
        .map_err(|help| anyhow::anyhow!("failed to build bitmap codec capabilities: {help}"))?;
    let connector = connector::Config {
        credentials: Credentials::UsernamePassword {
            username: connect.username.unwrap_or_default(),
            password: connect.password.unwrap_or_default(),
        },
        domain: connect.domain,
        enable_tls: true,
        enable_credssp: true,
        desktop_size: connector::DesktopSize {
            width: connect.width,
            height: connect.height,
        },
        desktop_scale_factor: 0,
        keyboard_type: ironrdp::pdu::gcc::KeyboardType::IbmEnhanced,
        keyboard_subtype: 0,
        keyboard_layout: 0,
        keyboard_functional_keys_count: 12,
        ime_file_name: String::new(),
        bitmap: Some(connector::BitmapConfig {
            lossy_compression: true,
            color_depth: 32,
            codecs,
        }),
        dig_product_id: String::new(),
        client_build: client_build()?,
        client_name: whoami::fallible::hostname().unwrap_or_else(|_| "onetcli-rdp".to_string()),
        client_dir: "C:\\Windows\\System32\\mstscax.dll".to_string(),
        alternate_shell: String::new(),
        work_dir: String::new(),
        platform: platform_type(),
        hardware_id: None,
        license_cache: None,
        request_data: None,
        autologon: true,
        enable_audio_playback: false,
        enable_server_pointer: true,
        pointer_software_rendering: false,
        multitransport_flags: None,
        compression_type: Some(CompressionType::Rdp61),
        performance_flags: PerformanceFlags::default(),
        timezone_info: TimezoneInfo::default(),
    };

    Ok(Config {
        log_file: None,
        gw: None,
        kerberos_config: None,
        destination: connect.destination.parse::<Destination>()?,
        connector,
        clipboard_type: ClipboardType::Enable,
        rdcleanpath: None,
        fake_events_interval: None,
        dvc_pipe_proxies: Vec::new(),
        #[cfg(windows)]
        dvc_plugins: Vec::new(),
    })
}

fn output_event_to_helper(event: RdpOutputEvent) -> HelperEvent {
    match event {
        RdpOutputEvent::Image {
            buffer,
            width,
            height,
        } => HelperEvent::frame(width.get(), height.get(), rdp_u32_pixels_to_rgba(&buffer)),
        RdpOutputEvent::ConnectionFailure(error) => HelperEvent::ConnectionFailure {
            message: format!("{error:#}"),
        },
        RdpOutputEvent::Terminated(result) => HelperEvent::Terminated {
            message: match result {
                Ok(reason) => reason.to_string(),
                Err(error) => format!("{error:#}"),
            },
        },
        RdpOutputEvent::PointerDefault => HelperEvent::CursorDefault,
        RdpOutputEvent::PointerHidden => HelperEvent::CursorHidden,
        RdpOutputEvent::PointerPosition { x, y } => HelperEvent::CursorPosition { x, y },
        RdpOutputEvent::PointerBitmap(_) => HelperEvent::CursorDefault,
    }
}

fn send_operations<const N: usize>(
    input_database: &mut Database,
    input_tx: &mpsc::UnboundedSender<RdpInputEvent>,
    operations: [Operation; N],
) {
    let events = input_database.apply(operations);
    send_fast_path(input_tx, events);
}

fn send_text(
    input_database: &mut Database,
    input_tx: &mpsc::UnboundedSender<RdpInputEvent>,
    text: &str,
) {
    for character in text.chars() {
        send_operations(
            input_database,
            input_tx,
            [
                Operation::UnicodeKeyPressed(character),
                Operation::UnicodeKeyReleased(character),
            ],
        );
    }
}

fn send_fast_path(
    input_tx: &mpsc::UnboundedSender<RdpInputEvent>,
    events: SmallVec<[ironrdp::pdu::input::fast_path::FastPathInputEvent; 2]>,
) {
    if !events.is_empty() {
        let _ = input_tx.send(RdpInputEvent::FastPath(events));
    }
}

fn mouse_button_operation(button: HelperMouseButton, pressed: bool) -> Operation {
    let button = match button {
        HelperMouseButton::Left => MouseButton::Left,
        HelperMouseButton::Middle => MouseButton::Middle,
        HelperMouseButton::Right => MouseButton::Right,
        HelperMouseButton::X1 => MouseButton::X1,
        HelperMouseButton::X2 => MouseButton::X2,
    };
    if pressed {
        Operation::MouseButtonPressed(button)
    } else {
        Operation::MouseButtonReleased(button)
    }
}

fn key_operation(code: u16, extended: bool, pressed: bool) -> anyhow::Result<Operation> {
    let code = u8::try_from(code).context("RDP scancode must fit in u8")?;
    let scancode = Scancode::from_u8(extended, code);
    Ok(if pressed {
        Operation::KeyPressed(scancode)
    } else {
        Operation::KeyReleased(scancode)
    })
}

fn client_build() -> anyhow::Result<u32> {
    let version = semver::Version::parse(env!("CARGO_PKG_VERSION"))?;
    Ok((version.major * 100 + version.minor * 10 + version.patch).try_into()?)
}

fn platform_type() -> MajorPlatformType {
    match whoami::platform() {
        whoami::Platform::Windows => MajorPlatformType::WINDOWS,
        whoami::Platform::Linux => MajorPlatformType::UNIX,
        whoami::Platform::MacOS => MajorPlatformType::MACINTOSH,
        whoami::Platform::Ios => MajorPlatformType::IOS,
        whoami::Platform::Android => MajorPlatformType::ANDROID,
        _ => MajorPlatformType::UNSPECIFIED,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn key_operation_builds_plain_scancode_events() {
        let operation = key_operation(0x39, false, true).expect("space key operation");

        assert_key_operation(operation, true, false, 0x39);
    }

    #[test]
    fn key_operation_builds_extended_scancode_events() {
        let operation = key_operation(0x48, true, false).expect("arrow key operation");

        assert_key_operation(operation, false, true, 0x48);
    }

    #[test]
    fn key_operation_rejects_out_of_range_scancode() {
        assert!(key_operation(0x100, false, true).is_err());
    }

    #[test]
    fn apply_clipboard_text_request_advertises_local_text() {
        let (input_tx, mut input_rx) = RdpInputEvent::create_channel();
        let (output_tx, _output_rx) = std_mpsc::channel::<HelperEvent>();
        let (clipboard, _factory) = text_clipboard(input_tx.clone(), output_tx);
        let mut input_database = Database::new();

        let keep_running = apply_input_request(
            HelperRequest::ClipboardText {
                text: "local 中文".to_string(),
            },
            &input_tx,
            &mut input_database,
            &clipboard,
        )
        .expect("clipboard request applies");

        assert!(keep_running);
        match input_rx.try_recv().expect("clipboard advertise") {
            RdpInputEvent::Clipboard(
                ironrdp::cliprdr::backend::ClipboardMessage::SendInitiateCopy(formats),
            ) => {
                assert!(formats.iter().any(|format| format.id()
                    == ironrdp::cliprdr::pdu::ClipboardFormatId::CF_UNICODETEXT))
            }
            other => panic!("expected clipboard advertise, got {other:?}"),
        }
    }

    fn assert_key_operation(
        operation: Operation,
        expected_pressed: bool,
        expected_extended: bool,
        expected_code: u8,
    ) {
        let (pressed, scancode) = match operation {
            Operation::KeyPressed(scancode) => (true, scancode),
            Operation::KeyReleased(scancode) => (false, scancode),
            other => panic!("expected key operation, got {other:?}"),
        };

        assert_eq!(expected_pressed, pressed);
        assert_eq!((expected_extended, expected_code), scancode.as_u8());
    }
}
