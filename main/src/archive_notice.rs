use gpui::{App, IntoElement, ParentElement, Styled, Window, div, px};
use gpui_component::{
    ActiveTheme, StyledExt as _, WindowExt,
    button::{Button, ButtonVariants as _},
    h_flex, v_flex,
};
use rust_i18n::t;

pub const NAVOP_WEBSITE_URL: &str = "https://navop.dev";
pub const NAVOP_GITHUB_URL: &str = "https://github.com/feigeCode/navop";

pub fn schedule_archive_notice(window: &mut Window, cx: &mut App) {
    window.defer(cx, |window, cx| {
        window.open_dialog(cx, |dialog, _window, cx| {
            dialog
                .title(t!("ArchiveNotice.title").to_string())
                .w(px(540.0))
                .overlay_closable(false)
                .child(notice_body(cx))
                .footer_element(notice_footer())
        });
    });
}

fn notice_body(cx: &App) -> impl IntoElement {
    v_flex()
        .gap_4()
        .child(
            div()
                .self_start()
                .px_3()
                .py_1()
                .rounded_full()
                .bg(cx.theme().warning.opacity(0.14))
                .text_color(cx.theme().warning)
                .text_sm()
                .font_semibold()
                .child(t!("ArchiveNotice.badge").to_string()),
        )
        .child(
            div()
                .text_base()
                .line_height(px(24.0))
                .child(t!("ArchiveNotice.message").to_string()),
        )
        .child(
            v_flex()
                .gap_2()
                .p_4()
                .rounded_lg()
                .border_1()
                .border_color(cx.theme().border)
                .bg(cx.theme().secondary.opacity(0.45))
                .child(div().text_lg().font_semibold().child("Navop"))
                .child(
                    div()
                        .text_sm()
                        .text_color(cx.theme().muted_foreground)
                        .child(t!("ArchiveNotice.navop_description").to_string()),
                )
                .child(
                    div()
                        .text_sm()
                        .text_color(cx.theme().primary)
                        .child(NAVOP_WEBSITE_URL),
                ),
        )
}

fn notice_footer() -> impl IntoElement {
    h_flex()
        .w_full()
        .justify_between()
        .gap_2()
        .child(
            Button::new("archive-notice-continue")
                .label(t!("ArchiveNotice.continue_action").to_string())
                .on_click(|_, window, cx| window.close_dialog(cx)),
        )
        .child(
            h_flex()
                .gap_2()
                .child(
                    Button::new("archive-notice-github")
                        .outline()
                        .label(t!("ArchiveNotice.github_action").to_string())
                        .on_click(|_, _, cx| cx.open_url(NAVOP_GITHUB_URL)),
                )
                .child(
                    Button::new("archive-notice-website")
                        .primary()
                        .label(t!("ArchiveNotice.website_action").to_string())
                        .on_click(|_, _, cx| cx.open_url(NAVOP_WEBSITE_URL)),
                ),
        )
}
