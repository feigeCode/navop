use crate::home_tab::{
    HomePage, NewConnectionShortcut, OpenConnectionQuickOpen, OpenLocalTerminalShortcut,
};
use crate::persistent_connection_sidebar::{
    PersistentConnectionSidebar, PersistentConnectionSidebarEvent,
};
use crate::resource_mount::{
    MountPreferences, MountVisibility, ResourceDeclaration, ResourceKind,
    plan_mount_with_navigation_width, resource_uses_connection_tree_state,
};
use gpui::prelude::FluentBuilder as _;
use gpui::{
    App, AppContext, Context, Entity, ExternalPaths, InteractiveElement, IntoElement, KeyBinding,
    Keystroke, ParentElement, Render, StatefulInteractiveElement as _, Styled, Task, Window,
    actions, div, px,
};
use gpui_component::{
    WindowExt,
    button::{Button, ButtonVariant, ButtonVariants as _},
    checkbox::Checkbox,
    dialog::{DialogButtonProps, DialogFooter},
    kbd::Kbd,
    notification::Notification,
    render_dialog_layer, render_notification_layer, render_sheet_layer,
};
use one_core::keybindings::{action_id, rebind_keybindings, shortcuts_for};
use one_core::settings::{AppLayout, CloseButtonBehavior};
use raw_window_handle::HasWindowHandle;
#[cfg(any(target_os = "macos", target_os = "windows"))]
use raw_window_handle::RawWindowHandle;
use rust_i18n::t;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

static ALWAYS_ON_TOP: AtomicBool = AtomicBool::new(false);

struct AlwaysOnTopNotification;

actions!(
    navop_app,
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
        CloseActiveTab,
        ToggleConnectionSidebar,
        CloseActiveWindow,
        QuitApp,
    ]
);

#[derive(Clone)]
pub struct GlobalHomePage {
    pub home_page: Entity<HomePage>,
    pub home_tab_id: String,
}

impl gpui::Global for GlobalHomePage {}

/// 新建连接向导的「更多连接 / 安装扩展」入口：切到扩展管理 tab。
/// 没有 nsh 宿主（32 位 Windows）时整个入口不注册，`app_kit` 自然不展示它。
#[cfg(feature = "nsh-host")]
struct NavopExtensionsHost;

/// 扩展连接种类的 id 前缀与分隔符：`ext:{extension_id}::{contribution_id}`。
///
/// 向导只把 kind id 当不透明字符串（选中 key、表单复用键），扩展侧需要在
/// 构造表单时拆回 (extension_id, contribution_id)。
#[cfg(feature = "nsh-host")]
const EXTENSION_KIND_PREFIX: &str = "ext:";
#[cfg(feature = "nsh-host")]
const EXTENSION_KIND_SEPARATOR: &str = "::";

#[cfg(feature = "nsh-host")]
fn extension_kind_id(extension_id: &str, contribution_id: &str) -> gpui::SharedString {
    format!("{EXTENSION_KIND_PREFIX}{extension_id}{EXTENSION_KIND_SEPARATOR}{contribution_id}")
        .into()
}

#[cfg(feature = "nsh-host")]
fn split_extension_kind_id(kind_id: &str) -> Option<(&str, &str)> {
    kind_id
        .strip_prefix(EXTENSION_KIND_PREFIX)?
        .split_once(EXTENSION_KIND_SEPARATOR)
}

/// 扩展种类的标签占位：真实名字来自运行时清单，恒走 `label_override`。
#[cfg(feature = "nsh-host")]
fn extension_kind_label(_cx: &App) -> gpui::SharedString {
    gpui::SharedString::default()
}

#[cfg(feature = "nsh-host")]
impl app_kit::ExtensionsHost for NavopExtensionsHost {
    fn open_extensions(
        &self,
        parent: gpui::AnyWindowHandle,
        _window: &mut Window,
        cx: &mut App,
    ) -> bool {
        let Some(home) = cx.try_global::<GlobalHomePage>() else {
            return false;
        };
        let home = home.home_page.clone();
        parent
            .update(cx, |_, window, cx| {
                home.update(cx, |home, cx| home.add_extensions_tab(window, cx));
            })
            .is_ok()
    }

    /// 已安装扩展贡献的连接：向导把它们归入「中间件」类目。
    fn connection_kinds(&self, cx: &App) -> Vec<domain_api::ConnectionKindDescriptor> {
        let Some(catalog) = crate::connection_visuals::extension_catalog_from_cx(cx) else {
            return Vec::new();
        };
        let mut kinds: Vec<_> = catalog
            .resource_connections()
            .map(|contribution| {
                let mut kind = domain_api::ConnectionKindDescriptor::new(
                    extension_kind_id(&contribution.extension_id, &contribution.id),
                    extension_kind_label,
                    domain_api::ConnectionKindGroup::Extension,
                    0,
                )
                .with_label(contribution.label.clone());
                if let Some(description) = contribution.description.clone() {
                    kind = kind.with_description(description);
                }
                if let Some(icon) = contribution.icon_path.clone() {
                    kind = kind.with_icon(domain_api::ConnectionKindIcon::File(icon));
                }
                kind
            })
            .collect();
        // 目录底层是 HashMap，按显示名排序保证卡片顺序稳定。
        kinds.sort_by_key(|kind| kind.display_label(cx).to_lowercase());
        kinds
    }

    fn build_connection_form(
        &self,
        request: domain_api::ConnectionFormRequest,
        window: &mut Window,
        cx: &mut App,
    ) -> Option<gpui::AnyView> {
        // 编辑 / 复制走主页的连接动作（`HomePage::show_extension_form`），向导只新建。
        if request.editing_connection.is_some() || request.initial_connection.is_some() {
            return None;
        }
        let (extension_id, contribution_id) = split_extension_kind_id(request.kind_id.as_ref())?;
        let contribution = crate::connection_visuals::extension_catalog_from_cx(cx)?
            .resource_connection(extension_id, contribution_id)?
            .clone();
        let (workspaces, teams) = match cx.try_global::<GlobalHomePage>() {
            Some(global) => {
                let workspaces = global.home_page.read(cx).workspaces.clone();
                (
                    workspaces,
                    one_core::cloud_sync::get_cached_team_options(cx),
                )
            }
            None => (Vec::new(), Vec::new()),
        };
        let config = universal_plugins::ExtensionConnectionFormConfig {
            contribution,
            editing_connection: None,
            initial_workspace_id: request.initial_workspace_id,
            workspaces,
            teams,
        };
        Some(
            cx.new(|cx| universal_plugins::ExtensionConnectionForm::new(config, window, cx))
                .into(),
        )
    }
}

#[derive(Clone)]
pub struct GlobalNavopApp {
    pub app: Entity<NavopApp>,
}

impl gpui::Global for GlobalNavopApp {}

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
            let Some(home) = cx
                .try_global::<GlobalHomePage>()
                .map(|global| global.home_page.clone())
            else {
                return;
            };
            home.update(cx, |home, cx| {
                home.open_connection_from_quick(&connection, window, cx)
            });
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

/// 右上角 GitHub 图标入口：用系统浏览器打开开源仓库。
fn github_button_handler() -> Arc<dyn Fn(&mut Window, &mut App) + Send + Sync> {
    Arc::new(|_window: &mut Window, cx: &mut App| {
        cx.open_url(crate::setting_tab::GITHUB_URL);
    })
}

/// Await bounded application-owned resource teardown before invoking GPUI's
/// platform quit routine. Native RDP hosts drain before their shared SSH
/// transports so COM/child-window cleanup retains a live application owner.
///
/// This is intentionally the only production helper that calls `cx.quit()`.
/// Repeated callers join the same idempotent Native RDP drain and
/// `SshSessionService::shutdown` lifecycle.
pub(crate) fn shutdown_application_resources_and_quit(cx: &mut App, reason: &'static str) {
    let domain_shutdowns = app_kit::shutdown_domains(cx);

    cx.spawn(async move |cx| {
        for shutdown in domain_shutdowns {
            shutdown.await;
        }

        let plugin_shutdown_task = cx.update(|cx| universal_plugins::spawn_shutdown(cx));
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

        // 托盘与全局热键的句柄都是 `!Send` 的平台对象，只能留在创建它们的主线程上。
        // 必须在 `cx.quit()` 之前显式释放，不能留给进程退出的 TLS 析构链：Linux(ksni)
        // 的 `TrayIcon::drop` 会回到 async-io，而 async-io 的线程本地是在托盘安装过程中
        // 才注册的，退出时反而先被销毁 —— 析构器撞上已销毁的 TLS 只能 panic → abort
        // （issue #336）。详见 `system_tray::shutdown`。
        let _ = cx.update(|_cx| {
            crate::system_tray::shutdown();
            crate::app_init::shutdown_system_hotkey();
        });

        let _ = cx.update(|cx| cx.quit());
    })
    .detach();
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct InitialContentLayout {
    home_tab_id: &'static str,
    pin_home: bool,
    active_pinned_index: Option<usize>,
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

fn initial_content_layout() -> InitialContentLayout {
    InitialContentLayout {
        home_tab_id: "home",
        pin_home: true,
        active_pinned_index: Some(0),
    }
}

use gpui_component::ActiveTheme;
use gpui_component::dock::ToggleZoom;
use one_core::llm::manager::GlobalProviderState;
use one_core::settings::{AppSettings, MainWindowState, StartupDefaultPage};
use one_core::storage::manager::get_config_dir;
use one_core::tab_container::{
    GlobalTabContainer, TabContainer, TabContainerEvent, TabContentRegistry, TabItem,
};
use one_core::tab_navigation::{
    ActiveTabSlot, TabCycleDirection, tab_number_target, tab_slot_after_cycle,
};
use one_core::themes;
#[cfg(unix)]
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};

use crate::setting_tab;

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

/// 按标签 id 激活（托盘「会话」菜单项入口）。id 已消失时静默忽略。
pub(crate) fn activate_tab_by_id(id: &str, window: &mut Window, cx: &mut App) {
    let Some(container) = cx.try_global::<GlobalTabContainer>() else {
        return;
    };
    let container = container.tab_container.clone();

    container.update(cx, |tc, cx| {
        if let Some(index) = tc.tabs().iter().position(|tab| tab.id().as_ref() == id) {
            tc.set_active_index(index, window, cx);
        }
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
    Keystroke::parse_with_os(shortcut, one_core::keybindings::current_os())
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

fn close_active_tab(cx: &mut App) {
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
                hp.close_active_tab(window, cx);
            });
        });
    });
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
    // 主窗口隐藏到托盘后没有 key window，但「显式退出」仍然必须落在主窗口上走
    // 退出确认与标签页关闭检查，不能直接拆资源退出（那会绕过确认）。
    // 只在完全没有活动窗口时回退到注册的主窗口，辅助窗口语义不变。
    let Some(active_window) = cx
        .active_window()
        .or_else(crate::app_init::main_window_handle)
    else {
        shutdown_application_resources_and_quit(cx, "quit without an active window");
        return;
    };
    cx.defer(move |cx| {
        _ = active_window.update(cx, |_, window, cx| {
            request_window_quit(window, cx);
        });
    });
}

/// 走既有退出流程的公共入口：托盘「退出 Navop」、`QuitApp` 快捷键、应用菜单都经过它。
pub(crate) fn request_window_quit(window: &mut Window, cx: &mut App) {
    let Some(app) = cx
        .try_global::<GlobalNavopApp>()
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

fn close_active_tab_default_shortcuts() -> &'static [&'static str] {
    if cfg!(target_os = "macos") {
        &["cmd-shift-w"]
    } else {
        &["ctrl-f4", "alt-shift-w"]
    }
}

fn close_active_window_default_shortcut() -> &'static str {
    default_shortcut("cmd-w", "ctrl-shift-w")
}

const LOG_FILE_NAME: &str = "navop.log";

/// 单个日志文件的大小上限。超过后轮转到 `<文件名>.1`，
/// 避免长期使用或错误风暴把日志撑到 GB 级（issue #534 实测一晚 214G）。
pub(crate) const LOG_FILE_MAX_BYTES: u64 = 64 * 1024 * 1024;

/// 轮转后保留的历史文件后缀，只保留最近一份，所以磁盘上限约为 2 × `LOG_FILE_MAX_BYTES`。
const LOG_FILE_ROTATED_SUFFIX: &str = ".1";

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

/// 只在启动时轮转不够：一次长时间运行里的错误风暴能把单个文件写到几十 GB。
/// 这个写入器在**运行期**也检查体积，超过上限就改名成 `<文件名>.1`（覆盖上一份）
/// 并重新创建文件，磁盘占用因此稳定在 2 × 上限。
///
/// 写入器内部**不能**再打日志：它本身就是日志落盘路径，重入会递归。
/// 轮转失败一律静默降级为继续追加，与启动轮转的“体积不收敛也不能挡住日志”一致。
pub(crate) struct RotatingLogWriter {
    path: PathBuf,
    file: Option<std::fs::File>,
    written: u64,
    max_bytes: u64,
}

impl RotatingLogWriter {
    pub(crate) fn open(path: &Path) -> std::io::Result<Self> {
        Self::open_with_limit(path, LOG_FILE_MAX_BYTES)
    }

    fn open_with_limit(path: &Path, max_bytes: u64) -> std::io::Result<Self> {
        let file = log_file_appender(path)?;
        let written = file.metadata().map(|metadata| metadata.len()).unwrap_or(0);
        Ok(Self {
            path: path.to_path_buf(),
            file: Some(file),
            written,
            max_bytes,
        })
    }

    /// 把当前文件挪到 `.1` 并重新打开同名文件。
    fn rotate(&mut self) {
        // 先关闭句柄：Windows 上被自己打开的文件能否改名取决于共享模式，关掉最稳。
        self.file = None;

        let mut rotated = self.path.as_os_str().to_os_string();
        rotated.push(LOG_FILE_ROTATED_SUFFIX);
        let rotated = PathBuf::from(rotated);
        let _ = std::fs::remove_file(&rotated);
        let _ = std::fs::rename(&self.path, &rotated);

        if let Ok(file) = default_log_file_options().open(&self.path) {
            self.written = file.metadata().map(|metadata| metadata.len()).unwrap_or(0);
            self.file = Some(file);
        }
    }
}

impl std::io::Write for RotatingLogWriter {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        // `written > 0`：单条日志本身就超上限时不要每次写都反复轮转。
        if self.written > 0 && self.written.saturating_add(buf.len() as u64) > self.max_bytes {
            self.rotate();
        }

        let Some(file) = self.file.as_mut() else {
            return Err(std::io::Error::other("log file is not open"));
        };
        let written = file.write(buf)?;
        self.written = self.written.saturating_add(written as u64);
        Ok(written)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        match self.file.as_mut() {
            Some(file) => file.flush(),
            None => Ok(()),
        }
    }
}

pub(crate) fn log_file_appender(path: &Path) -> std::io::Result<std::fs::File> {
    if let Some(parent) = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        std::fs::create_dir_all(parent)?;
    }

    // 在打开之前轮转：改名后重新创建同名文件，本次写入照常落到干净的日志里。
    rotate_oversized_log(path);

    default_log_file_options().open(path)
}

fn default_log_file_options() -> std::fs::OpenOptions {
    let mut options = std::fs::OpenOptions::new();
    options.create(true).append(true);
    #[cfg(unix)]
    options.mode(0o600);
    options
}

/// 日志超过 [`LOG_FILE_MAX_BYTES`] 时把它改名为 `<文件名>.1`（覆盖上一份）。
///
/// 改名走 inode，因此即使另有实例正持有旧文件也不会丢日志——对方继续写 `.1`，
/// 本实例写新文件。轮转失败（权限、磁盘满等）只意味着体积不收敛，绝不能挡住日志本身。
fn rotate_oversized_log(path: &Path) {
    let Ok(metadata) = std::fs::metadata(path) else {
        return;
    };
    if metadata.len() < LOG_FILE_MAX_BYTES {
        return;
    }

    let mut rotated = path.as_os_str().to_os_string();
    rotated.push(LOG_FILE_ROTATED_SUFFIX);
    let rotated = PathBuf::from(rotated);

    // Unix 上 `rename` 会覆盖已存在的目标，Windows 上不保证，先删更稳。
    let _ = std::fs::remove_file(&rotated);
    if let Err(error) = std::fs::rename(path, &rotated) {
        tracing::warn!(%error, path = %path.display(), "日志轮转失败，继续追加到原文件");
    }
}

pub fn init(cx: &mut App) -> anyhow::Result<()> {
    gpui_component::init(cx);
    one_core::themes::load_bundled(cx);
    one_core::themes::load_imported(cx);
    setting_tab::init_settings(cx);
    one_core::init(cx)?;
    #[cfg(feature = "shell")]
    app_kit::set_shell_host(Arc::new(crate::home_tab::domain_host::NavopShellHost));
    // 新建连接向导的「更多连接 / 安装扩展」入口由应用壳实现，`app_kit` 不感知扩展运行时。
    #[cfg(feature = "nsh-host")]
    app_kit::set_extensions_host(Arc::new(NavopExtensionsHost));
    ai_chat_view::init(cx);
    mcp_approval::init(cx);
    crate::ai_chat_acp::init(cx);
    one_ui::init_table_display_settings(
        cx,
        one_ui::TableDisplaySettings::new(AppSettings::global(cx).table_row_height)
            .with_column_comment(AppSettings::global(cx).table_show_column_comment),
    );
    let table_keybindings = table_keybindings(cx);
    one_ui::init(cx, table_keybindings);
    // ai_chat_view 的会话搜索快捷键随 `ai_chat_view::init(cx)` 一起注册（见该 crate 的 lib.rs）。
    crate::auth::init(cx);
    crate::license::init(cx);
    {
        let global_provider_state = cx.global::<GlobalProviderState>().clone();
        global_provider_state
            .set_proxy_settings(&AppSettings::global(cx).global_proxy)
            .expect("LLM 代理初始化失败");
        global_provider_state
            .set_request_timeout_secs(Some(AppSettings::global(cx).ai_chat.request_timeout_secs));
    }
    // 领域专属子系统（数据库缓存、SQL 编辑器、Redis/Mongo 视图、终端视图等）
    // 由各领域包自己初始化，公共壳只知道“按注册顺序调用”。
    app_kit::init_domains(cx)?;
    crate::personal_sync_runtime::init(cx);
    // 独立产品（`navop-shell` / `navop-data`）用 `app_kit` 托管的通用 MCP 宿主
    // (`mcp_host`)；全量产品才启动自己的 `public_mcp_runtime`，否则两套 MCP
    // 会同时监听，发现文件与工具集互相覆盖。
    if app_kit::identity()
        .is_none_or(|identity| identity.app_id == domain_api::AppIdentity::FULL.app_id)
    {
        crate::public_mcp_runtime::init(cx);
    }
    crate::home_tab::init(cx);
    cx.bind_keys(init_keybindings(cx));
    init_action_handlers(cx);

    let registry = TabContentRegistry::new();
    cx.set_global(registry);

    cx.activate(true);
    Ok(())
}

pub fn refresh_keybindings(cx: &mut App) {
    cx.bind_keys(refreshable_keybindings(cx));
    crate::home_tab::refresh_keybindings(cx);
    // AI 工作台的快捷键是 app 级（无 context 冲突），留在原位即可。
    ai_chat_view::find_shortcut::refresh_keybindings(cx);
    ai_chat_view::session_shortcut::refresh_keybindings(cx);
    let table_keybindings = table_keybindings(cx);
    one_ui::refresh_keybindings(cx, table_keybindings);
    remote_file_editor::refresh_keybindings(cx);
    notes::refresh_keybindings(cx);
    // 领域快捷键放在通用绑定之后：顺序契约见 `app_kit::refresh_domain_keybindings`。
    app_kit::refresh_domain_keybindings(cx);
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
    .with_find(
        shortcuts_for(
            cx,
            action_id::TABLE_FIND,
            &[table_shortcut("cmd-f", "ctrl-f")],
        ),
        shortcuts_for(
            cx,
            action_id::TABLE_FIND_NEXT,
            &[table_shortcut("cmd-g", "ctrl-g")],
        ),
        shortcuts_for(
            cx,
            action_id::TABLE_FIND_PREVIOUS,
            &[table_shortcut("cmd-shift-g", "ctrl-shift-g")],
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
            .map(|key| one_core::keybindings::load_dynamic_keybinding(&key, ToggleZoom, None, cx)),
    );
    keybindings.extend(
        shortcuts_for(
            cx,
            action_id::WINDOW_CLOSE_ACTIVE_WINDOW,
            &[close_active_window_default_shortcut()],
        )
        .into_iter()
        .map(|key| {
            one_core::keybindings::load_dynamic_keybinding(&key, CloseActiveWindow, None, cx)
        }),
    );
    keybindings.extend(
        shortcuts_for(
            cx,
            action_id::WINDOW_TOGGLE_CONNECTION_SIDEBAR,
            &[default_shortcut("cmd-b", "ctrl-b")],
        )
        .into_iter()
        .map(|key| {
            one_core::keybindings::load_dynamic_keybinding(&key, ToggleConnectionSidebar, None, cx)
        }),
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
        .map(|key| {
            one_core::keybindings::load_dynamic_keybinding(&key, ToggleFullscreen, None, cx)
        }),
    );
    keybindings.extend(
        shortcuts_for(
            cx,
            action_id::WINDOW_TOGGLE_ALWAYS_ON_TOP,
            &[default_shortcut("ctrl-cmd-t", "ctrl-alt-t")],
        )
        .into_iter()
        .map(|key| {
            one_core::keybindings::load_dynamic_keybinding(&key, ToggleAlwaysOnTop, None, cx)
        }),
    );
    keybindings.extend(
        shortcuts_for(
            cx,
            action_id::APP_DUPLICATE_TAB,
            &[default_shortcut("cmd-shift-t", "alt-shift-t")],
        )
        .into_iter()
        .map(|key| one_core::keybindings::load_dynamic_keybinding(&key, DuplicateTab, None, cx)),
    );
    keybindings.extend(
        shortcuts_for(
            cx,
            action_id::APP_OPEN_TAB_SWITCHER,
            &[default_shortcut("cmd-j", "ctrl-j")],
        )
        .into_iter()
        .map(|key| one_core::keybindings::load_dynamic_keybinding(&key, OpenTabSwitcher, None, cx)),
    );
    keybindings.extend(
        shortcuts_for(cx, action_id::APP_SWITCH_NEXT_TAB, &["ctrl-tab"])
            .into_iter()
            .map(|key| {
                one_core::keybindings::load_dynamic_keybinding(&key, SwitchNextTab, None, cx)
            }),
    );
    keybindings.extend(
        shortcuts_for(cx, action_id::APP_SWITCH_PREVIOUS_TAB, &["ctrl-shift-tab"])
            .into_iter()
            .map(|key| {
                one_core::keybindings::load_dynamic_keybinding(&key, SwitchPreviousTab, None, cx)
            }),
    );
    keybindings.extend(
        shortcuts_for(
            cx,
            action_id::APP_CLOSE_ACTIVE_TAB,
            close_active_tab_default_shortcuts(),
        )
        .into_iter()
        .map(|key| one_core::keybindings::load_dynamic_keybinding(&key, CloseActiveTab, None, cx)),
    );
    keybindings.extend(
        shortcuts_for(
            cx,
            action_id::APP_QUIT,
            &[default_shortcut("cmd-q", "alt-f4")],
        )
        .into_iter()
        .map(|key| one_core::keybindings::load_dynamic_keybinding(&key, QuitApp, None, cx)),
    );
    // 领域注册的键位（例如 SFTP 视图的粘贴）。
    keybindings.extend(app_kit::domain_keybindings(cx));

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
    // 领域注册的键位（例如 SFTP 视图的粘贴）。
    keybindings.extend(app_kit::domain_keybindings(cx));
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
        action_id::APP_CLOSE_ACTIVE_TAB,
        close_active_tab_default_shortcuts(),
        None,
        CloseActiveTab,
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
    cx.on_action(|_: &CloseActiveTab, cx| close_active_tab(cx));
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
                    hp.show_new_connection_dialog(None, window, cx);
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
                #[cfg(feature = "shell")]
                home_page.update(cx, |hp, cx| {
                    hp.open_local_terminal_tab(None, window, cx);
                });
            });
        });
    });
    cx.on_action(|_: &ai_chat_view::OpenLlmProviderSettings, cx| {
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
                home_page.update(cx, |hp, cx| hp.add_llm_providers_settings_tab(window, cx));
            });
        });
    });
}

fn close_active_window(cx: &mut App) {
    let Some(active_window) = cx.active_window() else {
        return;
    };

    if Some(active_window) == crate::app_init::main_window_handle() {
        cx.defer(move |cx| {
            let _ = active_window.update(cx, |_, window, cx| {
                handle_main_window_close_request(window, cx);
            });
        });
        return;
    }

    one_core::window_close::request_close_window(active_window, cx);
}

/// 主窗口关闭统一入口：标题栏、系统关闭事件和“关闭当前窗口”快捷键共用。
fn handle_main_window_close_request(window: &mut Window, cx: &mut App) {
    match crate::system_tray::main_window_close_action(
        crate::system_tray::is_available(),
        AppSettings::current(cx).close_button_behavior,
    ) {
        crate::system_tray::MainWindowCloseAction::AskUser => {
            let Some(app) = cx
                .try_global::<GlobalNavopApp>()
                .map(|global| global.app.clone())
            else {
                return;
            };
            let _ = app.update(cx, |app, cx| {
                app.show_close_choice_prompt(window, cx);
            });
        }
        crate::system_tray::MainWindowCloseAction::HideToTray => {
            let Some(app) = cx
                .try_global::<GlobalNavopApp>()
                .map(|global| global.app.clone())
            else {
                return;
            };
            let _ = app.update(cx, |app, cx| {
                app.hide_main_window_to_tray(window, cx);
            });
        }
        crate::system_tray::MainWindowCloseAction::RequestQuit => {
            request_window_quit(window, cx);
        }
    }
}

pub struct NavopApp {
    tab_container: Entity<TabContainer>,
    connection_sidebar: Entity<PersistentConnectionSidebar>,
    quit_state: QuitRequestState,
    /// 「最小化到托盘 / 退出应用」弹窗是否已经打开：标题栏关闭按钮可能被连点，
    /// 不能叠出第二个完全相同的弹窗。
    close_choice_prompt_open: bool,
    main_window_size_save_task: Option<Task<()>>,
    _appearance_subscription: gpui::Subscription,
    _settings_subscription: gpui::Subscription,
    resource_overlay_dismissed: bool,
    #[cfg(feature = "shell")]
    resource_resize_drag: Option<crate::resource_mount::ResourceResizeDrag>,
    /// 拖动中的资源栏宽度（键为 `stable_width_key`）。外壳每帧都从设置求解布局宽度，
    /// 而设置落盘有防抖，所以拖动期间必须用这份会话内覆盖值，面板才会跟手。
    resource_width_drag: Option<(String, f32)>,
    #[cfg(feature = "shell")]
    resource_width_persist_task: Option<Task<()>>,
}

/// 关闭弹窗里「记住我的选择」勾选框的状态。
///
/// 刻意做成独立实体而不是直接读 [`NavopApp`] 的字段：弹窗 body 由 `Root` 渲染，
/// `NavopApp` 的 `notify()` 不会重绘它，勾选框必须自己负责刷新。
struct CloseChoiceRememberState {
    remember: bool,
}

impl Render for CloseChoiceRememberState {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        Checkbox::new("tray-close-remember")
            .checked(self.remember)
            .label(t!("Tray.close_remember").to_string())
            .on_click(cx.listener(|this, checked, _, cx| {
                this.remember = *checked;
                cx.notify();
            }))
    }
}

impl NavopApp {
    /// 仅读取活动标签元数据，避免外壳借用 HomePage 时再次读同一实体。
    fn active_resource_metadata(
        &self,
        cx: &App,
    ) -> (Option<&'static str>, Option<ResourceKind>, bool) {
        let Some(content) = self.tab_container.read(cx).active_content_snapshot() else {
            return (None, None, false);
        };
        let key = content.content_key(cx);
        if matches!(
            key,
            "Home" | "Notes" | "AiWorkbench" | "Settings" | "Terminal"
        ) {
            return (Some(key), None, false);
        }
        let kind = match key {
            "Database" => ResourceKind::DbObjects,
            "Redis" => ResourceKind::RedisKeys,
            "MongoDB" => ResourceKind::MongoCollections,
            _ => ResourceKind::None,
        };
        (Some(key), Some(kind), key == "Database")
    }

    fn active_resource(&self, cx: &App) -> (ResourceKind, bool, bool) {
        let Some(content) = self.tab_container.read(cx).active_content_snapshot() else {
            return (ResourceKind::None, false, false);
        };
        match content.content_key(cx) {
            "Home" => {
                let managing = content
                    .view()
                    .downcast::<HomePage>()
                    .ok()
                    .is_some_and(|home| home.read(cx).navigation_connection_manager_open);
                if managing {
                    // 中央全集管理展开时收起左侧连接树，避免相同功能重复。
                    (ResourceKind::None, false, false)
                } else {
                    (ResourceKind::Connections, false, false)
                }
            }
            "Notes" => (ResourceKind::Notes, false, false),
            "AiWorkbench" => (ResourceKind::AiSessions, false, false),
            "Settings" => (ResourceKind::SettingsCategories, false, false),
            "Database" => (ResourceKind::DbObjects, false, false),
            "Redis" => (ResourceKind::RedisKeys, false, false),
            "MongoDB" => (ResourceKind::MongoCollections, false, false),
            #[cfg(feature = "shell")]
            "Terminal" => {
                let ssh = content
                    .view()
                    .downcast::<terminal_view::TerminalWorkspace>()
                    .ok()
                    .is_some_and(|terminal| {
                        terminal.read(cx).connection_kind(cx)
                            == terminal_view::TerminalConnectionKind::Ssh
                    });
                (ResourceKind::Connections, ssh, !ssh)
            }
            _ => (ResourceKind::None, false, false),
        }
    }

    fn render_global_navigation(
        &self,
        active_key: Option<&str>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<gpui::AnyElement> {
        let home = cx.try_global::<GlobalHomePage>()?.home_page.clone();
        Some(home.update(cx, |home, cx| {
            home.render_sidebar_for_active(active_key, window, cx)
                .into_any_element()
        }))
    }

    fn render_application_resource(
        &mut self,
        kind: ResourceKind,
        width: gpui::Pixels,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<gpui::AnyElement> {
        let _ = window;
        match kind {
            ResourceKind::Connections => Some(
                self.connection_sidebar
                    .update(cx, |tree, cx| tree.render_application_resource(width, cx)),
            ),
            ResourceKind::Notes => {
                let notes = self
                    .tab_container
                    .read(cx)
                    .active_content_snapshot()?
                    .view()
                    .downcast::<notes::NotesView>()
                    .ok()?;
                Some(notes.update(cx, |notes, cx| notes.render_external_sidebar(width, cx)))
            }
            ResourceKind::AiSessions => {
                let ai = self
                    .tab_container
                    .read(cx)
                    .active_content_snapshot()?
                    .view()
                    .downcast::<ai_chat_view::WorkbenchShell>()
                    .ok()?;
                ai.update(cx, |ai, cx| ai.render_external_session_nav(width, cx))
            }
            ResourceKind::SettingsCategories => {
                let settings = self
                    .tab_container
                    .read(cx)
                    .active_content_snapshot()?
                    .view()
                    .downcast::<crate::setting_tab::SettingsPanel>()
                    .ok()?;
                Some(settings.update(cx, |settings, cx| {
                    settings.render_external_categories(width, cx)
                }))
            }
            ResourceKind::DbObjects | ResourceKind::RedisKeys | ResourceKind::MongoCollections => {
                let view = self.tab_container.read(cx).application_resource_view(cx)?;
                // 面板右缘不再自己描边：分隔线由外壳的拖动条画在内容起点，
                // 否则灰底上会多出一道和连接面板不一致的接缝。
                Some(
                    div()
                        .w(width)
                        .h_full()
                        .min_w_0()
                        .bg(cx.theme().sidebar)
                        .child(view)
                        .into_any_element(),
                )
            }
            _ => None,
        }
    }

    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let app_entity = cx.entity();
        cx.set_global(GlobalNavopApp {
            app: app_entity.clone(),
        });
        window.on_window_should_close(cx, move |window, cx| {
            // 托盘可用但用户还没固化偏好时先询问；窗口一律保留（返回 false），
            // 由选择决定隐藏到托盘还是走退出确认。
            handle_main_window_close_request(window, cx);
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
        let layout = initial_content_layout();
        // 侧边栏展开状态完全跟随用户上次保存的选择，进入主页不强制展开。
        let connection_sidebar_expanded = settings.connection_sidebar_expanded;
        let tab_container = cx.new(|cx| {
            let mut container = TabContainer::new(window, cx)
                .with_tab_bar_when_empty(true)
                .with_add_tab_button(add_tab_button_handler())
                .with_tab_quick_open(tab_quick_open_resolver())
                .with_settings_button(settings_button_handler())
                .with_github_button(github_button_handler());

            container = container.with_navigation_sidebar_toggle(connection_sidebar_expanded);

            #[cfg(target_os = "macos")]
            {
                container = container
                    .with_macos_titlebar_inset(true)
                    .with_left_padding(
                        one_ui::theme_geometry()
                            .layout
                            .macos_title_bar_content_padding,
                    )
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
            home_tab_id: layout.home_tab_id.to_owned(),
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
        home_page.update(cx, |home, cx| {
            home.set_connection_sidebar(connection_sidebar.clone(), cx)
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

        tab_container.update(cx, |tc, cx| {
            if layout.pin_home {
                let home_tab = TabItem::new(layout.home_tab_id, "app", home_page.clone());
                tc.insert_pinned_tab_at(0, home_tab, cx);
            }
        });

        cx.subscribe(&tab_container, |_this, _, event: &TabContainerEvent, cx| {
            let app = cx.entity();
            match event {
                TabContainerEvent::NavigationSidebarToggled { expanded } => {
                    let expanded = *expanded;
                    cx.defer(move |cx| {
                        app.update(cx, |app, cx| {
                            // 开关作用于当前标签的资源栏，不再一律算到连接树上。
                            app.set_active_resource_sidebar_expanded(expanded, cx);
                        });
                    });
                }
                TabContainerEvent::WorkbenchNavToggled { expanded } => {
                    let expanded = *expanded;
                    cx.defer(move |cx| {
                        app.update(cx, |app, cx| {
                            let content = app
                                .tab_container
                                .read(cx)
                                .active_content_snapshot()
                                .map(|content| content.view());
                            if let Some(content) = content {
                                if let Ok(ai) = content.downcast::<ai_chat_view::WorkbenchShell>() {
                                    ai.update(cx, |ai, cx| {
                                        ai.set_session_nav_collapsed(!expanded, cx)
                                    });
                                }
                            }
                        });
                    });
                }
                TabContainerEvent::HomeMenuToggled { expanded } => {
                    let expanded = *expanded;
                    let home_page = cx
                        .try_global::<GlobalHomePage>()
                        .map(|home| home.home_page.clone());
                    if let Some(home_page) = home_page {
                        cx.defer(move |cx| {
                            home_page.update(cx, |home, cx| {
                                home.set_sidebar_expanded(expanded, cx);
                            });
                        });
                    }
                }
                TabContainerEvent::TabActivated { .. } => {
                    cx.defer(move |cx| {
                        app.update(cx, |app, cx| {
                            app.sync_connection_sidebar_theme(cx);
                            app.resource_overlay_dismissed = false;
                            // 切页影响主页判定（常驻侧栏显隐），需要重渲染。
                            cx.notify();
                        });
                    });
                }
                TabContainerEvent::LayoutChanged
                | TabContainerEvent::TabClosed { .. }
                | TabContainerEvent::TabDuplicated { .. } => {
                    cx.defer(move |cx| {
                        app.update(cx, |app, cx| {
                            app.sync_connection_sidebar_theme(cx);
                            cx.notify();
                        });
                    });
                }
                TabContainerEvent::TabAttention {
                    tab_id,
                    tab_title,
                    tab_is_active,
                    window_is_active,
                    attention,
                } => {
                    let options =
                        app_kit::integration::desktop_notification::DesktopNotificationOptions::from_settings(
                            AppSettings::global(cx),
                        );
                    app_kit::integration::desktop_notification::notify_tab_attention(
                        options,
                        tab_id,
                        tab_title.as_ref(),
                        *tab_is_active,
                        *window_is_active,
                        attention,
                    );
                }
            }
        })
        .detach();
        // 标签集合/激活态变化 → 重建托盘「会话」菜单。菜单展开与否不可观测，只能推送。
        cx.subscribe(&tab_container, |_this, _, event: &TabContainerEvent, cx| {
            if !matches!(event, TabContainerEvent::TabAttention { .. }) {
                cx.defer(|cx| crate::system_tray::sync_sessions_from(cx));
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
        if settings.startup_default_page == StartupDefaultPage::AiWorkbench {
            let home_for_ai_workbench = home_page.clone();
            window.defer(cx, move |window, cx| {
                home_for_ai_workbench.update(cx, |home, cx| {
                    home.add_ai_workbench_tab(window, cx);
                });
            });
        }
        let appearance_subscription = window.observe_window_appearance(|_, cx| {
            let settings = AppSettings::current(cx);
            if settings.auto_switch_theme || settings.theme_mode == "system" {
                themes::apply_appearance(&settings, cx);
            }
        });
        one_ui::set_interface_density(match AppSettings::global(cx).interface_density {
            one_core::settings::AppearanceDensity::Compact => one_ui::InterfaceDensity::Compact,
            one_core::settings::AppearanceDensity::Normal => one_ui::InterfaceDensity::Normal,
            one_core::settings::AppearanceDensity::Comfortable => {
                one_ui::InterfaceDensity::Comfortable
            }
        });
        tab_container.update(cx, |tabs, cx| tabs.set_layout_mode(cx));

        // 外观/密度/布局改变只重新排列同一批实体，不重建工作区。
        let settings_subscription = cx.observe_global::<AppSettings>(|this, cx| {
            this.resource_overlay_dismissed = false;
            one_ui::set_interface_density(match AppSettings::global(cx).interface_density {
                one_core::settings::AppearanceDensity::Compact => one_ui::InterfaceDensity::Compact,
                one_core::settings::AppearanceDensity::Normal => one_ui::InterfaceDensity::Normal,
                one_core::settings::AppearanceDensity::Comfortable => {
                    one_ui::InterfaceDensity::Comfortable
                }
            });
            this.tab_container
                .update(cx, |tabs, cx| tabs.set_layout_mode(cx));
            cx.notify();
        });
        Self {
            tab_container,
            connection_sidebar,
            quit_state: QuitRequestState::default(),
            close_choice_prompt_open: false,
            main_window_size_save_task: None,
            _appearance_subscription: appearance_subscription,
            _settings_subscription: settings_subscription,
            resource_overlay_dismissed: false,
            #[cfg(feature = "shell")]
            resource_resize_drag: None,
            resource_width_drag: None,
            #[cfg(feature = "shell")]
            resource_width_persist_task: None,
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

    /// 数据库类标签的左侧对象树也由外壳托管；它的宿主与连接树的折叠状态无关，
    /// 折叠只让外壳不画，业务页不能因此再切回内部树。
    fn hosts_database_tree(&self, cx: &App) -> bool {
        matches!(
            self.active_resource(cx).0,
            ResourceKind::DbObjects | ResourceKind::RedisKeys | ResourceKind::MongoCollections
        )
    }

    fn external_primary_sidebar(&self, cx: &App) -> bool {
        AppSettings::global(cx).app_layout == AppLayout::GlobalNavigation
            || self.hosts_database_tree(cx)
    }

    /// 某个资源栏现在是展开还是收起。
    ///
    /// 连接树自己维护折叠位（`connection_sidebar_expanded`），其它资源栏读设置里
    /// 各自的条目；没有记录时按展开算，和「新开的资源栏默认可见」一致。
    fn resource_sidebar_expanded(&self, resource: ResourceKind, cx: &App) -> bool {
        if resource_uses_connection_tree_state(resource) {
            return self.connection_sidebar.read(cx).is_expanded();
        }
        AppSettings::global(cx)
            .resource_sidebar_expanded
            .get(crate::resource_mount::stable_resource_key(resource))
            .copied()
            .unwrap_or(true)
    }

    pub(crate) fn toggle_application_resource(&mut self, cx: &mut Context<Self>) {
        let expanded = {
            let (resource, _, _) = self.active_resource(cx);
            !self.resource_sidebar_expanded(resource, cx)
        };
        self.set_active_resource_sidebar_expanded(expanded, cx);
    }

    /// 把「当前标签的资源栏」设成展开/收起。
    ///
    /// 标签栏上的资源栏开关（`NavigationSidebarToggled`）也走这里：在数据库页收起的
    /// 是对象树，在笔记页收起的是目录，不能一律算到连接树上，否则点了没反应。
    pub(crate) fn set_active_resource_sidebar_expanded(
        &mut self,
        expanded: bool,
        cx: &mut Context<Self>,
    ) {
        let (resource, _, _) = self.active_resource(cx);
        if expanded {
            self.resource_overlay_dismissed = false;
        }
        if resource_uses_connection_tree_state(resource) {
            let external = self.external_primary_sidebar(cx);
            self.tab_container.update(cx, |tabs, cx| {
                tabs.prepare_external_primary_sidebar(external, cx)
            });
            self.set_connection_sidebar_expanded(expanded, cx);
            return;
        }
        self.persist_resource_sidebar_expanded(resource, expanded, cx);
    }

    /// 只翻转当前资源栏的展开位：其它资源栏各自的偏好保持原样。
    fn persist_resource_sidebar_expanded(
        &mut self,
        resource: ResourceKind,
        expanded: bool,
        cx: &mut Context<Self>,
    ) {
        let key = crate::resource_mount::stable_resource_key(resource);
        if let Err(error) = AppSettings::try_update_and_save(cx, |settings| {
            settings
                .resource_sidebar_expanded
                .insert(key.to_owned(), expanded);
        }) {
            tracing::warn!(resource = key, %error, "资源栏展开状态保存失败，仅保留本次会话中的调整");
        }
        cx.notify();
    }

    fn persist_connection_sidebar_expanded(&mut self, expanded: bool, cx: &mut Context<Self>) {
        let external = self.external_primary_sidebar(cx);
        self.tab_container.update(cx, |tabs, cx| {
            tabs.prepare_external_primary_sidebar(external, cx);
            tabs.set_external_primary_sidebar(external, cx);
        });
        if AppSettings::try_update_and_save(cx, |settings| {
            settings.connection_sidebar_expanded = expanded;
        })
        .is_err()
        {
            self.connection_sidebar
                .update(cx, |sidebar, cx| sidebar.set_tree_expanded(!expanded, cx));
            return;
        }
        self.tab_container.update(cx, |tabs, cx| {
            tabs.set_navigation_sidebar_expanded(expanded, cx)
        });
        cx.notify();
    }

    /// 拖动资源栏：`position` 为光标位置，`initial` 是按下时的面板宽度。
    /// 拖动中只更新会话内覆盖值（外壳每帧据此求解宽度），松手才写设置并防抖落盘，
    /// 否则宽度要等 180ms 的保存防抖生效，面板就像「不会跟着动」。
    #[cfg(feature = "shell")]
    fn resize_application_resource(
        &mut self,
        kind: ResourceKind,
        position: Option<gpui::Point<gpui::Pixels>>,
        initial: gpui::Pixels,
        cx: &mut Context<Self>,
    ) {
        let Some(width) = crate::resource_mount::update_resource_resize_drag(
            &mut self.resource_resize_drag,
            position,
            initial,
        ) else {
            self.finish_application_resource_resize(kind, cx);
            return;
        };
        let width = crate::resource_mount::sanitize_resource_width(kind, f32::from(width));
        if kind == ResourceKind::Connections {
            self.connection_sidebar.update(cx, |tree, _| {
                tree.set_application_resource_width(gpui::px(width))
            });
        }
        let key = crate::resource_mount::stable_width_key(AppSettings::global(cx).app_layout, kind);
        self.resource_width_drag = Some((key, width));
        cx.notify();
    }

    /// 独立产品（不带 shell）没有资源拖动手柄；保留同名空实现让共享渲染代码编译。
    #[cfg(not(feature = "shell"))]
    fn resize_application_resource(
        &mut self,
        _kind: ResourceKind,
        _position: Option<gpui::Point<gpui::Pixels>>,
        _initial: gpui::Pixels,
        _cx: &mut Context<Self>,
    ) {
    }

    /// 松手后把拖动中的宽度写进设置（内存立即生效、磁盘保存防抖），再撤掉覆盖值。
    #[cfg(feature = "shell")]
    fn finish_application_resource_resize(&mut self, kind: ResourceKind, cx: &mut Context<Self>) {
        let Some((key, width)) = self.resource_width_drag.take() else {
            return;
        };
        AppSettings::update(cx, |settings| {
            settings.resource_widths.insert(key, width);
        });
        self.persist_application_resource_width(kind, width, cx);
        cx.notify();
    }

    /// 连续拖动只更新内存，停止后再原子保存；保存失败不改变持久化文件。
    #[cfg(feature = "shell")]
    fn persist_application_resource_width(
        &mut self,
        kind: ResourceKind,
        width: f32,
        cx: &mut Context<Self>,
    ) {
        // 键按拖动发生时的布局生成：应用菜单布局同样要记住宽度，不能只存图标布局。
        let key = crate::resource_mount::stable_width_key(AppSettings::global(cx).app_layout, kind);
        let task = cx.spawn(async move |_, cx| {
            smol::Timer::after(std::time::Duration::from_millis(180)).await;
            cx.update(|cx| {
                if let Err(error) = AppSettings::try_update_and_save(cx, |settings| {
                    settings.resource_widths.insert(key.clone(), width);
                }) {
                    tracing::warn!(resource = %key, %error, "资源栏宽度保存失败，仅保留本次会话中的调整");
                }
            });
        });
        self.resource_width_persist_task = Some(task);
    }

    #[cfg(feature = "shell")]
    fn sync_connection_sidebar_theme(&mut self, cx: &mut Context<Self>) {
        // 应用菜单布局下连接面板属于外壳，颜色已经由外壳 token 统一；再套终端主题的
        // 侧栏色会让面板变成另一个底色，和图标栏、标签栏对不上。
        if AppSettings::global(cx).app_layout == AppLayout::HomeNavigation {
            self.connection_sidebar
                .update(cx, |sidebar, cx| sidebar.set_terminal_palette(None, cx));
            return;
        }
        let terminal_active = {
            let tabs = self.tab_container.read(cx);
            if tabs.is_pinned_tab_active() {
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
        let palette = terminal_active
            .then(|| {
                app_kit::registry()
                    .shell_extras()
                    .and_then(|extras| extras.sidebar_colors(cx))
            })
            .flatten()
            .map(crate::persistent_connection_sidebar::SidebarPalette::from);
        self.connection_sidebar
            .update(cx, |sidebar, cx| sidebar.set_terminal_palette(palette, cx));
    }

    #[cfg(not(feature = "shell"))]
    fn sync_connection_sidebar_theme(&mut self, _cx: &mut Context<Self>) {}

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

    /// 关闭按钮的「最小化到托盘 / 退出应用」弹窗。
    ///
    /// 只在托盘可用且用户没有固化过偏好时打开；两条去向都复用既有流程
    /// （隐藏到托盘 / 退出确认），弹窗本身只负责让用户选一次。
    fn show_close_choice_prompt(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.close_choice_prompt_open {
            return;
        }
        self.close_choice_prompt_open = true;

        let remember = cx.new(|_| CloseChoiceRememberState { remember: false });
        let app_for_minimize = cx.entity().downgrade();
        let app_for_quit = cx.entity().downgrade();
        let app_for_close = cx.entity().downgrade();
        // 弹窗标题带产品名：三个产品同机运行时，光看窗口分不清是哪个应用。
        let app_name = app_kit::display_name();

        window.open_dialog(cx, move |dialog, _window, _cx| {
            let remember_for_minimize = remember.clone();
            let remember_for_quit = remember.clone();
            let app_for_minimize = app_for_minimize.clone();
            let app_for_quit = app_for_quit.clone();
            let app_for_close = app_for_close.clone();

            // 两条出路都自己渲染，不用默认的 ok/cancel footer：默认 footer 只有两个固定
            // 按钮，「取消」一旦被当成第二条出路，Esc 和点遮罩也会走成退出应用；并且
            // button_props 若在 confirm 之后设置，还会把 show_cancel 重置、吞掉第二个按钮。
            let minimize_button = Button::new("tray-close-minimize")
                .label(t!("Tray.close_minimize").to_string())
                .with_variant(ButtonVariant::Primary)
                .on_click(move |_, window, cx| {
                    let remember = remember_for_minimize.read(cx).remember;
                    window.close_dialog(cx);
                    let _ = app_for_minimize.update(cx, |app, cx| {
                        app.minimize_main_window_to_tray(remember, window, cx);
                    });
                });
            let quit_button = Button::new("tray-close-quit")
                .label(t!("Tray.close_quit").to_string())
                .on_click(move |_, window, cx| {
                    let remember = remember_for_quit.read(cx).remember;
                    window.close_dialog(cx);
                    let _ = app_for_quit.update(cx, |app, cx| {
                        app.quit_from_close_choice_prompt(remember, window, cx);
                    });
                });

            dialog
                .title(t!("Tray.close_title_app", app = app_name).to_string())
                .child(t!("Tray.close_message_app", app = app_name).to_string())
                .child(remember.clone())
                .footer(
                    DialogFooter::new()
                        .child(quit_button)
                        .child(minimize_button),
                )
                .on_close(move |_, _, cx| {
                    // Esc 或点遮罩只是关掉询问，不做任何动作；但不能留下「弹窗还开着」
                    // 的标记，否则下次点关闭按钮再也不弹。
                    let _ = app_for_close.update(cx, |app, _cx| {
                        app.close_choice_prompt_open = false;
                    });
                })
        });
    }

    /// 弹窗选择「最小化到托盘」。
    fn minimize_main_window_to_tray(
        &mut self,
        remember: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.close_choice_prompt_open = false;
        if remember {
            self.remember_close_button_behavior(CloseButtonBehavior::MinimizeToTray, cx);
        }
        self.hide_main_window_to_tray(window, cx);
    }

    /// 弹窗选择「退出应用」。
    fn quit_from_close_choice_prompt(
        &mut self,
        remember: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.close_choice_prompt_open = false;
        if remember {
            self.remember_close_button_behavior(CloseButtonBehavior::Quit, cx);
        }
        self.request_quit(window, cx);
    }

    fn remember_close_button_behavior(
        &mut self,
        behavior: CloseButtonBehavior,
        cx: &mut Context<Self>,
    ) {
        AppSettings::update_and_save(cx, |settings| {
            settings.close_button_behavior = behavior;
        });
    }

    /// 隐藏主窗口到托盘。
    ///
    /// 隐藏失败必须回退到退出确认：宁可直接退出，也不能留下一个用户找不到、
    /// 也没法恢复的隐藏窗口。
    fn hide_main_window_to_tray(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        match crate::window_visibility::hide_main_window(window) {
            // 冒烟与排障的唯一可观测点：窗口不可见之后，日志是唯一能证明
            // 「关闭按钮走的是托盘路径、而不是退出路径」的证据。
            Ok(()) => tracing::info!("主窗口已隐藏到系统托盘，进程继续在后台运行"),
            Err(error) => {
                tracing::warn!(%error, "隐藏主窗口失败，回退到退出确认");
                self.request_quit(window, cx);
            }
        }
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
            let app_name = app_kit::display_name();
            dialog
                .title(t!("Quit.confirm_title_app", app = app_name).to_string())
                .child(t!("Quit.confirm_message_app", app = app_name).to_string())
                .confirm()
                .button_props(
                    DialogButtonProps::default()
                        .ok_text(t!("Quit.confirm_action_app", app = app_name).to_string())
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
#[path = "visual_layout_tests.rs"]
mod visual_layout_tests;

#[cfg(test)]
mod tests {
    use super::{
        LOG_FILE_MAX_BYTES, LOG_FILE_NAME, RotatingLogWriter, close_active_window_default_shortcut,
        configured_log_file_path, default_log_file_path, initial_content_layout, log_file_appender,
    };
    use std::io::Write;

    /// gpui 的绑定优先级：**无 context 的绑定等价于最深的 context**，同深度
    /// 后注册者胜。`notes::init`（main.rs 里排在 `navop_app::init` 之后）把
    /// `secondary-/` 全局绑到了「切换源码模式」上，而 `navop_app::init` 里的
    /// `sql_editor_view::init` 排在它前面 —— 于是焦点落在 SQL 编辑器里按
    /// cmd+/ 时，先匹配到的是 notes 的动作，注释快捷键看起来「没绑上」。
    #[gpui::test]
    fn sql_editor_comment_shortcut_is_not_shadowed_by_notes_global_shortcut(
        cx: &mut gpui::TestAppContext,
    ) {
        let winner = cx.update(|cx| {
            gpui_component::init(cx);
            // 复刻真实启动顺序：navop_app::init（内含 sql_editor_view::init）→ notes::init
            db_view::sql_editor_view::init(cx);
            notes::init(cx);
            // main.rs 在同一位置再登记一次（见 sql_editor_keybindings_are_registered_after_notes_init）
            db_view::sql_editor_view::refresh_keybindings(cx);
            comment_shortcut_winner(cx)
        });

        assert!(
            winner.as_ref().is_some_and(|binding| binding
                .action()
                .partial_eq(&db_view::sql_editor_view::ToggleLineComment)),
            "焦点在 SQL 编辑器里按注释快捷键，第一个匹配的绑定必须是注释动作，实际是：{}",
            winner
                .map(|binding| binding.action().name().to_string())
                .unwrap_or_else(|| "<无绑定>".to_string())
        );
    }

    /// 焦点落在「SqlEditor > Input」时，注释快捷键的第一个匹配绑定。
    ///
    /// 直接向 keymap 要优先级列表：这比模拟按键先一步，能看出到底是谁赢了
    /// 同深度的平局（`bindings_for_input` 已按优先级排好序）。
    fn comment_shortcut_winner(cx: &gpui::App) -> Option<gpui::KeyBinding> {
        let contexts = vec![
            gpui::KeyContext::parse("Root").expect("valid context"),
            gpui::KeyContext::parse("SqlEditor").expect("valid context"),
            gpui::KeyContext::parse("Input").expect("valid context"),
        ];
        let keystroke = gpui::Keystroke::parse(comment_shortcut_spec()).expect("valid keystroke");
        let keymap = cx.key_bindings();
        let keymap = keymap.borrow();
        let (bindings, _) = keymap.bindings_for_input(&[keystroke], &contexts);
        bindings.first().cloned()
    }

    fn comment_shortcut_spec() -> &'static str {
        if cfg!(target_os = "macos") {
            "cmd-/"
        } else {
            "ctrl-/"
        }
    }

    /// 守卫：main.rs 里的登记顺序不能被改回去。
    ///
    /// 上面的行为用例只能证明「这个顺序下能赢」，证不了生产代码真用这个顺序（测试
    /// 是自己拼的调用）。这里直接盯 main.rs 的源码：`notes::init` 之后必须紧跟
    /// SQL 编辑器的重新登记。
    #[test]
    fn sql_editor_keybindings_are_registered_after_notes_init() {
        let source = include_str!("lib.rs");
        let notes_init = source.find("notes::init(cx);").expect("notes::init");
        let rebind = source
            .find("app_kit::refresh_domain_keybindings(cx);")
            .expect("领域快捷键重新登记");

        assert!(
            notes_init < rebind,
            "SQL 编辑器的快捷键必须在 notes::init 之后重新登记，否则注释快捷键会被 notes 的全局 secondary-/ 挤掉"
        );
    }

    #[test]
    fn initial_layout_pins_home_and_honors_the_startup_page_setting() {
        let layout = initial_content_layout();
        assert!(layout.pin_home);
        assert_eq!("home", layout.home_tab_id);
        assert_eq!(Some(0), layout.active_pinned_index);

        let source = include_str!("navop_app.rs");
        let constructor = source
            .split("pub fn new(window:")
            .nth(1)
            .and_then(|source| {
                source
                    .split("fn persist_connection_sidebar_expanded")
                    .next()
            })
            .expect("application constructor");
        assert!(constructor.contains("settings.startup_default_page"));
        assert!(constructor.contains("StartupDefaultPage::AiWorkbench"));
        assert!(constructor.contains("home.add_ai_workbench_tab(window, cx)"));
        assert!(!constructor.contains("DefaultAgentChatPanel::new_workbench"));
    }

    #[test]
    fn home_content_cannot_inherit_a_background_terminal_sidebar_theme() {
        let source = include_str!("navop_app.rs");
        assert!(source.contains("if tabs.is_pinned_tab_active()"));
    }

    #[test]
    fn home_is_a_noncloseable_tab_with_the_line_home_icon() {
        let source = include_str!("home_tab/home_layout.rs");
        assert!(source.contains("impl TabContent for HomePage"));
        assert!(source.contains("NAVOP_HOME_LINE_ICON"));
        assert!(source.contains("fn closeable"));
        assert!(source.contains("false"));
    }

    #[test]
    fn startup_master_key_prompt_is_scheduled_even_when_home_is_not_active() {
        let source = include_str!("navop_app.rs");
        let constructor = source
            .split("pub fn new(window:")
            .nth(1)
            .and_then(|source| {
                source
                    .split("\n    pub(crate) fn set_home_page_style")
                    .next()
            })
            .expect("NavopApp::new source");

        assert!(constructor.contains("home.show_pending_master_key_prompt(window, cx)"));
    }

    #[test]
    fn connection_and_terminal_shortcuts_cannot_replace_the_startup_lock_dialog() {
        let source = include_str!("navop_app.rs");
        let handlers = source
            .split("fn init_action_handlers(")
            .nth(1)
            .and_then(|source| source.split("\npub struct NavopApp").next())
            .expect("init_action_handlers source");

        assert_eq!(
            4,
            handlers
                .matches("home_page.read(cx).startup_master_key_lock_active(cx)")
                .count(),
            "连接、终端与 AI 供应商设置入口都必须先让启动主密钥锁对话框"
        );
    }

    #[test]
    fn platform_close_shortcut_uses_the_same_policy_for_main_and_auxiliary_windows() {
        let source = include_str!("navop_app.rs");
        let production = source
            .split("#[cfg(test)]")
            .next()
            .expect("production source before tests");
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
        assert!(close_handler.contains("cx.active_window()"));
        assert!(close_handler.contains("crate::app_init::main_window_handle()"));
        assert!(close_handler.contains("handle_main_window_close_request(window, cx)"));
        assert!(close_handler.contains("one_core::window_close::request_close_window"));
        assert!(!close_handler.contains("remote_file_editor"));
        assert!(!close_handler.contains("window.remove_window()"));
        assert_eq!(
            2,
            production
                .matches("handle_main_window_close_request(window, cx);")
                .count(),
            "系统关闭事件与快捷键必须共用同一个主窗口关闭策略"
        );
    }

    #[test]
    fn close_active_tab_shortcut_is_registered_and_avoids_single_ctrl_letter_defaults() {
        let source = include_str!("navop_app.rs");
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
        let close_tab_handler = source
            .split("fn close_active_tab(")
            .nth(1)
            .and_then(|source| source.split("\n}\n\n").next())
            .expect("close_active_tab source");

        assert!(keybindings.contains("action_id::APP_CLOSE_ACTIVE_TAB"));
        assert!(keybindings.contains("CloseActiveTab"));
        assert!(refreshable_keybindings.contains("action_id::APP_CLOSE_ACTIVE_TAB"));
        assert!(keybindings.contains("close_active_tab_default_shortcuts()"));
        assert!(refreshable_keybindings.contains("close_active_tab_default_shortcuts()"));
        assert!(close_tab_handler.contains("hp.close_active_tab(window, cx)"));
    }

    #[test]
    fn every_resource_sidebar_keeps_its_own_expanded_state() {
        let source = include_str!("navop_app.rs");
        let render = source
            .rsplit("impl Render for NavopApp")
            .next()
            .and_then(|render| render.split("#[cfg(test)]").next())
            .expect("外壳渲染代码");

        assert!(
            !render.contains("desired_visible: self.connection_sidebar.read(cx).is_expanded()"),
            "所有资源栏共用连接树的折叠位时，收起笔记目录会把数据库对象树一起收起来"
        );
        assert!(render.contains("desired_visible: self.resource_sidebar_expanded(resource, cx)"));

        let helper = source
            .split(
                "fn resource_sidebar_expanded(&self, resource: ResourceKind, cx: &App) -> bool {",
            )
            .nth(1)
            .and_then(|rest| rest.split("\n    }\n").next())
            .expect("资源栏展开状态的读取实现");
        assert!(helper.contains("resource_uses_connection_tree_state(resource)"));
        assert!(helper.contains("self.connection_sidebar.read(cx).is_expanded()"));
        assert!(helper.contains(".resource_sidebar_expanded"));
        assert!(helper.contains("stable_resource_key(resource)"));
        assert!(
            helper.contains(".unwrap_or(true)"),
            "没记录过的资源栏要默认展开，新资源不能一上来就是收起的"
        );

        let toggle = source
            .split("pub(crate) fn toggle_application_resource(")
            .nth(1)
            .and_then(|rest| rest.split("\n    }\n").next())
            .expect("资源栏开关实现");
        assert!(toggle.contains("let (resource, _, _) = self.active_resource(cx);"));
        assert!(toggle.contains("self.resource_sidebar_expanded(resource, cx)"));
        assert!(
            toggle.contains("self.set_active_resource_sidebar_expanded(expanded, cx)"),
            "开关必须按当前资源落盘，不能写死连接树"
        );

        let setter = source
            .split("pub(crate) fn set_active_resource_sidebar_expanded(")
            .nth(1)
            .and_then(|rest| rest.split("\n    }\n").next())
            .expect("按资源设置展开状态的实现");
        assert!(setter.contains("resource_uses_connection_tree_state(resource)"));
        assert!(setter.contains("self.set_connection_sidebar_expanded(expanded, cx)"));
        assert!(setter.contains("self.persist_resource_sidebar_expanded(resource, expanded, cx)"));

        // 标签栏上的资源栏开关也要按当前资源走：数据库页点开关收起的是对象树，
        // 一律交给连接树的话点了就没有任何反应。
        let toggled_subscription = source
            .split("TabContainerEvent::NavigationSidebarToggled { expanded } => {")
            .nth(1)
            .and_then(|rest| rest.split("\n                TabContainerEvent::").next())
            .expect("标签栏开关事件处理");
        assert!(
            toggled_subscription.contains("app.set_active_resource_sidebar_expanded(expanded, cx)")
        );
        assert!(
            !toggled_subscription.contains("app.set_connection_sidebar_expanded(expanded, cx)")
        );

        let persist = source
            .split("fn persist_resource_sidebar_expanded(")
            .nth(1)
            .and_then(|rest| rest.split("\n    }\n").next())
            .expect("资源栏展开状态的保存实现");
        assert!(persist.contains(".resource_sidebar_expanded"));
        assert!(persist.contains(".insert(key.to_owned(), expanded)"));
        assert!(
            !persist.contains("settings.connection_sidebar_expanded"),
            "非连接资源栏不能借连接树的折叠位落盘，否则它们的值还是同一个"
        );
    }

    #[test]
    fn notes_page_keeps_the_top_sidebar_toggle_when_the_shell_does_not_host_it() {
        let source = include_str!("navop_app.rs");
        let render = source
            .rsplit("impl Render for NavopApp")
            .next()
            .and_then(|render| render.split("#[cfg(test)]").next())
            .expect("外壳渲染代码");

        // 主页布局不托管笔记目录，求解出来的可见性必然是 Hidden；顶部开关要退回
        // 用页面自己的折叠位，否则笔记标签上这个按钮整块不可见。
        assert!(
            render.contains("let inline_notes_nav = (layout == AppLayout::HomeNavigation"),
            "主页布局的笔记目录留在页面里，顶部开关得单独取折叠位"
        );
        assert!(render.contains("resource == ResourceKind::Notes"));
        assert!(render.contains("plan.visibility == MountVisibility::Hidden"));
        assert!(render.contains("self.resource_sidebar_expanded(ResourceKind::Notes, cx)"));
        assert!(
            render.contains("let inline_navigation = if has_resource {"),
            "有面板托管时仍以求解出来的可见性为准"
        );
        assert!(render.contains("tabs.set_navigation_sidebar_toggle(inline_navigation, cx)"));
        assert!(
            render.contains("has_resource || inline_navigation.is_some()"),
            "开关按钮的可用性要包含页面内部托管的资源栏"
        );
        assert!(
            render.contains("notes.set_sidebar_collapsed(notes_collapsed, cx)"),
            "笔记页得跟着顶部开关收起/展开目录"
        );
        assert!(
            !render.contains("notes.set_sidebar_collapsed(!self."),
            "闭包里不能再去借 App，要在闭包外先算好折叠位"
        );
    }

    #[test]
    fn tree_auto_hide_selects_floating_overlay_otherwise_docked_panel() {
        let source = include_str!("navop_app.rs");
        let render = source.rsplit("impl Render for NavopApp").next().unwrap();
        assert!(render.contains("MountVisibility::Overlay"));
        assert!(render.contains("resource-overlay-backdrop"));
        assert!(render.contains("MountVisibility::Visible"));
        assert!(render.contains("plan_mount_with_navigation_width("));
        assert!(render.contains("resource_overlay_dismissed"));
        assert!(render.contains("let overlay_left = px(navigation_width)"));
        assert!(render.contains(".left(overlay_left)"));
        assert!(render.contains("set_external_sidebar(external"));
        assert!(render.contains("set_external_session_nav(external"));
        assert!(render.contains("settings.set_external_categories(false)"));
    }

    #[test]
    fn startup_preserves_the_saved_connection_sidebar_state() {
        let source = include_str!("navop_app.rs");
        let constructor = source
            .split("pub fn new(window:")
            .nth(1)
            .unwrap()
            .split("pub(crate) fn set_connection_sidebar_expanded")
            .next()
            .unwrap();
        assert!(
            constructor.contains(
                "let connection_sidebar_expanded = settings.connection_sidebar_expanded;"
            )
        );
        assert!(
            constructor.contains("with_navigation_sidebar_toggle(connection_sidebar_expanded)")
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
        let source = include_str!("navop_app.rs").replace("\r\n", "\n");
        let start = source.find("fn quit_app").expect("quit_app function");
        let end = source[start..]
            .find("\n}\n\nfn request_active_window_quit")
            .map(|offset| start + offset)
            .expect("quit_app function end");
        let quit_fn = &source[start..end];

        assert!(!quit_fn.contains("cx.quit()"));
        assert!(quit_fn.contains("request_active_window_quit(cx)"));
    }

    #[test]
    fn confirmed_and_update_quit_paths_await_all_application_resource_shutdown() {
        let source = include_str!("navop_app.rs").replace("\r\n", "\n");
        let helper_start = source
            .find("pub(crate) fn shutdown_application_resources_and_quit")
            .expect("shared application resource shutdown helper");
        let helper_end = source[helper_start..]
            .find("\n}\n\n#[derive(Clone, Copy")
            .map(|offset| helper_start + offset)
            .expect("shared application resource shutdown helper end");
        let helper = &source[helper_start..helper_end];
        // 领域资源（原生 RDP、SSH 传输等）由各自领域包关停，公共壳只负责
        // 在退出前按注册顺序 await 它们，然后再关停插件、托盘与热键。
        let collect_domains = helper
            .find("app_kit::shutdown_domains(cx)")
            .expect("collect domain shutdown tasks");
        let await_domains = helper
            .find("shutdown.await;")
            .expect("await domain shutdown tasks");
        let platform_quit = helper
            .find("cx.update(|cx| cx.quit())")
            .expect("platform quit");

        assert!(collect_domains < await_domains);
        assert!(await_domains < platform_quit);

        let confirm_start = source.find("fn confirm_quit").expect("confirm_quit");
        let confirm_end = source[confirm_start..]
            .find("\n    }\n}\n\n#[cfg(test)]")
            .map(|offset| confirm_start + offset)
            .expect("confirm_quit end");
        let confirm_quit = &source[confirm_start..confirm_end];
        assert!(confirm_quit.contains("shutdown_application_resources_and_quit"));
        assert!(!confirm_quit.contains("cx.quit()"));

        // 更新弹窗已迁到 `app_kit::update`：它必须走宿主持有的退出漏斗
        // （`app_kit::update::host::shutdown_application`，main 注册的 hook 正是
        // `shutdown_application_resources_and_quit`），而不是自己调 `cx.quit()`。
        let update_dialog = include_str!("../../../crates/app_kit/src/update/dialog.rs");
        assert!(update_dialog.contains("shutdown_application(cx"));
        assert!(!update_dialog.contains("cx.quit()"));

        let main_lib = include_str!("lib.rs").replace("\r\n", "\n");
        assert!(
            main_lib
                .contains("set_shutdown_hook(navop_app::shutdown_application_resources_and_quit)")
        );
    }

    /// 退出漏斗必须先把留在主线程 TLS 上的原生平台句柄释放掉，再调 `cx.quit()`。
    ///
    /// Linux(ksni) 的 `TrayIcon::drop` 会跑 `async_io::block_on()`，而 async-io 的线程
    /// 本地 `driver::CACHE` 注册得比 `TRAY_ICON` 晚 ⇒ 进程退出时反而先被销毁；
    /// 析构链走到 `TRAY_ICON` 时访问它就是 `AccessError` panic，而 TLS 析构器里不能
    /// panic，只能 abort（issue #336：退出流程全部正常，进程却以 SIGABRT 收尾并留 core）。
    #[test]
    fn quit_path_releases_platform_handles_before_quitting() {
        let source = include_str!("navop_app.rs").replace("\r\n", "\n");
        let helper_start = source
            .find("pub(crate) fn shutdown_application_resources_and_quit")
            .expect("shared application resource shutdown helper");
        let helper_end = source[helper_start..]
            .find("\n}\n\n#[derive(Clone, Copy")
            .map(|offset| helper_start + offset)
            .expect("shared application resource shutdown helper end");
        let helper = &source[helper_start..helper_end];

        let tray_release = helper
            .find("crate::system_tray::shutdown();")
            .expect("tray handle release");
        let hotkey_release = helper
            .find("crate::app_init::shutdown_system_hotkey();")
            .expect("global hotkey manager release");
        let platform_quit = helper
            .find("cx.update(|cx| cx.quit())")
            .expect("platform quit");

        assert!(
            tray_release < platform_quit,
            "托盘句柄必须在 cx.quit() 之前释放，否则退回 TLS 析构链就是 SIGABRT"
        );
        assert!(
            hotkey_release < platform_quit,
            "全局热键句柄必须在 cx.quit() 之前释放"
        );
    }

    #[test]
    fn navop_app_registers_window_close_guard() {
        let source = include_str!("navop_app.rs");
        let start = source.find("pub fn new").expect("NavopApp::new");
        let end = source[start..]
            .find("\n        let tab_container")
            .map(|offset| start + offset)
            .expect("NavopApp::new setup");
        let new_fn = &source[start..end];

        assert!(new_fn.contains("on_window_should_close"));
        assert!(new_fn.contains("handle_main_window_close_request(window, cx)"));
    }

    #[test]
    fn main_window_close_consults_the_system_tray_policy() {
        let source = include_str!("navop_app.rs");
        let start = source
            .find("fn handle_main_window_close_request")
            .expect("main window close handler");
        let end = source[start..]
            .find("\n}\n")
            .map(|offset| start + offset)
            .expect("main window close handler end");
        let close_fn = &source[start..end];
        // needle 运行时拼接：断言字面量不能把 include_str! 守卫自己命中。
        let policy_fn = ["crate::system_tray::main_window_close_", "action("].concat();
        let availability_fn = ["crate::system_tray::is_", "available()"].concat();

        assert!(
            contains_code(close_fn, &policy_fn),
            "关闭按钮必须按托盘可用性分流"
        );
        assert!(contains_code(close_fn, &availability_fn));
        assert!(contains_code(close_fn, "MainWindowCloseAction::AskUser"));
        assert!(contains_code(close_fn, "show_close_choice_prompt"));
        assert!(contains_code(close_fn, "MainWindowCloseAction::HideToTray"));
        assert!(contains_code(close_fn, "hide_main_window_to_tray"));
        assert!(contains_code(
            close_fn,
            "MainWindowCloseAction::RequestQuit"
        ));
        assert!(contains_code(close_fn, "request_window_quit(window, cx)"));
    }

    /// 弹窗必须真的给用户两条出路（最小化 / 退出），并且「记住我的选择」写回设置；
    /// 少了任一条，用户点关闭按钮就会被卡在一个只有默认按钮的弹窗里。
    #[test]
    fn close_choice_prompt_offers_both_destinations_and_remembers_the_choice() {
        let source = include_str!("navop_app.rs");
        let prompt = fn_body(source, "fn show_close_choice_prompt");
        let remember_label = ["Tray.close_", "remember"].concat();
        let minimize_label = ["Tray.close_", "minimize"].concat();
        let quit_label = ["Tray.close_", "quit"].concat();

        // 两条出路必须是各自独立的按钮：默认 footer 只有 ok/cancel，
        // button_props 跟在 confirm 后面还会把 show_cancel 重置掉、吞掉第二个按钮，
        // 所以这里禁止回退到默认 footer。
        assert!(contains_code(prompt, "DialogFooter::new()"));
        assert!(contains_code(
            prompt,
            r#"Button::new("tray-close-minimize")"#
        ));
        assert!(contains_code(prompt, r#"Button::new("tray-close-quit")"#));
        assert!(!contains_code(prompt, ".confirm()"));
        assert!(!contains_code(prompt, ".button_props("));
        assert!(contains_code(prompt, &minimize_label));
        assert!(contains_code(prompt, &quit_label));
        assert!(contains_code(prompt, "window.close_dialog(cx)"));
        assert!(contains_code(prompt, "CloseChoiceRememberState"));
        assert!(contains_code(prompt, "minimize_main_window_to_tray"));
        assert!(contains_code(prompt, "quit_from_close_choice_prompt"));
        // 连点关闭按钮不能叠出第二个弹窗。
        assert!(contains_code(prompt, "if self.close_choice_prompt_open"));

        // 「记住我的选择」勾选框自带实体，标签与受控回写都在它的 render 里。
        let checkbox = fn_body(source, "impl Render for CloseChoiceRememberState");
        assert!(contains_code(checkbox, &remember_label));
        assert!(contains_code(checkbox, "Checkbox::new"));
        assert!(contains_code(checkbox, "cx.listener"));

        let minimize = fn_body(source, "fn minimize_main_window_to_tray");
        assert!(contains_code(
            minimize,
            "CloseButtonBehavior::MinimizeToTray"
        ));
        let quit = fn_body(source, "fn quit_from_close_choice_prompt");
        assert!(contains_code(quit, "CloseButtonBehavior::Quit"));
        // 记住选择必须落盘，否则下次启动又被问一遍。
        let remember = fn_body(source, "fn remember_close_button_behavior");
        assert!(contains_code(remember, "AppSettings::update_and_save"));
        assert!(contains_code(
            remember,
            "settings.close_button_behavior = behavior"
        ));
    }

    /// 隐藏失败必须回退到退出确认，不能留下一个用户找不到、也没法恢复的窗口。
    #[test]
    fn hiding_to_the_tray_falls_back_to_quit_when_it_fails() {
        let source = include_str!("navop_app.rs");
        let body = fn_body(source, "fn hide_main_window_to_tray");
        let hide = ["crate::window_visibility::", "hide_main_window(window)"].concat();

        assert!(contains_code(body, &hide));
        assert!(contains_code(body, "Err(error) =>"));
        assert!(contains_code(body, "self.request_quit(window, cx)"));
    }

    /// 取一个顶层方法的完整函数体（从签名到下一个同缩进的 `\n    }`）。
    fn fn_body<'a>(source: &'a str, signature: &str) -> &'a str {
        let start = source
            .find(signature)
            .unwrap_or_else(|| panic!("{signature}"));
        let end = source[start..]
            .find("\n    }\n")
            .map(|offset| start + offset)
            .unwrap_or_else(|| panic!("{signature} end"));
        &source[start..end]
    }

    /// 源码守卫比对用：两侧都去掉全部空白再比对，避免 rustfmt 折行或补空格让断言
    /// 静默失效（`(window, cx)` 这类参数列表也会被 rustfmt 重新折行）。
    fn contains_code(source: &str, needle: &str) -> bool {
        fn compact(text: &str) -> String {
            text.split_whitespace().collect()
        }
        compact(source).contains(&compact(needle))
    }

    #[test]
    fn explicit_quit_still_reaches_the_main_window_after_hiding_to_the_tray() {
        let source = include_str!("navop_app.rs");
        let start = source
            .find("fn request_active_window_quit")
            .expect("request_active_window_quit");
        let end = source[start..]
            .find("\n/// 走既有退出流程的公共入口")
            .map(|offset| start + offset)
            .expect("request_active_window_quit end");
        let body = &source[start..end];
        let fallback = [
            "cx.active_window().or_else(crate::app_init::",
            "main_window_handle)",
        ]
        .concat();

        assert!(
            contains_code(body, &fallback),
            "隐藏到托盘后没有 key window，显式退出必须回退到主窗口而不是直接拆资源"
        );
        assert!(contains_code(body, "request_window_quit(window, cx)"));
    }

    #[test]
    fn navop_app_persists_window_state_after_bounds_changes() {
        let source = include_str!("navop_app.rs");
        let start = source.find("pub fn new").expect("NavopApp::new");
        let end = source[start..]
            .find("\n        let settings")
            .map(|offset| start + offset)
            .expect("NavopApp::new window setup");
        let new_fn = &source[start..end];

        assert!(new_fn.contains("observe_window_bounds"));
        assert!(new_fn.contains("save_main_window_state(window, cx)"));
    }

    #[test]
    fn saved_main_window_state_includes_position_size_and_display() {
        let source = include_str!("navop_app.rs");
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
        let source = include_str!("navop_app.rs");
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
    fn domain_init_precedes_public_mcp_init() {
        let source = include_str!("navop_app.rs");
        let domains = source.find("app_kit::init_domains(cx)?;").unwrap();
        let public_mcp = source.find("crate::public_mcp_runtime::init(cx);").unwrap();

        // 领域初始化里包含 Mongo 原生驱动工厂（`data_domain` 负责），必须早于
        // 公共 MCP 工具注册表，否则资源池会拿到内嵌驱动。
        assert!(domains < public_mcp);
    }

    #[test]
    fn window_transparency_does_not_fade_the_main_window_content() {
        let source = include_str!("navop_app.rs");
        let whole_tree_opacity = [".opacity(AppSettings::global(cx).", "window_opacity)"].concat();

        assert!(
            !contains_code(source, &whole_tree_opacity),
            "window opacity must only affect the background; applying it to the main window root also fades text and controls"
        );
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
        let path = configured_log_file_path("  /tmp/navop.log  ").expect("应返回日志路径");
        assert_eq!(path, std::path::PathBuf::from("/tmp/navop.log"));
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

        assert_eq!(path, std::path::PathBuf::from(r"D:\Navop\logs\navop.log"));
    }

    #[test]
    fn log_file_appender_creates_parent_directories_and_appends() {
        let path = std::env::temp_dir()
            .join(format!("navop-log-test-{}", std::process::id()))
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

    #[test]
    fn log_file_appender_rotates_oversized_log() {
        let dir =
            std::env::temp_dir().join(format!("onetcli-log-rotate-test-{}", std::process::id()));
        let path = dir.join("app.log");
        std::fs::create_dir_all(&dir).expect("应创建测试目录");

        // 用 set_len 撑到上限，避免真的写 64MB。
        let oversized = std::fs::File::create(&path).expect("应创建测试日志");
        oversized
            .set_len(LOG_FILE_MAX_BYTES)
            .expect("应把测试日志撑到大小上限");
        drop(oversized);

        let _file = log_file_appender(&path).expect("应轮转后重新创建日志文件");

        let rotated = dir.join("app.log.1");
        assert_eq!(
            LOG_FILE_MAX_BYTES,
            std::fs::metadata(&rotated)
                .expect("应保留轮转后的历史日志")
                .len()
        );
        assert_eq!(
            0,
            std::fs::metadata(&path).expect("应新建空日志").len(),
            "轮转后本次运行写入的应是干净文件"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn log_file_appender_keeps_undersized_log_in_place() {
        let dir =
            std::env::temp_dir().join(format!("onetcli-log-keep-test-{}", std::process::id()));
        let path = dir.join("app.log");
        std::fs::create_dir_all(&dir).expect("应创建测试目录");
        std::fs::write(&path, "0123456789\n").expect("应写入测试日志");

        let _file = log_file_appender(&path).expect("应追加到原日志");

        assert_eq!(
            "0123456789\n",
            std::fs::read_to_string(&path).expect("应读取日志")
        );
        assert!(
            !dir.join("app.log.1").exists(),
            "未达上限不得轮转，否则会丢日志"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn rotating_log_writer_rotates_when_the_running_file_grows_past_the_limit() {
        let dir = std::env::temp_dir().join(format!(
            "onetcli-log-runtime-rotate-test-{}",
            std::process::id()
        ));
        let path = dir.join("app.log");
        std::fs::create_dir_all(&dir).expect("应创建测试目录");

        let mut writer = RotatingLogWriter::open_with_limit(&path, 16).expect("应创建日志写入器");
        writer.write_all(b"first-chunk").expect("应写入第一块");
        assert!(!dir.join("app.log.1").exists(), "未达上限不得在运行期轮转");

        writer.write_all(b"-second").expect("应写入第二块");
        writer.flush().expect("应落盘");

        assert_eq!(
            std::fs::read_to_string(&path).expect("应读取当前日志"),
            "-second",
            "超过上限后旧内容应被轮转出当前文件"
        );
        assert_eq!(
            std::fs::read_to_string(dir.join("app.log.1")).expect("应读取轮转日志"),
            "first-chunk"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn rotating_log_writer_keeps_one_oversized_record_without_looping() {
        let dir = std::env::temp_dir().join(format!(
            "onetcli-log-oversized-record-test-{}",
            std::process::id()
        ));
        let path = dir.join("app.log");
        std::fs::create_dir_all(&dir).expect("应创建测试目录");

        let mut writer = RotatingLogWriter::open_with_limit(&path, 8).expect("应创建日志写入器");
        let record = vec![b'x'; 64];
        writer.write_all(&record).expect("应写入超长单条日志");
        writer.flush().expect("应落盘");

        assert_eq!(
            std::fs::metadata(&path).expect("应读取日志元数据").len(),
            record.len() as u64,
            "单条日志本身超上限时不能反复轮转，只能让它落盘"
        );
        assert!(!dir.join("app.log.1").exists());

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn rotating_log_writer_keeps_bounded_disk_usage_across_rotations() {
        let dir = std::env::temp_dir().join(format!(
            "onetcli-log-bounded-disk-test-{}",
            std::process::id()
        ));
        let path = dir.join("app.log");
        std::fs::create_dir_all(&dir).expect("应创建测试目录");

        let mut writer = RotatingLogWriter::open_with_limit(&path, 16).expect("应创建日志写入器");
        for chunk in [b"aaaaaaaaaaaa".as_slice(), b"bbbbbbbbbbbb", b"cccccccccccc"] {
            writer.write_all(chunk).expect("应写入日志块");
        }
        writer.flush().expect("应落盘");

        assert_eq!(
            std::fs::read_to_string(&path).expect("应读取当前日志"),
            "cccccccccccc"
        );
        assert_eq!(
            std::fs::read_to_string(dir.join("app.log.1")).expect("应读取轮转日志"),
            "bbbbbbbbbbbb",
            "只保留最近一份历史日志，磁盘占用才能稳定在 2 × 上限"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }
}

impl Render for NavopApp {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let sheet_layer = render_sheet_layer(window, cx);
        let dialog_layer = render_dialog_layer(window, cx);
        let notification_layer = render_notification_layer(window, cx);
        let main_content = self.tab_container.clone();
        let layout = AppSettings::global(cx).app_layout;
        let (active_key, resource_kind, _) = self.active_resource_metadata(cx);
        let chrome_palette = crate::chrome_palette::chrome_palette(cx);
        // A 是跨页图标导航；B 的应用菜单只在主页显示。
        // 离开主页后不再保留左侧菜单，也不占用工作区宽度。
        let global_navigation = (layout == AppLayout::GlobalNavigation
            || active_key == Some("Home"))
        .then(|| self.render_global_navigation(active_key, window, cx))
        .flatten();
        // 两种布局共用外壳，只有 A 需要额外的系统红绿灯专属区域。
        let global_navigation = global_navigation.map(|navigation| {
            let navigation_selector = if layout == AppLayout::GlobalNavigation {
                "application-navigation"
            } else {
                "application-navigation-shell"
            };
            gpui_component::v_flex()
                .debug_selector(move || navigation_selector.to_owned())
                .bg(if layout == AppLayout::GlobalNavigation {
                    chrome_palette.rail
                } else {
                    chrome_palette.menu
                })
                .when(layout == AppLayout::GlobalNavigation, |rail| {
                    rail.w(px(52.)).h_full()
                })
                .when(layout == AppLayout::HomeNavigation, |menu| menu.h_full())
                .flex_shrink_0()
                .when(layout == AppLayout::GlobalNavigation, |rail| {
                    rail.child(
                        div()
                            .id("macos-navigation-traffic-light-inset")
                            .debug_selector(|| "macos-navigation-traffic-light-inset".to_owned())
                            .w_full()
                            .h(one_ui::theme_geometry().layout.title_bar)
                            .flex_shrink_0()
                            .window_control_area(gpui::WindowControlArea::Drag)
                            .bg(chrome_palette.rail),
                    )
                })
                // 主页布局的 Tab 栏贯通整个窗口，红绿灯避让已经在 Tab 栏内部
                // （`set_macos_titlebar_inset`）预留过；侧栏列再从下方让出一层
                // title_bar 高度，等于把品牌区和导航整段往下推了 38px，顶部
                // 只剩一片空白。这里不再重复留白，侧栏直接贴着 Tab 栏开始。
                .child(navigation)
                .into_any_element()
        });
        let (resource, is_ssh_workspace, is_local) = resource_kind
            .map(|resource| (resource, false, false))
            .unwrap_or_else(|| self.active_resource(cx));
        let declaration = ResourceDeclaration {
            resource,
            is_ssh_workspace,
            is_local,
        };
        let navigation_width = if global_navigation.is_none() {
            0.
        } else if layout == AppLayout::HomeNavigation
            && cx
                .try_global::<GlobalHomePage>()
                .is_some_and(|home| home.home_page.read(cx).home_sidebar_collapsed())
        {
            58.
        } else if layout == AppLayout::HomeNavigation {
            184.
        } else {
            52.
        };
        let settings = AppSettings::global(cx);
        // 拖动期间用会话内覆盖值，松手后才回落到设置；否则宽度要等保存防抖生效才动。
        let resource_width_key = crate::resource_mount::stable_width_key(layout, resource);
        let persisted_resource_width = self
            .resource_width_drag
            .as_ref()
            .filter(|(key, _)| *key == resource_width_key)
            .map(|(_, width)| *width)
            .or_else(|| {
                settings
                    .resource_widths
                    .get(resource_width_key.as_str())
                    .copied()
            });
        let plan = plan_mount_with_navigation_width(
            layout,
            &declaration,
            f32::from(window.viewport_size().width),
            navigation_width,
            &MountPreferences {
                desired_visible: self.resource_sidebar_expanded(resource, cx),
                width: persisted_resource_width,
            },
        );
        // 「浮动侧边栏」开关：连接树浮在内容之上、不占布局宽度；点内容区或
        // 打开连接后自动收起。其它资源仍是停靠列，浮动与否取决于宽度预算。
        // 两种外壳布局都支持浮动：图标栏布局同样的位置也在 workspace 左缘，
        // 只是左偏移换成了 52px 的图标栏，卡片位置由 navigation_width 统一给出。
        let floating_connection = resource == ResourceKind::Connections
            && self.connection_sidebar.read(cx).is_auto_hide_tree()
            && plan.visibility == MountVisibility::Visible;
        let plan = if floating_connection {
            crate::resource_mount::ResourceMountPlan {
                visibility: MountVisibility::Overlay,
                slot: crate::resource_mount::MountSlot::Overlay,
                ..plan
            }
        } else {
            plan
        };
        let resource_visible = plan.visibility == MountVisibility::Visible
            || (plan.visibility == MountVisibility::Overlay && !self.resource_overlay_dismissed);
        // 业务页的宿主模式要在外壳读取 sidebar_contributions 之前确定，
        // 否则数据库页会先画自己的内嵌树，下一帧才被外壳接管。
        let external = self.external_primary_sidebar(cx);
        self.tab_container.update(cx, |tabs, cx| {
            tabs.set_external_primary_sidebar(external, cx)
        });
        // 应用菜单与 Tab 底层分区，活动 Tab 才与 Content 使用相同内容表面。
        self.tab_container.update(cx, |tabs, cx| {
            tabs.set_tab_bar_bg_color(chrome_palette.tab_bar, cx);
            tabs.set_active_tab_bg_color(chrome_palette.content, cx);
            // 非活动 Tab 铺一层画布色：壳色 Tab 栏上不再是一排看不见的文字，
            // 悬停时再抬到内容表面，等于预览「点下去会变成什么样」。
            tabs.set_inactive_tab_bg_color(cx.theme().muted, cx);
            tabs.set_inactive_tab_hover_color(cx.theme().background, cx);
            tabs.set_inactive_tab_border_color(cx.theme().border, cx);
        });
        let mut resource_element = (!floating_connection)
            .then(|| {
                plan.width.and_then(|width| {
                    let rendered = resource_visible
                        .then(|| self.render_application_resource(resource, px(width), window, cx))
                        .flatten()?;
                    if plan.width.is_some() {
                        let app = cx.entity();
                        // 拖动条压在面板右缘上（不占布局宽度）：面板和内容直接相接，
                        // 分隔线落在面板边框这一像素上，不会在两者之间露出一条缝。
                        let resize = one_ui::resize_handle::resize_handle::<
                            one_ui::resize_handle::ResizePanel,
                            Self,
                        >(
                            crate::resource_mount::stable_width_key(layout, resource),
                            gpui::Axis::Horizontal,
                        )
                        .placement(one_ui::resize_handle::HandlePlacement::Left)
                        .idle_line_color(gpui::transparent_black())
                        .on_resize(move |position, _, cx| {
                            app.update(cx, |app, cx| {
                                app.resize_application_resource(
                                    resource,
                                    position,
                                    gpui::px(width),
                                    cx,
                                );
                            });
                        });
                        Some(
                            gpui::div()
                                .relative()
                                .flex_shrink_0()
                                .h_full()
                                .w(px(width))
                                .child(rendered)
                                .child(resize)
                                .into_any_element(),
                        )
                    } else {
                        Some(rendered)
                    }
                })
            })
            .flatten();
        // 按钮是否可用只看资源和宿主，不看当前是否展开；折叠后仍能再次展开。
        let has_resource = matches!(
            plan.visibility,
            MountVisibility::Visible | MountVisibility::Collapsed | MountVisibility::Overlay
        );
        // 主页布局里笔记目录留在页面内部，外壳不接管这块面板（`base_slot` 返回
        // 空），于是顶部开关既没有面板可管、笔记页自己又不再画折叠把手，目录就
        // 收不起来了。这里让顶部开关继续表达「笔记目录是否展开」，状态仍存在
        // 按资源分开的那张表里，笔记页每帧把自己的目录跟着它同步。
        let inline_notes_nav = (layout == AppLayout::HomeNavigation
            && resource == ResourceKind::Notes
            && plan.visibility == MountVisibility::Hidden)
            .then(|| self.resource_sidebar_expanded(ResourceKind::Notes, cx));
        // 宿主模式在上面已经同步过，这里直接沿用，避免同一帧两套判定。
        // 主页标签下，标签栏的开关位用来收起/展开左侧应用菜单；
        // 其他标签仍交回资源栏，避免两个折叠对象争同一个按钮。
        let home_menu_expanded = (layout == AppLayout::HomeNavigation
            && active_key == Some("Home"))
        .then(|| {
            cx.try_global::<GlobalHomePage>()
                .map(|home| !home.home_page.read(cx).home_sidebar_collapsed())
        })
        .flatten();
        // AI 工作台标签下，同一个开关位改驱动工作台自己的会话导航栏；
        // 外部托管（经典图标布局）时导航栏在外壳里，仍走资源栏通道。
        let workbench_nav = (layout == AppLayout::HomeNavigation
            && active_key == Some("AiWorkbench")
            && !self.external_primary_sidebar(cx))
        .then(|| {
            self.tab_container
                .read(cx)
                .active_content_snapshot()
                .and_then(|content| {
                    content
                        .view()
                        .downcast::<ai_chat_view::WorkbenchShell>()
                        .ok()
                        .map(|ai| !ai.read(cx).session_nav_collapsed())
                })
        })
        .flatten();
        // 外壳接管面板时用求解出来的可见性；面板留在页面里时用页面自己的
        // 折叠位，否则这一页的顶部开关会整块不可见。
        let inline_navigation = if has_resource {
            Some(resource_visible)
        } else {
            inline_notes_nav
        };
        self.tab_container.update(cx, |tabs, cx| {
            tabs.set_resource_sidebar_toggle_visible(
                has_resource || inline_navigation.is_some(),
                cx,
            );
            tabs.set_navigation_sidebar_toggle(inline_navigation, cx);
            tabs.set_home_menu_toggle(home_menu_expanded, cx);
            tabs.set_workbench_nav_toggle(workbench_nav, cx);
        });
        // 同一 TabContainer 只保留一份；外壳分别调用其 Tab 栏与内容 renderer。
        // 不把整个容器放进上下两个位置，避免重复 Tab 栏和重复内容。
        main_content.update(cx, |tabs, cx| tabs.set_tab_content_visible(true, cx));
        let tab_content = main_content.update(cx, |tabs, cx| {
            tabs.render_tab_content(window, cx).into_any_element()
        });
        let tab_bar = div()
            .id("application-tab-bar")
            .debug_selector(|| "application-tab-bar".to_owned())
            .flex_shrink_0()
            .top_0()
            .left_0()
            .right_0()
            .h(one_ui::theme_geometry().layout.tab_bar)
            .w_full()
            .child(main_content.update(cx, |tabs, cx| {
                tabs.render_tab_bar(window, cx).into_any_element()
            }))
            .into_any_element();
        let tab_content_host = div()
            .relative()
            .size_full()
            .min_w_0()
            .min_h_0()
            .overflow_hidden()
            .child(tab_content)
            .into_any_element();
        if let Some(content) = self.tab_container.read(cx).active_content_snapshot() {
            if let Ok(notes) = content.view().downcast::<notes::NotesView>() {
                // 目录宿主由外壳接管时页面不画侧栏；留在页面里时（主页布局）把
                // 顶部开关的折叠位同步过去，让同一个按钮在两种宿主下都有效。
                let notes_collapsed = !self.resource_sidebar_expanded(ResourceKind::Notes, cx);
                notes.update(cx, |notes, cx| {
                    // A 的目录宿主不随折叠改变；收起后由顶部按钮重新展开。
                    notes.set_external_sidebar(external);
                    notes.set_sidebar_collapsed(notes_collapsed, cx);
                    notes.set_external_sidebar_toggle_handler(
                        cx.try_global::<GlobalNavopApp>().map(|global| {
                            let app = global.app.clone();
                            std::rc::Rc::new(move |_: &gpui::Window, cx: &mut gpui::App| {
                                // 折叠动作会把「外部侧栏」状态转回当前内容视图本身；
                                // 在视图自己的更新栈里同步做这件事，等于重入更新同一个
                                // 实体，gpui 会直接 panic（鼠标抬起事件里还会 abort）。
                                // 推迟到本次事件处理结束之后再改。
                                let app = app.clone();
                                cx.defer(move |cx| {
                                    app.update(cx, |app, cx| app.toggle_application_resource(cx));
                                });
                            })
                                as std::rc::Rc<dyn Fn(&gpui::Window, &mut gpui::App)>
                        }),
                    );
                });
            }
            if let Ok(settings) = content
                .view()
                .downcast::<crate::setting_tab::SettingsPanel>()
            {
                // 设置页始终保留其原有分类栏，不借到应用外壳。
                settings.update(cx, |settings, _| {
                    settings.set_external_categories(false);
                });
            }
            if let Ok(ai) = content.view().downcast::<ai_chat_view::WorkbenchShell>() {
                ai.update(cx, |ai, cx| {
                    ai.set_external_session_nav(external && plan.width.is_some(), cx);
                    ai.set_external_session_nav_toggle_handler(
                        cx.try_global::<GlobalNavopApp>().map(|global| {
                            let app = global.app.clone();
                            std::rc::Rc::new(move |_: &gpui::Window, cx: &mut gpui::App| {
                                // 折叠动作会把「外部侧栏」状态转回当前内容视图本身；
                                // 在视图自己的更新栈里同步做这件事，等于重入更新同一个
                                // 实体，gpui 会直接 panic（鼠标抬起事件里还会 abort）。
                                // 推迟到本次事件处理结束之后再改。
                                let app = app.clone();
                                cx.defer(move |cx| {
                                    app.update(cx, |app, cx| app.toggle_application_resource(cx));
                                });
                            })
                                as std::rc::Rc<dyn Fn(&gpui::Window, &mut gpui::App)>
                        }),
                    );
                });
            }
        }
        #[cfg(target_os = "macos")]
        self.tab_container.update(cx, |tabs, cx| {
            // B 的 Tab 栏贯通整个窗口，左上角不再单独预留一层红绿灯区域。
            tabs.set_macos_titlebar_inset(
                layout == AppLayout::HomeNavigation || global_navigation.is_none(),
                cx,
            );
            tabs.set_left_padding(
                if global_navigation.is_some() {
                    gpui::px(12.)
                } else {
                    one_ui::theme_geometry()
                        .layout
                        .macos_title_bar_content_padding
                },
                cx,
            )
        });
        let overlay = (plan.visibility == MountVisibility::Overlay)
            .then(|| {
                if floating_connection {
                    Some(self.connection_sidebar.update(cx, |sidebar, cx| {
                        sidebar.render_floating_tree(px(navigation_width), window, cx)
                    }))
                } else {
                    resource_element.take()
                }
            })
            .flatten();
        let overlay_left = px(navigation_width);
        let (global_rail, home_menu) = if layout == AppLayout::GlobalNavigation {
            (global_navigation, None)
        } else {
            (None, global_navigation)
        };
        let mut embedded_tab_bar = None;
        let mut full_width_tab_bar = None;
        if layout == AppLayout::GlobalNavigation {
            embedded_tab_bar = Some(tab_bar);
        } else {
            full_width_tab_bar = Some(tab_bar);
        }
        gpui_component::v_flex()
            .size_full()
            .relative()
            .items_stretch()
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
                this.toggle_application_resource(cx);
            }))
            .bg(cx.theme().background)
            .when_some(global_rail, |layout, navigation| layout.child(navigation))
            .child(
                gpui_component::v_flex()
                    .absolute()
                    .top_0()
                    .right_0()
                    .bottom_0()
                    .left(if layout == AppLayout::GlobalNavigation {
                        px(52.)
                    } else {
                        px(0.)
                    })
                    .when(layout == AppLayout::HomeNavigation, |workspace| {
                        // 顶部 Tab 已在纵向根布局中占位，下方侧栏与 Content 从其底部开始。
                        workspace.top(one_ui::theme_geometry().layout.tab_bar)
                    })
                    .when_some(embedded_tab_bar, |host, tab_bar| {
                        host.child(
                            div()
                                .flex_shrink_0()
                                .h(one_ui::theme_geometry().layout.tab_bar)
                                .w_full()
                                .child(tab_bar),
                        )
                    })
                    .child(
                        gpui_component::h_flex()
                            .flex_1()
                            .min_w_0()
                            .min_h_0()
                            .when_some(home_menu, |host, menu| host.child(menu))
                            .when(plan.visibility == MountVisibility::Visible, |layout| {
                                layout.when_some(resource_element.take(), |layout, resource| {
                                    layout.child(resource)
                                })
                            })
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .min_h_0()
                                    .h_full()
                                    .child(tab_content_host),
                            ),
                    ),
            )
            .children(full_width_tab_bar)
            .when_some(overlay, |this, resource| {
                if floating_connection {
                    // 浮动的连接树：遮罩只盖内容区，点一下就把树收回去；
                    // 树本身是圆角卡片（见 render_floating_tree），不吃遮罩点击。
                    return this
                        .child(
                            div()
                                .id("resource-overlay-backdrop")
                                .absolute()
                                .top(one_ui::theme_geometry().layout.tab_bar)
                                .bottom_0()
                                .right_0()
                                .left(px(navigation_width))
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.set_connection_sidebar_expanded(false, cx);
                                })),
                        )
                        .child(resource);
                }
                this.child(
                    div()
                        .id("resource-overlay-backdrop")
                        .absolute()
                        .inset_0()
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.resource_overlay_dismissed = true;
                            cx.notify();
                        }))
                        .child(
                            div()
                                .absolute()
                                .top(one_ui::theme_geometry().layout.tab_bar)
                                .bottom_0()
                                .left(overlay_left)
                                .id("resource-overlay-content")
                                .on_click(|_, _, cx| cx.stop_propagation())
                                .child(resource),
                        ),
                )
            })
            .children(sheet_layer)
            .children(dialog_layer)
            .children(notification_layer)
    }
}
