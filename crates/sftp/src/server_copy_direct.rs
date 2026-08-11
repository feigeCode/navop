use crate::remote_exec::{
    RemoteCommandTimeout, exec_remote_command, exec_remote_command_with_input,
    exec_remote_command_with_input_deadline,
};
use crate::server_copy::DirectCopyStrategy;
use crate::server_copy_command::{
    DIRECT_COPY_HOST_KEY_ALIAS, DirectCopyPayloadLengths, build_direct_copy_wrapper,
    scp_item_is_safe, source_ssh_options_probe_command, target_ssh_command, validate_endpoint,
};
use crate::{
    DirectoryConflictPolicy, RusshSftpClient, ServerCopyItem, SftpClient, TransferCancelled,
    TransferProgress,
};
use anyhow::{Result, anyhow, bail};
use ssh::{SshConnectConfig, SshSessionManager};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;
use tokio::time::Instant;
use zeroize::Zeroize;

pub(crate) use crate::server_copy_command::build_direct_copy_commands;

const DIRECT_COPY_AUTH_TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct DirectCopyCapabilities {
    pub source_ssh: bool,
    pub source_auth_helpers: bool,
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
}

impl DirectCopyPlan {
    pub(crate) fn strategy(&self) -> DirectCopyStrategy {
        self.strategy
    }
}

struct DirectCopyHostKey {
    known_hosts: Vec<u8>,
}

impl DirectCopyHostKey {
    fn lengths(&self) -> DirectCopyPayloadLengths {
        DirectCopyPayloadLengths {
            known_hosts: self.known_hosts.len(),
        }
    }

    fn payload(&self) -> Vec<u8> {
        self.known_hosts.clone()
    }
}

impl Drop for DirectCopyHostKey {
    fn drop(&mut self) {
        self.known_hosts.zeroize();
    }
}

pub(crate) fn choose_direct_copy_strategy(
    capabilities: &DirectCopyCapabilities,
) -> Option<DirectCopyStrategy> {
    if !capabilities.source_ssh || !capabilities.source_auth_helpers || !capabilities.targets_absent
    {
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

    let target_session = SshSessionManager::new(target_config.clone());
    let capabilities = probe_capabilities(
        source,
        &target_session,
        target_config.port,
        items,
        cancelled.clone(),
    )
    .await?;
    let Some(strategy) = choose_direct_copy_strategy(&capabilities) else {
        return Ok(None);
    };
    build_direct_copy_commands(
        strategy,
        &target_config.username,
        &target_config.host,
        target_config.port,
        items,
    )?;
    if cancelled.load(Ordering::Relaxed) {
        return Err(TransferCancelled.into());
    }
    Ok(Some(DirectCopyPlan { strategy }))
}

pub(crate) async fn execute_direct_copy(
    source: &SshSessionManager,
    target: &mut RusshSftpClient,
    target_config: &SshConnectConfig,
    plan: DirectCopyPlan,
    items: &[ServerCopyItem],
    cancelled: Arc<AtomicBool>,
    progress: Arc<dyn Fn(TransferProgress) + Send + Sync>,
) -> Result<()> {
    ensure_not_cancelled(&cancelled)?;
    let host_key = load_direct_copy_host_key(target, &cancelled)?;
    ensure_not_cancelled(&cancelled)?;
    authenticate_direct_copy(source, target_config, &host_key, cancelled.clone()).await?;

    if !targets_are_absent(target, items, cancelled.clone()).await? {
        bail!(
            "one or more direct copy targets appeared while waiting for approval. No direct write \
or local relay was started"
        );
    }

    let commands = build_direct_copy_commands(
        plan.strategy,
        &target_config.username,
        &target_config.host,
        target_config.port,
        items,
    )?;
    if commands.len() != items.len() {
        bail!("direct copy plan item count mismatch");
    }

    let total = items.iter().map(|item| item.size).sum();
    let mut transferred = 0;
    for (item_index, (command, item)) in commands.iter().zip(items).enumerate() {
        if cancelled.load(Ordering::Relaxed) {
            return Err(TransferCancelled.into());
        }
        progress(TransferProgress {
            transferred,
            total,
            current_file: Some(item.source_path.clone()),
            current_file_total: item.size,
            ..TransferProgress::default()
        });
        reserve_target(target, item, item_index).await?;
        let output =
            execute_direct_command(source, command, &host_key, cancelled.clone(), None).await?;
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

fn load_direct_copy_host_key(
    target_client: &RusshSftpClient,
    cancelled: &AtomicBool,
) -> Result<DirectCopyHostKey> {
    ensure_not_cancelled(cancelled)?;
    let known_hosts = format!(
        "{DIRECT_COPY_HOST_KEY_ALIAS} {}\n",
        target_client
            .accepted_server_public_key()
            .ok_or_else(|| anyhow!("verified target SSH host key is unavailable"))?
    )
    .into_bytes();
    ensure_not_cancelled(cancelled)?;
    Ok(DirectCopyHostKey { known_hosts })
}

async fn authenticate_direct_copy(
    source: &SshSessionManager,
    target: &SshConnectConfig,
    host_key: &DirectCopyHostKey,
    cancelled: Arc<AtomicBool>,
) -> Result<()> {
    let command = target_ssh_command(target, "true")?;
    let output = execute_direct_command(
        source,
        &command,
        host_key,
        cancelled.clone(),
        Some(Instant::now() + DIRECT_COPY_AUTH_TIMEOUT),
    )
    .await
    .map_err(|error| {
        if error.is::<RemoteCommandTimeout>() {
            anyhow!(
                "source server did not finish authenticating to the target within {} seconds: \
{error}. Verify source-to-target network reachability and configure the source server with an SSH \
key or SSH agent authorized by the target. Navop does not send target passwords or private keys \
for direct transfer. Navop relay was not started",
                DIRECT_COPY_AUTH_TIMEOUT.as_secs()
            )
        } else {
            error
        }
    })?;
    if output.exit_status == 0 {
        return Ok(());
    }
    bail!(
        "source server could not authenticate directly to the target (status {}): {}. Verify \
source-to-target network reachability and configure the source server with an SSH key or SSH agent \
authorized by the target. Navop does not send target passwords or private keys for direct \
transfer. The target host key was pinned from Navop's verified target connection. Navop relay was \
not started",
        output.exit_status,
        command_error(&output.stderr, &output.stdout)
    )
}

async fn execute_direct_command(
    source: &SshSessionManager,
    command: &str,
    host_key: &DirectCopyHostKey,
    cancelled: Arc<AtomicBool>,
    deadline: Option<Instant>,
) -> Result<crate::remote_exec::RemoteCommandOutput> {
    let wrapper = build_direct_copy_wrapper(command, host_key.lengths())?;
    let mut payload = host_key.payload();
    let result = match deadline {
        Some(deadline) => {
            exec_remote_command_with_input_deadline(
                source,
                &wrapper,
                &payload,
                cancelled.clone(),
                deadline,
            )
            .await
        }
        None => exec_remote_command_with_input(source, &wrapper, &payload, cancelled.clone()).await,
    };
    payload.zeroize();
    result
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
    target: &SshSessionManager,
    target_port: u16,
    items: &[ServerCopyItem],
    cancelled: Arc<AtomicBool>,
) -> Result<DirectCopyCapabilities> {
    let source_ssh = probe(source, "command -v ssh >/dev/null 2>&1", &cancelled).await?;
    if !source_ssh {
        return Ok(DirectCopyCapabilities::default());
    }
    let source_ssh = probe(
        source,
        &source_ssh_options_probe_command(target_port),
        &cancelled,
    )
    .await?;
    if !source_ssh {
        return Ok(DirectCopyCapabilities::default());
    }
    let source_auth_helpers = probe(
        source,
        "command -v mktemp >/dev/null 2>&1 && \
command -v chmod >/dev/null 2>&1 && \
command -v dd >/dev/null 2>&1 && \
command -v rm >/dev/null 2>&1 && \
command -v wc >/dev/null 2>&1",
        &cancelled,
    )
    .await?;
    if !source_auth_helpers {
        return Ok(DirectCopyCapabilities {
            source_ssh,
            ..DirectCopyCapabilities::default()
        });
    }
    probe_transfer_tools(
        source,
        target,
        items,
        cancelled,
        source_ssh,
        source_auth_helpers,
    )
    .await
}

async fn probe_transfer_tools(
    source: &SshSessionManager,
    target: &SshSessionManager,
    items: &[ServerCopyItem],
    cancelled: Arc<AtomicBool>,
    source_ssh: bool,
    source_auth_helpers: bool,
) -> Result<DirectCopyCapabilities> {
    let rsync_probe = "command -v rsync >/dev/null 2>&1 && \
rsync --protect-args --version >/dev/null 2>&1";
    let source_rsync = probe(source, rsync_probe, &cancelled).await?;
    let target_rsync = probe(target, rsync_probe, &cancelled).await?;
    let source_scp = probe(source, "command -v scp >/dev/null 2>&1", &cancelled).await?;
    let target_scp = probe(target, "command -v scp >/dev/null 2>&1", &cancelled).await?;
    Ok(DirectCopyCapabilities {
        source_ssh,
        source_auth_helpers,
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
    session: &SshSessionManager,
    command: &str,
    cancelled: &Arc<AtomicBool>,
) -> Result<bool> {
    match exec_remote_command(session, command, cancelled.clone()).await {
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

fn ensure_not_cancelled(cancelled: &AtomicBool) -> Result<()> {
    if cancelled.load(Ordering::Relaxed) {
        return Err(TransferCancelled.into());
    }
    Ok(())
}

#[cfg(test)]
#[path = "server_copy_direct_tests.rs"]
mod tests;
