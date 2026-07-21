use super::*;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

static TEST_REPOSITORY_ID: AtomicU64 = AtomicU64::new(0);

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
fn unchanged_file_yields_empty_diff_instead_of_error() {
    let root = initialized_repository();
    let repository = discover_repository(&root).unwrap().unwrap();
    let change = GitChange {
        path: PathBuf::from("main.rs"),
        original_path: None,
        kind: GitChangeKind::Modified,
        staged: false,
    };

    let diff = load_diff(&repository, &change).unwrap();

    assert!(diff.is_empty());
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn unborn_repository_diff_includes_changes_after_staging() {
    let root = empty_repository();
    std::fs::write(root.join("main.rs"), "staged\n").unwrap();
    run_test_git(&root, &["add", "main.rs"]);
    std::fs::write(root.join("main.rs"), "working tree\n").unwrap();

    let repository = discover_repository(&root).unwrap().unwrap();
    let change = load_changes(&repository)
        .unwrap()
        .into_iter()
        .find(|change| change.path == Path::new("main.rs"))
        .unwrap();
    let diff = load_diff(&repository, &change).unwrap();

    assert!(diff.contains("+working tree"));
    assert!(!diff.contains("+staged"));
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn invalid_repository_still_reports_diff_failure() {
    let root = unique_test_path();
    std::fs::create_dir_all(&root).unwrap();
    let repository = GitRepository {
        root: root.clone(),
        branch: None,
    };
    let change = GitChange {
        path: PathBuf::from("main.rs"),
        original_path: None,
        kind: GitChangeKind::Modified,
        staged: false,
    };

    assert!(load_diff(&repository, &change).is_err());
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

#[test]
fn branch_parser_separates_local_and_remote_branches() {
    let branches = parse_branches(
        "refs/heads/dev\tdev\t*\torigin/dev\n\
         refs/heads/main\tmain\t\torigin/main\n\
         refs/remotes/origin/HEAD\torigin/HEAD\t\t\n\
         refs/remotes/origin/dev\torigin/dev\t\t\n",
    )
    .unwrap();

    assert_eq!(3, branches.len());
    assert_eq!(GitBranchKind::Local, branches[0].kind);
    assert_eq!("dev", branches[0].name);
    assert!(branches[0].current);
    assert_eq!(Some("origin/dev".to_string()), branches[0].upstream);
    assert_eq!(GitBranchKind::Remote, branches[2].kind);
    assert_eq!("origin/dev", branches[2].name);
    assert!(branches.iter().all(|branch| branch.name != "origin"));
}

#[test]
fn local_branch_operations_create_switch_rename_and_merge() {
    let root = initialized_repository();
    let mut repository = discover_repository(&root).unwrap().unwrap();
    let base_branch = repository.branch.clone().unwrap();
    create_branch(&repository, "feature/test").unwrap();
    repository.branch = current_branch(&root);
    assert_eq!(Some("feature/test".to_string()), repository.branch);

    rename_branch(&repository, "feature/test", "feature/renamed").unwrap();
    std::fs::write(root.join("feature.txt"), "feature\n").unwrap();
    run_test_git(&root, &["add", "feature.txt"]);
    run_test_git(&root, &["commit", "-q", "-m", "feature"]);
    run_test_git(&root, &["switch", "-q", &base_branch]);

    merge_branch(&repository, "feature/renamed").unwrap();
    assert_eq!(
        "feature\n",
        std::fs::read_to_string(root.join("feature.txt")).unwrap()
    );

    let branch = load_branches(&repository)
        .unwrap()
        .into_iter()
        .find(|branch| branch.name == "feature/renamed")
        .unwrap();
    delete_branch(&repository, &branch).unwrap();
    assert!(
        load_branches(&repository)
            .unwrap()
            .iter()
            .all(|branch| branch.name != "feature/renamed")
    );
    let _ = std::fs::remove_dir_all(root);
}

fn initialized_repository() -> PathBuf {
    let root = empty_repository();
    std::fs::write(root.join("main.rs"), "fn main() {}\n").unwrap();
    run_test_git(&root, &["add", "main.rs"]);
    run_test_git(&root, &["commit", "-q", "-m", "initial"]);
    root
}

fn empty_repository() -> PathBuf {
    let root = unique_test_path();
    std::fs::create_dir_all(&root).unwrap();
    run_test_git(&root, &["init", "-q"]);
    run_test_git(&root, &["config", "user.name", "Workspace Explorer Test"]);
    run_test_git(
        &root,
        &["config", "user.email", "workspace-explorer@example.invalid"],
    );
    root
}

fn unique_test_path() -> PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    std::env::temp_dir().join(format!(
        "workspace-explorer-git-{}-{nonce}-{}",
        std::process::id(),
        TEST_REPOSITORY_ID.fetch_add(1, Ordering::Relaxed)
    ))
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
