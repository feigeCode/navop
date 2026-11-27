use nsh_base::{ButtonElement, button, theme};
use nsh_core::prelude::*;

pub(crate) fn initialize() {
    // Reading the route in the root initializes its surface-local signal before
    // the first sidebar frame.
    let _ = crate::route::current();
}

pub(crate) fn page_header(
    title: &'static str,
    description: impl Into<SharedString>,
) -> impl IntoElement {
    let description = description.into();
    view()
        .flex_none()
        .gap_1()
        .px_6()
        .pt_5()
        .pb_4()
        .border_b_1()
        .border_b_color(move || theme().border)
        .child(
            text(title)
                .text_size(px(20.0))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(move || theme().foreground),
        )
        .child(
            text(description)
                .text_sm()
                .text_color(move || theme().muted_foreground),
        )
}

pub(crate) fn card() -> View {
    view()
        .p_4()
        .gap_3()
        .rounded(px(12.0))
        .border_1()
        .border_color(move || theme().border)
        .bg(move || theme().card)
}

pub(crate) fn primary_button(label: &'static str) -> ButtonElement {
    button()
        .px_4()
        .py_2()
        .rounded(move || theme().radius)
        .bg(move || theme().primary)
        .active(|s| s.opacity(0.7))
        .child(
            text(label)
                .text_sm()
                .font_weight(FontWeight::MEDIUM)
                .text_color(move || theme().primary_foreground),
        )
}

pub(crate) fn secondary_button(label: &'static str) -> ButtonElement {
    button()
        .px_4()
        .py_2()
        .rounded(move || theme().radius)
        .border_1()
        .border_color(move || theme().border)
        .bg(move || theme().background)
        .active(|s| s.bg(move || theme().muted))
        .child(text(label).text_sm().text_color(move || theme().foreground))
}

pub(crate) fn stat(
    label: impl Into<SharedString>,
    value: impl Into<SharedString>,
) -> impl IntoElement {
    let label = label.into();
    let value = value.into();
    card()
        .flex_1()
        .min_w(px(150.0))
        .child(
            text(value)
                .text_size(px(26.0))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(move || theme().foreground),
        )
        .child(
            text(label)
                .text_sm()
                .text_color(move || theme().muted_foreground),
        )
}
