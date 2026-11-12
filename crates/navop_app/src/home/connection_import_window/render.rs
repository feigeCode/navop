use gpui::prelude::FluentBuilder;
use gpui::{
    AnyElement, Context, FontWeight, InteractiveElement, IntoElement, ParentElement, Render,
    Styled, Window, div, px,
};
use gpui_component::{
    ActiveTheme, Disableable, Icon, Sizable, Size,
    button::{Button, ButtonVariants as _},
    checkbox::Checkbox,
    h_flex,
    scroll::ScrollableElement,
    v_flex,
};
use one_assets::IconName;
use rust_i18n::t;

use super::ConnectionImportWindow;
use preview::render_preview_row;
use source::render_source_row;

mod preview;
mod source;

impl ConnectionImportWindow {
    fn render_preview_actions(&self, cx: &mut Context<Self>) -> impl IntoElement {
        h_flex()
            .gap_2()
            .child(
                Button::new("refresh-importers")
                    .small()
                    .icon(IconName::Refresh)
                    .disabled(self.scanning)
                    .on_click(cx.listener(|this, _, _, cx| this.refresh_sources(cx))),
            )
            .child(
                Button::new("scan-importers")
                    .small()
                    .primary()
                    .icon(IconName::Play)
                    .label(t!("Home.ConnectionImport.scan").to_string())
                    .disabled(!self.model.can_scan() || self.scanning)
                    .on_click(cx.listener(|this, _, _, cx| this.scan_selected(cx))),
            )
            .child(
                Button::new("save-selected-imports")
                    .small()
                    .icon(IconName::Check)
                    .label(t!("Home.ConnectionImport.save_selected").to_string())
                    .disabled(self.scanning || self.model.batch_save_row_ids().is_empty())
                    .on_click(cx.listener(|this, _, _, cx| this.save_selected(cx))),
            )
    }

    fn render_preview_status(&self, cx: &mut Context<Self>) -> impl IntoElement {
        h_flex()
            .gap_2()
            .items_center()
            .when_some(self.status_message.clone(), |this, message| {
                this.child(div().text_xs().text_color(cx.theme().danger).child(message))
            })
            .when(self.scanning, |this| {
                this.child(
                    h_flex()
                        .gap_2()
                        .items_center()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child(Icon::new(IconName::LoaderCircle).with_size(Size::Small))
                        .child(t!("Home.ConnectionImport.scanning").to_string()),
                )
            })
    }

    fn render_preview_toolbar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let workspace_group_count = self.model.workspace_group_paths().len();
        div().overflow_hidden().child(
            h_flex()
                .items_center()
                .justify_between()
                .w_full()
                .gap_3()
                .child(
                    v_flex()
                        .gap_1()
                        .min_w_0()
                        .child(
                            div()
                                .text_sm()
                                .font_weight(FontWeight::SEMIBOLD)
                                .child(t!("Home.ConnectionImport.preview_results").to_string()),
                        )
                        .child(
                            div()
                                .text_xs()
                                .text_color(cx.theme().muted_foreground)
                                .child(
                                    t!(
                                        "Home.ConnectionImport.savable_count",
                                        savable = self.model.batch_save_row_ids().len(),
                                        total = self.model.rows().len()
                                    )
                                    .to_string(),
                                ),
                        )
                        .when(workspace_group_count > 0, |this| {
                            this.child(
                                div()
                                    .text_xs()
                                    .text_color(cx.theme().muted_foreground)
                                    .child(
                                        t!(
                                            "Home.ConnectionImport.workspace_group_count",
                                            groups = workspace_group_count
                                        )
                                        .to_string(),
                                    ),
                            )
                        }),
                )
                .child(
                    h_flex()
                        .gap_2()
                        .items_center()
                        .flex_shrink_0()
                        .child(self.render_preview_status(cx))
                        .child(self.render_preview_actions(cx)),
                ),
        )
    }

    /// 导入默认项：统一套用到这次迁移出来的 SSH 连接上，避免逐条改高级设置。
    fn render_migration_defaults(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let defaults = self.model.defaults();
        v_flex()
            .gap_1()
            .p_2()
            .border_1()
            .border_color(cx.theme().border)
            .rounded(px(6.0))
            .child(
                h_flex()
                    .gap_4()
                    .items_center()
                    .child(
                        div()
                            .text_xs()
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(t!("Home.ConnectionImport.migration_defaults").to_string()),
                    )
                    .child(
                        Checkbox::new("import-allow-legacy-algorithms")
                            .checked(defaults.allow_legacy_algorithms)
                            .label(t!("Home.ConnectionImport.allow_legacy_algorithms").to_string())
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.model.toggle_allow_legacy_algorithms();
                                cx.notify();
                            })),
                    )
                    .child(
                        Checkbox::new("import-disable-shell-integration")
                            .checked(defaults.disable_shell_integration)
                            .label(
                                t!("Home.ConnectionImport.disable_shell_integration").to_string(),
                            )
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.model.toggle_disable_shell_integration();
                                cx.notify();
                            })),
                    ),
            )
            .child(
                div()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child(t!("Home.ConnectionImport.migration_defaults_hint").to_string()),
            )
    }

    fn render_sources(&self, cx: &mut Context<Self>) -> impl IntoElement {
        v_flex()
            .flex_none()
            .w(px(320.0))
            .h_full()
            .min_h_0()
            .border_r_1()
            .border_color(cx.theme().border)
            .bg(cx.theme().sidebar)
            .p_3()
            .gap_2()
            .overflow_y_scrollbar()
            .child(
                div()
                    .text_sm()
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(t!("Home.ConnectionImport.sources").to_string()),
            )
            .child(
                div()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child(t!("Home.ConnectionImport.sources_scan_hint").to_string()),
            )
            .children(
                self.model
                    .sources()
                    .iter()
                    .map(|source| render_source_row(source, self.scanning, cx)),
            )
    }

    fn render_preview(&self, cx: &mut Context<Self>) -> impl IntoElement {
        v_flex()
            .flex_1()
            .h_full()
            .min_h_0()
            .min_w_0()
            .p_4()
            .gap_3()
            .child(self.render_preview_toolbar(cx))
            .child(self.render_migration_defaults(cx))
            .child(
                div().flex_1().min_h_0().overflow_hidden().child(
                    v_flex()
                        .size_full()
                        .gap_2()
                        .pb_3()
                        .overflow_y_scrollbar()
                        .when(self.model.rows().is_empty(), |this| {
                            this.child(self.render_empty_state(cx))
                        })
                        .children(
                            self.model
                                .rows()
                                .iter()
                                .map(|row| render_preview_row(row, cx)),
                        ),
                ),
            )
    }

    fn render_empty_state(&self, cx: &mut Context<Self>) -> AnyElement {
        let message = if self.loading_sources {
            t!("Home.ConnectionImport.loading_extensions")
        } else if self.scanning {
            t!("Home.ConnectionImport.scanning_selected_sources")
        } else {
            t!("Home.ConnectionImport.select_sources_hint")
        };
        div()
            .p_4()
            .border_1()
            .border_color(cx.theme().border)
            .rounded(px(6.0))
            .text_sm()
            .text_color(cx.theme().muted_foreground)
            .child(message.to_string())
            .into_any_element()
    }
}

impl Render for ConnectionImportWindow {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        v_flex()
            .size_full()
            .track_focus(&self.focus_handle)
            .bg(cx.theme().background)
            .child(
                h_flex()
                    .flex_1()
                    .min_h_0()
                    .w_full()
                    .overflow_hidden()
                    .child(self.render_sources(cx))
                    .child(self.render_preview(cx)),
            )
    }
}
