use one_core::settings::{
    AppSettings, McpPermissionMode, McpServerMode, ToolExposureToolsetSettings,
};
use public_mcp::discovery::PublicMcpMode;
use public_mcp::permissions::PermissionMode;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PublicMcpStartConfig {
    pub enabled: bool,
    pub mode: PublicMcpMode,
    pub permission_mode: PermissionMode,
    pub permission_profile: &'static str,
    pub toolsets: ToolExposureToolsetSettings,
}

impl PublicMcpStartConfig {
    #[cfg(test)]
    pub fn from_settings_and_env(
        settings: &AppSettings,
        env_override: PublicMcpEnvOverride,
    ) -> Self {
        Self::from_settings_session_and_env(settings, false, env_override)
    }

    pub fn from_settings_session_and_env(
        settings: &AppSettings,
        session_enabled: bool,
        env_override: PublicMcpEnvOverride,
    ) -> Self {
        let permission_mode = env_override
            .permission_mode
            .unwrap_or_else(|| map_permission_mode(settings.mcp.permission_mode));
        Self {
            enabled: env_override
                .enabled
                .unwrap_or_else(|| configured_enabled(settings, session_enabled)),
            mode: map_server_mode(settings.mcp.server_mode),
            permission_mode,
            permission_profile: permission_profile_id(permission_mode),
            toolsets: settings.tool_exposure.mcp.clone(),
        }
    }

    pub fn requires_runtime_restart(&self, next: &Self) -> bool {
        self.enabled != next.enabled || self.mode != next.mode || self.toolsets != next.toolsets
    }
}

fn configured_enabled(settings: &AppSettings, session_enabled: bool) -> bool {
    effective_server_enabled(settings, session_enabled)
}

pub fn effective_server_enabled(settings: &AppSettings, session_enabled: bool) -> bool {
    match settings.mcp.server_mode {
        McpServerMode::Temporary => session_enabled,
        McpServerMode::Persistent => settings.mcp.server_enabled,
    }
}

pub fn apply_server_enabled_for_mode(
    settings: &mut AppSettings,
    session_enabled: &mut bool,
    enabled: bool,
) {
    match settings.mcp.server_mode {
        McpServerMode::Temporary => {
            *session_enabled = enabled;
            settings.mcp.server_enabled = false;
        }
        McpServerMode::Persistent => {
            *session_enabled = false;
            settings.mcp.server_enabled = enabled;
        }
    }
}

pub fn apply_server_mode_preserving_enabled(
    settings: &mut AppSettings,
    session_enabled: &mut bool,
    mode: McpServerMode,
) {
    let enabled = effective_server_enabled(settings, *session_enabled);
    settings.mcp.server_mode = mode;
    apply_server_enabled_for_mode(settings, session_enabled, enabled);
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PublicMcpEnvOverride {
    pub enabled: Option<bool>,
    pub permission_mode: Option<PermissionMode>,
}

impl PublicMcpEnvOverride {
    pub fn from_env() -> Self {
        Self {
            enabled: bool_env("NAVOP_PUBLIC_MCP").or_else(|| bool_env("ONETCLI_PUBLIC_MCP")),
            permission_mode: permission_env("NAVOP_PUBLIC_MCP_PERMISSION")
                .or_else(|| permission_env("ONETCLI_PUBLIC_MCP_PERMISSION")),
        }
    }
}

fn map_server_mode(mode: McpServerMode) -> PublicMcpMode {
    match mode {
        McpServerMode::Temporary => PublicMcpMode::Temporary,
        McpServerMode::Persistent => PublicMcpMode::Persistent,
    }
}

fn map_permission_mode(mode: McpPermissionMode) -> PermissionMode {
    match mode {
        McpPermissionMode::Deny => PermissionMode::Deny,
        McpPermissionMode::Ask => PermissionMode::Ask,
        McpPermissionMode::Allow => PermissionMode::Allow,
    }
}

fn permission_profile_id(mode: PermissionMode) -> &'static str {
    match mode {
        PermissionMode::Deny => McpPermissionMode::Deny.profile_id(),
        PermissionMode::Ask => McpPermissionMode::Ask.profile_id(),
        PermissionMode::Allow => McpPermissionMode::Allow.profile_id(),
    }
}

fn bool_env(name: &str) -> Option<bool> {
    match std::env::var(name).ok()?.to_ascii_lowercase().as_str() {
        "1" | "true" | "yes" => Some(true),
        "0" | "false" | "no" => Some(false),
        _ => None,
    }
}

fn permission_env(name: &str) -> Option<PermissionMode> {
    match std::env::var(name).ok()?.to_ascii_lowercase().as_str() {
        "allow" => Some(PermissionMode::Allow),
        "ask" => Some(PermissionMode::Ask),
        "deny" => Some(PermissionMode::Deny),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::{PublicMcpEnvOverride, PublicMcpStartConfig};
    use one_core::settings::{AppSettings, McpPermissionMode, McpServerMode};
    use public_mcp::discovery::PublicMcpMode;
    use public_mcp::permissions::PermissionMode;

    fn settings() -> AppSettings {
        let mut settings = AppSettings::default();
        settings.mcp.server_enabled = true;
        settings.mcp.server_mode = McpServerMode::Persistent;
        settings.mcp.permission_mode = McpPermissionMode::Ask;
        settings
    }

    #[test]
    fn runtime_config_reads_global_mcp_settings() {
        let config = PublicMcpStartConfig::from_settings_and_env(
            &settings(),
            PublicMcpEnvOverride::default(),
        );

        assert!(config.enabled);
        assert_eq!(config.mode, PublicMcpMode::Persistent);
        assert_eq!(config.permission_mode, PermissionMode::Ask);
        assert_eq!("confirm", config.permission_profile);
    }

    #[test]
    fn runtime_config_keeps_public_mcp_disabled_by_default() {
        let config = PublicMcpStartConfig::from_settings_and_env(
            &AppSettings::default(),
            PublicMcpEnvOverride::default(),
        );

        assert!(!config.enabled);
        assert_eq!(config.mode, PublicMcpMode::Temporary);
        assert_eq!(config.permission_mode, PermissionMode::Deny);
    }

    #[test]
    fn runtime_config_env_override_takes_precedence_for_developer_launches() {
        let config = PublicMcpStartConfig::from_settings_and_env(
            &AppSettings::default(),
            PublicMcpEnvOverride {
                enabled: Some(true),
                permission_mode: Some(PermissionMode::Allow),
            },
        );

        assert!(config.enabled);
        assert_eq!(config.mode, PublicMcpMode::Temporary);
        assert_eq!(config.permission_mode, PermissionMode::Allow);
    }

    #[test]
    fn temporary_mode_uses_session_enabled_instead_of_persisted_enabled() {
        let mut settings = AppSettings::default();
        settings.mcp.server_enabled = true;
        settings.mcp.server_mode = McpServerMode::Temporary;

        let disabled = PublicMcpStartConfig::from_settings_session_and_env(
            &settings,
            false,
            PublicMcpEnvOverride::default(),
        );
        let enabled = PublicMcpStartConfig::from_settings_session_and_env(
            &settings,
            true,
            PublicMcpEnvOverride::default(),
        );

        assert!(!disabled.enabled);
        assert!(enabled.enabled);
        assert_eq!(PublicMcpMode::Temporary, enabled.mode);
    }

    #[test]
    fn permission_mode_change_does_not_require_runtime_restart() {
        let mut current = PublicMcpStartConfig::from_settings_and_env(
            &settings(),
            PublicMcpEnvOverride::default(),
        );
        let mut next = current.clone();
        current.permission_mode = PermissionMode::Deny;
        next.permission_mode = PermissionMode::Allow;

        assert!(!current.requires_runtime_restart(&next));
    }

    #[test]
    fn toolset_change_requires_runtime_restart() {
        let current = PublicMcpStartConfig::from_settings_and_env(
            &settings(),
            PublicMcpEnvOverride::default(),
        );
        let mut next = current.clone();
        next.toolsets.terminal = !next.toolsets.terminal;

        assert!(current.requires_runtime_restart(&next));
    }

    #[test]
    fn persistent_mode_uses_persisted_enabled_instead_of_session_enabled() {
        let mut settings = AppSettings::default();
        settings.mcp.server_enabled = false;
        settings.mcp.server_mode = McpServerMode::Persistent;

        let config = PublicMcpStartConfig::from_settings_session_and_env(
            &settings,
            true,
            PublicMcpEnvOverride::default(),
        );

        assert!(!config.enabled);
        assert_eq!(PublicMcpMode::Persistent, config.mode);
    }

    #[test]
    fn applying_temporary_enabled_keeps_persisted_enabled_false() {
        let mut settings = AppSettings::default();
        settings.mcp.server_mode = McpServerMode::Temporary;
        settings.mcp.server_enabled = true;
        let mut session_enabled = false;

        super::apply_server_enabled_for_mode(&mut settings, &mut session_enabled, true);

        assert!(session_enabled);
        assert!(!settings.mcp.server_enabled);
        assert!(super::effective_server_enabled(&settings, session_enabled));
    }

    #[test]
    fn applying_persistent_enabled_writes_persisted_enabled() {
        let mut settings = AppSettings::default();
        settings.mcp.server_mode = McpServerMode::Persistent;
        let mut session_enabled = true;

        super::apply_server_enabled_for_mode(&mut settings, &mut session_enabled, true);

        assert!(!session_enabled);
        assert!(settings.mcp.server_enabled);
        assert!(super::effective_server_enabled(&settings, session_enabled));
    }

    #[test]
    fn switching_mode_preserves_current_effective_enabled_in_target_storage() {
        let mut settings = AppSettings::default();
        settings.mcp.server_mode = McpServerMode::Temporary;
        let mut session_enabled = true;

        super::apply_server_mode_preserving_enabled(
            &mut settings,
            &mut session_enabled,
            McpServerMode::Persistent,
        );

        assert_eq!(McpServerMode::Persistent, settings.mcp.server_mode);
        assert!(!session_enabled);
        assert!(settings.mcp.server_enabled);

        super::apply_server_mode_preserving_enabled(
            &mut settings,
            &mut session_enabled,
            McpServerMode::Temporary,
        );

        assert_eq!(McpServerMode::Temporary, settings.mcp.server_mode);
        assert!(session_enabled);
        assert!(!settings.mcp.server_enabled);
    }
}
