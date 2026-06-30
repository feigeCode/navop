mod beekeeper;
mod credentials;
mod datagrip;
mod dbeaver;
mod model;
mod sequel_ace;
mod simple_encryptor;
mod tableplus;
mod xshell;

pub use credentials::{
    CredentialQuery, CredentialStore, NoopCredentialStore, SystemCredentialStore,
};
pub use datagrip::parse_datagrip_data_sources_xml;
pub use dbeaver::parse_dbeaver_data_sources_json;
pub use model::{
    ImportError, ImportOptions, ImportSourceKind, ImportSourceStatus, ImportedConnection,
    ImportedSshAuthMethod, ImportedSshConnection, PasswordImportStatus, SourceAvailability,
};
pub use sequel_ace::parse_sequel_ace_favorites_plist_with_credentials;
pub use tableplus::parse_tableplus_connections_json_with_credentials;
pub use xshell::parse_xshell_session;

use one_core::storage::{DatabaseType, DbConnectionConfig, SshAuthMethod, SshParams};
use std::path::Path;

pub fn list_sources() -> Vec<ImportSourceStatus> {
    vec![
        ImportSourceStatus::new(
            ImportSourceKind::TablePlus,
            tableplus::detect_availability(),
        ),
        ImportSourceStatus::new(
            ImportSourceKind::SequelAce,
            sequel_ace::detect_availability(),
        ),
        ImportSourceStatus::new(ImportSourceKind::DBeaver, dbeaver::detect_availability()),
        ImportSourceStatus::new(
            ImportSourceKind::BeekeeperStudio,
            beekeeper::detect_availability(),
        ),
        ImportSourceStatus::new(ImportSourceKind::DataGrip, datagrip::detect_availability()),
        ImportSourceStatus::new(ImportSourceKind::Xshell, xshell::detect_availability()),
    ]
}

pub fn preview_connections(
    kind: ImportSourceKind,
    options: ImportOptions,
) -> Result<Vec<ImportedConnection>, ImportError> {
    let system_credentials = SystemCredentialStore;
    preview_connections_with_credentials(kind, options, &system_credentials)
}

pub fn preview_connections_with_credentials(
    kind: ImportSourceKind,
    options: ImportOptions,
    credentials: &dyn CredentialStore,
) -> Result<Vec<ImportedConnection>, ImportError> {
    match kind {
        ImportSourceKind::DBeaver => dbeaver::preview_default_connections(options),
        ImportSourceKind::TablePlus => tableplus::preview_default_connections(options, credentials),
        ImportSourceKind::SequelAce => {
            sequel_ace::preview_default_connections(options, credentials)
        }
        ImportSourceKind::BeekeeperStudio => beekeeper::preview_default_connections(options),
        ImportSourceKind::DataGrip => datagrip::preview_default_connections(options),
        ImportSourceKind::Xshell => Err(ImportError::UnsupportedSource(
            kind.display_name().to_string(),
        )),
    }
}

pub fn preview_connections_from_path(
    kind: ImportSourceKind,
    path: impl AsRef<Path>,
    options: ImportOptions,
) -> Result<Vec<ImportedConnection>, ImportError> {
    let no_credentials = NoopCredentialStore;
    preview_connections_from_path_with_credentials(kind, path, options, &no_credentials)
}

pub fn preview_connections_from_path_with_credentials(
    kind: ImportSourceKind,
    path: impl AsRef<Path>,
    options: ImportOptions,
    credentials: &dyn CredentialStore,
) -> Result<Vec<ImportedConnection>, ImportError> {
    match kind {
        ImportSourceKind::DBeaver => dbeaver::preview_connections_from_path(path, options),
        ImportSourceKind::TablePlus => {
            tableplus::preview_connections_from_path(path, options, credentials)
        }
        ImportSourceKind::SequelAce => {
            sequel_ace::preview_connections_from_path(path, options, credentials)
        }
        ImportSourceKind::BeekeeperStudio => {
            beekeeper::preview_connections_from_path(path, options)
        }
        ImportSourceKind::DataGrip => datagrip::preview_connections_from_path(path, options),
        ImportSourceKind::Xshell => Err(ImportError::UnsupportedSource(
            kind.display_name().to_string(),
        )),
    }
}

pub fn preview_ssh_connections(
    kind: ImportSourceKind,
    options: ImportOptions,
) -> Result<Vec<ImportedSshConnection>, ImportError> {
    match kind {
        ImportSourceKind::Xshell => xshell::preview_default_ssh_connections(options),
        _ => Err(ImportError::UnsupportedSource(
            kind.display_name().to_string(),
        )),
    }
}

pub fn preview_ssh_connections_from_path(
    kind: ImportSourceKind,
    path: impl AsRef<Path>,
    options: ImportOptions,
) -> Result<Vec<ImportedSshConnection>, ImportError> {
    match kind {
        ImportSourceKind::Xshell => xshell::preview_ssh_connections_from_path(path, options),
        _ => Err(ImportError::UnsupportedSource(
            kind.display_name().to_string(),
        )),
    }
}

pub fn to_db_connection_config(
    imported: ImportedConnection,
) -> Result<DbConnectionConfig, ImportError> {
    let port = imported
        .port
        .or_else(|| default_port(&imported.database_type))
        .unwrap_or_default();

    Ok(DbConnectionConfig {
        id: String::new(),
        database_type: imported.database_type,
        name: imported.name,
        host: imported.host,
        port,
        username: imported.username,
        password: imported.password.unwrap_or_default(),
        database: imported.database,
        service_name: None,
        sid: None,
        workspace_id: None,
        extra_params: imported.extra_params,
    })
}

pub fn to_ssh_params(imported: ImportedSshConnection) -> Result<SshParams, ImportError> {
    Ok(SshParams {
        host: imported.host,
        port: imported.port,
        username: imported.username,
        auth_method: to_ssh_auth_method(imported.auth_method),
        connect_timeout: None,
        keepalive_interval: None,
        keepalive_max: None,
        default_directory: None,
        init_script: None,
        disable_shell_integration: None,
        jump_server: None,
        proxy: None,
    })
}

fn to_ssh_auth_method(auth_method: ImportedSshAuthMethod) -> SshAuthMethod {
    match auth_method {
        ImportedSshAuthMethod::Password { password } => SshAuthMethod::Password {
            password: password.unwrap_or_default(),
        },
        ImportedSshAuthMethod::PrivateKey {
            key_path,
            passphrase,
        } => SshAuthMethod::PrivateKey {
            key_path,
            passphrase,
        },
        ImportedSshAuthMethod::Agent => SshAuthMethod::Agent,
        ImportedSshAuthMethod::AutoPublicKey => SshAuthMethod::AutoPublicKey,
    }
}

pub fn duplicate_fingerprint(imported: &ImportedConnection) -> String {
    let port = imported
        .port
        .or_else(|| default_port(&imported.database_type))
        .unwrap_or_default();
    format!(
        "{}|{}|{}|{}|{}",
        imported.database_type.storage_key(),
        imported.host.trim().to_lowercase(),
        port,
        imported.username.trim().to_lowercase(),
        imported.database.as_deref().unwrap_or_default().trim()
    )
}

fn default_port(database_type: &DatabaseType) -> Option<u16> {
    match database_type {
        DatabaseType::MySQL => Some(3306),
        DatabaseType::PostgreSQL => Some(5432),
        DatabaseType::MSSQL => Some(1433),
        DatabaseType::Oracle => Some(1521),
        DatabaseType::ClickHouse => Some(8123),
        DatabaseType::SQLite | DatabaseType::DuckDB | DatabaseType::External { .. } => None,
    }
}
