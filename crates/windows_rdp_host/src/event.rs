use std::collections::VecDeque;
use std::ffi::c_void;
use std::mem::size_of;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::slice;
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::{Mutex, MutexGuard};

use crate::ffi::{ABI_VERSION, NavopRdpEvent};

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

impl EventBridge {
    pub(crate) fn new(generation: u64) -> Self {
        Self {
            generation,
            lifecycle: AtomicU8::new(CallbackLifecycle::Open as u8),
            queue: Mutex::new(VecDeque::new()),
        }
    }

    pub(crate) fn begin_closing(&self) {
        self.lifecycle
            .store(CallbackLifecycle::Closing as u8, Ordering::Release);
        self.lock_queue().clear();
    }

    pub(crate) fn mark_closed(&self) {
        self.lifecycle
            .store(CallbackLifecycle::Closed as u8, Ordering::Release);
        self.lock_queue().clear();
    }

    #[cfg(test)]
    pub(crate) fn drain(&self) -> Vec<OwnedNativeEvent> {
        self.lock_queue().drain(..).collect()
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
