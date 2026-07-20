//! 终端侧边栏模块
//!
//! 提供终端视图的侧边栏功能，包括：
//! - 设置面板（搜索、字体、主题）
//! - 快捷命令面板
//! - AI 聊天面板
//! - 文件管理器面板（仅 SSH 终端）

mod broadcast_input_panel;
pub mod file_manager_panel;
mod history_command_panel;
mod quick_command_panel;
mod server_monitor_panel;
mod settings_panel;
pub(crate) mod tool_dock;

use broadcast_input_panel::{BroadcastInputPanel, BroadcastInputPanelConfig};
pub use file_manager_panel::{FileManagerPanel, FileManagerPanelEvent};
pub use history_command_panel::{HistoryCommandPanel, HistoryCommandPanelEvent};
pub use quick_command_panel::QuickCommandPanel;
pub use server_monitor_panel::{ServerMonitorPanel, ServerMonitorPanelEvent};
pub use settings_panel::SettingsPanel;

use crate::{
    TerminalHighlightRule,
    broadcast_registry::{broadcast_input_registry, init_broadcast_input_registry},
    theme::{TerminalColors, TerminalTheme},
};
use ai_chat_view::{
    AgentChatTheme, CodeBlockAction, DefaultAgentChatPanel, DefaultAgentChatPanelEvent,
    LanguageMatcher, MentionItem, build_mentions_from_connections, build_resource_catalog,
    build_sidebar_resource_state,
};
use gpui::prelude::FluentBuilder;
use gpui::{
    AnyElement, AnyView, App, AppContext, Context, Entity, EventEmitter, FocusHandle, Focusable,
    InteractiveElement, IntoElement, ParentElement, Pixels, Render, SharedString,
    StatefulInteractiveElement, Styled, Subscription, Window, div, px,
};
use gpui_component::{
    Icon, IconName, Sizable, Size,
    button::{Button, ButtonVariants},
    h_flex, v_flex,
};
use one_core::layout::TOOLBAR_WIDTH;
use one_core::sidebar_contribution::SidebarPlacement;
use one_core::storage::{
    ConnectionRepository, GlobalStorageState, TerminalHistoryScope, models::StoredConnection,
    traits::Repository,
};
use rust_i18n::t;
use ssh::SshSessionManager;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use terminal::terminal::{SshTerminalConfig, TerminalConnectionKind};

fn terminal_ai_system_instruction(connection_kind: TerminalConnectionKind) -> String {
    let (environment, code_language) = match connection_kind {
        TerminalConnectionKind::Local => (
            format!(
                "运行 Navop 的本地 {} 终端；优先遵循当前 shell 与本机平台语法，不要默认假定为 Linux",
                std::env::consts::OS
            ),
            if cfg!(target_os = "windows") {
                "powershell"
            } else {
                "bash"
            },
        ),
        TerminalConnectionKind::Ssh => ("远程 Linux shell 环境".to_string(), "bash"),
        TerminalConnectionKind::Serial => ("串口终端环境".to_string(), "text"),
    };
    format!(
        r#"你是终端侧边栏中的命令助手，当前目标是{environment}。
请严格遵循以下规则：
1. 当用户请求安装、配置、排查、运维或执行命令时，优先返回可以直接在当前目标终端执行的命令。
2. 所有命令都必须放在 Markdown 代码块中，代码块语言使用 {code_language}。
3. 每个代码块只能包含一条命令，不要在同一个代码块中放多条命令，不要使用 &&、; 或换行把多个命令塞进同一个代码块，除非用户明确要求组合命令。
4. 如果任务需要多步骤，请拆成多个独立代码块，每个代码块只对应一步的一条命令。
5. 解释、注意事项、风险提示、步骤标题必须写在代码块外面，保持简洁。
6. 如果命令依赖 sudo、包管理器或发行版差异，请先简短说明再给命令。
7. 如果用户明确要求其他平台、非命令答案或更详细的解释，再按用户要求调整。"#
    )
}

fn agent_theme_from_terminal_theme(theme: &TerminalTheme) -> AgentChatTheme {
    let colors = theme.colors();
    AgentChatTheme {
        is_dark: theme.is_dark(),
        background: colors.background,
        foreground: colors.foreground,
        muted: colors.muted,
        muted_foreground: colors.muted_foreground,
        border: colors.border,
        panel: colors.muted,
        panel_hover: colors.muted.opacity(0.72),
        accent: colors.accent,
        accent_foreground: colors.accent_foreground,
        code_background: colors.muted,
        code_foreground: colors.foreground,
        table_header: colors.muted,
        table_row: colors.background,
        table_row_alt: colors.muted.opacity(0.35),
        quote_border: colors.border,
        link: colors.accent,
    }
}

fn build_terminal_ai_context(
    current_connection: &StoredConnection,
    all_connections: &[StoredConnection],
) -> (
    agent_runtime::AgentResourceScope,
    agent_runtime::ResourceCatalog,
    Vec<MentionItem>,
) {
    let catalog = terminal_ai_connection_catalog(current_connection, all_connections);
    build_sidebar_resource_state(
        current_connection,
        &catalog,
        agent_runtime::DefaultTargetReason::CurrentTerminal,
    )
}

fn build_live_terminal_ai_context(
    current_terminal: agent_runtime::ResourceRef,
    all_connections: &[StoredConnection],
) -> (
    agent_runtime::AgentResourceScope,
    agent_runtime::ResourceCatalog,
    Vec<MentionItem>,
) {
    let mut resources = vec![current_terminal.clone()];
    resources.extend(build_resource_catalog(all_connections));
    let catalog = agent_runtime::ResourceCatalog::new(resources);
    let scope = agent_runtime::AgentResourceScope::single_default(
        current_terminal.clone(),
        agent_runtime::DefaultTargetReason::CurrentTerminal,
    );
    let detail = std::iter::once("terminal".to_string())
        .chain(current_terminal.aliases.iter().cloned())
        .collect::<Vec<_>>()
        .join(" · ");
    let mut mentions = vec![MentionItem::new(
        current_terminal.id.as_str(),
        current_terminal.label.clone(),
        detail,
        current_terminal.kind.as_str(),
    )];
    mentions.extend(build_mentions_from_connections(all_connections));
    (scope, catalog, mentions)
}

fn terminal_ai_connection_catalog(
    current_connection: &StoredConnection,
    all_connections: &[StoredConnection],
) -> Vec<StoredConnection> {
    let mut catalog = Vec::with_capacity(all_connections.len().max(1));
    catalog.push(current_connection.clone());
    catalog.extend(
        all_connections
            .iter()
            .filter(|connection| !same_connection(connection, current_connection))
            .cloned(),
    );
    catalog
}

fn same_connection(left: &StoredConnection, right: &StoredConnection) -> bool {
    match (left.id, right.id) {
        (Some(left_id), Some(right_id)) => left_id == right_id,
        _ => {
            left.name == right.name
                && left.connection_type == right.connection_type
                && left.params == right.params
        }
    }
}

fn load_terminal_ai_connections(cx: &App) -> Vec<StoredConnection> {
    let Some(storage) = cx.try_global::<GlobalStorageState>() else {
        return Vec::new();
    };
    let Some(repo) = storage.storage.get::<ConnectionRepository>() else {
        return Vec::new();
    };
    repo.list().unwrap_or_else(|error| {
        tracing::warn!(%error, "Failed to load terminal AI connection catalog");
        Vec::new()
    })
}

/// 侧边栏面板类型
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SidebarPanel {
    /// 设置面板（搜索 + 字体 + 主题）
    Settings,
    /// 快捷命令面板
    QuickCommand,
    /// 历史命令面板
    HistoryCommand,
    /// AI 聊天面板
    AiChat,
    /// SSH 广播输入面板
    BroadcastInput,
    /// 文件管理器面板（仅 SSH 终端）
    FileManager,
    /// 服务器监控面板（仅 SSH 终端）
    ServerMonitor,
}

impl SidebarPanel {
    pub const ALL: [SidebarPanel; 7] = [
        SidebarPanel::Settings,
        SidebarPanel::QuickCommand,
        SidebarPanel::HistoryCommand,
        SidebarPanel::AiChat,
        SidebarPanel::BroadcastInput,
        SidebarPanel::FileManager,
        SidebarPanel::ServerMonitor,
    ];

    pub fn all() -> &'static [SidebarPanel] {
        &Self::ALL
    }

    pub fn local_id(&self) -> &'static str {
        match self {
            SidebarPanel::Settings => "terminal.settings",
            SidebarPanel::QuickCommand => "terminal.quick-command",
            SidebarPanel::HistoryCommand => "terminal.history-command",
            SidebarPanel::AiChat => "terminal.ai-chat",
            SidebarPanel::BroadcastInput => "terminal.broadcast-input",
            SidebarPanel::FileManager => "terminal.file-manager",
            SidebarPanel::ServerMonitor => "terminal.server-monitor",
        }
    }

    pub fn icon_name(&self) -> IconName {
        match self {
            SidebarPanel::Settings => IconName::Settings,
            SidebarPanel::QuickCommand => IconName::TerminalQuickCommandColor,
            SidebarPanel::HistoryCommand => IconName::TerminalHistoryColor,
            SidebarPanel::AiChat => IconName::AI,
            SidebarPanel::BroadcastInput => IconName::TerminalBroadcastColor,
            SidebarPanel::FileManager => IconName::TerminalFileManagerColor,
            SidebarPanel::ServerMonitor => IconName::TerminalServerMonitorColor,
        }
    }

    /// 获取面板图标
    pub fn icon(&self) -> Icon {
        match self {
            SidebarPanel::Settings => IconName::SettingColor.color(),
            SidebarPanel::QuickCommand => IconName::TerminalQuickCommandColor.color(),
            SidebarPanel::HistoryCommand => IconName::TerminalHistoryColor.color(),
            SidebarPanel::AiChat => IconName::AI.color(),
            SidebarPanel::BroadcastInput => IconName::TerminalBroadcastColor.color(),
            SidebarPanel::FileManager => IconName::TerminalFileManagerColor.color(),
            SidebarPanel::ServerMonitor => IconName::TerminalServerMonitorColor.color(),
        }
    }

    /// 获取面板标题
    pub fn title(&self) -> &'static str {
        match self {
            SidebarPanel::Settings => "Settings",
            SidebarPanel::QuickCommand => "Quick Commands",
            SidebarPanel::HistoryCommand => "History Commands",
            SidebarPanel::AiChat => "AI Chat",
            SidebarPanel::BroadcastInput => "Broadcast Input",
            SidebarPanel::FileManager => "File Manager",
            SidebarPanel::ServerMonitor => "Server Monitor",
        }
    }

    pub(crate) fn needs_internal_tool_frame_header(&self) -> bool {
        !matches!(self, SidebarPanel::AiChat | SidebarPanel::FileManager)
    }
}

fn terminal_sidebar_available_panels(
    has_file_manager: bool,
    has_server_monitor: bool,
    history_supported: bool,
    broadcast_supported: bool,
) -> Vec<SidebarPanel> {
    let mut panels = vec![SidebarPanel::Settings, SidebarPanel::AiChat];
    if broadcast_supported {
        panels.push(SidebarPanel::BroadcastInput);
    }
    if has_file_manager {
        panels.push(SidebarPanel::FileManager);
    }
    if has_server_monitor {
        panels.push(SidebarPanel::ServerMonitor);
    }
    if history_supported {
        panels.push(SidebarPanel::HistoryCommand);
    }
    panels.push(SidebarPanel::QuickCommand);
    panels
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ToolPanelState {
    pub open: bool,
    pub placement: SidebarPlacement,
}

impl Default for ToolPanelState {
    fn default() -> Self {
        Self {
            open: false,
            placement: SidebarPlacement::Right,
        }
    }
}

#[derive(Clone, Debug)]
pub struct TerminalToolDockState {
    panels: Vec<SidebarPanel>,
    states: HashMap<SidebarPanel, ToolPanelState>,
}

impl TerminalToolDockState {
    pub fn new(panels: impl IntoIterator<Item = SidebarPanel>) -> Self {
        let mut seen = HashSet::new();
        let panels = panels
            .into_iter()
            .filter(|panel| seen.insert(*panel))
            .collect::<Vec<_>>();
        let states = panels
            .iter()
            .copied()
            .map(|panel| (panel, ToolPanelState::default()))
            .collect();
        Self { panels, states }
    }

    pub fn toolbar_visible(&self) -> bool {
        true
    }

    fn close_open_tools_at_placement(
        &mut self,
        placement: SidebarPlacement,
        except: SidebarPanel,
    ) -> bool {
        let mut changed = false;
        for (panel, state) in self.states.iter_mut() {
            if *panel != except && state.open && state.placement == placement {
                state.open = false;
                changed = true;
            }
        }
        changed
    }

    pub fn open_tool(&mut self, panel: SidebarPanel) -> bool {
        let Some(placement) = self.states.get(&panel).map(|state| state.placement) else {
            return false;
        };
        let changed = self.close_open_tools_at_placement(placement, panel);
        let Some(state) = self.states.get_mut(&panel) else {
            return changed;
        };
        if state.open {
            return changed;
        }
        state.open = true;
        true
    }

    pub fn close_tool(&mut self, panel: SidebarPanel) -> bool {
        let Some(state) = self.states.get_mut(&panel) else {
            return false;
        };
        if !state.open {
            return false;
        }
        state.open = false;
        true
    }

    pub fn close_all(&mut self) -> bool {
        let mut changed = false;
        for state in self.states.values_mut() {
            if state.open {
                state.open = false;
                changed = true;
            }
        }
        changed
    }

    pub fn toggle_tool(&mut self, panel: SidebarPanel) -> bool {
        if self.is_tool_open(panel) {
            self.close_tool(panel)
        } else {
            self.open_tool(panel)
        }
    }

    pub fn move_tool(&mut self, panel: SidebarPanel, placement: SidebarPlacement) -> bool {
        let Some(is_open) = self.states.get(&panel).map(|state| state.open) else {
            return false;
        };
        let changed = if is_open {
            self.close_open_tools_at_placement(placement, panel)
        } else {
            false
        };
        let Some(state) = self.states.get_mut(&panel) else {
            return changed;
        };
        if state.placement == placement {
            return changed;
        }
        state.placement = placement;
        true
    }

    pub fn is_tool_open(&self, panel: SidebarPanel) -> bool {
        self.states
            .get(&panel)
            .map(|state| state.open)
            .unwrap_or(false)
    }

    pub fn panel_placement(&self, panel: SidebarPanel) -> SidebarPlacement {
        self.states
            .get(&panel)
            .map(|state| state.placement)
            .unwrap_or(SidebarPlacement::Right)
    }

    pub fn open_panels(&self) -> Vec<(SidebarPanel, SidebarPlacement)> {
        self.panels
            .iter()
            .filter_map(|panel| {
                let state = self.states.get(panel)?;
                state.open.then_some((*panel, state.placement))
            })
            .collect()
    }

    pub fn first_open_panel(&self) -> Option<SidebarPanel> {
        self.open_panels().first().map(|(panel, _)| *panel)
    }
}

/// 终端侧边栏事件
#[derive(Clone, Debug)]
pub enum TerminalSidebarEvent {
    /// 面板切换
    PanelChanged(Option<SidebarPanel>),
    /// 搜索模式变化
    SearchPatternChanged(String),
    /// 搜索前一个
    SearchPrevious,
    /// 搜索下一个
    SearchNext,
    /// 字体大小变更
    FontSizeChanged(f32),
    /// 字体变更
    FontFamilyChanged(String),
    /// 主题变更
    ThemeChanged(TerminalTheme),
    /// 粘贴命令到终端输入区（不自动回车）
    ExecuteCommand(String),
    /// 请求询问 AI
    AskAi,
    /// 粘贴代码到终端（用于AI生成的代码块）
    PasteCodeToTerminal(String),
    /// 光标闪烁变更
    CursorBlinkChanged(bool),
    /// 非 bracketed 模式下，多行粘贴确认开关
    ConfirmMultilinePasteChanged(bool),
    /// 高危命令确认开关
    ConfirmHighRiskCommandChanged(bool),
    /// 选中自动复制开关
    AutoCopyChanged(bool),
    /// 自动补全开关
    AutocompleteChanged(bool),
    /// 中键粘贴开关
    MiddleClickPasteChanged(bool),
    /// 右键快速粘贴开关
    RightClickPasteChanged(bool),
    /// SSH 粘贴图片上传开关
    PasteImageUploadChanged(bool),
    /// vim/TUI 滚轮转方向键开关
    VimScrollToArrowKeysChanged(bool),
    /// 路径与终端同步开关
    SyncPathChanged(bool),
    /// 自定义高亮规则变更
    CustomHighlightsChanged(Vec<TerminalHighlightRule>),
    /// 在终端中 cd 到指定路径
    CdToTerminal(String),
    /// 请求将终端当前工作目录同步到文件管理器
    SyncWorkingDir,
}

/// 终端侧边栏组件
pub struct TerminalSidebar {
    /// 终端工具面板 dock 状态
    tool_dock: TerminalToolDockState,
    /// 设置面板
    settings_panel: Entity<SettingsPanel>,
    /// 快捷命令面板
    quick_command_panel: Entity<QuickCommandPanel>,
    /// 历史命令面板
    history_command_panel: Option<Entity<HistoryCommandPanel>>,
    /// AI 聊天面板
    ai_chat_panel: Entity<DefaultAgentChatPanel>,
    /// SSH 广播输入面板
    broadcast_input_panel: Option<Entity<BroadcastInputPanel>>,
    /// 文件管理器面板（仅 SSH 终端时创建）
    file_manager_panel: Option<Entity<FileManagerPanel>>,
    /// 服务器监控面板（仅 SSH 终端时创建）
    server_monitor_panel: Option<Entity<ServerMonitorPanel>>,
    /// 路径与终端同步开关（默认开启）
    sync_path_enabled: bool,
    /// 焦点句柄
    focus_handle: FocusHandle,
    /// 终端主题配色（用于侧边栏工具栏）
    colors: TerminalColors,
    /// 订阅句柄
    _subs: Vec<Subscription>,
}

impl TerminalSidebar {
    pub fn new(
        connection_id: Option<i64>,
        connection_kind: TerminalConnectionKind,
        stored_connection: Option<StoredConnection>,
        terminal_ai_resource: Option<agent_runtime::ResourceRef>,
        ssh_config: Option<SshTerminalConfig>,
        ssh_session_manager: Option<Arc<SshSessionManager>>,
        initial_theme: &TerminalTheme,
        initial_font_size: Pixels,
        initial_font_family: SharedString,
        sync_path_enabled: bool,
        history_scope: Option<TerminalHistoryScope>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let colors = initial_theme.colors();
        let has_file_manager = stored_connection.is_some();
        let broadcast_supported = ssh_config.is_some();
        if broadcast_supported {
            init_broadcast_input_registry(cx);
        }
        let history_user = ssh_config
            .as_ref()
            .map(|config| config.ssh_config.username.clone())
            .or_else(|| std::env::var("USER").ok());
        let auto_show_server_monitor = ServerMonitorPanel::load_monitor_enabled(connection_id);
        let settings_panel = cx.new(|cx| {
            SettingsPanel::new(
                initial_theme,
                initial_font_size,
                initial_font_family,
                has_file_manager,
                true,
                true,
                true,
                false,
                true,
                sync_path_enabled,
                true,
                window,
                cx,
            )
        });
        let broadcast_input_panel = broadcast_supported
            .then(|| broadcast_input_registry(cx))
            .flatten()
            .map(|registry| {
                let config = BroadcastInputPanelConfig {
                    registry,
                    colors: colors.clone(),
                };
                cx.new(|cx| BroadcastInputPanel::new(config, window, cx))
            });
        let quick_command_panel =
            cx.new(|cx| QuickCommandPanel::new(connection_id, colors.clone(), window, cx));
        let history_command_panel = history_scope.map(|scope| {
            cx.new(|cx| {
                HistoryCommandPanel::new(scope, history_user.clone(), colors.clone(), window, cx)
            })
        });
        let ai_chat_panel = if let Some(connection) = stored_connection.as_ref() {
            let connections = load_terminal_ai_connections(cx);
            let (scope, catalog, mentions) = build_terminal_ai_context(connection, &connections);
            cx.new(|cx| {
                DefaultAgentChatPanel::new_sidebar_with_scope_and_catalog(
                    scope, catalog, mentions, window, cx,
                )
            })
        } else if let Some(terminal_resource) = terminal_ai_resource {
            let connections = load_terminal_ai_connections(cx);
            let (scope, catalog, mentions) =
                build_live_terminal_ai_context(terminal_resource, &connections);
            cx.new(|cx| {
                DefaultAgentChatPanel::new_sidebar_with_scope_and_catalog(
                    scope, catalog, mentions, window, cx,
                )
            })
        } else {
            cx.new(|cx| DefaultAgentChatPanel::new(window, cx))
        };

        // 仅 SSH 终端（有 StoredConnection）时创建文件管理器面板
        let file_manager_panel =
            stored_connection
                .zip(ssh_session_manager.clone())
                .map(|(conn, manager)| {
                    cx.new(|cx| FileManagerPanel::new(conn, manager, colors.clone(), window, cx))
                });
        let server_monitor_panel = ssh_config
            .zip(ssh_session_manager)
            .map(|(_config, manager)| {
                cx.new(|cx| {
                    ServerMonitorPanel::new(
                        connection_id,
                        manager,
                        auto_show_server_monitor,
                        colors.clone(),
                        cx,
                    )
                })
            });

        // 注册 bash/sh 代码块操作，并注入终端专属提示词
        let sidebar_entity = cx.entity();
        let ai_theme = agent_theme_from_terminal_theme(initial_theme);
        ai_chat_panel.update(cx, |panel, cx| {
            panel.set_theme(Some(ai_theme), cx);
            panel.set_sidebar_header_visible(true, cx);
            panel.set_sidebar_frame_controls(true, SidebarPlacement::Right, cx);
            panel.set_system_instruction(Some(terminal_ai_system_instruction(connection_kind)), cx);
            // 注册复制操作（默认已有，这里只是确保）
            // 注册粘贴到终端操作
            if let Some(paste_action) = CodeBlockAction::new("paste-to-terminal")
                .icon(IconName::SquareTerminal)
                .label(t!("TerminalSidebar.paste_to_terminal").to_string())
                .matcher(LanguageMatcher::shell())
                .on_click({
                    let sidebar = sidebar_entity.clone();
                    move |code, _lang, _window, cx| {
                        sidebar.update(cx, |_this, cx| {
                            cx.emit(TerminalSidebarEvent::PasteCodeToTerminal(code.clone()));
                        });
                    }
                })
                .build()
            {
                panel.register_code_block_action(paste_action, cx);
            }
        });

        // 订阅设置面板事件
        let set_sub = cx.subscribe(
            &settings_panel,
            |this, _, event: &settings_panel::SettingsPanelEvent, cx| match event {
                settings_panel::SettingsPanelEvent::Close => {
                    this.close_tool(SidebarPanel::Settings, cx);
                }
                settings_panel::SettingsPanelEvent::SearchPatternChanged(pattern) => {
                    cx.emit(TerminalSidebarEvent::SearchPatternChanged(pattern.clone()));
                }
                settings_panel::SettingsPanelEvent::SearchPrevious => {
                    cx.emit(TerminalSidebarEvent::SearchPrevious);
                }
                settings_panel::SettingsPanelEvent::SearchNext => {
                    cx.emit(TerminalSidebarEvent::SearchNext);
                }
                settings_panel::SettingsPanelEvent::FontSizeChanged(size) => {
                    cx.emit(TerminalSidebarEvent::FontSizeChanged(*size));
                }
                settings_panel::SettingsPanelEvent::FontFamilyChanged(family) => {
                    cx.emit(TerminalSidebarEvent::FontFamilyChanged(family.clone()));
                }
                settings_panel::SettingsPanelEvent::ThemeChanged(theme) => {
                    this.colors = theme.colors();
                    cx.emit(TerminalSidebarEvent::ThemeChanged(theme.clone()));
                }
                settings_panel::SettingsPanelEvent::CursorBlinkChanged(enabled) => {
                    cx.emit(TerminalSidebarEvent::CursorBlinkChanged(*enabled));
                }
                settings_panel::SettingsPanelEvent::ConfirmMultilinePasteChanged(enabled) => {
                    cx.emit(TerminalSidebarEvent::ConfirmMultilinePasteChanged(*enabled));
                }
                settings_panel::SettingsPanelEvent::ConfirmHighRiskCommandChanged(enabled) => {
                    cx.emit(TerminalSidebarEvent::ConfirmHighRiskCommandChanged(
                        *enabled,
                    ));
                }
                settings_panel::SettingsPanelEvent::AutoCopyChanged(enabled) => {
                    cx.emit(TerminalSidebarEvent::AutoCopyChanged(*enabled));
                }
                settings_panel::SettingsPanelEvent::AutocompleteChanged(enabled) => {
                    cx.emit(TerminalSidebarEvent::AutocompleteChanged(*enabled));
                }
                settings_panel::SettingsPanelEvent::MiddleClickPasteChanged(enabled) => {
                    cx.emit(TerminalSidebarEvent::MiddleClickPasteChanged(*enabled));
                }
                settings_panel::SettingsPanelEvent::RightClickPasteChanged(enabled) => {
                    cx.emit(TerminalSidebarEvent::RightClickPasteChanged(*enabled));
                }
                settings_panel::SettingsPanelEvent::PasteImageUploadChanged(enabled) => {
                    cx.emit(TerminalSidebarEvent::PasteImageUploadChanged(*enabled));
                }
                settings_panel::SettingsPanelEvent::VimScrollToArrowKeysChanged(enabled) => {
                    cx.emit(TerminalSidebarEvent::VimScrollToArrowKeysChanged(*enabled));
                }
                settings_panel::SettingsPanelEvent::SyncPathChanged(enabled) => {
                    this.sync_path_enabled = *enabled;
                    cx.emit(TerminalSidebarEvent::SyncPathChanged(*enabled));
                }
                settings_panel::SettingsPanelEvent::CustomHighlightsChanged(rules) => {
                    cx.emit(TerminalSidebarEvent::CustomHighlightsChanged(rules.clone()));
                }
            },
        );

        // 订阅快捷命令面板事件
        let quick_sub = cx.subscribe(
            &quick_command_panel,
            |this, _, event: &quick_command_panel::QuickCommandPanelEvent, cx| match event {
                quick_command_panel::QuickCommandPanelEvent::Close => {
                    this.close_tool(SidebarPanel::QuickCommand, cx);
                }
                quick_command_panel::QuickCommandPanelEvent::ExecuteCommand(cmd) => {
                    cx.emit(TerminalSidebarEvent::ExecuteCommand(cmd.clone()));
                }
            },
        );

        let history_sub = history_command_panel.as_ref().map(|panel| {
            cx.subscribe(
                panel,
                |_this, _, event: &HistoryCommandPanelEvent, cx| match event {
                    HistoryCommandPanelEvent::ExecuteCommand(command) => {
                        cx.emit(TerminalSidebarEvent::ExecuteCommand(command.clone()));
                    }
                },
            )
        });

        // 订阅 AI 聊天面板关闭事件
        let ai_chat_sub = cx.subscribe(
            &ai_chat_panel,
            |this, _, event: &DefaultAgentChatPanelEvent, cx| match event {
                DefaultAgentChatPanelEvent::Close => {
                    this.close_tool(SidebarPanel::AiChat, cx);
                }
                DefaultAgentChatPanelEvent::MoveTo(placement) => {
                    this.move_tool(SidebarPanel::AiChat, *placement, cx);
                }
            },
        );

        let mut subs = vec![set_sub, quick_sub, ai_chat_sub];
        if let Some(sub) = history_sub {
            subs.push(sub);
        }

        // 订阅文件管理器面板事件
        if let Some(ref fm_panel) = file_manager_panel {
            let fm_sub =
                cx.subscribe(
                    fm_panel,
                    |this, _, event: &FileManagerPanelEvent, cx| match event {
                        FileManagerPanelEvent::Close => {
                            this.close_tool(SidebarPanel::FileManager, cx);
                        }
                        FileManagerPanelEvent::MoveTo(placement) => {
                            this.move_tool(SidebarPanel::FileManager, *placement, cx);
                        }
                        FileManagerPanelEvent::CdToTerminal(path) => {
                            cx.emit(TerminalSidebarEvent::CdToTerminal(path.clone()));
                        }
                        FileManagerPanelEvent::SyncWorkingDir => {
                            cx.emit(TerminalSidebarEvent::SyncWorkingDir);
                        }
                    },
                );
            subs.push(fm_sub);
        }

        if let Some(ref monitor_panel) = server_monitor_panel {
            let monitor_sub = cx.subscribe(
                monitor_panel,
                |this, _, event: &ServerMonitorPanelEvent, cx| match event {
                    ServerMonitorPanelEvent::Close => {
                        this.close_tool(SidebarPanel::ServerMonitor, cx);
                    }
                },
            );
            subs.push(monitor_sub);
        }

        let available_panels = terminal_sidebar_available_panels(
            file_manager_panel.is_some(),
            server_monitor_panel.is_some(),
            history_command_panel.is_some(),
            broadcast_input_panel.is_some(),
        );
        Self {
            tool_dock: TerminalToolDockState::new(available_panels),
            settings_panel,
            quick_command_panel,
            history_command_panel,
            ai_chat_panel,
            broadcast_input_panel,
            file_manager_panel,
            server_monitor_panel,
            sync_path_enabled,
            focus_handle: cx.focus_handle(),
            colors,
            _subs: subs,
        }
    }

    /// 获取当前激活的面板
    pub fn active_panel(&self) -> Option<SidebarPanel> {
        self.tool_dock.first_open_panel()
    }

    /// 设置激活的面板
    pub fn set_active_panel(&mut self, panel: Option<SidebarPanel>, cx: &mut Context<Self>) {
        let changed = match panel {
            Some(panel) => self.open_tool_internal(panel, cx),
            None => self.tool_dock.close_all(),
        };
        if changed {
            cx.emit(TerminalSidebarEvent::PanelChanged(panel));
            cx.notify();
        }
    }

    /// 切换面板
    pub fn toggle_panel(&mut self, panel: SidebarPanel, cx: &mut Context<Self>) {
        if self.tool_dock.is_tool_open(panel) {
            self.close_tool(panel, cx);
        } else {
            self.open_tool(panel, cx);
        }
    }

    /// 是否显示侧边栏
    pub fn is_visible(&self) -> bool {
        !self.tool_dock.open_panels().is_empty()
    }

    pub fn toolbar_visible(&self) -> bool {
        self.tool_dock.toolbar_visible()
    }

    pub fn is_tool_open(&self, panel: SidebarPanel) -> bool {
        self.tool_dock.is_tool_open(panel)
    }

    pub fn open_tool_panels(&self) -> Vec<(SidebarPanel, SidebarPlacement)> {
        self.tool_dock.open_panels()
    }

    pub fn panel_placement(&self, panel: SidebarPanel) -> SidebarPlacement {
        self.tool_dock.panel_placement(panel)
    }

    pub(crate) fn colors(&self) -> TerminalColors {
        self.colors.clone()
    }

    pub fn panel_view(&self, panel: SidebarPanel) -> Option<AnyView> {
        match panel {
            SidebarPanel::Settings => Some(self.settings_panel.clone().into()),
            SidebarPanel::QuickCommand => Some(self.quick_command_panel.clone().into()),
            SidebarPanel::HistoryCommand => self
                .history_command_panel
                .as_ref()
                .map(|panel| panel.clone().into()),
            SidebarPanel::AiChat => Some(self.ai_chat_panel.clone().into()),
            SidebarPanel::BroadcastInput => self
                .broadcast_input_panel
                .as_ref()
                .map(|panel| panel.clone().into()),
            SidebarPanel::FileManager => self
                .file_manager_panel
                .as_ref()
                .map(|panel| panel.clone().into()),
            SidebarPanel::ServerMonitor => self
                .server_monitor_panel
                .as_ref()
                .map(|panel| panel.clone().into()),
        }
    }

    fn toolbar_snapshot(&self) -> TerminalToolbarSnapshot {
        TerminalToolbarSnapshot {
            colors: self.colors.clone(),
            buttons: self
                .tool_dock
                .panels
                .iter()
                .copied()
                .map(|panel| TerminalToolbarButtonSnapshot {
                    panel,
                    open: self.tool_dock.is_tool_open(panel),
                })
                .collect(),
        }
    }

    pub fn open_tool(&mut self, panel: SidebarPanel, cx: &mut Context<Self>) {
        if self.open_tool_internal(panel, cx) {
            cx.emit(TerminalSidebarEvent::PanelChanged(Some(panel)));
            cx.notify();
        }
    }

    pub fn close_tool(&mut self, panel: SidebarPanel, cx: &mut Context<Self>) {
        if self.tool_dock.close_tool(panel) {
            cx.emit(TerminalSidebarEvent::PanelChanged(self.active_panel()));
            cx.notify();
        }
    }

    pub fn move_tool(
        &mut self,
        panel: SidebarPanel,
        placement: SidebarPlacement,
        cx: &mut Context<Self>,
    ) {
        if self.tool_dock.move_tool(panel, placement) {
            self.update_panel_frame_placement(panel, placement, cx);
            cx.emit(TerminalSidebarEvent::PanelChanged(Some(panel)));
            cx.notify();
        }
    }

    fn open_tool_internal(&mut self, panel: SidebarPanel, cx: &mut Context<Self>) -> bool {
        self.prepare_panel_open(panel, cx);
        self.update_panel_frame_placement(panel, self.panel_placement(panel), cx);
        self.tool_dock.open_tool(panel)
    }

    fn update_panel_frame_placement(
        &self,
        panel: SidebarPanel,
        placement: SidebarPlacement,
        cx: &mut Context<Self>,
    ) {
        match panel {
            SidebarPanel::AiChat => {
                self.ai_chat_panel.update(cx, |panel, cx| {
                    panel.set_sidebar_frame_controls(true, placement, cx);
                });
            }
            SidebarPanel::FileManager => {
                if let Some(ref fm_panel) = self.file_manager_panel {
                    fm_panel.update(cx, |panel, cx| {
                        panel.set_frame_placement(placement, cx);
                    });
                }
            }
            _ => {}
        }
    }

    fn prepare_panel_open(&self, panel: SidebarPanel, cx: &mut Context<Self>) {
        if panel == SidebarPanel::FileManager {
            if let Some(ref fm_panel) = self.file_manager_panel {
                fm_panel.update(cx, |panel, cx| {
                    panel.connect_if_idle(cx);
                });
            }
        }
        if panel == SidebarPanel::ServerMonitor {
            if let Some(ref monitor_panel) = self.server_monitor_panel {
                monitor_panel.update(cx, |panel, cx| {
                    panel.restore_monitoring(cx);
                });
            }
        }
    }

    /// 更新设置面板的当前主题
    pub fn update_current_theme(
        &mut self,
        theme: &TerminalTheme,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.colors = theme.colors();
        // 更新设置面板（会同时更新颜色和主题）
        let theme_clone = theme.clone();
        self.settings_panel.update(cx, |panel, cx| {
            panel.set_current_theme(theme_clone, window, cx);
        });
        self.quick_command_panel.update(cx, |panel, cx| {
            panel.set_colors(self.colors.clone(), cx);
        });
        if let Some(ref history_panel) = self.history_command_panel {
            history_panel.update(cx, |panel, cx| {
                panel.set_colors(self.colors.clone(), cx);
            });
        }
        self.ai_chat_panel.update(cx, |panel, cx| {
            panel.set_theme(Some(agent_theme_from_terminal_theme(theme)), cx);
        });
        if let Some(ref broadcast_panel) = self.broadcast_input_panel {
            broadcast_panel.update(cx, |panel, cx| {
                panel.set_colors(self.colors.clone(), cx);
            });
        }
        if let Some(ref fm_panel) = self.file_manager_panel {
            fm_panel.update(cx, |panel, cx| {
                panel.set_colors(self.colors.clone(), cx);
            });
        }
        if let Some(ref monitor_panel) = self.server_monitor_panel {
            monitor_panel.update(cx, |panel, cx| {
                panel.set_colors(self.colors.clone(), cx);
            });
        }

        cx.notify();
    }

    pub fn refresh_history_commands(&mut self, cx: &mut Context<Self>) {
        if let Some(ref history_panel) = self.history_command_panel {
            history_panel.update(cx, |panel, cx| {
                panel.refresh_commands(cx);
            });
        }
    }

    pub fn set_font_size(&mut self, font_size: f32, window: &mut Window, cx: &mut Context<Self>) {
        self.settings_panel.update(cx, |panel, cx| {
            panel.set_font_size(font_size, window, cx);
        });
    }

    pub fn set_font_family(
        &mut self,
        font_family: SharedString,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.settings_panel.update(cx, |panel, cx| {
            panel.set_font_family(font_family, window, cx);
        });
    }

    pub fn set_auto_copy(&mut self, enabled: bool, cx: &mut Context<Self>) {
        self.settings_panel.update(cx, |panel, cx| {
            panel.set_auto_copy(enabled, cx);
        });
    }

    pub fn set_autocomplete_enabled(&mut self, enabled: bool, cx: &mut Context<Self>) {
        self.settings_panel.update(cx, |panel, cx| {
            panel.set_autocomplete_enabled(enabled, cx);
        });
    }

    pub fn set_middle_click_paste(&mut self, enabled: bool, cx: &mut Context<Self>) {
        self.settings_panel.update(cx, |panel, cx| {
            panel.set_middle_click_paste(enabled, cx);
        });
    }

    pub fn set_right_click_paste(&mut self, enabled: bool, cx: &mut Context<Self>) {
        self.settings_panel.update(cx, |panel, cx| {
            panel.set_right_click_paste(enabled, cx);
        });
    }

    pub fn set_paste_image_upload(&mut self, enabled: bool, cx: &mut Context<Self>) {
        self.settings_panel.update(cx, |panel, cx| {
            panel.set_paste_image_upload(enabled, cx);
        });
    }

    pub fn set_vim_scroll_to_arrow_keys(&mut self, enabled: bool, cx: &mut Context<Self>) {
        self.settings_panel.update(cx, |panel, cx| {
            panel.set_vim_scroll_to_arrow_keys(enabled, cx);
        });
    }

    pub fn set_sync_path_enabled(&mut self, enabled: bool, cx: &mut Context<Self>) {
        self.sync_path_enabled = enabled;
        self.settings_panel.update(cx, |panel, cx| {
            panel.set_sync_path(enabled, cx);
        });
    }

    pub fn set_cursor_blink(&mut self, enabled: bool, cx: &mut Context<Self>) {
        self.settings_panel.update(cx, |panel, cx| {
            panel.set_cursor_blink(enabled, cx);
        });
    }

    pub fn set_confirm_multiline_paste(&mut self, enabled: bool, cx: &mut Context<Self>) {
        self.settings_panel.update(cx, |panel, cx| {
            panel.set_confirm_multiline_paste(enabled, cx);
        });
    }

    pub fn set_confirm_high_risk_command(&mut self, enabled: bool, cx: &mut Context<Self>) {
        self.settings_panel.update(cx, |panel, cx| {
            panel.set_confirm_high_risk_command(enabled, cx);
        });
    }

    pub fn set_custom_highlights(
        &mut self,
        rules: Vec<TerminalHighlightRule>,
        cx: &mut Context<Self>,
    ) {
        self.settings_panel.update(cx, |panel, cx| {
            panel.set_custom_highlights(rules, cx);
        });
    }

    /// 更新搜索输入框的值
    pub fn set_search_value(&self, value: &str, window: &mut Window, cx: &mut Context<Self>) {
        self.settings_panel.update(cx, |panel, cx| {
            panel.set_search_value(value, window, cx);
        });
    }

    /// 获取搜索输入框的值
    pub fn search_value(&self, cx: &App) -> String {
        self.settings_panel.read(cx).search_value(cx)
    }

    /// 询问 AI
    pub fn ask_ai(&mut self, message: String, cx: &mut Context<Self>) {
        self.open_tool_internal(SidebarPanel::AiChat, cx);

        // 发送消息到 AI 聊天面板
        self.ai_chat_panel.update(cx, |panel, cx| {
            panel.send_external_message(message, cx);
        });

        cx.emit(TerminalSidebarEvent::AskAi);
        cx.notify();
    }

    /// 添加快捷命令（外部调用）
    pub fn add_quick_command(&self, command: String, window: &mut Window, cx: &mut Context<Self>) {
        self.quick_command_panel.update(cx, |panel, cx| {
            panel.add_command_external(command, window, cx);
        });
    }

    /// 从终端 OSC 7 同步路径到文件管理器
    ///
    /// 检查 `sync_path_enabled` 且存在文件管理器面板时，导航到指定路径。
    pub fn sync_file_manager_path(&mut self, path: String, cx: &mut Context<Self>) {
        if !self.sync_path_enabled {
            return;
        }
        if let Some(ref fm_panel) = self.file_manager_panel {
            fm_panel.update(cx, |panel, cx| {
                panel.sync_navigate_to(path, cx);
            });
        }
    }

    /// 设置文件管理器的初始工作目录（连接前调用）
    ///
    /// 当终端收到 OSC 7 但文件管理器尚未连接时，缓存路径供首次连接使用。
    pub fn set_file_manager_initial_dir(&mut self, path: String, cx: &mut Context<Self>) {
        if let Some(ref fm_panel) = self.file_manager_panel {
            fm_panel.update(cx, |panel, _cx| {
                panel.set_initial_working_dir(path);
            });
        }
    }

    /// 在终端重连时同步重建文件管理器连接
    pub fn reconnect_file_manager(&mut self, working_dir: Option<String>, cx: &mut Context<Self>) {
        if let Some(ref fm_panel) = self.file_manager_panel {
            fm_panel.update(cx, |panel, cx| {
                panel.reconnect_with_working_dir(working_dir.clone(), cx);
            });
        }
    }

    pub fn reconnect_server_monitor(&mut self, cx: &mut Context<Self>) {
        if let Some(ref monitor_panel) = self.server_monitor_panel {
            monitor_panel.update(cx, |panel, cx| {
                panel.reconnect(cx);
            });
        }
    }

    /// 渲染工具栏按钮
    fn render_toolbar_button(
        &self,
        panel: SidebarPanel,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let is_active = self.tool_dock.is_tool_open(panel);
        let accent_color = self.colors.accent;
        let accent_fg = self.colors.accent_foreground;
        let muted_fg = self.colors.muted_foreground;
        let muted_bg = self.colors.muted;

        div()
            .id(SharedString::from(format!("toolbar-btn-{:?}", panel)))
            .w(px(36.0))
            .h(px(36.0))
            .flex()
            .items_center()
            .justify_center()
            .rounded_md()
            .cursor_pointer()
            .when(is_active, |this| this.bg(accent_color))
            .when(!is_active, |this| this.hover(|s| s.bg(muted_bg)))
            .on_click(cx.listener(move |this, _event, _window, cx| {
                this.toggle_panel(panel, cx);
            }))
            .child(
                Icon::new(panel.icon())
                    .with_size(Size::Large)
                    .text_color(if is_active { accent_fg } else { muted_fg }),
            )
    }

    /// 渲染工具栏
    pub fn render_toolbar(&self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let border_color = self.colors.border;
        let muted_bg = self.colors.background;

        v_flex()
            .flex_shrink_0()
            .w(TOOLBAR_WIDTH)
            .h_full()
            .bg(muted_bg)
            .border_l_1()
            .border_color(border_color)
            .justify_between()
            .child(
                v_flex().items_center().py_2().gap_1().children(
                    self.tool_dock
                        .panels
                        .iter()
                        .copied()
                        .map(|panel| self.render_toolbar_button(panel, window, cx)),
                ),
            )
            .into_any_element()
    }

    /// 渲染面板内容
    pub fn render_panel_content(
        &self,
        panel: SidebarPanel,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let Some(view) = self.panel_view(panel) else {
            return div().into_any_element();
        };
        let needs_embedded_header = matches!(
            panel,
            SidebarPanel::Settings
                | SidebarPanel::QuickCommand
                | SidebarPanel::HistoryCommand
                | SidebarPanel::BroadcastInput
                | SidebarPanel::ServerMonitor
        );
        if !needs_embedded_header {
            return view.into_any_element();
        }

        v_flex()
            .size_full()
            .child(self.render_embedded_panel_header(panel, cx))
            .child(div().flex_1().min_h_0().overflow_hidden().child(view))
            .into_any_element()
    }

    fn render_embedded_panel_header(
        &self,
        panel: SidebarPanel,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let border = self.colors.border;
        let header_bg = self.colors.muted;
        let text = self.colors.foreground;
        let title: SharedString = match panel {
            SidebarPanel::Settings => t!("Settings.title").to_string().into(),
            _ => panel.title().into(),
        };

        h_flex()
            .flex_shrink_0()
            .w_full()
            .h(px(40.0))
            .px_3()
            .items_center()
            .justify_between()
            .border_b_1()
            .border_color(border)
            .bg(header_bg)
            .child(
                h_flex()
                    .gap_2()
                    .items_center()
                    .child(
                        Icon::new(panel.icon_name())
                            .with_size(Size::Small)
                            .text_color(text),
                    )
                    .child(
                        div()
                            .text_sm()
                            .font_weight(gpui::FontWeight::MEDIUM)
                            .text_color(text)
                            .child(title),
                    ),
            )
            .child(
                Button::new(SharedString::from(format!(
                    "close-terminal-sidebar-panel-{}",
                    panel.local_id()
                )))
                .icon(IconName::Close)
                .ghost()
                .xsmall()
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.close_tool(panel, cx);
                })),
            )
            .into_any_element()
    }
}

#[derive(Clone)]
struct TerminalToolbarSnapshot {
    colors: TerminalColors,
    buttons: Vec<TerminalToolbarButtonSnapshot>,
}

#[derive(Clone, Copy)]
struct TerminalToolbarButtonSnapshot {
    panel: SidebarPanel,
    open: bool,
}

pub(crate) struct TerminalSidebarToolbar {
    sidebar: Entity<TerminalSidebar>,
}

impl TerminalSidebarToolbar {
    pub(crate) fn new(sidebar: Entity<TerminalSidebar>) -> Self {
        Self { sidebar }
    }

    fn render_button(
        &self,
        button: TerminalToolbarButtonSnapshot,
        colors: TerminalColors,
    ) -> impl IntoElement {
        let sidebar = self.sidebar.clone();
        let panel = button.panel;
        let accent_color = colors.accent;
        let accent_fg = colors.accent_foreground;
        let muted_fg = colors.muted_foreground;
        let muted_bg = colors.muted;
        div()
            .id(SharedString::from(format!("toolbar-btn-{:?}", panel)))
            .w(px(36.0))
            .h(px(36.0))
            .flex()
            .items_center()
            .justify_center()
            .rounded_md()
            .cursor_pointer()
            .when(button.open, |this| this.bg(accent_color))
            .when(!button.open, |this| this.hover(move |s| s.bg(muted_bg)))
            .on_click(move |_, _window, cx| {
                sidebar.update(cx, |sidebar, cx| {
                    sidebar.toggle_panel(panel, cx);
                });
            })
            .child(
                Icon::new(panel.icon())
                    .with_size(Size::Large)
                    .text_color(if button.open { accent_fg } else { muted_fg }),
            )
    }
}

impl Render for TerminalSidebarToolbar {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let snapshot = self.sidebar.read(cx).toolbar_snapshot();

        v_flex()
            .flex_shrink_0()
            .w(TOOLBAR_WIDTH)
            .h_full()
            .bg(snapshot.colors.background)
            .border_l_1()
            .border_color(snapshot.colors.border)
            .child(
                v_flex().flex_1().items_center().py_2().gap_1().children(
                    snapshot
                        .buttons
                        .iter()
                        .copied()
                        .map(|button| self.render_button(button, snapshot.colors.clone())),
                ),
            )
    }
}

pub(crate) struct TerminalSidebarToolPanel {
    sidebar: Entity<TerminalSidebar>,
    panel: SidebarPanel,
}

impl TerminalSidebarToolPanel {
    pub(crate) fn new(sidebar: Entity<TerminalSidebar>, panel: SidebarPanel) -> Self {
        Self { sidebar, panel }
    }
}

impl Render for TerminalSidebarToolPanel {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let panel_view = self.sidebar.read(cx).panel_view(self.panel);

        div()
            .size_full()
            .overflow_hidden()
            .when_some(panel_view, |this, view| this.child(view))
    }
}

impl EventEmitter<TerminalSidebarEvent> for TerminalSidebar {}

impl Focusable for TerminalSidebar {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for TerminalSidebar {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let bg_color = self.colors.background;
        let active_panel = self.active_panel();

        h_flex()
            .h_full()
            .flex_shrink_0()
            .bg(bg_color)
            .when(active_panel.is_some(), |this| this.w_full())
            .when(active_panel.is_none(), |this| this.w(TOOLBAR_WIDTH))
            .when_some(active_panel, |this, panel| {
                this.child(
                    v_flex()
                        .flex_1()
                        .h_full()
                        .min_w_0()
                        .overflow_hidden()
                        .bg(bg_color)
                        .child(self.render_panel_content(panel, window, cx)),
                )
            })
            .child(self.render_toolbar(window, cx))
    }
}

#[cfg(test)]
mod tests {
    use super::{
        SidebarPanel, TerminalToolDockState, agent_theme_from_terminal_theme,
        build_live_terminal_ai_context, build_terminal_ai_context, terminal_ai_system_instruction,
        terminal_sidebar_available_panels,
    };
    use crate::theme::TerminalTheme;
    use agent_runtime::{ResourceCapability, ResourceKind, ResourceRef};
    use one_core::sidebar_contribution::SidebarPlacement;
    use one_core::storage::{ConnectionType, StoredConnection};
    use terminal::terminal::TerminalConnectionKind;

    fn stored_connection(id: i64, name: &str, connection_type: ConnectionType) -> StoredConnection {
        StoredConnection {
            id: Some(id),
            name: name.to_string(),
            connection_type,
            params: "{}".to_string(),
            workspace_id: None,
            selected_databases: None,
            remark: None,
            sync_enabled: true,
            cloud_id: None,
            last_synced_at: None,
            last_used_at: None,
            sort_order: None,
            created_at: None,
            updated_at: None,
            team_id: None,
            owner_id: None,
        }
    }

    #[test]
    fn local_terminal_ai_context_uses_the_live_session_as_current_target() {
        let live_terminal =
            ResourceRef::new("local-terminal-1", ResourceKind::Terminal, "local terminal")
                .with_capability(ResourceCapability::TerminalExec)
                .with_capability(ResourceCapability::TerminalControl);
        let saved = stored_connection(42, "prod", ConnectionType::SshSftp);

        let (scope, catalog, mentions) =
            build_live_terminal_ai_context(live_terminal.clone(), &[saved]);
        let resources = scope.to_resource_context();

        assert_eq!(
            Some(live_terminal.id.clone()),
            resources.current,
            "the local visible terminal must be the AI default target"
        );
        assert_eq!(Some(&live_terminal), resources.current());
        assert!(catalog.resources.iter().any(|resource| {
            resource.id == live_terminal.id
                && resource
                    .capabilities
                    .contains(&ResourceCapability::TerminalExec)
        }));
        assert!(
            mentions
                .iter()
                .any(|mention| mention.id == "local-terminal-1")
        );
    }

    #[test]
    fn local_terminal_ai_instruction_uses_the_host_platform() {
        let instruction = terminal_ai_system_instruction(TerminalConnectionKind::Local);

        assert!(instruction.contains(std::env::consts::OS));
        assert!(instruction.contains("不要默认假定为 Linux"));
    }

    #[test]
    fn tool_dock_keeps_toolbar_visible_when_no_panel_is_open() {
        let dock = TerminalToolDockState::new([SidebarPanel::Settings, SidebarPanel::AiChat]);

        assert!(dock.toolbar_visible());
        assert!(dock.open_panels().is_empty());
    }

    #[test]
    fn terminal_toolbar_only_renders_top_level_tools() {
        let source = include_str!("mod.rs");
        let toolbar_render = source
            .split_once("impl Render for TerminalSidebarToolbar")
            .expect("terminal toolbar render implementation")
            .1
            .split_once("pub(crate) struct TerminalSidebarToolPanel")
            .expect("terminal toolbar render boundary")
            .0;

        assert!(!toolbar_render.contains("quick_command_group"));
        assert!(!toolbar_render.contains("render_group_button"));
    }

    #[test]
    fn sidebar_default_panels_do_not_include_rich_input() {
        assert_eq!(7, SidebarPanel::all().len());
        assert!(
            SidebarPanel::all()
                .iter()
                .all(|panel| panel.local_id() != "terminal.rich-input")
        );
    }

    #[test]
    fn history_command_panel_is_available_for_local_terminals() {
        let panels = terminal_sidebar_available_panels(false, false, true, false);

        assert!(panels.contains(&SidebarPanel::HistoryCommand));
        assert!(!panels.contains(&SidebarPanel::BroadcastInput));
        assert!(!panels.contains(&SidebarPanel::FileManager));
        assert!(!panels.contains(&SidebarPanel::ServerMonitor));
    }

    #[test]
    fn history_command_panel_is_available_for_ssh_terminals() {
        let panels = terminal_sidebar_available_panels(true, true, true, true);

        assert!(panels.contains(&SidebarPanel::HistoryCommand));
        assert!(panels.contains(&SidebarPanel::BroadcastInput));
        assert!(panels.contains(&SidebarPanel::FileManager));
        assert!(panels.contains(&SidebarPanel::ServerMonitor));
    }

    #[test]
    fn history_command_panel_is_not_available_for_serial_terminals() {
        let panels = terminal_sidebar_available_panels(false, false, false, false);

        assert!(!panels.contains(&SidebarPanel::HistoryCommand));
        assert!(!panels.contains(&SidebarPanel::BroadcastInput));
        assert!(!panels.contains(&SidebarPanel::FileManager));
        assert!(!panels.contains(&SidebarPanel::ServerMonitor));
    }

    #[test]
    fn file_manager_uses_its_own_header_in_internal_tool_frame() {
        assert!(!SidebarPanel::FileManager.needs_internal_tool_frame_header());
        assert!(SidebarPanel::Settings.needs_internal_tool_frame_header());
        assert!(!SidebarPanel::AiChat.needs_internal_tool_frame_header());
        assert!(SidebarPanel::ServerMonitor.needs_internal_tool_frame_header());
    }

    #[test]
    fn agent_theme_preserves_terminal_dark_mode_for_markdown() {
        let terminal_theme = TerminalTheme::matrix();
        let agent_theme = agent_theme_from_terminal_theme(&terminal_theme);
        let markdown_style = agent_theme.markdown_style();

        assert!(terminal_theme.is_dark());
        assert!(agent_theme.is_dark);
        assert!(markdown_style.is_dark);
        assert_eq!(
            Some(agent_theme.code_background),
            markdown_style.code_background
        );
        assert_eq!(Some(agent_theme.table_header), markdown_style.table_header);
        assert_eq!(Some(agent_theme.quote_border), markdown_style.quote_border);
    }

    #[test]
    fn terminal_ai_context_keeps_current_connection_default_and_mentions_all_connections() {
        let current = stored_connection(2, "current-terminal", ConnectionType::SshSftp);
        let connections = vec![
            stored_connection(1, "app-db", ConnectionType::Database),
            current.clone(),
            stored_connection(3, "cache", ConnectionType::Redis),
        ];

        let (scope, catalog, mentions) = build_terminal_ai_context(&current, &connections);

        assert_eq!(1, scope.selected.len());
        assert_eq!(
            Some("current-terminal"),
            scope
                .to_resource_context()
                .current()
                .map(|resource| resource.label.as_str())
        );
        assert_eq!(
            vec!["current-terminal", "app-db", "cache"],
            mentions
                .iter()
                .map(|mention| mention.label.as_str())
                .collect::<Vec<_>>()
        );
        assert_eq!(
            vec!["current-terminal", "app-db", "cache"],
            catalog
                .resources
                .iter()
                .map(|resource| resource.label.as_str())
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn tool_dock_can_keep_multiple_tools_open_at_different_edges() {
        let mut dock = TerminalToolDockState::new([SidebarPanel::Settings, SidebarPanel::AiChat]);

        dock.open_tool(SidebarPanel::Settings);
        dock.move_tool(SidebarPanel::Settings, SidebarPlacement::Left);
        dock.open_tool(SidebarPanel::AiChat);
        dock.move_tool(SidebarPanel::AiChat, SidebarPlacement::Bottom);

        assert_eq!(
            dock.open_panels(),
            vec![
                (SidebarPanel::Settings, SidebarPlacement::Left),
                (SidebarPanel::AiChat, SidebarPlacement::Bottom),
            ],
        );
        assert!(dock.toolbar_visible());
    }

    #[test]
    fn tool_dock_opening_tool_closes_existing_tool_at_same_edge() {
        let mut dock = TerminalToolDockState::new([SidebarPanel::Settings, SidebarPanel::AiChat]);

        dock.open_tool(SidebarPanel::Settings);
        dock.open_tool(SidebarPanel::AiChat);

        assert_eq!(
            dock.open_panels(),
            vec![(SidebarPanel::AiChat, SidebarPlacement::Right)],
        );
        assert!(dock.toolbar_visible());
    }

    #[test]
    fn tool_dock_moving_tool_to_occupied_edge_closes_existing_tool() {
        let mut dock = TerminalToolDockState::new([SidebarPanel::Settings, SidebarPanel::AiChat]);

        dock.open_tool(SidebarPanel::Settings);
        dock.move_tool(SidebarPanel::Settings, SidebarPlacement::Left);
        dock.open_tool(SidebarPanel::AiChat);
        dock.move_tool(SidebarPanel::Settings, SidebarPlacement::Right);

        assert_eq!(
            dock.open_panels(),
            vec![(SidebarPanel::Settings, SidebarPlacement::Right)],
        );
        assert!(dock.toolbar_visible());
    }

    #[test]
    fn tool_dock_moving_open_panel_keeps_it_open() {
        let mut dock = TerminalToolDockState::new([SidebarPanel::Settings, SidebarPanel::AiChat]);

        dock.open_tool(SidebarPanel::AiChat);
        assert!(dock.move_tool(SidebarPanel::AiChat, SidebarPlacement::Bottom));

        assert_eq!(
            dock.open_panels(),
            vec![(SidebarPanel::AiChat, SidebarPlacement::Bottom)],
        );
        assert!(dock.toolbar_visible());
    }

    #[test]
    fn tool_dock_closes_one_panel_without_hiding_toolbar_or_other_panels() {
        let mut dock = TerminalToolDockState::new([SidebarPanel::Settings, SidebarPanel::AiChat]);

        dock.open_tool(SidebarPanel::Settings);
        dock.open_tool(SidebarPanel::AiChat);
        dock.close_tool(SidebarPanel::Settings);

        assert_eq!(
            dock.open_panels(),
            vec![(SidebarPanel::AiChat, SidebarPlacement::Right)],
        );
        assert!(dock.toolbar_visible());
    }
}
