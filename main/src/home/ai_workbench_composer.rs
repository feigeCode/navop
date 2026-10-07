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
use gpui::{
    App, AppContext as _, AsyncApp, Entity, Global, SharedString, Task, WeakEntity, Window,
};
use gpui_component::{WindowExt as _, notification::Notification};
use rust_i18n::t;
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
    /// 「本会话要用独立 worktree」的用户意图：勾了但还没建。
    ///
    /// 勾选不再立刻建 worktree —— 那会在磁盘上留目录、还把工作区根切走。
    /// 意图留在这里，等第一次发送时由 [`prepare_worktree`] 真正创建。
    worktree_intent: bool,
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
    let worktree_intent = cache.worktree_intent;
    let previous_root = cache.root.clone();
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
    // 意图的两条出路：
    // - 已经跑在 worktree 上 → 达成，落成常态的 `worktree.enabled`；
    // - 换了工作区（首次填充之后）→ 作废，意图属于某一个仓库，用户切走就是想
    //   在新地方干活，不该还挂着旧仓库的 pending。
    let root_changed = previous_root
        .as_deref()
        .is_some_and(|previous| previous != root.as_path());
    let worktree_intent = worktree_intent && !worktree.enabled && !root_changed;
    cx.set_global(ComposerGitCache {
        explorer: explorer.downgrade(),
        root: Some(root),
        repo_root: repository.map(|repository| repository.root),
        is_git_repo,
        branches,
        worktree,
        worktree_intent,
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
///
/// `worktree_root` 是新建 worktree 的落点：`None` 用 `~/.navop/worktrees`
/// （生产路径），测试注入临时目录以免污染用户 home。
pub(crate) fn composer_context_source(
    shell: WeakEntity<WorkbenchShell>,
    explorer: Entity<WorkspaceExplorer>,
    worktree_root: Option<PathBuf>,
    cx: &mut App,
) -> ComposerContextSource {
    cx.set_global(ComposerGitCache {
        explorer: explorer.downgrade(),
        root: None,
        repo_root: None,
        is_git_repo: false,
        branches: Vec::new(),
        worktree: ComposerWorktreeState::default(),
        worktree_intent: false,
    });
    refresh_composer_git(cx);

    let snapshot = Arc::new(move |cx: &mut App| {
        // 当前工作区根取 Explorer 的，**不能**读外壳的同名字段。
        //
        // 1. 外壳侧 `refresh_composer_context` 的调用点全是
        //    `shell.update(cx, |shell, cx| shell.refresh_composer_context(cx))`
        //    —— 刷新期间外壳已被租借。快照回头 `shell.read(cx)` 就是 GPUI 的双重
        //    租借：`cannot read WorkbenchShell while it is already being updated`，
        //    而 macOS 的事件回调是 `extern "C"`，panic 无法 unwind，会升级成
        //    `fatal runtime error` 直接 abort 整个进程（点一下底栏的分支 /
        //    Worktree 就崩一次）。
        // 2. Explorer 的根才是真源：`set_root_manually` → `apply_root_change`
        //    先改自己的字段再 emit `RootChanged`，而 emit 是延后派发的，外壳那份
        //    镜像要等派发落地才更新。切 worktree 后紧接着刷新底栏，读外壳会拿到
        //    旧根，worktree 开关会闪回「关」。
        let explorer = cx
            .try_global::<ComposerGitCache>()
            .and_then(|cache| cache.explorer.upgrade());
        let root = explorer
            .as_ref()
            .map(|explorer| explorer.read_with(cx, |explorer, _| explorer.root().to_path_buf()));
        // 兜底自愈，只认「根目录变了」与「之前没认出仓库、现在认出了」两种跳变；
        // 后者为什么必须是单向的，见 `composer_cache_is_stale` 的注释。
        let (cached_root, cached_repository_root) = cx
            .try_global::<ComposerGitCache>()
            .map(|cache| (cache.root.clone(), cache.repo_root.clone()))
            .unwrap_or_default();
        let explorer_has_repository = explorer.as_ref().is_some_and(|explorer| {
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
                // 勾了但还没建：chip 要显示待创建（勾选态 + 「首次对话时创建」的
                // 提示），而不是假装已经有一个 worktree 在跑。
                let worktree = if cache.worktree_intent && !cache.worktree.enabled {
                    ComposerWorktreeState::pending()
                } else {
                    cache.worktree.clone()
                };
                (cache.is_git_repo, cache.branches.clone(), worktree)
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
        if enabled {
            // 只记意图，**不建** worktree：勾一下就在磁盘上留目录、还把工作区根
            // 切走（连带丢弃 ACP 连接、重载项目 skills），代价太大。
            // 真正创建在第一次发送时，见 `prepare_worktree`。
            if set_worktree_intent(cx, true) {
                refresh_composer_context(&worktree_shell, cx);
            }
            return;
        }
        if take_worktree_intent(cx) {
            // 还没建就取消：纯撤销，磁盘上没有任何东西要收拾。
            refresh_composer_context(&worktree_shell, cx);
            return;
        }
        let Some(repository) =
            worktree_explorer.read_with(cx, |explorer, _| explorer.repository().cloned())
        else {
            return;
        };
        // 已经建出来了，关掉勾选只切回主工作区，**不删除** worktree：里面可能有
        // 没提交的改动，删除不可逆，不能由一个复选框的单击决定。
        let target = match git::list_worktrees(&repository) {
            Ok(entries) => match entries.into_iter().find(|entry| entry.is_main) {
                Some(entry) => entry.path,
                None => return,
            },
            Err(error) => {
                tracing::warn!(%error, "读取 worktree 列表失败（来自输入框底栏）");
                return;
            }
        };
        worktree_explorer.update(cx, |explorer, cx| explorer.set_root_manually(target, cx));
        refresh_composer_git(cx);
        refresh_composer_context(&worktree_shell, cx);
    });

    let prepare_explorer = explorer.clone();
    let prepare_worktree_root = worktree_root;
    let prepare_worktree = Arc::new(move |cx: &mut App| -> Task<Result<(), SharedString>> {
        let explorer = prepare_explorer.clone();
        let Some(repository) = explorer.read_with(cx, |explorer, _| explorer.repository().cloned())
        else {
            return Task::ready(Err(SharedString::from("当前工作区不是 Git 仓库")));
        };
        // `git worktree add` 是子进程，大仓库要数秒：放后台跑，别冻结输入框。
        let worktree_root = prepare_worktree_root.clone();
        let create = cx.background_spawn(async move {
            let created = match worktree_root.as_deref() {
                Some(root) => git::create_worktree_in(root, &repository, &repository.root, None),
                None => git::create_worktree(&repository, &repository.root, None),
            };
            created
                .map(|created| created.worktree_root)
                .map_err(|error| error.to_string())
        });
        let explorer = explorer.downgrade();
        cx.spawn(async move |cx: &mut AsyncApp| {
            let root = create.await.map_err(SharedString::from)?;
            // 只切根：`apply_root_change` 会级联 `RootChanged`，宿主订阅再把根同步给
            // 外壳与聊天面板 —— 视图是在那一步之后才继续发那条被拦下的提交的。
            explorer
                .update(cx, |explorer, cx| explorer.set_root_manually(root, cx))
                .map_err(|_| SharedString::from("工作区已关闭"))?;
            Ok(())
        })
    });

    // 报错交给宿主：通知要用组件库的窗口状态，视图那边不该背这个依赖。
    let report_worktree_failure = Arc::new(
        |message: &SharedString, window: &mut Window, cx: &mut App| {
            window.push_notification(
                Notification::error(
                    t!("Home.worktree_create_failed", error = message.as_ref()).to_string(),
                )
                .autohide(false),
                cx,
            );
        },
    );

    ComposerContextSource {
        snapshot,
        select_workspace,
        browse_workspace,
        select_branch,
        toggle_worktree,
        prepare_worktree,
        report_worktree_failure,
    }
}

/// 改写「要用独立 worktree」的意图；返回是否发生了实际变化。
fn set_worktree_intent(cx: &mut App, intent: bool) -> bool {
    if cx
        .try_global::<ComposerGitCache>()
        .is_none_or(|cache| cache.worktree_intent == intent)
    {
        return false;
    }
    cx.global_mut::<ComposerGitCache>().worktree_intent = intent;
    true
}

/// 清掉「要用独立 worktree」的意图；返回此前是否有意图。
///
/// 与 [`set_worktree_intent`] 的区别是它回答「刚才挂着意图吗」—— 取消勾选时靠这个
/// 区分「纯撤销」和「已经建出来了，得切回主工作区」。
fn take_worktree_intent(cx: &mut App) -> bool {
    if cx
        .try_global::<ComposerGitCache>()
        .is_none_or(|cache| !cache.worktree_intent)
    {
        return false;
    }
    cx.global_mut::<ComposerGitCache>().worktree_intent = false;
    true
}

/// 让外壳重新取一次底栏快照（意图 / 根变化后）。
fn refresh_composer_context(shell: &WeakEntity<WorkbenchShell>, cx: &mut App) {
    if let Some(shell) = shell.upgrade() {
        shell.update(cx, |shell, cx| shell.refresh_composer_context(cx));
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
    use ai_chat_view::{
        WorkbenchPanelEntry, WorkbenchPanelKind, WorkbenchShellConfig, WorkbenchState,
    };
    use gpui::{TestAppContext, VisualTestContext};
    use std::sync::Mutex;
    use workspace_explorer::{WorkspaceEditor, WorkspaceExplorerConfig};

    use super::*;
    use crate::home::ai_workbench::workspace_theme;

    /// 快照必须能在「外壳已被租借」时安全取到 —— 这正是宿主动作末了的时序。
    ///
    /// 线上表现：点底栏的分支 / Worktree 就崩，日志是
    ///
    /// ```text
    /// cannot read ai_chat_view::workbench::shell::WorkbenchShell while it is
    /// already being updated      (gpui/src/app/entity_map.rs)
    /// ```
    ///
    /// 原因是 `select_branch` / `toggle_worktree` 的最后一行是
    /// `shell.update(cx, |shell, cx| shell.refresh_composer_context(cx))` ——
    /// 刷新期间外壳已被租借，而快照闭包回头 `shell.read(cx)` 取「当前工作区根」，
    /// 于是双重租借。macOS 的事件回调是 `extern "C"`，panic 无法 unwind，会升级成
    /// `fatal runtime error` 直接 abort 整个进程 —— 点一下崩一次。
    ///
    /// 这条测试按线上时序直接调真实的宿主快照闭包（`composer_context_source` 造出来的
    /// 那一个），并断言它拿到的是 Explorer 的根，而不是绕道外壳取镜像。
    #[gpui::test]
    fn composer_snapshot_is_safe_while_the_caller_holds_the_shell(cx: &mut TestAppContext) {
        cx.update(|cx| gpui_component::init(cx));

        let workspace = tempfile::tempdir().expect("temp workspace");
        let root = workspace.path().to_path_buf();
        let explorer = cx.update(|cx| {
            let theme = workspace_theme(cx);
            let editor = cx.new(|_| WorkspaceEditor::new(theme));
            let root = root.clone();
            cx.new(|cx| {
                WorkspaceExplorer::new(
                    WorkspaceExplorerConfig {
                        root,
                        editor,
                        theme,
                        show_frame_controls: false,
                        backend: None,
                    },
                    cx,
                )
            })
        });

        let explorer_for_shell = explorer.clone();
        let (shell, cx) = cx.add_window_view(move |window, cx| {
            WorkbenchShell::new(
                WorkbenchShellConfig {
                    panels: vec![WorkbenchPanelEntry::new(
                        WorkbenchPanelKind::Files,
                        explorer_for_shell,
                    )],
                    session_nav: None,
                    session_source: None,
                    initial_state: WorkbenchState::new(WorkbenchPanelKind::Files),
                    theme: None,
                    subscriptions: Vec::new(),
                    workspace_root: None,
                },
                window,
                cx,
            )
        });
        let cx: &mut VisualTestContext = cx;

        let source = {
            let shell_weak = shell.downgrade();
            cx.update(move |_window, cx| composer_context_source(shell_weak, explorer, None, cx))
        };
        let snapshot = source.snapshot.clone();

        // 宿主动作末了的刷新：外壳此刻正被租借。
        let captured = shell.update_in(cx, |_shell, _window, cx| snapshot(cx));
        // Explorer 会把根规范化（macOS 上 `/var` 会被解成 `/private/var`），
        // 断言前先过一遍同样的规范化。
        let expected = std::fs::canonicalize(&root).expect("canonical workspace root");
        assert_eq!(
            Some(expected.to_string_lossy().to_string()),
            captured
                .workspace
                .path
                .as_ref()
                .map(|path| path.to_string()),
            "快照必须取到 Explorer 的当前根，而不是绕道外壳"
        );
    }

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

    /// 建一个真 Git 仓库的临时工作区；返回（保活用的 TempDir，仓库根）。
    ///
    /// `git worktree add` 要求仓库至少有一个提交，否则报
    /// `fatal: invalid reference: HEAD`。
    ///
    /// 仓库落在具名子目录 `project/` 而不是 TempDir 自身：`tempfile` 的目录名以 `.`
    /// 开头（`.tmpJtcIrf`），而 worktree 分支名是用仓库目录名拼的 ——
    /// `navop/.tmpJtcIrf-bd946841` 会被 git 判为
    /// `fatal: ... is not a valid branch name`（路径分量不许以 `.` 开头）。
    fn git_workspace() -> (tempfile::TempDir, PathBuf) {
        let dir = tempfile::tempdir().expect("temp workspace");
        let root = dir.path().join("project");
        std::fs::create_dir(&root).expect("create project dir");
        let run_git = |args: &[&str]| {
            let status = std::process::Command::new("git")
                .args(args)
                .current_dir(&root)
                .status()
                .expect("run git");
            assert!(status.success(), "git {args:?} failed");
        };
        run_git(&["init", "-q"]);
        run_git(&["config", "user.email", "tests@navop.invalid"]);
        run_git(&["config", "user.name", "Navop Tests"]);
        std::fs::write(root.join("README.md"), "hello\n").expect("write README");
        run_git(&["add", "."]);
        run_git(&["commit", "-q", "-m", "init"]);
        (dir, root)
    }

    /// 仓库里注册的 worktree 数（含主工作区）。
    fn worktree_count(root: &Path) -> usize {
        let repository = git::discover_repository(root)
            .expect("discover repository")
            .expect("仓库存在");
        git::list_worktrees(&repository)
            .expect("list worktrees")
            .len()
    }

    /// 指向临时仓库的 Explorer + 外壳，以及接好线的上下文栏数据源。
    ///
    /// `worktree_root` 注入新建 worktree 的落点。生产路径是 `~/.navop/worktrees`，
    /// 测试必须换成临时目录：在那个路径建 worktree 不只会留垃圾目录，还会往这个
    /// 仓库的 `.git/worktrees` 里注册一条指向临时目录、随后就失效的条目。
    fn composer_fixture<'a>(
        root: &Path,
        worktree_root: Option<PathBuf>,
        cx: &'a mut TestAppContext,
    ) -> (
        Entity<WorkbenchShell>,
        ComposerContextSource,
        &'a mut VisualTestContext,
    ) {
        let root = root.to_path_buf();
        let explorer = cx.update(|cx| {
            let theme = workspace_theme(cx);
            let editor = cx.new(|_| WorkspaceEditor::new(theme));
            let root = root.clone();
            cx.new(|cx| {
                WorkspaceExplorer::new(
                    WorkspaceExplorerConfig {
                        root,
                        editor,
                        theme,
                        show_frame_controls: false,
                        backend: None,
                    },
                    cx,
                )
            })
        });

        let explorer_for_shell = explorer.clone();
        let (shell, cx) = cx.add_window_view(move |window, cx| {
            WorkbenchShell::new(
                WorkbenchShellConfig {
                    panels: vec![WorkbenchPanelEntry::new(
                        WorkbenchPanelKind::Files,
                        explorer_for_shell,
                    )],
                    session_nav: None,
                    session_source: None,
                    initial_state: WorkbenchState::new(WorkbenchPanelKind::Files),
                    theme: None,
                    subscriptions: Vec::new(),
                    workspace_root: None,
                },
                window,
                cx,
            )
        });
        let shell_weak = shell.downgrade();
        let source = cx.update(move |_window, cx| {
            composer_context_source(shell_weak, explorer, worktree_root, cx)
        });
        (shell, source, cx)
    }

    /// 勾选 Worktree 只记意图：磁盘上不许冒出 worktree，根也不许被切走。
    ///
    /// 原来的行为是一勾就 `create_worktree` + `set_root_manually` —— 勾一下就在
    /// `~/.navop/worktrees` 里留个目录，还把工作区根换掉（连带丢弃 ACP 连接、
    /// 重载项目 skills）。创建推迟到第一次发送，见
    /// [`the_first_submission_creates_the_worktree`]。
    #[gpui::test]
    fn toggling_worktree_on_only_records_the_intent(cx: &mut TestAppContext) {
        cx.update(|cx| gpui_component::init(cx));

        let (_workspace, root) = git_workspace();
        let (_shell, source, cx) = composer_fixture(&root, None, cx);
        // Explorer 异步发现仓库；底栏的分支 / Worktree 入口依赖它。
        cx.run_until_parked();

        assert!(
            cx.update(|_window, cx| (source.snapshot)(cx))
                .workspace
                .is_git_repo,
            "前置：临时仓库必须被认出来，否则 Worktree 入口根本不存在"
        );
        assert_eq!(1, worktree_count(&root), "前置：只有主工作区");

        let toggle = source.toggle_worktree.clone();
        cx.update(|_window, cx| toggle(true, cx));

        assert_eq!(1, worktree_count(&root), "勾选本身不该在磁盘上建东西");
        let snapshot = cx.update(|_window, cx| (source.snapshot)(cx));
        assert!(snapshot.worktree.enabled, "勾选态要亮着");
        assert!(
            snapshot.worktree.pending,
            "还没创建，chip 必须处于「待创建」"
        );
        assert!(
            snapshot.worktree.label.is_none(),
            "worktree 名字要等创建时才有"
        );

        let toggle = source.toggle_worktree.clone();
        cx.update(|_window, cx| toggle(false, cx));

        let snapshot = cx.update(|_window, cx| (source.snapshot)(cx));
        assert!(!snapshot.worktree.enabled, "取消勾选后不该还亮着");
        assert!(!snapshot.worktree.pending, "意图要一起清掉");
        assert_eq!(1, worktree_count(&root), "取消勾选同样不该动磁盘");
    }

    /// 第一次发送才真正建 worktree，并把工作区根切过去。
    #[gpui::test]
    fn the_first_submission_creates_the_worktree(cx: &mut TestAppContext) {
        cx.update(|cx| gpui_component::init(cx));

        let (_workspace, root) = git_workspace();
        let worktrees = tempfile::tempdir().expect("worktree root");
        let (_shell, source, cx) =
            composer_fixture(&root, Some(worktrees.path().to_path_buf()), cx);
        cx.run_until_parked();

        let toggle = source.toggle_worktree.clone();
        cx.update(|_window, cx| toggle(true, cx));
        assert_eq!(1, worktree_count(&root), "勾选本身不建");

        let prepare = source.prepare_worktree.clone();
        let task = cx.update(|_window, cx| prepare(cx));
        // 把 `Result` 捞回来：只看「任务跑完了」会把 `Err` 当成成功，掩盖真正的失败。
        let outcome = Arc::new(Mutex::new(None::<Result<(), SharedString>>));
        let outcome_for_wait = outcome.clone();
        cx.update(|_window, cx| {
            cx.spawn(async move |_cx: &mut AsyncApp| {
                let result = task.await;
                *outcome_for_wait.lock().expect("lock outcome") = Some(result);
            })
            .detach();
        });
        // 后台 git 子进程 + 主线程切根：轮询推到完成。
        for _ in 0..300 {
            if outcome.lock().expect("lock outcome").is_some() {
                break;
            }
            cx.run_until_parked();
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        let result = outcome
            .lock()
            .expect("lock outcome")
            .clone()
            .expect("准备 worktree 超时");
        assert!(result.is_ok(), "prepare_worktree 失败：{result:?}");

        assert_eq!(2, worktree_count(&root), "第一次发送才建出 worktree");
        let switched = cx.update(|_window, cx| {
            cx.try_global::<ComposerGitCache>()
                .and_then(|cache| cache.explorer.upgrade())
                .map(|explorer| explorer.read(cx).root().to_path_buf())
        });
        let switched = switched.expect("explorer is alive");
        assert_ne!(root, switched, "根必须切到新 worktree");
        let worktrees_root = std::fs::canonicalize(worktrees.path()).expect("canonical root");
        assert!(
            switched.starts_with(&worktrees_root),
            "worktree 要建在注入的根下，实际落到了 {switched:?}"
        );

        // 线上这一步由 `RootChanged` 的订阅者做（`main/src/home/ai_workbench.rs`）；
        // fixture 没注册订阅，手动补上。
        //
        // 但得先等 Explorer 在后台把仓库重新发现出来：`apply_root_change` 会把
        // `repository` 清成 `None`，再异步从新根加载。抢在那之前刷新，看到的还是
        // 空状态（`is_git_repo == false`），断言会随后台任务的快慢飘。
        for _ in 0..300 {
            let rediscovered = cx.update(|_window, cx| {
                cx.try_global::<ComposerGitCache>()
                    .and_then(|cache| cache.explorer.upgrade())
                    .and_then(|explorer| {
                        explorer
                            .read(cx)
                            .repository()
                            .map(|repository| repository.root.clone())
                    })
            });
            if rediscovered
                .as_deref()
                .is_some_and(|repo_root| repo_root != root)
            {
                break;
            }
            cx.run_until_parked();
            std::thread::sleep(std::time::Duration::from_millis(10));
        }

        cx.update(|_window, cx| refresh_composer_git(cx));
        let snapshot = cx.update(|_window, cx| (source.snapshot)(cx));
        assert!(
            snapshot.worktree.enabled,
            "创建完就该显示「跑在 worktree 上」"
        );
        assert!(!snapshot.worktree.pending, "意图已经达成，不该还标着待创建");
    }
}
