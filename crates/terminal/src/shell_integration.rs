pub(crate) fn normalized_shell_integration_script(script: &str) -> String {
    script.replace("\r\n", "\n").replace('\r', "\n")
}

pub(crate) fn embedded_shell_integration_script() -> String {
    normalized_shell_integration_script(include_str!("shell_integration.sh"))
}

#[cfg(test)]
mod tests {
    use super::{embedded_shell_integration_script, normalized_shell_integration_script};
    #[cfg(unix)]
    use std::{fs, os::unix::fs::PermissionsExt, process::Command};

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
    fn embedded_shell_integration_script_enables_vim_mouse() {
        let script = embedded_shell_integration_script();
        assert!(script.contains("--cmd 'set mouse=a'"));
        assert!(script.contains("--cmd 'nnoremap <ScrollWheelUp> gkzz'"));
        assert!(script.contains("--cmd 'nnoremap <ScrollWheelDown> gjzz'"));
        assert!(script.contains("function vim {"));
        assert!(script.contains("__onetcli_can_wrap_command vim"));
    }

    #[cfg(unix)]
    #[test]
    fn bash_vim_wrapper_preserves_args() {
        assert_vim_wrapper_preserves_args("bash");
    }

    #[cfg(unix)]
    #[test]
    fn zsh_vim_wrapper_preserves_args() {
        if !shell_available("zsh") {
            return;
        }
        assert_vim_wrapper_preserves_args("zsh");
    }

    #[cfg(unix)]
    #[test]
    fn bash_vim_wrapper_does_not_override_alias() {
        assert_vim_wrapper_does_not_override_alias("bash");
    }

    #[cfg(unix)]
    #[test]
    fn zsh_vim_wrapper_does_not_override_alias() {
        if !shell_available("zsh") {
            return;
        }
        assert_vim_wrapper_does_not_override_alias("zsh");
    }

    #[cfg(unix)]
    #[test]
    fn bash_vim_wrapper_does_not_override_function() {
        assert_vim_wrapper_does_not_override_function("bash");
    }

    #[cfg(unix)]
    #[test]
    fn zsh_vim_wrapper_does_not_override_function() {
        if !shell_available("zsh") {
            return;
        }
        assert_vim_wrapper_does_not_override_function("zsh");
    }

    #[cfg(unix)]
    #[test]
    fn bash_vim_mouse_can_be_disabled() {
        assert_vim_mouse_can_be_disabled("bash");
    }

    #[cfg(unix)]
    #[test]
    fn zsh_vim_mouse_can_be_disabled() {
        if !shell_available("zsh") {
            return;
        }
        assert_vim_mouse_can_be_disabled("zsh");
    }

    #[cfg(unix)]
    #[test]
    fn bash_nvim_wrapper_preserves_args() {
        assert_nvim_wrapper_preserves_args("bash");
    }

    #[cfg(unix)]
    #[test]
    fn zsh_nvim_wrapper_preserves_args() {
        if !shell_available("zsh") {
            return;
        }
        assert_nvim_wrapper_preserves_args("zsh");
    }

    #[cfg(unix)]
    fn assert_vim_wrapper_preserves_args(shell: &str) {
        let output = run_interactive_shell(
            shell,
            "source \"$ONETCLI_TEST_SCRIPT\"\nvim 'a b.txt' -- '--weird;$HOME'",
        );

        assert!(output.status.success());
        assert_eq!(
            strip_shell_integration_osc(&String::from_utf8_lossy(&output.stdout)),
            "--cmd\nset mouse=a\n--cmd\nnnoremap <ScrollWheelUp> gkzz\n--cmd\nnnoremap <ScrollWheelDown> gjzz\n--cmd\ninoremap <ScrollWheelUp> <C-o>gk<C-o>zz\n--cmd\ninoremap <ScrollWheelDown> <C-o>gj<C-o>zz\na b.txt\n--\n--weird;$HOME\n"
        );
    }

    #[cfg(unix)]
    fn assert_vim_wrapper_does_not_override_alias(shell: &str) {
        let output = run_interactive_shell(
            shell,
            "shopt -s expand_aliases 2>/dev/null || true\nalias vim='echo alias-safe'\nsource \"$ONETCLI_TEST_SCRIPT\"\nvim",
        );

        assert!(output.status.success());
        assert_eq!(
            strip_shell_integration_osc(&String::from_utf8_lossy(&output.stdout)),
            "alias-safe\n"
        );
    }

    #[cfg(unix)]
    fn assert_vim_wrapper_does_not_override_function(shell: &str) {
        let output = run_interactive_shell(
            shell,
            "vim() { echo function-safe; }\nsource \"$ONETCLI_TEST_SCRIPT\"\nvim",
        );

        assert!(output.status.success());
        assert_eq!(
            strip_shell_integration_osc(&String::from_utf8_lossy(&output.stdout)),
            "function-safe\n"
        );
    }

    #[cfg(unix)]
    fn assert_vim_mouse_can_be_disabled(shell: &str) {
        let output = run_interactive_shell(
            shell,
            "source \"$ONETCLI_TEST_SCRIPT\"\nONETCLI_VIM_MOUSE=0 vim file.txt",
        );

        assert!(output.status.success());
        assert_eq!(
            strip_shell_integration_osc(&String::from_utf8_lossy(&output.stdout)),
            "file.txt\n"
        );
    }

    #[cfg(unix)]
    fn assert_nvim_wrapper_preserves_args(shell: &str) {
        let output = run_interactive_shell(shell, "source \"$ONETCLI_TEST_SCRIPT\"\nnvim file.txt");

        assert!(output.status.success());
        assert_eq!(
            strip_shell_integration_osc(&String::from_utf8_lossy(&output.stdout)),
            "--cmd\nset mouse=a\n--cmd\nnnoremap <ScrollWheelUp> gkzz\n--cmd\nnnoremap <ScrollWheelDown> gjzz\n--cmd\ninoremap <ScrollWheelUp> <C-o>gk<C-o>zz\n--cmd\ninoremap <ScrollWheelDown> <C-o>gj<C-o>zz\nfile.txt\n"
        );
    }

    #[cfg(unix)]
    fn shell_available(shell: &str) -> bool {
        let available = Command::new(shell).arg("--version").output().is_ok();
        if !available {
            eprintln!("跳过 {shell} 行为测试：当前环境未安装该 shell");
        }
        available
    }

    #[cfg(unix)]
    fn run_interactive_shell(shell: &str, command: &str) -> std::process::Output {
        let temp_dir = std::env::temp_dir().join(format!(
            "onetcli-shell-integration-test-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let bin_dir = temp_dir.join("bin");
        let home_dir = temp_dir.join("home");
        let zdot_dir = temp_dir.join("zsh");
        fs::create_dir_all(&bin_dir).expect("应创建测试 bin 目录");
        fs::create_dir_all(&home_dir).expect("应创建测试 HOME 目录");
        fs::create_dir_all(&zdot_dir).expect("应创建测试 ZDOTDIR 目录");

        let script_path = temp_dir.join("shell_integration.sh");
        fs::write(&script_path, embedded_shell_integration_script()).expect("应写入集成脚本");

        write_fake_editor(&bin_dir.join("vim"));
        write_fake_editor(&bin_dir.join("nvim"));

        let command_path = temp_dir.join("command.sh");
        fs::write(&command_path, command).expect("应写入测试命令脚本");

        let path = format!(
            "{}:{}",
            bin_dir.display(),
            std::env::var("PATH").unwrap_or_default()
        );
        let output = Command::new(shell)
            .arg("-i")
            .arg(&command_path)
            .env("PATH", path)
            .env("HOME", &home_dir)
            .env("ZDOTDIR", &zdot_dir)
            .env("ONETCLI_TEST_SCRIPT", &script_path)
            .output()
            .expect("应执行 shell 行为测试");

        let _ = fs::remove_dir_all(&temp_dir);
        output
    }

    #[cfg(unix)]
    fn write_fake_editor(path: &std::path::Path) {
        fs::write(
            path,
            "#!/bin/sh\nfor arg in \"$@\"; do printf '%s\\n' \"$arg\"; done\n",
        )
        .expect("应写入 fake editor");
        fs::set_permissions(path, fs::Permissions::from_mode(0o755))
            .expect("应设置 fake editor 可执行权限");
    }

    #[cfg(unix)]
    fn strip_shell_integration_osc(output: &str) -> String {
        output
            .replace("\u{1b}]133;C\u{7}", "")
            .replace("\u{1b}]133;D;0\u{7}", "")
            .replace("\u{1b}]133;A\u{7}", "")
            .replace("\u{1b}]133;B\u{7}", "")
    }
}
