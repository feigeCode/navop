use super::*;

impl HomePage {
    /// 首页导航侧栏当前占用宽度（折叠/展开），供网格几何计算共用。
    pub(super) fn home_sidebar_width(&self) -> gpui::Pixels {
        if self.sidebar_collapsed {
            HOME_SIDEBAR_COLLAPSED_WIDTH
        } else {
            HOME_SIDEBAR_EXPANDED_WIDTH
        }
    }

    pub(super) fn render_sidebar(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let width = self.home_sidebar_width();
        h_flex()
            .h_full()
            .flex_shrink_0()
            .w(width)
            .bg(cx.theme().sidebar)
            .border_r_1()
            .border_color(cx.theme().sidebar_border)
            .child(
                v_flex()
                    .flex_1()
                    .min_w_0()
                    .h_full()
                    .overflow_hidden()
                    .child(
                        div()
                            .id("home-applications-scroll")
                            .flex_1()
                            .min_h_0()
                            .overflow_y_scroll()
                            .px_2()
                            .py_1()
                            .child(self.render_application_navigation(window, cx)),
                    )
                    .child(
                        v_flex()
                            .px_2()
                            .pt_2()
                            .pb_1()
                            .gap_1()
                            .border_t_1()
                            .border_color(cx.theme().sidebar_border)
                            .child(self.render_account_entry(window, cx))
                            .child(self.render_settings_entry(window, cx)),
                    ),
            )
    }
}
