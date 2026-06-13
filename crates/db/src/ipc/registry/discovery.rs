use one_core::storage::get_config_dir;
use std::path::{Path, PathBuf};

const DRIVER_DIR_NAME: &str = "ipc-drivers";
const ENV_DRIVER_DIR: &str = "ONETCLI_IPC_DRIVER_DIR";
const APP_SHARE_DIR: &str = "onetcli";
const DUCKDB_DRIVER_BINARY: &str = if cfg!(windows) {
    "duckdb_driver.exe"
} else {
    "duckdb_driver"
};

pub(super) fn default_driver_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    extend_env_driver_dirs(&mut dirs);
    push_unique(&mut dirs, default_user_driver_dir());
    extend_bundled_driver_dirs(&mut dirs);
    extend_dev_driver_dirs(&mut dirs);
    dirs
}

pub(super) fn default_user_driver_dir() -> PathBuf {
    get_config_dir()
        .map(|dir| dir.join(DRIVER_DIR_NAME))
        .unwrap_or_else(|_| PathBuf::from(DRIVER_DIR_NAME))
}

pub(super) fn bundled_driver_dirs_from_exe(exe: &Path) -> Vec<PathBuf> {
    let Some(exe_dir) = exe.parent() else {
        return Vec::new();
    };

    let mut dirs = Vec::new();
    push_macos_resource_dir(&mut dirs, exe_dir);
    push_unix_prefix_share_dir(&mut dirs, exe_dir);
    push_unique(&mut dirs, exe_dir.join(DRIVER_DIR_NAME));
    dirs
}

fn extend_env_driver_dirs(dirs: &mut Vec<PathBuf>) {
    if let Some(value) = std::env::var_os(ENV_DRIVER_DIR) {
        for dir in std::env::split_paths(&value) {
            push_unique(dirs, dir);
        }
    }
}

fn extend_bundled_driver_dirs(dirs: &mut Vec<PathBuf>) {
    if let Ok(exe) = std::env::current_exe() {
        for dir in bundled_driver_dirs_from_exe(&exe) {
            push_unique(dirs, dir);
        }
    }
}

fn push_macos_resource_dir(dirs: &mut Vec<PathBuf>, exe_dir: &Path) {
    if exe_dir.file_name().and_then(|name| name.to_str()) != Some("MacOS") {
        return;
    }

    let Some(contents_dir) = exe_dir.parent() else {
        return;
    };
    let Some(app_dir) = contents_dir.parent() else {
        return;
    };
    if app_dir.extension().and_then(|ext| ext.to_str()) == Some("app") {
        push_unique(dirs, contents_dir.join("Resources").join(DRIVER_DIR_NAME));
    }
}

fn push_unix_prefix_share_dir(dirs: &mut Vec<PathBuf>, exe_dir: &Path) {
    if exe_dir.file_name().and_then(|name| name.to_str()) != Some("bin") {
        return;
    }

    if let Some(prefix) = exe_dir.parent() {
        push_unique(
            dirs,
            prefix
                .join("share")
                .join(APP_SHARE_DIR)
                .join(DRIVER_DIR_NAME),
        );
    }
}

#[cfg(debug_assertions)]
fn extend_dev_driver_dirs(dirs: &mut Vec<PathBuf>) {
    let Ok(exe) = std::env::current_exe() else {
        return;
    };
    let Some(exe_dir) = super::entry::current_binary_dir_from_exe(&exe) else {
        return;
    };
    if !exe_dir.join(DUCKDB_DRIVER_BINARY).is_file() {
        return;
    }

    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let Some(workspace_dir) = manifest_dir.parent().and_then(|dir| dir.parent()) else {
        return;
    };
    push_unique(dirs, workspace_dir.join("crates").join("duckdb_driver"));
}

#[cfg(not(debug_assertions))]
fn extend_dev_driver_dirs(_dirs: &mut Vec<PathBuf>) {}

fn push_unique(dirs: &mut Vec<PathBuf>, dir: PathBuf) {
    if !dirs.iter().any(|existing| existing == &dir) {
        dirs.push(dir);
    }
}
