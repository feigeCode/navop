use crate::storage::DeleteCredentialOutcome;
use crate::storage::traits::Repository;

use super::reference_scan_tests::{insert_credential, repositories, ssh_connection};

#[test]
fn protected_delete_reports_references_and_fails_closed_on_malformed_json() {
    let (_temp, connection, repository) = repositories();
    let credential_id = insert_credential(&repository);
    let mut ssh = ssh_connection(credential_id);
    repository.insert(&mut ssh).expect("insert ssh");

    let outcome = repository
        .credential_repository()
        .delete_checked(credential_id)
        .expect("protected delete");
    assert!(matches!(outcome, DeleteCredentialOutcome::Referenced(hits) if hits.len() == 3));

    connection
        .with_connection(|connection| {
            connection.execute(
                "UPDATE connections SET params = '{' WHERE id = ?1",
                [ssh.id.unwrap()],
            )?;
            Ok(())
        })
        .expect("corrupt connection params");
    let error = repository
        .credential_repository()
        .delete_checked(credential_id)
        .expect_err("malformed JSON must block deletion");
    assert!(error.to_string().contains("parse connection"));
    assert!(
        repository
            .credential_repository()
            .exists(credential_id)
            .expect("credential still exists")
    );
}

#[test]
fn protected_delete_distinguishes_deleted_and_missing_credentials() {
    let (_temp, _connection, repository) = repositories();
    let credential_id = insert_credential(&repository);
    let credentials = repository.credential_repository();

    assert_eq!(
        DeleteCredentialOutcome::Deleted,
        credentials.delete_checked(credential_id).unwrap()
    );
    assert_eq!(
        DeleteCredentialOutcome::NotFound,
        credentials.delete_checked(credential_id).unwrap()
    );
}

#[test]
fn repository_delete_cannot_bypass_reference_protection() {
    let (_temp, _connection, repository) = repositories();
    let credential_id = insert_credential(&repository);
    let mut ssh = ssh_connection(credential_id);
    repository.insert(&mut ssh).expect("insert ssh");
    let credentials = repository.credential_repository();

    let error = Repository::delete(&credentials, credential_id)
        .expect_err("generic repository deletion must remain protected");

    assert!(error.to_string().contains("still referenced"));
    assert!(credentials.exists(credential_id).unwrap());
}
