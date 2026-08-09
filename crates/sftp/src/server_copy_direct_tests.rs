use super::{
    DirectCopyCapabilities, build_direct_copy_commands, choose_direct_copy_strategy,
    requires_relay_for_directory_replace,
};
use crate::{DirectCopyStrategy, DirectoryConflictPolicy, ServerCopyItem};

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
        batch_auth: true,
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
fn direct_copy_relays_when_batch_auth_probe_fails() {
    let capabilities = DirectCopyCapabilities {
        batch_auth: false,
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
    assert!(!commands[0].contains(" -s "));
    assert!(commands[0].contains("-o BatchMode=yes"));
    assert!(commands[0].contains("-o NumberOfPasswordPrompts=0"));
    assert!(commands[0].contains("-o StrictHostKeyChecking=yes"));
    assert!(commands[0].contains("-o ForwardAgent=no"));
    assert!(commands[0].contains("-o RequestTTY=no"));
    assert!(commands[0].contains("-o ClearAllForwardings=yes"));
    assert!(commands[0].contains("-o ProxyJump=none"));
    assert!(commands[0].contains("-o ProxyCommand=none"));
    assert!(commands[0].contains("-o ConnectTimeout=10"));
    assert!(commands[0].contains("-p 2222"));
    assert!(commands[0].contains("'deploy@server.example:/dst/a.txt'"));
}

#[test]
fn scp_command_uses_strict_batch_ssh() {
    let commands = build_direct_copy_commands(
        DirectCopyStrategy::Scp,
        "deploy",
        "server.example",
        2222,
        &[item("/src/app.txt", "/dst/app.txt", false)],
    )
    .expect("scp command");

    assert_eq!(1, commands.len());
    assert!(commands[0].starts_with("scp -B"));
    assert!(commands[0].contains("-o BatchMode=yes"));
    assert!(commands[0].contains("-o StrictHostKeyChecking=yes"));
    assert!(commands[0].contains("-P 2222"));
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
fn rejects_nul_and_invalid_user_or_host() {
    assert!(
        build_direct_copy_commands(
            DirectCopyStrategy::Rsync,
            "bad user",
            "server.example",
            22,
            &[item("/src/a.txt", "/dst/a.txt", false)],
        )
        .is_err()
    );
    assert!(
        build_direct_copy_commands(
            DirectCopyStrategy::Scp,
            "deploy",
            "server;example",
            22,
            &[item("/src/a.txt", "/dst/a.txt", false)],
        )
        .is_err()
    );
    assert!(
        build_direct_copy_commands(
            DirectCopyStrategy::Rsync,
            "deploy",
            "server.example",
            22,
            &[item("/src/\0bad", "/dst/a.txt", false)],
        )
        .is_err()
    );
    assert!(
        build_direct_copy_commands(
            DirectCopyStrategy::Scp,
            "-deploy",
            "server.example",
            22,
            &[item("/src/a.txt", "/dst/a.txt", false)],
        )
        .is_err()
    );
    assert!(
        build_direct_copy_commands(
            DirectCopyStrategy::Scp,
            "deploy",
            "-server.example",
            22,
            &[item("/src/a.txt", "/dst/a.txt", false)],
        )
        .is_err()
    );
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
fn direct_commands_never_disable_host_key_checking_or_embed_credentials() {
    for strategy in [DirectCopyStrategy::Rsync, DirectCopyStrategy::Scp] {
        let command = build_direct_copy_commands(
            strategy,
            "deploy",
            "server.example",
            22,
            &[item(
                "/src/PASSWORD-SECRET",
                "/dst/PRIVATE-KEY-SECRET",
                false,
            )],
        )
        .expect("direct command")
        .remove(0);

        assert!(command.contains("StrictHostKeyChecking=yes"));
        assert!(!command.contains("StrictHostKeyChecking=no"));
        assert!(!command.contains("accept-new"));
        assert!(!command.contains("sshpass"));
        assert!(!command.contains("RSYNC_PASSWORD"));
        assert!(!command.contains(" -i "));
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

#[test]
fn rsync_requires_absolute_paths_without_parent_components() {
    for invalid_path in ["relative/path", "/src/../secret"] {
        assert!(
            build_direct_copy_commands(
                DirectCopyStrategy::Rsync,
                "deploy",
                "server.example",
                22,
                &[item(invalid_path, "/dst/a.txt", false)],
            )
            .is_err()
        );
    }
}
