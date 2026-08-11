use super::{
    DirectCopyCapabilities, build_direct_copy_commands, choose_direct_copy_strategy,
    requires_relay_for_directory_replace,
};
use crate::server_copy_command::{
    DirectCopyPayloadLengths, build_direct_copy_wrapper, source_ssh_options_probe_command,
};
use crate::{DirectCopyStrategy, DirectoryConflictPolicy, ServerCopyItem};
#[cfg(unix)]
use std::io::Write;
#[cfg(unix)]
use std::process::{Command, Stdio};

const KNOWN_HOSTS: &[u8] =
    b"navop-direct-copy-target ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAITestKey\n";

fn known_hosts_lengths() -> DirectCopyPayloadLengths {
    DirectCopyPayloadLengths {
        known_hosts: KNOWN_HOSTS.len(),
    }
}

fn item(source: &str, target: &str, is_dir: bool) -> ServerCopyItem {
    ServerCopyItem {
        source_path: source.to_string(),
        target_path: target.to_string(),
        is_dir,
        size: 0,
        directory_conflict_policy: DirectoryConflictPolicy::Merge,
    }
}

fn rsync_capabilities() -> DirectCopyCapabilities {
    DirectCopyCapabilities {
        source_ssh: true,
        source_auth_helpers: true,
        targets_absent: true,
        source_rsync: true,
        target_rsync: true,
        rsync_protected_args: true,
        source_scp: true,
        target_scp: true,
        scp_safe_paths: true,
    }
}

#[test]
fn direct_copy_prefers_rsync_when_all_requirements_are_available() {
    assert_eq!(
        Some(DirectCopyStrategy::Rsync),
        choose_direct_copy_strategy(&rsync_capabilities())
    );
}

#[test]
fn direct_copy_uses_scp_when_rsync_is_unavailable() {
    let capabilities = DirectCopyCapabilities {
        source_rsync: false,
        ..rsync_capabilities()
    };

    assert_eq!(
        Some(DirectCopyStrategy::Scp),
        choose_direct_copy_strategy(&capabilities)
    );
}

#[test]
fn direct_copy_relays_when_source_auth_helpers_are_missing() {
    let capabilities = DirectCopyCapabilities {
        source_auth_helpers: false,
        ..rsync_capabilities()
    };

    assert_eq!(None, choose_direct_copy_strategy(&capabilities));
}

#[test]
fn direct_copy_relays_when_any_target_already_exists() {
    let capabilities = DirectCopyCapabilities {
        targets_absent: false,
        ..rsync_capabilities()
    };

    assert_eq!(None, choose_direct_copy_strategy(&capabilities));
}

#[test]
fn direct_copy_relays_when_scp_path_is_not_safe() {
    let capabilities = DirectCopyCapabilities {
        source_rsync: false,
        scp_safe_paths: false,
        ..rsync_capabilities()
    };

    assert_eq!(None, choose_direct_copy_strategy(&capabilities));
}

#[test]
fn direct_copy_relays_when_rsync_lacks_protected_args() {
    let capabilities = DirectCopyCapabilities {
        rsync_protected_args: false,
        source_scp: false,
        ..rsync_capabilities()
    };

    assert_eq!(None, choose_direct_copy_strategy(&capabilities));
}

#[test]
fn direct_copy_relays_for_directory_replacement() {
    let mut directory = item("/src/app", "/dst/app", true);
    directory.directory_conflict_policy = DirectoryConflictPolicy::Replace;

    assert!(requires_relay_for_directory_replace(&[directory]));
    assert!(!requires_relay_for_directory_replace(&[item(
        "/src/app", "/dst/app", true
    )]));
    assert!(!requires_relay_for_directory_replace(&[item(
        "/src/app.txt",
        "/dst/app.txt",
        false
    )]));
}

#[test]
fn direct_commands_use_only_source_side_public_key_authentication() {
    for strategy in [DirectCopyStrategy::Rsync, DirectCopyStrategy::Scp] {
        let command = build_direct_copy_commands(
            strategy,
            "deploy",
            "server.example",
            2222,
            &[item("/src/app.txt", "/dst/app.txt", false)],
        )
        .expect("direct command")
        .remove(0);

        assert!(command.contains("-o BatchMode=yes"));
        assert!(command.contains("-o NumberOfPasswordPrompts=0"));
        assert!(command.contains("-o PreferredAuthentications=publickey"));
        assert!(command.contains("-o PubkeyAuthentication=yes"));
        assert!(command.contains("-o PasswordAuthentication=no"));
        assert!(command.contains("-o KbdInteractiveAuthentication=no"));
        assert_common_security_options(&command);
        assert!(!command.contains("-o BatchMode=no"));
        assert!(!command.contains("SSH_ASKPASS"));
        assert!(!command.contains("navop_secret"));
        assert!(!command.contains("navop_key"));
        assert!(!command.contains("navop_cert"));
        assert!(!command.contains("CertificateFile"));
        assert!(!command.contains("IdentityAgent=none"));
        assert!(!command.contains("IdentitiesOnly=yes"));
        assert!(!command.contains("PasswordAuthentication=yes"));

        if strategy == DirectCopyStrategy::Scp {
            assert!(command.starts_with("scp -B "));
        }
    }
}

#[test]
fn rsync_command_uses_strict_batch_ssh_and_port() {
    let commands = build_direct_copy_commands(
        DirectCopyStrategy::Rsync,
        "deploy",
        "server.example",
        2222,
        &[item("/src/a.txt", "/dst/a.txt", false)],
    )
    .expect("rsync command");

    assert_eq!(1, commands.len());
    assert!(commands[0].contains("rsync -a --protect-args"));
    assert!(commands[0].contains("-p 2222"));
    assert!(commands[0].contains("'deploy@server.example:/dst/a.txt'"));
}

#[test]
fn wrapper_stages_only_the_verified_known_hosts_payload() {
    let wrapper = build_direct_copy_wrapper("ssh target true", known_hosts_lengths())
        .expect("direct copy wrapper");

    assert_protected_wrapper(&wrapper);
    assert!(wrapper.contains("navop_known_hosts=\"$navop_tmp/known_hosts\""));
    assert!(wrapper.contains(&format!("count={}", KNOWN_HOSTS.len())));
    assert!(!wrapper.contains("SSH_ASKPASS"));
    assert!(!wrapper.contains("navop_secret"));
    assert!(!wrapper.contains("navop_key"));
    assert!(!wrapper.contains("navop_cert"));
    assert!(!wrapper.contains("identity-cert.pub"));
}

#[test]
fn wrapper_rejects_an_empty_host_key_payload() {
    assert!(build_direct_copy_wrapper("true", DirectCopyPayloadLengths::default()).is_err());
}

#[cfg(unix)]
#[test]
fn scp_wrapper_passes_pinned_known_hosts_without_any_credentials() {
    let command = build_direct_copy_commands(
        DirectCopyStrategy::Scp,
        "deploy",
        "server.example",
        2222,
        &[item("/src/app.txt", "/dst/app.txt", false)],
    )
    .expect("scp command")
    .remove(0);
    let wrapper = build_direct_copy_wrapper(&command, known_hosts_lengths()).expect("scp wrapper");

    let directory = tempfile::tempdir().expect("temporary directory");
    let fake_scp = directory.path().join("scp");
    let captured_known_hosts = directory.path().join("known-hosts");
    let captured_environment = directory.path().join("environment");
    let captured_args = directory.path().join("scp-args");
    std::fs::write(
        &fake_scp,
        r#"#!/bin/sh
set -eu
: > "$NAVOP_CAPTURED_ARGS"
previous=
known_hosts=
for arg in "$@"; do
  printf '%s\0' "$arg" >> "$NAVOP_CAPTURED_ARGS"
  if [ "$previous" = "-o" ]; then
    case "$arg" in
      UserKnownHostsFile=*) known_hosts=${arg#UserKnownHostsFile=} ;;
    esac
  fi
  previous=$arg
done
[ -n "$known_hosts" ]
cat "$known_hosts" > "$NAVOP_CAPTURED_KNOWN_HOSTS"
printf '%s\n%s\n%s\n' "${SSH_ASKPASS-}" "${SSH_ASKPASS_REQUIRE-}" "${DISPLAY-}" \
  > "$NAVOP_CAPTURED_ENVIRONMENT"
"#,
    )
    .expect("fake scp script");
    use std::os::unix::fs::PermissionsExt;
    let mut permissions = std::fs::metadata(&fake_scp)
        .expect("fake scp metadata")
        .permissions();
    permissions.set_mode(0o700);
    std::fs::set_permissions(&fake_scp, permissions).expect("make fake scp executable");

    let inherited_path = std::env::var_os("PATH").unwrap_or_default();
    let mut child = Command::new("/bin/sh")
        .arg("-c")
        .arg(&wrapper)
        .env(
            "PATH",
            format!(
                "{}:{}",
                directory.path().display(),
                inherited_path.to_string_lossy()
            ),
        )
        .env("NAVOP_CAPTURED_KNOWN_HOSTS", &captured_known_hosts)
        .env("NAVOP_CAPTURED_ENVIRONMENT", &captured_environment)
        .env("NAVOP_CAPTURED_ARGS", &captured_args)
        .env_remove("SSH_ASKPASS")
        .env_remove("SSH_ASKPASS_REQUIRE")
        .env_remove("DISPLAY")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("execute generated wrapper");
    child
        .stdin
        .take()
        .expect("wrapper stdin")
        .write_all(KNOWN_HOSTS)
        .expect("write host-key payload");
    let output = child.wait_with_output().expect("wait for wrapper");
    assert!(
        output.status.success(),
        "wrapper failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    assert_eq!(
        KNOWN_HOSTS,
        std::fs::read(captured_known_hosts)
            .expect("captured known_hosts")
            .as_slice()
    );
    assert_eq!(
        "\n\n\n",
        std::fs::read_to_string(captured_environment).expect("captured environment")
    );
    let arguments = std::fs::read(captured_args).expect("captured scp arguments");
    let arguments = arguments
        .split(|byte| *byte == 0)
        .filter(|argument| !argument.is_empty())
        .map(|argument| String::from_utf8(argument.to_vec()).expect("UTF-8 scp argument"))
        .collect::<Vec<_>>();
    assert!(arguments.iter().any(|argument| argument == "-B"));
    assert!(
        arguments
            .iter()
            .any(|argument| argument.starts_with("UserKnownHostsFile=/tmp/navop-direct-copy."))
    );
    assert!(
        arguments
            .iter()
            .any(|argument| argument == "HostKeyAlias=navop-direct-copy-target")
    );
    assert!(
        arguments
            .iter()
            .any(|argument| argument == "PreferredAuthentications=publickey")
    );
    assert!(
        arguments
            .iter()
            .any(|argument| argument == "PasswordAuthentication=no")
    );
}

#[test]
fn ssh_option_probe_parses_public_key_options_without_credentials() {
    let command = source_ssh_options_probe_command(2222);
    assert!(command.starts_with("ssh -F /dev/null -G "));
    assert!(command.contains("-o BatchMode=yes"));
    assert!(command.contains("-o PreferredAuthentications=publickey"));
    assert!(command.contains("-o PubkeyAuthentication=yes"));
    assert!(command.contains("-o PasswordAuthentication=no"));
    assert!(command.contains("-o KbdInteractiveAuthentication=no"));
    assert!(command.contains("-o StrictHostKeyChecking=yes"));
    assert!(command.contains("-o ProxyJump=none"));
    assert!(command.contains("navop@navop.invalid >/dev/null 2>&1"));
    assert!(!command.contains("$navop_"));
    assert!(!command.contains("SSH_ASKPASS"));
    assert!(!command.contains("IdentityAgent=none"));
}

#[test]
fn scp_direct_copy_rejects_directories() {
    assert!(
        build_direct_copy_commands(
            DirectCopyStrategy::Scp,
            "deploy",
            "server.example",
            22,
            &[item("/src/app", "/dst/app", true)],
        )
        .is_err()
    );
}

#[test]
fn rsync_directory_copy_targets_the_reserved_directory_contents() {
    let command = build_direct_copy_commands(
        DirectCopyStrategy::Rsync,
        "deploy",
        "server.example",
        22,
        &[item("/src/app", "/dst/app", true)],
    )
    .expect("rsync command")
    .remove(0);

    assert!(command.contains("'/src/app/'"));
    assert!(command.contains("'deploy@server.example:/dst/app/'"));
}

#[test]
fn ipv6_target_is_bracketed() {
    let commands = build_direct_copy_commands(
        DirectCopyStrategy::Scp,
        "deploy",
        "2001:db8::10",
        22,
        &[item("/src/a.txt", "/dst/a.txt", false)],
    )
    .expect("scp command");

    assert!(commands[0].contains("'deploy@[2001:db8::10]:/dst/a.txt'"));
}

#[test]
fn rejects_nul_and_invalid_user_host_or_path() {
    for (username, host, source) in [
        ("bad user", "server.example", "/src/a.txt"),
        ("deploy", "server;example", "/src/a.txt"),
        ("deploy", "server.example", "/src/\0bad"),
        ("-deploy", "server.example", "/src/a.txt"),
        ("deploy", "-server.example", "/src/a.txt"),
        ("deploy", "server.example", "relative/path"),
        ("deploy", "server.example", "/src/../secret"),
    ] {
        assert!(
            build_direct_copy_commands(
                DirectCopyStrategy::Rsync,
                username,
                host,
                22,
                &[item(source, "/dst/a.txt", false)],
            )
            .is_err()
        );
    }
}

#[test]
fn scp_rejects_paths_with_spaces_or_shell_metacharacters() {
    for unsafe_path in [
        "/src/a b",
        "/src/a;touch-pwned",
        "/src/$HOME",
        "/src/a'b",
        "/src/../secret",
    ] {
        assert!(
            build_direct_copy_commands(
                DirectCopyStrategy::Scp,
                "deploy",
                "server.example",
                22,
                &[item(unsafe_path, "/dst/a.txt", false)],
            )
            .is_err(),
            "{unsafe_path} should not be accepted by scp"
        );
    }
}

#[test]
fn rsync_quotes_spaces_metacharacters_and_single_quotes() {
    let commands = build_direct_copy_commands(
        DirectCopyStrategy::Rsync,
        "deploy",
        "server.example",
        22,
        &[item(
            "/src/a b;$(touch pwned)'file",
            "/dst/a b;$(touch pwned)'file",
            false,
        )],
    )
    .expect("rsync command");

    assert!(commands[0].contains("'/src/a b;$(touch pwned)'\"'\"'file'"));
    assert!(commands[0].contains("'deploy@server.example:/dst/a b;$(touch pwned)'\"'\"'file'"));
}

#[test]
fn commands_never_weaken_host_key_checking_or_embed_authentication_secrets() {
    let forbidden = [
        "PASSWORD-SECRET",
        "PRIVATE-KEY-SECRET",
        "PASSPHRASE-SECRET",
        "StrictHostKeyChecking=no",
        "UserKnownHostsFile=/dev/null",
        "accept-new",
        "sshpass",
        "RSYNC_PASSWORD",
        "SSH_ASKPASS",
        "navop_secret",
        "navop_key",
        "navop_cert",
    ];
    for strategy in [DirectCopyStrategy::Rsync, DirectCopyStrategy::Scp] {
        let command = build_direct_copy_commands(
            strategy,
            "deploy",
            "server.example",
            22,
            &[item("/src/a.txt", "/dst/a.txt", false)],
        )
        .expect("direct command")
        .remove(0);

        assert_common_security_options(&command);
        for value in forbidden {
            assert!(!command.contains(value), "command contains {value}");
        }
    }
}

#[test]
fn scp_command_does_not_require_double_dash_support() {
    let command = build_direct_copy_commands(
        DirectCopyStrategy::Scp,
        "deploy",
        "server.example",
        22,
        &[item("/src/a.txt", "/dst/a.txt", false)],
    )
    .expect("scp command")
    .remove(0);

    assert!(!command.contains(" -- "));
}

fn assert_common_security_options(command: &str) {
    assert!(command.contains("-o StrictHostKeyChecking=yes"));
    assert!(command.contains("-o UserKnownHostsFile="));
    assert!(command.contains("-o HostKeyAlias=navop-direct-copy-target"));
    assert!(command.contains("-o ForwardAgent=no"));
    assert!(command.contains("-o RequestTTY=no"));
    assert!(command.contains("-o ClearAllForwardings=yes"));
    assert!(command.contains("-o ProxyJump=none"));
    assert!(command.contains("-o ProxyCommand=none"));
    assert!(command.contains("-o ControlMaster=no"));
    assert!(command.contains("-o ControlPath=none"));
    assert!(command.contains("-o ConnectTimeout=10"));
    assert!(command.contains("-o ServerAliveInterval=5"));
    assert!(command.contains("-o ServerAliveCountMax=3"));
}

fn assert_protected_wrapper(wrapper: &str) {
    assert!(wrapper.contains("set -eu"));
    assert!(wrapper.contains("umask 077"));
    assert!(wrapper.contains("mktemp -d /tmp/navop-direct-copy.XXXXXX"));
    assert!(wrapper.contains("trap navop_cleanup EXIT HUP INT TERM"));
    assert!(wrapper.contains("rm -rf -- \"$navop_tmp\""));
    assert!(wrapper.contains("exec 0</dev/null"));
    assert!(wrapper.contains("wc -c"));
}
