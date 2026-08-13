use std::env;
use std::process::ExitCode;

const DEFAULT_PORT: u16 = 3389;
const DEFAULT_WIDTH: u32 = 1280;
const DEFAULT_HEIGHT: u32 = 720;
const DEFAULT_TIMEOUT_SECONDS: u64 = 60;

#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
struct Config {
    host: String,
    port: u16,
    username: Option<String>,
    domain: Option<String>,
    password: Option<String>,
    width: u32,
    height: u32,
    timeout_seconds: u64,
}

enum ParseOutcome {
    Run(Config),
    Help,
}

fn main() -> ExitCode {
    let password = env::var("NAVOP_RDP_PASSWORD").ok();
    let outcome = match parse_args(env::args().skip(1), password) {
        Ok(outcome) => outcome,
        Err(error) => {
            eprintln!("argument error: {error}\n");
            eprintln!("{}", usage());
            return ExitCode::from(2);
        }
    };

    match outcome {
        ParseOutcome::Help => {
            println!("{}", usage());
            ExitCode::SUCCESS
        }
        ParseOutcome::Run(config) => run(config),
    }
}

fn parse_args<I, S>(args: I, password: Option<String>) -> Result<ParseOutcome, String>
where
    I: IntoIterator<Item = S>,
    S: Into<String>,
{
    let mut host = None;
    let mut port = DEFAULT_PORT;
    let mut username = None;
    let mut domain = None;
    let mut width = DEFAULT_WIDTH;
    let mut height = DEFAULT_HEIGHT;
    let mut timeout_seconds = DEFAULT_TIMEOUT_SECONDS;
    let mut args = args.into_iter().map(Into::into);

    while let Some(argument) = args.next() {
        match argument.as_str() {
            "-h" | "--help" => return Ok(ParseOutcome::Help),
            "--host" => host = Some(next_value(&mut args, "--host")?),
            "--port" => port = parse_number(next_value(&mut args, "--port")?, "--port")?,
            "--username" => username = Some(next_value(&mut args, "--username")?),
            "--domain" => domain = Some(next_value(&mut args, "--domain")?),
            "--width" => {
                width = parse_positive_dimension(next_value(&mut args, "--width")?, "--width")?
            }
            "--height" => {
                height = parse_positive_dimension(next_value(&mut args, "--height")?, "--height")?
            }
            "--timeout-seconds" => {
                timeout_seconds = parse_positive_number(
                    next_value(&mut args, "--timeout-seconds")?,
                    "--timeout-seconds",
                )?
            }
            _ => return Err(format!("unknown argument `{argument}`")),
        }
    }

    let host = host.ok_or_else(|| "missing required argument `--host <host>`".to_owned())?;
    if host.is_empty() {
        return Err("`--host` must not be empty".to_owned());
    }

    Ok(ParseOutcome::Run(Config {
        host,
        port,
        username,
        domain,
        password,
        width,
        height,
        timeout_seconds,
    }))
}

fn next_value<I>(args: &mut I, option: &str) -> Result<String, String>
where
    I: Iterator<Item = String>,
{
    args.next()
        .ok_or_else(|| format!("missing value for `{option}`"))
}

fn parse_number<T>(value: String, option: &str) -> Result<T, String>
where
    T: std::str::FromStr,
{
    value
        .parse()
        .map_err(|_| format!("invalid value `{value}` for `{option}`"))
}

fn parse_positive_number<T>(value: String, option: &str) -> Result<T, String>
where
    T: std::str::FromStr + Default + PartialOrd,
{
    let parsed = parse_number(value, option)?;
    if parsed <= T::default() {
        return Err(format!("`{option}` must be greater than zero"));
    }
    Ok(parsed)
}

fn parse_positive_dimension(value: String, option: &str) -> Result<u32, String> {
    let dimension = parse_positive_number(value, option)?;
    if dimension > i32::MAX as u32 {
        return Err(format!("`{option}` must not exceed {}", i32::MAX));
    }
    Ok(dimension)
}

fn usage() -> &'static str {
    "Minimal GPUI Windows native RDP smoke client

Usage:
  gpui-rdp-smoke --host <host> [options]

Required:
  --host <host>                 RDP server name or IP address

Options:
  --port <port>                 RDP port (default: 3389)
  --username <username>         User name
  --domain <domain>             Windows domain
  --width <pixels>              Remote desktop width (default: 1280)
  --height <pixels>             Remote desktop height (default: 720)
  --timeout-seconds <seconds>   Login diagnostic timeout (default: 60)
  -h, --help                    Print this help

Password:
  Read only from NAVOP_RDP_PASSWORD. Do not put a password on the command line."
}

#[cfg(not(target_os = "windows"))]
fn run(_config: Config) -> ExitCode {
    eprintln!("gpui-rdp-smoke is only supported on Windows");
    ExitCode::from(2)
}

#[cfg(target_os = "windows")]
fn run(config: Config) -> ExitCode {
    // GPUI normally renders Windows windows through a DirectComposition
    // visual attached to the top-level HWND. The RDP ActiveX control is a
    // traditional child HWND, so that composition visual can cover the child
    // even when the RDP session is connected and painting. Force GPUI's HWND
    // swap-chain path before the platform singleton is constructed.
    //
    // SAFETY: this is the first Windows-specific operation in the process,
    // before GPUI creates worker threads or reads its renderer environment.
    unsafe {
        env::set_var("GPUI_DISABLE_DIRECT_COMPOSITION", "1");
    }
    println!("presentation: GPUI DirectComposition disabled for native child HWND hosting");
    windows_app::run(config);
    ExitCode::SUCCESS
}

#[cfg(target_os = "windows")]
mod windows_app {
    use std::time::{Duration, Instant};

    use gpui::{
        AppContext, Bounds, Context, IntoElement, ParentElement, Pixels, QuitMode, Render, Styled,
        Subscription, Task, TitlebarOptions, Window, WindowBounds, WindowOptions, div, px, rgb,
        size,
    };
    use raw_window_handle::RawWindowHandle;
    use windows_rdp_host::{
        WindowsRdpColorDepth, WindowsRdpConnectionOptions, WindowsRdpConnectionState,
        WindowsRdpCredentialBundle, WindowsRdpEvent, WindowsRdpHost, WindowsRdpHostError,
        WindowsRdpHostLifecycle, WindowsRdpHostOptions, WindowsRdpParentWindow,
    };

    use super::Config;

    const POLL_INTERVAL: Duration = Duration::from_millis(100);

    pub(super) fn run(config: Config) {
        let window_width = config.width as f32;
        let window_height = config.height as f32;

        gpui_platform::application()
            .with_quit_mode(QuitMode::LastWindowClosed)
            .run(move |cx| {
                let bounds = Bounds::centered(None, size(px(window_width), px(window_height)), cx);
                let options = WindowOptions {
                    window_bounds: Some(WindowBounds::Windowed(bounds)),
                    titlebar: Some(TitlebarOptions {
                        title: Some("Navop GPUI Native RDP Smoke".into()),
                        ..Default::default()
                    }),
                    ..Default::default()
                };

                if let Err(error) = cx.open_window(options, move |window, cx| {
                    window.activate_window();
                    cx.new(|cx| SmokeView::new(config, window, cx))
                }) {
                    eprintln!("ERROR: stage=open_gpui_window error={error}");
                }
            });
    }

    struct SmokeView {
        host: Option<WindowsRdpHost>,
        status: String,
        started_at: Instant,
        timeout: Duration,
        login_complete: bool,
        terminal_failure: bool,
        timed_out: bool,
        last_connection_state: Option<WindowsRdpConnectionState>,
        last_bounds: Option<(i32, i32)>,
        _poll_task: Task<()>,
        _bounds_subscription: Subscription,
    }

    impl SmokeView {
        fn new(config: Config, window: &mut Window, cx: &mut Context<Self>) -> Self {
            let timeout = Duration::from_secs(config.timeout_seconds);

            let poll_task = cx.spawn(async move |view, cx| {
                loop {
                    cx.background_executor().timer(POLL_INTERVAL).await;
                    if view
                        .update_in(cx, |view, _window, cx| view.poll_host(cx))
                        .is_err()
                    {
                        break;
                    }
                }
            });

            cx.defer_in(window, move |view, window, cx| {
                println!("initialize: deferred GPUI window callback started");
                let (host, status, last_bounds) = initialize_host(config, window);
                view.host = host;
                view.status = status;
                view.started_at = Instant::now();
                view.last_bounds = last_bounds;
                cx.notify();
            });

            let bounds_subscription = cx.observe_window_bounds(window, |view, window, cx| {
                view.resize_host(window);
                cx.notify();
            });

            let weak_view = cx.entity().downgrade();
            window.on_window_should_close(cx, move |_window, cx| {
                weak_view
                    .update(cx, |view, _cx| view.prepare_close())
                    .unwrap_or(true)
            });

            Self {
                host: None,
                status: "GPUI window ready; native RDP initialization is deferred".to_owned(),
                started_at: Instant::now(),
                timeout,
                login_complete: false,
                terminal_failure: false,
                timed_out: false,
                last_connection_state: None,
                last_bounds: None,
                _poll_task: poll_task,
                _bounds_subscription: bounds_subscription,
            }
        }

        fn poll_host(&mut self, cx: &mut Context<Self>) {
            let previous_status = self.status.clone();
            let Some(host) = self.host.as_ref() else {
                return;
            };
            if host.lifecycle() != WindowsRdpHostLifecycle::Open {
                return;
            }

            let events = host.drain_events();
            for raw in events {
                println!("raw event: {raw:?}");
                let event = WindowsRdpEvent::from(raw);
                println!("event: {event:?}");
                self.handle_event(event);
            }

            let connection_state = self
                .host
                .as_mut()
                .expect("host remains present while polling")
                .connection_state();
            match connection_state {
                Ok(state) if self.last_connection_state != Some(state) => {
                    println!("connection state: {state:?}");
                    self.last_connection_state = Some(state);
                }
                Ok(_) => {}
                Err(error) => {
                    log_host_error("connection_state", error);
                    self.status = "Failed to query RDP connection state; see console".to_owned();
                    self.terminal_failure = true;
                }
            }

            if !self.login_complete
                && !self.terminal_failure
                && self.started_at.elapsed() >= self.timeout
            {
                self.timed_out = true;
                self.terminal_failure = true;
                self.status = format!(
                    "RDP login did not complete within {} seconds; see console",
                    self.timeout.as_secs()
                );
                eprintln!(
                    "timeout: elapsed_seconds={} connection_state={:?}",
                    self.started_at.elapsed().as_secs(),
                    self.last_connection_state
                );
                eprintln!("RESULT: TIMEOUT");
            }

            if self.status != previous_status {
                cx.notify();
            }
        }

        fn handle_event(&mut self, event: WindowsRdpEvent) {
            match event {
                WindowsRdpEvent::Connecting { .. } => {
                    self.status = "RDP is connecting".to_owned();
                }
                WindowsRdpEvent::Connected { .. } => {
                    self.status = "RDP transport connected; waiting for login".to_owned();
                }
                WindowsRdpEvent::LoginComplete { .. } => {
                    self.login_complete = true;
                    self.status = "RDP login complete".to_owned();
                    println!("RESULT: LOGIN_COMPLETE");
                }
                WindowsRdpEvent::Warning { warning, .. } => {
                    eprintln!(
                        "diagnostic: event=Warning kind={:?} code={}",
                        warning.kind(),
                        warning.code()
                    );
                }
                WindowsRdpEvent::FatalError { error, .. } => {
                    self.terminal_failure = true;
                    self.status = "RDP fatal error; see console".to_owned();
                    eprintln!(
                        "diagnostic: event=FatalError kind={:?} code={}",
                        error.kind(),
                        error.code()
                    );
                    eprintln!("RESULT: FATAL_ERROR");
                }
                WindowsRdpEvent::LogonError { error, .. } => {
                    self.terminal_failure = true;
                    self.status = "RDP logon error; see console".to_owned();
                    eprintln!(
                        "diagnostic: event=LogonError kind={:?} code={}",
                        error.kind(),
                        error.code()
                    );
                    eprintln!("RESULT: LOGON_ERROR");
                }
                WindowsRdpEvent::Disconnected { reason, .. } => {
                    self.status = "RDP disconnected; see console".to_owned();
                    eprintln!(
                        "diagnostic: event=Disconnected category={:?} disconnect_code={} extended_code={:?}",
                        reason.category(),
                        reason.disconnect_code(),
                        reason.extended_code()
                    );
                    if !self.login_complete {
                        self.terminal_failure = true;
                        eprintln!("RESULT: DISCONNECTED_BEFORE_LOGIN");
                    }
                }
                WindowsRdpEvent::CloseConfirmed { .. } => {
                    println!("close: native close confirmed");
                }
                _ => {}
            }
        }

        fn resize_host(&mut self, window: &Window) {
            let Some(host) = self.host.as_mut() else {
                return;
            };
            if host.lifecycle() != WindowsRdpHostLifecycle::Open {
                return;
            }

            let bounds = physical_viewport_size(window);
            if self.last_bounds == Some(bounds) {
                return;
            }
            self.last_bounds = Some(bounds);
            println!(
                "resize: physical_width={} physical_height={}",
                bounds.0, bounds.1
            );
            if let Err(error) = host.set_bounds(0, 0, bounds.0, bounds.1) {
                log_host_error("set_bounds", error);
            }
        }

        fn prepare_close(&mut self) -> bool {
            let Some(mut host) = self.host.take() else {
                return true;
            };

            println!(
                "close: starting lifecycle={:?} login_complete={} terminal_failure={} timed_out={}",
                host.lifecycle(),
                self.login_complete,
                self.terminal_failure,
                self.timed_out
            );

            if host.lifecycle() == WindowsRdpHostLifecycle::Open {
                if let Err(error) = host.set_visible(false) {
                    log_host_error("close_set_visible", error);
                }
                if let Err(error) = host.disconnect() {
                    log_host_error("close_disconnect", error);
                }
            }

            match host.close() {
                Ok(()) => {
                    println!("close: completed");
                    true
                }
                Err(error) => {
                    log_host_error("close", error);
                    self.status =
                        "Native RDP close failed; wait briefly and close again".to_owned();
                    self.host = Some(host);
                    false
                }
            }
        }
    }

    impl Render for SmokeView {
        fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
            div()
                .size_full()
                .flex()
                .items_center()
                .justify_center()
                .bg(rgb(0x111827))
                .text_color(rgb(0xf9fafb))
                .p(px(16.0))
                .child(self.status.clone())
        }
    }

    fn initialize_host(
        config: Config,
        window: &Window,
    ) -> (Option<WindowsRdpHost>, String, Option<(i32, i32)>) {
        println!(
            "config: host={} port={} username_present={} domain_present={} password_env_present={} desktop={}x{} timeout_seconds={}",
            config.host,
            config.port,
            config.username.is_some(),
            config.domain.is_some(),
            config.password.is_some(),
            config.width,
            config.height,
            config.timeout_seconds
        );

        let mut credentials = WindowsRdpCredentialBundle::new();
        if let Some(username) = config.username {
            credentials.set_username(username);
        }
        if let Some(domain) = config.domain {
            credentials.set_domain(domain);
        }
        if let Some(password) = config.password {
            credentials.set_server_password(password);
        }

        let connection_options = match WindowsRdpConnectionOptions::new(
            config.host,
            config.port,
            config.width,
            config.height,
            WindowsRdpColorDepth::Bpp32,
        ) {
            Ok(options) => options,
            Err(error) => {
                log_host_error("connection_options", error);
                return (
                    None,
                    "Invalid RDP connection options; see console".to_owned(),
                    None,
                );
            }
        };

        match WindowsRdpHost::probe() {
            Ok(capabilities) if capabilities.is_available() => {
                println!("probe: available=true capabilities={capabilities:?}");
            }
            Ok(capabilities) => {
                eprintln!("ERROR: stage=probe error=native_boundary_unavailable");
                eprintln!("ERROR_DEBUG: stage=probe capabilities={capabilities:?}");
                return (
                    None,
                    "Windows native RDP boundary is unavailable; see console".to_owned(),
                    None,
                );
            }
            Err(error) => {
                log_host_error("probe", error);
                return (
                    None,
                    "Windows native RDP probe failed; see console".to_owned(),
                    None,
                );
            }
        }

        let raw = match raw_window_handle::HasWindowHandle::window_handle(window) {
            Ok(handle) => handle.as_raw(),
            Err(error) => {
                eprintln!("ERROR: stage=get_gpui_window_handle error={error}");
                return (
                    None,
                    "Failed to get the GPUI native window handle; see console".to_owned(),
                    None,
                );
            }
        };
        let RawWindowHandle::Win32(handle) = raw else {
            eprintln!("ERROR: stage=get_gpui_window_handle error=handle_is_not_win32");
            return (
                None,
                "GPUI did not expose a Win32 HWND; see console".to_owned(),
                None,
            );
        };
        let hwnd = handle.hwnd.get() as usize;
        println!("create: parent_hwnd=0x{hwnd:016X}");
        let parent = unsafe { WindowsRdpParentWindow::from_raw(hwnd) };
        let mut host = match unsafe {
            WindowsRdpHost::create_with_parent(parent, WindowsRdpHostOptions::new(1))
        } {
            Ok(host) => {
                println!(
                    "create: success generation={} lifecycle={:?}",
                    host.generation(),
                    host.lifecycle()
                );
                host
            }
            Err(error) => {
                log_host_error("create_with_parent", error);
                return (
                    None,
                    "Windows native RDP host creation failed; see console".to_owned(),
                    None,
                );
            }
        };

        let bounds = physical_viewport_size(window);
        println!(
            "bounds: physical_width={} physical_height={} scale_factor={}",
            bounds.0,
            bounds.1,
            window.scale_factor()
        );
        if let Err(error) = host.set_bounds(0, 0, bounds.0, bounds.1) {
            return failed_after_create(host, "set_bounds", error);
        }
        if let Err(error) = host.set_visible(true) {
            return failed_after_create(host, "set_visible", error);
        }
        if let Err(error) = host.apply_credentials(&credentials) {
            return failed_after_create(host, "apply_credentials", error);
        }
        println!("credentials: applied");
        if let Err(error) = host.connect(&connection_options) {
            return failed_after_create(host, "connect", error);
        }
        println!("connect: synchronous call succeeded; waiting for events");
        if let Err(error) = host.focus() {
            log_host_error("focus_best_effort", error);
        } else {
            println!("focus: success");
        }

        (
            Some(host),
            "RDP connect requested; waiting for native events".to_owned(),
            Some(bounds),
        )
    }

    fn failed_after_create(
        mut host: WindowsRdpHost,
        stage: &str,
        error: WindowsRdpHostError,
    ) -> (Option<WindowsRdpHost>, String, Option<(i32, i32)>) {
        log_host_error(stage, error);
        if host.lifecycle() == WindowsRdpHostLifecycle::Open {
            if let Err(error) = host.set_visible(false) {
                log_host_error("failure_cleanup_set_visible", error);
            }
        }
        match host.close() {
            Ok(()) => (
                None,
                format!("RDP initialization failed at {stage}; see console"),
                None,
            ),
            Err(error) => {
                log_host_error("failure_cleanup_close", error);
                (
                    Some(host),
                    format!(
                        "RDP initialization failed at {stage}; native cleanup needs another close attempt"
                    ),
                    None,
                )
            }
        }
    }

    fn physical_viewport_size(window: &Window) -> (i32, i32) {
        let viewport = window.viewport_size();
        let scale_factor = window.scale_factor();
        (
            physical_pixels(viewport.width, scale_factor),
            physical_pixels(viewport.height, scale_factor),
        )
    }

    fn physical_pixels(value: Pixels, scale_factor: f32) -> i32 {
        (f32::from(value) * scale_factor)
            .round()
            .clamp(1.0, i32::MAX as f32) as i32
    }

    fn log_host_error(stage: &str, error: WindowsRdpHostError) {
        eprintln!("ERROR: stage={stage} error={error}");
        eprintln!("ERROR_DEBUG: stage={stage} error={error:?}");
        eprintln!(
            "ERROR_FIELDS: stage={stage} native_result={:?} native_stage={:?} win32_code={:?} hresult_code={:?} hresult_kind={:?}",
            error.native_result(),
            error.stage(),
            error.win32_code(),
            error.hresult().map(|value| value.code()),
            error.hresult().map(|value| value.kind())
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(args: &[&str]) -> Result<ParseOutcome, String> {
        parse_args(args.iter().copied(), None)
    }

    fn parse_error(args: &[&str]) -> String {
        match parse(args) {
            Ok(_) => panic!("expected argument parsing to fail"),
            Err(error) => error,
        }
    }

    #[test]
    fn parses_required_host_and_defaults() {
        let ParseOutcome::Run(config) = parse(&["--host", "rdp.example"]).unwrap() else {
            panic!("expected run configuration");
        };

        assert_eq!(config.host, "rdp.example");
        assert_eq!(config.port, DEFAULT_PORT);
        assert_eq!(config.width, DEFAULT_WIDTH);
        assert_eq!(config.height, DEFAULT_HEIGHT);
        assert_eq!(config.timeout_seconds, DEFAULT_TIMEOUT_SECONDS);
        assert!(config.username.is_none());
        assert!(config.domain.is_none());
        assert!(config.password.is_none());
    }

    #[test]
    fn parses_all_options_and_password_source() {
        let ParseOutcome::Run(config) = parse_args(
            [
                "--host",
                "10.0.0.5",
                "--port",
                "3390",
                "--username",
                "alice",
                "--domain",
                "EXAMPLE",
                "--width",
                "1600",
                "--height",
                "900",
                "--timeout-seconds",
                "90",
            ],
            Some("secret".to_owned()),
        )
        .unwrap() else {
            panic!("expected run configuration");
        };

        assert_eq!(config.port, 3390);
        assert_eq!(config.username.as_deref(), Some("alice"));
        assert_eq!(config.domain.as_deref(), Some("EXAMPLE"));
        assert_eq!(config.width, 1600);
        assert_eq!(config.height, 900);
        assert_eq!(config.timeout_seconds, 90);
        assert_eq!(config.password.as_deref(), Some("secret"));
    }

    #[test]
    fn rejects_missing_host_unknown_arguments_and_invalid_numbers() {
        assert!(parse_error(&[]).contains("--host"));
        assert!(parse_error(&["--wat"]).contains("unknown argument"));
        assert!(parse_error(&["--host", "rdp.example", "--port", "not-a-port"]).contains("--port"));
        assert!(
            parse_error(&["--host", "rdp.example", "--width", "0"]).contains("greater than zero")
        );
        assert!(
            parse_error(&["--host", "rdp.example", "--height", "2147483648"])
                .contains("must not exceed")
        );
    }

    #[test]
    fn help_does_not_require_host() {
        assert!(matches!(parse(&["--help"]).unwrap(), ParseOutcome::Help));
    }
}
