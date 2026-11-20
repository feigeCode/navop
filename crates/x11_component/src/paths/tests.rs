use super::{atomic_write, component_root, prune_versions, prune_versions_preserving};

fn test_root(name: &str) -> std::path::PathBuf {
    let root = std::env::temp_dir().join(format!("x11-paths-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    root
}

#[test]
fn atomic_write_replaces_existing_files() {
    let root = test_root("replace");
    let path = root.join("runtime.json");
    atomic_write(&path, b"first").unwrap();
    atomic_write(&path, b"second").unwrap();
    assert_eq!(std::fs::read(path).unwrap(), b"second");
    assert_eq!(std::fs::read_dir(&root).unwrap().count(), 1);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn keeps_latest_two_versions() {
    let root = test_root("prune");
    for version in ["1.0.0", "1.2.0", "1.1.0"] {
        std::fs::create_dir_all(root.join("versions").join(version)).unwrap();
    }
    prune_versions(&root).unwrap();
    assert!(!root.join("versions/1.0.0").exists());
    assert!(root.join("versions/1.1.0").exists());
    assert!(root.join("versions/1.2.0").exists());
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn prune_preserves_active_old_version() {
    let root = test_root("prune-active");
    for version in ["0.9.0", "1.0.0", "1.1.0", "1.2.0"] {
        std::fs::create_dir_all(root.join("versions").join(version)).unwrap();
    }
    prune_versions_preserving(&root, &["0.9.0".into()].into()).unwrap();
    assert!(root.join("versions/0.9.0").exists());
    assert!(!root.join("versions/1.0.0").exists());
    assert!(root.join("versions/1.1.0").exists());
    assert!(root.join("versions/1.2.0").exists());
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn installing_older_version_preserves_current_and_installed_version() {
    let root = test_root("prune-older-current");
    for version in ["1.8.0", "1.9.0", "1.10.0"] {
        std::fs::create_dir_all(root.join("versions").join(version)).unwrap();
    }
    super::write_current(&root, "1.8.0").unwrap();
    prune_versions_preserving(&root, &["1.8.0".into()].into()).unwrap();
    assert!(
        root.join("versions/1.8.0").exists(),
        "刚写入 current 的版本不得删除"
    );
    assert!(
        root.join("versions/1.9.0").exists(),
        "仍保留最新两个非保护版本"
    );
    assert!(root.join("versions/1.10.0").exists());
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn injected_product_data_dir_is_not_nested() {
    let root = component_root(std::path::Path::new("/tmp/navop"));
    assert_eq!(root, std::path::Path::new("/tmp/navop/components/x11"));
}
