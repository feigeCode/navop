use std::env;
#[cfg(windows)]
use std::path::{Path, PathBuf};

fn main() {
    println!("cargo:rustc-check-cfg=cfg(windows_rdp_host_native)");
    println!("cargo:rerun-if-changed=native/mstscax_import.cpp");
    println!("cargo:rerun-if-changed=native/credential.cpp");
    println!("cargo:rerun-if-changed=native/event_dispatch.cpp");
    println!("cargo:rerun-if-changed=native/event_sink.cpp");
    println!("cargo:rerun-if-changed=native/host.cpp");
    println!("cargo:rerun-if-changed=native/active_x_host.cpp");
    println!("cargo:rerun-if-changed=native/configuration.cpp");
    println!("cargo:rerun-if-changed=native/lifecycle.cpp");
    println!("cargo:rerun-if-changed=native/host_internal.h");
    println!("cargo:rerun-if-changed=native/windows_rdp_host.h");
    println!("cargo:rerun-if-env-changed=VCToolsInstallDir");

    let target_os = required_env("CARGO_CFG_TARGET_OS");
    if target_os != "windows" {
        return;
    }

    validate_windows_target();

    #[cfg(windows)]
    build_native_host();

    #[cfg(not(windows))]
    panic!("Windows RDP host requires a Windows build host");
}

fn required_env(name: &str) -> String {
    env::var(name).unwrap_or_else(|_| panic!("missing Cargo build variable {name}"))
}

fn validate_windows_target() {
    let host = required_env("HOST");
    let target = required_env("TARGET");
    let target_env = required_env("CARGO_CFG_TARGET_ENV");
    let target_arch = required_env("CARGO_CFG_TARGET_ARCH");

    assert!(
        host.contains("-windows-"),
        "cannot build Windows RDP host target {target} from host {host}"
    );
    assert_eq!(
        target_env, "msvc",
        "Windows RDP host only supports MSVC targets"
    );
    assert!(
        matches!(target_arch.as_str(), "x86_64" | "x86"),
        "Windows RDP host only supports x86_64 and x86, got {target_arch}"
    );
}

#[cfg(windows)]
fn build_native_host() {
    let out_dir = PathBuf::from(required_env("OUT_DIR"));
    generate_type_library_bindings(&out_dir);

    native_cpp_build(&out_dir)
        .file("native/credential.cpp")
        .file("native/event_dispatch.cpp")
        .file("native/event_sink.cpp")
        .file("native/host.cpp")
        .file("native/active_x_host.cpp")
        .file("native/configuration.cpp")
        .file("native/lifecycle.cpp")
        .compile("windows_rdp_host");

    let vc_tools = PathBuf::from(required_env("VCToolsInstallDir"));
    let target_arch = required_env("CARGO_CFG_TARGET_ARCH");
    let architecture = match target_arch.as_str() {
        "x86_64" => "x64",
        "x86" => "x86",
        other => panic!("unsupported ATL library architecture {other}"),
    };
    let atl_library_dir = vc_tools.join("atlmfc").join("lib").join(architecture);
    assert!(
        atl_library_dir.join("atls.lib").is_file(),
        "ATL static library not found: {}",
        atl_library_dir.join("atls.lib").display()
    );
    println!(
        "cargo:rustc-link-search=native={}",
        atl_library_dir.display()
    );
    println!("cargo:rustc-link-lib=static=atls");
    for library in ["ole32", "oleaut32", "user32", "uuid"] {
        println!("cargo:rustc-link-lib={library}");
    }
    println!("cargo:rustc-cfg=windows_rdp_host_native");
}

#[cfg(windows)]
fn generate_type_library_bindings(out_dir: &Path) {
    let mut importer = native_cpp_build(out_dir);
    importer.file("native/mstscax_import.cpp");
    importer
        .try_compile_intermediates()
        .unwrap_or_else(|error| panic!("failed to import the system RDP type library: {error}"));

    let generated_header = out_dir.join("mstscax.tlh");
    assert!(
        generated_header.is_file(),
        "MSVC #import did not generate {}",
        generated_header.display()
    );
}

#[cfg(windows)]
fn native_cpp_build(out_dir: &Path) -> cc::Build {
    let mut build = cc::Build::new();
    build
        .cpp(true)
        .include("native")
        .include(out_dir)
        .out_dir(out_dir)
        .flag("/EHsc")
        .flag("/std:c++17")
        .flag("/permissive-")
        .flag("/W4")
        .flag("/WX")
        .define("UNICODE", None)
        .define("_UNICODE", None);
    build
}
