use domain_api::AppIdentity;
use gpui::{
    App, Bounds, Context, IntoElement, QuitMode, Render, TitlebarOptions, Window, WindowBounds,
    WindowOptions, div, prelude::*, px, size,
};

struct ShellPlaceholder {
    identity: &'static AppIdentity,
}

impl Render for ShellPlaceholder {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .size_full()
            .flex()
            .items_center()
            .justify_center()
            .child(format!(
                "{} 骨架已启动：app_id={}，数据目录={}",
                self.identity.display_name, self.identity.app_id, self.identity.data_dir_name
            ))
    }
}

fn main() {
    let identity = &AppIdentity::SHELL;
    app_kit::configure_identity(identity);
    gpui_platform::application()
        .with_quit_mode(QuitMode::LastWindowClosed)
        .run(move |cx: &mut App| {
            let bounds = Bounds::centered(None, size(px(900.0), px(600.0)), cx);
            let options = WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                titlebar: Some(TitlebarOptions {
                    title: Some(identity.display_name.into()),
                    ..Default::default()
                }),
                app_id: Some(identity.app_id.to_owned()),
                ..Default::default()
            };
            if let Err(error) = cx.open_window(options, |_window, cx| {
                cx.new(|_cx| ShellPlaceholder { identity })
            }) {
                eprintln!("failed to open navop-shell window: {error}");
            }
        });
}
