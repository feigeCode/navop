use one_core::storage::DatabaseType;
use std::collections::HashMap;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ImportSourceKind {
    DBeaver,
    TablePlus,
    SequelAce,
    BeekeeperStudio,
}

impl ImportSourceKind {
    pub fn display_name(self) -> &'static str {
        match self {
            Self::DBeaver => "DBeaver",
            Self::TablePlus => "TablePlus",
            Self::SequelAce => "Sequel Ace",
            Self::BeekeeperStudio => "Beekeeper Studio",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SourceAvailability {
    NotInstalled,
    Installed,
    Unsupported,
    NoConnections,
    Available { connection_count: usize },
    PermissionRequired,
    Error { message: String },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ImportSourceStatus {
    pub kind: ImportSourceKind,
    pub display_name: String,
    pub availability: SourceAvailability,
}

impl ImportSourceStatus {
    pub fn new(kind: ImportSourceKind, availability: SourceAvailability) -> Self {
        Self {
            kind,
            display_name: kind.display_name().to_string(),
            availability,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ImportOptions {
    pub include_passwords: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PasswordImportStatus {
    Included,
    Missing,
    Unsupported,
    PermissionDenied,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ImportedConnection {
    pub source: ImportSourceKind,
    pub source_id: String,
    pub name: String,
    pub database_type: DatabaseType,
    pub host: String,
    pub port: Option<u16>,
    pub username: String,
    pub password: Option<String>,
    pub database: Option<String>,
    pub extra_params: HashMap<String, String>,
    pub password_status: PasswordImportStatus,
}

#[derive(Debug, thiserror::Error)]
pub enum ImportError {
    #[error("invalid source data: {0}")]
    InvalidSourceData(String),
    #[error("unable to read source data: {0}")]
    ReadSourceData(String),
    #[error("unsupported import source: {0}")]
    UnsupportedSource(String),
    #[error("unsupported database type: {0}")]
    UnsupportedDatabaseType(String),
    #[error("missing required field: {0}")]
    MissingField(&'static str),
    #[error("source data not found: {0}")]
    SourceDataNotFound(String),
}
