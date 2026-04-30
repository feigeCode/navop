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
    captured: CapturedMfaRequest,
}

impl JumpServerMfaResponder {
    pub fn new(responses: Vec<String>, captured: CapturedMfaRequest) -> Self {
        Self {
            responses,
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

        if mfa_responses_are_complete(&request.prompts, &self.responses) {
            return Ok(self.responses.clone());
        }

        self.captured.store(request);
        Err(anyhow!(JUMP_MFA_REQUIRED_MARKER))
    }
}

pub fn is_jump_mfa_required_error(error: &str) -> bool {
    error.contains(JUMP_MFA_REQUIRED_MARKER)
}

pub fn form_mfa_request_from_keyboard_interactive(
    request: &KeyboardInteractiveRequest,
) -> Option<FormMfaRequest> {
    if request.target != KeyboardInteractiveTarget::JumpServer || request.prompts.is_empty() {
        return None;
    }

    Some(FormMfaRequest {
        name: request.name.clone(),
        instructions: request.instructions.clone(),
        prompts: request
            .prompts
            .iter()
            .map(form_mfa_prompt_from_keyboard_interactive)
            .collect(),
    })
}

pub fn mfa_responses_are_complete(
    prompts: &[KeyboardInteractivePrompt],
    responses: &[String],
) -> bool {
    prompts.len() == responses.len()
        && responses
            .iter()
            .all(|response| !response.trim().is_empty())
}

fn form_mfa_prompt_from_keyboard_interactive(
    prompt: &KeyboardInteractivePrompt,
) -> FormMfaPrompt {
    FormMfaPrompt {
        prompt: prompt.prompt.clone(),
        echo: prompt.echo,
    }
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
    fn mfa_responses_require_one_non_empty_value_per_prompt() {
        let prompts = vec![prompt("Code:"), prompt("Backup:")];

        assert!(mfa_responses_are_complete(
            &prompts,
            &["123456".to_string(), "abcdef".to_string()]
        ));
        assert!(!mfa_responses_are_complete(
            &prompts,
            &["123456".to_string()]
        ));
        assert!(!mfa_responses_are_complete(
            &prompts,
            &["123456".to_string(), " ".to_string()]
        ));
    }

    #[test]
    fn form_mfa_request_only_accepts_jump_server_prompts() {
        let request = KeyboardInteractiveRequest {
            target: KeyboardInteractiveTarget::JumpServer,
            name: "Verification".to_string(),
            instructions: "Enter code".to_string(),
            prompts: vec![prompt("Code:")],
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
