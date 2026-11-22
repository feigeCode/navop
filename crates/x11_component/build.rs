#[path = "build_support.rs"]
mod build_support;

fn main() {
    println!(
        "cargo:rerun-if-env-changed={}",
        build_support::BUNDLED_ARCHIVE_ENV
    );
    println!("cargo:rerun-if-env-changed=NAVOP_X11_PUBLIC_KEY_BASE64");
    println!(
        "cargo:rerun-if-changed=../../resources/windows/x11-component/navop-x11-manifest.json"
    );
    println!("cargo:rerun-if-changed=../../resources/windows/x11-component/public-key.txt");

    let out_dir = std::path::PathBuf::from(std::env::var_os("OUT_DIR").expect("OUT_DIR 缺失"));
    let target_os = std::env::var("CARGO_CFG_TARGET_OS").expect("CARGO_CFG_TARGET_OS 缺失");
    let target_arch = std::env::var("CARGO_CFG_TARGET_ARCH").expect("CARGO_CFG_TARGET_ARCH 缺失");
    // 内嵌组件由 `bundled` 特性显式开启：未开启时即使环境变量在，也一律忽略，
    // 避免发布流水线残留的 env 把组件意外塞进产物。
    let archive = if std::env::var_os("CARGO_FEATURE_BUNDLED").is_some() {
        std::env::var_os(build_support::BUNDLED_ARCHIVE_ENV)
    } else {
        if std::env::var_os(build_support::BUNDLED_ARCHIVE_ENV).is_some() {
            println!(
                "cargo:warning={} 已设置，但未启用 x11_component 的 bundled 特性，本次不内嵌 X11 组件",
                build_support::BUNDLED_ARCHIVE_ENV
            );
        }
        None
    };
    match build_support::configure(archive, &target_os, &target_arch, &out_dir) {
        Ok(Some(build)) => {
            println!("cargo:rerun-if-changed={}", build.source.display());
            println!(
                "cargo:rustc-env={}={}",
                build_support::BUNDLED_TARGET_ENV,
                build.target
            );
        }
        Ok(None) => {
            println!("cargo:rustc-env={}=", build_support::BUNDLED_TARGET_ENV);
        }
        Err(error) => panic!("{error}"),
    }
}
