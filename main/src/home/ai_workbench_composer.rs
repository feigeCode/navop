//! 输入框下方上下文栏的宿主接线：工作区 / 分支 / Worktree。
//!
//! 视图侧（`ai_chat_view::ComposerContextSource`）只做展示与转发，真实语义都在这里：
//! 工作区走 Explorer 的根切换，分支 / Worktree 走 `workspace_explorer::git`。
//!
//! 之所以要一层进程内缓存：快照闭包在每次 `AgentChatView::sync_composer` 都会被调用，
//! 而分支 / worktree 查询要起 `git` 子进程。缓存把「起进程」压到根目录变化与显式
//! 刷新两处，快照本身退化成纯内存读。

use std::path::{Path, PathBuf};
use std::sync::Arc;

use ai_chat_view::{
    ComposerBranchOption, ComposerContextSnapshot, ComposerContextSource, ComposerWorkspaceInfo,
    ComposerWorkspaceOption, ComposerWorktreeState, WorkbenchShell,
};
use gpui::{App, Entity, Global, WeakEntity, Window};
use workspace_explorer::{WorkspaceExplorer, git};

use super::ai_workbench::recent_workspace_roots;

/// 当前工作区的 Git 状态缓存；键是根目录，根变了就算失效。
struct ComposerGitCache {
    /// 取根目录与仓库句柄的来源。弱引用：外壳与面板不因它被吊住。
    explorer: WeakEntity<WorkspaceExplorer>,
    root: Option<PathBuf>,
    /// 上一次读到仓库时它的根目录；`None` 表示那次没看到仓库。
    ///
    /// 只用来判断「是否刚认出仓库」这一次单向跳变（见快照闭包里的兜底自愈），
    /// 不作为常规失效条件 —— 两边不收敛时它会让每次渲染都起 git 子进程。
    repo_root: Option<PathBuf>,
    is_git_repo: bool,
    branches: Vec<ComposerBranchOption>,
    worktree: ComposerWorktreeState,
}

impl Global for ComposerGitCache {}

/// 重新读一次根目录的 Git 状态。
///
/// 调用点：装机时、根目录变化时、切分支 / 切 worktree 之后。
/// 这里的 git 查询是同步子进程调用，不要放进渲染路径。
pub(crate) fn refresh_composer_git(cx: &mut App) {
    let Some(cache) = cx.try_global::<ComposerGitCache>() else {
        return;
    };
    let explorer = cache.explorer.clone();
    let Some(explorer) = explorer.upgrade() else {
        return;
    };
    let (root, repository) = explorer.read_with(cx, |explorer, _| {
        (explorer.root().to_path_buf(), explorer.repository().cloned())
    });
    let (is_git_repo, branches, worktree) = match repository.as_ref() {
        Some(repository) => {
            let branches = git::load_branches(repository)
                .unwrap_or_default()
                .into_iter()
                .map(|branch| {
                    ComposerBranchOption::new(
                        branch.name,
                        branch.current,
                        matches!(branch.kind, git::GitBranchKind::Remote),
                    )
                })
                .collect();
            (
                true,
                branches,
                current_worktree_state(repository, &root),
            )
        }
        None => (false, Vec::new(), ComposerWorktreeState::default()),
    };
    cx.set_global(ComposerGitCache {
        explorer: explorer.downgrade(),
        root: Some(root),
        repo_root: repository.map(|repository| repository.root),
        is_git_repo,
        branches,
        worktree,
    });
}

/// 当前根目录是不是一个（非主）worktree。
///
/// 判据来自 git 自己的 `worktree list`，不靠路径前缀猜：主工作区与不受管的
/// 检出都不会被当成「本会话的 worktree」。
fn current_worktree_state(
    repository: &git::GitRepository,
    root: &Path,
) -> ComposerWorktreeState {
    let Ok(entries) = git::list_worktrees(repository) else {
        return ComposerWorktreeState::default();
    };
    // macOS 的 `/var` → `/private/var` 等符号链接会让字符串比较失配。
    let canonical = std::fs::canonicalize(root).unwrap_or_else(|_| root.to_path_buf());
    match entries.into_iter().find(|entry| entry.path == canonical) {
        Some(entry) if !entry.is_main => ComposerWorktreeState::new(
            true,
            entry
                .path
                .file_name()
                .map(|name| name.to_string_lossy().into_owned()),
        ),
        _ => ComposerWorktreeState::default(),
    }
}

/// 组装上下文栏数据源。
///
/// 四个动作闭包都自持外壳弱引用：外壳构造早于本函数，且切换工作区必须走外壳的
/// 既有语义（「会话已有消息就开新对话」），不能在这里重写一遍。
pub(crate) fn composer_context_source(
    shell: WeakEntity<WorkbenchShell>,
    explorer: Entity<WorkspaceExplorer>,
    cx: &mut App,
) -> ComposerContextSource {
    cx.set_global(ComposerGitCache {
        explorer: explorer.downgrade(),
        root: None,
        repo_root: None,
        is_git_repo: false,
        branches: Vec::new(),
        worktree: ComposerWorktreeState::default(),
    });
    refresh_composer_git(cx);

    let snapshot_shell = shell.clone();
    let snapshot = Arc::new(move |cx: &mut App| {
        let root = snapshot_shell.upgrade().and_then(|shell| {
            shell
                .read(cx)
                .workspace_root()
                .map(|root| root.to_path_buf())
        });
        // 兜底自愈，只认「根目录变了」与「之前没认出仓库、现在认出了」两种跳变；
        // 后者为什么必须是单向的，见 `composer_cache_is_stale` 的注释。
        let (cached_root, cached_repository_root) = cx
            .try_global::<ComposerGitCache>()
            .map(|cache| (cache.root.clone(), cache.repo_root.clone()))
            .unwrap_or_default();
        let explorer_has_repository = cx
            .try_global::<ComposerGitCache>()
            .and_then(|cache| cache.explorer.upgrade())
            .is_some_and(|explorer| {
                explorer.read_with(cx, |explorer, _| explorer.repository().is_some())
            });
        let stale = composer_cache_is_stale(
            cached_root.as_deref(),
            root.as_deref(),
            cached_repository_root.as_deref(),
            explorer_has_repository,
        );
        if stale {
            refresh_composer_git(cx);
        }
        let (is_git_repo, branches, worktree) = cx
            .try_global::<ComposerGitCache>()
            .map(|cache| {
                (
                    cache.is_git_repo,
                    cache.branches.clone(),
                    cache.worktree.clone(),
                )
            })
            .unwrap_or_default();

        let workspace = match root.as_ref() {
            Some(root) => {
                ComposerWorkspaceInfo::new(workspace_label(root), Some(path_text(root)))
                    .with_git_repo(is_git_repo)
            }
            None => ComposerWorkspaceInfo::default(),
        };
        let mut roots = recent_workspace_roots(cx);
        if let Some(root) = root.as_ref()
            && !roots.contains(root)
        {
            roots.insert(0, root.clone());
        }
        let workspace_options = roots
            .into_iter()
            .map(|path| {
                ComposerWorkspaceOption::new(
                    workspace_label(&path),
                    path_text(&path),
                    root.as_ref() == Some(&path),
                )
            })
            .collect();

        ComposerContextSnapshot {
            workspace,
            workspace_options,
            branches,
            worktree,
        }
    });

    let switch_shell = shell.clone();
    let select_workspace = Arc::new(move |root: &Path, _window: &mut Window, cx: &mut App| {
        let Some(shell) = switch_shell.upgrade() else {
            return;
        };
        let root = root.to_path_buf();
        shell.update(cx, |shell, cx| {
            shell.open_workspace_from_composer(&root, cx)
        });
    });

    let browse_explorer = explorer.clone();
    let browse_workspace = Arc::new(move |window: &mut Window, cx: &mut App| {
        browse_explorer.update(cx, |explorer, cx| explorer.choose_root(window, cx));
    });

    let branch_explorer = explorer.clone();
    let branch_shell = shell.clone();
    let select_branch = Arc::new(move |name: &gpui::SharedString, cx: &mut App| {
        let Some(repository) =
            branch_explorer.read_with(cx, |explorer, _| explorer.repository().cloned())
        else {
            return;
        };
        let Ok(branches) = git::load_branches(&repository) else {
            return;
        };
        let Some(branch) = branches
            .into_iter()
            .find(|branch| branch.name == name.as_ref())
        else {
            return;
        };
        if let Err(error) = git::switch_branch(&repository, &branch) {
            tracing::warn!(%error, branch = %name, "切换分支失败（来自输入框底栏）");
        }
        // Explorer 自己缓存的当前分支 / 变更列表也要跟着走，否则侧栏显示的还是旧分支。
        branch_explorer.update(cx, |explorer, cx| explorer.refresh_git_changes(cx));
        refresh_composer_git(cx);
        if let Some(shell) = branch_shell.upgrade() {
            shell.update(cx, |shell, cx| shell.refresh_composer_context(cx));
        }
    });

    let worktree_explorer = explorer.clone();
    let worktree_shell = shell.clone();
    let toggle_worktree = Arc::new(move |enabled: bool, cx: &mut App| {
        let Some(repository) =
            worktree_explorer.read_with(cx, |explorer, _| explorer.repository().cloned())
        else {
            return;
        };
        let target = if enabled {
            // 新建的 worktree 落在 `~/.navop/worktrees/<项目>-<随机>`，
            // 分支带 `navop/` 前缀（由 workspace_explorer 统一管理）。
            match git::create_worktree(&repository, &repository.root, None) {
                Ok(created) => created.worktree_root,
                Err(error) => {
                    tracing::warn!(%error, "创建 worktree 失败（来自输入框底栏）");
                    return;
                }
            }
        } else {
            // 关掉只切回主工作区，**不删除** worktree：里面可能有没提交的改动，
            // 删除不可逆，不能由一个复选框的单击决定。
            match git::list_worktrees(&repository) {
                Ok(entries) => match entries.into_iter().find(|entry| entry.is_main) {
                    Some(entry) => entry.path,
                    None => return,
                },
                Err(error) => {
                    tracing::warn!(%error, "读取 worktree 列表失败（来自输入框底栏）");
                    return;
                }
            }
        };
        worktree_explorer.update(cx, |explorer, cx| explorer.set_root_manually(target, cx));
        refresh_composer_git(cx);
        if let Some(shell) = worktree_shell.upgrade() {
            shell.update(cx, |shell, cx| shell.refresh_composer_context(cx));
        }
    });

    ComposerContextSource {
        snapshot,
        select_workspace,
        browse_workspace,
        select_branch,
        toggle_worktree,
    }
}

/// 工作区显示名：目录名；拿不到目录名时退回整条路径（根目录这种极端情形）。
fn workspace_label(path: &Path) -> String {
    path.file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| path_text(path))
}

fn path_text(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

/// 快照缓存该不该重读。
///
/// 两个条件之一成立就重读：
/// - **根目录变了**：缓存记的是别的根，内容整体作废。
/// - **刚认出仓库**：Explorer 是异步发现仓库的，装机那一帧 `repository()` 还是
///   `None`；只比根目录的话，那份「不是 Git 仓库」的结论会连同正确的根一起被
///   永久缓存住，底栏的分支 / Worktree 入口再也不出现（这是实际踩过的 bug，
///   截图取证过）。常规路径由 `RepositoryChanged` 事件覆盖，这里是兜底。
///
/// 「刚认出仓库」必须是**单向**的：一旦缓存里记下了仓库根，条件就不再成立。
/// 写成 `cached_repository_root != 当前仓库根` 是错的 —— 两边一旦不收敛就会每次
/// 渲染都去起 `git` 子进程（`load_branches` / `list_worktrees`）。
fn composer_cache_is_stale(
    cached_root: Option<&Path>,
    requested_root: Option<&Path>,
    cached_repository_root: Option<&Path>,
    explorer_has_repository: bool,
) -> bool {
    cached_root != requested_root
        || (cached_repository_root.is_none() && explorer_has_repository)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_matching_root_without_a_repository_is_not_re_read() {
        let root = Path::new("/work/navop");
        // 真正的非 Git 目录：没仓库、根也没变 —— 不该反复重读。
        assert!(!composer_cache_is_stale(
            Some(root),
            Some(root),
            None,
            false
        ));
    }

    #[test]
    fn a_newly_discovered_repository_forces_a_re_read() {
        let root = Path::new("/work/navop");
        // 装机那一帧缓存下的是「不是 Git 仓库」；仓库一出现就必须重读，
        // 否则分支 / Worktree 入口永远不出现。
        assert!(composer_cache_is_stale(Some(root), Some(root), None, true));
    }

    #[test]
    fn a_repository_already_cached_does_not_loop() {
        let root = Path::new("/work/navop");
        let repository = Path::new("/work/navop");
        // 单向：已经记下仓库之后，条件不再成立 —— 不会每次渲染都起 git 子进程。
        assert!(!composer_cache_is_stale(
            Some(root),
            Some(root),
            Some(repository),
            true
        ));
    }

    #[test]
    fn a_changed_or_cleared_root_forces_a_re_read() {
        let previous = Path::new("/work/navop");
        let next = Path::new("/work/other");
        assert!(composer_cache_is_stale(
            Some(previous),
            Some(next),
            Some(previous),
            true
        ));
        // 首次装机 / 根被清空
        assert!(composer_cache_is_stale(None, Some(next), None, false));
        assert!(composer_cache_is_stale(Some(previous), None, None, false));
    }
}
