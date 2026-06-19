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
    resolve_relative_command(&mut manifest.entry.command, &manifest.manifest_dir, exe_dir);
    for command in manifest.entry.commands.values_mut() {
        resolve_relative_command(command, &manifest.manifest_dir, exe_dir);
    }
}

fn resolve_relative_command(command: &mut String, manifest_dir: &Path, exe_dir: &Path) {
    let command_path = Path::new(command);
    if command_path.is_absolute() {
        return;
    }
    if manifest_dir.join(command_path).is_file() {
        return;
    }

    let Some(file_name) = command_path.file_name() else {
        return;
    };
    let sibling = exe_dir.join(file_name);
    if sibling.is_file() {
        *command = sibling.to_string_lossy().into_owned();
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
