use std::collections::VecDeque;
use std::ffi::c_void;
use std::fmt;
use std::mem::size_of;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::slice;
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::{Mutex, MutexGuard};

use crate::ffi::{
    ABI_VERSION, EVENT_AUTHENTICATION_WARNING_DISMISSED, EVENT_AUTHENTICATION_WARNING_DISPLAYED,
    EVENT_CLOSE_CONFIRMED, EVENT_CONNECTED, EVENT_CONNECTING, EVENT_DISCONNECTED,
    EVENT_ENTER_FULLSCREEN, EVENT_FATAL_ERROR, EVENT_FOCUS_RELEASED, EVENT_LEAVE_FULLSCREEN,
    EVENT_LOGIN_COMPLETE, EVENT_LOGON_ERROR, EVENT_NETWORK_STATUS_CHANGED, EVENT_RECONNECTED,
    EVENT_RECONNECTING, EVENT_REMOTE_DESKTOP_SIZE_CHANGED, EVENT_WARNING, MAX_EVENT_PAYLOAD_BYTES,
    NavopRdpEvent,
};

/// An owned event copied from the native callback boundary.
///
/// `kind`, `code`, and `payload` are intentionally opaque. Keeping unknown
/// values intact lets a newer native event producer interoperate with an older
/// Rust facade without losing diagnostics.
#[derive(Clone, PartialEq, Eq)]
pub struct WindowsRdpRawEvent {
    pub generation: u64,
    pub kind: u32,
    pub code: i32,
    pub payload: Vec<u8>,
}

impl fmt::Debug for WindowsRdpRawEvent {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("WindowsRdpRawEvent")
            .field("generation", &self.generation)
            .field("kind", &self.kind)
            .field("code", &self.code)
            .field("payload_len", &self.payload.len())
            .finish()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct OwnedNativeEvent {
    pub(crate) generation: u64,
    pub(crate) kind: u32,
    pub(crate) code: i32,
    pub(crate) payload: Vec<u8>,
}

#[derive(Clone, Copy)]
#[repr(u8)]
enum CallbackLifecycle {
    Open = 0,
    Closing = 1,
    Closed = 2,
}

pub(crate) struct EventBridge {
    generation: u64,
    lifecycle: AtomicU8,
    queue: Mutex<VecDeque<OwnedNativeEvent>>,
}

impl From<OwnedNativeEvent> for WindowsRdpRawEvent {
    fn from(event: OwnedNativeEvent) -> Self {
        Self {
            generation: event.generation,
            kind: event.kind,
            code: event.code,
            payload: event.payload,
        }
    }
}

/// Stable semantic event shape for the ActiveX event sink.
///
/// Native DISPID decoding stays on the native side. This layer decodes the
/// architecture-independent byte protocol documented by `NavopRdpEvent`; an
/// unknown kind or malformed known payload remains available as `Unknown`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WindowsRdpEvent {
    HostReady {
        generation: u64,
        capabilities: crate::capabilities::WindowsRdpHostCapabilities,
    },
    Connecting {
        generation: u64,
    },
    Connected {
        generation: u64,
    },
    LoginComplete {
        generation: u64,
    },
    Reconnecting {
        generation: u64,
        attempt: u32,
        max_attempts: Option<u32>,
    },
    Reconnected {
        generation: u64,
    },
    NetworkStatusChanged {
        generation: u64,
        quality: Option<u32>,
    },
    RemoteDesktopSizeChanged {
        generation: u64,
        width: u32,
        height: u32,
    },
    FullscreenChanged {
        generation: u64,
        fullscreen: bool,
    },
    AuthenticationWarning {
        generation: u64,
        visible: bool,
    },
    Warning {
        generation: u64,
        code: i32,
    },
    FatalError {
        generation: u64,
        code: i32,
    },
    LogonError {
        generation: u64,
        code: i32,
    },
    Disconnected {
        generation: u64,
        reason: crate::error::WindowsRdpDisconnectReason,
    },
    CloseConfirmed {
        generation: u64,
    },
    FocusReleased {
        generation: u64,
    },
    Unknown {
        event: WindowsRdpRawEvent,
    },
}

impl From<WindowsRdpRawEvent> for WindowsRdpEvent {
    fn from(event: WindowsRdpRawEvent) -> Self {
        let generation = event.generation;
        let decoded = match event.kind {
            EVENT_CONNECTING if has_no_code_or_payload(&event) => {
                Some(Self::Connecting { generation })
            }
            EVENT_CONNECTED if has_no_code_or_payload(&event) => {
                Some(Self::Connected { generation })
            }
            EVENT_LOGIN_COMPLETE if has_no_code_or_payload(&event) => {
                Some(Self::LoginComplete { generation })
            }
            EVENT_RECONNECTING if event.code == 0 => {
                decode_reconnecting(generation, &event.payload)
            }
            EVENT_RECONNECTED if has_no_code_or_payload(&event) => {
                Some(Self::Reconnected { generation })
            }
            EVENT_NETWORK_STATUS_CHANGED if event.code == 0 => decode_optional_u32(&event.payload)
                .map(|quality| Self::NetworkStatusChanged {
                    generation,
                    quality,
                }),
            EVENT_REMOTE_DESKTOP_SIZE_CHANGED if event.code == 0 => decode_u32_pair(&event.payload)
                .map(|(width, height)| Self::RemoteDesktopSizeChanged {
                    generation,
                    width,
                    height,
                }),
            EVENT_ENTER_FULLSCREEN if has_no_code_or_payload(&event) => {
                Some(Self::FullscreenChanged {
                    generation,
                    fullscreen: true,
                })
            }
            EVENT_LEAVE_FULLSCREEN if has_no_code_or_payload(&event) => {
                Some(Self::FullscreenChanged {
                    generation,
                    fullscreen: false,
                })
            }
            EVENT_AUTHENTICATION_WARNING_DISPLAYED if has_no_code_or_payload(&event) => {
                Some(Self::AuthenticationWarning {
                    generation,
                    visible: true,
                })
            }
            EVENT_AUTHENTICATION_WARNING_DISMISSED if has_no_code_or_payload(&event) => {
                Some(Self::AuthenticationWarning {
                    generation,
                    visible: false,
                })
            }
            EVENT_WARNING if event.payload.is_empty() => Some(Self::Warning {
                generation,
                code: event.code,
            }),
            EVENT_FATAL_ERROR if event.payload.is_empty() => Some(Self::FatalError {
                generation,
                code: event.code,
            }),
            EVENT_LOGON_ERROR if event.payload.is_empty() => Some(Self::LogonError {
                generation,
                code: event.code,
            }),
            EVENT_DISCONNECTED => {
                decode_optional_i32(&event.payload).map(|extended_code| Self::Disconnected {
                    generation,
                    reason: crate::error::WindowsRdpDisconnectReason::unknown(
                        event.code,
                        extended_code,
                    ),
                })
            }
            EVENT_CLOSE_CONFIRMED if has_no_code_or_payload(&event) => {
                Some(Self::CloseConfirmed { generation })
            }
            EVENT_FOCUS_RELEASED if has_no_code_or_payload(&event) => {
                Some(Self::FocusReleased { generation })
            }
            _ => None,
        };

        decoded.unwrap_or(Self::Unknown { event })
    }
}

fn has_no_code_or_payload(event: &WindowsRdpRawEvent) -> bool {
    event.code == 0 && event.payload.is_empty()
}

fn decode_reconnecting(generation: u64, payload: &[u8]) -> Option<WindowsRdpEvent> {
    match payload.len() {
        4 => Some(WindowsRdpEvent::Reconnecting {
            generation,
            attempt: decode_u32(payload)?,
            max_attempts: None,
        }),
        8 => {
            let (attempt, max_attempts) = decode_u32_pair(payload)?;
            Some(WindowsRdpEvent::Reconnecting {
                generation,
                attempt,
                max_attempts: Some(max_attempts),
            })
        }
        _ => None,
    }
}

fn decode_optional_u32(payload: &[u8]) -> Option<Option<u32>> {
    match payload.len() {
        0 => Some(None),
        4 => Some(Some(decode_u32(payload)?)),
        _ => None,
    }
}

fn decode_optional_i32(payload: &[u8]) -> Option<Option<i32>> {
    match payload.len() {
        0 => Some(None),
        4 => Some(Some(i32::from_le_bytes(payload.try_into().ok()?))),
        _ => None,
    }
}

fn decode_u32(payload: &[u8]) -> Option<u32> {
    Some(u32::from_le_bytes(payload.try_into().ok()?))
}

fn decode_u32_pair(payload: &[u8]) -> Option<(u32, u32)> {
    if payload.len() != 8 {
        return None;
    }

    let first = u32::from_le_bytes(payload[..4].try_into().ok()?);
    let second = u32::from_le_bytes(payload[4..].try_into().ok()?);
    Some((first, second))
}

impl EventBridge {
    pub(crate) fn new(generation: u64) -> Self {
        Self {
            generation,
            lifecycle: AtomicU8::new(CallbackLifecycle::Open as u8),
            queue: Mutex::new(VecDeque::new()),
        }
    }

    pub(crate) fn begin_closing(&self) {
        let mut queue = self.lock_queue();
        self.lifecycle
            .store(CallbackLifecycle::Closing as u8, Ordering::Release);
        queue.clear();
    }

    pub(crate) fn mark_closed(&self) {
        let mut queue = self.lock_queue();
        self.lifecycle
            .store(CallbackLifecycle::Closed as u8, Ordering::Release);
        queue.clear();
    }

    pub(crate) fn drain(&self) -> Vec<WindowsRdpRawEvent> {
        let mut queue = self.lock_queue();
        if self.lifecycle.load(Ordering::Acquire) != CallbackLifecycle::Open as u8 {
            queue.clear();
            return Vec::new();
        }
        queue.drain(..).map(WindowsRdpRawEvent::from).collect()
    }

    fn enqueue(&self, event_generation: u64, kind: u32, code: i32, payload: &[u8]) {
        if event_generation != self.generation {
            return;
        }

        let owned_event = OwnedNativeEvent {
            generation: event_generation,
            kind,
            code,
            payload: payload.to_vec(),
        };
        let mut queue = self.lock_queue();
        if self.lifecycle.load(Ordering::Acquire) != CallbackLifecycle::Open as u8 {
            return;
        }
        queue.push_back(owned_event);
    }

    fn lock_queue(&self) -> MutexGuard<'_, VecDeque<OwnedNativeEvent>> {
        self.queue
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

pub(crate) unsafe extern "C" fn native_event_callback(
    context: *mut c_void,
    event: *const NavopRdpEvent,
    payload: *const u8,
) {
    let _ = catch_unwind(AssertUnwindSafe(|| {
        if context.is_null() || event.is_null() {
            return;
        }

        // SAFETY: Native callbacks must provide a readable event prefix. Fields
        // are read individually so no field beyond struct_size is accessed.
        let struct_size = unsafe { std::ptr::addr_of!((*event).struct_size).read() };
        if struct_size < size_of::<NavopRdpEvent>() as u32 {
            return;
        }

        // SAFETY: The validated struct_size includes every field in the current
        // NavopRdpEvent layout.
        let abi_version = unsafe { std::ptr::addr_of!((*event).abi_version).read() };
        if abi_version != ABI_VERSION {
            return;
        }
        // SAFETY: The current layout was validated above.
        let reserved = unsafe { std::ptr::addr_of!((*event).reserved).read() };
        if reserved != 0 {
            return;
        }
        // SAFETY: The current layout was validated above.
        let generation_low = unsafe { std::ptr::addr_of!((*event).generation_low).read() };
        // SAFETY: The current layout was validated above.
        let generation_high = unsafe { std::ptr::addr_of!((*event).generation_high).read() };
        let event_generation = u64::from(generation_low) | (u64::from(generation_high) << 32);
        // SAFETY: The current layout was validated above.
        let kind = unsafe { std::ptr::addr_of!((*event).kind).read() };
        // SAFETY: The current layout was validated above.
        let code = unsafe { std::ptr::addr_of!((*event).code).read() };
        // SAFETY: The current layout was validated above.
        let payload_len = unsafe { std::ptr::addr_of!((*event).payload_len).read() };
        if payload_len > MAX_EVENT_PAYLOAD_BYTES {
            return;
        }
        let payload_len = payload_len as usize;

        let payload = if payload_len == 0 {
            &[]
        } else {
            if payload.is_null() {
                return;
            }
            // SAFETY: The callback ABI requires payload to point to payload_len
            // readable bytes for the duration of this callback.
            unsafe { slice::from_raw_parts(payload, payload_len) }
        };

        // SAFETY: WindowsRdpHost registers a stable Box<EventBridge> address and
        // keeps it alive until native callback unregistration succeeds.
        let bridge = unsafe { &*context.cast::<EventBridge>() };
        bridge.enqueue(event_generation, kind, code, payload);
    }));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn raw(kind: u32, code: i32, payload: impl Into<Vec<u8>>) -> WindowsRdpRawEvent {
        WindowsRdpRawEvent {
            generation: 42,
            kind,
            code,
            payload: payload.into(),
        }
    }

    #[test]
    fn canonical_wire_events_decode_to_stable_semantics() {
        let cases = [
            (
                raw(EVENT_CONNECTING, 0, []),
                WindowsRdpEvent::Connecting { generation: 42 },
            ),
            (
                raw(EVENT_CONNECTED, 0, []),
                WindowsRdpEvent::Connected { generation: 42 },
            ),
            (
                raw(EVENT_LOGIN_COMPLETE, 0, []),
                WindowsRdpEvent::LoginComplete { generation: 42 },
            ),
            (
                raw(EVENT_RECONNECTING, 0, 3_u32.to_le_bytes()),
                WindowsRdpEvent::Reconnecting {
                    generation: 42,
                    attempt: 3,
                    max_attempts: None,
                },
            ),
            (
                raw(
                    EVENT_RECONNECTING,
                    0,
                    [3_u32.to_le_bytes(), 10_u32.to_le_bytes()].concat(),
                ),
                WindowsRdpEvent::Reconnecting {
                    generation: 42,
                    attempt: 3,
                    max_attempts: Some(10),
                },
            ),
            (
                raw(EVENT_RECONNECTED, 0, []),
                WindowsRdpEvent::Reconnected { generation: 42 },
            ),
            (
                raw(EVENT_NETWORK_STATUS_CHANGED, 0, []),
                WindowsRdpEvent::NetworkStatusChanged {
                    generation: 42,
                    quality: None,
                },
            ),
            (
                raw(EVENT_NETWORK_STATUS_CHANGED, 0, 87_u32.to_le_bytes()),
                WindowsRdpEvent::NetworkStatusChanged {
                    generation: 42,
                    quality: Some(87),
                },
            ),
            (
                raw(
                    EVENT_REMOTE_DESKTOP_SIZE_CHANGED,
                    0,
                    [1920_u32.to_le_bytes(), 1080_u32.to_le_bytes()].concat(),
                ),
                WindowsRdpEvent::RemoteDesktopSizeChanged {
                    generation: 42,
                    width: 1920,
                    height: 1080,
                },
            ),
            (
                raw(EVENT_ENTER_FULLSCREEN, 0, []),
                WindowsRdpEvent::FullscreenChanged {
                    generation: 42,
                    fullscreen: true,
                },
            ),
            (
                raw(EVENT_LEAVE_FULLSCREEN, 0, []),
                WindowsRdpEvent::FullscreenChanged {
                    generation: 42,
                    fullscreen: false,
                },
            ),
            (
                raw(EVENT_AUTHENTICATION_WARNING_DISPLAYED, 0, []),
                WindowsRdpEvent::AuthenticationWarning {
                    generation: 42,
                    visible: true,
                },
            ),
            (
                raw(EVENT_AUTHENTICATION_WARNING_DISMISSED, 0, []),
                WindowsRdpEvent::AuthenticationWarning {
                    generation: 42,
                    visible: false,
                },
            ),
            (
                raw(EVENT_WARNING, -7, []),
                WindowsRdpEvent::Warning {
                    generation: 42,
                    code: -7,
                },
            ),
            (
                raw(EVENT_FATAL_ERROR, i32::MIN, []),
                WindowsRdpEvent::FatalError {
                    generation: 42,
                    code: i32::MIN,
                },
            ),
            (
                raw(EVENT_LOGON_ERROR, i32::MAX, []),
                WindowsRdpEvent::LogonError {
                    generation: 42,
                    code: i32::MAX,
                },
            ),
            (
                raw(EVENT_DISCONNECTED, 2308, []),
                WindowsRdpEvent::Disconnected {
                    generation: 42,
                    reason: crate::error::WindowsRdpDisconnectReason::unknown(2308, None),
                },
            ),
            (
                raw(EVENT_DISCONNECTED, 2308, (-55_i32).to_le_bytes()),
                WindowsRdpEvent::Disconnected {
                    generation: 42,
                    reason: crate::error::WindowsRdpDisconnectReason::unknown(2308, Some(-55)),
                },
            ),
            (
                raw(EVENT_CLOSE_CONFIRMED, 0, []),
                WindowsRdpEvent::CloseConfirmed { generation: 42 },
            ),
            (
                raw(EVENT_FOCUS_RELEASED, 0, []),
                WindowsRdpEvent::FocusReleased { generation: 42 },
            ),
        ];

        for (raw, expected) in cases {
            assert_eq!(WindowsRdpEvent::from(raw), expected);
        }
    }

    #[test]
    fn malformed_known_events_remain_complete_raw_events() {
        let malformed = [
            raw(EVENT_CONNECTING, 1, []),
            raw(EVENT_CONNECTED, 0, [0]),
            raw(EVENT_LOGIN_COMPLETE, 1, []),
            raw(EVENT_RECONNECTING, 0, [1, 2, 3]),
            raw(EVENT_RECONNECTING, 0, [0; 9]),
            raw(EVENT_RECONNECTING, 1, 1_u32.to_le_bytes()),
            raw(EVENT_RECONNECTED, 0, [0]),
            raw(EVENT_NETWORK_STATUS_CHANGED, 0, [0; 3]),
            raw(EVENT_NETWORK_STATUS_CHANGED, 0, [0; 5]),
            raw(EVENT_NETWORK_STATUS_CHANGED, 1, []),
            raw(EVENT_REMOTE_DESKTOP_SIZE_CHANGED, 0, [0; 7]),
            raw(EVENT_REMOTE_DESKTOP_SIZE_CHANGED, 0, [0; 9]),
            raw(
                EVENT_REMOTE_DESKTOP_SIZE_CHANGED,
                1,
                [1_u32.to_le_bytes(), 2_u32.to_le_bytes()].concat(),
            ),
            raw(EVENT_ENTER_FULLSCREEN, 0, [0]),
            raw(EVENT_LEAVE_FULLSCREEN, 1, []),
            raw(EVENT_AUTHENTICATION_WARNING_DISPLAYED, 0, [0]),
            raw(EVENT_AUTHENTICATION_WARNING_DISMISSED, 1, []),
            raw(EVENT_WARNING, 9, [0]),
            raw(EVENT_FATAL_ERROR, 9, [0]),
            raw(EVENT_LOGON_ERROR, 9, [0]),
            raw(EVENT_DISCONNECTED, 9, [0; 3]),
            raw(EVENT_DISCONNECTED, 9, [0; 5]),
            raw(EVENT_CLOSE_CONFIRMED, 0, [0]),
            raw(EVENT_FOCUS_RELEASED, 1, []),
        ];

        for event in malformed {
            let expected = event.clone();
            assert_eq!(
                WindowsRdpEvent::from(event),
                WindowsRdpEvent::Unknown { event: expected }
            );
        }
    }

    #[test]
    fn unknown_event_kind_preserves_generation_code_and_payload() {
        let event = raw(u32::MAX, i32::MIN, [1, 2, 3, 4]);
        let expected = event.clone();

        assert_eq!(
            WindowsRdpEvent::from(event),
            WindowsRdpEvent::Unknown { event: expected }
        );
    }

    #[test]
    fn disconnected_preserves_signed_32_bit_code_patterns() {
        let event = raw(EVENT_DISCONNECTED, i32::MIN, i32::MAX.to_le_bytes());

        assert_eq!(
            WindowsRdpEvent::from(event),
            WindowsRdpEvent::Disconnected {
                generation: 42,
                reason: crate::error::WindowsRdpDisconnectReason::unknown(i32::MIN, Some(i32::MAX),),
            }
        );
    }

    #[test]
    fn callback_rejects_payloads_above_the_protocol_limit_before_reading_them() {
        let bridge = EventBridge::new(42);
        let event = NavopRdpEvent::current(42, EVENT_CONNECTED, 0, MAX_EVENT_PAYLOAD_BYTES + 1);

        unsafe {
            native_event_callback(
                (&bridge as *const EventBridge).cast_mut().cast(),
                &event,
                std::ptr::dangling(),
            );
        }

        assert!(bridge.drain().is_empty());
    }

    #[test]
    fn raw_event_debug_redacts_opaque_payload_contents() {
        let event = WindowsRdpRawEvent {
            generation: 42,
            kind: 7,
            code: -9,
            payload: b"opaque-payload-sentinel".to_vec(),
        };

        let debug = format!("{event:?}");

        assert!(debug.contains("generation: 42"));
        assert!(debug.contains("kind: 7"));
        assert!(debug.contains("code: -9"));
        assert!(debug.contains("payload_len: 23"));
        assert!(!debug.contains("opaque-payload-sentinel"));
        assert_eq!(event.payload, b"opaque-payload-sentinel");
    }

    #[test]
    fn drain_rejects_events_once_closing_is_observable() {
        let bridge = EventBridge::new(42);
        bridge.enqueue(42, 7, -9, &[1, 2, 3]);

        // Model the close/drain race window directly: Closing is observable,
        // while the thread performing close has not yet cleared the queue.
        bridge
            .lifecycle
            .store(CallbackLifecycle::Closing as u8, Ordering::Release);

        assert!(bridge.drain().is_empty());
        assert!(bridge.lock_queue().is_empty());
    }
}
