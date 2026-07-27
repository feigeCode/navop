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
prompt $E]133;A$E\$E]7;file://localhost/$P$E\$P$G$E]133;B$E\
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
    if let Err(error) = write_script_if_changed(&path, script) {
        tracing::warn!("写入 Windows shell integration 失败: {error}");
        return (Vec::new(), Vec::new());
    }

    tracing::debug!("已配置 Windows {kind:?} Shell Integration");
    (
        vec![("ONETCLI_SHELL_INTEGRATION".into(), "1".into())],
        integration_args(kind, &path.to_string_lossy()),
    )
}

#[cfg(any(test, target_os = "windows"))]
fn write_script_if_changed(path: &Path, script: &str) -> std::io::Result<()> {
    // The integration files are process-scoped and identical for every new
    // terminal.  Avoid rewriting them on every launch, since writes to a
    // temporary .ps1/.cmd file may trigger another antivirus scan on Windows.
    if fs::read(path).is_ok_and(|current| current == script.as_bytes()) {
        return Ok(());
    }
    fs::write(path, script)
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
            // `-File` lets CreateProcess/Alacritty quote the path as one
            // argument.  Building a `-Command` string here would require
            // another layer of PowerShell quoting (and used to add avoidable
            // startup parsing work).
            "-NoLogo".into(),
            "-NoExit".into(),
            "-ExecutionPolicy".into(),
            "Bypass".into(),
            "-File".into(),
            path.into(),
        ],
        WindowsShellKind::Cmd => {
            // Keep the batch path as a separate argument.  Alacritty applies
            // C-runtime escaping when `escape_args` is enabled; embedding
            // quotes in a `/K` command string therefore turns them into
            // `\"`, which cmd.exe treats literally instead of as quoting.
            // With three arguments the final command line is the canonical:
            // `cmd.exe /K call "C:\\path with spaces\\script.cmd"`.
            vec!["/K".into(), "call".into(), path.into()]
        }
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
        assert!(script.contains("$E]133;A"));
        assert!(script.contains("$E]133;B"));
        assert!(script.contains("$E\\"));
    }

    #[test]
    fn powershell_arguments_keep_shell_open_after_loading_integration() {
        let args = integration_args(WindowsShellKind::PowerShell, r"C:\Temp\onetcli.ps1");

        assert_eq!(
            args,
            vec![
                "-NoLogo",
                "-NoExit",
                "-ExecutionPolicy",
                "Bypass",
                "-File",
                r"C:\Temp\onetcli.ps1"
            ]
        );
    }

    #[test]
    fn cmd_arguments_run_integration_batch_and_keep_shell_open() {
        let args = integration_args(WindowsShellKind::Cmd, r"C:\Temp\onetcli.cmd");

        assert_eq!(args, vec!["/K", "call", r"C:\Temp\onetcli.cmd"]);
    }

    #[test]
    fn cmd_arguments_keep_a_spaced_path_as_a_single_argument() {
        let path = r"C:\Users\Wang\AppData\Local\Temp\onetcli session\shell_integration.cmd";
        let args = integration_args(WindowsShellKind::Cmd, path);

        assert_eq!(args, vec!["/K", "call", path]);
        assert!(
            args.iter().all(|arg| !arg.contains('"')),
            "cmd integration arguments must not contain embedded quotes: {args:?}"
        );
    }

    #[test]
    fn powershell_arguments_keep_a_quoted_path_as_a_single_argument() {
        let path = r"C:\Users\Wang\AppData\Local\Temp\onetcli session\shell's.ps1";
        let args = integration_args(WindowsShellKind::PowerShell, path);

        assert_eq!(args.last().map(String::as_str), Some(path));
        assert!(
            !args.iter().any(|arg| arg == "-Command"),
            "PowerShell integration should not build a nested command string: {args:?}"
        );
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
        assert_eq!(args.first().map(String::as_str), Some("-NoLogo"));
        assert_eq!(
            args.last().map(String::as_str),
            Some(
                session_dir
                    .join("shell_integration.ps1")
                    .to_string_lossy()
                    .as_ref()
            )
        );
        assert!(session_dir.join("shell_integration.ps1").is_file());
        let _ = fs::remove_dir_all(session_dir);
    }

    #[test]
    fn reuses_an_unchanged_integration_script_without_rewriting_it() {
        let session_dir = std::env::temp_dir().join(format!(
            "onetcli-windows-integration-reuse-test-{}",
            std::process::id()
        ));
        let integration_path = session_dir.join("shell_integration.ps1");
        let _ = fs::remove_dir_all(&session_dir);
        fs::create_dir_all(&session_dir).expect("should create integration test directory");
        fs::write(&integration_path, powershell_integration_script())
            .expect("should seed the integration script");

        let mut permissions = fs::metadata(&integration_path)
            .expect("should read integration script metadata")
            .permissions();
        permissions.set_readonly(true);
        fs::set_permissions(&integration_path, permissions)
            .expect("should make integration script read-only");

        let (env, args) = prepare_in_dir("pwsh.exe", &session_dir);

        assert_eq!(env, vec![("ONETCLI_SHELL_INTEGRATION".into(), "1".into())]);
        assert_eq!(
            args.last().map(String::as_str),
            Some(integration_path.to_string_lossy().as_ref())
        );

        let mut permissions = fs::metadata(&integration_path)
            .expect("should read integration script metadata")
            .permissions();
        permissions.set_readonly(false);
        let _ = fs::set_permissions(&integration_path, permissions);
        let _ = fs::remove_dir_all(session_dir);
    }
}
