use std::env;

fn main() {
    println!("cargo:rustc-check-cfg=cfg(windows_rdp_probe_native)");
    println!("cargo:rerun-if-changed=native/windows_rdp_probe.cpp");
    println!("cargo:rerun-if-changed=native/windows_rdp_probe.h");

    let target_os = required_env("CARGO_CFG_TARGET_OS");
    if target_os != "windows" {
        return;
    }

    validate_windows_target();

    #[cfg(windows)]
    build_native_probe();

    #[cfg(not(windows))]
    panic!("Windows RDP probe requires a Windows build host");
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
        "cannot build Windows RDP probe target {target} from host {host}"
    );
    assert_eq!(
        target_env, "msvc",
        "Windows RDP probe only supports MSVC targets"
    );
    assert!(
        matches!(target_arch.as_str(), "x86_64" | "x86"),
        "Windows RDP probe only supports x86_64 and x86, got {target_arch}"
    );
}

#[cfg(windows)]
fn build_native_probe() {
    cc::Build::new()
        .cpp(true)
        .file("native/windows_rdp_probe.cpp")
        .include("native")
        .flag("/EHsc")
        .flag("/std:c++17")
        .flag("/permissive-")
        .define("UNICODE", None)
        .define("_UNICODE", None)
        .compile("windows_rdp_probe");

    for library in ["atl", "ole32", "oleaut32", "user32", "uuid", "version"] {
        println!("cargo:rustc-link-lib={library}");
    }
    println!("cargo:rustc-cfg=windows_rdp_probe_native");
}
