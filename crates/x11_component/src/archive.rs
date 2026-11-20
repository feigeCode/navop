use std::io::{Cursor, Read as _};
use std::path::{Path, PathBuf};

use zip::ZipArchive;

use crate::X11ComponentError;
use crate::manifest::{Artifact, ArtifactFormat};

const MAX_ARCHIVE_ENTRIES: usize = 4096;
const MAX_UNPACKED_BYTES: u64 = 512 * 1024 * 1024;

pub fn unpack(
    payload: &[u8],
    artifact: &Artifact,
    staging: &Path,
) -> Result<PathBuf, X11ComponentError> {
    match artifact.format {
        ArtifactFormat::Binary => write_binary(payload, artifact, staging)?,
        ArtifactFormat::Zip => unpack_zip(payload, staging)?,
    }
    let executable = staging.join(artifact.executable_relative_path());
    if !executable.is_file() {
        return Err(install("发布物缺少 executable_path 指向的文件"));
    }
    make_executable(&executable)?;
    Ok(executable)
}

fn write_binary(
    payload: &[u8],
    artifact: &Artifact,
    staging: &Path,
) -> Result<(), X11ComponentError> {
    let path = staging.join(artifact.executable_relative_path());
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(io_error)?;
    }
    std::fs::write(path, payload).map_err(io_error)
}

fn unpack_zip(payload: &[u8], staging: &Path) -> Result<(), X11ComponentError> {
    let mut archive = ZipArchive::new(Cursor::new(payload)).map_err(zip_error)?;
    if archive.len() > MAX_ARCHIVE_ENTRIES {
        return Err(install("zip 条目数超过安全上限"));
    }
    let mut total = 0_u64;
    for index in 0..archive.len() {
        let mut entry = archive.by_index(index).map_err(zip_error)?;
        total = total
            .checked_add(entry.size())
            .filter(|size| *size <= MAX_UNPACKED_BYTES)
            .ok_or_else(|| install("zip 展开大小超过安全上限"))?;
        extract_entry(&mut entry, staging)?;
    }
    Ok(())
}

fn extract_entry(
    entry: &mut zip::read::ZipFile<'_>,
    staging: &Path,
) -> Result<(), X11ComponentError> {
    if entry.name().contains('\\') || is_symlink(entry.unix_mode()) {
        return Err(install("zip 包含不兼容路径或符号链接"));
    }
    let relative = entry
        .enclosed_name()
        .ok_or_else(|| install("zip 包含目录穿越"))?;
    let destination = staging.join(relative);
    if entry.is_dir() {
        return std::fs::create_dir_all(destination).map_err(io_error);
    }
    if !entry.is_file() {
        return Err(install("zip 包含特殊文件"));
    }
    if let Some(parent) = destination.parent() {
        std::fs::create_dir_all(parent).map_err(io_error)?;
    }
    let mut output = std::fs::File::create(destination).map_err(io_error)?;
    let expected = entry.size();
    let copied = std::io::copy(&mut entry.take(expected + 1), &mut output).map_err(io_error)?;
    if copied != expected {
        return Err(install("zip 条目展开长度异常"));
    }
    Ok(())
}

fn is_symlink(mode: Option<u32>) -> bool {
    mode.is_some_and(|mode| mode & 0o170000 == 0o120000)
}

fn make_executable(path: &Path) -> Result<(), X11ComponentError> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        let mut permissions = std::fs::metadata(path).map_err(io_error)?.permissions();
        permissions.set_mode(0o700);
        std::fs::set_permissions(path, permissions).map_err(io_error)?;
    }
    Ok(())
}

fn zip_error(error: zip::result::ZipError) -> X11ComponentError {
    install(error.to_string())
}

fn io_error(error: std::io::Error) -> X11ComponentError {
    install(error.to_string())
}

fn install(reason: impl Into<String>) -> X11ComponentError {
    X11ComponentError::Install(reason.into())
}
