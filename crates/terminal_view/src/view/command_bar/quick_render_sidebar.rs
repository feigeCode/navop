use super::*;
use crate::view::command_bar::quick_render::{QuickGroupSummary, group_color};
use gpui::{
    AnyElement, Context, InteractiveElement, IntoElement, ParentElement,
    StatefulInteractiveElement, Styled, div, px,
};
use gpui_component::{
    ActiveTheme, Sizable,
    button::{Button, ButtonVariants},
    h_flex, v_flex,
};
use rust_i18n::t;

const QUICK_GROUP_SIDEBAR_WIDTH: f32 = 160.0;

impl TerminalCommandBar {
    pub(super) fn render_quick_group_sidebar(&self, cx: &mut Context<Self>) -> AnyElement {
        v_flex()
            .w(px(QUICK_GROUP_SIDEBAR_WIDTH))
            .h_full()
            .flex_shrink_0()
            .overflow_hidden()
            .border_r_1()
            .border_color(self.colors.border)
            .bg(self.colors.muted)
            .p_2()
            .gap_1()
            .child(self.render_quick_sidebar_header(cx))
            .children(
                self.quick_group_summaries()
                    .into_iter()
                    .enumerate()
                    .map(|(index, group)| self.render_quick_group_filter(index, group, cx)),
            )
            .into_any_element()
    }

    fn render_quick_sidebar_header(&self, cx: &mut Context<Self>) -> AnyElement {
        h_flex()
            .h(px(32.0))
            .items_center()
            .justify_between()
            .px_2()
            .child(
                div()
                    .text_xs()
                    .font_weight(gpui::FontWeight::MEDIUM)
                    .text_color(self.colors.muted_foreground)
                    .child(t!("TerminalCommandBar.quick_commands").to_string()),
            )
            .child(
                Button::new("terminal-command-quick-close")
                    .icon(gpui_component::IconName::Close)
                    .ghost()
                    .xsmall()
                    .tooltip(t!("TerminalCommandBar.close_quick_commands").to_string())
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.toggle_quick_commands(window, cx);
                    })),
            )
            .into_any_element()
    }

    fn render_quick_group_filter(
        &self,
        index: usize,
        group: QuickGroupSummary,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let active = self.quick_group_filter == group.filter;
        let filter = group.filter.clone();
        let color = group_color(group.color.as_deref(), self.colors.accent);
        h_flex()
            .id(("terminal-command-quick-group-filter", index))
            .w_full()
            .gap_2()
            .items_center()
            .rounded(cx.theme().radius)
            .px_2()
            .py_1()
            .cursor_pointer()
            .bg(if active {
                self.colors.background
            } else {
                self.colors.muted
            })
            .text_color(if active {
                self.colors.foreground
            } else {
                self.colors.muted_foreground
            })
            .hover(|row| row.bg(self.colors.background))
            .on_click(cx.listener(move |this, _, _window, cx| {
                this.select_quick_group(filter.clone(), cx);
            }))
            .child(div().size_2().rounded_full().bg(color))
            .child(
                div()
                    .min_w_0()
                    .flex_1()
                    .truncate()
                    .text_xs()
                    .child(group.label),
            )
            .child(
                div()
                    .min_w(px(22.0))
                    .rounded_full()
                    .bg(self.colors.background)
                    .px_1()
                    .text_center()
                    .text_xs()
                    .child(group.count.to_string()),
            )
            .into_any_element()
    }
}
