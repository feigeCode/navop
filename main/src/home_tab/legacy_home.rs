use super::*;

impl EventEmitter<TabContentEvent> for HomePage {}

impl TabContent for HomePage {
    fn content_key(&self) -> &'static str {
        "Home"
    }

    fn title(&self, _cx: &App) -> SharedString {
        SharedString::from(t!("Home.title"))
    }

    fn icon(&self, _cx: &App) -> Option<Icon> {
        Some(IconName::Home.color())
    }

    fn closeable(&self, _cx: &App) -> bool {
        false
    }

    fn width_size(&self, _cx: &App) -> Option<Size> {
        Some(Size::Small)
    }
}

impl HomePage {
    pub(super) fn render_legacy_home(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        h_flex()
            .size_full()
            .min_w_0()
            .overflow_hidden()
            .child(self.render_sidebar(window, cx))
            .child(
                v_flex()
                    .flex_1()
                    .min_w_0()
                    .h_full()
                    .overflow_hidden()
                    .bg(cx.theme().background)
                    .child(self.render_toolbar(window, cx))
                    .child(
                        div()
                            .flex_1()
                            .w_full()
                            .min_w_0()
                            .overflow_hidden()
                            .bg(cx.theme().muted)
                            .child(self.render_content_area(cx)),
                    ),
            )
            .into_any_element()
    }
}
