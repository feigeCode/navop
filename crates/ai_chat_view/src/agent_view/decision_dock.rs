//! 决策栏（`DecisionDock`）：输入区上方的待审批条。
//!
//! 与时间线里的审批卡**共享同一决策源**——两者都读
//! [`AgentTranscript::pending_decisions`]，点下去派发的也是内联卡片用的**同一个 action**。
//! 因此：两处永远是同一个决策、只按一次就生效、重复点击天然幂等。
//!
//! 纪律（照方案 §6.4）：
//! - **三个授权域不合并**：本地工具 / ACP permission / Public MCP 各自走各自的入口，这里只做来源标注；
//! - **不虚构选项**：provider 没下发的按钮就不显示（而不是显示了不生效）；
//! - **不抢焦点**：详情展开是纯展示动作，绝不把用户打字时的 Enter 变成「允许」；
//! - **敏感参数只展示已脱敏的版本**（脱敏发生在写卡片时，这里不再加工）。

use gpui::prelude::FluentBuilder;
use gpui::{
    AnyElement, InteractiveElement, IntoElement, ParentElement, SharedString, Styled, div,
};
use gpui_component::{h_flex, v_flex};
use one_assets::IconName;
use rust_i18n::t;

use super::*;
use crate::pending_decision::{DecisionAuthority, DecisionOption, PendingDecision};

/// 详情区展示的脱敏 JSON 上限（字符）。
const MAX_DETAIL_CHARS: usize = 2000;

impl AgentChatView {
    /// 决策栏。没有未决决策时返回 `None`（不占位、不留空条）。
    pub(super) fn render_decision_dock(
        &self,
        theme: &AgentChatTheme,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let decisions = self.transcript.pending_decisions();
        let (current, rest) = decisions.split_first()?;
        let warning = cx.theme().warning;
        let expanded = self.decision_details.as_deref() == Some(current.id.as_str());
        let has_options = !current.options.is_empty();

        let buttons = h_flex()
            .flex_shrink_0()
            .gap_1()
            .children(
                current
                    .options
                    .iter()
                    .map(|option| decision_option_button(current, option)),
            );

        let details = expanded.then(|| render_details(current, theme, cx));
        let toggle_id = current.id.clone();

        Some(
            div()
                .debug_selector(|| "ai-chat-decision-dock".to_string())
                .w_full()
                .min_w_0()
                .flex_shrink_0()
                .border_b_1()
                .border_color(warning)
                .bg(warning.opacity(0.12))
                .child(
                    h_flex()
                        .w_full()
                        .min_w_0()
                        .items_start()
                        .gap_2()
                        .px_3()
                        .py_2()
                        .child(
                            Icon::new(IconName::TriangleAlert)
                                .small()
                                .text_color(warning)
                                .flex_shrink_0(),
                        )
                        .child(
                            v_flex()
                                .flex_1()
                                .min_w_0()
                                .gap_0p5()
                                .child(
                                    div()
                                        .w_full()
                                        .min_w_0()
                                        .truncate()
                                        .text_sm()
                                        .text_color(theme.foreground)
                                        .child(
                                            t!(
                                                "AgentUi.decision_dock_title",
                                                target = current.target_label()
                                            )
                                            .to_string(),
                                        ),
                                )
                                .child(
                                    div().w_full().text_xs().text_color(theme.muted_foreground).child(
                                        t!("AgentUi.decision_dock_subtitle").to_string(),
                                    ),
                                )
                                .child(
                                    div().w_full().text_xs().text_color(theme.muted_foreground).child(
                                        t!(
                                            "AgentUi.decision_authority_note",
                                            authority = t!(current.authority.label_key())
                                        )
                                        .to_string(),
                                    ),
                                )
                                .when_some(details, |this, details| this.child(details)),
                        )
                        .child(authority_chip(current.authority, theme))
                        .when(has_options, |this| {
                            this.child(
                                Button::new(SharedString::from(format!(
                                    "decision-details-{toggle_id}"
                                )))
                                .small()
                                .outline()
                                .label(
                                    if expanded {
                                        t!("AgentUi.decision_hide_details")
                                    } else {
                                        t!("AgentUi.decision_show_details")
                                    }
                                    .to_string(),
                                )
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.decision_details =
                                        if this.decision_details.as_deref() == Some(toggle_id.as_str())
                                        {
                                            None
                                        } else {
                                            Some(toggle_id.clone())
                                        };
                                    cx.notify();
                                })),
                            )
                        })
                        .when(rest.len() > 0, |this| {
                            this.child(
                                div()
                                    .flex_shrink_0()
                                    .text_xs()
                                    .text_color(theme.muted_foreground)
                                    .child(
                                        t!("AgentUi.decision_dock_more", count = rest.len())
                                            .to_string(),
                                    ),
                            )
                        })
                        .child(buttons),
                )
                .into_any_element(),
        )
    }
}

/// 授权域标注。纯展示，不参与解析。
fn authority_chip(authority: DecisionAuthority, theme: &AgentChatTheme) -> AnyElement {
    let color = match authority {
        DecisionAuthority::LocalTool => theme.muted_foreground,
        DecisionAuthority::AcpPermission => theme.accent,
        DecisionAuthority::PublicMcp => theme.muted_foreground,
    };
    div()
        .flex_shrink_0()
        .px_2()
        .py_0p5()
        .rounded_full()
        .border_1()
        .border_color(color)
        .text_xs()
        .text_color(color)
        .child(t!(authority.label_key()).to_string())
        .into_any_element()
}

fn render_details(
    decision: &PendingDecision,
    theme: &AgentChatTheme,
    cx: &mut Context<AgentChatView>,
) -> AnyElement {
    let body = if decision.input_json.trim().is_empty() {
        decision.question.clone()
    } else {
        truncate_chars(&decision.input_json, MAX_DETAIL_CHARS)
    };
    v_flex()
        .debug_selector(|| "ai-chat-decision-details".to_string())
        .w_full()
        .min_w_0()
        .gap_1()
        .pt_1()
        .child(
            div()
                .w_full()
                .min_w_0()
                .rounded_md()
                .border_1()
                .border_color(theme.border)
                .bg(theme.background)
                .p_2()
                .text_xs()
                .text_color(theme.foreground)
                .font_family(cx.theme().mono_font_family.clone())
                .child(body),
        )
        .child(
            div()
                .w_full()
                .text_xs()
                .text_color(theme.muted_foreground)
                .child(t!("AgentUi.decision_details_masked").to_string()),
        )
        .child(
            div()
                .w_full()
                .truncate()
                .text_xs()
                .text_color(theme.muted_foreground)
                .child(t!("AgentUi.decision_request_id", id = decision.id.as_str()).to_string()),
        )
        .into_any_element()
}

/// 选项按钮：派发的是内联卡片用的**同一个 action**，所以两处共享同一个命令入口。
fn decision_option_button(decision: &PendingDecision, option: &DecisionOption) -> Button {
    let id = decision.id.clone();
    let option_id = option.option_id.clone();
    let authority = decision.authority;
    let selector = format!("decision-option-{}-{}", decision.id, option.option_id);
    let mut button = Button::new(SharedString::from(selector.clone()))
        .debug_selector(move || selector.clone())
        .small()
        .label(option.name.clone())
        .on_click(move |_, window, cx| {
            dispatch_decision(authority, &id, &option_id, window, cx);
        });
    button = if option.is_reject() {
        button.danger()
    } else {
        button.primary()
    };
    button
}

/// 唯一的落地入口：把决策转成对应的 action。
///
/// 三个授权域各派各的 action：本地工具 / Public MCP 走 `ApproveToolCall` /
/// `RejectToolCall`——`resolve_pending_tool_action` 内部先查 Public MCP 待批表、
/// 再查本地工具确认卡，因此这里不需要（也不应该）重复判断授权域；
/// ACP 走 `SelectAcpPermissionOption` 并原样回传 provider 的 `option_id`。
fn dispatch_decision(
    authority: DecisionAuthority,
    id: &str,
    option_id: &str,
    window: &mut Window,
    cx: &mut App,
) {
    if authority.is_acp() {
        window.dispatch_action(
            Box::new(SelectAcpPermissionOption {
                request_id: id.to_string(),
                option_id: option_id.to_string(),
            }),
            cx,
        );
        return;
    }
    if option_id == crate::pending_decision::LOCAL_TOOL_ALLOW_OPTION {
        window.dispatch_action(
            Box::new(ApproveToolCall {
                call_id: id.to_string(),
            }),
            cx,
        );
    } else {
        window.dispatch_action(
            Box::new(RejectToolCall {
                call_id: id.to_string(),
            }),
            cx,
        );
    }
}

fn truncate_chars(text: &str, max_chars: usize) -> String {
    if text.chars().count() <= max_chars {
        return text.to_string();
    }
    text.chars().take(max_chars).collect()
}
