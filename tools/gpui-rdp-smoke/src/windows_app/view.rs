use std::time::{Duration, Instant};

use gpui::{
    Context, IntoElement, ParentElement, Render, Styled, Subscription, Task, Window, div, px, rgb,
};
use windows_rdp_host::{WindowsRdpConnectionState, WindowsRdpEvent, WindowsRdpHostLifecycle};

use super::{log_host_error, physical_viewport_size, session};
use crate::cli::Config;

mod support;

const POLL_INTERVAL: Duration = Duration::from_millis(100);
const LOGIN_PRESENTATION_REFRESH_DELAY: Duration = Duration::from_millis(300);

pub(super) struct SmokeView {
    session: Option<session::NativeSession>,
    status: String,
    started_at: Instant,
    timeout: Duration,
    login_complete: bool,
    terminal_failure: bool,
    timed_out: bool,
    last_connection_state: Option<WindowsRdpConnectionState>,
    last_bounds: Option<(i32, i32)>,
    _poll_task: Task<()>,
    login_presentation_refresh_task: Option<Task<()>>,
    _bounds_subscription: Subscription,
}

impl SmokeView {
    pub(super) fn new(config: Config, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let timeout = Duration::from_secs(config.timeout_seconds);
        let poll_task = support::spawn_poll_task(cx);
        support::defer_initialization(config, window, cx);
        let bounds_subscription = support::observe_bounds(window, cx);
        support::register_close_handler(window, cx);
        Self {
            session: None,
            status: "GPUI window ready; native RDP initialization is deferred".to_owned(),
            started_at: Instant::now(),
            timeout,
            login_complete: false,
            terminal_failure: false,
            timed_out: false,
            last_connection_state: None,
            last_bounds: None,
            _poll_task: poll_task,
            login_presentation_refresh_task: None,
            _bounds_subscription: bounds_subscription,
        }
    }

    pub(super) fn poll_host(&mut self, window: &Window, cx: &mut Context<Self>) {
        let previous_status = self.status.clone();
        self.synchronize_presentation(window);
        if !self.host_is_open() {
            return;
        }
        self.poll_events(window, cx);
        self.poll_connection_state();
        self.check_timeout();
        if self.status != previous_status {
            cx.notify();
        }
    }

    fn host_is_open(&self) -> bool {
        self.session
            .as_ref()
            .is_some_and(|session| session.host.lifecycle() == WindowsRdpHostLifecycle::Open)
    }

    fn poll_events(&mut self, window: &Window, cx: &mut Context<Self>) {
        let events = self
            .session
            .as_ref()
            .expect("open host requires a session")
            .host
            .drain_events();
        for raw in events {
            println!("raw event: {raw:?}");
            let event = WindowsRdpEvent::from(raw);
            println!("event: {event:?}");
            self.handle_event(event, window, cx);
        }
    }

    fn poll_connection_state(&mut self) {
        let result = self
            .session
            .as_mut()
            .expect("session remains present while polling")
            .host
            .connection_state();
        match result {
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
    }

    fn check_timeout(&mut self) {
        if self.login_complete || self.terminal_failure || self.started_at.elapsed() < self.timeout
        {
            return;
        }
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

    fn handle_event(&mut self, event: WindowsRdpEvent, window: &Window, cx: &mut Context<Self>) {
        match event {
            WindowsRdpEvent::Connecting { .. } => self.status = "RDP is connecting".to_owned(),
            WindowsRdpEvent::Connected { .. } => {
                self.status = "RDP transport connected; waiting for login".to_owned();
                self.refresh_connected_presentation(window, "host shown after connected");
            }
            WindowsRdpEvent::LoginComplete { .. } => self.handle_login_complete(window, cx),
            WindowsRdpEvent::Warning { warning, .. } => eprintln!(
                "diagnostic: event=Warning kind={:?} code={}",
                warning.kind(),
                warning.code()
            ),
            WindowsRdpEvent::FatalError { error, .. } => self.handle_fatal_error(error),
            WindowsRdpEvent::LogonError { error, .. } => self.handle_logon_error(error),
            WindowsRdpEvent::Disconnected { reason, .. } => self.handle_disconnected(reason),
            WindowsRdpEvent::CloseConfirmed { .. } => println!("close: native close confirmed"),
            _ => {}
        }
    }

    fn handle_login_complete(&mut self, window: &Window, cx: &mut Context<Self>) {
        self.refresh_connected_presentation(window, "host refreshed after login complete");
        self.login_complete = true;
        self.status = "RDP login complete".to_owned();
        if let Some(generation) = self
            .session
            .as_ref()
            .map(|session| session.host.generation())
        {
            println!(
                "presentation: scheduling delayed login refresh generation={generation} delay_ms={}",
                LOGIN_PRESENTATION_REFRESH_DELAY.as_millis()
            );
            self.login_presentation_refresh_task = Some(support::spawn_login_presentation_refresh(
                generation,
                LOGIN_PRESENTATION_REFRESH_DELAY,
                cx,
            ));
        }
        println!("RESULT: LOGIN_COMPLETE");
    }

    fn refresh_connected_presentation(&mut self, window: &Window, stage: &'static str) {
        let bounds = physical_viewport_size(window);
        let Some(session) = self.session.as_mut() else {
            return;
        };
        if session.host.lifecycle() != WindowsRdpHostLifecycle::Open {
            return;
        }
        if let Err(error) = session.overlay.refresh(0, 0, bounds.0, bounds.1) {
            eprintln!("ERROR: stage=refresh_connected_overlay error={error}");
            return;
        }
        if let Err(error) = session.host.set_bounds(0, 0, bounds.0, bounds.1) {
            log_host_error("refresh_connected_bounds", error);
            return;
        }
        if let Err(error) = session.host.set_visible(true) {
            log_host_error("refresh_connected_visible", error);
            return;
        }
        self.last_bounds = Some(bounds);
        println!("presentation: {stage}");
        if let Err(error) = session.host.focus() {
            log_host_error("refresh_connected_focus_best_effort", error);
        } else {
            println!("focus: success after native presentation refresh");
        }
    }

    fn handle_fatal_error(&mut self, error: windows_rdp_host::WindowsRdpFatalError) {
        self.terminal_failure = true;
        self.status = "RDP fatal error; see console".to_owned();
        eprintln!(
            "diagnostic: event=FatalError kind={:?} code={}",
            error.kind(),
            error.code()
        );
        eprintln!("RESULT: FATAL_ERROR");
    }

    fn handle_logon_error(&mut self, error: windows_rdp_host::WindowsRdpLogonError) {
        self.terminal_failure = true;
        self.status = "RDP logon error; see console".to_owned();
        eprintln!(
            "diagnostic: event=LogonError kind={:?} code={}",
            error.kind(),
            error.code()
        );
        eprintln!("RESULT: LOGON_ERROR");
    }

    fn handle_disconnected(&mut self, reason: windows_rdp_host::WindowsRdpDisconnectReason) {
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

    fn synchronize_presentation(&mut self, window: &Window) {
        let Some(session) = self.session.as_mut() else {
            return;
        };
        if session.host.lifecycle() != WindowsRdpHostLifecycle::Open {
            return;
        }
        let bounds = physical_viewport_size(window);
        if let Err(error) = session.overlay.synchronize(0, 0, bounds.0, bounds.1) {
            eprintln!("ERROR: stage=synchronize_overlay error={error}");
            return;
        }
        if self.last_bounds == Some(bounds) {
            return;
        }
        self.last_bounds = Some(bounds);
        println!(
            "resize: physical_width={} physical_height={}",
            bounds.0, bounds.1
        );
        if let Err(error) = session.host.set_bounds(0, 0, bounds.0, bounds.1) {
            log_host_error("set_bounds", error);
        }
    }

    fn prepare_close(&mut self) -> bool {
        let Some(mut session) = self.session.take() else {
            return true;
        };
        println!(
            "close: starting lifecycle={:?} login_complete={} terminal_failure={} timed_out={}",
            session.host.lifecycle(),
            self.login_complete,
            self.terminal_failure,
            self.timed_out
        );
        if let Err(error) = session.overlay.hide() {
            eprintln!("ERROR: stage=hide_overlay_before_close error={error}");
        }
        session.prepare_host_close();
        match session.host.close() {
            Ok(()) => match session.overlay.close() {
                Ok(()) => {
                    println!("close: completed");
                    true
                }
                Err(error) => {
                    eprintln!("ERROR: stage=close_overlay error={error}");
                    self.session = Some(session);
                    false
                }
            },
            Err(error) => {
                log_host_error("close", error);
                self.status = "Native RDP close failed; wait briefly and close again".to_owned();
                self.session = Some(session);
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
