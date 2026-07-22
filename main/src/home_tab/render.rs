use super::*;

impl Focusable for HomePage {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

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

impl Render for HomePage {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let global_user = GlobalCurrentUser::get_user(cx);
        if global_user.is_none() && self.current_user.is_some() {
            self.current_user = None;
            self.team_options.clear();
        }
        // 检测会话过期：token 刷新失败时由回调设置静态标志，在此处响应
        if crate::auth::check_and_reset_session_expired() {
            self.current_user = None;
            self.team_options.clear();
            // 延迟弹出登录对话框，避免在 render 中直接修改窗口
            let view = cx.entity();
            window.defer(cx, move |window, cx| {
                view.update(cx, |this, cx| {
                    this.show_login_dialog(window, cx);
                });
            });
        }

        if self.master_key_unlock_prompt_pending && self.saved_connections_locked() {
            self.master_key_unlock_prompt_pending = false;
            let view = cx.entity();
            window.defer(cx, move |window, cx| {
                view.update(cx, |this, cx| {
                    this.show_encryption_key_dialog(window, cx);
                });
            });
        }

        // 检测认证错误：登录/注册失败时显示错误提示
        if let Some(error) = self.auth_error.take() {
            let view = cx.entity();
            window.defer(cx, move |window, cx| {
                let error_msg = error.clone();
                let view_for_ok = view.clone();
                window.open_dialog(cx, move |dialog, _window, _cx| {
                    let view_clone = view_for_ok.clone();
                    dialog
                        .title(t!("Auth.auth_error_title").to_string())
                        .child(error_msg.clone().into_any_element())
                        .alert()
                        .on_ok(move |_, window, cx| {
                            // 关闭错误对话框后重新弹出登录对话框
                            view_clone.update(cx, |this, cx| {
                                this.show_login_dialog(window, cx);
                            });
                            true
                        })
                });
            });
        }

        let legacy = self.home_page_style == HomePageStyle::Legacy;
        let content = v_flex()
            .flex_1()
            .min_w_0()
            .h_full()
            .overflow_hidden()
            .bg(cx.theme().background)
            .when(legacy, |content| {
                content.child(self.render_toolbar(window, cx))
            })
            .child(
                div()
                    .flex_1()
                    .w_full()
                    .min_w_0()
                    .overflow_hidden()
                    .bg(if legacy {
                        cx.theme().muted
                    } else {
                        cx.theme().background
                    })
                    .child(if legacy {
                        self.render_content_area(cx)
                    } else {
                        self.render_modern_home(window, cx)
                    }),
            );

        div()
            .size_full()
            .min_w_0()
            .track_focus(&self.focus_handle)
            .child(
                h_flex()
                    .size_full()
                    .min_w_0()
                    .overflow_hidden()
                    .when(self.home_page_style == HomePageStyle::Legacy, |layout| {
                        layout.child(self.render_sidebar(window, cx))
                    })
                    .child(content),
            )
    }
}
