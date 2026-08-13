use std::time::{Duration, Instant};

use gpui::{
    Context, IntoElement, ParentElement, Render, Styled, Subscription, Task, Window, div, px, rgb,
};
use windows_rdp_host::{
    WindowsRdpConnectionState, WindowsRdpHostLifecycle, WindowsRdpSessionDisplaySettings,
};

use super::{log_host_error, physical_viewport_size, session};
use crate::cli::Config;

mod display;
mod events;
mod presentation;
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
    latest_session_display_settings: Option<WindowsRdpSessionDisplaySettings>,
    last_session_display_settings: Option<WindowsRdpSessionDisplaySettings>,
    display_epoch: u64,
    display_retry_after: Option<Instant>,
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
            latest_session_display_settings: None,
            last_session_display_settings: None,
            display_epoch: 0,
            display_retry_after: None,
            _poll_task: poll_task,
            login_presentation_refresh_task: None,
            _bounds_subscription: bounds_subscription,
        }
    }

    pub(super) fn poll_host(&mut self, window: &Window, cx: &mut Context<Self>) {
        let previous_status = self.status.clone();
        if !self.host_is_open() {
            return;
        }
        self.poll_events(window, cx);
        self.synchronize_presentation(window);
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

    fn synchronize_presentation(&mut self, window: &Window) {
        if !self.host_is_open() {
            return;
        }
        let bounds = physical_viewport_size(window);
        let bounds_changed = self.last_bounds != Some(bounds);
        {
            let session = self.session.as_mut().expect("open host requires a session");
            if let Err(error) = session.overlay.synchronize((0, 0, bounds.0, bounds.1)) {
                eprintln!("ERROR: stage=synchronize_overlay error={error}");
                return;
            }
            if bounds_changed {
                println!(
                    "resize: physical_width={} physical_height={}",
                    bounds.0, bounds.1
                );
                if let Err(error) = session.host.set_bounds(0, 0, bounds.0, bounds.1) {
                    log_host_error("set_bounds", error);
                    return;
                }
            }
        }
        if bounds_changed {
            self.last_bounds = Some(bounds);
        }
        let display_updated =
            self.synchronize_session_display_settings(window, false, "viewport_changed");
        if bounds_changed || display_updated {
            self.log_composition_diagnostics("viewport_changed");
        }
    }

    fn prepare_close(&mut self) -> bool {
        self.display_epoch = self.display_epoch.wrapping_add(1);
        self.login_complete = false;
        self.login_presentation_refresh_task = None;
        self.display_retry_after = None;
        self.latest_session_display_settings = None;
        self.last_session_display_settings = None;
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
