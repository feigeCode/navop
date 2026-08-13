mod session;
mod view;

use gpui::{
    AppContext, Bounds, Pixels, QuitMode, TitlebarOptions, WindowBounds, WindowOptions, px, size,
};
use windows_rdp_host::{WindowsRdpHostError, WindowsRdpHostOptions};

use crate::cli::Config;

pub(super) fn run(config: Config) {
    let window_width = config.width as f32;
    let window_height = config.height as f32;
    gpui_platform::application()
        .with_quit_mode(QuitMode::LastWindowClosed)
        .run(move |cx| {
            let bounds = Bounds::centered(None, size(px(window_width), px(window_height)), cx);
            let options = WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                titlebar: Some(TitlebarOptions {
                    title: Some("Navop GPUI Native RDP Smoke".into()),
                    ..Default::default()
                }),
                ..Default::default()
            };
            if let Err(error) = cx.open_window(options, move |window, cx| {
                window.activate_window();
                cx.new(|cx| view::SmokeView::new(config, window, cx))
            }) {
                eprintln!("ERROR: stage=open_gpui_window error={error}");
            }
        });
}

fn host_options() -> WindowsRdpHostOptions {
    WindowsRdpHostOptions::new(1)
}

fn physical_viewport_size(window: &gpui::Window) -> (i32, i32) {
    let viewport = window.viewport_size();
    let scale_factor = window.scale_factor();
    (
        physical_pixels(viewport.width, scale_factor),
        physical_pixels(viewport.height, scale_factor),
    )
}

fn physical_pixels(value: Pixels, scale_factor: f32) -> i32 {
    (f32::from(value) * scale_factor)
        .round()
        .clamp(1.0, i32::MAX as f32) as i32
}

fn log_host_error(stage: &str, error: WindowsRdpHostError) {
    eprintln!("ERROR: stage={stage} error={error}");
    eprintln!("ERROR_DEBUG: stage={stage} error={error:?}");
    eprintln!(
        "ERROR_FIELDS: stage={stage} native_result={:?} native_stage={:?} win32_code={:?} hresult_code={:?} hresult_kind={:?}",
        error.native_result(),
        error.stage(),
        error.win32_code(),
        error.hresult().map(|value| value.code()),
        error.hresult().map(|value| value.kind())
    );
}
