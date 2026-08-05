use one_core::settings::{
    AppSettings, LocalTerminalCustomProfile, LocalTerminalProfileKind, LocalTerminalProfileSettings,
};

use super::{
    local_config_from_settings, local_config_from_settings_with_profile,
    resolve_git_bash_from_environment, resolve_windows_profile,
    resolve_windows_profile_with_git_bash,
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
fn windows_wsl_profile_resolves_the_wsl_command() {
    let (wsl, wsl_args) = resolve_windows_profile(LocalTerminalProfileKind::Wsl).unwrap();
    assert!(wsl.to_ascii_lowercase().ends_with("wsl.exe"));
    assert!(wsl_args.is_empty());
}

#[test]
fn windows_git_bash_profile_uses_the_resolved_command_and_login_arguments() {
    let expected = r"C:\Program Files\Git\bin\bash.exe".to_string();
    let (git_bash, git_bash_args) =
        resolve_windows_profile_with_git_bash(LocalTerminalProfileKind::GitBash, || {
            Ok(expected.clone())
        })
        .unwrap();
    assert_eq!(expected, git_bash);
    assert_eq!(vec!["--login", "-i"], git_bash_args);
}

#[test]
fn git_bash_resolution_supports_machine_and_per_user_installations() {
    let temp = tempfile::tempdir().unwrap();
    let machine_root = temp.path().join("Program Files");
    let machine_bash = machine_root.join("Git/bin/bash.exe");
    std::fs::create_dir_all(machine_bash.parent().unwrap()).unwrap();
    std::fs::write(&machine_bash, []).unwrap();

    let resolved = resolve_git_bash_from_environment(
        None,
        Some(machine_root.into_os_string()),
        None,
        None,
        None,
    )
    .unwrap();
    assert_eq!(machine_bash.to_string_lossy(), resolved);

    std::fs::remove_file(&machine_bash).unwrap();
    let local_app_data = temp.path().join("LocalAppData");
    let user_bash = local_app_data.join("Programs/Git/bin/bash.exe");
    std::fs::create_dir_all(user_bash.parent().unwrap()).unwrap();
    std::fs::write(&user_bash, []).unwrap();

    let resolved = resolve_git_bash_from_environment(
        None,
        None,
        None,
        Some(local_app_data.into_os_string()),
        None,
    )
    .unwrap();
    assert_eq!(user_bash.to_string_lossy(), resolved);
}

#[test]
fn git_bash_resolution_accepts_a_verified_git_for_windows_path_entry() {
    let temp = tempfile::tempdir().unwrap();
    let git_root = temp.path().join("PortableGit");
    let bash = git_root.join("bin/bash.exe");
    for file in [
        &bash,
        &git_root.join("cmd/git.exe"),
        &git_root.join("etc/profile"),
    ] {
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(file, []).unwrap();
    }

    let path = std::env::join_paths([git_root.join("bin")]).unwrap();
    let resolved = resolve_git_bash_from_environment(None, None, None, None, Some(path)).unwrap();
    assert_eq!(bash.to_string_lossy(), resolved);
}

#[test]
fn git_bash_resolution_follows_the_git_cmd_path_entry_to_bin_bash() {
    let temp = tempfile::tempdir().unwrap();
    let git_root = temp.path().join("Custom Git");
    let bash = git_root.join("bin/bash.exe");
    for file in [
        &bash,
        &git_root.join("cmd/git.exe"),
        &git_root.join("etc/profile"),
    ] {
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(file, []).unwrap();
    }

    let path = std::env::join_paths([git_root.join("cmd")]).unwrap();
    let resolved = resolve_git_bash_from_environment(None, None, None, None, Some(path)).unwrap();
    assert_eq!(bash.to_string_lossy(), resolved);
}

#[test]
fn git_bash_resolution_rejects_an_unrelated_bash_alias() {
    let temp = tempfile::tempdir().unwrap();
    let windows_apps = temp.path().join("Microsoft/WindowsApps");
    std::fs::create_dir_all(&windows_apps).unwrap();
    std::fs::write(windows_apps.join("bash.exe"), []).unwrap();

    let path = std::env::join_paths([windows_apps]).unwrap();
    let error = resolve_git_bash_from_environment(None, None, None, None, Some(path))
        .expect_err("an arbitrary bash.exe must not be used as Git Bash");
    assert!(error.to_string().contains("Git Bash was not found"));
}

#[test]
fn git_bash_resolution_fails_instead_of_launching_a_bare_bash_command() {
    let error = resolve_git_bash_from_environment(None, None, None, None, None)
        .expect_err("missing Git for Windows should be reported");
    assert!(error.to_string().contains("Git Bash was not found"));
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
