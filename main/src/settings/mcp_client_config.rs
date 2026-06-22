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
    ClientConfigHealth, ClientConfigInstall, claude_desktop_config_path, codex_config_path,
    helper_unavailable_health, inspect_claude_desktop_config, inspect_codex_config,
    install_claude_desktop_config, install_codex_config,
};
use rust_i18n::t;
use std::path::PathBuf;

pub fn mcp_client_config_items() -> Vec<SettingItem> {
    vec![mcp_helper_install_item()]
        .into_iter()
        .chain(
            [McpClientConfigTarget::Codex, McpClientConfigTarget::Claude]
                .into_iter()
                .map(mcp_client_config_item),
        )
        .collect()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum McpClientConfigTarget {
    Codex,
    Claude,
}

impl McpClientConfigTarget {
    fn title_key(self) -> &'static str {
        match self {
            Self::Codex => "Settings.General.Mcp.install_codex_config",
            Self::Claude => "Settings.General.Mcp.install_claude_config",
        }
    }

    fn description_key(self) -> &'static str {
        match self {
            Self::Codex => "Settings.General.Mcp.install_codex_config_desc",
            Self::Claude => "Settings.General.Mcp.install_claude_config_desc",
        }
    }

    fn button_id(self) -> &'static str {
        match self {
            Self::Codex => "mcp-install-codex-config",
            Self::Claude => "mcp-install-claude-config",
        }
    }
}

fn mcp_client_config_item(target: McpClientConfigTarget) -> SettingItem {
    SettingItem::new(
        t!(target.title_key()),
        SettingField::render(move |_, _, cx| {
            let model = client_config_item_view_model(inspect_client_config_for_target(target));
            v_flex()
                .gap_1()
                .child(
                    h_flex().child(
                        Button::new(target.button_id())
                            .primary()
                            .label(t!("Settings.General.Mcp.install_client_config"))
                            .disabled(!model.install_enabled)
                            .on_click(move |_, window, cx| {
                                install_client_config(target, window, cx)
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
    install_enabled: bool,
}

fn client_config_item_view_model(
    inspected: Result<(PathBuf, ClientConfigHealth)>,
) -> McpClientConfigItemViewModel {
    match inspected {
        Ok((_, health)) => McpClientConfigItemViewModel {
            status: t!(client_config_health_label_key(health)).to_string(),
            install_enabled: client_config_install_enabled(health),
        },
        Err(error) => McpClientConfigItemViewModel {
            status: error.to_string(),
            install_enabled: false,
        },
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
        McpClientConfigTarget::Claude => {
            let path = claude_desktop_config_path()
                .ok_or_else(|| anyhow::anyhow!("Claude config path is unavailable"))?;
            install_claude_desktop_config(&path, &install)?;
            path
        }
    };
    Ok(config_path)
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
        McpClientConfigTarget::Claude => {
            let path = claude_desktop_config_path()
                .ok_or_else(|| anyhow::anyhow!("Claude config path is unavailable"))?;
            let health = inspect_claude_desktop_config(&path, &install)?;
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
        ClientConfigHealth::MissingHelper => {
            "Settings.General.Mcp.client_config_status_missing_helper"
        }
        ClientConfigHealth::UnusableHelper => {
            "Settings.General.Mcp.client_config_status_unusable_helper"
        }
    }
}

fn client_config_install_enabled(health: ClientConfigHealth) -> bool {
    !matches!(
        health,
        ClientConfigHealth::MissingHelper | ClientConfigHealth::UnusableHelper
    )
}

#[cfg(test)]
pub(crate) fn mcp_client_config_item_ids() -> Vec<&'static str> {
    std::iter::once(mcp_helper_install_item_id())
        .chain(
            [McpClientConfigTarget::Codex, McpClientConfigTarget::Claude]
                .iter()
                .map(|target| target.button_id()),
        )
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use public_mcp::client_config::ClientConfigHealth;

    #[test]
    fn client_config_health_labels_match_config_states() {
        assert_eq!(
            "Settings.General.Mcp.client_config_status_up_to_date",
            client_config_health_label_key(ClientConfigHealth::UpToDate)
        );
        assert_eq!(
            "Settings.General.Mcp.client_config_status_not_installed",
            client_config_health_label_key(ClientConfigHealth::NotInstalled)
        );
        assert_eq!(
            "Settings.General.Mcp.client_config_status_needs_repair",
            client_config_health_label_key(ClientConfigHealth::NeedsRepair)
        );
        assert_eq!(
            "Settings.General.Mcp.client_config_status_missing_helper",
            client_config_health_label_key(ClientConfigHealth::MissingHelper)
        );
        assert_eq!(
            "Settings.General.Mcp.client_config_status_unusable_helper",
            client_config_health_label_key(ClientConfigHealth::UnusableHelper)
        );
    }

    #[test]
    fn client_config_install_button_is_disabled_when_helper_is_unavailable() {
        assert!(!client_config_install_enabled(
            ClientConfigHealth::MissingHelper
        ));
        assert!(!client_config_install_enabled(
            ClientConfigHealth::UnusableHelper
        ));
        assert!(client_config_install_enabled(
            ClientConfigHealth::NotInstalled
        ));
        assert!(client_config_install_enabled(
            ClientConfigHealth::NeedsRepair
        ));
        assert!(client_config_install_enabled(ClientConfigHealth::UpToDate));
    }
}
