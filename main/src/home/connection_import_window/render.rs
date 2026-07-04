use gpui::prelude::FluentBuilder;
use gpui::{
    AnyElement, Context, FontWeight, InteractiveElement, IntoElement, ParentElement, Render,
    Styled, Window, div, px,
};
use gpui_component::{
    ActiveTheme, Disableable, Icon, IconName, Sizable, Size, TitleBar,
    button::{Button, ButtonVariants as _},
    h_flex,
    scroll::ScrollableElement,
    v_flex,
};
use rust_i18n::t;

use super::ConnectionImportWindow;
use preview::render_preview_row;
use source::render_source_row;

mod preview;
mod source;

impl ConnectionImportWindow {
    fn render_header(&self, cx: &mut Context<Self>) -> impl IntoElement {
        TitleBar::new().child(
            h_flex()
                .items_center()
                .justify_between()
                .w_full()
                .px_3()
                .child(div().w(px(220.0)))
                .child(
                    div()
                        .text_sm()
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(cx.theme().foreground)
                        .child(t!("Home.import").to_string()),
                )
                .child(
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
                                .label("扫描")
                                .disabled(!self.model.can_scan() || self.scanning)
                                .on_click(cx.listener(|this, _, _, cx| this.scan_selected(cx))),
                        )
                        .child(
                            Button::new("save-selected-imports")
                                .small()
                                .icon(IconName::Check)
                                .label("保存所选")
                                .disabled(
                                    self.scanning || self.model.batch_save_row_ids().is_empty(),
                                )
                                .on_click(cx.listener(|this, _, _, cx| this.save_selected(cx))),
                        ),
                ),
        )
    }

    fn render_sources(&self, cx: &mut Context<Self>) -> impl IntoElement {
        v_flex()
            .w(px(260.0))
            .h_full()
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
                    .child("应用来源"),
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
            .min_w_0()
            .p_4()
            .gap_3()
            .child(self.render_preview_summary(cx))
            .child(
                v_flex()
                    .flex_1()
                    .gap_2()
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
            )
    }

    fn render_preview_summary(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let selected = self.model.batch_save_row_ids().len();
        h_flex()
            .items_center()
            .justify_between()
            .child(
                v_flex()
                    .gap_1()
                    .child(
                        div()
                            .text_sm()
                            .font_weight(FontWeight::SEMIBOLD)
                            .child("预览结果"),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child(format!(
                                "可保存 {} / {} 条",
                                selected,
                                self.model.rows().len()
                            )),
                    ),
            )
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
                        .child("正在扫描"),
                )
            })
    }

    fn render_empty_state(&self, cx: &mut Context<Self>) -> AnyElement {
        let message = if self.loading_sources {
            "正在加载导入扩展"
        } else if self.scanning {
            "正在扫描选中的导入来源"
        } else {
            "选择来源后点击扫描"
        };
        div()
            .p_4()
            .border_1()
            .border_color(cx.theme().border)
            .rounded(px(6.0))
            .text_sm()
            .text_color(cx.theme().muted_foreground)
            .child(message)
            .into_any_element()
    }
}

impl Render for ConnectionImportWindow {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        v_flex()
            .size_full()
            .track_focus(&self.focus_handle)
            .bg(cx.theme().background)
            .child(self.render_header(cx))
            .child(
                h_flex()
                    .flex_1()
                    .w_full()
                    .overflow_hidden()
                    .child(self.render_sources(cx))
                    .child(self.render_preview(cx)),
            )
    }
}
