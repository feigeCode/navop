use gpui::App;
use gpui_component::setting::{SettingField, SettingGroup, SettingItem};
use one_core::settings::{AppSettings, ToolExposureSettings, ToolExposureToolsetSettings};
use rust_i18n::t;

pub fn mcp_tool_exposure_setting_group(default_settings: &ToolExposureSettings) -> SettingGroup {
    tool_exposure_group(
        ToolExposureSurface::Mcp,
        &default_settings.mcp,
        ToolExposureCapabilities::current(),
    )
}

pub fn agent_tool_exposure_setting_group(default_settings: &ToolExposureSettings) -> SettingGroup {
    tool_exposure_group(
        ToolExposureSurface::Agent,
        &default_settings.agent,
        ToolExposureCapabilities::current(),
    )
}

/// 当前产品的领域能力，用来按产品裁剪工具暴露设置页。
///
/// 终端与数据库两组工具集只在装配了对应领域的产品里存在：`navop-shell` 没有
/// 数据库领域，页面上出现数据库 / Redis / MongoDB 开关只会保存一个永远不生效的
/// 值；反过来的 `navop-data` 也一样。能力显式传参，单测不依赖领域注册表的全局
/// 初始化顺序。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct ToolExposureCapabilities {
    shell: bool,
    database: bool,
}

impl ToolExposureCapabilities {
    /// 全量产品两个领域都装配；独立产品只装配自己的领域。
    fn current() -> Self {
        Self {
            shell: crate::product::supports_shell_domain(),
            database: crate::product::supports_database_domain(),
        }
    }

    #[cfg(test)]
    const FULL: Self = Self {
        shell: true,
        database: true,
    };

    #[cfg(test)]
    const SHELL_ONLY: Self = Self {
        shell: true,
        database: false,
    };

    #[cfg(test)]
    const DATABASE_ONLY: Self = Self {
        shell: false,
        database: true,
    };
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
    Mongo,
    Database,
    Notes,
    InternalFunctions,
    Extensions,
}

/// 工具集所属领域，用来按产品裁剪设置页。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ToolExposureDomain {
    /// 终端 / SSH / SFTP：只有装配了终端领域的产品才有这些工具集。
    Shell,
    /// 数据库 / Redis / MongoDB：只有装配了数据库领域的产品才有这些工具集。
    Database,
    /// 连接、笔记、内置函数、扩展：三产品一致。
    General,
}

impl ToolExposureItem {
    const MCP_VISIBLE: [Self; 11] = [
        Self::Terminal,
        Self::TerminalSshExec,
        Self::TerminalExec,
        Self::Connections,
        Self::Sftp,
        Self::Redis,
        Self::Mongo,
        Self::Database,
        Self::Notes,
        Self::InternalFunctions,
        Self::Extensions,
    ];
    const AGENT_VISIBLE: [Self; 11] = [
        Self::Terminal,
        Self::TerminalSshExec,
        Self::TerminalExec,
        Self::Connections,
        Self::Sftp,
        Self::Redis,
        Self::Mongo,
        Self::Database,
        Self::Notes,
        Self::InternalFunctions,
        Self::Extensions,
    ];

    fn domain(self) -> ToolExposureDomain {
        match self {
            Self::Terminal | Self::TerminalSshExec | Self::TerminalExec | Self::Sftp => {
                ToolExposureDomain::Shell
            }
            Self::Redis | Self::Mongo | Self::Database => ToolExposureDomain::Database,
            Self::Connections | Self::Notes | Self::InternalFunctions | Self::Extensions => {
                ToolExposureDomain::General
            }
        }
    }

    /// 该工具集在当前产品里是否真的存在。
    ///
    /// 领域能力决定终端 / 数据库两组工具集；扩展工具集还要求进程带 nsh 宿主，
    /// 否则只会保存一个无效开关。
    fn available(self, capabilities: ToolExposureCapabilities, nsh_ui: bool) -> bool {
        if self == Self::Extensions && !nsh_ui {
            return false;
        }
        match self.domain() {
            ToolExposureDomain::Shell => capabilities.shell,
            ToolExposureDomain::Database => capabilities.database,
            ToolExposureDomain::General => true,
        }
    }

    #[cfg(test)]
    fn id(self) -> &'static str {
        match self {
            Self::Terminal => "terminal",
            Self::TerminalSshExec => "terminal_ssh_exec",
            Self::TerminalExec => "terminal_exec",
            Self::Connections => "connections",
            Self::Sftp => "sftp",
            Self::Redis => "redis",
            Self::Mongo => "mongo",
            Self::Database => "database",
            Self::Notes => "notes",
            Self::InternalFunctions => "internal_functions",
            Self::Extensions => "extensions",
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
            Self::Mongo => "Settings.General.ToolExposure.mongo",
            Self::Database => "Settings.General.ToolExposure.database",
            Self::Notes => "Settings.General.ToolExposure.notes",
            Self::InternalFunctions => "Settings.General.ToolExposure.internal_functions",
            Self::Extensions => "Settings.General.ToolExposure.extensions",
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
            Self::Mongo => "Settings.General.ToolExposure.mongo_desc",
            Self::Database => "Settings.General.ToolExposure.database_desc",
            Self::Notes => "Settings.General.ToolExposure.notes_desc",
            Self::InternalFunctions => "Settings.General.ToolExposure.internal_functions_desc",
            Self::Extensions => "Settings.General.ToolExposure.extensions_desc",
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
            Self::Mongo => settings.mongo,
            Self::Database => settings.database,
            Self::Notes => settings.notes,
            Self::InternalFunctions => settings.internal_functions,
            Self::Extensions => settings.extensions,
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
            Self::Mongo => settings.mongo = enabled,
            Self::Database => settings.database = enabled,
            Self::Notes => settings.notes = enabled,
            Self::InternalFunctions => settings.internal_functions = enabled,
            Self::Extensions => settings.extensions = enabled,
        }
    }
}

fn tool_exposure_group(
    surface: ToolExposureSurface,
    default_settings: &ToolExposureToolsetSettings,
    capabilities: ToolExposureCapabilities,
) -> SettingGroup {
    SettingGroup::new()
        .title(t!(surface.title_key()))
        .description(t!(surface.description_key()))
        .items(tool_exposure_items(surface, default_settings, capabilities))
}

fn tool_exposure_items(
    surface: ToolExposureSurface,
    default_settings: &ToolExposureToolsetSettings,
    capabilities: ToolExposureCapabilities,
) -> Vec<SettingItem> {
    surface
        .visible_items()
        .iter()
        .filter(|item| item.available(capabilities, crate::nsh_ui_available()))
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
fn tool_exposure_item_ids(
    surface: ToolExposureSurface,
    capabilities: ToolExposureCapabilities,
) -> Vec<&'static str> {
    surface
        .visible_items()
        .iter()
        .filter(|item| item.available(capabilities, crate::nsh_ui_available()))
        .map(|item| item.id())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 全量产品的完整清单；扩展项只在带 nsh 宿主的构建里存在。
    fn full_item_ids() -> Vec<&'static str> {
        let mut ids = vec![
            "terminal",
            "terminal_ssh_exec",
            "terminal_exec",
            "connections",
            "sftp",
            "redis",
            "mongo",
            "database",
            "notes",
            "internal_functions",
            "extensions",
        ];
        if !crate::nsh_ui_available() {
            ids.pop();
        }
        ids
    }

    fn with_extensions(ids: &[&'static str]) -> Vec<&'static str> {
        let mut ids = ids.to_vec();
        if crate::nsh_ui_available() {
            ids.push("extensions");
        }
        ids
    }

    #[test]
    fn full_product_exposes_all_toolsets() {
        for surface in [ToolExposureSurface::Mcp, ToolExposureSurface::Agent] {
            assert_eq!(
                full_item_ids(),
                tool_exposure_item_ids(surface, ToolExposureCapabilities::FULL)
            );
        }
    }

    /// 终端产品没有数据库领域：数据库 / Redis / MongoDB 开关操作的是不存在的工具集。
    #[test]
    fn shell_product_hides_database_toolsets() {
        let expected = with_extensions(&[
            "terminal",
            "terminal_ssh_exec",
            "terminal_exec",
            "connections",
            "sftp",
            "notes",
            "internal_functions",
        ]);
        for surface in [ToolExposureSurface::Mcp, ToolExposureSurface::Agent] {
            assert_eq!(
                expected,
                tool_exposure_item_ids(surface, ToolExposureCapabilities::SHELL_ONLY)
            );
        }
    }

    /// 数据库产品没有终端领域：终端 / SSH / SFTP 开关操作的是不存在的工具集。
    #[test]
    fn database_product_hides_terminal_toolsets() {
        let expected = with_extensions(&[
            "connections",
            "redis",
            "mongo",
            "database",
            "notes",
            "internal_functions",
        ]);
        for surface in [ToolExposureSurface::Mcp, ToolExposureSurface::Agent] {
            assert_eq!(
                expected,
                tool_exposure_item_ids(surface, ToolExposureCapabilities::DATABASE_ONLY)
            );
        }
    }
}
