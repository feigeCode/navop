use super::*;
use std::time::{SystemTime, UNIX_EPOCH};

#[test]
fn porcelain_parser_handles_regular_untracked_and_renamed_entries() {
    let changes =
        parse_porcelain_v1_z(b" M src/lib.rs\0?? new file.rs\0R  src/new.rs\0src/old.rs\0")
            .unwrap();

    assert_eq!(3, changes.len());
    assert_eq!(Path::new("new file.rs"), changes[0].path);
    assert_eq!(GitChangeKind::Untracked, changes[0].kind);
    assert_eq!(Path::new("src/lib.rs"), changes[1].path);
    assert_eq!(GitChangeKind::Modified, changes[1].kind);
    assert_eq!(Path::new("src/new.rs"), changes[2].path);
    assert_eq!(Some(PathBuf::from("src/old.rs")), changes[2].original_path);
    assert!(changes[2].staged);
}

#[test]
fn conflict_statuses_are_grouped_as_conflicted() {
    assert_eq!(GitChangeKind::Conflicted, change_kind('U', 'U'));
    assert_eq!(GitChangeKind::Conflicted, change_kind('A', 'A'));
    assert_eq!(GitChangeKind::Deleted, change_kind(' ', 'D'));
}

#[test]
fn repository_changes_and_diff_are_loaded_from_real_git_state() {
    let root = initialized_repository();
    std::fs::write(
        root.join("main.rs"),
        "fn main() { println!(\"changed\"); }\n",
    )
    .unwrap();

    let repository = discover_repository(&root).unwrap().unwrap();
    let change = load_changes(&repository)
        .unwrap()
        .into_iter()
        .find(|change| change.path == Path::new("main.rs"))
        .unwrap();
    let diff = load_diff(&repository, &change).unwrap();

    assert_eq!(GitChangeKind::Modified, change.kind);
    assert!(diff.contains("-fn main() {}"));
    assert!(diff.contains("+fn main() { println!(\"changed\"); }"));
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn untracked_text_diff_marks_missing_final_newline() {
    let root = initialized_repository();
    std::fs::write(root.join("new.txt"), "new content").unwrap();
    let repository = discover_repository(&root).unwrap().unwrap();
    let change = load_changes(&repository)
        .unwrap()
        .into_iter()
        .find(|change| change.path == Path::new("new.txt"))
        .unwrap();

    let diff = load_diff(&repository, &change).unwrap();

    assert!(diff.contains("new file mode 100644"));
    assert!(diff.contains("+new content"));
    assert!(diff.contains("\\ No newline at end of file"));
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn staged_rename_keeps_original_path_and_rename_diff() {
    let root = initialized_repository();
    run_test_git(&root, &["mv", "main.rs", "renamed.rs"]);
    let repository = discover_repository(&root).unwrap().unwrap();
    let change = load_changes(&repository)
        .unwrap()
        .into_iter()
        .find(|change| change.path == Path::new("renamed.rs"))
        .unwrap();

    let diff = load_diff(&repository, &change).unwrap();

    assert_eq!(GitChangeKind::Renamed, change.kind);
    assert_eq!(Some(PathBuf::from("main.rs")), change.original_path);
    assert!(diff.contains("rename from main.rs"));
    assert!(diff.contains("rename to renamed.rs"));
    let _ = std::fs::remove_dir_all(root);
}

fn initialized_repository() -> PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::temp_dir().join(format!(
        "workspace-explorer-git-{}-{nonce}",
        std::process::id()
    ));
    std::fs::create_dir_all(&root).unwrap();
    run_test_git(&root, &["init", "-q"]);
    run_test_git(&root, &["config", "user.name", "Workspace Explorer Test"]);
    run_test_git(
        &root,
        &["config", "user.email", "workspace-explorer@example.invalid"],
    );
    std::fs::write(root.join("main.rs"), "fn main() {}\n").unwrap();
    run_test_git(&root, &["add", "main.rs"]);
    run_test_git(&root, &["commit", "-q", "-m", "initial"]);
    root
}

fn run_test_git(root: &Path, args: &[&str]) {
    let output = Command::new("git")
        .current_dir(root)
        .args(args)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "git {args:?} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}
