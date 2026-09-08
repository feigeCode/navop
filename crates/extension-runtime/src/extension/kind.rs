use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExtensionKind {
    Language,
    LanguageBundle,
    DatabaseDriver,
    RemoteDesktopProvider,
    AcpAgent,
    Composite,
    /// 不再支持的旧 kind（如 mcp_helper）。仅用于容错解析遗留市场清单，
    /// 不参与安装与列表。
    #[serde(other)]
    Unsupported,
}

impl ExtensionKind {
    pub fn dir_name(self) -> &'static str {
        match self {
            Self::Language => "languages",
            Self::LanguageBundle => "language_bundles",
            Self::DatabaseDriver => "database_drivers",
            Self::RemoteDesktopProvider => "remote_desktop_providers",
            Self::AcpAgent => "acp_agents",
            Self::Composite => "composite",
            Self::Unsupported => "unsupported",
        }
    }

    pub fn all() -> &'static [Self] {
        &[
            Self::Language,
            Self::LanguageBundle,
            Self::DatabaseDriver,
            Self::RemoteDesktopProvider,
            Self::AcpAgent,
            Self::Composite,
        ]
    }
}
