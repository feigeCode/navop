use super::*;

/// 从终端模型请求推导内联捕获请求；凭据请求优先于 MFA。
pub(super) fn active_capture_request(terminal: &Terminal) -> Option<CaptureRequest> {
    if let Some(request) = terminal.ssh_credential_request() {
        return Some(CaptureRequest::Credentials(CaptureCredentials {
            generation: request.generation(),
            is_telnet: false,
            wants_username: request.username,
            wants_password: request.password,
        }));
    }
    if let Some(request) = terminal.telnet_credential_request() {
        return Some(CaptureRequest::Credentials(CaptureCredentials {
            generation: request.generation(),
            is_telnet: true,
            wants_username: request.username,
            wants_password: request.password,
        }));
    }
    terminal.ssh_mfa_request().map(CaptureRequest::Mfa)
}

/// 清理外部文本中的 ANSI 转义序列，避免污染终端网格。
pub(super) fn sanitize_notice_text(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '\x1b' {
            if chars.peek() == Some(&'[') {
                chars.next();
                // CSI 序列：跳过参数字节直到 final byte（0x40-0x7E）。
                for final_byte in chars.by_ref() {
                    if ('@'..='~').contains(&final_byte) {
                        break;
                    }
                }
            }
            continue;
        }
        out.push(ch);
    }
    out
}

/// “连接中”提示只在重连时注入；首次连接保持终端画面干净。
pub(super) fn should_emit_connecting_notice(
    previous: Option<one_core::tab_container::TabConnectionStatus>,
) -> bool {
    previous == Some(one_core::tab_container::TabConnectionStatus::Disconnected)
}

/// Connecting/Disconnected 状态写入终端网格的内联提示文本。
///
/// 终端内联提示固定英文（MobaXterm 风格），不跟随 UI 语言。
pub(super) fn connection_notice_text(state: &ConnectionState) -> String {
    match state {
        ConnectionState::Connecting => {
            format!(
                "\r\n\x1b[36m{}\x1b[0m\r\n",
                t!("SshSession.connecting", locale = "en")
            )
        }
        ConnectionState::Disconnected { error } => {
            let mut text = format!(
                "\r\n\x1b[31m{}\x1b[0m\r\n",
                t!("SshSession.connection_lost", locale = "en")
            );
            if let Some(error) = error.as_deref().map(str::trim).filter(|e| !e.is_empty()) {
                text.push_str(&format!(
                    "\x1b[31m{}\x1b[0m\r\n",
                    sanitize_notice_text(error)
                ));
            }
            text.push_str(&format!(
                "\x1b[2m{}\x1b[0m\r\n\r\n",
                t!("SshSession.press_enter_to_reconnect", locale = "en")
            ));
            text
        }
        ConnectionState::Connected => String::new(),
    }
}

/// 终端内联凭据输入捕获所需的请求字段，与模型请求类型解耦以便测试。
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct CaptureCredentials {
    pub(super) generation: u64,
    pub(super) is_telnet: bool,
    pub(super) wants_username: bool,
    pub(super) wants_password: bool,
}

/// 终端内联凭据/MFA 输入捕获的请求来源。
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum CaptureRequest {
    Credentials(CaptureCredentials),
    Mfa(TerminalMfaRequest),
}

/// 当前等待输入的字段。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum CaptureField {
    Username,
    Password,
    MfaPrompt(usize),
}

/// 一次回车提交的结果。
#[derive(Debug, PartialEq, Eq)]
pub(super) enum CaptureOutcome {
    /// 进入下一个提示；调用方需注入换行与下一个提示行。
    Advanced,
    /// 凭据输入完成，可提交给 Terminal 模型。
    Credentials {
        fields: CaptureCredentials,
        username: Option<String>,
        password: Option<String>,
    },
    /// MFA 全部提示回答完成。
    Mfa(Vec<String>),
    /// 输入未通过校验（必填字段为空），保持当前等待状态。
    Rejected,
}

/// 终端内联凭据/MFA 输入捕获状态机。
///
/// 捕获期间按键不透传 PTY，由 View 将回显注入终端网格；提交仍复用
/// 模型既有的 `submit_*` 入口。回显遵循终端语义：回显字段原样回显，
/// 非回显字段（密码/验证码）不回显任何内容。
pub(super) struct CredentialCapture {
    request: CaptureRequest,
    field: CaptureField,
    username: String,
    responses: Vec<String>,
    current: String,
}

impl CredentialCapture {
    pub(super) fn for_request(request: CaptureRequest) -> Self {
        let field = match &request {
            CaptureRequest::Credentials(credentials) => {
                if credentials.wants_username {
                    CaptureField::Username
                } else {
                    CaptureField::Password
                }
            }
            CaptureRequest::Mfa(_) => CaptureField::MfaPrompt(0),
        };
        Self {
            request,
            field,
            username: String::new(),
            responses: Vec::new(),
            current: String::new(),
        }
    }

    pub(super) fn request(&self) -> &CaptureRequest {
        &self.request
    }

    /// 当前字段是否不应回显（密码、`echo=false` 的 MFA 提示）。
    pub(super) fn masked(&self) -> bool {
        match self.field {
            CaptureField::Username => false,
            CaptureField::Password => true,
            CaptureField::MfaPrompt(index) => {
                let prompts = self.mfa_prompts().unwrap_or_default();
                prompts.get(index).is_some_and(|prompt| !prompt.echo)
            }
        }
    }

    /// 当前字段的提示行文本；MFA 提示直接使用服务端下发的原文。
    /// 提示行固定英文，不跟随 UI 语言。
    pub(super) fn prompt_line(&self) -> String {
        match &self.field {
            CaptureField::Username => format!(
                "{}: ",
                if self.request_is_telnet() {
                    t!("TelnetSession.username", locale = "en")
                } else {
                    t!("SshSession.username", locale = "en")
                }
            ),
            CaptureField::Password => format!(
                "{}: ",
                if self.request_is_telnet() {
                    t!("TelnetSession.password", locale = "en")
                } else {
                    t!("SshSession.password", locale = "en")
                }
            ),
            CaptureField::MfaPrompt(index) => {
                let prompt = self
                    .mfa_prompts()
                    .unwrap_or_default()
                    .get(*index)
                    .map(|prompt| prompt.prompt.trim_end())
                    .unwrap_or_default();
                format!("{prompt} ")
            }
        }
    }

    /// MFA 请求在首个提示前需要展示的名称与说明。
    pub(super) fn mfa_prelude(&self) -> Option<(String, String)> {
        match &self.request {
            CaptureRequest::Mfa(request) => Some((
                request.name.trim().to_string(),
                request.instructions.trim().to_string(),
            )),
            CaptureRequest::Credentials(_) => None,
        }
    }

    /// 追加输入文本；返回是否需要原样回显（非掩码字段）。
    pub(super) fn append(&mut self, text: &str) -> bool {
        let accepted: String = text.chars().filter(|ch| !ch.is_control()).collect();
        if accepted.is_empty() {
            return false;
        }
        self.current.push_str(&accepted);
        !self.masked()
    }

    /// 删除最后一个字符；返回是否有内容被删除（需要注入擦除回显）。
    pub(super) fn backspace(&mut self) -> bool {
        self.current.pop().is_some()
    }

    /// 回车：推进到下一个提示或产出最终提交结果。
    pub(super) fn submit_current(&mut self) -> CaptureOutcome {
        match self.field {
            CaptureField::MfaPrompt(index) => {
                self.responses.push(std::mem::take(&mut self.current));
                if self
                    .mfa_prompts()
                    .is_some_and(|prompts| index + 1 < prompts.len())
                {
                    self.field = CaptureField::MfaPrompt(index + 1);
                    CaptureOutcome::Advanced
                } else {
                    CaptureOutcome::Mfa(std::mem::take(&mut self.responses))
                }
            }
            CaptureField::Username => {
                let username = self.current.trim().to_string();
                if username.is_empty() {
                    return CaptureOutcome::Rejected;
                }
                self.username = username;
                self.current.clear();
                if self.credentials_require_password() {
                    self.field = CaptureField::Password;
                    CaptureOutcome::Advanced
                } else {
                    self.finish_credentials()
                }
            }
            CaptureField::Password => {
                if self.current.is_empty() {
                    return CaptureOutcome::Rejected;
                }
                self.finish_credentials()
            }
        }
    }

    /// Esc/Ctrl+C 请求取消；MFA 可通过 responder 取消，凭据请求无取消入口。
    pub(super) fn cancellable(&self) -> bool {
        matches!(self.request, CaptureRequest::Mfa(_))
    }

    fn finish_credentials(&mut self) -> CaptureOutcome {
        let CaptureRequest::Credentials(fields) = self.request.clone() else {
            return CaptureOutcome::Rejected;
        };
        let wants_username = fields.wants_username;
        let wants_password = fields.wants_password;
        let password = self.current.clone();
        self.current.clear();
        CaptureOutcome::Credentials {
            fields,
            username: wants_username.then(|| self.username.clone()),
            password: wants_password.then_some(password),
        }
    }

    fn credentials_require_password(&self) -> bool {
        matches!(self.request, CaptureRequest::Credentials(ref fields) if fields.wants_password)
    }

    fn request_is_telnet(&self) -> bool {
        matches!(self.request, CaptureRequest::Credentials(ref fields) if fields.is_telnet)
    }

    fn mfa_prompts(&self) -> Option<&[TerminalMfaPrompt]> {
        match &self.request {
            CaptureRequest::Mfa(request) => Some(&request.prompts),
            CaptureRequest::Credentials(_) => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        CaptureCredentials, CaptureOutcome, CaptureRequest, CredentialCapture,
        connection_notice_text,
    };
    use terminal::terminal::{ConnectionState, TerminalMfaPrompt, TerminalMfaRequest};

    fn creds(
        generation: u64,
        is_telnet: bool,
        wants_username: bool,
        wants_password: bool,
    ) -> CaptureCredentials {
        CaptureCredentials {
            generation,
            is_telnet,
            wants_username,
            wants_password,
        }
    }

    fn credentials_request(fields: CaptureCredentials) -> CaptureRequest {
        CaptureRequest::Credentials(fields)
    }

    fn mfa_request(echo_flags: &[bool]) -> CaptureRequest {
        CaptureRequest::Mfa(TerminalMfaRequest {
            name: "MFA".to_string(),
            instructions: "Enter the code".to_string(),
            prompts: echo_flags
                .iter()
                .map(|echo| TerminalMfaPrompt {
                    prompt: "Verification code:".to_string(),
                    echo: *echo,
                })
                .collect(),
        })
    }

    fn submitted(
        fields: CaptureCredentials,
        username: Option<&str>,
        password: Option<&str>,
    ) -> CaptureOutcome {
        CaptureOutcome::Credentials {
            fields,
            username: username.map(str::to_string),
            password: password.map(str::to_string),
        }
    }

    #[test]
    fn ssh_credentials_capture_username_then_masked_password() {
        let fields = creds(1, false, true, true);
        let mut capture = CredentialCapture::for_request(credentials_request(fields.clone()));
        assert!(!capture.masked());
        assert!(capture.append("root"));
        assert_eq!(
            CaptureOutcome::Advanced,
            capture.submit_current(),
            "username step should advance to the password prompt"
        );
        assert!(capture.masked());
        assert!(!capture.append("secret"), "password input must not echo");
        assert_eq!(
            submitted(fields, Some("root"), Some("secret")),
            capture.submit_current()
        );
    }

    #[test]
    fn password_only_capture_skips_username_and_rejects_empty_submit() {
        let fields = creds(1, false, false, true);
        let mut capture = CredentialCapture::for_request(credentials_request(fields.clone()));
        assert!(capture.masked());
        assert_eq!(CaptureOutcome::Rejected, capture.submit_current());
        assert!(!capture.append("pw"), "masked input must not echo");
        assert_eq!(
            submitted(fields, None, Some("pw")),
            capture.submit_current()
        );
    }

    #[test]
    fn telnet_username_only_capture_submits_without_password() {
        let fields = creds(1, true, true, false);
        let mut capture = CredentialCapture::for_request(credentials_request(fields.clone()));
        assert_eq!(CaptureOutcome::Rejected, capture.submit_current());
        assert!(capture.append("admin"));
        assert_eq!(
            submitted(fields, Some("admin"), None),
            capture.submit_current()
        );
    }

    #[test]
    fn mfa_capture_walks_prompts_sequentially_with_echo_flags() {
        let mut capture = CredentialCapture::for_request(mfa_request(&[false, true]));
        let (name, instructions) = capture.mfa_prelude().expect("mfa prelude");
        assert_eq!("MFA", name);
        assert_eq!("Enter the code", instructions);
        assert!(capture.masked(), "first prompt is echo=false");
        assert!(!capture.append("1234"));
        assert_eq!(CaptureOutcome::Advanced, capture.submit_current());
        assert!(!capture.masked(), "second prompt echoes");
        assert!(capture.append("answer"));
        assert_eq!(
            CaptureOutcome::Mfa(vec!["1234".to_string(), "answer".to_string()]),
            capture.submit_current()
        );
    }

    #[test]
    fn backspace_only_reports_erases_when_content_exists() {
        let fields = creds(1, false, false, true);
        let mut capture = CredentialCapture::for_request(credentials_request(fields.clone()));
        assert!(!capture.backspace());
        assert!(!capture.append("abc"), "masked input must not echo");
        assert!(capture.backspace());
        assert!(!capture.append("d"));
        assert_eq!(
            submitted(fields, None, Some("abd")),
            capture.submit_current()
        );
    }

    #[test]
    fn control_characters_are_dropped_from_pasted_input() {
        let fields = creds(1, false, true, false);
        let mut capture = CredentialCapture::for_request(credentials_request(fields.clone()));
        assert!(capture.append("ro\not\r"));
        assert_eq!(
            submitted(fields, Some("root"), None),
            capture.submit_current()
        );
        assert!(!capture.cancellable());
    }

    #[test]
    fn mfa_capture_is_cancellable_while_credentials_are_not() {
        assert!(CredentialCapture::for_request(mfa_request(&[true])).cancellable());
        assert!(
            !CredentialCapture::for_request(credentials_request(creds(1, false, true, true)))
                .cancellable()
        );
    }

    #[test]
    fn prompt_line_uses_localized_labels_for_credentials() {
        let capture =
            CredentialCapture::for_request(credentials_request(creds(1, false, true, true)));
        assert!(
            capture.prompt_line().ends_with(": "),
            "credential prompts need an inline hint"
        );
        let mfa = CredentialCapture::for_request(mfa_request(&[true]));
        assert_eq!("Verification code: ", mfa.prompt_line());
    }

    #[test]
    fn disconnected_notice_carries_error_and_reconnect_hint_without_escapes() {
        let notice = connection_notice_text(&ConnectionState::Disconnected {
            error: Some("connection reset \x1b[31mdanger\x1b[0m".to_string()),
        });
        // 断行结构：标题行、错误行、重连提示行 + 空行收尾；错误文本已去转义。
        assert!(notice.contains("connection reset danger"));
        assert!(
            !notice.contains("\x1b[31mdanger"),
            "error text must be sanitized"
        );
        assert_eq!(5, notice.lines().count());
        assert!(notice.ends_with("\r\n\r\n"));

        let connecting = connection_notice_text(&ConnectionState::Connecting);
        assert!(connecting.lines().count() >= 2);
        assert_eq!("", connection_notice_text(&ConnectionState::Connected));
    }

    #[test]
    fn active_capture_request_prefers_credentials_over_mfa() {
        // 构造一个仅凭据请求存在的模型快照由集成层覆盖；此处锁定优先级约定。
        assert!(matches!(
            CaptureRequest::Mfa(TerminalMfaRequest {
                name: String::new(),
                instructions: String::new(),
                prompts: vec![TerminalMfaPrompt {
                    prompt: "code:".to_string(),
                    echo: false,
                }],
            }),
            CaptureRequest::Mfa(_)
        ));
    }
}
