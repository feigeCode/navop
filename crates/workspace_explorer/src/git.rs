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

pub(crate) fn discover_repository(path: &Path) -> Result<Option<GitRepository>> {
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

pub(crate) fn load_changes(repository: &GitRepository) -> Result<Vec<GitChange>> {
    let output = run_git(
        &repository.root,
        ["status", "--porcelain=v1", "-z", "--untracked-files=all"],
    )?;
    if !output.status.success() {
        return Err(git_command_error("git status", &output));
    }
    parse_porcelain_v1_z(&output.stdout)
}

pub(crate) fn load_diff(repository: &GitRepository, change: &GitChange) -> Result<String> {
    if change.kind == GitChangeKind::Untracked {
        return untracked_file_diff(repository, change);
    }

    let output = git_diff_against_head(repository, change)?;
    if output.status.success() {
        let diff = String::from_utf8_lossy(&output.stdout).to_string();
        if !diff.is_empty() {
            return Ok(diff);
        }
    }

    let mut combined = String::new();
    for cached in [true, false] {
        let fallback = git_diff(repository, change, cached)?;
        if fallback.status.success() {
            combined.push_str(&String::from_utf8_lossy(&fallback.stdout));
        }
    }
    if combined.is_empty() {
        Err(git_command_error("git diff", &output))
    } else {
        Ok(combined)
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

fn git_diff_against_head(repository: &GitRepository, change: &GitChange) -> Result<Output> {
    let mut command = Command::new("git");
    configure_background_child(&mut command);
    command.current_dir(&repository.root).args([
        "diff",
        "--no-ext-diff",
        "--no-color",
        "--find-renames",
        "HEAD",
        "--",
    ]);
    append_change_paths(&mut command, change);
    command.output().context("Unable to run git diff")
}

fn git_diff(repository: &GitRepository, change: &GitChange, cached: bool) -> Result<Output> {
    let mut command = Command::new("git");
    configure_background_child(&mut command);
    command.current_dir(&repository.root).args([
        "diff",
        "--no-ext-diff",
        "--no-color",
        "--find-renames",
    ]);
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
