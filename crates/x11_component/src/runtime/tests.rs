use x11_forwarding::MagicCookie;

use super::RuntimeIdentity;

#[test]
fn restart_restores_endpoint_and_cookie() {
    let root = std::env::temp_dir().join(format!("x11-runtime-{}", std::process::id()));
    std::fs::create_dir_all(&root).unwrap();
    let path = root.join("runtime.json");
    let identity = RuntimeIdentity::loopback(6017, MagicCookie::from_slice(&[4; 16]).unwrap());
    identity.write(&path).unwrap();
    let restored = RuntimeIdentity::read(&path).unwrap();
    assert_eq!(restored.endpoint, identity.endpoint);
    assert_eq!(restored.cookie_hex(), identity.cookie_hex());
    assert_eq!(restored.cookie().unwrap().bytes(), &[4; 16]);
    std::fs::remove_dir_all(root).unwrap();
}

#[cfg(unix)]
#[test]
fn runtime_identity_is_always_owner_read_write_only() {
    use std::os::unix::fs::PermissionsExt as _;
    let root = std::env::temp_dir().join(format!("x11-runtime-mode-{}", std::process::id()));
    std::fs::create_dir_all(&root).unwrap();
    let path = root.join("runtime.json");
    let identity = RuntimeIdentity::loopback(6018, MagicCookie::from_slice(&[8; 16]).unwrap());
    identity.write(&path).unwrap();
    assert_eq!(
        std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
        0o600
    );
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
    RuntimeIdentity::read(&path).unwrap();
    assert_eq!(
        std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
        0o600
    );
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn non_loopback_identity_is_rejected() {
    let root = std::env::temp_dir().join(format!("x11-runtime-bad-{}", std::process::id()));
    std::fs::create_dir_all(&root).unwrap();
    let path = root.join("runtime.json");
    let mut identity = RuntimeIdentity::loopback(6000, MagicCookie::from_slice(&[4; 16]).unwrap());
    identity.endpoint = super::PersistentEndpoint::Inet {
        host: "0.0.0.0".into(),
        port: 6000,
    };
    std::fs::write(&path, serde_json::to_vec(&identity).unwrap()).unwrap();
    assert!(RuntimeIdentity::read(&path).is_err());
    std::fs::remove_dir_all(root).unwrap();
}
