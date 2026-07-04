use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExtensionKind {
    Language,
    LanguageBundle,
    DatabaseDriver,
    RemoteDesktopProvider,
    McpHelper,
    AcpAgent,
    Composite,
}

impl ExtensionKind {
    pub fn dir_name(self) -> &'static str {
        match self {
            Self::Language => "languages",
            Self::LanguageBundle => "language_bundles",
            Self::DatabaseDriver => "database_drivers",
            Self::RemoteDesktopProvider => "remote_desktop_providers",
            Self::McpHelper => "mcp_helpers",
            Self::AcpAgent => "acp_agents",
            Self::Composite => "composite",
        }
    }

    pub fn all() -> &'static [Self] {
        &[
            Self::Language,
            Self::LanguageBundle,
            Self::DatabaseDriver,
            Self::RemoteDesktopProvider,
            Self::McpHelper,
            Self::AcpAgent,
            Self::Composite,
        ]
    }
}
