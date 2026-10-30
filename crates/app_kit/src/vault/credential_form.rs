//! 凭据编辑表单（弹窗形态，不是 Tab 标签页）。
//!
//! 只暴露名称 / 用户名 / 密码三个字段：私钥路径、私钥内容、passphrase、终端
//! 自动登录规则在新建时保持默认、在编辑时原样保留，避免"改个名字却把私钥清空"。
//! 敏感字段由仓库层用主密钥加密，因此保存前必须已解锁——未解锁时仓库报错，这里
//! 把错误原文交给用户并提示先解锁。

use gpui::prelude::FluentBuilder as _;
use gpui::{App, AppContext as _, ParentElement as _, Styled as _, Window, div, px};
use gpui_component::input::{Input, InputState};
use gpui_component::{ActiveTheme as _, WindowExt as _, h_flex, v_flex};
use one_core::connection_notifier::{ConnectionDataEvent, emit_connection_event_from_app};
use one_core::storage::traits::Repository as _;
use one_core::storage::{CredentialEntry, CredentialRepository, GlobalStorageState};

use crate::vault::{set_error, shared_callback};

pub type SavedCallback = Box<dyn Fn(&mut App) + 'static>;

/// 提交时需要的输入、原始条目与回调
struct FormSubmit {
    name: gpui::Entity<InputState>,
    user: gpui::Entity<InputState>,
    password: gpui::Entity<InputState>,
    error: gpui::Entity<Option<String>>,
    entry: CredentialEntry,
    editing: bool,
    on_saved: std::rc::Rc<std::cell::RefCell<Option<SavedCallback>>>,
}

/// 打开「新建 / 编辑凭据」弹窗。
pub fn show_credential_form(
    window: &mut Window,
    cx: &mut App,
    existing: Option<CredentialEntry>,
    on_saved: Option<SavedCallback>,
) {
    let editing = existing.is_some();
    let entry = existing.unwrap_or_else(|| CredentialEntry::new(""));
    let (name_input, user_input, password_input) = build_inputs(window, cx, &entry);
    let error_message = cx.new(|_| Option::<String>::None);
    let on_saved = shared_callback(on_saved);
    let focus_input = name_input.clone();
    let title = if editing {
        "编辑凭据"
    } else {
        "新增凭据"
    };

    window.open_dialog(cx, move |dialog, _window, cx| {
        let state = FormSubmit {
            name: name_input.clone(),
            user: user_input.clone(),
            password: password_input.clone(),
            error: error_message.clone(),
            entry: entry.clone(),
            editing,
            on_saved: on_saved.clone(),
        };
        dialog
            .title(title)
            .width(px(560.))
            .confirm()
            .on_cancel(|_, _, _| true)
            .on_ok(move |_, window, cx: &mut App| submit_form(&state, window, cx))
            .child(form_body(
                &name_input,
                &user_input,
                &password_input,
                &error_message,
                cx,
            ))
    });

    focus_input.update(cx, |input, cx| input.focus(window, cx));
}

/// 校验名称 → 保存（新建 / 更新）→ 广播事件并刷新列表
fn submit_form(state: &FormSubmit, window: &mut Window, cx: &mut App) -> bool {
    let name = state.name.read(cx).value().trim().to_string();
    if name.is_empty() {
        set_error(&state.error, "凭据名称不能为空", cx);
        return false;
    }
    let mut updated = state.entry.clone();
    updated.name = name;
    updated.username = non_empty(state.user.read(cx).value().to_string());
    updated.password = non_empty(state.password.read(cx).value().to_string());

    match save(&mut updated, state.editing, cx) {
        Ok(id) => {
            emit_connection_event_from_app(connection_event(id, state.editing), cx);
            window.push_notification(
                if state.editing {
                    "凭据已更新"
                } else {
                    "凭据已创建"
                },
                cx,
            );
            if let Some(callback) = state.on_saved.borrow_mut().take() {
                callback(cx);
            }
            true
        }
        Err(error) => {
            tracing::error!(%error, "保存凭据失败");
            set_error(&state.error, &error, cx);
            false
        }
    }
}

/// 弹窗内容：名称 / 用户名 / 密码 + 加密说明 + 错误行
fn form_body(
    name: &gpui::Entity<InputState>,
    user: &gpui::Entity<InputState>,
    password: &gpui::Entity<InputState>,
    error: &gpui::Entity<Option<String>>,
    cx: &App,
) -> gpui::Div {
    v_flex()
        .gap_4()
        .p_4()
        .child(field_row("名称", name))
        .child(field_row("用户名", user))
        .child(
            h_flex()
                .items_center()
                .gap_3()
                .child(div().text_sm().flex_shrink_0().w(px(80.)).child("密码"))
                .child(Input::new(password).mask_toggle().w_full()),
        )
        .child(
            div()
                .text_xs()
                .text_color(cx.theme().muted_foreground)
                .child("敏感字段使用主密钥加密保存；未解锁主密钥时无法保存。"),
        )
        .when_some(error.read(cx).clone(), |this, message| {
            this.child(div().text_sm().text_color(cx.theme().danger).child(message))
        })
}

/// 三个输入框：名称 / 用户名（明文）与密码（遮蔽）
fn build_inputs(
    window: &mut Window,
    cx: &mut App,
    entry: &CredentialEntry,
) -> (
    gpui::Entity<InputState>,
    gpui::Entity<InputState>,
    gpui::Entity<InputState>,
) {
    let name = text_input(window, cx, "例如：生产环境账号", entry.name.clone());
    let user = text_input(
        window,
        cx,
        "可选用户名",
        entry.username.clone().unwrap_or_default(),
    );
    let password = masked_input(
        window,
        cx,
        "可选密码",
        entry.password.clone().unwrap_or_default(),
    );
    (name, user, password)
}

fn text_input(
    window: &mut Window,
    cx: &mut App,
    placeholder: &'static str,
    value: String,
) -> gpui::Entity<InputState> {
    let input = cx.new(|cx| {
        let state = InputState::new(window, cx)
            .placeholder(placeholder)
            .masked(false);
        if value.is_empty() {
            state
        } else {
            state.default_value(value)
        }
    });
    input
}

fn masked_input(
    window: &mut Window,
    cx: &mut App,
    placeholder: &'static str,
    value: String,
) -> gpui::Entity<InputState> {
    cx.new(|cx| {
        let state = InputState::new(window, cx)
            .placeholder(placeholder)
            .masked(true);
        if value.is_empty() {
            state
        } else {
            state.default_value(value)
        }
    })
}

fn field_row(label: &'static str, input: &gpui::Entity<InputState>) -> gpui::Div {
    h_flex()
        .items_center()
        .gap_3()
        .child(div().text_sm().flex_shrink_0().w(px(80.)).child(label))
        .child(Input::new(input).w_full())
}

/// 空字符串归一成 `None`：仓库用 `None` 表示"没有这个字段"。
fn non_empty(value: String) -> Option<String> {
    (!value.is_empty()).then_some(value)
}

fn save(entry: &mut CredentialEntry, editing: bool, cx: &mut App) -> Result<i64, String> {
    let storage = cx.global::<GlobalStorageState>().storage.clone();
    let repository = storage
        .get::<CredentialRepository>()
        .ok_or_else(|| "凭据仓库尚未注册".to_string())?;
    if editing {
        let id = entry
            .id
            .ok_or_else(|| "凭据不存在，可能已被删除。".to_string())?;
        repository
            .update(entry)
            .map_err(|error| error.to_string())?;
        Ok(id)
    } else {
        repository.insert(entry).map_err(|error| error.to_string())
    }
}

fn connection_event(id: i64, editing: bool) -> ConnectionDataEvent {
    if editing {
        ConnectionDataEvent::CredentialUpdated { credential_id: id }
    } else {
        ConnectionDataEvent::CredentialCreated { credential_id: id }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_strings_become_none() {
        assert_eq!(non_empty(String::new()), None);
        assert_eq!(non_empty("u".to_string()), Some("u".to_string()));
    }
}
