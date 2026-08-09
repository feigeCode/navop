use super::{
    DirectCopyCapabilities, build_direct_copy_commands, choose_direct_copy_strategy,
    read_optional_credential_file, requires_relay_for_directory_replace,
};
use crate::server_copy_command::{
    DirectCopyAuthMode, DirectCopyPayloadLengths, build_direct_copy_wrapper,
    source_ssh_options_probe_command,
};
use crate::{DirectCopyStrategy, DirectoryConflictPolicy, ServerCopyItem};
use ssh::SshAuth;
#[cfg(unix)]
use std::io::Write;
#[cfg(unix)]
use std::process::{Command, Stdio};
use std::sync::atomic::AtomicBool;

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

fn existing_auth() -> DirectCopyAuthMode {
    DirectCopyAuthMode::ExistingIdentity
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
fn rsync_existing_identity_uses_strict_batch_ssh_and_port() {
    let commands = build_direct_copy_commands(
        DirectCopyStrategy::Rsync,
        existing_auth(),
        "deploy",
        "server.example",
        2222,
        &[item("/src/a.txt", "/dst/a.txt", false)],
    )
    .expect("rsync command");

    assert_eq!(1, commands.len());
    assert!(commands[0].contains("rsync -a --protect-args"));
    assert!(commands[0].contains("-o BatchMode=yes"));
    assert!(commands[0].contains("-o NumberOfPasswordPrompts=0"));
    assert_common_security_options(&commands[0]);
    assert!(commands[0].contains("-p 2222"));
    assert!(commands[0].contains("'deploy@server.example:/dst/a.txt'"));
}

#[test]
fn password_commands_use_askpass_compatible_ssh_options_without_embedding_password() {
    let password = "PASSWORD-SECRET";
    for strategy in [DirectCopyStrategy::Rsync, DirectCopyStrategy::Scp] {
        let command = build_direct_copy_commands(
            strategy,
            DirectCopyAuthMode::Password,
            "deploy",
            "server.example",
            2222,
            &[item("/src/app.txt", "/dst/app.txt", false)],
        )
        .expect("password command")
        .remove(0);

        assert!(command.contains("-o BatchMode=no"));
        assert!(command.contains("-o NumberOfPasswordPrompts=1"));
        assert!(command.contains("-o PreferredAuthentications=password,keyboard-interactive"));
        assert!(command.contains("-o PubkeyAuthentication=no"));
        assert_common_security_options(&command);
        assert!(!command.contains(password));
        assert!(!command.contains("sshpass"));
        assert!(!command.contains("RSYNC_PASSWORD"));
        if strategy == DirectCopyStrategy::Scp {
            assert!(command.starts_with("scp "));
            assert!(!command.starts_with("scp -B"));
        }

        let wrapper = build_direct_copy_wrapper(
            &command,
            DirectCopyAuthMode::Password,
            DirectCopyPayloadLengths {
                secret: password.len(),
                ..DirectCopyPayloadLengths::default()
            },
        )
        .expect("password wrapper");
        assert_protected_wrapper(&wrapper);
        assert!(wrapper.contains("SSH_ASKPASS_REQUIRE=force"));
        assert!(wrapper.contains("SSH_ASKPASS=\"$navop_askpass\""));
        assert!(wrapper.contains(&format!("count={}", password.len())));
        assert!(!wrapper.contains(password));
        assert!(!wrapper.contains("sshpass"));
    }
}

#[test]
fn private_key_commands_use_temporary_identity_and_optional_certificate() {
    let auth = DirectCopyAuthMode::PrivateKey {
        has_passphrase: true,
        has_certificate: true,
    };
    let command = build_direct_copy_commands(
        DirectCopyStrategy::Rsync,
        auth,
        "deploy",
        "server.example",
        2222,
        &[item("/src/app.txt", "/dst/app.txt", false)],
    )
    .expect("private key command")
    .remove(0);

    assert!(command.contains("-o BatchMode=no"));
    assert!(command.contains("-o IdentitiesOnly=yes"));
    assert!(command.contains("-o IdentityAgent=none"));
    assert!(command.contains("-e \"ssh "));
    assert!(command.contains("-i '$navop_key'"));
    assert!(command.contains("-o CertificateFile='$navop_cert'"));
    assert!(command.contains("-o PasswordAuthentication=no"));
    assert_common_security_options(&command);

    let wrapper = build_direct_copy_wrapper(
        &command,
        auth,
        DirectCopyPayloadLengths {
            private_key: 120,
            certificate: 80,
            secret: 16,
        },
    )
    .expect("private key wrapper");
    assert_protected_wrapper(&wrapper);
    assert!(wrapper.contains("navop_key=\"$navop_tmp/identity\""));
    assert!(wrapper.contains("navop_cert=\"$navop_tmp/identity-cert.pub\""));
    assert!(wrapper.contains("chmod 600 \"$navop_key\""));
    assert!(wrapper.contains("chmod 600 \"$navop_cert\""));
    assert!(wrapper.contains("chmod 700 \"$navop_askpass\""));
}

#[cfg(unix)]
#[test]
fn rsync_wrapper_expands_temporary_identity_paths_before_invoking_rsync() {
    let auth = DirectCopyAuthMode::PrivateKey {
        has_passphrase: false,
        has_certificate: true,
    };
    let command = build_direct_copy_commands(
        DirectCopyStrategy::Rsync,
        auth,
        "deploy",
        "server.example",
        2222,
        &[item("/src/app.txt", "/dst/app.txt", false)],
    )
    .expect("private key rsync command")
    .remove(0);
    let private_key = b"PRIVATE-KEY";
    let certificate = b"SSH-CERTIFICATE";
    let wrapper = build_direct_copy_wrapper(
        &command,
        auth,
        DirectCopyPayloadLengths {
            private_key: private_key.len(),
            certificate: certificate.len(),
            secret: 0,
        },
    )
    .expect("private key wrapper");

    let directory = tempfile::tempdir().expect("temporary directory");
    let fake_rsync = directory.path().join("rsync");
    let captured_args = directory.path().join("rsync-args");
    std::fs::write(
        &fake_rsync,
        "#!/bin/sh\n: > \"$NAVOP_RSYNC_ARGS\"\nfor arg in \"$@\"; do printf '%s\\0' \"$arg\" >> \"$NAVOP_RSYNC_ARGS\"; done\n",
    )
    .expect("fake rsync script");
    use std::os::unix::fs::PermissionsExt;
    let mut permissions = std::fs::metadata(&fake_rsync)
        .expect("fake rsync metadata")
        .permissions();
    permissions.set_mode(0o700);
    std::fs::set_permissions(&fake_rsync, permissions).expect("make fake rsync executable");

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
        .env("NAVOP_RSYNC_ARGS", &captured_args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("execute generated wrapper");
    let mut payload = Vec::new();
    payload.extend_from_slice(private_key);
    payload.extend_from_slice(certificate);
    child
        .stdin
        .take()
        .expect("wrapper stdin")
        .write_all(&payload)
        .expect("write credential payload");
    let output = child.wait_with_output().expect("wait for wrapper");
    assert!(
        output.status.success(),
        "wrapper failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let captured = std::fs::read(captured_args).expect("captured rsync arguments");
    let arguments = captured
        .split(|byte| *byte == 0)
        .filter(|argument| !argument.is_empty())
        .map(|argument| String::from_utf8(argument.to_vec()).expect("UTF-8 rsync argument"))
        .collect::<Vec<_>>();
    let remote_shell_index = arguments
        .iter()
        .position(|argument| argument == "-e")
        .expect("rsync -e argument")
        + 1;
    let remote_shell = arguments
        .get(remote_shell_index)
        .expect("rsync remote shell argument");
    assert!(remote_shell.contains("-i '/tmp/navop-direct-copy."));
    assert!(remote_shell.contains("/identity'"));
    assert!(remote_shell.contains("CertificateFile='/tmp/navop-direct-copy."));
    assert!(remote_shell.contains("/identity-cert.pub'"));
    assert!(!remote_shell.contains("$navop_key"));
    assert!(!remote_shell.contains("$navop_cert"));
}

#[test]
fn unencrypted_private_key_stays_in_batch_mode_without_askpass() {
    let auth = DirectCopyAuthMode::PrivateKey {
        has_passphrase: false,
        has_certificate: false,
    };
    let command = build_direct_copy_commands(
        DirectCopyStrategy::Scp,
        auth,
        "deploy",
        "server.example",
        22,
        &[item("/src/a.txt", "/dst/a.txt", false)],
    )
    .expect("private key scp command")
    .remove(0);
    assert!(command.starts_with("scp -B"));
    assert!(command.contains("-o BatchMode=yes"));
    assert!(command.contains("-i \"$navop_key\""));
    assert!(!command.contains("CertificateFile"));

    let wrapper = build_direct_copy_wrapper(
        &command,
        auth,
        DirectCopyPayloadLengths {
            private_key: 120,
            ..DirectCopyPayloadLengths::default()
        },
    )
    .expect("private key wrapper");
    assert_protected_wrapper(&wrapper);
    assert!(!wrapper.contains("SSH_ASKPASS"));
}

#[test]
fn existing_identity_wrapper_has_no_temporary_credential_directory() {
    let wrapper = build_direct_copy_wrapper(
        "ssh target true",
        DirectCopyAuthMode::ExistingIdentity,
        DirectCopyPayloadLengths::default(),
    )
    .expect("existing identity wrapper");
    assert_eq!("exec ssh target true </dev/null", wrapper);
    assert!(!wrapper.contains("mktemp"));
}

#[test]
fn wrapper_rejects_payload_shape_mismatches() {
    assert!(
        build_direct_copy_wrapper(
            "true",
            DirectCopyAuthMode::ExistingIdentity,
            DirectCopyPayloadLengths {
                secret: 1,
                ..DirectCopyPayloadLengths::default()
            }
        )
        .is_err()
    );
    assert!(
        build_direct_copy_wrapper(
            "true",
            DirectCopyAuthMode::PrivateKey {
                has_passphrase: false,
                has_certificate: false,
            },
            DirectCopyPayloadLengths::default()
        )
        .is_err()
    );
    assert!(
        build_direct_copy_wrapper(
            "true",
            DirectCopyAuthMode::PrivateKey {
                has_passphrase: true,
                has_certificate: false,
            },
            DirectCopyPayloadLengths {
                private_key: 1,
                ..DirectCopyPayloadLengths::default()
            }
        )
        .is_err()
    );
}

#[test]
fn empty_optional_private_key_fields_are_treated_as_unconfigured() {
    for auth in [
        SshAuth::PrivateKey {
            key_path: "/tmp/key".to_string(),
            passphrase: Some(String::new()),
            certificate_path: Some(String::new()),
        },
        SshAuth::PrivateKeyContent {
            private_key: "PRIVATE-KEY".to_string(),
            passphrase: Some(String::new()),
            certificate_path: Some(String::new()),
        },
    ] {
        assert_eq!(
            DirectCopyAuthMode::PrivateKey {
                has_passphrase: false,
                has_certificate: false,
            },
            DirectCopyAuthMode::from_auth(&auth)
        );
    }
}

#[tokio::test]
async fn empty_optional_certificate_path_is_not_read() {
    let certificate =
        read_optional_credential_file(Some(""), "target SSH certificate", &AtomicBool::new(false))
            .await
            .expect("empty certificate path should mean no certificate");
    assert!(certificate.is_empty());
}

#[test]
fn ssh_option_probe_parses_direct_auth_options_without_credentials() {
    for auth in [
        DirectCopyAuthMode::ExistingIdentity,
        DirectCopyAuthMode::Password,
        DirectCopyAuthMode::PrivateKey {
            has_passphrase: true,
            has_certificate: true,
        },
    ] {
        let command = source_ssh_options_probe_command(2222, auth);
        assert!(command.starts_with("ssh -F /dev/null -G "));
        assert!(command.contains("-o StrictHostKeyChecking=yes"));
        assert!(command.contains("-o ProxyJump=none"));
        assert!(command.contains("navop@navop.invalid >/dev/null 2>&1"));
        assert!(!command.contains("$navop_"));
        assert!(!command.contains("PASSWORD-SECRET"));
        assert!(!command.contains("PRIVATE-KEY-SECRET"));
    }
}

#[test]
fn scp_direct_copy_rejects_directories() {
    assert!(
        build_direct_copy_commands(
            DirectCopyStrategy::Scp,
            existing_auth(),
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
        existing_auth(),
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
        existing_auth(),
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
                existing_auth(),
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
                existing_auth(),
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
        existing_auth(),
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
    let secrets = ["PASSWORD-SECRET", "PRIVATE-KEY-SECRET", "PASSPHRASE-SECRET"];
    for strategy in [DirectCopyStrategy::Rsync, DirectCopyStrategy::Scp] {
        for auth in [
            DirectCopyAuthMode::ExistingIdentity,
            DirectCopyAuthMode::Password,
            DirectCopyAuthMode::PrivateKey {
                has_passphrase: true,
                has_certificate: true,
            },
        ] {
            let command = build_direct_copy_commands(
                strategy,
                auth,
                "deploy",
                "server.example",
                22,
                &[item("/src/a.txt", "/dst/a.txt", false)],
            )
            .expect("direct command")
            .remove(0);

            assert_common_security_options(&command);
            assert!(!command.contains("StrictHostKeyChecking=no"));
            assert!(!command.contains("UserKnownHostsFile=/dev/null"));
            assert!(!command.contains("accept-new"));
            assert!(!command.contains("sshpass"));
            assert!(!command.contains("RSYNC_PASSWORD"));
            for secret in secrets {
                assert!(!command.contains(secret));
            }
        }
    }
}

#[test]
fn scp_command_does_not_require_double_dash_support() {
    let command = build_direct_copy_commands(
        DirectCopyStrategy::Scp,
        existing_auth(),
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
    assert!(command.contains("-o ForwardAgent=no"));
    assert!(command.contains("-o RequestTTY=no"));
    assert!(command.contains("-o ClearAllForwardings=yes"));
    assert!(command.contains("-o ProxyJump=none"));
    assert!(command.contains("-o ProxyCommand=none"));
    assert!(command.contains("-o ControlMaster=no"));
    assert!(command.contains("-o ControlPath=none"));
    assert!(command.contains("-o ConnectTimeout=10"));
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
