use std::collections::HashMap;

use connection_tunnel::SshTunnelConfig;

use crate::storage::traits::Repository;
use crate::storage::{
    ConnectionType, CredentialEntry, CredentialReferenceLocation, RemoteDesktopProtocol,
};

use super::reference_scan_tests::{
    database_connection, insert_credential, mongodb_connection, port_forwarding_connection,
    redis_connection, remote_desktop_connection, repositories, ssh_connection, tunnel_connection,
};

fn insert_connection(
    repository: &crate::storage::ConnectionRepository,
    mut connection: crate::storage::StoredConnection,
) -> i64 {
    repository
        .insert(&mut connection)
        .expect("insert connection")
}

fn insert_plain_ssh(repository: &crate::storage::ConnectionRepository) -> i64 {
    let mut connection = ssh_connection(i64::MAX);
    let mut params = connection.to_ssh_params().expect("parse SSH fixture");
    params.credential_reference = None;
    params.jump_server = None;
    params.proxy = None;
    connection.params = serde_json::to_string(&params).expect("serialize SSH fixture");
    insert_connection(repository, connection)
}

fn set_tunnel_enabled(
    connection: &mut crate::storage::StoredConnection,
    kind: &str,
    enabled: bool,
) {
    match kind {
        "database" => {
            let mut params = connection
                .to_db_connection()
                .expect("parse database fixture");
            params
                .extra_params
                .insert("ssh_tunnel_enabled".to_string(), enabled.to_string());
            connection.params = serde_json::to_string(&params).expect("serialize database");
        }
        "redis" => {
            let mut params = connection.to_redis_params().expect("parse Redis fixture");
            params.ssh_tunnel.as_mut().expect("Redis tunnel").enabled = enabled;
            connection.params = serde_json::to_string(&params).expect("serialize Redis");
        }
        "mongodb" => {
            let mut params = connection
                .to_mongodb_params()
                .expect("parse MongoDB fixture");
            params.ssh_tunnel.as_mut().expect("MongoDB tunnel").enabled = enabled;
            connection.params = serde_json::to_string(&params).expect("serialize MongoDB");
        }
        _ => unreachable!("unsupported tunnel fixture"),
    }
}

#[test]
fn disabled_tunnels_do_not_create_credential_references() {
    let (_temp, _connection, repository) = repositories();
    let credential_id = insert_credential(&repository);
    let ssh_id = insert_plain_ssh(&repository);

    for kind in ["database", "redis", "mongodb"] {
        let mut connection = tunnel_connection(kind, ssh_id);
        set_tunnel_enabled(&mut connection, kind, false);
        insert_connection(&repository, connection);
    }

    let hits = repository
        .credential_repository()
        .referencing_connections(credential_id)
        .expect("scan disabled tunnels");
    assert!(hits.is_empty());
}

#[test]
fn enabled_tunnels_without_connection_ids_fail_closed() {
    for kind in ["database", "redis", "mongodb"] {
        let (_temp, _connection, repository) = repositories();
        let credential_id = insert_credential(&repository);
        let mut connection = tunnel_connection(kind, 42);
        match kind {
            "database" => {
                let mut params = connection.to_db_connection().expect("parse database");
                params.extra_params.remove("ssh_connection_id");
                connection.params = serde_json::to_string(&params).expect("serialize database");
            }
            "redis" => {
                let mut params = connection.to_redis_params().expect("parse Redis");
                params.ssh_tunnel = Some(SshTunnelConfig {
                    enabled: true,
                    connection_id: None,
                    ..Default::default()
                });
                connection.params = serde_json::to_string(&params).expect("serialize Redis");
            }
            "mongodb" => {
                let mut params = connection.to_mongodb_params().expect("parse MongoDB");
                params.ssh_tunnel = Some(SshTunnelConfig {
                    enabled: true,
                    connection_id: None,
                    ..Default::default()
                });
                connection.params = serde_json::to_string(&params).expect("serialize MongoDB");
            }
            _ => unreachable!(),
        }
        insert_connection(&repository, connection);

        let error = repository
            .credential_repository()
            .referencing_connections(credential_id)
            .expect_err("missing tunnel ID must fail closed");
        assert!(
            error.to_string().contains("has no connection ID"),
            "{error}"
        );
    }
}

#[test]
fn invalid_database_tunnel_id_fails_closed() {
    let (_temp, _connection, repository) = repositories();
    let credential_id = insert_credential(&repository);
    let mut connection = tunnel_connection("database", 42);
    let mut params = connection.to_db_connection().expect("parse database");
    params
        .extra_params
        .insert("ssh_connection_id".to_string(), "not-an-id".to_string());
    connection.params = serde_json::to_string(&params).expect("serialize database");
    insert_connection(&repository, connection);

    let error = repository
        .credential_repository()
        .referencing_connections(credential_id)
        .expect_err("invalid database tunnel ID must fail closed");
    assert!(
        error
            .to_string()
            .contains("Invalid database SSH connection ID not-an-id"),
        "{error}"
    );
}

#[test]
fn missing_and_non_ssh_tunnel_targets_fail_closed() {
    let (_temp, _connection, repository) = repositories();
    let credential_id = insert_credential(&repository);
    insert_connection(&repository, port_forwarding_connection(999_999));
    let error = repository
        .credential_repository()
        .referencing_connections(credential_id)
        .expect_err("missing SSH target must fail closed");
    assert!(error.to_string().contains("was not found"), "{error}");

    let (_temp, _connection, repository) = repositories();
    let credential_id = insert_credential(&repository);
    let database_id = insert_connection(&repository, database_connection(i64::MAX));
    insert_connection(&repository, port_forwarding_connection(database_id));
    let error = repository
        .credential_repository()
        .referencing_connections(credential_id)
        .expect_err("non-SSH target must fail closed");
    assert!(
        error.to_string().contains("is not an SSH connection"),
        "{error}"
    );
}

#[test]
fn vnc_primary_and_proxy_references_are_scanned() {
    let (_temp, _connection, repository) = repositories();
    let credential_id = insert_credential(&repository);
    let mut connection = remote_desktop_connection(credential_id);
    let mut params = connection
        .to_remote_desktop_params()
        .expect("parse remote desktop fixture");
    params.protocol = RemoteDesktopProtocol::Vnc;
    connection.connection_type = ConnectionType::Vnc;
    connection.params = serde_json::to_string(&params).expect("serialize VNC");
    insert_connection(&repository, connection);

    let hits = repository
        .credential_repository()
        .referencing_connections(credential_id)
        .expect("scan VNC references");
    assert_eq!(2, hits.len());
    assert!(hits.iter().any(|hit| {
        hit.connection_type == ConnectionType::Vnc
            && hit.location == CredentialReferenceLocation::Primary
    }));
    assert!(hits.iter().any(|hit| {
        hit.connection_type == ConnectionType::Vnc
            && hit.location == CredentialReferenceLocation::Proxy
    }));
}

#[test]
fn unrelated_json_numbers_do_not_create_references() {
    let (_temp, _connection, repository) = repositories();
    let credential_id = insert_credential(&repository);
    let mut connection = database_connection(i64::MAX);
    let mut params = connection.to_db_connection().expect("parse database");
    params.credential_reference = None;
    params.proxy = None;
    params.extra_params = HashMap::from([(
        "unrelated_numeric_value".to_string(),
        credential_id.to_string(),
    )]);
    connection.params = serde_json::to_string(&params).expect("serialize database");
    insert_connection(&repository, connection);

    let hits = repository
        .credential_repository()
        .referencing_connections(credential_id)
        .expect("scan unrelated number");
    assert!(hits.is_empty());
}

#[test]
fn malformed_serial_params_are_ignored() {
    let (_temp, _connection, repository) = repositories();
    let credential_id = insert_credential(&repository);
    let mut connection = database_connection(i64::MAX);
    connection.connection_type = ConnectionType::Serial;
    connection.params = "{".to_string();
    insert_connection(&repository, connection);

    let hits = repository
        .credential_repository()
        .referencing_connections(credential_id)
        .expect("serial does not support credential references");
    assert!(hits.is_empty());
}

#[test]
fn scanning_summaries_does_not_require_an_unlocked_vault() {
    let (_temp, credential_id, repository) = {
        let (temp, _connection, repository) = repositories();
        let credential_id = super::with_master_key(|| {
            let mut credential = CredentialEntry::new("Locked", "username_password");
            credential.password = Some("secret".to_string());
            repository
                .credential_repository()
                .insert(&mut credential)
                .expect("insert encrypted credential")
        });
        (temp, credential_id, repository)
    };
    insert_connection(&repository, ssh_connection(credential_id));

    let hits = repository
        .credential_repository()
        .referencing_connections(credential_id)
        .expect("scan while vault is locked");
    assert_eq!(3, hits.len());
}
