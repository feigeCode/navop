use crate::ServerCopyItem;
use crate::server_copy::DirectCopyStrategy;
use anyhow::{Result, bail};
use ssh::{SshAuth, SshConnectConfig};

pub(crate) const DIRECT_COPY_HOST_KEY_ALIAS: &str = "navop-direct-copy-target";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum DirectCopyAuthMode {
    ExistingIdentity,
    Password,
    PrivateKey {
        has_passphrase: bool,
        has_certificate: bool,
    },
}

impl DirectCopyAuthMode {
    pub(crate) fn from_auth(auth: &SshAuth) -> Self {
        match auth {
            SshAuth::Password(_) => Self::Password,
            SshAuth::PrivateKey {
                passphrase,
                certificate_path,
                ..
            }
            | SshAuth::PrivateKeyContent {
                passphrase,
                certificate_path,
                ..
            } => Self::PrivateKey {
                has_passphrase: non_empty(passphrase.as_deref()),
                has_certificate: non_empty(certificate_path.as_deref()),
            },
            SshAuth::Agent | SshAuth::AutoPublicKey => Self::ExistingIdentity,
        }
    }

    pub(crate) fn private_key_material_flags(self) -> (bool, bool) {
        match self {
            Self::PrivateKey {
                has_passphrase,
                has_certificate,
            } => (has_passphrase, has_certificate),
            Self::ExistingIdentity | Self::Password => (false, false),
        }
    }

    pub(crate) fn needs_source_helpers(self) -> bool {
        true
    }

    fn uses_askpass(self) -> bool {
        matches!(
            self,
            Self::Password
                | Self::PrivateKey {
                    has_passphrase: true,
                    ..
                }
        )
    }

    fn uses_batch_mode(self) -> bool {
        !self.uses_askpass()
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct DirectCopyPayloadLengths {
    pub known_hosts: usize,
    pub private_key: usize,
    pub certificate: usize,
    pub secret: usize,
}

pub(crate) fn build_direct_copy_commands(
    strategy: DirectCopyStrategy,
    auth_mode: DirectCopyAuthMode,
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
        .map(|item| build_item_command(strategy, auth_mode, username, host, port, item))
        .collect()
}

pub(crate) fn target_ssh_command(
    target: &SshConnectConfig,
    auth_mode: DirectCopyAuthMode,
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
        ssh_options("-p", target.port, auth_mode, AuthPathStyle::ShellArguments)
    ))
}

pub(crate) fn source_ssh_options_probe_command(port: u16, auth_mode: DirectCopyAuthMode) -> String {
    format!(
        "ssh -F /dev/null -G {} navop@navop.invalid >/dev/null 2>&1",
        ssh_options("-p", port, auth_mode, AuthPathStyle::Probe)
    )
}

pub(crate) fn build_direct_copy_wrapper(
    command: &str,
    auth_mode: DirectCopyAuthMode,
    lengths: DirectCopyPayloadLengths,
) -> Result<String> {
    validate_payload_lengths(auth_mode, lengths)?;

    let mut wrapper = String::from(
        "set -eu\n\
umask 077\n\
navop_tmp=$(mktemp -d /tmp/navop-direct-copy.XXXXXX)\n\
navop_cleanup() {\n\
  navop_status=$?\n\
  trap - EXIT HUP INT TERM\n\
  rm -rf -- \"$navop_tmp\"\n\
  exit \"$navop_status\"\n\
}\n\
trap navop_cleanup EXIT HUP INT TERM\n",
    );
    append_payload_file(
        &mut wrapper,
        "navop_known_hosts",
        "known_hosts",
        lengths.known_hosts,
    );

    match auth_mode {
        DirectCopyAuthMode::ExistingIdentity => {}
        DirectCopyAuthMode::Password => {
            append_payload_file(&mut wrapper, "navop_secret", "secret", lengths.secret);
            append_askpass_helper(&mut wrapper);
        }
        DirectCopyAuthMode::PrivateKey {
            has_passphrase,
            has_certificate,
        } => {
            append_payload_file(&mut wrapper, "navop_key", "identity", lengths.private_key);
            if has_certificate {
                append_payload_file(
                    &mut wrapper,
                    "navop_cert",
                    "identity-cert.pub",
                    lengths.certificate,
                );
            }
            if has_passphrase {
                append_payload_file(&mut wrapper, "navop_secret", "secret", lengths.secret);
                append_askpass_helper(&mut wrapper);
            }
        }
    }

    wrapper.push_str("exec 0</dev/null\nset +e\n");
    if auth_mode.uses_askpass() {
        wrapper
            .push_str("DISPLAY=navop:0 SSH_ASKPASS_REQUIRE=force SSH_ASKPASS=\"$navop_askpass\" ");
    }
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
    auth_mode: DirectCopyAuthMode,
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
        DirectCopyStrategy::Rsync => build_rsync_command(port, auth_mode, &source, &destination),
        DirectCopyStrategy::Scp => Ok(build_scp_command(port, auth_mode, &source, &destination)),
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

fn build_rsync_command(
    port: u16,
    auth_mode: DirectCopyAuthMode,
    source: &str,
    destination: &str,
) -> Result<String> {
    let remote_shell = shell_double_quote_with_internal_variable_expansion(&format!(
        "ssh {}",
        ssh_options("-p", port, auth_mode, AuthPathStyle::RsyncRemoteShell)
    ))?;
    Ok(format!(
        "rsync -a --protect-args -e {remote_shell} -- {source} {destination}"
    ))
}

fn build_scp_command(
    port: u16,
    auth_mode: DirectCopyAuthMode,
    source: &str,
    destination: &str,
) -> String {
    let batch_flag = if auth_mode.uses_batch_mode() {
        " -B"
    } else {
        ""
    };
    format!(
        "scp{batch_flag} {} {source} {destination}",
        ssh_options("-P", port, auth_mode, AuthPathStyle::ShellArguments)
    )
}

#[derive(Clone, Copy)]
enum AuthPathStyle {
    ShellArguments,
    RsyncRemoteShell,
    Probe,
}

fn ssh_options(
    port_flag: &str,
    port: u16,
    auth_mode: DirectCopyAuthMode,
    path_style: AuthPathStyle,
) -> String {
    let common = "-o StrictHostKeyChecking=yes -o ForwardAgent=no -o RequestTTY=no \
-o ClearAllForwardings=yes -o ProxyJump=none -o ProxyCommand=none \
-o ControlMaster=no -o ControlPath=none -o ConnectTimeout=10 \
-o ServerAliveInterval=5 -o ServerAliveCountMax=3";
    let host_key = match path_style {
        AuthPathStyle::ShellArguments => {
            "-o UserKnownHostsFile=\"$navop_known_hosts\" \
-o HostKeyAlias=navop-direct-copy-target"
        }
        AuthPathStyle::RsyncRemoteShell => {
            "-o UserKnownHostsFile='$navop_known_hosts' \
-o HostKeyAlias=navop-direct-copy-target"
        }
        AuthPathStyle::Probe => {
            "-o UserKnownHostsFile=/tmp/navop-direct-copy-probe-known-hosts \
-o HostKeyAlias=navop-direct-copy-target"
        }
    };
    let auth = match auth_mode {
        DirectCopyAuthMode::ExistingIdentity => {
            "-o BatchMode=yes -o NumberOfPasswordPrompts=0".to_string()
        }
        DirectCopyAuthMode::Password => "-o BatchMode=no -o NumberOfPasswordPrompts=1 \
-o PreferredAuthentications=password,keyboard-interactive \
-o PubkeyAuthentication=no -o PasswordAuthentication=yes \
-o KbdInteractiveAuthentication=yes"
            .to_string(),
        DirectCopyAuthMode::PrivateKey {
            has_passphrase,
            has_certificate,
        } => {
            let batch = if has_passphrase {
                "-o BatchMode=no -o NumberOfPasswordPrompts=1"
            } else {
                "-o BatchMode=yes -o NumberOfPasswordPrompts=0"
            };
            let key = match path_style {
                AuthPathStyle::ShellArguments => "-i \"$navop_key\"",
                AuthPathStyle::RsyncRemoteShell => "-i '$navop_key'",
                AuthPathStyle::Probe => "-i /tmp/navop-direct-copy-probe-identity",
            };
            let certificate = if has_certificate {
                match path_style {
                    AuthPathStyle::ShellArguments => " -o CertificateFile=\"$navop_cert\"",
                    AuthPathStyle::RsyncRemoteShell => " -o CertificateFile='$navop_cert'",
                    AuthPathStyle::Probe => {
                        " -o CertificateFile=/tmp/navop-direct-copy-probe-identity-cert.pub"
                    }
                }
            } else {
                ""
            };
            format!(
                "{batch} -o IdentitiesOnly=yes -o IdentityAgent=none \
-o PreferredAuthentications=publickey -o PasswordAuthentication=no \
-o KbdInteractiveAuthentication=no {key}{certificate}"
            )
        }
    };
    format!("{auth} {host_key} {common} {port_flag} {port}")
}

fn validate_payload_lengths(
    auth_mode: DirectCopyAuthMode,
    lengths: DirectCopyPayloadLengths,
) -> Result<()> {
    if lengths.known_hosts == 0 {
        bail!("direct copy requires a verified target host key");
    }
    match auth_mode {
        DirectCopyAuthMode::ExistingIdentity => {
            if lengths.private_key != 0 || lengths.certificate != 0 || lengths.secret != 0 {
                bail!("existing-identity direct copy must not receive a credential payload");
            }
        }
        DirectCopyAuthMode::Password => {
            if lengths.private_key != 0 || lengths.certificate != 0 {
                bail!("password direct copy received an invalid credential payload");
            }
        }
        DirectCopyAuthMode::PrivateKey {
            has_passphrase,
            has_certificate,
        } => {
            if lengths.private_key == 0
                || has_certificate != (lengths.certificate > 0)
                || has_passphrase != (lengths.secret > 0)
            {
                bail!("private-key direct copy received an invalid credential payload");
            }
        }
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
echo 'Navop credential payload was incomplete' >&2; exit 125; }}\n\
chmod 600 \"${variable}\"\n"
    ));
}

fn append_askpass_helper(wrapper: &mut String) {
    wrapper.push_str(
        "navop_askpass=\"$navop_tmp/askpass\"\n\
printf '%s\\n' '#!/bin/sh' 'exec cat \"${0%/*}/secret\"' > \"$navop_askpass\"\n\
chmod 700 \"$navop_askpass\"\n",
    );
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
        .replace("$navop_key", "")
        .replace("$navop_cert", "");
    if without_allowed_variables.contains('$') {
        bail!("unexpected shell variable in rsync remote shell");
    }
    let escaped = value
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('`', "\\`");
    Ok(format!("\"{escaped}\""))
}

fn non_empty(value: Option<&str>) -> bool {
    value.is_some_and(|value| !value.is_empty())
}
