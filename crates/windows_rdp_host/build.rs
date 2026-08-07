use std::env;
#[cfg(windows)]
use std::path::PathBuf;

fn main() {
    println!("cargo:rustc-check-cfg=cfg(windows_rdp_host_native)");
    println!("cargo:rerun-if-changed=native/credential.cpp");
    println!("cargo:rerun-if-changed=native/host.cpp");
    println!("cargo:rerun-if-changed=native/host_internal.h");
    println!("cargo:rerun-if-changed=native/windows_rdp_host.h");

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

    cc::Build::new()
        .cpp(true)
        .file("native/credential.cpp")
        .file("native/host.cpp")
        .include("native")
        .out_dir(&out_dir)
        .flag("/EHsc")
        .flag("/std:c++17")
        .flag("/permissive-")
        .flag("/W4")
        .flag("/WX")
        .compile("windows_rdp_host");

    println!("cargo:rustc-cfg=windows_rdp_host_native");
}
