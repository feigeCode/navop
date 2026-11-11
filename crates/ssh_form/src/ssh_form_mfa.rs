use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};

use anyhow::{Result, anyhow};
use async_trait::async_trait;
use otp::OtpSecret;
use rust_i18n::t;
use ssh::{
    KeyboardInteractivePrompt, KeyboardInteractiveRequest, KeyboardInteractiveResponder,
    KeyboardInteractiveTarget,
};

const JUMP_MFA_REQUIRED_MARKER: &str = "__onetcli_jump_mfa_required__";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FormMfaPrompt {
    pub prompt: String,
    pub echo: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FormMfaRequest {
    pub name: String,
    pub instructions: String,
    pub prompts: Vec<FormMfaPrompt>,
}

#[derive(Clone, Default)]
pub struct CapturedMfaRequest {
    inner: Arc<Mutex<Option<KeyboardInteractiveRequest>>>,
}

impl CapturedMfaRequest {
    pub fn take(&self) -> Option<KeyboardInteractiveRequest> {
        self.inner.lock().ok()?.take()
    }

    fn store(&self, request: KeyboardInteractiveRequest) {
        if let Ok(mut inner) = self.inner.lock() {
            *inner = Some(request);
        }
    }
}

pub struct JumpServerMfaResponder {
    responses: Vec<String>,
    password: Option<String>,
    captured: CapturedMfaRequest,
}

impl JumpServerMfaResponder {
    pub fn new(
        responses: Vec<String>,
        password: Option<String>,
        captured: CapturedMfaRequest,
    ) -> Self {
        Self {
            responses,
            password,
            captured,
        }
    }
}

#[async_trait]
impl KeyboardInteractiveResponder for JumpServerMfaResponder {
    async fn respond(&self, request: KeyboardInteractiveRequest) -> Result<Vec<String>> {
        if request.target != KeyboardInteractiveTarget::JumpServer {
            return Err(anyhow!(
                t!("SSH.mfa_target_dynamic_not_supported").to_string()
            ));
        }

        if let Some(answers) = keyboard_interactive_answers(
            &request.prompts,
            &self.responses,
            self.password.as_deref(),
        ) {
            return Ok(answers);
        }

        self.captured.store(request);
        Err(anyhow!(JUMP_MFA_REQUIRED_MARKER))
    }
}

/// 连接测试阶段的一次性验证码应答器。
///
/// 主机配置了 TOTP 密钥后，keyboard-interactive 里的验证码提示由本地生成应答；
/// 跳板机的其它提示交给 `JumpServerMfaResponder`，继续走表单输入流程。
pub struct OtpAutoAnswerResponder {
    otp: OtpSecret,
    jump_password: Option<String>,
    target_password: Option<String>,
    jump_fallback: Option<JumpServerMfaResponder>,
    /// 同一次测试里多次索取验证码时递增，避免相邻 steps 生成同一个 6 位码。
    step_offset: AtomicU32,
}

impl OtpAutoAnswerResponder {
    pub fn new(
        otp: OtpSecret,
        jump_password: Option<String>,
        target_password: Option<String>,
        jump_fallback: Option<JumpServerMfaResponder>,
    ) -> Self {
        Self {
            otp,
            jump_password,
            target_password,
            jump_fallback,
            step_offset: AtomicU32::new(0),
        }
    }

    fn saved_password(&self, target: KeyboardInteractiveTarget) -> Option<&str> {
        match target {
            KeyboardInteractiveTarget::JumpServer => self.jump_password.as_deref(),
            KeyboardInteractiveTarget::TargetServer => self.target_password.as_deref(),
        }
    }
}

#[async_trait]
impl KeyboardInteractiveResponder for OtpAutoAnswerResponder {
    async fn respond(&self, request: KeyboardInteractiveRequest) -> Result<Vec<String>> {
        let step_offset = self.step_offset.fetch_add(1, Ordering::Relaxed);
        if let Some(answers) = ssh::auto_answer_keyboard_interactive(
            &request,
            Some(&self.otp),
            self.saved_password(request.target),
            step_offset,
            otp::current_unix_seconds(),
        ) {
            return Ok(answers);
        }

        match &self.jump_fallback {
            Some(fallback) => fallback.respond(request).await,
            None => Err(anyhow!(
                t!("SSH.mfa_target_dynamic_not_supported").to_string()
            )),
        }
    }
}

pub fn is_jump_mfa_required_error(error: &(dyn std::error::Error + 'static)) -> bool {
    error.to_string().contains(JUMP_MFA_REQUIRED_MARKER)
        || error.source().is_some_and(is_jump_mfa_required_error)
}

pub fn form_mfa_request_from_keyboard_interactive(
    request: &KeyboardInteractiveRequest,
) -> Option<FormMfaRequest> {
    if request.target != KeyboardInteractiveTarget::JumpServer || request.prompts.is_empty() {
        return None;
    }

    let prompts = request
        .prompts
        .iter()
        .filter(|prompt| !is_password_prompt(&prompt.prompt))
        .map(form_mfa_prompt_from_keyboard_interactive)
        .collect::<Vec<_>>();

    if prompts.is_empty() {
        return None;
    }

    Some(FormMfaRequest {
        name: request.name.clone(),
        instructions: request.instructions.clone(),
        prompts,
    })
}

pub fn keyboard_interactive_answers(
    prompts: &[KeyboardInteractivePrompt],
    responses: &[String],
    password: Option<&str>,
) -> Option<Vec<String>> {
    let mut response_index = 0;
    let mut answers = Vec::with_capacity(prompts.len());

    for prompt in prompts {
        if is_password_prompt(&prompt.prompt) {
            answers.push(password?.to_string());
        } else {
            let response = responses.get(response_index)?;
            if response.trim().is_empty() {
                return None;
            }
            answers.push(response.clone());
            response_index += 1;
        }
    }

    if response_index == responses.len()
        || prompts
            .iter()
            .all(|prompt| is_password_prompt(&prompt.prompt))
    {
        Some(answers)
    } else {
        None
    }
}

fn form_mfa_prompt_from_keyboard_interactive(prompt: &KeyboardInteractivePrompt) -> FormMfaPrompt {
    FormMfaPrompt {
        prompt: prompt.prompt.clone(),
        echo: prompt.echo,
    }
}

fn is_password_prompt(prompt: &str) -> bool {
    prompt
        .trim()
        .trim_end_matches(':')
        .eq_ignore_ascii_case("password")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn prompt(label: &str) -> KeyboardInteractivePrompt {
        KeyboardInteractivePrompt {
            prompt: label.to_string(),
            echo: false,
        }
    }

    #[test]
    fn keyboard_interactive_answers_merge_password_and_mfa_responses() {
        let prompts = vec![prompt("Password:"), prompt("Verification code:")];

        assert_eq!(
            Some(vec!["secret".to_string(), "123456".to_string()]),
            keyboard_interactive_answers(&prompts, &["123456".to_string()], Some("secret"))
        );
        assert_eq!(
            None,
            keyboard_interactive_answers(&prompts, &["123456".to_string()], None)
        );
        assert_eq!(
            None,
            keyboard_interactive_answers(&prompts, &[" ".to_string()], Some("secret"))
        );
    }

    #[test]
    fn keyboard_interactive_answers_allow_password_round_before_mfa_round() {
        assert_eq!(
            Some(vec!["secret".to_string()]),
            keyboard_interactive_answers(
                &[prompt("Password:")],
                &["123456".to_string()],
                Some("secret")
            )
        );
    }

    fn otp_secret() -> otp::OtpSecret {
        // RFC 6238 的 SHA1 测试种子（ASCII "12345678901234567890"）。
        otp::OtpSecret::from_base32("GEZDGNBVGY3TQOJQGEZDGNBVGY3TQOJQ").expect("测试密钥应有效")
    }

    #[tokio::test]
    async fn otp_responder_answers_verification_code_automatically() {
        let responder =
            OtpAutoAnswerResponder::new(otp_secret(), None, Some("secret".into()), None);
        let request = KeyboardInteractiveRequest {
            target: KeyboardInteractiveTarget::TargetServer,
            name: "Authentication".to_string(),
            instructions: String::new(),
            prompts: vec![prompt("Password:"), prompt("Verification code:")],
        };

        let answers = responder.respond(request).await.expect("应自动应答");

        assert_eq!(2, answers.len());
        assert_eq!("secret", answers[0]);
        assert_eq!(6, answers[1].len());
        assert!(answers[1].chars().all(|c| c.is_ascii_digit()));
    }

    #[tokio::test]
    async fn otp_responder_falls_back_to_jump_mfa_without_otp_prompt() {
        let captured = CapturedMfaRequest::default();
        let responder = OtpAutoAnswerResponder::new(
            otp_secret(),
            Some("jump-secret".into()),
            Some("secret".into()),
            Some(JumpServerMfaResponder::new(
                Vec::new(),
                Some("jump-secret".into()),
                captured.clone(),
            )),
        );
        let request = KeyboardInteractiveRequest {
            target: KeyboardInteractiveTarget::JumpServer,
            name: "Jump".to_string(),
            instructions: String::new(),
            prompts: vec![prompt("Additional info:")],
        };

        let error = responder
            .respond(request)
            .await
            .expect_err("缺少应答文本时应回到表单");

        assert!(is_jump_mfa_required_error(error.as_ref()));
        assert!(captured.take().is_some());
    }

    #[test]
    fn jump_mfa_required_error_matches_error_chain() {
        let error =
            anyhow!(JUMP_MFA_REQUIRED_MARKER).context("已取消 MFA/keyboard-interactive 二次认证");

        assert!(is_jump_mfa_required_error(error.as_ref()));
    }

    #[test]
    fn form_mfa_request_only_accepts_jump_server_prompts() {
        let request = KeyboardInteractiveRequest {
            target: KeyboardInteractiveTarget::JumpServer,
            name: "Verification".to_string(),
            instructions: "Enter code".to_string(),
            prompts: vec![prompt("Password:"), prompt("Code:")],
        };

        let form_request = form_mfa_request_from_keyboard_interactive(&request).unwrap();

        assert_eq!("Verification", form_request.name);
        assert_eq!("Enter code", form_request.instructions);
        assert_eq!(
            vec![FormMfaPrompt {
                prompt: "Code:".to_string(),
                echo: false,
            }],
            form_request.prompts
        );

        let target_request = KeyboardInteractiveRequest {
            target: KeyboardInteractiveTarget::TargetServer,
            ..request
        };
        assert!(form_mfa_request_from_keyboard_interactive(&target_request).is_none());
    }
}
