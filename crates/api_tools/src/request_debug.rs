use crate::http::{KeyValue, PreparedRequest};
use crate::scripting::ScriptResult;

pub(crate) fn actual_request_text(request: &PreparedRequest) -> String {
    let mut lines = vec![format!("{} {}", request.method, request.url)];
    lines.extend(
        request
            .headers
            .iter()
            .map(|(key, value)| format!("{key}: {value}")),
    );
    if !request.body.is_empty() {
        lines.push(String::new());
        lines.push(String::from_utf8_lossy(&request.body).into_owned());
    }
    lines.join("\n")
}

pub(crate) fn curl_command(request: &PreparedRequest) -> String {
    let mut parts = vec!["curl".to_string()];
    if request.method.label() != "GET" || !request.body.is_empty() {
        parts.push(format!("-X {}", request.method));
    }
    for (key, value) in &request.headers {
        parts.push(format!("-H {}", shell_quote(&format!("{key}: {value}"))));
    }
    if !request.body.is_empty() {
        parts.push(format!(
            "--data-raw {}",
            shell_quote(&String::from_utf8_lossy(&request.body))
        ));
    }
    parts.push(shell_quote(&request.url));
    parts.join(" \\\n  ")
}

fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\"'\"'"))
}

pub(crate) fn response_cookie_pair(header: &str) -> KeyValue {
    let pair = header.split(';').next().unwrap_or_default();
    let (name, value) = pair.split_once('=').unwrap_or((pair, ""));
    KeyValue::new(name.trim(), value.trim())
}

pub(crate) fn console_text(
    pre_result: Option<&ScriptResult>,
    test_result: Option<&ScriptResult>,
) -> String {
    let mut sections = Vec::new();
    append_script_result(&mut sections, "Pre-request", pre_result);
    append_script_result(&mut sections, "Tests", test_result);
    sections.join("\n\n")
}

fn append_script_result(sections: &mut Vec<String>, title: &str, result: Option<&ScriptResult>) {
    let Some(result) = result else {
        return;
    };
    let mut lines = vec![format!("── {title} ──")];
    lines.extend(result.logs.iter().cloned());
    if let Some(error) = &result.error {
        lines.push(format!("Error: {error}"));
    }
    if result.assertions_passed > 0 || result.assertions_failed > 0 {
        lines.push(format!(
            "{} passed, {} failed",
            result.assertions_passed, result.assertions_failed
        ));
    }
    sections.push(lines.join("\n"));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::http::RequestMethod;

    #[test]
    fn curl_command_escapes_quotes_and_includes_request_details() {
        let request = PreparedRequest {
            method: RequestMethod::Post,
            url: "https://example.com/users?name=O'Reilly".to_string(),
            headers: vec![("X-Trace".to_string(), "it's-safe".to_string())],
            body: br#"{"name":"O'Reilly"}"#.to_vec(),
        };

        let curl = curl_command(&request);
        assert!(curl.contains("-X POST"));
        assert!(curl.contains("-H 'X-Trace: it'\"'\"'s-safe'"));
        assert!(curl.contains("--data-raw"));
        assert!(curl.contains("'https://example.com/users?name=O'\"'\"'Reilly'"));
    }

    #[test]
    fn curl_command_keeps_get_method_when_request_has_a_body() {
        let request = PreparedRequest {
            method: RequestMethod::Get,
            url: "https://example.com/search".to_string(),
            headers: Vec::new(),
            body: br#"{"query":"navop"}"#.to_vec(),
        };

        let curl = curl_command(&request);
        assert!(curl.contains("-X GET"));
        assert!(curl.contains("--data-raw"));
    }

    #[test]
    fn actual_request_includes_body_after_blank_line() {
        let request = PreparedRequest {
            method: RequestMethod::Put,
            url: "https://example.com/users/1".to_string(),
            headers: vec![("Content-Type".to_string(), "application/json".to_string())],
            body: br#"{"active":true}"#.to_vec(),
        };

        assert_eq!(
            actual_request_text(&request),
            "PUT https://example.com/users/1\nContent-Type: application/json\n\n{\"active\":true}"
        );
    }

    #[test]
    fn response_cookie_ignores_attributes() {
        let cookie = response_cookie_pair("session=abc; Path=/; HttpOnly");
        assert_eq!(cookie.key, "session");
        assert_eq!(cookie.value, "abc");
    }

    #[test]
    fn console_text_includes_pre_and_test_results() {
        let pre = ScriptResult {
            logs: vec!["prepared".to_string()],
            error: Some("boom".to_string()),
            ..Default::default()
        };
        let tests = ScriptResult {
            logs: vec!["✓ PASS: status".to_string()],
            assertions_passed: 1,
            ..Default::default()
        };
        let text = console_text(Some(&pre), Some(&tests));
        assert!(text.contains("── Pre-request ──"));
        assert!(text.contains("Error: boom"));
        assert!(text.contains("── Tests ──"));
        assert!(text.contains("1 passed, 0 failed"));
    }
}
