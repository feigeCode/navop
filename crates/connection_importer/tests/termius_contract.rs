use connection_importer::{
    ImportOptions, ImportSourceKind, ImportedSshAuthMethod, PasswordImportStatus,
    parse_termius_hosts_json, preview_ssh_connections_from_path,
};
use std::fs;

const TERMIUS_SAMPLE: &str = r#"
{
  "hosts": [
    {
      "id": "term-prod",
      "label": "Termius Prod",
      "address": "termius.example.com",
      "port": 2200,
      "username": "deploy",
      "identity": {
        "type": "key",
        "key_path": "/Users/me/.ssh/termius_id"
      }
    },
    {
      "id": "term-jump",
      "name": "Termius Jump",
      "hostname": "jump.term.example.com",
      "user": "ops"
    }
  ]
}
"#;

#[test]
fn termius_parser_reads_exported_hosts() {
    let imported = parse_termius_hosts_json(
        TERMIUS_SAMPLE,
        ImportOptions {
            include_passwords: false,
        },
    )
    .expect("Termius JSON should parse");

    assert_eq!(2, imported.len());
    let prod = &imported[0];
    assert_eq!(ImportSourceKind::Termius, prod.source);
    assert_eq!("term-prod", prod.source_id);
    assert_eq!("Termius Prod", prod.name);
    assert_eq!("termius.example.com", prod.host);
    assert_eq!(2200, prod.port);
    assert_eq!("deploy", prod.username);
    assert_eq!(
        ImportedSshAuthMethod::PrivateKey {
            key_path: "/Users/me/.ssh/termius_id".to_string(),
            passphrase: None,
        },
        prod.auth_method
    );
    assert_eq!(PasswordImportStatus::Unsupported, prod.password_status);
}

#[test]
fn termius_parser_uses_default_port_and_password_placeholder() {
    let imported = parse_termius_hosts_json(
        TERMIUS_SAMPLE,
        ImportOptions {
            include_passwords: true,
        },
    )
    .expect("Termius JSON should parse");

    let jump = &imported[1];
    assert_eq!("Termius Jump", jump.name);
    assert_eq!("jump.term.example.com", jump.host);
    assert_eq!(22, jump.port);
    assert_eq!("ops", jump.username);
    assert_eq!(
        ImportedSshAuthMethod::Password { password: None },
        jump.auth_method
    );
    assert_eq!(PasswordImportStatus::Unsupported, jump.password_status);
}

#[test]
fn termius_preview_reads_json_files_from_directory() {
    let temp_dir = tempfile::tempdir().expect("temp dir");
    fs::write(temp_dir.path().join("hosts.json"), TERMIUS_SAMPLE).expect("write Termius JSON");

    let imported = preview_ssh_connections_from_path(
        ImportSourceKind::Termius,
        temp_dir.path(),
        ImportOptions {
            include_passwords: false,
        },
    )
    .expect("Termius directory should preview");

    assert_eq!(2, imported.len());
    assert_eq!("Termius Prod", imported[0].name);
}
