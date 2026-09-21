use anyhow::{Context as _, Result, anyhow};
use process_util::configure_background_child;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GitRepository {
    pub root: PathBuf,
    pub branch: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GitBranchKind {
    Local,
    Remote,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GitBranch {
    pub name: String,
    pub kind: GitBranchKind,
    pub current: bool,
    pub upstream: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GitChangeKind {
    Added,
    Modified,
    Deleted,
    Renamed,
    Untracked,
    Conflicted,
}

impl GitChangeKind {
    pub fn badge(self) -> &'static str {
        match self {
            Self::Added => "A",
            Self::Modified => "M",
            Self::Deleted => "D",
            Self::Renamed => "R",
            Self::Untracked => "U",
            Self::Conflicted => "!",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GitChange {
    pub path: PathBuf,
    pub original_path: Option<PathBuf>,
    pub kind: GitChangeKind,
    pub staged: bool,
}

pub fn discover_repository(path: &Path) -> Result<Option<GitRepository>> {
    let output = run_git(path, ["rev-parse", "--show-toplevel"])?;
    if !output.status.success() {
        return Ok(None);
    }
    let root = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if root.is_empty() {
        return Ok(None);
    }
    let root = PathBuf::from(root);
    let branch = current_branch(&root);
    Ok(Some(GitRepository { root, branch }))
}

/// 本应用创建的 worktree 分支前缀。
///
/// 删除时只清理该前缀下的分支，绝不碰用户自己的分支。
const MANAGED_WORKTREE_BRANCH_PREFIX: &str = "navop/";

/// 新建 worktree 的结果：包含被注册到仓库里的真实路径。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CreatedWorktree {
    /// 从当前项目映射到新 worktree 内的路径（可能不是 worktree 根）。
    pub path: PathBuf,
    /// 新 worktree 的根目录。
    pub worktree_root: PathBuf,
    pub branch: String,
}

/// 仓库已注册的一个 worktree。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WorktreeEntry {
    pub path: PathBuf,
    /// `None` 表示 detached HEAD。
    pub branch: Option<String>,
    /// 是否是仓库主工作区（不可删除）。
    pub is_main: bool,
    /// 是否受本应用管理（分支带 `navop/` 前缀）。
    pub managed: bool,
}

pub fn create_worktree(
    repository: &GitRepository,
    project_root: &Path,
    base_branch: Option<&str>,
) -> Result<CreatedWorktree> {
    let worktree_root = dirs::home_dir()
        .ok_or_else(|| anyhow!("Home directory is unavailable"))?
        .join(".navop/worktrees");
    create_worktree_in(&worktree_root, repository, project_root, base_branch)
}

/// 在指定根目录下创建 worktree；根目录可注入，便于测试不污染用户目录。
pub(crate) fn create_worktree_in(
    worktree_root: &Path,
    repository: &GitRepository,
    project_root: &Path,
    base_branch: Option<&str>,
) -> Result<CreatedWorktree> {
    let relative_project = project_root
        .strip_prefix(&repository.root)
        .unwrap_or(Path::new(""));
    let base = base_branch
        .map(str::trim)
        .filter(|branch| !branch.is_empty())
        .map(str::to_owned)
        .or_else(|| {
            repository
                .branch
                .clone()
                .filter(|branch| !branch.starts_with("detached@"))
        })
        .unwrap_or_else(|| "HEAD".to_string());
    let name = format!(
        "{}-{}",
        repository
            .root
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("project"),
        &uuid::Uuid::new_v4().simple().to_string()[..8]
    );
    let worktree_root = worktree_root.join(&name);
    fs::create_dir_all(
        worktree_root
            .parent()
            .ok_or_else(|| anyhow!("Worktree root has no parent"))?,
    )?;
    let branch = format!("{MANAGED_WORKTREE_BRANCH_PREFIX}{name}");
    let output = run_git_vec(
        &repository.root,
        vec![
            "worktree".to_string(),
            "add".to_string(),
            "-b".to_string(),
            branch.clone(),
            worktree_root.to_string_lossy().into_owned(),
            base,
        ],
    )?;
    if !output.status.success() {
        return Err(git_command_error("git worktree add", &output));
    }
    // 统一使用真实路径：macOS 的 `/var` → `/private/var` 等符号链接会让字符串比较失配。
    let worktree_root = fs::canonicalize(&worktree_root).unwrap_or(worktree_root);
    Ok(CreatedWorktree {
        path: worktree_root.join(relative_project),
        worktree_root,
        branch,
    })
}

/// 列出仓库已注册的 worktree。数据来自 git 自身，跨重启保持有效。
pub fn list_worktrees(repository: &GitRepository) -> Result<Vec<WorktreeEntry>> {
    let output = run_git(&repository.root, ["worktree", "list", "--porcelain"])?;
    if !output.status.success() {
        return Err(git_command_error("git worktree list", &output));
    }
    let main_root = canonical_or_self(&repository.root);
    Ok(parse_worktrees(&String::from_utf8_lossy(&output.stdout), &main_root))
}

fn canonical_or_self(path: &Path) -> PathBuf {
    fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}

/// 删除一个受管理的 worktree：先移除工作目录，再删除其分支。
///
/// 主工作区与不受管理的 worktree 会被拒绝，避免误删用户自己的检出。
pub fn remove_worktree(repository: &GitRepository, path: &Path) -> Result<()> {
    let target = canonical_or_self(path);
    let entry = list_worktrees(repository)?
        .into_iter()
        .find(|entry| entry.path == target)
        .ok_or_else(|| anyhow!("Worktree is not registered: {}", path.display()))?;
    if entry.is_main {
        return Err(anyhow!("Refusing to remove the main worktree"));
    }
    if !entry.managed {
        return Err(anyhow!(
            "Refusing to remove a worktree this app did not create: {}",
            path.display()
        ));
    }
    let output = run_git_vec(
        &repository.root,
        vec![
            "worktree".to_string(),
            "remove".to_string(),
            "--force".to_string(),
            path.to_string_lossy().into_owned(),
        ],
    )?;
    if !output.status.success() {
        return Err(git_command_error("git worktree remove", &output));
    }
    if let Some(branch) = entry.branch.as_deref().filter(|branch| is_managed_branch(branch)) {
        let output = run_git_vec(
            &repository.root,
            vec![
                "branch".to_string(),
                "-D".to_string(),
                branch.to_string(),
            ],
        )?;
        if !output.status.success() {
            return Err(git_command_error("git branch -D", &output));
        }
    }
    // 清理 git 侧残留的 worktree 记录。
    let _ = run_git(&repository.root, ["worktree", "prune"]);
    Ok(())
}

/// Capture current worktree, including untracked files, without touching user's index.
pub fn capture_worktree_snapshot(repository: &GitRepository) -> Result<String> {
    let common_dir = git_stdout(&repository.root, &["rev-parse", "--git-common-dir"])?;
    let common_dir = PathBuf::from(common_dir.trim());
    let common_dir = if common_dir.is_absolute() {
        common_dir
    } else {
        repository.root.join(common_dir)
    };
    let temporary_index = common_dir.join(format!("navop-checkpoint-index-{}", uuid::Uuid::new_v4()));
    let result = (|| {
        let head = git_stdout(&repository.root, &["rev-parse", "--verify", "HEAD"])?;
        run_git_with_index(&repository.root, &temporary_index, &["read-tree", &head])?;
        run_git_with_index(&repository.root, &temporary_index, &["add", "-A", "--", "."])?;
        let tree = git_stdout_with_index(&repository.root, &temporary_index, &["write-tree"])?;
        let commit = git_output_with_index(
            &repository.root,
            &temporary_index,
            &["commit-tree", tree.trim(), "-p", &head, "-m", "Navop worktree checkpoint"],
        )?;
        let commit = commit.trim();
        (!commit.is_empty())
            .then(|| commit.to_string())
            .ok_or_else(|| anyhow!("git commit-tree returned no object id"))
    })();
    let _ = fs::remove_file(&temporary_index);
    let _ = fs::remove_file(temporary_index.with_extension("lock"));
    result
}

/// Compare two checkpoint/tree-ish values without changing working files or index.
pub fn diff_snapshots(repository: &GitRepository, before: &str, after: &str) -> Result<String> {
    let output = run_git_vec(
        &repository.root,
        vec![
            "diff".to_string(),
            "--binary".to_string(),
            "--find-renames".to_string(),
            before.to_string(),
            after.to_string(),
        ],
    )?;
    if !output.status.success() {
        return Err(git_command_error("git diff snapshots", &output));
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

/// 提交全部变更（含 untracked）。
///
/// 走用户真实 index：`add -A` 后 `git commit`。与快照路径不同——checkpoint 是
/// dangling commit 不动分支；这里必须真正推进分支，绕过 index 直接 update-ref
/// 会让真实 index 与新 HEAD 脱节（staged 状态错乱、出现假删除）。
pub fn commit_all(repository: &GitRepository, message: &str) -> Result<()> {
    let message = message.trim();
    if message.is_empty() {
        return Err(anyhow!("Commit message cannot be empty"));
    }
    run_git_operation(
        repository,
        "git add -A",
        vec![
            "add".to_string(),
            "-A".to_string(),
            "--".to_string(),
            ".".to_string(),
        ],
    )?;
    run_git_operation(
        repository,
        "git commit",
        vec![
            "commit".to_string(),
            "-m".to_string(),
            message.to_string(),
        ],
    )
}

fn run_git_with_index(cwd: &Path, index: &Path, args: &[&str]) -> Result<()> {
    let output = git_command(cwd, args).env("GIT_INDEX_FILE", index).output()?;
    if output.status.success() {
        Ok(())
    } else {
        Err(git_command_error("git checkpoint", &output))
    }
}

fn git_stdout_with_index(cwd: &Path, index: &Path, args: &[&str]) -> Result<String> {
    let output = git_command(cwd, args).env("GIT_INDEX_FILE", index).output()?;
    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).trim().to_owned())
    } else {
        Err(git_command_error("git checkpoint", &output))
    }
}

fn git_output_with_index(cwd: &Path, index: &Path, args: &[&str]) -> Result<String> {
    let output = git_command(cwd, &[])
        .env("GIT_INDEX_FILE", index)
        .args(["-c", "user.name=Navop", "-c", "user.email=navop@localhost"])
        .args(args)
        .output()?;
    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).trim().to_owned())
    } else {
        Err(git_command_error("git commit-tree", &output))
    }
}

fn git_command(cwd: &Path, args: &[&str]) -> Command {
    let mut command = Command::new("git");
    configure_background_child(&mut command);
    command.current_dir(cwd).args(args);
    command
}

fn git_stdout(cwd: &Path, args: &[&str]) -> Result<String> {
    let output = git_command(cwd, args).output()?;
    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).trim().to_owned())
    } else {
        Err(git_command_error("git", &output))
    }
}

fn is_managed_branch(branch: &str) -> bool {
    branch.starts_with(MANAGED_WORKTREE_BRANCH_PREFIX)
}

fn parse_worktrees(output: &str, main_root: &Path) -> Vec<WorktreeEntry> {
    let mut entries = Vec::new();
    let mut path: Option<PathBuf> = None;
    let mut branch: Option<String> = None;
    let mut flush = |path: &mut Option<PathBuf>, branch: &mut Option<String>| {
        if let Some(path) = path.take() {
            let canonical = canonical_or_self(&path);
            let branch = branch.take();
            entries.push(WorktreeEntry {
                is_main: canonical == main_root,
                managed: branch.as_deref().is_some_and(is_managed_branch),
                path: canonical,
                branch,
            });
        }
    };
    for line in output.lines() {
        if line.is_empty() {
            flush(&mut path, &mut branch);
            continue;
        }
        if let Some(value) = line.strip_prefix("worktree ") {
            flush(&mut path, &mut branch);
            path = Some(PathBuf::from(value));
        } else if let Some(value) = line.strip_prefix("branch ") {
            branch = value
                .strip_prefix("refs/heads/")
                .map(str::to_owned)
                .or_else(|| Some(value.to_owned()));
        }
    }
    flush(&mut path, &mut branch);
    entries
}


pub fn load_changes(repository: &GitRepository) -> Result<Vec<GitChange>> {
    let output = run_git(
        &repository.root,
        ["status", "--porcelain=v1", "-z", "--untracked-files=all"],
    )?;
    if !output.status.success() {
        return Err(git_command_error("git status", &output));
    }
    parse_porcelain_v1_z(&output.stdout)
}

pub fn stage_change(repository: &GitRepository, change: &GitChange) -> Result<()> {
    let mut args = vec!["add".to_string(), "-A".to_string(), "--".to_string()];
    append_change_path_args(&mut args, change);
    run_git_operation(repository, "git add", args)
}

pub fn unstage_change(repository: &GitRepository, change: &GitChange) -> Result<()> {
    let mut args = if repository_has_head(repository)? {
        vec![
            "reset".to_string(),
            "-q".to_string(),
            "HEAD".to_string(),
            "--".to_string(),
        ]
    } else {
        vec![
            "rm".to_string(),
            "--cached".to_string(),
            "-r".to_string(),
            "--ignore-unmatch".to_string(),
            "--".to_string(),
        ]
    };
    append_change_path_args(&mut args, change);
    run_git_operation(repository, "git unstage", args)
}

/// Restores all index and working-tree changes represented by `change`.
///
/// Paths that exist in HEAD are restored from HEAD. Newly-added and untracked
/// paths are removed from the working tree after their index entries have been
/// reset. Handling each rename path independently avoids `git restore`
/// rejecting the new side of a rename because it does not exist in HEAD.
pub fn discard_change(repository: &GitRepository, change: &GitChange) -> Result<()> {
    if change.kind == GitChangeKind::Untracked {
        let path = repository.root.join(&change.path);
        remove_worktree_path(&path)?;
        remove_empty_parent_directories(&repository.root, path.parent())?;
        return Ok(());
    }

    if !repository_has_head(repository)? {
        if change.staged {
            unstage_change(repository, change)?;
        }
        for path in change_paths(change) {
            let path = repository.root.join(path);
            remove_worktree_path_if_present(&path)?;
            remove_empty_parent_directories(&repository.root, path.parent())?;
        }
        return Ok(());
    }

    let mut reset_args = vec![
        "reset".to_string(),
        "-q".to_string(),
        "HEAD".to_string(),
        "--".to_string(),
    ];
    append_change_path_args(&mut reset_args, change);
    run_git_operation(repository, "git reset", reset_args)?;

    for path in change_paths(change) {
        if path_exists_in_head(repository, path)? {
            run_git_operation(
                repository,
                "git restore",
                vec![
                    "restore".to_string(),
                    "--source=HEAD".to_string(),
                    "--worktree".to_string(),
                    "--".to_string(),
                    path.to_string_lossy().into_owned(),
                ],
            )?;
        } else {
            let path = repository.root.join(path);
            remove_worktree_path_if_present(&path)?;
            remove_empty_parent_directories(&repository.root, path.parent())?;
        }
    }
    Ok(())
}

pub fn load_branches(repository: &GitRepository) -> Result<Vec<GitBranch>> {
    let output = run_git(
        &repository.root,
        [
            "for-each-ref",
            "--sort=refname",
            "--format=%(refname)\t%(refname:short)\t%(HEAD)\t%(upstream:short)",
            "refs/heads",
            "refs/remotes",
        ],
    )?;
    if !output.status.success() {
        return Err(git_command_error("git for-each-ref", &output));
    }
    parse_branches(&String::from_utf8_lossy(&output.stdout))
}

pub fn switch_branch(repository: &GitRepository, branch: &GitBranch) -> Result<()> {
    let args = match branch.kind {
        GitBranchKind::Local => vec!["switch".to_string(), branch.name.clone()],
        GitBranchKind::Remote => vec![
            "switch".to_string(),
            "--track".to_string(),
            branch.name.clone(),
        ],
    };
    run_git_operation(repository, "git switch", args)
}

pub fn create_branch(repository: &GitRepository, name: &str) -> Result<()> {
    validate_branch_name(repository, name)?;
    run_git_operation(
        repository,
        "git switch -c",
        vec!["switch".to_string(), "-c".to_string(), name.to_string()],
    )
}

pub fn rename_branch(
    repository: &GitRepository,
    old_name: &str,
    new_name: &str,
) -> Result<()> {
    validate_branch_name(repository, new_name)?;
    run_git_operation(
        repository,
        "git branch -m",
        vec![
            "branch".to_string(),
            "-m".to_string(),
            old_name.to_string(),
            new_name.to_string(),
        ],
    )
}

pub fn merge_branch(repository: &GitRepository, name: &str) -> Result<()> {
    run_git_operation(
        repository,
        "git merge",
        vec![
            "merge".to_string(),
            "--no-edit".to_string(),
            name.to_string(),
        ],
    )
}

pub fn delete_branch(repository: &GitRepository, branch: &GitBranch) -> Result<()> {
    let args = match branch.kind {
        GitBranchKind::Local => vec!["branch".to_string(), "-d".to_string(), branch.name.clone()],
        GitBranchKind::Remote => {
            let (remote, name) = branch
                .name
                .split_once('/')
                .ok_or_else(|| anyhow!("Invalid remote branch name: {}", branch.name))?;
            vec![
                "push".to_string(),
                remote.to_string(),
                "--delete".to_string(),
                name.to_string(),
            ]
        }
    };
    run_git_operation(repository, "git branch delete", args)
}

pub fn fetch_branches(repository: &GitRepository) -> Result<()> {
    run_git_operation(
        repository,
        "git fetch",
        ["fetch", "--all", "--prune"]
            .into_iter()
            .map(str::to_string)
            .collect(),
    )
}

pub fn push_branch(repository: &GitRepository, branch: &GitBranch) -> Result<()> {
    if branch.kind != GitBranchKind::Local {
        return Err(anyhow!("Only local branches can be pushed"));
    }
    let args = if let Some(upstream) = branch.upstream.as_deref() {
        let (remote, remote_branch) = upstream
            .split_once('/')
            .ok_or_else(|| anyhow!("Invalid upstream branch: {upstream}"))?;
        vec![
            "push".to_string(),
            remote.to_string(),
            format!("{}:{remote_branch}", branch.name),
        ]
    } else {
        let remote = default_remote(repository)?;
        vec![
            "push".to_string(),
            "-u".to_string(),
            remote,
            branch.name.clone(),
        ]
    };
    run_git_operation(repository, "git push", args)
}

pub fn load_diff(repository: &GitRepository, change: &GitChange) -> Result<String> {
    if change.kind == GitChangeKind::Untracked {
        return untracked_file_diff(repository, change);
    }

    let base = diff_base(repository)?;
    if let Some(diff) = try_load_diff(repository, change, base, true)? {
        return Ok(diff);
    }
    if let Some(diff) = try_load_diff(repository, change, base, false)? {
        return Ok(diff);
    }
    Ok(String::new())
}

const EMPTY_TREE: &str = "4b825dc642cb6eb9a060e54bf8d69288fbee4904";

/// Requests enough context lines for the side-by-side diff view to render the
/// whole file instead of isolated hunks.
const FULL_FILE_CONTEXT: &str = "--unified=1000000";

fn diff_base(repository: &GitRepository) -> Result<&'static str> {
    let output = run_git(
        &repository.root,
        ["rev-parse", "--verify", "--quiet", "HEAD"],
    )?;
    Ok(if output.status.success() {
        "HEAD"
    } else {
        EMPTY_TREE
    })
}

/// Loads the diff for a change, returning `Ok(None)` when the file no longer
/// differs from HEAD (for example a stale change entry after a commit) or when
/// the full-context command failed and the caller should retry with the
/// default context window.
fn try_load_diff(
    repository: &GitRepository,
    change: &GitChange,
    base: &str,
    full_context: bool,
) -> Result<Option<String>> {
    let output = git_diff_against_base(repository, change, base, full_context)?;
    if output.status.success() {
        let diff = String::from_utf8_lossy(&output.stdout).to_string();
        if !diff.is_empty() {
            return Ok(Some(diff));
        }
        return Ok(None);
    } else if full_context {
        return Ok(None);
    }

    let mut combined = String::new();
    let mut fallback_succeeded = false;
    for cached in [true, false] {
        let fallback = git_diff(repository, change, cached, full_context)?;
        if fallback.status.success() {
            fallback_succeeded = true;
            combined.push_str(&String::from_utf8_lossy(&fallback.stdout));
        }
    }
    if !combined.is_empty() {
        Ok(Some(combined))
    } else if fallback_succeeded {
        Ok(None)
    } else {
        Err(git_command_error("git diff", &output))
    }
}

fn current_branch(root: &Path) -> Option<String> {
    let branch = run_git(root, ["symbolic-ref", "--quiet", "--short", "HEAD"]).ok()?;
    if branch.status.success() {
        let value = String::from_utf8_lossy(&branch.stdout).trim().to_string();
        return (!value.is_empty()).then_some(value);
    }
    let revision = run_git(root, ["rev-parse", "--short", "HEAD"]).ok()?;
    revision.status.success().then(|| {
        let value = String::from_utf8_lossy(&revision.stdout).trim().to_string();
        format!("detached@{value}")
    })
}

fn parse_branches(output: &str) -> Result<Vec<GitBranch>> {
    let mut branches = Vec::new();
    for line in output.lines().filter(|line| !line.is_empty()) {
        let mut fields = line.splitn(4, '\t');
        let ref_name = fields
            .next()
            .ok_or_else(|| anyhow!("Invalid git branch entry"))?;
        let name = fields
            .next()
            .ok_or_else(|| anyhow!("Invalid git branch entry"))?;
        let head = fields
            .next()
            .ok_or_else(|| anyhow!("Invalid git branch entry"))?;
        let upstream = fields
            .next()
            .ok_or_else(|| anyhow!("Invalid git branch entry"))?;
        let kind = if ref_name.starts_with("refs/heads/") {
            GitBranchKind::Local
        } else if ref_name.starts_with("refs/remotes/") {
            GitBranchKind::Remote
        } else {
            continue;
        };
        if kind == GitBranchKind::Remote && ref_name.ends_with("/HEAD") {
            continue;
        }
        branches.push(GitBranch {
            name: name.to_string(),
            kind,
            current: head == "*",
            upstream: (!upstream.is_empty()).then(|| upstream.to_string()),
        });
    }
    Ok(branches)
}

fn validate_branch_name(repository: &GitRepository, name: &str) -> Result<()> {
    let name = name.trim();
    if name.is_empty() {
        return Err(anyhow!("Branch name cannot be empty"));
    }
    let output = run_git_vec(
        &repository.root,
        vec![
            "check-ref-format".to_string(),
            "--branch".to_string(),
            name.to_string(),
        ],
    )?;
    if output.status.success() {
        Ok(())
    } else {
        Err(git_command_error("Invalid branch name", &output))
    }
}

fn default_remote(repository: &GitRepository) -> Result<String> {
    let output = run_git(&repository.root, ["remote"])?;
    if !output.status.success() {
        return Err(git_command_error("git remote", &output));
    }
    let remotes = String::from_utf8_lossy(&output.stdout)
        .lines()
        .map(str::trim)
        .filter(|remote| !remote.is_empty())
        .map(str::to_string)
        .collect::<Vec<_>>();
    remotes
        .iter()
        .find(|remote| remote.as_str() == "origin")
        .cloned()
        .or_else(|| remotes.first().cloned())
        .ok_or_else(|| anyhow!("No Git remote is configured"))
}

fn run_git_operation(repository: &GitRepository, label: &str, args: Vec<String>) -> Result<()> {
    let output = run_git_vec(&repository.root, args)?;
    if output.status.success() {
        Ok(())
    } else {
        Err(git_command_error(label, &output))
    }
}

fn repository_has_head(repository: &GitRepository) -> Result<bool> {
    let output = run_git(
        &repository.root,
        ["rev-parse", "--verify", "--quiet", "HEAD"],
    )?;
    Ok(output.status.success())
}

fn path_exists_in_head(repository: &GitRepository, path: &Path) -> Result<bool> {
    let output = run_git_vec(
        &repository.root,
        vec![
            "ls-tree".to_string(),
            "-r".to_string(),
            "--name-only".to_string(),
            "-z".to_string(),
            "HEAD".to_string(),
            "--".to_string(),
            path.to_string_lossy().into_owned(),
        ],
    )?;
    if !output.status.success() {
        return Err(git_command_error("git ls-tree", &output));
    }
    Ok(output
        .stdout
        .split(|byte| *byte == 0)
        .any(|entry| entry == path.as_os_str().as_encoded_bytes()))
}

fn change_paths(change: &GitChange) -> impl Iterator<Item = &Path> {
    std::iter::once(change.path.as_path()).chain(change.original_path.as_deref())
}

fn append_change_path_args(args: &mut Vec<String>, change: &GitChange) {
    args.push(change.path.to_string_lossy().into_owned());
    if let Some(original_path) = change.original_path.as_ref() {
        args.push(original_path.to_string_lossy().into_owned());
    }
}

fn remove_worktree_path_if_present(path: &Path) -> Result<()> {
    if fs::symlink_metadata(path).is_err() {
        return Ok(());
    }
    remove_worktree_path(path)
}

fn remove_worktree_path(path: &Path) -> Result<()> {
    let metadata = fs::symlink_metadata(path)
        .with_context(|| format!("Unable to inspect {}", path.display()))?;
    if metadata.is_dir() && !metadata.file_type().is_symlink() {
        fs::remove_dir_all(path)
            .with_context(|| format!("Unable to remove directory {}", path.display()))
    } else {
        fs::remove_file(path).with_context(|| format!("Unable to remove {}", path.display()))
    }
}

fn remove_empty_parent_directories(root: &Path, mut parent: Option<&Path>) -> Result<()> {
    while let Some(directory) = parent.filter(|directory| *directory != root) {
        match fs::remove_dir(directory) {
            Ok(()) => parent = directory.parent(),
            Err(error) if error.kind() == std::io::ErrorKind::DirectoryNotEmpty => break,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                parent = directory.parent();
            }
            Err(error) => {
                return Err(error)
                    .with_context(|| format!("Unable to remove {}", directory.display()));
            }
        }
    }
    Ok(())
}

fn git_diff_against_base(
    repository: &GitRepository,
    change: &GitChange,
    base: &str,
    full_context: bool,
) -> Result<Output> {
    let mut command = Command::new("git");
    configure_background_child(&mut command);
    command.current_dir(&repository.root).args([
        "diff",
        "--no-ext-diff",
        "--no-color",
        "--find-renames",
    ]);
    if full_context {
        command.arg(FULL_FILE_CONTEXT);
    }
    command.args([base, "--"]);
    append_change_paths(&mut command, change);
    command.output().context("Unable to run git diff")
}

fn git_diff(
    repository: &GitRepository,
    change: &GitChange,
    cached: bool,
    full_context: bool,
) -> Result<Output> {
    let mut command = Command::new("git");
    configure_background_child(&mut command);
    command.current_dir(&repository.root).args([
        "diff",
        "--no-ext-diff",
        "--no-color",
        "--find-renames",
    ]);
    if full_context {
        command.arg(FULL_FILE_CONTEXT);
    }
    if cached {
        command.arg("--cached");
    }
    command.arg("--");
    append_change_paths(&mut command, change);
    command.output().context("Unable to run git diff")
}

fn append_change_paths(command: &mut Command, change: &GitChange) {
    command.arg(&change.path);
    if let Some(original_path) = change.original_path.as_ref() {
        command.arg(original_path);
    }
}

fn untracked_file_diff(repository: &GitRepository, change: &GitChange) -> Result<String> {
    let full_path = repository.root.join(&change.path);
    let bytes =
        fs::read(&full_path).with_context(|| format!("Unable to read {}", full_path.display()))?;
    let text = String::from_utf8(bytes).context("Untracked file is not UTF-8 text")?;
    let line_count = text.lines().count();
    let path = change.path.to_string_lossy();
    let mut diff = format!(
        "diff --git a/{path} b/{path}\nnew file mode 100644\n--- /dev/null\n+++ b/{path}\n@@ -0,0 +1,{line_count} @@\n"
    );
    for line in text.split_inclusive('\n') {
        diff.push('+');
        diff.push_str(line);
    }
    if !text.is_empty() && !text.ends_with('\n') {
        diff.push_str("\n\\ No newline at end of file\n");
    }
    Ok(diff)
}

fn run_git<const N: usize>(repo: &Path, args: [&str; N]) -> Result<Output> {
    let mut command = Command::new("git");
    configure_background_child(&mut command);
    command
        .current_dir(repo)
        .args(args)
        .output()
        .context("Unable to run git")
}

fn run_git_vec(repo: &Path, args: Vec<String>) -> Result<Output> {
    let mut command = Command::new("git");
    configure_background_child(&mut command);
    command
        .current_dir(repo)
        .args(args)
        .output()
        .context("Unable to run git")
}

fn git_command_error(label: &str, output: &Output) -> anyhow::Error {
    let message = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stderr),
        String::from_utf8_lossy(&output.stdout)
    );
    anyhow!("{label} failed: {}", message.trim())
}

fn parse_porcelain_v1_z(bytes: &[u8]) -> Result<Vec<GitChange>> {
    let fields = bytes.split(|byte| *byte == 0).collect::<Vec<_>>();
    let mut changes = Vec::new();
    let mut index = 0;
    while index < fields.len() {
        let field = fields[index];
        index += 1;
        if field.is_empty() {
            continue;
        }
        if field.len() < 4 || field[2] != b' ' {
            return Err(anyhow!("Invalid git status entry"));
        }
        let index_status = field[0] as char;
        let worktree_status = field[1] as char;
        let path = PathBuf::from(String::from_utf8_lossy(&field[3..]).into_owned());
        let renamed = matches!(index_status, 'R' | 'C') || matches!(worktree_status, 'R' | 'C');
        let original_path = if renamed {
            let Some(original) = fields.get(index).filter(|value| !value.is_empty()) else {
                return Err(anyhow!("Missing original path for renamed git entry"));
            };
            index += 1;
            Some(PathBuf::from(
                String::from_utf8_lossy(original).into_owned(),
            ))
        } else {
            None
        };
        changes.push(GitChange {
            path,
            original_path,
            kind: change_kind(index_status, worktree_status),
            staged: index_status != ' ' && index_status != '?',
        });
    }
    changes.sort_by(|left, right| left.path.cmp(&right.path));
    Ok(changes)
}

fn change_kind(index: char, worktree: char) -> GitChangeKind {
    if index == '?' && worktree == '?' {
        GitChangeKind::Untracked
    } else if matches!(index, 'U')
        || matches!(worktree, 'U')
        || matches!((index, worktree), ('A', 'A') | ('D', 'D'))
    {
        GitChangeKind::Conflicted
    } else if matches!(index, 'R' | 'C') || matches!(worktree, 'R' | 'C') {
        GitChangeKind::Renamed
    } else if index == 'D' || worktree == 'D' {
        GitChangeKind::Deleted
    } else if index == 'A' || worktree == 'A' {
        GitChangeKind::Added
    } else {
        GitChangeKind::Modified
    }
}

#[cfg(test)]
mod tests;
