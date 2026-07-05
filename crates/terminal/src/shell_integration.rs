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
}
