use crate::public_mcp_runtime::{mcp_server_enabled, set_mcp_server_enabled, set_mcp_server_mode};
use crate::settings::mcp_client_config::mcp_client_config_items;
use crate::settings::mcp_status::mcp_runtime_status_item;
use gpui::{App, SharedString};
use gpui_component::setting::{SettingField, SettingGroup, SettingItem};
use one_core::settings::{
    AppSettings, McpPermissionMode, McpServerMode, McpSettings, McpToolsetSettings,
};
use rust_i18n::t;

pub fn mcp_setting_group(default_settings: &McpSettings) -> SettingGroup {
    let mut items = mcp_server_items(default_settings);
    items.push(mcp_runtime_status_item());
    items.extend(mcp_client_config_items());
    items.extend(mcp_toolset_items(&default_settings.toolsets));

    SettingGroup::new()
        .title(t!("Settings.General.Mcp.group_title"))
        .items(items)
}

fn mcp_server_items(default_settings: &McpSettings) -> Vec<SettingItem> {
    vec![
        SettingItem::new(
            t!("Settings.General.Mcp.server_enabled"),
            SettingField::switch(
                |cx: &App| mcp_server_enabled(cx),
                |val: bool, cx: &mut App| {
                    set_mcp_server_enabled(cx, val);
                },
            )
            .default_value(default_settings.server_enabled),
        )
        .description(t!("Settings.General.Mcp.server_enabled_desc").to_string()),
        SettingItem::new(
            t!("Settings.General.Mcp.server_mode"),
            SettingField::dropdown(
                mcp_server_mode_options(),
                |cx: &App| SharedString::from(AppSettings::global(cx).mcp.server_mode.as_str()),
                |val: SharedString, cx: &mut App| {
                    set_mcp_server_mode(cx, McpServerMode::from_str(val.as_ref()));
                },
            )
            .default_value(SharedString::from(default_settings.server_mode.as_str())),
        )
        .description(t!("Settings.General.Mcp.server_mode_desc").to_string()),
        SettingItem::new(
            t!("Settings.General.Mcp.permission_mode"),
            SettingField::dropdown(
                mcp_permission_mode_options(),
                |cx: &App| SharedString::from(AppSettings::global(cx).mcp.permission_mode.as_str()),
                |val: SharedString, cx: &mut App| {
                    AppSettings::update_and_save(cx, |settings| {
                        settings.mcp.permission_mode = McpPermissionMode::from_str(val.as_ref());
                    });
                },
            )
            .default_value(SharedString::from(
                default_settings.permission_mode.as_str(),
            )),
        )
        .description(t!("Settings.General.Mcp.permission_mode_desc").to_string()),
    ]
}

fn mcp_server_mode_options() -> Vec<(SharedString, SharedString)> {
    vec![
        (
            McpServerMode::Temporary.as_str().into(),
            t!("Settings.General.Mcp.server_mode_temporary").into(),
        ),
        (
            McpServerMode::Persistent.as_str().into(),
            t!("Settings.General.Mcp.server_mode_persistent").into(),
        ),
    ]
}

fn mcp_permission_mode_options() -> Vec<(SharedString, SharedString)> {
    vec![
        (
            McpPermissionMode::Deny.as_str().into(),
            t!("Settings.General.Mcp.permission_profile_safe").into(),
        ),
        (
            McpPermissionMode::Ask.as_str().into(),
            t!("Settings.General.Mcp.permission_profile_confirm").into(),
        ),
        (
            McpPermissionMode::Allow.as_str().into(),
            t!("Settings.General.Mcp.permission_profile_auto").into(),
        ),
    ]
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum McpToolset {
    Terminal,
    Connections,
    Sftp,
    Redis,
    Database,
}

impl McpToolset {
    const VISIBLE: [Self; 5] = [
        Self::Terminal,
        Self::Connections,
        Self::Sftp,
        Self::Redis,
        Self::Database,
    ];

    #[cfg(test)]
    fn id(self) -> &'static str {
        match self {
            Self::Terminal => "terminal",
            Self::Connections => "connections",
            Self::Sftp => "sftp",
            Self::Redis => "redis",
            Self::Database => "database",
        }
    }

    fn title_key(self) -> &'static str {
        match self {
            Self::Terminal => "Settings.General.Mcp.toolset_terminal",
            Self::Connections => "Settings.General.Mcp.toolset_connections",
            Self::Sftp => "Settings.General.Mcp.toolset_sftp",
            Self::Redis => "Settings.General.Mcp.toolset_redis",
            Self::Database => "Settings.General.Mcp.toolset_database",
        }
    }

    fn description_key(self) -> &'static str {
        match self {
            Self::Terminal => "Settings.General.Mcp.toolset_terminal_desc",
            Self::Connections => "Settings.General.Mcp.toolset_connections_desc",
            Self::Sftp => "Settings.General.Mcp.toolset_sftp_desc",
            Self::Redis => "Settings.General.Mcp.toolset_redis_desc",
            Self::Database => "Settings.General.Mcp.toolset_database_desc",
        }
    }

    fn get(self, settings: &McpToolsetSettings) -> bool {
        match self {
            Self::Terminal => settings.terminal,
            Self::Connections => settings.connections,
            Self::Sftp => settings.sftp,
            Self::Redis => settings.redis,
            Self::Database => settings.database,
        }
    }

    fn set(self, settings: &mut McpToolsetSettings, enabled: bool) {
        match self {
            Self::Terminal => settings.terminal = enabled,
            Self::Connections => settings.connections = enabled,
            Self::Sftp => settings.sftp = enabled,
            Self::Redis => settings.redis = enabled,
            Self::Database => settings.database = enabled,
        }
    }
}

fn mcp_toolset_items(default_settings: &McpToolsetSettings) -> Vec<SettingItem> {
    McpToolset::VISIBLE
        .iter()
        .map(|toolset| mcp_toolset_item(*toolset, default_settings))
        .collect()
}

fn mcp_toolset_item(toolset: McpToolset, default_settings: &McpToolsetSettings) -> SettingItem {
    let getter_toolset = toolset;
    let setter_toolset = toolset;

    SettingItem::new(
        t!(toolset.title_key()),
        SettingField::checkbox(
            move |cx: &App| getter_toolset.get(&AppSettings::global(cx).mcp.toolsets),
            move |val: bool, cx: &mut App| {
                AppSettings::update_and_save(cx, |settings| {
                    setter_toolset.set(&mut settings.mcp.toolsets, val);
                });
            },
        )
        .default_value(toolset.get(default_settings)),
    )
    .description(t!(toolset.description_key()).to_string())
}

#[cfg(test)]
fn mcp_toolset_item_ids() -> Vec<&'static str> {
    McpToolset::VISIBLE
        .iter()
        .map(|toolset| toolset.id())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mcp_server_mode_options_match_persisted_values() {
        let values = mcp_server_mode_options()
            .into_iter()
            .map(|(value, _)| value.to_string())
            .collect::<Vec<_>>();

        assert_eq!(vec!["temporary", "persistent"], values);
    }

    #[test]
    fn mcp_permission_mode_options_match_persisted_values() {
        let options = mcp_permission_mode_options();
        let values = options
            .iter()
            .map(|(value, _)| value.to_string())
            .collect::<Vec<_>>();
        let labels = options
            .into_iter()
            .map(|(_, label)| label.to_string())
            .collect::<Vec<_>>();

        assert_eq!(vec!["deny", "ask", "allow"], values);
        assert_eq!(vec!["Safe", "Confirm", "Auto"], labels);
    }

    #[test]
    fn mcp_toolset_items_only_expose_production_ready_toolsets() {
        let ids = mcp_toolset_item_ids();

        assert_eq!(
            vec!["terminal", "connections", "sftp", "redis", "database"],
            ids
        );
    }

    #[test]
    fn mcp_client_config_items_include_codex_claude_code_and_copy_button() {
        let ids = crate::settings::mcp_client_config::mcp_client_config_item_ids();

        assert_eq!(
            vec![
                "mcp-install-helper",
                "mcp-install-codex-config",
                "mcp-install-claude-code-config",
                "mcp-copy-agent-config"
            ],
            ids
        );
    }
}
