use std::sync::{Arc, Mutex};

use anyhow::{anyhow, Result};
use async_trait::async_trait;
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
