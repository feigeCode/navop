//! 快捷命令面板
//!
//! 支持命令的新增、编辑、分组、置顶和删除功能。

use gpui::prelude::*;
use gpui::{
    App, AppContext, ClipboardItem, ColorExt as _, Context, Entity, EventEmitter, FocusHandle,
    Focusable, InteractiveElement, IntoElement, Keystroke, ListSizingBehavior, MouseButton,
    ParentElement, Render, SharedString, Styled, UniformListScrollHandle, Window, div,
    uniform_list,
};
use gpui_component::{
    ActiveTheme, Icon, IconName, Sizable, Size, WindowExt,
    button::{Button, ButtonCustomVariant, ButtonVariant},
    checkbox::Checkbox,
    dialog::DialogButtonProps,
    h_flex,
    input::{Input, InputEvent, InputState, Textarea, TextareaState},
    menu::{ContextMenuExt, PopupMenu, PopupMenuItem},
    notification::Notification,
    radio::{Radio, RadioGroup},
    select::{Select, SelectItem, SelectState},
    tooltip::Tooltip,
    v_flex,
};
use one_core::keybindings::shortcut_spec_from_keystroke;
use one_core::storage::{
    GlobalStorageState, QuickCommand, QuickCommandRepository, traits::Repository,
};
use one_ui::{IconButton, IconButtonRole};
use palette::IntoColor;
use rust_i18n::t;
use std::{ops::Range, sync::Arc};

use crate::quick_command_sync::emit_quick_commands_changed;
use crate::theme::TerminalColors;
use crate::view::quick_command_executes_on_click;

/// 快捷命令面板事件
#[derive(Clone, Debug)]
pub enum QuickCommandPanelEvent {
    /// 关闭面板
    Close,
    /// 粘贴命令到终端输入区（不自动回车）
    ExecuteCommand(String),
    /// 快捷命令数据已变更
    QuickCommandsChanged,
}

fn notify_quick_commands_changed(cx: &mut Context<QuickCommandPanel>) {
    cx.emit(QuickCommandPanelEvent::QuickCommandsChanged);
    emit_quick_commands_changed(cx);
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum QuickCommandGroupFilter {
    All,
    Ungrouped,
    Group(String),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum QuickCommandScope {
    CurrentConnection,
    Global,
}

fn connection_id_for_scope(
    panel_connection_id: Option<i64>,
    scope: QuickCommandScope,
) -> Option<i64> {
    match (panel_connection_id, scope) {
        (Some(connection_id), QuickCommandScope::CurrentConnection) => Some(connection_id),
        _ => None,
    }
}

fn normalize_quick_command_value(value: &str) -> String {
    let command = value.trim();
    if command.is_empty() {
        return String::new();
    }
    if value.ends_with(['\r', '\n']) {
        format!("{command}\n")
    } else {
        command.to_string()
    }
}

/// 按「点击执行」开关维护命令末尾的换行标记：
/// 勾选时保证末尾有换行（触发即执行），取消时去掉末尾换行（仅粘贴）。
fn apply_auto_run_marker(command: &str, auto_run: bool) -> String {
    if auto_run {
        if command.ends_with(['\r', '\n']) {
            command.to_string()
        } else {
            format!("{command}\n")
        }
    } else {
        command.trim_end_matches(['\r', '\n']).to_string()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum ShortcutCapture {
    Clear,
    Invalid,
    Shortcut(String),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ShortcutCaptureLabel<'a> {
    PressShortcut,
    Unassigned,
    Shortcut(&'a str),
}

fn shortcut_capture_label(shortcut: Option<&str>, is_focused: bool) -> ShortcutCaptureLabel<'_> {
    match shortcut {
        Some(shortcut) => ShortcutCaptureLabel::Shortcut(shortcut),
        None if is_focused => ShortcutCaptureLabel::PressShortcut,
        None => ShortcutCaptureLabel::Unassigned,
    }
}

fn capture_quick_command_shortcut(keystroke: &Keystroke) -> ShortcutCapture {
    if keystroke.key == "escape" {
        return ShortcutCapture::Clear;
    }
    if !keystroke.modifiers.control
        && !keystroke.modifiers.alt
        && !keystroke.modifiers.shift
        && !keystroke.modifiers.platform
    {
        return ShortcutCapture::Invalid;
    }

    shortcut_spec_from_keystroke(keystroke)
        .map(ShortcutCapture::Shortcut)
        .unwrap_or(ShortcutCapture::Invalid)
}

fn validated_quick_command_shortcut(
    shortcut: Option<String>,
    invalid_shortcut: bool,
) -> Result<Option<String>, ()> {
    if invalid_shortcut {
        return Err(());
    }

    shortcut
        .map(|shortcut| {
            let keystroke = Keystroke::parse(&shortcut).map_err(|_| ())?;
            match capture_quick_command_shortcut(&keystroke) {
                ShortcutCapture::Shortcut(shortcut) => Ok(shortcut),
                ShortcutCapture::Clear | ShortcutCapture::Invalid => Err(()),
            }
        })
        .transpose()
}

fn command_matches_group_filter(command: &QuickCommand, filter: &QuickCommandGroupFilter) -> bool {
    match filter {
        QuickCommandGroupFilter::All => true,
        QuickCommandGroupFilter::Ungrouped => command
            .group_name
            .as_ref()
            .map(|group| group.trim().is_empty())
            .unwrap_or(true),
        QuickCommandGroupFilter::Group(group) => command
            .group_name
            .as_deref()
            .map(|name| name.trim() == group)
            .unwrap_or(false),
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct QuickCommandGroupChip {
    filter: QuickCommandGroupFilter,
    color: Option<String>,
}

fn quick_command_groups(commands: &[QuickCommand]) -> Vec<QuickCommandGroupChip> {
    let mut grouped = std::collections::BTreeMap::<String, Option<String>>::new();
    for command in commands {
        let Some(name) = command.group_name.as_ref() else {
            continue;
        };
        let trimmed = name.trim();
        if trimmed.is_empty() {
            continue;
        }
        let entry = grouped.entry(trimmed.to_string()).or_insert(None);
        if entry.is_none() {
            *entry = command
                .group_color
                .clone()
                .filter(|color| !color.trim().is_empty());
        }
    }

    let mut groups = vec![
        QuickCommandGroupChip {
            filter: QuickCommandGroupFilter::All,
            color: None,
        },
        QuickCommandGroupChip {
            filter: QuickCommandGroupFilter::Ungrouped,
            color: None,
        },
    ];
    groups.extend(
        grouped
            .into_iter()
            .map(|(name, color)| QuickCommandGroupChip {
                filter: QuickCommandGroupFilter::Group(name),
                color,
            }),
    );
    groups
}

fn quick_command_group_chip_label(filter: &QuickCommandGroupFilter) -> String {
    match filter {
        QuickCommandGroupFilter::All => "全部".to_string(),
        QuickCommandGroupFilter::Ungrouped => "未分组".to_string(),
        QuickCommandGroupFilter::Group(name) => name.clone(),
    }
}

fn quick_command_group_color(color: Option<&str>, fallback: gpui::Hsla) -> gpui::Hsla {
    match color.unwrap_or_default() {
        "blue" => gpui::rgb(0x3b82f6).into_color(),
        "cyan" => gpui::rgb(0x06b6d4).into_color(),
        "green" => gpui::rgb(0x22c55e).into_color(),
        "yellow" => gpui::rgb(0xeab308).into_color(),
        "orange" => gpui::rgb(0xf97316).into_color(),
        "red" => gpui::rgb(0xef4444).into_color(),
        "pink" => gpui::rgb(0xec4899).into_color(),
        "purple" => gpui::rgb(0xa855f7).into_color(),
        "gray" => gpui::rgb(0x64748b).into_color(),
        _ => fallback,
    }
}

fn quick_command_group_color_choices() -> Vec<(&'static str, &'static str)> {
    vec![
        ("", "默认"),
        ("blue", "蓝"),
        ("cyan", "青"),
        ("green", "绿"),
        ("yellow", "黄"),
        ("orange", "橙"),
        ("red", "红"),
        ("pink", "粉"),
        ("purple", "紫"),
        ("gray", "灰"),
    ]
}

fn new_command_group_defaults(
    filter: &QuickCommandGroupFilter,
    commands: &[QuickCommand],
) -> (String, String) {
    let QuickCommandGroupFilter::Group(selected_name) = filter else {
        return (String::new(), String::new());
    };
    let color = quick_command_groups(commands)
        .into_iter()
        .find_map(|chip| match chip.filter {
            QuickCommandGroupFilter::Group(name) if name == *selected_name => chip.color,
            _ => None,
        })
        .unwrap_or_default();
    (selected_name.clone(), color)
}

#[derive(Clone, PartialEq)]
struct ColorSelectItem {
    value: String,
    label: SharedString,
}

impl SelectItem for ColorSelectItem {
    type Value = String;

    fn title(&self) -> SharedString {
        self.label.clone()
    }

    fn value(&self) -> &Self::Value {
        &self.value
    }
}

fn quick_command_group_color_items() -> Vec<ColorSelectItem> {
    [
        ("", "默认"),
        ("blue", "蓝"),
        ("cyan", "青"),
        ("green", "绿"),
        ("yellow", "黄"),
        ("orange", "橙"),
        ("red", "红"),
        ("pink", "粉"),
        ("purple", "紫"),
        ("gray", "灰"),
    ]
    .into_iter()
    .map(|(value, label)| ColorSelectItem {
        value: value.to_string(),
        label: SharedString::from(label),
    })
    .collect()
}

fn normalize_group_fields(
    group_name: String,
    group_color: Option<String>,
) -> (Option<String>, Option<String>) {
    let group_name = group_name.trim().to_string();
    if group_name.is_empty() {
        return (None, None);
    }
    let group_color = group_color
        .map(|color| color.trim().to_string())
        .filter(|color| !color.is_empty());
    (Some(group_name), group_color)
}

fn quick_command_dialog_button_variants(
    colors: &TerminalColors,
    cx: &App,
) -> (ButtonVariant, ButtonVariant) {
    let ok = ButtonCustomVariant::new(cx)
        .color(colors.accent)
        .foreground(colors.accent_foreground)
        .hover(colors.accent.opacity(0.88))
        .active(colors.accent.opacity(0.76));
    let cancel = ButtonCustomVariant::new(cx)
        .color(colors.muted)
        .foreground(colors.foreground)
        .hover(colors.border)
        .active(colors.border.opacity(0.82));
    (ButtonVariant::Custom(ok), ButtonVariant::Custom(cancel))
}

struct QuickCommandEditorState {
    shortcut: Option<String>,
    invalid_shortcut: bool,
    /// 点击触发后是否自动回车执行
    auto_run: bool,
    scope: QuickCommandScope,
    panel_connection_id: Option<i64>,
    shortcut_focus_handle: FocusHandle,
    colors: TerminalColors,
}

impl QuickCommandEditorState {
    fn new(
        shortcut: Option<String>,
        auto_run: bool,
        scope: QuickCommandScope,
        panel_connection_id: Option<i64>,
        colors: TerminalColors,
        cx: &mut Context<Self>,
    ) -> Self {
        Self {
            shortcut,
            invalid_shortcut: false,
            auto_run,
            scope,
            panel_connection_id,
            shortcut_focus_handle: cx.focus_handle(),
            colors,
        }
    }
}

impl Focusable for QuickCommandEditorState {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.shortcut_focus_handle.clone()
    }
}

impl Render for QuickCommandEditorState {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let capture_text = match shortcut_capture_label(
            self.shortcut.as_deref(),
            self.shortcut_focus_handle.is_focused(window),
        ) {
            ShortcutCaptureLabel::PressShortcut => t!("QuickCommand.press_shortcut").to_string(),
            ShortcutCaptureLabel::Unassigned => t!("QuickCommand.shortcut_unassigned").to_string(),
            ShortcutCaptureLabel::Shortcut(shortcut) => shortcut.to_string(),
        };
        let shortcut_focus_handle = self.shortcut_focus_handle.clone();
        let selected_scope = match self.scope {
            QuickCommandScope::CurrentConnection => 0,
            QuickCommandScope::Global => 1,
        };
        let editor = cx.entity().clone();
        let editor_auto_run = cx.entity().clone();

        v_flex()
            .gap_1()
            .child(
                h_flex()
                    .id("quick-command-shortcut-scope-row")
                    .items_start()
                    .gap_3()
                    .child(
                        v_flex()
                            .flex_1()
                            .gap_1()
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(self.colors.muted_foreground)
                                    .child(t!("QuickCommand.shortcut").to_string()),
                            )
                            .child(
                                div()
                                    .id("quick-command-shortcut-capture")
                                    .w_full()
                                    .px_3()
                                    .py_2()
                                    .rounded_md()
                                    .border_1()
                                    .border_color(if self.invalid_shortcut {
                                        gpui::red()
                                    } else if self.shortcut_focus_handle.is_focused(window) {
                                        self.colors.accent
                                    } else {
                                        self.colors.border
                                    })
                                    .bg(self.colors.muted)
                                    .text_sm()
                                    .text_color(self.colors.foreground)
                                    .cursor_pointer()
                                    .track_focus(&self.shortcut_focus_handle)
                                    .on_click(move |_, window, cx| {
                                        shortcut_focus_handle.focus(window, cx);
                                    })
                                    .on_key_down(cx.listener(
                                        |this, event: &gpui::KeyDownEvent, window, cx| {
                                            window.prevent_default();
                                            cx.stop_propagation();
                                            match capture_quick_command_shortcut(&event.keystroke) {
                                                ShortcutCapture::Clear => {
                                                    this.shortcut = None;
                                                    this.invalid_shortcut = false;
                                                }
                                                ShortcutCapture::Invalid => {
                                                    this.invalid_shortcut = true;
                                                }
                                                ShortcutCapture::Shortcut(shortcut) => {
                                                    this.shortcut = Some(shortcut);
                                                    this.invalid_shortcut = false;
                                                }
                                            }
                                            cx.notify();
                                        },
                                    ))
                                    .child(capture_text),
                            )
                            .when(self.invalid_shortcut, |this| {
                                this.child(
                                    div()
                                        .text_xs()
                                        .text_color(gpui::red())
                                        .child(t!("QuickCommand.invalid_shortcut").to_string()),
                                )
                            }),
                    )
                    .child(
                        v_flex()
                            .w(gpui::px(280.0))
                            .flex_shrink_0()
                            .gap_1()
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(self.colors.muted_foreground)
                                    .child(t!("QuickCommand.scope").to_string()),
                            )
                            .when_some(self.panel_connection_id, |this, _| {
                                this.child(
                                    RadioGroup::horizontal("quick-command-scope")
                                        .selected_index(Some(selected_scope))
                                        .on_click(move |index, _, cx| {
                                            editor.update(cx, |this, cx| {
                                                this.scope = if *index == 0 {
                                                    QuickCommandScope::CurrentConnection
                                                } else {
                                                    QuickCommandScope::Global
                                                };
                                                cx.notify();
                                            });
                                        })
                                        .children([
                                            Radio::new("quick-command-current-connection").label(
                                                t!("QuickCommand.current_connection").to_string(),
                                            ),
                                            Radio::new("quick-command-global").label(
                                                t!("QuickCommand.global_shared").to_string(),
                                            ),
                                        ]),
                                )
                            })
                            .when(self.panel_connection_id.is_none(), |this| {
                                this.child(
                                    div()
                                        .text_sm()
                                        .text_color(self.colors.foreground)
                                        .child(t!("QuickCommand.global_shared").to_string()),
                                )
                            })
                            .when(self.scope == QuickCommandScope::Global, |this| {
                                this.child(
                                    div()
                                        .text_xs()
                                        .text_color(self.colors.muted_foreground)
                                        .child(
                                            t!("QuickCommand.global_shared_description")
                                                .to_string(),
                                        ),
                                )
                            }),
                    ),
            )
            .child(
                h_flex()
                    .id("quick-command-auto-run-row")
                    .items_center()
                    .gap_2()
                    .child(
                        Checkbox::new("quick-command-auto-run")
                            .label(t!("QuickCommand.auto_run").to_string())
                            .checked(self.auto_run)
                            .on_click(move |_, _, cx| {
                                editor_auto_run.update(cx, |this, cx| {
                                    this.auto_run = !this.auto_run;
                                    cx.notify();
                                });
                            }),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(self.colors.muted_foreground)
                            .child(t!("QuickCommand.auto_run_hint").to_string()),
                    ),
            )
    }
}

/// 快捷命令面板组件
pub struct QuickCommandPanel {
    /// 搜索输入框状态
    search_input_state: Entity<InputState>,
    /// 快捷命令列表
    commands: Vec<QuickCommand>,
    /// 过滤后的非置顶命令列表
    filtered_commands: Vec<QuickCommand>,
    /// 连接 ID
    connection_id: Option<i64>,
    /// 焦点句柄
    focus_handle: FocusHandle,
    /// 订阅
    _subscriptions: Vec<gpui::Subscription>,
    /// 是否正在加载
    is_loading: bool,
    /// 搜索关键词
    search_query: String,
    /// 当前分组筛选
    group_filter: QuickCommandGroupFilter,
    /// 列表滚动句柄
    scroll_handle: UniformListScrollHandle,
    /// 终端主题配色
    colors: TerminalColors,
}

impl QuickCommandPanel {
    pub fn new(
        connection_id: Option<i64>,
        colors: TerminalColors,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let search_input_state = cx.new(|cx| {
            InputState::new(window, cx).placeholder(t!("QuickCommand.search").to_string())
        });
        let input_entity = search_input_state.clone();
        let subscriptions = vec![cx.subscribe_in(
            &search_input_state,
            window,
            move |this, _state, event, _window, cx| {
                if let InputEvent::Change = event {
                    this.search_query = input_entity.read(cx).value().to_string();
                    this.filter_commands();
                    cx.notify();
                }
            },
        )];

        let mut panel = Self {
            search_input_state,
            commands: Vec::new(),
            filtered_commands: Vec::new(),
            connection_id,
            focus_handle: cx.focus_handle(),
            _subscriptions: subscriptions,
            is_loading: false,
            search_query: String::new(),
            group_filter: QuickCommandGroupFilter::All,
            scroll_handle: UniformListScrollHandle::new(),
            colors,
        };
        panel.load_commands(cx);
        panel
    }

    pub fn set_colors(&mut self, colors: TerminalColors, cx: &mut Context<Self>) {
        self.colors = colors;
        cx.notify();
    }

    fn set_group_filter(&mut self, group_filter: QuickCommandGroupFilter, cx: &mut Context<Self>) {
        if self.group_filter == group_filter {
            return;
        }
        self.group_filter = group_filter;
        self.filter_commands();
        cx.notify();
    }

    fn sort_commands(&mut self) {
        self.commands.sort_by(|a, b| {
            b.pinned
                .cmp(&a.pinned)
                .then_with(|| a.sort_order.cmp(&b.sort_order))
                .then_with(|| b.created_at.cmp(&a.created_at))
        });
    }

    fn pinned_commands(&self) -> Vec<QuickCommand> {
        self.commands
            .iter()
            .filter(|command| command.pinned)
            .cloned()
            .collect()
    }

    /// 加载快捷命令
    pub fn load_commands(&mut self, cx: &mut Context<Self>) {
        self.is_loading = true;
        let storage = cx.global::<GlobalStorageState>().storage.clone();
        let Some(repo) = storage.get::<QuickCommandRepository>() else {
            tracing::error!("QuickCommandRepository not found");
            self.is_loading = false;
            cx.notify();
            return;
        };

        match repo.list_by_connection(self.connection_id) {
            Ok(commands) => {
                self.commands = commands;
                self.sort_commands();
                self.filter_commands();
            }
            Err(error) => tracing::error!(%error, "Failed to load commands"),
        }

        self.is_loading = false;
        cx.notify();
    }

    /// 从外部添加快捷命令（例如右键菜单）
    pub fn add_command_external(
        &mut self,
        command: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.open_command_editor(None, Some(command), window, cx);
    }

    fn save_new_command(
        &mut self,
        mut command: QuickCommand,
        cx: &mut Context<Self>,
    ) -> anyhow::Result<()> {
        let storage = cx.global::<GlobalStorageState>().storage.clone();
        let repo = storage
            .get::<QuickCommandRepository>()
            .ok_or_else(|| anyhow::anyhow!("QuickCommandRepository not found"))?;
        command.sort_order = repo.next_sort_order(command.connection_id).unwrap_or(0);
        repo.insert(&mut command)?;
        self.load_commands(cx);
        notify_quick_commands_changed(cx);
        Ok(())
    }

    fn save_existing_command(
        &mut self,
        command: QuickCommand,
        cx: &mut Context<Self>,
    ) -> anyhow::Result<()> {
        let storage = cx.global::<GlobalStorageState>().storage.clone();
        let repo = storage
            .get::<QuickCommandRepository>()
            .ok_or_else(|| anyhow::anyhow!("QuickCommandRepository not found"))?;
        repo.update(&command)?;
        self.load_commands(cx);
        notify_quick_commands_changed(cx);
        Ok(())
    }

    fn open_command_editor(
        &mut self,
        existing: Option<QuickCommand>,
        initial_command: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let initial_name = existing
            .as_ref()
            .and_then(|command| command.name.clone())
            .unwrap_or_default();
        let initial_description = existing
            .as_ref()
            .and_then(|command| command.description.clone())
            .unwrap_or_default();
        let (default_group_name, default_group_color) =
            new_command_group_defaults(&self.group_filter, &self.commands);
        let initial_group_name = existing
            .as_ref()
            .and_then(|command| command.group_name.clone())
            .unwrap_or(default_group_name);
        let initial_group_color = existing
            .as_ref()
            .and_then(|command| command.group_color.clone())
            .unwrap_or(default_group_color);
        let initial_command = existing
            .as_ref()
            .map(|command| command.command.clone())
            .or(initial_command)
            .unwrap_or_default();
        let initial_auto_run = quick_command_executes_on_click(&initial_command);
        let initial_shortcut = existing
            .as_ref()
            .and_then(|command| command.shortcut.clone());
        let initial_scope = if self.connection_id.is_some()
            && existing
                .as_ref()
                .is_none_or(|command| command.connection_id.is_some())
        {
            QuickCommandScope::CurrentConnection
        } else {
            QuickCommandScope::Global
        };

        let name_state = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder(t!("QuickCommand.name_placeholder").to_string())
                .default_value(&initial_name)
        });
        let description_state = cx.new(|cx| {
            TextareaState::new(window, cx)
                .placeholder(t!("QuickCommand.description_placeholder").to_string())
                .rows(2)
                .default_value(&initial_description)
        });
        let group_state = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder(t!("QuickCommand.group_placeholder").to_string())
                .default_value(&initial_group_name)
        });
        let color_items = quick_command_group_color_items();
        let selected_color_index = color_items
            .iter()
            .position(|item| item.value == initial_group_color)
            .unwrap_or(0);
        let color_state = cx.new(|cx| {
            SelectState::new(
                color_items,
                Some(gpui_component::IndexPath::new(selected_color_index)),
                window,
                cx,
            )
        });
        let command_state = cx.new(|cx| {
            TextareaState::new(window, cx)
                .placeholder(t!("QuickCommand.command_placeholder").to_string())
                .rows(4)
                .default_value(&initial_command)
        });
        let editor_state = cx.new(|cx| {
            QuickCommandEditorState::new(
                initial_shortcut.clone(),
                initial_auto_run,
                initial_scope,
                self.connection_id,
                self.colors.clone(),
                cx,
            )
        });

        let title = if existing.is_some() {
            t!("QuickCommand.edit").to_string()
        } else {
            t!("QuickCommand.add").to_string()
        };
        let ok_text = if existing.is_some() {
            t!("QuickCommand.save").to_string()
        } else {
            t!("QuickCommand.add_action").to_string()
        };
        let colors = self.colors.clone();
        let view = cx.entity().clone();
        window.open_dialog(cx, move |dialog, _window, dialog_cx| {
            let view_ok = view.clone();
            let existing_ok = existing.clone();
            let name_ok = name_state.clone();
            let description_ok = description_state.clone();
            let group_ok = group_state.clone();
            let color_ok = color_state.clone();
            let command_ok = command_state.clone();
            let editor_ok = editor_state.clone();
            let (ok_variant, cancel_variant) =
                quick_command_dialog_button_variants(&colors, dialog_cx);
            dialog
                .width(gpui::px(640.0))
                .bg(colors.background)
                .text_color(colors.foreground)
                .border_color(colors.border)
                .title(title.clone())
                .child(
                    v_flex()
                        .gap_2()
                        .bg(colors.background)
                        .text_color(colors.foreground)
                        .child(
                            h_flex()
                                .id("quick-command-primary-fields-row")
                                .items_center()
                                .gap_2()
                                .child(
                                    div()
                                        .w(gpui::px(84.0))
                                        .flex_shrink_0()
                                        .text_xs()
                                        .text_color(colors.muted_foreground)
                                        .child(t!("QuickCommand.name").to_string()),
                                )
                                .child(
                                    div()
                                        .flex_1()
                                        .child(Input::new(&name_state).small().w_full()),
                                ),
                        )
                        .child(
                            h_flex()
                                .items_start()
                                .gap_2()
                                .child(
                                    div()
                                        .w(gpui::px(84.0))
                                        .pt_2()
                                        .flex_shrink_0()
                                        .text_xs()
                                        .text_color(colors.muted_foreground)
                                        .child(t!("QuickCommand.description").to_string()),
                                )
                                .child(
                                    div()
                                        .flex_1()
                                        .h(gpui::px(64.0))
                                        .child(Textarea::new(&description_state).w_full().h_full()),
                                ),
                        )
                        .child(
                            h_flex()
                                .id("quick-command-group-fields-row")
                                .items_center()
                                .gap_2()
                                .child(
                                    div()
                                        .w(gpui::px(84.0))
                                        .flex_shrink_0()
                                        .text_xs()
                                        .text_color(colors.muted_foreground)
                                        .child(t!("QuickCommand.group").to_string()),
                                )
                                .child(
                                    div()
                                        .flex_1()
                                        .child(Input::new(&group_state).small().w_full()),
                                )
                                .child(
                                    div()
                                        .w(gpui::px(84.0))
                                        .flex_shrink_0()
                                        .text_xs()
                                        .text_color(colors.muted_foreground)
                                        .child(t!("QuickCommand.group_color").to_string()),
                                )
                                .child(
                                    div()
                                        .w(gpui::px(144.0))
                                        .flex_shrink_0()
                                        .child(Select::new(&color_state).small().w_full()),
                                ),
                        )
                        .child(
                            h_flex()
                                .items_start()
                                .gap_2()
                                .child(
                                    div()
                                        .w(gpui::px(84.0))
                                        .pt_2()
                                        .flex_shrink_0()
                                        .text_xs()
                                        .text_color(colors.muted_foreground)
                                        .child(t!("QuickCommand.command").to_string()),
                                )
                                .child(
                                    div()
                                        .flex_1()
                                        .h(gpui::px(108.0))
                                        .child(Textarea::new(&command_state).w_full().h_full()),
                                ),
                        )
                        .child(editor_state.clone())
                        .into_any_element(),
                )
                .confirm()
                .button_props(
                    DialogButtonProps::default()
                        .show_cancel(true)
                        .ok_text(ok_text.clone())
                        .ok_variant(ok_variant)
                        .cancel_text(t!("QuickCommand.cancel").to_string())
                        .cancel_variant(cancel_variant),
                )
                .on_ok(move |_, window, cx| {
                    let command = normalize_quick_command_value(&command_ok.read(cx).value());
                    if command.is_empty() {
                        window.push_notification(
                            Notification::error(t!("QuickCommand.command_required").to_string())
                                .autohide(true),
                            cx,
                        );
                        return false;
                    }
                    let auto_run = editor_ok.read(cx).auto_run;
                    let command = apply_auto_run_marker(&command, auto_run);
                    let name = name_ok.read(cx).value().trim().to_string();
                    let description = description_ok.read(cx).value().trim().to_string();
                    let group_name = group_ok.read(cx).value().trim().to_string();
                    let group_color = color_ok.read(cx).selected_value().cloned();
                    let (group_name, group_color) = normalize_group_fields(group_name, group_color);
                    let (shortcut, scope, invalid_shortcut) = {
                        let editor = editor_ok.read(cx);
                        (
                            editor.shortcut.clone(),
                            editor.scope,
                            editor.invalid_shortcut,
                        )
                    };
                    let shortcut =
                        match validated_quick_command_shortcut(shortcut, invalid_shortcut) {
                            Ok(shortcut) => shortcut,
                            Err(()) => {
                                window.push_notification(
                                    Notification::error(
                                        t!("QuickCommand.invalid_shortcut").to_string(),
                                    )
                                    .autohide(true),
                                    cx,
                                );
                                return false;
                            }
                        };
                    let connection_id =
                        connection_id_for_scope(view_ok.read(cx).connection_id, scope);
                    view_ok.update(cx, |this, cx| {
                        let result = if let Some(mut existing) = existing_ok.clone() {
                            existing.name = (!name.is_empty()).then_some(name.clone());
                            existing.description =
                                (!description.is_empty()).then_some(description.clone());
                            existing.group_name = group_name.clone();
                            existing.group_color = group_color.clone();
                            existing.command = command.clone();
                            existing.shortcut = shortcut.clone();
                            existing.connection_id = connection_id;
                            this.save_existing_command(existing, cx)
                        } else {
                            let mut new_command = QuickCommand::new(command.clone());
                            new_command.name = (!name.is_empty()).then_some(name.clone());
                            new_command.description =
                                (!description.is_empty()).then_some(description.clone());
                            new_command.group_name = group_name.clone();
                            new_command.group_color = group_color.clone();
                            new_command.shortcut = shortcut.clone();
                            new_command.connection_id = connection_id;
                            this.save_new_command(new_command, cx)
                        };
                        match result {
                            Ok(()) => true,
                            Err(error) => {
                                window.push_notification(
                                    Notification::error(
                                        t!("QuickCommand.save_failed", error = error).to_string(),
                                    )
                                    .autohide(true),
                                    cx,
                                );
                                false
                            }
                        }
                    })
                })
        });
    }

    /// 删除快捷命令
    fn delete_command(&mut self, id: i64, cx: &mut Context<Self>) {
        let storage = cx.global::<GlobalStorageState>().storage.clone();
        let Some(repo) = storage.get::<QuickCommandRepository>() else {
            tracing::error!("QuickCommandRepository not found");
            return;
        };
        match repo.delete(id) {
            Ok(()) => {
                self.commands.retain(|command| command.id != Some(id));
                self.filter_commands();
                cx.notify();
                notify_quick_commands_changed(cx);
            }
            Err(error) => tracing::error!(%error, "Failed to delete command"),
        }
    }

    /// 切换置顶状态
    fn toggle_pin(&mut self, id: i64, cx: &mut Context<Self>) {
        let storage = cx.global::<GlobalStorageState>().storage.clone();
        let Some(repo) = storage.get::<QuickCommandRepository>() else {
            tracing::error!("QuickCommandRepository not found");
            return;
        };
        match repo.toggle_pin(id) {
            Ok(pinned) => {
                if let Some(command) = self
                    .commands
                    .iter_mut()
                    .find(|command| command.id == Some(id))
                {
                    command.pinned = pinned;
                }
                self.sort_commands();
                self.filter_commands();
                cx.notify();
                notify_quick_commands_changed(cx);
            }
            Err(error) => tracing::error!(%error, "Failed to toggle pin"),
        }
    }

    /// 过滤非置顶命令；置顶命令始终在固定区域显示。
    fn filter_commands(&mut self) {
        let query = self.search_query.to_lowercase();
        self.filtered_commands = self
            .commands
            .iter()
            .filter(|command| !command.pinned)
            .filter(|command| command_matches_group_filter(command, &self.group_filter))
            .filter(|command| {
                query.is_empty()
                    || command.command.to_lowercase().contains(&query)
                    || command
                        .name
                        .as_ref()
                        .map(|name| name.to_lowercase().contains(&query))
                        .unwrap_or(false)
                    || command
                        .group_name
                        .as_ref()
                        .map(|group| group.to_lowercase().contains(&query))
                        .unwrap_or(false)
                    || command
                        .description
                        .as_ref()
                        .map(|description| description.to_lowercase().contains(&query))
                        .unwrap_or(false)
            })
            .cloned()
            .collect();
    }

    fn quick_command_repo(&self, cx: &App) -> Option<Arc<QuickCommandRepository>> {
        cx.try_global::<GlobalStorageState>()
            .and_then(|state| state.storage.get::<QuickCommandRepository>())
    }

    fn rename_group(&mut self, old_name: String, new_name: Option<String>, cx: &mut Context<Self>) {
        let Some(repo) = self.quick_command_repo(cx) else {
            return;
        };
        match repo.rename_group(&old_name, new_name.as_deref()) {
            Ok(()) => {
                self.group_filter = match new_name {
                    Some(name) if !name.trim().is_empty() => QuickCommandGroupFilter::Group(name),
                    _ => QuickCommandGroupFilter::Ungrouped,
                };
                self.load_commands(cx);
                notify_quick_commands_changed(cx);
            }
            Err(error) => {
                tracing::error!(%error, %old_name, "Failed to rename quick command group")
            }
        }
    }

    fn recolor_group(&mut self, group_name: String, color: Option<String>, cx: &mut Context<Self>) {
        let Some(repo) = self.quick_command_repo(cx) else {
            return;
        };
        match repo.recolor_group(&group_name, color.as_deref()) {
            Ok(()) => {
                self.load_commands(cx);
                notify_quick_commands_changed(cx);
            }
            Err(error) => {
                tracing::error!(%error, %group_name, "Failed to recolor quick command group")
            }
        }
    }

    fn clear_group(&mut self, group_name: String, cx: &mut Context<Self>) {
        let Some(repo) = self.quick_command_repo(cx) else {
            return;
        };
        match repo.clear_group(&group_name) {
            Ok(()) => {
                self.group_filter = QuickCommandGroupFilter::Ungrouped;
                self.load_commands(cx);
                notify_quick_commands_changed(cx);
            }
            Err(error) => {
                tracing::error!(%error, %group_name, "Failed to clear quick command group")
            }
        }
    }

    fn open_rename_group_dialog(
        panel: Entity<Self>,
        group_name: String,
        colors: TerminalColors,
        window: &mut Window,
        cx: &mut App,
    ) {
        let input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder(t!("QuickCommand.rename_group_placeholder").to_string())
                .default_value(&group_name)
        });
        window.open_dialog(cx, move |dialog, _window, dialog_cx| {
            let input_ok = input.clone();
            let panel_ok = panel.clone();
            let original = group_name.clone();
            let (ok_variant, cancel_variant) =
                quick_command_dialog_button_variants(&colors, dialog_cx);
            dialog
                .bg(colors.background)
                .text_color(colors.foreground)
                .border_color(colors.border)
                .title(t!("QuickCommand.rename_group").to_string())
                .child(
                    div()
                        .bg(colors.background)
                        .child(Input::new(&input).small().w_full()),
                )
                .confirm()
                .button_props(
                    DialogButtonProps::default()
                        .show_cancel(true)
                        .ok_text(t!("QuickCommand.save").to_string())
                        .ok_variant(ok_variant)
                        .cancel_text(t!("QuickCommand.cancel").to_string())
                        .cancel_variant(cancel_variant),
                )
                .on_ok(move |_, _, cx| {
                    let next = input_ok.read(cx).value().trim().to_string();
                    panel_ok.update(cx, |panel, cx| {
                        panel.rename_group(
                            original.clone(),
                            (!next.is_empty()).then_some(next.clone()),
                            cx,
                        );
                    });
                    true
                })
        });
    }

    fn open_delete_group_dialog(
        panel: Entity<Self>,
        group_name: String,
        colors: TerminalColors,
        window: &mut Window,
        cx: &mut App,
    ) {
        window.open_dialog(cx, move |dialog, _window, dialog_cx| {
            let panel_ok = panel.clone();
            let original = group_name.clone();
            let (ok_variant, cancel_variant) =
                quick_command_dialog_button_variants(&colors, dialog_cx);
            dialog
                .bg(colors.background)
                .text_color(colors.foreground)
                .border_color(colors.border)
                .title(t!("QuickCommand.delete_group").to_string())
                .child(
                    div()
                        .bg(colors.background)
                        .text_color(colors.foreground)
                        .child(
                            t!("QuickCommand.delete_group_confirm", name = group_name).to_string(),
                        ),
                )
                .confirm()
                .button_props(
                    DialogButtonProps::default()
                        .show_cancel(true)
                        .ok_text(t!("QuickCommand.delete_group").to_string())
                        .ok_variant(ok_variant)
                        .cancel_text(t!("QuickCommand.cancel").to_string())
                        .cancel_variant(cancel_variant),
                )
                .on_ok(move |_, _, cx| {
                    panel_ok.update(cx, |panel, cx| {
                        panel.clear_group(original.clone(), cx);
                    });
                    true
                })
        });
    }

    fn group_context_menu(
        panel: Entity<Self>,
        chip: QuickCommandGroupChip,
        menu: PopupMenu,
        colors: TerminalColors,
    ) -> PopupMenu {
        let QuickCommandGroupFilter::Group(group_name) = &chip.filter else {
            return menu;
        };
        let mut menu = menu;
        let group_name_for_rename = group_name.clone();
        let group_name_for_delete = group_name.clone();

        let rename_panel = panel.clone();
        let rename_colors = colors.clone();
        menu = menu.item(
            PopupMenuItem::new(t!("QuickCommand.rename_group").to_string()).on_click(
                move |_, window, cx| {
                    Self::open_rename_group_dialog(
                        rename_panel.clone(),
                        group_name_for_rename.clone(),
                        rename_colors.clone(),
                        window,
                        cx,
                    );
                },
            ),
        );

        menu = menu.separator();
        for (value, label) in quick_command_group_color_choices() {
            let recolor_panel = panel.clone();
            let group_name = group_name.clone();
            let value = value.to_string();
            let checked = chip.color.as_deref().unwrap_or_default() == value;
            menu = menu.item(
                PopupMenuItem::new(t!("QuickCommand.color_option", color = label).to_string())
                    .checked(checked)
                    .on_click(move |_, _, cx| {
                        recolor_panel.update(cx, |panel, cx| {
                            panel.recolor_group(
                                group_name.clone(),
                                (!value.trim().is_empty()).then_some(value.clone()),
                                cx,
                            );
                        });
                    }),
            );
        }

        let delete_panel = panel;
        menu.separator().item(
            PopupMenuItem::new(t!("QuickCommand.delete_group").to_string()).on_click(
                move |_, window, cx| {
                    Self::open_delete_group_dialog(
                        delete_panel.clone(),
                        group_name_for_delete.clone(),
                        colors.clone(),
                        window,
                        cx,
                    );
                },
            ),
        )
    }

    fn paste_command(&self, command: String, cx: &mut Context<Self>) {
        cx.emit(QuickCommandPanelEvent::ExecuteCommand(command));
    }

    fn command_tooltip(command: &QuickCommand) -> String {
        let mut lines = Vec::new();
        if let Some(name) = command.name.as_ref().filter(|name| !name.trim().is_empty()) {
            lines.push(name.clone());
        }
        if let Some(group) = command
            .group_name
            .as_ref()
            .filter(|group| !group.trim().is_empty())
        {
            lines.push(format!("分组：{group}"));
        }
        if let Some(description) = command
            .description
            .as_ref()
            .filter(|description| !description.trim().is_empty())
        {
            lines.push(description.clone());
        }
        if !lines.is_empty() {
            lines.push(String::new());
        }
        lines.push(command.command.clone());
        lines.join("\n")
    }

    fn copy_command(&self, command: &str, window: &mut Window, cx: &mut Context<Self>) {
        cx.write_to_clipboard(ClipboardItem::new_string(command.to_string()));
        window.push_notification(
            Notification::success(t!("QuickCommand.copied").to_string()).autohide(true),
            cx,
        );
    }

    fn confirm_delete_command(
        &mut self,
        id: i64,
        command: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let view = cx.entity().clone();
        let colors = self.colors.clone();
        window.open_dialog(cx, move |dialog, _window, dialog_cx| {
            let view_ok = view.clone();
            let (ok_variant, cancel_variant) =
                quick_command_dialog_button_variants(&colors, dialog_cx);
            let preview = if command.chars().count() > 120 {
                format!("{}...", command.chars().take(120).collect::<String>())
            } else {
                command.clone()
            };
            dialog
                .bg(colors.background)
                .text_color(colors.foreground)
                .border_color(colors.border)
                .title(t!("QuickCommand.delete_confirm_title").to_string())
                .child(
                    v_flex()
                        .gap_2()
                        .bg(colors.background)
                        .text_color(colors.foreground)
                        .child(t!("QuickCommand.delete_confirm_message").to_string())
                        .child(
                            div()
                                .text_xs()
                                .text_color(colors.muted_foreground)
                                .child(preview),
                        )
                        .into_any_element(),
                )
                .confirm()
                .button_props(
                    DialogButtonProps::default()
                        .ok_text(t!("QuickCommand.delete_action").to_string())
                        .ok_variant(ok_variant)
                        .cancel_text(t!("Common.cancel").to_string())
                        .cancel_variant(cancel_variant),
                )
                .on_ok(move |_, _, cx| {
                    view_ok.update(cx, |this, cx| this.delete_command(id, cx));
                    true
                })
        });
    }

    fn render_search_bar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let has_query = !self.search_query.is_empty();
        let border = self.colors.border;
        let muted_fg = self.colors.muted_foreground;
        h_flex()
            .flex_shrink_0()
            .h_8()
            .px_2()
            .gap_2()
            .items_center()
            .border_b_1()
            .border_color(border)
            .child(Icon::new(IconName::Search).xsmall().text_color(muted_fg))
            .child(
                div().flex_1().child(
                    Input::new(&self.search_input_state)
                        .xsmall()
                        .appearance(false)
                        .cleanable(has_query),
                ),
            )
            .child(
                IconButton::new("add-command", IconName::Plus)
                    .role(IconButtonRole::Compact)
                    .tooltip(t!("QuickCommand.add_tooltip").to_string())
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.open_command_editor(None, None, window, cx);
                    })),
            )
    }

    fn render_group_chip(
        &self,
        chip: QuickCommandGroupChip,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let is_active = self.group_filter == chip.filter;
        let is_named = matches!(chip.filter, QuickCommandGroupFilter::Group(_));
        let filter = chip.filter.clone();
        let label = quick_command_group_chip_label(&chip.filter);
        let dot_color = quick_command_group_color(chip.color.as_deref(), self.colors.accent);
        let panel_for_menu = cx.entity().clone();
        let chip_for_menu = chip.clone();
        let colors_for_menu = self.colors.clone();

        let chip_element = h_flex()
            .id(SharedString::from(format!("quick-command-group-{label}")))
            .flex_shrink_0()
            .h_7()
            .px_2()
            .gap_1()
            .items_center()
            .rounded_full()
            .cursor_pointer()
            .bg(if is_active {
                self.colors.accent.opacity(0.16)
            } else {
                self.colors.muted.opacity(0.72)
            })
            .border_1()
            .border_color(if is_active {
                self.colors.accent
            } else {
                self.colors.border
            })
            .hover(|style| style.bg(self.colors.muted))
            .on_click(cx.listener(move |this, _, _, cx| {
                this.set_group_filter(filter.clone(), cx);
            }))
            .when(is_named, |this| {
                this.child(div().size_2().flex_shrink_0().rounded_full().bg(dot_color))
            })
            .child(
                div()
                    .text_xs()
                    .font_weight(if is_active {
                        gpui::FontWeight::SEMIBOLD
                    } else {
                        gpui::FontWeight::NORMAL
                    })
                    .text_color(if is_active {
                        self.colors.accent
                    } else {
                        self.colors.foreground
                    })
                    .child(label),
            );

        if is_named {
            chip_element
                .context_menu(move |menu, _window, _cx| {
                    Self::group_context_menu(
                        panel_for_menu.clone(),
                        chip_for_menu.clone(),
                        menu,
                        colors_for_menu.clone(),
                    )
                })
                .into_any_element()
        } else {
            chip_element.into_any_element()
        }
    }

    fn render_group_bar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let groups = quick_command_groups(&self.commands);
        div()
            .flex_shrink_0()
            .w_full()
            .min_w_0()
            .overflow_x_hidden()
            .border_b_1()
            .border_color(self.colors.border)
            .child(
                h_flex()
                    .id("quick-command-groups-scroll")
                    .w_full()
                    .min_w_0()
                    .gap_1()
                    .px_2()
                    .py_1()
                    .overflow_x_scroll()
                    .children(
                        groups
                            .into_iter()
                            .map(|chip| self.render_group_chip(chip, cx)),
                    ),
            )
    }

    fn render_command_item(
        &self,
        index: usize,
        command: &QuickCommand,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let value = command.command.clone();
        let value_for_row = value.clone();
        let value_for_paste = value.clone();
        let value_for_copy = value.clone();
        let value_for_delete = value.clone();
        let existing_for_edit = command.clone();
        let tooltip = Self::command_tooltip(command);
        let id = command.id.unwrap_or(0);
        let is_pinned = command.pinned;
        let item_group = SharedString::from(format!("quick-cmd-group-{index}"));
        let display = command
            .name
            .as_ref()
            .filter(|name| !name.trim().is_empty())
            .cloned()
            .unwrap_or_else(|| value.clone());
        let pin_color = cx.theme().warning;
        let muted_bg = self.colors.muted;

        div()
            .id(SharedString::from(format!("quick-cmd-item-{index}")))
            .group(item_group.clone())
            .w_full()
            .px_3()
            .py_2()
            .rounded_md()
            .cursor_pointer()
            .hover(|style| style.bg(muted_bg))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, _, _, cx| this.paste_command(value_for_row.clone(), cx)),
            )
            .child(
                h_flex()
                    .w_full()
                    .items_center()
                    .justify_between()
                    .child(
                        h_flex()
                            .flex_1()
                            .min_w_0()
                            .gap_2()
                            .items_center()
                            .when(is_pinned, |this| {
                                this.child(
                                    Icon::new(IconName::Star)
                                        .with_size(Size::XSmall)
                                        .text_color(pin_color),
                                )
                            })
                            .child(
                                div()
                                    .id(SharedString::from(format!("quick-cmd-text-{index}")))
                                    .flex_1()
                                    .min_w_0()
                                    .text_sm()
                                    .overflow_hidden()
                                    .text_ellipsis()
                                    .tooltip(move |window, cx| {
                                        Tooltip::new(tooltip.clone()).build(window, cx)
                                    })
                                    .child(display),
                            ),
                    )
                    .child(
                        h_flex()
                            .flex_shrink_0()
                            .gap_1()
                            .ml_2()
                            .invisible()
                            .group_hover(item_group, |style| style.visible())
                            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                            .child(
                                IconButton::new(
                                    SharedString::from(format!("pin-{index}")),
                                    if is_pinned {
                                        IconName::StarOff
                                    } else {
                                        IconName::Star
                                    },
                                )
                                .role(IconButtonRole::Compact)
                                .tooltip(if is_pinned {
                                    t!("QuickCommand.unpin_tooltip").to_string()
                                } else {
                                    t!("QuickCommand.pin_tooltip").to_string()
                                })
                                .when(is_pinned, |this| this.text_color(pin_color))
                                .on_click(cx.listener(
                                    move |this, _, _, cx| {
                                        this.toggle_pin(id, cx);
                                    },
                                )),
                            )
                            .child(
                                IconButton::new(
                                    SharedString::from(format!("edit-{index}")),
                                    IconName::Edit,
                                )
                                .role(IconButtonRole::Compact)
                                .tooltip(t!("QuickCommand.edit_tooltip").to_string())
                                .on_click(cx.listener(
                                    move |this, _, window, cx| {
                                        this.open_command_editor(
                                            Some(existing_for_edit.clone()),
                                            None,
                                            window,
                                            cx,
                                        );
                                    },
                                )),
                            )
                            .child(
                                IconButton::new(
                                    SharedString::from(format!("copy-{index}")),
                                    IconName::Copy,
                                )
                                .role(IconButtonRole::Compact)
                                .tooltip(t!("QuickCommand.copy_tooltip").to_string())
                                .on_click(cx.listener(
                                    move |this, _, window, cx| {
                                        this.copy_command(&value_for_copy, window, cx);
                                    },
                                )),
                            )
                            .child(
                                IconButton::new(
                                    SharedString::from(format!("delete-{index}")),
                                    IconName::Remove,
                                )
                                .role(IconButtonRole::Compact)
                                .text_color(cx.theme().danger)
                                .tooltip(t!("QuickCommand.delete_tooltip").to_string())
                                .on_click(cx.listener(
                                    move |this, _, window, cx| {
                                        this.confirm_delete_command(
                                            id,
                                            value_for_delete.clone(),
                                            window,
                                            cx,
                                        );
                                    },
                                )),
                            )
                            .child(
                                IconButton::new(
                                    SharedString::from(format!("paste-{index}")),
                                    IconName::Paste,
                                )
                                .role(IconButtonRole::Compact)
                                .tooltip(t!("QuickCommand.paste_tooltip").to_string())
                                .on_click(cx.listener(
                                    move |this, _, _, cx| {
                                        this.paste_command(value_for_paste.clone(), cx);
                                    },
                                )),
                            ),
                    ),
            )
    }

    fn render_empty_state(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let muted_fg = self.colors.muted_foreground;
        let search_empty = self.search_query.is_empty();
        v_flex()
            .size_full()
            .items_center()
            .justify_center()
            .gap_2()
            .child(
                Icon::new(IconName::SquareTerminal)
                    .with_size(Size::Large)
                    .text_color(muted_fg),
            )
            .child(div().text_sm().text_color(muted_fg).child(if search_empty {
                t!("QuickCommand.empty_group").to_string()
            } else {
                t!("QuickCommand.no_matches").to_string()
            }))
            .when(search_empty, |this| {
                this.child(
                    Button::new("add-first-command")
                        .label(t!("QuickCommand.add_command").to_string())
                        .small()
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.open_command_editor(None, None, window, cx);
                        })),
                )
            })
    }

    fn render_loading_state(&self) -> impl IntoElement {
        let muted_fg = self.colors.muted_foreground;
        v_flex()
            .size_full()
            .items_center()
            .justify_center()
            .gap_2()
            .child(
                Icon::new(IconName::Loader)
                    .with_size(Size::Medium)
                    .text_color(muted_fg),
            )
            .child(
                div()
                    .text_sm()
                    .text_color(muted_fg)
                    .child(t!("QuickCommand.loading").to_string()),
            )
    }
}

impl EventEmitter<QuickCommandPanelEvent> for QuickCommandPanel {}

impl Focusable for QuickCommandPanel {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for QuickCommandPanel {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let pinned_commands = self.pinned_commands();
        let pinned_empty = pinned_commands.is_empty();
        let commands_empty = self.filtered_commands.is_empty() && pinned_empty;
        let item_count = self.filtered_commands.len();

        v_flex()
            .size_full()
            .bg(self.colors.background)
            .text_color(self.colors.foreground)
            .child(self.render_search_bar(cx))
            .child(self.render_group_bar(cx))
            .when(self.is_loading, |this| {
                this.child(self.render_loading_state())
            })
            .when(!self.is_loading && !pinned_empty, |this| {
                this.child(
                    v_flex()
                        .flex_shrink_0()
                        .gap_1()
                        .px_2()
                        .pt_1()
                        .pb_1()
                        .children(pinned_commands.iter().enumerate().map(|(index, command)| {
                            self.render_command_item(index + 10_000, command, cx)
                        })),
                )
            })
            .when(!self.is_loading && commands_empty, |this| {
                this.child(self.render_empty_state(cx))
            })
            .when(
                !self.is_loading && !self.filtered_commands.is_empty(),
                |this| {
                    this.child(
                        uniform_list("quick-command-list", item_count, {
                            cx.processor(move |state: &mut Self, range: Range<usize>, _, cx| {
                                range
                                    .map(|index| {
                                        let command = state.filtered_commands[index].clone();
                                        state.render_command_item(index, &command, cx)
                                    })
                                    .collect()
                            })
                        })
                        .flex_1()
                        .size_full()
                        .px_2()
                        .py_1()
                        .track_scroll(&self.scroll_handle)
                        .with_sizing_behavior(ListSizingBehavior::Auto),
                    )
                },
            )
    }
}

#[cfg(test)]
mod tests {
    use super::{
        QuickCommandGroupFilter, QuickCommandScope, ShortcutCapture, ShortcutCaptureLabel,
        apply_auto_run_marker, capture_quick_command_shortcut, command_matches_group_filter,
        connection_id_for_scope, new_command_group_defaults, normalize_group_fields,
        normalize_quick_command_value, quick_command_group_chip_label,
        quick_command_group_color_items, quick_command_groups, shortcut_capture_label,
        validated_quick_command_shortcut,
    };
    use gpui::Keystroke;
    use one_core::storage::QuickCommand;

    fn command_in_group(group_name: Option<&str>) -> QuickCommand {
        let mut command = QuickCommand::new("echo test".to_string());
        command.group_name = group_name.map(str::to_string);
        command
    }

    fn grouped_command(name: Option<&str>, color: Option<&str>) -> QuickCommand {
        let mut command = command_in_group(name);
        command.group_color = color.map(str::to_string);
        command
    }

    #[test]
    fn auto_run_marker_controls_trailing_newline() {
        // 勾选「点击执行」：保证末尾有且仅有一个换行标记
        assert_eq!("echo hi\n", apply_auto_run_marker("echo hi", true));
        assert_eq!("echo hi\n", apply_auto_run_marker("echo hi\n", true));
        // 取消勾选：去掉末尾换行，点击后仅粘贴
        assert_eq!("echo hi", apply_auto_run_marker("echo hi\n", false));
        assert_eq!("echo hi", apply_auto_run_marker("echo hi\r\n", false));
        assert_eq!("echo hi", apply_auto_run_marker("echo hi", false));
        // 多行命令内部的换行不受影响
        assert_eq!("cd /tmp\nls", apply_auto_run_marker("cd /tmp\nls\n", false));
        assert_eq!("cd /tmp\nls\n", apply_auto_run_marker("cd /tmp\nls", true));
    }

    #[test]
    fn panel_group_chips_include_fixed_filters_and_sorted_full_named_groups() {
        let groups = quick_command_groups(&[
            grouped_command(Some("zeta deployment"), Some("purple")),
            grouped_command(None, None),
            grouped_command(Some("alpha database"), Some("green")),
            grouped_command(Some("  "), Some("red")),
        ]);

        assert_eq!(4, groups.len());
        assert_eq!(QuickCommandGroupFilter::All, groups[0].filter);
        assert_eq!("全部", quick_command_group_chip_label(&groups[0].filter));
        assert_eq!(QuickCommandGroupFilter::Ungrouped, groups[1].filter);
        assert_eq!("未分组", quick_command_group_chip_label(&groups[1].filter));
        assert_eq!(
            QuickCommandGroupFilter::Group("alpha database".to_string()),
            groups[2].filter
        );
        assert_eq!(
            "alpha database",
            quick_command_group_chip_label(&groups[2].filter)
        );
        assert_eq!(Some("green"), groups[2].color.as_deref());
        assert_eq!(
            QuickCommandGroupFilter::Group("zeta deployment".to_string()),
            groups[3].filter
        );
        assert_eq!(
            "zeta deployment",
            quick_command_group_chip_label(&groups[3].filter)
        );
        assert_eq!(Some("purple"), groups[3].color.as_deref());
    }

    #[test]
    fn panel_group_chip_preserves_first_non_empty_color() {
        let groups = quick_command_groups(&[
            grouped_command(Some("deploy"), None),
            grouped_command(Some("deploy"), Some("cyan")),
            grouped_command(Some("deploy"), Some("red")),
        ]);

        assert_eq!(Some("cyan"), groups[2].color.as_deref());
    }

    #[test]
    fn new_command_defaults_to_selected_named_group_only() {
        let commands = vec![grouped_command(Some("deploy"), Some("orange"))];

        assert_eq!(
            ("deploy".to_string(), "orange".to_string()),
            new_command_group_defaults(
                &QuickCommandGroupFilter::Group("deploy".to_string()),
                &commands,
            )
        );
        assert_eq!(
            (String::new(), String::new()),
            new_command_group_defaults(&QuickCommandGroupFilter::All, &commands)
        );
        assert_eq!(
            (String::new(), String::new()),
            new_command_group_defaults(&QuickCommandGroupFilter::Ungrouped, &commands)
        );
    }

    #[test]
    fn panel_renders_horizontally_scrollable_groups_below_search() {
        let source = include_str!("quick_command_panel.rs");
        let render = source
            .split_once("impl Render for QuickCommandPanel")
            .expect("QuickCommandPanel render implementation")
            .1
            .split_once("#[cfg(test)]")
            .expect("QuickCommandPanel render boundary")
            .0;
        let group_bar = source
            .split_once("fn render_group_bar")
            .expect("group bar implementation")
            .1
            .split_once("fn render_command_item")
            .expect("group bar implementation boundary")
            .0;
        let search = render
            .find(".child(self.render_search_bar(cx))")
            .expect("search bar render call");
        let groups = render
            .find(".child(self.render_group_bar(cx))")
            .expect("group bar render call");

        assert!(search < groups);
        assert!(group_bar.contains(".overflow_x_scroll()"));
    }

    #[test]
    fn ungrouped_filter_accepts_missing_or_blank_group_names() {
        let filter = QuickCommandGroupFilter::Ungrouped;

        assert!(command_matches_group_filter(
            &command_in_group(None),
            &filter
        ));
        assert!(command_matches_group_filter(
            &command_in_group(Some("  ")),
            &filter
        ));
        assert!(!command_matches_group_filter(
            &command_in_group(Some("deploy")),
            &filter
        ));
    }

    #[test]
    fn named_group_filter_ignores_outer_whitespace_but_remains_case_sensitive() {
        let filter = QuickCommandGroupFilter::Group("deploy".to_string());

        assert!(command_matches_group_filter(
            &command_in_group(Some("deploy")),
            &filter
        ));
        assert!(command_matches_group_filter(
            &command_in_group(Some(" deploy ")),
            &filter
        ));
        assert!(!command_matches_group_filter(
            &command_in_group(Some("Deploy")),
            &filter
        ));
        assert!(!command_matches_group_filter(
            &command_in_group(None),
            &filter
        ));
    }

    #[test]
    fn quick_command_group_color_palette_matches_upstream() {
        let values = quick_command_group_color_items()
            .into_iter()
            .map(|item| item.value)
            .collect::<Vec<_>>();

        assert_eq!(
            values,
            vec![
                "", "blue", "cyan", "green", "yellow", "orange", "red", "pink", "purple", "gray",
            ]
        );
    }

    #[test]
    fn blank_group_name_always_clears_group_color() {
        assert_eq!(
            normalize_group_fields("  ".to_string(), Some("blue".to_string())),
            (None, None)
        );
    }

    #[test]
    fn group_fields_are_trimmed_and_default_color_is_omitted() {
        assert_eq!(
            normalize_group_fields(" deploy ".to_string(), Some("  ".to_string())),
            (Some("deploy".to_string()), None)
        );
        assert_eq!(
            normalize_group_fields(" deploy ".to_string(), Some("green".to_string())),
            (Some("deploy".to_string()), Some("green".to_string()))
        );
    }

    #[test]
    fn quick_command_scope_maps_to_connection_or_global_storage() {
        assert_eq!(
            Some(42),
            connection_id_for_scope(Some(42), QuickCommandScope::CurrentConnection)
        );
        assert_eq!(
            None,
            connection_id_for_scope(Some(42), QuickCommandScope::Global)
        );
        assert_eq!(
            None,
            connection_id_for_scope(None, QuickCommandScope::CurrentConnection)
        );
    }

    #[test]
    fn shortcut_capture_requires_a_modifier_and_escape_clears() {
        assert_eq!(
            ShortcutCapture::Shortcut("ctrl-alt-k".to_string()),
            capture_quick_command_shortcut(
                &Keystroke::parse("alt-ctrl-k").expect("parse valid shortcut")
            )
        );
        assert_eq!(
            ShortcutCapture::Invalid,
            capture_quick_command_shortcut(
                &Keystroke::parse("k").expect("parse unmodified shortcut")
            )
        );
        assert_eq!(
            ShortcutCapture::Invalid,
            capture_quick_command_shortcut(
                &Keystroke::parse("ctrl").expect("parse modifier shortcut")
            )
        );
        assert_eq!(
            ShortcutCapture::Clear,
            capture_quick_command_shortcut(
                &Keystroke::parse("escape").expect("parse escape shortcut")
            )
        );
    }

    #[test]
    fn captured_shortcut_is_displayed_immediately_while_capture_keeps_focus() {
        assert_eq!(
            ShortcutCaptureLabel::Shortcut("ctrl-alt-k"),
            shortcut_capture_label(Some("ctrl-alt-k"), true)
        );
        assert_eq!(
            ShortcutCaptureLabel::PressShortcut,
            shortcut_capture_label(None, true)
        );
        assert_eq!(
            ShortcutCaptureLabel::Shortcut("ctrl-alt-k"),
            shortcut_capture_label(Some("ctrl-alt-k"), false)
        );
        assert_eq!(
            ShortcutCaptureLabel::Unassigned,
            shortcut_capture_label(None, false)
        );
    }

    #[test]
    fn invalid_shortcut_capture_blocks_saving_without_overwriting_the_previous_value() {
        assert_eq!(
            Err(()),
            validated_quick_command_shortcut(Some("ctrl-k".to_string()), true)
        );
        assert_eq!(
            Err(()),
            validated_quick_command_shortcut(Some("cmd-not-a-real-key".to_string()), false)
        );
        assert_eq!(
            Err(()),
            validated_quick_command_shortcut(Some("k".to_string()), false)
        );
        assert_eq!(
            Ok(Some("ctrl-k".to_string())),
            validated_quick_command_shortcut(Some("ctrl-k".to_string()), false)
        );
        assert_eq!(
            Ok(Some("ctrl-alt-k".to_string())),
            validated_quick_command_shortcut(Some("alt-ctrl-k".to_string()), false)
        );
        assert_eq!(Ok(None), validated_quick_command_shortcut(None, false));
    }

    #[test]
    fn quick_command_editor_preserves_exactly_one_explicit_trailing_enter() {
        assert_eq!("ls -la\n", normalize_quick_command_value("  ls -la  \n"));
        assert_eq!("ls -la\n", normalize_quick_command_value("ls -la\r\n"));
        assert_eq!("ls -la\n", normalize_quick_command_value("ls -la\r"));
        assert_eq!("ls -la", normalize_quick_command_value("  ls -la  "));
        assert_eq!("", normalize_quick_command_value("  \n"));
    }

    #[test]
    fn command_editor_renders_shortcut_capture_and_scope_controls() {
        let source = include_str!("quick_command_panel.rs");
        let editor = source
            .split_once("impl Render for QuickCommandEditorState")
            .expect("quick command editor render implementation")
            .1
            .split_once("impl QuickCommandPanel")
            .expect("quick command editor render boundary")
            .0;

        assert!(editor.contains(".track_focus(&self.shortcut_focus_handle)"));
        assert!(editor.contains(".on_key_down("));
        assert!(editor.contains("RadioGroup::horizontal(\"quick-command-scope\")"));
        assert!(!editor.contains("RadioGroup::vertical(\"quick-command-scope\")"));
        assert!(editor.contains(".id(\"quick-command-shortcut-scope-row\")"));
        assert!(editor.contains(".w(gpui::px(280.0))"));
        assert!(editor.contains("QuickCommandScope::CurrentConnection"));
        assert!(editor.contains("QuickCommandScope::Global"));
        assert!(editor.contains("QuickCommand.shortcut_unassigned"));
        assert!(editor.contains("QuickCommand.global_shared_description"));
    }

    #[test]
    fn command_editor_groups_short_fields_into_compact_rows() {
        let source = include_str!("quick_command_panel.rs");
        let editor = source
            .split_once("fn open_command_editor(")
            .expect("quick command editor implementation")
            .1
            .split_once("\n    fn ")
            .expect("quick command editor boundary")
            .0;

        assert!(editor.contains(".id(\"quick-command-primary-fields-row\")"));
        assert!(editor.contains(".id(\"quick-command-group-fields-row\")"));
        assert!(editor.contains(".width(gpui::px(640.0))"));
    }

    #[test]
    fn quick_command_mutations_emit_cache_refresh_events() {
        let source = include_str!("quick_command_panel.rs");
        for function in [
            "fn save_new_command",
            "fn save_existing_command",
            "fn delete_command",
            "fn toggle_pin",
            "fn rename_group",
            "fn recolor_group",
            "fn clear_group",
        ] {
            let implementation = source
                .split_once(function)
                .unwrap_or_else(|| panic!("{function} implementation"))
                .1
                .split_once("\n    fn ")
                .map(|(body, _)| body)
                .unwrap_or_else(|| {
                    source
                        .split_once(function)
                        .expect("function implementation")
                        .1
                });
            assert!(
                implementation.contains("notify_quick_commands_changed(cx)"),
                "{function} must notify local and global command-bar caches after a successful mutation"
            );
        }
    }
}
