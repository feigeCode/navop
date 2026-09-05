use crate::home_tab::{
    HomePage, NewConnectionShortcut, OpenConnectionQuickOpen, OpenLocalTerminalShortcut,
};
use crate::persistent_connection_sidebar::{
    PersistentConnectionSidebar, PersistentConnectionSidebarEvent,
};
use gpui::prelude::FluentBuilder as _;
use gpui::{
    AnyElement, App, AppContext, AsyncApp, ColorExt as _, Context, Entity, ExternalPaths,
    Focusable, InteractiveElement, IntoElement, KeyBinding, Keystroke, ParentElement, Render,
    Styled, Task, Window, actions, div,
};
use gpui_component::{WindowExt, dialog::DialogButtonProps, kbd::Kbd, notification::Notification};
use one_core::gpui_tokio::{JoinError, Tokio};
use one_core::keybindings::{action_id, rebind_keybindings, shortcuts_for};
use raw_window_handle::HasWindowHandle;
#[cfg(any(target_os = "macos", target_os = "windows"))]
use raw_window_handle::RawWindowHandle;
use rust_i18n::t;
use ssh::{SshSessionService, SshSessionShutdownReport};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

static ALWAYS_ON_TOP: AtomicBool = AtomicBool::new(false);

struct AlwaysOnTopNotification;

actions!(
    onetcli_app,
    [
        ActivateTab1,
        ActivateTab2,
        ActivateTab3,
        ActivateTab4,
        ActivateTab5,
        ActivateTab6,
        ActivateTab7,
        ActivateTab8,
        ActivateTab9,
        ToggleFullscreen,
        ToggleAlwaysOnTop,
        MinimizeWindow,
        DuplicateTab,
        OpenTabSwitcher,
        SwitchNextTab,
        SwitchPreviousTab,
        ToggleConnectionSidebar,
        CloseActiveWindow,
        QuitApp,
    ]
);

#[derive(Clone)]
pub struct GlobalTabContainer {
    pub tab_container: Entity<TabContainer>,
}

impl gpui::Global for GlobalTabContainer {}

impl GlobalTabContainer {
    pub fn primary_pane(&self) -> Entity<TabContainer> {
        self.tab_container.clone()
    }
}

#[derive(Clone)]
pub struct GlobalHomePage {
    pub home_page: Entity<HomePage>,
}

impl gpui::Global for GlobalHomePage {}

#[derive(Clone)]
pub struct GlobalOnetCliApp {
    pub app: Entity<OnetCliApp>,
}

impl gpui::Global for GlobalOnetCliApp {}

/// The tab-bar Home button lives in the generic tab container, which cannot
/// reach the app-level HomePage; route clicks through the global lookup.
fn home_button_handler() -> Arc<dyn Fn(&mut Window, &mut App) + Send + Sync> {
    Arc::new(|window: &mut Window, cx: &mut App| {
        let Some(home_page) = cx
            .try_global::<GlobalHomePage>()
            .map(|global| global.home_page.clone())
        else {
            return;
        };
        HomePage::show_home(&home_page, window, cx);
    })
}

fn add_tab_button_handler() -> Arc<dyn Fn(&mut Window, &mut App) + Send + Sync> {
    Arc::new(|window: &mut Window, cx: &mut App| {
        let Some(home_page) = cx
            .try_global::<GlobalHomePage>()
            .map(|global| global.home_page.clone())
        else {
            return;
        };
        home_page.update(cx, |home, cx| home.show_connection_quick_open(window, cx));
    })
}

fn tab_quick_open_resolver() -> one_core::tab_switcher::QuickOpenResolver {
    Arc::new(|query| {
        let connection = crate::home::home_connection_quick_open::temporary_ssh_connection(query)?;
        let title = connection.name.clone().into();
        let action: one_core::tab_switcher::QuickOpenAction = Arc::new(move |window, cx| {
            let Some(home) = cx.try_global::<GlobalHomePage>().map(|global| global.home_page.clone()) else {
                return;
            };
            home.update(cx, |home, cx| home.open_connection_from_quick(&connection, window, cx));
        });
        Some((title, action))
    })
}

#[test]
fn tab_quick_open_resolves_only_supported_ssh_commands() {
    let resolver = tab_quick_open_resolver();
    for query in ["ssh host", "ssh -p 2222 user@host", "ssh user@host -p2222"] {
        let (title, _) = resolver(query).expect("SSH command should produce an actionable result");
        assert!(title.contains("host"));
    }
    for query in ["", "Settings", "ssh -p 0 host", "ssh -i key host"] {
        assert!(resolver(query).is_none(), "{query}");
    }
}

/// The tab-bar Settings button follows the same pattern: the generic tab
/// container cannot construct the app-owned settings tab, so clicks are routed
/// through the global HomePage lookup.
fn settings_button_handler() -> Arc<dyn Fn(&mut Window, &mut App) + Send + Sync> {
    Arc::new(|window: &mut Window, cx: &mut App| {
        let Some(home_page) = cx
            .try_global::<GlobalHomePage>()
            .map(|global| global.home_page.clone())
        else {
            return;
        };
        home_page.update(cx, |home, cx| {
            home.add_settings_tab(window, cx);
        });
    })
}

/// The application-owned SSH session lifecycle.
///
/// The `ssh` crate deliberately stays independent of GPUI. This narrow
/// wrapper installs exactly one service in the application and lets app code
/// pass service clones to consumers without making any terminal or file view
/// the owner of shared transports.
#[derive(Clone)]
pub(crate) struct GlobalSshSessionService {
    service: SshSessionService,
}

impl gpui::Global for GlobalSshSessionService {}

impl GlobalSshSessionService {
    fn new() -> Self {
        Self {
            service: SshSessionService::new(),
        }
    }

    pub(crate) fn service(&self) -> SshSessionService {
        self.service.clone()
    }
}

fn init_ssh_session_service(cx: &mut App) {
    assert!(
        cx.try_global::<GlobalSshSessionService>().is_none(),
        "SSH session service must have exactly one application owner"
    );

    let global = GlobalSshSessionService::new();
    let fallback_service = global.service();
    cx.set_global(global);

    // Normal Navop quit paths await shutdown before asking GPUI to quit.
    // This callback is an idempotent fallback for platform-driven quit paths.
    // GPUI only gives quit callbacks a short fixed budget, so it must not be
    // the primary owner shutdown path.
    cx.on_app_quit(move |cx| {
        let rdp_shutdown_report =
            remote_desktop_view::fail_closed_windows_native_rdp_for_platform_quit(cx);
        log_windows_native_rdp_shutdown("gpui quit fallback", rdp_shutdown_report);

        let service = fallback_service.clone();
        let shutdown_task = Tokio::spawn(cx, async move { service.shutdown().await });
        async move {
            log_ssh_session_shutdown("gpui quit fallback", shutdown_task.await);
        }
    })
    .detach();
}

fn spawn_ssh_session_shutdown(
    cx: &App,
) -> Option<Task<Result<SshSessionShutdownReport, JoinError>>> {
    let service = cx.try_global::<GlobalSshSessionService>()?.service();
    Some(Tokio::spawn(cx, async move { service.shutdown().await }))
}

fn log_ssh_session_shutdown(
    reason: &'static str,
    result: Result<SshSessionShutdownReport, JoinError>,
) {
    match result {
        Ok(report)
            if report.timed_out
                || report.manager_failures > 0
                || report.managers_remaining > 0
                || report.registry_tasks_remaining > 0 =>
        {
            tracing::warn!(
                reason,
                timed_out = report.timed_out,
                managers_requested = report.managers_requested,
                managers_completed = report.managers_completed,
                manager_failures = report.manager_failures,
                managers_remaining = report.managers_remaining,
                registry_tasks_remaining = report.registry_tasks_remaining,
                "SSH session service shutdown completed with incomplete cleanup"
            );
        }
        Ok(report) => {
            tracing::info!(
                reason,
                managers_requested = report.managers_requested,
                managers_completed = report.managers_completed,
                "SSH session service shutdown completed"
            );
        }
        Err(error) => {
            tracing::error!(
                reason,
                %error,
                "SSH session service shutdown task failed"
            );
        }
    }
}

fn log_windows_native_rdp_shutdown(
    reason: &'static str,
    report: remote_desktop_view::WindowsNativeRdpShutdownReport,
) {
    if report.incomplete() {
        tracing::warn!(
            reason,
            requested = report.requested(),
            destroyed = report.destroyed(),
            timed_out_leaked = report.timed_out_leaked(),
            owner_lost = report.owner_lost(),
            controller_unavailable = report.controller_unavailable(),
            "Windows native RDP shutdown completed with incomplete cleanup"
        );
    } else {
        tracing::info!(
            reason,
            requested = report.requested(),
            destroyed = report.destroyed(),
            "Windows native RDP shutdown completed"
        );
    }
}

/// Await bounded application-owned resource teardown before invoking GPUI's
/// platform quit routine. Native RDP hosts drain before their shared SSH
/// transports so COM/child-window cleanup retains a live application owner.
///
/// This is intentionally the only production helper that calls `cx.quit()`.
/// Repeated callers join the same idempotent Native RDP drain and
/// `SshSessionService::shutdown` lifecycle.
pub(crate) fn shutdown_application_resources_and_quit(cx: &mut App, reason: &'static str) {
    let rdp_shutdown_task = remote_desktop_view::shutdown_windows_native_rdp(cx);

    cx.spawn(async move |cx| {
        let rdp_shutdown_report = rdp_shutdown_task.await;
        log_windows_native_rdp_shutdown(reason, rdp_shutdown_report);

        let ssh_shutdown_task = cx.update(|cx| spawn_ssh_session_shutdown(cx));
        if let Some(shutdown_task) = ssh_shutdown_task {
            let shutdown_result = shutdown_task.await;
            log_ssh_session_shutdown(reason, shutdown_result);
        } else {
            tracing::error!(
                reason,
                "SSH session service global is missing; quitting after remaining application teardown"
            );
        }

        #[cfg(feature = "shell-plugins")]
        let plugin_shutdown_task = cx.update(|cx| crate::universal_plugins::spawn_shutdown(cx));
        #[cfg(feature = "shell-plugins")]
        if let Some(shutdown_task) = plugin_shutdown_task {
            if let Err(error) = shutdown_task.await {
                tracing::warn!(
                    reason,
                    %error,
                    "Universal plugin shutdown did not complete"
                );
            }
        } else {
            tracing::error!(
                reason,
                "Universal plugin service global is missing; quitting after remaining application teardown"
            );
        }

        let _ = cx.update(|cx| cx.quit());
    })
    .detach();
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct InitialContentLayout {
    home_tab_id: &'static str,
    workbench_tab_id: &'static str,
    pin_home: bool,
    pin_workbench: bool,
    active_pinned_index: Option<usize>,
    main_content: MainContent,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum QuitRequestDecision {
    OpenPrompt,
    Ignore,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct QuitRequestState {
    prompt_open: bool,
    in_progress: bool,
}

impl QuitRequestState {
    fn request(&mut self) -> QuitRequestDecision {
        if self.prompt_open || self.in_progress {
            return QuitRequestDecision::Ignore;
        }
        self.prompt_open = true;
        QuitRequestDecision::OpenPrompt
    }

    fn cancel_prompt(&mut self) {
        self.prompt_open = false;
    }

    fn confirm_prompt(&mut self) -> bool {
        if self.in_progress {
            return false;
        }
        self.prompt_open = false;
        self.in_progress = true;
        true
    }

    fn finish_close(&mut self, closed: bool) {
        if !closed {
            self.in_progress = false;
        }
    }
}

fn initial_content_layout(
    home_page_style: HomePageStyle,
    startup_default_page: StartupDefaultPage,
) -> InitialContentLayout {
    let pin_home = home_page_style == HomePageStyle::Legacy;
    let pin_workbench = startup_default_page == StartupDefaultPage::AiWorkbench;
    InitialContentLayout {
        home_tab_id: "home",
        workbench_tab_id: "ai-workbench",
        pin_home,
        pin_workbench,
        active_pinned_index: match (pin_home, pin_workbench, startup_default_page) {
            (true, true, _) => Some(1),
            (true, false, _) => Some(0),
            (false, true, _) => Some(0),
            (false, false, _) => None,
        },
        main_content: if pin_home || pin_workbench {
            MainContent::Tabs
        } else {
            MainContent::Home
        },
    }
}

#[cfg(target_os = "macos")]
use gpui::px;

use gpui_component::dock::ToggleZoom;
use gpui_component::{ActiveTheme, Root};
use one_core::llm::manager::GlobalProviderState;
use one_core::llm::notifier::emit_provider_config_changed;
use one_core::llm::storage::{ProviderRepository, refresh_onetcli_models};
use one_core::settings::{
    AppSettings, GlobalCurrentUser, HomePageStyle, MainWindowState, StartupDefaultPage,
};
use one_core::storage::manager::get_config_dir;
use one_core::tab_container::{TabContainer, TabContainerEvent, TabContentRegistry, TabItem};
use one_core::tab_navigation::{
    ActiveTabSlot, TabCycleDirection, tab_number_target, tab_slot_after_cycle,
};
use one_core::themes;
use sftp_view::{PasteUpload as SftpPasteUpload, SFTP_VIEW_CONTEXT};
#[cfg(unix)]
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};

use crate::setting_tab;
use db::GlobalDbState;
use one_core::storage::{ConnectionRepository, GlobalStorageState, traits::Repository};

fn activate_tab_by_number(number: usize, cx: &mut App) {
    let Some(active_window) = cx.active_window() else {
        return;
    };
    let Some(container) = cx.try_global::<GlobalTabContainer>() else {
        return;
    };
    let container = container.tab_container.clone();

    cx.defer(move |cx| {
        _ = active_window.update(cx, |_, window, cx| {
            container.update(cx, |tc, cx| {
                match tab_number_target(number, tc.pinned_tab_count(), tc.tabs().len()) {
                    Some(ActiveTabSlot::Pinned(index)) => {
                        tc.activate_pinned_tab_at(index, window, cx);
                    }
                    Some(ActiveTabSlot::Regular(index)) => {
                        tc.set_active_index(index, window, cx);
                    }
                    None => {}
                }
            });
        });
    });
}

fn switch_tab(direction: TabCycleDirection, cx: &mut App) {
    let Some(active_window) = cx.active_window() else {
        return;
    };
    let Some(container) = cx.try_global::<GlobalTabContainer>() else {
        return;
    };
    let container = container.tab_container.clone();

    cx.defer(move |cx| {
        _ = active_window.update(cx, |_, window, cx| {
            container.update(cx, |tc, cx| {
                let active_slot = tc
                    .active_pinned_index()
                    .map(ActiveTabSlot::Pinned)
                    .unwrap_or_else(|| ActiveTabSlot::Regular(tc.active_index()));
                let Some(next_slot) = tab_slot_after_cycle(
                    active_slot,
                    tc.pinned_tab_count(),
                    tc.tabs().len(),
                    direction,
                ) else {
                    return;
                };
                match next_slot {
                    ActiveTabSlot::Pinned(index) => tc.activate_pinned_tab_at(index, window, cx),
                    ActiveTabSlot::Regular(index) => tc.set_active_index(index, window, cx),
                }
            });
        });
    });
}

fn open_tab_switcher(cx: &mut App) {
    let Some(active_window) = cx.active_window() else {
        return;
    };
    let Some(container) = cx.try_global::<GlobalTabContainer>() else {
        return;
    };
    let container = container.primary_pane();

    cx.defer(move |cx| {
        _ = active_window.update(cx, |_, window, cx| {
            container.update(cx, |tc, cx| {
                tc.open_tab_switcher(window, cx);
            });
        });
    });
}

fn toggle_fullscreen(cx: &mut App) {
    let Some(active_window) = cx.active_window() else {
        return;
    };
    cx.defer(move |cx| {
        _ = active_window.update(cx, |_, window, _| {
            window.toggle_fullscreen();
        });
    });
}

fn toggle_always_on_top(cx: &mut App) {
    let Some(active_window) = cx.active_window() else {
        return;
    };
    cx.defer(move |cx| {
        _ = active_window.update(cx, |_, window, cx| {
            let next = next_always_on_top_state(window);
            let shortcut = always_on_top_shortcut_label(cx);
            match set_window_always_on_top(window, next) {
                Ok(()) => {
                    ALWAYS_ON_TOP.store(next, Ordering::Relaxed);
                    #[cfg(target_os = "macos")]
                    if should_activate_after_always_on_top_change(next) {
                        window.activate_window();
                    }
                    show_always_on_top_notification(window, cx, next, &shortcut);
                }
                Err(err) => {
                    tracing::warn!("窗口置顶切换失败: {err:?}");
                    show_always_on_top_error_notification(window, cx, &shortcut, &err);
                }
            }
        });
    });
}

fn always_on_top_shortcut_label(cx: &App) -> String {
    let shortcut = shortcuts_for(
        cx,
        action_id::WINDOW_TOGGLE_ALWAYS_ON_TOP,
        &[default_shortcut("ctrl-cmd-t", "ctrl-alt-t")],
    )
    .into_iter()
    .next()
    .unwrap_or_else(|| default_shortcut("ctrl-cmd-t", "ctrl-alt-t").to_string());
    shortcut_label(&shortcut)
}

fn shortcut_label(shortcut: &str) -> String {
    Keystroke::parse(shortcut)
        .map(|keystroke| Kbd::format(&keystroke).to_string())
        .unwrap_or_else(|_| shortcut.to_string())
}

fn always_on_top_notification_message(enabled: bool, shortcut: &str) -> String {
    if enabled {
        format!("窗口已置顶。再次按 {shortcut} 可取消置顶。")
    } else {
        format!("窗口已取消置顶。按 {shortcut} 可重新置顶。")
    }
}

fn always_on_top_error_notification_message(shortcut: &str, error: &anyhow::Error) -> String {
    format!("窗口置顶切换失败：{error:#}。可再次按 {shortcut} 重试。")
}

fn show_always_on_top_notification(
    window: &mut Window,
    cx: &mut App,
    enabled: bool,
    shortcut: &str,
) {
    let message = always_on_top_notification_message(enabled, shortcut);
    let notification = if enabled {
        Notification::success(message)
    } else {
        Notification::info(message)
    };
    window.push_notification(
        notification.id::<AlwaysOnTopNotification>().autohide(true),
        cx,
    );
}

fn show_always_on_top_error_notification(
    window: &mut Window,
    cx: &mut App,
    shortcut: &str,
    error: &anyhow::Error,
) {
    window.push_notification(
        Notification::error(always_on_top_error_notification_message(shortcut, error))
            .id::<AlwaysOnTopNotification>()
            .autohide(true),
        cx,
    );
}

fn next_always_on_top_state(_window: &Window) -> bool {
    let cached = ALWAYS_ON_TOP.load(Ordering::Relaxed);

    #[cfg(target_os = "macos")]
    let observed = window_always_on_top(_window).ok();
    #[cfg(not(target_os = "macos"))]
    let observed = None;

    next_always_on_top_from_state(cached, observed)
}

fn next_always_on_top_from_state(cached: bool, observed: Option<bool>) -> bool {
    !observed.unwrap_or(cached)
}

#[cfg(target_os = "macos")]
fn should_activate_after_always_on_top_change(always_on_top: bool) -> bool {
    always_on_top
}

fn set_window_always_on_top(window: &Window, _always_on_top: bool) -> anyhow::Result<()> {
    let handle = HasWindowHandle::window_handle(window)
        .map_err(|err| anyhow::anyhow!("获取窗口句柄失败: {err:?}"))?
        .as_raw();
    match handle {
        #[cfg(target_os = "macos")]
        RawWindowHandle::AppKit(handle) => {
            set_macos_always_on_top(handle.ns_view.as_ptr(), _always_on_top)
        }
        #[cfg(target_os = "windows")]
        RawWindowHandle::Win32(handle) => {
            set_windows_always_on_top(handle.hwnd.get(), _always_on_top)
        }
        _ => Err(anyhow::anyhow!("当前平台暂不支持窗口置顶")),
    }
}

#[cfg(target_os = "macos")]
const NS_NORMAL_WINDOW_LEVEL: isize = 0;
#[cfg(target_os = "macos")]
const NS_FLOATING_WINDOW_LEVEL: isize = 3;

#[cfg(target_os = "macos")]
fn with_macos_objc<R>(f: impl FnOnce(MacosObjcFns) -> R) -> R {
    type Id = *mut std::ffi::c_void;
    type Sel = *mut std::ffi::c_void;

    #[link(name = "objc")]
    unsafe extern "C" {
        #[link_name = "sel_registerName"]
        fn sel_register_name(name: *const std::ffi::c_char) -> Sel;
        #[link_name = "objc_msgSend"]
        fn objc_msg_send();
    }

    unsafe {
        let objc_msg_send = objc_msg_send as *const ();
        // objc_msgSend must be called with the exact Objective-C method ABI.
        f(MacosObjcFns {
            sel_register_name,
            objc_msg_send_id: std::mem::transmute::<*const (), unsafe extern "C" fn(Id, Sel) -> Id>(
                objc_msg_send,
            ),
            objc_msg_send_isize: std::mem::transmute::<
                *const (),
                unsafe extern "C" fn(Id, Sel) -> isize,
            >(objc_msg_send),
            objc_msg_send_void_isize: std::mem::transmute::<
                *const (),
                unsafe extern "C" fn(Id, Sel, isize),
            >(objc_msg_send),
        })
    }
}

#[cfg(target_os = "macos")]
struct MacosObjcFns {
    sel_register_name: unsafe extern "C" fn(*const std::ffi::c_char) -> *mut std::ffi::c_void,
    objc_msg_send_id:
        unsafe extern "C" fn(*mut std::ffi::c_void, *mut std::ffi::c_void) -> *mut std::ffi::c_void,
    objc_msg_send_isize:
        unsafe extern "C" fn(*mut std::ffi::c_void, *mut std::ffi::c_void) -> isize,
    objc_msg_send_void_isize:
        unsafe extern "C" fn(*mut std::ffi::c_void, *mut std::ffi::c_void, isize),
}

#[cfg(target_os = "macos")]
fn macos_window_level(always_on_top: bool) -> isize {
    if always_on_top {
        NS_FLOATING_WINDOW_LEVEL
    } else {
        NS_NORMAL_WINDOW_LEVEL
    }
}

#[cfg(target_os = "macos")]
fn is_macos_always_on_top_level(level: isize) -> bool {
    level != NS_NORMAL_WINDOW_LEVEL
}

#[cfg(target_os = "macos")]
fn macos_ns_window_from_view(
    ns_view: *mut std::ffi::c_void,
) -> anyhow::Result<*mut std::ffi::c_void> {
    if ns_view.is_null() {
        return Err(anyhow::anyhow!("获取 NSView 失败"));
    }

    let window_selector = std::ffi::CString::new("window")?;
    with_macos_objc(|objc| unsafe {
        let ns_window = (objc.objc_msg_send_id)(
            ns_view.cast(),
            (objc.sel_register_name)(window_selector.as_ptr()),
        );
        if ns_window.is_null() {
            Err(anyhow::anyhow!("获取 NSWindow 失败"))
        } else {
            Ok(ns_window)
        }
    })
}

#[cfg(target_os = "macos")]
fn macos_ns_window_level(ns_window: *mut std::ffi::c_void) -> anyhow::Result<isize> {
    let level_selector = std::ffi::CString::new("level")?;
    Ok(with_macos_objc(|objc| unsafe {
        (objc.objc_msg_send_isize)(ns_window, (objc.sel_register_name)(level_selector.as_ptr()))
    }))
}

#[cfg(target_os = "macos")]
fn set_macos_ns_window_level(ns_window: *mut std::ffi::c_void, level: isize) -> anyhow::Result<()> {
    let set_level_selector = std::ffi::CString::new("setLevel:")?;
    with_macos_objc(|objc| unsafe {
        (objc.objc_msg_send_void_isize)(
            ns_window,
            (objc.sel_register_name)(set_level_selector.as_ptr()),
            level,
        );
    });
    Ok(())
}

#[cfg(target_os = "macos")]
fn window_always_on_top(window: &Window) -> anyhow::Result<bool> {
    let handle = HasWindowHandle::window_handle(window)
        .map_err(|err| anyhow::anyhow!("获取窗口句柄失败: {err:?}"))?
        .as_raw();
    match handle {
        RawWindowHandle::AppKit(handle) => {
            let ns_window = macos_ns_window_from_view(handle.ns_view.as_ptr())?;
            Ok(is_macos_always_on_top_level(macos_ns_window_level(
                ns_window,
            )?))
        }
        _ => Err(anyhow::anyhow!("当前平台暂不支持读取窗口置顶状态")),
    }
}

#[cfg(target_os = "macos")]
fn set_macos_always_on_top(
    ns_view: *mut std::ffi::c_void,
    always_on_top: bool,
) -> anyhow::Result<()> {
    let ns_window = macos_ns_window_from_view(ns_view)?;
    let level = macos_window_level(always_on_top);
    set_macos_ns_window_level(ns_window, level)?;
    let actual = macos_ns_window_level(ns_window)?;
    if actual != level {
        return Err(anyhow::anyhow!(
            "设置 NSWindow level 失败: expected {level}, actual {actual}"
        ));
    }
    Ok(())
}

#[cfg(target_os = "windows")]
fn set_windows_always_on_top(hwnd: isize, always_on_top: bool) -> anyhow::Result<()> {
    use windows::Win32::Foundation::HWND;
    use windows::Win32::UI::WindowsAndMessaging::{
        HWND_NOTOPMOST, HWND_TOPMOST, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SetWindowPos,
    };

    let insert_after = if always_on_top {
        HWND_TOPMOST
    } else {
        HWND_NOTOPMOST
    };
    unsafe {
        SetWindowPos(
            HWND(hwnd as *mut _),
            Some(insert_after),
            0,
            0,
            0,
            0,
            SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
        )?;
    }
    Ok(())
}

fn duplicate_tab(cx: &mut App) {
    let Some(active_window) = cx.active_window() else {
        return;
    };
    let Some(home) = cx.try_global::<GlobalHomePage>() else {
        return;
    };
    let home_page = home.home_page.clone();

    cx.defer(move |cx| {
        _ = active_window.update(cx, |_, window, cx| {
            home_page.update(cx, |hp, cx| {
                hp.duplicate_active_tab(window, cx);
            });
        });
    });
}

fn quit_app(cx: &mut App) {
    request_active_window_quit(cx);
}

fn request_active_window_quit(cx: &mut App) {
    let Some(active_window) = cx.active_window() else {
        shutdown_application_resources_and_quit(cx, "quit without an active window");
        return;
    };
    cx.defer(move |cx| {
        _ = active_window.update(cx, |_, window, cx| {
            request_window_quit(window, cx);
        });
    });
}

fn request_window_quit(window: &mut Window, cx: &mut App) {
    let Some(app) = cx
        .try_global::<GlobalOnetCliApp>()
        .map(|global| global.app.clone())
    else {
        shutdown_application_resources_and_quit(cx, "quit without the application entity");
        return;
    };
    app.update(cx, |app, cx| {
        app.request_quit(window, cx);
    });
}

fn default_shortcut(macos: &'static str, other: &'static str) -> &'static str {
    if cfg!(target_os = "macos") {
        macos
    } else {
        other
    }
}

fn close_active_window_default_shortcut() -> &'static str {
    default_shortcut("cmd-w", "ctrl-shift-w")
}

const LOG_FILE_NAME: &str = "onetcli.log";

pub(crate) fn configured_log_file_path(value: &str) -> anyhow::Result<PathBuf> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        Ok(default_log_file_path()?)
    } else {
        let path = PathBuf::from(trimmed);
        // 旧配置允许填写完整文件路径；无扩展名的新值则按日志目录处理。
        let is_directory = path.is_dir()
            || trimmed.ends_with('/')
            || trimmed.ends_with('\\')
            || (!path.is_file() && path.extension().is_none());

        if is_directory {
            Ok(path.join(LOG_FILE_NAME))
        } else {
            Ok(path)
        }
    }
}

fn default_log_file_path() -> anyhow::Result<PathBuf> {
    Ok(get_config_dir()?.join("logs").join(LOG_FILE_NAME))
}

pub(crate) fn log_file_appender(path: &Path) -> std::io::Result<std::fs::File> {
    if let Some(parent) = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        std::fs::create_dir_all(parent)?;
    }

    let mut options = std::fs::OpenOptions::new();
    options.create(true).append(true);
    #[cfg(unix)]
    options.mode(0o600);
    options.open(path)
}

/// 内置 Navop AI 模型列表的定期刷新间隔（已登录时）。
const ONETCLI_MODEL_REFRESH_INTERVAL: Duration = Duration::from_secs(15 * 60);
/// 未登录时检查登录态的轮询间隔。
const ONETCLI_MODEL_REFRESH_LOGIN_POLL: Duration = Duration::from_secs(30);
/// 应用启动后首次拉取前的延迟，等待会话恢复与云客户端就绪。
const ONETCLI_MODEL_REFRESH_STARTUP_DELAY: Duration = Duration::from_secs(3);

/// 启动后按固定间隔拉取内置 Navop AI 云端模型列表并持久化到本地。
///
/// 模型列表依赖云账号：未登录时仅以较短间隔轮询登录态、不做拉取；
/// 一旦登录立即拉取一次，之后按固定间隔定期刷新，并通知 AI 面板更新模型选项。
fn spawn_onetcli_model_refresh(cx: &mut App) {
    let storage = cx.global::<GlobalStorageState>().storage.clone();
    let provider_state = cx.global::<GlobalProviderState>().clone();

    cx.spawn(async move |cx: &mut AsyncApp| {
        // 先等待云客户端 / 会话恢复就绪
        cx.background_executor()
            .timer(ONETCLI_MODEL_REFRESH_STARTUP_DELAY)
            .await;

        loop {
            let logged_in = cx.update(|cx| GlobalCurrentUser::get_user(cx).is_some());
            let interval = if logged_in {
                if let Some(repo) = storage.get::<ProviderRepository>() {
                    match refresh_onetcli_models(&repo, &provider_state).await {
                        Ok(Some(_)) => {
                            cx.update(|cx| emit_provider_config_changed(cx));
                        }
                        Ok(None) => {}
                        Err(e) => {
                            tracing::warn!("内置 Navop AI 模型列表刷新失败: {e}");
                        }
                    }
                }
                ONETCLI_MODEL_REFRESH_INTERVAL
            } else {
                ONETCLI_MODEL_REFRESH_LOGIN_POLL
            };
            cx.background_executor().timer(interval).await;
        }
    })
    .detach();
}

pub fn init(cx: &mut App) -> anyhow::Result<()> {
    gpui_component::init(cx);
    one_core::themes::load_bundled(cx);
    one_core::themes::load_imported(cx);
    setting_tab::init_settings(cx);
    one_core::init(cx)?;
    init_ssh_session_service(cx);
    ai_chat_view::init(cx);
    crate::public_mcp_approval::init(cx);
    crate::ai_chat_acp::init(cx);
    one_ui::init_table_display_settings(
        cx,
        one_ui::TableDisplaySettings::new(AppSettings::global(cx).table_row_height),
    );
    let table_keybindings = table_keybindings(cx);
    one_ui::init(cx, table_keybindings);
    db_view::search_shortcut::init(cx);
    db_view::sql_editor_view::init(cx);
    crate::auth::init(cx);
    crate::license::init(cx);
    {
        let auth_service = crate::auth::get_auth_service(cx);
        let global_provider_state = cx.global::<GlobalProviderState>().clone();
        global_provider_state.set_cloud_client(auth_service.cloud_client());
        global_provider_state
            .set_proxy_settings(&AppSettings::global(cx).global_proxy)
            .expect("LLM 代理初始化失败");
    }
    spawn_onetcli_model_refresh(cx);
    db::init_cache(cx);
    // 启动后台磁盘缓存清理任务
    if let Some(cache) = cx.try_global::<db::GlobalNodeCache>() {
        cache.start_cleanup_task(cx);
    }
    terminal_view::init(cx);
    redis_view::init(cx);
    crate::personal_sync_runtime::init(cx);
    mongodb_view::init(cx);
    mqtt_view::init(cx);
    #[cfg(not(all(feature = "builtin-redis", feature = "builtin-mongodb")))]
    init_native_data_driver_factories(cx);
    crate::public_mcp_runtime::init(cx);
    remote_desktop_view::init(cx);
    crate::home_tab::init(cx);
    cx.bind_keys(init_keybindings(cx));
    init_action_handlers(cx);

    let registry = TabContentRegistry::new();
    cx.set_global(registry);

    let storage_state = cx.global::<GlobalStorageState>();
    let conn_repo = storage_state.storage.get::<ConnectionRepository>();
    let db_state = GlobalDbState::with_connection_repository(conn_repo);
    db_state.start_cleanup_task(cx);
    cx.set_global(db_state);
    db_view::init_ask_ai_notifier(cx);
    cx.activate(true);
    Ok(())
}

#[cfg(not(all(feature = "builtin-redis", feature = "builtin-mongodb")))]
fn init_native_data_driver_factories(cx: &mut App) {
    let Some(root) = extension_runtime::extension::extensions_root() else {
        return;
    };
    let driver_root = root.join("database_drivers");
    #[cfg(not(feature = "builtin-redis"))]
    redis_view::init_with_factory(
        cx,
        redis_runtime::RedisConnectionFactory::from_installed_root(driver_root.clone()),
    );
    #[cfg(not(feature = "builtin-mongodb"))]
    mongodb_view::init_with_factory(
        cx,
        mongodb_runtime::MongoConnectionFactory::from_installed_root(driver_root),
    );
}

pub fn refresh_keybindings(cx: &mut App) {
    cx.bind_keys(refreshable_keybindings(cx));
    crate::home_tab::refresh_keybindings(cx);
    db_view::search_shortcut::refresh_keybindings(cx);
    db_view::sql_editor_view::refresh_keybindings(cx);
    terminal_view::refresh_keybindings(cx);
    redis_view::refresh_keybindings(cx);
    remote_desktop_view::refresh_keybindings(cx);
    let table_keybindings = table_keybindings(cx);
    one_ui::refresh_keybindings(cx, table_keybindings);
    remote_file_editor::refresh_keybindings(cx);
    notes::refresh_keybindings(cx);
}

fn table_keybindings(cx: &App) -> one_ui::TableKeybindings {
    use one_core::keybindings::{action_id, shortcuts_for};

    one_ui::TableKeybindings::new(
        shortcuts_for(cx, action_id::TABLE_CANCEL, &["escape"]),
        shortcuts_for(
            cx,
            action_id::TABLE_COPY,
            &[table_shortcut("cmd-c", "ctrl-c")],
        ),
        shortcuts_for(
            cx,
            action_id::TABLE_PASTE,
            &[table_shortcut("cmd-v", "ctrl-v")],
        ),
        shortcuts_for(
            cx,
            action_id::TABLE_SELECT_ALL,
            &[table_shortcut("cmd-a", "ctrl-a")],
        ),
    )
}

fn table_shortcut(macos: &'static str, other: &'static str) -> &'static str {
    if cfg!(target_os = "macos") {
        macos
    } else {
        other
    }
}

fn init_keybindings(cx: &App) -> Vec<KeyBinding> {
    let mut keybindings = vec![];
    keybindings.extend(
        shortcuts_for(cx, action_id::WINDOW_TOGGLE_ZOOM, &["shift-escape"])
            .into_iter()
            .map(|key| KeyBinding::new(&key, ToggleZoom, None)),
    );
    keybindings.extend(
        shortcuts_for(
            cx,
            action_id::WINDOW_CLOSE_ACTIVE_WINDOW,
            &[close_active_window_default_shortcut()],
        )
        .into_iter()
        .map(|key| KeyBinding::new(&key, CloseActiveWindow, None)),
    );
    keybindings.extend(
        shortcuts_for(
            cx,
            action_id::WINDOW_TOGGLE_CONNECTION_SIDEBAR,
            &[default_shortcut("cmd-b", "ctrl-b")],
        )
        .into_iter()
        .map(|key| KeyBinding::new(&key, ToggleConnectionSidebar, None)),
    );
    keybindings.extend(vec![
        #[cfg(target_os = "macos")]
        KeyBinding::new("cmd-1", ActivateTab1, None),
        #[cfg(target_os = "macos")]
        KeyBinding::new("cmd-2", ActivateTab2, None),
        #[cfg(target_os = "macos")]
        KeyBinding::new("cmd-3", ActivateTab3, None),
        #[cfg(target_os = "macos")]
        KeyBinding::new("cmd-4", ActivateTab4, None),
        #[cfg(target_os = "macos")]
        KeyBinding::new("cmd-5", ActivateTab5, None),
        #[cfg(target_os = "macos")]
        KeyBinding::new("cmd-6", ActivateTab6, None),
        #[cfg(target_os = "macos")]
        KeyBinding::new("cmd-7", ActivateTab7, None),
        #[cfg(target_os = "macos")]
        KeyBinding::new("cmd-8", ActivateTab8, None),
        #[cfg(target_os = "macos")]
        KeyBinding::new("cmd-9", ActivateTab9, None),
        #[cfg(not(target_os = "macos"))]
        KeyBinding::new("alt-1", ActivateTab1, None),
        #[cfg(not(target_os = "macos"))]
        KeyBinding::new("alt-2", ActivateTab2, None),
        #[cfg(not(target_os = "macos"))]
        KeyBinding::new("alt-3", ActivateTab3, None),
        #[cfg(not(target_os = "macos"))]
        KeyBinding::new("alt-4", ActivateTab4, None),
        #[cfg(not(target_os = "macos"))]
        KeyBinding::new("alt-5", ActivateTab5, None),
        #[cfg(not(target_os = "macos"))]
        KeyBinding::new("alt-6", ActivateTab6, None),
        #[cfg(not(target_os = "macos"))]
        KeyBinding::new("alt-7", ActivateTab7, None),
        #[cfg(not(target_os = "macos"))]
        KeyBinding::new("alt-8", ActivateTab8, None),
        #[cfg(not(target_os = "macos"))]
        KeyBinding::new("alt-9", ActivateTab9, None),
    ]);
    keybindings.extend(
        shortcuts_for(
            cx,
            action_id::WINDOW_TOGGLE_FULLSCREEN,
            &[default_shortcut("ctrl-cmd-f", "alt-enter")],
        )
        .into_iter()
        .map(|key| KeyBinding::new(&key, ToggleFullscreen, None)),
    );
    keybindings.extend(
        shortcuts_for(
            cx,
            action_id::WINDOW_TOGGLE_ALWAYS_ON_TOP,
            &[default_shortcut("ctrl-cmd-t", "ctrl-alt-t")],
        )
        .into_iter()
        .map(|key| KeyBinding::new(&key, ToggleAlwaysOnTop, None)),
    );
    keybindings.extend(
        shortcuts_for(
            cx,
            action_id::APP_DUPLICATE_TAB,
            &[default_shortcut("cmd-shift-t", "alt-shift-t")],
        )
        .into_iter()
        .map(|key| KeyBinding::new(&key, DuplicateTab, None)),
    );
    keybindings.extend(
        shortcuts_for(
            cx,
            action_id::APP_OPEN_TAB_SWITCHER,
            &[default_shortcut("cmd-j", "ctrl-j")],
        )
        .into_iter()
        .map(|key| KeyBinding::new(&key, OpenTabSwitcher, None)),
    );
    keybindings.extend(
        shortcuts_for(cx, action_id::APP_SWITCH_NEXT_TAB, &["ctrl-tab"])
            .into_iter()
            .map(|key| KeyBinding::new(&key, SwitchNextTab, None)),
    );
    keybindings.extend(
        shortcuts_for(cx, action_id::APP_SWITCH_PREVIOUS_TAB, &["ctrl-shift-tab"])
            .into_iter()
            .map(|key| KeyBinding::new(&key, SwitchPreviousTab, None)),
    );
    keybindings.extend(
        shortcuts_for(
            cx,
            action_id::APP_QUIT,
            &[default_shortcut("cmd-q", "alt-f4")],
        )
        .into_iter()
        .map(|key| KeyBinding::new(&key, QuitApp, None)),
    );
    keybindings.push(KeyBinding::new(
        default_shortcut("cmd-v", "ctrl-v"),
        SftpPasteUpload,
        Some(SFTP_VIEW_CONTEXT),
    ));

    keybindings
}

fn refreshable_keybindings(cx: &App) -> Vec<KeyBinding> {
    let mut keybindings = Vec::new();
    keybindings.extend(rebind_keybindings(
        cx,
        action_id::WINDOW_TOGGLE_ZOOM,
        &["shift-escape"],
        None,
        ToggleZoom,
    ));
    keybindings.extend(rebind_keybindings(
        cx,
        action_id::WINDOW_CLOSE_ACTIVE_WINDOW,
        &[close_active_window_default_shortcut()],
        None,
        CloseActiveWindow,
    ));
    keybindings.extend(rebind_keybindings(
        cx,
        action_id::WINDOW_TOGGLE_CONNECTION_SIDEBAR,
        &[default_shortcut("cmd-b", "ctrl-b")],
        None,
        ToggleConnectionSidebar,
    ));
    keybindings.push(KeyBinding::new(
        default_shortcut("cmd-v", "ctrl-v"),
        SftpPasteUpload,
        Some(SFTP_VIEW_CONTEXT),
    ));
    keybindings.extend(rebind_keybindings(
        cx,
        action_id::WINDOW_TOGGLE_FULLSCREEN,
        &[default_shortcut("ctrl-cmd-f", "alt-enter")],
        None,
        ToggleFullscreen,
    ));
    keybindings.extend(rebind_keybindings(
        cx,
        action_id::WINDOW_TOGGLE_ALWAYS_ON_TOP,
        &[default_shortcut("ctrl-cmd-t", "ctrl-alt-t")],
        None,
        ToggleAlwaysOnTop,
    ));
    keybindings.extend(rebind_keybindings(
        cx,
        action_id::APP_DUPLICATE_TAB,
        &[default_shortcut("cmd-shift-t", "alt-shift-t")],
        None,
        DuplicateTab,
    ));
    keybindings.extend(rebind_keybindings(
        cx,
        action_id::APP_OPEN_TAB_SWITCHER,
        &[default_shortcut("cmd-j", "ctrl-j")],
        None,
        OpenTabSwitcher,
    ));
    keybindings.extend(rebind_keybindings(
        cx,
        action_id::APP_SWITCH_NEXT_TAB,
        &["ctrl-tab"],
        None,
        SwitchNextTab,
    ));
    keybindings.extend(rebind_keybindings(
        cx,
        action_id::APP_SWITCH_PREVIOUS_TAB,
        &["ctrl-shift-tab"],
        None,
        SwitchPreviousTab,
    ));
    keybindings.extend(rebind_keybindings(
        cx,
        action_id::APP_QUIT,
        &[default_shortcut("cmd-q", "alt-f4")],
        None,
        QuitApp,
    ));
    keybindings
}

fn init_action_handlers(cx: &mut App) {
    cx.on_action(|_: &ActivateTab1, cx| activate_tab_by_number(1, cx));
    cx.on_action(|_: &ActivateTab2, cx| activate_tab_by_number(2, cx));
    cx.on_action(|_: &ActivateTab3, cx| activate_tab_by_number(3, cx));
    cx.on_action(|_: &ActivateTab4, cx| activate_tab_by_number(4, cx));
    cx.on_action(|_: &ActivateTab5, cx| activate_tab_by_number(5, cx));
    cx.on_action(|_: &ActivateTab6, cx| activate_tab_by_number(6, cx));
    cx.on_action(|_: &ActivateTab7, cx| activate_tab_by_number(7, cx));
    cx.on_action(|_: &ActivateTab8, cx| activate_tab_by_number(8, cx));
    cx.on_action(|_: &ActivateTab9, cx| activate_tab_by_number(9, cx));
    cx.on_action(|_: &ToggleFullscreen, cx| toggle_fullscreen(cx));
    cx.on_action(|_: &ToggleAlwaysOnTop, cx| toggle_always_on_top(cx));
    cx.on_action(|_: &DuplicateTab, cx| duplicate_tab(cx));
    cx.on_action(|_: &OpenTabSwitcher, cx| open_tab_switcher(cx));
    cx.on_action(|_: &SwitchNextTab, cx| switch_tab(TabCycleDirection::Next, cx));
    cx.on_action(|_: &SwitchPreviousTab, cx| switch_tab(TabCycleDirection::Previous, cx));
    cx.on_action(|_: &CloseActiveWindow, cx| close_active_window(cx));
    cx.on_action(|_: &QuitApp, cx| quit_app(cx));
    cx.on_action(|_: &OpenConnectionQuickOpen, cx| {
        let Some(active_window) = cx.active_window() else {
            return;
        };
        let Some(home) = cx.try_global::<GlobalHomePage>() else {
            return;
        };
        let home_page = home.home_page.clone();
        cx.defer(move |cx| {
            _ = active_window.update(cx, |_, window, cx| {
                if home_page.read(cx).startup_master_key_lock_active(cx) {
                    return;
                }
                if window.has_active_dialog(cx) {
                    window.close_all_dialogs(cx);
                }
                home_page.update(cx, |hp, cx| {
                    hp.show_connection_quick_open(window, cx);
                });
            });
        });
    });
    cx.on_action(|_: &NewConnectionShortcut, cx| {
        let Some(active_window) = cx.active_window() else {
            return;
        };
        let Some(home) = cx.try_global::<GlobalHomePage>() else {
            return;
        };
        let home_page = home.home_page.clone();
        cx.defer(move |cx| {
            _ = active_window.update(cx, |_, window, cx| {
                if home_page.read(cx).startup_master_key_lock_active(cx) {
                    return;
                }
                if window.has_active_dialog(cx) {
                    window.close_all_dialogs(cx);
                }
                home_page.update(cx, |hp, cx| {
                    hp.show_new_connection_dialog(window, cx);
                });
            });
        });
    });
    cx.on_action(|_: &OpenLocalTerminalShortcut, cx| {
        let Some(active_window) = cx.active_window() else {
            return;
        };
        let Some(home) = cx.try_global::<GlobalHomePage>() else {
            return;
        };
        let home_page = home.home_page.clone();
        cx.defer(move |cx| {
            _ = active_window.update(cx, |_, window, cx| {
                if home_page.read(cx).startup_master_key_lock_active(cx) {
                    return;
                }
                if window.has_active_dialog(cx) {
                    window.close_all_dialogs(cx);
                }
                home_page.update(cx, |hp, cx| {
                    hp.add_terminal_tab(window, cx);
                });
            });
        });
    });
}

fn close_active_window(cx: &mut App) {
    let Some(active_window) = crate::app_init::resolve_active_non_main_window(cx) else {
        return;
    };

    one_core::window_close::request_close_window(active_window, cx);
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum MainContent {
    Home,
    Tabs,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum MainContentPresentation {
    HomeWithTabBar,
    Tabs,
}

fn main_content_presentation(main_content: MainContent) -> MainContentPresentation {
    match main_content {
        MainContent::Home => MainContentPresentation::HomeWithTabBar,
        MainContent::Tabs => MainContentPresentation::Tabs,
    }
}

pub struct OnetCliApp {
    tab_container: Entity<TabContainer>,
    home_page: Entity<HomePage>,
    connection_sidebar: Entity<PersistentConnectionSidebar>,
    main_content: MainContent,
    home_page_style: HomePageStyle,
    quit_state: QuitRequestState,
    main_window_size_save_task: Option<Task<()>>,
    _appearance_subscription: gpui::Subscription,
}

impl OnetCliApp {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let app_entity = cx.entity();
        cx.set_global(GlobalOnetCliApp {
            app: app_entity.clone(),
        });
        let app = app_entity.downgrade();
        window.on_window_should_close(cx, move |window, cx| {
            let _ = app.update(cx, |app, cx| {
                app.request_quit(window, cx);
            });
            false
        });
        cx.observe_window_bounds(window, |app, window, cx| {
            app.main_window_size_save_task = Some(cx.spawn_in(window, async move |app, cx| {
                cx.background_executor()
                    .timer(Duration::from_millis(150))
                    .await;
                app.update_in(cx, |app, window, cx| {
                    app.save_main_window_state(window, cx);
                    app.main_window_size_save_task.take();
                })
                .ok();
            }));
        })
        .detach();

        let settings = AppSettings::current(cx);
        let home_page_style = settings.home_page_style;
        let show_navigation_sidebar_toggle = home_page_style.uses_persistent_sidebar();
        let layout = initial_content_layout(home_page_style, settings.startup_default_page);
        let initial_home_active = layout.main_content == MainContent::Home;
        // 侧边栏展开状态完全跟随用户上次保存的选择，进入主页不强制展开。
        let connection_sidebar_expanded = settings.connection_sidebar_expanded;
        let tab_container = cx.new(|cx| {
            let mut container = TabContainer::new(window, cx)
                .with_tab_bar_when_empty(true)
                .with_add_tab_button(add_tab_button_handler())
                .with_tab_quick_open(tab_quick_open_resolver())
                .with_settings_button(settings_button_handler());

            if show_navigation_sidebar_toggle {
                container = container
                    .with_navigation_sidebar_toggle(connection_sidebar_expanded)
                    .with_home_button(initial_home_active, home_button_handler());
            }

            #[cfg(target_os = "macos")]
            {
                container = container
                    .with_macos_titlebar_inset(true)
                    .with_left_padding(if home_page_style.uses_persistent_sidebar() {
                        px(0.0)
                    } else {
                        px(80.0)
                    })
                    .with_top_padding(px(4.0));
            }

            #[cfg(not(target_os = "macos"))]
            {
                // 窗口置顶按钮注入：点击时切换置顶并刷新按钮视觉状态
                let on_toggle: Arc<dyn Fn(&mut Window, &mut App) + Send + Sync> =
                    Arc::new(|_window: &mut Window, cx: &mut App| {
                        toggle_always_on_top(cx);
                        if let Some(tab_container) = cx
                            .try_global::<GlobalTabContainer>()
                            .map(|global| global.tab_container.clone())
                        {
                            tab_container.update(cx, |_, cx| cx.notify());
                        }
                    });
                let is_active: Arc<dyn Fn() -> bool + Send + Sync> =
                    Arc::new(|| ALWAYS_ON_TOP.load(Ordering::Relaxed));
                let on_close: Arc<dyn Fn(&mut Window, &mut App) + Send + Sync> =
                    Arc::new(request_window_quit);
                container = container
                    .with_window_controls(true)
                    .with_window_close_action(on_close)
                    .with_always_on_top_control(on_toggle, is_active);
            }

            container
        });

        cx.set_global(GlobalTabContainer {
            tab_container: tab_container.clone(),
        });
        let tab_container_clone = tab_container.clone();
        let home_page = cx.new(|cx| HomePage::new(tab_container_clone, window, cx));
        cx.set_global(GlobalHomePage {
            home_page: home_page.clone(),
        });
        let home_for_startup_prompt = home_page.clone();
        window.defer(cx, move |window, cx| {
            home_for_startup_prompt.update(cx, |home, cx| {
                home.show_pending_master_key_prompt(window, cx);
            });
        });
        let connection_sidebar = cx.new(|cx| {
            PersistentConnectionSidebar::new(
                home_page.clone(),
                connection_sidebar_expanded,
                window,
                cx,
            )
        });
        cx.subscribe(
            &connection_sidebar,
            |this, _, event: &PersistentConnectionSidebarEvent, cx| match event {
                PersistentConnectionSidebarEvent::TreeVisibilityChanged { expanded } => {
                    this.persist_connection_sidebar_expanded(*expanded, cx)
                }
            },
        )
        .detach();

        let main_content = layout.main_content;
        home_page.update(cx, |home, cx| {
            home.set_home_active(main_content == MainContent::Home, cx);
            if show_navigation_sidebar_toggle {
                home.set_persistent_sidebar_expanded(connection_sidebar_expanded, cx);
            }
        });
        tab_container.update(cx, |tc, cx| {
            tc.set_tab_content_visible(main_content == MainContent::Tabs, cx);
            tc.set_active_presentation_obscured_by_main_content(
                main_content != MainContent::Tabs,
                cx,
            );
            if layout.pin_home {
                let home_tab = TabItem::new(layout.home_tab_id, "app", home_page.clone());
                tc.insert_pinned_tab_at(0, home_tab, cx);
            }
            if layout.pin_workbench {
                let connections = cx
                    .global::<GlobalStorageState>()
                    .storage
                    .get::<ConnectionRepository>()
                    .and_then(|repo| repo.list().ok())
                    .unwrap_or_default();
                let (scope, catalog, mentions) =
                    ai_chat_view::build_workbench_resource_state(&connections);
                let workbench = cx.new(|cx| {
                    ai_chat_view::DefaultAgentChatPanel::new_workbench_with_scope_and_catalog(
                        scope, catalog, mentions, window, cx,
                    )
                });
                let workbench_tab = TabItem::new(layout.workbench_tab_id, "app", workbench);
                tc.add_pinned_tab(workbench_tab, cx);
            }
        });

        cx.subscribe(&tab_container, |this, _, event: &TabContainerEvent, cx| {
            let app = cx.entity();
            match event {
                TabContainerEvent::NavigationSidebarToggled { expanded } => {
                    if !this.home_page_style.uses_persistent_sidebar() {
                        return;
                    }
                    let expanded = *expanded;
                    cx.defer(move |cx| {
                        app.update(cx, |app, cx| {
                            app.set_connection_sidebar_expanded(expanded, cx);
                        });
                    });
                }
                TabContainerEvent::TabActivated { .. } => {
                    cx.defer(move |cx| {
                        app.update(cx, |app, cx| {
                            app.set_main_content(MainContent::Tabs, cx);
                            app.show_home_if_tab_container_is_empty(cx);
                            app.sync_connection_sidebar_theme(cx);
                        });
                    });
                }
                TabContainerEvent::LayoutChanged | TabContainerEvent::TabClosed { .. } => {
                    cx.defer(move |cx| {
                        app.update(cx, |app, cx| {
                            app.show_home_if_tab_container_is_empty(cx);
                            app.sync_connection_sidebar_theme(cx);
                            cx.notify();
                        });
                    });
                }
            }
        })
        .detach();
        if let Some(active_pinned_index) = layout.active_pinned_index {
            let tabs = tab_container.clone();
            window.defer(cx, move |window, cx| {
                tabs.update(cx, |tabs, cx| {
                    tabs.activate_pinned_tab_at(active_pinned_index, window, cx);
                });
            });
        }
        let appearance_subscription = window.observe_window_appearance(|_, cx| {
            let settings = AppSettings::current(cx);
            if settings.auto_switch_theme || settings.theme_mode == "system" {
                themes::apply_appearance(&settings, cx);
            }
        });

        Self {
            tab_container,
            home_page: home_page.clone(),
            connection_sidebar,
            main_content,
            home_page_style,
            quit_state: QuitRequestState::default(),
            main_window_size_save_task: None,
            _appearance_subscription: appearance_subscription,
        }
    }

    pub(crate) fn show_home(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.home_page_style == HomePageStyle::Legacy {
            let tabs = self.tab_container.clone();
            window.defer(cx, move |window, cx| {
                tabs.update(cx, |tabs, cx| {
                    tabs.activate_pinned_tab_by_id("home", window, cx);
                });
            });
        } else {
            self.set_main_content(MainContent::Home, cx);
            self.home_page.read(cx).focus_handle(cx).focus(window, cx);
            self.sync_connection_sidebar_theme(cx);
        }
    }

    fn set_main_content(&mut self, main_content: MainContent, cx: &mut Context<Self>) {
        self.tab_container.update(cx, |tabs, cx| {
            tabs.set_tab_content_visible(main_content == MainContent::Tabs, cx);
            // The tab-bar Home button replaces the old rail Home entry, so
            // its selected state must track which main content is active.
            tabs.set_home_button_active(main_content == MainContent::Home, cx);
            // The modern Home page is not a pinned tab, so the active tab stays
            // present in the TabContainer while Home renders. Mark the active
            // tab content as obscured so Windows-native RDP overlays are
            // deactivated and stop intercepting mouse/keyboard input on Home.
            tabs.set_active_presentation_obscured_by_main_content(
                main_content != MainContent::Tabs,
                cx,
            );
        });
        if self.main_content == main_content {
            return;
        }
        self.main_content = main_content;
        self.home_page.update(cx, |home, cx| {
            home.set_home_active(main_content == MainContent::Home, cx)
        });
        cx.notify();
    }

    fn show_home_if_tab_container_is_empty(&mut self, cx: &mut Context<Self>) {
        if self.home_page_style != HomePageStyle::Modern {
            return;
        }
        if self.main_content != MainContent::Tabs {
            return;
        }
        let tab_container_is_empty = {
            let tabs = self.tab_container.read(cx);
            tabs.tabs().is_empty() && !tabs.is_pinned_tab_active()
        };
        if tab_container_is_empty {
            self.set_main_content(MainContent::Home, cx);
        }
    }

    fn render_main_content(&self, cx: &App) -> AnyElement {
        match main_content_presentation(self.main_content) {
            MainContentPresentation::HomeWithTabBar => div()
                .flex()
                .flex_col()
                .size_full()
                .min_w_0()
                .min_h_0()
                .overflow_hidden()
                .child(
                    div()
                        .id("home-tab-bar-slot")
                        .relative()
                        .w_full()
                        .h(cx.theme().geometry.layout.tab_bar)
                        .flex_shrink_0()
                        .overflow_hidden()
                        .child(self.tab_container.clone()),
                )
                .child(
                    div()
                        .id("home-page-content")
                        .flex_1()
                        .min_w_0()
                        .min_h_0()
                        .overflow_hidden()
                        .child(self.home_page.clone()),
                )
                .into_any_element(),
            MainContentPresentation::Tabs => self.tab_container.clone().into_any_element(),
        }
    }

    pub(crate) fn set_home_page_style(&mut self, style: HomePageStyle, cx: &mut Context<Self>) {
        let previous_style = self.home_page_style;
        self.home_page_style = style;
        AppSettings::update_and_save(cx, |settings| settings.home_page_style = style);

        if let Some(home) = cx
            .try_global::<GlobalHomePage>()
            .map(|global| global.home_page.clone())
        {
            home.update(cx, |home, cx| {
                home.set_home_page_style(style, cx);
                home.set_persistent_sidebar_expanded(
                    style.uses_persistent_sidebar()
                        && AppSettings::current(cx).connection_sidebar_expanded,
                    cx,
                );
            });
        }

        let expanded = self.connection_sidebar.read(cx).is_expanded();
        let home_active = self.main_content == MainContent::Home;
        self.tab_container.update(cx, |tabs, cx| {
            tabs.set_navigation_sidebar_toggle(
                style.uses_persistent_sidebar().then_some(expanded),
                cx,
            );
            tabs.set_home_button(
                style
                    .uses_persistent_sidebar()
                    .then(|| (home_active, home_button_handler())),
                cx,
            );
            #[cfg(target_os = "macos")]
            tabs.set_left_padding(
                if style.uses_persistent_sidebar() {
                    px(0.0)
                } else {
                    px(80.0)
                },
                cx,
            );
        });

        if previous_style != style {
            let app = cx.entity();
            if let Some(active_window) = cx.active_window() {
                cx.defer(move |cx| {
                    let _ = active_window.update(cx, |_, window, cx| {
                        app.update(cx, |app, cx| {
                            app.sync_home_tab_layout(previous_style, style, window, cx);
                        })
                    });
                });
            }
        }
        cx.notify();
    }

    fn sync_home_tab_layout(
        &mut self,
        previous_style: HomePageStyle,
        style: HomePageStyle,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match (previous_style, style) {
            (HomePageStyle::Modern, HomePageStyle::Legacy) => {
                let home_tab_id = "home";
                self.tab_container.update(cx, |tabs, cx| {
                    if !tabs.has_pinned_tab_by_id(home_tab_id) {
                        let home_tab = TabItem::new(home_tab_id, "app", self.home_page.clone());
                        tabs.insert_pinned_tab_at(0, home_tab, cx);
                    }
                    tabs.activate_pinned_tab_by_id(home_tab_id, window, cx);
                });
                self.set_main_content(MainContent::Tabs, cx);
            }
            (HomePageStyle::Legacy, HomePageStyle::Modern) => {
                let home_removed = self.tab_container.update(cx, |tabs, cx| {
                    tabs.remove_pinned_tab_by_id("home", window, cx)
                });
                if home_removed {
                    let has_active_tab = {
                        let tabs = self.tab_container.read(cx);
                        tabs.is_pinned_tab_active() || !tabs.tabs().is_empty()
                    };
                    self.set_main_content(
                        if has_active_tab {
                            MainContent::Tabs
                        } else {
                            MainContent::Home
                        },
                        cx,
                    );
                }
            }
            _ => {}
        }
    }

    pub(crate) fn set_connection_sidebar_expanded(
        &mut self,
        expanded: bool,
        cx: &mut Context<Self>,
    ) {
        self.connection_sidebar
            .update(cx, |sidebar, cx| sidebar.set_tree_expanded(expanded, cx));
        self.persist_connection_sidebar_expanded(expanded, cx);
    }

    fn persist_connection_sidebar_expanded(&mut self, expanded: bool, cx: &mut Context<Self>) {
        AppSettings::update_and_save(cx, |settings| {
            settings.connection_sidebar_expanded = expanded;
        });
        if let Some(home) = cx
            .try_global::<GlobalHomePage>()
            .map(|global| global.home_page.clone())
        {
            home.update(cx, |home, cx| {
                home.set_persistent_sidebar_expanded(expanded, cx)
            });
        }
        self.tab_container.update(cx, |tabs, cx| {
            tabs.set_navigation_sidebar_expanded(expanded, cx)
        });
        cx.notify();
    }

    /// Home start-center tiles live on the HomePage entity, which cannot reach
    /// the sidebar directly, so auto-hide collapsing is routed through here.
    pub(crate) fn collapse_connection_sidebar_if_auto_hide(&mut self, cx: &mut Context<Self>) {
        if !self.home_page_style.uses_persistent_sidebar() {
            return;
        }
        self.connection_sidebar
            .update(cx, |sidebar, cx| sidebar.collapse_if_auto_hide(cx));
    }

    fn sync_connection_sidebar_theme(&mut self, cx: &mut Context<Self>) {
        let terminal_active = {
            let tabs = self.tab_container.read(cx);
            if self.main_content == MainContent::Home || tabs.is_pinned_tab_active() {
                false
            } else {
                tabs.active_tab()
                    .filter(|tab| tab.content().content_key(cx) == "Terminal")
                    .and_then(|tab| {
                        tab.content()
                            .view()
                            .downcast::<terminal_view::TerminalWorkspace>()
                            .ok()
                    })
                    .is_some()
            }
        };
        let colors = terminal_active
            .then(|| terminal_view::TerminalColors::from_application_theme(cx.theme()));
        self.connection_sidebar
            .update(cx, |sidebar, cx| sidebar.set_terminal_colors(colors, cx));
    }

    fn save_main_window_state(&self, window: &Window, cx: &mut App) {
        let bounds = window.window_bounds().get_bounds();
        let display_uuid = window
            .display(cx)
            .and_then(|display| display.uuid().ok())
            .map(|uuid| uuid.to_string());
        let Some(state) = MainWindowState::new(
            f32::from(bounds.origin.x),
            f32::from(bounds.origin.y),
            f32::from(bounds.size.width),
            f32::from(bounds.size.height),
            display_uuid,
        ) else {
            return;
        };
        if AppSettings::current(cx).main_window_state.as_ref() == Some(&state) {
            return;
        }
        AppSettings::update_and_save(cx, |settings| {
            settings.main_window_state = Some(state);
        });
    }

    fn request_quit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.save_main_window_state(window, cx);
        if self.tab_container.read(cx).is_empty() {
            self.confirm_quit(window, cx);
            return;
        }
        if self.quit_state.request() == QuitRequestDecision::OpenPrompt {
            self.show_quit_confirmation(window, cx);
        }
    }

    fn show_quit_confirmation(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let app_for_ok = cx.entity().downgrade();
        let app_for_cancel = cx.entity().downgrade();
        let app_for_close = cx.entity().downgrade();
        window.open_dialog(cx, move |dialog, _window, _cx| {
            let app_for_ok = app_for_ok.clone();
            let app_for_cancel = app_for_cancel.clone();
            let app_for_close = app_for_close.clone();
            dialog
                .title(t!("Quit.confirm_title").to_string())
                .child(t!("Quit.confirm_message").to_string())
                .confirm()
                .button_props(
                    DialogButtonProps::default()
                        .ok_text(t!("Quit.confirm_action").to_string())
                        .cancel_text(t!("Common.cancel").to_string()),
                )
                .on_ok(move |_, window, cx: &mut App| {
                    let _ = app_for_ok.update(cx, |app, cx| {
                        app.confirm_quit(window, cx);
                    });
                    true
                })
                .on_cancel(move |_, _, cx| {
                    let _ = app_for_cancel.update(cx, |app, _cx| {
                        app.quit_state.cancel_prompt();
                    });
                    true
                })
                .on_close(move |_, _, cx| {
                    let _ = app_for_close.update(cx, |app, _cx| {
                        app.quit_state.cancel_prompt();
                    });
                })
        });
    }

    fn confirm_quit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.quit_state.confirm_prompt() {
            return;
        }
        let close_task = self
            .tab_container
            .update(cx, |tabs, cx| tabs.close_all_tabs(window, cx));
        cx.spawn(async move |this, cx| {
            let can_quit = close_task.await;
            let _ = this.update(cx, |app, cx| {
                app.quit_state.finish_close(can_quit);
                if can_quit {
                    shutdown_application_resources_and_quit(cx, "confirmed application quit");
                }
            });
        })
        .detach();
    }
}

#[cfg(test)]
mod tests {
    use super::{
        GlobalSshSessionService, LOG_FILE_NAME, MainContent, MainContentPresentation,
        close_active_window_default_shortcut, configured_log_file_path, default_log_file_path,
        init_ssh_session_service, initial_content_layout, log_file_appender,
        main_content_presentation,
    };
    use one_core::gpui_tokio::Tokio;
    use one_core::settings::{HomePageStyle, StartupDefaultPage};
    use ssh::SshSessionServiceState;
    use std::io::Write;

    #[test]
    fn initial_layout_keeps_legacy_home_pinned_before_the_ai_workbench() {
        let layout = initial_content_layout(HomePageStyle::Legacy, StartupDefaultPage::AiWorkbench);

        assert!(layout.pin_home);
        assert_eq!("home", layout.home_tab_id);
        assert_eq!("ai-workbench", layout.workbench_tab_id);
        assert!(layout.pin_workbench);
        assert_eq!(Some(1), layout.active_pinned_index);
        assert_eq!(MainContent::Tabs, layout.main_content);
    }

    #[test]
    fn initial_layout_keeps_modern_home_outside_the_tab_container() {
        let home_layout = initial_content_layout(HomePageStyle::Modern, StartupDefaultPage::Home);
        let ai_layout =
            initial_content_layout(HomePageStyle::Modern, StartupDefaultPage::AiWorkbench);

        assert!(!home_layout.pin_home);
        assert!(!home_layout.pin_workbench);
        assert_eq!(None, home_layout.active_pinned_index);
        assert_eq!(MainContent::Home, home_layout.main_content);
        assert!(!ai_layout.pin_home);
        assert!(ai_layout.pin_workbench);
        assert_eq!(Some(0), ai_layout.active_pinned_index);
        assert_eq!(MainContent::Tabs, ai_layout.main_content);
    }

    #[test]
    fn legacy_home_is_a_pinned_tab_while_modern_home_remains_standalone() {
        let source = include_str!("onetcli_app.rs");
        let constructor = source
            .split("pub fn new(window:")
            .nth(1)
            .and_then(|source| {
                source
                    .split("\n    pub(crate) fn set_home_page_style")
                    .next()
            })
            .expect("OnetCliApp::new source");

        assert!(constructor.contains("home_page: home_page.clone()"));
        assert!(constructor.contains("main_content"));
        assert!(
            constructor
                .contains("tc.set_tab_content_visible(main_content == MainContent::Tabs, cx)")
        );
        assert!(
            constructor.contains(
                "tc.set_active_presentation_obscured_by_main_content(\n                main_content != MainContent::Tabs,"
            )
        );
        assert!(!constructor.contains("set_base_content"));
        assert!(constructor.contains("let home_tab ="));
        assert!(
            constructor.contains("TabItem::new(layout.home_tab_id, \"app\", home_page.clone())")
        );
        assert!(constructor.contains("tc.insert_pinned_tab_at(0, home_tab, cx)"));
    }

    #[test]
    fn home_content_cannot_inherit_a_background_terminal_sidebar_theme() {
        let source = include_str!("onetcli_app.rs").replace("\r\n", "\n");

        assert!(source.contains(
            "if self.main_content == MainContent::Home || tabs.is_pinned_tab_active() {\n                false"
        ));
    }

    #[test]
    fn legacy_home_implements_tab_content_with_the_bright_home_icon() {
        let app_source = include_str!("onetcli_app.rs");
        let legacy_home_source = include_str!("home_tab/legacy_home.rs");

        assert!(app_source.contains("MainContentPresentation::HomeWithTabBar => div()"));
        assert!(app_source.contains(".id(\"home-tab-bar-slot\")"));
        assert!(app_source.contains(".h(cx.theme().geometry.layout.tab_bar)"));
        assert!(app_source.contains(".id(\"home-page-content\")"));
        assert!(app_source.contains("with_tab_bar_when_empty(true)"));
        assert!(!app_source.contains(".id(\"home-content-overlay\")"));
        assert!(app_source.contains(
            "MainContentPresentation::Tabs => self.tab_container.clone().into_any_element()"
        ));
        assert!(legacy_home_source.contains("impl TabContent for HomePage"));
        assert!(legacy_home_source.contains("impl EventEmitter<TabContentEvent> for HomePage"));
        assert!(legacy_home_source.contains("Some(IconName::Home.color())"));
        assert!(legacy_home_source.contains("fn closeable"));
        assert!(legacy_home_source.contains("false"));
    }

    #[test]
    fn home_always_keeps_the_tab_bar_visible() {
        assert_eq!(
            MainContentPresentation::HomeWithTabBar,
            main_content_presentation(MainContent::Home)
        );
        assert_eq!(
            MainContentPresentation::Tabs,
            main_content_presentation(MainContent::Tabs)
        );
    }

    #[test]
    fn modern_home_does_not_layout_the_active_tab_content() {
        let source = include_str!("onetcli_app.rs").replace("\r\n", "\n");
        let setter = source
            .split("fn set_main_content(")
            .nth(1)
            .and_then(|source| {
                source
                    .split("\n    fn show_home_if_tab_container_is_empty")
                    .next()
            })
            .expect("set_main_content source");

        assert!(
            setter.contains("tabs.set_tab_content_visible(main_content == MainContent::Tabs, cx);")
        );
        assert!(setter.contains("tabs.set_active_presentation_obscured_by_main_content("));
    }

    #[test]
    fn stale_tab_activation_cannot_replace_modern_home_with_an_empty_container() {
        let source = include_str!("onetcli_app.rs").replace("\r\n", "\n");
        let activated_arm = source
            .split("TabContainerEvent::TabActivated { .. } =>")
            .nth(1)
            .and_then(|source| {
                source
                    .split("TabContainerEvent::LayoutChanged | TabContainerEvent::TabClosed")
                    .next()
            })
            .expect("TabActivated event arm");
        let set_tabs = activated_arm
            .find("app.set_main_content(MainContent::Tabs, cx);")
            .expect("TabActivated switches to tabs");
        let empty_guard = activated_arm
            .find("app.show_home_if_tab_container_is_empty(cx);")
            .expect("TabActivated rechecks the empty-container fallback");
        let sync_theme = activated_arm
            .find("app.sync_connection_sidebar_theme(cx);")
            .expect("TabActivated syncs the sidebar theme");
        assert!(set_tabs < empty_guard);
        assert!(empty_guard < sync_theme);

        let fallback = source
            .split("fn show_home_if_tab_container_is_empty")
            .nth(1)
            .and_then(|source| source.split("\n    fn render_main_content").next())
            .expect("empty-container fallback");
        assert!(fallback.contains("HomePageStyle::Modern"));
        assert!(fallback.contains("tabs.tabs().is_empty() && !tabs.is_pinned_tab_active()"));
        assert!(fallback.contains("self.set_main_content(MainContent::Home, cx);"));
    }

    #[test]
    fn startup_master_key_prompt_is_scheduled_even_when_home_is_not_active() {
        let source = include_str!("onetcli_app.rs");
        let constructor = source
            .split("pub fn new(window:")
            .nth(1)
            .and_then(|source| {
                source
                    .split("\n    pub(crate) fn set_home_page_style")
                    .next()
            })
            .expect("OnetCliApp::new source");

        assert!(constructor.contains("home.show_pending_master_key_prompt(window, cx)"));
    }

    #[test]
    fn connection_and_terminal_shortcuts_cannot_replace_the_startup_lock_dialog() {
        let source = include_str!("onetcli_app.rs");
        let handlers = source
            .split("fn init_action_handlers(")
            .nth(1)
            .and_then(|source| source.split("\npub struct OnetCliApp").next())
            .expect("init_action_handlers source");

        assert_eq!(
            3,
            handlers
                .matches("home_page.read(cx).startup_master_key_lock_active(cx)")
                .count()
        );
    }

    #[test]
    fn platform_close_shortcut_closes_only_the_active_auxiliary_window() {
        let source = include_str!("onetcli_app.rs");
        let keybindings = source
            .split("fn init_keybindings(")
            .nth(1)
            .and_then(|source| source.split("\nfn refreshable_keybindings").next())
            .expect("init_keybindings source");
        let refreshable_keybindings = source
            .split("fn refreshable_keybindings(")
            .nth(1)
            .and_then(|source| source.split("\nfn init_action_handlers").next())
            .expect("refreshable_keybindings source");
        let close_handler = source
            .split("fn close_active_window(")
            .nth(1)
            .and_then(|source| source.split("\n}\n\n").next())
            .expect("close_active_window source");
        let close_shortcut = source
            .split("fn close_active_window_default_shortcut()")
            .nth(1)
            .and_then(|source| source.split("\n}\n").next())
            .expect("close shortcut source");

        assert!(keybindings.contains("action_id::WINDOW_CLOSE_ACTIVE_WINDOW"));
        assert!(keybindings.contains("CloseActiveWindow"));
        assert!(keybindings.contains("close_active_window_default_shortcut()"));
        assert!(refreshable_keybindings.contains("close_active_window_default_shortcut()"));
        assert!(close_shortcut.contains(r#"default_shortcut("cmd-w", "ctrl-shift-w")"#));
        assert!(!close_shortcut.contains("ctrl-d"));
        assert_eq!(
            close_active_window_default_shortcut(),
            if cfg!(target_os = "macos") {
                "cmd-w"
            } else {
                "ctrl-shift-w"
            }
        );
        assert!(!keybindings.contains("ClosePanel"));
        assert!(close_handler.contains("resolve_active_non_main_window(cx)"));
        assert!(close_handler.contains("one_core::window_close::request_close_window"));
        assert!(!close_handler.contains("remote_file_editor"));
        assert!(!close_handler.contains("window.remove_window()"));
    }

    #[test]
    fn collapsed_connection_sidebar_keeps_top_bar_home_and_tree_controls() {
        let source = include_str!("onetcli_app.rs");
        let tab_bar = include_str!("../../crates/core/src/tab_container.rs");
        let sidebar_source = include_str!("persistent_connection_sidebar/mod.rs");
        let render = source
            .rsplit("impl Render for OnetCliApp")
            .next()
            .expect("OnetCliApp render source");

        // The old always-visible navigation rail is gone: Home and the tree
        // collapse control both live in the top tab bar now.
        assert!(tab_bar.contains("\"tab-bar-home\""));
        assert!(tab_bar.contains("\"navigation-sidebar-toggle\""));
        assert!(source.contains("HomePage::show_home(&home_page, window, cx)"));
        assert!(source.contains("set_home_button_active("));
        assert!(
            !render.contains("layout.child(self.connection_sidebar.clone())"),
            "连接树之外不再渲染常驻 rail 栏"
        );
        assert!(sidebar_source.contains("fn is_expanded"));
        assert!(sidebar_source.contains("fn render_floating_tree"));
    }

    #[test]
    fn auto_hide_off_renders_a_docked_split_panel_instead_of_a_floating_overlay() {
        let source = include_str!("onetcli_app.rs");
        let render = source
            .rsplit("impl Render for OnetCliApp")
            .next()
            .expect("OnetCliApp render source");

        assert!(
            render.contains("is_auto_hide_tree()"),
            "主渲染需读取连接树的自动隐藏开关"
        );
        assert!(
            render.contains("render_docked_connection_tree"),
            "非自动隐藏时应渲染并排的分割面板"
        );
        assert!(
            render.contains("show_persistent_sidebar && sidebar_expanded && auto_hide_tree"),
            "浮层连接树仅应在自动隐藏开启时渲染，避免遮挡终端"
        );
    }

    #[test]
    fn entering_home_preserves_the_saved_connection_sidebar_state() {
        let source = include_str!("onetcli_app.rs");
        let set_main_content = source
            .split("fn set_main_content(")
            .nth(1)
            .and_then(|source| source.split("\n    fn ").next())
            .expect("set_main_content source");
        assert!(
            !set_main_content.contains("set_connection_sidebar_expanded(true, cx)"),
            "进入主页时不得强制展开常驻侧边栏，应保留用户收起状态"
        );
        assert!(
            !source.contains("settings.connection_sidebar_expanded\n            || (show_navigation_sidebar_toggle && initial_home_active)"),
            "启动直接进入主页时也不得覆盖已保存的收起状态"
        );
        assert!(
            source.contains("set_persistent_sidebar_expanded(connection_sidebar_expanded, cx)"),
            "主页内容布局需与侧边栏展开状态保持同步"
        );
    }

    #[test]
    fn expanded_docked_sidebar_does_not_indent_the_macos_tab_bar() {
        let source = include_str!("onetcli_app.rs");
        let render = source
            .rsplit("impl Render for OnetCliApp")
            .next()
            .expect("OnetCliApp render source");
        let tab_bar = include_str!("../../crates/core/src/tab_container.rs");
        let tab_bar_impl = tab_bar.split("mod tests").next().unwrap();

        // 停靠展开时左侧由侧边栏负责，tab 栏不得再叠加红绿灯缩进。
        assert!(
            tab_bar_impl.contains("navigation_sidebar_expanded != Some(true)"),
            "仅收起或无侧边栏开关时才保留 macOS 红绿灯留白"
        );
        // 自动隐藏的浮动树不覆盖 tab 栏，此时 tab 栏仍需自己保留留白。
        assert!(
            render.contains("macos_title_bar_content_padding")
                && render.contains("set_left_padding("),
            "浮动树展开时 tab 栏需在渲染时同步 macOS 红绿灯留白"
        );
    }

    #[test]
    fn floating_connection_tree_occludes_scroll_from_reaching_the_tab_content() {
        let sidebar_source = include_str!("persistent_connection_sidebar/mod.rs");
        let floating = sidebar_source
            .split("fn render_floating_tree")
            .nth(1)
            .and_then(|source| source.split("\n    pub(crate) fn new").next())
            .expect("render_floating_tree source");

        assert!(
            floating.contains(".occlude()"),
            "浮动侧边栏应 occlude 命中测试，阻止滚轮事件传播到覆盖在下方的内容区"
        );
        assert!(
            floating.contains("stop_propagation"),
            "浮动侧边栏仍需拦截鼠标按下，避免点击穿透触发内容区行为"
        );
    }

    #[test]
    fn home_style_switches_between_legacy_home_and_modern_persistent_sidebar() {
        let app = include_str!("onetcli_app.rs");
        let home = include_str!("home_tab/render.rs");
        let legacy_home = include_str!("home_tab/legacy_home.rs");
        let sidebar = include_str!("home_tab/sidebar.rs");
        let sidebar_navigation = include_str!("home_tab/sidebar_navigation.rs");
        let persistent_sidebar = include_str!("persistent_connection_sidebar/mod.rs");
        let persistent_filter = include_str!("persistent_connection_sidebar/filter_bar.rs");
        let modern_home = include_str!("home_tab/modern_home.rs");
        let settings = include_str!("setting_tab.rs");

        assert!(app.contains("home_page_style.uses_persistent_sidebar()"));
        assert!(app.contains("set_navigation_sidebar_toggle("));
        // 常驻 rail 移除后，Home 与连接树开关由标签栏承载。
        assert!(app.contains("with_home_button("));
        assert!(app.contains("set_home_button("));
        assert!(home.contains("self.render_legacy_home(window, cx)"));
        assert!(home.contains("self.render_modern_home(window, cx)"));
        assert!(legacy_home.contains("self.render_sidebar(window, cx)"));
        assert!(sidebar_navigation.contains("for filter in visible_connection_types()"));
        assert!(!sidebar.contains("\"legacy-open-home\""));
        assert!(sidebar_navigation.contains("\"legacy-more-connection-types\""));
        assert!(sidebar_navigation.contains("\"legacy-more-applications\""));
        assert!(persistent_filter.contains("\"persistent-filter-button\""));
        assert!(persistent_filter.contains("ConnectionType::all()"));
        assert!(modern_home.contains("all_navigation_applications("));
        assert!(!persistent_sidebar.contains("HomePageStyle"));
        assert!(!persistent_sidebar.contains("render_legacy_sidebar"));
        assert!(settings.contains("HomePageStyle::Legacy"));
        assert!(settings.contains("HomePageStyle::Modern"));
        assert!(!settings.contains("ConnectionDisplay.connection_tree"));
    }

    #[test]
    fn next_always_on_top_state_prefers_observed_window_state() {
        assert!(super::next_always_on_top_from_state(false, None));
        assert!(!super::next_always_on_top_from_state(true, None));
        assert!(super::next_always_on_top_from_state(true, Some(false)));
        assert!(!super::next_always_on_top_from_state(false, Some(true)));
    }

    #[test]
    fn always_on_top_notification_message_includes_shortcut() {
        assert_eq!(
            "窗口已置顶。再次按 ⌃⌘T 可取消置顶。",
            super::always_on_top_notification_message(true, "⌃⌘T")
        );
        assert_eq!(
            "窗口已取消置顶。按 ⌃⌘T 可重新置顶。",
            super::always_on_top_notification_message(false, "⌃⌘T")
        );
    }

    #[test]
    fn always_on_top_error_notification_message_includes_shortcut() {
        let error = anyhow::anyhow!("NSWindow level 写入失败");

        assert_eq!(
            "窗口置顶切换失败：NSWindow level 写入失败。可再次按 ⌃⌘T 重试。",
            super::always_on_top_error_notification_message("⌃⌘T", &error)
        );
    }

    #[test]
    fn shortcut_label_formats_configured_shortcut() {
        #[cfg(target_os = "macos")]
        let expected = "⌃⌘T";
        #[cfg(not(target_os = "macos"))]
        let expected = "Ctrl+Win+T";

        assert_eq!(expected, super::shortcut_label("ctrl-cmd-t"));
    }

    #[test]
    fn quit_action_routes_through_active_window_quit_request() {
        let source = include_str!("onetcli_app.rs").replace("\r\n", "\n");
        let start = source.find("fn quit_app").expect("quit_app function");
        let end = source[start..]
            .find("\n}\n\nfn request_active_window_quit")
            .map(|offset| start + offset)
            .expect("quit_app function end");
        let quit_fn = &source[start..end];

        assert!(!quit_fn.contains("cx.quit()"));
        assert!(quit_fn.contains("request_active_window_quit(cx)"));
    }

    #[gpui::test]
    fn ssh_session_service_has_one_application_global_owner(cx: &mut gpui::TestAppContext) {
        let (first, second, runtime) = cx.update(|cx| {
            one_core::gpui_tokio::init(cx);
            init_ssh_session_service(cx);

            let first = cx.global::<GlobalSshSessionService>().service();
            let second = cx.global::<GlobalSshSessionService>().service();
            (first, second, Tokio::handle(cx))
        });

        let report = runtime.block_on(first.shutdown());

        assert!(!report.timed_out);
        assert_eq!(SshSessionServiceState::Stopped, second.snapshot().state);
    }

    #[test]
    fn confirmed_and_update_quit_paths_await_all_application_resource_shutdown() {
        let source = include_str!("onetcli_app.rs").replace("\r\n", "\n");
        let helper_start = source
            .find("pub(crate) fn shutdown_application_resources_and_quit")
            .expect("shared application resource shutdown helper");
        let helper_end = source[helper_start..]
            .find("\n}\n\n#[derive(Clone, Copy")
            .map(|offset| helper_start + offset)
            .expect("shared application resource shutdown helper end");
        let helper = &source[helper_start..helper_end];
        let start_rdp_shutdown = helper
            .find("remote_desktop_view::shutdown_windows_native_rdp(cx)")
            .expect("start Windows native RDP shutdown");
        let await_rdp_shutdown = helper
            .find("let rdp_shutdown_report = rdp_shutdown_task.await;")
            .expect("await Windows native RDP shutdown");
        let await_ssh_shutdown = helper
            .find("let shutdown_result = shutdown_task.await;")
            .expect("await SSH shutdown");
        let platform_quit = helper
            .find("cx.update(|cx| cx.quit())")
            .expect("platform quit");

        assert!(start_rdp_shutdown < await_rdp_shutdown);
        assert!(await_rdp_shutdown < await_ssh_shutdown);
        assert!(await_ssh_shutdown < platform_quit);
        assert!(helper.contains("log_windows_native_rdp_shutdown"));

        let confirm_start = source.find("fn confirm_quit").expect("confirm_quit");
        let confirm_end = source[confirm_start..]
            .find("\n    }\n}\n\n#[cfg(test)]")
            .map(|offset| confirm_start + offset)
            .expect("confirm_quit end");
        let confirm_quit = &source[confirm_start..confirm_end];
        assert!(confirm_quit.contains("shutdown_application_resources_and_quit"));
        assert!(!confirm_quit.contains("cx.quit()"));

        let update_dialog = include_str!("update/dialog.rs");
        assert!(update_dialog.contains("shutdown_application_resources_and_quit"));
        assert!(!update_dialog.contains("cx.quit()"));
    }

    #[test]
    fn platform_quit_fails_closed_native_rdp_before_ssh_without_recursive_quit() {
        let source = include_str!("onetcli_app.rs").replace("\r\n", "\n");
        let start = source
            .find("fn init_ssh_session_service")
            .expect("init_ssh_session_service");
        let end = source[start..]
            .find("\n}\n\nfn spawn_ssh_session_shutdown")
            .map(|offset| start + offset)
            .expect("init_ssh_session_service end");
        let init = &source[start..end];
        let callback_start = init
            .find("cx.on_app_quit(move |cx|")
            .expect("platform quit callback");
        let callback = &init[callback_start..];
        let fail_closed_rdp = callback
            .find("remote_desktop_view::fail_closed_windows_native_rdp_for_platform_quit(cx)")
            .expect("Native RDP platform-quit fail-closed fallback");
        let spawn_ssh = callback
            .find("Tokio::spawn(cx")
            .expect("SSH platform-quit fallback");

        assert!(fail_closed_rdp < spawn_ssh);
        assert!(callback.contains("log_windows_native_rdp_shutdown"));
        assert!(!callback.contains("shutdown_application_resources_and_quit"));
        assert!(!callback.contains("shutdown_windows_native_rdp(cx)"));
        assert!(!callback.contains("cx.quit()"));
    }

    #[test]
    fn onetcli_app_registers_window_close_guard() {
        let source = include_str!("onetcli_app.rs");
        let start = source.find("pub fn new").expect("OnetCliApp::new");
        let end = source[start..]
            .find("\n        let tab_container")
            .map(|offset| start + offset)
            .expect("OnetCliApp::new setup");
        let new_fn = &source[start..end];

        assert!(new_fn.contains("on_window_should_close"));
        assert!(new_fn.contains("request_quit(window, cx)"));
    }

    #[test]
    fn onetcli_app_persists_window_state_after_bounds_changes() {
        let source = include_str!("onetcli_app.rs");
        let start = source.find("pub fn new").expect("OnetCliApp::new");
        let end = source[start..]
            .find("\n        let settings")
            .map(|offset| start + offset)
            .expect("OnetCliApp::new window setup");
        let new_fn = &source[start..end];

        assert!(new_fn.contains("observe_window_bounds"));
        assert!(new_fn.contains("save_main_window_state(window, cx)"));
    }

    #[test]
    fn saved_main_window_state_includes_position_size_and_display() {
        let source = include_str!("onetcli_app.rs");
        let start = source
            .find("fn save_main_window_state")
            .expect("save_main_window_state");
        let end = source[start..]
            .find("\n    fn request_quit")
            .map(|offset| start + offset)
            .expect("save_main_window_state end");
        let save = &source[start..end];

        assert!(save.contains("bounds.origin.x"));
        assert!(save.contains("bounds.origin.y"));
        assert!(save.contains("bounds.size.width"));
        assert!(save.contains("bounds.size.height"));
        assert!(save.contains("let display_uuid = window"));
        assert!(save.contains(".display(cx)"));
        assert!(save.contains("display.uuid()"));
        assert!(save.contains("settings.main_window_state = Some(state)"));
    }

    #[test]
    fn request_quit_skips_confirmation_when_no_tabs_are_open() {
        let source = include_str!("onetcli_app.rs");
        let request_start = source.find("fn request_quit").expect("request_quit");
        let request_end = source[request_start..]
            .find("\n    fn show_quit_confirmation")
            .map(|offset| request_start + offset)
            .expect("request_quit end");
        let request_quit = &source[request_start..request_end];

        assert!(request_quit.contains("self.tab_container.read(cx).is_empty()"));
        assert!(request_quit.contains("self.confirm_quit(window, cx);"));
        assert!(request_quit.contains("show_quit_confirmation"));
    }

    #[test]
    fn native_driver_factories_are_ready_before_public_mcp_init() {
        let source = include_str!("onetcli_app.rs");
        let redis_init = source.find("redis_view::init(cx);").unwrap();
        let mongo_init = source.find("mongodb_view::init(cx);").unwrap();
        let native_factories = source
            .find("init_native_data_driver_factories(cx);")
            .unwrap();
        let public_mcp = source.find("crate::public_mcp_runtime::init(cx);").unwrap();

        assert!(redis_init < native_factories);
        assert!(mongo_init < native_factories);
        assert!(native_factories < public_mcp);
    }

    #[test]
    fn mongodb_open_strategy_guards_the_saved_native_driver_variant() {
        let source = include_str!("home/home_strategy.rs");
        let strategy = source
            .find("impl ConnectionOpenStrategy for MongoOpenStrategy")
            .unwrap();
        let body = &source[strategy..];
        let requirement = body
            .find("mongodb_driver_id(&connection)")
            .expect("MongoDB saved driver requirement");
        let guard = body
            .find("open_native_driver_connection_with_guard")
            .expect("native driver install guard");
        let open = body
            .find("open_mongodb_tab_with_mode")
            .expect("MongoDB tab open callback");

        assert!(requirement < guard);
        assert!(guard < open);
    }

    #[test]
    fn redis_open_strategy_guards_the_native_driver() {
        let source = include_str!("home/home_strategy.rs");
        let strategy = source
            .find("impl ConnectionOpenStrategy for RedisOpenStrategy")
            .unwrap();
        let body = &source[strategy..];
        let backend = body
            .find("default_backend_kind")
            .expect("Redis backend selection");
        let requirement = body
            .find("DEFAULT_REDIS_DRIVER_ID")
            .expect("Redis native driver requirement");
        let guard = body
            .find("open_native_driver_connection_with_guard")
            .expect("native driver install guard");
        let open = body
            .find("open_redis_tab_with_mode")
            .expect("Redis tab open callback");

        assert!(backend < requirement);
        assert!(requirement < guard);
        assert!(guard < open);
    }

    #[test]
    fn redis_factory_reloads_the_installed_driver_registry() {
        let source = include_str!("onetcli_app.rs");
        let init = source.find("fn init_native_data_driver_factories").unwrap();
        let body = &source[init..];

        assert!(body.contains("RedisConnectionFactory::from_installed_root"));
    }

    #[test]
    fn quit_state_opens_prompt_for_first_request() {
        let mut state = super::QuitRequestState::default();

        assert_eq!(super::QuitRequestDecision::OpenPrompt, state.request());
        assert!(state.prompt_open);
    }

    #[test]
    fn quit_state_ignores_duplicate_prompt_and_in_progress_requests() {
        let mut prompt_state = super::QuitRequestState {
            prompt_open: true,
            in_progress: false,
        };
        assert_eq!(super::QuitRequestDecision::Ignore, prompt_state.request());

        let mut running_state = super::QuitRequestState {
            prompt_open: false,
            in_progress: true,
        };
        assert_eq!(super::QuitRequestDecision::Ignore, running_state.request());
    }

    #[test]
    fn quit_state_resets_after_cancel_or_failed_close() {
        let mut state = super::QuitRequestState {
            prompt_open: true,
            in_progress: false,
        };

        state.cancel_prompt();
        assert_eq!(super::QuitRequestState::default(), state);

        state.prompt_open = true;
        assert!(state.confirm_prompt());
        assert_eq!(
            super::QuitRequestState {
                prompt_open: false,
                in_progress: true,
            },
            state
        );

        state.finish_close(false);
        assert_eq!(super::QuitRequestState::default(), state);
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn macos_window_level_maps_toggle_state() {
        assert_eq!(0, super::macos_window_level(false));
        assert_eq!(3, super::macos_window_level(true));
        assert!(!super::is_macos_always_on_top_level(0));
        assert!(super::is_macos_always_on_top_level(3));
        assert!(super::is_macos_always_on_top_level(-1));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn macos_always_on_top_activation_only_happens_when_enabled() {
        assert!(super::should_activate_after_always_on_top_change(true));
        assert!(!super::should_activate_after_always_on_top_change(false));
    }

    #[test]
    fn configured_log_file_path_uses_default_for_empty_value() {
        let default_path = default_log_file_path().expect("应返回默认日志路径");

        assert_eq!(configured_log_file_path("").unwrap(), default_path);
        assert_eq!(configured_log_file_path("   ").unwrap(), default_path);
    }

    #[test]
    fn configured_log_file_path_trims_value() {
        let path = configured_log_file_path("  /tmp/onetcli.log  ").expect("应返回日志路径");
        assert_eq!(path, std::path::PathBuf::from("/tmp/onetcli.log"));
    }

    #[test]
    fn configured_log_file_path_treats_extensionless_path_as_directory() {
        let directory =
            std::env::temp_dir().join(format!("onetcli-log-directory-test-{}", std::process::id()));

        let path = configured_log_file_path(&directory.to_string_lossy()).expect("应返回日志路径");

        assert_eq!(path, directory.join(LOG_FILE_NAME));
    }

    #[test]
    fn configured_log_file_path_uses_existing_directory() {
        let directory = std::env::temp_dir().join(format!(
            "onetcli-existing-log-directory-test-{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&directory).expect("应创建测试日志目录");

        let path = configured_log_file_path(&directory.to_string_lossy()).expect("应返回日志路径");

        assert_eq!(path, directory.join(LOG_FILE_NAME));

        let _ = std::fs::remove_dir_all(directory);
    }

    #[test]
    fn configured_log_file_path_preserves_existing_extensionless_file() {
        let file_path = std::env::temp_dir().join(format!(
            "onetcli-extensionless-log-file-test-{}",
            std::process::id()
        ));
        std::fs::write(&file_path, "").expect("应创建无扩展名测试文件");

        let path = configured_log_file_path(&file_path.to_string_lossy()).expect("应返回日志路径");

        assert_eq!(path, file_path);

        let _ = std::fs::remove_file(path);
    }

    #[cfg(windows)]
    #[test]
    fn configured_log_file_path_accepts_windows_directory() {
        let path = configured_log_file_path(r"D:\Navop\logs").expect("应返回 Windows 日志文件路径");

        assert_eq!(path, std::path::PathBuf::from(r"D:\Navop\logs\onetcli.log"));
    }

    #[test]
    fn log_file_appender_creates_parent_directories_and_appends() {
        let path = std::env::temp_dir()
            .join(format!("onetcli-log-test-{}", std::process::id()))
            .join("nested")
            .join("app.log");

        {
            let mut file = log_file_appender(&path).expect("应创建日志文件");
            writeln!(file, "first").expect("应写入第一行");
        }
        {
            let mut file = log_file_appender(&path).expect("应重新打开日志文件");
            writeln!(file, "second").expect("应追加第二行");
        }

        let content = std::fs::read_to_string(&path).expect("应读取日志文件");
        assert_eq!(content, "first\nsecond\n");

        let _ = std::fs::remove_dir_all(path.parent().unwrap().parent().unwrap());
    }

    #[cfg(unix)]
    #[test]
    fn log_file_appender_creates_private_file() {
        use std::os::unix::fs::PermissionsExt;

        let path = std::env::temp_dir()
            .join(format!(
                "onetcli-log-permission-test-{}",
                std::process::id()
            ))
            .join("app.log");
        let _file = log_file_appender(&path).expect("应创建日志文件");

        let mode = std::fs::metadata(&path)
            .expect("应读取日志文件元数据")
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(mode, 0o600);

        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }
}

impl Render for OnetCliApp {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let sheet_layer = Root::render_sheet_layer(window, cx);
        let dialog_layer = Root::render_dialog_layer(window, cx);
        let notification_layer = Root::render_notification_layer(window, cx);
        let main_content = self.render_main_content(cx);
        let show_persistent_sidebar = self.home_page_style.uses_persistent_sidebar();
        let sidebar_expanded = self.connection_sidebar.read(cx).is_expanded();
        let auto_hide_tree = self.connection_sidebar.read(cx).is_auto_hide_tree();
        let docked_tree = show_persistent_sidebar && sidebar_expanded && !auto_hide_tree;
        let floating_tree = if show_persistent_sidebar && sidebar_expanded && auto_hide_tree {
            Some(
                self.connection_sidebar
                    .update(cx, |sidebar, cx| sidebar.render_floating_tree(window, cx)),
            )
        } else {
            None
        };
        let docked_tree_element = if docked_tree {
            Some(self.connection_sidebar.update(cx, |sidebar, cx| {
                sidebar.render_docked_connection_tree(window, cx)
            }))
        } else {
            None
        };
        // 展开且停靠时，左侧由侧边栏自己负责（含 macOS 红绿灯区域），tab 栏
        // 不再缩进；自动隐藏的浮动树只覆盖 tab 栏以下区域，此时 tab 栏仍铺满
        // 窗口宽度，需要自己保留红绿灯留白。
        #[cfg(target_os = "macos")]
        if show_persistent_sidebar && sidebar_expanded {
            let tab_bar_left_padding = if auto_hide_tree {
                cx.theme().geometry.layout.macos_title_bar_content_padding
            } else {
                px(0.0)
            };
            self.tab_container.update(cx, |tabs, cx| {
                tabs.set_left_padding(tab_bar_left_padding, cx)
            });
        }
        div()
            .size_full()
            .relative()
            .opacity(AppSettings::global(cx).window_opacity)
            .drag_over::<ExternalPaths>(|element, _, _, cx| {
                element.bg(cx.theme().primary.opacity(0.08))
            })
            .on_drop(cx.listener(|_, paths: &ExternalPaths, window, cx| {
                for path in paths.paths() {
                    crate::file_open::open_input(
                        crate::file_open::FileOpenInput::Path(path.clone()),
                        window,
                        cx,
                    );
                }
            }))
            .on_action(cx.listener(|this, _: &ToggleConnectionSidebar, _, cx| {
                if !this.home_page_style.uses_persistent_sidebar() {
                    return;
                }
                let expanded = !this.connection_sidebar.read(cx).is_expanded();
                this.set_connection_sidebar_expanded(expanded, cx);
            }))
            .bg(cx.theme().background)
            .child({
                gpui_component::h_flex()
                    .size_full()
                    .min_w_0()
                    .overflow_hidden()
                    .when_some(docked_tree_element, |layout, tree| layout.child(tree))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .h_full()
                            .when(
                                show_persistent_sidebar && sidebar_expanded && auto_hide_tree,
                                |this| {
                                    this.on_mouse_down(
                                        gpui::MouseButton::Left,
                                        cx.listener(
                                            |this, event: &gpui::MouseDownEvent, _window, cx| {
                                                if !this.connection_sidebar.read(cx).is_expanded() {
                                                    return;
                                                }
                                                let layout = cx.theme().geometry.layout;
                                                // The navigation rail is gone; everything below
                                                // the tab bar on the left now belongs to the
                                                // tree overlay itself, so any content click
                                                // below the tab bar dismisses the auto-hide tree.
                                                let in_terminal = event.position.y > layout.tab_bar;
                                                if in_terminal {
                                                    this.set_connection_sidebar_expanded(false, cx);
                                                }
                                            },
                                        ),
                                    )
                                },
                            )
                            .child(main_content),
                    )
            })
            .when(
                show_persistent_sidebar && sidebar_expanded && auto_hide_tree,
                |this| this.child(floating_tree.unwrap()),
            )
            .children(sheet_layer)
            .children(dialog_layer)
            .children(notification_layer)
    }
}
