//! AI 工作台外壳的宿主组装。
//!
//! 外壳本身在 `ai_chat_view::workbench`，这里把四个面板接上：
//! 会话（内置 agent）、审阅（工作区编辑器）、文件（工作区浏览器）、终端。
//! 会话列表由外壳直接读会话面板，所以内建侧栏会被压制。

use ai_chat_view::{
    DefaultAgentChatPanel, MentionItem, WorkbenchPanelEntry, WorkbenchPanelKind, WorkbenchShell,
    WorkbenchShellConfig,
};
use gpui::{App, AppContext as _, Entity, Subscription, Window};
use gpui_component::ActiveTheme as _;
use terminal::LocalConfig;
use terminal_view::TerminalView;
use workspace_explorer::{
    WorkspaceEditor, WorkspaceExplorer, WorkspaceExplorerConfig, WorkspaceExplorerEvent,
    WorkspaceTheme,
};

/// 工作区主题：面板背景、边框与强调色取应用主题，语义色同样取应用主题。
fn workspace_theme(cx: &App) -> WorkspaceTheme {
    let theme = cx.theme();
    WorkspaceTheme {
        background: theme.background,
        foreground: theme.foreground,
        muted: theme.muted,
        muted_foreground: theme.muted_foreground,
        border: theme.border,
        accent: theme.accent,
        accent_foreground: theme.accent_foreground,
        danger: theme.danger,
        warning: theme.warning,
        success: theme.success,
    }
}

/// 工作台初始工作区。
///
/// 顺序：上次保存的工作区 → 进程当前目录 → 用户主目录。绝不回退到 `/`：
/// 那会得到一个不是工作区的"工作区"，用户也无从判断当前上下文。
fn workspace_root(cx: &App) -> std::path::PathBuf {
    let saved = one_core::settings::AppSettings::current(cx)
        .ai_chat
        .last_workspace_root;
    saved
        .filter(|path| path.is_dir())
        .or_else(|| std::env::current_dir().ok().filter(|path| path.is_dir()))
        .or_else(dirs::home_dir)
        .unwrap_or_else(|| std::path::PathBuf::from("."))
}

fn recent_workspace_roots(cx: &App) -> Vec<std::path::PathBuf> {
    one_core::settings::AppSettings::current(cx)
        .ai_chat
        .recent_workspace_roots
        .into_iter()
        .filter(|path| path.is_dir())
        .collect()
}

/// 把当前工作区记入最近列表；由 Explorer 的根目录变化驱动。
fn remember_workspace_root(root: &std::path::Path, cx: &mut App) {
    one_core::settings::AppSettings::update_and_save(cx, |settings| {
        settings.ai_chat.remember_workspace_root(root);
    });
}

fn default_terminal_config(root: &std::path::Path) -> LocalConfig {
    LocalConfig {
        shell: None,
        args: Vec::new(),
        working_dir: Some(root.to_string_lossy().into_owned()),
        env: Vec::new(),
    }
}

/// 构建 AI 工作台标签页内容。
pub(crate) fn build_ai_workbench_shell(
    scope: agent_runtime::AgentResourceScope,
    catalog: agent_runtime::ResourceCatalog,
    mentions: Vec<MentionItem>,
    window: &mut Window,
    cx: &mut App,
) -> Entity<WorkbenchShell> {
    let theme = workspace_theme(cx);
    let workspace_root = workspace_root(cx);
    let editor = cx.new(|_| WorkspaceEditor::new(theme));
    let explorer = cx.new(|cx| {
        WorkspaceExplorer::new(
            WorkspaceExplorerConfig {
                root: workspace_root.clone(),
                editor: editor.clone(),
                theme,
                show_frame_controls: false,
                backend: None,
            },
            cx,
        )
    });
    let chat = cx.new(|cx| {
        DefaultAgentChatPanel::new_workbench_with_scope_and_catalog(scope, catalog, mentions, window, cx)
            .with_tab_closeable(true)
            .with_workspace_root(workspace_root.clone())
    });
    let recents = recent_workspace_roots(cx);
    explorer.update(cx, |explorer, cx| explorer.set_recent_roots(recents, cx));
    // 初始工作区也要进入最近列表，否则第一次打开时菜单里是空的。
    remember_workspace_root(&workspace_root, cx);
    chat.update(cx, |panel, cx| panel.set_sidebar_suppressed(true, cx));
    let terminal_root = workspace_root.clone();
    let terminal = cx.new(|cx| {
        TerminalView::new(default_terminal_config(&terminal_root), window, cx).with_workspace_pane()
    });

    let shell = cx.new(|cx| {
        WorkbenchShell::new(
            WorkbenchShellConfig {
                panels: vec![
                    WorkbenchPanelEntry::new(WorkbenchPanelKind::Chat, chat.clone()),
                    WorkbenchPanelEntry::new(WorkbenchPanelKind::Review, editor.clone()),
                    WorkbenchPanelEntry::new(WorkbenchPanelKind::Files, explorer.clone()),
                    WorkbenchPanelEntry::new(WorkbenchPanelKind::Terminal, terminal.clone()),
                ],
                session_nav: None,
                session_source: Some(chat.clone()),
                initial_active: WorkbenchPanelKind::Chat,
                theme: None,
                subscriptions: Vec::new(),
                workspace_root: Some(workspace_root.clone()),
            },
            window,
            cx,
        )
    });

    // 外壳先于订阅存在：这里在构造之后再接线，根目录变化同时刷新外壳标题与聊天面板。
    let chat_for_root = chat.clone();
    let shell_for_root = shell.clone();
    let root_subscription: Subscription = cx.subscribe(
        &explorer,
        move |_, event: &WorkspaceExplorerEvent, cx| {
            if let WorkspaceExplorerEvent::RootChanged(root) = event {
                remember_workspace_root(root, cx);
                chat_for_root.update(cx, |panel, cx| {
                    panel.set_workspace_root(root.clone(), cx);
                });
                let root = root.clone();
                shell_for_root.update(cx, |shell, cx| shell.set_workspace_root(root, cx));
            }
        },
    );
    shell.update(cx, |shell, cx| shell.add_subscription(root_subscription, cx));
    shell
}
