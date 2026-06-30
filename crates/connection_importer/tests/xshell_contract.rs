use connection_importer::{
    ImportOptions, ImportSourceKind, ImportedSshAuthMethod, PasswordImportStatus,
    parse_xshell_session, preview_ssh_connections_from_path, to_ssh_params,
};
use one_core::storage::SshAuthMethod;
use std::fs;

const XSHELL_PRIVATE_KEY_SAMPLE: &str = r#"
[CONNECTION]
Protocol=SSH
Host=ssh.example.com
Port=2222

[AUTHENTICATION]
UserName=deploy
Method=PUBLICKEY
UserKey=C:\Users\me\.ssh\id_rsa
"#;

const XSHELL_PASSWORD_SAMPLE: &str = r#"
[CONNECTION]
Protocol=SSH
Host=jump.example.com

[AUTHENTICATION]
UserName=ops
Method=PASSWORD
"#;

const XSHELL_CONNECTION_USERNAME_SAMPLE: &str = r#"
[CONNECTION]
Protocol=SSH
Host=legacy.example.com
UserName=legacy

[AUTHENTICATION]
Method=PASSWORD
"#;

#[test]
fn xshell_parser_reads_private_key_session_fields() {
    let imported = parse_xshell_session(
        XSHELL_PRIVATE_KEY_SAMPLE.as_bytes(),
        "sessions/prod.xsh",
        "Prod SSH",
        ImportOptions {
            include_passwords: false,
        },
    )
    .expect("Xshell session should parse")
    .expect("SSH session should import");

    assert_eq!(ImportSourceKind::Xshell, imported.source);
    assert_eq!("sessions/prod.xsh", imported.source_id);
    assert_eq!("Prod SSH", imported.name);
    assert_eq!("ssh.example.com", imported.host);
    assert_eq!(2222, imported.port);
    assert_eq!("deploy", imported.username);
    assert_eq!(
        ImportedSshAuthMethod::PrivateKey {
            key_path: r"C:\Users\me\.ssh\id_rsa".to_string(),
            passphrase: None,
        },
        imported.auth_method
    );
    assert_eq!(PasswordImportStatus::Unsupported, imported.password_status);
}

#[test]
fn xshell_parser_maps_password_session_to_empty_password_auth() {
    let imported = parse_xshell_session(
        XSHELL_PASSWORD_SAMPLE.as_bytes(),
        "jump.xsh",
        "Jump",
        ImportOptions {
            include_passwords: true,
        },
    )
    .expect("Xshell session should parse")
    .expect("SSH session should import");

    assert_eq!("jump.example.com", imported.host);
    assert_eq!(22, imported.port);
    assert_eq!("ops", imported.username);
    assert_eq!(
        ImportedSshAuthMethod::Password { password: None },
        imported.auth_method
    );
    assert_eq!(PasswordImportStatus::Unsupported, imported.password_status);
}

#[test]
fn xshell_parser_falls_back_to_connection_username() {
    let imported = parse_xshell_session(
        XSHELL_CONNECTION_USERNAME_SAMPLE.as_bytes(),
        "legacy.xsh",
        "Legacy",
        ImportOptions {
            include_passwords: false,
        },
    )
    .expect("Xshell session should parse")
    .expect("SSH session should import");

    assert_eq!("legacy", imported.username);
}

#[test]
fn xshell_imported_session_converts_to_ssh_params() {
    let imported = parse_xshell_session(
        XSHELL_PRIVATE_KEY_SAMPLE.as_bytes(),
        "prod.xsh",
        "Prod SSH",
        ImportOptions {
            include_passwords: false,
        },
    )
    .expect("Xshell session should parse")
    .expect("SSH session should import");

    let params = to_ssh_params(imported).expect("SSH params should convert");

    assert_eq!("ssh.example.com", params.host);
    assert_eq!(2222, params.port);
    assert_eq!("deploy", params.username);
    assert!(matches!(
        params.auth_method,
        SshAuthMethod::PrivateKey { ref key_path, passphrase: None }
            if key_path == r"C:\Users\me\.ssh\id_rsa"
    ));
}

#[test]
fn preview_ssh_connections_reads_xshell_sessions_from_directory() {
    let temp_dir = tempfile::tempdir().expect("temp dir");
    fs::write(
        temp_dir.path().join("prod.xsh"),
        XSHELL_PRIVATE_KEY_SAMPLE.as_bytes(),
    )
    .expect("write Xshell session");

    let imported = preview_ssh_connections_from_path(
        ImportSourceKind::Xshell,
        temp_dir.path(),
        ImportOptions {
            include_passwords: false,
        },
    )
    .expect("Xshell directory should preview");

    assert_eq!(1, imported.len());
    assert_eq!("prod", imported[0].name);
    assert_eq!("ssh.example.com", imported[0].host);
}
