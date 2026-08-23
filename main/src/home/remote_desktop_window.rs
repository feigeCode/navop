use gpui::{AppContext, Focusable};
use one_core::popup_window::{PopupWindowOptions, open_popup_window};
use remote_desktop::RemoteDesktopConnectionOptions;
use remote_desktop_view::{RemoteDesktopView, RemoteDesktopViewConfig};
use rust_i18n::t;

const DEFAULT_WINDOW_WIDTH: f32 = 1280.0;
const DEFAULT_WINDOW_HEIGHT: f32 = 800.0;
const MIN_WINDOW_WIDTH: f32 = 640.0;
const MIN_WINDOW_HEIGHT: f32 = 480.0;

fn remote_desktop_window_options(title: String) -> PopupWindowOptions {
    PopupWindowOptions::new(title)
        .size(DEFAULT_WINDOW_WIDTH, DEFAULT_WINDOW_HEIGHT)
        .min_width(MIN_WINDOW_WIDTH)
        .min_height(MIN_WINDOW_HEIGHT)
        .fullscreen(true)
        // 全屏 RDP 的 ActiveX 子窗口会覆盖整个客户区并抢走鼠标/键盘输入，
        // 隐藏标题栏后 hover 显示与关闭都会失效。保留标题栏，让窗口控件
        // 始终可见可点（overlay 只覆盖标题栏下方的内容区）。
        .hide_titlebar_when_fullscreen(false)
        .fullscreen_hint(t!("Connection.fullscreen_exit_hint").to_string())
}

pub(crate) fn open_remote_desktop_fullscreen_window(
    options: RemoteDesktopConnectionOptions,
    title: String,
    cx: &mut gpui::App,
) {
    open_popup_window(
        remote_desktop_window_options(title.clone()),
        move |window, cx| {
            let view = cx.new(|cx| {
                RemoteDesktopView::new(
                    RemoteDesktopViewConfig {
                        options,
                        title,
                        tab_index: None,
                    },
                    window.window_handle(),
                    cx,
                )
            });
            view.read(cx).focus_handle(cx).focus(window, cx);
            view
        },
        cx,
    );
}

#[cfg(test)]
mod tests {
    #[test]
    fn fullscreen_window_keeps_titlebar_visible() {
        let options = super::remote_desktop_window_options("RDP".to_string());

        assert!(options.fullscreen);
        assert!(!options.hide_titlebar_when_fullscreen);
        assert!(options.fullscreen_hint.is_some());
    }
}
