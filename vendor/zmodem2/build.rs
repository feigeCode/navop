// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2017-2020 Alexey Arbuzov
// Copyright (c) 2023-2025 Jarkko Sakkinen

use std::env;
use std::path::PathBuf;

fn main() {
    println!("cargo::rustc-check-cfg=cfg(has_lrzsz)");

    let rz = find_prog("ZMODEM_RZ_BIN", &["rz", "lrzsz-rz"]);
    let sz = find_prog("ZMODEM_SZ_BIN", &["sz", "lrzsz-sz"]);

    match (rz, sz) {
        (Some(rz_path), Some(sz_path)) => {
            println!("cargo:rustc-cfg=has_lrzsz");
            println!("cargo:rustc-env=ZMODEM_RZ_BIN={}", rz_path.display());
            println!("cargo:rustc-env=ZMODEM_SZ_BIN={}", sz_path.display());
        }
        _ => {
            println!("cargo:warning=lrzsz not found");
        }
    }
}

fn find_prog(env_var: &str, candidates: &[&str]) -> Option<PathBuf> {
    if let Ok(path) = env::var(env_var) {
        let p = PathBuf::from(path);
        if p.is_file() {
            return Some(p);
        }
    }

    let path = env::var_os("PATH")?;
    for dir in env::split_paths(&path) {
        for &name in candidates {
            let full = dir.join(name);
            if full.is_file() {
                return Some(full);
            }
        }
    }

    None
}
