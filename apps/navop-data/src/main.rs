mod home;
mod sort;

use domain_api::{AppIdentity, Product, ProductFeatures};
use gpui::{
    App, AppContext as _, AssetSource, Bounds, QuitMode, SharedString, TitlebarOptions,
    WindowBounds, WindowOptions, px, size,
};
use gpui_component::Root;
use one_core::settings::AppSettings;
use one_core::tab_container::TabContainer;
use std::borrow::Cow;
use std::sync::Arc;

use home::DataHome;

struct DataProduct;

impl Product for DataProduct {
    type Assets = DataAssets;

    fn identity(&self) -> &'static AppIdentity {
        &AppIdentity::DATA
    }

    fn features(&self) -> ProductFeatures {
        ProductFeatures::DATA
    }

    fn assets(&self) -> Self::Assets {
        DataAssets::new()
    }

    fn domains(&self) -> Vec<Arc<dyn domain_api::Domain>> {
        vec![Arc::new(data_domain::DataDomain)]
    }

    fn init(&self, cx: &mut App) -> anyhow::Result<()> {
        gpui_component::init(cx);
        one_core::init(cx)?;
        let settings = AppSettings::load();
        settings.apply(cx);
        cx.set_global(settings);
        one_ui::init(cx, Default::default());
        db::init_cache(cx);
        extension_runtime::init(cx);
        db_view::search_shortcut::init(cx);
        db_view::sql_editor_view::init(cx);
        redis_view::init(cx);
        mongodb_view::init(cx);
        Ok(())
    }
}

struct DataAssets {
    builtin: gpui_component_assets::Assets,
    icons: one_assets::Assets,
}

impl DataAssets {
    fn new() -> Self {
        Self {
            builtin: gpui_component_assets::Assets::new(""),
            icons: one_assets::Assets::new(),
        }
    }
}

impl AssetSource for DataAssets {
    fn load(&self, path: &str) -> anyhow::Result<Option<Cow<'static, [u8]>>> {
        if let Some(asset) = self.icons.load(path)? {
            return Ok(Some(asset));
        }
        self.builtin.load(path)
    }

    fn list(&self, path: &str) -> anyhow::Result<Vec<SharedString>> {
        let mut assets = self.icons.list(path)?;
        assets.extend(self.builtin.list(path)?);
        Ok(assets)
    }
}

fn main() {
    let product = DataProduct;
    let identity = product.identity();
    gpui_platform::application()
        .with_assets(DataAssets::new())
        .with_quit_mode(QuitMode::LastWindowClosed)
        .run(move |cx: &mut App| {
            if let Err(error) = app_kit::bootstrap(&product, cx) {
                eprintln!("failed to initialize navop-data: {error:#}");
                cx.quit();
                return;
            }
            let bounds = Bounds::centered(None, size(px(1100.0), px(720.0)), cx);
            let options = WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                titlebar: Some(TitlebarOptions {
                    title: Some(identity.display_name.into()),
                    ..Default::default()
                }),
                app_id: Some(identity.app_id.to_owned()),
                ..Default::default()
            };
            if let Err(error) = cx.open_window(options, |window, cx| {
                window.activate_window();
                let tab_container = cx.new(|cx| TabContainer::new(window, cx));
                let home = cx.new(|cx| DataHome::new(tab_container, cx));
                cx.new(|cx| Root::new(home, window, cx))
            }) {
                eprintln!("failed to open navop-data window: {error}");
            }
        });
}
