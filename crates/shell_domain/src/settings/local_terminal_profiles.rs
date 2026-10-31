use gpui::SharedString;
use one_core::settings::LocalTerminalProfileKind;
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
        LocalTerminalProfileKind::System => t!("LocalTerminal.system").to_string(),
        LocalTerminalProfileKind::Zsh => t!("LocalTerminal.zsh").to_string(),
        LocalTerminalProfileKind::Bash => t!("LocalTerminal.bash").to_string(),
        LocalTerminalProfileKind::Fish => t!("LocalTerminal.fish").to_string(),
        LocalTerminalProfileKind::Nushell => t!("LocalTerminal.nushell").to_string(),
        LocalTerminalProfileKind::PowerShell => t!("LocalTerminal.powershell").to_string(),
        LocalTerminalProfileKind::Cmd => t!("LocalTerminal.cmd").to_string(),
        LocalTerminalProfileKind::Wsl => t!("LocalTerminal.wsl").to_string(),
        LocalTerminalProfileKind::GitBash => t!("LocalTerminal.git_bash").to_string(),
        LocalTerminalProfileKind::Custom => t!("LocalTerminal.custom").to_string(),
    }
}

pub(crate) fn setting_options(include_windows: bool) -> Vec<(SharedString, SharedString)> {
    kinds(include_windows)
        .into_iter()
        .map(|kind| (kind.as_str().into(), label(kind).into()))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::setting_options;
    use one_core::settings::LocalTerminalProfileKind;

    #[test]
    fn windows_options_include_wsl_git_bash_and_custom() {
        let options = setting_options(true)
            .into_iter()
            .map(|(value, _)| LocalTerminalProfileKind::parse(value.as_ref()))
            .collect::<Vec<_>>();

        assert!(options.contains(&LocalTerminalProfileKind::System));
        assert!(options.contains(&LocalTerminalProfileKind::PowerShell));
        assert!(options.contains(&LocalTerminalProfileKind::Cmd));
        assert!(options.contains(&LocalTerminalProfileKind::Wsl));
        assert!(options.contains(&LocalTerminalProfileKind::GitBash));
        assert!(options.contains(&LocalTerminalProfileKind::Custom));
    }

    #[test]
    fn unix_options_include_common_shells() {
        let options = setting_options(false)
            .into_iter()
            .map(|(value, _)| LocalTerminalProfileKind::parse(value.as_ref()))
            .collect::<Vec<_>>();

        assert!(options.contains(&LocalTerminalProfileKind::Zsh));
        assert!(options.contains(&LocalTerminalProfileKind::Bash));
        assert!(options.contains(&LocalTerminalProfileKind::Fish));
        assert!(options.contains(&LocalTerminalProfileKind::Nushell));
        assert!(!options.contains(&LocalTerminalProfileKind::PowerShell));
        assert!(options.contains(&LocalTerminalProfileKind::Custom));
    }
}
