use super::*;
use crate::explorer::load::load_workspace;

#[test]
fn non_repository_workspace_snapshot_keeps_the_requested_root() {
    let temp = std::env::temp_dir().join(format!(
        "workspace-explorer-non-repo-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&temp);
    std::fs::create_dir_all(&temp).unwrap();
    std::fs::write(temp.join("readme.txt"), "hello").unwrap();

    let snapshot = load_workspace(temp.clone(), false, false).unwrap();

    assert_eq!(temp.canonicalize().unwrap(), snapshot.root);
    assert!(snapshot.repository.is_none());
    assert_eq!(1, snapshot.entries.len());
    let _ = std::fs::remove_dir_all(temp);
}

#[test]
fn stale_git_results_are_rejected_after_workspace_changes() {
    let current = PathBuf::from("/workspace/current");
    let old = PathBuf::from("/workspace/old");

    let current_identity = identity(4, Some(&current));
    assert!(accepts_git_result(
        current_identity,
        identity(4, Some(&current))
    ));
    assert!(!accepts_git_result(
        current_identity,
        identity(3, Some(&current))
    ));
    assert!(!accepts_git_result(
        current_identity,
        identity(4, Some(&old))
    ));
    assert!(!accepts_git_result(current_identity, identity(4, None)));
}

#[test]
fn repository_root_stays_stable_when_terminal_moves_into_a_subdirectory() {
    let root = PathBuf::from("/workspace/repository");
    let child = root.join("src");
    let sibling = PathBuf::from("/workspace/other");

    assert!(!should_update_root(&root, &root, true));
    assert!(!should_update_root(&root, &child, true));
    assert!(should_update_root(&root, &sibling, true));
    assert!(should_update_root(&root, &child, false));
}

fn identity<'a>(generation: u64, repository: Option<&'a PathBuf>) -> GitResultIdentity<'a> {
    GitResultIdentity {
        generation,
        repository: repository.map(PathBuf::as_path),
    }
}
