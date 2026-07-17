use public_mcp::discovery::{
    DiscoveryDocument, PublicMcpMode, legacy_public_mcp_discovery_path_from,
    public_mcp_discovery_path_from, read_discovery, remove_discovery, write_discovery,
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
    assert_eq!("navop", loaded.app);
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

#[test]
fn discovery_validation_accepts_navop_and_legacy_onetcli_apps() {
    let mut document = DiscoveryDocument::new(
        1,
        SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 49152),
        "a".repeat(64),
        PublicMcpMode::Persistent,
    );
    document.validate_for_stdio_bridge().unwrap();
    document.app = "onetcli".to_string();
    document.validate_for_stdio_bridge().unwrap();
    document.app = "other".to_string();
    assert!(document.validate_for_stdio_bridge().is_err());
}

#[test]
fn discovery_paths_prefer_new_navop_brand_and_retain_legacy_path() {
    let root = std::path::Path::new("/tmp/config");
    assert_eq!(
        root.join("navop/public-mcp.json"),
        public_mcp_discovery_path_from(root)
    );
    assert_eq!(
        root.join("onetcli/public-mcp.json"),
        legacy_public_mcp_discovery_path_from(root)
    );
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
