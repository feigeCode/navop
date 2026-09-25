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
    // 固定初始分支名：`git init` 建哪个默认分支取决于本机 `init.defaultBranch`，
    // 不固定的话断言上游 ref 名的测试会随环境漂移（本机是 `main`，CI 上未必）。
    run_test_git(&root, &["symbolic-ref", "HEAD", "refs/heads/master"]);
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

#[test]
fn push_current_branch_publishes_to_default_remote_and_sets_upstream() {
    let root = initialized_repository();
    let repository = discover_repository(&root).unwrap().unwrap();
    // 本地 bare 仓库当远端，测试不触网。
    let remote_path = unique_test_path();
    std::fs::create_dir_all(&remote_path).unwrap();
    run_test_git(&remote_path, &["init", "-q", "--bare"]);
    // 远端也要固定初始分支：bare 仓库的 HEAD 同样随 `init.defaultBranch` 漂移，
    // 它指向哪个分支决定了远端的 HEAD 能不能被 `rev-parse` 解出来。
    run_test_git(&remote_path, &["symbolic-ref", "HEAD", "refs/heads/master"]);
    run_test_git(&root, &["remote", "add", "origin", remote_path.to_str().unwrap()]);
    run_test_git(&root, &["commit", "-q", "--allow-empty", "-m", "to push"]);

    push_current_branch(&repository).unwrap();

    let upstream = run_git_stdout(&root, &["rev-parse", "--abbrev-ref", "--symbolic-full-name", "@{u}"]);
    assert_eq!(
        "origin/master",
        upstream.trim(),
        "首次 push 应建立上游跟踪"
    );
    let remote_head = run_git_stdout(&remote_path, &["rev-parse", "HEAD"]);
    let local_head = run_git_stdout(&root, &["rev-parse", "HEAD"]);
    assert_eq!(local_head.trim(), remote_head.trim());

    std::fs::remove_dir_all(&remote_path).ok();
}

#[test]
fn push_current_branch_refuses_without_current_branch() {
    let root = initialized_repository();
    run_test_git(&root, &["checkout", "-q", "--detach"]);
    let repository = discover_repository(&root).unwrap().unwrap();

    let error = push_current_branch(&repository).unwrap_err();

    assert!(error.to_string().contains("detached"), "{error}");
}

#[test]
fn commit_context_lists_changes_untracked_and_bounds_size() {
    let root = initialized_repository();
    let repository = discover_repository(&root).unwrap().unwrap();
    std::fs::write(root.join("main.rs"), "fn main() {}\n// changed\n").unwrap();
    std::fs::write(root.join("brand_new.rs"), "pub fn new() {}\n").unwrap();

    let context = commit_context(&repository, 4 * 1024).unwrap();

    assert!(context.contains("main.rs"), "{context}");
    assert!(context.contains("New files:"), "{context}");
    assert!(context.contains("brand_new.rs"), "{context}");
    assert!(context.contains("Diff"), "{context}");

    let tiny = commit_context(&repository, 120).unwrap();
    assert!(
        tiny.contains("truncated"),
        "小上限必须截断: {tiny}"
    );
    assert!(tiny.len() <= 140);
}

#[test]
fn restore_checkpoint_rewrites_tracked_files_and_removes_untracked_ones() {
    let root = initialized_repository();
    let repository = discover_repository(&root).unwrap().unwrap();
    std::fs::write(root.join("main.rs"), "fn main() { println!(\"turn 1\"); }\n").unwrap();
    let snapshot = capture_worktree_snapshot(&repository).unwrap();

    // 往后再走一轮：改 tracked、加 untracked。
    std::fs::write(root.join("main.rs"), "fn main() { println!(\"turn 2\"); }\n").unwrap();
    std::fs::write(root.join("added.txt"), "added in turn 2\n").unwrap();

    restore_checkpoint(&repository, &snapshot).unwrap();

    let restored = std::fs::read_to_string(root.join("main.rs")).unwrap();
    assert!(restored.contains("turn 1"), "{restored}");
    assert!(
        !root.join("added.txt").exists(),
        "快照之后新增的 untracked 必须被清掉"
    );
}

#[test]
fn restore_checkpoint_leaves_ignored_files_alone() {
    let root = initialized_repository();
    std::fs::write(root.join(".gitignore"), "build/\n").unwrap();
    run_test_git(&root, &["add", ".gitignore"]);
    run_test_git(&root, &["commit", "-q", "-m", "ignore build"]);
    let repository = discover_repository(&root).unwrap().unwrap();
    let snapshot = capture_worktree_snapshot(&repository).unwrap();

    std::fs::create_dir_all(root.join("build")).unwrap();
    std::fs::write(root.join("build/out.bin"), "artifact\n").unwrap();

    restore_checkpoint(&repository, &snapshot).unwrap();

    assert!(
        root.join("build/out.bin").exists(),
        "`clean -fd` 不带 `-x`，.gitignore 命中的文件不能被删"
    );
}

#[test]
fn restore_checkpoint_backup_undoes_the_restore() {
    let root = initialized_repository();
    let repository = discover_repository(&root).unwrap().unwrap();
    std::fs::write(root.join("main.rs"), "fn main() { println!(\"turn 1\"); }\n").unwrap();
    let snapshot = capture_worktree_snapshot(&repository).unwrap();
    std::fs::write(root.join("main.rs"), "fn main() { println!(\"turn 2\"); }\n").unwrap();
    std::fs::write(root.join("added.txt"), "added in turn 2\n").unwrap();

    let restore = restore_checkpoint(&repository, &snapshot).unwrap();
    let undo = restore_checkpoint(&repository, &restore.backup).unwrap();

    assert_eq!(undo.target, restore.backup);
    let undone = std::fs::read_to_string(root.join("main.rs")).unwrap();
    assert!(undone.contains("turn 2"), "{undone}");
    assert!(
        root.join("added.txt").exists(),
        "撤销恢复应把当时的新文件带回来"
    );
}

#[test]
fn restore_checkpoint_reports_unavailable_snapshot() {
    let root = initialized_repository();
    let repository = discover_repository(&root).unwrap().unwrap();

    let error =
        restore_checkpoint(&repository, "0123456789abcdef0123456789abcdef01234567").unwrap_err();

    assert!(error.to_string().contains("unavailable"), "{error}");
}

#[test]
fn resolve_commit_accepts_a_commit_and_rejects_a_tree() {
    let root = initialized_repository();
    let repository = discover_repository(&root).unwrap().unwrap();

    let head = resolve_commit(&repository, "HEAD").unwrap();
    assert_eq!(40, head.len());

    let tree = run_git_stdout(&root, &["rev-parse", "HEAD^{tree}"]);
    assert!(
        resolve_commit(&repository, tree.trim()).is_err(),
        "树对象不是 commit，必须被拒绝"
    );
}

#[test]
fn turn_checkpoints_are_anchored_listed_and_deleted() {
    let root = initialized_repository();
    let repository = discover_repository(&root).unwrap().unwrap();
    std::fs::write(root.join("main.rs"), "fn main() {}\n// turn 1\n").unwrap();
    let first = capture_worktree_snapshot(&repository).unwrap();
    std::fs::write(root.join("main.rs"), "fn main() {}\n// turn 2\n").unwrap();
    let second = capture_worktree_snapshot(&repository).unwrap();

    anchor_turn_checkpoint(&repository, "turn_aaaa", &first).unwrap();
    anchor_turn_checkpoint(&repository, "turn_bbbb", &second).unwrap();

    let checkpoints = turn_checkpoints(&repository).unwrap();
    assert_eq!(2, checkpoints.len());
    assert_eq!(Some(&first), checkpoints.get("turn_aaaa"));
    assert_eq!(Some(&second), checkpoints.get("turn_bbbb"));

    delete_turn_checkpoints(&repository, &["turn_aaaa".to_string()]).unwrap();

    let remaining = turn_checkpoints(&repository).unwrap();
    assert_eq!(1, remaining.len());
    assert!(remaining.contains_key("turn_bbbb"));
}

#[test]
fn restore_backup_slot_holds_the_latest_pre_restore_state() {
    let root = initialized_repository();
    let repository = discover_repository(&root).unwrap().unwrap();
    std::fs::write(root.join("main.rs"), "fn main() { println!(\"turn 1\"); }\n").unwrap();
    let snapshot = capture_worktree_snapshot(&repository).unwrap();
    anchor_checkpoint(&repository, &snapshot).unwrap();
    assert_eq!(None, restore_backup(&repository).unwrap());

    std::fs::write(root.join("main.rs"), "fn main() { println!(\"turn 2\"); }\n").unwrap();
    let restore = restore_checkpoint(&repository, &snapshot).unwrap();

    assert_eq!(
        Some(restore.backup.clone()),
        restore_backup(&repository).unwrap(),
        "最近一次恢复前的现场要落进撤销槽位"
    );
    assert_eq!(
        Some(snapshot),
        anchored_checkpoint(&repository).unwrap(),
        "恢复动作不能改写基线 ref"
    );
}

#[test]
fn turn_checkpoints_are_isolated_from_the_baseline_ref() {
    let root = initialized_repository();
    let repository = discover_repository(&root).unwrap().unwrap();
    let snapshot = capture_worktree_snapshot(&repository).unwrap();
    anchor_checkpoint(&repository, &snapshot).unwrap();

    assert!(
        turn_checkpoints(&repository).unwrap().is_empty(),
        "基线 ref 不能被当成某一轮"
    );
    assert_eq!(Some(snapshot), anchored_checkpoint(&repository).unwrap());
}

#[test]
fn turn_checkpoints_and_backups_coexist_with_the_baseline_ref() {
    let root = initialized_repository();
    let repository = discover_repository(&root).unwrap().unwrap();
    let snapshot = capture_worktree_snapshot(&repository).unwrap();

    // git 不允许同一路径既是 ref 又是目录：基线、逐轮、撤销槽位三者必须能同时存在。
    // 逐轮 ref 曾经挂在基线 ref 之下，这条就是那个坑的回归守卫。
    anchor_checkpoint(&repository, &snapshot).unwrap();
    anchor_turn_checkpoint(&repository, "turn_cafe", &snapshot).unwrap();
    anchor_restore_backup(&repository, &snapshot).unwrap();

    assert_eq!(
        Some(snapshot.clone()),
        anchored_checkpoint(&repository).unwrap()
    );
    assert_eq!(
        Some(snapshot.clone()),
        turn_checkpoints(&repository)
            .unwrap()
            .get("turn_cafe")
            .cloned()
    );
    assert_eq!(Some(snapshot), restore_backup(&repository).unwrap());
}

#[test]
fn turn_checkpoint_ref_rejects_unsafe_turn_ids() {
    let root = initialized_repository();
    let repository = discover_repository(&root).unwrap().unwrap();

    for turn_id in ["", "../escape", "turn/1", "turn 1", "turn~1"] {
        assert!(
            turn_checkpoint_ref_name(&repository, turn_id).is_err(),
            "`{turn_id}` 应被拒绝"
        );
    }
    assert!(turn_checkpoint_ref_name(&repository, "turn_1a2b3c").is_ok());
}
