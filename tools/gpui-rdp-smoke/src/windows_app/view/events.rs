use gpui::{Context, Window};
use windows_rdp_host::WindowsRdpEvent;

use super::{LOGIN_PRESENTATION_REFRESH_DELAY, SmokeView, support};
use crate::windows_app::log_host_error;

impl SmokeView {
    pub(super) fn poll_events(&mut self, window: &Window, cx: &mut Context<Self>) {
        let (generation, events) = {
            let session = self.session.as_ref().expect("open host requires a session");
            (session.host.generation(), session.host.drain_events())
        };
        for raw in events {
            println!("raw event: {raw:?}");
            let event = WindowsRdpEvent::from(raw);
            println!("event: {event:?}");
            if event.generation() != generation {
                println!(
                    "event: stale generation skipped event_generation={} current_generation={generation}",
                    event.generation()
                );
                continue;
            }
            self.handle_event(event, window, cx);
        }
    }

    pub(super) fn poll_connection_state(&mut self) {
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

    fn handle_event(&mut self, event: WindowsRdpEvent, window: &Window, cx: &mut Context<Self>) {
        match event {
            WindowsRdpEvent::Connecting { .. } => self.status = "RDP is connecting".to_owned(),
            WindowsRdpEvent::Connected { .. } => self.handle_connected(window),
            WindowsRdpEvent::LoginComplete { .. } => self.handle_login_complete(window, cx),
            WindowsRdpEvent::Reconnecting {
                attempt,
                max_attempts,
                ..
            } => self.handle_reconnecting(attempt, max_attempts),
            WindowsRdpEvent::Reconnected { .. } => self.handle_reconnected(window, cx),
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

    fn handle_connected(&mut self, window: &Window) {
        self.status = "RDP transport connected; waiting for login".to_owned();
        self.present_connected(window, "connected");
    }

    fn handle_login_complete(&mut self, window: &Window, cx: &mut Context<Self>) {
        if self.login_complete {
            println!("presentation: duplicate login complete ignored");
            return;
        }
        self.begin_display_epoch();
        self.login_complete = true;
        self.present_login_complete(window, "login_complete");
        self.status = "RDP login complete".to_owned();
        self.schedule_login_compensation(cx);
        println!("RESULT: LOGIN_COMPLETE");
    }

    fn handle_reconnecting(&mut self, attempt: u32, max_attempts: Option<u32>) {
        self.begin_display_epoch();
        self.login_complete = false;
        self.status = "RDP is reconnecting".to_owned();
        println!(
            "presentation: reconnecting attempt={attempt} max_attempts={max_attempts:?} epoch={}",
            self.display_epoch
        );
    }

    fn handle_reconnected(&mut self, window: &Window, cx: &mut Context<Self>) {
        self.begin_display_epoch();
        self.login_complete = true;
        self.present_login_complete(window, "reconnected");
        self.status = "RDP reconnected".to_owned();
        self.schedule_login_compensation(cx);
    }

    fn begin_display_epoch(&mut self) {
        self.display_epoch = self.display_epoch.wrapping_add(1);
        self.login_presentation_refresh_task = None;
        self.display_retry_after = None;
        self.last_session_display_settings = None;
    }

    fn schedule_login_compensation(&mut self, cx: &mut Context<Self>) {
        let Some(generation) = self
            .session
            .as_ref()
            .map(|session| session.host.generation())
        else {
            return;
        };
        let token = support::LoginPresentationRefreshToken {
            generation,
            epoch: self.display_epoch,
        };
        println!(
            "presentation: scheduling delayed login refresh generation={generation} epoch={} delay_ms={}",
            self.display_epoch,
            LOGIN_PRESENTATION_REFRESH_DELAY.as_millis()
        );
        self.login_presentation_refresh_task = Some(support::spawn_login_presentation_refresh(
            token,
            LOGIN_PRESENTATION_REFRESH_DELAY,
            cx,
        ));
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
        self.begin_display_epoch();
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
        self.login_complete = false;
    }
}
