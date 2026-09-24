//! 左侧端点切换到「没有记住密码」的连接时，弹窗收集本次连接使用的凭据。
//!
//! 与主连接的内嵌凭据面板（`SftpCredentialInputs`）分开：左侧切换是弹窗，
//! 目标由 `connection_id` + `generation` 双重校验，避免切走后提交。

use crate::SftpView;
use crate::left_remote_state::LeftRemoteConnectionState;
use crate::ssh_config::{self, SshCredentialPromptPolicy};
use gpui::{Context, Entity, IntoElement, ParentElement, Styled, WeakEntity, Window, div, prelude::*, px};
use gpui_component::{
    ActiveTheme, WindowExt as _,
    button::{Button, ButtonVariants},
    dialog::DialogFooter,
    input::{Input, InputState},
    v_flex,
};
use rust_i18n::t;

/// 凭据弹窗对应的左侧连接目标。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct LeftCredentialPromptTarget {
    connection_id: i64,
    generation: u64,
}

impl LeftCredentialPromptTarget {
    fn is_current(self, view: &SftpView) -> bool {
        view.left_remote_id() == Some(self.connection_id)
            && view.is_current_left_connection_generation(self.generation)
    }
}

/// 左侧凭据弹窗的输入状态；只要弹窗可能重开就保留在视图上。
pub(crate) struct LeftCredentialInputs {
    pub(crate) target: LeftCredentialPromptTarget,
    pub(crate) policy: SshCredentialPromptPolicy,
    pub(crate) username: Option<Entity<InputState>>,
    pub(crate) password: Option<Entity<InputState>>,
    pub(crate) error: Option<String>,
}

/// 空用户名/密码的本地化提示；都合规时返回 `None`。
fn credential_validation_error(
    policy: SshCredentialPromptPolicy,
    username: Option<&str>,
    password: Option<&str>,
) -> Option<String> {
    if policy.username && username.is_some_and(|value| value.trim().is_empty()) {
        Some(t!("Credentials.username_required").to_string())
    } else if policy.password && password.is_some_and(str::is_empty) {
        Some(t!("Credentials.password_required").to_string())
    } else {
        None
    }
}

impl SftpView {
    /// 为左侧连接目标准备凭据输入，并弹出弹窗。
    pub(crate) fn open_left_credential_prompt(
        &mut self,
        connection_id: i64,
        policy: SshCredentialPromptPolicy,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.close_state.is_closing() {
            return;
        }
        let generation = self.next_left_connection_generation();
        let username = policy.username.then(|| {
            cx.new(|cx| InputState::new(window, cx).placeholder(t!("Credentials.username").to_string()))
        });
        let password = policy.password.then(|| {
            cx.new(|cx| {
                InputState::new(window, cx)
                    .placeholder(t!("Credentials.password").to_string())
                    .masked(true)
            })
        });
        self.left_credential_inputs = Some(LeftCredentialInputs {
            target: LeftCredentialPromptTarget {
                connection_id,
                generation,
            },
            policy,
            username: username.clone(),
            password: password.clone(),
            error: None,
        });
        if let Some(endpoint) = self.left_remote.as_mut() {
            endpoint.state = LeftRemoteConnectionState::AwaitingCredentials;
            endpoint.loading = false;
        }
        // 端点切换弹窗会在本帧稍后关闭自己，推迟一拍再开凭据弹窗，
        // 否则新弹窗会被那次关闭一并清掉。
        cx.defer_in(window, |this, window, cx| {
            this.show_left_credential_prompt(window, cx);
        });
        if let Some(input) = username.or(password) {
            input.update(cx, |state, cx| state.focus(window, cx));
        }
        cx.notify();
    }

    /// 打开（或重开）左侧凭据弹窗；没有待处理的目标时什么都不做。
    pub(crate) fn show_left_credential_prompt(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(inputs) = self.left_credential_inputs.as_ref() else {
            return;
        };
        if self.close_state.is_closing() || !inputs.target.is_current(self) {
            return;
        }
        let target = inputs.target;
        let username_input = inputs.username.clone();
        let password_input = inputs.password.clone();
        let error = inputs.error.clone();
        let view = cx.entity().downgrade();

        window.open_dialog(cx, move |dialog, _window, cx| {
            dialog
                .title(t!("Credentials.title").to_string())
                .w(px(420.))
                .child(
                    v_flex()
                        .gap_3()
                        .child(
                            div()
                                .text_sm()
                                .text_color(cx.theme().muted_foreground)
                                .child(t!("Credentials.hint").to_string()),
                        )
                        .when_some(username_input.clone(), |this, input| {
                            this.child(
                                v_flex()
                                    .gap_1()
                                    .child(
                                        div()
                                            .text_sm()
                                            .text_color(cx.theme().muted_foreground)
                                            .child(t!("Credentials.username").to_string()),
                                    )
                                    .child(Input::new(&input)),
                            )
                        })
                        .when_some(password_input.clone(), |this, input| {
                            this.child(
                                v_flex()
                                    .gap_1()
                                    .child(
                                        div()
                                            .text_sm()
                                            .text_color(cx.theme().muted_foreground)
                                            .child(t!("Credentials.password").to_string()),
                                    )
                                    .child(Input::new(&input).mask_toggle()),
                            )
                        })
                        .when_some(error.clone(), |this, error| {
                            this.child(
                                div().text_sm().text_color(cx.theme().danger).child(error),
                            )
                        }),
                )
                .footer(left_credential_prompt_footer(target, view.clone()))
                .overlay_closable(false)
                .close_button(false)
                .keyboard(false)
        });
    }

    /// 提交左侧凭据：注入运行时凭据后重连，凭据不落库。
    pub(crate) fn submit_left_credentials(
        &mut self,
        target: LeftCredentialPromptTarget,
        cx: &mut Context<Self>,
    ) {
        if self.close_state.is_closing() {
            return;
        }
        let Some(inputs) = self.left_credential_inputs.as_ref() else {
            return;
        };
        if inputs.target != target || !target.is_current(self) {
            return;
        }
        let policy = inputs.policy;
        let username = inputs
            .username
            .as_ref()
            .map(|input| input.read(cx).text().to_string());
        let password = inputs
            .password
            .as_ref()
            .map(|input| input.read(cx).text().to_string());

        if let Some(error) =
            credential_validation_error(policy, username.as_deref(), password.as_deref())
        {
            self.apply_left_credential_error(error, cx);
            return;
        }

        let applied = self
            .left_remote
            .as_mut()
            .map(|endpoint| {
                // FTP 模式下运行时凭据只作用于 FTP 配置；SSH 模式作用于 SSH 配置。
                if endpoint.remote_file_ftp.is_some() {
                    let base = endpoint
                        .remote_file_ftp
                        .clone()
                        .expect("FTP mode checked above");
                    ssh_config::ftp_config_with_runtime_credentials(
                        &base,
                        username.as_deref(),
                        password.as_deref(),
                    )
                    .map(|config| endpoint.remote_file_ftp = Some(config))
                } else {
                    ssh_config::ssh_config_with_runtime_credentials(
                        &endpoint.config,
                        username.as_deref(),
                        password.as_deref(),
                    )
                    .map(|config| endpoint.config = config)
                }
            });
        match applied {
            Some(Ok(())) => {
                if let Some(inputs) = self.left_credential_inputs.as_mut() {
                    inputs.error = None;
                }
                if let Some(endpoint) = self.left_remote.as_mut() {
                    endpoint.state = LeftRemoteConnectionState::Connecting;
                    endpoint.loading = false;
                }
                self.connect_left_remote(cx);
                // 建连会推进代次，同步弹窗目标，认证失败时才能回填错误重试。
                if let Some(inputs) = self.left_credential_inputs.as_mut() {
                    inputs.target.generation = self.left_connection_generation.current();
                }
                cx.notify();
            }
            Some(Err(error)) => {
                self.apply_left_credential_error(error.to_string(), cx);
            }
            None => {}
        }
    }

    /// 取消左侧凭据录入，回到本地端点，避免左侧停在只能靠弹窗恢复的状态。
    pub(crate) fn cancel_left_credential_prompt(
        &mut self,
        target: LeftCredentialPromptTarget,
        cx: &mut Context<Self>,
    ) {
        let Some(inputs) = self.left_credential_inputs.as_ref() else {
            return;
        };
        if inputs.target != target || !target.is_current(self) {
            return;
        }
        self.switch_left_to_local(cx);
    }

    /// 把错误回填到左侧凭据弹窗，等用户修正后重试；目标已失效时返回 `false`。
    pub(crate) fn apply_left_credential_error(
        &mut self,
        error: String,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(target) = self
            .left_credential_inputs
            .as_ref()
            .map(|inputs| inputs.target)
        else {
            return false;
        };
        if self.close_state.is_closing() || !target.is_current(self) {
            return false;
        }
        if let Some(inputs) = self.left_credential_inputs.as_mut() {
            inputs.error = Some(error);
        }
        if let Some(endpoint) = self.left_remote.as_mut() {
            endpoint.state = LeftRemoteConnectionState::AwaitingCredentials;
            endpoint.loading = false;
        }
        cx.notify();
        true
    }
}

fn left_credential_prompt_footer(
    target: LeftCredentialPromptTarget,
    view: WeakEntity<SftpView>,
) -> DialogFooter {
    let cancel_view = view.clone();
    let connect_view = view;
    DialogFooter::new().children(vec![
        Button::new("sftp-left-credential-cancel")
            .label(t!("Credentials.cancel").to_string())
            .on_click(move |_, window, cx| {
                window.close_dialog(cx);
                let _ = cancel_view.update(cx, |this, cx| {
                    this.cancel_left_credential_prompt(target, cx);
                });
            })
            .into_any_element(),
        Button::new("sftp-left-credential-connect")
            .label(t!("Credentials.connect").to_string())
            .primary()
            .on_click(move |_, window, cx| {
                window.close_dialog(cx);
                let _ = connect_view.update(cx, |this, cx| {
                    this.submit_left_credentials(target, cx);
                });
            })
            .into_any_element(),
    ])
}

#[cfg(test)]
mod tests {
    use super::credential_validation_error;
    use crate::ssh_config::SshCredentialPromptPolicy;

    #[test]
    fn blank_required_fields_are_reported_locally() {
        let both = SshCredentialPromptPolicy {
            username: true,
            password: true,
        };

        assert!(credential_validation_error(both, Some("  "), Some("secret")).is_some());
        assert!(credential_validation_error(both, Some("deploy"), Some("")).is_some());
        assert!(credential_validation_error(both, Some("deploy"), Some("secret")).is_none());
    }

    #[test]
    fn fields_outside_the_policy_are_not_validated() {
        let password_only = SshCredentialPromptPolicy {
            username: false,
            password: true,
        };

        assert!(credential_validation_error(password_only, Some(""), None).is_none());
        assert!(credential_validation_error(password_only, None, Some("")).is_some());
    }
}
