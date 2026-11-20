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
    match build_support::configure(
        std::env::var_os(build_support::BUNDLED_ARCHIVE_ENV),
        &target_os,
        &target_arch,
        &out_dir,
    ) {
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
