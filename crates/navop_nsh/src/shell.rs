use nsh_base::theme;
use nsh_core::prelude::*;

use crate::{pages, route, route::Group, route::Route};

pub(crate) fn sidebar() -> impl IntoElement {
    view()
        .w(px(222.0))
        .h_full()
        .flex_none()
        .border_r_1()
        .border_r_color(move || theme().sidebar_border)
        .bg(move || theme().sidebar)
        .child(
            view()
                .px_4()
                .pt_5()
                .pb_4()
                .gap_1()
                .child(
                    text("Navop")
                        .text_size(px(18.0))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(move || theme().sidebar_foreground),
                )
                .child(
                    text("nsh workbench")
                        .text_xs()
                        .text_color(move || theme().sidebar_accent_foreground),
                ),
        )
        .child(
            scroll().flex_1().child(
                view()
                    .px_2()
                    .pb_4()
                    .children(Group::ALL.into_iter().map(group)),
            ),
        )
        .child(
            view()
                .px_4()
                .py_3()
                .border_t_1()
                .border_t_color(move || theme().sidebar_border)
                .child(
                    text("Rust pages · nsh core")
                        .text_xs()
                        .text_color(move || theme().sidebar_accent_foreground),
                ),
        )
}

fn group(group: Group) -> impl IntoElement {
    view()
        .gap_1()
        .child(
            text(group.label())
                .px_2()
                .pt_4()
                .pb_1()
                .text_size(px(10.0))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(move || theme().sidebar_accent_foreground)
                .letter_spacing(px(0.7)),
        )
        .children(group.routes().map(nav_item))
}

fn nav_item(route: Route) -> impl IntoElement {
    let selected = move || route::current() == route;
    view()
        .h(px(34.0))
        .flex_row()
        .items_center()
        .gap_2()
        .px_3()
        .rounded(move || theme().radius)
        .role(AccessibilityRole::Button)
        .label(route.label())
        .accessibility_state(move || AccessibilityState {
            selected: selected(),
            ..AccessibilityState::default()
        })
        .bg(move || {
            if selected() {
                theme().sidebar_accent
            } else {
                theme().sidebar
            }
        })
        .hover(|s| s.bg(move || theme().sidebar_accent))
        .active(|s| s.opacity(0.7))
        .on_press(move || route::navigate(route))
        .child(view().w(px(3.0)).h(px(16.0)).rounded_full().bg(move || {
            if selected() {
                theme().sidebar_primary
            } else {
                AdaptiveColor::TRANSPARENT
            }
        }))
        .child(text(route.label()).text_sm().text_color(move || {
            if selected() {
                theme().sidebar_accent_foreground
            } else {
                theme().sidebar_foreground
            }
        }))
}

pub(crate) fn content() -> impl IntoElement {
    view()
        .flex_1()
        .h_full()
        .min_w_0()
        .overflow_hidden()
        .bg(move || theme().background)
        .child(dynamic(page))
}

fn page() -> AnyElement {
    match route::current() {
        Route::Home => pages::home().into_element(),
        Route::Workspaces => pages::workspaces().into_element(),
        Route::Connections => pages::connections().into_element(),
        Route::Notes => pages::notes().into_element(),
        Route::Json => pages::json().into_element(),
        Route::KnownHosts => pages::known_hosts().into_element(),
        Route::Settings => pages::settings().into_element(),
        Route::Database => pages::database().into_element(),
        Route::Redis => pages::redis().into_element(),
        Route::Mongo => pages::mongodb().into_element(),
        Route::Sftp => pages::sftp().into_element(),
        Route::Terminal => pages::terminal().into_element(),
        Route::PortForwarding => pages::port_forwarding().into_element(),
        Route::RemoteDesktop => pages::remote_desktop().into_element(),
        Route::Ai => pages::ai().into_element(),
        Route::Monitor => pages::monitor().into_element(),
        Route::Team => pages::team().into_element(),
        Route::Toolbox => pages::toolbox().into_element(),
        Route::SessionLogs => pages::session_logs().into_element(),
        Route::CredentialVault => pages::credentials().into_element(),
        Route::Extensions => pages::extensions().into_element(),
    }
}
