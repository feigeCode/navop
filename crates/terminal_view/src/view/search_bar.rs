use super::*;

impl TerminalView {
    pub(super) fn render_terminal_search_bar(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let colors = self.current_theme.colors();
        h_flex()
            .id("terminal-search-bar")
            .key_context(TERMINAL_SEARCH_CONTEXT)
            .absolute()
            .top(px(20.0))
            .right(px(20.0))
            .occlude()
            .w(px(340.0))
            .max_w(px(420.0))
            .gap_1()
            .p_2()
            .rounded_md()
            .border_1()
            .border_color(colors.border)
            .bg(colors.background)
            .shadow_lg()
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .on_action(cx.listener(Self::close_terminal_search))
            .on_action(cx.listener(Self::search_forward))
            .on_action(cx.listener(Self::search_backward))
            .child(
                Input::new(&self.search_input)
                    .small()
                    .w_full()
                    .prefix(Icon::new(IconName::Search).xsmall()),
            )
            .child(
                Button::new("terminal-search-prev")
                    .icon(IconName::ChevronUp)
                    .ghost()
                    .small()
                    .on_click(cx.listener(|this, _, _window, cx| {
                        this.search_backward_internal(cx);
                    })),
            )
            .child(
                Button::new("terminal-search-next")
                    .icon(IconName::ChevronDown)
                    .ghost()
                    .small()
                    .on_click(cx.listener(|this, _, _window, cx| {
                        this.search_forward_internal(cx);
                    })),
            )
            .child(
                Button::new("terminal-search-close")
                    .icon(IconName::Close)
                    .ghost()
                    .small()
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.close_search(window, cx);
                    })),
            )
            .into_any_element()
    }
}
