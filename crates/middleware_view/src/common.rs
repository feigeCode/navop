//! 四页共用支撑:加载状态、通知助手、分页条与表格单元格渲染。

use gpui::{
    App, AppContext, ColorExt, Div, Entity, Hsla, InteractiveElement, IntoElement, ParentElement,
    SharedString, Styled, Window, div,
};
use gpui_component::{
    ActiveTheme, Disableable, Icon, IconName, IconSize, Sizable, WindowExt,
    button::{Button, ButtonVariants as _},
    h_flex,
    notification::Notification,
    select::{SelectItem, SelectState},
};
use rust_i18n::t;

/// 页面数据加载状态
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) enum LoadState {
    /// 尚未加载
    #[default]
    Idle,
    /// 加载中
    Loading,
    /// 加载完成
    Loaded,
    /// 加载失败
    Failed(String),
}

/// 在异步上下文中推送通知(经激活窗口)
pub(crate) fn notify_async(cx: &mut gpui::AsyncApp, notification: Notification) {
    let _ = cx.update(|cx| {
        if let Some(window) = cx.active_window() {
            _ = window.update(cx, |_, window, cx| {
                window.push_notification(notification.autohide(true), cx);
            });
        }
    });
}

/// 分页条渲染参数
pub(crate) struct PaginationSpec {
    /// 结果总数
    pub total: u64,
    /// 当前页(1 起)
    pub page: usize,
    /// 每页条数
    pub page_size: usize,
    /// 后端显式报告的"还有更多"(无精确总数时用于放宽下一页禁用)
    pub has_more: bool,
}

/// 组装带"上一页/下一页"的分页信息条
pub(crate) fn pagination_bar<Prev, Next>(
    prefix: &str,
    spec: PaginationSpec,
    border_color: Hsla,
    on_prev: Prev,
    on_next: Next,
) -> impl IntoElement
where
    Prev: Fn(&gpui::ClickEvent, &mut Window, &mut App) + 'static,
    Next: Fn(&gpui::ClickEvent, &mut Window, &mut App) + 'static,
{
    let PaginationSpec {
        total,
        page,
        page_size,
        has_more,
    } = spec;
    let pages = (total as usize).div_ceil(page_size.max(1)).max(1);
    let info = t!(
        "Middleware.total_pages",
        total = total.to_string().as_str(),
        page = page.to_string().as_str(),
        pages = pages.to_string().as_str()
    )
    .to_string();
    let can_next = has_more || page < pages;
    h_flex()
        .id(SharedString::from(format!("{prefix}-pagination")))
        .w_full()
        .px_2()
        .py_1()
        .gap_2()
        .items_center()
        .border_t_1()
        .border_color(border_color)
        .child(div().flex_1())
        .child(div().text_xs().child(info))
        .child(
            Button::new(SharedString::from(format!("{prefix}-prev")))
                .small()
                .ghost()
                .label(t!("Middleware.prev_page").to_string())
                .disabled(page <= 1)
                .on_click(move |event, window, cx| on_prev(event, window, cx)),
        )
        .child(
            Button::new(SharedString::from(format!("{prefix}-next")))
                .small()
                .ghost()
                .label(t!("Middleware.next_page").to_string())
                .disabled(!can_next)
                .on_click(move |event, window, cx| on_next(event, window, cx)),
        )
}

/// 表格文本单元格(截断显示)
pub(crate) fn text_cell(text: impl Into<SharedString>) -> Div {
    div().w_full().truncate().child(text.into())
}

/// 表格空态渲染(标题 + 说明)
pub(crate) fn render_table_empty(
    title: SharedString,
    detail: SharedString,
    cx: &App,
) -> impl IntoElement {
    h_flex().size_full().items_center().justify_center().child(
        div()
            .flex()
            .flex_col()
            .items_center()
            .gap_2()
            .child(
                div()
                    .text_base()
                    .font_weight(gpui::FontWeight::SEMIBOLD)
                    .text_color(cx.theme().muted_foreground)
                    .child(title),
            )
            .child(
                div()
                    .text_sm()
                    .text_color(cx.theme().muted_foreground.opacity(0.7))
                    .child(detail),
            ),
    )
}

/// 页面工具栏(图标 + 标题 + 右侧刷新按钮)
pub(crate) fn page_header(
    id: &str,
    icon: IconName,
    title: SharedString,
    border_color: Hsla,
    on_refresh: impl Fn(&gpui::ClickEvent, &mut Window, &mut App) + 'static,
) -> impl IntoElement {
    h_flex()
        .id(SharedString::from(format!("{id}-header")))
        .w_full()
        .px_3()
        .py_2()
        .gap_2()
        .items_center()
        .border_b_1()
        .border_color(border_color)
        .child(Icon::new(icon).with_size(IconSize::Small))
        .child(
            div()
                .text_sm()
                .font_weight(gpui::FontWeight::SEMIBOLD)
                .child(title),
        )
        .child(div().flex_1())
        .child(
            Button::new(SharedString::from(format!("{id}-refresh")))
                .small()
                .ghost()
                .label(t!("Middleware.refresh").to_string())
                .on_click(move |event, window, cx| on_refresh(event, window, cx)),
        )
}

/// 能力位不支持占位(居中提示)
pub(crate) fn render_unsupported(
    cx: &App,
    title: SharedString,
    detail: SharedString,
) -> impl IntoElement {
    h_flex().size_full().items_center().justify_center().child(
        div()
            .flex()
            .flex_col()
            .items_center()
            .gap_2()
            .child(
                div()
                    .text_base()
                    .font_weight(gpui::FontWeight::SEMIBOLD)
                    .text_color(cx.theme().muted_foreground)
                    .child(title),
            )
            .child(
                div()
                    .text_sm()
                    .text_color(cx.theme().muted_foreground.opacity(0.7))
                    .child(detail),
            ),
    )
}

/// 每页条数下拉选项(10/20/50)
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct PageSizeItem(pub usize);

impl SelectItem for PageSizeItem {
    type Value = usize;

    fn title(&self) -> SharedString {
        self.0.to_string().into()
    }

    fn value(&self) -> &Self::Value {
        &self.0
    }
}

/// 创建默认选中 10 的"每页条数"下拉状态
pub(crate) fn page_size_select(
    window: &mut Window,
    cx: &mut App,
) -> Entity<SelectState<Vec<PageSizeItem>>> {
    cx.new(|cx| {
        let items = vec![PageSizeItem(10), PageSizeItem(20), PageSizeItem(50)];
        let mut state = SelectState::new(items, None, window, cx);
        state.set_selected_value(&10, window, cx);
        state
    })
}

/// 字节序列的十六进制展示("AA BB CC",空格分隔)
pub(crate) fn hex_dump(bytes: &[u8]) -> String {
    bytes
        .iter()
        .map(|byte| format!("{byte:02X}"))
        .collect::<Vec<_>>()
        .join(" ")
}
