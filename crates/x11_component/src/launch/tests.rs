use std::time::Duration;

use x11_forwarding::{MagicCookie, ServerEndpoint};

use crate::lock::ComponentLock;
use crate::runtime::RuntimeIdentity;

use super::LaunchSpec;

#[test]
fn initial_port_conflict_rotates_endpoint_but_preserves_cookie() {
    let listener = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let port = listener.local_addr().unwrap().port();
    let root = std::env::temp_dir().join(format!("x11-port-retry-{}", std::process::id()));
    let cookie = MagicCookie::from_slice(&[6; 16]).unwrap();
    let mut spec = LaunchSpec {
        root: root.clone(),
        executable: root.join("yserver"),
        version: "manual".into(),
        min_vulkan: "1.3.0".into(),
        identity: RuntimeIdentity::loopback(port, cookie),
    };
    spec.refresh_endpoint_after_conflict().unwrap();
    let ServerEndpoint::Inet { port: rotated, .. } = spec.endpoint() else {
        panic!("应保留 TCP 端点类型");
    };
    assert_ne!(rotated, port);
    assert!(rotated >= 6000);
    assert_eq!(spec.cookie().unwrap().bytes(), &[6; 16]);
    let persisted = RuntimeIdentity::read(&root.join("runtime.json")).unwrap();
    assert_eq!(persisted.endpoint, spec.identity.endpoint);
    std::fs::remove_dir_all(root).unwrap();
}

#[tokio::test(flavor = "current_thread")]
async fn build_does_not_wait_for_another_instances_startup_probe() {
    let root = std::env::temp_dir().join(format!(
        "x11-nonblocking-build-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let data_dir = root;
    let component_root = data_dir.join("components/x11");
    std::fs::create_dir_all(&component_root).unwrap();
    let lock = ComponentLock::acquire(&component_root).unwrap();
    let (release_tx, release_rx) = std::sync::mpsc::channel();
    let releaser = std::thread::spawn(move || {
        let _ = release_rx.recv_timeout(Duration::from_secs(2));
        drop(lock);
    });
    let started = std::time::Instant::now();
    let mut spec = LaunchSpec::manual(data_dir.clone(), component_root.join("yserver")).unwrap();
    let elapsed = started.elapsed();
    release_tx.send(()).unwrap();
    releaser.join().unwrap();
    assert!(
        elapsed < Duration::from_millis(200),
        "build 在 async worker 等待组件锁 {elapsed:?}"
    );
    assert!(!component_root.join("runtime.json").exists());
    spec.reload_identity_for_spawn().unwrap();
    assert!(component_root.join("runtime.json").is_file());
    std::fs::remove_dir_all(data_dir).unwrap();
}

#[test]
fn spawn_reloads_identity_written_by_another_instance() {
    let root = std::env::temp_dir().join(format!(
        "x11-identity-reload-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let original = RuntimeIdentity::loopback(6010, MagicCookie::from_slice(&[1; 16]).unwrap());
    original.write(&root.join("runtime.json")).unwrap();
    let mut spec = LaunchSpec {
        root: root.clone(),
        executable: root.join("yserver"),
        version: "manual".into(),
        min_vulkan: "1.3.0".into(),
        identity: original,
    };
    let latest = RuntimeIdentity::loopback(6020, MagicCookie::from_slice(&[2; 16]).unwrap());
    latest.write(&root.join("runtime.json")).unwrap();
    spec.reload_identity_for_spawn().unwrap();
    assert_eq!(spec.identity.endpoint, latest.endpoint);
    assert_eq!(spec.cookie().unwrap(), latest.cookie().unwrap());
    std::fs::remove_dir_all(root).unwrap();
}
