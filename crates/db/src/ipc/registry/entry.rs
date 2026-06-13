use super::IpcDriverManifest;
use std::path::{Path, PathBuf};

pub(super) fn resolve_entry_command(manifest: &mut IpcDriverManifest) {
    let Ok(exe) = std::env::current_exe() else {
        return;
    };
    let Some(exe_dir) = current_binary_dir_from_exe(&exe) else {
        return;
    };
    resolve_relative_entry_command(manifest, &exe_dir);
}

pub(super) fn resolve_relative_entry_command(manifest: &mut IpcDriverManifest, exe_dir: &Path) {
    let command = Path::new(&manifest.entry.command);
    if command.is_absolute() {
        return;
    }
    if manifest.manifest_dir.join(command).is_file() {
        return;
    }

    let Some(file_name) = command.file_name() else {
        return;
    };
    let sibling = exe_dir.join(file_name);
    if sibling.is_file() {
        manifest.entry.command = sibling.to_string_lossy().into_owned();
    }
}

pub(super) fn current_binary_dir_from_exe(exe: &Path) -> Option<PathBuf> {
    let dir = exe.parent()?;
    if dir.file_name().and_then(|name| name.to_str()) == Some("deps") {
        dir.parent().map(Path::to_path_buf)
    } else {
        Some(dir.to_path_buf())
    }
}
