//! 主密钥「修改」与「重置」弹窗（三产品共享）。
//!
//! 与「首次设置 / 解锁」（[`super::unlock`]）分开：那两个场景只有一个输入框，
//! 修改需要「旧 + 新 + 确认」，重置需要强提示与确认词，混在一起每个场景都要
//! 背无关控件。文案硬编码简体中文，`app_kit` 不依赖各产品的 `rust_i18n`。

use std::cell::RefCell;
use std::rc::Rc;

use gpui::prelude::FluentBuilder as _;
use gpui::{App, AppContext as _, Entity, ParentElement as _, Styled as _, Window, div, px};
use gpui_component::input::{Input, InputState};
use gpui_component::{ActiveTheme as _, WindowExt as _, h_flex, v_flex};

use crate::vault::master_key_flow::{self, RESET_CONFIRM_WORD};
use crate::vault::reset::{self, RESET_SCOPE_ROWS, ResetScope, scope_row, scope_toggle};
use crate::vault::{MasterKeyDialogCallback, key_field, masked_input, set_error, shared_callback};

/// `on_ok` 会多次调用（用户可能改完再提交），回调只能取出一次
type SharedCallback = Rc<RefCell<Option<MasterKeyDialogCallback>>>;
/// 云端重加密是异步的，结果由这个 notifier 弹回窗口，同样只能取一次
type SharedNotifier = Rc<RefCell<Option<master_key_flow::CloudReencryptNotifier>>>;

/// 「修改」弹窗提交时需要的全部状态，避免 `on_ok` 参数过长
struct ChangeOk {
    old: Entity<InputState>,
    new: Entity<InputState>,
    confirm: Entity<InputState>,
    error: Entity<Option<String>>,
    notifier: SharedNotifier,
    on_success: SharedCallback,
}

/// 「重置」弹窗提交时需要的状态
struct ResetOk {
    confirm: Entity<InputState>,
    error: Entity<Option<String>>,
    on_success: SharedCallback,
}

/// 修改主密钥弹窗：旧密钥 + 新密钥 + 确认新密钥。
pub fn show_change_master_key_dialog(
    window: &mut Window,
    cx: &mut App,
    on_success: Option<MasterKeyDialogCallback>,
) {
    if !one_core::crypto::has_repo_password_set() {
        window.push_notification(one_core::crypto::CryptoError::NoPasswordSet.to_string(), cx);
        return;
    }

    let old_input = masked_input(window, cx, "输入旧主密钥");
    let new_input = masked_input(window, cx, "输入新主密钥");
    let confirm_input = masked_input(window, cx, "确认您的主密钥");
    let error_message = cx.new(|_| Option::<String>::None);
    let on_success = shared_callback(on_success);
    let notifier = reencrypt_notifier(window);
    let focus_input = old_input.clone();

    window.open_dialog(cx, move |dialog, _window, cx| {
        let state = ChangeOk {
            old: old_input.clone(),
            new: new_input.clone(),
            confirm: confirm_input.clone(),
            error: error_message.clone(),
            notifier: notifier.clone(),
            on_success: on_success.clone(),
        };
        dialog
            .title("修改主密钥")
            .width(px(560.))
            .confirm()
            .on_cancel(|_, _, _| true)
            .on_ok(move |_, window, cx: &mut App| submit_change(&state, window, cx))
            .child(change_dialog_body(
                &old_input,
                &new_input,
                &confirm_input,
                &error_message,
                cx,
            ))
    });

    focus_input.update(cx, |input, cx| input.focus(window, cx));
}

/// 把异步重加密结果弹成窗口通知
fn reencrypt_notifier(window: &Window) -> SharedNotifier {
    let window_handle = window.window_handle();
    let notifier: Option<master_key_flow::CloudReencryptNotifier> =
        Some(Box::new(move |cx: &mut App, message: String| {
            let _ = cx.update_window(window_handle, |_, window, cx| {
                window.push_notification(message, cx);
            });
        }));
    Rc::new(RefCell::new(notifier))
}

/// 修改弹窗的表单内容（提示 + 三个输入框 + 错误行）
fn change_dialog_body(
    old: &Entity<InputState>,
    new: &Entity<InputState>,
    confirm: &Entity<InputState>,
    error: &Entity<Option<String>>,
    cx: &App,
) -> gpui::Div {
    v_flex()
        .gap_4()
        .p_4()
        .child(
            div().text_sm().text_color(cx.theme().warning).child(
                "修改主密钥会重新加密本地连接、钥匙串条目与团队密钥缓存，并用新密钥重写已同步的云端数据。请保持 Navop 打开直至完成。",
            ),
        )
        .child(key_field("旧主密钥：", old))
        .child(key_field("新主密钥：", new))
        .child(key_field("确认主密钥：", confirm))
        .when_some(error.read(cx).clone(), |this, message| {
            this.child(div().text_sm().text_color(cx.theme().danger).child(message))
        })
}

/// 校验输入 → 执行换钥 → 弹结果通知
fn submit_change(state: &ChangeOk, window: &mut Window, cx: &mut App) -> bool {
    let old_key = state.old.read(cx).value().to_string();
    let new_key = state.new.read(cx).value().to_string();
    let confirm_key = state.confirm.read(cx).value().to_string();
    if old_key.is_empty() || new_key.is_empty() {
        set_error(&state.error, "主密钥不能为空", cx);
        return false;
    }
    if new_key != confirm_key {
        set_error(&state.error, "两次输入的密钥不一致", cx);
        return false;
    }

    let notifier = state.notifier.borrow_mut().take();
    match master_key_flow::apply_change_master_key(&old_key, &new_key, notifier, cx) {
        Ok(outcome) => {
            tracing::info!(
                "主密钥修改完成：连接 {}，钥匙串 {}，团队密钥缓存 {}，版本 {}，云端重加密已启动 {}",
                outcome.connections,
                outcome.credentials,
                outcome.team_key_caches,
                outcome.key_version,
                outcome.cloud_reencrypt_started
            );
            let mut message = format!(
                "主密钥已更新。已重新加密 {} 个连接、{} 个钥匙串条目、{} 个团队密钥缓存（版本 {}）。",
                outcome.connections,
                outcome.credentials,
                outcome.team_key_caches,
                outcome.key_version
            );
            if outcome.cloud_reencrypt_started {
                message.push_str("\n正在用新密钥同步云端数据…");
            }
            window.push_notification(message, cx);
            if let Some(callback) = state.on_success.borrow_mut().take() {
                callback(cx);
            }
            true
        }
        Err(error) => {
            set_error(&state.error, &error, cx);
            false
        }
    }
}

/// 重置主密钥弹窗：强提示 + 删除清单 + 输入 RESET 二次确认。
pub fn show_reset_master_key_dialog(
    window: &mut Window,
    cx: &mut App,
    on_success: Option<MasterKeyDialogCallback>,
) {
    let scope = reset::reset_scope(cx);
    let confirm_input = cx.new(|cx| {
        InputState::new(window, cx)
            .placeholder(RESET_CONFIRM_WORD)
            .masked(false)
    });
    let error_message = cx.new(|_| Option::<String>::None);
    let on_success = shared_callback(on_success);
    let focus_input = confirm_input.clone();

    window.open_dialog(cx, move |dialog, _window, cx| {
        let state = ResetOk {
            confirm: confirm_input.clone(),
            error: error_message.clone(),
            on_success: on_success.clone(),
        };
        dialog
            .title("重置主密钥")
            .width(px(560.))
            .confirm()
            .on_cancel(|_, _, _| true)
            .on_ok(move |_, window, cx: &mut App| submit_reset(&state, window, cx))
            .child(reset_dialog_body(scope, &confirm_input, &error_message, cx))
    });

    focus_input.update(cx, |input, cx| input.focus(window, cx));
}

/// 重置弹窗的表单内容：风险提示 + 勾选清单 + RESET 输入框
fn reset_dialog_body(
    scope: ResetScope,
    confirm: &Entity<InputState>,
    error: &Entity<Option<String>>,
    cx: &App,
) -> gpui::Div {
    let mut content = v_flex()
        .gap_3()
        .p_4()
        .child(
            div()
                .text_sm()
                .text_color(cx.theme().danger)
                .child("重置将清除本地加密数据以及与该密钥关联的云端数据，且无法撤销。"),
        )
        .child(
            div()
                .text_sm()
                .font_weight(gpui::FontWeight::SEMIBOLD)
                .child("以下数据将被永久删除："),
        );
    for (index, (id, title, description)) in RESET_SCOPE_ROWS.iter().enumerate() {
        let mut scope = scope;
        content = content.child(scope_row(
            id,
            title,
            description,
            scope_toggle(&mut scope, index, None),
            index,
            cx,
        ));
    }
    content
        .child(
            div()
                .text_sm()
                .font_weight(gpui::FontWeight::SEMIBOLD)
                .child("输入 RESET 以继续："),
        )
        .child(h_flex().gap_2().child(Input::new(confirm).w_full()))
        .when_some(error.read(cx).clone(), |this, message| {
            this.child(div().text_sm().text_color(cx.theme().danger).child(message))
        })
}

/// 校验 RESET 确认词 → 执行重置 → 弹结果通知
fn submit_reset(state: &ResetOk, window: &mut Window, cx: &mut App) -> bool {
    if state.confirm.read(cx).value().trim() != RESET_CONFIRM_WORD {
        set_error(&state.error, "确认文本不匹配，请输入 RESET 以继续。", cx);
        return false;
    }
    // 勾选项是弹窗里现场改的，执行前重新读一次全局范围
    match reset::reset_master_key_data(reset::reset_scope(cx), cx) {
        Ok(outcome) => {
            tracing::info!("主密钥重置完成：已删除 {} 项", outcome.deleted.len());
            let message = if outcome.failed.is_empty() {
                "重置完成，Navop 已回到初始设置状态。".to_string()
            } else {
                format!("以下内容未能删除：{}", outcome.failed.join("; "))
            };
            window.push_notification(message, cx);
            if let Some(callback) = state.on_success.borrow_mut().take() {
                callback(cx);
            }
            true
        }
        Err(error) => {
            set_error(&state.error, &error, cx);
            false
        }
    }
}
