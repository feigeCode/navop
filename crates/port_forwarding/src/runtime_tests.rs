use one_core::storage::{
    ConnectionType, PortForwardingKind, PortForwardingParams, SshAuthMethod, SshParams,
    StoredConnection,
};

use crate::{
    PortForwardingRuntime, build_dynamic_forwarding_request, build_local_forwarding_request,
};

fn ssh_connection(id: i64) -> StoredConnection {
    let mut connection = StoredConnection::new_ssh(
        "bastion".to_string(),
        SshParams {
            host: "bastion.example.com".to_string(),
            port: 22,
            username: "deploy".to_string(),
            auth_method: SshAuthMethod::Agent,
            connect_timeout: Some(12),
            keepalive_interval: Some(20),
            keepalive_max: Some(3),
            default_directory: None,
            init_script: None,
            disable_shell_integration: None,
            x11_forwarding: None,
            jump_server: None,
            proxy: None,
            os_id: None,
            icon: None,
        },
        None,
    );
    connection.id = Some(id);
    connection
}

fn local_forwarding_connection(ssh_connection_id: i64) -> StoredConnection {
    StoredConnection::new_port_forwarding(
        "postgres tunnel".to_string(),
        PortForwardingParams {
            ssh_connection_id,
            kind: PortForwardingKind::Local,
            bind_host: "127.0.0.1".to_string(),
            bind_port: 15432,
            target_host: "db.internal".to_string(),
            target_port: 5432,
        },
        None,
    )
}

fn dynamic_forwarding_connection(ssh_connection_id: i64) -> StoredConnection {
    StoredConnection::new_port_forwarding(
        "socks tunnel".to_string(),
        PortForwardingParams {
            ssh_connection_id,
            kind: PortForwardingKind::Dynamic,
            bind_host: "127.0.0.1".to_string(),
            bind_port: 1080,
            target_host: String::new(),
            target_port: 0,
        },
        None,
    )
}

#[test]
fn local_request_uses_referenced_ssh_connection_and_forwarding_params() {
    let request =
        build_local_forwarding_request(&local_forwarding_connection(7), &ssh_connection(7))
            .unwrap();

    assert_eq!(request.bind_host, "127.0.0.1");
    assert_eq!(request.bind_port, 15432);
    assert_eq!(request.target_host, "db.internal");
    assert_eq!(request.target_port, 5432);
    assert_eq!(request.ssh_config.host, "bastion.example.com");
    assert_eq!(request.ssh_config.port, 22);
    assert_eq!(request.ssh_config.username, "deploy");
    assert_eq!(request.ssh_config.timeout.unwrap().as_secs(), 12);
    assert!(matches!(request.ssh_config.auth, ssh::SshAuth::Agent));
}

#[test]
fn local_request_rejects_non_matching_ssh_connection() {
    let error =
        match build_local_forwarding_request(&local_forwarding_connection(7), &ssh_connection(8)) {
            Ok(_) => panic!("expected mismatched SSH connection to fail"),
            Err(error) => error,
        };

    assert!(error.to_string().contains("referenced SSH connection"));
}

#[test]
fn local_request_rejects_non_port_forwarding_connection() {
    let error = match build_local_forwarding_request(&ssh_connection(7), &ssh_connection(7)) {
        Ok(_) => panic!("expected non Port Forwarding connection to fail"),
        Err(error) => error,
    };

    assert!(error.to_string().contains("Port Forwarding"));
}

#[test]
fn local_request_rejects_non_ssh_reference() {
    let mut reference = ssh_connection(7);
    reference.connection_type = ConnectionType::Database;

    let error = match build_local_forwarding_request(&local_forwarding_connection(7), &reference) {
        Ok(_) => panic!("expected non SSH/SFTP reference to fail"),
        Err(error) => error,
    };

    assert!(error.to_string().contains("SSH/SFTP"));
}

#[test]
fn dynamic_request_uses_referenced_ssh_connection_and_bind_params() {
    let request =
        build_dynamic_forwarding_request(&dynamic_forwarding_connection(7), &ssh_connection(7))
            .unwrap();

    assert_eq!(request.bind_host, "127.0.0.1");
    assert_eq!(request.bind_port, 1080);
    assert_eq!(request.ssh_config.host, "bastion.example.com");
    assert_eq!(request.ssh_config.username, "deploy");
}

#[tokio::test]
async fn stopping_unknown_connection_is_idempotent() {
    let mut runtime = PortForwardingRuntime::new();

    let stopped = runtime.stop(42).await.unwrap();

    assert!(!stopped);
    assert!(!runtime.is_running(42));
}
