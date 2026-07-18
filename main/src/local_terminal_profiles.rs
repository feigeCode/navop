use gpui::SharedString;
use one_core::settings::LocalTerminalProfileKind;
use rust_i18n::t;

pub(crate) fn kinds(include_windows: bool) -> Vec<LocalTerminalProfileKind> {
    let mut kinds = vec![
        LocalTerminalProfileKind::System,
        LocalTerminalProfileKind::PowerShell,
    ];
    if include_windows {
        kinds.extend([
            LocalTerminalProfileKind::Cmd,
            LocalTerminalProfileKind::Wsl,
            LocalTerminalProfileKind::GitBash,
        ]);
    }
    kinds.push(LocalTerminalProfileKind::Custom);
    kinds
}

pub(crate) fn label(kind: LocalTerminalProfileKind) -> String {
    match kind {
        LocalTerminalProfileKind::System => t!("Settings.General.LocalTerminal.system").to_string(),
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

pub(crate) fn launch_options(
    include_windows: bool,
    custom_program: &str,
) -> Vec<(LocalTerminalProfileKind, String)> {
    kinds(include_windows)
        .into_iter()
        .map(|kind| {
            let label = if kind == LocalTerminalProfileKind::Custom {
                let program = custom_program.trim();
                if program.is_empty() {
                    label(kind)
                } else {
                    format!("{} ({program})", label(kind))
                }
            } else {
                label(kind)
            };
            (kind, label)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use one_core::settings::LocalTerminalProfileKind;

    use super::{kinds, launch_options};

    #[test]
    fn kinds_match_platform_capabilities() {
        assert_eq!(
            vec![
                LocalTerminalProfileKind::System,
                LocalTerminalProfileKind::PowerShell,
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
    fn custom_launch_label_exposes_configured_program() {
        let options = launch_options(false, "opencode");
        assert!(
            options
                .iter()
                .any(|(kind, label)| *kind == LocalTerminalProfileKind::Custom
                    && label.contains("opencode"))
        );
    }
}
