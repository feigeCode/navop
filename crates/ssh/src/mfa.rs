//! keyboard-interactive 认证中的一次性验证码（TOTP）自动应答。
//!
//! 不少双因素 SSH 主机在 keyboard-interactive 阶段先要密码、再要验证码
//! （PAM 的 `Verification code:`）。配置了 OTP 密钥的主机应当由客户端本地
//! 生成验证码直接应答，而不是每次都弹输入框让用户手输。

use otp::OtpSecret;

use crate::{KeyboardInteractivePrompt, KeyboardInteractiveRequest};

/// 服务端用于索要一次性验证码的提示标记（小写匹配）。
const OTP_PROMPT_MARKERS: &[&str] = &[
    "one-time password",
    "one time password",
    "otp",
    "verification code",
    "security code",
    "authentication code",
    "passcode",
    "token",
];

/// 判断提示是否为一次性验证码提示。
pub fn is_otp_prompt(prompt: &str) -> bool {
    let prompt = normalize_prompt(prompt);
    OTP_PROMPT_MARKERS
        .iter()
        .any(|marker| prompt.contains(marker))
}

/// 判断提示是否为密码提示。
///
/// 验证码提示里也可能带 `code` 之类的字眼，必须先按 OTP 提示排除，
/// 否则会把一次性验证码当成密码填进去。
pub fn is_password_prompt(prompt: &str) -> bool {
    if is_otp_prompt(prompt) {
        return false;
    }
    let prompt = normalize_prompt(prompt);
    prompt == "password"
        || prompt.ends_with("'s password")
        || prompt.starts_with("password for ")
        || prompt == "enter password"
}

/// 尝试用已保存的密码与 OTP 密钥自动应答整组提示。
///
/// 只有「所有提示都能自动应答」且请求里确实包含 OTP 提示时才返回应答；
/// 否则返回 `None`，交回调用方的交互式流程（弹输入框），避免劫持普通 MFA
/// 提示或漏填密码。
///
/// `step_offset` 用于连续认证时错开时间窗口（例如先跳板机后目标服务器，
/// 或者同一主机重连时前一个验证码尚未过期），每次调用递增即可；
/// `unix_now` 是当前 Unix 时间戳（秒）。
pub fn auto_answer_keyboard_interactive(
    request: &KeyboardInteractiveRequest,
    otp: Option<&OtpSecret>,
    password: Option<&str>,
    step_offset: u32,
    unix_now: u64,
) -> Option<Vec<String>> {
    let otp = otp?;
    if !request
        .prompts
        .iter()
        .any(|prompt| is_otp_prompt(&prompt.prompt))
    {
        return None;
    }

    let mut answers = Vec::with_capacity(request.prompts.len());
    let mut code: Option<String> = None;
    for prompt in &request.prompts {
        if is_otp_prompt(&prompt.prompt) {
            if code.is_none() {
                let offset_seconds = u64::from(step_offset) * otp.period.max(1);
                code = Some(otp.generate(unix_now.saturating_add(offset_seconds)).ok()?);
            }
            answers.push(code.clone()?);
        } else if is_password_prompt(&prompt.prompt) {
            answers.push(password?.to_string());
        } else {
            return None;
        }
    }
    Some(answers)
}

fn normalize_prompt(prompt: &str) -> String {
    prompt
        .trim()
        .trim_end_matches(|c: char| c == ':' || c.is_whitespace())
        .to_ascii_lowercase()
}

/// 提示里是否包含需要用户手动输入的项（即无法自动应答的项）。
pub fn needs_manual_input(prompts: &[KeyboardInteractivePrompt]) -> bool {
    prompts
        .iter()
        .any(|prompt| !is_otp_prompt(&prompt.prompt) && !is_password_prompt(&prompt.prompt))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::KeyboardInteractiveTarget;

    fn request(prompts: &[&str]) -> KeyboardInteractiveRequest {
        KeyboardInteractiveRequest {
            target: KeyboardInteractiveTarget::TargetServer,
            name: "Keyboard-interactive authentication".to_string(),
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

    fn otp() -> OtpSecret {
        // RFC 6238 SHA1 种子：ASCII "12345678901234567890"
        OtpSecret::parse("GEZDGNBVGY3TQOJQGEZDGNBVGY3TQOJQ").expect("valid base32")
    }

    #[test]
    fn recognizes_otp_prompts() {
        assert!(is_otp_prompt("Verification code: "));
        assert!(is_otp_prompt("One-time password:"));
        assert!(is_otp_prompt("OTP:"));
        assert!(is_otp_prompt("Enter passcode:"));
        assert!(!is_otp_prompt("Password:"));
        assert!(!is_otp_prompt("Password for alice@example.com:"));
    }

    #[test]
    fn password_prompt_detection_keeps_otp_apart() {
        assert!(is_password_prompt("Password: "));
        assert!(is_password_prompt("alice@example.com's password:"));
        assert!(is_password_prompt("Password for alice:"));
        assert!(!is_password_prompt("Verification code:"));
        assert!(!is_password_prompt("Passcode:"));
    }

    #[test]
    fn auto_answers_code_only_prompt() {
        let request = request(&["Verification code: "]);
        let answers = auto_answer_keyboard_interactive(&request, Some(&otp()), None, 0, 59)
            .expect("should auto answer");
        assert_eq!(answers, vec!["287082".to_string()]);
    }

    #[test]
    fn auto_answers_password_then_code() {
        let request = request(&["Password: ", "Verification code: "]);
        let answers =
            auto_answer_keyboard_interactive(&request, Some(&otp()), Some("secret"), 0, 59)
                .expect("should auto answer");
        assert_eq!(answers, vec!["secret".to_string(), "287082".to_string()]);
    }

    #[test]
    fn step_offset_shifts_the_time_window() {
        let request = request(&["Verification code: "]);
        let answers = auto_answer_keyboard_interactive(&request, Some(&otp()), None, 1, 59)
            .expect("should auto answer");
        // 59 + 30 = 89s → 窗口 60..90，与时间 1111111109 的 081804 不同，
        // 这里只断言与 offset 0 的结果不同，避免把测试绑死在具体窗口。
        assert_ne!(answers, vec!["287082".to_string()]);
        assert_eq!(answers, vec!["359152".to_string()]); // 时间 89 的 RFC 6238 SHA1 值
    }

    #[test]
    fn falls_back_without_otp_secret() {
        let request = request(&["Verification code: "]);
        assert_eq!(
            auto_answer_keyboard_interactive(&request, None, None, 0, 59),
            None
        );
    }

    #[test]
    fn falls_back_when_password_is_missing() {
        let request = request(&["Password: ", "Verification code: "]);
        assert_eq!(
            auto_answer_keyboard_interactive(&request, Some(&otp()), None, 0, 59),
            None
        );
    }

    #[test]
    fn does_not_hijack_prompts_without_otp() {
        let request = request(&["Password: ", "Second factor: "]);
        assert_eq!(
            auto_answer_keyboard_interactive(&request, Some(&otp()), Some("secret"), 0, 59),
            None
        );
    }

    #[test]
    fn detects_manual_prompts() {
        assert!(!needs_manual_input(
            &request(&["Password: ", "Verification code: "]).prompts
        ));
        assert!(needs_manual_input(
            &request(&["Answer my question: "]).prompts
        ));
    }
}
