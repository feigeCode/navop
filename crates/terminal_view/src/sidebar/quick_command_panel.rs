//! 快捷命令面板
//!
//! 支持命令的新增、编辑、分组、置顶和删除功能。

use gpui::prelude::*;
use gpui::{
    App, AppContext, ClipboardItem, Context, Entity, EventEmitter, FocusHandle, Focusable,
    InteractiveElement, IntoElement, ListSizingBehavior, MouseButton, ParentElement, Render,
    SharedString, Styled, UniformListScrollHandle, Window, div, uniform_list,
};
use gpui_component::{
    ActiveTheme, Icon, IconName, Sizable, Size, WindowExt,
    button::{Button, ButtonCustomVariant, ButtonVariant, ButtonVariants},
    dialog::DialogButtonProps,
    h_flex,
    input::{Input, InputEvent, InputState, LocalInputStyle},
    notification::Notification,
    select::{Select, SelectItem, SelectState},
    tooltip::Tooltip,
    v_flex,
};
use one_core::storage::{
    GlobalStorageState, QuickCommand, QuickCommandRepository, traits::Repository,
};
use rust_i18n::t;
use std::ops::Range;

use crate::theme::TerminalColors;

/// 快捷命令面板事件
#[derive(Clone, Debug)]
pub enum QuickCommandPanelEvent {
    /// 关闭面板
    Close,
    /// 粘贴命令到终端输入区（不自动回车）
    ExecuteCommand(String),
    /// 快捷命令数据发生变化
    CommandsChanged(Vec<QuickCommand>),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum QuickCommandGroupFilter {
    All,
    Ungrouped,
    Group(String),
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
            .map(|name| name == group)
            .unwrap_or(false),
    }
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

fn quick_command_dialog_input_style(colors: &TerminalColors) -> LocalInputStyle {
    LocalInputStyle {
        background: colors.muted,
        foreground: colors.foreground,
        muted_foreground: colors.muted_foreground,
        border: colors.border,
    }
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
        let search_input_state = cx.new(|cx| InputState::new(window, cx).placeholder("Search"));
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

    pub fn current_commands(&self) -> Vec<QuickCommand> {
        self.commands.clone()
    }

    pub fn set_group_filter(
        &mut self,
        group_filter: QuickCommandGroupFilter,
        cx: &mut Context<Self>,
    ) {
        if self.group_filter == group_filter {
            return;
        }
        self.group_filter = group_filter;
        self.filter_commands();
        cx.notify();
    }

    fn emit_commands_changed(&self, cx: &mut Context<Self>) {
        cx.emit(QuickCommandPanelEvent::CommandsChanged(
            self.commands.clone(),
        ));
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
                self.emit_commands_changed(cx);
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
        command.connection_id = self.connection_id;
        command.sort_order = repo.next_sort_order(self.connection_id).unwrap_or(0);
        repo.insert(&mut command)?;
        self.commands.push(command);
        self.sort_commands();
        self.filter_commands();
        self.emit_commands_changed(cx);
        cx.notify();
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
        if let Some(existing) = self
            .commands
            .iter_mut()
            .find(|existing| existing.id == command.id)
        {
            *existing = command;
        }
        self.sort_commands();
        self.filter_commands();
        self.emit_commands_changed(cx);
        cx.notify();
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
        let initial_group_name = existing
            .as_ref()
            .and_then(|command| command.group_name.clone())
            .unwrap_or_default();
        let initial_group_color = existing
            .as_ref()
            .and_then(|command| command.group_color.clone())
            .unwrap_or_default();
        let initial_command = existing
            .as_ref()
            .map(|command| command.command.clone())
            .or(initial_command)
            .unwrap_or_default();

        let name_state = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("输入简短名称（可选）")
                .default_value(&initial_name)
        });
        let description_state = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("输入备注或使用说明（可选）")
                .multi_line(true)
                .rows(2)
                .default_value(&initial_description)
        });
        let group_state = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("输入分组名称（可选）")
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
            InputState::new(window, cx)
                .placeholder("输入命令")
                .multi_line(true)
                .rows(4)
                .default_value(&initial_command)
        });

        let title = if existing.is_some() {
            "编辑快捷命令"
        } else {
            "新增快捷命令"
        };
        let ok_text = if existing.is_some() {
            "保存"
        } else {
            "新增"
        };
        let colors = self.colors.clone();
        let input_style = quick_command_dialog_input_style(&colors);
        let view = cx.entity().clone();
        window.open_dialog(cx, move |dialog, _window, dialog_cx| {
            let view_ok = view.clone();
            let existing_ok = existing.clone();
            let name_ok = name_state.clone();
            let description_ok = description_state.clone();
            let group_ok = group_state.clone();
            let color_ok = color_state.clone();
            let command_ok = command_state.clone();
            let (ok_variant, cancel_variant) =
                quick_command_dialog_button_variants(&colors, dialog_cx);
            dialog
                .bg(colors.background)
                .text_color(colors.foreground)
                .border_color(colors.border)
                .title(title)
                .confirm()
                .child(
                    v_flex()
                        .gap_3()
                        .bg(colors.background)
                        .text_color(colors.foreground)
                        .child(
                            v_flex()
                                .gap_1()
                                .child(
                                    div()
                                        .text_xs()
                                        .text_color(colors.muted_foreground)
                                        .child("名称"),
                                )
                                .child(
                                    Input::new(&name_state)
                                        .small()
                                        .w_full()
                                        .local_style(input_style)
                                        .caret_color(colors.foreground),
                                ),
                        )
                        .child(
                            v_flex()
                                .gap_1()
                                .child(
                                    div()
                                        .text_xs()
                                        .text_color(colors.muted_foreground)
                                        .child("说明"),
                                )
                                .child(
                                    div().w_full().h(gpui::px(76.0)).child(
                                        Input::new(&description_state)
                                            .small()
                                            .w_full()
                                            .h_full()
                                            .local_style(input_style)
                                            .caret_color(colors.foreground),
                                    ),
                                ),
                        )
                        .child(
                            v_flex()
                                .gap_1()
                                .child(
                                    div()
                                        .text_xs()
                                        .text_color(colors.muted_foreground)
                                        .child("分组"),
                                )
                                .child(
                                    Input::new(&group_state)
                                        .small()
                                        .w_full()
                                        .local_style(input_style)
                                        .caret_color(colors.foreground),
                                ),
                        )
                        .child(
                            v_flex()
                                .gap_1()
                                .child(
                                    div()
                                        .text_xs()
                                        .text_color(colors.muted_foreground)
                                        .child("分组颜色"),
                                )
                                .child(
                                    Select::new(&color_state)
                                        .small()
                                        .w_full()
                                        .local_style(input_style)
                                        .local_menu_item_style(
                                            colors.foreground,
                                            colors.muted,
                                            colors.accent,
                                            colors.accent_foreground,
                                        ),
                                ),
                        )
                        .child(
                            v_flex()
                                .gap_1()
                                .child(
                                    div()
                                        .text_xs()
                                        .text_color(colors.muted_foreground)
                                        .child("命令"),
                                )
                                .child(
                                    div().w_full().h(gpui::px(132.0)).child(
                                        Input::new(&command_state)
                                            .small()
                                            .w_full()
                                            .h_full()
                                            .local_style(input_style)
                                            .caret_color(colors.foreground),
                                    ),
                                ),
                        )
                        .into_any_element(),
                )
                .button_props(
                    DialogButtonProps::default()
                        .ok_text(ok_text)
                        .ok_variant(ok_variant)
                        .cancel_text("取消")
                        .cancel_variant(cancel_variant),
                )
                .on_ok(move |_, window, cx| {
                    let command = command_ok.read(cx).value().trim().to_string();
                    if command.is_empty() {
                        window.push_notification(
                            Notification::error("命令不能为空").autohide(true),
                            cx,
                        );
                        return false;
                    }
                    let name = name_ok.read(cx).value().trim().to_string();
                    let description = description_ok.read(cx).value().trim().to_string();
                    let group_name = group_ok.read(cx).value().trim().to_string();
                    let group_color = color_ok.read(cx).selected_value().cloned();
                    let (group_name, group_color) = normalize_group_fields(group_name, group_color);
                    view_ok.update(cx, |this, cx| {
                        let result = if let Some(mut existing) = existing_ok.clone() {
                            existing.name = (!name.is_empty()).then_some(name.clone());
                            existing.description =
                                (!description.is_empty()).then_some(description.clone());
                            existing.group_name = group_name.clone();
                            existing.group_color = group_color.clone();
                            existing.command = command.clone();
                            this.save_existing_command(existing, cx)
                        } else {
                            let mut new_command = QuickCommand::new(command.clone());
                            new_command.name = (!name.is_empty()).then_some(name.clone());
                            new_command.description =
                                (!description.is_empty()).then_some(description.clone());
                            new_command.group_name = group_name.clone();
                            new_command.group_color = group_color.clone();
                            this.save_new_command(new_command, cx)
                        };
                        if let Err(error) = result {
                            window.push_notification(
                                Notification::error(format!("保存快捷命令失败: {error}"))
                                    .autohide(true),
                                cx,
                            );
                        }
                    });
                    true
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
                self.emit_commands_changed(cx);
                cx.notify();
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
                self.emit_commands_changed(cx);
                cx.notify();
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
                Button::new("add-command")
                    .icon(IconName::Plus)
                    .ghost()
                    .xsmall()
                    .tooltip(t!("QuickCommand.add_tooltip").to_string())
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.open_command_editor(None, None, window, cx);
                    })),
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
                                Button::new(SharedString::from(format!("pin-{index}")))
                                    .icon(if is_pinned {
                                        IconName::StarOff
                                    } else {
                                        IconName::Star
                                    })
                                    .ghost()
                                    .xsmall()
                                    .when(is_pinned, |this| this.text_color(pin_color))
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        this.toggle_pin(id, cx);
                                    })),
                            )
                            .child(
                                Button::new(SharedString::from(format!("edit-{index}")))
                                    .icon(IconName::Edit)
                                    .ghost()
                                    .xsmall()
                                    .on_click(cx.listener(move |this, _, window, cx| {
                                        this.open_command_editor(
                                            Some(existing_for_edit.clone()),
                                            None,
                                            window,
                                            cx,
                                        );
                                    })),
                            )
                            .child(
                                Button::new(SharedString::from(format!("copy-{index}")))
                                    .icon(IconName::Copy)
                                    .ghost()
                                    .xsmall()
                                    .on_click(cx.listener(move |this, _, window, cx| {
                                        this.copy_command(&value_for_copy, window, cx);
                                    })),
                            )
                            .child(
                                Button::new(SharedString::from(format!("delete-{index}")))
                                    .icon(IconName::Remove)
                                    .danger()
                                    .xsmall()
                                    .on_click(cx.listener(move |this, _, window, cx| {
                                        this.confirm_delete_command(
                                            id,
                                            value_for_delete.clone(),
                                            window,
                                            cx,
                                        );
                                    })),
                            )
                            .child(
                                Button::new(SharedString::from(format!("paste-{index}")))
                                    .icon(IconName::Paste)
                                    .ghost()
                                    .xsmall()
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        this.paste_command(value_for_paste.clone(), cx);
                                    })),
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
                "当前分组暂无命令"
            } else {
                "没有匹配的命令"
            }))
            .when(search_empty, |this| {
                this.child(
                    Button::new("add-first-command")
                        .label("新增命令")
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
            .child(div().text_sm().text_color(muted_fg).child("Loading..."))
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
        QuickCommandGroupFilter, command_matches_group_filter, normalize_group_fields,
        quick_command_dialog_input_style, quick_command_group_color_items,
    };
    use crate::theme::TerminalColors;
    use gpui::rgb;
    use one_core::storage::QuickCommand;

    fn command_in_group(group_name: Option<&str>) -> QuickCommand {
        let mut command = QuickCommand::new("echo test".to_string());
        command.group_name = group_name.map(str::to_string);
        command
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
    fn named_group_filter_requires_an_exact_group_name() {
        let filter = QuickCommandGroupFilter::Group("deploy".to_string());

        assert!(command_matches_group_filter(
            &command_in_group(Some("deploy")),
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
    fn quick_command_dialog_input_style_uses_terminal_palette() {
        let colors = TerminalColors {
            background: rgb(0x101010).into(),
            foreground: rgb(0xf0f0f0).into(),
            muted: rgb(0x202020).into(),
            muted_foreground: rgb(0x909090).into(),
            border: rgb(0x303030).into(),
            accent: rgb(0x3366ff).into(),
            accent_foreground: rgb(0xffffff).into(),
        };

        let style = quick_command_dialog_input_style(&colors);

        assert_eq!(colors.muted, style.background);
        assert_eq!(colors.foreground, style.foreground);
        assert_eq!(colors.muted_foreground, style.muted_foreground);
        assert_eq!(colors.border, style.border);
    }
}
