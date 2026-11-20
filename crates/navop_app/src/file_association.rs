//! 文件关联与 URL scheme 注册。
//!
//! 实现已迁入 [`app_kit::file_association`]：接管哪些扩展名由各产品的
//! `AppIdentity::file_associations` 决定。URL scheme 还要叠加用户开关：
//! `jms://` 始终注册，`ssh://`/`sftp://` 等通用 scheme 只有用户启用后才写进系统。

pub use app_kit::file_association::*;

/// 按当前设置重新注册关联（设置里切换协议开关后调用）。
pub(crate) fn schedule_registration(cx: &mut gpui::App) {
    if !one_core::app_paths::is_installed() {
        // 开发态/便携版不写系统关联，避免改掉本机正常的链接处理程序。
        tracing::debug!("application is not installed; skipping URL scheme registration");
        return;
    }
    let schemes = one_core::settings::AppSettings::global(cx).enabled_deeplink_schemes();
    app_kit::file_association::schedule_registration_with_schemes(cx, schemes);
}
