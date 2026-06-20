use public_mcp::discovery::{
    DiscoveryDocument, PublicMcpMode, read_discovery, remove_discovery, write_discovery,
};
use std::net::{IpAddr, Ipv4Addr, SocketAddr};

#[test]
fn discovery_round_trips_and_remove_is_idempotent() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("public-mcp.json");
    let document = DiscoveryDocument::new(
        std::process::id(),
        SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 49152),
        "secret-token".to_string(),
        PublicMcpMode::Temporary,
    );

    write_discovery(&path, &document).unwrap();

    let loaded = read_discovery(&path).unwrap();
    assert_eq!(document.version, loaded.version);
    assert_eq!(document.app, loaded.app);
    assert_eq!(document.pid, loaded.pid);
    assert_eq!(document.host, loaded.host);
    assert_eq!(document.port, loaded.port);
    assert_eq!(document.token, loaded.token);
    assert_eq!(document.mode, loaded.mode);

    remove_discovery(&path).unwrap();
    remove_discovery(&path).unwrap();
    assert!(!path.exists());
}

#[cfg(unix)]
#[test]
fn discovery_file_is_user_only_on_unix() {
    use std::os::unix::fs::PermissionsExt;

    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("public-mcp.json");
    let document = DiscoveryDocument::new(
        std::process::id(),
        SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 49152),
        "secret-token".to_string(),
        PublicMcpMode::Persistent,
    );

    write_discovery(&path, &document).unwrap();

    let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
    assert_eq!(0o600, mode);
}
