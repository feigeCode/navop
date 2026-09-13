use super::*;
use crate::navigation_applications::{NavigationApplication, home_applications};
use one_ui::IconSize;

impl HomePage {
    pub(super) fn render_application_workbench(
        &self,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let show_team = is_feature_enabled(Feature::TeamManagement, cx);
        let mut cards = h_flex()
            .id("home-workbench-cards")
            .w_full()
            .gap_3()
            .flex()
            .flex_wrap();
        let card_width =
            ((window.bounds().size.width - px(328.0)) / 3.0).clamp(px(220.0), px(360.0));
        for application in home_applications(show_team) {
            cards = cards.child(render_workbench_card(application, card_width, cx));
        }
        cards = cards.child(render_workbench_card(
            NavigationApplication::Settings,
            card_width,
            cx,
        ));
        v_flex()
            .id("home-workbench")
            .w_full()
            .gap_2()
            .child(
                h_flex()
                    .items_center()
                    .gap_2()
                    .child(
                        div()
                            .text_lg()
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(t!("Home.workbench_title").to_string()),
                    )
                    .child(
                        div()
                            .text_sm()
                            .text_color(cx.theme().muted_foreground)
                            .child(t!("Home.workbench_hint").to_string()),
                    ),
            )
            .child(cards)
            .into_any_element()
    }
}

fn render_workbench_card(
    application: NavigationApplication,
    width: Pixels,
    cx: &Context<HomePage>,
) -> AnyElement {
    let id = format!("home-workbench-{:?}", application);
    let title = application.label();
    let icon = application.icon();
    let title_for_key = title.clone();
    let on_click = cx.listener(move |home, _, window, cx| {
        if application == NavigationApplication::Settings {
            home.add_settings_tab(window, cx);
        } else {
            home.activate_navigation_application(application, window, cx);
        }
    });
    div()
        .id(id)
        .w(width)
        .max_w(px(360.0))
        .min_h(px(112.0))
        .flex_shrink_0()
        .p_4()
        .rounded_lg()
        .border_1()
        .border_color(cx.theme().border)
        .bg(cx.theme().background)
        .cursor_pointer()
        .focusable()
        .hover(|style| style.border_color(cx.theme().primary))
        .focus_visible(|style| style.border_color(cx.theme().primary))
        .on_click(on_click)
        .child(
            h_flex()
                .items_start()
                .gap_3()
                .child(
                    div()
                        .size(px(34.0))
                        .flex_shrink_0()
                        .flex()
                        .items_center()
                        .justify_center()
                        .rounded(px(9.0))
                        .bg(cx.theme().muted)
                        .child(Icon::new(icon).mono().with_size(IconSize::Medium)),
                )
                .child(
                    v_flex()
                        .gap_1()
                        .child(
                            div()
                                .text_base()
                                .font_weight(FontWeight::MEDIUM)
                                .child(title_for_key),
                        )
                        .child(
                            div()
                                .text_xs()
                                .text_color(cx.theme().muted_foreground)
                                .child(t!("Home.workbench_entry_hint").to_string()),
                        ),
                )
                .child(div().flex_1())
                .child(
                    Icon::new(IconName::ArrowRight)
                        .with_size(IconSize::Small)
                        .text_color(cx.theme().muted_foreground),
                ),
        )
        .into_any_element()
}
