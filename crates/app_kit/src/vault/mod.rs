//! 主密钥解锁 / 设置流程与凭据库弹窗（三个产品共享）。
//!
//! 迁移自 `main/src/master_key_flow.rs`、`main/src/master_key_dialogs.rs` 与
//! `main/src/credential_vault/*`。本地加解密与仓库层都在 `one_core`，这里只负责
//! 流程编排与 UI，因此独立产品也能直接使用：没有主密钥 UI 的产品接上本模块后，
//! 首次设置 / 解锁主密钥、查看与编辑凭据库都能真实工作。
//!
//! # 宿主钩子
//!
//! 有 4 件事依赖宿主产品（全量产品有云同步与笔记，独立产品没有），通过
//! [`VaultHostHooks`] 注入，未注入时全部 no-op：
//!
//! - **云端重加密**（[`VaultHostHooks::reencrypt_cloud`]）：修改主密钥 / 升级 KDF
//!   之后，把云端密文用新密钥重写一遍。默认不启动，返回 `false`。
//! - **云端清空**（[`VaultHostHooks::wipe_cloud`]）：重置主密钥时按勾选删除个人
//!   同步数据与团队数据。默认什么都不做。
//! - **密钥版本上报**（[`VaultHostHooks::report_key_version`]）：把当前主密钥的
//!   验证数据与版本号写到云端，其他设备据此识别"云端密钥被改过"。默认不上报。
//! - **笔记目录**（[`VaultHostHooks::notes_root`]）：重置时删除的笔记目录。默认
//!   没有笔记功能，返回 `None`。
//!
//! 注意：钩子只覆盖"宿主专有能力"。主密钥解锁、KDF 迁移、本地密文轮换、凭据库
//! 读写都在本模块内完成，钩子为空不会跳过这些逻辑。

use std::path::PathBuf;
use std::sync::{Arc, OnceLock};

use gpui::{App, AppContext as _, Entity, ParentElement as _, Styled as _, Window, div, px};
use gpui_component::h_flex;
use gpui_component::input::{Input, InputState};

use crate::vault::master_key_flow::CloudReencryptNotifier;

pub mod credential_form;
pub mod credential_list;
pub mod credentials;
pub mod dialogs;
pub mod master_key_flow;
pub mod reset;
pub mod unlock;

/// 重置时云端要清掉的范围（由 [`reset::ResetScope`] 推导）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CloudWipeScope {
    /// 个人同步数据与版本信息。
    pub personal_sync: bool,
    /// 云端团队共享数据。
    pub team_data: bool,
}

/// 宿主产品提供的 vault 钩子，默认实现见 [`VaultHostHooks::default`]。
pub struct VaultHostHooks {
    /// 云端重加密：参数为旧密钥、新密钥、密钥版本、完成通知与 `App`，
    /// 返回是否真的启动了（未登录 / 未配置同步时为 `false`）。
    pub reencrypt_cloud: Arc<
        dyn Fn(&str, &str, u32, Option<CloudReencryptNotifier>, &mut App) -> bool + Send + Sync,
    >,
    /// 云端清空。
    pub wipe_cloud: Arc<dyn Fn(CloudWipeScope, &mut App) + Send + Sync>,
    /// 上报主密钥版本到云端。
    pub report_key_version: Arc<dyn Fn(&mut App) + Send + Sync>,
    /// 笔记目录；`None` 表示该产品没有笔记功能。
    pub notes_root: Arc<dyn Fn() -> Option<PathBuf> + Send + Sync>,
}

impl Default for VaultHostHooks {
    fn default() -> Self {
        Self {
            reencrypt_cloud: Arc::new(|_, _, _, _, _| false),
            wipe_cloud: Arc::new(|_, _| {}),
            report_key_version: Arc::new(|_| {}),
            notes_root: Arc::new(|| None),
        }
    }
}

static HOST_HOOKS: OnceLock<Arc<VaultHostHooks>> = OnceLock::new();

/// 注册宿主钩子（幂等：只有第一次生效，重复调用直接忽略）。
pub fn install_host_hooks(hooks: VaultHostHooks) {
    if HOST_HOOKS.set(Arc::new(hooks)).is_err() {
        tracing::debug!("vault 宿主钩子已注册，忽略重复安装");
    }
}

/// 已注册的宿主钩子；独立产品没注册时返回 `None`。
pub fn host_hooks() -> Option<&'static VaultHostHooks> {
    HOST_HOOKS.get().map(Arc::as_ref)
}

/// 确保主密钥可用。
///
/// 已解锁返回 `true`；未设置或已锁定时弹「首次设置 / 解锁」弹窗并返回 `false`
/// （弹窗是异步的，成功与否通过弹窗内部的 KDF 迁移 / 版本上报回调体现，
/// 不阻塞调用方）。
pub fn ensure_master_key_ready(window: &mut Window, cx: &mut App) -> bool {
    if one_core::crypto::has_master_key() {
        return true;
    }
    unlock::show_master_key_dialog(window, cx);
    false
}

/// 主密钥弹窗成功后的回调（用于让调用方刷新界面状态）。
pub type MasterKeyDialogCallback = Box<dyn Fn(&mut App) + 'static>;

/// 用 `Rc<RefCell<..>>` 包一层：`open_dialog` 的 build 闭包与 `Dialog::on_ok`
/// 都要求 `Fn`，不能把捕获的回调值移出来；`RefCell` 让回调取出后置空，只触发一次。
pub(crate) fn shared_callback(
    callback: Option<MasterKeyDialogCallback>,
) -> std::rc::Rc<std::cell::RefCell<Option<MasterKeyDialogCallback>>> {
    std::rc::Rc::new(std::cell::RefCell::new(callback))
}

/// 一行「标签 + 密钥输入框」。
pub(crate) fn key_field(label: &'static str, input: &Entity<InputState>) -> gpui::Div {
    h_flex()
        .items_center()
        .gap_3()
        .child(div().text_sm().flex_shrink_0().w(px(120.)).child(label))
        .child(Input::new(input).mask_toggle().w_full())
}

/// 新建一个遮蔽输入的密钥框。
pub(crate) fn masked_input(
    window: &mut Window,
    cx: &mut App,
    placeholder: &'static str,
) -> Entity<InputState> {
    cx.new(|cx| {
        InputState::new(window, cx)
            .placeholder(placeholder)
            .masked(true)
    })
}

/// 写入弹窗内的错误提示。
pub(crate) fn set_error(error: &Entity<Option<String>>, message: &str, cx: &mut App) {
    error.update(cx, |slot, cx| {
        *slot = Some(message.to_string());
        cx.notify();
    });
}

/// 打开凭据库弹窗（锁定状态也能查看摘要）。
pub fn open_credential_vault(window: &mut Window, cx: &mut App) {
    credentials::open_credential_vault(window, cx);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_hooks_are_noop_and_install_is_idempotent() {
        let default = VaultHostHooks::default();
        assert!((default.notes_root)().is_none());

        install_host_hooks(VaultHostHooks::default());
        let first = host_hooks().expect("hooks installed");
        // 第二次安装不生效：全局钩子只认第一次注册的那一份
        install_host_hooks(VaultHostHooks::default());
        assert!(std::ptr::eq(first, host_hooks().expect("hooks installed")));
    }
}
