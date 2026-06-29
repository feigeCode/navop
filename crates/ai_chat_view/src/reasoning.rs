use crate::{ChatMessageUIGeneric, MessageExtension};
use gpui::prelude::FluentBuilder;
use gpui::{AnyElement, App, IntoElement, ParentElement, SharedString, Styled, Window, div};
use gpui_component::{
    ActiveTheme, IconName, Sizable,
    button::{Button, ButtonVariants},
    h_flex,
    text::TextView,
    v_flex,
};

pub fn render_reasoning_block<E: MessageExtension>(
    msg: &ChatMessageUIGeneric<E>,
    window: &mut Window,
    cx: &mut App,
) -> AnyElement {
    let state_id = SharedString::from(format!("reasoning-expanded-{}", msg.id));
    let expanded_state = window.use_keyed_state(state_id, cx, |_, _| {
        msg.is_streaming || msg.is_reasoning_expanded
    });
    let is_expanded = *expanded_state.read(cx);

    v_flex()
        .w_full()
        .min_w_0()
        .gap_1()
        .pl_2()
        .border_l_2()
        .border_color(cx.theme().border.opacity(0.7))
        .child(reasoning_header(msg, is_expanded, expanded_state, cx))
        .when(is_expanded, |this| this.child(reasoning_body(msg, cx)))
        .into_any_element()
}

fn reasoning_header<E: MessageExtension>(
    msg: &ChatMessageUIGeneric<E>,
    is_expanded: bool,
    expanded_state: gpui::Entity<bool>,
    cx: &mut App,
) -> AnyElement {
    let icon = if is_expanded {
        IconName::ChevronDown
    } else {
        IconName::ChevronRight
    };
    let tooltip = if is_expanded {
        "收起思考"
    } else {
        "展开思考"
    };

    h_flex()
        .w_full()
        .min_w_0()
        .items_center()
        .gap_1()
        .child(
            Button::new(SharedString::from(format!("reasoning-toggle-{}", msg.id)))
                .ghost()
                .xsmall()
                .icon(icon)
                .tooltip(tooltip)
                .on_click(move |_, _, cx| {
                    expanded_state.update(cx, |expanded, cx| {
                        *expanded = !*expanded;
                        cx.notify();
                    });
                }),
        )
        .child(
            div()
                .flex_1()
                .min_w_0()
                .text_xs()
                .text_color(cx.theme().muted_foreground)
                .truncate()
                .child("思考过程"),
        )
        .into_any_element()
}

fn reasoning_body<E: MessageExtension>(msg: &ChatMessageUIGeneric<E>, cx: &App) -> AnyElement {
    div()
        .w_full()
        .min_w_0()
        .pl_6()
        .pr_2()
        .pb_1()
        .text_xs()
        .text_color(cx.theme().muted_foreground)
        .child(
            TextView::markdown(
                SharedString::from(format!("reasoning-msg-{}", msg.id)),
                msg.reasoning_content.clone(),
            )
            .text_xs()
            .selectable(true),
        )
        .into_any_element()
}
