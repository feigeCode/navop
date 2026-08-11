use crate::ServerCopyItem;
use crate::server_copy::DirectCopyStrategy;
use anyhow::{Result, bail};
use ssh::SshConnectConfig;

pub(crate) const DIRECT_COPY_HOST_KEY_ALIAS: &str = "navop-direct-copy-target";

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct DirectCopyPayloadLengths {
    pub known_hosts: usize,
}

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
        ssh_options("-p", target.port, AuthPathStyle::ShellArguments)
    ))
}

pub(crate) fn source_ssh_options_probe_command(port: u16) -> String {
    format!(
        "ssh -F /dev/null -G {} navop@navop.invalid >/dev/null 2>&1",
        ssh_options("-p", port, AuthPathStyle::Probe)
    )
}

pub(crate) fn build_direct_copy_wrapper(
    command: &str,
    lengths: DirectCopyPayloadLengths,
    identity_file_name: &str,
) -> Result<String> {
    validate_payload_lengths(lengths)?;
    validate_identity_file_name(identity_file_name)?;

    let mut wrapper = String::from(
        "set -eu\n\
umask 077\n\
[ -n \"${HOME:-}\" ] || { echo 'Navop could not determine the source home directory' >&2; exit 72; }\n\
navop_tmp=$(mktemp -d /tmp/navop-direct-copy.XXXXXX)\n\
navop_cleanup() {\n\
  navop_status=$?\n\
  trap - EXIT HUP INT TERM\n\
  rm -rf -- \"$navop_tmp\"\n\
  exit \"$navop_status\"\n\
}\n\
trap navop_cleanup EXIT HUP INT TERM\n",
    );
    wrapper.push_str(&format!(
        "navop_identity=\"$HOME/.ssh/{identity_file_name}\"\n\
[ -f \"$navop_identity\" ] && [ ! -L \"$navop_identity\" ] || {{ \
echo 'Navop dedicated SSH key is unavailable on the source server' >&2; exit 73; }}\n\
chmod 600 \"$navop_identity\"\n"
    ));
    append_payload_file(
        &mut wrapper,
        "navop_known_hosts",
        "known_hosts",
        lengths.known_hosts,
    );

    wrapper.push_str("exec 0</dev/null\nset +e\n");
    wrapper.push_str(command);
    wrapper.push_str(
        " </dev/null\n\
navop_status=$?\n\
set -e\n\
exit \"$navop_status\"\n",
    );
    Ok(wrapper)
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
    let remote_shell = shell_double_quote_with_internal_variable_expansion(&format!(
        "ssh {}",
        ssh_options("-p", port, AuthPathStyle::RsyncRemoteShell)
    ))?;
    Ok(format!(
        "rsync -a --protect-args -e {remote_shell} -- {source} {destination}"
    ))
}

fn build_scp_command(port: u16, source: &str, destination: &str) -> String {
    format!(
        "scp -B {} {source} {destination}",
        ssh_options("-P", port, AuthPathStyle::ShellArguments)
    )
}

#[derive(Clone, Copy)]
enum AuthPathStyle {
    ShellArguments,
    RsyncRemoteShell,
    Probe,
}

fn ssh_options(port_flag: &str, port: u16, path_style: AuthPathStyle) -> String {
    // Keep the source server's proxy configuration available. Some SSH clients also interpret
    // ProxyJump=none as a literal jump host named "none" instead of disabling proxying.
    let common = "-o StrictHostKeyChecking=yes -o ForwardAgent=no -o RequestTTY=no \
-o ClearAllForwardings=yes -o ControlMaster=no -o ControlPath=none -o ConnectTimeout=10 \
-o ServerAliveInterval=5 -o ServerAliveCountMax=3";
    let host_key = match path_style {
        AuthPathStyle::ShellArguments => {
            "-o UserKnownHostsFile=\"$navop_known_hosts\" \
-o HostKeyAlias=navop-direct-copy-target -i \"$navop_identity\""
        }
        AuthPathStyle::RsyncRemoteShell => {
            "-o UserKnownHostsFile='$navop_known_hosts' \
-o HostKeyAlias=navop-direct-copy-target -i '$navop_identity'"
        }
        AuthPathStyle::Probe => {
            "-o UserKnownHostsFile=/tmp/navop-direct-copy-probe-known-hosts \
-o HostKeyAlias=navop-direct-copy-target \
-i /tmp/navop-direct-copy-probe-identity"
        }
    };
    let auth = "-o BatchMode=yes -o NumberOfPasswordPrompts=0 -o IdentitiesOnly=yes \
-o PreferredAuthentications=publickey -o PubkeyAuthentication=yes \
-o PasswordAuthentication=no -o KbdInteractiveAuthentication=no";
    format!("{auth} {host_key} {common} {port_flag} {port}")
}

fn validate_payload_lengths(lengths: DirectCopyPayloadLengths) -> Result<()> {
    if lengths.known_hosts == 0 {
        bail!("direct copy requires a verified target host key");
    }
    Ok(())
}

fn validate_identity_file_name(file_name: &str) -> Result<()> {
    if !file_name.starts_with("navop_direct_copy_")
        || !file_name
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || character == '_')
    {
        bail!("invalid dedicated SSH key file name");
    }
    Ok(())
}

fn append_payload_file(wrapper: &mut String, variable: &str, name: &str, length: usize) {
    wrapper.push_str(&format!("{variable}=\"$navop_tmp/{name}\"\n"));
    if length == 0 {
        wrapper.push_str(&format!(": > \"${variable}\"\n"));
    } else {
        wrapper.push_str(&format!(
            "dd bs=1 count={length} of=\"${variable}\" 2>/dev/null\n"
        ));
    }
    wrapper.push_str(&format!(
        "[ \"$(wc -c < \"${variable}\")\" -eq {length} ] || {{ \
echo 'Navop host-key payload was incomplete' >&2; exit 125; }}\n\
chmod 600 \"${variable}\"\n"
    ));
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

fn shell_double_quote_with_internal_variable_expansion(value: &str) -> Result<String> {
    if value.chars().any(char::is_control) {
        bail!("value contains control characters");
    }
    let without_allowed_variables = value
        .replace("$navop_known_hosts", "")
        .replace("$navop_identity", "");
    if without_allowed_variables.contains('$') {
        bail!("unexpected shell variable in rsync remote shell");
    }
    let escaped = value
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('`', "\\`");
    Ok(format!("\"{escaped}\""))
}
