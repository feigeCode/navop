use connection_importer::{
    ImportOptions, ImportSourceKind, ImportedSshAuthMethod, PasswordImportStatus,
    parse_finalshell_connections_json, preview_ssh_connections_from_path,
};
use std::fs;

const FINALSHELL_SAMPLE: &str = r#"
{
  "connections": [
    {
      "id": "fs-prod",
      "name": "FinalShell Prod",
      "host": "fs.example.com",
      "port": 2222,
      "user_name": "deploy",
      "auth_type": "publicKey",
      "private_key": "/Users/me/.ssh/id_ed25519"
    },
    {
      "id": "fs-jump",
      "name": "FinalShell Jump",
      "host": "jump.example.com",
      "username": "ops",
      "auth_type": "password"
    }
  ]
}
"#;

#[test]
fn finalshell_parser_reads_json_connections() {
    let imported = parse_finalshell_connections_json(
        FINALSHELL_SAMPLE,
        ImportOptions {
            include_passwords: false,
        },
    )
    .expect("FinalShell JSON should parse");

    assert_eq!(2, imported.len());
    let prod = &imported[0];
    assert_eq!(ImportSourceKind::FinalShell, prod.source);
    assert_eq!("fs-prod", prod.source_id);
    assert_eq!("FinalShell Prod", prod.name);
    assert_eq!("fs.example.com", prod.host);
    assert_eq!(2222, prod.port);
    assert_eq!("deploy", prod.username);
    assert_eq!(
        ImportedSshAuthMethod::PrivateKey {
            key_path: "/Users/me/.ssh/id_ed25519".to_string(),
            passphrase: None,
        },
        prod.auth_method
    );
    assert_eq!(PasswordImportStatus::Unsupported, prod.password_status);
}

#[test]
fn finalshell_parser_maps_password_connection_without_importing_secret() {
    let imported = parse_finalshell_connections_json(
        FINALSHELL_SAMPLE,
        ImportOptions {
            include_passwords: true,
        },
    )
    .expect("FinalShell JSON should parse");

    let jump = &imported[1];
    assert_eq!("jump.example.com", jump.host);
    assert_eq!(22, jump.port);
    assert_eq!("ops", jump.username);
    assert_eq!(
        ImportedSshAuthMethod::Password { password: None },
        jump.auth_method
    );
    assert_eq!(PasswordImportStatus::Unsupported, jump.password_status);
}

#[test]
fn finalshell_preview_reads_json_files_from_directory() {
    let temp_dir = tempfile::tempdir().expect("temp dir");
    fs::write(temp_dir.path().join("connections.json"), FINALSHELL_SAMPLE)
        .expect("write FinalShell JSON");

    let imported = preview_ssh_connections_from_path(
        ImportSourceKind::FinalShell,
        temp_dir.path(),
        ImportOptions {
            include_passwords: false,
        },
    )
    .expect("FinalShell directory should preview");

    assert_eq!(2, imported.len());
    assert_eq!("FinalShell Prod", imported[0].name);
}
