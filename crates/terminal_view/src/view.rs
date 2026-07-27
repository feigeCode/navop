use alacritty_terminal::grid::Dimensions;
use alacritty_terminal::index::{Column, Line, Point as AlacPoint, Side};
use alacritty_terminal::selection::SelectionType;
use alacritty_terminal::term::TermMode;
use alacritty_terminal::term::cell::Flags;
use gpui::prelude::FluentBuilder;
use gpui::*;
use gpui_component::button::{Button, ButtonVariants};
use gpui_component::dialog::DialogButtonProps;
use gpui_component::input::{Input, InputState};
use gpui_component::menu::{ContextMenuExt, PopupMenu, PopupMenuItem};
use gpui_component::notification::Notification;
use gpui_component::scroll::{ScrollableElement, Scrollbar, ScrollbarHandle, ScrollbarShow};
use gpui_component::slider::{Slider, SliderEvent, SliderState, SliderValue};
use gpui_component::{
    ActiveTheme, BlinkCursor, Disableable, Icon, IconName, Selectable, Sizable, WindowExt, h_flex,
    kbd::Kbd, v_flex,
};
use one_core::gpui_tokio::Tokio;
use one_core::keybindings::{
    action_id, keystroke_matches_shortcuts, rebind_keybindings, shortcuts_for,
};
use one_core::settings::{AppSettings, resolve_installed_grid_monospace_font_family};
use std::borrow::Cow;
use std::cell::{Cell as StdCell, RefCell};
use std::collections::HashMap;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::{Arc, Mutex as StdMutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

pub(crate) mod block_selection;
mod history_prompt_rules;
mod mouse_input;
mod paste_safety;
mod workspace_support;

use workspace_support::TerminalRenderMode;
pub(crate) use workspace_support::{TerminalPaneEvent, TerminalWorkspaceSidebarSnapshot};

use crate::addon::{
    AddonManager, CustomHighlightAddon, SearchAddon, TerminalAddonFrameContext,
    TerminalAddonMouseContext, register_default_addons,
};
use crate::broadcast_input::BroadcastClientId;
use crate::broadcast_registry::{broadcast_input_registry, init_broadcast_input_registry};
use crate::cd_completion::{
    CdCompletionQuery, build_cd_completion_suggestions, parse_cd_completion_query,
};
use crate::history_prompt::{HistoryPromptAccept, HistoryPromptMode, HistoryPromptState};
use crate::public_mcp::TerminalPublicMcpRegistration;
use crate::settings::{
    GlobalTerminalLocalSettings, TerminalHighlightRule, TerminalSettings, TerminalSettingsEvent,
    current_settings, update_settings,
};
use crate::sidebar::tool_dock::{
    TerminalToolDockLayout, render_internal_tool_panel_frame, right_tool_region_width,
};
use crate::sidebar::{
    LocalWorkspaceSidebar, SidebarPanel, TerminalSidebar, TerminalSidebarEvent,
    TerminalSidebarToolPanel, TerminalSidebarToolbar,
};
use crate::terminal_element::{RenderCache, TerminalElement};
use crate::theme::{
    DEFAULT_LINE_HEIGHT_SCALE, MAX_FONT_SIZE, MIN_FONT_SIZE, TerminalTheme, default_font_fallbacks,
    default_monospace_font, normalize_terminal_primary_font, terminal_cell_width_from_advances,
};
use crate::view::block_selection::{
    BlockSelection, block_selection_text_from_rows, should_start_block_selection,
};
#[cfg(test)]
use history_prompt_rules::should_refresh_history_commands_for_terminal_event;
use history_prompt_rules::{
    HISTORY_PROMPT_DROPDOWN_MAX_WIDTH, HISTORY_PROMPT_DROPDOWN_MIN_WIDTH,
    history_prompt_active_background, history_prompt_available, history_prompt_dropdown_background,
    history_prompt_dropdown_origin, history_prompt_overlay_bounds,
    should_confirm_local_terminal_close, should_dismiss_history_prompt_for_keystroke,
    should_dismiss_history_prompt_for_mouse, should_dismiss_history_prompt_for_scroll,
    should_reset_history_prompt_for_terminal_event, terminal_history_scope,
};
use mouse_input::{
    encode_mouse_modifiers, mouse_button_code, sgr_mouse_button_report, sgr_mouse_mode_enabled,
    sgr_mouse_wheel_report, should_defer_inline_history_prompt_input_to_text_system,
    should_defer_sgr_left_press, should_extend_selection_on_shift_click,
    should_scroll_to_bottom_on_user_input, should_start_selection_from_pending_sgr_press,
    take_whole_scroll_lines,
};
use one_core::layout::{SIDEBAR_MAX_WIDTH, SIDEBAR_MIN_WIDTH, TOOLBAR_WIDTH};
use one_core::sidebar_contribution::{SidebarContribution, SidebarPlacement};
use one_core::storage::models::{ActiveConnections, StoredConnection};
use one_core::storage::{ConnectionRepository, GlobalStorageState, traits::Repository};
use one_core::tab_container::{TabContent, TabContentEvent, TabContentView};
use one_ui::resize_handle::{HandlePlacement, ResizePanel, resize_handle};
use paste_safety::{
    UnbracketedPasteHazard, detect_unbracketed_paste_hazard, multiline_non_empty_line_count,
};
#[cfg(test)]
use paste_safety::{has_trailing_line_continuation, has_unterminated_shell_quote};
use remote_image_preview::image_from_local_path;
use rust_i18n::t;
use sftp::{RusshSftpClient, SftpClient};
use std::ops::Deref;
use terminal::LocalConfig;
use terminal::terminal::{
    ConnectionState, SshConnectionUpdate, Terminal, TerminalConnectionKind, TerminalModelEvent,
    TerminalScrollProxy, resolve_local_working_dir,
};
use tokio::sync::Mutex;
use workspace_explorer::{WorkspaceEditor, WorkspaceEditorEvent};

mod actions;
mod appearance;
mod clipboard;
mod clipboard_image;
mod close;
mod command_bar;
mod command_bar_events;
mod command_bar_model;
mod connection_overlay;
mod constructors;
mod helpers;
mod history_actions;
mod history_query;
mod history_render;
mod init_config;
mod initialization;
mod input_handler;
mod keybindings;
mod mouse_down;
mod mouse_selection;
mod paste_confirmation;
mod preferences;
mod recording_footer;
mod recording_playback_config;
mod recording_playback_controls;
mod recording_playback_footer;
mod recording_playback_render;
mod registrations;
mod render;
mod render_layout;
mod render_surface;
mod resize_event_handler;
mod scroll;
mod selection_search;
mod sidebar_events;
mod state;
mod tab_content;
mod terminal_events;
mod terminal_layout;
mod terminal_render;
mod text_input;
mod tool_dock;
mod vi_input;

use actions::*;
use command_bar::{TerminalCommandBar, TerminalCommandBarConfig, TerminalCommandBarEvent};
use helpers::*;
use init_config::TerminalViewInit;
use keybindings::{
    TERMINAL_CLEAR_SCREEN_SHORTCUT, TERMINAL_CONTEXT, TERMINAL_COPY_SHORTCUT,
    TERMINAL_PASTE_SHORTCUT, TERMINAL_SELECT_ALL_SHORTCUT, TERMINAL_TOGGLE_VI_MODE_SHORTCUT,
    terminal_paste_defaults, terminal_shortcut_label,
};
pub use keybindings::{init, refresh_keybindings};
pub use recording_playback_config::RecordingPlaybackViewConfig;
use resize_event_handler::ResizeEventHandler;
pub(crate) use state::TerminalDuplicateSource;
use state::*;
use tab_content::{recording_playback_display_name, recording_playback_tab_title};

pub(crate) const TERMINAL_TOOLS_SIDEBAR_DEFAULT_WIDTH: Pixels = px(400.0);

/// Terminal view component - supports both Local and SSH backends.
pub struct TerminalView {
    /// Terminal model entity
    terminal: Entity<Terminal>,
    duplicate_source: Option<TerminalDuplicateSource>,
    recording_playback_name: Option<SharedString>,
    /// 本地终端工作目录
    local_working_dir: Option<PathBuf>,
    /// 光标闪烁管理器
    blink_manager: Entity<BlinkCursor>,
    /// 侧边栏
    sidebar: Entity<TerminalSidebar>,
    /// 本地工作区文件编辑器（仅本地终端）
    workspace_editor: Option<Entity<WorkspaceEditor>>,
    /// 终端底部命令输入栏
    command_bar: Entity<TerminalCommandBar>,
    sidebar_toolbar: Entity<TerminalSidebarToolbar>,
    sidebar_tool_panels: HashMap<SidebarPanel, Entity<TerminalSidebarToolPanel>>,

    font_size: Pixels,
    line_height: Pixels,
    font_family: SharedString,
    font_fallbacks: Vec<SharedString>,
    line_height_scale: f32,
    cell_width: Pixels,
    font_metrics: Option<TerminalFontMetrics>,

    last_size: Option<(usize, usize)>,
    /// 上一帧 alacritty 是否处于 alt screen 模式。
    ///
    /// 用于检测主屏与备用屏切换:进入 alt screen 时主动调用 nudge_resize
    /// 重发当前尺寸给 PTY,触发 SIGWINCH,让 TUI 应用刷新整屏画面,
    /// 避免出现底部残留上一次渲染内容的问题。
    last_alt_screen: bool,
    scroll_lines_accumulated: f32,

    mouse_state: MouseState,
    block_selection: Option<BlockSelection>,
    addon_manager: AddonManager,

    _subscriptions: Vec<Subscription>,

    mouse_position: Option<Point<Pixels>>,

    render_cache: RenderCache,
    focus_handle: FocusHandle,

    terminal_bounds: Bounds<Pixels>,

    ime_state: Option<ImeState>,
    history_prompt: HistoryPromptState,
    /// shell prompt 当前是否处于可输入阶段，由 OSC 133 生命周期维护。
    shell_prompt_input_active: bool,
    /// 本地 shell 命令是否处于执行阶段，由 OSC 133;C 到下一次 prompt/input 维护。
    local_command_running: bool,
    /// InlineSuggest 防抖任务（30ms 延迟刷新建议）
    suggestion_debounce: Option<Task<()>>,
    /// 当前 pane 是否正在等待用户选择录制文件保存目录。
    recording_path_prompt_pending: bool,
    /// 当前 pane 最近一次录制控制错误；在 Footer 中直接展示。
    recording_control_error: Option<String>,
    /// 仅在录制时间持续增长时存在，用于每秒刷新 Footer。
    recording_ticker: Option<Task<()>>,
    /// 当前 pane 独占的 Playback seek 控件状态。
    recording_playback_slider: Entity<SliderState>,
    /// 用户拖动期间只预览位置，Release 后才重建 Playback grid。
    recording_playback_slider_dragging: bool,
    /// 最近一次 Playback 控制错误；成功控制后清除。
    recording_playback_control_error: Option<String>,
    /// 仅在 Playback 正在播放时存在，drop 即取消。
    recording_playback_ticker: Option<Task<()>>,
    /// `cd` 目录补全的独立 SFTP 连接
    cd_completion_client: Option<Arc<Mutex<RusshSftpClient>>>,
    /// 按父目录缓存远端子目录名，减少重复 SFTP 请求
    cd_completion_cache: HashMap<String, Vec<String>>,
    /// 当前正在加载目录候选的父目录
    cd_completion_loading_parent: Option<String>,
    ssh_mfa_inputs: Vec<SshMfaInput>,
    focus_terminal_after_connect: bool,

    current_theme: TerminalTheme,

    /// 标签页序号（用于多实例显示）
    tab_index: Option<usize>,

    /// 是否启用光标闪烁
    cursor_blink_enabled: bool,
    /// 非 bracketed 模式下，多行粘贴是否弹确认
    confirm_multiline_paste: bool,
    /// 高危命令是否弹确认
    confirm_high_risk_command: bool,
    /// 选中自动复制
    auto_copy_on_select: bool,
    /// 是否启用历史自动补全
    autocomplete_enabled: bool,
    /// 中键粘贴
    middle_click_paste: bool,
    /// 右键快速粘贴
    right_click_paste: bool,
    /// SSH 粘贴图片上传
    paste_image_upload: bool,
    /// 在 vim/less/man 等 alt-screen TUI 中,把鼠标滚轮转为方向键发送到 PTY
    vim_scroll_to_arrow_keys: bool,
    broadcast_client_id: Option<BroadcastClientId>,

    /// 侧边栏面板大小
    sidebar_panel_size: Pixels,
    /// 正在调整大小的面板
    resizing: Option<ResizingPanel>,
    /// 视图边界
    view_bounds: Bounds<Pixels>,

    scrollbar_metrics: Rc<RefCell<TerminalScrollbarMetrics>>,
    scrollbar_handle: TerminalScrollbarHandle,
    public_mcp_registration: Option<TerminalPublicMcpRegistration>,
    render_mode: TerminalRenderMode,
}

#[cfg(test)]
mod tests;
