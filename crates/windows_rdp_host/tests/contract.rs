use std::fs;
use std::path::{Path, PathBuf};

const HOST_CRATE: &str = "crates/windows_rdp_host";
const ABI_VERSION: &str = "NAVOP_RDP_ABI_VERSION UINT32_C(1)";
const HOST_TEST: &str = "cargo test --locked -p windows_rdp_host --target $RustTarget";

fn workspace_root() -> PathBuf {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));

    manifest_dir
        .ancestors()
        .find(|candidate| {
            candidate.join("Cargo.toml").is_file()
                && candidate.join("script/install-window.ps1").is_file()
                && candidate.join(".github/workflows/ci.yml").is_file()
        })
        .map(Path::to_path_buf)
        .expect("unable to locate Navop workspace root")
}

fn read(relative_path: &str) -> String {
    let path = workspace_root().join(relative_path);
    fs::read_to_string(&path).unwrap_or_else(|error| {
        panic!("failed to read {}: {error}", path.display());
    })
}

fn assert_contains_all(path: &str, required: &[&str]) {
    let contents = read(path);

    for needle in required {
        assert!(contents.contains(needle), "{path} must contain `{needle}`");
    }
}

fn assert_excludes_all(path: &str, forbidden: &[&str]) {
    let contents = read(path);

    for needle in forbidden {
        assert!(
            !contents.contains(needle),
            "{path} must not contain `{needle}`"
        );
    }
}

fn assert_tokens_in_scope(path: &str, scope_start: &str, scope_end: &str, ordered_tokens: &[&str]) {
    let contents = read(path);
    let (_, after_start) = contents
        .split_once(scope_start)
        .unwrap_or_else(|| panic!("{path} must contain scope start `{scope_start}`"));
    let (scope, _) = after_start
        .split_once(scope_end)
        .unwrap_or_else(|| panic!("{path} must contain scope end `{scope_end}`"));

    let mut remaining = scope;
    for token in ordered_tokens {
        let position = remaining
            .find(token)
            .unwrap_or_else(|| panic!("{path} scope must contain `{token}` in order"));
        remaining = &remaining[position + token.len()..];
    }
}

#[test]
fn workspace_declares_the_decoupled_host_crate() {
    assert_contains_all(
        "Cargo.toml",
        &["\"crates/windows_rdp_host\"", "\"tools/windows-rdp-probe\""],
    );
    assert_contains_all(
        &format!("{HOST_CRATE}/Cargo.toml"),
        &[
            "name = \"windows_rdp_host\"",
            "[target.'cfg(windows)'.build-dependencies]",
            "cc = \"1.2.65\"",
            "[lints]",
            "workspace = true",
        ],
    );
    assert_excludes_all(
        &format!("{HOST_CRATE}/Cargo.toml"),
        &["gpui", "remote_desktop", "remote_desktop_view", "windows ="],
    );
}

#[test]
fn c_abi_is_versioned_fixed_width_and_opaque() {
    let header = &format!("{HOST_CRATE}/native/windows_rdp_host.h");

    assert_contains_all(
        header,
        &[
            "#include <stdint.h>",
            "#include <stddef.h>",
            ABI_VERSION,
            "typedef struct NativeRdpHost NativeRdpHost;",
            "typedef int32_t NavopRdpResult;",
            "NAVOP_RDP_RESULT_OK",
            "NAVOP_RDP_RESULT_INVALID_ARGUMENT",
            "NAVOP_RDP_RESULT_ABI_MISMATCH",
            "NAVOP_RDP_RESULT_ALLOCATION_FAILED",
            "NAVOP_RDP_RESULT_INTERNAL_ERROR",
            "NAVOP_RDP_RESULT_UNAVAILABLE",
            "typedef struct NavopRdpProbeOptions",
            "typedef struct NavopRdpProbeResult",
            "typedef struct NavopRdpCreateOptions",
            "uint32_t struct_size;",
            "uint32_t abi_version;",
            "uint32_t generation_low;",
            "uint32_t generation_high;",
            "struct_size values greater than or equal to the",
            "preserve an",
            "caller-provided size",
            "leave unknown trailing fields",
            "extern \"C\"",
            "navop_rdp_probe(",
            "navop_rdp_create(",
            "NativeRdpHost** out_host",
            "navop_rdp_destroy(",
            "NativeRdpHost** host",
            "may release the native object only after",
            "leaves",
            "handle non-null",
            "retains ownership for the caller",
            "must not",
            "safe to retry",
        ],
    );
    assert_excludes_all(
        header,
        &[
            "typedef enum",
            "enum NavopRdpResult",
            "HWND",
            "IUnknown",
            "BSTR",
            "wchar_t",
            "std::",
        ],
    );
}

#[test]
fn cpp_and_rust_freeze_the_same_struct_layout() {
    assert_contains_all(
        &format!("{HOST_CRATE}/native/windows_rdp_host.h"),
        &[
            "static_assert(sizeof(NavopRdpResult) == 4)",
            "static_assert(sizeof(NavopRdpProbeOptions) == 8)",
            "static_assert(alignof(NavopRdpProbeOptions) == 4)",
            "static_assert(offsetof(NavopRdpProbeOptions, struct_size) == 0)",
            "static_assert(offsetof(NavopRdpProbeOptions, abi_version) == 4)",
            "static_assert(sizeof(NavopRdpProbeResult) == 16)",
            "static_assert(alignof(NavopRdpProbeResult) == 4)",
            "static_assert(offsetof(NavopRdpProbeResult, struct_size) == 0)",
            "static_assert(offsetof(NavopRdpProbeResult, abi_version) == 4)",
            "static_assert(offsetof(NavopRdpProbeResult, available) == 8)",
            "static_assert(offsetof(NavopRdpProbeResult, reserved) == 12)",
            "static_assert(sizeof(NavopRdpCreateOptions) == 16)",
            "static_assert(alignof(NavopRdpCreateOptions) == 4)",
            "static_assert(offsetof(NavopRdpCreateOptions, struct_size) == 0)",
            "static_assert(offsetof(NavopRdpCreateOptions, abi_version) == 4)",
            "static_assert(offsetof(NavopRdpCreateOptions, generation_low) == 8)",
            "static_assert(offsetof(NavopRdpCreateOptions, generation_high) == 12)",
        ],
    );
    assert_contains_all(
        &format!("{HOST_CRATE}/src/ffi.rs"),
        &[
            "pub(crate) const ABI_VERSION: u32 = 1;",
            "#[repr(C)]",
            "struct NavopRdpProbeOptions",
            "struct NavopRdpProbeResult",
            "struct NavopRdpCreateOptions",
            "size_of::<NavopRdpProbeOptions>()",
            "align_of::<NavopRdpProbeOptions>()",
            "size_of::<NavopRdpProbeResult>()",
            "align_of::<NavopRdpProbeResult>()",
            "size_of::<NavopRdpCreateOptions>()",
            "align_of::<NavopRdpCreateOptions>()",
        ],
    );
}

#[test]
fn event_callback_abi_is_versioned_owned_and_architecture_independent() {
    let header = &format!("{HOST_CRATE}/native/windows_rdp_host.h");

    assert_contains_all(
        header,
        &[
            "typedef struct NavopRdpEvent",
            "typedef struct NavopRdpEventCallbackOptions",
            "typedef void (*NavopRdpEventCallback)(",
            "void* context",
            "const NavopRdpEvent* event",
            "const uint8_t* payload",
            "uint32_t kind;",
            "uint32_t reserved;",
            "int32_t code;",
            "uint32_t payload_len;",
            "callback payload is borrowed only for the duration",
            "owner thread",
            "does not retain callback or callback_context",
            "no callback is in flight",
            "must not synchronously call",
            "navop_rdp_register_event_callback(",
            "navop_rdp_unregister_event_callback(",
        ],
    );
    assert_contains_all(
        &format!("{HOST_CRATE}/src/event.rs"),
        &[
            "struct OwnedNativeEvent",
            "struct EventBridge",
            "VecDeque<OwnedNativeEvent>",
            "AtomicU8",
            "Mutex<VecDeque<OwnedNativeEvent>>",
            "unsafe extern \"C\" fn native_event_callback",
            "catch_unwind",
            "payload.to_vec()",
            "event_generation != self.generation",
            "CallbackLifecycle::Closing",
            "CallbackLifecycle::Closed",
        ],
    );
}

#[test]
fn native_callback_gate_validates_before_retaining_and_closes_before_destroy() {
    let source = &format!("{HOST_CRATE}/native/host.cpp");

    assert_tokens_in_scope(
        source,
        "extern \"C\" NavopRdpResult navop_rdp_register_event_callback(",
        "\n}\n\nextern \"C\" NavopRdpResult navop_rdp_unregister_event_callback(",
        &[
            "try {",
            "host == nullptr",
            "options == nullptr",
            "callback == nullptr",
            "validate_struct_size(",
            "options->struct_size",
            "validate_abi_version(",
            "options->abi_version",
            "join_generation(",
            "generation != host->generation",
            "host->callback_state != CallbackState::Open",
            "host->callback != nullptr",
            "host->callback = callback;",
            "host->callback_context = callback_context;",
            "return NAVOP_RDP_RESULT_OK;",
            "catch (...)",
        ],
    );
    assert_tokens_in_scope(
        source,
        "extern \"C\" NavopRdpResult navop_rdp_unregister_event_callback(",
        "\n}\n\nextern \"C\" NavopRdpResult navop_rdp_destroy(",
        &[
            "try {",
            "host == nullptr",
            "close_callback_gate(host);",
            "return NAVOP_RDP_RESULT_OK;",
            "catch (...)",
        ],
    );
    assert_tokens_in_scope(
        source,
        "extern \"C\" NavopRdpResult navop_rdp_destroy(",
        "\n}",
        &[
            "NativeRdpHost* owned = *host;",
            "close_callback_gate(owned);",
            "*host = nullptr;",
            "delete owned;",
        ],
    );
}

#[test]
fn native_callback_dispatch_enforces_owner_thread_and_quiescent_close() {
    let header = &format!("{HOST_CRATE}/native/windows_rdp_host.h");
    let internal_header = &format!("{HOST_CRATE}/native/host_internal.h");
    let dispatch_source = &format!("{HOST_CRATE}/native/event_dispatch.cpp");

    assert_contains_all(
        header,
        &[
            "NAVOP_RDP_RESULT_WRONG_THREAD",
            "NAVOP_RDP_RESULT_CALLBACK_IN_FLIGHT",
            "Wrong-thread calls",
            "callback is in flight",
            "preserve",
            "later owner-thread turn",
        ],
    );
    assert_contains_all(
        internal_header,
        &[
            "uint32_t owner_thread_id;",
            "uint32_t callbacks_in_flight;",
            "ensure_owner_thread(",
            "close_callback_gate(",
            "dispatch_event(",
        ],
    );
    assert_contains_all(
        dispatch_source,
        &[
            "#include <windows.h>",
            "class CallbackDispatchScope",
            "host_->callbacks_in_flight += UINT32_C(1);",
            "host_->callbacks_in_flight -= UINT32_C(1);",
            "GetCurrentThreadId()",
            "NAVOP_RDP_RESULT_WRONG_THREAD",
            "NAVOP_RDP_RESULT_CALLBACK_IN_FLIGHT",
            "host->callback_state != CallbackState::Open",
            "host->callback == nullptr",
            "host->callbacks_in_flight == UINT32_MAX",
            "NavopRdpEventCallback callback = host->callback;",
            "void* callback_context = host->callback_context;",
            "CallbackDispatchScope callback_scope(host);",
            "callback(callback_context, event, payload);",
            "host->callback = nullptr;",
            "host->callback_context = nullptr;",
            "host->callback_state = CallbackState::Closed;",
            "extern \"C\" NavopRdpResult navop_rdp_test_dispatch_event(",
            "try {",
            "catch (...)",
        ],
    );
    assert_excludes_all(
        header,
        &["navop_rdp_test_dispatch_event(", "callbacks_in_flight"],
    );
    assert_contains_all(
        &format!("{HOST_CRATE}/native/host.cpp"),
        &[
            "GetCurrentThreadId()",
            "ensure_owner_thread(host)",
            "NavopRdpResult close_result = close_callback_gate(owned);",
            "if (close_result != NAVOP_RDP_RESULT_OK)",
        ],
    );
    assert_contains_all(
        &format!("{HOST_CRATE}/native/credential.cpp"),
        &["ensure_owner_thread(host)"],
    );
    assert_contains_all(
        &format!("{HOST_CRATE}/src/ffi.rs"),
        &["RESULT_WRONG_THREAD", "RESULT_CALLBACK_IN_FLIGHT"],
    );
    assert_contains_all(
        &format!("{HOST_CRATE}/src/error.rs"),
        &[
            "WrongThread",
            "CallbackInFlight",
            "ffi::RESULT_WRONG_THREAD",
            "ffi::RESULT_CALLBACK_IN_FLIGHT",
        ],
    );
    assert_contains_all(
        &format!("{HOST_CRATE}/src/native_tests.rs"),
        &[
            "native_dispatch_invokes_the_registered_callback_once",
            "reentrant_unregister_is_rejected_until_callback_returns",
            "reentrant_destroy_preserves_the_handle_until_callback_returns",
            "wrong_thread_dispatch_unregister_and_destroy_are_rejected",
            "native_dispatch_rejects_invalid_events_without_poisoning_callback",
            "navop_rdp_test_dispatch_event",
        ],
    );
    assert_contains_all(
        &format!("{HOST_CRATE}/build.rs"),
        &[
            "cargo:rerun-if-changed=native/event_dispatch.cpp",
            ".file(\"native/event_dispatch.cpp\")",
        ],
    );
}

#[test]
fn event_callback_layout_is_frozen_without_pointer_sized_struct_fields() {
    assert_contains_all(
        &format!("{HOST_CRATE}/native/windows_rdp_host.h"),
        &[
            "static_assert(sizeof(NavopRdpEvent) == 32)",
            "static_assert(alignof(NavopRdpEvent) == 4)",
            "static_assert(offsetof(NavopRdpEvent, struct_size) == 0)",
            "static_assert(offsetof(NavopRdpEvent, abi_version) == 4)",
            "static_assert(offsetof(NavopRdpEvent, kind) == 8)",
            "static_assert(offsetof(NavopRdpEvent, reserved) == 12)",
            "static_assert(offsetof(NavopRdpEvent, generation_low) == 16)",
            "static_assert(offsetof(NavopRdpEvent, generation_high) == 20)",
            "static_assert(offsetof(NavopRdpEvent, code) == 24)",
            "static_assert(offsetof(NavopRdpEvent, payload_len) == 28)",
            "static_assert(sizeof(NavopRdpEventCallbackOptions) == 16)",
            "static_assert(alignof(NavopRdpEventCallbackOptions) == 4)",
        ],
    );
    assert_contains_all(
        &format!("{HOST_CRATE}/src/ffi.rs"),
        &[
            "struct NavopRdpEvent",
            "struct NavopRdpEventCallbackOptions",
            "size_of::<NavopRdpEvent>()",
            "align_of::<NavopRdpEvent>()",
            "size_of::<NavopRdpEventCallbackOptions>()",
            "align_of::<NavopRdpEventCallbackOptions>()",
            "const _: () = {",
            "unsafe extern \"C\" fn(",
            "register_event_callback",
            "unregister_event_callback",
        ],
    );
}

#[test]
fn credential_transport_is_versioned_borrowed_and_architecture_specific() {
    let header = &format!("{HOST_CRATE}/native/windows_rdp_host.h");

    assert_contains_all(
        header,
        &[
            "typedef struct NavopRdpBorrowedSecret",
            "const uint16_t* data;",
            "uint32_t len;",
            "typedef struct NavopRdpCredentialBundle",
            "uint32_t struct_size;",
            "uint32_t abi_version;",
            "NavopRdpBorrowedSecret server_password;",
            "NavopRdpBorrowedSecret gateway_password;",
            "uint32_t flags;",
            "borrowed only for the synchronous call",
            "must not retain",
            "navop_rdp_apply_credentials(",
            "const NavopRdpCredentialBundle* credentials",
            "INTPTR_MAX == INT64_MAX",
            "sizeof(NavopRdpBorrowedSecret) == 16",
            "alignof(NavopRdpBorrowedSecret) == 8",
            "offsetof(NavopRdpBorrowedSecret, data) == 0",
            "offsetof(NavopRdpBorrowedSecret, len) == 8",
            "sizeof(NavopRdpCredentialBundle) == 48",
            "alignof(NavopRdpCredentialBundle) == 8",
            "offsetof(NavopRdpCredentialBundle, struct_size) == 0",
            "offsetof(NavopRdpCredentialBundle, abi_version) == 4",
            "offsetof(NavopRdpCredentialBundle, server_password) == 8",
            "offsetof(NavopRdpCredentialBundle, gateway_password) == 24",
            "offsetof(NavopRdpCredentialBundle, flags) == 40",
            "INTPTR_MAX == INT32_MAX",
            "sizeof(NavopRdpBorrowedSecret) == 8",
            "alignof(NavopRdpBorrowedSecret) == 4",
            "offsetof(NavopRdpBorrowedSecret, len) == 4",
            "sizeof(NavopRdpCredentialBundle) == 28",
            "alignof(NavopRdpCredentialBundle) == 4",
            "offsetof(NavopRdpCredentialBundle, server_password) == 8",
            "offsetof(NavopRdpCredentialBundle, gateway_password) == 16",
            "offsetof(NavopRdpCredentialBundle, flags) == 24",
        ],
    );
    assert_contains_all(
        &format!("{HOST_CRATE}/src/ffi.rs"),
        &[
            "struct NavopRdpBorrowedSecret",
            "data: *const u16",
            "len: u32",
            "struct NavopRdpCredentialBundle",
            "server_password: NavopRdpBorrowedSecret",
            "gateway_password: NavopRdpBorrowedSecret",
            "type ApplyCredentialsFn",
            "apply_credentials: ApplyCredentialsFn",
            "navop_rdp_apply_credentials(",
            "target_pointer_width = \"64\"",
            "size_of::<NavopRdpBorrowedSecret>() == 16",
            "size_of::<NavopRdpCredentialBundle>() == 48",
            "target_pointer_width = \"32\"",
            "size_of::<NavopRdpBorrowedSecret>() == 8",
            "size_of::<NavopRdpCredentialBundle>() == 28",
        ],
    );
}

#[test]
fn rust_credentials_are_zeroizing_redacted_and_not_persisted_in_the_host() {
    assert_contains_all(
        &format!("{HOST_CRATE}/Cargo.toml"),
        &["[dependencies]", "zeroize.workspace = true"],
    );
    assert_contains_all(
        &format!("{HOST_CRATE}/src/credential.rs"),
        &[
            "pub struct WindowsRdpCredentialBundle",
            "Zeroizing<Vec<u16>>",
            "Zeroizing::new(password)",
            "encode_utf16()",
            "impl fmt::Debug for WindowsRdpCredentialBundle",
            "\"<redacted",
            "NavopRdpBorrowedSecret",
            "NavopRdpCredentialBundle",
            "u32::try_from",
        ],
    );
    assert_excludes_all(
        &format!("{HOST_CRATE}/src/credential.rs"),
        &[
            "derive(Clone",
            "derive(Serialize",
            "derive(Deserialize",
            "impl Clone for WindowsRdpCredentialBundle",
            "impl serde::Serialize",
            "impl serde::Deserialize",
            "use serde",
            "serde::",
            "log::",
            "tracing::",
            "println!",
            "eprintln!",
        ],
    );
    assert_contains_all(
        &format!("{HOST_CRATE}/src/handle.rs"),
        &[
            "pub fn apply_credentials(",
            "credentials.as_native()",
            "(self.bindings.apply_credentials)(self.raw, &native_credentials)",
            "HostLifecycle::Open",
        ],
    );
    assert_excludes_all(
        &format!("{HOST_CRATE}/src/handle.rs"),
        &[
            "server_password:",
            "gateway_password:",
            "credentials: WindowsRdpCredentialBundle",
        ],
    );
}

#[test]
fn native_credentials_validate_copy_and_wipe_on_every_exit_path() {
    let source = &format!("{HOST_CRATE}/native/credential.cpp");

    assert_contains_all(
        source,
        &[
            "#include <windows.h>",
            "class SensitiveUtf16Buffer",
            "~SensitiveUtf16Buffer() noexcept",
            "SecureZeroMemory(",
            "delete[]",
            "std::memcpy(",
            "std::nothrow",
            "validate_borrowed_secret(",
            "secret.len == UINT32_C(0)",
            "secret.data == nullptr",
            "(std::numeric_limits<size_t>::max)()",
        ],
    );
    assert_tokens_in_scope(
        source,
        "extern \"C\" NavopRdpResult navop_rdp_apply_credentials(",
        "\n}",
        &[
            "try {",
            "host == nullptr",
            "credentials == nullptr",
            "validate_struct_size(",
            "credentials->struct_size",
            "validate_abi_version(",
            "credentials->abi_version",
            "credentials->flags != UINT32_C(0)",
            "host->callback_state != CallbackState::Open",
            "validate_borrowed_secret(credentials->server_password)",
            "validate_borrowed_secret(credentials->gateway_password)",
            "SensitiveUtf16Buffer server_password;",
            "SensitiveUtf16Buffer gateway_password;",
            "server_password.copy_from(credentials->server_password)",
            "gateway_password.copy_from(credentials->gateway_password)",
            "return NAVOP_RDP_RESULT_OK;",
            "catch (...)",
            "NAVOP_RDP_RESULT_INTERNAL_ERROR",
        ],
    );
    assert_excludes_all(
        source,
        &[
            "std::wstring",
            "std::u16string",
            "wcslen",
            "lstrlenW",
            "ClearTextPassword",
            "GatewayPassword",
            "CComBSTR",
            "BSTR",
            "IMsRdp",
            "AtlAx",
        ],
    );
    assert_contains_all(
        &format!("{HOST_CRATE}/build.rs"),
        &[
            "cargo:rerun-if-changed=native/credential.cpp",
            ".file(\"native/credential.cpp\")",
        ],
    );
}

#[test]
fn credentials_do_not_expand_options_events_errors_or_dump_collection() {
    assert_excludes_all(
        &format!("{HOST_CRATE}/src/options.rs"),
        &["password", "credential", "secret"],
    );
    assert_excludes_all(
        &format!("{HOST_CRATE}/src/event.rs"),
        &["password", "credential", "secret"],
    );
    assert_excludes_all(
        &format!("{HOST_CRATE}/src/error.rs"),
        &["password", "credential", "secret"],
    );
    for path in [
        "src/lib.rs",
        "src/ffi.rs",
        "src/credential.rs",
        "src/handle.rs",
        "native/windows_rdp_host.h",
        "native/host.cpp",
        "native/credential.cpp",
        "build.rs",
    ] {
        assert_excludes_all(
            &format!("{HOST_CRATE}/{path}"),
            &[
                "MiniDumpWriteDump",
                "MiniDumpWithFullMemory",
                "WER_DUMP_TYPE",
                "DumpType = 2",
            ],
        );
    }
}

#[test]
fn native_entrypoints_validate_headers_and_contain_failures() {
    let source = &format!("{HOST_CRATE}/native/host.cpp");

    assert_tokens_in_scope(
        source,
        "extern \"C\" NavopRdpResult navop_rdp_probe(",
        "\n}\n\nextern \"C\" NavopRdpResult navop_rdp_create(",
        &[
            "try {",
            "options == nullptr",
            "out_result == nullptr",
            "const uint32_t caller_result_size = out_result->struct_size;",
            "validate_struct_size(",
            "options->struct_size",
            "validate_abi_version(",
            "options->abi_version",
            "validate_struct_size(",
            "caller_result_size",
            "validate_abi_version(",
            "out_result->abi_version",
            "out_result->struct_size = caller_result_size;",
            "catch (...)",
            "NAVOP_RDP_RESULT_INTERNAL_ERROR",
        ],
    );
    assert_tokens_in_scope(
        source,
        "extern \"C\" NavopRdpResult navop_rdp_create(",
        "\n}\n\nextern \"C\" NavopRdpResult navop_rdp_destroy(",
        &[
            "try {",
            "out_host == nullptr",
            "*out_host = nullptr;",
            "options == nullptr",
            "validate_struct_size(",
            "options->struct_size",
            "validate_abi_version(",
            "options->abi_version",
            "new (std::nothrow) NativeRdpHost",
            "NAVOP_RDP_RESULT_ALLOCATION_FAILED",
            "*out_host = host;",
            "catch (...)",
            "NAVOP_RDP_RESULT_INTERNAL_ERROR",
        ],
    );
    assert_tokens_in_scope(
        source,
        "extern \"C\" NavopRdpResult navop_rdp_destroy(",
        "\n}",
        &[
            "try {",
            "host == nullptr",
            "*host == nullptr",
            "NativeRdpHost* owned = *host;",
            "*host = nullptr;",
            "delete owned;",
            "catch (...)",
            "NAVOP_RDP_RESULT_INTERNAL_ERROR",
        ],
    );
    assert_excludes_all(
        source,
        &[
            "validate_header(",
            "OleInitialize",
            "AtlAx",
            "mstscax",
            "HWND",
            "CComPtr",
        ],
    );
}

#[test]
fn rust_facade_owns_only_the_opaque_handle_and_uses_idempotent_destroy() {
    assert_contains_all(
        &format!("{HOST_CRATE}/src/lib.rs"),
        &[
            "pub use capabilities::WindowsRdpHostCapabilities;",
            "pub use error::WindowsRdpHostError;",
            "pub use handle::WindowsRdpHost;",
            "pub use lifecycle::WindowsRdpHostLifecycle;",
            "pub use options::WindowsRdpHostOptions;",
            "mod event;",
            "mod lifecycle;",
        ],
    );
    assert_contains_all(
        &format!("{HOST_CRATE}/src/handle.rs"),
        &[
            "pub struct WindowsRdpHost",
            "raw: *mut NativeRdpHost",
            "pub fn probe()",
            "pub fn create(",
            "pub fn close(&mut self)",
            "pub const fn lifecycle(&self) -> WindowsRdpHostLifecycle",
            "impl Drop for WindowsRdpHost",
            "(self.bindings.destroy)(&mut self.raw)",
            "WindowsRdpHostLifecycle::Open",
            "WindowsRdpHostLifecycle::Closing",
            "WindowsRdpHostLifecycle::Closed",
            "begin_closing",
            "unregister_event_callback",
            "close_retries_unregister_then_destroy_failures_without_reopening_callback_gate",
            "registration_failure_preserves_original_error_when_destroy_does_not_clear_handle",
            "drop_preserves_callback_context_when_unregister_keeps_failing",
            "if self.close().is_err() && self.callback_registered",
            "Box::leak(event_bridge)",
        ],
    );
    assert_contains_all(
        &format!("{HOST_CRATE}/src/lifecycle.rs"),
        &[
            "pub enum WindowsRdpHostLifecycle",
            "Open",
            "Closing",
            "Closed",
            "callback admission",
        ],
    );
    for path in [
        "src/lib.rs",
        "src/ffi.rs",
        "src/handle.rs",
        "src/lifecycle.rs",
        "src/event.rs",
        "src/options.rs",
        "src/capabilities.rs",
        "src/error.rs",
    ] {
        assert_excludes_all(
            &format!("{HOST_CRATE}/{path}"),
            &[
                "HWND",
                "IUnknown",
                "BSTR",
                "CComPtr",
                "gpui",
                "remote_desktop_view",
            ],
        );
    }
}

#[test]
fn build_is_windows_hosted_msvc_only_and_ci_runs_non_activex_host_tests() {
    assert_contains_all(
        &format!("{HOST_CRATE}/build.rs"),
        &[
            "cargo:rustc-check-cfg=cfg(windows_rdp_host_native)",
            "CARGO_CFG_TARGET_OS",
            "CARGO_CFG_TARGET_ENV",
            "CARGO_CFG_TARGET_ARCH",
            "HOST",
            "TARGET",
            "OUT_DIR",
            "host.cpp",
            "cpp(true)",
            "/std:c++17",
            "/EHsc",
            "/W4",
            "/WX",
            "windows_rdp_host_native",
            "x86_64",
            "x86",
            "msvc",
        ],
    );
    assert_excludes_all(
        &format!("{HOST_CRATE}/build.rs"),
        &["atls", "ole32", "mstscax", "AtlAx"],
    );
    let script_path = "script/build-windows-rdp-probe.ps1";
    assert_contains_all(
        script_path,
        &[
            "cargo build --locked -p windows-rdp-probe --target $RustTarget",
            HOST_TEST,
            "Compile-only probe gate and native host runtime tests",
        ],
    );
    let script = read(script_path);
    assert_eq!(
        script
            .matches("cargo test --locked -p windows_rdp_host --target $RustTarget")
            .count(),
        1,
        "{script_path} must contain exactly one host test command"
    );
    assert_excludes_all(
        script_path,
        &[
            "cargo test --locked -p windows_rdp_host --target $RustTarget --no-run",
            "windows_rdp_host.exe",
            "windows-rdp-probe.exe",
        ],
    );
}
