#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
// Apple ld emits an advisory when the very large test binary exceeds its compact
// unwind encoding limit. Keep normal linker diagnostics but silence this known
// test-only advisory.
#![cfg_attr(test, allow(linker_messages))]

rust_i18n::i18n!("locales", fallback = "en");

mod auth;

mod ai_chat_acp;
mod app_init;
mod connection_sort;
mod connection_visuals;
mod credential_vault;
mod env_file;
mod extension_connection_form;
mod extension_connection_tab;
mod extension_update;
mod file_association;
mod file_open;
mod home;
mod home_tab;
mod license;
mod local_terminal_profiles;
mod navigation_quick_open;
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
mod session_logs;
mod setting_tab;
mod settings;
mod shell_plugin_host;
mod shell_plugin_tab;
mod sync_conflict_dialog;
mod team_management;
mod universal_plugins;
mod update;
mod user_avatar;
#[cfg(any(target_os = "windows", test))]
mod windows_single_instance;

use crate::onetcli_app::{GlobalTabContainer, OnetCliApp};
use gpui::*;

use gpui_component::{DialogStateChanged, Root};
use gpui_component_assets::Assets;
use one_core::settings::{AppSettings, MainWindowSize, MainWindowState};
use std::path::PathBuf;
use std::sync::Arc;
use tracing::{info, warn};

struct AppAssets {
    builtin: Assets,
    driver: db::ipc::DriverAssetSource,
}

pub(crate) const NAVOP_ICON_ASSET_PATH: &str = "navop/app-icon.png";

/// Navop 自带品牌图标(TDengine/MQTT)。
///
/// 外部 gpui-component 的 `IconName` 无法在本仓库扩展变体,这些 SVG
/// 以 include_bytes 内嵌并按路径对外提供(路径常量定义在 one-core)。
fn navop_brand_icon(path: &str) -> Option<std::borrow::Cow<'static, [u8]>> {
    let bytes: &'static [u8] = match path {
        one_core::storage::NAVOP_TDENGINE_COLOR_ICON => {
            include_bytes!("../../resources/icons/tdengine-color.svg")
        }
        one_core::storage::NAVOP_TDENGINE_LINE_COLOR_ICON => {
            include_bytes!("../../resources/icons/tdengine-line-color.svg")
        }
        one_core::storage::NAVOP_MQTT_COLOR_ICON => {
            include_bytes!("../../resources/icons/mqtt-color.svg")
        }
        one_core::storage::NAVOP_MQTT_LINE_ICON => {
            include_bytes!("../../resources/icons/mqtt-line.svg")
        }
        _ => return None,
    };
    Some(std::borrow::Cow::Borrowed(bytes))
}

const NAVOP_APP_ID: &str = "navop";
const NAVOP_WINDOW_TITLE: &str = "Navop";
const DEFAULT_MAIN_WINDOW_WIDTH: f32 = 1800.0;
const DEFAULT_MAIN_WINDOW_HEIGHT: f32 = 1260.0;
const MAIN_WINDOW_DISPLAY_RATIO: f32 = 0.9;

enum AppOpenRequest {
    ActivateAndOpenPaths(Vec<PathBuf>),
    Open(file_open::FileOpenInput),
}

fn default_main_window_size(display_size: Option<Size<Pixels>>) -> Size<Pixels> {
    display_size
        .map(|display_size| {
            size(
                px(f32::from(display_size.width) * MAIN_WINDOW_DISPLAY_RATIO),
                px(f32::from(display_size.height) * MAIN_WINDOW_DISPLAY_RATIO),
            )
        })
        .unwrap_or_else(|| {
            size(
                px(DEFAULT_MAIN_WINDOW_WIDTH),
                px(DEFAULT_MAIN_WINDOW_HEIGHT),
            )
        })
}

fn initial_main_window_size(
    saved_state: Option<&MainWindowState>,
    legacy_saved_size: Option<MainWindowSize>,
    display_size: Option<Size<Pixels>>,
) -> Size<Pixels> {
    let Some(display_size) = display_size else {
        return saved_state
            .and_then(|saved| MainWindowSize::new(saved.width, saved.height))
            .or_else(|| {
                legacy_saved_size.and_then(|saved| MainWindowSize::new(saved.width, saved.height))
            })
            .map(|saved| size(px(saved.width), px(saved.height)))
            .unwrap_or_else(|| default_main_window_size(None));
    };

    let default_size = default_main_window_size(Some(display_size));
    let saved_size = saved_state
        .and_then(|saved| MainWindowSize::new(saved.width, saved.height))
        .or_else(|| {
            legacy_saved_size.and_then(|saved| MainWindowSize::new(saved.width, saved.height))
        });
    let Some(saved_size) = saved_size else {
        return default_size;
    };

    let saved_size = size(px(saved_size.width), px(saved_size.height));
    if saved_size.width <= display_size.width && saved_size.height <= display_size.height {
        saved_size
    } else {
        default_size
    }
}

fn bounds_fit_display(window_bounds: Bounds<Pixels>, display_bounds: Bounds<Pixels>) -> bool {
    window_bounds.origin.x >= display_bounds.origin.x
        && window_bounds.origin.y >= display_bounds.origin.y
        && window_bounds.right() <= display_bounds.right()
        && window_bounds.bottom() <= display_bounds.bottom()
}

fn initial_main_window_bounds(
    saved_state: Option<&MainWindowState>,
    legacy_saved_size: Option<MainWindowSize>,
    display: Option<&dyn PlatformDisplay>,
) -> Bounds<Pixels> {
    initial_main_window_bounds_for_display(
        saved_state,
        legacy_saved_size,
        display.map(|display| display.visible_bounds()),
    )
}

fn initial_main_window_bounds_for_display(
    saved_state: Option<&MainWindowState>,
    legacy_saved_size: Option<MainWindowSize>,
    display_bounds: Option<Bounds<Pixels>>,
) -> Bounds<Pixels> {
    let Some(display_bounds) = display_bounds else {
        return Bounds::new(
            point(px(0.0), px(0.0)),
            initial_main_window_size(saved_state, legacy_saved_size, None),
        );
    };

    let window_size =
        initial_main_window_size(saved_state, legacy_saved_size, Some(display_bounds.size));
    if let Some(saved_state) = saved_state
        && let Some(saved_bounds) = MainWindowState::new(
            saved_state.x,
            saved_state.y,
            saved_state.width,
            saved_state.height,
            saved_state.display_uuid.clone(),
        )
        .map(|saved| {
            Bounds::new(
                point(px(saved.x), px(saved.y)),
                size(px(saved.width), px(saved.height)),
            )
        })
        && saved_bounds.size == window_size
        && bounds_fit_display(saved_bounds, display_bounds)
    {
        return saved_bounds;
    }

    Bounds::centered_at(display_bounds.center(), window_size)
}

fn main_window_options(
    window_bounds: Bounds<Pixels>,
    display_id: Option<DisplayId>,
) -> WindowOptions {
    let mut titlebar = gpui_component::TitleBar::title_bar_options();
    titlebar.title = Some(NAVOP_WINDOW_TITLE.into());

    WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(window_bounds)),
        titlebar: Some(titlebar),
        window_min_size: Some(size(px(640.0), px(480.0))),
        window_background: WindowBackgroundAppearance::Transparent,
        display_id,
        #[cfg(target_os = "linux")]
        window_decorations: Some(WindowDecorations::Client),
        kind: WindowKind::Normal,
        app_id: Some(NAVOP_APP_ID.to_owned()),
        app_owns_titlebar_drag: true,
        ..Default::default()
    }
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
        if path == NAVOP_ICON_ASSET_PATH {
            return Ok(Some(std::borrow::Cow::Borrowed(include_bytes!(
                "../../resources/navop-icon.png"
            ))));
        }

        if let Some(asset) = navop_brand_icon(path) {
            return Ok(Some(asset));
        }

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
    env_file::load_env_files();

    // GPUI 的 Windows 平台默认通过 DirectComposition visual 呈现窗口内容，
    // 该 visual 会盖住传统 child HWND（例如 RDP ActiveX 控件），即使连接
    // 成功、child 可见，远端桌面区域也表现为白屏。原生 RDP 后端必须在
    // platform 单例构造之前让 GPUI 走经典 HWND swap-chain 路径，与
    // `tools/gpui-rdp-smoke` 保持一致；该环境变量只在构造时读取一次。
    // 使用共享编译期标记（`remote_desktop_view/windows-native-rdp` 也会
    // 启用它），而不是 main 自身的 feature，保证两种 feature 写法都生效。
    if remote_desktop::windows_native_rdp_compiled() {
        // SAFETY: 进程尚未创建 GPUI platform，也没有任何线程会读取该
        // 环境变量；与 smoke 工具在进程首部执行相同操作。
        unsafe {
            std::env::set_var("GPUI_DISABLE_DIRECT_COMPOSITION", "1");
        }
    }

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
        db::ipc::set_host_version(env!("CARGO_PKG_VERSION"))
            .expect("main package version must be valid semver");
        extension_runtime::set_current_host_version(env!("CARGO_PKG_VERSION"))
            .expect("main package version must be valid semver");
        if let Err(error) = onetcli_app::init(cx) {
            tracing::error!(error = %error, "failed to initialize Navop application state");
            eprintln!("Failed to initialize Navop application state: {error:#}");
            cx.quit();
            return;
        }
        if !one_core::app_paths::is_portable() {
            file_association::schedule_registration(cx);
        }
        notes::init(cx);
        #[cfg(feature = "api-testing")]
        api_tools::init(cx);
        extension_runtime::init(cx);
        universal_plugins::init(cx);

        let settings = AppSettings::current(cx);
        let saved_state = settings.main_window_state.as_ref();
        let saved_display = saved_state.and_then(|saved_state| {
            saved_state.display_uuid.as_deref().and_then(|saved_uuid| {
                cx.displays().into_iter().find(|display| {
                    display
                        .uuid()
                        .ok()
                        .is_some_and(|uuid| uuid.to_string() == saved_uuid)
                })
            })
        });
        let display = saved_display.clone().or_else(|| cx.primary_display());
        let state_to_restore = if saved_display.is_some()
            || saved_state.is_some_and(|state| state.display_uuid.is_none())
        {
            saved_state
        } else {
            None
        };
        let legacy_saved_size = saved_state
            .is_none()
            .then_some(settings.main_window_size)
            .flatten();
        let window_bounds =
            initial_main_window_bounds(state_to_restore, legacy_saved_size, display.as_deref());
        let options =
            main_window_options(window_bounds, display.as_ref().map(|display| display.id()));

        cx.spawn(async move |cx| {
            let main_window = match cx.open_window(options, |window, cx| {
                window.activate_window();
                app_init::init_window_systems(window, cx);
                update::schedule_update_check(window, cx);
                extension_update::schedule_plugin_update_check(window, cx);
                let view = cx.new(|cx| OnetCliApp::new(window, cx));
                let root = cx.new(|cx| Root::new(view, window, cx));
                let tab_container = cx.global::<GlobalTabContainer>().tab_container.clone();
                cx.subscribe(&root, move |_, event: &DialogStateChanged, cx| {
                    tab_container.update(cx, |tabs, cx| {
                        tabs.set_active_presentation_obscured_by_dialog(event.active_count > 0, cx);
                    });
                })
                .detach();
                root
            }) {
                Ok(window) => window,
                Err(error) => {
                    tracing::error!(error = %error, "failed to open the Navop main window");
                    eprintln!("Failed to open the Navop main window: {error:#}");
                    let _ = cx.update(|cx| {
                        onetcli_app::shutdown_application_resources_and_quit(
                            cx,
                            "main window initialization failed",
                        );
                    });
                    return Ok::<_, anyhow::Error>(());
                }
            };
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
    use one_core::settings::{MainWindowSize, MainWindowState};

    #[test]
    fn main_does_not_route_business_cli() {
        let source = include_str!("main.rs");
        let handler_name = ["handle", "cli", "command"].join("_");

        assert!(!source.contains(&handler_name));
        assert!(source.contains("update::handle_update_command()"));
    }

    #[test]
    fn startup_loads_environment_files_before_handling_commands() {
        let source = include_str!("main.rs");
        let load = source
            .find("env_file::load_env_files()")
            .expect("environment file loading");
        let update = source
            .find("update::handle_update_command()")
            .expect("update command handling");

        assert!(load < update);
    }

    #[test]
    fn environment_files_are_disabled_for_release_builds() {
        let runtime_loader = include_str!("env_file.rs").replace("\r\n", "\n");
        let build_script = include_str!("../../crates/core/build.rs");

        assert!(
            runtime_loader.contains("#[cfg(not(debug_assertions))]\npub fn load_env_files() {}")
        );
        assert!(build_script.contains("std::env::var(\"PROFILE\").as_deref() == Ok(\"debug\")"));
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
    fn windows_native_rdp_disables_direct_composition_before_application_creation() {
        let source = include_str!("main.rs");
        let marker = source
            .find("remote_desktop::windows_native_rdp_compiled()")
            .expect("windows-native-rdp capability marker");
        let setter = source
            .find("GPUI_DISABLE_DIRECT_COMPOSITION")
            .expect("windows-native-rdp must disable GPUI DirectComposition");
        let application = source
            .find("gpui_platform::application()")
            .expect("GPUI application creation");

        assert!(marker < setter);
        assert!(setter < application);
        assert!(source.contains("std::env::set_var(\"GPUI_DISABLE_DIRECT_COMPOSITION\", \"1\")"));
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
    fn main_window_dialog_state_obscures_active_native_presentation() {
        let source = include_str!("main.rs").replace("\r\n", "\n");

        let _ = std::any::TypeId::of::<gpui_component::DialogStateChanged>();
        assert!(source.contains("use gpui_component::{DialogStateChanged, Root};"));
        assert!(source.contains("let root = cx.new(|cx| Root::new(view, window, cx));"));
        assert!(source.contains("cx.subscribe(&root,"));
        assert!(source.contains("event: &DialogStateChanged"));
        assert!(source.contains("event.active_count > 0"));
        assert!(source.contains("set_active_presentation_obscured_by_dialog"));
        assert!(source.contains(".detach();\n                root"));
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
    fn main_window_open_failure_is_reported_and_quits() {
        let source = include_str!("main.rs");
        let open = source
            .find("let main_window = match cx.open_window")
            .expect("main window open error handling");
        let request_loop = source[open..]
            .find("while let Ok(request)")
            .expect("startup request loop");
        let error_path = &source[open..open + request_loop];

        assert!(error_path.contains("failed to open the Navop main window"));
        assert!(error_path.contains("Failed to open the Navop main window: {error:#}"));
        assert!(error_path.contains("shutdown_application_resources_and_quit"));
    }

    #[test]
    fn first_launch_uses_ninety_percent_of_display() {
        let actual =
            super::initial_main_window_size(None, None, Some(size(px(2000.0), px(1000.0))));

        assert_eq!(size(px(1800.0), px(900.0)), actual);
    }

    #[test]
    fn saved_window_size_falls_back_to_default_when_it_does_not_fit() {
        let saved = MainWindowState::new(0.0, 0.0, 1600.0, 1200.0, None);
        let actual = super::initial_main_window_size(
            saved.as_ref(),
            None,
            Some(size(px(1200.0), px(800.0))),
        );

        assert_eq!(size(px(1080.0), px(720.0)), actual);
    }

    #[test]
    fn saved_window_size_is_restored_when_it_fits_display() {
        let saved = MainWindowState::new(100.0, 120.0, 1000.0, 700.0, None);
        let actual = super::initial_main_window_size(
            saved.as_ref(),
            None,
            Some(size(px(1200.0), px(800.0))),
        );

        assert_eq!(size(px(1000.0), px(700.0)), actual);
    }

    #[test]
    fn legacy_saved_window_size_is_restored() {
        let saved = MainWindowSize::new(1000.0, 700.0);
        let actual =
            super::initial_main_window_size(None, saved, Some(size(px(1200.0), px(800.0))));

        assert_eq!(size(px(1000.0), px(700.0)), actual);
    }

    #[test]
    fn bounds_fit_display_requires_the_entire_window_to_be_visible() {
        let display = gpui::Bounds::new(
            gpui::point(px(-1920.0), px(0.0)),
            size(px(1920.0), px(1080.0)),
        );

        assert!(super::bounds_fit_display(
            gpui::Bounds::new(
                gpui::point(px(-1800.0), px(100.0)),
                size(px(1200.0), px(800.0)),
            ),
            display,
        ));
        assert!(!super::bounds_fit_display(
            gpui::Bounds::new(
                gpui::point(px(-1800.0), px(100.0)),
                size(px(1800.0), px(1000.0)),
            ),
            display,
        ));
    }

    #[test]
    fn saved_window_bounds_restore_position_on_the_target_display() {
        let display = gpui::Bounds::new(
            gpui::point(px(-1920.0), px(0.0)),
            size(px(1920.0), px(1080.0)),
        );
        let saved = MainWindowState::new(-1800.0, 100.0, 1200.0, 800.0, Some("display-2".into()));

        let actual =
            super::initial_main_window_bounds_for_display(saved.as_ref(), None, Some(display));

        assert_eq!(
            gpui::Bounds::new(
                gpui::point(px(-1800.0), px(100.0)),
                size(px(1200.0), px(800.0)),
            ),
            actual,
        );
    }

    #[test]
    fn saved_window_bounds_center_when_the_saved_position_is_not_visible() {
        let display = gpui::Bounds::new(
            gpui::point(px(-1920.0), px(0.0)),
            size(px(1920.0), px(1080.0)),
        );
        let saved = MainWindowState::new(100.0, 100.0, 1200.0, 800.0, Some("display-2".into()));

        let actual =
            super::initial_main_window_bounds_for_display(saved.as_ref(), None, Some(display));

        assert_eq!(
            gpui::Bounds::new(
                gpui::point(px(-1560.0), px(140.0)),
                size(px(1200.0), px(800.0)),
            ),
            actual,
        );
    }

    #[test]
    fn custom_titlebar_drag_is_owned_by_the_application() {
        let source = include_str!("main.rs");
        let option = ["app_owns_titlebar", "_drag: true"].concat();

        assert!(source.contains(&option));
    }

    #[test]
    fn main_window_identifies_itself_to_desktop_environment() {
        let bounds = gpui::Bounds {
            origin: gpui::point(px(0.0), px(0.0)),
            size: size(px(800.0), px(600.0)),
        };
        let options = super::main_window_options(bounds, None);

        assert_eq!(Some("navop"), options.app_id.as_deref());
        assert_eq!(
            Some("Navop"),
            options
                .titlebar
                .as_ref()
                .and_then(|titlebar| titlebar.title.as_deref())
        );
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
    fn windows_native_rdp_feature_is_declared_and_enabled_by_default() {
        let main_manifest = include_str!("../Cargo.toml");
        let remote_desktop_view_manifest =
            include_str!("../../crates/remote_desktop_view/Cargo.toml");
        let main_features = feature_block(main_manifest);
        let remote_desktop_view_features = feature_block(remote_desktop_view_manifest);
        let main_default = main_features
            .lines()
            .find(|line| line.trim_start().starts_with("default ="))
            .expect("main must declare default features");
        let remote_desktop_view_default = remote_desktop_view_features
            .lines()
            .find(|line| line.trim_start().starts_with("default ="))
            .expect("remote_desktop_view must declare default features");

        assert!(
            main_features
                .contains("windows-native-rdp = [\"remote_desktop_view/windows-native-rdp\"]")
        );
        assert!(
            main_default.contains("windows-native-rdp"),
            "the Windows native RDP backend must be part of the default build"
        );
        assert!(
            remote_desktop_view_features.contains(
                "windows-native-rdp = [\n    \"dep:raw-window-handle\",\n    \"dep:windows_rdp_host\",\n    \"remote_desktop/windows-native-rdp\",\n]"
            ),
            "the feature must enable native presentation dependencies and the shared capability marker"
        );
        assert_eq!("default = []", remote_desktop_view_default.trim());
        assert!(dependency_is_optional_or_absent(
            remote_desktop_view_manifest,
            "raw-window-handle"
        ));
        assert!(dependency_is_optional_or_absent(
            remote_desktop_view_manifest,
            "windows_rdp_host"
        ));
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
