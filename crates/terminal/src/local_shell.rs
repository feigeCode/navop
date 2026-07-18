use anyhow::{Context, Result, bail};
use one_core::settings::{AppSettings, LocalTerminalProfileKind, LocalTerminalProfileSettings};

use crate::LocalConfig;

pub fn local_config_from_settings(
    settings: &AppSettings,
    working_dir: Option<String>,
) -> Result<LocalConfig> {
    local_config_from_profile_settings(&settings.local_terminal_profile, working_dir)
}

pub fn local_config_from_settings_with_profile(
    settings: &AppSettings,
    profile_kind: LocalTerminalProfileKind,
    working_dir: Option<String>,
) -> Result<LocalConfig> {
    let mut profile = settings.local_terminal_profile.clone();
    profile.kind = profile_kind;
    local_config_from_profile_settings(&profile, working_dir)
}

fn local_config_from_profile_settings(
    profile: &LocalTerminalProfileSettings,
    working_dir: Option<String>,
) -> Result<LocalConfig> {
    let (shell, args) = resolve_profile(profile)?;
    Ok(LocalConfig {
        shell,
        args,
        working_dir,
        ..LocalConfig::default()
    })
}

fn resolve_profile(
    profile: &LocalTerminalProfileSettings,
) -> Result<(Option<String>, Vec<String>)> {
    match profile.kind {
        LocalTerminalProfileKind::System => Ok((None, Vec::new())),
        LocalTerminalProfileKind::Custom => resolve_custom_profile(profile),
        kind => resolve_builtin_profile(kind),
    }
}

fn resolve_custom_profile(
    profile: &LocalTerminalProfileSettings,
) -> Result<(Option<String>, Vec<String>)> {
    let program = profile.custom_program.trim();
    if program.is_empty() {
        bail!("custom terminal program is required");
    }
    let args = shell_words::split(profile.custom_arguments.trim())
        .context("invalid custom terminal arguments")?;
    Ok((Some(program.to_string()), args))
}

#[cfg(target_os = "windows")]
fn resolve_builtin_profile(
    kind: LocalTerminalProfileKind,
) -> Result<(Option<String>, Vec<String>)> {
    let (command, args) = resolve_windows_profile(kind)?;
    Ok((Some(command), args))
}

#[cfg(not(target_os = "windows"))]
fn resolve_builtin_profile(
    kind: LocalTerminalProfileKind,
) -> Result<(Option<String>, Vec<String>)> {
    match kind {
        LocalTerminalProfileKind::PowerShell => Ok((Some("pwsh".to_string()), Vec::new())),
        LocalTerminalProfileKind::Cmd
        | LocalTerminalProfileKind::Wsl
        | LocalTerminalProfileKind::GitBash => {
            tracing::warn!("当前平台不支持所选 Windows 本地终端 profile，回退系统默认 shell");
            Ok((None, Vec::new()))
        }
        _ => Ok((None, Vec::new())),
    }
}

#[cfg(any(test, target_os = "windows"))]
fn resolve_windows_profile(kind: LocalTerminalProfileKind) -> Result<(String, Vec<String>)> {
    match kind {
        LocalTerminalProfileKind::PowerShell => Ok((resolve_powershell(), Vec::new())),
        LocalTerminalProfileKind::Cmd => Ok((resolve_cmd(), Vec::new())),
        LocalTerminalProfileKind::Wsl => Ok((resolve_wsl(), Vec::new())),
        LocalTerminalProfileKind::GitBash => {
            Ok((resolve_git_bash(), vec!["--login".into(), "-i".into()]))
        }
        _ => bail!("profile is not a Windows terminal profile"),
    }
}

#[cfg(any(test, target_os = "windows"))]
fn resolve_powershell() -> String {
    find_in_path("pwsh.exe")
        .or_else(|| system32_path(&["WindowsPowerShell", "v1.0", "powershell.exe"]))
        .or_else(|| find_in_path("powershell.exe"))
        .unwrap_or_else(|| "powershell.exe".to_string())
}

#[cfg(any(test, target_os = "windows"))]
fn resolve_cmd() -> String {
    std::env::var_os("COMSPEC")
        .map(std::path::PathBuf::from)
        .filter(|path| path.is_file())
        .map(|path| path.to_string_lossy().into_owned())
        .or_else(|| system32_path(&["cmd.exe"]))
        .unwrap_or_else(|| "cmd.exe".to_string())
}

#[cfg(any(test, target_os = "windows"))]
fn resolve_wsl() -> String {
    system32_path(&["wsl.exe"])
        .or_else(|| find_in_path("wsl.exe"))
        .unwrap_or_else(|| "wsl.exe".to_string())
}

#[cfg(any(test, target_os = "windows"))]
fn resolve_git_bash() -> String {
    ["ProgramFiles", "ProgramFiles(x86)"]
        .into_iter()
        .filter_map(std::env::var_os)
        .map(std::path::PathBuf::from)
        .map(|root| root.join("Git").join("bin").join("bash.exe"))
        .find(|path| path.is_file())
        .map(|path| path.to_string_lossy().into_owned())
        .or_else(|| find_in_path("bash.exe"))
        .unwrap_or_else(|| "bash.exe".to_string())
}

#[cfg(any(test, target_os = "windows"))]
fn system32_path(parts: &[&str]) -> Option<String> {
    let mut path = std::path::PathBuf::from(std::env::var_os("SystemRoot")?);
    path.push("System32");
    path.extend(parts);
    path.is_file().then(|| path.to_string_lossy().into_owned())
}

#[cfg(any(test, target_os = "windows"))]
fn find_in_path(program: &str) -> Option<String> {
    std::env::var_os("PATH")
        .as_deref()
        .into_iter()
        .flat_map(std::env::split_paths)
        .map(|dir| dir.join(program))
        .find(|path| path.is_file())
        .map(|path| path.to_string_lossy().into_owned())
}

#[cfg(test)]
mod tests {
    use one_core::settings::{AppSettings, LocalTerminalProfileKind, LocalTerminalProfileSettings};

    use super::{
        local_config_from_settings, local_config_from_settings_with_profile,
        resolve_windows_profile,
    };

    #[test]
    fn system_profile_keeps_automatic_shell_resolution() {
        let settings = AppSettings::default();

        let config = local_config_from_settings(&settings, Some("/tmp".to_string())).unwrap();

        assert!(config.shell.is_none());
        assert!(config.args.is_empty());
        assert_eq!(Some("/tmp".to_string()), config.working_dir);
    }

    #[test]
    fn custom_profile_parses_program_and_quoted_arguments_without_shell_execution() {
        let settings = AppSettings {
            local_terminal_profile: LocalTerminalProfileSettings {
                kind: LocalTerminalProfileKind::Custom,
                custom_program: " /opt/homebrew/bin/fish ".to_string(),
                custom_arguments: "--login -C 'echo ready'".to_string(),
            },
            ..AppSettings::default()
        };

        let config = local_config_from_settings(&settings, None).unwrap();

        assert_eq!(Some("/opt/homebrew/bin/fish".to_string()), config.shell);
        assert_eq!(vec!["--login", "-C", "echo ready"], config.args);
    }

    #[test]
    fn custom_profile_rejects_missing_program_and_invalid_arguments() {
        let mut settings = AppSettings::default();
        settings.local_terminal_profile.kind = LocalTerminalProfileKind::Custom;
        assert!(local_config_from_settings(&settings, None).is_err());

        settings.local_terminal_profile.custom_program = "fish".to_string();
        settings.local_terminal_profile.custom_arguments = "'unterminated".to_string();
        assert!(local_config_from_settings(&settings, None).is_err());
    }

    #[test]
    fn temporary_profile_override_keeps_custom_command_settings() {
        let settings = AppSettings {
            local_terminal_profile: LocalTerminalProfileSettings {
                kind: LocalTerminalProfileKind::System,
                custom_program: "fish".to_string(),
                custom_arguments: "--login".to_string(),
            },
            ..AppSettings::default()
        };

        let config = local_config_from_settings_with_profile(
            &settings,
            LocalTerminalProfileKind::Custom,
            None,
        )
        .unwrap();

        assert_eq!(Some("fish".to_string()), config.shell);
        assert_eq!(vec!["--login"], config.args);
    }

    #[test]
    fn windows_profiles_resolve_wsl_and_git_bash_commands() {
        let (wsl, wsl_args) = resolve_windows_profile(LocalTerminalProfileKind::Wsl).unwrap();
        assert!(wsl.to_ascii_lowercase().ends_with("wsl.exe"));
        assert!(wsl_args.is_empty());

        let (git_bash, git_bash_args) =
            resolve_windows_profile(LocalTerminalProfileKind::GitBash).unwrap();
        assert!(git_bash.to_ascii_lowercase().ends_with("bash.exe"));
        assert_eq!(vec!["--login", "-i"], git_bash_args);
    }
}
