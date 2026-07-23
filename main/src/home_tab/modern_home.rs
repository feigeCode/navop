use gpui::{
    AnyElement, FontWeight, InteractiveElement, IntoElement, ParentElement, SharedString,
    StatefulInteractiveElement, Styled, Window, div, px,
    prelude::FluentBuilder as _,
};
use gpui_component::{
    ActiveTheme, Disableable, Icon, IconName, Sizable, Size, StyledExt,
    button::{Button, ButtonVariants as _},
    h_flex, v_flex,
};
use one_core::storage::StoredConnection;
use rust_i18n::t;

use super::{HomePage, modern_home_shortcuts::render_shortcuts};
use crate::home::connection_import_window::show_connection_import_window;

impl HomePage {
    pub(super) fn render_modern_home(
        &self,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) -> AnyElement {
        let view = cx.entity();
        let import_view = view.clone();
        let notes_view = view.clone();
        let ai_view = view.clone();
        let extensions_view = view.clone();
        let sync_view = view.clone();
        let key_view = view.clone();
        let syncing = self.syncing;

        div()
            .id("modern-home-start-center")
            .size_full()
            .overflow_y_scroll()
            .child(
                v_flex()
                    .min_h_full()
                    .w_full()
                    .items_center()
                    .justify_center()
                    .px_6()
                    .py_8()
                    .child(
                        v_flex()
                            .w_full()
                            .max_w(px(760.0))
                            .gap_6()
                            .child(render_brand(cx))
                            .child(self.render_primary_actions(view, window, cx))
                            .child(render_account_actions(
                                syncing, sync_view, key_view, window, cx,
                            ))
                            .when_some(
                                self.render_recent_connections(window, cx),
                                |this, recent| this.child(recent),
                            )
                            .child(render_tool_cards(
                                import_view,
                                notes_view,
                                ai_view,
                                extensions_view,
                                window,
                                cx,
                            ))
                            .child(render_shortcuts(cx)),
                    ),
            )
            .into_any_element()
    }
}

fn render_brand(cx: &gpui::App) -> impl IntoElement {
    v_flex()
        .items_center()
        .gap_1()
        .child(
            div()
                .text_xl()
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(cx.theme().foreground)
                .child("Navop"),
        )
        .child(
            div()
                .text_xs()
                .text_color(cx.theme().muted_foreground)
                .child(t!("Home.StartCenter.subtitle")),
        )
}

fn render_account_actions(
    syncing: bool,
    sync_view: gpui::Entity<HomePage>,
    key_view: gpui::Entity<HomePage>,
    window: &mut Window,
    cx: &gpui::App,
) -> impl IntoElement {
    let has_key = one_core::crypto::has_master_key();
    h_flex()
        .w_full()
        .justify_center()
        .gap_2()
        .child(
            Button::new("modern-home-sync")
                .icon(if syncing {
                    IconName::LoaderCircle
                } else {
                    IconName::Refresh
                })
                .ghost()
                .label(if syncing {
                    t!("Home.syncing").to_string()
                } else {
                    t!("Home.sync").to_string()
                })
                .disabled(syncing)
                .tooltip(t!("Home.sync_tooltip"))
                .on_click(window.listener_for(&sync_view, |home, _, _, cx| {
                    home.trigger_sync(cx);
                })),
        )
        .child(
            Button::new("modern-home-keys")
                .icon(IconName::Key)
                .ghost()
                .label(if has_key {
                    t!("Encryption.personal_key_unlocked").to_string()
                } else {
                    t!("Encryption.personal_key_locked").to_string()
                })
                .tooltip(t!("Encryption.keys_tooltip"))
                .on_click(window.listener_for(&key_view, |home, _, window, cx| {
                    home.show_encryption_key_dialog(window, cx);
                })),
        )
        .text_color(cx.theme().foreground)
}

impl HomePage {
    fn render_primary_actions(
        &self,
        view: gpui::Entity<HomePage>,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) -> impl IntoElement {
        h_flex()
            .w_full()
            .justify_center()
            .flex_wrap()
            .gap_3()
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
            .child(self.render_local_terminal_button(window, cx))
            .child(
                Button::new("modern-home-quick-open")
                    .icon(IconName::Search)
                    .outline()
                    .label(t!("Home.StartCenter.quick_open"))
                    .on_click(window.listener_for(&view, |home, _, window, cx| {
                        home.show_connection_quick_open(window, cx);
                    })),
            )
            .text_color(cx.theme().foreground)
    }

    /// Recently opened connections, most recent first, so the home page works
    /// as a dashboard instead of a splash screen. Hidden when empty.
    fn render_recent_connections(
        &self,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) -> Option<impl IntoElement> {
        let mut recent: Vec<StoredConnection> = self
            .connections
            .iter()
            .filter(|conn| conn.last_used_at.is_some())
            .cloned()
            .collect();
        recent.sort_by_key(|conn| std::cmp::Reverse(conn.last_used_at));
        recent.truncate(6);
        if recent.is_empty() {
            return None;
        }

        let cards: Vec<AnyElement> = recent
            .into_iter()
            .map(|conn| self.render_recent_connection_card(conn, window, cx))
            .collect();
        Some(
            v_flex()
                .w_full()
                .gap_2()
                .child(section_title(t!("Home.StartCenter.recent"), cx))
                .child(div().grid().grid_cols(2).gap_3().children(cards)),
        )
    }

    fn render_recent_connection_card(
        &self,
        conn: StoredConnection,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) -> AnyElement {
        let icon = self.connection_icon(&conn, px(20.0));
        let name = conn.name.clone();
        let type_label = conn.connection_type.label().to_string();
        h_flex()
            .id(SharedString::from(format!(
                "recent-conn-{}",
                conn.id.unwrap_or(0)
            )))
            .min_h(px(56.0))
            .items_center()
            .gap_3()
            .px_4()
            .py_3()
            .rounded_lg()
            .border_1()
            .border_color(cx.theme().border)
            .bg(cx.theme().background)
            .cursor_pointer()
            .hover(|style| style.bg(cx.theme().muted))
            .on_click(window.listener_for(&cx.entity(), move |home, _, window, cx| {
                home.open_connection_from_quick(&conn, window, cx);
            }))
            .child(icon)
            .child(
                v_flex()
                    .min_w_0()
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
            .into_any_element()
    }
}

fn render_tool_cards(
    import_view: gpui::Entity<HomePage>,
    notes_view: gpui::Entity<HomePage>,
    ai_view: gpui::Entity<HomePage>,
    extensions_view: gpui::Entity<HomePage>,
    window: &mut Window,
    cx: &gpui::App,
) -> impl IntoElement {
    v_flex()
        .w_full()
        .gap_2()
        .child(section_title(t!("Home.StartCenter.tools"), cx))
        .child(
            div()
                .grid()
                .grid_cols(2)
                .gap_3()
                .child(tool_card(
                    "modern-home-import",
                    IconName::Upload,
                    t!("Home.other_app_import").to_string(),
                    t!("Home.StartCenter.import_description").to_string(),
                    import_view.clone(),
                    window,
                    |_, window, cx| {
                        show_connection_import_window(cx.entity(), window.window_handle(), cx);
                    },
                    cx,
                ))
                .child(tool_card(
                    "modern-home-notes",
                    IconName::BookOpen,
                    t!("Home.notes").to_string(),
                    t!("Home.StartCenter.notes_description").to_string(),
                    notes_view,
                    window,
                    |home, window, cx| {
                        home.add_notes_tab(window, cx);
                    },
                    cx,
                ))
                .child(tool_card(
                    "modern-home-ai",
                    IconName::Bot,
                    t!("Settings.General.Startup.default_page_ai_workbench").to_string(),
                    t!("Home.StartCenter.ai_description").to_string(),
                    ai_view,
                    window,
                    |home, window, cx| {
                        home.add_ai_workbench_tab(window, cx);
                    },
                    cx,
                ))
                .child(tool_card(
                    "modern-home-extensions",
                    IconName::ExtensionsColor,
                    t!("Home.extensions").to_string(),
                    t!("Home.StartCenter.extensions_description").to_string(),
                    extensions_view,
                    window,
                    |home, window, cx| {
                        home.add_extensions_tab(window, cx);
                    },
                    cx,
                )),
        )
}

fn tool_card(
    id: &'static str,
    icon: IconName,
    title: String,
    description: String,
    view: gpui::Entity<HomePage>,
    window: &mut Window,
    on_click: impl Fn(&mut HomePage, &mut Window, &mut gpui::Context<HomePage>) + 'static,
    cx: &gpui::App,
) -> impl IntoElement {
    h_flex()
        .id(id)
        .min_h(px(84.0))
        .items_center()
        .gap_3()
        .p_4()
        .rounded_lg()
        .border_1()
        .border_color(cx.theme().border)
        .bg(cx.theme().background)
        .cursor_pointer()
        .hover(|style| style.bg(cx.theme().muted))
        .on_click(window.listener_for(&view, move |home, _, window, cx| {
            on_click(home, window, cx);
        }))
        .child(Icon::new(icon).with_size(Size::Medium))
        .child(
            v_flex()
                .min_w_0()
                .gap_1()
                .child(div().text_sm().font_semibold().child(title))
                .child(
                    div()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child(description),
                ),
        )
}

fn section_title(title: impl IntoElement, cx: &gpui::App) -> impl IntoElement {
    div()
        .text_xs()
        .font_semibold()
        .text_color(cx.theme().muted_foreground)
        .child(title)
}
