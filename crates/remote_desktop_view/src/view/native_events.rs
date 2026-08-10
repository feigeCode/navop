use windows_rdp_host::{
    WindowsRdpDisconnectReason, WindowsRdpEvent, WindowsRdpFatalError, WindowsRdpHost,
    WindowsRdpLogonError, WindowsRdpRawEvent, WindowsRdpWarning,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum NativeRdpConnectionPhase {
    Waiting,
    Connecting,
    Active,
    Reconnecting,
    Disconnected,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum NativeRdpUiEffect {
    CloseConfirmed,
    FocusReleased,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct NativeRdpEventState {
    generation: u64,
    phase: NativeRdpConnectionPhase,
    host_ready: bool,
    remote_size: Option<(u32, u32)>,
    reconnect_attempt: Option<(u32, Option<u32>)>,
    authentication_warning_visible: bool,
    last_disconnect: Option<WindowsRdpDisconnectReason>,
    last_warning: Option<WindowsRdpWarning>,
    last_fatal_error: Option<WindowsRdpFatalError>,
    last_logon_error: Option<WindowsRdpLogonError>,
    network_quality: Option<u32>,
    fullscreen: bool,
    close_confirmed: bool,
    focus_release_pending: bool,
}

impl NativeRdpEventState {
    pub(super) const fn new(generation: u64) -> Self {
        Self {
            generation,
            phase: NativeRdpConnectionPhase::Waiting,
            host_ready: false,
            remote_size: None,
            reconnect_attempt: None,
            authentication_warning_visible: false,
            last_disconnect: None,
            last_warning: None,
            last_fatal_error: None,
            last_logon_error: None,
            network_quality: None,
            fullscreen: false,
            close_confirmed: false,
            focus_release_pending: false,
        }
    }

    pub(super) fn reset_for_generation(&mut self, generation: u64) {
        *self = Self::new(generation);
    }

    pub(super) const fn close_confirmed(&self) -> bool {
        self.close_confirmed
    }

    pub(super) fn take_focus_release_pending(&mut self) -> bool {
        std::mem::take(&mut self.focus_release_pending)
    }

    pub(super) fn apply(
        &mut self,
        current_generation: u64,
        event: WindowsRdpEvent,
    ) -> Option<NativeRdpUiEffect> {
        if current_generation != self.generation || event.generation() != current_generation {
            return None;
        }

        match event {
            WindowsRdpEvent::HostReady { capabilities, .. } => {
                self.host_ready = capabilities.is_available();
            }
            WindowsRdpEvent::Connecting { .. } => {
                self.phase = NativeRdpConnectionPhase::Connecting;
            }
            WindowsRdpEvent::Connected { .. }
            | WindowsRdpEvent::LoginComplete { .. }
            | WindowsRdpEvent::Reconnected { .. } => {
                self.phase = NativeRdpConnectionPhase::Active;
                self.reconnect_attempt = None;
            }
            WindowsRdpEvent::Reconnecting {
                attempt,
                max_attempts,
                ..
            } => {
                self.phase = NativeRdpConnectionPhase::Reconnecting;
                self.reconnect_attempt = Some((attempt, max_attempts));
            }
            WindowsRdpEvent::NetworkStatusChanged { quality, .. } => {
                self.network_quality = quality;
            }
            WindowsRdpEvent::RemoteDesktopSizeChanged { width, height, .. } => {
                self.remote_size = Some((width, height));
            }
            WindowsRdpEvent::FullscreenChanged { fullscreen, .. } => {
                self.fullscreen = fullscreen;
            }
            WindowsRdpEvent::AuthenticationWarning { visible, .. } => {
                self.authentication_warning_visible = visible;
            }
            WindowsRdpEvent::Warning { warning, .. } => {
                self.last_warning = Some(warning);
            }
            WindowsRdpEvent::FatalError { error, .. } => {
                self.last_fatal_error = Some(error);
            }
            WindowsRdpEvent::LogonError { error, .. } => {
                self.last_logon_error = Some(error);
            }
            WindowsRdpEvent::Disconnected { reason, .. } => {
                self.phase = NativeRdpConnectionPhase::Disconnected;
                self.reconnect_attempt = None;
                self.last_disconnect = Some(reason);
            }
            WindowsRdpEvent::CloseConfirmed { .. } => {
                self.close_confirmed = true;
                return Some(NativeRdpUiEffect::CloseConfirmed);
            }
            WindowsRdpEvent::FocusReleased { .. } => {
                self.focus_release_pending = true;
                return Some(NativeRdpUiEffect::FocusReleased);
            }
            WindowsRdpEvent::Unknown { .. } => {}
        }

        None
    }
}

pub(super) trait NativeRdpEventSource {
    fn generation(&self) -> u64;
    fn drain_events(&self) -> Vec<WindowsRdpRawEvent>;
}

impl NativeRdpEventSource for WindowsRdpHost {
    fn generation(&self) -> u64 {
        WindowsRdpHost::generation(self)
    }

    fn drain_events(&self) -> Vec<WindowsRdpRawEvent> {
        WindowsRdpHost::drain_events(self)
    }
}

pub(super) fn drain_native_events(
    source: &impl NativeRdpEventSource,
    state: &mut NativeRdpEventState,
) -> Vec<NativeRdpUiEffect> {
    let current_generation = source.generation();
    source
        .drain_events()
        .into_iter()
        .filter_map(|event| state.apply(current_generation, event.into()))
        .collect()
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;

    use windows_rdp_host::{
        WindowsRdpDiagnosticCategory, WindowsRdpFatalErrorKind, WindowsRdpLogonErrorKind,
        WindowsRdpRawEvent, WindowsRdpWarningKind,
    };

    use super::*;

    const EVENT_CONNECTING: u32 = 1;
    const EVENT_DISCONNECTED: u32 = 15;
    const EVENT_CLOSE_CONFIRMED: u32 = 16;

    struct FakeEventSource {
        generation: u64,
        events: RefCell<Vec<WindowsRdpRawEvent>>,
    }

    impl FakeEventSource {
        fn new(generation: u64, events: Vec<WindowsRdpRawEvent>) -> Self {
            Self {
                generation,
                events: RefCell::new(events),
            }
        }
    }

    impl NativeRdpEventSource for FakeEventSource {
        fn generation(&self) -> u64 {
            self.generation
        }

        fn drain_events(&self) -> Vec<WindowsRdpRawEvent> {
            std::mem::take(&mut *self.events.borrow_mut())
        }
    }

    fn raw(
        generation: u64,
        kind: u32,
        code: i32,
        payload: impl Into<Vec<u8>>,
    ) -> WindowsRdpRawEvent {
        WindowsRdpRawEvent {
            generation,
            kind,
            code,
            payload: payload.into(),
        }
    }

    fn apply(state: &mut NativeRdpEventState, event: WindowsRdpEvent) -> Option<NativeRdpUiEffect> {
        state.apply(state.generation, event)
    }

    #[test]
    fn stale_generation_does_not_change_current_state() {
        let mut state = NativeRdpEventState::new(8);
        let before = state.clone();

        assert_eq!(
            None,
            state.apply(8, WindowsRdpEvent::Connecting { generation: 7 })
        );
        assert_eq!(before, state);
    }

    #[test]
    fn connection_and_reconnection_events_reach_active_state() {
        let mut state = NativeRdpEventState::new(8);

        apply(&mut state, WindowsRdpEvent::Connecting { generation: 8 });
        assert_eq!(NativeRdpConnectionPhase::Connecting, state.phase);

        apply(&mut state, WindowsRdpEvent::Connected { generation: 8 });
        assert_eq!(NativeRdpConnectionPhase::Active, state.phase);

        apply(
            &mut state,
            WindowsRdpEvent::Reconnecting {
                generation: 8,
                attempt: 3,
                max_attempts: Some(10),
            },
        );
        assert_eq!(NativeRdpConnectionPhase::Reconnecting, state.phase);
        assert_eq!(Some((3, Some(10))), state.reconnect_attempt);

        apply(&mut state, WindowsRdpEvent::Reconnected { generation: 8 });
        assert_eq!(NativeRdpConnectionPhase::Active, state.phase);
        assert_eq!(None, state.reconnect_attempt);

        apply(&mut state, WindowsRdpEvent::LoginComplete { generation: 8 });
        assert_eq!(NativeRdpConnectionPhase::Active, state.phase);
    }

    #[test]
    fn disconnect_preserves_stable_category_and_both_raw_codes() {
        let mut state = NativeRdpEventState::new(8);
        let reason =
            WindowsRdpDisconnectReason::new(WindowsRdpDiagnosticCategory::Network, 2308, Some(262));

        apply(
            &mut state,
            WindowsRdpEvent::Disconnected {
                generation: 8,
                reason,
            },
        );

        assert_eq!(NativeRdpConnectionPhase::Disconnected, state.phase);
        assert_eq!(Some(reason), state.last_disconnect);
        assert_eq!(
            WindowsRdpDiagnosticCategory::Network,
            state.last_disconnect.unwrap().category()
        );
        assert_eq!(2308, state.last_disconnect.unwrap().disconnect_code());
        assert_eq!(Some(262), state.last_disconnect.unwrap().extended_code());
    }

    #[test]
    fn diagnostics_keep_signed_raw_codes_without_creating_ui_text() {
        let mut state = NativeRdpEventState::new(8);

        apply(
            &mut state,
            WindowsRdpEvent::Warning {
                generation: 8,
                warning: WindowsRdpWarning::from_native_code(1),
            },
        );
        apply(
            &mut state,
            WindowsRdpEvent::FatalError {
                generation: 8,
                error: WindowsRdpFatalError::from_native_code(100),
            },
        );
        apply(
            &mut state,
            WindowsRdpEvent::LogonError {
                generation: 8,
                error: WindowsRdpLogonError::from_native_code(-1_073_741_715),
            },
        );

        assert_eq!(
            Some(WindowsRdpWarningKind::BitmapCacheCorrupt),
            state.last_warning.map(WindowsRdpWarning::kind)
        );
        assert_eq!(Some(1), state.last_warning.map(WindowsRdpWarning::code));
        assert_eq!(
            Some(WindowsRdpFatalErrorKind::WinsockInitialization),
            state.last_fatal_error.map(WindowsRdpFatalError::kind)
        );
        assert_eq!(
            Some(100),
            state.last_fatal_error.map(WindowsRdpFatalError::code)
        );
        assert_eq!(
            Some(WindowsRdpLogonErrorKind::BadCredentials),
            state.last_logon_error.map(WindowsRdpLogonError::kind)
        );
        assert_eq!(
            Some(-1_073_741_715),
            state.last_logon_error.map(WindowsRdpLogonError::code)
        );
    }

    #[test]
    fn presentation_independent_state_events_are_reduced() {
        let mut state = NativeRdpEventState::new(8);

        apply(
            &mut state,
            WindowsRdpEvent::RemoteDesktopSizeChanged {
                generation: 8,
                width: 1920,
                height: 1080,
            },
        );
        apply(
            &mut state,
            WindowsRdpEvent::NetworkStatusChanged {
                generation: 8,
                quality: Some(87),
            },
        );
        apply(
            &mut state,
            WindowsRdpEvent::AuthenticationWarning {
                generation: 8,
                visible: true,
            },
        );
        apply(
            &mut state,
            WindowsRdpEvent::FullscreenChanged {
                generation: 8,
                fullscreen: true,
            },
        );

        assert_eq!(Some((1920, 1080)), state.remote_size);
        assert_eq!(Some(87), state.network_quality);
        assert!(state.authentication_warning_visible);
        assert!(state.fullscreen);
    }

    #[test]
    fn close_and_focus_events_produce_explicit_owner_thread_effects() {
        let mut state = NativeRdpEventState::new(8);

        assert!(!state.close_confirmed());
        assert!(!state.take_focus_release_pending());
        assert_eq!(
            Some(NativeRdpUiEffect::CloseConfirmed),
            apply(
                &mut state,
                WindowsRdpEvent::CloseConfirmed { generation: 8 }
            )
        );
        assert!(state.close_confirmed());
        assert_eq!(
            Some(NativeRdpUiEffect::FocusReleased),
            apply(&mut state, WindowsRdpEvent::FocusReleased { generation: 8 })
        );
        assert!(state.take_focus_release_pending());
        assert!(!state.take_focus_release_pending());
    }

    #[test]
    fn unknown_and_malformed_events_do_not_destroy_existing_state() {
        let mut state = NativeRdpEventState::new(8);
        apply(&mut state, WindowsRdpEvent::Connected { generation: 8 });
        let before = state.clone();

        apply(&mut state, raw(8, EVENT_CONNECTING, 1, []).into());

        assert_eq!(before, state);
    }

    #[test]
    fn adapter_drains_owned_raw_events_then_decodes_and_filters_on_owner_generation() {
        let source = FakeEventSource::new(
            8,
            vec![
                raw(7, EVENT_CONNECTING, 0, []),
                raw(8, EVENT_CONNECTING, 0, []),
                raw(8, EVENT_DISCONNECTED, 2308, 262_i32.to_le_bytes()),
                raw(8, EVENT_CLOSE_CONFIRMED, 0, []),
            ],
        );
        let mut state = NativeRdpEventState::new(8);

        let effects = drain_native_events(&source, &mut state);

        assert_eq!(NativeRdpConnectionPhase::Disconnected, state.phase);
        assert_eq!(
            Some(WindowsRdpDiagnosticCategory::Network),
            state
                .last_disconnect
                .map(WindowsRdpDisconnectReason::category)
        );
        assert_eq!(vec![NativeRdpUiEffect::CloseConfirmed], effects);
        assert!(state.close_confirmed());
        assert!(source.drain_events().is_empty());
        assert!(drain_native_events(&source, &mut state).is_empty());
        assert!(
            state.close_confirmed(),
            "a regular owner-thread drain must not hide close confirmation from the close poll"
        );
    }

    #[test]
    fn source_and_state_generation_mismatch_drops_the_drained_batch() {
        let source = FakeEventSource::new(9, vec![raw(9, EVENT_CONNECTING, 0, [])]);
        let mut state = NativeRdpEventState::new(8);
        let before = state.clone();

        assert!(drain_native_events(&source, &mut state).is_empty());
        assert_eq!(before, state);
        assert!(source.drain_events().is_empty());

        state.reset_for_generation(9);
        let source = FakeEventSource::new(9, vec![raw(9, EVENT_CONNECTING, 0, [])]);
        assert!(drain_native_events(&source, &mut state).is_empty());
        assert_eq!(NativeRdpConnectionPhase::Connecting, state.phase);
    }
}
