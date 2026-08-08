use super::delete_entry;
use anyhow::{Context as _, Result};
use std::fs;
use std::path::{Path, PathBuf};

pub(crate) fn copy_entry(source: &Path, destination_dir: &Path) -> Result<PathBuf> {
    let transfer = validate_transfer(source, destination_dir)?;
    copy_path(source, &transfer.destination, &transfer.file_type)?;
    Ok(transfer.destination)
}

pub(crate) fn move_entry(source: &Path, destination_dir: &Path) -> Result<PathBuf> {
    let transfer = validate_transfer(source, destination_dir)?;
    match fs::rename(source, &transfer.destination) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::CrossesDevices => {
            copy_path(source, &transfer.destination, &transfer.file_type)?;
            delete_entry(source).with_context(|| {
                format!(
                    "Copied {} to {}, but could not remove the source",
                    source.display(),
                    transfer.destination.display()
                )
            })?;
        }
        Err(error) => {
            return Err(error).with_context(|| {
                format!(
                    "Unable to move {} to {}",
                    source.display(),
                    transfer.destination.display()
                )
            });
        }
    }
    Ok(transfer.destination)
}

struct Transfer {
    destination: PathBuf,
    file_type: fs::FileType,
}

fn validate_transfer(source: &Path, destination_dir: &Path) -> Result<Transfer> {
    let metadata = fs::symlink_metadata(source)
        .with_context(|| format!("Unable to inspect {}", source.display()))?;
    let destination_root = destination_dir
        .canonicalize()
        .with_context(|| format!("Unable to open {}", destination_dir.display()))?;
    if !destination_root.is_dir() {
        anyhow::bail!("{} is not a directory", destination_dir.display());
    }
    reject_descendant_destination(source, &destination_root, &metadata)?;
    let file_name = source
        .file_name()
        .ok_or_else(|| anyhow::anyhow!("Cannot transfer workspace root"))?;
    let destination = destination_dir.join(file_name);
    ensure_destination_available(&destination)?;
    Ok(Transfer {
        destination,
        file_type: metadata.file_type(),
    })
}

fn reject_descendant_destination(
    source: &Path,
    destination_dir: &Path,
    metadata: &fs::Metadata,
) -> Result<()> {
    if !metadata.is_dir() || metadata.file_type().is_symlink() {
        return Ok(());
    }
    let canonical_source = source
        .canonicalize()
        .with_context(|| format!("Unable to open {}", source.display()))?;
    if destination_dir.starts_with(&canonical_source) {
        anyhow::bail!("Cannot paste a directory inside itself");
    }
    Ok(())
}

fn ensure_destination_available(destination: &Path) -> Result<()> {
    match fs::symlink_metadata(destination) {
        Ok(_) => anyhow::bail!("{} already exists", destination.display()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => {
            Err(error).with_context(|| format!("Unable to inspect {}", destination.display()))
        }
    }
}

fn copy_path(source: &Path, destination: &Path, file_type: &fs::FileType) -> Result<()> {
    if file_type.is_symlink() {
        return copy_symlink(source, destination);
    }
    if file_type.is_dir() {
        return copy_directory(source, destination);
    }
    fs::copy(source, destination)
        .map(|_| ())
        .with_context(|| format!("Unable to copy {}", source.display()))
}

fn copy_directory(source: &Path, destination: &Path) -> Result<()> {
    fs::create_dir(destination)
        .with_context(|| format!("Unable to create {}", destination.display()))?;
    let result = copy_directory_contents(source, destination);
    if result.is_err() {
        let _ = fs::remove_dir_all(destination);
    }
    result
}

fn copy_directory_contents(source: &Path, destination: &Path) -> Result<()> {
    for entry in
        fs::read_dir(source).with_context(|| format!("Unable to read {}", source.display()))?
    {
        let entry = entry?;
        let file_type = entry.file_type()?;
        copy_path(
            &entry.path(),
            &destination.join(entry.file_name()),
            &file_type,
        )?;
    }
    let permissions = fs::metadata(source)?.permissions();
    fs::set_permissions(destination, permissions).with_context(|| {
        format!(
            "Unable to preserve permissions for {}",
            destination.display()
        )
    })
}

#[cfg(unix)]
fn copy_symlink(source: &Path, destination: &Path) -> Result<()> {
    let target = fs::read_link(source)
        .with_context(|| format!("Unable to read symbolic link {}", source.display()))?;
    std::os::unix::fs::symlink(target, destination)
        .with_context(|| format!("Unable to copy symbolic link {}", source.display()))
}

#[cfg(windows)]
fn copy_symlink(source: &Path, destination: &Path) -> Result<()> {
    let target = fs::read_link(source)
        .with_context(|| format!("Unable to read symbolic link {}", source.display()))?;
    if source.is_dir() {
        std::os::windows::fs::symlink_dir(target, destination)
    } else {
        std::os::windows::fs::symlink_file(target, destination)
    }
    .with_context(|| format!("Unable to copy symbolic link {}", source.display()))
}
