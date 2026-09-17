pub(crate) fn normalized_shell_integration_script(script: &str) -> String {
    script.replace("\r\n", "\n").replace('\r', "\n")
}

pub(crate) fn embedded_shell_integration_script() -> String {
    normalized_shell_integration_script(include_str!("shell_integration.sh"))
}

#[cfg(test)]
mod tests {
    use super::{embedded_shell_integration_script, normalized_shell_integration_script};
    use std::process::Command;

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

    #[test]
    fn bash_last_history_command_ignores_histtimeformat_prefix() {
        let bash = std::path::Path::new("/bin/bash");
        if !bash.exists() {
            return;
        }

        let script_path = std::env::temp_dir().join(format!(
            "onetcli-shell-integration-test-{}.sh",
            std::process::id()
        ));
        let script = embedded_shell_integration_script()
            .replace("[[ $- != *i* ]] && return", ":")
            .replace("[[ -n \"${_ONETCLI_SHELL_INTEGRATED:-}\" ]] && return", ":");
        std::fs::write(&script_path, script).expect("write shell integration script");

        let output = Command::new(bash)
            .arg("--noprofile")
            .arg("--norc")
            .arg("-c")
            .arg(format!(
                "source '{}'; trap - DEBUG; HISTFILE=/dev/null; \
                 HISTTIMEFORMAT='%F %T root '; set -o history; \
                 history -s 'cd /data/Seeyon/Comi/comi-install/config/nginx'; \
                 printf 'RESULT:%s\\n' \"$(__onetcli_last_history_command)\"",
                script_path.display()
            ))
            .output()
            .expect("run bash");
        let _ = std::fs::remove_file(&script_path);

        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(stdout.contains("RESULT:cd /data/Seeyon/Comi/comi-install/config/nginx"));
    }

    #[test]
    fn repeated_same_command_emits_record_each_time() {
        let bash = std::path::Path::new("/bin/bash");
        if !bash.exists() {
            return;
        }

        let script_path = std::env::temp_dir().join(format!(
            "onetcli-shell-integration-repeat-test-{}.sh",
            std::process::id()
        ));
        let script = embedded_shell_integration_script()
            .replace("[[ $- != *i* ]] && return", ":")
            .replace("[[ -n \"${_ONETCLI_SHELL_INTEGRATED:-}\" ]] && return", ":");
        std::fs::write(&script_path, script).expect("write shell integration script");

        let output = Command::new(bash)
            .arg("--noprofile")
            .arg("--norc")
            .arg("-c")
            .arg(format!(
                "source '{}'; trap - DEBUG; \
                 __onetcli_last_history_command() {{ printf 'git status'; }}; \
                 __onetcli_emit_recorded_command; __onetcli_emit_recorded_command",
                script_path.display()
            ))
            .output()
            .expect("run bash");
        let _ = std::fs::remove_file(&script_path);

        let stdout = String::from_utf8_lossy(&output.stdout);
        assert_eq!(2, stdout.matches("1337;Command=").count());
    }

    /// 返回 `[HH:MM:SS]` 时间戳在输出中的起始字节位置。
    fn timestamp_position(output: &str) -> Option<usize> {
        let bytes = output.as_bytes();
        bytes.windows(10).position(|window| {
            window[0] == b'['
                && window[3] == b':'
                && window[6] == b':'
                && window[9] == b']'
                && [1, 2, 4, 5, 7, 8]
                    .iter()
                    .all(|&index| window[index].is_ascii_digit())
        })
    }

    #[test]
    fn prompt_timestamp_appears_before_prompt_only_when_enabled() {
        let bash = std::path::Path::new("/bin/bash");
        if !bash.exists() {
            return;
        }

        let script_path = std::env::temp_dir().join(format!(
            "onetcli-shell-integration-timestamp-test-{}.sh",
            std::process::id()
        ));
        let script = embedded_shell_integration_script()
            .replace("[[ $- != *i* ]] && return", ":")
            .replace("[[ -n \"${_ONETCLI_SHELL_INTEGRATED:-}\" ]] && return", ":");
        std::fs::write(&script_path, script).expect("write shell integration script");

        let run_precmd = |timestamp_enabled: bool| {
            let mut command = Command::new(bash);
            command
                .arg("--noprofile")
                .arg("--norc")
                .arg("-c")
                .arg(format!(
                    "source '{}'; __onetcli_precmd_common 0",
                    script_path.display()
                ));
            if timestamp_enabled {
                command.env("_ONETCLI_TIMESTAMP", "1");
            }
            let output = command.output().expect("run bash");
            String::from_utf8_lossy(&output.stdout).to_string()
        };

        let enabled = run_precmd(true);
        let timestamp = timestamp_position(&enabled)
            .unwrap_or_else(|| panic!("开启后应打印 [HH:MM:SS]，实际输出: {enabled:?}"));
        let prompt_start = enabled.find("133;A").expect("应发出 133;A");
        assert!(
            timestamp < prompt_start,
            "时间戳应位于提示符标记 133;A 之前，实际输出: {enabled:?}"
        );

        let disabled = run_precmd(false);
        assert!(
            timestamp_position(&disabled).is_none() && disabled.contains("133;A"),
            "未开启时不应打印时间戳，但仍应发出 133;A，实际输出: {disabled:?}"
        );

        let _ = std::fs::remove_file(&script_path);
    }
}
