#[cfg(any(test, target_os = "macos", target_os = "windows"))]
use anyhow::bail;
use anyhow::{Context, Result};
use one_core::settings::{
    AppSettings, LocalTerminalCustomProfile, LocalTerminalProfileKind, LocalTerminalProfileSettings,
};

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

pub fn local_config_from_custom_profile(
    profile: &LocalTerminalCustomProfile,
    working_dir: Option<String>,
) -> Result<LocalConfig> {
    let (shell, args) = resolve_custom_command(&profile.command)?;
    Ok(LocalConfig {
        shell: Some(shell),
        args,
        working_dir,
        ..LocalConfig::default()
    })
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
    let profile = profile
        .selected_custom_profile()
        .context("custom terminal profile is required")?;
    let (program, args) = resolve_custom_command(&profile.command)?;
    Ok((Some(program), args))
}

fn resolve_custom_command(command: &str) -> Result<(String, Vec<String>)> {
    let parts = shell_words::split(command.trim()).context("invalid custom terminal command")?;
    let (program, args) = parts
        .split_first()
        .context("custom terminal command is required")?;
    Ok((resolve_application_bundle(program)?, args.to_vec()))
}

#[cfg(target_os = "macos")]
fn resolve_application_bundle(program: &str) -> Result<String> {
    if !program.ends_with(".app") {
        return Ok(program.to_string());
    }
    let bundle = std::path::Path::new(program);
    let plist_path = bundle.join("Contents").join("Info.plist");
    let plist = plist::Value::from_file(&plist_path)
        .with_context(|| format!("failed to read {}", plist_path.display()))?;
    let executable = plist
        .as_dictionary()
        .and_then(|value| value.get("CFBundleExecutable"))
        .and_then(plist::Value::as_string)
        .context("application bundle has no CFBundleExecutable")?;
    let executable = bundle.join("Contents").join("MacOS").join(executable);
    if !executable.is_file() {
        bail!(
            "application bundle executable does not exist: {}",
            executable.display()
        );
    }
    Ok(executable.to_string_lossy().into_owned())
}

#[cfg(not(target_os = "macos"))]
fn resolve_application_bundle(program: &str) -> Result<String> {
    Ok(program.to_string())
}

#[cfg(target_os = "windows")]
fn resolve_builtin_profile(
    kind: LocalTerminalProfileKind,
) -> Result<(Option<String>, Vec<String>)> {
    match kind {
        LocalTerminalProfileKind::PowerShell
        | LocalTerminalProfileKind::Cmd
        | LocalTerminalProfileKind::Wsl
        | LocalTerminalProfileKind::GitBash => {
            let (command, args) = resolve_windows_profile(kind)?;
            Ok((Some(command), args))
        }
        LocalTerminalProfileKind::Zsh
        | LocalTerminalProfileKind::Bash
        | LocalTerminalProfileKind::Fish
        | LocalTerminalProfileKind::Nushell => {
            tracing::warn!("当前平台不支持所选 Unix 本地终端 profile，回退系统默认 shell");
            Ok((None, Vec::new()))
        }
        _ => Ok((None, Vec::new())),
    }
}

#[cfg(not(target_os = "windows"))]
fn resolve_builtin_profile(
    kind: LocalTerminalProfileKind,
) -> Result<(Option<String>, Vec<String>)> {
    match kind {
        LocalTerminalProfileKind::Zsh => Ok((Some(resolve_unix_shell("zsh")?), vec!["-l".into()])),
        LocalTerminalProfileKind::Bash => {
            Ok((Some(resolve_unix_shell("bash")?), vec!["--login".into()]))
        }
        LocalTerminalProfileKind::Fish => {
            Ok((Some(resolve_unix_shell("fish")?), vec!["--login".into()]))
        }
        LocalTerminalProfileKind::Nushell => Ok((Some(resolve_unix_shell("nu")?), Vec::new())),
        LocalTerminalProfileKind::PowerShell
        | LocalTerminalProfileKind::Cmd
        | LocalTerminalProfileKind::Wsl
        | LocalTerminalProfileKind::GitBash => {
            tracing::warn!("当前平台不支持所选 Windows 本地终端 profile，回退系统默认 shell");
            Ok((None, Vec::new()))
        }
        _ => Ok((None, Vec::new())),
    }
}

#[cfg(not(target_os = "windows"))]
fn resolve_unix_shell(program: &str) -> Result<String> {
    std::env::var_os("PATH")
        .as_deref()
        .into_iter()
        .flat_map(std::env::split_paths)
        .map(|dir| dir.join(program))
        .find(|path| path.is_file())
        .map(|path| path.to_string_lossy().into_owned())
        .with_context(|| format!("{program} was not found in PATH"))
}

#[cfg(any(test, target_os = "windows"))]
fn resolve_windows_profile(kind: LocalTerminalProfileKind) -> Result<(String, Vec<String>)> {
    match kind {
        LocalTerminalProfileKind::Zsh
        | LocalTerminalProfileKind::Bash
        | LocalTerminalProfileKind::Fish
        | LocalTerminalProfileKind::Nushell => bail!("profile is not a Windows terminal profile"),
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
#[path = "local_shell_tests.rs"]
mod tests;
