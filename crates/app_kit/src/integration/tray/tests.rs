//! 托盘逻辑的守卫测试：纯函数行为 + 平台后端的关键约束（源码守卫）。
//!
//! 源码守卫刻意用 `include_str!` 指向真正实现所在文件，而不是 main 侧的 shim：
//! 搬走代码之后，断言必须还能盯住真实实现，不能在 shim 上空转。

use one_core::settings::CloseButtonBehavior;

use super::menu::{
    MENU_ID_NEW_CONNECTION, MENU_ID_OPEN_SETTINGS, MENU_ID_QUIT, MENU_ID_SESSION_PREFIX,
    MENU_ID_SHOW, TRAY_SESSION_LABEL_MAX_CHARS, TRAY_SESSION_LIMIT, command_for_menu_id,
    session_menu_entries,
};
use super::{MainWindowCloseAction, TrayCommand, main_window_close_action};

const TRAY_SOURCE: &str = include_str!("../tray.rs");
const COMMANDS_SOURCE: &str = include_str!("commands.rs");
const DESKTOP_SOURCE: &str = include_str!("desktop.rs");

#[test]
fn close_action_asks_the_user_until_a_choice_is_remembered() {
    assert_eq!(
        MainWindowCloseAction::AskUser,
        main_window_close_action(true, CloseButtonBehavior::Ask)
    );
    assert_eq!(
        MainWindowCloseAction::HideToTray,
        main_window_close_action(true, CloseButtonBehavior::MinimizeToTray)
    );
    assert_eq!(
        MainWindowCloseAction::RequestQuit,
        main_window_close_action(true, CloseButtonBehavior::Quit)
    );
}

#[test]
fn close_action_without_a_tray_never_hides_the_window() {
    for behavior in [
        CloseButtonBehavior::Ask,
        CloseButtonBehavior::MinimizeToTray,
        CloseButtonBehavior::Quit,
    ] {
        assert_eq!(
            MainWindowCloseAction::RequestQuit,
            main_window_close_action(false, behavior),
            "托盘不可用时任何偏好都不能隐藏窗口"
        );
    }
}

#[test]
fn menu_ids_map_to_the_minimal_command_set() {
    assert_eq!(
        Some(TrayCommand::ShowMainWindow),
        command_for_menu_id(MENU_ID_SHOW)
    );
    assert_eq!(
        Some(TrayCommand::QuitApplication),
        command_for_menu_id(MENU_ID_QUIT)
    );
    assert_eq!(
        Some(TrayCommand::NewConnection),
        command_for_menu_id(MENU_ID_NEW_CONNECTION)
    );
    assert_eq!(
        Some(TrayCommand::OpenSettings),
        command_for_menu_id(MENU_ID_OPEN_SETTINGS)
    );
    assert_eq!(None, command_for_menu_id("navop.tray.unknown"));
    assert_eq!(None, command_for_menu_id(""));
}

#[test]
fn session_menu_ids_round_trip_into_activation_commands() {
    assert_eq!(
        Some(TrayCommand::ActivateSession {
            id: "tab-42".to_owned()
        }),
        command_for_menu_id(&format!("{MENU_ID_SESSION_PREFIX}tab-42"))
    );
    // 前缀后面的 id 不能为空，否则「空会话」会变成可点击的幽灵菜单项。
    assert_eq!(None, command_for_menu_id(MENU_ID_SESSION_PREFIX));
}

#[test]
fn session_entries_are_capped_and_their_labels_are_flattened() {
    let sessions: Vec<(String, String)> = (0..TRAY_SESSION_LIMIT + 5)
        .map(|index| (format!("tab-{index}"), format!("会话 {index}")))
        .collect();
    let entries = session_menu_entries(&sessions);

    assert_eq!(TRAY_SESSION_LIMIT, entries.len());
    assert_eq!(format!("{MENU_ID_SESSION_PREFIX}tab-0"), entries[0].0);

    let long = "标".repeat(TRAY_SESSION_LABEL_MAX_CHARS + 10);
    let entries = session_menu_entries(&[("tab-long".to_owned(), long)]);
    assert_eq!(
        format!("{}…", "标".repeat(TRAY_SESSION_LABEL_MAX_CHARS)),
        entries[0].1
    );

    let entries = session_menu_entries(&[("tab-multi".to_owned(), "第一行\n第二行".to_owned())]);
    assert_eq!("第一行 第二行", entries[0].1);
}

#[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
#[test]
fn tray_menu_opens_on_left_click_and_no_longer_hijacks_it_for_the_window() {
    assert!(DESKTOP_SOURCE.contains(".with_menu_on_left_click(true)"));
    assert!(DESKTOP_SOURCE.contains(".with_menu_on_right_click(true)"));
    // 左键被菜单占用后不能再派发「显示主窗口」：Windows 的后端会在弹出菜单的同一
    // 次 WM_LBUTTONUP 里继续发点击事件，留着就会「菜单和窗口一起弹」。
    let click_mapper = ["fn command_for_tray_", "click"].concat();
    let click_handler = ["TrayIconEvent::set_event_", "handler"].concat();

    assert!(!DESKTOP_SOURCE.contains(&click_mapper));
    assert!(!DESKTOP_SOURCE.contains(&click_handler));
}

#[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
#[test]
fn embedded_brand_icon_decodes_to_a_square_rgba_sized_for_the_tray() {
    let edge = super::desktop::TRAY_ICON_EDGE;
    let (rgba, width, height) =
        super::desktop::decode_tray_icon(super::brand_icon_png()).expect("品牌图标必须可解码");

    assert_eq!(edge, width);
    assert_eq!(edge, height);
    assert_eq!((width * height * 4) as usize, rgba.len());
    assert!(
        rgba.chunks_exact(4).any(|pixel| pixel[3] > 0),
        "缩放后的托盘图标不能是全透明"
    );
}

#[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
#[test]
fn decode_rejects_payloads_that_are_not_images() {
    assert!(super::desktop::decode_tray_icon(b"not a png").is_err());
    assert!(super::desktop::decode_tray_icon(&[]).is_err());
}

#[test]
fn dispatcher_is_the_only_place_that_touches_the_command_channel() {
    let dispatch_fn = TRAY_SOURCE
        .split("fn dispatch(command: TrayCommand)")
        .nth(1)
        .and_then(|source| source.split("\n}").next())
        .expect("dispatch source");

    // 平台回调只允许入队；真正的 GPUI 操作必须在 commands::execute_command 里。
    assert!(dispatch_fn.contains("COMMAND_TX"));
    assert!(!dispatch_fn.contains("update_window"));
    assert!(!dispatch_fn.contains("Window"));
}

#[test]
fn tray_quit_reuses_the_existing_quit_request_instead_of_quitting_directly() {
    let execute_fn = COMMANDS_SOURCE
        .split("fn execute_command(command: TrayCommand")
        .nth(1)
        .and_then(|source| source.split("\n/// 恢复主窗口，拆成").next())
        .expect("execute_command source");
    let quit_arm = execute_fn
        .split("TrayCommand::QuitApplication => {")
        .nth(1)
        .and_then(|rest| rest.split("\n    }\n\n    Ok(())").next())
        .expect("QuitApplication arm");
    // needle 运行时拼接，避免断言字面量把 include_str! 守卫自己命中。
    let direct_quit = ["cx", ".quit()"].concat();

    assert!(quit_arm.contains("request_window_quit"));
    assert!(quit_arm.contains("restore_main_window"));
    assert!(
        !quit_arm.contains(&direct_quit),
        "托盘退出必须先恢复主窗口，再走既有确认与标签页关闭流程"
    );

    // 直接退出的唯一例外是「主窗口 handle 已失效」这条异常路径，
    // 正常托盘退出路径不得依赖它。
    assert_eq!(1, COMMANDS_SOURCE.matches(&direct_quit).count());
}

/// 回归守卫：原生 order-front / activate 必须在 GPUI 窗口借用**之外**执行。
///
/// 2026-09-17 真机实测：把 `makeKeyAndOrderFront:` + `NSApplication::activate` 放在
/// `cx.update_window(...)` 的借用里，AppKit 会同步回调 `gpui/src/window.rs` 的
/// `on_active_status_change` → `handle.update(...)`，抢同一个 App 借用失败，日志只留下
/// `ERROR gpui::window: RefCell already borrowed`（窗口先隐藏再恢复时才出现）。
///
/// 搬进 app_kit 之后，原生实现留在宿主注入的 [`super::TrayHost::show_main_window`]：
/// 它拿到 `&mut AsyncApp`，可以在自己的借用内取句柄、在借用外改原生状态。这里守卫的是
/// 「app_kit 不许在借用里自己做原生恢复」这条边界。
#[test]
fn native_restore_is_delegated_outside_the_gpui_window_borrow() {
    let restore_fn = COMMANDS_SOURCE
        .split("fn restore_main_window(")
        .nth(1)
        .and_then(|rest| rest.split("\n/// 按 id 激活标签").next())
        .expect("restore_main_window source");

    let host_branch = restore_fn
        .split("if let Some(show) = host().show_main_window {")
        .nth(1)
        .and_then(|rest| rest.split("\n    }").next())
        .expect("host show branch");
    assert!(host_branch.contains("show(handle, cx)"));
    assert!(
        !host_branch.contains("update_window"),
        "原生恢复必须整个交给宿主回调，不能在 app_kit 的窗口借用里做"
    );

    // 默认实现只做 GPUI 自己的延迟激活：平台层把它投递到前台执行器，本身不在借用内。
    let default_branch = restore_fn
        .split(".update_window(handle,")
        .nth(1)
        .and_then(|rest| rest.split("})").next())
        .expect("default update_window closure");
    assert!(default_branch.contains("activate_window"));
}

#[test]
fn tray_icon_is_kept_alive_on_the_creating_thread() {
    // TrayIcon 是 Rc<RefCell<..>>（!Send），只能线程本地持有，且必须真的存住。
    assert!(DESKTOP_SOURCE.contains("thread_local!"));
    assert!(DESKTOP_SOURCE.contains("static TRAY_ICON: RefCell<Option<tray_icon::TrayIcon>>"));
    assert!(DESKTOP_SOURCE.contains("*slot.borrow_mut() = Some(tray)"));
}

#[test]
fn tray_backends_avoid_the_gtk_and_appindicator_linux_dependencies() {
    let manifest = include_str!("../../../Cargo.toml");
    let linux_deps = manifest
        .split("[target.'cfg(target_os = \"linux\")'.dependencies]")
        .nth(1)
        .and_then(|rest| rest.split("\n[").next())
        .expect("linux target dependencies");
    // needle 运行时拼接：Cargo.toml 的注释里就写着这两个 crate 名。
    let appindicator_feature = ["libapp", "indicator"].concat();
    let xdo_feature = ["lib", "xdo"].concat();

    assert!(linux_deps.contains("tray-icon"));
    assert!(linux_deps.contains("features = [\"ksni\"]"));
    assert!(linux_deps.contains("default-features = false"));
    assert!(!linux_deps.contains(&appindicator_feature));
    assert!(!linux_deps.contains(&xdo_feature));
}

/// 退出守卫：托盘句柄必须在退出前从 `thread_local!` 里**取出来真的析构**。
///
/// 不能留给进程退出的 TLS 析构链：Linux(ksni) 的 `TrayIcon::drop` 会跑
/// `Handle::shutdown().wait()` → `async_io::block_on()`，而 async-io 的线程本地
/// `driver::CACHE` 是在同一次 `desktop::install()` 里、`TRAY_ICON` 之后才注册的，
/// TLS 析构逆序执行 ⇒ 退出时 `CACHE` 先没，`TRAY_ICON` 的析构器再去访问它就直接
/// `AccessError` panic，而 TLS 析构器里不能 panic，只能 abort（issue #336）。
#[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
#[test]
fn tray_handle_is_released_before_the_process_teardown_chain() {
    // 公开入口存在，并且会同时掐掉可用性标记：句柄交出去之后任何菜单操作都必须
    // 变成空操作，不能再去碰一个已经 drop 的托盘。
    let public = TRAY_SOURCE
        .split("pub fn shutdown() {")
        .nth(1)
        .and_then(|rest| rest.split("\n}").next())
        .expect("tray::shutdown source");
    assert!(public.contains("TRAY_READY.store(false"));

    // 后端实现必须把句柄 take 出来、在 `TRAY_ICON.with` 的借用之外析构。
    // needle 运行时拼接，避免 include_str! 守卫命中自己的断言文本。
    let release = ["slot.borrow_mut()", ".take()"].concat();
    let drop_outside = ["drop(tray", ");"].concat();
    let backend = DESKTOP_SOURCE
        .split("pub(crate) fn shutdown() {")
        .nth(1)
        .and_then(|rest| rest.split("\n    }").next())
        .expect("desktop tray shutdown source");

    assert!(backend.contains(&release));
    assert!(backend.contains(&drop_outside));
}
