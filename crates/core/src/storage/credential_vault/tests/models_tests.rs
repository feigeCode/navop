use crate::storage::{
    CredentialEntry, CredentialReference, DbConnectionConfig, MongoDBParams, RedisParams,
    ReferencedCredentialFields, RemoteDesktopParams, SshParams,
    resolve_credential_reference_strict,
};

#[test]
fn new_credentials_are_local_only_by_default() {
    assert!(!CredentialEntry::new("Production", "username_password").sync_enabled);
}

#[test]
fn debug_output_redacts_all_secret_fields() {
    let mut credential = CredentialEntry::new("Production", "composite");
    credential.password = Some("super-secret-password".to_string());
    credential.private_key_content = Some("super-secret-private-key".to_string());
    credential.passphrase = Some("super-secret-passphrase".to_string());

    let output = format!("{credential:?}");

    assert!(!output.contains("super-secret-password"));
    assert!(!output.contains("super-secret-private-key"));
    assert!(!output.contains("super-secret-passphrase"));
    assert_eq!(3, output.matches("<redacted>").count());
}

#[test]
fn serialization_omits_all_secret_and_local_key_fields() {
    let mut credential = CredentialEntry::new("Production", "composite");
    credential.username = Some("deploy".to_string());
    credential.password = Some("super-secret-password".to_string());
    credential.private_key_path = Some("/Users/me/.ssh/id_ed25519".to_string());
    credential.private_key_content = Some("super-secret-private-key".to_string());
    credential.passphrase = Some("super-secret-passphrase".to_string());

    let json = serde_json::to_value(&credential).expect("serialize safe credential metadata");

    assert_eq!(Some("deploy"), json["username"].as_str());
    for forbidden in [
        "password",
        "private_key_path",
        "private_key_content",
        "passphrase",
    ] {
        assert!(json.get(forbidden).is_none(), "{forbidden} must be absent");
    }
    let encoded = json.to_string();
    assert!(!encoded.contains("super-secret-password"));
    assert!(!encoded.contains("super-secret-private-key"));
    assert!(!encoded.contains("super-secret-passphrase"));
    assert!(!encoded.contains("/Users/me/.ssh/id_ed25519"));
}

#[test]
fn private_key_content_takes_precedence_over_local_path() {
    let mut credential = CredentialEntry::new("SSH", "ssh_key");
    credential.private_key_path = Some("/Users/me/.ssh/id_ed25519".to_string());
    credential.private_key_content = Some("-----BEGIN PRIVATE KEY-----".to_string());

    assert_eq!(
        Some("-----BEGIN PRIVATE KEY-----"),
        credential.private_key()
    );
}

#[test]
fn strict_resolution_rejects_missing_or_empty_selected_fields() {
    let reference = CredentialReference {
        credential_id: 42,
        credential_cloud_id: None,
        username: false,
        password: true,
        private_key: false,
        passphrase: false,
    };
    let manual = ReferencedCredentialFields::default();
    let missing = resolve_credential_reference_strict(manual.clone(), &reference, None)
        .expect_err("missing credential must fail");
    assert!(missing.to_string().contains("credential 42 was not found"));

    let credential = CredentialEntry::new("Empty", "password");
    let empty = resolve_credential_reference_strict(manual, &reference, Some(&credential))
        .expect_err("selected empty password must fail");
    assert!(empty.to_string().contains("has no password"));
}

#[test]
fn strict_resolution_preserves_unselected_manual_fields() {
    let mut credential = CredentialEntry::new("Shared password", "password");
    credential.username = Some("vault-user".to_string());
    credential.password = Some("vault-password".to_string());
    let reference = CredentialReference {
        credential_id: 1,
        credential_cloud_id: None,
        username: false,
        password: true,
        private_key: false,
        passphrase: false,
    };
    let resolved = resolve_credential_reference_strict(
        ReferencedCredentialFields::new(
            Some("manual-user".to_string()),
            Some("manual-password".to_string()),
            None,
            None,
        ),
        &reference,
        Some(&credential),
    )
    .expect("resolve selected password");

    assert_eq!(Some("manual-user"), resolved.username.as_deref());
    assert_eq!(Some("vault-password"), resolved.password.as_deref());
}

#[test]
fn legacy_connection_json_without_credential_reference_still_deserializes() {
    let ssh = r#"{"host":"example.com","port":22,"username":"root","auth_method":{"Password":{"password":"pw"}},"terminal_encoding":"utf8","terminal_type":"xterm-256color"}"#;
    let db = r#"{"database_type":"MySQL","host":"localhost","port":3306,"username":"root","password":"pw","database":null,"service_name":null,"sid":null}"#;
    let redis = r#"{"host":"localhost","port":6379,"password":"pw","username":null,"db_index":0}"#;
    let mongo = r#"{"host":"localhost","port":27017,"username":"root","password":"pw"}"#;
    let remote = r#"{"protocol":"Rdp","host":"localhost","port":3389,"username":"root","password":"pw","domain":null}"#;

    assert!(
        serde_json::from_str::<SshParams>(ssh)
            .unwrap()
            .credential_reference
            .is_none()
    );
    assert!(
        serde_json::from_str::<DbConnectionConfig>(db)
            .unwrap()
            .credential_reference
            .is_none()
    );
    assert!(
        serde_json::from_str::<RedisParams>(redis)
            .unwrap()
            .credential_reference
            .is_none()
    );
    assert!(
        serde_json::from_str::<MongoDBParams>(mongo)
            .unwrap()
            .credential_reference
            .is_none()
    );
    assert!(
        serde_json::from_str::<RemoteDesktopParams>(remote)
            .unwrap()
            .credential_reference
            .is_none()
    );
}
