#[cfg(any(test, target_os = "windows"))]
use std::fs;
#[cfg(any(test, target_os = "windows"))]
use std::path::Path;

const POWERSHELL_SCRIPT: &str = r#"
if ($global:__OnetCliShellIntegrated) {
    return
}
$global:__OnetCliShellIntegrated = $true
$global:__OnetCliOriginalPrompt = $function:prompt

function global:__OnetCliWriteOsc([string] $Payload) {
    [Console]::Write(([char]27).ToString() + ']' + $Payload + [char]7)
}

function global:prompt {
    $path = (Get-Location).Path.Replace('\', '/')
    __OnetCliWriteOsc "7;file://localhost/$path"
    __OnetCliWriteOsc '133;A'

    if ($null -ne $global:__OnetCliOriginalPrompt) {
        $promptText = & $global:__OnetCliOriginalPrompt
    } else {
        $promptText = "PS $((Get-Location).Path)> "
    }

    __OnetCliWriteOsc '133;B'
    return $promptText
}
"#;

const CMD_SCRIPT: &str = r#"@echo off
prompt $E]7;file://localhost/$P$E\$P$G
"#;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum WindowsShellKind {
    PowerShell,
    Cmd,
    Unsupported,
}

#[cfg(target_os = "windows")]
pub(crate) fn prepare(program: &str) -> (Vec<(String, String)>, Vec<String>) {
    let session_dir = std::env::temp_dir().join(format!("onetcli-{}", std::process::id()));
    prepare_in_dir(program, &session_dir)
}

#[cfg(any(test, target_os = "windows"))]
fn prepare_in_dir(program: &str, session_dir: &Path) -> (Vec<(String, String)>, Vec<String>) {
    let kind = detect_shell_kind(program);
    let Some((extension, script)) = integration_file(kind) else {
        tracing::debug!("未知 Windows shell 类型 '{program}'，跳过 Shell Integration 注入");
        return (Vec::new(), Vec::new());
    };
    if let Err(error) = fs::create_dir_all(session_dir) {
        tracing::warn!("无法创建临时目录 {}: {error}", session_dir.display());
        return (Vec::new(), Vec::new());
    }
    let path = session_dir.join(format!("shell_integration.{extension}"));
    if let Err(error) = fs::write(&path, script) {
        tracing::warn!("写入 Windows shell integration 失败: {error}");
        return (Vec::new(), Vec::new());
    }

    tracing::debug!("已配置 Windows {kind:?} Shell Integration");
    (
        vec![("ONETCLI_SHELL_INTEGRATION".into(), "1".into())],
        integration_args(kind, &path.to_string_lossy()),
    )
}

fn detect_shell_kind(program: &str) -> WindowsShellKind {
    let normalized = program.replace('\\', "/").to_ascii_lowercase();
    let file_name = normalized.rsplit('/').next().unwrap_or(&normalized);
    match file_name {
        "powershell" | "powershell.exe" | "pwsh" | "pwsh.exe" => WindowsShellKind::PowerShell,
        "cmd" | "cmd.exe" => WindowsShellKind::Cmd,
        _ => WindowsShellKind::Unsupported,
    }
}

fn integration_file(kind: WindowsShellKind) -> Option<(&'static str, &'static str)> {
    match kind {
        WindowsShellKind::PowerShell => Some(("ps1", powershell_integration_script())),
        WindowsShellKind::Cmd => Some(("cmd", cmd_integration_script())),
        WindowsShellKind::Unsupported => None,
    }
}

fn integration_args(kind: WindowsShellKind, path: &str) -> Vec<String> {
    match kind {
        WindowsShellKind::PowerShell => vec![
            "-NoExit".into(),
            "-ExecutionPolicy".into(),
            "Bypass".into(),
            "-Command".into(),
            format!(". '{}'", path.replace('\'', "''")),
        ],
        WindowsShellKind::Cmd => vec!["/K".into(), format!("call \"{path}\"")],
        WindowsShellKind::Unsupported => Vec::new(),
    }
}

fn powershell_integration_script() -> &'static str {
    POWERSHELL_SCRIPT
}

fn cmd_integration_script() -> &'static str {
    CMD_SCRIPT
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn powershell_script_emits_current_directory_osc_event() {
        let script = powershell_integration_script();

        assert!(script.contains("Get-Location"));
        assert!(script.contains("file://localhost/"));
        assert!(script.contains("[char]27"));
        assert!(script.contains("[char]7"));
    }

    #[test]
    fn cmd_script_uses_dynamic_prompt_path_and_st_terminator() {
        let script = cmd_integration_script();

        assert!(script.contains("$P"));
        assert!(script.contains("$E]7;file://localhost/"));
        assert!(script.contains("$E\\"));
    }

    #[test]
    fn powershell_arguments_keep_shell_open_after_loading_integration() {
        let args = integration_args(WindowsShellKind::PowerShell, r"C:\Temp\onetcli.ps1");

        assert_eq!(
            args,
            vec![
                "-NoExit",
                "-ExecutionPolicy",
                "Bypass",
                "-Command",
                ". 'C:\\Temp\\onetcli.ps1'"
            ]
        );
    }

    #[test]
    fn cmd_arguments_run_integration_batch_and_keep_shell_open() {
        let args = integration_args(WindowsShellKind::Cmd, r"C:\Temp\onetcli.cmd");

        assert_eq!(args, vec!["/K", r#"call "C:\Temp\onetcli.cmd""#]);
    }

    #[test]
    fn detects_supported_windows_shell_executables() {
        assert_eq!(
            detect_shell_kind(r"C:\Program Files\PowerShell\7\pwsh.exe"),
            WindowsShellKind::PowerShell
        );
        assert_eq!(
            detect_shell_kind(r"C:\Windows\System32\cmd.exe"),
            WindowsShellKind::Cmd
        );
        assert_eq!(
            detect_shell_kind(r"C:\Tools\custom.exe"),
            WindowsShellKind::Unsupported
        );
    }

    #[test]
    fn prepares_powershell_integration_in_session_directory() {
        let session_dir = std::env::temp_dir().join(format!(
            "onetcli-windows-integration-test-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&session_dir);

        let (env, args) = prepare_in_dir("pwsh.exe", &session_dir);

        assert_eq!(env, vec![("ONETCLI_SHELL_INTEGRATION".into(), "1".into())]);
        assert_eq!(args.first().map(String::as_str), Some("-NoExit"));
        assert!(session_dir.join("shell_integration.ps1").is_file());
        let _ = fs::remove_dir_all(session_dir);
    }
}
