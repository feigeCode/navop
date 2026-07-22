use one_core::settings::{
    AppSettings, LocalTerminalCustomProfile, LocalTerminalProfileKind, LocalTerminalProfileSettings,
};

use super::{
    local_config_from_settings, local_config_from_settings_with_profile, resolve_windows_profile,
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
            ..Default::default()
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
            ..Default::default()
        },
        ..AppSettings::default()
    };
    let config =
        local_config_from_settings_with_profile(&settings, LocalTerminalProfileKind::Custom, None)
            .unwrap();
    assert_eq!(Some("fish".to_string()), config.shell);
    assert_eq!(vec!["--login"], config.args);
}

#[test]
fn named_custom_profile_parses_a_full_command() {
    let profile = LocalTerminalCustomProfile {
        id: "fish-login".to_string(),
        name: "Fish Login".to_string(),
        command: "fish --login -C 'echo ready'".to_string(),
    };
    let config = super::local_config_from_custom_profile(&profile, None).unwrap();
    assert_eq!(Some("fish".to_string()), config.shell);
    assert_eq!(vec!["--login", "-C", "echo ready"], config.args);
}

#[cfg(target_os = "macos")]
#[test]
fn macos_app_bundle_resolves_to_its_internal_executable() {
    let temp = tempfile::tempdir().unwrap();
    let bundle = temp.path().join("Demo.app");
    let macos = bundle.join("Contents/MacOS");
    std::fs::create_dir_all(&macos).unwrap();
    std::fs::write(macos.join("demo-shell"), "#!/bin/sh\n").unwrap();
    let mut dictionary = plist::Dictionary::new();
    dictionary.insert(
        "CFBundleExecutable".to_string(),
        plist::Value::String("demo-shell".to_string()),
    );
    plist::Value::Dictionary(dictionary)
        .to_file_xml(bundle.join("Contents/Info.plist"))
        .unwrap();
    let profile = LocalTerminalCustomProfile {
        id: "demo".to_string(),
        name: "Demo".to_string(),
        command: bundle.to_string_lossy().into_owned(),
    };
    let config = super::local_config_from_custom_profile(&profile, None).unwrap();
    assert_eq!(
        Some(macos.join("demo-shell").to_string_lossy().into_owned()),
        config.shell
    );
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

#[cfg(not(target_os = "windows"))]
#[test]
fn windows_profiles_fall_back_to_system_shell_on_other_platforms() {
    for kind in [
        LocalTerminalProfileKind::PowerShell,
        LocalTerminalProfileKind::Cmd,
        LocalTerminalProfileKind::Wsl,
        LocalTerminalProfileKind::GitBash,
    ] {
        let profile = LocalTerminalProfileSettings {
            kind,
            ..LocalTerminalProfileSettings::default()
        };
        let (shell, args) = super::resolve_profile(&profile).unwrap();
        assert!(shell.is_none(), "{kind:?} should use the system shell");
        assert!(args.is_empty(), "{kind:?} should not pass shell arguments");
    }
}
