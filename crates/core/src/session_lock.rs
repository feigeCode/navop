//! Session lock / unlock password prompts used by the tab context menu.
//!
//! These dialogs are generic password prompts (mirroring SecureCRT's Lock/Unlock
//! Session flow). The actual lock state lives on the session content; this module
//! only collects and validates the password and dialog options, then returns a
//! request that the caller applies to the target tabs.

use std::fmt::Write as _;
use std::sync::{Arc, Mutex as StdMutex};

use gpui::prelude::FluentBuilder as _;
use gpui::{
    App, AppContext as _, Entity, FontWeight, ParentElement, Styled as _, Task, Window, div, px,
};
use gpui_component::checkbox::Checkbox;
use gpui_component::dialog::DialogButtonProps;
use gpui_component::input::{Input, InputState};
use gpui_component::{ActiveTheme as _, WindowExt as _, h_flex, v_flex};
use rust_i18n::t;
use sha2::{Digest, Sha256};

/// Fixed salt prefix used when hashing a lock password.
const LOCK_PASSWORD_SALT: &str = "navop-session-lock:";

/// Result of the lock dialog.
#[derive(Debug, Clone)]
pub struct LockSessionRequest {
    /// Pre-computed hash of the entered lock password.
    pub password_hash: String,
    /// Whether the terminal output should be hidden while locked.
    pub hide_output: bool,
    /// Whether all open sessions should be locked with this password.
    pub lock_all: bool,
}

/// Result of the unlock dialog.
#[derive(Debug, Clone)]
pub struct UnlockSessionRequest {
    /// Pre-computed hash of the entered unlock password.
    pub password_hash: String,
    /// Whether all locked sessions should be unlocked.
    pub unlock_all: bool,
}

/// Hash a lock password so the plaintext is never kept in memory.
pub fn hash_session_lock_password(password: &str) -> String {
    let mut input = String::with_capacity(LOCK_PASSWORD_SALT.len() + password.len());
    input.push_str(LOCK_PASSWORD_SALT);
    input.push_str(password);
    let digest = Sha256::digest(input.as_bytes());
    let mut hex = String::with_capacity(digest.len() * 2);
    for byte in digest {
        let _ = write!(&mut hex, "{byte:02x}");
    }
    hex
}

/// Open the Lock Session dialog. Resolves to `Some` only when the password is
/// non-empty and the two password fields match.
pub fn prompt_session_lock(window: &mut Window, cx: &mut App) -> Task<Option<LockSessionRequest>> {
    let password_input = cx.new(|cx| {
        InputState::new(window, cx)
            .placeholder(t!("SessionLock.password_placeholder"))
            .masked(true)
    });
    let confirm_input = cx.new(|cx| {
        InputState::new(window, cx)
            .placeholder(t!("SessionLock.confirm_password_placeholder"))
            .masked(true)
    });
    let options = cx.new(|_| LockDialogOptions::default());
    let error_message = cx.new(|_| Option::<String>::None);

    let password_for_ok = password_input.clone();
    let confirm_for_ok = confirm_input.clone();
    let options_for_ok = options.clone();
    let error_for_ok = error_message.clone();

    let password_for_render = password_input.clone();
    let confirm_for_render = confirm_input.clone();
    let options_for_render = options.clone();
    let error_for_render = error_message.clone();

    let (tx, rx) = tokio::sync::oneshot::channel::<Option<LockSessionRequest>>();
    let tx = Arc::new(StdMutex::new(Some(tx)));

    window.open_dialog(cx, move |dialog, _window, cx| {
        let password_for_ok = password_for_ok.clone();
        let confirm_for_ok = confirm_for_ok.clone();
        let options_for_ok = options_for_ok.clone();
        let error_for_ok = error_for_ok.clone();
        let tx_ok = tx.clone();
        dialog
            .title(t!("SessionLock.title").to_string())
            .w(px(460.))
            .confirm()
            .button_props(
                DialogButtonProps::default()
                    .ok_text(t!("SessionLock.lock").to_string())
                    .cancel_text(t!("Common.cancel").to_string())
                    .show_cancel(true),
            )
            .on_ok(move |_, _, cx: &mut App| {
                let password = password_for_ok.read(cx).text().to_string();
                let confirm = confirm_for_ok.read(cx).text().to_string();
                if password.is_empty() {
                    error_for_ok.update(cx, |msg, cx| {
                        *msg = Some(t!("SessionLock.password_empty").to_string());
                        cx.notify();
                    });
                    return false;
                }
                if password != confirm {
                    error_for_ok.update(cx, |msg, cx| {
                        *msg = Some(t!("SessionLock.password_mismatch").to_string());
                        cx.notify();
                    });
                    return false;
                }
                if let Some(tx) = tx_ok.lock().unwrap().take() {
                    let options = options_for_ok.read(cx);
                    let _ = tx.send(Some(LockSessionRequest {
                        password_hash: hash_session_lock_password(&password),
                        hide_output: options.hide_output,
                        lock_all: options.lock_all,
                    }));
                }
                true
            })
            .on_cancel(|_, _, _| true)
            .overlay_closable(false)
            .close_button(true)
            .child(
                v_flex()
                    .gap_3()
                    .p_4()
                    .child(password_row(
                        t!("SessionLock.password").as_ref(),
                        &password_for_render,
                    ))
                    .child(password_row(
                        t!("SessionLock.confirm_password").as_ref(),
                        &confirm_for_render,
                    ))
                    .child(
                        Checkbox::new("session-lock-all")
                            .label(t!("SessionLock.lock_all").to_string())
                            .checked(options_for_render.read(cx).lock_all)
                            .on_click({
                                let options = options_for_render.clone();
                                move |checked, _, cx| {
                                    options.update(cx, |opts, cx| {
                                        opts.lock_all = *checked;
                                        cx.notify();
                                    });
                                }
                            }),
                    )
                    .child(
                        Checkbox::new("session-lock-hide-output")
                            .label(t!("SessionLock.hide_output").to_string())
                            .checked(options_for_render.read(cx).hide_output)
                            .on_click({
                                let options = options_for_render.clone();
                                move |checked, _, cx| {
                                    options.update(cx, |opts, cx| {
                                        opts.hide_output = *checked;
                                        cx.notify();
                                    });
                                }
                            }),
                    )
                    .when_some(error_for_render.read(cx).clone(), |this, msg| {
                        this.child(div().text_sm().text_color(cx.theme().danger).child(msg))
                    }),
            )
    });
    password_input.update(cx, |input, cx| input.focus(window, cx));

    cx.spawn(async move |_cx| rx.await.ok().flatten())
}

/// Open the Unlock Session dialog. Resolves to `Some` when the password is
/// non-empty; the caller verifies the hash against the locked sessions.
pub fn prompt_session_unlock(
    window: &mut Window,
    cx: &mut App,
) -> Task<Option<UnlockSessionRequest>> {
    let password_input = cx.new(|cx| {
        InputState::new(window, cx)
            .placeholder(t!("SessionLock.password_placeholder"))
            .masked(true)
    });
    let options = cx.new(|_| UnlockDialogOptions::default());
    let error_message = cx.new(|_| Option::<String>::None);

    let password_for_ok = password_input.clone();
    let options_for_ok = options.clone();
    let error_for_ok = error_message.clone();

    let password_for_render = password_input.clone();
    let options_for_render = options.clone();
    let error_for_render = error_message.clone();

    let (tx, rx) = tokio::sync::oneshot::channel::<Option<UnlockSessionRequest>>();
    let tx = Arc::new(StdMutex::new(Some(tx)));

    window.open_dialog(cx, move |dialog, _window, cx| {
        let password_for_ok = password_for_ok.clone();
        let options_for_ok = options_for_ok.clone();
        let error_for_ok = error_for_ok.clone();
        let tx_ok = tx.clone();
        dialog
            .title(t!("SessionLock.unlock_title").to_string())
            .w(px(460.))
            .confirm()
            .button_props(
                DialogButtonProps::default()
                    .ok_text(t!("SessionLock.unlock").to_string())
                    .cancel_text(t!("Common.cancel").to_string())
                    .show_cancel(true),
            )
            .on_ok(move |_, _, cx: &mut App| {
                let password = password_for_ok.read(cx).text().to_string();
                if password.is_empty() {
                    error_for_ok.update(cx, |msg, cx| {
                        *msg = Some(t!("SessionLock.password_empty").to_string());
                        cx.notify();
                    });
                    return false;
                }
                if let Some(tx) = tx_ok.lock().unwrap().take() {
                    let options = options_for_ok.read(cx);
                    let _ = tx.send(Some(UnlockSessionRequest {
                        password_hash: hash_session_lock_password(&password),
                        unlock_all: options.unlock_all,
                    }));
                }
                true
            })
            .on_cancel(|_, _, _| true)
            .overlay_closable(false)
            .close_button(true)
            .child(
                v_flex()
                    .gap_3()
                    .p_4()
                    .child(password_row(
                        t!("SessionLock.password").as_ref(),
                        &password_for_render,
                    ))
                    .child(
                        Checkbox::new("session-unlock-all")
                            .label(t!("SessionLock.unlock_all").to_string())
                            .checked(options_for_render.read(cx).unlock_all)
                            .on_click({
                                let options = options_for_render.clone();
                                move |checked, _, cx| {
                                    options.update(cx, |opts, cx| {
                                        opts.unlock_all = *checked;
                                        cx.notify();
                                    });
                                }
                            }),
                    )
                    .when_some(error_for_render.read(cx).clone(), |this, msg| {
                        this.child(div().text_sm().text_color(cx.theme().danger).child(msg))
                    }),
            )
    });
    password_input.update(cx, |input, cx| input.focus(window, cx));

    cx.spawn(async move |_cx| rx.await.ok().flatten())
}

fn password_row(label: &str, input: &Entity<InputState>) -> impl gpui::IntoElement {
    h_flex()
        .items_center()
        .gap_3()
        .child(
            div()
                .text_sm()
                .font_weight(FontWeight::SEMIBOLD)
                .flex_shrink_0()
                .w(px(110.))
                .child(label.to_string()),
        )
        .child(Input::new(input).mask_toggle().w_full())
}

#[derive(Default)]
struct LockDialogOptions {
    lock_all: bool,
    hide_output: bool,
}

#[derive(Default)]
struct UnlockDialogOptions {
    unlock_all: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hash_is_deterministic_hex() {
        let a = hash_session_lock_password("secret");
        let b = hash_session_lock_password("secret");
        assert_eq!(a, b);
        assert_eq!(a.len(), 64);
        assert!(a.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn hash_is_salted_and_sensitive_to_input() {
        let plain = hash_session_lock_password("secret");
        let _ = plain;
        let other = hash_session_lock_password("Secret");
        assert_ne!(hash_session_lock_password("secret"), other);

        let direct = format!("{:x}", Sha256::digest(b"secret"));
        assert_ne!(
            hash_session_lock_password("secret"),
            direct,
            "must be salted"
        );
    }
}
