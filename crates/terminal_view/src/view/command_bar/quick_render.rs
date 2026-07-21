use super::*;
use crate::view::command_bar_model::{QuickCommandGroup, group_quick_commands};
use gpui::{AnyElement, Context, InteractiveElement, IntoElement, ParentElement, Styled, px, rgb};
use gpui_component::{ActiveTheme, h_flex, v_flex};
use rust_i18n::t;

const QUICK_POPOVER_WIDTH: f32 = 720.0;
const QUICK_POPOVER_MAX_HEIGHT: f32 = 420.0;

#[derive(Clone)]
pub(super) struct QuickGroupSummary {
    pub filter: QuickGroupFilter,
    pub label: String,
    pub color: Option<String>,
    pub count: usize,
}

impl TerminalCommandBar {
    pub(super) fn render_quick_commands(&self, cx: &mut Context<Self>) -> AnyElement {
        let groups = self.filtered_quick_groups();
        let bottom_offset = if self.collapsed { 38.0 } else { 54.0 };
        v_flex()
            .absolute()
            .bottom(px(bottom_offset))
            .right_3()
            .w(px(QUICK_POPOVER_WIDTH))
            .max_w(gpui::relative(0.96))
            .max_h(px(QUICK_POPOVER_MAX_HEIGHT))
            .overflow_hidden()
            .occlude()
            .rounded(cx.theme().radius)
            .border_1()
            .border_color(self.colors.border)
            .bg(self.colors.background)
            .shadow_lg()
            .on_key_down(cx.listener(Self::handle_quick_key_down))
            .on_mouse_down_out(cx.listener(|this, _, window, cx| {
                let command_bar = cx.entity().downgrade();
                window.defer(cx, move |window, cx| {
                    let _ = command_bar.update(cx, |this, cx| {
                        if this.quick_commands_open {
                            this.close_quick_commands(window, cx);
                        }
                    });
                });
                let _ = this;
            }))
            .child(
                h_flex()
                    .h(px(QUICK_POPOVER_MAX_HEIGHT))
                    .min_h_0()
                    .child(self.render_quick_group_sidebar(cx))
                    .child(self.render_quick_body(groups, cx)),
            )
            .into_any_element()
    }

    pub(super) fn filtered_quick_groups(&self) -> Vec<QuickCommandGroup> {
        let mut groups = group_quick_commands(&self.quick_commands, &self.quick_query);
        match &self.quick_group_filter {
            QuickGroupFilter::All => groups,
            QuickGroupFilter::Ungrouped => {
                groups.retain(|group| group.name.is_none());
                groups
            }
            QuickGroupFilter::Group(name) => {
                groups.retain(|group| group.name.as_deref() == Some(name.as_str()));
                groups
            }
        }
    }

    pub(super) fn quick_group_summaries(&self) -> Vec<QuickGroupSummary> {
        let groups = group_quick_commands(&self.quick_commands, "");
        let mut summaries = vec![QuickGroupSummary {
            filter: QuickGroupFilter::All,
            label: t!("TerminalCommandBar.all_groups").to_string(),
            color: None,
            count: self.quick_commands.len(),
        }];
        summaries.extend(groups.into_iter().map(|group| {
            QuickGroupSummary {
                filter: group
                    .name
                    .as_ref()
                    .map_or(QuickGroupFilter::Ungrouped, |name| {
                        QuickGroupFilter::Group(name.clone())
                    }),
                label: group
                    .name
                    .unwrap_or_else(|| t!("TerminalCommandBar.ungrouped").to_string()),
                color: group.color,
                count: group.commands.len(),
            }
        }));
        summaries
    }
}

pub(super) fn group_color(value: Option<&str>, fallback: gpui::Hsla) -> gpui::Hsla {
    match value.unwrap_or_default() {
        "blue" => rgb(0x3b82f6).into(),
        "cyan" => rgb(0x06b6d4).into(),
        "green" => rgb(0x22c55e).into(),
        "yellow" => rgb(0xeab308).into(),
        "orange" => rgb(0xf97316).into(),
        "red" => rgb(0xef4444).into(),
        "pink" => rgb(0xec4899).into(),
        "purple" => rgb(0xa855f7).into(),
        "gray" => rgb(0x64748b).into(),
        _ => fallback,
    }
}
