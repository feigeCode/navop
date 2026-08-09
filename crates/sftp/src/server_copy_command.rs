use crate::ServerCopyItem;
use crate::server_copy::DirectCopyStrategy;
use anyhow::{Result, bail};
use ssh::SshConnectConfig;

pub(crate) fn build_direct_copy_commands(
    strategy: DirectCopyStrategy,
    username: &str,
    host: &str,
    port: u16,
    items: &[ServerCopyItem],
) -> Result<Vec<String>> {
    validate_endpoint(username, host)?;
    if items.is_empty() {
        bail!("direct server copy requires at least one item");
    }
    items
        .iter()
        .map(|item| build_item_command(strategy, username, host, port, item))
        .collect()
}

pub(crate) fn target_ssh_command(
    target: &SshConnectConfig,
    remote_command: &str,
) -> Result<String> {
    validate_endpoint(&target.username, &target.host)?;
    let endpoint = shell_quote(&format!(
        "{}@{}",
        target.username,
        bracket_ipv6(&target.host)
    ))?;
    let remote_command = shell_quote(remote_command)?;
    Ok(format!(
        "ssh {} {endpoint} {remote_command}",
        strict_ssh_options("-p", target.port)
    ))
}

pub(crate) fn validate_endpoint(username: &str, host: &str) -> Result<()> {
    if username.is_empty()
        || username.starts_with('-')
        || !username
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || "._-".contains(character))
    {
        bail!("invalid SSH username for direct server copy");
    }
    if host.is_empty()
        || host.starts_with('-')
        || !host
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || ".:-_%".contains(character))
    {
        bail!("invalid SSH host for direct server copy");
    }
    Ok(())
}

pub(crate) fn scp_item_is_safe(item: &ServerCopyItem) -> bool {
    scp_path_is_safe(&item.source_path) && scp_path_is_safe(&item.target_path)
}

fn build_item_command(
    strategy: DirectCopyStrategy,
    username: &str,
    host: &str,
    port: u16,
    item: &ServerCopyItem,
) -> Result<String> {
    validate_path(&item.source_path)?;
    validate_path(&item.target_path)?;
    if strategy == DirectCopyStrategy::Scp && (item.is_dir || !scp_item_is_safe(item)) {
        bail!("path is not safe for scp direct copy");
    }
    let source_path = copy_source_path(strategy, item);
    let target_path = copy_target_path(strategy, item);
    let source = shell_quote(&source_path)?;
    let destination = shell_quote(&remote_destination(username, host, &target_path))?;
    match strategy {
        DirectCopyStrategy::Rsync => build_rsync_command(port, &source, &destination),
        DirectCopyStrategy::Scp => Ok(build_scp_command(port, &source, &destination)),
    }
}

fn copy_source_path(strategy: DirectCopyStrategy, item: &ServerCopyItem) -> String {
    if strategy == DirectCopyStrategy::Rsync && item.is_dir && item.source_path != "/" {
        format!("{}/", item.source_path.trim_end_matches('/'))
    } else {
        item.source_path.clone()
    }
}

fn copy_target_path(strategy: DirectCopyStrategy, item: &ServerCopyItem) -> String {
    if strategy == DirectCopyStrategy::Rsync && item.is_dir && item.target_path != "/" {
        format!("{}/", item.target_path.trim_end_matches('/'))
    } else {
        item.target_path.clone()
    }
}

fn build_rsync_command(port: u16, source: &str, destination: &str) -> Result<String> {
    let remote_shell = shell_quote(&strict_ssh_options("-p", port))?;
    Ok(format!(
        "rsync -a --protect-args -e {remote_shell} -- {source} {destination}"
    ))
}

fn build_scp_command(port: u16, source: &str, destination: &str) -> String {
    format!(
        "scp -B {} {source} {destination}",
        strict_ssh_options("-P", port)
    )
}

fn strict_ssh_options(port_flag: &str, port: u16) -> String {
    format!(
        "-o BatchMode=yes -o NumberOfPasswordPrompts=0 -o StrictHostKeyChecking=yes \
-o ForwardAgent=no -o RequestTTY=no -o ClearAllForwardings=yes \
-o ProxyJump=none -o ProxyCommand=none -o ControlMaster=no -o ControlPath=none \
-o ConnectTimeout=10 {port_flag} {port}"
    )
}

fn remote_destination(username: &str, host: &str, path: &str) -> String {
    format!("{username}@{}:{path}", bracket_ipv6(host))
}

fn bracket_ipv6(host: &str) -> String {
    if host.contains(':') {
        format!("[{host}]")
    } else {
        host.to_string()
    }
}

fn validate_path(path: &str) -> Result<()> {
    if !path.starts_with('/')
        || path.chars().any(char::is_control)
        || path.split('/').any(|component| component == "..")
    {
        bail!("invalid path for direct server copy");
    }
    Ok(())
}

fn scp_path_is_safe(path: &str) -> bool {
    path.starts_with('/')
        && path
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || "/._-".contains(character))
        && !path.split('/').any(|component| component == "..")
}

fn shell_quote(value: &str) -> Result<String> {
    if value.chars().any(char::is_control) {
        bail!("value contains control characters");
    }
    Ok(format!("'{}'", value.replace('\'', "'\"'\"'")))
}
