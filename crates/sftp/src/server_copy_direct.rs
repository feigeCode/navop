use crate::remote_exec::exec_remote_command;
use crate::server_copy::DirectCopyStrategy;
use crate::server_copy_command::{scp_item_is_safe, target_ssh_command, validate_endpoint};
use crate::{
    DirectoryConflictPolicy, RusshSftpClient, ServerCopyItem, SftpClient, TransferCancelled,
    TransferProgress,
};
use anyhow::{Result, bail};
use ssh::{SshConnectConfig, SshSessionManager};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

pub(crate) use crate::server_copy_command::build_direct_copy_commands;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct DirectCopyCapabilities {
    pub source_ssh: bool,
    pub batch_auth: bool,
    pub targets_absent: bool,
    pub source_rsync: bool,
    pub target_rsync: bool,
    pub rsync_protected_args: bool,
    pub source_scp: bool,
    pub target_scp: bool,
    pub scp_safe_paths: bool,
}

pub(crate) struct DirectCopyPlan {
    strategy: DirectCopyStrategy,
    commands: Vec<String>,
}

impl DirectCopyPlan {
    pub(crate) fn strategy(&self) -> DirectCopyStrategy {
        self.strategy
    }
}

pub(crate) fn choose_direct_copy_strategy(
    capabilities: &DirectCopyCapabilities,
) -> Option<DirectCopyStrategy> {
    if !capabilities.source_ssh || !capabilities.batch_auth || !capabilities.targets_absent {
        return None;
    }
    if capabilities.source_rsync && capabilities.target_rsync && capabilities.rsync_protected_args {
        return Some(DirectCopyStrategy::Rsync);
    }
    if capabilities.source_scp && capabilities.target_scp && capabilities.scp_safe_paths {
        return Some(DirectCopyStrategy::Scp);
    }
    None
}

pub(crate) fn requires_relay_for_directory_replace(items: &[ServerCopyItem]) -> bool {
    items.iter().any(|item| {
        item.is_dir && item.directory_conflict_policy == DirectoryConflictPolicy::Replace
    })
}

pub(crate) async fn prepare_direct_copy(
    source: &SshSessionManager,
    target: &mut RusshSftpClient,
    target_config: &SshConnectConfig,
    items: &[ServerCopyItem],
    cancelled: Arc<AtomicBool>,
) -> Result<Option<DirectCopyPlan>> {
    if target_config.jump_server.is_some() || target_config.proxy.is_some() {
        return Ok(None);
    }
    if requires_relay_for_directory_replace(items) {
        return Ok(None);
    }
    if !targets_are_absent(target, items, cancelled.clone()).await? {
        return Ok(None);
    }
    if validate_endpoint(&target_config.username, &target_config.host).is_err() {
        return Ok(None);
    }

    let capabilities = probe_capabilities(source, target_config, items, cancelled.clone()).await?;
    let Some(strategy) = choose_direct_copy_strategy(&capabilities) else {
        return Ok(None);
    };
    let commands = build_direct_copy_commands(
        strategy,
        &target_config.username,
        &target_config.host,
        target_config.port,
        items,
    )?;
    if cancelled.load(Ordering::Relaxed) {
        return Err(TransferCancelled.into());
    }
    Ok(Some(DirectCopyPlan { strategy, commands }))
}

pub(crate) async fn execute_direct_copy(
    source: &SshSessionManager,
    target: &mut RusshSftpClient,
    plan: DirectCopyPlan,
    items: &[ServerCopyItem],
    cancelled: Arc<AtomicBool>,
    progress: Arc<dyn Fn(TransferProgress) + Send + Sync>,
) -> Result<()> {
    if plan.commands.len() != items.len() {
        bail!("direct copy plan item count mismatch");
    }
    if !targets_are_absent(target, items, cancelled.clone()).await? {
        bail!(
            "one or more direct copy targets appeared while waiting for approval. No direct write \
or local relay was started"
        );
    }
    let total = items.iter().map(|item| item.size).sum();
    let mut transferred = 0;
    for (item_index, (command, item)) in plan.commands.iter().zip(items).enumerate() {
        if cancelled.load(Ordering::Relaxed) {
            return Err(TransferCancelled.into());
        }
        reserve_target(target, item, item_index).await?;
        let output = exec_remote_command(source, command, cancelled.clone()).await?;
        if output.exit_status != 0 {
            bail!(
                "{} direct copy failed with status {}: {}. Local relay was not started because \
the target may be partially written",
                strategy_name(plan.strategy),
                output.exit_status,
                command_error(&output.stderr, &output.stdout)
            );
        }
        transferred += item.size;
        progress(TransferProgress {
            transferred,
            total,
            ..TransferProgress::default()
        });
    }
    Ok(())
}

async fn reserve_target(
    target: &mut RusshSftpClient,
    item: &ServerCopyItem,
    completed_items: usize,
) -> Result<()> {
    if let Err(error) = target
        .reserve_direct_copy_target(&item.target_path, item.is_dir)
        .await
    {
        if target.stat(&item.target_path).await?.is_some() {
            return target_appeared_error(&item.target_path, completed_items);
        }
        bail!(
            "failed to reserve direct copy target {} without overwriting it: {error}. Local relay \
was not started",
            item.target_path
        );
    }
    Ok(())
}

fn target_appeared_error(path: &str, completed_items: usize) -> Result<()> {
    if completed_items == 0 {
        bail!(
            "direct copy target appeared while waiting to start: {path}. No direct write or local \
relay was started"
        );
    }
    bail!(
        "direct copy target appeared after {completed_items} item(s) were copied: {path}. Local \
relay was not started because earlier targets may already be written"
    )
}

async fn probe_capabilities(
    source: &SshSessionManager,
    target: &SshConnectConfig,
    items: &[ServerCopyItem],
    cancelled: Arc<AtomicBool>,
) -> Result<DirectCopyCapabilities> {
    let source_ssh = probe(source, "command -v ssh >/dev/null 2>&1", &cancelled).await?;
    if !source_ssh {
        return Ok(DirectCopyCapabilities::default());
    }
    let batch_auth = probe(source, &target_ssh_command(target, "true")?, &cancelled).await?;
    if !batch_auth {
        return Ok(DirectCopyCapabilities {
            source_ssh,
            ..DirectCopyCapabilities::default()
        });
    }
    probe_transfer_tools(source, target, items, cancelled, source_ssh, batch_auth).await
}

async fn probe_transfer_tools(
    source: &SshSessionManager,
    target: &SshConnectConfig,
    items: &[ServerCopyItem],
    cancelled: Arc<AtomicBool>,
    source_ssh: bool,
    batch_auth: bool,
) -> Result<DirectCopyCapabilities> {
    let rsync_probe = "command -v rsync >/dev/null 2>&1 && \
rsync --protect-args --version >/dev/null 2>&1";
    let source_rsync = probe(source, rsync_probe, &cancelled).await?;
    let target_rsync = probe(
        source,
        &target_ssh_command(target, rsync_probe)?,
        &cancelled,
    )
    .await?;
    let source_scp = probe(source, "command -v scp >/dev/null 2>&1", &cancelled).await?;
    let target_scp = probe(
        source,
        &target_ssh_command(target, "command -v scp >/dev/null 2>&1")?,
        &cancelled,
    )
    .await?;
    Ok(DirectCopyCapabilities {
        source_ssh,
        batch_auth,
        targets_absent: true,
        source_rsync,
        target_rsync,
        rsync_protected_args: source_rsync && target_rsync,
        source_scp,
        target_scp,
        scp_safe_paths: items
            .iter()
            .all(|item| !item.is_dir && scp_item_is_safe(item)),
    })
}

async fn targets_are_absent(
    target: &mut RusshSftpClient,
    items: &[ServerCopyItem],
    cancelled: Arc<AtomicBool>,
) -> Result<bool> {
    for item in items {
        if cancelled.load(Ordering::Relaxed) {
            return Err(TransferCancelled.into());
        }
        if target.stat(&item.target_path).await?.is_some() {
            return Ok(false);
        }
    }
    Ok(true)
}

async fn probe(
    source: &SshSessionManager,
    command: &str,
    cancelled: &Arc<AtomicBool>,
) -> Result<bool> {
    match exec_remote_command(source, command, cancelled.clone()).await {
        Ok(output) => Ok(output.exit_status == 0),
        Err(error) if error.is::<TransferCancelled>() => Err(error),
        Err(error) => {
            tracing::debug!(%error, "direct server copy capability probe failed");
            Ok(false)
        }
    }
}

fn strategy_name(strategy: DirectCopyStrategy) -> &'static str {
    match strategy {
        DirectCopyStrategy::Rsync => "rsync",
        DirectCopyStrategy::Scp => "scp",
    }
}

fn command_error(stderr: &str, stdout: &str) -> String {
    let message = if stderr.trim().is_empty() {
        stdout.trim()
    } else {
        stderr.trim()
    };
    if message.is_empty() {
        "remote command returned no error output".to_string()
    } else {
        message.to_string()
    }
}

#[cfg(test)]
#[path = "server_copy_direct_tests.rs"]
mod tests;
