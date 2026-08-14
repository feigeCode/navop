use std::ffi::c_void;
use std::marker::PhantomData;
use std::rc::Rc;

mod diagnostics;
mod ffi;
mod lifecycle;
mod window;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct WindowsNativeOverlayBounds {
    pub(crate) x: i32,
    pub(crate) y: i32,
    pub(crate) width: i32,
    pub(crate) height: i32,
}

impl WindowsNativeOverlayBounds {
    fn right(self) -> i32 {
        self.x.saturating_add(self.width)
    }

    fn bottom(self) -> i32 {
        self.y.saturating_add(self.height)
    }
}

#[derive(Debug)]
pub(crate) struct WindowsNativeOverlayError {
    stage: &'static str,
    detail: String,
}

impl WindowsNativeOverlayError {
    fn new(stage: &'static str, detail: String) -> Self {
        Self { stage, detail }
    }
}

impl std::fmt::Display for WindowsNativeOverlayError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "{}: {}", self.stage, self.detail)
    }
}

impl std::error::Error for WindowsNativeOverlayError {}

pub(crate) struct WindowsNativeOverlay {
    owner: usize,
    window: usize,
    owner_thread: u32,
    generation: u64,
    last_bounds: Option<WindowsNativeOverlayBounds>,
    requested_visible: bool,
    _thread_affinity: PhantomData<Rc<()>>,
}

pub(super) fn window_pointer(window: usize) -> *mut c_void {
    window as *mut c_void
}
