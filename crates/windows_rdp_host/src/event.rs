use std::collections::VecDeque;
use std::ffi::c_void;
use std::fmt;
use std::mem::size_of;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::slice;
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::{Mutex, MutexGuard};

use crate::ffi::{ABI_VERSION, NavopRdpEvent};

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

/// Stable semantic event shape for the future ActiveX event sink.
///
/// Native DISPID and payload decoding are deliberately not inferred here. A
/// native event that is not yet understood remains available as `Unknown`.
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
        Self::Unknown { event }
    }
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
        let payload_len = unsafe { std::ptr::addr_of!((*event).payload_len).read() } as usize;

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
