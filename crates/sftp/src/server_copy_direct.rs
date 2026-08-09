use crate::remote_exec::{exec_remote_command, exec_remote_command_with_input};
use crate::server_copy::DirectCopyStrategy;
use crate::server_copy_command::{
    DirectCopyAuthMode, DirectCopyPayloadLengths, build_direct_copy_wrapper, scp_item_is_safe,
    source_ssh_options_probe_command, target_ssh_command, validate_endpoint,
};
use crate::{
    DirectoryConflictPolicy, RusshSftpClient, ServerCopyItem, SftpClient, TransferCancelled,
    TransferProgress,
};
use anyhow::{Result, anyhow, bail};
use ssh::{SshAuth, SshConnectConfig, SshSessionManager};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use zeroize::Zeroize;

pub(crate) use crate::server_copy_command::build_direct_copy_commands;

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
    auth_mode: DirectCopyAuthMode,
}

impl DirectCopyPlan {
    pub(crate) fn strategy(&self) -> DirectCopyStrategy {
        self.strategy
    }
}

struct DirectCopyCredentials {
    auth_mode: DirectCopyAuthMode,
    private_key: Vec<u8>,
    certificate: Vec<u8>,
    secret: Vec<u8>,
}

impl DirectCopyCredentials {
    fn empty(auth_mode: DirectCopyAuthMode) -> Self {
        Self {
            auth_mode,
            private_key: Vec::new(),
            certificate: Vec::new(),
            secret: Vec::new(),
        }
    }

    fn lengths(&self) -> DirectCopyPayloadLengths {
        DirectCopyPayloadLengths {
            private_key: self.private_key.len(),
            certificate: self.certificate.len(),
            secret: self.secret.len(),
        }
    }

    fn payload(&self) -> Vec<u8> {
        let mut payload =
            Vec::with_capacity(self.private_key.len() + self.certificate.len() + self.secret.len());
        payload.extend_from_slice(&self.private_key);
        payload.extend_from_slice(&self.certificate);
        payload.extend_from_slice(&self.secret);
        payload
    }
}

impl Drop for DirectCopyCredentials {
    fn drop(&mut self) {
        self.private_key.zeroize();
        self.certificate.zeroize();
        self.secret.zeroize();
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

    let auth_mode = DirectCopyAuthMode::from_auth(&target_config.auth);
    let target_session = SshSessionManager::new(target_config.clone());
    let capabilities = probe_capabilities(
        source,
        &target_session,
        target_config.port,
        auth_mode,
        items,
        cancelled.clone(),
    )
    .await?;
    let Some(strategy) = choose_direct_copy_strategy(&capabilities) else {
        return Ok(None);
    };
    build_direct_copy_commands(
        strategy,
        auth_mode,
        &target_config.username,
        &target_config.host,
        target_config.port,
        items,
    )?;
    if cancelled.load(Ordering::Relaxed) {
        return Err(TransferCancelled.into());
    }
    Ok(Some(DirectCopyPlan {
        strategy,
        auth_mode,
    }))
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
    let credentials =
        load_direct_copy_credentials(target_config, plan.auth_mode, &cancelled).await?;
    ensure_not_cancelled(&cancelled)?;
    authenticate_direct_copy(source, target_config, &credentials, cancelled.clone()).await?;

    if !targets_are_absent(target, items, cancelled.clone()).await? {
        bail!(
            "one or more direct copy targets appeared while waiting for approval. No direct write \
or local relay was started"
        );
    }

    let commands = build_direct_copy_commands(
        plan.strategy,
        plan.auth_mode,
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
        reserve_target(target, item, item_index).await?;
        let output =
            execute_authenticated_command(source, command, &credentials, cancelled.clone()).await?;
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

async fn load_direct_copy_credentials(
    target: &SshConnectConfig,
    expected_mode: DirectCopyAuthMode,
    cancelled: &AtomicBool,
) -> Result<DirectCopyCredentials> {
    ensure_not_cancelled(cancelled)?;
    let credentials = match &target.auth {
        SshAuth::Password(password) => DirectCopyCredentials {
            auth_mode: DirectCopyAuthMode::Password,
            private_key: Vec::new(),
            certificate: Vec::new(),
            secret: password.as_bytes().to_vec(),
        },
        SshAuth::PrivateKey {
            key_path,
            passphrase,
            certificate_path,
        } => DirectCopyCredentials {
            auth_mode: DirectCopyAuthMode::from_auth(&target.auth),
            private_key: read_credential_file(key_path, "target private key", cancelled).await?,
            certificate: read_optional_credential_file(
                certificate_path.as_deref(),
                "target SSH certificate",
                cancelled,
            )
            .await?,
            secret: passphrase
                .as_deref()
                .filter(|passphrase| !passphrase.is_empty())
                .map_or_else(Vec::new, |passphrase| passphrase.as_bytes().to_vec()),
        },
        SshAuth::PrivateKeyContent {
            private_key,
            passphrase,
            certificate_path,
        } => DirectCopyCredentials {
            auth_mode: DirectCopyAuthMode::from_auth(&target.auth),
            private_key: private_key.as_bytes().to_vec(),
            certificate: read_optional_credential_file(
                certificate_path.as_deref(),
                "target SSH certificate",
                cancelled,
            )
            .await?,
            secret: passphrase
                .as_deref()
                .filter(|passphrase| !passphrase.is_empty())
                .map_or_else(Vec::new, |passphrase| passphrase.as_bytes().to_vec()),
        },
        SshAuth::Agent | SshAuth::AutoPublicKey => {
            DirectCopyCredentials::empty(DirectCopyAuthMode::ExistingIdentity)
        }
    };

    ensure_not_cancelled(cancelled)?;
    if credentials.auth_mode != expected_mode {
        bail!("target authentication changed while preparing direct server copy");
    }
    Ok(credentials)
}

async fn read_credential_file(
    path: &str,
    description: &str,
    cancelled: &AtomicBool,
) -> Result<Vec<u8>> {
    ensure_not_cancelled(cancelled)?;
    let credential = tokio::fs::read(path)
        .await
        .map_err(|error| anyhow!("failed to read configured {description}: {error}"))?;
    ensure_not_cancelled(cancelled)?;
    Ok(credential)
}

async fn read_optional_credential_file(
    path: Option<&str>,
    description: &str,
    cancelled: &AtomicBool,
) -> Result<Vec<u8>> {
    match path.filter(|path| !path.is_empty()) {
        Some(path) => read_credential_file(path, description, cancelled).await,
        None => Ok(Vec::new()),
    }
}

async fn authenticate_direct_copy(
    source: &SshSessionManager,
    target: &SshConnectConfig,
    credentials: &DirectCopyCredentials,
    cancelled: Arc<AtomicBool>,
) -> Result<()> {
    let command = target_ssh_command(target, credentials.auth_mode, "true")?;
    let output =
        execute_authenticated_command(source, &command, credentials, cancelled.clone()).await?;
    if output.exit_status == 0 {
        return Ok(());
    }
    bail!(
        "source server could not authenticate directly to the target (status {}): {}. Verify the \
target credentials and ensure the target host key already exists in the source server's \
known_hosts. Navop relay was not started",
        output.exit_status,
        command_error(&output.stderr, &output.stdout)
    )
}

async fn execute_authenticated_command(
    source: &SshSessionManager,
    command: &str,
    credentials: &DirectCopyCredentials,
    cancelled: Arc<AtomicBool>,
) -> Result<crate::remote_exec::RemoteCommandOutput> {
    let wrapper = build_direct_copy_wrapper(command, credentials.auth_mode, credentials.lengths())?;
    let mut payload = credentials.payload();
    let result =
        exec_remote_command_with_input(source, &wrapper, &payload, cancelled.clone()).await;
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
    auth_mode: DirectCopyAuthMode,
    items: &[ServerCopyItem],
    cancelled: Arc<AtomicBool>,
) -> Result<DirectCopyCapabilities> {
    let source_ssh = probe(source, "command -v ssh >/dev/null 2>&1", &cancelled).await?;
    if !source_ssh {
        return Ok(DirectCopyCapabilities::default());
    }
    let source_ssh = probe(
        source,
        &source_ssh_options_probe_command(target_port, auth_mode),
        &cancelled,
    )
    .await?;
    if !source_ssh {
        return Ok(DirectCopyCapabilities::default());
    }
    let source_auth_helpers = if auth_mode.needs_source_helpers() {
        probe(
            source,
            "command -v mktemp >/dev/null 2>&1 && \
command -v chmod >/dev/null 2>&1 && \
command -v dd >/dev/null 2>&1 && \
command -v rm >/dev/null 2>&1 && \
command -v cat >/dev/null 2>&1 && \
command -v wc >/dev/null 2>&1",
            &cancelled,
        )
        .await?
    } else {
        true
    };
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
