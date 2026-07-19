use gpui::{App, AppContext, ParentElement, Styled, Window, div};
use gpui_component::{
    ActiveTheme, Disableable, WindowExt,
    button::{Button, ButtonVariants as _},
    h_flex,
    notification::Notification,
    setting::{SettingField, SettingItem},
    v_flex,
};
use public_mcp::client_config::ClientConfigInstall;
use rust_i18n::t;
use std::{path::PathBuf, process::Command};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SkillTarget {
    Codex,
    Agents,
}

const NAVOP_CLI_PACKAGE: &str = "@navop/cli@latest";

impl SkillTarget {
    fn id(self) -> &'static str {
        match self {
            Self::Codex => "codex",
            Self::Agents => "agents",
        }
    }

    fn button_id(self) -> &'static str {
        match self {
            Self::Codex => "mcp-install-codex-skill",
            Self::Agents => "mcp-install-agents-skill",
        }
    }

    fn title_key(self) -> &'static str {
        match self {
            Self::Codex => "Settings.General.Mcp.install_codex_skill",
            Self::Agents => "Settings.General.Mcp.install_agents_skill",
        }
    }

    fn user_path(self) -> Option<PathBuf> {
        let home = dirs::home_dir()?;
        Some(match self {
            Self::Codex => home.join(".codex/skills/navop"),
            Self::Agents => home.join(".agents/skills/navop"),
        })
    }
}

pub(crate) fn mcp_skill_install_items() -> Vec<SettingItem> {
    [SkillTarget::Codex, SkillTarget::Agents]
        .into_iter()
        .map(mcp_skill_install_item)
        .collect()
}

fn mcp_skill_install_item(target: SkillTarget) -> SettingItem {
    SettingItem::new(
        t!(target.title_key()),
        SettingField::render(move |_, _, cx| {
            let path = target.user_path();
            let installed = path
                .as_ref()
                .is_some_and(|path| path.join("SKILL.md").is_file());
            let status = match path.as_ref() {
                Some(path) if installed => t!(
                    "Settings.General.Mcp.skill_status_installed",
                    path = path.display().to_string()
                )
                .to_string(),
                Some(path) => t!(
                    "Settings.General.Mcp.skill_status_not_installed",
                    path = path.display().to_string()
                )
                .to_string(),
                None => t!("Settings.General.Mcp.skill_status_home_unavailable").to_string(),
            };
            let label = if installed {
                t!("Settings.General.Mcp.update_skill")
            } else {
                t!("Settings.General.Mcp.install_skill")
            };
            v_flex()
                .gap_1()
                .child(
                    h_flex().child(
                        Button::new(target.button_id())
                            .primary()
                            .label(label.to_string())
                            .disabled(path.is_none())
                            .on_click(move |_, window, cx| {
                                install_skill(target, installed, window, cx)
                            }),
                    ),
                )
                .child(
                    div()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child(status),
                )
        }),
    )
    .description(t!("Settings.General.Mcp.install_skill_desc").to_string())
}

fn skill_install_args(target: SkillTarget, force: bool) -> Vec<String> {
    let mut args = vec![
        "-y".to_string(),
        NAVOP_CLI_PACKAGE.to_string(),
        "skill".to_string(),
        "install".to_string(),
        "--target".to_string(),
        target.id().to_string(),
        "--scope".to_string(),
        "user".to_string(),
        "--json".to_string(),
    ];
    if force {
        args.push("--force".to_string());
    }
    args
}

fn install_skill(target: SkillTarget, force: bool, window: &mut Window, cx: &mut App) {
    let install = match ClientConfigInstall::from_current_app() {
        Ok(install) => install,
        Err(error) => {
            window.push_notification(
                Notification::error(
                    t!(
                        "Settings.General.Mcp.install_skill_failed",
                        error = error.to_string()
                    )
                    .to_string(),
                )
                .autohide(true),
                cx,
            );
            return;
        }
    };
    let launcher = install.launcher_path;
    let args = skill_install_args(target, force);
    let target_window = window.window_handle();
    let task = cx.background_spawn(smol::unblock(move || {
        Command::new(launcher).args(args).output()
    }));

    window
        .spawn(cx, async move |cx| {
            let result = task.await;
            let _ = cx.update_window(target_window, |_, window, cx| {
                let notification = match result {
                    Ok(output) if output.status.success() => Notification::success(
                        t!("Settings.General.Mcp.install_skill_success").to_string(),
                    ),
                    Ok(output) => {
                        let error = String::from_utf8_lossy(&output.stderr).trim().to_string();
                        Notification::error(
                            t!("Settings.General.Mcp.install_skill_failed", error = error)
                                .to_string(),
                        )
                    }
                    Err(error) => Notification::error(
                        t!(
                            "Settings.General.Mcp.install_skill_failed",
                            error = error.to_string()
                        )
                        .to_string(),
                    ),
                };
                window.push_notification(notification.autohide(true), cx);
                window.refresh();
            });
        })
        .detach();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn skill_install_uses_latest_cli_package_without_a_shell() {
        let args = skill_install_args(SkillTarget::Codex, false);

        assert_eq!("-y", args[0]);
        assert_eq!(NAVOP_CLI_PACKAGE, args[1]);
        assert_eq!(
            vec![
                "skill", "install", "--target", "codex", "--scope", "user", "--json"
            ],
            args[2..]
        );
        assert!(!args.iter().any(|arg| arg == "--force"));
    }

    #[test]
    fn updating_an_existing_skill_requires_explicit_force() {
        let args = skill_install_args(SkillTarget::Agents, true);

        assert!(args.iter().any(|arg| arg == "--force"));
        assert!(args.windows(2).any(|args| args == ["--target", "agents"]));
    }
}
