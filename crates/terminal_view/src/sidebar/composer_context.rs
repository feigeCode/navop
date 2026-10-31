//! 终端侧栏 AI 面板的底栏上下文：工作区 / 分支 / Worktree。
//!
//! 侧栏面板是在 `terminal_view` 里建的，拿不到 AI 工作台那份宿主数据源
//! （它长在 `navop_app` 的 `WorkbenchShell` 上），于是底栏只剩一颗空的
//! 「选择工作区」，分支与 Worktree 永远不渲染。为了和 AI 工作台看到同一套东西，
//! 这里直接按「当前工作区」自己算一份：
//!
//! - 工作区口径与工作台一致：设置里的上次工作区 → 应用当前目录 → 家目录；
//! - 仓库信息走 `workspace_explorer` 的 git 能力，**不进渲染路径**：
//!   只认「根目录变了」这一种失效，换根与动作之后显式刷新；
//! - 选中工作区 / 分支之后，把聊天面板的根一起切过去 —— 面板自己的
//!   `workspace_root` 才是 ACP 会话真正的 cwd，chip 与实际不能两张皮。

use std::path::{Path, PathBuf};
use std::sync::Arc;

use ai_chat_view::{
    ComposerBranchOption, ComposerContextSnapshot, ComposerContextSource, ComposerWorkspaceInfo,
    ComposerWorkspaceOption, ComposerWorktreeState, DefaultAgentChatPanel,
};
use gpui::{App, AppContext as _, Global, SharedString, Task, WeakEntity, Window};
use gpui_component::WindowExt as _;
use one_core::settings::AppSettings;
use rust_i18n::t;
use workspace_explorer::git;

/// 缓存上一次读到的仓库信息，避免每次渲染都起 git 子进程。
struct ComposerGitCache {
    root: Option<PathBuf>,
    is_git_repo: bool,
    branches: Vec<ComposerBranchOption>,
    worktree: ComposerWorktreeState,
    /// 勾了「用独立 worktree」但还没建：等第一次发送时再创建。
    worktree_intent: bool,
}

impl Global for ComposerGitCache {}

/// 组装侧栏 AI 面板的底栏数据源。
///
/// 动作闭包只持面板弱引用：面板是这些闭包的宿主，反向强引用会成环。
pub(crate) fn composer_context_source(
    panel: WeakEntity<DefaultAgentChatPanel>,
    cx: &mut App,
) -> ComposerContextSource {
    refresh_composer_git(cx);

    let snapshot = Arc::new(move |cx: &mut App| {
        let root = workspace_root(cx);
        // 兜底自愈：根目录被换掉（工作台里切的、或上一次会话留下的）时重读一次。
        let stale = cx
            .try_global::<ComposerGitCache>()
            .is_none_or(|cache| cache.root.as_deref() != Some(root.as_path()));
        if stale {
            refresh_composer_git(cx);
        }
        let (is_git_repo, branches, worktree) = cx
            .try_global::<ComposerGitCache>()
            .map(|cache| {
                let worktree = if cache.worktree_intent && !cache.worktree.enabled {
                    ComposerWorktreeState::pending()
                } else {
                    cache.worktree.clone()
                };
                (cache.is_git_repo, cache.branches.clone(), worktree)
            })
            .unwrap_or_default();

        let workspace = ComposerWorkspaceInfo::new(workspace_label(&root), Some(path_text(&root)))
            .with_git_repo(is_git_repo);
        let workspace_options = workspace_options(cx, &root);

        // 只报快照，**不改**面板的根：这里可能正处在渲染路径上，切根会顺带
        // 重载技能、作废 ACP 连接；根只在动作里改（见下方各闭包）。
        ComposerContextSnapshot {
            workspace,
            workspace_options,
            branches,
            worktree,
        }
    });

    let select_panel = panel.clone();
    let select_workspace = Arc::new(move |root: &Path, _window: &mut Window, cx: &mut App| {
        let root = root.to_path_buf();
        remember_workspace_root(&root, cx);
        refresh_composer_git(cx);
        if let Some(panel) = select_panel.upgrade() {
            panel.update(cx, |panel, cx| {
                panel.set_workspace_root(root.clone(), cx);
                panel.refresh_composer_context(cx);
            });
        }
    });

    let browse_panel = panel.clone();
    let browse_workspace = Arc::new(move |window: &mut Window, cx: &mut App| {
        let view = browse_panel.clone();
        let prompt = cx.prompt_for_paths(gpui::PathPromptOptions {
            files: false,
            multiple: false,
            directories: true,
            prompt: Some(t!("WorkspaceExplorer.frame.choose_root").to_string().into()),
        });
        window
            .spawn(cx, async move |cx| {
                let Ok(Ok(Some(paths))) = prompt.await else {
                    return;
                };
                let Some(path) = paths.into_iter().next() else {
                    return;
                };
                let Some(panel) = view.upgrade() else {
                    return;
                };
                let _ = cx.update(|_window, cx| {
                    // 根先落到设置里，再把面板的根与底栏一起刷新：顺序反了会读回旧根。
                    remember_workspace_root_in(&path, cx);
                    panel.update(cx, |panel, cx| {
                        panel.set_workspace_root(path.clone(), cx);
                        panel.refresh_composer_context(cx);
                    });
                    refresh_composer_git(cx);
                });
            })
            .detach();
    });

    let branch_panel = panel.clone();
    let select_branch = Arc::new(move |name: &SharedString, cx: &mut App| {
        let root = workspace_root(cx);
        let Some(repository) = discover(&root) else {
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
            tracing::warn!(%error, branch = %name, "切换分支失败（来自侧栏底栏）");
            return;
        }
        refresh_composer_git(cx);
        if let Some(panel) = branch_panel.upgrade() {
            panel.update(cx, |panel, cx| panel.refresh_composer_context(cx));
        }
    });

    let worktree_panel = panel.clone();
    let toggle_worktree = Arc::new(move |enabled: bool, cx: &mut App| {
        if enabled {
            // 只记意图，**不建** worktree：勾一下就在磁盘上留目录、还把工作区根切走
            // （连带丢弃 ACP 连接），代价太大。真正创建在第一次发送时。
            if set_worktree_intent(cx, true) {
                refresh_composer_context(&worktree_panel, cx);
            }
            return;
        }
        if take_worktree_intent(cx) {
            refresh_composer_context(&worktree_panel, cx);
            return;
        }
        // 已经建出来了：关掉勾选只切回主工作区，**不删除** worktree —— 里面可能有
        // 没提交的改动，删除不可逆，不该由一次单击决定。
        let root = workspace_root(cx);
        let Some(repository) = discover(&root) else {
            return;
        };
        let Ok(entries) = git::list_worktrees(&repository) else {
            return;
        };
        let Some(main) = entries.into_iter().find(|entry| entry.is_main) else {
            return;
        };
        remember_workspace_root(&main.path, cx);
        refresh_composer_git(cx);
        if let Some(panel) = worktree_panel.upgrade() {
            panel.update(cx, |panel, cx| {
                panel.set_workspace_root(main.path.clone(), cx);
                panel.refresh_composer_context(cx);
            });
        }
    });

    let prepare_panel = panel.clone();
    let prepare_worktree = Arc::new(move |cx: &mut App| -> Task<Result<(), SharedString>> {
        let root = workspace_root(cx);
        let Some(repository) = discover(&root) else {
            return Task::ready(Err(SharedString::from("当前工作区不是 Git 仓库")));
        };
        // `git worktree add` 是子进程，大仓库要数秒：放后台跑，别冻结输入框。
        let create = cx.background_spawn(async move {
            git::create_worktree(&repository, &repository.root, None)
                .map(|created| created.worktree_root)
                .map_err(|error| error.to_string())
        });
        let panel = prepare_panel.clone();
        cx.spawn(async move |cx: &mut gpui::AsyncApp| {
            let root = create.await.map_err(SharedString::from)?;
            cx.update(|cx| {
                remember_workspace_root_in(&root, cx);
                refresh_composer_git(cx);
                // 切根之后视图会接着把被拦下的那条提交发出去（见
                // `AgentChatView::apply_workspace_root` 里的 `resume_gated_submission`）。
                if let Some(panel) = panel.upgrade() {
                    panel.update(cx, |panel, cx| {
                        panel.set_workspace_root(root.clone(), cx);
                        panel.refresh_composer_context(cx);
                    });
                }
            });
            Ok(())
        })
    });

    let report_worktree_failure = Arc::new(
        |message: &SharedString, window: &mut Window, cx: &mut App| {
            window.push_notification(
                gpui_component::notification::Notification::error(
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

/// 重新读一次当前工作区的 Git 状态。
///
/// 同步子进程调用，**不要**放进渲染路径：只在装机、换根、切分支/Worktree 后调用。
fn refresh_composer_git(cx: &mut App) {
    let root = workspace_root(cx);
    let worktree_intent = cx
        .try_global::<ComposerGitCache>()
        .map(|cache| cache.worktree_intent)
        .unwrap_or(false);

    let (is_git_repo, branches, worktree) = git_info(&root);

    // 意图的两条出路：已经跑在 worktree 上 → 达成，落成常态的 `enabled`；
    // 换了工作区 → 作废（意图属于某一个仓库）。
    let previous_root = cx
        .try_global::<ComposerGitCache>()
        .and_then(|cache| cache.root.clone());
    let root_changed = previous_root
        .as_deref()
        .is_some_and(|previous| previous != root.as_path());
    let worktree_intent = worktree_intent && !worktree.enabled && !root_changed;

    cx.set_global(ComposerGitCache {
        root: Some(root),
        is_git_repo,
        branches,
        worktree,
        worktree_intent,
    });
}

/// 读一个根目录的 Git 信息：是不是仓库、有哪些分支、是不是跑在 worktree 上。
///
/// 单独拆出来是为了能只喂一个路径就测（见文件末尾的测试），不必先造一个 App。
fn git_info(root: &Path) -> (bool, Vec<ComposerBranchOption>, ComposerWorktreeState) {
    let Some(repository) = discover(root) else {
        return (false, Vec::new(), ComposerWorktreeState::default());
    };
    let branches = git::load_branches(&repository)
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
    (true, branches, current_worktree_state(&repository, root))
}

/// 当前根目录是不是一个（非主）worktree。
///
/// 判据来自 git 自己的 `worktree list`，不靠路径前缀猜：主工作区与不受管的检出
/// 都不会被当成「本会话的 worktree」。
fn current_worktree_state(repository: &git::GitRepository, root: &Path) -> ComposerWorktreeState {
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

fn discover(root: &Path) -> Option<git::GitRepository> {
    match git::discover_repository(root) {
        Ok(repository) => repository,
        Err(error) => {
            tracing::warn!(%error, "读取 Git 仓库失败（来自侧栏底栏）");
            None
        }
    }
}

/// 侧栏装机时要用的初始工作区（与底栏快照同一口径）。
pub(crate) fn current_workspace_root(cx: &App) -> PathBuf {
    workspace_root(cx)
}

/// 当前工作区：设置里的上次工作区 → 应用当前目录 → 家目录。
///
/// 与 AI 工作台口径一致，否则同一个应用里两处「工作区」会各说各话。
fn workspace_root(cx: &App) -> PathBuf {
    let saved = AppSettings::current(cx).ai_chat.last_workspace_root;
    saved
        .filter(|path| is_workspace_dir(path))
        .or_else(|| {
            std::env::current_dir()
                .ok()
                .filter(|path| is_workspace_dir(path))
        })
        .or_else(|| std::env::var_os("HOME").map(PathBuf::from))
        .unwrap_or_else(|| PathBuf::from("."))
}

/// 目录存在、且不是文件系统根：从 Finder 启动时 cwd 是 `/`，放它进工作区没有意义。
fn is_workspace_dir(path: &Path) -> bool {
    path.is_dir() && path.parent().is_some()
}

fn workspace_options(cx: &App, current: &Path) -> Vec<ComposerWorkspaceOption> {
    let mut roots: Vec<PathBuf> = AppSettings::current(cx)
        .ai_chat
        .recent_workspace_roots
        .into_iter()
        .filter(|path| is_workspace_dir(path))
        .collect();
    if !roots.iter().any(|path| path == current) && is_workspace_dir(current) {
        roots.insert(0, current.to_path_buf());
    }
    roots
        .into_iter()
        .map(|path| {
            ComposerWorkspaceOption::new(workspace_label(&path), path_text(&path), path == current)
        })
        .collect()
}

fn workspace_label(path: &Path) -> String {
    path.file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| path_text(path))
}

fn path_text(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

fn remember_workspace_root(root: &Path, cx: &mut App) {
    remember_workspace_root_in(root, cx);
}

fn remember_workspace_root_in(root: &Path, cx: &mut App) {
    AppSettings::update_and_save(cx, |settings| {
        settings.ai_chat.remember_workspace_root(root);
    });
}

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

fn refresh_composer_context(panel: &WeakEntity<DefaultAgentChatPanel>, cx: &mut App) {
    if let Some(panel) = panel.upgrade() {
        panel.update(cx, |panel, cx| panel.refresh_composer_context(cx));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command;

    /// 在临时目录里建一个仓库：`git init` 之后 git 会自己给出初始分支名
    /// （老版本是 `master`、新版本看 `init.defaultBranch`），所以断言只认
    /// 「有一个 current 分支」，不写死名字。
    fn init_repo(dir: &Path) {
        let run = |args: &[&str]| {
            let status = Command::new("git")
                .args(args)
                .current_dir(dir)
                .status()
                .expect("run git");
            assert!(status.success(), "git {args:?} failed");
        };
        run(&["init", "-q"]);
        run(&["config", "user.email", "test@example.com"]);
        run(&["config", "user.name", "test"]);
        std::fs::write(dir.join("README.md"), "hello").expect("write file");
        run(&["add", "."]);
        run(&["commit", "-q", "-m", "init"]);
    }

    #[test]
    fn plain_directory_reports_no_git() {
        let dir = tempfile::tempdir().expect("temp dir");
        let (is_git_repo, branches, worktree) = git_info(dir.path());
        assert!(!is_git_repo);
        assert!(branches.is_empty());
        assert!(!worktree.enabled);
    }

    #[test]
    fn git_directory_reports_branches_and_main_checkout() {
        let dir = tempfile::tempdir().expect("temp dir");
        init_repo(dir.path());

        let (is_git_repo, branches, worktree) = git_info(dir.path());
        assert!(is_git_repo);
        assert!(
            branches.iter().any(|branch| branch.current),
            "至少要有一颗标成当前分支：{branches:?}"
        );
        // 主工作区不算「本会话的 worktree」——否则底栏会一上来就显示「独立 worktree」。
        assert!(!worktree.enabled);
    }

    #[test]
    fn workspace_without_parent_is_not_a_workspace() {
        assert!(!is_workspace_dir(Path::new("/")));
    }

    #[test]
    fn workspace_label_uses_the_directory_name() {
        assert_eq!("navop", workspace_label(Path::new("/tmp/navop")));
    }
}
