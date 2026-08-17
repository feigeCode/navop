use crate::storage::traits::Repository;
use crate::storage::{ConnectionType, DeleteCredentialOutcome};

use super::reference_scan_tests::{
    insert_credential, port_forwarding_connection, repositories, ssh_connection,
};

#[test]
fn port_forwarding_indirect_reference_protects_credential_deletion() {
    let (_temp, _connection, repository) = repositories();
    let credential_id = insert_credential(&repository);
    let mut ssh = ssh_connection(credential_id);
    let ssh_id = repository.insert(&mut ssh).expect("insert SSH");
    let mut forwarding = port_forwarding_connection(ssh_id);
    repository
        .insert(&mut forwarding)
        .expect("insert port forwarding");

    let outcome = repository
        .credential_repository()
        .delete_checked(credential_id)
        .expect("protected delete");
    let DeleteCredentialOutcome::Referenced(hits) = outcome else {
        panic!("port forwarding must protect referenced credential");
    };
    let forwarding_hits = hits
        .iter()
        .filter(|hit| hit.connection_type == ConnectionType::PortForwarding)
        .collect::<Vec<_>>();
    assert_eq!(3, forwarding_hits.len());
    assert!(
        forwarding_hits
            .iter()
            .all(|hit| hit.via_ssh_connection_id == Some(ssh_id))
    );
    assert!(
        repository
            .credential_repository()
            .exists(credential_id)
            .expect("credential still exists")
    );
}

#[test]
fn protected_delete_does_not_require_master_key_for_existing_credential() {
    let (_temp, _connection, repository) = repositories();
    let credential_id = super::with_master_key(|| {
        let mut credential = crate::storage::CredentialEntry::new("Encrypted");
        credential.password = Some("secret".to_string());
        repository
            .credential_repository()
            .insert(&mut credential)
            .expect("insert encrypted credential")
    });

    assert_eq!(
        DeleteCredentialOutcome::Deleted,
        repository
            .credential_repository()
            .delete_checked(credential_id)
            .expect("delete while vault is locked")
    );
}
