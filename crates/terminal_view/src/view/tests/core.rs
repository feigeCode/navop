use super::*;

#[test]
fn wrapped_addon_line_text_joins_visual_continuation_lines() {
    let lines = vec![
        WrappedLineSegment::new("见 /Users/demo/project/crates/extension-protoco", true),
        WrappedLineSegment::new("l/src/row.rs:22 和其他文本", false),
    ];

    let joined = wrapped_addon_line_text(&lines, 1, 4, 0);

    assert_eq!(
        "见 /Users/demo/project/crates/extension-protocol/src/row.rs:22 和其他文本",
        joined.text
    );
    assert_eq!(0, joined.screen_line);
    assert_eq!(
        "见 /Users/demo/project/crates/extension-protoco"
            .chars()
            .count()
            + 4,
        joined.column
    );
}
#[test]
fn local_terminal_close_confirms_while_command_is_running() {
    assert!(should_confirm_local_terminal_close(
        TerminalConnectionKind::Local,
        true,
        TermMode::empty(),
        None,
    ));
}

#[test]
fn local_terminal_close_confirms_while_tui_is_running() {
    assert!(should_confirm_local_terminal_close(
        TerminalConnectionKind::Local,
        false,
        TermMode::ALT_SCREEN,
        None,
    ));
}

#[test]
fn local_terminal_close_does_not_confirm_when_shell_is_idle() {
    assert!(!should_confirm_local_terminal_close(
        TerminalConnectionKind::Local,
        false,
        TermMode::empty(),
        None,
    ));
}

#[test]
fn tab_duplicate_is_supported_for_local_ssh_and_serial_terminals() {
    let ssh = StoredConnection::new_ssh(
        "ssh".to_string(),
        SshParams {
            host: "localhost".to_string(),
            port: 22,
            username: "user".to_string(),
            auth_method: SshAuthMethod::Agent,
            connect_timeout: None,
            keepalive_interval: None,
            keepalive_max: None,
            default_directory: None,
            init_script: None,
            disable_shell_integration: None,
            jump_server: None,
            proxy: None,
            os_id: None,
            icon: None,
        },
        None,
    );
    let serial = StoredConnection::new_serial(
        "serial".to_string(),
        SerialParams {
            port_name: "/dev/ttyS0".to_string(),
            ..Default::default()
        },
        None,
    );

    assert!(terminal_tab_duplicate_supported(
        &TerminalDuplicateSource::Local(LocalConfig::default())
    ));
    assert!(terminal_tab_duplicate_supported(
        &TerminalDuplicateSource::Ssh {
            connection: ssh,
            working_dir: None,
            sync_path_with_terminal: true,
        },
    ));
    assert!(terminal_tab_duplicate_supported(
        &TerminalDuplicateSource::Serial(serial)
    ));
}

#[test]
fn duplicate_source_for_local_terminal_prefers_current_working_dir() {
    let mut config = LocalConfig::default();
    config.working_dir = Some("/tmp/original".to_string());

    let source = terminal_duplicate_source_with_cwd(
        TerminalDuplicateSource::Local(config),
        Some("/tmp/current"),
    );

    let TerminalDuplicateSource::Local(config) = source else {
        panic!("expected local duplicate source");
    };
    assert_eq!(Some("/tmp/current"), config.working_dir.as_deref());
}

#[test]
fn duplicate_source_keeps_original_local_dir_when_current_dir_is_blank() {
    let mut config = LocalConfig::default();
    config.working_dir = Some("/tmp/original".to_string());

    let source =
        terminal_duplicate_source_with_cwd(TerminalDuplicateSource::Local(config), Some("  "));

    let TerminalDuplicateSource::Local(config) = source else {
        panic!("expected local duplicate source");
    };
    assert_eq!(Some("/tmp/original"), config.working_dir.as_deref());
}

#[test]
fn duplicate_source_for_ssh_terminal_prefers_current_working_dir() {
    let ssh = StoredConnection::new_ssh(
        "ssh".to_string(),
        SshParams {
            host: "localhost".to_string(),
            port: 22,
            username: "user".to_string(),
            auth_method: SshAuthMethod::Agent,
            connect_timeout: None,
            keepalive_interval: None,
            keepalive_max: None,
            default_directory: None,
            init_script: None,
            disable_shell_integration: None,
            jump_server: None,
            proxy: None,
            os_id: None,
            icon: None,
        },
        None,
    );

    let source = terminal_duplicate_source_with_cwd(
        TerminalDuplicateSource::Ssh {
            connection: ssh,
            working_dir: Some("/srv/original".to_string()),
            sync_path_with_terminal: true,
        },
        Some("/srv/current"),
    );

    let TerminalDuplicateSource::Ssh {
        working_dir,
        sync_path_with_terminal,
        ..
    } = source
    else {
        panic!("expected ssh duplicate source");
    };
    assert_eq!(Some("/srv/current"), working_dir.as_deref());
    assert!(sync_path_with_terminal);
}

#[test]
fn terminal_history_scope_matches_supported_connection_kinds() {
    let local = terminal_history_scope(TerminalConnectionKind::Local, None)
        .expect("local terminal should have history scope");
    let ssh = terminal_history_scope(TerminalConnectionKind::Ssh, Some(42))
        .expect("ssh terminal with id should have history scope");

    assert_eq!("local", local.scope_key);
    assert_eq!("ssh:42", ssh.scope_key);
    assert!(terminal_history_scope(TerminalConnectionKind::Ssh, None).is_none());
    assert!(terminal_history_scope(TerminalConnectionKind::Serial, Some(7)).is_none());
}

#[test]
fn remote_clipboard_image_path_uses_tmp_prefix_and_format_extension() {
    let path = remote_clipboard_image_path(ImageFormat::Png, 1_720_000_000_123);

    assert_eq!("/tmp/onetcli-paste-1720000000123.png", path);
}

#[test]
fn remote_clipboard_image_path_uses_jpg_for_jpeg_images() {
    let path = remote_clipboard_image_path(ImageFormat::Jpeg, 42);

    assert_eq!("/tmp/onetcli-paste-42.jpg", path);
}

#[test]
fn clipboard_image_from_item_extracts_image_entry() {
    let image = Image::from_bytes(ImageFormat::Png, vec![1, 2, 3]);
    let item = ClipboardItem::new_image(&image);
    let extracted = clipboard_image_from_item(&item).expect("image should be extracted");

    assert_eq!(ImageFormat::Png, extracted.format);
    assert_eq!(vec![1, 2, 3], extracted.bytes);
}

#[test]
fn clipboard_image_from_item_ignores_text_clipboard() {
    let item = ClipboardItem::new_string("/tmp/image.png".to_string());

    assert!(clipboard_image_from_item(&item).is_none());
}

#[test]
fn clipboard_image_upload_is_only_for_ssh_shell_paste() {
    assert!(should_upload_clipboard_image_to_remote_cli(
        true,
        TerminalConnectionKind::Ssh,
        TermMode::empty()
    ));
    assert!(should_upload_clipboard_image_to_remote_cli(
        true,
        TerminalConnectionKind::Ssh,
        TermMode::BRACKETED_PASTE
    ));

    assert!(!should_upload_clipboard_image_to_remote_cli(
        true,
        TerminalConnectionKind::Local,
        TermMode::empty()
    ));
    assert!(!should_upload_clipboard_image_to_remote_cli(
        true,
        TerminalConnectionKind::Serial,
        TermMode::empty()
    ));
    assert!(!should_upload_clipboard_image_to_remote_cli(
        false,
        TerminalConnectionKind::Ssh,
        TermMode::empty()
    ));
}

#[test]
fn clipboard_image_upload_intercepts_ssh_tui_modes() {
    for mode in [
        TermMode::ALT_SCREEN,
        TermMode::MOUSE_MODE,
        TermMode::DISAMBIGUATE_ESC_CODES,
        TermMode::FOCUS_IN_OUT,
        TermMode::VI,
    ] {
        assert!(should_upload_clipboard_image_to_remote_cli(
            true,
            TerminalConnectionKind::Ssh,
            mode
        ));
        assert!(!should_upload_clipboard_image_to_remote_cli(
            false,
            TerminalConnectionKind::Ssh,
            mode
        ));
    }
}
