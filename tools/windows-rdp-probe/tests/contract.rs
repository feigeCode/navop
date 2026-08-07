use std::fs;
use std::path::{Path, PathBuf};
#[cfg(not(windows))]
use std::process::Command;

const X64_TARGET: &str = "x86_64-pc-windows-msvc";
const X86_TARGET: &str = "i686-pc-windows-msvc";
const PROBE_BUILD: &str = "cargo build --locked -p windows-rdp-probe";
const RDP_TYPELIB_LIBID: &str = "8C11EFA1-92C3-11D1-BC1E-00C04FA31489";

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
fn native_probe_uses_supported_atl_and_rdp_interfaces() {
    assert_contains_all(
        "tools/windows-rdp-probe/native/windows_rdp_probe.cpp",
        &[
            "atlhost.h",
            &format!("#import \"libid:{RDP_TYPELIB_LIBID}\""),
            "raw_interfaces_only",
            "named_guids",
            "no_namespace",
            "exclude(\"UINT_PTR\")",
            "#include \"mstscax.tlh\"",
            "AtlAxWinInit",
            "ATLAXWIN_CLASSW",
            "AtlAxCreateControlEx",
            "945EE98E-B376-4EC2-B2E5-64C9410F93B7",
            "B2B3FA47-3F11-4148-AD24-DFF8684A16D0",
            "IMsRdpClient10",
            "CComPtr<IUnknown>",
            "QueryInterface",
            "get_Version",
            "VS_FFI_SIGNATURE",
            "dll_version_reason=",
            "module-not-loaded",
            "module-path-unavailable",
            "version-info-size-unavailable",
            "version-info-read-failed",
            "fixed-info-unavailable",
            "invalid-fixed-info-signature",
        ],
    );
    assert_excludes_all(
        "tools/windows-rdp-probe/native/windows_rdp_probe.cpp",
        &[
            "IMsRdpClient11",
            "IMsRdpClient12",
            "#include <mstscax.h>",
            "#include \"mstscax.h\"",
            "#import \"mstscax.dll\"",
            "#import <mstscax.dll>",
            "CLSID_MsRdpClient12",
            "ATLAXWIN_CLASS,",
            "CComPtr<IMsRdpClientNonScriptable8>",
            "struct IMsRdpClientNonScriptable8",
            "interface IMsRdpClientNonScriptable8",
        ],
    );

    let path = "tools/windows-rdp-probe/native/windows_rdp_probe.cpp";
    assert_tokens_in_scope(
        path,
        &format!("#import \"libid:{RDP_TYPELIB_LIBID}\""),
        "#include \"mstscax.tlh\"",
        &[
            "raw_interfaces_only",
            "named_guids",
            "no_namespace",
            "exclude(\"UINT_PTR\")",
        ],
    );
    assert_tokens_in_scope(
        path,
        "HWND create_host_window(HWND parent) {",
        "\n}\n\nHRESULT create_rdp_control",
        &["CreateWindowExW(", "ATLAXWIN_CLASSW"],
    );
}

#[test]
fn native_probe_uses_the_published_msrdpclient12_and_nonscriptable8_guids() {
    let path = "tools/windows-rdp-probe/native/windows_rdp_probe.cpp";

    assert_tokens_in_scope(
        path,
        "constexpr CLSID kMsRdpClient12Clsid = {",
        "\n};",
        &[
            "0x945ee98e",
            "0xb376",
            "0x4ec2",
            "{0xb2, 0xe5, 0x64, 0xc9, 0x41, 0x0f, 0x93, 0xb7}",
        ],
    );
    assert_tokens_in_scope(
        path,
        "constexpr IID kMsRdpClientNonScriptable8Iid = {",
        "\n};",
        &[
            "0xb2b3fa47",
            "0x3f11",
            "0x4148",
            "{0xad, 0x24, 0xdf, 0xf8, 0x68, 0x4a, 0x16, 0xd0}",
        ],
    );
}

#[test]
fn optional_nonscriptable8_probe_uses_an_explicit_iid_without_a_generated_vtable() {
    let path = "tools/windows-rdp-probe/native/windows_rdp_probe.cpp";

    assert_tokens_in_scope(
        path,
        "int inspect_control(IUnknown* control) {",
        "\n}\n\nint run_probe",
        &[
            "CComPtr<IMsRdpClient10>",
            "IID_PPV_ARGS(&client)",
            "CComPtr<IUnknown>",
            "QueryInterface",
            "kMsRdpClientNonScriptable8Iid",
            "Attach(",
            "query-nonscriptable8",
            "get_Version",
        ],
    );
}

#[test]
fn native_probe_initializes_and_cleans_up_in_lifecycle_order() {
    let path = "tools/windows-rdp-probe/native/windows_rdp_probe.cpp";

    assert_tokens_in_scope(
        path,
        "~ProbeResources() {",
        "\n    }\n};",
        &[
            "control.Release();",
            "container.Release();",
            "DestroyWindow(host);",
            "DestroyWindow(parent);",
            "AtlAxWinTerm();",
            "OleUninitialize();",
        ],
    );
    assert_tokens_in_scope(
        path,
        "int run_probe(ProbeResources& resources) {",
        "\n}\n\n}  // namespace",
        &[
            "OleInitialize(nullptr)",
            "AtlAxWinInit()",
            "create_parent_window()",
            "create_host_window(resources.parent)",
            "create_rdp_control(resources)",
            "inspect_control(resources.control)",
        ],
    );
}

#[test]
fn dll_version_failure_remains_non_fatal_and_diagnostic() {
    assert_tokens_in_scope(
        "tools/windows-rdp-probe/native/windows_rdp_probe.cpp",
        "int inspect_control(IUnknown* control) {",
        "\n}\n\nint run_probe",
        &[
            "read_loaded_dll_version(dll_version, dll_version_reason)",
            "\"windows-rdp-probe stage=inspect status=ok \"",
            "if (has_dll_version) {",
            "} else {",
            "\"dll_version=unavailable dll_version_reason=%s\\n\"",
            "return 0;",
        ],
    );
}

#[test]
fn temporary_probe_abi_matches_between_cpp_and_rust() {
    assert_contains_all(
        "tools/windows-rdp-probe/native/windows_rdp_probe.h",
        &["extern \"C\"", "int32_t windows_rdp_probe_run(void);"],
    );
    assert_contains_all(
        "tools/windows-rdp-probe/native/windows_rdp_probe.cpp",
        &["extern \"C\" int32_t windows_rdp_probe_run(void)"],
    );
    assert_contains_all(
        "tools/windows-rdp-probe/src/main.rs",
        &["unsafe extern \"C\"", "fn windows_rdp_probe_run() -> i32;"],
    );
}

#[test]
fn build_contract_is_windows_hosted_msvc_only() {
    assert_contains_all(
        "tools/windows-rdp-probe/build.rs",
        &[
            "CARGO_CFG_TARGET_OS",
            "CARGO_CFG_TARGET_ENV",
            "CARGO_CFG_TARGET_ARCH",
            "HOST",
            "TARGET",
            "OUT_DIR",
            "windows_rdp_probe.cpp",
            "cpp(true)",
            "include(&out_dir)",
            "windows_rdp_probe_native",
            "x86_64",
            "\"x86\"",
            "msvc",
        ],
    );
    assert_contains_all(
        "tools/windows-rdp-probe/src/main.rs",
        &[
            "cfg(windows_rdp_probe_native)",
            "windows_rdp_probe_run",
            "requires-windows-msvc-atl",
        ],
    );
}

#[test]
fn windows_setup_requires_native_desktop_atl_and_system_typelib() {
    assert_contains_all(
        "script/install-window.ps1",
        &[
            "Microsoft.VisualStudio.Workload.NativeDesktop",
            "Microsoft.VisualStudio.Component.VC.ATL",
            "$vs2022VersionRange = \"[17.0,18.0)\"",
            "Get-ScoopRoot",
            "Add-ScoopToCurrentPath",
            "buckets\\extras",
            "apps\\cmake\\current",
        ],
    );
    assert_contains_all(
        "script/build-windows-rdp-probe.ps1",
        &[
            "vswhere",
            "Microsoft.VisualStudio.Workload.NativeDesktop",
            "Microsoft.VisualStudio.Component.VC.ATL",
            "atlbase.h",
            "vcvarsall.bat",
            X64_TARGET,
            X86_TARGET,
            PROBE_BUILD,
            "$vs2022VersionRange = \"[17.0,18.0)\"",
            "chcp 65001 >nul",
            "Compile-only probe gate",
        ],
    );
    assert_excludes_all(
        "script/build-windows-rdp-probe.ps1",
        &["mstscax.h", "mstscax.dll", "regsvr32"],
    );
}

#[test]
fn ci_and_release_build_both_probe_architectures() {
    assert_contains_all(
        ".github/workflows/ci.yml",
        &[X64_TARGET, X86_TARGET, "script/build-windows-rdp-probe.ps1"],
    );
    assert_contains_all(
        ".github/workflows/release.yml",
        &[X64_TARGET, X86_TARGET, "script/build-windows-rdp-probe.ps1"],
    );
    for path in [".github/workflows/ci.yml", ".github/workflows/release.yml"] {
        assert_excludes_all(
            path,
            &["cargo run -p windows-rdp-probe", "windows-rdp-probe.exe"],
        );
    }
}

#[test]
fn probe_does_not_redistribute_or_register_mstscax() {
    let paths = [
        "tools/windows-rdp-probe/build.rs",
        "tools/windows-rdp-probe/native/windows_rdp_probe.cpp",
        "script/install-window.ps1",
        "script/build-windows-rdp-probe.ps1",
        ".github/workflows/ci.yml",
        ".github/workflows/release.yml",
    ];
    let forbidden = [
        "regsvr32",
        "RegisterServer",
        "curl mstscax.dll",
        "wget mstscax.dll",
        "Copy-Item mstscax.dll",
        "#import \"mstscax.dll\"",
        "#import <mstscax.dll>",
    ];

    for path in paths {
        assert_excludes_all(path, &forbidden);
    }
}

#[cfg(not(windows))]
#[test]
fn unsupported_host_probe_has_stable_success_output() {
    let output = Command::new(env!("CARGO_BIN_EXE_windows-rdp-probe"))
        .output()
        .expect("failed to run windows-rdp-probe");

    assert!(
        output.status.success(),
        "unsupported-host probe must exit successfully, got {:?}",
        output.status.code()
    );
    assert_eq!(
        String::from_utf8(output.stdout).expect("probe stdout must be UTF-8"),
        "windows-rdp-probe status=unsupported reason=requires-windows-msvc-atl\n"
    );
    assert!(
        output.stderr.is_empty(),
        "unsupported-host probe must not write stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}
