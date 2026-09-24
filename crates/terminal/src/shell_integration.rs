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

    /// 写出可 source 的 integration 脚本（去掉交互/幂等守卫，便于在测试里直接 source）。
    fn sourceable_script_for_test(suffix: &str) -> std::path::PathBuf {
        let script_path = std::env::temp_dir().join(format!(
            "onetcli-shell-integration-{}-{}.sh",
            suffix,
            std::process::id()
        ));
        let script = embedded_shell_integration_script()
            .replace("[[ $- != *i* ]] && return", ":")
            .replace("[[ -n \"${_ONETCLI_SHELL_INTEGRATED:-}\" ]] && return", ":");
        std::fs::write(&script_path, script).expect("write shell integration script");
        script_path
    }

    /// 取脚本注册到 PROMPT_COMMAND 的钩子文本。
    fn bash_prompt_command_from_script(script_path: &std::path::Path) -> String {
        let output = Command::new("/bin/bash")
            .arg("--noprofile")
            .arg("--norc")
            .arg("-c")
            .arg(format!(
                "source '{}'; printf '%s' \"$PROMPT_COMMAND\"",
                script_path.display()
            ))
            .output()
            .expect("run bash");
        String::from_utf8_lossy(&output.stdout).to_string()
    }

    /// issue #217：PROMPT_COMMAND 的值可能被外部环境导出（如服务器上的 `set -a`
    /// 或显式 export）并被任意子 shell 继承，而那些 shell 里并没有本文件的函数定义。
    /// 裸函数名钩子会让每个提示符都多输出一行“未找到命令”，钩子必须自带存在性判断。
    #[test]
    fn bash_prompt_hook_is_silent_when_integration_functions_are_absent() {
        if !std::path::Path::new("/bin/bash").exists() {
            return;
        }

        let script_path = sourceable_script_for_test("hook-export");
        let prompt_command = bash_prompt_command_from_script(&script_path);
        let _ = std::fs::remove_file(&script_path);

        assert!(
            prompt_command.contains("command -v __onetcli_precmd_bash"),
            "PROMPT_COMMAND 钩子应自带存在性判断，实际值: {prompt_command:?}"
        );

        let mut child = Command::new("/bin/bash")
            .arg("--noprofile")
            .arg("--norc")
            .arg("-i")
            .env("PROMPT_COMMAND", &prompt_command)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .expect("spawn interactive bash");
        {
            use std::io::Write;
            let stdin = child.stdin.as_mut().expect("child stdin");
            stdin
                .write_all(b"echo inherited-hook-check\nexit\n")
                .expect("write child stdin");
        }
        let output = child.wait_with_output().expect("wait interactive bash");
        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);

        assert!(
            !stdout.contains("__onetcli_precmd_bash") && !stderr.contains("__onetcli_precmd_bash"),
            "继承了 PROMPT_COMMAND 但没有 integration 函数的 shell 不应报错，stdout: {stdout:?} stderr: {stderr:?}"
        );
    }

    /// 钩子改成命令串后，仍必须把上一条命令的退出码传给 integration（133;D 不能恒为 0）。
    #[test]
    fn bash_prompt_hook_forwards_last_exit_code() {
        if !std::path::Path::new("/bin/bash").exists() {
            return;
        }

        let script_path = sourceable_script_for_test("hook-exit-code");
        let output = Command::new("/bin/bash")
            .arg("--noprofile")
            .arg("--norc")
            .arg("-c")
            .arg(format!(
                "source '{}'; trap - DEBUG; false; eval \"$PROMPT_COMMAND\"",
                script_path.display()
            ))
            .output()
            .expect("run bash");
        let _ = std::fs::remove_file(&script_path);

        // source 时注册的 DEBUG trap 会在 `trap - DEBUG` 生效前触发一次 133;C，与本用例无关。
        let stdout = String::from_utf8_lossy(&output.stdout)
            .strip_prefix("\u{1b}]133;C\u{7}")
            .map(str::to_string)
            .unwrap_or_else(|| String::from_utf8_lossy(&output.stdout).to_string());
        assert!(
            stdout.starts_with("\u{1b}]133;D;1\u{7}"),
            "PROMPT_COMMAND 钩子应先上报上一条命令的退出码、且自身不产生额外输出，stdout: {stdout:?}"
        );
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
