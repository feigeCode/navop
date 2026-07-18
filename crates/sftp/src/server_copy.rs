use crate::{FileEntry, ProgressCallback, RusshSftpClient, SftpClient, TransferCancelled};
use anyhow::Result;
use ssh::SshConnectConfig;
use std::future::Future;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ServerCopyItem {
    pub source_path: String,
    pub target_path: String,
    pub is_dir: bool,
    pub size: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CopyStrategy {
    Direct,
    Relay,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CopyPlanEntry {
    pub source_path: String,
    pub target_path: String,
    pub is_dir: bool,
    pub size: u64,
}

pub struct ServerCopyRequest {
    pub source_config: SshConnectConfig,
    pub target_config: SshConnectConfig,
    pub items: Vec<ServerCopyItem>,
    pub cancelled: Arc<AtomicBool>,
    pub progress: Arc<dyn Fn(crate::TransferProgress) + Send + Sync>,
}

pub(crate) struct CopyFileRequest<'a> {
    pub source_path: &'a str,
    pub target_path: &'a str,
    pub cancelled: Arc<AtomicBool>,
    pub completed: u64,
    pub file_size: u64,
    pub total: u64,
    pub progress: &'a (dyn Fn(crate::TransferProgress) + Send + Sync),
}

pub fn choose_copy_strategy(direct_available: bool) -> CopyStrategy {
    if direct_available {
        CopyStrategy::Direct
    } else {
        CopyStrategy::Relay
    }
}

pub fn join_copy_path(base: &str, name: &str) -> String {
    let base = base.trim_end_matches('/');
    if base.is_empty() {
        format!("/{name}")
    } else {
        format!("{base}/{name}")
    }
}

pub fn build_copy_plan(items: &[ServerCopyItem], entries: &[Vec<FileEntry>]) -> Vec<CopyPlanEntry> {
    let mut plan = Vec::new();
    for (item, descendants) in items.iter().zip(entries) {
        plan.push(CopyPlanEntry {
            source_path: item.source_path.clone(),
            target_path: item.target_path.clone(),
            is_dir: item.is_dir,
            size: if item.is_dir { 0 } else { item.size },
        });

        if item.is_dir {
            let source_root = item.source_path.trim_end_matches('/');
            let target_root = item.target_path.trim_end_matches('/');
            for entry in descendants {
                let relative = entry
                    .path
                    .strip_prefix(source_root)
                    .unwrap_or(&entry.path)
                    .trim_start_matches('/');
                let target_path = join_copy_path(target_root, relative);
                plan.push(CopyPlanEntry {
                    source_path: entry.path.clone(),
                    target_path,
                    is_dir: entry.is_dir,
                    size: entry.size,
                });
            }
        }
    }
    plan
}

pub async fn relay_copy(
    source: &mut RusshSftpClient,
    target: &mut RusshSftpClient,
    items: &[ServerCopyItem],
    cancelled: Arc<AtomicBool>,
    progress: ProgressCallback,
) -> Result<()> {
    let mut descendants = Vec::with_capacity(items.len());
    for item in items {
        if cancelled.load(Ordering::Relaxed) {
            return Err(TransferCancelled.into());
        }
        if item.is_dir {
            descendants.push(
                source
                    .list_dir_recursive(&item.source_path, cancelled.clone())
                    .await?,
            );
        } else {
            descendants.push(Vec::new());
        }
    }

    let plan = build_copy_plan(items, &descendants);
    let total = plan
        .iter()
        .filter(|entry| !entry.is_dir)
        .map(|entry| entry.size)
        .sum();
    let mut transferred = 0;

    for entry in plan.iter().filter(|entry| entry.is_dir) {
        if cancelled.load(Ordering::Relaxed) {
            return Err(TransferCancelled.into());
        }
        match target.stat(&entry.target_path).await? {
            Some(metadata) if metadata.is_dir => {}
            Some(_) => {
                target.delete(&entry.target_path, false).await?;
                target.mkdir(&entry.target_path).await?;
            }
            None => target.mkdir(&entry.target_path).await?,
        }
    }

    for entry in plan.iter().filter(|entry| !entry.is_dir) {
        if cancelled.load(Ordering::Relaxed) {
            return Err(TransferCancelled.into());
        }
        if let Some(metadata) = target.stat(&entry.target_path).await? {
            if metadata.is_dir {
                target
                    .delete_recursive(&entry.target_path, cancelled.clone(), Box::new(|_| {}))
                    .await?;
            }
        }
        source
            .copy_file_to(
                target,
                CopyFileRequest {
                    source_path: &entry.source_path,
                    target_path: &entry.target_path,
                    cancelled: cancelled.clone(),
                    completed: transferred,
                    file_size: entry.size,
                    total,
                    progress: &progress,
                },
            )
            .await?;
        transferred += entry.size;
    }

    progress(crate::TransferProgress {
        transferred,
        total,
        speed: 0.0,
        current_file: None,
        current_file_transferred: 0,
        current_file_total: 0,
    });
    Ok(())
}

pub async fn copy_between_servers(request: ServerCopyRequest) -> Result<CopyStrategy> {
    let direct_progress = request.progress.clone();
    let direct_source = request.source_config.clone();
    let direct_target = request.target_config.clone();
    let direct_items = request.items.clone();
    let direct_cancelled = request.cancelled.clone();
    run_with_fallback(
        move || async move {
            crate::direct_copy::try_direct_copy(
                direct_source,
                direct_target,
                &direct_items,
                direct_cancelled,
                move |progress| direct_progress(progress),
            )
            .await
        },
        move || async move {
            let mut source = RusshSftpClient::connect(request.source_config).await?;
            let mut target = RusshSftpClient::connect(request.target_config).await?;
            let relay_progress = request.progress;
            relay_copy(
                &mut source,
                &mut target,
                &request.items,
                request.cancelled,
                Box::new(move |progress| relay_progress(progress)),
            )
            .await
        },
    )
    .await
}

async fn run_with_fallback<D, DF, R, RF>(direct: D, relay: R) -> Result<CopyStrategy>
where
    D: FnOnce() -> DF,
    DF: Future<Output = Result<()>>,
    R: FnOnce() -> RF,
    RF: Future<Output = Result<()>>,
{
    match direct().await {
        Ok(()) => Ok(CopyStrategy::Direct),
        Err(error) if error.downcast_ref::<TransferCancelled>().is_some() => Err(error),
        Err(error) => {
            tracing::warn!(%error, "direct server copy failed; using SFTP relay");
            relay().await?;
            Ok(CopyStrategy::Relay)
        }
    }
}

#[cfg(test)]
#[path = "server_copy_tests.rs"]
mod tests;
