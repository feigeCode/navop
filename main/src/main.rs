#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

rust_i18n::i18n!("locales", fallback = "en");

mod auth;

mod ai_chat_acp;
mod app_init;
mod external_driver_display;
mod file_association;
mod file_open;
mod home;
mod home_tab;
mod license;
mod local_terminal_profiles;
pub mod new_connection;
mod onetcli_app;
mod persistent_connection_sidebar;
mod personal_sync_conflicts;
mod personal_sync_runtime;
#[cfg(test)]
mod personal_sync_runtime_tests;
mod personal_sync_status;
mod public_mcp_approval;
mod public_mcp_runtime;
mod setting_tab;
mod settings;
mod sync_conflict_dialog;
mod team_management;
mod update;
mod user_avatar;
#[cfg(any(target_os = "windows", test))]
mod windows_single_instance;

use crate::onetcli_app::OnetCliApp;
use gpui::*;

use gpui_component::Root;
use gpui_component_assets::Assets;
use one_core::settings::{AppSettings, MainWindowSize};
use std::path::PathBuf;
use std::sync::Arc;
use tracing::{info, warn};

struct AppAssets {
    builtin: Assets,
    driver: db::ipc::DriverAssetSource,
}

const DEFAULT_MAIN_WINDOW_WIDTH: f32 = 1800.0;
const DEFAULT_MAIN_WINDOW_HEIGHT: f32 = 1260.0;
const MAIN_WINDOW_DISPLAY_RATIO: f32 = 0.9;

enum AppOpenRequest {
    ActivateAndOpenPaths(Vec<PathBuf>),
    Open(file_open::FileOpenInput),
}

fn initial_main_window_size(
    saved: Option<MainWindowSize>,
    display_size: Option<Size<Pixels>>,
) -> Size<Pixels> {
    let mut result = saved
        .and_then(|saved| MainWindowSize::new(saved.width, saved.height))
        .map(|saved| size(px(saved.width), px(saved.height)))
        .unwrap_or_else(|| {
            size(
                px(DEFAULT_MAIN_WINDOW_WIDTH),
                px(DEFAULT_MAIN_WINDOW_HEIGHT),
            )
        });
    if let Some(display_size) = display_size {
        let maximum = size(
            px(f32::from(display_size.width) * MAIN_WINDOW_DISPLAY_RATIO),
            px(f32::from(display_size.height) * MAIN_WINDOW_DISPLAY_RATIO),
        );
        if saved.is_none() {
            result = maximum;
        }
        result.width = result.width.min(maximum.width);
        result.height = result.height.min(maximum.height);
    }
    result
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

    let startup_arguments =
        match one_core::app_paths::parse_startup_arguments(std::env::args_os().skip(1)) {
            Ok(arguments) => arguments,
            Err(error) => {
                eprintln!("Failed to parse startup arguments: {error:#}");
                return;
            }
        };
    let path_context = match one_core::app_paths::process_context() {
        Ok(context) => context,
        Err(error) => {
            eprintln!("Failed to resolve application paths: {error:#}");
            return;
        }
    };
    let resolved_paths = match one_core::app_paths::resolve_app_paths(
        &startup_arguments.path_overrides,
        &path_context,
    ) {
        Ok(paths) => paths,
        Err(error) => {
            eprintln!("Failed to resolve application paths: {error:#}");
            return;
        }
    };
    let startup_paths = startup_arguments
        .remaining
        .into_iter()
        .map(PathBuf::from)
        .collect::<Vec<_>>();
    let (startup_request_tx, startup_request_rx) = smol::channel::unbounded();

    #[cfg(target_os = "windows")]
    {
        use windows_single_instance::{SingleInstanceOutcome, StartupRequest};

        let forwarded_request_tx = startup_request_tx.clone();
        match windows_single_instance::claim_or_forward(
            resolved_paths.config_dir(),
            StartupRequest::new(startup_paths.clone()),
            move |request| {
                if let Err(error) = forwarded_request_tx
                    .try_send(AppOpenRequest::ActivateAndOpenPaths(request.into_paths()))
                {
                    tracing::warn!(%error, "failed to enqueue forwarded startup request");
                }
            },
        ) {
            Ok(SingleInstanceOutcome::Primary) => {}
            Ok(SingleInstanceOutcome::Forwarded) => return,
            Err(error) => {
                tracing::warn!(
                    %error,
                    "failed to establish Windows single-instance listener; continuing startup"
                );
            }
        }
    }

    if let Err(error) =
        startup_request_tx.try_send(AppOpenRequest::ActivateAndOpenPaths(startup_paths))
    {
        tracing::warn!(%error, "failed to enqueue initial startup request");
    }

    if !resolved_paths.is_portable()
        && let Err(error) = one_core::app_dirs::migrate_legacy_directories()
    {
        eprintln!("Failed to migrate legacy application directories: {error:#}");
    }
    if let Err(error) =
        one_core::app_paths::initialize_app_paths(&startup_arguments.path_overrides, &path_context)
    {
        eprintln!("Failed to initialize application paths: {error:#}");
        return;
    }

    let app = gpui_platform::application()
        .with_assets(AppAssets::new())
        .with_quit_mode(QuitMode::LastWindowClosed);
    app.on_open_urls({
        let startup_request_tx = startup_request_tx.clone();
        move |urls| {
            for url in urls {
                if let Err(error) = startup_request_tx
                    .try_send(AppOpenRequest::Open(file_open::FileOpenInput::Url(url)))
                {
                    tracing::warn!(%error, "failed to enqueue platform file-open event");
                }
            }
        }
    });

    app.run(move |cx| {
        onetcli_app::init(cx);
        if !one_core::app_paths::is_portable() {
            file_association::schedule_registration(cx);
        }
        notes::init(cx);
        extension_runtime::set_current_host_version(env!("CARGO_PKG_VERSION"))
            .expect("main package version must be valid semver");
        extension_runtime::init(cx);

        let saved_size = AppSettings::current(cx).main_window_size;
        let display_size = cx.primary_display().map(|display| display.bounds().size);
        let window_size = initial_main_window_size(saved_size, display_size);

        let window_bounds = Bounds::centered(None, window_size, cx);
        let options = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(window_bounds)),
            #[cfg(not(target_os = "linux"))]
            titlebar: Some(gpui_component::TitleBar::title_bar_options()),
            window_min_size: Some(Size {
                width: px(640.),
                height: px(480.),
            }),
            window_background: gpui::WindowBackgroundAppearance::Transparent,
            #[cfg(target_os = "linux")]
            window_decorations: Some(gpui::WindowDecorations::Client),
            kind: WindowKind::Normal,
            app_owns_titlebar_drag: true,
            ..Default::default()
        };

        cx.spawn(async move |cx| {
            let main_window = cx.open_window(options, |window, cx| {
                window.activate_window();
                app_init::init_window_systems(window, cx);
                update::schedule_update_check(window, cx);
                let view = cx.new(|cx| OnetCliApp::new(window, cx));
                cx.new(|cx| Root::new(view, window, cx))
            })?;
            let main_window = main_window.into();

            while let Ok(request) = startup_request_rx.recv().await {
                if cx
                    .update_window(main_window, |_, window, cx| {
                        window.activate_window();
                        match request {
                            AppOpenRequest::ActivateAndOpenPaths(paths) => {
                                for path in paths {
                                    let input = file_open::FileOpenInput::Path(path);
                                    file_open::open_input(input, window, cx);
                                }
                            }
                            AppOpenRequest::Open(input) => {
                                file_open::open_input(input, window, cx);
                            }
                        }
                    })
                    .is_err()
                {
                    break;
                }
            }

            Ok::<_, anyhow::Error>(())
        })
        .detach();
    });
}

#[cfg(test)]
mod embedded_cli_removal_tests {
    use gpui::{px, size};
    use one_core::settings::MainWindowSize;

    #[test]
    fn main_does_not_route_business_cli() {
        let source = include_str!("main.rs");
        let handler_name = ["handle", "cli", "command"].join("_");

        assert!(!source.contains(&handler_name));
        assert!(source.contains("update::handle_update_command()"));
    }

    #[test]
    fn associated_files_are_accepted_from_startup_and_platform_events() {
        let source = include_str!("main.rs");

        assert!(source.contains("startup_arguments.remaining"));
        assert!(source.contains("app.on_open_urls"));
        assert!(source.contains("startup_request_rx.recv().await"));
        assert!(source.contains("file_open::open_input(input, window, cx)"));
    }

    #[test]
    fn windows_single_instance_gate_precedes_application_creation() {
        let source = include_str!("main.rs");
        let gate = source
            .find("windows_single_instance::claim_or_forward")
            .expect("Windows single-instance gate");
        let application = source
            .find("gpui_platform::application()")
            .expect("GPUI application creation");

        assert!(gate < application);
        assert!(source.contains("SingleInstanceOutcome::Forwarded => return"));
    }

    #[test]
    fn forwarded_startup_request_activates_existing_window_before_opening_files() {
        let source = include_str!("main.rs");
        let receiver = source
            .find("startup_request_rx.recv().await")
            .expect("forwarded startup request receiver");
        let activation = source[receiver..]
            .find("window.activate_window()")
            .expect("existing window activation");
        let open = source[receiver..]
            .find("file_open::open_input(input, window, cx)")
            .expect("forwarded file open");

        assert!(activation < open);
    }

    #[test]
    fn startup_schedules_file_association_migration() {
        let source = include_str!("main.rs");

        assert!(source.contains("file_association::schedule_registration(cx)"));
    }

    #[test]
    fn startup_migrates_legacy_application_directories_before_loading_assets() {
        let source = include_str!("main.rs");
        let resolution = source
            .find("resolve_app_paths")
            .expect("startup path resolution");
        let initialization = source
            .find("initialize_app_paths")
            .expect("startup path initialization");
        let migration = source
            .find("migrate_legacy_directories()")
            .expect("startup directory migration");
        let assets = source
            .find("AppAssets::new()")
            .expect("application asset initialization");

        assert!(resolution < migration);
        assert!(migration < initialization);
        assert!(migration < assets);
    }

    #[test]
    fn portable_mode_does_not_register_host_file_associations() {
        let source = include_str!("main.rs");

        assert!(source.contains("if !one_core::app_paths::is_portable()"));
        assert!(source.contains("file_association::schedule_registration(cx)"));
    }

    #[test]
    fn first_launch_uses_ninety_percent_of_display() {
        let actual = super::initial_main_window_size(None, Some(size(px(2000.0), px(1000.0))));

        assert_eq!(size(px(1800.0), px(900.0)), actual);
    }

    #[test]
    fn saved_window_size_is_restored_and_capped_to_display() {
        let saved = MainWindowSize::new(1600.0, 1200.0);
        let actual = super::initial_main_window_size(saved, Some(size(px(1200.0), px(800.0))));

        assert_eq!(size(px(1080.0), px(720.0)), actual);
    }

    #[test]
    fn custom_titlebar_drag_is_owned_by_the_application() {
        let source = include_str!("main.rs");
        let option = ["app_owns_titlebar", "_drag: true"].concat();

        assert!(source.contains(&option));
    }
}

#[cfg(test)]
mod native_driver_feature_contract_tests {
    fn feature_block(manifest: &str) -> &str {
        manifest
            .split_once("[features]")
            .map(|(_, features)| features)
            .unwrap_or_default()
            .split_once("[lints]")
            .map(|(features, _)| features)
            .unwrap_or_default()
    }

    fn dependency_is_optional_or_absent(manifest: &str, dependency: &str) -> bool {
        manifest
            .lines()
            .find(|line| line.trim_start().starts_with(&format!("{dependency} =")))
            .is_none_or(|line| line.contains("optional = true"))
    }

    #[test]
    fn builtin_native_driver_features_are_declared_and_default_off() {
        let manifest = include_str!("../Cargo.toml");
        let features = feature_block(manifest);
        let default_line = features
            .lines()
            .find(|line| line.trim_start().starts_with("default ="))
            .expect("main must declare default features");

        assert!(features.contains("builtin-redis ="));
        assert!(features.contains("builtin-mongodb ="));
        assert!(!default_line.contains("builtin-redis"));
        assert!(!default_line.contains("builtin-mongodb"));
    }

    #[test]
    fn direct_native_database_sdks_are_optional_or_absent() {
        let redis_view = include_str!("../../crates/redis_view/Cargo.toml");
        let mongodb_view = include_str!("../../crates/mongodb_view/Cargo.toml");
        let onetcli_runtime = include_str!("../../crates/onetcli_runtime/Cargo.toml");

        assert!(dependency_is_optional_or_absent(redis_view, "redis_client"));
        assert!(dependency_is_optional_or_absent(
            onetcli_runtime,
            "redis_client"
        ));
        assert!(dependency_is_optional_or_absent(mongodb_view, "mongodb"));
    }
}
