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

fn workspace_root(cx: &App) -> std::path::PathBuf {
    let settings = one_core::settings::AppSettings::current(cx);
    settings
        .ai_chat
        .last_workspace_root
        .filter(|path| path.is_dir())
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_default())
}

fn default_terminal_config() -> LocalConfig {
    LocalConfig {
        shell: None,
        args: Vec::new(),
        working_dir: std::env::current_dir()
            .ok()
            .map(|path| path.to_string_lossy().into_owned()),
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
    let chat_for_root = chat.clone();
    let root_subscription: Subscription = cx.subscribe(
        &explorer,
        move |_, event: &WorkspaceExplorerEvent, cx| {
            if let WorkspaceExplorerEvent::RootChanged(root) = event {
                chat_for_root.update(cx, |panel, cx| {
                    panel.set_workspace_root(root.clone(), cx);
                });
            }
        },
    );
    chat.update(cx, |panel, cx| panel.set_sidebar_suppressed(true, cx));
    let terminal_root = workspace_root.clone();
    let terminal = cx.new(|cx| {
        let mut config = default_terminal_config();
        config.working_dir = Some(terminal_root.to_string_lossy().into_owned());
        TerminalView::new(config, window, cx).with_workspace_pane()
    });

    cx.new(|cx| {
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
                subscriptions: vec![root_subscription],
            },
            window,
            cx,
        )
    })
}
