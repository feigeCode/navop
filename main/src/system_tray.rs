//! 系统托盘：实现已下沉到 `app_kit::integration::tray`，全量产品与独立产品共用。
//!
//! main 侧只负责两件事：
//!
//! 1. 注入平台相关回调 —— 原生 order-front 必须发生在 GPUI 借用之外，只有 main
//!    知道 `window_visibility` 与 `navop_app` 的那套实现；
//! 2. 保持菜单 / 关闭按钮语义不变：`show_quick_actions(false)`，菜单仍是
//!    「会话 … → 显示 Navop → 退出 Navop」，关闭按钮行为仍由
//!    [`main_window_close_action`] 决定。
//!
//! 五个已经踩实的坑（菜单回调不碰 GPUI、恢复窗口必须在借用外、退出不能绕过既有
//! 退出确认……）连同它们的源码守卫测试都搬到了
//! `crates/app_kit/src/integration/tray.rs`：测试盯的是真实实现，而不是这个 shim。

use gpui::{AnyWindowHandle, App, AppContext as _, AsyncApp, Window};

pub(crate) use app_kit::integration::tray::{
    MainWindowCloseAction, is_available, main_window_close_action, shutdown, sync_sessions_from,
};

/// 初始化托盘。返回值表示是否真的装上了（无托盘环境返回 false，不致命）。
pub(crate) fn init(cx: &mut App) -> bool {
    app_kit::integration::tray::install(host(), config(), cx)
}

fn config() -> app_kit::integration::tray::TrayConfig {
    // 托盘文案跟随产品身份：三个产品同机运行时，菜单里必须能区分「这是哪个应用」。
    let display_name = app_kit::identity()
        .map(|identity| identity.display_name)
        .unwrap_or("Navop");
    app_kit::integration::tray::TrayConfig::new(display_name).show_quick_actions(false)
}

fn host() -> app_kit::integration::tray::TrayHost {
    app_kit::integration::tray::TrayHost {
        main_window: Some(crate::app_init::main_window_handle),
        show_main_window: Some(show_main_window),
        hide_main_window: Some(hide_main_window),
        activate_tab: Some(crate::navop_app::activate_tab_by_id),
        request_quit: Some(request_quit),
    }
}

fn show_main_window(handle: AnyWindowHandle, cx: &mut AsyncApp) -> anyhow::Result<()> {
    // 原生 order-front 必须在 GPUI 借用之外执行：借用内只 activate + 取目标句柄，
    // 真正的 `show_main_window` 放在借用结束之后。
    let target = cx.update_window(handle, |_, window, _| {
        window.activate_window();
        crate::window_visibility::main_window_target(window)
    })??;
    crate::window_visibility::show_main_window(target)
}

fn hide_main_window(window: &mut Window) -> anyhow::Result<()> {
    crate::window_visibility::hide_main_window(window)
}

fn request_quit(_handle: AnyWindowHandle, window: &mut Window, cx: &mut App) {
    // 退出确认原本就是「当前窗口」语义；句柄只用来确认主窗口还在（调用方已经保证）。
    crate::navop_app::request_window_quit(window, cx);
}

#[cfg(test)]
mod tests {
    const TRAY_SOURCE: &str = include_str!("../../crates/app_kit/src/integration/tray.rs");
    const COMMANDS_SOURCE: &str =
        include_str!("../../crates/app_kit/src/integration/tray/commands.rs");
    const SHIM_SOURCE: &str = include_str!("system_tray.rs");

    #[test]
    fn system_tray_backend_really_lives_in_app_kit() {
        // 守卫必须盯真实实现：搬走之后这些符号只能出现在 app_kit 里。
        for needle in [
            "pub fn install(",
            "pub fn is_available(",
            "pub fn sync_sessions_from(",
            "pub fn shutdown(",
            "pub use menu::main_window_close_action;",
        ] {
            assert!(
                TRAY_SOURCE.contains(needle),
                "app_kit 托盘实现缺少 {needle}"
            );
        }
        assert!(COMMANDS_SOURCE.contains("fn restore_main_window("));
        assert!(
            COMMANDS_SOURCE.contains("host().show_main_window"),
            "宿主注入的原生恢复实现必须被真正调用"
        );
    }

    #[test]
    fn system_tray_shim_injects_the_native_window_entry_points() {
        for needle in [
            "main_window: Some(crate::app_init::main_window_handle)",
            "show_main_window: Some(show_main_window)",
            "hide_main_window: Some(hide_main_window)",
            "activate_tab: Some(crate::navop_app::activate_tab_by_id)",
            "request_quit: Some(request_quit)",
        ] {
            assert!(SHIM_SOURCE.contains(needle), "shim 少了宿主注入 {needle}");
        }
        assert_eq!(
            SHIM_SOURCE
                .matches("crate::window_visibility::show_main_window(target)")
                .count(),
            2,
            "恢复窗口的父窗口句柄与目标句柄必须分开取"
        );
    }

    #[test]
    fn system_tray_menu_semantics_are_unchanged_for_the_full_product() {
        assert!(SHIM_SOURCE.contains(r#"TrayConfig::new("Navop").show_quick_actions(false)"#));
        assert!(
            SHIM_SOURCE.contains("pub(crate) fn init(cx: &mut App) -> bool"),
            "lib.rs 的生命周期契约仍按 `system_tray::init(cx)` 扫描"
        );
    }

    #[test]
    fn system_tray_quit_reuses_the_existing_quit_request_instead_of_quitting_directly() {
        // 注意要匹配 `=> {`：`execute_command` 开头还有一处
        // `if command == TrayCommand::QuitApplication` 的异常分支，
        // 只按名字 split 会截在 match 分支之前。
        let quit_arm = COMMANDS_SOURCE
            .split("TrayCommand::QuitApplication => {")
            .nth(1)
            .expect("退出分支必须存在")
            .split("\n    }\n\n    Ok(())")
            .next()
            .expect("退出分支的结尾必须可定位");
        assert!(quit_arm.contains("request_window_quit"));
        assert!(quit_arm.contains("restore_main_window"));

        let direct_quit = ["cx", ".quit()"].concat();
        assert_eq!(
            COMMANDS_SOURCE.matches(&direct_quit).count(),
            1,
            "只有主窗口已经消失时允许直接退出"
        );
    }
}
