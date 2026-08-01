use super::UpdateDialogView;
use crate::NAVOP_ICON_ASSET_PATH;
use gpui::{AnyElement, Context, IntoElement, ParentElement, Render, Styled, Window, div, px};
use gpui_component::{
    ActiveTheme, Disableable, Icon, Sizable, StyledExt,
    button::{Button, ButtonVariants as _},
    checkbox::Checkbox,
    h_flex,
    progress::Progress,
    text::TextView,
    v_flex,
};
use rust_i18n::t;

impl UpdateDialogView {
    fn render_app_icon(&self, size: f32) -> AnyElement {
        Icon::default()
            .path(NAVOP_ICON_ASSET_PATH)
            .color()
            .with_size(px(size))
            .into_any_element()
    }

    fn release_notes(&self) -> String {
        self.info
            .release_notes
            .as_deref()
            .filter(|notes| !notes.trim().is_empty())
            .map(str::to_owned)
            .unwrap_or_else(|| {
                format!(
                    "## {}\n\n{}",
                    t!("Update.release_notes"),
                    t!("Update.release_notes_unavailable")
                )
            })
    }

    fn render_available_update(&self, cx: &mut Context<Self>) -> AnyElement {
        let message = if self.info.is_local_simulation {
            t!(
                "Update.simulation_message",
                latest = self.info.latest_version,
                current = self.info.current_version
            )
            .to_string()
        } else {
            t!(
                "Update.message",
                latest = self.info.latest_version,
                current = self.info.current_version
            )
            .to_string()
        };
        let action_text = if one_core::app_paths::is_portable() && !self.info.is_local_simulation {
            t!("Update.open_release_page").to_string()
        } else if self.info.is_local_simulation {
            t!("Update.simulation_action").to_string()
        } else {
            t!("Update.action_install").to_string()
        };
        let available_title = if self.info.is_local_simulation {
            t!("Update.simulation_available_title").to_string()
        } else {
            t!("Update.available_title").to_string()
        };

        v_flex()
            .size_full()
            .gap_4()
            .px_5()
            .pb_5()
            .child(
                h_flex()
                    .items_center()
                    .gap_4()
                    .child(self.render_app_icon(68.0))
                    .child(
                        v_flex()
                            .min_w_0()
                            .gap_1()
                            .child(
                                div()
                                    .text_xl()
                                    .font_semibold()
                                    .text_color(cx.theme().foreground)
                                    .child(available_title),
                            )
                            .child(
                                div()
                                    .text_base()
                                    .text_color(cx.theme().foreground)
                                    .child(message),
                            ),
                    ),
            )
            .child(
                div()
                    .h(px(230.0))
                    .w_full()
                    .overflow_hidden()
                    .rounded(px(8.0))
                    .border_1()
                    .border_color(cx.theme().border)
                    .bg(cx.theme().background)
                    .p_4()
                    .child(
                        TextView::markdown("update-release-notes", self.release_notes())
                            .selectable(true)
                            .scrollable(true)
                            .size_full(),
                    ),
            )
            .child(
                Checkbox::new("update-auto-download")
                    .checked(self.auto_update)
                    .label(t!("Update.auto_download").to_string())
                    .on_click(cx.listener(|this, checked, _, cx| {
                        this.set_auto_update(*checked, cx);
                    })),
            )
            .child(
                h_flex()
                    .w_full()
                    .items_center()
                    .justify_between()
                    .child(
                        Button::new("update-skip-version")
                            .small()
                            .rounded(px(18.0))
                            .label(t!("Update.skip_version").to_string())
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.skip_version(window, cx);
                            })),
                    )
                    .child(
                        h_flex()
                            .items_center()
                            .gap_3()
                            .child(
                                Button::new("update-later")
                                    .small()
                                    .rounded(px(18.0))
                                    .w(px(126.0))
                                    .label(t!("Update.remind_later").to_string())
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.on_cancel(window, cx);
                                    })),
                            )
                            .child(
                                Button::new("update-action")
                                    .small()
                                    .primary()
                                    .rounded(px(18.0))
                                    .w(px(126.0))
                                    .label(action_text)
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.on_ok_action(window, cx);
                                    })),
                            ),
                    ),
            )
            .into_any_element()
    }

    fn compact_heading(&self) -> String {
        if self.applying {
            t!("Update.applying_heading").to_string()
        } else if self.completed && self.info.is_local_simulation {
            t!("Update.simulation_complete_heading").to_string()
        } else if self.completed {
            t!("Update.download_complete_heading").to_string()
        } else if self.cancelling {
            t!("Update.cancelling").to_string()
        } else if self.cancelled {
            t!("Update.cancelled").to_string()
        } else if self.error_message.is_some() {
            t!("Update.download_failed").to_string()
        } else {
            t!("Update.downloading_heading").to_string()
        }
    }

    fn compact_action_text(&self) -> String {
        if self.downloading {
            t!("Update.cancel").to_string()
        } else if self.completed && self.info.is_local_simulation {
            t!("Update.close").to_string()
        } else if self.completed {
            t!("Update.action_install").to_string()
        } else {
            t!("Update.close").to_string()
        }
    }

    fn render_download_progress(&self, cx: &mut Context<Self>) -> AnyElement {
        h_flex()
            .size_full()
            .items_center()
            .gap_5()
            .px_5()
            .pb_5()
            .child(self.render_app_icon(64.0))
            .child(
                v_flex()
                    .min_w_0()
                    .flex_1()
                    .gap_3()
                    .child(
                        div()
                            .text_xl()
                            .font_semibold()
                            .text_color(cx.theme().foreground)
                            .child(self.compact_heading()),
                    )
                    .child(Progress::new("update-progress").value(self.progress_value()))
                    .child(
                        h_flex()
                            .w_full()
                            .items_center()
                            .justify_between()
                            .gap_3()
                            .child(
                                div()
                                    .min_w_0()
                                    .text_base()
                                    .text_color(cx.theme().foreground)
                                    .child(
                                        if self.completed
                                            || self.cancelled
                                            || self.error_message.is_some()
                                        {
                                            self.status_message()
                                        } else {
                                            self.progress_label()
                                        },
                                    ),
                            )
                            .child(
                                Button::new("update-compact-action")
                                    .small()
                                    .rounded(px(18.0))
                                    .w(px(112.0))
                                    .label(self.compact_action_text())
                                    .disabled(self.cancelling || self.applying)
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        if this.completed {
                                            this.on_ok_action(window, cx);
                                        } else {
                                            this.on_cancel(window, cx);
                                        }
                                    })),
                            ),
                    ),
            )
            .into_any_element()
    }
}

impl Render for UpdateDialogView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self.downloading
            || self.cancelling
            || self.cancelled
            || self.completed
            || self.downloaded_bytes > 0
            || self.error_message.is_some()
        {
            self.render_download_progress(cx)
        } else {
            self.render_available_update(cx)
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn update_dialog_keeps_reference_layout_controls() {
        let source = include_str!("dialog_render.rs");

        assert!(source.contains("update-auto-download"));
        assert!(source.contains("update-skip-version"));
        assert!(source.contains("update-release-notes"));
        assert!(source.contains("render_download_progress"));
        assert!(source.contains("update-compact-action"));
        assert!(source.contains("|| self.cancelled"));
        assert!(source.contains("self.completed"));
        assert!(source.contains("|| self.error_message.is_some()"));
    }
}
