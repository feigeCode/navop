use std::ffi::c_void;
use std::marker::PhantomData;
use std::mem::{align_of, size_of};

pub(crate) const ABI_VERSION: u32 = 1;
pub(crate) const CREATE_WITH_PARENT_ABI_VERSION: u32 = 1;
pub(crate) const SESSION_DISPLAY_SETTINGS_ABI_VERSION: u32 = 1;
pub(crate) const CONNECTION_FLAG_AUDIO_PLAYBACK_DISABLED: u32 = 1 << 0;
pub(crate) const CONNECTION_FLAGS_KNOWN: u32 = CONNECTION_FLAG_AUDIO_PLAYBACK_DISABLED;

pub(crate) const DISPLAY_FLAG_SMART_SIZING: u32 = 1 << 0;
pub(crate) const DISPLAY_FLAG_USE_MULTIMON: u32 = 1 << 1;
pub(crate) const DISPLAY_FLAG_SPAN_MONITORS: u32 = 1 << 2;
pub(crate) const DISPLAY_FLAGS_KNOWN: u32 =
    DISPLAY_FLAG_SMART_SIZING | DISPLAY_FLAG_USE_MULTIMON | DISPLAY_FLAG_SPAN_MONITORS;

pub(crate) const RESOURCE_FLAG_CLIPBOARD: u32 = 1 << 0;
pub(crate) const RESOURCE_FLAG_DRIVES: u32 = 1 << 1;
pub(crate) const RESOURCE_FLAG_DYNAMIC_DRIVES: u32 = 1 << 2;
pub(crate) const RESOURCE_FLAG_DYNAMIC_DEVICES: u32 = 1 << 3;
pub(crate) const RESOURCE_FLAG_PRINTERS: u32 = 1 << 4;
pub(crate) const RESOURCE_FLAG_SERIAL_PORTS: u32 = 1 << 5;
pub(crate) const RESOURCE_FLAG_SMART_CARDS: u32 = 1 << 6;
pub(crate) const RESOURCE_FLAG_CAMERAS: u32 = 1 << 7;
pub(crate) const RESOURCE_FLAG_MICROPHONES: u32 = 1 << 8;
pub(crate) const RESOURCE_FLAG_POS_DEVICES: u32 = 1 << 9;
pub(crate) const RESOURCE_FLAGS_KNOWN: u32 = RESOURCE_FLAG_CLIPBOARD
    | RESOURCE_FLAG_DRIVES
    | RESOURCE_FLAG_DYNAMIC_DRIVES
    | RESOURCE_FLAG_DYNAMIC_DEVICES
    | RESOURCE_FLAG_PRINTERS
    | RESOURCE_FLAG_SERIAL_PORTS
    | RESOURCE_FLAG_SMART_CARDS
    | RESOURCE_FLAG_CAMERAS
    | RESOURCE_FLAG_MICROPHONES
    | RESOURCE_FLAG_POS_DEVICES;

pub(crate) const AUDIO_FLAG_CAPTURE: u32 = 1 << 0;
pub(crate) const AUDIO_FLAGS_KNOWN: u32 = AUDIO_FLAG_CAPTURE;

pub(crate) const INPUT_FLAG_ENABLE_WINDOWS_KEY: u32 = 1 << 0;
pub(crate) const INPUT_FLAG_GRAB_FOCUS_ON_CONNECT: u32 = 1 << 1;
pub(crate) const INPUT_FLAGS_KNOWN: u32 =
    INPUT_FLAG_ENABLE_WINDOWS_KEY | INPUT_FLAG_GRAB_FOCUS_ON_CONNECT;

pub(crate) const PERFORMANCE_FLAG_WALLPAPER: u32 = 1 << 0;
pub(crate) const PERFORMANCE_FLAG_FULL_WINDOW_DRAG: u32 = 1 << 1;
pub(crate) const PERFORMANCE_FLAG_MENU_ANIMATIONS: u32 = 1 << 2;
pub(crate) const PERFORMANCE_FLAG_THEMES: u32 = 1 << 3;
pub(crate) const PERFORMANCE_FLAG_CURSOR_SHADOW: u32 = 1 << 4;
pub(crate) const PERFORMANCE_FLAG_CURSOR_SETTINGS: u32 = 1 << 5;
pub(crate) const PERFORMANCE_FLAG_FONT_SMOOTHING: u32 = 1 << 6;
pub(crate) const PERFORMANCE_FLAG_DESKTOP_COMPOSITION: u32 = 1 << 7;
pub(crate) const PERFORMANCE_FLAG_BITMAP_CACHE: u32 = 1 << 8;
pub(crate) const PERFORMANCE_FLAGS_KNOWN: u32 = PERFORMANCE_FLAG_WALLPAPER
    | PERFORMANCE_FLAG_FULL_WINDOW_DRAG
    | PERFORMANCE_FLAG_MENU_ANIMATIONS
    | PERFORMANCE_FLAG_THEMES
    | PERFORMANCE_FLAG_CURSOR_SHADOW
    | PERFORMANCE_FLAG_CURSOR_SETTINGS
    | PERFORMANCE_FLAG_FONT_SMOOTHING
    | PERFORMANCE_FLAG_DESKTOP_COMPOSITION
    | PERFORMANCE_FLAG_BITMAP_CACHE;

pub(crate) const SECURITY_FLAG_ENABLE_CREDSSP: u32 = 1 << 0;
pub(crate) const SECURITY_FLAG_PUBLIC_MODE: u32 = 1 << 1;
pub(crate) const SECURITY_FLAG_ENCRYPTION_ENABLED: u32 = 1 << 2;
pub(crate) const SECURITY_FLAGS_KNOWN: u32 =
    SECURITY_FLAG_ENABLE_CREDSSP | SECURITY_FLAG_PUBLIC_MODE | SECURITY_FLAG_ENCRYPTION_ENABLED;

pub(crate) const GATEWAY_FLAG_BYPASS_LOCAL: u32 = 1 << 0;
pub(crate) const GATEWAY_FLAGS_KNOWN: u32 = GATEWAY_FLAG_BYPASS_LOCAL;

pub(crate) const CONNECTION_FLAG_ADMIN_SESSION: u32 = 1 << 0;
pub(crate) const CONNECTION_FLAG_AUTO_RECONNECT: u32 = 1 << 1;
pub(crate) const CONNECTION_POLICY_FLAGS_KNOWN: u32 =
    CONNECTION_FLAG_ADMIN_SESSION | CONNECTION_FLAG_AUTO_RECONNECT;

pub(crate) type NativeResult = i32;

pub(crate) const RESULT_OK: NativeResult = 0;
pub(crate) const RESULT_INVALID_ARGUMENT: NativeResult = 1;
pub(crate) const RESULT_ABI_MISMATCH: NativeResult = 2;
pub(crate) const RESULT_ALLOCATION_FAILED: NativeResult = 3;
pub(crate) const RESULT_INTERNAL_ERROR: NativeResult = 4;
pub(crate) const RESULT_UNAVAILABLE: NativeResult = 5;
pub(crate) const RESULT_WRONG_THREAD: NativeResult = 6;
pub(crate) const RESULT_CALLBACK_IN_FLIGHT: NativeResult = 7;
pub(crate) const RESULT_INVALID_STATE: NativeResult = 8;
pub(crate) const CREDENTIAL_LEGACY_SIZE: u32 = if cfg!(target_pointer_width = "64") {
    48
} else {
    28
};
pub(crate) const CONNECTION_LEGACY_SIZE: u32 = if cfg!(target_pointer_width = "64") {
    48
} else {
    36
};
#[allow(dead_code)]
pub(crate) const LAST_ERROR_LEGACY_SIZE: u32 = 24;

#[allow(dead_code)]
pub(crate) const CREATE_STAGE_NONE: u32 = 0;
#[allow(dead_code)]
pub(crate) const CREATE_STAGE_OLE_INITIALIZE: u32 = 1;
#[allow(dead_code)]
pub(crate) const CREATE_STAGE_ATL_AX_WIN_INIT: u32 = 2;
#[allow(dead_code)]
pub(crate) const CREATE_STAGE_CREATE_WINDOW: u32 = 3;
#[allow(dead_code)]
pub(crate) const CREATE_STAGE_CREATE_CONTROL: u32 = 4;
#[allow(dead_code)]
pub(crate) const CREATE_STAGE_QUERY_CLIENT: u32 = 5;
#[allow(dead_code)]
pub(crate) const CREATE_STAGE_QUERY_NON_SCRIPTABLE: u32 = 6;
#[allow(dead_code)]
pub(crate) const CREATE_STAGE_SET_PARENT: u32 = 7;
#[allow(dead_code)]
pub(crate) const CREATE_STAGE_EVENT_SUBSCRIPTION: u32 = 8;
#[allow(dead_code)]
pub(crate) const CREATE_STAGE_EXCEPTION: u32 = 9;

pub(crate) const CONNECTION_STATE_DISCONNECTED: u32 = 0;
pub(crate) const CONNECTION_STATE_CONNECTED: u32 = 1;
pub(crate) const CONNECTION_STATE_CONNECTING: u32 = 2;
pub(crate) const REQUEST_CLOSE_CAN_PROCEED: u32 = 0;
pub(crate) const REQUEST_CLOSE_WAIT_FOR_EVENTS: u32 = 1;

pub(crate) const EVENT_CONNECTING: u32 = 1;
pub(crate) const EVENT_CONNECTED: u32 = 2;
pub(crate) const EVENT_LOGIN_COMPLETE: u32 = 3;
pub(crate) const EVENT_RECONNECTING: u32 = 4;
pub(crate) const EVENT_RECONNECTED: u32 = 5;
pub(crate) const EVENT_NETWORK_STATUS_CHANGED: u32 = 6;
pub(crate) const EVENT_REMOTE_DESKTOP_SIZE_CHANGED: u32 = 7;
pub(crate) const EVENT_ENTER_FULLSCREEN: u32 = 8;
pub(crate) const EVENT_LEAVE_FULLSCREEN: u32 = 9;
pub(crate) const EVENT_AUTHENTICATION_WARNING_DISPLAYED: u32 = 10;
pub(crate) const EVENT_AUTHENTICATION_WARNING_DISMISSED: u32 = 11;
pub(crate) const EVENT_WARNING: u32 = 12;
pub(crate) const EVENT_FATAL_ERROR: u32 = 13;
pub(crate) const EVENT_LOGON_ERROR: u32 = 14;
pub(crate) const EVENT_DISCONNECTED: u32 = 15;
pub(crate) const EVENT_CLOSE_CONFIRMED: u32 = 16;
pub(crate) const EVENT_FOCUS_RELEASED: u32 = 17;
pub(crate) const MAX_EVENT_PAYLOAD_BYTES: u32 = 65_536;

#[repr(C)]
pub(crate) struct NativeRdpHost {
    _private: [u8; 0],
    _marker: PhantomData<(*mut u8, std::marker::PhantomPinned)>,
}

#[repr(C)]
pub(crate) struct NavopRdpProbeOptions {
    pub(crate) struct_size: u32,
    pub(crate) abi_version: u32,
}

impl NavopRdpProbeOptions {
    pub(crate) fn current() -> Self {
        Self {
            struct_size: size_of::<Self>() as u32,
            abi_version: ABI_VERSION,
        }
    }
}

#[repr(C)]
pub(crate) struct NavopRdpProbeResult {
    pub(crate) struct_size: u32,
    pub(crate) abi_version: u32,
    pub(crate) available: u32,
    pub(crate) reserved: u32,
}

impl NavopRdpProbeResult {
    pub(crate) fn current() -> Self {
        Self {
            struct_size: size_of::<Self>() as u32,
            abi_version: ABI_VERSION,
            available: 0,
            reserved: 0,
        }
    }

    pub(crate) fn has_current_layout(&self) -> bool {
        self.struct_size == size_of::<Self>() as u32
            && self.abi_version == ABI_VERSION
            && self.reserved == 0
    }
}

#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct NavopRdpLastError {
    pub(crate) struct_size: u32,
    pub(crate) abi_version: u32,
    pub(crate) result: NativeResult,
    pub(crate) hresult: i32,
    pub(crate) has_hresult: u32,
    pub(crate) reserved: u32,
    pub(crate) stage: u32,
    pub(crate) win32_code: u32,
    pub(crate) has_win32_code: u32,
}

impl NavopRdpLastError {
    pub(crate) fn current() -> Self {
        Self {
            struct_size: size_of::<Self>() as u32,
            abi_version: ABI_VERSION,
            result: RESULT_OK,
            hresult: 0,
            has_hresult: 0,
            reserved: 0,
            stage: CREATE_STAGE_NONE,
            win32_code: 0,
            has_win32_code: 0,
        }
    }

    pub(crate) fn has_current_layout(&self) -> bool {
        self.struct_size >= size_of::<Self>() as u32
            && self.abi_version == ABI_VERSION
            && self.has_hresult <= 1
            && self.has_win32_code <= 1
            && self.reserved == 0
    }
}

#[repr(C)]
pub(crate) struct NavopRdpCreateOptions {
    pub(crate) struct_size: u32,
    pub(crate) abi_version: u32,
    pub(crate) generation_low: u32,
    pub(crate) generation_high: u32,
}

impl NavopRdpCreateOptions {
    pub(crate) fn current(generation: u64) -> Self {
        Self {
            struct_size: size_of::<Self>() as u32,
            abi_version: ABI_VERSION,
            generation_low: generation as u32,
            generation_high: (generation >> 32) as u32,
        }
    }
}

#[repr(C)]
pub(crate) struct NavopRdpCreateWithParentOptions {
    pub(crate) struct_size: u32,
    pub(crate) abi_version: u32,
    pub(crate) generation_low: u32,
    pub(crate) generation_high: u32,
    pub(crate) parent_hwnd: usize,
}

impl NavopRdpCreateWithParentOptions {
    pub(crate) fn current(generation: u64, parent_hwnd: usize) -> Self {
        Self {
            struct_size: size_of::<Self>() as u32,
            abi_version: CREATE_WITH_PARENT_ABI_VERSION,
            generation_low: generation as u32,
            generation_high: (generation >> 32) as u32,
            parent_hwnd,
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct NavopRdpBounds {
    pub(crate) x: i32,
    pub(crate) y: i32,
    pub(crate) width: i32,
    pub(crate) height: i32,
}

impl NavopRdpBounds {
    pub(crate) const fn new(x: i32, y: i32, width: i32, height: i32) -> Self {
        Self {
            x,
            y,
            width,
            height,
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct NavopRdpSessionDisplaySettings {
    pub(crate) struct_size: u32,
    pub(crate) abi_version: u32,
    pub(crate) desktop_width: u32,
    pub(crate) desktop_height: u32,
    pub(crate) physical_width: u32,
    pub(crate) physical_height: u32,
    pub(crate) orientation: u32,
    pub(crate) desktop_scale_factor: u32,
    pub(crate) device_scale_factor: u32,
}

impl NavopRdpSessionDisplaySettings {
    pub(crate) const fn current(
        desktop_width: u32,
        desktop_height: u32,
        physical_width: u32,
        physical_height: u32,
        orientation: u32,
        desktop_scale_factor: u32,
        device_scale_factor: u32,
    ) -> Self {
        Self {
            struct_size: size_of::<Self>() as u32,
            abi_version: SESSION_DISPLAY_SETTINGS_ABI_VERSION,
            desktop_width,
            desktop_height,
            physical_width,
            physical_height,
            orientation,
            desktop_scale_factor,
            device_scale_factor,
        }
    }
}

#[repr(C)]
pub(crate) struct NavopRdpEvent {
    pub(crate) struct_size: u32,
    pub(crate) abi_version: u32,
    pub(crate) kind: u32,
    pub(crate) reserved: u32,
    pub(crate) generation_low: u32,
    pub(crate) generation_high: u32,
    pub(crate) code: i32,
    pub(crate) payload_len: u32,
}

impl NavopRdpEvent {
    #[cfg(test)]
    pub(crate) fn current(generation: u64, kind: u32, code: i32, payload_len: u32) -> Self {
        Self {
            struct_size: size_of::<Self>() as u32,
            abi_version: ABI_VERSION,
            kind,
            reserved: 0,
            generation_low: generation as u32,
            generation_high: (generation >> 32) as u32,
            code,
            payload_len,
        }
    }
}

#[repr(C)]
pub(crate) struct NavopRdpEventCallbackOptions {
    pub(crate) struct_size: u32,
    pub(crate) abi_version: u32,
    pub(crate) generation_low: u32,
    pub(crate) generation_high: u32,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub(crate) struct NavopRdpBorrowedUtf16 {
    pub(crate) data: *const u16,
    pub(crate) len: u32,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub(crate) struct NavopRdpBorrowedSecret {
    pub(crate) data: *const u16,
    pub(crate) len: u32,
}

#[repr(C)]
pub(crate) struct NavopRdpCredentialBundle {
    pub(crate) struct_size: u32,
    pub(crate) abi_version: u32,
    pub(crate) server_password: NavopRdpBorrowedSecret,
    pub(crate) gateway_password: NavopRdpBorrowedSecret,
    pub(crate) flags: u32,
    pub(crate) username: NavopRdpBorrowedUtf16,
    pub(crate) domain: NavopRdpBorrowedUtf16,
    pub(crate) gateway_username: NavopRdpBorrowedUtf16,
    pub(crate) gateway_domain: NavopRdpBorrowedUtf16,
}

#[repr(C)]
pub(crate) struct NavopRdpConnectionOptions {
    pub(crate) struct_size: u32,
    pub(crate) abi_version: u32,
    pub(crate) host: NavopRdpBorrowedUtf16,
    pub(crate) port: u32,
    pub(crate) desktop_width: i32,
    pub(crate) desktop_height: i32,
    pub(crate) color_depth: i32,
    pub(crate) flags: u32,
    pub(crate) legacy_reserved: u32,
    pub(crate) display_mode: u32,
    pub(crate) display_flags: u32,
    pub(crate) desktop_scale_factor: u32,
    pub(crate) device_scale_factor: u32,
    pub(crate) resource_flags: u32,
    pub(crate) audio_mode: u32,
    pub(crate) audio_quality: u32,
    pub(crate) audio_flags: u32,
    pub(crate) keyboard_hook_mode: u32,
    pub(crate) input_flags: u32,
    pub(crate) performance_preset: u32,
    pub(crate) performance_flags: u32,
    pub(crate) network_connection_type: u32,
    pub(crate) security_flags: u32,
    pub(crate) authentication_level: u32,
    pub(crate) gateway_mode: u32,
    pub(crate) gateway_flags: u32,
    pub(crate) gateway_credential_source: u32,
    pub(crate) gateway_hostname: NavopRdpBorrowedUtf16,
    pub(crate) keep_alive_seconds: u32,
    pub(crate) timeout_seconds: u32,
    pub(crate) connection_flags: u32,
    pub(crate) max_reconnect_attempts: u32,
}

impl NavopRdpConnectionOptions {
    pub(crate) fn current(
        host: NavopRdpBorrowedUtf16,
        port: u32,
        desktop_width: i32,
        desktop_height: i32,
        color_depth: i32,
    ) -> Self {
        Self {
            struct_size: size_of::<Self>() as u32,
            abi_version: ABI_VERSION,
            host,
            port,
            desktop_width,
            desktop_height,
            color_depth,
            flags: 0,
            legacy_reserved: 0,
            display_mode: 0,
            display_flags: 0,
            desktop_scale_factor: 100,
            device_scale_factor: 100,
            resource_flags: RESOURCE_FLAG_CLIPBOARD,
            audio_mode: 0,
            audio_quality: 0,
            audio_flags: 0,
            keyboard_hook_mode: 1,
            input_flags: INPUT_FLAGS_KNOWN,
            performance_preset: 0,
            performance_flags: PERFORMANCE_FLAGS_KNOWN,
            network_connection_type: 6,
            security_flags: SECURITY_FLAG_ENABLE_CREDSSP | SECURITY_FLAG_ENCRYPTION_ENABLED,
            authentication_level: 0,
            gateway_mode: 0,
            gateway_flags: GATEWAY_FLAG_BYPASS_LOCAL,
            gateway_credential_source: 0,
            gateway_hostname: NavopRdpBorrowedUtf16 {
                data: std::ptr::null(),
                len: 0,
            },
            keep_alive_seconds: 60,
            timeout_seconds: 600,
            connection_flags: CONNECTION_FLAG_AUTO_RECONNECT,
            max_reconnect_attempts: 20,
        }
    }
}

fn abi_field_available<T>(struct_size: u32, offset: usize) -> bool {
    offset
        .checked_add(size_of::<T>())
        .is_some_and(|end| end <= struct_size as usize)
}

unsafe fn read_abi_field<T: Copy>(
    base: *const c_void,
    struct_size: u32,
    offset: usize,
) -> Option<T> {
    abi_field_available::<T>(struct_size, offset)
        .then(|| unsafe { std::ptr::read_unaligned(base.cast::<u8>().add(offset).cast::<T>()) })
}

fn valid_borrowed_utf16(value: NavopRdpBorrowedUtf16) -> bool {
    value.len == 0 || !value.data.is_null()
}

fn valid_borrowed_secret(value: NavopRdpBorrowedSecret) -> bool {
    value.len == 0 || !value.data.is_null()
}

#[cfg(not(windows_rdp_host_native))]
unsafe fn normalize_connection_options(
    options: *const NavopRdpConnectionOptions,
) -> Result<NavopRdpConnectionOptions, NativeResult> {
    let base = options.cast::<c_void>();
    let struct_size = unsafe { read_abi_field::<u32>(base, size_of::<u32>() as u32, 0) }
        .ok_or(RESULT_INVALID_ARGUMENT)?;
    if struct_size < CONNECTION_LEGACY_SIZE {
        return Err(RESULT_INVALID_ARGUMENT);
    }

    let read_required = |offset| unsafe {
        read_abi_field::<u32>(base, struct_size, offset).ok_or(RESULT_INVALID_ARGUMENT)
    };
    let abi_version = read_required(std::mem::offset_of!(NavopRdpConnectionOptions, abi_version))?;
    if abi_version != ABI_VERSION {
        return Err(RESULT_ABI_MISMATCH);
    }

    let host = unsafe {
        read_abi_field::<NavopRdpBorrowedUtf16>(
            base,
            struct_size,
            std::mem::offset_of!(NavopRdpConnectionOptions, host),
        )
    }
    .ok_or(RESULT_INVALID_ARGUMENT)?;
    let port = read_required(std::mem::offset_of!(NavopRdpConnectionOptions, port))?;
    let desktop_width = unsafe {
        read_abi_field::<i32>(
            base,
            struct_size,
            std::mem::offset_of!(NavopRdpConnectionOptions, desktop_width),
        )
    }
    .ok_or(RESULT_INVALID_ARGUMENT)?;
    let desktop_height = unsafe {
        read_abi_field::<i32>(
            base,
            struct_size,
            std::mem::offset_of!(NavopRdpConnectionOptions, desktop_height),
        )
    }
    .ok_or(RESULT_INVALID_ARGUMENT)?;
    let color_depth = unsafe {
        read_abi_field::<i32>(
            base,
            struct_size,
            std::mem::offset_of!(NavopRdpConnectionOptions, color_depth),
        )
    }
    .ok_or(RESULT_INVALID_ARGUMENT)?;
    let flags = read_required(std::mem::offset_of!(NavopRdpConnectionOptions, flags))?;

    let mut normalized =
        NavopRdpConnectionOptions::current(host, port, desktop_width, desktop_height, color_depth);
    normalized.flags = flags;

    macro_rules! copy_optional_field {
        ($field:ident) => {
            if let Some(value) = unsafe {
                read_abi_field(
                    base,
                    struct_size,
                    std::mem::offset_of!(NavopRdpConnectionOptions, $field),
                )
            } {
                normalized.$field = value;
            }
        };
    }

    copy_optional_field!(display_mode);
    copy_optional_field!(display_flags);
    copy_optional_field!(desktop_scale_factor);
    copy_optional_field!(device_scale_factor);
    copy_optional_field!(resource_flags);
    let audio_mode_available = abi_field_available::<u32>(
        struct_size,
        std::mem::offset_of!(NavopRdpConnectionOptions, audio_mode),
    );
    copy_optional_field!(audio_mode);
    copy_optional_field!(audio_quality);
    copy_optional_field!(audio_flags);
    copy_optional_field!(keyboard_hook_mode);
    copy_optional_field!(input_flags);
    copy_optional_field!(performance_preset);
    copy_optional_field!(performance_flags);
    copy_optional_field!(network_connection_type);
    copy_optional_field!(security_flags);
    copy_optional_field!(authentication_level);
    copy_optional_field!(gateway_mode);
    copy_optional_field!(gateway_flags);
    copy_optional_field!(gateway_credential_source);
    copy_optional_field!(gateway_hostname);
    copy_optional_field!(keep_alive_seconds);
    copy_optional_field!(timeout_seconds);
    copy_optional_field!(connection_flags);
    copy_optional_field!(max_reconnect_attempts);

    if !audio_mode_available && flags & CONNECTION_FLAG_AUDIO_PLAYBACK_DISABLED != 0 {
        normalized.audio_mode = 2;
    }

    Ok(normalized)
}

#[cfg(not(windows_rdp_host_native))]
fn validate_connection_options(options: &NavopRdpConnectionOptions) -> NativeResult {
    if options.flags & !CONNECTION_FLAGS_KNOWN != 0
        || options.display_flags & !DISPLAY_FLAGS_KNOWN != 0
        || options.resource_flags & !RESOURCE_FLAGS_KNOWN != 0
        || options.audio_flags & !AUDIO_FLAGS_KNOWN != 0
        || options.input_flags & !INPUT_FLAGS_KNOWN != 0
        || options.performance_flags & !PERFORMANCE_FLAGS_KNOWN != 0
        || options.security_flags & !SECURITY_FLAGS_KNOWN != 0
        || options.gateway_flags & !GATEWAY_FLAGS_KNOWN != 0
        || options.connection_flags & !CONNECTION_POLICY_FLAGS_KNOWN != 0
    {
        return RESULT_INVALID_ARGUMENT;
    }

    if !matches!(options.display_mode, 0 | 1)
        || !matches!(options.audio_mode, 0 | 1 | 2)
        || !matches!(options.audio_quality, 0 | 1 | 2)
        || !matches!(options.keyboard_hook_mode, 0 | 1 | 2)
        || !(0..=4).contains(&options.performance_preset)
        || !(0..=6).contains(&options.network_connection_type)
        || !(0..=2).contains(&options.authentication_level)
        || !matches!(options.gateway_mode, 0 | 1 | 2)
        || !matches!(options.gateway_credential_source, 0 | 1 | 4)
        || !(100..=500).contains(&options.desktop_scale_factor)
        || !matches!(options.device_scale_factor, 100 | 140 | 180)
        || options.keep_alive_seconds.checked_mul(1_000).is_none()
        || options.timeout_seconds > i32::MAX as u32
        || options.max_reconnect_attempts > i32::MAX as u32
    {
        return RESULT_INVALID_ARGUMENT;
    }

    if options.host.len == 0
        || options.host.len as usize > crate::options::WINDOWS_RDP_MAX_HOST_UTF16_CODE_UNITS
        || options.host.data.is_null()
    {
        return RESULT_INVALID_ARGUMENT;
    }
    let host_slice =
        unsafe { std::slice::from_raw_parts(options.host.data, options.host.len as usize) };
    if host_slice.contains(&0) {
        return RESULT_INVALID_ARGUMENT;
    }

    if !(1..=u32::from(u16::MAX)).contains(&options.port)
        || options.desktop_width <= 0
        || options.desktop_height <= 0
        || !matches!(options.color_depth, 8 | 15 | 16 | 24 | 32)
    {
        return RESULT_INVALID_ARGUMENT;
    }

    let gateway = options.gateway_hostname;
    if !valid_borrowed_utf16(gateway)
        || gateway.len as usize > crate::policy::WINDOWS_RDP_MAX_GATEWAY_HOST_UTF16_CODE_UNITS
        || (options.gateway_mode == 1 && gateway.len == 0)
    {
        return RESULT_INVALID_ARGUMENT;
    }
    if gateway.len > 0 {
        let gateway_slice =
            unsafe { std::slice::from_raw_parts(gateway.data, gateway.len as usize) };
        if gateway_slice.contains(&0) {
            return RESULT_INVALID_ARGUMENT;
        }
    }

    RESULT_OK
}

const _: () = {
    assert!(size_of::<NativeResult>() == 4);

    assert!(size_of::<NavopRdpProbeOptions>() == 8);
    assert!(align_of::<NavopRdpProbeOptions>() == 4);
    assert!(std::mem::offset_of!(NavopRdpProbeOptions, struct_size) == 0);
    assert!(std::mem::offset_of!(NavopRdpProbeOptions, abi_version) == 4);

    assert!(size_of::<NavopRdpProbeResult>() == 16);
    assert!(align_of::<NavopRdpProbeResult>() == 4);
    assert!(std::mem::offset_of!(NavopRdpProbeResult, struct_size) == 0);
    assert!(std::mem::offset_of!(NavopRdpProbeResult, abi_version) == 4);
    assert!(std::mem::offset_of!(NavopRdpProbeResult, available) == 8);
    assert!(std::mem::offset_of!(NavopRdpProbeResult, reserved) == 12);

    assert!(size_of::<NavopRdpLastError>() == 36);
    assert!(align_of::<NavopRdpLastError>() == 4);
    assert!(std::mem::offset_of!(NavopRdpLastError, struct_size) == 0);
    assert!(std::mem::offset_of!(NavopRdpLastError, abi_version) == 4);
    assert!(std::mem::offset_of!(NavopRdpLastError, result) == 8);
    assert!(std::mem::offset_of!(NavopRdpLastError, hresult) == 12);
    assert!(std::mem::offset_of!(NavopRdpLastError, has_hresult) == 16);
    assert!(std::mem::offset_of!(NavopRdpLastError, reserved) == 20);
    assert!(std::mem::offset_of!(NavopRdpLastError, stage) == 24);
    assert!(std::mem::offset_of!(NavopRdpLastError, win32_code) == 28);
    assert!(std::mem::offset_of!(NavopRdpLastError, has_win32_code) == 32);

    assert!(size_of::<NavopRdpCreateOptions>() == 16);
    assert!(align_of::<NavopRdpCreateOptions>() == 4);
    assert!(std::mem::offset_of!(NavopRdpCreateOptions, struct_size) == 0);
    assert!(std::mem::offset_of!(NavopRdpCreateOptions, abi_version) == 4);
    assert!(std::mem::offset_of!(NavopRdpCreateOptions, generation_low) == 8);
    assert!(std::mem::offset_of!(NavopRdpCreateOptions, generation_high) == 12);

    assert!(std::mem::offset_of!(NavopRdpCreateWithParentOptions, struct_size) == 0);
    assert!(std::mem::offset_of!(NavopRdpCreateWithParentOptions, abi_version) == 4);
    assert!(std::mem::offset_of!(NavopRdpCreateWithParentOptions, generation_low) == 8);
    assert!(std::mem::offset_of!(NavopRdpCreateWithParentOptions, generation_high) == 12);
    assert!(std::mem::offset_of!(NavopRdpCreateWithParentOptions, parent_hwnd) == 16);

    assert!(size_of::<NavopRdpBounds>() == 16);
    assert!(align_of::<NavopRdpBounds>() == 4);
    assert!(std::mem::offset_of!(NavopRdpBounds, x) == 0);
    assert!(std::mem::offset_of!(NavopRdpBounds, y) == 4);
    assert!(std::mem::offset_of!(NavopRdpBounds, width) == 8);
    assert!(std::mem::offset_of!(NavopRdpBounds, height) == 12);

    assert!(size_of::<NavopRdpSessionDisplaySettings>() == 36);
    assert!(align_of::<NavopRdpSessionDisplaySettings>() == 4);
    assert!(std::mem::offset_of!(NavopRdpSessionDisplaySettings, struct_size) == 0);
    assert!(std::mem::offset_of!(NavopRdpSessionDisplaySettings, abi_version) == 4);
    assert!(std::mem::offset_of!(NavopRdpSessionDisplaySettings, desktop_width) == 8);
    assert!(std::mem::offset_of!(NavopRdpSessionDisplaySettings, desktop_height) == 12);
    assert!(std::mem::offset_of!(NavopRdpSessionDisplaySettings, physical_width) == 16);
    assert!(std::mem::offset_of!(NavopRdpSessionDisplaySettings, physical_height) == 20);
    assert!(std::mem::offset_of!(NavopRdpSessionDisplaySettings, orientation) == 24);
    assert!(std::mem::offset_of!(NavopRdpSessionDisplaySettings, desktop_scale_factor) == 28);
    assert!(std::mem::offset_of!(NavopRdpSessionDisplaySettings, device_scale_factor) == 32);

    assert!(size_of::<NavopRdpEvent>() == 32);
    assert!(align_of::<NavopRdpEvent>() == 4);
    assert!(std::mem::offset_of!(NavopRdpEvent, struct_size) == 0);
    assert!(std::mem::offset_of!(NavopRdpEvent, abi_version) == 4);
    assert!(std::mem::offset_of!(NavopRdpEvent, kind) == 8);
    assert!(std::mem::offset_of!(NavopRdpEvent, reserved) == 12);
    assert!(std::mem::offset_of!(NavopRdpEvent, generation_low) == 16);
    assert!(std::mem::offset_of!(NavopRdpEvent, generation_high) == 20);
    assert!(std::mem::offset_of!(NavopRdpEvent, code) == 24);
    assert!(std::mem::offset_of!(NavopRdpEvent, payload_len) == 28);

    assert!(size_of::<NavopRdpEventCallbackOptions>() == 16);
    assert!(align_of::<NavopRdpEventCallbackOptions>() == 4);
    assert!(std::mem::offset_of!(NavopRdpEventCallbackOptions, struct_size) == 0);
    assert!(std::mem::offset_of!(NavopRdpEventCallbackOptions, abi_version) == 4);
    assert!(std::mem::offset_of!(NavopRdpEventCallbackOptions, generation_low) == 8);
    assert!(std::mem::offset_of!(NavopRdpEventCallbackOptions, generation_high) == 12);

    assert!(std::mem::offset_of!(NavopRdpBorrowedSecret, data) == 0);
    assert!(std::mem::offset_of!(NavopRdpCredentialBundle, struct_size) == 0);
    assert!(std::mem::offset_of!(NavopRdpCredentialBundle, abi_version) == 4);
    assert!(std::mem::offset_of!(NavopRdpCredentialBundle, server_password) == 8);
    assert!(std::mem::offset_of!(NavopRdpCredentialBundle, username) > 8);
    assert!(
        std::mem::offset_of!(NavopRdpCredentialBundle, domain)
            > std::mem::offset_of!(NavopRdpCredentialBundle, username)
    );
    assert!(
        std::mem::offset_of!(NavopRdpCredentialBundle, gateway_username)
            > std::mem::offset_of!(NavopRdpCredentialBundle, domain)
    );
    assert!(
        std::mem::offset_of!(NavopRdpCredentialBundle, gateway_domain)
            > std::mem::offset_of!(NavopRdpCredentialBundle, gateway_username)
    );
    assert!(std::mem::offset_of!(NavopRdpBorrowedUtf16, data) == 0);
    assert!(std::mem::offset_of!(NavopRdpConnectionOptions, struct_size) == 0);
    assert!(std::mem::offset_of!(NavopRdpConnectionOptions, abi_version) == 4);
    assert!(std::mem::offset_of!(NavopRdpConnectionOptions, host) == 8);
};

#[cfg(target_pointer_width = "64")]
const _: () = {
    assert!(size_of::<NavopRdpCreateWithParentOptions>() == 24);
    assert!(align_of::<NavopRdpCreateWithParentOptions>() == 8);
    assert!(size_of::<NavopRdpBorrowedSecret>() == 16);
    assert!(align_of::<NavopRdpBorrowedSecret>() == 8);
    assert!(std::mem::offset_of!(NavopRdpBorrowedSecret, len) == 8);
    assert!(size_of::<NavopRdpCredentialBundle>() == 112);
    assert!(align_of::<NavopRdpCredentialBundle>() == 8);
    assert!(std::mem::offset_of!(NavopRdpCredentialBundle, gateway_password) == 24);
    assert!(std::mem::offset_of!(NavopRdpCredentialBundle, flags) == 40);
    assert!(std::mem::offset_of!(NavopRdpCredentialBundle, username) == 48);
    assert!(std::mem::offset_of!(NavopRdpCredentialBundle, domain) == 64);
    assert!(std::mem::offset_of!(NavopRdpCredentialBundle, gateway_username) == 80);
    assert!(std::mem::offset_of!(NavopRdpCredentialBundle, gateway_domain) == 96);
    assert!(size_of::<NavopRdpBorrowedUtf16>() == 16);
    assert!(align_of::<NavopRdpBorrowedUtf16>() == 8);
    assert!(std::mem::offset_of!(NavopRdpBorrowedUtf16, len) == 8);
    assert!(size_of::<NavopRdpConnectionOptions>() == 152);
    assert!(align_of::<NavopRdpConnectionOptions>() == 8);
    assert!(std::mem::offset_of!(NavopRdpConnectionOptions, port) == 24);
    assert!(std::mem::offset_of!(NavopRdpConnectionOptions, desktop_width) == 28);
    assert!(std::mem::offset_of!(NavopRdpConnectionOptions, desktop_height) == 32);
    assert!(std::mem::offset_of!(NavopRdpConnectionOptions, color_depth) == 36);
    assert!(std::mem::offset_of!(NavopRdpConnectionOptions, flags) == 40);
    assert!(std::mem::offset_of!(NavopRdpConnectionOptions, legacy_reserved) == 44);
    assert!(std::mem::offset_of!(NavopRdpConnectionOptions, display_mode) == 48);
    assert!(std::mem::offset_of!(NavopRdpConnectionOptions, display_flags) == 52);
    assert!(std::mem::offset_of!(NavopRdpConnectionOptions, desktop_scale_factor) == 56);
    assert!(std::mem::offset_of!(NavopRdpConnectionOptions, device_scale_factor) == 60);
    assert!(std::mem::offset_of!(NavopRdpConnectionOptions, resource_flags) == 64);
    assert!(std::mem::offset_of!(NavopRdpConnectionOptions, audio_mode) == 68);
    assert!(std::mem::offset_of!(NavopRdpConnectionOptions, audio_quality) == 72);
    assert!(std::mem::offset_of!(NavopRdpConnectionOptions, audio_flags) == 76);
    assert!(std::mem::offset_of!(NavopRdpConnectionOptions, keyboard_hook_mode) == 80);
    assert!(std::mem::offset_of!(NavopRdpConnectionOptions, input_flags) == 84);
    assert!(std::mem::offset_of!(NavopRdpConnectionOptions, performance_preset) == 88);
    assert!(std::mem::offset_of!(NavopRdpConnectionOptions, performance_flags) == 92);
    assert!(std::mem::offset_of!(NavopRdpConnectionOptions, network_connection_type) == 96);
    assert!(std::mem::offset_of!(NavopRdpConnectionOptions, security_flags) == 100);
    assert!(std::mem::offset_of!(NavopRdpConnectionOptions, authentication_level) == 104);
    assert!(std::mem::offset_of!(NavopRdpConnectionOptions, gateway_mode) == 108);
    assert!(std::mem::offset_of!(NavopRdpConnectionOptions, gateway_flags) == 112);
    assert!(std::mem::offset_of!(NavopRdpConnectionOptions, gateway_credential_source) == 116);
    assert!(std::mem::offset_of!(NavopRdpConnectionOptions, gateway_hostname) == 120);
    assert!(std::mem::offset_of!(NavopRdpConnectionOptions, keep_alive_seconds) == 136);
    assert!(std::mem::offset_of!(NavopRdpConnectionOptions, timeout_seconds) == 140);
    assert!(std::mem::offset_of!(NavopRdpConnectionOptions, connection_flags) == 144);
    assert!(std::mem::offset_of!(NavopRdpConnectionOptions, max_reconnect_attempts) == 148);
};

#[cfg(target_pointer_width = "32")]
const _: () = {
    assert!(size_of::<NavopRdpCreateWithParentOptions>() == 20);
    assert!(align_of::<NavopRdpCreateWithParentOptions>() == 4);
    assert!(size_of::<NavopRdpBorrowedSecret>() == 8);
    assert!(align_of::<NavopRdpBorrowedSecret>() == 4);
    assert!(std::mem::offset_of!(NavopRdpBorrowedSecret, len) == 4);
    assert!(size_of::<NavopRdpCredentialBundle>() == 60);
    assert!(align_of::<NavopRdpCredentialBundle>() == 4);
    assert!(std::mem::offset_of!(NavopRdpCredentialBundle, gateway_password) == 16);
    assert!(std::mem::offset_of!(NavopRdpCredentialBundle, flags) == 24);
    assert!(std::mem::offset_of!(NavopRdpCredentialBundle, username) == 28);
    assert!(std::mem::offset_of!(NavopRdpCredentialBundle, domain) == 36);
    assert!(std::mem::offset_of!(NavopRdpCredentialBundle, gateway_username) == 44);
    assert!(std::mem::offset_of!(NavopRdpCredentialBundle, gateway_domain) == 52);
    assert!(size_of::<NavopRdpBorrowedUtf16>() == 8);
    assert!(align_of::<NavopRdpBorrowedUtf16>() == 4);
    assert!(std::mem::offset_of!(NavopRdpBorrowedUtf16, len) == 4);
    assert!(size_of::<NavopRdpConnectionOptions>() == 136);
    assert!(align_of::<NavopRdpConnectionOptions>() == 4);
    assert!(std::mem::offset_of!(NavopRdpConnectionOptions, port) == 16);
    assert!(std::mem::offset_of!(NavopRdpConnectionOptions, desktop_width) == 20);
    assert!(std::mem::offset_of!(NavopRdpConnectionOptions, desktop_height) == 24);
    assert!(std::mem::offset_of!(NavopRdpConnectionOptions, color_depth) == 28);
    assert!(std::mem::offset_of!(NavopRdpConnectionOptions, flags) == 32);
    assert!(std::mem::offset_of!(NavopRdpConnectionOptions, legacy_reserved) == 36);
    assert!(std::mem::offset_of!(NavopRdpConnectionOptions, display_mode) == 40);
    assert!(std::mem::offset_of!(NavopRdpConnectionOptions, display_flags) == 44);
    assert!(std::mem::offset_of!(NavopRdpConnectionOptions, desktop_scale_factor) == 48);
    assert!(std::mem::offset_of!(NavopRdpConnectionOptions, device_scale_factor) == 52);
    assert!(std::mem::offset_of!(NavopRdpConnectionOptions, resource_flags) == 56);
    assert!(std::mem::offset_of!(NavopRdpConnectionOptions, audio_mode) == 60);
    assert!(std::mem::offset_of!(NavopRdpConnectionOptions, audio_quality) == 64);
    assert!(std::mem::offset_of!(NavopRdpConnectionOptions, audio_flags) == 68);
    assert!(std::mem::offset_of!(NavopRdpConnectionOptions, keyboard_hook_mode) == 72);
    assert!(std::mem::offset_of!(NavopRdpConnectionOptions, input_flags) == 76);
    assert!(std::mem::offset_of!(NavopRdpConnectionOptions, performance_preset) == 80);
    assert!(std::mem::offset_of!(NavopRdpConnectionOptions, performance_flags) == 84);
    assert!(std::mem::offset_of!(NavopRdpConnectionOptions, network_connection_type) == 88);
    assert!(std::mem::offset_of!(NavopRdpConnectionOptions, security_flags) == 92);
    assert!(std::mem::offset_of!(NavopRdpConnectionOptions, authentication_level) == 96);
    assert!(std::mem::offset_of!(NavopRdpConnectionOptions, gateway_mode) == 100);
    assert!(std::mem::offset_of!(NavopRdpConnectionOptions, gateway_flags) == 104);
    assert!(std::mem::offset_of!(NavopRdpConnectionOptions, gateway_credential_source) == 108);
    assert!(std::mem::offset_of!(NavopRdpConnectionOptions, gateway_hostname) == 112);
    assert!(std::mem::offset_of!(NavopRdpConnectionOptions, keep_alive_seconds) == 120);
    assert!(std::mem::offset_of!(NavopRdpConnectionOptions, timeout_seconds) == 124);
    assert!(std::mem::offset_of!(NavopRdpConnectionOptions, connection_flags) == 128);
    assert!(std::mem::offset_of!(NavopRdpConnectionOptions, max_reconnect_attempts) == 132);
};

impl NavopRdpEventCallbackOptions {
    pub(crate) fn current(generation: u64) -> Self {
        Self {
            struct_size: size_of::<Self>() as u32,
            abi_version: ABI_VERSION,
            generation_low: generation as u32,
            generation_high: (generation >> 32) as u32,
        }
    }
}

pub(crate) type NativeEventCallback =
    unsafe extern "C" fn(context: *mut c_void, event: *const NavopRdpEvent, payload: *const u8);

pub(crate) type ProbeFn = unsafe fn(
    options: *const NavopRdpProbeOptions,
    out_result: *mut NavopRdpProbeResult,
) -> NativeResult;
pub(crate) type CreateFn = unsafe fn(
    options: *const NavopRdpCreateOptions,
    out_host: *mut *mut NativeRdpHost,
) -> NativeResult;
pub(crate) type CreateWithParentV2Fn = unsafe fn(
    options: *const NavopRdpCreateWithParentOptions,
    out_host: *mut *mut NativeRdpHost,
    out_error: *mut NavopRdpLastError,
) -> NativeResult;
pub(crate) type GetLastErrorFn =
    unsafe fn(host: *mut NativeRdpHost, out_error: *mut NavopRdpLastError) -> NativeResult;
pub(crate) type SetBoundsFn =
    unsafe fn(host: *mut NativeRdpHost, bounds: *const NavopRdpBounds) -> NativeResult;
pub(crate) type UpdateSessionDisplaySettingsFn = unsafe fn(
    host: *mut NativeRdpHost,
    settings: *const NavopRdpSessionDisplaySettings,
) -> NativeResult;
pub(crate) type SetVisibleFn = unsafe fn(host: *mut NativeRdpHost, visible: u32) -> NativeResult;
pub(crate) type FocusFn = unsafe fn(host: *mut NativeRdpHost) -> NativeResult;
pub(crate) type DestroyFn = unsafe fn(host: *mut *mut NativeRdpHost) -> NativeResult;
pub(crate) type RegisterEventCallbackFn = unsafe fn(
    host: *mut NativeRdpHost,
    options: *const NavopRdpEventCallbackOptions,
    callback: Option<NativeEventCallback>,
    callback_context: *mut c_void,
) -> NativeResult;
pub(crate) type UnregisterEventCallbackFn = unsafe fn(host: *mut NativeRdpHost) -> NativeResult;
pub(crate) type ApplyCredentialsFn = unsafe fn(
    host: *mut NativeRdpHost,
    credentials: *const NavopRdpCredentialBundle,
) -> NativeResult;
pub(crate) type ConnectFn =
    unsafe fn(host: *mut NativeRdpHost, options: *const NavopRdpConnectionOptions) -> NativeResult;
pub(crate) type GetConnectionStateFn =
    unsafe fn(host: *mut NativeRdpHost, out_state: *mut u32) -> NativeResult;
pub(crate) type RequestCloseFn =
    unsafe fn(host: *mut NativeRdpHost, out_status: *mut u32) -> NativeResult;
pub(crate) type DisconnectFn = unsafe fn(host: *mut NativeRdpHost) -> NativeResult;

#[derive(Clone, Copy)]
pub(crate) struct NativeBindings {
    pub(crate) probe: ProbeFn,
    pub(crate) create: CreateFn,
    pub(crate) create_with_parent_v2: CreateWithParentV2Fn,
    pub(crate) get_last_error: GetLastErrorFn,
    pub(crate) set_bounds: SetBoundsFn,
    pub(crate) update_session_display_settings: UpdateSessionDisplaySettingsFn,
    pub(crate) set_visible: SetVisibleFn,
    pub(crate) focus: FocusFn,
    pub(crate) destroy: DestroyFn,
    pub(crate) register_event_callback: RegisterEventCallbackFn,
    pub(crate) unregister_event_callback: UnregisterEventCallbackFn,
    pub(crate) apply_credentials: ApplyCredentialsFn,
    pub(crate) connect: ConnectFn,
    pub(crate) get_connection_state: GetConnectionStateFn,
    pub(crate) request_close: RequestCloseFn,
    pub(crate) disconnect: DisconnectFn,
}

pub(crate) const NATIVE_BINDINGS: NativeBindings = NativeBindings {
    probe,
    create,
    create_with_parent_v2,
    get_last_error,
    set_bounds,
    update_session_display_settings,
    set_visible,
    focus,
    destroy,
    register_event_callback,
    unregister_event_callback,
    apply_credentials,
    connect,
    get_connection_state,
    request_close,
    disconnect,
};

#[cfg(windows_rdp_host_native)]
unsafe extern "C" {
    fn navop_rdp_probe(
        options: *const NavopRdpProbeOptions,
        out_result: *mut NavopRdpProbeResult,
    ) -> NativeResult;
    fn navop_rdp_create(
        options: *const NavopRdpCreateOptions,
        out_host: *mut *mut NativeRdpHost,
    ) -> NativeResult;
    fn navop_rdp_create_with_parent_v2(
        options: *const NavopRdpCreateWithParentOptions,
        out_host: *mut *mut NativeRdpHost,
        out_error: *mut NavopRdpLastError,
    ) -> NativeResult;
    fn navop_rdp_get_last_error(
        host: *mut NativeRdpHost,
        out_error: *mut NavopRdpLastError,
    ) -> NativeResult;
    fn navop_rdp_set_bounds(
        host: *mut NativeRdpHost,
        bounds: *const NavopRdpBounds,
    ) -> NativeResult;
    fn navop_rdp_update_session_display_settings(
        host: *mut NativeRdpHost,
        settings: *const NavopRdpSessionDisplaySettings,
    ) -> NativeResult;
    fn navop_rdp_set_visible(host: *mut NativeRdpHost, visible: u32) -> NativeResult;
    fn navop_rdp_focus(host: *mut NativeRdpHost) -> NativeResult;
    fn navop_rdp_register_event_callback(
        host: *mut NativeRdpHost,
        options: *const NavopRdpEventCallbackOptions,
        callback: Option<NativeEventCallback>,
        callback_context: *mut c_void,
    ) -> NativeResult;
    fn navop_rdp_unregister_event_callback(host: *mut NativeRdpHost) -> NativeResult;
    fn navop_rdp_apply_credentials(
        host: *mut NativeRdpHost,
        credentials: *const NavopRdpCredentialBundle,
    ) -> NativeResult;
    fn navop_rdp_connect(
        host: *mut NativeRdpHost,
        options: *const NavopRdpConnectionOptions,
    ) -> NativeResult;
    fn navop_rdp_get_connection_state(
        host: *mut NativeRdpHost,
        out_state: *mut u32,
    ) -> NativeResult;
    fn navop_rdp_request_close(host: *mut NativeRdpHost, out_status: *mut u32) -> NativeResult;
    fn navop_rdp_disconnect(host: *mut NativeRdpHost) -> NativeResult;
    fn navop_rdp_destroy(host: *mut *mut NativeRdpHost) -> NativeResult;
}

#[cfg(windows_rdp_host_native)]
unsafe fn probe(
    options: *const NavopRdpProbeOptions,
    out_result: *mut NavopRdpProbeResult,
) -> NativeResult {
    unsafe { navop_rdp_probe(options, out_result) }
}

#[cfg(windows_rdp_host_native)]
unsafe fn create(
    options: *const NavopRdpCreateOptions,
    out_host: *mut *mut NativeRdpHost,
) -> NativeResult {
    unsafe { navop_rdp_create(options, out_host) }
}

#[cfg(windows_rdp_host_native)]
unsafe fn create_with_parent_v2(
    options: *const NavopRdpCreateWithParentOptions,
    out_host: *mut *mut NativeRdpHost,
    out_error: *mut NavopRdpLastError,
) -> NativeResult {
    unsafe { navop_rdp_create_with_parent_v2(options, out_host, out_error) }
}

#[cfg(windows_rdp_host_native)]
unsafe fn get_last_error(
    host: *mut NativeRdpHost,
    out_error: *mut NavopRdpLastError,
) -> NativeResult {
    unsafe { navop_rdp_get_last_error(host, out_error) }
}

#[cfg(windows_rdp_host_native)]
unsafe fn set_bounds(host: *mut NativeRdpHost, bounds: *const NavopRdpBounds) -> NativeResult {
    unsafe { navop_rdp_set_bounds(host, bounds) }
}

#[cfg(windows_rdp_host_native)]
unsafe fn update_session_display_settings(
    host: *mut NativeRdpHost,
    settings: *const NavopRdpSessionDisplaySettings,
) -> NativeResult {
    unsafe { navop_rdp_update_session_display_settings(host, settings) }
}

#[cfg(windows_rdp_host_native)]
unsafe fn set_visible(host: *mut NativeRdpHost, visible: u32) -> NativeResult {
    unsafe { navop_rdp_set_visible(host, visible) }
}

#[cfg(windows_rdp_host_native)]
unsafe fn focus(host: *mut NativeRdpHost) -> NativeResult {
    unsafe { navop_rdp_focus(host) }
}

#[cfg(windows_rdp_host_native)]
unsafe fn register_event_callback(
    host: *mut NativeRdpHost,
    options: *const NavopRdpEventCallbackOptions,
    callback: Option<NativeEventCallback>,
    callback_context: *mut c_void,
) -> NativeResult {
    unsafe { navop_rdp_register_event_callback(host, options, callback, callback_context) }
}

#[cfg(windows_rdp_host_native)]
unsafe fn unregister_event_callback(host: *mut NativeRdpHost) -> NativeResult {
    unsafe { navop_rdp_unregister_event_callback(host) }
}

#[cfg(windows_rdp_host_native)]
unsafe fn apply_credentials(
    host: *mut NativeRdpHost,
    credentials: *const NavopRdpCredentialBundle,
) -> NativeResult {
    unsafe { navop_rdp_apply_credentials(host, credentials) }
}

#[cfg(windows_rdp_host_native)]
unsafe fn connect(
    host: *mut NativeRdpHost,
    options: *const NavopRdpConnectionOptions,
) -> NativeResult {
    unsafe { navop_rdp_connect(host, options) }
}

#[cfg(windows_rdp_host_native)]
unsafe fn get_connection_state(host: *mut NativeRdpHost, out_state: *mut u32) -> NativeResult {
    unsafe { navop_rdp_get_connection_state(host, out_state) }
}

#[cfg(windows_rdp_host_native)]
unsafe fn request_close(host: *mut NativeRdpHost, out_status: *mut u32) -> NativeResult {
    unsafe { navop_rdp_request_close(host, out_status) }
}

#[cfg(windows_rdp_host_native)]
unsafe fn disconnect(host: *mut NativeRdpHost) -> NativeResult {
    unsafe { navop_rdp_disconnect(host) }
}

#[cfg(windows_rdp_host_native)]
unsafe fn destroy(host: *mut *mut NativeRdpHost) -> NativeResult {
    unsafe { navop_rdp_destroy(host) }
}

#[cfg(not(windows_rdp_host_native))]
unsafe fn probe(
    options: *const NavopRdpProbeOptions,
    out_result: *mut NavopRdpProbeResult,
) -> NativeResult {
    if options.is_null() || out_result.is_null() {
        return RESULT_INVALID_ARGUMENT;
    }

    let options_struct_size = unsafe { std::ptr::addr_of!((*options).struct_size).read() };
    if options_struct_size < size_of::<NavopRdpProbeOptions>() as u32 {
        return RESULT_INVALID_ARGUMENT;
    }
    let options_abi_version = unsafe { std::ptr::addr_of!((*options).abi_version).read() };
    if options_abi_version != ABI_VERSION {
        return RESULT_ABI_MISMATCH;
    }

    let caller_result_size = unsafe { std::ptr::addr_of!((*out_result).struct_size).read() };
    if caller_result_size < size_of::<NavopRdpProbeResult>() as u32 {
        return RESULT_INVALID_ARGUMENT;
    }
    let result_abi_version = unsafe { std::ptr::addr_of!((*out_result).abi_version).read() };
    if result_abi_version != ABI_VERSION {
        return RESULT_ABI_MISMATCH;
    }

    unsafe {
        *out_result = NavopRdpProbeResult {
            struct_size: caller_result_size,
            abi_version: ABI_VERSION,
            available: 0,
            reserved: 0,
        };
    }
    RESULT_OK
}

#[cfg(not(windows_rdp_host_native))]
unsafe fn create(
    options: *const NavopRdpCreateOptions,
    out_host: *mut *mut NativeRdpHost,
) -> NativeResult {
    if out_host.is_null() {
        return RESULT_INVALID_ARGUMENT;
    }
    unsafe {
        *out_host = std::ptr::null_mut();
    }
    if options.is_null() {
        return RESULT_INVALID_ARGUMENT;
    }

    let options_struct_size = unsafe { std::ptr::addr_of!((*options).struct_size).read() };
    if options_struct_size < size_of::<NavopRdpCreateOptions>() as u32 {
        return RESULT_INVALID_ARGUMENT;
    }
    let options_abi_version = unsafe { std::ptr::addr_of!((*options).abi_version).read() };
    if options_abi_version != ABI_VERSION {
        return RESULT_ABI_MISMATCH;
    }

    RESULT_UNAVAILABLE
}

#[cfg(not(windows_rdp_host_native))]
unsafe fn create_with_parent_v2(
    options: *const NavopRdpCreateWithParentOptions,
    out_host: *mut *mut NativeRdpHost,
    out_error: *mut NavopRdpLastError,
) -> NativeResult {
    if out_error.is_null() {
        return RESULT_INVALID_ARGUMENT;
    }
    let error_size = unsafe { std::ptr::addr_of!((*out_error).struct_size).read() };
    if error_size < LAST_ERROR_LEGACY_SIZE {
        return RESULT_INVALID_ARGUMENT;
    }
    let error_abi = unsafe { std::ptr::addr_of!((*out_error).abi_version).read() };
    if error_abi != ABI_VERSION {
        return RESULT_ABI_MISMATCH;
    }
    unsafe {
        (*out_error).struct_size = error_size;
        (*out_error).abi_version = ABI_VERSION;
        (*out_error).result = RESULT_OK;
        (*out_error).hresult = 0;
        (*out_error).has_hresult = 0;
        (*out_error).reserved = 0;
        if error_size
            >= std::mem::offset_of!(NavopRdpLastError, stage) as u32 + size_of::<u32>() as u32
        {
            (*out_error).stage = CREATE_STAGE_NONE;
        }
        if error_size
            >= std::mem::offset_of!(NavopRdpLastError, win32_code) as u32 + size_of::<u32>() as u32
        {
            (*out_error).win32_code = 0;
        }
        if error_size
            >= std::mem::offset_of!(NavopRdpLastError, has_win32_code) as u32
                + size_of::<u32>() as u32
        {
            (*out_error).has_win32_code = 0;
        }
    }
    if out_host.is_null() {
        unsafe { (*out_error).result = RESULT_INVALID_ARGUMENT };
        return RESULT_INVALID_ARGUMENT;
    }
    unsafe {
        *out_host = std::ptr::null_mut();
    }
    if options.is_null() {
        unsafe { (*out_error).result = RESULT_INVALID_ARGUMENT };
        return RESULT_INVALID_ARGUMENT;
    }

    let options_struct_size = unsafe { std::ptr::addr_of!((*options).struct_size).read() };
    if options_struct_size < size_of::<NavopRdpCreateWithParentOptions>() as u32 {
        unsafe { (*out_error).result = RESULT_INVALID_ARGUMENT };
        return RESULT_INVALID_ARGUMENT;
    }
    let options_abi_version = unsafe { std::ptr::addr_of!((*options).abi_version).read() };
    if options_abi_version != CREATE_WITH_PARENT_ABI_VERSION {
        unsafe { (*out_error).result = RESULT_ABI_MISMATCH };
        return RESULT_ABI_MISMATCH;
    }
    if unsafe { std::ptr::addr_of!((*options).parent_hwnd).read() } == 0 {
        unsafe { (*out_error).result = RESULT_INVALID_ARGUMENT };
        return RESULT_INVALID_ARGUMENT;
    }

    unsafe { (*out_error).result = RESULT_UNAVAILABLE };
    RESULT_UNAVAILABLE
}

#[cfg(not(windows_rdp_host_native))]
unsafe fn get_last_error(
    host: *mut NativeRdpHost,
    out_error: *mut NavopRdpLastError,
) -> NativeResult {
    if host.is_null() || out_error.is_null() {
        return RESULT_INVALID_ARGUMENT;
    }
    RESULT_UNAVAILABLE
}

#[cfg(not(windows_rdp_host_native))]
unsafe fn set_bounds(host: *mut NativeRdpHost, bounds: *const NavopRdpBounds) -> NativeResult {
    if host.is_null() || bounds.is_null() {
        return RESULT_INVALID_ARGUMENT;
    }

    let width = unsafe { std::ptr::addr_of!((*bounds).width).read() };
    let height = unsafe { std::ptr::addr_of!((*bounds).height).read() };
    if width < 0 || height < 0 {
        return RESULT_INVALID_ARGUMENT;
    }

    RESULT_UNAVAILABLE
}

#[cfg(not(windows_rdp_host_native))]
unsafe fn update_session_display_settings(
    host: *mut NativeRdpHost,
    settings: *const NavopRdpSessionDisplaySettings,
) -> NativeResult {
    if host.is_null() || settings.is_null() {
        return RESULT_INVALID_ARGUMENT;
    }
    let struct_size = unsafe { std::ptr::addr_of!((*settings).struct_size).read() };
    if struct_size < size_of::<NavopRdpSessionDisplaySettings>() as u32 {
        return RESULT_INVALID_ARGUMENT;
    }
    let abi_version = unsafe { std::ptr::addr_of!((*settings).abi_version).read() };
    if abi_version != SESSION_DISPLAY_SETTINGS_ABI_VERSION {
        return RESULT_ABI_MISMATCH;
    }
    let desktop_width = unsafe { std::ptr::addr_of!((*settings).desktop_width).read() };
    let desktop_height = unsafe { std::ptr::addr_of!((*settings).desktop_height).read() };
    let physical_width = unsafe { std::ptr::addr_of!((*settings).physical_width).read() };
    let physical_height = unsafe { std::ptr::addr_of!((*settings).physical_height).read() };
    let desktop_scale_factor =
        unsafe { std::ptr::addr_of!((*settings).desktop_scale_factor).read() };
    let device_scale_factor = unsafe { std::ptr::addr_of!((*settings).device_scale_factor).read() };
    if desktop_width == 0
        || desktop_height == 0
        || physical_width == 0
        || physical_height == 0
        || desktop_scale_factor == 0
        || device_scale_factor == 0
    {
        return RESULT_INVALID_ARGUMENT;
    }
    RESULT_UNAVAILABLE
}

#[cfg(not(windows_rdp_host_native))]
unsafe fn set_visible(host: *mut NativeRdpHost, visible: u32) -> NativeResult {
    if host.is_null() || visible > 1 {
        return RESULT_INVALID_ARGUMENT;
    }
    RESULT_UNAVAILABLE
}

#[cfg(not(windows_rdp_host_native))]
unsafe fn focus(host: *mut NativeRdpHost) -> NativeResult {
    if host.is_null() {
        return RESULT_INVALID_ARGUMENT;
    }
    RESULT_UNAVAILABLE
}

#[cfg(not(windows_rdp_host_native))]
unsafe fn register_event_callback(
    host: *mut NativeRdpHost,
    options: *const NavopRdpEventCallbackOptions,
    callback: Option<NativeEventCallback>,
    _callback_context: *mut c_void,
) -> NativeResult {
    if host.is_null() || options.is_null() || callback.is_none() {
        return RESULT_INVALID_ARGUMENT;
    }

    let options_struct_size = unsafe { std::ptr::addr_of!((*options).struct_size).read() };
    if options_struct_size < size_of::<NavopRdpEventCallbackOptions>() as u32 {
        return RESULT_INVALID_ARGUMENT;
    }
    let options_abi_version = unsafe { std::ptr::addr_of!((*options).abi_version).read() };
    if options_abi_version != ABI_VERSION {
        return RESULT_ABI_MISMATCH;
    }

    RESULT_UNAVAILABLE
}

#[cfg(not(windows_rdp_host_native))]
unsafe fn unregister_event_callback(host: *mut NativeRdpHost) -> NativeResult {
    if host.is_null() {
        return RESULT_INVALID_ARGUMENT;
    }
    RESULT_OK
}

#[cfg(not(windows_rdp_host_native))]
unsafe fn apply_credentials(
    host: *mut NativeRdpHost,
    credentials: *const NavopRdpCredentialBundle,
) -> NativeResult {
    if host.is_null() || credentials.is_null() {
        return RESULT_INVALID_ARGUMENT;
    }

    let base = credentials.cast::<c_void>();
    let Some(struct_size) = (unsafe { read_abi_field::<u32>(base, size_of::<u32>() as u32, 0) })
    else {
        return RESULT_INVALID_ARGUMENT;
    };
    if struct_size < CREDENTIAL_LEGACY_SIZE {
        return RESULT_INVALID_ARGUMENT;
    }

    let read_required_u32 = |offset| unsafe {
        read_abi_field::<u32>(base, struct_size, offset).ok_or(RESULT_INVALID_ARGUMENT)
    };
    let abi_version =
        match read_required_u32(std::mem::offset_of!(NavopRdpCredentialBundle, abi_version)) {
            Ok(value) => value,
            Err(result) => return result,
        };
    if abi_version != ABI_VERSION {
        return RESULT_ABI_MISMATCH;
    }
    let flags = match read_required_u32(std::mem::offset_of!(NavopRdpCredentialBundle, flags)) {
        Ok(value) => value,
        Err(result) => return result,
    };
    if flags != 0 {
        return RESULT_INVALID_ARGUMENT;
    }

    let server_password = match unsafe {
        read_abi_field::<NavopRdpBorrowedSecret>(
            base,
            struct_size,
            std::mem::offset_of!(NavopRdpCredentialBundle, server_password),
        )
        .ok_or(RESULT_INVALID_ARGUMENT)
    } {
        Ok(value) => value,
        Err(result) => return result,
    };
    if !valid_borrowed_secret(server_password) {
        return RESULT_INVALID_ARGUMENT;
    }
    let gateway_password = match unsafe {
        read_abi_field::<NavopRdpBorrowedSecret>(
            base,
            struct_size,
            std::mem::offset_of!(NavopRdpCredentialBundle, gateway_password),
        )
        .ok_or(RESULT_INVALID_ARGUMENT)
    } {
        Ok(value) => value,
        Err(result) => return result,
    };
    if !valid_borrowed_secret(gateway_password) {
        return RESULT_INVALID_ARGUMENT;
    }

    macro_rules! validate_optional_identity {
        ($field:ident) => {
            if let Some(value) = unsafe {
                read_abi_field::<NavopRdpBorrowedUtf16>(
                    base,
                    struct_size,
                    std::mem::offset_of!(NavopRdpCredentialBundle, $field),
                )
            } {
                if !valid_borrowed_utf16(value) {
                    return RESULT_INVALID_ARGUMENT;
                }
            }
        };
    }

    validate_optional_identity!(username);
    validate_optional_identity!(domain);
    validate_optional_identity!(gateway_username);
    validate_optional_identity!(gateway_domain);

    RESULT_UNAVAILABLE
}

#[cfg(not(windows_rdp_host_native))]
unsafe fn connect(
    host: *mut NativeRdpHost,
    options: *const NavopRdpConnectionOptions,
) -> NativeResult {
    if host.is_null() || options.is_null() {
        return RESULT_INVALID_ARGUMENT;
    }

    let normalized = match unsafe { normalize_connection_options(options) } {
        Ok(options) => options,
        Err(result) => return result,
    };
    let validation_result = validate_connection_options(&normalized);
    if validation_result != RESULT_OK {
        return validation_result;
    }

    RESULT_UNAVAILABLE
}

#[cfg(not(windows_rdp_host_native))]
unsafe fn get_connection_state(host: *mut NativeRdpHost, out_state: *mut u32) -> NativeResult {
    if out_state.is_null() {
        return RESULT_INVALID_ARGUMENT;
    }
    unsafe {
        *out_state = CONNECTION_STATE_DISCONNECTED;
    }
    if host.is_null() {
        return RESULT_INVALID_ARGUMENT;
    }
    RESULT_UNAVAILABLE
}

#[cfg(not(windows_rdp_host_native))]
unsafe fn request_close(host: *mut NativeRdpHost, out_status: *mut u32) -> NativeResult {
    if out_status.is_null() {
        return RESULT_INVALID_ARGUMENT;
    }
    unsafe {
        *out_status = REQUEST_CLOSE_CAN_PROCEED;
    }
    if host.is_null() {
        return RESULT_INVALID_ARGUMENT;
    }
    RESULT_UNAVAILABLE
}

#[cfg(not(windows_rdp_host_native))]
unsafe fn disconnect(host: *mut NativeRdpHost) -> NativeResult {
    if host.is_null() {
        return RESULT_INVALID_ARGUMENT;
    }
    RESULT_UNAVAILABLE
}

#[cfg(not(windows_rdp_host_native))]
unsafe fn destroy(host: *mut *mut NativeRdpHost) -> NativeResult {
    if host.is_null() {
        return RESULT_INVALID_ARGUMENT;
    }
    unsafe {
        *host = std::ptr::null_mut();
    }
    RESULT_OK
}

#[cfg(test)]
mod tests {
    use std::mem::{align_of, size_of};

    use super::*;

    #[test]
    fn abi_constants_match_the_native_header() {
        assert_eq!(ABI_VERSION, 1);
        assert_eq!(RESULT_OK, 0);
        assert_eq!(RESULT_INVALID_ARGUMENT, 1);
        assert_eq!(RESULT_ABI_MISMATCH, 2);
        assert_eq!(RESULT_ALLOCATION_FAILED, 3);
        assert_eq!(RESULT_INTERNAL_ERROR, 4);
        assert_eq!(RESULT_UNAVAILABLE, 5);
        assert_eq!(RESULT_WRONG_THREAD, 6);
        assert_eq!(RESULT_CALLBACK_IN_FLIGHT, 7);
        assert_eq!(RESULT_INVALID_STATE, 8);
        assert_eq!(size_of::<NativeResult>(), 4);
    }

    #[test]
    fn fixed_width_abi_struct_layout_is_architecture_independent() {
        assert_eq!(size_of::<NavopRdpProbeOptions>(), 8);
        assert_eq!(align_of::<NavopRdpProbeOptions>(), 4);
        assert_eq!(size_of::<NavopRdpProbeResult>(), 16);
        assert_eq!(align_of::<NavopRdpProbeResult>(), 4);
        assert_eq!(size_of::<NavopRdpLastError>(), 36);
        assert_eq!(align_of::<NavopRdpLastError>(), 4);
        assert_eq!(std::mem::offset_of!(NavopRdpLastError, struct_size), 0);
        assert_eq!(std::mem::offset_of!(NavopRdpLastError, abi_version), 4);
        assert_eq!(std::mem::offset_of!(NavopRdpLastError, result), 8);
        assert_eq!(std::mem::offset_of!(NavopRdpLastError, hresult), 12);
        assert_eq!(std::mem::offset_of!(NavopRdpLastError, has_hresult), 16);
        assert_eq!(std::mem::offset_of!(NavopRdpLastError, reserved), 20);
        assert_eq!(std::mem::offset_of!(NavopRdpLastError, stage), 24);
        assert_eq!(std::mem::offset_of!(NavopRdpLastError, win32_code), 28);
        assert_eq!(std::mem::offset_of!(NavopRdpLastError, has_win32_code), 32);
        assert_eq!(size_of::<NavopRdpCreateOptions>(), 16);
        assert_eq!(align_of::<NavopRdpCreateOptions>(), 4);
        assert_eq!(size_of::<NavopRdpBounds>(), 16);
        assert_eq!(align_of::<NavopRdpBounds>(), 4);
        assert_eq!(std::mem::offset_of!(NavopRdpBounds, x), 0);
        assert_eq!(std::mem::offset_of!(NavopRdpBounds, y), 4);
        assert_eq!(std::mem::offset_of!(NavopRdpBounds, width), 8);
        assert_eq!(std::mem::offset_of!(NavopRdpBounds, height), 12);
        assert_eq!(size_of::<NavopRdpSessionDisplaySettings>(), 36);
        assert_eq!(align_of::<NavopRdpSessionDisplaySettings>(), 4);
        assert_eq!(
            std::mem::offset_of!(NavopRdpSessionDisplaySettings, struct_size),
            0
        );
        assert_eq!(
            std::mem::offset_of!(NavopRdpSessionDisplaySettings, abi_version),
            4
        );
        assert_eq!(
            std::mem::offset_of!(NavopRdpSessionDisplaySettings, desktop_width),
            8
        );
        assert_eq!(
            std::mem::offset_of!(NavopRdpSessionDisplaySettings, desktop_height),
            12
        );
        assert_eq!(
            std::mem::offset_of!(NavopRdpSessionDisplaySettings, physical_width),
            16
        );
        assert_eq!(
            std::mem::offset_of!(NavopRdpSessionDisplaySettings, physical_height),
            20
        );
        assert_eq!(
            std::mem::offset_of!(NavopRdpSessionDisplaySettings, orientation),
            24
        );
        assert_eq!(
            std::mem::offset_of!(NavopRdpSessionDisplaySettings, desktop_scale_factor),
            28
        );
        assert_eq!(
            std::mem::offset_of!(NavopRdpSessionDisplaySettings, device_scale_factor),
            32
        );
        assert_eq!(size_of::<NavopRdpEvent>(), 32);
        assert_eq!(align_of::<NavopRdpEvent>(), 4);
        assert_eq!(size_of::<NavopRdpEventCallbackOptions>(), 16);
        assert_eq!(align_of::<NavopRdpEventCallbackOptions>(), 4);
        assert_eq!(std::mem::offset_of!(NavopRdpEvent, struct_size), 0);
        assert_eq!(std::mem::offset_of!(NavopRdpEvent, abi_version), 4);
        assert_eq!(std::mem::offset_of!(NavopRdpEvent, kind), 8);
        assert_eq!(std::mem::offset_of!(NavopRdpEvent, reserved), 12);
        assert_eq!(std::mem::offset_of!(NavopRdpEvent, generation_low), 16);
        assert_eq!(std::mem::offset_of!(NavopRdpEvent, generation_high), 20);
        assert_eq!(std::mem::offset_of!(NavopRdpEvent, code), 24);
        assert_eq!(std::mem::offset_of!(NavopRdpEvent, payload_len), 28);
        assert_eq!(
            std::mem::offset_of!(NavopRdpEventCallbackOptions, struct_size),
            0
        );
        assert_eq!(
            std::mem::offset_of!(NavopRdpEventCallbackOptions, abi_version),
            4
        );
        assert_eq!(
            std::mem::offset_of!(NavopRdpEventCallbackOptions, generation_low),
            8
        );
        assert_eq!(
            std::mem::offset_of!(NavopRdpEventCallbackOptions, generation_high),
            12
        );
    }

    #[test]
    fn credential_layout_matches_the_current_pointer_width() {
        assert_eq!(std::mem::offset_of!(NavopRdpBorrowedSecret, data), 0);
        assert_eq!(
            std::mem::offset_of!(NavopRdpCredentialBundle, struct_size),
            0
        );
        assert_eq!(
            std::mem::offset_of!(NavopRdpCredentialBundle, abi_version),
            4
        );
        assert_eq!(
            std::mem::offset_of!(NavopRdpCredentialBundle, server_password),
            8
        );

        #[cfg(target_pointer_width = "64")]
        {
            assert_eq!(size_of::<NavopRdpBorrowedSecret>(), 16);
            assert_eq!(align_of::<NavopRdpBorrowedSecret>(), 8);
            assert_eq!(std::mem::offset_of!(NavopRdpBorrowedSecret, len), 8);
            assert_eq!(size_of::<NavopRdpCredentialBundle>(), 112);
            assert_eq!(align_of::<NavopRdpCredentialBundle>(), 8);
            assert_eq!(
                std::mem::offset_of!(NavopRdpCredentialBundle, gateway_password),
                24
            );
            assert_eq!(std::mem::offset_of!(NavopRdpCredentialBundle, flags), 40);
            assert_eq!(std::mem::offset_of!(NavopRdpCredentialBundle, username), 48);
            assert_eq!(std::mem::offset_of!(NavopRdpCredentialBundle, domain), 64);
            assert_eq!(
                std::mem::offset_of!(NavopRdpCredentialBundle, gateway_username),
                80
            );
            assert_eq!(
                std::mem::offset_of!(NavopRdpCredentialBundle, gateway_domain),
                96
            );
        }

        #[cfg(target_pointer_width = "32")]
        {
            assert_eq!(size_of::<NavopRdpBorrowedSecret>(), 8);
            assert_eq!(align_of::<NavopRdpBorrowedSecret>(), 4);
            assert_eq!(std::mem::offset_of!(NavopRdpBorrowedSecret, len), 4);
            assert_eq!(size_of::<NavopRdpCredentialBundle>(), 60);
            assert_eq!(align_of::<NavopRdpCredentialBundle>(), 4);
            assert_eq!(
                std::mem::offset_of!(NavopRdpCredentialBundle, gateway_password),
                16
            );
            assert_eq!(std::mem::offset_of!(NavopRdpCredentialBundle, flags), 24);
            assert_eq!(std::mem::offset_of!(NavopRdpCredentialBundle, username), 28);
            assert_eq!(std::mem::offset_of!(NavopRdpCredentialBundle, domain), 36);
            assert_eq!(
                std::mem::offset_of!(NavopRdpCredentialBundle, gateway_username),
                44
            );
            assert_eq!(
                std::mem::offset_of!(NavopRdpCredentialBundle, gateway_domain),
                52
            );
        }
    }

    #[test]
    fn connection_layout_matches_the_current_pointer_width() {
        assert_eq!(std::mem::offset_of!(NavopRdpBorrowedUtf16, data), 0);
        assert_eq!(
            std::mem::offset_of!(NavopRdpConnectionOptions, struct_size),
            0
        );
        assert_eq!(
            std::mem::offset_of!(NavopRdpConnectionOptions, abi_version),
            4
        );
        assert_eq!(std::mem::offset_of!(NavopRdpConnectionOptions, host), 8);

        #[cfg(target_pointer_width = "64")]
        {
            assert_eq!(size_of::<NavopRdpBorrowedUtf16>(), 16);
            assert_eq!(align_of::<NavopRdpBorrowedUtf16>(), 8);
            assert_eq!(std::mem::offset_of!(NavopRdpBorrowedUtf16, len), 8);
            assert_eq!(size_of::<NavopRdpConnectionOptions>(), 152);
            assert_eq!(align_of::<NavopRdpConnectionOptions>(), 8);
            assert_eq!(std::mem::offset_of!(NavopRdpConnectionOptions, port), 24);
            assert_eq!(
                std::mem::offset_of!(NavopRdpConnectionOptions, desktop_width),
                28
            );
            assert_eq!(
                std::mem::offset_of!(NavopRdpConnectionOptions, desktop_height),
                32
            );
            assert_eq!(
                std::mem::offset_of!(NavopRdpConnectionOptions, color_depth),
                36
            );
            assert_eq!(std::mem::offset_of!(NavopRdpConnectionOptions, flags), 40);
            assert_eq!(
                std::mem::offset_of!(NavopRdpConnectionOptions, display_mode),
                48
            );
            assert_eq!(
                std::mem::offset_of!(NavopRdpConnectionOptions, gateway_hostname),
                120
            );
            assert_eq!(
                std::mem::offset_of!(NavopRdpConnectionOptions, max_reconnect_attempts),
                148
            );
        }

        #[cfg(target_pointer_width = "32")]
        {
            assert_eq!(size_of::<NavopRdpBorrowedUtf16>(), 8);
            assert_eq!(align_of::<NavopRdpBorrowedUtf16>(), 4);
            assert_eq!(std::mem::offset_of!(NavopRdpBorrowedUtf16, len), 4);
            assert_eq!(size_of::<NavopRdpConnectionOptions>(), 136);
            assert_eq!(align_of::<NavopRdpConnectionOptions>(), 4);
            assert_eq!(std::mem::offset_of!(NavopRdpConnectionOptions, port), 16);
            assert_eq!(
                std::mem::offset_of!(NavopRdpConnectionOptions, desktop_width),
                20
            );
            assert_eq!(
                std::mem::offset_of!(NavopRdpConnectionOptions, desktop_height),
                24
            );
            assert_eq!(
                std::mem::offset_of!(NavopRdpConnectionOptions, color_depth),
                28
            );
            assert_eq!(std::mem::offset_of!(NavopRdpConnectionOptions, flags), 32);
            assert_eq!(
                std::mem::offset_of!(NavopRdpConnectionOptions, display_mode),
                40
            );
            assert_eq!(
                std::mem::offset_of!(NavopRdpConnectionOptions, gateway_hostname),
                112
            );
            assert_eq!(
                std::mem::offset_of!(NavopRdpConnectionOptions, max_reconnect_attempts),
                132
            );
        }
    }

    #[test]
    fn create_options_split_the_generation_without_abi_alignment_risk() {
        let options = NavopRdpCreateOptions::current(0x1122_3344_aabb_ccdd);

        assert_eq!(options.generation_low, 0xaabb_ccdd);
        assert_eq!(options.generation_high, 0x1122_3344);
    }

    #[test]
    fn callback_options_split_the_generation_without_abi_alignment_risk() {
        let options = NavopRdpEventCallbackOptions::current(0x1122_3344_aabb_ccdd);

        assert_eq!(options.generation_low, 0xaabb_ccdd);
        assert_eq!(options.generation_high, 0x1122_3344);
    }

    #[cfg(not(windows_rdp_host_native))]
    #[test]
    fn non_windows_probe_preserves_an_extended_caller_result_size() {
        let options = NavopRdpProbeOptions::current();
        let mut result = NavopRdpProbeResult::current();
        result.struct_size += 16;

        let native_result = unsafe { probe(&options, &mut result) };

        assert_eq!(native_result, RESULT_OK);
        assert_eq!(
            result.struct_size,
            size_of::<NavopRdpProbeResult>() as u32 + 16
        );
        assert_eq!(result.abi_version, ABI_VERSION);
        assert_eq!(result.reserved, 0);
    }

    #[cfg(not(windows_rdp_host_native))]
    #[test]
    fn non_windows_credentials_validate_size_before_version_and_borrowed_pointers() {
        let host = std::ptr::NonNull::<NativeRdpHost>::dangling().as_ptr();
        let mut credentials = NavopRdpCredentialBundle {
            struct_size: size_of::<NavopRdpCredentialBundle>() as u32,
            abi_version: ABI_VERSION,
            server_password: NavopRdpBorrowedSecret {
                data: std::ptr::null(),
                len: 0,
            },
            gateway_password: NavopRdpBorrowedSecret {
                data: std::ptr::null(),
                len: 0,
            },
            flags: 0,
            username: NavopRdpBorrowedUtf16 {
                data: std::ptr::null(),
                len: 0,
            },
            domain: NavopRdpBorrowedUtf16 {
                data: std::ptr::null(),
                len: 0,
            },
            gateway_username: NavopRdpBorrowedUtf16 {
                data: std::ptr::null(),
                len: 0,
            },
            gateway_domain: NavopRdpBorrowedUtf16 {
                data: std::ptr::null(),
                len: 0,
            },
        };

        credentials.struct_size = 4;
        credentials.abi_version += 1;
        assert_eq!(
            unsafe { apply_credentials(host, &credentials) },
            RESULT_INVALID_ARGUMENT
        );

        credentials.struct_size = size_of::<NavopRdpCredentialBundle>() as u32;
        assert_eq!(
            unsafe { apply_credentials(host, &credentials) },
            RESULT_ABI_MISMATCH
        );

        credentials.abi_version = ABI_VERSION;
        credentials.server_password.len = 1;
        assert_eq!(
            unsafe { apply_credentials(host, &credentials) },
            RESULT_INVALID_ARGUMENT
        );

        credentials.server_password.len = 0;
        assert_eq!(
            unsafe { apply_credentials(host, &credentials) },
            RESULT_UNAVAILABLE
        );

        credentials.username.len = 1;
        assert_eq!(
            unsafe { apply_credentials(host, &credentials) },
            RESULT_INVALID_ARGUMENT
        );
        credentials.username.len = 0;
        credentials.domain.len = 1;
        assert_eq!(
            unsafe { apply_credentials(host, &credentials) },
            RESULT_INVALID_ARGUMENT
        );
        credentials.domain.len = 0;

        credentials.struct_size = if cfg!(target_pointer_width = "64") {
            48
        } else {
            28
        };
        assert_eq!(
            unsafe { apply_credentials(host, &credentials) },
            RESULT_UNAVAILABLE
        );
    }

    #[cfg(not(windows_rdp_host_native))]
    #[test]
    fn non_windows_credentials_accept_append_only_callers_and_validate_gateway_identity() {
        #[repr(C)]
        struct ExtendedCredentials {
            base: NavopRdpCredentialBundle,
            trailing: [u8; 16],
        }

        let host = std::ptr::NonNull::<NativeRdpHost>::dangling().as_ptr();
        let empty_secret = NavopRdpBorrowedSecret {
            data: std::ptr::null(),
            len: 0,
        };
        let empty_text = NavopRdpBorrowedUtf16 {
            data: std::ptr::null(),
            len: 0,
        };
        let mut credentials = NavopRdpCredentialBundle {
            struct_size: size_of::<NavopRdpCredentialBundle>() as u32,
            abi_version: ABI_VERSION,
            server_password: empty_secret,
            gateway_password: empty_secret,
            flags: 0,
            username: empty_text,
            domain: empty_text,
            gateway_username: empty_text,
            gateway_domain: empty_text,
        };

        assert_eq!(
            unsafe { apply_credentials(host, &credentials) },
            RESULT_UNAVAILABLE
        );

        credentials.gateway_username.len = 1;
        assert_eq!(
            unsafe { apply_credentials(host, &credentials) },
            RESULT_INVALID_ARGUMENT
        );
        credentials.gateway_username.len = 0;
        credentials.gateway_domain.len = 1;
        assert_eq!(
            unsafe { apply_credentials(host, &credentials) },
            RESULT_INVALID_ARGUMENT
        );
        credentials.gateway_domain.len = 0;

        credentials.gateway_username.len = 1;
        credentials.struct_size = (std::mem::offset_of!(NavopRdpCredentialBundle, gateway_username)
            + size_of::<NavopRdpBorrowedUtf16>()
            - 1) as u32;
        assert_eq!(
            unsafe { apply_credentials(host, &credentials) },
            RESULT_UNAVAILABLE
        );
        credentials.gateway_username.len = 0;

        credentials.gateway_domain.len = 1;
        credentials.struct_size = (std::mem::offset_of!(NavopRdpCredentialBundle, gateway_domain)
            + size_of::<NavopRdpBorrowedUtf16>()
            - 1) as u32;
        assert_eq!(
            unsafe { apply_credentials(host, &credentials) },
            RESULT_UNAVAILABLE
        );

        let trailing = [0xA5; 16];
        let extended = ExtendedCredentials {
            base: NavopRdpCredentialBundle {
                struct_size: size_of::<ExtendedCredentials>() as u32,
                abi_version: ABI_VERSION,
                server_password: empty_secret,
                gateway_password: empty_secret,
                flags: 0,
                username: empty_text,
                domain: empty_text,
                gateway_username: empty_text,
                gateway_domain: empty_text,
            },
            trailing,
        };
        assert_eq!(
            unsafe { apply_credentials(host, &extended.base) },
            RESULT_UNAVAILABLE
        );
        assert_eq!(extended.trailing, trailing);
    }

    #[cfg(not(windows_rdp_host_native))]
    #[test]
    fn non_windows_connection_validates_layout_values_and_outputs_before_unavailable() {
        let host = std::ptr::NonNull::<NativeRdpHost>::dangling().as_ptr();
        let host_name: Vec<u16> = "rdp.example".encode_utf16().collect();
        let mut options = NavopRdpConnectionOptions::current(
            NavopRdpBorrowedUtf16 {
                data: host_name.as_ptr(),
                len: host_name.len() as u32,
            },
            3389,
            1920,
            1080,
            32,
        );

        options.struct_size = 4;
        options.abi_version += 1;
        assert_eq!(unsafe { connect(host, &options) }, RESULT_INVALID_ARGUMENT);

        options.struct_size = size_of::<NavopRdpConnectionOptions>() as u32;
        assert_eq!(unsafe { connect(host, &options) }, RESULT_ABI_MISMATCH);

        options.abi_version = ABI_VERSION;
        options.host.data = std::ptr::null();
        assert_eq!(unsafe { connect(host, &options) }, RESULT_INVALID_ARGUMENT);

        options.host.data = host_name.as_ptr();
        options.port = 0;
        assert_eq!(unsafe { connect(host, &options) }, RESULT_INVALID_ARGUMENT);

        options.port = 3389;
        options.flags = CONNECTION_FLAG_AUDIO_PLAYBACK_DISABLED;
        assert_eq!(unsafe { connect(host, &options) }, RESULT_UNAVAILABLE);

        options.flags = CONNECTION_FLAGS_KNOWN << 1;
        assert_eq!(unsafe { connect(host, &options) }, RESULT_INVALID_ARGUMENT);

        options.flags = 0;
        assert_eq!(unsafe { connect(host, &options) }, RESULT_UNAVAILABLE);

        let mut state = u32::MAX;
        assert_eq!(
            unsafe { get_connection_state(std::ptr::null_mut(), &mut state) },
            RESULT_INVALID_ARGUMENT
        );
        assert_eq!(state, CONNECTION_STATE_DISCONNECTED);

        let mut status = u32::MAX;
        assert_eq!(
            unsafe { request_close(std::ptr::null_mut(), &mut status) },
            RESULT_INVALID_ARGUMENT
        );
        assert_eq!(status, REQUEST_CLOSE_CAN_PROCEED);
    }

    #[cfg(not(windows_rdp_host_native))]
    #[test]
    fn non_windows_connection_accepts_legacy_current_and_larger_callers() {
        #[repr(C)]
        struct ExtendedConnectionOptions {
            base: NavopRdpConnectionOptions,
            trailing: [u8; 16],
        }

        let host = std::ptr::NonNull::<NativeRdpHost>::dangling().as_ptr();
        let host_name: Vec<u16> = "rdp.example".encode_utf16().collect();
        let mut options = NavopRdpConnectionOptions::current(
            NavopRdpBorrowedUtf16 {
                data: host_name.as_ptr(),
                len: host_name.len() as u32,
            },
            3389,
            1920,
            1080,
            32,
        );

        options.struct_size = if cfg!(target_pointer_width = "64") {
            48
        } else {
            36
        };
        options.legacy_reserved = u32::MAX;
        options.display_mode = u32::MAX;
        assert_eq!(unsafe { connect(host, &options) }, RESULT_UNAVAILABLE);

        options.struct_size -= 1;
        assert_eq!(unsafe { connect(host, &options) }, RESULT_INVALID_ARGUMENT);

        options = NavopRdpConnectionOptions::current(
            NavopRdpBorrowedUtf16 {
                data: host_name.as_ptr(),
                len: host_name.len() as u32,
            },
            3389,
            1920,
            1080,
            32,
        );
        options.legacy_reserved = u32::MAX;
        assert_eq!(unsafe { connect(host, &options) }, RESULT_UNAVAILABLE);

        let trailing = [0x5A; 16];
        let extended = ExtendedConnectionOptions {
            base: NavopRdpConnectionOptions {
                struct_size: size_of::<ExtendedConnectionOptions>() as u32,
                ..options
            },
            trailing,
        };
        assert_eq!(unsafe { connect(host, &extended.base) }, RESULT_UNAVAILABLE);
        assert_eq!(extended.trailing, trailing);
    }

    #[cfg(not(windows_rdp_host_native))]
    #[test]
    fn non_windows_connection_ignores_partial_append_fields_and_preserves_legacy_audio() {
        let host = std::ptr::NonNull::<NativeRdpHost>::dangling().as_ptr();
        let host_name: Vec<u16> = "rdp.example".encode_utf16().collect();
        let mut options = NavopRdpConnectionOptions::current(
            NavopRdpBorrowedUtf16 {
                data: host_name.as_ptr(),
                len: host_name.len() as u32,
            },
            3389,
            1920,
            1080,
            32,
        );

        options.display_mode = u32::MAX;
        options.struct_size = (std::mem::offset_of!(NavopRdpConnectionOptions, display_mode)
            + size_of::<u32>()
            - 1) as u32;
        assert_eq!(unsafe { connect(host, &options) }, RESULT_UNAVAILABLE);

        options = NavopRdpConnectionOptions::current(
            NavopRdpBorrowedUtf16 {
                data: host_name.as_ptr(),
                len: host_name.len() as u32,
            },
            3389,
            1920,
            1080,
            32,
        );
        options.gateway_hostname = NavopRdpBorrowedUtf16 {
            data: std::ptr::null(),
            len: 1,
        };
        options.struct_size = (std::mem::offset_of!(NavopRdpConnectionOptions, gateway_hostname)
            + size_of::<NavopRdpBorrowedUtf16>()
            - 1) as u32;
        assert_eq!(unsafe { connect(host, &options) }, RESULT_UNAVAILABLE);

        options.struct_size = if cfg!(target_pointer_width = "64") {
            48
        } else {
            36
        };
        options.flags = CONNECTION_FLAG_AUDIO_PLAYBACK_DISABLED;
        options.audio_mode = u32::MAX;
        assert_eq!(unsafe { connect(host, &options) }, RESULT_UNAVAILABLE);
    }

    #[cfg(not(windows_rdp_host_native))]
    #[test]
    fn non_windows_connection_validates_complete_policy_tail() {
        let host = std::ptr::NonNull::<NativeRdpHost>::dangling().as_ptr();
        let host_name: Vec<u16> = "rdp.example".encode_utf16().collect();
        let gateway_name: Vec<u16> = "gateway.example".encode_utf16().collect();
        let mut options = NavopRdpConnectionOptions::current(
            NavopRdpBorrowedUtf16 {
                data: host_name.as_ptr(),
                len: host_name.len() as u32,
            },
            3389,
            1920,
            1080,
            32,
        );

        macro_rules! invalid_field {
            ($field:ident, $value:expr) => {{
                let previous = options.$field;
                options.$field = $value;
                assert_eq!(
                    unsafe { connect(host, &options) },
                    RESULT_INVALID_ARGUMENT,
                    stringify!($field)
                );
                options.$field = previous;
            }};
        }

        invalid_field!(display_mode, 2);
        invalid_field!(display_flags, DISPLAY_FLAGS_KNOWN << 1);
        invalid_field!(desktop_scale_factor, 99);
        invalid_field!(desktop_scale_factor, 501);
        invalid_field!(device_scale_factor, 101);
        invalid_field!(resource_flags, RESOURCE_FLAGS_KNOWN << 1);
        invalid_field!(audio_mode, 3);
        invalid_field!(audio_quality, 3);
        invalid_field!(audio_flags, AUDIO_FLAGS_KNOWN << 1);
        invalid_field!(keyboard_hook_mode, 3);
        invalid_field!(input_flags, INPUT_FLAGS_KNOWN << 1);
        invalid_field!(performance_preset, 5);
        invalid_field!(performance_flags, PERFORMANCE_FLAGS_KNOWN << 1);
        invalid_field!(network_connection_type, 7);
        invalid_field!(security_flags, SECURITY_FLAGS_KNOWN << 1);
        invalid_field!(authentication_level, 3);
        invalid_field!(gateway_mode, 3);
        invalid_field!(gateway_flags, GATEWAY_FLAGS_KNOWN << 1);
        invalid_field!(gateway_credential_source, 2);
        invalid_field!(keep_alive_seconds, u32::MAX);
        invalid_field!(timeout_seconds, i32::MAX as u32 + 1);
        invalid_field!(connection_flags, CONNECTION_POLICY_FLAGS_KNOWN << 1);
        invalid_field!(max_reconnect_attempts, i32::MAX as u32 + 1);

        options.gateway_mode = 1;
        assert_eq!(unsafe { connect(host, &options) }, RESULT_INVALID_ARGUMENT);

        options.gateway_hostname = NavopRdpBorrowedUtf16 {
            data: std::ptr::null(),
            len: 1,
        };
        assert_eq!(unsafe { connect(host, &options) }, RESULT_INVALID_ARGUMENT);

        let gateway_with_nul: Vec<u16> = "gateway\0example".encode_utf16().collect();
        options.gateway_hostname = NavopRdpBorrowedUtf16 {
            data: gateway_with_nul.as_ptr(),
            len: gateway_with_nul.len() as u32,
        };
        assert_eq!(unsafe { connect(host, &options) }, RESULT_INVALID_ARGUMENT);

        let oversized_gateway =
            vec![b'g' as u16; crate::policy::WINDOWS_RDP_MAX_GATEWAY_HOST_UTF16_CODE_UNITS + 1];
        options.gateway_hostname = NavopRdpBorrowedUtf16 {
            data: oversized_gateway.as_ptr(),
            len: oversized_gateway.len() as u32,
        };
        assert_eq!(unsafe { connect(host, &options) }, RESULT_INVALID_ARGUMENT);

        options.gateway_hostname = NavopRdpBorrowedUtf16 {
            data: gateway_name.as_ptr(),
            len: gateway_name.len() as u32,
        };
        assert_eq!(unsafe { connect(host, &options) }, RESULT_UNAVAILABLE);
    }
}
