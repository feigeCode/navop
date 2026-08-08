use crate::{FileEntry, SftpClient, TransferCancelled};
use anyhow::Result;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

pub fn total_file_size(entries: &[FileEntry]) -> u64 {
    entries
        .iter()
        .filter(|entry| !entry.is_dir)
        .map(|entry| entry.size)
        .sum()
}

pub async fn calculate_directory_size<C>(
    client: &mut C,
    path: &str,
    cancelled: Arc<AtomicBool>,
) -> Result<u64>
where
    C: SftpClient + ?Sized,
{
    ensure_not_cancelled(&cancelled)?;
    let entries = client.list_dir_recursive(path, cancelled.clone()).await?;
    ensure_not_cancelled(&cancelled)?;
    Ok(total_file_size(&entries))
}

pub fn remote_path_is_same_or_descendant(parent: &str, candidate: &str) -> bool {
    let parent = normalized_components(parent);
    let candidate = normalized_components(candidate);
    candidate.starts_with(&parent)
}

fn normalized_components(path: &str) -> Vec<&str> {
    let mut components = Vec::new();
    for component in path.split('/') {
        match component {
            "" | "." => {}
            ".." => {
                components.pop();
            }
            value => components.push(value),
        }
    }
    components
}

fn ensure_not_cancelled(cancelled: &AtomicBool) -> Result<()> {
    if cancelled.load(Ordering::Relaxed) {
        return Err(TransferCancelled.into());
    }
    Ok(())
}

#[cfg(test)]
#[path = "file_operations_tests.rs"]
mod tests;
