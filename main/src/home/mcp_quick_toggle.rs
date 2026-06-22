#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct McpQuickToggleState {
    pub(crate) enabled: bool,
    pub(crate) label_key: &'static str,
    pub(crate) tooltip_key: &'static str,
    pub(crate) notification_key: &'static str,
}

pub(crate) fn mcp_quick_toggle_state(enabled: bool) -> McpQuickToggleState {
    if enabled {
        return McpQuickToggleState {
            enabled: true,
            label_key: "Home.mcp_disable",
            tooltip_key: "Home.mcp_disable_tooltip",
            notification_key: "Home.mcp_disabled_notification",
        };
    }

    McpQuickToggleState {
        enabled: false,
        label_key: "Home.mcp_enable",
        tooltip_key: "Home.mcp_enable_tooltip",
        notification_key: "Home.mcp_enabled_notification",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quick_toggle_state_follows_global_mcp_setting() {
        let disabled = mcp_quick_toggle_state(false);
        assert!(!disabled.enabled);
        assert_eq!("Home.mcp_enable", disabled.label_key);
        assert_eq!("Home.mcp_enable_tooltip", disabled.tooltip_key);

        let enabled = mcp_quick_toggle_state(true);
        assert!(enabled.enabled);
        assert_eq!("Home.mcp_disable", enabled.label_key);
        assert_eq!("Home.mcp_disable_tooltip", enabled.tooltip_key);
    }
}
