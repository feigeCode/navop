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
fn change_operations_stage_unstage_and_discard_worktree_changes() {
    let root = initialized_repository();
    std::fs::write(root.join("main.rs"), "modified\n").unwrap();
    let repository = discover_repository(&root).unwrap().unwrap();
    let modified = find_change(&repository, "main.rs");

    stage_change(&repository, &modified).unwrap();
    assert!(find_change(&repository, "main.rs").staged);

    let staged = find_change(&repository, "main.rs");
    unstage_change(&repository, &staged).unwrap();
    assert!(!find_change(&repository, "main.rs").staged);

    let unstaged = find_change(&repository, "main.rs");
    discard_change(&repository, &unstaged).unwrap();
    assert_eq!(
        "fn main() {}\n",
        std::fs::read_to_string(root.join("main.rs")).unwrap()
    );
    assert!(load_changes(&repository).unwrap().is_empty());
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn discard_removes_untracked_and_staged_added_files() {
    let root = initialized_repository();
    let repository = discover_repository(&root).unwrap().unwrap();

    std::fs::create_dir(root.join("untracked")).unwrap();
    std::fs::write(root.join("untracked/file.txt"), "untracked\n").unwrap();
    let untracked = find_change(&repository, "untracked/file.txt");
    discard_change(&repository, &untracked).unwrap();
    assert!(!root.join("untracked").exists());

    std::fs::write(root.join("added.txt"), "added\n").unwrap();
    run_test_git(&root, &["add", "added.txt"]);
    let added = find_change(&repository, "added.txt");
    assert!(added.staged);
    discard_change(&repository, &added).unwrap();
    assert!(!root.join("added.txt").exists());
    assert!(load_changes(&repository).unwrap().is_empty());
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn discard_staged_rename_restores_original_path() {
    let root = initialized_repository();
    run_test_git(&root, &["mv", "main.rs", "renamed.rs"]);
    let repository = discover_repository(&root).unwrap().unwrap();
    let renamed = find_change(&repository, "renamed.rs");

    discard_change(&repository, &renamed).unwrap();

    assert!(root.join("main.rs").is_file());
    assert!(!root.join("renamed.rs").exists());
    assert!(load_changes(&repository).unwrap().is_empty());
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn unborn_repository_can_unstage_and_discard_added_file() {
    let root = empty_repository();
    std::fs::write(root.join("new.txt"), "new\n").unwrap();
    run_test_git(&root, &["add", "new.txt"]);
    let repository = discover_repository(&root).unwrap().unwrap();

    let staged = find_change(&repository, "new.txt");
    unstage_change(&repository, &staged).unwrap();
    assert!(root.join("new.txt").is_file());
    assert_eq!(
        GitChangeKind::Untracked,
        find_change(&repository, "new.txt").kind
    );

    let untracked = find_change(&repository, "new.txt");
    discard_change(&repository, &untracked).unwrap();
    assert!(!root.join("new.txt").exists());
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

#[test]
fn pushing_branch_sets_up_and_updates_remote_branch() {
    let root = initialized_repository();
    let remote = unique_test_path();
    std::fs::create_dir_all(&remote).unwrap();
    run_test_git(&remote, &["init", "-q", "--bare"]);
    run_test_git(
        &root,
        &["remote", "add", "origin", remote.to_str().unwrap()],
    );
    let repository = discover_repository(&root).unwrap().unwrap();
    let branch = load_branches(&repository)
        .unwrap()
        .into_iter()
        .find(|branch| branch.current)
        .unwrap();

    push_branch(&repository, &branch).unwrap();

    let remote_ref = Command::new("git")
        .current_dir(&remote)
        .args([
            "show-ref",
            "--verify",
            &format!("refs/heads/{}", branch.name),
        ])
        .output()
        .unwrap();
    assert!(remote_ref.status.success());
    let _ = std::fs::remove_dir_all(root);
    let _ = std::fs::remove_dir_all(remote);
}

fn find_change(repository: &GitRepository, path: &str) -> GitChange {
    load_changes(repository)
        .unwrap()
        .into_iter()
        .find(|change| change.path == Path::new(path))
        .unwrap_or_else(|| panic!("missing change for {path}"))
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
    run_test_git(&root, &["config", "core.autocrlf", "false"]);
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

#[test]
fn worktrees_are_created_listed_and_removed_with_their_branch() {
    let root = initialized_repository();
    let repository = discover_repository(&root).unwrap().unwrap();
    let worktree_root = unique_test_path();

    let created = create_worktree_in(&worktree_root, &repository, &root, None).unwrap();
    assert!(created.branch.starts_with("navop/"));
    assert!(created.worktree_root.is_dir());
    assert_eq!(created.worktree_root, created.path, "repo 根即项目根");

    let listed = list_worktrees(&repository).unwrap();
    assert!(listed.iter().any(|entry| entry.is_main), "主工作区应被标记");
    let linked = listed
        .iter()
        .find(|entry| entry.path == created.worktree_root)
        .expect("新建 worktree 应出现在列表里");
    assert!(linked.managed);
    assert!(!linked.is_main);

    remove_worktree(&repository, &created.worktree_root).unwrap();

    let after = list_worktrees(&repository).unwrap();
    assert!(
        after
            .iter()
            .all(|entry| entry.path != created.worktree_root),
        "删除后不应再出现在列表里"
    );
    let branches = run_git_stdout(&root, &["branch", "--list", &created.branch]);
    assert!(branches.trim().is_empty(), "受管分支应一并删除");

    std::fs::remove_dir_all(&worktree_root).ok();
}

#[test]
fn removing_the_main_worktree_is_refused() {
    let root = initialized_repository();
    let repository = discover_repository(&root).unwrap().unwrap();

    let error = remove_worktree(&repository, &repository.root).unwrap_err();

    assert!(error.to_string().contains("main worktree"), "{error}");
}

#[test]
fn worktree_parser_marks_managed_and_main_entries() {
    let output = "\
worktree /repo
HEAD abc
branch refs/heads/main

worktree /wt/one
HEAD def
branch refs/heads/navop/one

worktree /wt/detached
HEAD 0123
detached
";
    let entries = parse_worktrees(output, Path::new("/repo"));

    assert_eq!(3, entries.len());
    assert!(entries[0].is_main);
    assert!(!entries[0].managed);
    assert_eq!(Some("navop/one".to_string()), entries[1].branch);
    assert!(entries[1].managed);
    assert_eq!(None, entries[2].branch);
    assert!(!entries[2].managed);
}

fn run_git_stdout(root: &Path, args: &[&str]) -> String {
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
    String::from_utf8_lossy(&output.stdout).into_owned()
}

#[test]
fn worktree_snapshot_captures_dirty_and_untracked_files_without_touching_index() {
    let root = initialized_repository();
    let repository = discover_repository(&root).unwrap().unwrap();
    std::fs::write(root.join("main.rs"), "fn main() { println!(\"changed\"); }\n").unwrap();
    std::fs::write(root.join("untracked.txt"), "new\n").unwrap();

    let snapshot = capture_worktree_snapshot(&repository).unwrap();
    let diff = diff_snapshots(&repository, "HEAD", &snapshot).unwrap();
    let status = run_git_stdout(&root, &["status", "--porcelain"]);

    assert!(diff.contains("main.rs"));
    assert!(diff.contains("untracked.txt"));
    assert!(status.contains(" M main.rs"));
    assert!(status.contains("?? untracked.txt"));
}

#[test]
fn commit_all_commits_dirty_and_untracked_and_leaves_a_clean_tree() {
    let root = initialized_repository();
    let repository = discover_repository(&root).unwrap().unwrap();
    std::fs::write(root.join("main.rs"), "fn main() { println!(\"v2\"); }\n").unwrap();
    std::fs::write(root.join("added.txt"), "new file\n").unwrap();

    commit_all(&repository, "navop: commit all test").unwrap();

    let log = run_git_stdout(&root, &["log", "-1", "--format=%s"]);
    assert_eq!("navop: commit all test", log.trim());
    let committed = run_git_stdout(&root, &["show", "--stat", "--format=", "HEAD"]);
    assert!(committed.contains("main.rs"), "{committed}");
    assert!(committed.contains("added.txt"), "{committed}");
    let status = run_git_stdout(&root, &["status", "--porcelain"]);
    assert!(status.trim().is_empty(), "提交后工作区应干净: {status}");
}

#[test]
fn commit_all_rejects_blank_messages() {
    let root = initialized_repository();
    let repository = discover_repository(&root).unwrap().unwrap();

    let error = commit_all(&repository, "   ").unwrap_err();

    assert!(error.to_string().contains("empty"), "{error}");
}

#[test]
fn checkpoints_survive_restart_via_anchored_refs() {
    let root = initialized_repository();
    let repository = discover_repository(&root).unwrap().unwrap();
    std::fs::write(root.join("turn1.txt"), "one\n").unwrap();

    let snapshot = capture_worktree_snapshot(&repository).unwrap();
    anchor_checkpoint(&repository, &snapshot).unwrap();

    // "重启"：只从仓库 ref 恢复，不再依赖内存值。
    let restored = anchored_checkpoint(&repository)
        .unwrap()
        .expect("checkpoint should survive via ref");
    assert_eq!(snapshot, restored);

    // ref 不占分支名空间。
    let branches = run_git_stdout(&root, &["branch", "--list"]);
    assert!(
        !branches.contains("navop/checkpoints"),
        "checkpoint ref 不应出现在分支列表: {branches}"
    );
}

#[test]
fn anchored_checkpoint_is_none_before_first_capture() {
    let root = initialized_repository();
    let repository = discover_repository(&root).unwrap().unwrap();

    assert_eq!(None, anchored_checkpoint(&repository).unwrap());
}
