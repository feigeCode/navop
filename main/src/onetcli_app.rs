use crate::home_tab::{HomePage, NewConnectionShortcut, OpenConnectionQuickOpen};
use gpui::{
    App, AppContext, Context, Entity, IntoElement, KeyBinding, ParentElement, Render, Styled,
    Window, actions, div,
};
use gpui_component::WindowExt;
use one_core::keybindings::{action_id, rebind_keybindings, shortcuts_for};

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
        MinimizeWindow,
        DuplicateTab,
        QuitApp,
    ]
);

#[derive(Clone)]
pub struct GlobalTabContainer {
    pub tab_container: Entity<TabContainer>,
}

impl gpui::Global for GlobalTabContainer {}

#[derive(Clone)]
pub struct GlobalHomePage {
    pub home_page: Entity<HomePage>,
}

impl gpui::Global for GlobalHomePage {}

#[cfg(target_os = "macos")]
use gpui::px;

use gpui_component::dock::{ClosePanel, ToggleZoom};
use gpui_component::{ActiveTheme, Root};
use one_core::llm::manager::GlobalProviderState;
use one_core::settings::AppSettings;
use one_core::storage::manager::get_config_dir;
use one_core::tab_container::{TabContainer, TabContentRegistry, TabItem};
#[cfg(unix)]
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};

use crate::setting_tab;
use db::GlobalDbState;
use one_core::storage::{ConnectionRepository, GlobalStorageState};

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
                if number == 1 && tc.has_pinned_tab() {
                    tc.activate_pinned_tab(window, cx);
                    return;
                }

                let index = if tc.has_pinned_tab() {
                    number.saturating_sub(2)
                } else {
                    number.saturating_sub(1)
                };

                if index < tc.tabs().len() {
                    tc.set_active_index(index, window, cx);
                }
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
    cx.quit();
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
    setting_tab::init_settings(cx);
    gpui_component::init(cx);
    one_core::init(cx);
    one_ui::init(cx);
    db_view::sql_editor_view::init(cx);
    db_view::chatdb::agents::init(cx);
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
    mongodb_view::init(cx);
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
    db_view::sql_editor_view::refresh_keybindings(cx);
    terminal_view::refresh_keybindings(cx);
    redis_view::refresh_keybindings(cx);
    one_ui::refresh_keybindings(cx);
    remote_file_editor::refresh_keybindings(cx);
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
            action_id::APP_DUPLICATE_TAB,
            &[default_shortcut("cmd-shift-t", "alt-shift-t")],
        )
        .into_iter()
        .map(|key| KeyBinding::new(&key, DuplicateTab, None)),
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
    keybindings.extend(rebind_keybindings(
        cx,
        action_id::WINDOW_TOGGLE_FULLSCREEN,
        &[default_shortcut("ctrl-cmd-f", "alt-enter")],
        None,
        ToggleFullscreen,
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
    cx.on_action(|_: &DuplicateTab, cx| duplicate_tab(cx));
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
}

pub struct OnetCliApp {
    tab_container: Entity<TabContainer>,
}

impl OnetCliApp {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
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
                    .with_top_padding(px(4.0))
            }

            #[cfg(not(target_os = "macos"))]
            {
                container = container.with_window_controls(true)
            }

            container
        });

        cx.set_global(GlobalTabContainer {
            tab_container: tab_container.clone(),
        });
        // Set HomePage as the pinned tab (always visible, not scrollable)
        {
            let tab_container_clone = tab_container.clone();
            tab_container.update(cx, |tc, cx| {
                let home_page = cx.new(|cx| HomePage::new(tab_container_clone, window, cx));
                cx.set_global(GlobalHomePage {
                    home_page: home_page.clone(),
                });
                let home_tab = TabItem::new("home", "app", home_page);
                tc.set_pinned_tab(home_tab, cx);
                tc.activate_pinned_tab(window, cx);
            });
        }

        Self { tab_container }
    }
}

#[cfg(test)]
mod tests {
    use super::{configured_log_file_path, default_log_file_path, log_file_appender};
    use std::io::Write;

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
