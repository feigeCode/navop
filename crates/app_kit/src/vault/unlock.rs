//! 首次设置 / 解锁主密钥弹窗（独立产品的主要入口）。
//!
//! 全量产品 `navop` 的主页有自己更完整的弹窗（含云同步说明与连接刷新），这里
//! 是给 `navop-shell` / `navop-data` 这类"没有主密钥 UI"的产品用的通用版本：
//! 一个遮蔽输入框 + 设置 / 解锁两种模式，成功后做 KDF 迁移与密钥版本上报。
//!
//! 启动保护（`master_key_on_startup_required`）下密钥只留在内存：弹窗不可关闭，
//! 且用 `*_for_session` 系列 API，避免把密钥写进钥匙串。

use gpui::prelude::FluentBuilder as _;
use gpui::{App, AppContext as _, Entity, ParentElement as _, Styled as _, Window, div, px};
use gpui_component::input::{Input, InputState};
use gpui_component::{ActiveTheme as _, WindowExt as _, h_flex, v_flex};
use one_core::crypto;
use one_core::settings::AppSettings;

use crate::vault::master_key_flow;
use crate::vault::set_error;

/// 弹窗所处模式：首次设置 / 普通解锁 / 启动保护下解锁
#[derive(Clone, Copy)]
struct DialogMode {
    first_setup: bool,
    require_on_startup: bool,
    startup_lock: bool,
}

/// 提交时需要的输入与错误槽
struct Submit {
    mode: DialogMode,
    input: Entity<InputState>,
    error: Entity<Option<String>>,
}

fn dialog_mode(cx: &App) -> DialogMode {
    let first_setup = !crypto::has_repo_password_set();
    let require_on_startup = AppSettings::global(cx).master_key_on_startup_required();
    // 启动锁：已设置主密钥但本次启动还没解锁。此时不允许跳过，也不能预填。
    let startup_lock = require_on_startup && !first_setup && !crypto::has_master_key();
    DialogMode {
        first_setup,
        require_on_startup,
        startup_lock,
    }
}

/// 弹出「首次设置」或「解锁」弹窗。
pub fn show_master_key_dialog(window: &mut Window, cx: &mut App) {
    let mode = dialog_mode(cx);
    let key_input = build_key_input(window, cx, mode);
    let error_message = cx.new(|_| Option::<String>::None);
    let focus_input = key_input.clone();
    let title = if mode.first_setup {
        "设置主密钥"
    } else {
        "解锁"
    };
    let cancellable = !mode.startup_lock;

    window.open_dialog(cx, move |dialog, _window, cx| {
        let state = Submit {
            mode,
            input: key_input.clone(),
            error: error_message.clone(),
        };
        dialog
            .title(title)
            .width(px(520.))
            .confirm()
            .overlay_closable(cancellable)
            .close_button(cancellable)
            .on_cancel(move |_, _, _| cancellable)
            .on_ok(move |_, _window, cx: &mut App| submit(&state, cx))
            .on_close(move |_window, _result, cx| {
                if crypto::has_master_key() {
                    // 密钥已就绪：把版本同步到云端（独立产品未注册钩子时 no-op）
                    master_key_flow::report_key_version_to_cloud(cx);
                }
            })
            .child(dialog_body(
                mode.startup_lock,
                &key_input,
                &error_message,
                cx,
            ))
    });

    focus_input.update(cx, |input, cx| input.focus(window, cx));
}

/// 输入框；启动锁下不预填钥匙串里的密钥
fn build_key_input(window: &mut Window, cx: &mut App, mode: DialogMode) -> Entity<InputState> {
    let initial = (!mode.startup_lock)
        .then(crypto::get_raw_master_key)
        .flatten()
        .or_else(|| {
            (!mode.startup_lock)
                .then(|| one_core::key_storage::get_key_storage())
                .and_then(|storage| storage.load())
        });
    cx.new(|cx| {
        let state = InputState::new(window, cx)
            .placeholder("主密钥：")
            .masked(true);
        match initial {
            Some(value) => state.default_value(value),
            None => state,
        }
    })
}

/// 校验输入 → 设置或验证主密钥 → 成功后就地迁移 KDF
fn submit(state: &Submit, cx: &mut App) -> bool {
    let input_key = state.input.read(cx).value().to_string();
    if input_key.is_empty() {
        set_error(&state.error, "主密钥不能为空", cx);
        return false;
    }

    let mode = state.mode;
    let result = match (mode.first_setup, mode.require_on_startup) {
        (true, true) => crypto::set_master_key_for_session(&input_key),
        (true, false) => crypto::set_master_key(&input_key),
        (false, true) => crypto::verify_and_set_master_key_for_session(&input_key),
        (false, false) => crypto::verify_and_set_master_key(&input_key),
    };

    match result {
        Ok(()) => {
            // 旧数据可能是固定盐格式：解锁后顺带升级到 Argon2id（后台完成）
            master_key_flow::run_kdf_upgrade_after_unlock(cx);
            master_key_flow::report_key_version_to_cloud(cx);
            true
        }
        Err(error) => {
            tracing::error!("主密钥解锁失败: {error}");
            set_error(&state.error, &master_key_error_message(&error), cx);
            false
        }
    }
}

/// 弹窗内容：启动锁说明 + 输入框 + 不可找回提示 + 错误行
fn dialog_body(
    startup_lock: bool,
    input: &Entity<InputState>,
    error: &Entity<Option<String>>,
    cx: &App,
) -> gpui::Div {
    v_flex()
        .gap_4()
        .p_4()
        .when(startup_lock, |content| {
            content.child(
                div()
                    .text_sm()
                    .text_color(cx.theme().muted_foreground)
                    .child("已启用启动保护，请输入主密钥解锁 Navop。"),
            )
        })
        .child(key_field(input))
        .child(
            div()
                .text_sm()
                .text_color(cx.theme().warning)
                .child("主密钥无法找回：忘记或丢失后，本地加密数据与已同步的云端数据将无法解密。"),
        )
        .when_some(error.read(cx).clone(), |this, message| {
            this.child(div().text_sm().text_color(cx.theme().danger).child(message))
        })
}

fn key_field(input: &Entity<InputState>) -> gpui::Div {
    h_flex()
        .items_center()
        .gap_3()
        .child(div().text_sm().flex_shrink_0().w(px(80.)).child("主密钥："))
        .child(Input::new(input).mask_toggle().w_full())
}

fn master_key_error_message(error: &crypto::CryptoError) -> String {
    match error {
        crypto::CryptoError::InvalidOldPassword => "主密钥错误".to_string(),
        _ => "无法安全保存或读取主密钥，数据仍保持锁定。".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn invalid_key_maps_to_a_clear_message() {
        assert_eq!(
            master_key_error_message(&crypto::CryptoError::InvalidOldPassword),
            "主密钥错误"
        );
        assert!(
            master_key_error_message(&crypto::CryptoError::NoPasswordSet)
                .contains("数据仍保持锁定")
        );
    }
}
