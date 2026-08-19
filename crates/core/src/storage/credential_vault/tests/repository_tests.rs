use crate::crypto;
use crate::storage::traits::Repository;
use crate::storage::{CredentialEntry, CredentialSummary};

use super::{crypto_guard, test_repository, with_master_key};

#[test]
fn repository_encrypts_secrets_at_rest_and_decrypts_on_read() {
    with_master_key(|| {
        let (_temp, connection, repository) = test_repository();
        let mut credential = CredentialEntry::new("Production");
        credential.username = Some("deploy".to_string());
        credential.password = Some("plain-password".to_string());
        credential.private_key_content = Some("plain-private-key".to_string());
        credential.passphrase = Some("plain-passphrase".to_string());
        credential.sync_enabled = true;
        let id = repository
            .insert(&mut credential)
            .expect("insert credential");

        let stored: (String, String, String) = connection
            .with_connection(|connection| {
                Ok(connection.query_row(
                    "SELECT password, private_key_content, passphrase
                     FROM credential_entries WHERE id = ?1",
                    [id],
                    |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
                )?)
            })
            .expect("read encrypted values");
        for secret in [&stored.0, &stored.1, &stored.2] {
            assert!(secret.starts_with("ENC:"));
            assert!(!secret.contains("plain-"));
        }

        let loaded = repository
            .get(id)
            .expect("read credential")
            .expect("credential exists");
        assert_eq!(Some("plain-password"), loaded.password.as_deref());
        assert_eq!(
            Some("plain-private-key"),
            loaded.private_key_content.as_deref()
        );
        assert_eq!(Some("plain-passphrase"), loaded.passphrase.as_deref());
        assert!(loaded.sync_enabled);
    });
}

#[test]
fn repository_refuses_to_persist_secrets_without_master_key() {
    let _guard = crypto_guard();
    crypto::clear_master_key();
    let (_temp, _connection, repository) = test_repository();
    let mut credential = CredentialEntry::new("Production");
    credential.password = Some("must-not-be-plaintext".to_string());

    let error = repository
        .insert(&mut credential)
        .expect_err("secret insert must fail while locked");

    assert!(error.to_string().contains("without a master key"));
    assert_eq!(0, repository.count().expect("credential count"));
}

#[test]
fn repository_round_trips_sync_metadata() {
    with_master_key(|| {
        let (_temp, _connection, repository) = test_repository();
        let mut credential = CredentialEntry::new("Shared");
        credential.sync_enabled = true;
        credential.cloud_id = Some("cloud-credential-id".to_string());
        credential.team_id = Some("team-id".to_string());
        credential.owner_id = Some("owner-id".to_string());
        let id = repository
            .insert(&mut credential)
            .expect("insert credential");

        let loaded = repository.get(id).unwrap().unwrap();

        assert!(loaded.sync_enabled);
        assert_eq!(Some("cloud-credential-id"), loaded.cloud_id.as_deref());
        assert_eq!(Some("team-id"), loaded.team_id.as_deref());
        assert_eq!(Some("owner-id"), loaded.owner_id.as_deref());
    });
}

#[test]
fn repository_rejects_legacy_plaintext_and_corrupted_ciphertext() {
    with_master_key(|| {
        let (_temp, connection, repository) = test_repository();
        let mut credential = CredentialEntry::new("Legacy");
        let id = repository
            .insert(&mut credential)
            .expect("insert credential");

        for unsafe_value in ["legacy-plaintext", "ENC:not-valid-base64"] {
            connection
                .with_connection(|connection| {
                    connection.execute(
                        "UPDATE credential_entries SET password = ?1 WHERE id = ?2",
                        rusqlite::params![unsafe_value, id],
                    )?;
                    Ok(())
                })
                .expect("replace stored secret");

            let error = repository
                .get(id)
                .expect_err("unsafe stored secret must not be returned");
            assert!(
                error.to_string().contains("cannot be decrypted safely")
                    || error.to_string().contains("decryption failed")
            );
        }
    });
}

#[test]
fn repository_update_encrypts_replaces_and_clears_secrets() {
    with_master_key(|| {
        let (_temp, connection, repository) = test_repository();
        let mut credential = CredentialEntry::new("Rotating");
        credential.password = Some("first-secret".to_string());
        let id = repository
            .insert(&mut credential)
            .expect("insert credential");

        credential.password = Some("second-secret".to_string());
        repository.update(&credential).expect("replace secret");
        let stored: String = connection
            .with_connection(|connection| {
                Ok(connection.query_row(
                    "SELECT password FROM credential_entries WHERE id = ?1",
                    [id],
                    |row| row.get(0),
                )?)
            })
            .expect("read encrypted secret");
        assert!(stored.starts_with("ENC:"));
        assert!(!stored.contains("second-secret"));
        assert_eq!(
            Some("second-secret"),
            repository.get(id).unwrap().unwrap().password.as_deref()
        );

        credential.password = None;
        repository.update(&credential).expect("clear secret");
        assert_eq!(None, repository.get(id).unwrap().unwrap().password);
    });
}

#[test]
fn repository_rejects_ciphertext_encrypted_with_another_master_key() {
    let _guard = crypto_guard();
    crypto::set_master_key_for_session("credential-vault-first-key")
        .expect("configure first credential vault test key");
    let (_temp, _connection, repository) = test_repository();
    let mut credential = CredentialEntry::new("Wrong key");
    credential.password = Some("secret".to_string());
    let id = repository
        .insert(&mut credential)
        .expect("insert credential");

    crypto::set_master_key_for_session("credential-vault-second-key")
        .expect("configure second credential vault test key");
    let error = repository
        .get(id)
        .expect_err("wrong master key must fail closed");

    assert!(error.to_string().contains("decryption failed"));
    crypto::clear_master_key();
}

#[test]
fn repository_summary_reads_capabilities_while_vault_is_locked() {
    let _guard = crypto_guard();
    crypto::set_master_key_for_session("credential-vault-summary-key")
        .expect("configure credential vault summary test key");
    let (_temp, _connection, repository) = test_repository();
    let mut credential = CredentialEntry::new("Production SSH");
    credential.username = Some("deploy".to_string());
    credential.password = Some("secret".to_string());
    credential.private_key_path = Some("/Users/example/.ssh/id_ed25519".to_string());
    credential.private_key_content = Some("private-key".to_string());
    credential.passphrase = Some("passphrase".to_string());
    credential.sync_enabled = true;
    let id = repository
        .insert(&mut credential)
        .expect("insert credential");
    crypto::clear_master_key();

    let summary = repository
        .get_summary(id)
        .expect("read summary while locked")
        .expect("summary exists");
    assert_eq!("Production SSH", summary.name);
    assert_eq!(Some("deploy"), summary.username.as_deref());
    assert!(summary.has_password);
    assert!(summary.has_private_key_path);
    assert!(summary.has_private_key_content);
    assert!(summary.has_passphrase);
    assert!(summary.sync_enabled);
    assert_eq!(vec![summary], repository.list_summaries().unwrap());

    let error = repository
        .get(id)
        .expect_err("full credential must remain locked");
    assert!(error.to_string().contains("cannot be decrypted safely"));
}

#[test]
fn repository_summary_orders_rows_and_does_not_serialize_secrets() {
    let (_temp, connection, repository) = test_repository();
    let mut first = CredentialEntry::new("First");
    let first_id = repository.insert(&mut first).expect("insert first");
    let mut second = CredentialEntry::new("Second");
    second.private_key_path = Some("/local/key".to_string());
    let second_id = repository.insert(&mut second).expect("insert second");
    connection
        .with_connection(|connection| {
            connection.execute(
                "UPDATE credential_entries SET updated_at = ?1 WHERE id = ?2",
                rusqlite::params![20, first_id],
            )?;
            connection.execute(
                "UPDATE credential_entries SET updated_at = ?1 WHERE id = ?2",
                rusqlite::params![10, second_id],
            )?;
            Ok(())
        })
        .expect("set deterministic timestamps");

    let summaries = repository.list_summaries().expect("list summaries");
    assert_eq!(
        vec![first_id, second_id],
        summaries
            .iter()
            .map(|summary| summary.id)
            .collect::<Vec<_>>()
    );
    assert!(!summaries[0].has_private_key_path);
    assert!(summaries[1].has_private_key_path);
    assert!(
        repository
            .get_summary(i64::MAX)
            .expect("missing summary lookup")
            .is_none()
    );

    let json = serde_json::to_value(&summaries[1]).expect("serialize summary");
    for forbidden in [
        "password",
        "private_key_path",
        "private_key_content",
        "passphrase",
    ] {
        assert!(json.get(forbidden).is_none(), "{forbidden} must be absent");
    }
}

#[test]
fn credential_summary_type_contains_metadata_only() {
    fn assert_summary(_: &CredentialSummary) {}

    let (_temp, _connection, repository) = test_repository();
    let mut credential = CredentialEntry::new("Metadata");
    let id = repository
        .insert(&mut credential)
        .expect("insert credential");
    let summary = repository.get_summary(id).unwrap().unwrap();
    assert_summary(&summary);
}
