use gpui::{AppContext, Focusable};
use one_core::popup_window::{PopupWindowOptions, open_popup_window};
use remote_desktop::RemoteDesktopConnectionOptions;
use remote_desktop_view::{RemoteDesktopView, RemoteDesktopViewConfig};

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
        .hide_titlebar_when_fullscreen(false)
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
    fn fullscreen_window_keeps_titlebar_for_window_controls_and_dragging() {
        let options = super::remote_desktop_window_options("RDP".to_string());

        assert!(options.fullscreen);
        assert!(!options.hide_titlebar_when_fullscreen);
    }
}
