#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

rust_i18n::i18n!("locales", fallback = "en");

mod auth;

mod ai_chat_acp;
mod ai_chat_acp_approval;
mod app_init;
mod external_driver_display;
mod home;
mod home_tab;
mod license;
pub mod new_connection;
mod onetcli_app;
mod personal_sync_runtime;
#[cfg(test)]
mod personal_sync_runtime_tests;
mod personal_sync_status;
mod public_mcp_approval;
mod public_mcp_runtime;
mod setting_tab;
mod settings;
mod team_management;
mod update;
mod user_avatar;

use crate::onetcli_app::OnetCliApp;
use gpui::*;

use gpui_component::Root;
use gpui_component_assets::Assets;
use std::sync::Arc;
use tracing::{info, warn};

struct AppAssets {
    builtin: Assets,
    driver: db::ipc::DriverAssetSource,
}

impl AppAssets {
    fn new() -> Self {
        Self {
            builtin: Assets,
            driver: db::ipc::DriverAssetSource::new(
                Arc::new(db::ipc::DriverResourceLoader::new()),
                Arc::new(db::ipc::IpcDriverRegistry::load_default()),
            ),
        }
    }
}

impl AssetSource for AppAssets {
    fn load(&self, path: &str) -> Result<Option<std::borrow::Cow<'static, [u8]>>> {
        match self.driver.load(path) {
            Ok(Some(asset)) => {
                if path.starts_with("driver://") {
                    info!(
                        target: "driver_icon",
                        asset_path = path,
                        bytes = asset.len(),
                        "app asset source served driver asset"
                    );
                }
                Ok(Some(asset))
            }
            Ok(None) => {
                if path.starts_with("driver://") {
                    info!(
                        target: "driver_icon",
                        asset_path = path,
                        "driver asset source returned none; trying builtin assets"
                    );
                }
                self.builtin.load(path)
            }
            Err(error) => {
                warn!(
                    target: "driver_icon",
                    asset_path = path,
                    error = %error,
                    "driver asset source failed; trying builtin assets"
                );
                self.builtin.load(path)
            }
        }
    }

    fn list(&self, path: &str) -> Result<Vec<SharedString>> {
        let mut assets = self.driver.list(path).unwrap_or_default();
        assets.extend(self.builtin.list(path).unwrap_or_default());
        assets.sort();
        assets.dedup();
        Ok(assets)
    }
}

fn main() {
    if update::handle_update_command() {
        return;
    }
    if let Some(exit_code) = handle_cli_command() {
        std::process::exit(exit_code);
    }

    let app = gpui_platform::application()
        .with_assets(AppAssets::new())
        .with_quit_mode(QuitMode::LastWindowClosed);

    app.run(move |cx| {
        onetcli_app::init(cx);
        extension_runtime::init(cx);

        let mut window_size = size(px(1600.0), px(1200.0));
        if let Some(display) = cx.primary_display() {
            let display_size = display.bounds().size;
            window_size.width = window_size.width.min(display_size.width * 0.85);
            window_size.height = window_size.height.min(display_size.height * 0.85);
        }

        let window_bounds = Bounds::centered(None, window_size, cx);
        let options = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(window_bounds)),
            #[cfg(not(target_os = "linux"))]
            titlebar: Some(gpui_component::TitleBar::title_bar_options()),
            window_min_size: Some(Size {
                width: px(640.),
                height: px(480.),
            }),
            #[cfg(target_os = "linux")]
            window_background: gpui::WindowBackgroundAppearance::Transparent,
            #[cfg(target_os = "linux")]
            window_decorations: Some(gpui::WindowDecorations::Client),
            kind: WindowKind::Normal,
            ..Default::default()
        };

        cx.spawn(async move |cx| {
            cx.open_window(options, |window, cx| {
                window.activate_window();
                app_init::init_window_systems(window, cx);
                update::schedule_update_check(window, cx);
                let view = cx.new(|cx| OnetCliApp::new(window, cx));
                cx.new(|cx| Root::new(view, window, cx))
            })?;

            Ok::<_, anyhow::Error>(())
        })
        .detach();
    });
}

#[cfg(not(target_os = "windows"))]
fn handle_cli_command() -> Option<i32> {
    onetcli_runtime::cli_host::handle_command(|| crate::public_mcp_runtime::cli_tool_registry())
}

#[cfg(target_os = "windows")]
fn handle_cli_command() -> Option<i32> {
    None
}
