use gpui::{
    AnyElement, FontWeight, InteractiveElement, IntoElement, ParentElement, SharedString,
    StatefulInteractiveElement, Styled, Window, div, prelude::FluentBuilder as _, px,
};
use gpui_component::{
    ActiveTheme, Icon, IconName, IconSize, Sizable, StyledExt,
    button::{Button, ButtonVariants as _},
    h_flex, v_flex,
};
use one_core::storage::StoredConnection;
use rust_i18n::t;

use super::{
    HomePage,
    modern_home_shortcuts::{new_connection_shortcut, quick_open_shortcut, terminal_shortcut},
};
use crate::connection_visuals::ConnectionVisualSize;
use crate::home::connection_import_window::show_connection_import_window;

const START_CENTER_MAX_WIDTH: gpui::Pixels = px(1040.0);
const START_CENTER_MAIN_COLUMN_WIDTH: gpui::Pixels = px(580.0);
const START_CENTER_SIDE_COLUMN_WIDTH: gpui::Pixels = px(300.0);

impl HomePage {
    pub(super) fn render_modern_home(
        &self,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) -> AnyElement {
        let view = cx.entity();

        div()
            .id("modern-home-start-center")
            .size_full()
            .overflow_y_scroll()
            .scrollbar_width(px(0.0))
            .child(
                v_flex().w_full().items_center().px_5().py_3().child(
                    v_flex()
                        .w_full()
                        .max_w(START_CENTER_MAX_WIDTH)
                        .gap_3()
                        .child(self.render_start_center_hero(view, window, cx))
                        .child(
                            h_flex()
                                .w_full()
                                .min_w_0()
                                .items_stretch()
                                .flex_wrap()
                                .gap_3()
                                .child(
                                    div()
                                        .id("modern-home-recent-column")
                                        .min_w_0()
                                        .flex_basis(START_CENTER_MAIN_COLUMN_WIDTH)
                                        .flex_grow(2.0)
                                        .child(self.render_recent_connections_panel(window, cx)),
                                )
                                .child(
                                    v_flex()
                                        .id("modern-home-side-column")
                                        .min_w_0()
                                        .flex_basis(START_CENTER_SIDE_COLUMN_WIDTH)
                                        .flex_grow(1.0)
                                        .gap_3()
                                        .child(render_create_panel(cx.entity(), window, cx))
                                        .child(render_workspace_tools(cx.entity(), window, cx))
                                        .child(render_status_panel(
                                            self.syncing,
                                            cx.entity(),
                                            window,
                                            cx,
                                        )),
                                ),
                        ),
                ),
            )
            .into_any_element()
    }

    fn render_start_center_hero(
        &self,
        view: gpui::Entity<HomePage>,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) -> impl IntoElement {
        v_flex()
            .id("modern-home-hero")
            .w_full()
            .min_w_0()
            .gap_3()
            .px_5()
            .py_4()
            .rounded_xl()
            .border_1()
            .border_color(cx.theme().border)
            .bg(cx.theme().muted)
            .child(render_brand(cx))
            .child(
                h_flex()
                    .w_full()
                    .min_w_0()
                    .items_center()
                    .flex_wrap()
                    .gap_3()
                    .child(
                        h_flex()
                            .flex_none()
                            .items_center()
                            .gap_2()
                            .child(
                                Button::new("modern-home-new-connection")
                                    .icon(IconName::Plus)
                                    .primary()
                                    .large()
                                    .label(t!("Home.new_connection"))
                                    .on_click(window.listener_for(&view, |home, _, window, cx| {
                                        home.show_new_connection_dialog(window, cx);
                                    })),
                            )
                            .child(new_connection_shortcut(cx)),
                    )
                    .child(
                        h_flex()
                            .flex_none()
                            .items_center()
                            .gap_2()
                            .child(self.render_local_terminal_button(window, cx))
                            .child(terminal_shortcut(cx)),
                    )
                    .child(
                        h_flex()
                            .flex_none()
                            .items_center()
                            .gap_2()
                            .child(
                                Button::new("modern-home-quick-open")
                                    .icon(IconName::Search)
                                    .outline()
                                    .label(t!("Home.StartCenter.quick_open"))
                                    .on_click(window.listener_for(&view, |home, _, window, cx| {
                                        home.show_connection_quick_open(window, cx);
                                    })),
                            )
                            .child(quick_open_shortcut(cx)),
                    )
                    .text_color(cx.theme().foreground),
            )
    }

    /// Recently opened connections, most recent first, so the home page works
    /// as a dashboard instead of a splash screen. The panel remains visible
    /// when empty to preserve the start center's task hierarchy.
    fn render_recent_connections_panel(
        &self,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) -> impl IntoElement {
        let mut recent: Vec<StoredConnection> = self
            .connections
            .iter()
            .filter(|conn| conn.last_used_at.is_some())
            .cloned()
            .collect();
        recent.sort_by_key(|conn| std::cmp::Reverse(conn.last_used_at));
        recent.truncate(8);
        let recent_count = recent.len();

        surface_panel("modern-home-recent-panel", cx)
            .child(
                panel_header(
                    t!("Home.StartCenter.recent"),
                    Some(recent_count.to_string()),
                    cx,
                )
                .child(
                    div()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child(t!("Home.StartCenter.recent_description")),
                ),
            )
            .child(if recent.is_empty() {
                render_empty_recent(cx).into_any_element()
            } else {
                v_flex()
                    .w_full()
                    .gap_1()
                    .children(
                        recent
                            .into_iter()
                            .map(|conn| self.render_recent_connection_row(conn, window, cx)),
                    )
                    .into_any_element()
            })
    }

    fn render_recent_connection_row(
        &self,
        conn: StoredConnection,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) -> AnyElement {
        let icon = self.connection_icon(&conn, ConnectionVisualSize::Inline);
        let name = conn.name.clone();
        let type_label = conn.connection_type.label().to_string();
        let open_connection = conn.clone();
        let hover_border = cx.theme().list_active_border;
        let hover_background = cx.theme().muted;

        h_flex()
            .id(SharedString::from(format!(
                "recent-conn-{}",
                conn.id.unwrap_or(0)
            )))
            .w_full()
            .min_w_0()
            .min_h(px(50.0))
            .items_center()
            .gap_2()
            .px_3()
            .py_1()
            .rounded_lg()
            .border_1()
            .border_color(cx.theme().border)
            .bg(cx.theme().background)
            .cursor_pointer()
            .hover(move |style| style.bg(hover_background).border_color(hover_border))
            .on_click(
                window.listener_for(&cx.entity(), move |home, _, window, cx| {
                    home.open_connection_from_quick(&open_connection, window, cx);
                }),
            )
            .child(
                div()
                    .flex_none()
                    .size(px(32.0))
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded_md()
                    .bg(cx.theme().secondary)
                    .text_color(cx.theme().secondary_foreground)
                    .child(icon),
            )
            .child(
                v_flex()
                    .min_w_0()
                    .flex_grow(1.0)
                    .gap_0p5()
                    .child(
                        div()
                            .text_sm()
                            .font_semibold()
                            .overflow_hidden()
                            .text_ellipsis()
                            .whitespace_nowrap()
                            .child(name),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child(type_label),
                    ),
            )
            .child(
                Icon::new(IconName::ChevronRight)
                    .with_size(IconSize::Small)
                    .text_color(cx.theme().muted_foreground),
            )
            .into_any_element()
    }
}

fn render_brand(cx: &gpui::App) -> impl IntoElement {
    h_flex()
        .w_full()
        .min_w_0()
        .items_center()
        .gap_4()
        .child(
            div()
                .flex_none()
                .size(px(44.0))
                .flex()
                .items_center()
                .justify_center()
                .rounded_lg()
                .bg(cx.theme().primary.opacity(0.1))
                .text_color(cx.theme().primary)
                .child(Icon::new(IconName::ServerLine).with_size(IconSize::Large)),
        )
        .child(
            v_flex()
                .min_w_0()
                .flex_grow(1.0)
                .gap_1()
                .child(
                    div()
                        .text_xs()
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(cx.theme().primary)
                        .child(t!("Home.StartCenter.get_started")),
                )
                .child(
                    div()
                        .text_2xl()
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(cx.theme().foreground)
                        .child("Navop"),
                )
                .child(
                    div()
                        .text_sm()
                        .text_color(cx.theme().muted_foreground)
                        .child(t!("Home.StartCenter.subtitle")),
                ),
        )
}

fn render_create_panel(
    view: gpui::Entity<HomePage>,
    window: &mut Window,
    cx: &gpui::App,
) -> impl IntoElement {
    surface_panel("modern-home-create-panel", cx)
        .child(panel_header(
            t!("Home.StartCenter.create_and_import"),
            None,
            cx,
        ))
        .child(utility_row(
            "modern-home-import",
            IconName::Upload,
            t!("Home.other_app_import").to_string(),
            t!("Home.StartCenter.import_description").to_string(),
            view,
            window,
            |_, window, cx| {
                show_connection_import_window(cx.entity(), window.window_handle(), cx);
            },
            cx,
        ))
}

fn render_workspace_tools(
    view: gpui::Entity<HomePage>,
    window: &mut Window,
    cx: &gpui::App,
) -> impl IntoElement {
    surface_panel("modern-home-tools-panel", cx)
        .child(panel_header(
            t!("Home.StartCenter.workspace_tools"),
            None,
            cx,
        ))
        .child(
            v_flex()
                .w_full()
                .gap_1()
                .child(utility_row(
                    "modern-home-notes",
                    IconName::BookOpen,
                    t!("Home.notes").to_string(),
                    t!("Home.StartCenter.notes_description").to_string(),
                    view.clone(),
                    window,
                    |home, window, cx| {
                        home.add_notes_tab(window, cx);
                    },
                    cx,
                ))
                .child(utility_row(
                    "modern-home-ai",
                    IconName::Bot,
                    t!("Settings.General.Startup.default_page_ai_workbench").to_string(),
                    t!("Home.StartCenter.ai_description").to_string(),
                    view.clone(),
                    window,
                    |home, window, cx| {
                        home.add_ai_workbench_tab(window, cx);
                    },
                    cx,
                ))
                .child(utility_row(
                    "modern-home-extensions",
                    IconName::Apps,
                    t!("Home.extensions").to_string(),
                    t!("Home.StartCenter.extensions_description").to_string(),
                    view,
                    window,
                    |home, window, cx| {
                        home.add_extensions_tab(window, cx);
                    },
                    cx,
                )),
        )
}

fn render_status_panel(
    syncing: bool,
    view: gpui::Entity<HomePage>,
    window: &mut Window,
    cx: &gpui::App,
) -> impl IntoElement {
    let has_key = one_core::crypto::has_master_key();
    let sync_view = view.clone();
    let key_view = view;

    surface_panel("modern-home-status-panel", cx)
        .flex_grow(1.0)
        .child(panel_header(t!("Home.StartCenter.status"), None, cx))
        .child(
            v_flex()
                .w_full()
                .gap_1()
                .child(
                    status_row(
                        "modern-home-sync",
                        if syncing {
                            IconName::LoaderCircle
                        } else {
                            IconName::Refresh
                        },
                        if syncing {
                            t!("Home.syncing").to_string()
                        } else {
                            t!("Home.sync").to_string()
                        },
                        t!("Home.StartCenter.sync_description").to_string(),
                        !syncing,
                        cx,
                    )
                    .when(!syncing, |this| {
                        this.on_click(window.listener_for(&sync_view, |home, _, _, cx| {
                            home.trigger_sync(cx);
                        }))
                    }),
                )
                .child(
                    status_row(
                        "modern-home-keys",
                        if has_key {
                            IconName::CircleCheck
                        } else {
                            IconName::Key
                        },
                        if has_key {
                            t!("Encryption.personal_key_unlocked").to_string()
                        } else {
                            t!("Encryption.personal_key_locked").to_string()
                        },
                        if has_key {
                            t!("Home.StartCenter.key_description_unlocked").to_string()
                        } else {
                            t!("Home.StartCenter.key_description_locked").to_string()
                        },
                        true,
                        cx,
                    )
                    .on_click(window.listener_for(
                        &key_view,
                        |home, _, window, cx| {
                            home.show_encryption_key_dialog(window, cx);
                        },
                    )),
                ),
        )
}

fn surface_panel(id: &'static str, cx: &gpui::App) -> gpui::Stateful<gpui::Div> {
    v_flex()
        .id(id)
        .w_full()
        .min_w_0()
        .gap_2()
        .p_3()
        .rounded_xl()
        .border_1()
        .border_color(cx.theme().border)
        .bg(cx.theme().background)
}

fn panel_header(title: impl IntoElement, badge: Option<String>, cx: &gpui::App) -> gpui::Div {
    v_flex().w_full().gap_1().child(
        h_flex()
            .w_full()
            .min_w_0()
            .items_center()
            .justify_between()
            .child(
                div()
                    .min_w_0()
                    .flex_grow(1.0)
                    .text_sm()
                    .font_semibold()
                    .text_color(cx.theme().foreground)
                    .whitespace_nowrap()
                    .child(title),
            )
            .when_some(badge, |this, badge| {
                this.child(
                    div()
                        .flex_none()
                        .px_2()
                        .py_0p5()
                        .rounded_full()
                        .bg(cx.theme().secondary)
                        .text_xs()
                        .text_color(cx.theme().secondary_foreground)
                        .child(badge),
                )
            }),
    )
}

fn render_empty_recent(cx: &gpui::App) -> impl IntoElement {
    v_flex()
        .w_full()
        .min_h(px(140.0))
        .items_center()
        .justify_center()
        .gap_3()
        .rounded_lg()
        .border_1()
        .border_color(cx.theme().border)
        .bg(cx.theme().muted)
        .child(
            div()
                .size(px(40.0))
                .flex()
                .items_center()
                .justify_center()
                .rounded_lg()
                .bg(cx.theme().background)
                .text_color(cx.theme().muted_foreground)
                .child(Icon::new(IconName::LayoutDashboard).with_size(IconSize::Medium)),
        )
        .child(
            v_flex()
                .items_center()
                .gap_1()
                .child(
                    div()
                        .text_sm()
                        .font_semibold()
                        .child(t!("Home.StartCenter.no_recent")),
                )
                .child(
                    div()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child(t!("Home.StartCenter.no_recent_description")),
                ),
        )
}

#[allow(clippy::too_many_arguments)]
fn utility_row(
    id: &'static str,
    icon: IconName,
    title: String,
    description: String,
    view: gpui::Entity<HomePage>,
    window: &mut Window,
    on_click: impl Fn(&mut HomePage, &mut Window, &mut gpui::Context<HomePage>) + 'static,
    cx: &gpui::App,
) -> impl IntoElement {
    let hover_background = cx.theme().muted;

    h_flex()
        .id(id)
        .w_full()
        .min_w_0()
        .min_h(px(46.0))
        .items_center()
        .gap_2()
        .px_2()
        .py_1()
        .rounded_lg()
        .cursor_pointer()
        .hover(move |style| style.bg(hover_background))
        .on_click(window.listener_for(&view, move |home, _, window, cx| {
            on_click(home, window, cx);
        }))
        .child(
            div()
                .flex_none()
                .size(px(30.0))
                .flex()
                .items_center()
                .justify_center()
                .rounded_md()
                .bg(cx.theme().secondary)
                .text_color(cx.theme().secondary_foreground)
                .child(Icon::new(icon).with_size(IconSize::Small)),
        )
        .child(
            v_flex()
                .min_w_0()
                .flex_grow(1.0)
                .gap_0p5()
                .child(
                    div()
                        .text_sm()
                        .font_semibold()
                        .overflow_hidden()
                        .text_ellipsis()
                        .whitespace_nowrap()
                        .child(title),
                )
                .child(
                    div()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .overflow_hidden()
                        .text_ellipsis()
                        .whitespace_nowrap()
                        .child(description),
                ),
        )
        .child(
            Icon::new(IconName::ChevronRight)
                .with_size(IconSize::Small)
                .text_color(cx.theme().muted_foreground),
        )
}

fn status_row(
    id: &'static str,
    icon: IconName,
    title: String,
    description: String,
    interactive: bool,
    cx: &gpui::App,
) -> gpui::Stateful<gpui::Div> {
    let hover_background = cx.theme().muted;

    h_flex()
        .id(id)
        .w_full()
        .min_w_0()
        .min_h(px(44.0))
        .items_center()
        .gap_2()
        .px_2()
        .py_1()
        .rounded_lg()
        .when(interactive, |this| {
            this.cursor_pointer()
                .hover(move |style| style.bg(hover_background))
        })
        .child(
            Icon::new(icon)
                .with_size(IconSize::Small)
                .text_color(cx.theme().muted_foreground),
        )
        .child(
            v_flex()
                .min_w_0()
                .flex_grow(1.0)
                .gap_0p5()
                .child(
                    div()
                        .text_sm()
                        .font_semibold()
                        .overflow_hidden()
                        .text_ellipsis()
                        .whitespace_nowrap()
                        .child(title),
                )
                .child(
                    div()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .overflow_hidden()
                        .text_ellipsis()
                        .whitespace_nowrap()
                        .child(description),
                ),
        )
        .when(interactive, |this| {
            this.child(
                Icon::new(IconName::ChevronRight)
                    .with_size(IconSize::Small)
                    .text_color(cx.theme().muted_foreground),
            )
        })
}
