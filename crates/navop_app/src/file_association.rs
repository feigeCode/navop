//! 文件关联与 URL scheme 注册。
//!
//! 实现已迁入 [`app_kit::file_association`]：接管哪些扩展名由各产品的
//! `AppIdentity::file_associations` 决定。URL scheme 还要叠加用户开关：
//! `jms://` 默认启用，`ssh://`/`sftp://` 等通用 scheme 默认关闭；只有开关打开
//! 的 scheme 才会写进系统，关掉的会从系统里摘除。

/// 按当前设置重新注册关联（启动时调用）。
///
/// 注册完成后只在设置页已经展示过接管状态时刷新一次探测：启动阶段没必要为了
/// 一个没打开的页面去查注册表 / 起 `xdg-mime`。
pub(crate) fn schedule_registration(cx: &mut gpui::App) {
    schedule(cx, crate::settings::deeplink_status::refresh_if_active);
}

/// 按当前设置重新注册关联，并强制重新探测接管状态。
///
/// 设置页拨动开关后调用：用户就在这一页，必须马上看到系统侧的真实结果。
pub(crate) fn schedule_registration_after_toggle(cx: &mut gpui::App) {
    schedule(cx, crate::settings::deeplink_status::refresh);
}

fn schedule(cx: &mut gpui::App, on_finished: impl FnOnce(&mut gpui::App) + 'static) {
    if !one_core::app_paths::is_installed() {
        // 开发态/便携版不写系统关联，避免改掉本机正常的链接处理程序；
        // 但设置页仍要显示真实状态。
        tracing::debug!("application is not installed; skipping URL scheme registration");
        on_finished(cx);
        return;
    }
    let schemes = one_core::settings::AppSettings::global(cx).enabled_deeplink_schemes();
    app_kit::file_association::schedule_registration_with_schemes_and_then(
        cx,
        schemes,
        on_finished,
    );
}
