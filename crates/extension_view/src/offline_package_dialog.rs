use gpui::{
    App, AppContext, Context, FocusHandle, Focusable, IntoElement, ParentElement, Render,
    SharedString, Styled, Window, div,
};
use gpui_component::{
    ActiveTheme, Sizable,
    button::{Button, ButtonVariants as _},
    clipboard::Clipboard,
    h_flex, v_flex,
};
use rust_i18n::t;

/// 扩展市场地址。
const EXTENSION_MARKETPLACE_URL: &str = "https://navop.dev/zh-CN/extensions";
/// 离线包下载的 GitHub Releases 地址。
const OFFLINE_PACKAGE_RELEASES_URL: &str =
    "https://github.com/feigeCode/onetcli-extensions/releases";
/// 国内扩展下载地址。
const EXTENSION_MIRROR_URL: &str = "https://cnb.cool/navop-dev/navop-extensions";

/// 打开"离线包下载"弹窗。
pub(super) fn show_offline_package_dialog(cx: &mut App) {
    let options =
        one_core::popup_window::PopupWindowOptions::new(t!("Extension.offline_package_title"))
            .size(520.0, 320.0);
    one_core::popup_window::open_popup_window(
        options,
        |_window, cx| cx.new(|cx| OfflinePackageDialogView::new(cx)),
        None,
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
        v_flex()
            .gap_3()
            .size_full()
            .child(
                div().flex_1().p_4().child(
                    v_flex()
                        .gap_3()
                        .child(
                            div()
                                .text_sm()
                                .text_color(cx.theme().muted_foreground)
                                .child(t!("Extension.offline_package_hint").to_string()),
                        )
                        .child(url_row(
                            cx,
                            t!("Extension.marketplace"),
                            EXTENSION_MARKETPLACE_URL,
                            "offline-package-marketplace-copy",
                            "offline-package-marketplace-open",
                        ))
                        .child(url_row(
                            cx,
                            t!("Extension.offline_package_releases_label"),
                            OFFLINE_PACKAGE_RELEASES_URL,
                            "offline-package-releases-copy",
                            "offline-package-releases-open",
                        ))
                        .child(url_row(
                            cx,
                            t!("Extension.offline_package_mirror_label"),
                            EXTENSION_MIRROR_URL,
                            "offline-package-mirror-copy",
                            "offline-package-mirror-open",
                        )),
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

/// 渲染一条下载渠道:标签 + URL + 复制按钮 + 打开按钮。
fn url_row(
    cx: &App,
    label: impl Into<SharedString>,
    url: &'static str,
    copy_id: &'static str,
    open_id: &'static str,
) -> impl IntoElement {
    let label: SharedString = label.into();
    let url: SharedString = url.into();
    let url_for_open = url.clone();

    v_flex()
        .gap_1()
        .child(
            div()
                .text_xs()
                .text_color(cx.theme().muted_foreground)
                .child(label),
        )
        .child(
            h_flex()
                .items_center()
                .gap_1()
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .text_sm()
                        .text_color(cx.theme().link)
                        .child(url.clone()),
                )
                .child(Clipboard::new(copy_id).value(url))
                .child(
                    Button::new(open_id)
                        .xsmall()
                        .ghost()
                        .icon(gpui_component::IconName::ExternalLink)
                        .on_click(move |_, _, cx| {
                            cx.open_url(&url_for_open);
                        }),
                ),
        )
}
