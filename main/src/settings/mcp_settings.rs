use crate::public_mcp_runtime::{mcp_server_enabled, set_mcp_server_enabled, set_mcp_server_mode};
use crate::settings::mcp_client_config::mcp_client_config_items;
use crate::settings::mcp_status::mcp_runtime_status_item;
use gpui::{App, SharedString};
use gpui_component::setting::{SettingField, SettingGroup, SettingItem};
use one_core::settings::{AppSettings, McpPermissionMode, McpServerMode, McpSettings};
use rust_i18n::t;

pub fn mcp_setting_group(default_settings: &McpSettings) -> SettingGroup {
    let mut items = mcp_server_items(default_settings);
    items.push(mcp_runtime_status_item());
    items.extend(mcp_client_config_items());

    SettingGroup::new()
        .title(t!("Settings.General.Mcp.group_title"))
        .description(t!("Settings.General.Mcp.group_desc"))
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
    fn mcp_client_config_items_include_codex_claude_code_and_copy_button() {
        let ids = crate::settings::mcp_client_config::mcp_client_config_item_ids();

        assert_eq!(
            vec![
                "mcp-runtime-requirements",
                "mcp-install-codex-config",
                "mcp-install-claude-code-config",
                "mcp-copy-agent-config"
            ],
            ids
        );
    }
}
