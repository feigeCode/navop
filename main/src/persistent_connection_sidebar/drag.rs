use gpui::{
    Context, FontWeight, InteractiveElement, IntoElement, ParentElement, Render, Styled, Window,
    div, px,
};
use gpui_component::{ActiveTheme, Icon, IconName, Sizable, Size, h_flex};

#[derive(Clone)]
pub(super) struct DragConnection {
    pub connection_id: i64,
    pub name: String,
}

impl Render for DragConnection {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        h_flex()
            .id("persistent-drag-connection")
            .w(px(240.0))
            .px_3()
            .py_2()
            .gap_2()
            .items_center()
            .cursor_grabbing()
            .rounded(px(6.0))
            .border_1()
            .border_color(cx.theme().border)
            .bg(cx.theme().popover)
            .shadow_md()
            .child(Icon::new(IconName::Apps).with_size(Size::Small))
            .child(
                div()
                    .min_w_0()
                    .flex_1()
                    .overflow_hidden()
                    .whitespace_nowrap()
                    .text_ellipsis()
                    .text_sm()
                    .font_weight(FontWeight::MEDIUM)
                    .child(self.name.clone()),
            )
    }
}
