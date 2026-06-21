use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExtensionKind {
    Language,
    DatabaseDriver,
    RemoteDesktopProvider,
    Composite,
}

impl ExtensionKind {
    pub fn dir_name(self) -> &'static str {
        match self {
            Self::Language => "languages",
            Self::DatabaseDriver => "database_drivers",
            Self::RemoteDesktopProvider => "remote_desktop_providers",
            Self::Composite => "composite",
        }
    }

    pub fn all() -> &'static [Self] {
        &[
            Self::Language,
            Self::DatabaseDriver,
            Self::RemoteDesktopProvider,
            Self::Composite,
        ]
    }
}
