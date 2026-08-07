use std::env;
#[cfg(windows)]
use std::path::PathBuf;

fn main() {
    println!("cargo:rustc-check-cfg=cfg(windows_rdp_probe_native)");
    println!("cargo:rerun-if-changed=native/windows_rdp_probe.cpp");
    println!("cargo:rerun-if-changed=native/windows_rdp_probe.h");
    println!("cargo:rerun-if-env-changed=VCToolsInstallDir");

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
fn atl_library_dir() -> PathBuf {
    let vc_tools = PathBuf::from(required_env("VCToolsInstallDir"));
    let target_arch = required_env("CARGO_CFG_TARGET_ARCH");
    let architecture = match target_arch.as_str() {
        "x86_64" => "x64",
        "x86" => "x86",
        other => panic!("unsupported ATL library architecture {other}"),
    };
    let library_dir = vc_tools.join("atlmfc").join("lib").join(architecture);
    let atl_library = library_dir.join("atl.lib");

    assert!(
        atl_library.is_file(),
        "ATL library not found: {}",
        atl_library.display()
    );
    library_dir
}

#[cfg(windows)]
fn build_native_probe() {
    let out_dir = PathBuf::from(required_env("OUT_DIR"));
    let atl_library_dir = atl_library_dir();

    cc::Build::new()
        .cpp(true)
        .file("native/windows_rdp_probe.cpp")
        .include("native")
        .include(&out_dir)
        .out_dir(&out_dir)
        .flag("/EHsc")
        .flag("/std:c++17")
        .flag("/permissive-")
        .define("UNICODE", None)
        .define("_UNICODE", None)
        .compile("windows_rdp_probe");

    println!(
        "cargo:rustc-link-search=native={}",
        atl_library_dir.display()
    );
    for library in ["atl", "ole32", "oleaut32", "user32", "uuid", "version"] {
        println!("cargo:rustc-link-lib={library}");
    }
    println!("cargo:rustc-cfg=windows_rdp_probe_native");
}
