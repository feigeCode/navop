use super::*;

impl Render for TerminalView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let effective_font_family = self.prepare_render(window, cx);
        self.render_tool_dock(effective_font_family, cx)
    }
}
