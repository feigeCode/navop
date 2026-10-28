use domain_api::{AppIdentity, Product, ProductFeatures};
use gpui::{
    App, Bounds, Context, IntoElement, QuitMode, Render, TitlebarOptions, Window, WindowBounds,
    WindowOptions, div, prelude::*, px, size,
};
use std::sync::Arc;

struct DataProduct;

impl Product for DataProduct {
    type Assets = one_assets::Assets;

    fn identity(&self) -> &'static AppIdentity {
        &AppIdentity::DATA
    }

    fn features(&self) -> ProductFeatures {
        ProductFeatures::DATA
    }

    fn assets(&self) -> Self::Assets {
        one_assets::Assets::new()
    }

    fn domains(&self) -> Vec<Arc<dyn domain_api::Domain>> {
        Vec::new()
    }
}

struct DataPlaceholder {
    identity: &'static AppIdentity,
}

impl Render for DataPlaceholder {
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
    let product = DataProduct;
    let identity = product.identity();
    gpui_platform::application()
        .with_quit_mode(QuitMode::LastWindowClosed)
        .run(move |cx: &mut App| {
            if let Err(error) = app_kit::bootstrap(&product, cx) {
                eprintln!("failed to initialize navop-data: {error:#}");
                cx.quit();
                return;
            }
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
                cx.new(|_cx| DataPlaceholder { identity })
            }) {
                eprintln!("failed to open navop-data window: {error}");
            }
        });
}
