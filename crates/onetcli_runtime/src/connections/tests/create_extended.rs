use super::{connection_tool_registry, create_connection, repo};
use one_core::storage::traits::Repository;
use one_core::storage::{ConnectionType, RemoteDesktopProtocol, SerialFlowControl, SerialParity};
use serde_json::json;

#[test]
fn create_serial_connection_persists_serial_params() {
    let repo = repo();
    let registry = connection_tool_registry(repo.clone());
    let id = create_connection(
        &registry,
        json!({
            "kind": "serial",
            "values": {
                "name": "usb console",
                "port_name": "/dev/ttyUSB0",
                "baud_rate": 9600,
                "data_bits": 7,
                "stop_bits": 2,
                "parity": "Even",
                "flow_control": "Hardware"
            }
        }),
    );

    let stored = repo.get(id).unwrap().unwrap();
    let params = stored
        .to_serial_params()
        .expect("serial params should parse");
    assert_eq!(ConnectionType::Serial, stored.connection_type);
    assert_eq!("/dev/ttyUSB0", params.port_name);
    assert_eq!(9600, params.baud_rate);
    assert_eq!(7, params.data_bits);
    assert_eq!(2, params.stop_bits);
    assert_eq!(SerialParity::Even, params.parity);
    assert_eq!(SerialFlowControl::Hardware, params.flow_control);
}

#[test]
fn create_port_forwarding_connection_persists_forwarding_params() {
    let repo = repo();
    let registry = connection_tool_registry(repo.clone());
    let id = create_connection(
        &registry,
        json!({
            "kind": "port_forwarding",
            "values": {
                "name": "postgres tunnel",
                "ssh_connection_id": 42,
                "kind": "Dynamic",
                "bind_host": "127.0.0.1",
                "bind_port": 1080,
                "target_host": "db.internal",
                "target_port": 5432
            }
        }),
    );

    let stored = repo.get(id).unwrap().unwrap();
    let params = stored
        .to_port_forwarding_params()
        .expect("port forwarding params should parse");
    assert_eq!(ConnectionType::PortForwarding, stored.connection_type);
    assert_eq!(42, params.ssh_connection_id);
    assert_eq!("127.0.0.1", params.bind_host);
    assert_eq!(1080, params.bind_port);
    assert_eq!("db.internal", params.target_host);
    assert_eq!(5432, params.target_port);
}

#[test]
fn create_rdp_connection_persists_remote_desktop_params() {
    let repo = repo();
    let registry = connection_tool_registry(repo.clone());
    let id = create_connection(
        &registry,
        json!({
            "kind": "rdp",
            "values": {
                "name": "win host",
                "host": "10.0.1.30",
                "username": "administrator",
                "password": "secret",
                "domain": "corp",
                "read_only": true,
                "audio_playback": true
            }
        }),
    );

    let stored = repo.get(id).unwrap().unwrap();
    let params = stored
        .to_remote_desktop_params()
        .expect("rdp params should parse");
    assert_eq!(ConnectionType::Rdp, stored.connection_type);
    assert_eq!(RemoteDesktopProtocol::Rdp, params.protocol);
    assert_eq!(3389, params.port);
    assert_eq!(Some("corp"), params.domain.as_deref());
    assert!(params.read_only);
    assert!(params.audio_playback);
}

#[test]
fn create_vnc_connection_persists_remote_desktop_params() {
    let repo = repo();
    let registry = connection_tool_registry(repo.clone());
    let id = create_connection(
        &registry,
        json!({
            "kind": "vnc",
            "values": {
                "name": "linux desktop",
                "host": "10.0.1.31",
                "port": 5901,
                "password": "secret",
                "audio_playback": true
            }
        }),
    );

    let stored = repo.get(id).unwrap().unwrap();
    let params = stored
        .to_remote_desktop_params()
        .expect("vnc params should parse");
    assert_eq!(ConnectionType::Vnc, stored.connection_type);
    assert_eq!(RemoteDesktopProtocol::Vnc, params.protocol);
    assert_eq!(5901, params.port);
    assert_eq!(None, params.username);
    assert!(!params.audio_playback);
}
