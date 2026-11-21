//! SFTP 连接中的 keyboard-interactive/MFA 弹窗。
//!
//! 底层认证 future 运行在 Tokio 上，不能直接操作 GPUI。这里通过有界生命周期
//! 的 responder + oneshot 把提示送回视图：保存的密码自动应答，验证码等动态
//! 提示由用户填写；配置了 TOTP 密钥时优先本地生成验证码。

use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};

use anyhow::{Result, anyhow};
use async_trait::async_trait;
use gpui::{
    AppContext as _, Context, Entity, IntoElement, ParentElement, Styled, Subscription, WeakEntity,
    Window, div, px,
};
use gpui_component::{
    ActiveTheme, WindowExt as _,
    button::{Button, ButtonVariants},
    dialog::DialogFooter,
    input::{Input, InputEvent, InputState},
    v_flex,
};
use otp::OtpSecret;
use rust_i18n::t;
use ssh::{
    KeyboardInteractivePrompt, KeyboardInteractiveRequest, KeyboardInteractiveResponder,
    KeyboardInteractiveTarget, SshAuth, SshConnectConfig,
};
use tokio::sync::mpsc::UnboundedSender;
use tokio::sync::oneshot;

use crate::SftpView;

/// 一次 MFA 提示属于哪条 SFTP 建连链路。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SftpMfaTarget {
    Main { generation: u64 },
    Left { connection_id: i64, generation: u64 },
}

/// 需要用户填写的单个 keyboard-interactive 提示。
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct SftpMfaPrompt {
    pub(crate) prompt: String,
    pub(crate) echo: bool,
}

/// 从 Tokio 认证任务送到 GPUI 的提示事件。
pub(crate) struct SftpMfaEvent {
    pub(crate) target: SftpMfaTarget,
    pub(crate) name: String,
    pub(crate) instructions: String,
    pub(crate) prompts: Vec<SftpMfaPrompt>,
    pub(crate) response_tx: oneshot::Sender<Vec<String>>,
}

pub(crate) struct SftpMfaInputs {
    target: SftpMfaTarget,
    name: String,
    instructions: String,
    prompts: Vec<SftpMfaPrompt>,
    inputs: Vec<Entity<InputState>>,
    response_tx: Option<oneshot::Sender<Vec<String>>>,
    error: Option<String>,
    _subscriptions: Vec<Subscription>,
}

/// 提供给底层 SFTP 客户端的 keyboard-interactive 应答器。
#[derive(Clone)]
pub(crate) struct SftpMfaResponder {
    target: SftpMfaTarget,
    event_tx: UnboundedSender<SftpMfaEvent>,
    jump_password: Option<String>,
    target_password: Option<String>,
    otp: Option<OtpSecret>,
    step_offset: Arc<AtomicU32>,
}

impl SftpMfaResponder {
    pub(crate) fn new(
        target: SftpMfaTarget,
        event_tx: UnboundedSender<SftpMfaEvent>,
        jump_password: Option<String>,
        target_password: Option<String>,
        otp: Option<OtpSecret>,
    ) -> Self {
        Self {
            target,
            event_tx,
            jump_password,
            target_password,
            otp,
            step_offset: Arc::new(AtomicU32::new(0)),
        }
    }

    fn password(&self, target: KeyboardInteractiveTarget) -> Option<&str> {
        match target {
            KeyboardInteractiveTarget::JumpServer => self.jump_password.as_deref(),
            KeyboardInteractiveTarget::TargetServer => self.target_password.as_deref(),
        }
    }
}

#[async_trait]
impl KeyboardInteractiveResponder for SftpMfaResponder {
    async fn respond(&self, request: KeyboardInteractiveRequest) -> Result<Vec<String>> {
        let password = self.password(request.target);
        let step_offset = self.step_offset.fetch_add(1, Ordering::Relaxed);
        if let Some(answers) = ssh::auto_answer_keyboard_interactive(
            &request,
            self.otp.as_ref(),
            password,
            step_offset,
            otp::current_unix_seconds(),
        ) {
            return Ok(answers);
        }

        let prompts = manual_prompts(&request.prompts, password);
        if prompts.is_empty() {
            return answers_from_manual_responses(&request.prompts, &[], password);
        }

        let (response_tx, response_rx) = oneshot::channel();
        self.event_tx
            .send(SftpMfaEvent {
                target: self.target,
                name: request.name.clone(),
                instructions: request.instructions.clone(),
                prompts,
                response_tx,
            })
            .map_err(|_| anyhow!(t!("Ssh.auth_keyboard_interactive_cancelled").to_string()))?;
        let responses = response_rx
            .await
            .map_err(|_| anyhow!(t!("Ssh.auth_keyboard_interactive_cancelled").to_string()))?;
        answers_from_manual_responses(&request.prompts, &responses, password)
    }
}

/// 只保留不能由保存的密码自动应答的提示。
fn manual_prompts(
    prompts: &[KeyboardInteractivePrompt],
    password: Option<&str>,
) -> Vec<SftpMfaPrompt> {
    prompts
        .iter()
        .filter(|prompt| !(ssh::is_password_prompt(&prompt.prompt) && password.is_some()))
        .map(|prompt| SftpMfaPrompt {
            prompt: prompt.prompt.clone(),
            echo: prompt.echo,
        })
        .collect()
}

/// 把用户填写的动态提示与保存的密码按服务器原始顺序合并。
fn answers_from_manual_responses(
    prompts: &[KeyboardInteractivePrompt],
    responses: &[String],
    password: Option<&str>,
) -> Result<Vec<String>> {
    let mut response_index = 0;
    let mut answers = Vec::with_capacity(prompts.len());

    for prompt in prompts {
        if ssh::is_password_prompt(&prompt.prompt) {
            if let Some(password) = password {
                answers.push(password.to_string());
            } else {
                let response = responses
                    .get(response_index)
                    .ok_or_else(|| anyhow!("SSH password response is missing"))?;
                if response.trim().is_empty() {
                    return Err(anyhow!("SSH password response is empty"));
                }
                answers.push(response.clone());
                response_index += 1;
            }
        } else {
            let response = responses
                .get(response_index)
                .ok_or_else(|| anyhow!("SSH MFA response is missing"))?;
            if response.trim().is_empty() {
                return Err(anyhow!("SSH MFA response is empty"));
            }
            answers.push(response.clone());
            response_index += 1;
        }
    }

    if response_index == responses.len() {
        Ok(answers)
    } else {
        Err(anyhow!("SSH MFA response count does not match prompts"))
    }
}

impl SftpView {
    pub(crate) fn receive_sftp_mfa_event(
        &mut self,
        event: SftpMfaEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.close_state.is_closing() || !self.sftp_mfa_target_is_current(event.target) {
            return;
        }
        if self.sftp_mfa_inputs.is_some() {
            self.pending_sftp_mfa_events.push_back(event);
            return;
        }
        self.show_sftp_mfa_event(event, window, cx);
    }

    fn show_next_sftp_mfa_event(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        while self.sftp_mfa_inputs.is_none() {
            let Some(event) = self.pending_sftp_mfa_events.pop_front() else {
                return;
            };
            if self.close_state.is_closing() || !self.sftp_mfa_target_is_current(event.target) {
                continue;
            }
            self.show_sftp_mfa_event(event, window, cx);
        }
    }

    fn show_sftp_mfa_event(
        &mut self,
        event: SftpMfaEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let target = event.target;
        let inputs = event
            .prompts
            .iter()
            .map(|prompt| {
                cx.new(|cx| {
                    let placeholder = if prompt.prompt.trim().is_empty() {
                        t!("Mfa.response_placeholder").to_string()
                    } else {
                        prompt.prompt.clone()
                    };
                    let state = InputState::new(window, cx).placeholder(placeholder);
                    if prompt.echo {
                        state
                    } else {
                        state.masked(true)
                    }
                })
            })
            .collect::<Vec<_>>();

        let mut subscriptions = Vec::new();
        for input in &inputs {
            subscriptions.push(cx.subscribe_in(
                input,
                window,
                move |this, _, event: &InputEvent, window, cx| {
                    if !is_primary_enter(event) {
                        return;
                    }
                    let submitted = this.submit_sftp_mfa(target, cx);
                    finish_sftp_mfa_submission(submitted, window, cx);
                },
            ));
        }

        if let Some(input) = inputs.first() {
            input.update(cx, |state, cx| state.focus(window, cx));
        }
        self.sftp_mfa_inputs = Some(SftpMfaInputs {
            target,
            name: event.name,
            instructions: event.instructions,
            prompts: event.prompts,
            inputs,
            response_tx: Some(event.response_tx),
            error: None,
            _subscriptions: subscriptions,
        });
        self.show_sftp_mfa_prompt(window, cx);
        cx.notify();
    }

    fn show_sftp_mfa_prompt(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(inputs) = self.sftp_mfa_inputs.as_ref() else {
            return;
        };
        if self.close_state.is_closing() || !self.sftp_mfa_target_is_current(inputs.target) {
            return;
        }

        let target = inputs.target;
        let name = inputs.name.clone();
        let instructions = inputs.instructions.clone();
        let prompts = inputs.prompts.clone();
        let input_states = inputs.inputs.clone();
        let error = inputs.error.clone();
        let view = cx.entity().downgrade();

        window.open_dialog(cx, move |dialog, _window, cx| {
            let mut content = v_flex().gap_3();
            if !instructions.trim().is_empty() {
                content = content.child(
                    div()
                        .text_sm()
                        .text_color(cx.theme().muted_foreground)
                        .child(instructions.clone()),
                );
            }
            for (index, prompt) in prompts.iter().enumerate() {
                let input = input_states[index].clone();
                content = content.child(v_flex().gap_1().child(if prompt.echo {
                    Input::new(&input)
                } else {
                    Input::new(&input).mask_toggle()
                }));
            }
            if let Some(error) = error.clone() {
                content = content.child(div().text_sm().text_color(cx.theme().danger).child(error));
            }

            dialog
                .title(if name.trim().is_empty() {
                    t!("Mfa.title").to_string()
                } else {
                    name.clone()
                })
                .w(px(460.))
                .child(content)
                .footer(sftp_mfa_prompt_footer(target, view.clone()))
                .overlay_closable(false)
                .close_button(false)
                .keyboard(false)
        });
    }

    pub(crate) fn submit_sftp_mfa(
        &mut self,
        target: SftpMfaTarget,
        cx: &mut Context<Self>,
    ) -> bool {
        if !self.sftp_mfa_target_is_current(target) {
            return false;
        }
        let Some(inputs) = self.sftp_mfa_inputs.as_mut() else {
            return false;
        };
        if inputs.target != target {
            return false;
        }
        let responses = inputs
            .inputs
            .iter()
            .map(|input| input.read(cx).text().to_string())
            .collect::<Vec<_>>();
        if responses.iter().any(|response| response.trim().is_empty()) {
            inputs.error = Some(t!("Mfa.response_required").to_string());
            return false;
        }

        let Some(response_tx) = inputs.response_tx.take() else {
            return false;
        };
        let _ = response_tx.send(responses);
        self.sftp_mfa_inputs = None;
        cx.notify();
        true
    }

    pub(crate) fn cancel_sftp_mfa(&mut self, target: SftpMfaTarget, cx: &mut Context<Self>) {
        let Some(inputs) = self.sftp_mfa_inputs.as_ref() else {
            return;
        };
        if inputs.target != target {
            return;
        }
        self.sftp_mfa_inputs = None;
        cx.notify();
    }

    pub(crate) fn clear_sftp_mfa_for_main(&mut self) {
        if self
            .sftp_mfa_inputs
            .as_ref()
            .is_some_and(|inputs| matches!(inputs.target, SftpMfaTarget::Main { .. }))
        {
            self.sftp_mfa_inputs = None;
        }
        self.pending_sftp_mfa_events
            .retain(|event| !matches!(event.target, SftpMfaTarget::Main { .. }));
    }

    pub(crate) fn clear_sftp_mfa_for_left(&mut self) {
        if self
            .sftp_mfa_inputs
            .as_ref()
            .is_some_and(|inputs| matches!(inputs.target, SftpMfaTarget::Left { .. }))
        {
            self.sftp_mfa_inputs = None;
        }
        self.pending_sftp_mfa_events
            .retain(|event| !matches!(event.target, SftpMfaTarget::Left { .. }));
    }

    fn sftp_mfa_target_is_current(&self, target: SftpMfaTarget) -> bool {
        match target {
            SftpMfaTarget::Main { generation } => self.is_current_connection_generation(generation),
            SftpMfaTarget::Left {
                connection_id,
                generation,
            } => {
                self.left_remote_id() == Some(connection_id)
                    && self.is_current_left_connection_generation(generation)
            }
        }
    }
}

fn sftp_mfa_prompt_footer(target: SftpMfaTarget, view: WeakEntity<SftpView>) -> DialogFooter {
    let cancel_view = view.clone();
    let submit_view = view;
    DialogFooter::new().children(vec![
        Button::new("sftp-mfa-cancel")
            .label(t!("Common.cancel").to_string())
            .on_click(move |_, window, cx| {
                window.close_dialog(cx);
                let _ = cancel_view.update(cx, |this, cx| {
                    this.cancel_sftp_mfa(target, cx);
                    cx.defer_in(window, |this, window, cx| {
                        this.show_next_sftp_mfa_event(window, cx);
                    });
                });
            })
            .into_any_element(),
        Button::new("sftp-mfa-submit")
            .label(t!("Mfa.submit").to_string())
            .primary()
            .on_click(move |_, window, cx| {
                let _ = submit_view.update(cx, |this, cx| {
                    let submitted = this.submit_sftp_mfa(target, cx);
                    finish_sftp_mfa_submission(submitted, window, cx);
                });
            })
            .into_any_element(),
    ])
}

fn finish_sftp_mfa_submission(submitted: bool, window: &mut Window, cx: &mut Context<SftpView>) {
    window.close_dialog(cx);
    cx.defer_in(window, move |this, window, cx| {
        if submitted {
            this.show_next_sftp_mfa_event(window, cx);
        } else {
            this.show_sftp_mfa_prompt(window, cx);
        }
    });
}

fn is_primary_enter(event: &InputEvent) -> bool {
    matches!(
        event,
        InputEvent::PressEnter {
            secondary: false,
            ..
        }
    )
}

pub(crate) fn password_from_ssh_auth(auth: &SshAuth) -> Option<String> {
    match auth {
        SshAuth::Password(password) => Some(password.clone()),
        SshAuth::Chain(steps) => steps.iter().find_map(password_from_ssh_auth),
        _ => None,
    }
}

pub(crate) fn jump_password(config: &SshConnectConfig) -> Option<String> {
    config
        .jump_server
        .as_ref()
        .and_then(|jump| password_from_ssh_auth(&jump.auth))
}

#[cfg(test)]
mod tests {
    use super::{
        SftpMfaEvent, SftpMfaPrompt, SftpMfaResponder, SftpMfaTarget,
        answers_from_manual_responses, manual_prompts,
    };
    use otp::OtpSecret;
    use ssh::{
        KeyboardInteractivePrompt, KeyboardInteractiveRequest, KeyboardInteractiveResponder,
        KeyboardInteractiveTarget,
    };
    use std::sync::Arc;

    fn request(prompts: &[&str]) -> KeyboardInteractiveRequest {
        KeyboardInteractiveRequest {
            target: KeyboardInteractiveTarget::TargetServer,
            name: "MFA".to_string(),
            instructions: String::new(),
            prompts: prompts
                .iter()
                .map(|prompt| KeyboardInteractivePrompt {
                    prompt: (*prompt).to_string(),
                    echo: false,
                })
                .collect(),
        }
    }

    #[test]
    fn saved_password_is_not_prompted_again() {
        let request = request(&["Password: ", "Verification code: "]);
        let prompts = manual_prompts(&request.prompts, Some("saved-password"));

        assert_eq!(
            prompts,
            vec![SftpMfaPrompt {
                prompt: "Verification code: ".to_string(),
                echo: false,
            }]
        );
    }

    #[test]
    fn manual_responses_are_merged_back_in_server_order() {
        let request = request(&["Verification code: ", "Password: ", "Answer: "]);
        let answers = answers_from_manual_responses(
            &request.prompts,
            &["123456".to_string(), "blue".to_string()],
            Some("saved-password"),
        )
        .expect("responses should map");

        assert_eq!(
            answers,
            vec![
                "123456".to_string(),
                "saved-password".to_string(),
                "blue".to_string(),
            ]
        );
    }

    #[test]
    fn missing_password_becomes_a_manual_prompt() {
        let request = request(&["Password: "]);
        let prompts = manual_prompts(&request.prompts, None);

        assert_eq!(
            prompts,
            vec![SftpMfaPrompt {
                prompt: "Password: ".to_string(),
                echo: false,
            }]
        );
    }

    #[tokio::test]
    async fn responder_bridges_manual_challenge_and_saved_password() {
        let (event_tx, mut event_rx) = tokio::sync::mpsc::unbounded_channel();
        let responder = Arc::new(SftpMfaResponder::new(
            SftpMfaTarget::Main { generation: 7 },
            event_tx,
            None,
            Some("saved-password".to_string()),
            None,
        ));
        let request = request(&["Verification code: ", "Password: "]);
        let responding = {
            let responder = responder.clone();
            tokio::spawn(async move { responder.respond(request).await })
        };

        let SftpMfaEvent {
            target,
            prompts,
            response_tx,
            ..
        } = event_rx.recv().await.expect("prompt event");
        assert_eq!(target, SftpMfaTarget::Main { generation: 7 });
        assert_eq!(
            prompts,
            vec![SftpMfaPrompt {
                prompt: "Verification code: ".to_string(),
                echo: false,
            }]
        );
        response_tx
            .send(vec!["123456".to_string()])
            .expect("response receiver should be live");

        assert_eq!(
            responding.await.expect("responder task").expect("answers"),
            vec!["123456".to_string(), "saved-password".to_string()]
        );
    }

    #[tokio::test]
    async fn configured_otp_is_answered_without_opening_a_prompt() {
        let (event_tx, mut event_rx) = tokio::sync::mpsc::unbounded_channel();
        let otp = OtpSecret::from_base32("JBSWY3DPEHPK3PXP").expect("valid test OTP secret");
        let responder = SftpMfaResponder::new(
            SftpMfaTarget::Main { generation: 9 },
            event_tx,
            None,
            Some("saved-password".to_string()),
            Some(otp),
        );

        let answers = responder
            .respond(request(&["Verification code: ", "Password: "]))
            .await
            .expect("configured OTP should be answered automatically");

        assert_eq!(answers.len(), 2);
        assert_eq!(answers[0].len(), 6);
        assert!(
            answers[0]
                .chars()
                .all(|character| character.is_ascii_digit())
        );
        assert_eq!(answers[1], "saved-password");
        assert!(
            event_rx.try_recv().is_err(),
            "自动 TOTP 不应生成手工输入弹窗事件"
        );
    }
}
