//! 主密钥「修改」与「重置」弹窗——`navop_app` 侧兼容层
//!
//! 弹窗本体已下沉到 `app_kit::vault::dialogs`。这里保留原来的函数名与签名，
//! 只在打开前注册 [`crate::master_key_flow`] 里的宿主钩子（Navop Cloud / 个人
//! 同步 / 笔记目录），保证 `app_kit` 侧后续执行修改或重置时能用到 main 的实现。

use gpui::{App, Window};

pub(crate) use app_kit::vault::MasterKeyDialogCallback;

/// 修改主密钥：旧密钥 + 新密钥 + 确认新密钥
pub fn show_change_master_key_dialog(
    window: &mut Window,
    cx: &mut App,
    on_success: Option<MasterKeyDialogCallback>,
) {
    crate::master_key_flow::register_host_hooks();
    app_kit::vault::dialogs::show_change_master_key_dialog(window, cx, on_success);
}

/// 重置主密钥：强提示 + 勾选范围 + 确认词
pub fn show_reset_master_key_dialog(
    window: &mut Window,
    cx: &mut App,
    on_success: Option<MasterKeyDialogCallback>,
) {
    crate::master_key_flow::register_host_hooks();
    app_kit::vault::dialogs::show_reset_master_key_dialog(window, cx, on_success);
}
