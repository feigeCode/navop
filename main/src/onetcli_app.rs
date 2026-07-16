use crate::home_tab::{
    HomePage, NewConnectionShortcut, OpenConnectionQuickOpen, OpenLocalTerminalShortcut,
};
use gpui::{
    App, AppContext, Context, Entity, IntoElement, KeyBinding, Keystroke, ParentElement, Render,
    Styled, Window, actions, div,
};
use gpui_component::{WindowExt, dialog::DialogButtonProps, kbd::Kbd, notification::Notification};
use one_core::keybindings::{action_id, rebind_keybindings, shortcuts_for};
use raw_window_handle::HasWindowHandle;
#[cfg(any(target_os = "macos", target_os = "windows"))]
use raw_window_handle::RawWindowHandle;
use rust_i18n::t;
#[cfg(not(target_os = "macos"))]
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct InitialPinnedTabLayout {
    home_tab_id: &'static str,
    workbench_tab_id: &'static str,
    active_pinned_index: usize,
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

fn initial_home_tab_layout(startup_default_page: StartupDefaultPage) -> InitialPinnedTabLayout {
    InitialPinnedTabLayout {
        home_tab_id: "home",
        workbench_tab_id: "ai-workbench",
        active_pinned_index: active_pinned_index_for_startup_default_page(startup_default_page),
    }
}

fn active_pinned_index_for_startup_default_page(startup_default_page: StartupDefaultPage) -> usize {
    match startup_default_page {
        StartupDefaultPage::Home => 0,
        StartupDefaultPage::AiWorkbench => 1,
    }
}

#[cfg(target_os = "macos")]
use gpui::px;

use gpui_component::dock::{ClosePanel, ToggleZoom};
use gpui_component::{ActiveTheme, Root};
use one_core::llm::manager::GlobalProviderState;
use one_core::settings::{AppSettings, MainWindowSize, StartupDefaultPage};
use one_core::storage::manager::get_config_dir;
use one_core::tab_container::{TabContainer, TabContentRegistry, TabItem};
use one_core::tab_navigation::{
    ActiveTabSlot, TabCycleDirection, tab_number_target, tab_slot_after_cycle,
};
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
        cx.quit();
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
        cx.quit();
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

pub(crate) fn configured_log_file_path(value: &str) -> anyhow::Result<PathBuf> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        Ok(default_log_file_path()?)
    } else {
        Ok(PathBuf::from(trimmed))
    }
}

fn default_log_file_path() -> anyhow::Result<PathBuf> {
    Ok(get_config_dir()?.join("logs").join("onetcli.log"))
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

pub fn init(cx: &mut App) {
    gpui_component::init(cx);
    setting_tab::init_settings(cx);
    one_core::init(cx);
    ai_chat_view::init(cx);
    crate::public_mcp_approval::init(cx);
    crate::ai_chat_acp_approval::init(cx);
    crate::ai_chat_acp::init(cx);
    one_ui::init(cx);
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
    db::init_cache(cx);
    // 启动后台磁盘缓存清理任务
    if let Some(cache) = cx.try_global::<db::GlobalNodeCache>() {
        cache.start_cleanup_task(cx);
    }
    terminal_view::init(cx);
    redis_view::init(cx);
    crate::public_mcp_runtime::init(cx);
    crate::personal_sync_runtime::init(cx);
    mongodb_view::init(cx);
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
}

pub fn refresh_keybindings(cx: &mut App) {
    cx.bind_keys(refreshable_keybindings(cx));
    crate::home_tab::refresh_keybindings(cx);
    db_view::search_shortcut::refresh_keybindings(cx);
    db_view::sql_editor_view::refresh_keybindings(cx);
    terminal_view::refresh_keybindings(cx);
    redis_view::refresh_keybindings(cx);
    remote_desktop_view::refresh_keybindings(cx);
    one_ui::refresh_keybindings(cx);
    remote_file_editor::refresh_keybindings(cx);
    notes::refresh_keybindings(cx);
}

fn init_keybindings(cx: &App) -> Vec<KeyBinding> {
    let mut keybindings = vec![];
    keybindings.extend(
        shortcuts_for(cx, action_id::WINDOW_TOGGLE_ZOOM, &["shift-escape"])
            .into_iter()
            .map(|key| KeyBinding::new(&key, ToggleZoom, None)),
    );
    keybindings.extend(
        shortcuts_for(cx, action_id::WINDOW_CLOSE_PANEL, &["ctrl-w"])
            .into_iter()
            .map(|key| KeyBinding::new(&key, ClosePanel, None)),
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
        action_id::WINDOW_CLOSE_PANEL,
        &["ctrl-w"],
        None,
        ClosePanel,
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

pub struct OnetCliApp {
    tab_container: Entity<TabContainer>,
    quit_state: QuitRequestState,
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
                app.save_main_window_size(window, cx);
                app.request_quit(window, cx);
            });
            false
        });

        let tab_container = cx.new(|cx| {
            let mut container = TabContainer::new(window, cx)
                .with_tab_bar_colors(
                    Some(gpui::rgb(0x2b2b2b).into()),
                    Some(gpui::rgb(0x1e1e1e).into()),
                )
                .with_tab_item_colors(
                    Some(gpui::rgb(0x555555).into()),
                    Some(gpui::rgb(0x3a3a3a).into()),
                )
                .with_inactive_tab_bg_color(Some(gpui::rgb(0x3a3a3a).into()))
                .with_tab_content_colors(Some(gpui::white()), Some(gpui::rgb(0xaaaaaa).into()));

            #[cfg(target_os = "macos")]
            {
                container = container
                    .with_left_padding(px(80.0))
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
        // Initialize fixed tabs before the scrollable workspace tabs.
        {
            let layout = initial_home_tab_layout(AppSettings::current(cx).startup_default_page);
            let tab_container_clone = tab_container.clone();
            tab_container.update(cx, |tc, cx| {
                let home_page = cx.new(|cx| HomePage::new(tab_container_clone, window, cx));
                cx.set_global(GlobalHomePage {
                    home_page: home_page.clone(),
                });
                let home_tab = TabItem::new(layout.home_tab_id, "app", home_page);
                tc.add_pinned_tab(home_tab, cx);

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
                tc.activate_pinned_tab_at(layout.active_pinned_index, window, cx);
            });
        }

        Self {
            tab_container,
            quit_state: QuitRequestState::default(),
        }
    }

    fn save_main_window_size(&self, window: &Window, cx: &mut App) {
        let bounds = window.window_bounds().get_bounds();
        let width = f32::from(bounds.size.width);
        let height = f32::from(bounds.size.height);
        let Some(size) = MainWindowSize::new(width, height) else {
            return;
        };
        AppSettings::update_and_save(cx, |settings| settings.main_window_size = Some(size));
    }

    fn request_quit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
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
                .on_ok(move |_, window, cx| {
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
                    cx.quit();
                }
            });
        })
        .detach();
    }
}

#[cfg(test)]
mod tests {
    use super::{
        configured_log_file_path, default_log_file_path, initial_home_tab_layout, log_file_appender,
    };
    use one_core::settings::StartupDefaultPage;
    use std::io::Write;

    #[test]
    fn initial_layout_pins_home_and_ai_workbench_with_ai_active() {
        let layout = initial_home_tab_layout(StartupDefaultPage::AiWorkbench);

        assert_eq!("home", layout.home_tab_id);
        assert_eq!("ai-workbench", layout.workbench_tab_id);
        assert_eq!(1, layout.active_pinned_index);
    }

    #[test]
    fn initial_layout_uses_startup_default_page_for_active_pinned_tab() {
        let home_layout = initial_home_tab_layout(StartupDefaultPage::Home);
        let ai_layout = initial_home_tab_layout(StartupDefaultPage::AiWorkbench);

        assert_eq!(0, home_layout.active_pinned_index);
        assert_eq!(1, ai_layout.active_pinned_index);
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
        assert_eq!("⌃⌘T", super::shortcut_label("ctrl-cmd-t"));
    }

    #[test]
    fn quit_action_routes_through_active_window_quit_request() {
        let source = include_str!("onetcli_app.rs");
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

        div()
            .size_full()
            .relative()
            .bg(cx.theme().background)
            .child(div().size_full().child(self.tab_container.clone()))
            .children(sheet_layer)
            .children(dialog_layer)
            .children(notification_layer)
    }
}
