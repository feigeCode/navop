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

mod home;

use home::ShellHome;

struct ShellProduct;

impl Product for ShellProduct {
    type Assets = ShellAssets;

    fn identity(&self) -> &'static AppIdentity {
        &AppIdentity::SHELL
    }

    fn features(&self) -> ProductFeatures {
        ProductFeatures::SHELL
    }

    fn assets(&self) -> Self::Assets {
        ShellAssets::new()
    }

    fn domains(&self) -> Vec<Arc<dyn domain_api::Domain>> {
        vec![Arc::new(shell_domain::ShellDomain)]
    }

    fn init(&self, cx: &mut App) -> anyhow::Result<()> {
        gpui_component::init(cx);
        one_core::init(cx)?;
        let settings = AppSettings::load();
        settings.apply(cx);
        cx.set_global(settings);
        terminal_view::init(cx);
        remote_desktop_view::init(cx);
        mcp_approval::init(cx);
        mcp_host::enable_default_toolsets(
            cx,
            &[
                mcp_host::McpToolset::Terminal,
                mcp_host::McpToolset::Sftp,
                mcp_host::McpToolset::Connections,
            ],
        );
        Ok(())
    }
}

struct ShellAssets {
    builtin: gpui_component_assets::Assets,
    icons: one_assets::Assets,
}

impl ShellAssets {
    fn new() -> Self {
        Self {
            builtin: gpui_component_assets::Assets::new(""),
            icons: one_assets::Assets::new(),
        }
    }
}

impl AssetSource for ShellAssets {
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
    let product = ShellProduct;
    let identity = product.identity();
    gpui_platform::application()
        .with_assets(ShellAssets::new())
        .with_quit_mode(QuitMode::LastWindowClosed)
        .run(move |cx: &mut App| {
            if let Err(error) = app_kit::bootstrap(&product, cx) {
                eprintln!("failed to initialize navop-shell: {error:#}");
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
                let home = cx.new(|cx| ShellHome::new(tab_container, cx));
                cx.new(|cx| Root::new(home, window, cx))
            }) {
                eprintln!("failed to open navop-shell window: {error}");
            }
        });
}
