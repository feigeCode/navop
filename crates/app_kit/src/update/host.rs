//! 宿主注入点：`app_kit` 不依赖具体产品，需要产品参与的少数行为在这里注册。
//!
//! 目前只有两件事：退出应用前清理产品资源（会话、连接、插件），以及更新弹窗
//! 使用的品牌图标路径。两者都有兜底实现，因此独立产品不注册也能正常工作。

use std::sync::OnceLock;

use gpui::App;

/// 退出前清理产品资源的回调：参数是应用与退出原因。
pub type ShutdownHook = fn(&mut App, &'static str);

/// 未 bootstrap 时使用的产品显示名。
#[cfg(target_os = "windows")]
const DEFAULT_DISPLAY_NAME: &str = "Navop";
/// 默认的应用图标资源路径，与全量产品的 `AssetSource` 对齐。
const DEFAULT_ICON_ASSET_PATH: &str = "navop/app-icon.png";

static SHUTDOWN_HOOK: OnceLock<ShutdownHook> = OnceLock::new();
static ICON_ASSET_PATH: OnceLock<&'static str> = OnceLock::new();

/// 注册「退出前清理资源」回调；重复注册只有第一次生效。
pub fn set_shutdown_hook(hook: ShutdownHook) {
    let _ = SHUTDOWN_HOOK.set(hook);
}

/// 注册应用图标的资源路径；重复注册只有第一次生效。
pub fn set_icon_asset_path(path: &'static str) {
    let _ = ICON_ASSET_PATH.set(path);
}

/// 应用图标的资源路径。
pub fn icon_asset_path() -> &'static str {
    ICON_ASSET_PATH
        .get()
        .copied()
        .unwrap_or(DEFAULT_ICON_ASSET_PATH)
}

/// 当前产品的显示名。
#[cfg(target_os = "windows")]
pub(crate) fn display_name() -> &'static str {
    crate::identity().map_or(DEFAULT_DISPLAY_NAME, |identity| identity.display_name)
}

/// 退出应用：宿主注册了清理回调时先清理再退出，否则直接退出。
pub(crate) fn shutdown_application(cx: &mut App, reason: &'static str) {
    match SHUTDOWN_HOOK.get() {
        Some(hook) => hook(cx, reason),
        None => cx.quit(),
    }
}
