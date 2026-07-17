use crate::settings::mcp_agent_config_copy::mcp_agent_config_copy_item;
#[cfg(test)]
use crate::settings::mcp_agent_config_copy::mcp_agent_config_copy_item_id;
use crate::settings::mcp_helper_install::mcp_helper_install_item;
#[cfg(test)]
use crate::settings::mcp_helper_install::mcp_helper_install_item_id;
use anyhow::{Result, bail};
use gpui::{App, ParentElement, Styled, Window, div};
use gpui_component::{
    ActiveTheme, Disableable, WindowExt,
    button::{Button, ButtonVariants as _},
    h_flex,
    notification::Notification,
    setting::{SettingField, SettingItem},
    v_flex,
};
use public_mcp::client_config::{
    ClientConfigHealth, ClientConfigInstall, claude_code_config_path, codex_config_path,
    helper_unavailable_health, inspect_claude_code_config, inspect_codex_config,
    install_claude_code_config, install_codex_config, uninstall_claude_code_config,
    uninstall_codex_config,
};
use rust_i18n::t;
use std::path::PathBuf;

pub fn mcp_client_config_items() -> Vec<SettingItem> {
    vec![mcp_helper_install_item()]
        .into_iter()
        .chain(
            [
                McpClientConfigTarget::Codex,
                McpClientConfigTarget::ClaudeCode,
            ]
            .into_iter()
            .map(mcp_client_config_item),
        )
        .chain(std::iter::once(mcp_agent_config_copy_item()))
        .collect()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum McpClientConfigTarget {
    Codex,
    ClaudeCode,
}

impl McpClientConfigTarget {
    fn title_key(self) -> &'static str {
        match self {
            Self::Codex => "Settings.General.Mcp.install_codex_config",
            Self::ClaudeCode => "Settings.General.Mcp.install_claude_code_config",
        }
    }

    fn description_key(self) -> &'static str {
        match self {
            Self::Codex => "Settings.General.Mcp.install_codex_config_desc",
            Self::ClaudeCode => "Settings.General.Mcp.install_claude_code_config_desc",
        }
    }

    fn button_id(self) -> &'static str {
        match self {
            Self::Codex => "mcp-install-codex-config",
            Self::ClaudeCode => "mcp-install-claude-code-config",
        }
    }
}

fn mcp_client_config_item(target: McpClientConfigTarget) -> SettingItem {
    SettingItem::new(
        t!(target.title_key()),
        SettingField::render(move |_, _, cx| {
            let inspected = inspect_client_config_for_target(target);
            let health = inspected
                .as_ref()
                .map(|(_, h)| *h)
                .unwrap_or(ClientConfigHealth::NotInstalled);
            let model = client_config_item_view_model(&inspected);
            let action_label = client_config_action_label(health);
            v_flex()
                .gap_1()
                .child(
                    h_flex().child(
                        Button::new(target.button_id())
                            .primary()
                            .label(action_label)
                            .disabled(!model.action_enabled)
                            .on_click(move |_, window, cx| {
                                execute_client_config_action(target, health, window, cx)
                            }),
                    ),
                )
                .child(
                    div()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child(model.status),
                )
        }),
    )
    .description(t!(target.description_key()).to_string())
}

struct McpClientConfigItemViewModel {
    status: String,
    action_enabled: bool,
}

fn client_config_item_view_model(
    inspected: &Result<(PathBuf, ClientConfigHealth)>,
) -> McpClientConfigItemViewModel {
    match inspected {
        Ok((_, health)) => McpClientConfigItemViewModel {
            status: t!(client_config_health_label_key(*health)).to_string(),
            action_enabled: client_config_action_enabled(*health),
        },
        Err(error) => McpClientConfigItemViewModel {
            status: error.to_string(),
            action_enabled: false,
        },
    }
}

fn execute_client_config_action(
    target: McpClientConfigTarget,
    health: ClientConfigHealth,
    window: &mut Window,
    cx: &mut App,
) {
    match health {
        ClientConfigHealth::UpToDate => uninstall_client_config(target, window, cx),
        _ => install_client_config(target, window, cx),
    }
}

fn install_client_config(target: McpClientConfigTarget, window: &mut Window, cx: &mut App) {
    match install_client_config_for_target(target) {
        Ok(path) => window.push_notification(
            Notification::success(
                t!(
                    "Settings.General.Mcp.install_client_config_success",
                    path = path.display().to_string()
                )
                .to_string(),
            )
            .autohide(true),
            cx,
        ),
        Err(error) => window.push_notification(
            Notification::error(
                t!(
                    "Settings.General.Mcp.install_client_config_failed",
                    error = error.to_string()
                )
                .to_string(),
            )
            .autohide(true),
            cx,
        ),
    }
}

fn install_client_config_for_target(target: McpClientConfigTarget) -> Result<PathBuf> {
    let install = ClientConfigInstall::from_current_app()?;
    if let Some(health) = helper_unavailable_health(&install.launcher_path)? {
        bail!(
            "{}: {}",
            t!(client_config_health_label_key(health)),
            install.launcher_path.display()
        );
    }

    let config_path = match target {
        McpClientConfigTarget::Codex => {
            let path = codex_config_path()
                .ok_or_else(|| anyhow::anyhow!("Codex config path is unavailable"))?;
            install_codex_config(&path, &install)?;
            path
        }
        McpClientConfigTarget::ClaudeCode => {
            let path = claude_code_config_path()
                .ok_or_else(|| anyhow::anyhow!("Claude Code config path is unavailable"))?;
            install_claude_code_config(&path, &install)?;
            path
        }
    };
    Ok(config_path)
}

fn uninstall_client_config(target: McpClientConfigTarget, window: &mut Window, cx: &mut App) {
    match uninstall_client_config_for_target(target) {
        Ok(path) => window.push_notification(
            Notification::success(
                t!(
                    "Settings.General.Mcp.uninstall_client_config_success",
                    path = path.display().to_string()
                )
                .to_string(),
            )
            .autohide(true),
            cx,
        ),
        Err(error) => window.push_notification(
            Notification::error(
                t!(
                    "Settings.General.Mcp.uninstall_client_config_failed",
                    error = error.to_string()
                )
                .to_string(),
            )
            .autohide(true),
            cx,
        ),
    }
}

fn uninstall_client_config_for_target(target: McpClientConfigTarget) -> Result<PathBuf> {
    match target {
        McpClientConfigTarget::Codex => {
            let path = codex_config_path()
                .ok_or_else(|| anyhow::anyhow!("Codex config path is unavailable"))?;
            uninstall_codex_config(&path)?;
            Ok(path)
        }
        McpClientConfigTarget::ClaudeCode => {
            let path = claude_code_config_path()
                .ok_or_else(|| anyhow::anyhow!("Claude Code config path is unavailable"))?;
            uninstall_claude_code_config(&path)?;
            Ok(path)
        }
    }
}

fn inspect_client_config_for_target(
    target: McpClientConfigTarget,
) -> Result<(PathBuf, ClientConfigHealth)> {
    let install = ClientConfigInstall::from_current_app()?;
    let path = match target {
        McpClientConfigTarget::Codex => {
            let path = codex_config_path()
                .ok_or_else(|| anyhow::anyhow!("Codex config path is unavailable"))?;
            let health = inspect_codex_config(&path, &install)?;
            (path, health)
        }
        McpClientConfigTarget::ClaudeCode => {
            let path = claude_code_config_path()
                .ok_or_else(|| anyhow::anyhow!("Claude Code config path is unavailable"))?;
            let health = inspect_claude_code_config(&path, &install)?;
            (path, health)
        }
    };
    Ok(path)
}

fn client_config_health_label_key(health: ClientConfigHealth) -> &'static str {
    match health {
        ClientConfigHealth::UpToDate => "Settings.General.Mcp.client_config_status_up_to_date",
        ClientConfigHealth::NotInstalled => {
            "Settings.General.Mcp.client_config_status_not_installed"
        }
        ClientConfigHealth::NeedsRepair => "Settings.General.Mcp.client_config_status_needs_repair",
        ClientConfigHealth::NeedsMigration => {
            "Settings.General.Mcp.client_config_status_needs_repair"
        }
        ClientConfigHealth::PackageVersionOutdated => {
            "Settings.General.Mcp.client_config_status_needs_repair"
        }
        ClientConfigHealth::NodeUnavailable | ClientConfigHealth::NpxUnavailable => {
            "Settings.General.Mcp.client_config_status_missing_helper"
        }
        ClientConfigHealth::MissingHelper => {
            "Settings.General.Mcp.client_config_status_missing_helper"
        }
        ClientConfigHealth::UnusableHelper => {
            "Settings.General.Mcp.client_config_status_unusable_helper"
        }
    }
}

fn client_config_action_enabled(health: ClientConfigHealth) -> bool {
    !matches!(
        health,
        ClientConfigHealth::MissingHelper
            | ClientConfigHealth::UnusableHelper
            | ClientConfigHealth::NodeUnavailable
            | ClientConfigHealth::NpxUnavailable
    )
}

fn client_config_action_label(health: ClientConfigHealth) -> String {
    match health {
        ClientConfigHealth::UpToDate => {
            t!("Settings.General.Mcp.uninstall_client_config").to_string()
        }
        _ => t!("Settings.General.Mcp.install_client_config").to_string(),
    }
}

#[cfg(test)]
pub(crate) fn mcp_client_config_item_ids() -> Vec<&'static str> {
    std::iter::once(mcp_helper_install_item_id())
        .chain(
            [
                McpClientConfigTarget::Codex,
                McpClientConfigTarget::ClaudeCode,
            ]
            .iter()
            .map(|target| target.button_id()),
        )
        .chain(std::iter::once(mcp_agent_config_copy_item_id()))
        .collect()
}

#[cfg(test)]
#[path = "mcp_client_config_tests.rs"]
mod tests;
