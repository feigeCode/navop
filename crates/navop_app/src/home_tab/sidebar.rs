use super::*;
use gpui_component::StyledExt;

impl HomePage {
    /// 首页导航侧栏当前占用宽度（折叠/展开），供网格几何计算共用。
    pub(super) fn home_sidebar_width(&self) -> gpui::Pixels {
        if self.sidebar_collapsed {
            HOME_SIDEBAR_COLLAPSED_WIDTH
        } else {
            HOME_SIDEBAR_EXPANDED_WIDTH
        }
    }

    pub(crate) fn render_sidebar_for_active(
        &mut self,
        active_key: Option<&str>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let width = if HomePage::uses_global_navigation_layout(cx) {
            px(52.0)
        } else {
            self.home_sidebar_width()
        };
        let global = HomePage::uses_global_navigation_layout(cx);
        let chrome_palette = crate::chrome_palette::chrome_palette(cx);
        v_flex()
            .debug_selector(move || {
                if global {
                    "application-navigation-content".to_owned()
                } else {
                    "application-navigation".to_owned()
                }
            })
            .h_full()
            .flex_shrink_0()
            .w(width)
            .bg(if global {
                chrome_palette.rail
            } else {
                chrome_palette.menu
            })
            // 主页导航模式的菜单由外壳定位到贯通全窗口的 Tab 栏下方。
            .child(
                v_flex()
                    .id("global-application-navigation")
                    .flex_1()
                    .min_w_0()
                    .h_full()
                    .overflow_hidden()
                    .when(!global, |navigation| {
                        navigation.child(
                            h_flex()
                                .h(px(52.0))
                                .flex_shrink_0()
                                .items_center()
                                .px_5()
                                .text_base()
                                .font_semibold()
                                .text_color(cx.theme().sidebar_foreground)
                                .child("Navop"),
                        )
                    })
                    .child(
                        div()
                            .id("home-applications-scroll")
                            .flex_1()
                            .min_h_0()
                            .overflow_y_scroll()
                            .px(if global { px(6.0) } else { px(8.0) })
                            .py_2()
                            .child(self.render_application_navigation(active_key, window, cx)),
                    )
                    .child(
                        v_flex()
                            .px_2()
                            .pt_2()
                            .pb_2()
                            .gap_1()
                            .child(self.render_account_entry(window, cx))
                            .child(self.render_settings_entry(active_key, window, cx)),
                    ),
            )
    }
}
