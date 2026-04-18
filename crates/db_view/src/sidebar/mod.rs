//! 数据库视图侧边栏模块
//!
//! 提供数据库视图的侧边栏功能，包括：
//! - AI 聊天面板
//! - 可扩展的其他面板

pub mod cell_editor_notifier;
mod cell_preview_panel;

use crate::chatdb::chat_panel::{ChatPanel, ChatPanelEvent};
use crate::chatdb::db_connection_selector::DbSelectorContext;
use crate::sidebar::cell_editor_notifier::{
    CellEditorSidebarEvent, get_cell_editor_sidebar_notifier,
};
use crate::sidebar::cell_preview_panel::CellPreviewPanel;
use crate::table_data::data_grid::DataGrid;
use gpui::prelude::FluentBuilder;
use gpui::{
    AnyElement, App, AppContext, Context, Entity, EventEmitter, FocusHandle, Focusable,
    InteractiveElement, IntoElement, ParentElement, Render, SharedString,
    StatefulInteractiveElement, Styled, Subscription, Window, div, px,
};
use gpui_component::{ActiveTheme, Icon, IconName, Sizable, Size, v_flex};
use one_core::ai_chat::CodeBlockAction;
use one_core::ai_chat::ask_ai::{AskAiEvent, get_ask_ai_notifier};
use one_core::layout::TOOLBAR_WIDTH;

/// 侧边栏面板类型
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SidebarPanel {
    /// AI 聊天面板
    AiChat,
    /// 表格大文本预览编辑面板
    CellPreview,
}

impl SidebarPanel {
    /// 获取面板图标
    pub fn icon(&self) -> Icon {
        match self {
            SidebarPanel::AiChat => IconName::AI.color(),
            SidebarPanel::CellPreview => IconName::Edit.color(),
        }
    }
}

/// 数据库侧边栏事件
#[derive(Clone, Debug)]
pub enum DatabaseSidebarEvent {
    /// 面板切换
    PanelChanged,
    /// 请求询问 AI（由外部触发，内部处理）
    AskAi,
}

/// 数据库侧边栏组件
pub struct DatabaseSidebar {
    /// 当前激活的面板
    active_panel: Option<SidebarPanel>,
    /// AI 聊天面板
    chat_panel: Entity<ChatPanel>,
    /// 单元格大文本预览/编辑面板
    cell_preview_panel: Entity<CellPreviewPanel>,
    /// 当前绑定到单元格预览面板的数据表
    cell_preview_data_grid: Option<gpui::WeakEntity<DataGrid>>,
    /// 焦点句柄
    focus_handle: FocusHandle,
    /// 是否处于激活状态（用于控制事件响应）
    is_active: bool,
    /// 订阅句柄
    _subs: Vec<Subscription>,
}

impl DatabaseSidebar {
    pub fn new(
        window: &mut Window,
        cx: &mut Context<Self>,
        selector_context: DbSelectorContext,
    ) -> Self {
        let chat_panel =
            cx.new(|cx| ChatPanel::new_for_sidebar(window, cx, selector_context.clone()));
        let cell_preview_panel = cx.new(|cx| CellPreviewPanel::new(window, cx));

        let mut subs = Vec::new();

        // 订阅 ChatPanel 关闭事件
        subs.push(
            cx.subscribe(&chat_panel, |this, _, _event: &ChatPanelEvent, cx| {
                this.active_panel = None;
                cx.emit(DatabaseSidebarEvent::PanelChanged);
                cx.notify();
            }),
        );

        // 订阅全局 AskAi 通知器
        if let Some(notifier) = get_ask_ai_notifier(cx) {
            subs.push(
                cx.subscribe(&notifier, move |this, _, event: &AskAiEvent, cx| {
                    // 只有激活的 tab 才响应事件
                    if this.is_active {
                        let AskAiEvent::Request(message) = event;
                        this.ask_ai(message.clone(), cx);
                    }
                }),
            );
        }

        if let Some(notifier) = get_cell_editor_sidebar_notifier(cx) {
            subs.push(cx.subscribe_in(
                &notifier,
                window,
                move |this, _, event: &CellEditorSidebarEvent, window, cx| {
                    if !this.is_active {
                        return;
                    }
                    let CellEditorSidebarEvent::Toggle(data_grid) = event;
                    this.toggle_cell_preview(data_grid.clone(), window, cx);
                },
            ));
        }

        Self {
            active_panel: None,
            chat_panel,
            cell_preview_panel,
            cell_preview_data_grid: None,
            focus_handle: cx.focus_handle(),
            is_active: false,
            _subs: subs,
        }
    }

    /// 设置激活状态
    /// 当 tab 被激活时调用 set_active(true)，失活时调用 set_active(false)
    pub fn set_active(&mut self, active: bool, cx: &mut Context<Self>) {
        if !active {
            self.flush_cell_preview(cx);
        }
        self.is_active = active;
        cx.notify();
    }

    /// 设置激活的面板
    pub fn set_active_panel(&mut self, panel: Option<SidebarPanel>, cx: &mut Context<Self>) {
        if self.active_panel != panel {
            if self.active_panel == Some(SidebarPanel::CellPreview)
                && panel != Some(SidebarPanel::CellPreview)
            {
                self.close_cell_preview(None, cx);
            }
            self.active_panel = panel;
            cx.emit(DatabaseSidebarEvent::PanelChanged);
            cx.notify();
        }
    }

    /// 切换面板
    pub fn toggle_panel(&mut self, panel: SidebarPanel, cx: &mut Context<Self>) {
        if self.active_panel == Some(panel) {
            self.set_active_panel(None, cx);
        } else {
            self.set_active_panel(Some(panel), cx);
        }
    }

    /// 是否显示侧边栏面板
    pub fn is_panel_visible(&self) -> bool {
        self.active_panel.is_some()
    }

    /// 询问 AI
    pub fn ask_ai(&mut self, message: String, cx: &mut Context<Self>) {
        // 显示 AI 面板
        if self.active_panel != Some(SidebarPanel::AiChat) {
            self.active_panel = Some(SidebarPanel::AiChat);
        }

        // 发送消息到 AI 聊天面板
        self.chat_panel.update(cx, |panel, cx| {
            panel.send_external_message(message, cx);
        });

        cx.emit(DatabaseSidebarEvent::AskAi);
        cx.notify();
    }

    pub fn open_cell_preview(
        &mut self,
        data_grid: gpui::WeakEntity<crate::table_data::data_grid::DataGrid>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.set_cell_preview_button_state(false, cx);
        self.cell_preview_panel.update(cx, |panel, cx| {
            panel.bind_data_grid(data_grid.clone(), window, cx);
        });
        self.cell_preview_data_grid = Some(data_grid);
        self.set_cell_preview_button_state(true, cx);
        self.set_active_panel(Some(SidebarPanel::CellPreview), cx);
    }

    pub fn toggle_cell_preview(
        &mut self,
        data_grid: gpui::WeakEntity<crate::table_data::data_grid::DataGrid>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.active_panel == Some(SidebarPanel::CellPreview)
            && self.is_same_cell_preview_target(&data_grid)
        {
            self.close_cell_preview(Some(window), cx);
            self.active_panel = None;
            cx.emit(DatabaseSidebarEvent::PanelChanged);
            cx.notify();
            return;
        }

        self.open_cell_preview(data_grid, window, cx);
    }

    fn flush_cell_preview(&self, cx: &mut Context<Self>) {
        self.cell_preview_panel.update(cx, |panel, cx| {
            panel.flush_pending(cx);
        });
    }

    fn close_cell_preview(&mut self, window: Option<&mut Window>, cx: &mut Context<Self>) {
        self.flush_cell_preview(cx);
        self.set_cell_preview_button_state(false, cx);
        if let Some(window) = window {
            self.cell_preview_panel.update(cx, |panel, cx| {
                panel.unbind(window, cx);
            });
        }
        self.cell_preview_data_grid = None;
    }

    fn set_cell_preview_button_state(&self, open: bool, cx: &mut Context<Self>) {
        let Some(grid) = self
            .cell_preview_data_grid
            .as_ref()
            .and_then(|data_grid| data_grid.upgrade())
        else {
            return;
        };

        let _ = grid.update(cx, |grid, cx| {
            grid.set_large_text_editor_sidebar_open(open, cx);
        });
    }

    fn is_same_cell_preview_target(&self, data_grid: &gpui::WeakEntity<DataGrid>) -> bool {
        let Some(current) = self.cell_preview_data_grid.as_ref() else {
            return false;
        };

        let (Some(current), Some(next)) = (current.upgrade(), data_grid.upgrade()) else {
            return false;
        };

        current.entity_id() == next.entity_id()
    }

    /// 注册代码块操作
    pub fn register_code_block_action(&self, _action: CodeBlockAction, _cx: &mut Context<Self>) {}

    /// 渲染工具栏按钮
    fn render_toolbar_button(
        &self,
        panel: SidebarPanel,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let is_active = self.active_panel == Some(panel);
        let accent_color = cx.theme().accent;
        let accent_fg = cx.theme().accent_foreground;
        let muted_fg = cx.theme().muted_foreground;
        let muted_bg = cx.theme().muted;

        div()
            .id(SharedString::from(format!("sidebar-btn-{:?}", panel)))
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
                    .with_size(Size::Medium)
                    .text_color(if is_active { accent_fg } else { muted_fg }),
            )
    }

    /// 渲染工具栏
    pub fn render_toolbar(&self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let border_color = cx.theme().border;
        let muted_bg = cx.theme().muted;

        v_flex()
            .flex_shrink_0()
            .w(TOOLBAR_WIDTH)
            .h_full()
            .bg(muted_bg)
            .border_l_1()
            .border_color(border_color)
            .items_center()
            .py_2()
            .gap_1()
            .child(self.render_toolbar_button(SidebarPanel::AiChat, window, cx))
            .into_any_element()
    }

    /// 渲染面板内容
    pub fn render_panel_content(
        &self,
        panel: SidebarPanel,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> AnyElement {
        match panel {
            SidebarPanel::AiChat => self.chat_panel.clone().into_any_element(),
            SidebarPanel::CellPreview => self.cell_preview_panel.clone().into_any_element(),
        }
    }
}

impl EventEmitter<DatabaseSidebarEvent> for DatabaseSidebar {}

impl Focusable for DatabaseSidebar {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for DatabaseSidebar {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let border_color = cx.theme().border;
        let bg_color = cx.theme().background;
        let show_toolbar = self.active_panel != Some(SidebarPanel::CellPreview);

        gpui_component::h_flex()
            .h_full()
            .flex_shrink_0()
            .when(show_toolbar, |this| {
                this.child(self.render_toolbar(window, cx))
            })
            .when_some(self.active_panel, |this, panel| {
                this.flex_1().child(
                    v_flex()
                        .size_full()
                        .when(show_toolbar, |this| this.border_l_1())
                        .border_color(border_color)
                        .bg(bg_color)
                        .child(self.render_panel_content(panel, window, cx)),
                )
            })
    }
}
