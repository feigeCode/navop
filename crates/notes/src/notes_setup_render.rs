use crate::NotesView;
use gpui::{Context, IntoElement, ParentElement, Styled, div, px};
use gpui_component::{
    ActiveTheme, Icon, IconName, StyledExt,
    button::{Button, ButtonVariants},
    v_flex,
};
use rust_i18n::t;

impl NotesView {
    pub(crate) fn render_location_setup(&self, cx: &mut Context<Self>) -> gpui::AnyElement {
        v_flex()
            .size_full()
            .items_center()
            .justify_center()
            .child(
                v_flex()
                    .w(px(420.0))
                    .items_center()
                    .gap_3()
                    .p_6()
                    .rounded_lg()
                    .border_1()
                    .border_color(cx.theme().border)
                    .child(Icon::new(IconName::NotesColor).size_8())
                    .child(
                        div()
                            .text_lg()
                            .font_semibold()
                            .child(t!("Notes.location_not_configured").to_string()),
                    )
                    .child(
                        div()
                            .text_sm()
                            .text_center()
                            .text_color(cx.theme().muted_foreground)
                            .child(t!("Notes.location_setup_description").to_string()),
                    )
                    .child(
                        Button::new("configure_notes_location")
                            .label(t!("Notes.configure_location").to_string())
                            .primary()
                            .on_click(cx.listener(|view, _, window, cx| {
                                view.show_location_dialog(window, cx);
                            })),
                    ),
            )
            .into_any_element()
    }
}
