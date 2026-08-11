use crate::server_copy_direct::{execute_direct_copy, prepare_direct_copy};
use crate::{
    DirectoryConflictPolicy, FileEntry, ProgressCallback, RusshSftpClient, SftpClient,
    TransferCancelled,
};
use anyhow::Result;
use ssh::{SshConnectConfig, SshSessionManager};
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ServerCopyItem {
    pub source_path: String,
    pub target_path: String,
    pub is_dir: bool,
    pub size: u64,
    pub directory_conflict_policy: DirectoryConflictPolicy,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CopyPlanEntry {
    pub source_path: String,
    pub target_path: String,
    pub is_dir: bool,
    pub size: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DirectCopyStrategy {
    Rsync,
    Scp,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DirectCopyDecision {
    UseDirect,
    UseRelay,
    Cancel,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DirectCopyPreview {
    pub strategy: DirectCopyStrategy,
    pub source_host: String,
    pub source_port: u16,
    pub source_username: String,
    pub target_host: String,
    pub target_port: u16,
    pub target_username: String,
    pub item_count: usize,
}

impl DirectCopyPreview {
    fn new(
        strategy: DirectCopyStrategy,
        source: &SshConnectConfig,
        target: &SshConnectConfig,
        item_count: usize,
    ) -> Self {
        Self {
            strategy,
            source_host: source.host.clone(),
            source_port: source.port,
            source_username: source.username.clone(),
            target_host: target.host.clone(),
            target_port: target.port,
            target_username: target.username.clone(),
            item_count,
        }
    }
}

pub type DirectCopyApprovalFuture =
    Pin<Box<dyn Future<Output = DirectCopyDecision> + Send + 'static>>;
pub type DirectCopyApproval =
    Arc<dyn Fn(DirectCopyPreview) -> DirectCopyApprovalFuture + Send + Sync + 'static>;

pub struct ServerCopyRequest {
    pub source_config: SshConnectConfig,
    pub target_config: SshConnectConfig,
    pub items: Vec<ServerCopyItem>,
    pub cancelled: Arc<AtomicBool>,
    pub progress: Arc<dyn Fn(crate::TransferProgress) + Send + Sync>,
    pub direct_copy_approval: Option<DirectCopyApproval>,
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
        plan.extend(build_item_copy_plan(item, descendants, &item.target_path));
    }
    plan
}

fn build_item_copy_plan(
    item: &ServerCopyItem,
    descendants: &[FileEntry],
    target_root: &str,
) -> Vec<CopyPlanEntry> {
    let mut plan = vec![CopyPlanEntry {
        source_path: item.source_path.clone(),
        target_path: target_root.to_owned(),
        is_dir: item.is_dir,
        size: if item.is_dir { 0 } else { item.size },
    }];

    if item.is_dir {
        let source_root = item.source_path.trim_end_matches('/');
        let target_root = target_root.trim_end_matches('/');
        for entry in descendants {
            let relative = entry
                .path
                .strip_prefix(source_root)
                .unwrap_or(&entry.path)
                .trim_start_matches('/');
            plan.push(CopyPlanEntry {
                source_path: entry.path.clone(),
                target_path: join_copy_path(target_root, relative),
                is_dir: entry.is_dir,
                size: entry.size,
            });
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

    let total = build_copy_plan(items, &descendants)
        .iter()
        .filter(|entry| !entry.is_dir)
        .map(|entry| entry.size)
        .sum();
    let mut transferred = 0;

    for (item, item_descendants) in items.iter().zip(&descendants) {
        ensure_not_cancelled(&cancelled)?;
        let mut replacement =
            if item.is_dir && item.directory_conflict_policy == DirectoryConflictPolicy::Replace {
                Some(target.begin_directory_replace(&item.target_path).await?)
            } else {
                None
            };
        let target_root = replacement
            .as_ref()
            .map(|replacement| replacement.path().to_owned())
            .unwrap_or_else(|| item.target_path.clone());
        let mut item_plan = build_item_copy_plan(item, item_descendants, &target_root);
        item_plan.sort_by_key(|entry| {
            (
                !entry.is_dir,
                if entry.is_dir {
                    entry.target_path.len()
                } else {
                    0
                },
            )
        });

        let copy_result: Result<()> = async {
            for entry in item_plan.iter().filter(|entry| entry.is_dir) {
                ensure_not_cancelled(&cancelled)?;
                target.ensure_copy_directory(&entry.target_path).await?;
            }

            for entry in item_plan.iter().filter(|entry| !entry.is_dir) {
                ensure_not_cancelled(&cancelled)?;
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
            ensure_not_cancelled(&cancelled)
        }
        .await;

        if let Err(copy_error) = copy_result {
            if let Some(replacement) = replacement.take()
                && let Err(cleanup_error) = target.abort_directory_replace(replacement).await
            {
                return Err(anyhow::anyhow!(
                    "{}; additionally failed to remove staged directory for {}: {}",
                    copy_error,
                    item.target_path,
                    cleanup_error
                ));
            }
            return Err(copy_error);
        }

        if let Some(replacement) = replacement {
            target
                .commit_directory_replace(replacement, &item.target_path)
                .await?;
        }
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

pub async fn copy_between_servers(request: ServerCopyRequest) -> Result<()> {
    let ServerCopyRequest {
        source_config,
        target_config,
        items,
        cancelled,
        progress,
        direct_copy_approval,
    } = request;
    ensure_not_cancelled(&cancelled)?;
    let mut target = RusshSftpClient::connect(target_config.clone()).await?;
    ensure_not_cancelled(&cancelled)?;

    if direct_copy_route_is_simple(&source_config, &target_config) {
        let source_session = SshSessionManager::new(source_config.clone());
        if let Some(plan) = prepare_direct_copy(
            &source_session,
            &source_config,
            &mut target,
            &target_config,
            &items,
            cancelled.clone(),
        )
        .await?
        {
            let preview = DirectCopyPreview::new(
                plan.strategy(),
                &source_config,
                &target_config,
                items.len(),
            );
            let decision = request_direct_copy_approval(direct_copy_approval, preview).await;
            ensure_not_cancelled(&cancelled)?;
            if direct_copy_is_selected(decision)? {
                return execute_direct_copy(
                    &source_session,
                    &mut target,
                    &target_config,
                    plan,
                    &items,
                    cancelled,
                    progress,
                )
                .await;
            }
        }
    }

    ensure_not_cancelled(&cancelled)?;
    let mut source = RusshSftpClient::connect(source_config).await?;
    ensure_not_cancelled(&cancelled)?;
    let relay_progress = progress;
    relay_copy(
        &mut source,
        &mut target,
        &items,
        cancelled,
        Box::new(move |progress| relay_progress(progress)),
    )
    .await
}

fn direct_copy_is_selected(decision: DirectCopyDecision) -> Result<bool> {
    match decision {
        DirectCopyDecision::UseDirect => Ok(true),
        DirectCopyDecision::UseRelay => Ok(false),
        DirectCopyDecision::Cancel => Err(TransferCancelled.into()),
    }
}

fn direct_copy_route_is_simple(source: &SshConnectConfig, target: &SshConnectConfig) -> bool {
    source.jump_server.is_none()
        && source.proxy.is_none()
        && target.jump_server.is_none()
        && target.proxy.is_none()
}

async fn request_direct_copy_approval(
    approval: Option<DirectCopyApproval>,
    preview: DirectCopyPreview,
) -> DirectCopyDecision {
    match approval {
        Some(approval) => approval(preview).await,
        None => DirectCopyDecision::UseRelay,
    }
}

fn ensure_not_cancelled(cancelled: &AtomicBool) -> Result<()> {
    if cancelled.load(Ordering::Relaxed) {
        return Err(TransferCancelled.into());
    }
    Ok(())
}

#[cfg(test)]
#[path = "server_copy_tests.rs"]
mod tests;
