use gpui::App;
use gpui_component::setting::{SettingField, SettingGroup, SettingItem};
use one_core::settings::{AppSettings, ToolExposureSettings, ToolExposureToolsetSettings};
use rust_i18n::t;

pub fn mcp_tool_exposure_setting_group(default_settings: &ToolExposureSettings) -> SettingGroup {
    tool_exposure_group(ToolExposureSurface::Mcp, &default_settings.mcp)
}

pub fn agent_tool_exposure_setting_group(default_settings: &ToolExposureSettings) -> SettingGroup {
    tool_exposure_group(ToolExposureSurface::Agent, &default_settings.agent)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ToolExposureSurface {
    Mcp,
    Agent,
}

impl ToolExposureSurface {
    fn title_key(self) -> &'static str {
        match self {
            Self::Mcp => "Settings.General.ToolExposure.mcp_group_title",
            Self::Agent => "Settings.General.ToolExposure.agent_group_title",
        }
    }

    fn visible_items(self) -> &'static [ToolExposureItem] {
        match self {
            Self::Mcp => &ToolExposureItem::MCP_VISIBLE,
            Self::Agent => &ToolExposureItem::AGENT_VISIBLE,
        }
    }

    fn description_key(self) -> &'static str {
        match self {
            Self::Mcp => "Settings.General.ToolExposure.mcp_group_desc",
            Self::Agent => "Settings.General.ToolExposure.agent_group_desc",
        }
    }

    fn current<'a>(self, settings: &'a AppSettings) -> &'a ToolExposureToolsetSettings {
        match self {
            Self::Mcp => &settings.tool_exposure.mcp,
            Self::Agent => &settings.tool_exposure.agent,
        }
    }

    fn current_mut<'a>(self, settings: &'a mut AppSettings) -> &'a mut ToolExposureToolsetSettings {
        match self {
            Self::Mcp => &mut settings.tool_exposure.mcp,
            Self::Agent => &mut settings.tool_exposure.agent,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ToolExposureItem {
    Terminal,
    TerminalSshExec,
    TerminalExec,
    Connections,
    Sftp,
    Redis,
    Database,
    InternalFunctions,
}

impl ToolExposureItem {
    const MCP_VISIBLE: [Self; 7] = [
        Self::Terminal,
        Self::TerminalSshExec,
        Self::TerminalExec,
        Self::Connections,
        Self::Sftp,
        Self::Redis,
        Self::Database,
    ];
    const AGENT_VISIBLE: [Self; 8] = [
        Self::Terminal,
        Self::TerminalSshExec,
        Self::TerminalExec,
        Self::Connections,
        Self::Sftp,
        Self::Redis,
        Self::Database,
        Self::InternalFunctions,
    ];

    #[cfg(test)]
    fn id(self) -> &'static str {
        match self {
            Self::Terminal => "terminal",
            Self::TerminalSshExec => "terminal_ssh_exec",
            Self::TerminalExec => "terminal_exec",
            Self::Connections => "connections",
            Self::Sftp => "sftp",
            Self::Redis => "redis",
            Self::Database => "database",
            Self::InternalFunctions => "internal_functions",
        }
    }

    fn title_key(self) -> &'static str {
        match self {
            Self::Terminal => "Settings.General.ToolExposure.terminal",
            Self::TerminalSshExec => "Settings.General.ToolExposure.terminal_ssh_exec",
            Self::TerminalExec => "Settings.General.ToolExposure.terminal_exec",
            Self::Connections => "Settings.General.ToolExposure.connections",
            Self::Sftp => "Settings.General.ToolExposure.sftp",
            Self::Redis => "Settings.General.ToolExposure.redis",
            Self::Database => "Settings.General.ToolExposure.database",
            Self::InternalFunctions => "Settings.General.ToolExposure.internal_functions",
        }
    }

    fn description_key(self) -> &'static str {
        match self {
            Self::Terminal => "Settings.General.ToolExposure.terminal_desc",
            Self::TerminalSshExec => "Settings.General.ToolExposure.terminal_ssh_exec_desc",
            Self::TerminalExec => "Settings.General.ToolExposure.terminal_exec_desc",
            Self::Connections => "Settings.General.ToolExposure.connections_desc",
            Self::Sftp => "Settings.General.ToolExposure.sftp_desc",
            Self::Redis => "Settings.General.ToolExposure.redis_desc",
            Self::Database => "Settings.General.ToolExposure.database_desc",
            Self::InternalFunctions => "Settings.General.ToolExposure.internal_functions_desc",
        }
    }

    fn get(self, settings: &ToolExposureToolsetSettings) -> bool {
        match self {
            Self::Terminal => settings.terminal,
            Self::TerminalSshExec => settings.terminal_ssh_exec,
            Self::TerminalExec => settings.terminal_exec,
            Self::Connections => settings.connections,
            Self::Sftp => settings.sftp,
            Self::Redis => settings.redis,
            Self::Database => settings.database,
            Self::InternalFunctions => settings.internal_functions,
        }
    }

    fn set(self, settings: &mut ToolExposureToolsetSettings, enabled: bool) {
        match self {
            Self::Terminal => settings.terminal = enabled,
            Self::TerminalSshExec => settings.terminal_ssh_exec = enabled,
            Self::TerminalExec => settings.terminal_exec = enabled,
            Self::Connections => settings.connections = enabled,
            Self::Sftp => settings.sftp = enabled,
            Self::Redis => settings.redis = enabled,
            Self::Database => settings.database = enabled,
            Self::InternalFunctions => settings.internal_functions = enabled,
        }
    }
}

fn tool_exposure_group(
    surface: ToolExposureSurface,
    default_settings: &ToolExposureToolsetSettings,
) -> SettingGroup {
    SettingGroup::new()
        .title(t!(surface.title_key()))
        .description(t!(surface.description_key()))
        .items(tool_exposure_items(surface, default_settings))
}

fn tool_exposure_items(
    surface: ToolExposureSurface,
    default_settings: &ToolExposureToolsetSettings,
) -> Vec<SettingItem> {
    surface
        .visible_items()
        .iter()
        .map(|item| tool_exposure_item(surface, *item, default_settings))
        .collect()
}

fn tool_exposure_item(
    surface: ToolExposureSurface,
    item: ToolExposureItem,
    default_settings: &ToolExposureToolsetSettings,
) -> SettingItem {
    SettingItem::new(
        t!(item.title_key()),
        SettingField::checkbox(
            move |cx: &App| item.get(surface.current(AppSettings::global(cx))),
            move |val: bool, cx: &mut App| {
                AppSettings::update_and_save(cx, |settings| {
                    item.set(surface.current_mut(settings), val);
                });
                if surface == ToolExposureSurface::Agent {
                    ai_chat_view::emit_agent_tool_config_changed(cx);
                }
            },
        )
        .default_value(item.get(default_settings)),
    )
    .description(t!(item.description_key()).to_string())
}

#[cfg(test)]
fn mcp_tool_exposure_item_ids() -> Vec<&'static str> {
    ToolExposureSurface::Mcp
        .visible_items()
        .iter()
        .map(|item| item.id())
        .collect()
}

#[cfg(test)]
fn agent_tool_exposure_item_ids() -> Vec<&'static str> {
    ToolExposureSurface::Agent
        .visible_items()
        .iter()
        .map(|item| item.id())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mcp_tool_exposure_items_include_public_toolsets_and_terminal_flavors() {
        let ids = mcp_tool_exposure_item_ids();

        assert_eq!(
            vec![
                "terminal",
                "terminal_ssh_exec",
                "terminal_exec",
                "connections",
                "sftp",
                "redis",
                "database"
            ],
            ids
        );
    }

    #[test]
    fn agent_tool_exposure_items_include_internal_agent_toolsets() {
        let ids = agent_tool_exposure_item_ids();

        assert_eq!(
            vec![
                "terminal",
                "terminal_ssh_exec",
                "terminal_exec",
                "connections",
                "sftp",
                "redis",
                "database",
                "internal_functions"
            ],
            ids
        );
    }
}
