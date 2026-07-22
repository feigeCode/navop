use gpui::SharedString;
use one_core::settings::{
    LocalTerminalCustomProfile, LocalTerminalProfileKind, LocalTerminalProfileSettings,
};
use rust_i18n::t;

pub(crate) fn kinds(include_windows: bool) -> Vec<LocalTerminalProfileKind> {
    let mut kinds = vec![LocalTerminalProfileKind::System];
    if include_windows {
        kinds.extend([
            LocalTerminalProfileKind::PowerShell,
            LocalTerminalProfileKind::Cmd,
            LocalTerminalProfileKind::Wsl,
            LocalTerminalProfileKind::GitBash,
        ]);
    } else {
        kinds.extend([
            LocalTerminalProfileKind::Zsh,
            LocalTerminalProfileKind::Bash,
            LocalTerminalProfileKind::Fish,
            LocalTerminalProfileKind::Nushell,
        ]);
    }
    kinds.push(LocalTerminalProfileKind::Custom);
    kinds
}

pub(crate) fn effective_kind(
    configured: LocalTerminalProfileKind,
    include_windows: bool,
) -> LocalTerminalProfileKind {
    let available = if configured == LocalTerminalProfileKind::Custom {
        true
    } else {
        builtin_kinds(include_windows).contains(&configured)
    };
    if available {
        configured
    } else {
        LocalTerminalProfileKind::System
    }
}

fn builtin_kinds(include_windows: bool) -> Vec<LocalTerminalProfileKind> {
    kinds(include_windows)
        .into_iter()
        .filter(|kind| *kind != LocalTerminalProfileKind::Custom)
        .collect()
}

pub(crate) fn label(kind: LocalTerminalProfileKind) -> String {
    match kind {
        LocalTerminalProfileKind::System => t!("Settings.General.LocalTerminal.system").to_string(),
        LocalTerminalProfileKind::Zsh => t!("Settings.General.LocalTerminal.zsh").to_string(),
        LocalTerminalProfileKind::Bash => t!("Settings.General.LocalTerminal.bash").to_string(),
        LocalTerminalProfileKind::Fish => t!("Settings.General.LocalTerminal.fish").to_string(),
        LocalTerminalProfileKind::Nushell => {
            t!("Settings.General.LocalTerminal.nushell").to_string()
        }
        LocalTerminalProfileKind::PowerShell => {
            t!("Settings.General.LocalTerminal.powershell").to_string()
        }
        LocalTerminalProfileKind::Cmd => t!("Settings.General.LocalTerminal.cmd").to_string(),
        LocalTerminalProfileKind::Wsl => t!("Settings.General.LocalTerminal.wsl").to_string(),
        LocalTerminalProfileKind::GitBash => {
            t!("Settings.General.LocalTerminal.git_bash").to_string()
        }
        LocalTerminalProfileKind::Custom => t!("Settings.General.LocalTerminal.custom").to_string(),
    }
}

pub(crate) fn setting_options(include_windows: bool) -> Vec<(SharedString, SharedString)> {
    kinds(include_windows)
        .into_iter()
        .map(|kind| (kind.as_str().into(), label(kind).into()))
        .collect()
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum LocalTerminalLaunchTarget {
    Builtin(LocalTerminalProfileKind),
    Custom(LocalTerminalCustomProfile),
}

pub(crate) fn launch_options(
    include_windows: bool,
    settings: &LocalTerminalProfileSettings,
) -> Vec<(LocalTerminalLaunchTarget, String)> {
    builtin_kinds(include_windows)
        .into_iter()
        .map(|kind| (LocalTerminalLaunchTarget::Builtin(kind), label(kind)))
        .chain(
            settings
                .effective_custom_profiles()
                .into_iter()
                .map(|profile| {
                    let label = profile.name.clone();
                    (LocalTerminalLaunchTarget::Custom(profile), label)
                }),
        )
        .collect()
}

pub(crate) fn launch_target_is_default(
    target: &LocalTerminalLaunchTarget,
    settings: &LocalTerminalProfileSettings,
    include_windows: bool,
) -> bool {
    match target {
        LocalTerminalLaunchTarget::Builtin(kind) => {
            effective_kind(settings.kind, include_windows) == *kind
        }
        LocalTerminalLaunchTarget::Custom(profile) => {
            settings.kind == LocalTerminalProfileKind::Custom
                && settings
                    .selected_custom_profile()
                    .as_ref()
                    .map(|item| &item.id)
                    == Some(&profile.id)
        }
    }
}

#[cfg(test)]
mod tests {
    use one_core::settings::{
        LocalTerminalCustomProfile, LocalTerminalProfileKind, LocalTerminalProfileSettings,
    };

    use super::{
        LocalTerminalLaunchTarget, effective_kind, kinds, launch_options, launch_target_is_default,
    };

    #[test]
    fn kinds_match_platform_capabilities() {
        assert_eq!(
            vec![
                LocalTerminalProfileKind::System,
                LocalTerminalProfileKind::Zsh,
                LocalTerminalProfileKind::Bash,
                LocalTerminalProfileKind::Fish,
                LocalTerminalProfileKind::Nushell,
                LocalTerminalProfileKind::Custom,
            ],
            kinds(false)
        );
        assert_eq!(
            vec![
                LocalTerminalProfileKind::System,
                LocalTerminalProfileKind::PowerShell,
                LocalTerminalProfileKind::Cmd,
                LocalTerminalProfileKind::Wsl,
                LocalTerminalProfileKind::GitBash,
                LocalTerminalProfileKind::Custom,
            ],
            kinds(true)
        );
    }

    #[test]
    fn unavailable_configured_profile_falls_back_to_system() {
        assert_eq!(
            LocalTerminalProfileKind::System,
            effective_kind(LocalTerminalProfileKind::PowerShell, false)
        );
        assert_eq!(
            LocalTerminalProfileKind::PowerShell,
            effective_kind(LocalTerminalProfileKind::PowerShell, true)
        );
    }

    #[test]
    fn custom_launch_label_exposes_configured_program() {
        let settings = LocalTerminalProfileSettings {
            kind: LocalTerminalProfileKind::Custom,
            custom_profiles: vec![LocalTerminalCustomProfile {
                id: "opencode".to_string(),
                name: "OpenCode".to_string(),
                command: "opencode".to_string(),
            }],
            default_custom_profile_id: Some("opencode".to_string()),
            ..Default::default()
        };
        let options = launch_options(false, &settings);
        let target = options
            .iter()
            .find(|(_, label)| label == "OpenCode")
            .map(|(target, _)| target)
            .unwrap();
        assert!(matches!(target, LocalTerminalLaunchTarget::Custom(_)));
        assert!(launch_target_is_default(target, &settings, false));
    }
}
