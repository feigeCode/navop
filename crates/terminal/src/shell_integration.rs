pub(crate) fn normalized_shell_integration_script(script: &str) -> String {
    script.replace("\r\n", "\n").replace('\r', "\n")
}

pub(crate) fn embedded_shell_integration_script() -> String {
    normalized_shell_integration_script(include_str!("shell_integration.sh"))
}

#[cfg(test)]
mod tests {
    use super::{embedded_shell_integration_script, normalized_shell_integration_script};

    #[test]
    fn normalized_shell_integration_script_converts_crlf_to_lf() {
        assert_eq!(
            normalized_shell_integration_script("echo one\r\necho two\r\n"),
            "echo one\necho two\n"
        );
    }

    #[test]
    fn embedded_shell_integration_script_strips_carriage_returns() {
        let script = embedded_shell_integration_script();
        assert!(
            !script.contains('\r'),
            "嵌入式 shell integration 脚本不应保留 CR，避免远端 shell 解析失败"
        );
    }
}
