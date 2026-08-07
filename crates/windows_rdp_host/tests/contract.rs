use std::fs;
use std::path::{Path, PathBuf};

const HOST_CRATE: &str = "crates/windows_rdp_host";
const ABI_VERSION: &str = "NAVOP_RDP_ABI_VERSION UINT32_C(1)";
const HOST_BUILD: &str = "cargo test --locked -p windows_rdp_host --target $RustTarget --no-run";

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
            "pub use options::WindowsRdpHostOptions;",
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
            "impl Drop for WindowsRdpHost",
            "(self.bindings.destroy)(&mut self.raw)",
        ],
    );
    for path in [
        "src/lib.rs",
        "src/ffi.rs",
        "src/handle.rs",
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
fn build_is_windows_hosted_msvc_only_and_ci_links_without_running() {
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
            HOST_BUILD,
            "Compile-only probe gate and host gate",
        ],
    );
    let script = read(script_path);
    assert_eq!(
        script
            .matches("cargo test --locked -p windows_rdp_host --target $RustTarget")
            .count(),
        1,
        "{script_path} must contain exactly one host test command, and it must be the --no-run gate"
    );
    assert_excludes_all(
        script_path,
        &[
            "cargo test --locked -p windows_rdp_host --target $RustTarget\n",
            "windows_rdp_host.exe",
        ],
    );
}
