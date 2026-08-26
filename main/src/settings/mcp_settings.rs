use crate::public_mcp_runtime::{mcp_server_enabled, set_mcp_server_enabled, set_mcp_server_mode};
use crate::settings::mcp_client_config::mcp_client_config_items;
use crate::settings::mcp_skill_install::mcp_skill_install_items;
use crate::settings::mcp_status::mcp_runtime_status_item;
use gpui::{App, SharedString};
use gpui_component::setting::{NumberFieldOptions, SettingField, SettingGroup, SettingItem};
use one_core::settings::{
    AppSettings, DEFAULT_MCP_APPROVAL_TIMEOUT_MS, MAX_MCP_APPROVAL_TIMEOUT_MS, McpPermissionMode,
    McpServerMode, McpSettings,
};
use rust_i18n::t;

pub fn mcp_setting_group(default_settings: &McpSettings) -> SettingGroup {
    let mut items = mcp_server_items(default_settings);
    items.push(mcp_runtime_status_item());
    items.extend(mcp_client_config_items());
    items.extend(mcp_skill_install_items());

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
        SettingItem::new(
            t!("Settings.General.Mcp.approval_timeout"),
            SettingField::number_input(
                NumberFieldOptions {
                    min: 0.0,
                    max: MAX_MCP_APPROVAL_TIMEOUT_MS as f64,
                    step: 1_000.0,
                },
                |cx: &App| AppSettings::global(cx).mcp.approval_timeout_ms as f64,
                |value: f64, cx: &mut App| {
                    AppSettings::update_and_save(cx, |settings| {
                        settings.mcp.approval_timeout_ms = normalize_approval_timeout_ms(value);
                    });
                },
            )
            .default_value(default_settings.approval_timeout_ms as f64),
        )
        .description(t!("Settings.General.Mcp.approval_timeout_desc").to_string()),
    ]
}

fn normalize_approval_timeout_ms(value: f64) -> u64 {
    if !value.is_finite() {
        return DEFAULT_MCP_APPROVAL_TIMEOUT_MS;
    }
    (value.round() as u64).clamp(0, MAX_MCP_APPROVAL_TIMEOUT_MS)
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
        assert_eq!(
            vec![
                "Safe (Read-only)",
                "Confirm Before Running",
                "Run Automatically"
            ],
            labels
        );
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

    #[test]
    fn approval_timeout_values_are_clamped_to_runtime_bounds() {
        assert_eq!(0, normalize_approval_timeout_ms(0.0));
        assert_eq!(5_000, normalize_approval_timeout_ms(5_000.0));
        assert_eq!(
            DEFAULT_MCP_APPROVAL_TIMEOUT_MS,
            normalize_approval_timeout_ms(f64::NAN)
        );
        assert_eq!(
            MAX_MCP_APPROVAL_TIMEOUT_MS,
            normalize_approval_timeout_ms((MAX_MCP_APPROVAL_TIMEOUT_MS + 1) as f64)
        );
    }
}
