use super::{persistent_rollout_id, should_use_embedded};
use crate::X11ServerMode;

fn temp_root(name: &str) -> std::path::PathBuf {
    let path = std::env::temp_dir().join(format!(
        "x11-rollout-{name}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&path).unwrap();
    path
}

#[test]
fn rollout_id_is_persistent_and_bounds_are_enforced() {
    let root = temp_root("persistent");
    let first = persistent_rollout_id(&root).unwrap();
    let second = persistent_rollout_id(&root).unwrap();
    assert_eq!(first, second);
    assert_eq!(
        std::fs::read_to_string(root.join("rollout-id")).unwrap(),
        first.to_string()
    );
    assert!(!should_use_embedded(&root, X11ServerMode::Auto, 0).unwrap());
    assert!(should_use_embedded(&root, X11ServerMode::Auto, 100).unwrap());
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn explicit_embedded_allows_dogfood_and_external_skips_rollout() {
    let root = temp_root("dogfood");
    assert!(should_use_embedded(&root, X11ServerMode::Embedded, 0).unwrap());
    assert!(!should_use_embedded(&root, X11ServerMode::External, 100).unwrap());
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn corrupt_rollout_id_is_rejected() {
    let root = temp_root("invalid");
    std::fs::write(root.join("rollout-id"), "not-a-number").unwrap();
    assert!(persistent_rollout_id(&root).is_err());
    std::fs::remove_dir_all(root).unwrap();
}
