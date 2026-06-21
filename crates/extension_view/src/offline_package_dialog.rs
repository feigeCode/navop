use gpui::{
    App, AppContext, Context, FocusHandle, Focusable, IntoElement, ParentElement, Render,
    SharedString, Styled, Window, div,
};
use gpui_component::{
    ActiveTheme, Sizable, TitleBar,
    button::{Button, ButtonVariants as _},
    clipboard::Clipboard,
    h_flex, v_flex,
};
use rust_i18n::t;

/// 离线包下载的 GitHub Releases 地址。
const OFFLINE_PACKAGE_RELEASES_URL: &str =
    "https://github.com/feigeCode/onetcli-extensions/releases";

/// 打开"离线包下载"弹窗。
pub(super) fn show_offline_package_dialog(cx: &mut App) {
    let options =
        one_core::popup_window::PopupWindowOptions::new(t!("Extension.offline_package_title"))
            .size(480.0, 240.0);
    one_core::popup_window::open_popup_window(
        options,
        |_window, cx| cx.new(|cx| OfflinePackageDialogView::new(cx)),
        cx,
    );
}

struct OfflinePackageDialogView {
    focus_handle: FocusHandle,
}

impl OfflinePackageDialogView {
    fn new(cx: &mut Context<Self>) -> Self {
        Self {
            focus_handle: cx.focus_handle(),
        }
    }
}

impl Focusable for OfflinePackageDialogView {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for OfflinePackageDialogView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // URL 需要在多个闭包里使用,克隆一份独立持有,避免借用 cx。
        let url: SharedString = OFFLINE_PACKAGE_RELEASES_URL.into();
        let url_for_open = url.clone();

        v_flex()
            .gap_3()
            .size_full()
            .bg(cx.theme().background)
            .child(
                TitleBar::new().child(
                    div()
                        .flex()
                        .items_center()
                        .justify_center()
                        .flex_1()
                        .text_sm()
                        .font_weight(gpui::FontWeight::MEDIUM)
                        .child(t!("Extension.offline_package_title").to_string()),
                ),
            )
            .child(
                div().flex_1().p_4().child(
                    v_flex()
                        .gap_2()
                        .child(
                            div()
                                .text_sm()
                                .text_color(cx.theme().muted_foreground)
                                .child(t!("Extension.offline_package_hint").to_string()),
                        )
                        .child(
                            h_flex()
                                .items_center()
                                .gap_1()
                                .child(
                                    div()
                                        .text_sm()
                                        .text_color(cx.theme().link)
                                        .child(url.clone()),
                                )
                                .child(Clipboard::new("offline-package-url-copy").value(url))
                                .child(
                                    Button::new("offline-package-url-open")
                                        .xsmall()
                                        .ghost()
                                        .icon(gpui_component::IconName::ExternalLink)
                                        .on_click(move |_, _, cx| {
                                            cx.open_url(&url_for_open);
                                        }),
                                ),
                        ),
                ),
            )
            .child(
                h_flex()
                    .justify_end()
                    .gap_2()
                    .p_4()
                    .border_t_1()
                    .border_color(cx.theme().border)
                    .child(
                        Button::new("offline-package-close")
                            .small()
                            .primary()
                            .label(t!("Common.confirm").to_string())
                            .on_click(cx.listener(|_view, _, window, _cx| {
                                window.remove_window();
                            })),
                    ),
            )
    }
}
