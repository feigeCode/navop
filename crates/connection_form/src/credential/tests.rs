use one_core::storage::{CredentialReference, CredentialSummary};

use super::{
    CredentialCapabilities, CredentialSelectValue, build_reference, credential_select_items,
    normalize_reference,
};

fn summary() -> CredentialSummary {
    CredentialSummary {
        id: 42,
        name: "Production".to_string(),
        kind: "SSH".to_string(),
        username: Some("root".to_string()),
        has_password: true,
        has_private_key_path: true,
        has_private_key_content: false,
        has_passphrase: true,
        has_ssh_expect: false,
        sync_enabled: false,
        cloud_id: None,
        last_synced_at: None,
        team_id: None,
        owner_id: None,
        created_at: None,
        updated_at: None,
    }
}

#[test]
fn manual_mode_has_no_credential_reference() {
    assert_eq!(
        None,
        build_reference(
            CredentialSelectValue::Manual,
            CredentialCapabilities::login(),
            &[summary()],
        )
    );
}

#[test]
fn a_new_reference_only_selects_supported_available_fields() {
    assert_eq!(
        Some(CredentialReference {
            credential_id: 42,
            credential_cloud_id: None,
            username: true,
            password: true,
            private_key: false,
            passphrase: false,
        }),
        build_reference(
            CredentialSelectValue::Credential(42),
            CredentialCapabilities::login(),
            &[summary()],
        )
    );
}

#[test]
fn a_password_only_credential_only_references_the_password() {
    let mut password_only = summary();
    password_only.username = None;
    password_only.has_private_key_path = false;
    password_only.has_passphrase = false;

    assert_eq!(
        Some(CredentialReference {
            credential_id: 42,
            credential_cloud_id: None,
            username: false,
            password: true,
            private_key: false,
            passphrase: false,
        }),
        build_reference(
            CredentialSelectValue::Credential(42),
            CredentialCapabilities::login(),
            &[password_only],
        )
    );
}

#[test]
fn password_and_private_key_are_mutually_exclusive() {
    let reference = build_reference(
        CredentialSelectValue::Credential(42),
        CredentialCapabilities::all(),
        &[summary()],
    )
    .expect("credential should be selectable");

    assert!(reference.password);
    assert!(!reference.private_key);
    assert!(!reference.passphrase);
}

#[test]
fn normalization_uses_all_current_applicable_fields() {
    let reference = CredentialReference {
        credential_id: 42,
        credential_cloud_id: None,
        username: false,
        password: true,
        private_key: false,
        passphrase: false,
    };
    let mut missing_password = summary();
    missing_password.has_password = false;

    assert_eq!(
        CredentialReference {
            credential_id: 42,
            credential_cloud_id: None,
            username: true,
            password: false,
            private_key: false,
            passphrase: false,
        },
        normalize_reference(
            reference,
            CredentialCapabilities::login(),
            Some(&missing_password),
        )
    );
}

#[test]
fn normalization_expands_a_legacy_partial_reference_to_the_whole_credential() {
    let reference = CredentialReference {
        credential_id: 42,
        credential_cloud_id: None,
        username: false,
        password: true,
        private_key: false,
        passphrase: false,
    };

    assert_eq!(
        CredentialReference {
            credential_id: 42,
            credential_cloud_id: None,
            username: true,
            password: true,
            private_key: false,
            passphrase: false,
        },
        normalize_reference(reference, CredentialCapabilities::login(), Some(&summary()),)
    );
}

#[test]
fn selector_filters_credentials_without_an_applicable_field() {
    let mut key_only = summary();
    key_only.id = 7;
    key_only.name = "Key only".to_string();
    key_only.username = None;
    key_only.has_password = false;
    let items = credential_select_items(
        &[key_only, summary()],
        CredentialCapabilities::password_only(),
        None,
    );

    assert_eq!(2, items.len());
    assert_eq!(CredentialSelectValue::Manual, *items[0].value());
    assert_eq!(CredentialSelectValue::Credential(42), *items[1].value());
}
