use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use rand::{RngCore as _, rngs::OsRng};
use semver::Version;

use crate::state::{configured_data_dir, configured_navop_version};
use crate::{InstallConfig, X11ComponentError};

pub const COMPONENT_RELATIVE_PATH: &str = "components/x11";

pub fn component_root(data_dir: &Path) -> PathBuf {
    data_dir.join(COMPONENT_RELATIVE_PATH)
}

pub fn read_installed(root: &Path) -> Option<String> {
    let version = std::fs::read_to_string(root.join("current")).ok()?;
    let version = version.trim();
    Version::parse(version).ok()?;
    root.join("versions")
        .join(version)
        .exists()
        .then(|| version.to_string())
}

pub fn write_current(root: &Path, version: &str) -> Result<(), X11ComponentError> {
    atomic_write(&root.join("current"), version.as_bytes())
}

/// 同目录临时文件配合系统原子替换，Windows 也允许覆盖旧值。
pub fn atomic_write(path: &Path, bytes: &[u8]) -> Result<(), X11ComponentError> {
    atomic_write_mode(path, bytes, false)
}

/// cookie 等机密文件从临时文件创建起就只能由所有者读写。
pub fn atomic_write_private(path: &Path, bytes: &[u8]) -> Result<(), X11ComponentError> {
    atomic_write_mode(path, bytes, true)
}

fn atomic_write_mode(path: &Path, bytes: &[u8], private: bool) -> Result<(), X11ComponentError> {
    let parent = path
        .parent()
        .ok_or_else(|| X11ComponentError::Install("目标路径缺少父目录".into()))?;
    std::fs::create_dir_all(parent).map_err(install_error)?;
    let mut random = [0; 8];
    OsRng.fill_bytes(&mut random);
    let temporary = path.with_extension(format!("tmp-{}", hex::encode(random)));
    let request = WriteRequest {
        temporary: &temporary,
        path,
        bytes,
        private,
    };
    let result = write_and_replace(request);
    if result.is_err() {
        let _ = std::fs::remove_file(&temporary);
    }
    result
}

struct WriteRequest<'a> {
    temporary: &'a Path,
    path: &'a Path,
    bytes: &'a [u8],
    private: bool,
}

fn write_and_replace(request: WriteRequest<'_>) -> Result<(), X11ComponentError> {
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    if request.private {
        use std::os::unix::fs::OpenOptionsExt as _;
        options.mode(0o600);
    }
    let mut file = options.open(request.temporary).map_err(install_error)?;
    std::io::Write::write_all(&mut file, request.bytes).map_err(install_error)?;
    file.sync_all().map_err(install_error)?;
    replace_file(request.temporary, request.path)?;
    restrict_permissions(request.path, request.private)
}

#[cfg(unix)]
fn restrict_permissions(path: &Path, private: bool) -> Result<(), X11ComponentError> {
    if private {
        use std::os::unix::fs::PermissionsExt as _;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))
            .map_err(install_error)?;
    }
    Ok(())
}

#[cfg(not(unix))]
fn restrict_permissions(_path: &Path, _private: bool) -> Result<(), X11ComponentError> {
    Ok(())
}

#[cfg(not(target_os = "windows"))]
fn replace_file(temporary: &Path, path: &Path) -> Result<(), X11ComponentError> {
    std::fs::rename(temporary, path).map_err(install_error)
}

#[cfg(target_os = "windows")]
fn replace_file(temporary: &Path, path: &Path) -> Result<(), X11ComponentError> {
    use std::os::windows::ffi::OsStrExt as _;
    let source = temporary
        .as_os_str()
        .encode_wide()
        .chain(Some(0))
        .collect::<Vec<_>>();
    let destination = path
        .as_os_str()
        .encode_wide()
        .chain(Some(0))
        .collect::<Vec<_>>();
    // SAFETY：两个 UTF-16 缓冲区均以 NUL 结尾且在本调用期间存活。
    let replaced = unsafe { MoveFileExW(source.as_ptr(), destination.as_ptr(), 0x1 | 0x8) };
    if replaced == 0 {
        return Err(install_error(std::io::Error::last_os_error()));
    }
    Ok(())
}

#[cfg(target_os = "windows")]
#[link(name = "kernel32")]
unsafe extern "system" {
    fn MoveFileExW(existing: *const u16, replacement: *const u16, flags: u32) -> i32;
}

#[cfg(test)]
pub fn prune_versions(root: &Path) -> Result<(), X11ComponentError> {
    prune_versions_preserving(root, &BTreeSet::new())
}

pub fn prune_versions_preserving(
    root: &Path,
    protected: &BTreeSet<String>,
) -> Result<(), X11ComponentError> {
    let mut protected = protected.clone();
    if let Ok(current) = std::fs::read_to_string(root.join("current")) {
        protected.insert(current.trim().to_string());
    }
    prune_directory(&root.join("versions"), &protected)
}

fn prune_directory(
    versions_root: &Path,
    protected: &BTreeSet<String>,
) -> Result<(), X11ComponentError> {
    let mut versions = std::fs::read_dir(&versions_root)
        .map_err(install_error)?
        .filter_map(Result::ok)
        .filter_map(|entry| {
            let name = entry.file_name().to_string_lossy().into_owned();
            Version::parse(&name)
                .ok()
                .map(|version| (version, entry.path()))
        })
        .collect::<Vec<_>>();
    versions.sort_by(|left, right| right.0.cmp(&left.0));
    let retained = versions
        .iter()
        .take(2)
        .map(|(version, _)| version.clone())
        .collect::<BTreeSet<_>>();
    for (version, path) in versions {
        if !protected.contains(version.to_string().as_str()) && !retained.contains(&version) {
            std::fs::remove_dir_all(path).map_err(install_error)?;
        }
    }
    Ok(())
}

pub fn default_config() -> Result<InstallConfig, X11ComponentError> {
    if crate::X11_PUBLIC_KEY_BASE64.is_empty() {
        return Err(X11ComponentError::Config(
            "X11 组件发布公钥尚未配置，已 fail-closed".into(),
        ));
    }
    Ok(InstallConfig {
        data_dir: configured_data_dir()?,
        public_key_base64: crate::X11_PUBLIC_KEY_BASE64.into(),
        navop_version: configured_navop_version()?,
        target: current_target()?.into(),
    })
}

pub fn current_target() -> Result<&'static str, X11ComponentError> {
    match (std::env::consts::OS, std::env::consts::ARCH) {
        ("windows", "x86_64") => Ok("windows-x86_64"),
        ("windows", "aarch64") => Ok("windows-aarch64"),
        ("linux", "x86_64") => Ok("linux-x86_64"),
        ("linux", "aarch64") => Ok("linux-aarch64"),
        ("macos", _) => Err(X11ComponentError::UnsupportedPlatform("macOS".into())),
        (os, arch) => Err(X11ComponentError::UnsupportedPlatform(format!(
            "{os}-{arch}"
        ))),
    }
}

fn install_error(error: std::io::Error) -> X11ComponentError {
    X11ComponentError::Install(error.to_string())
}

#[cfg(test)]
mod tests;
