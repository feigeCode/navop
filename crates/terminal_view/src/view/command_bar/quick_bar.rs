//! 命令栏折叠行里的快捷指令横条。
//!
//! 这一行本身就是快捷指令栏：有置顶（pinned）指令时直接横向展示 chips；
//! 一旦用户给指令分了组（#486 希望像 SecureCRT 的按钮组那样直接点底部按钮），
//! 就按「分组标题 + 组内指令」平铺全部指令，组内仍把置顶指令排在最前。
//! 点击 chip 复用
//! [`TerminalCommandBar::choose_command`]，因此 {{参数}} 填写、自动执行标记与
//! 折叠态写入 PTY 的行为和弹层列表完全一致。

use super::*;
use crate::view::command_bar::quick_render::group_color;
use crate::view::command_bar_model::{QuickCommandGroup, group_quick_commands};
use gpui::{
    AnyElement, Context, Hsla, InteractiveElement, IntoElement, ParentElement,
    StatefulInteractiveElement, Styled, div, prelude::FluentBuilder as _, px,
};
use gpui_component::{ActiveTheme, Icon, Sizable as _, h_flex, tooltip::Tooltip};
use one_assets::IconName;
use rust_i18n::t;

const QUICK_CHIP_HEIGHT: f32 = 24.0;
const QUICK_CHIP_MAX_WIDTH: f32 = 180.0;

/// 过滤出需要常驻展示的置顶快捷指令，并保留仓储层已排序的顺序。
pub(super) fn pinned_quick_commands(commands: &[QuickCommand]) -> Vec<&QuickCommand> {
    commands.iter().filter(|command| command.pinned).collect()
}

/// chip 文案优先使用名称；没有名称时退回命令首行，避免多行命令撑破横条。
pub(super) fn quick_command_chip_label(command: &QuickCommand) -> String {
    command
        .name
        .as_deref()
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| {
            command
                .command
                .lines()
                .next()
                .unwrap_or_default()
                .trim()
                .to_string()
        })
}

/// 底部横条按分组平铺指令时的分组顺序与组内顺序。
///
/// 组内把置顶指令排在最前（保持仓储层给出的相对顺序），这样「置顶优先」与
/// 「分组可见」两个诉求可以同时成立。
pub(super) fn quick_bar_groups(commands: &[QuickCommand]) -> Vec<QuickCommandGroup> {
    let mut groups = group_quick_commands(commands, "");
    for group in &mut groups {
        group.commands.sort_by_key(|command| !command.pinned);
    }
    groups
}

/// 是否至少有一条指令属于具名分组；没有分组时沿用只显示置顶指令的旧行为。
pub(super) fn has_named_quick_group(commands: &[QuickCommand]) -> bool {
    commands.iter().any(|command| {
        command
            .group_name
            .as_deref()
            .map(str::trim)
            .is_some_and(|name| !name.is_empty())
    })
}

/// 分组的展示名；未分组指令返回 `None`，横条上不画标题。
pub(super) fn quick_bar_group_label(group: &QuickCommandGroup) -> Option<String> {
    group
        .name
        .as_deref()
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .map(str::to_string)
}

fn quick_command_chip_tooltip(command: &QuickCommand) -> String {
    let mut text = if command
        .name
        .as_deref()
        .is_some_and(|name| !name.trim().is_empty())
    {
        format!(
            "{}\n{}",
            quick_command_chip_label(command),
            command.command.trim()
        )
    } else {
        command.command.trim().to_string()
    };
    if let Some(description) = command
        .description
        .as_deref()
        .map(str::trim)
        .filter(|description| !description.is_empty())
    {
        text.push('\n');
        text.push_str(description);
    }
    text
}

impl TerminalCommandBar {
    /// 还没有置顶指令时的占位提示：告诉用户置顶后会显示在这里。
    fn render_quick_bar_hint(&self) -> AnyElement {
        h_flex()
            .id("terminal-quick-bar-hint")
            .flex_1()
            .min_w_0()
            .items_center()
            .gap_1()
            .px_1()
            .tooltip(|window, cx| {
                Tooltip::new(t!("TerminalCommandBar.quick_bar_hint_tooltip").to_string())
                    .build(window, cx)
            })
            .child(
                Icon::new(IconName::Pin)
                    .xsmall()
                    .text_color(self.colors.muted_foreground),
            )
            .child(
                div()
                    .min_w_0()
                    .truncate()
                    .text_xs()
                    .text_color(self.colors.muted_foreground)
                    .child(t!("TerminalCommandBar.quick_bar_hint").to_string()),
            )
            .into_any_element()
    }

    /// 折叠行左侧的 chip 滚动区。
    pub(super) fn render_quick_command_chips(&self, cx: &mut Context<Self>) -> AnyElement {
        if self.quick_commands.is_empty() {
            return self.render_quick_bar_hint().into_any_element();
        }

        let mut chips = Vec::new();
        if has_named_quick_group(&self.quick_commands) {
            for (group_index, group) in quick_bar_groups(&self.quick_commands)
                .into_iter()
                .enumerate()
            {
                if let Some(label) = quick_bar_group_label(&group) {
                    chips.push(self.render_quick_group_label(group_index, label, group.color));
                }
                for command in &group.commands {
                    chips.push(self.render_quick_command_chip(chips.len(), command, cx));
                }
            }
        } else {
            for command in pinned_quick_commands(&self.quick_commands) {
                chips.push(self.render_quick_command_chip(chips.len(), command, cx));
            }
        }

        if chips.is_empty() {
            return self.render_quick_bar_hint().into_any_element();
        }

        h_flex()
            .id("terminal-quick-bar-scroll")
            .flex_1()
            .min_w_0()
            .items_center()
            .gap_1()
            .overflow_x_scroll()
            .track_scroll(&self.quick_bar_scroll)
            .children(chips)
            .into_any_element()
    }

    /// 分组标题：只在用户真的建过分组时出现，颜色与分组色一致。
    fn render_quick_group_label(
        &self,
        index: usize,
        label: String,
        color: Option<String>,
    ) -> AnyElement {
        let dot = group_color(color.as_deref(), self.colors.accent);
        h_flex()
            .id(("terminal-quick-bar-group", index))
            .flex_shrink_0()
            .h(px(QUICK_CHIP_HEIGHT))
            .items_center()
            .gap_1()
            .px_1()
            .child(div().size_1p5().flex_shrink_0().rounded_full().bg(dot))
            .child(
                div()
                    .min_w_0()
                    .truncate()
                    .text_xs()
                    .text_color(self.colors.muted_foreground)
                    .child(label),
            )
            .into_any_element()
    }

    /// 单条指令 chip：名称/工具提示/分组圆点与弹层列表保持一致。
    fn render_quick_command_chip(
        &self,
        index: usize,
        command: &QuickCommand,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let border = self.colors.border;
        let muted = self.colors.muted;
        let foreground = self.colors.foreground;
        let radius = cx.theme().radius;
        let label = quick_command_chip_label(command);
        let tooltip = quick_command_chip_tooltip(command);
        let has_group = command.group_name.is_some() || command.group_color.is_some();
        let dot: Option<Hsla> =
            has_group.then(|| group_color(command.group_color.as_deref(), self.colors.accent));
        let value = command.command.clone();
        h_flex()
            .id(("terminal-quick-bar-item", index))
            .flex_shrink_0()
            .h(px(QUICK_CHIP_HEIGHT))
            .max_w(px(QUICK_CHIP_MAX_WIDTH))
            .min_w_0()
            .items_center()
            .gap_1()
            .px_2()
            .rounded(radius)
            .border_1()
            .border_color(border)
            .cursor_pointer()
            .overflow_hidden()
            .hover(move |chip| chip.bg(muted))
            .tooltip(move |window, cx| Tooltip::new(tooltip.clone()).build(window, cx))
            .on_click(cx.listener(move |this, _, window, cx| {
                this.choose_command(value.clone(), window, cx);
            }))
            .when_some(dot, |chip, dot| {
                chip.child(div().size_1p5().flex_shrink_0().rounded_full().bg(dot))
            })
            .child(
                div()
                    .min_w_0()
                    .truncate()
                    .text_xs()
                    .font_family("monospace")
                    .text_color(foreground)
                    .child(label),
            )
            .into_any_element()
    }
}

#[cfg(test)]
#[path = "quick_bar_tests.rs"]
mod tests;
