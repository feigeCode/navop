use super::*;
use crate::navigation_applications::{NavigationApplication, home_applications};
use one_ui::IconSize;

impl HomePage {
    pub(super) fn render_application_workbench(
        &self,
        _window: &Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let show_team = is_feature_enabled(Feature::TeamManagement, cx);
        let applications = home_applications(show_team)
            .into_iter()
            .chain([NavigationApplication::Settings])
            .collect::<Vec<_>>();
        let mut entries = h_flex()
            .id("home-workbench-entries")
            .w_full()
            .gap_2()
            .flex_wrap();
        for application in applications {
            entries = entries.child(render_workbench_entry(application, cx));
        }
        v_flex()
            .id("home-workbench")
            .w_full()
            .gap_3()
            .p_4()
            .rounded_lg()
            .border_1()
            .border_color(cx.theme().border)
            .bg(cx.theme().background)
            .child(
                h_flex()
                    .items_center()
                    .gap_2()
                    .child(
                        div()
                            .text_base()
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(t!("Home.workbench_title").to_string()),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child(t!("Home.workbench_hint").to_string()),
                    ),
            )
            .child(entries)
            .into_any_element()
    }

    pub(super) fn render_navigation_status_panel(
        &self,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let view = cx.entity();
        let user_name = self
            .current_user
            .as_ref()
            .map(UserInfo::resolved_display_name)
            .unwrap_or_else(|| t!("Auth.login").to_string());
        let syncing = self.syncing;
        let unlocked = crypto::has_master_key();
        v_flex()
            .id("home-navigation-status")
            .w_full()
            .gap_2()
            .p_4()
            .rounded_lg()
            .border_1()
            .border_color(cx.theme().border)
            .bg(cx.theme().background)
            .child(
                div()
                    .text_base()
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(t!("Home.status").to_string()),
            )
            .child(
                h_flex()
                    .w_full()
                    .gap_2()
                    .child(status_row(
                        "home-navigation-account",
                        IconName::CircleUser,
                        user_name,
                        if self.current_user.is_some() {
                            t!("Auth.logout").to_string()
                        } else {
                            t!("Auth.login").to_string()
                        },
                        view.clone(),
                        move |home, window, cx| {
                            if home.current_user.is_some() {
                                home.sign_out(cx);
                            } else {
                                home.show_login_dialog(window, cx);
                            }
                        },
                        cx,
                    ))
                    .child(status_row(
                        "home-navigation-sync",
                        IconName::Refresh,
                        t!("Home.sync").to_string(),
                        if syncing {
                            t!("Home.syncing").to_string()
                        } else {
                            t!("Home.sync_ready").to_string()
                        },
                        view.clone(),
                        |home, window, cx| home.handle_sync_click(window, cx),
                        cx,
                    ))
                    .child(status_row(
                        "home-navigation-key",
                        IconName::Key,
                        t!("Encryption.personal_key").to_string(),
                        if unlocked {
                            t!("Home.unlock_state_unlocked").to_string()
                        } else {
                            t!("Home.unlock_state_locked").to_string()
                        },
                        view,
                        |home, window, cx| home.show_encryption_key_dialog(window, cx),
                        cx,
                    )),
            )
            .into_any_element()
    }
}

fn status_row(
    id: &'static str,
    icon: IconName,
    title: String,
    detail: String,
    view: Entity<HomePage>,
    action: impl Fn(&mut HomePage, &mut Window, &mut Context<HomePage>) + 'static,
    cx: &Context<HomePage>,
) -> AnyElement {
    h_flex()
        .id(id)
        .flex_1()
        .min_w_0()
        .gap_2()
        .p_2()
        .rounded_md()
        .cursor_pointer()
        .hover(|style| style.bg(cx.theme().muted))
        .on_click(move |_, window, cx| view.update(cx, |home, cx| action(home, window, cx)))
        .child(Icon::new(icon).with_size(IconSize::Small))
        .child(
            v_flex()
                .min_w_0()
                .gap_0p5()
                .child(div().text_sm().child(title))
                .child(
                    div()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child(detail),
                ),
        )
        .into_any_element()
}

fn render_workbench_entry(
    application: NavigationApplication,
    cx: &Context<HomePage>,
) -> AnyElement {
    let id = format!("home-workbench-{:?}", application);
    let title = application.label();
    let icon = application.icon();
    let view = cx.entity();
    let click_view = view.clone();
    div()
        .id(id)
        .w(px(112.0))
        .h(px(82.0))
        .flex_shrink_0()
        .p_2()
        .flex()
        .flex_col()
        .items_center()
        .justify_center()
        .gap_1()
        .rounded_lg()
        .cursor_pointer()
        .focusable()
        .hover(|style| style.bg(cx.theme().muted))
        .focus_visible(|style| style.bg(cx.theme().muted))
        .on_click(move |_, window, cx| {
            click_view.update(cx, |home, cx| {
                if application == NavigationApplication::Settings {
                    home.add_settings_tab(window, cx);
                } else {
                    home.activate_navigation_application(application, window, cx);
                }
            });
        })
        .on_key_down(move |event, window, cx| {
            if matches!(event.keystroke.key.as_str(), "enter" | " ") {
                view.update(cx, |home, cx| {
                    if application == NavigationApplication::Settings {
                        home.add_settings_tab(window, cx);
                    } else {
                        home.activate_navigation_application(application, window, cx);
                    }
                });
            }
        })
        .child(
            div()
                .size(px(36.0))
                .flex()
                .items_center()
                .justify_center()
                .rounded_lg()
                .bg(cx.theme().muted)
                .child(Icon::new(icon).mono().with_size(IconSize::Medium)),
        )
        .child(div().text_sm().child(title))
        .into_any_element()
}
