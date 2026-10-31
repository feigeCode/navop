use std::collections::HashMap;

use connection_import_protocol::{DatabaseImportRecord, ImportDatabaseType, ImportRecord};
use one_core::storage::{
    DatabaseType, DbConnectionConfig, MongoDBParams, RedisMode, RedisParams, StoredConnection,
};
use rust_i18n::t;

use super::connection_import_draft::EditableImportDraft;
use super::connection_import_draft_conversion::{
    ConversionMode, mongodb_identity, normalize_identity_part, optional_port, optional_text,
    redis_identity, required_text,
};

struct DatabaseIdentity<'a> {
    database_type: &'a DatabaseType,
    host: &'a str,
    port: u16,
    username: &'a str,
    database: &'a str,
}

pub(crate) fn to_database_connection(
    draft: &EditableImportDraft,
    record: &ImportRecord,
    mode: ConversionMode,
) -> Result<StoredConnection, String> {
    let imported = record
        .database
        .as_ref()
        .ok_or_else(|| t!("Home.ConnectionImport.database_config_missing").to_string())?;
    let name = required_text(
        &draft.name,
        t!("Home.ConnectionImport.field_connection_name").as_ref(),
    )?;
    if let Some(connection) = native_external_connection(draft, imported, &name, mode)? {
        return Ok(connection);
    }

    let database_type = storage_database_type(&imported.database_type);
    let port = optional_port(&draft.port)?
        .or_else(|| default_database_port(&database_type))
        .unwrap_or_default();
    let host = database_host(draft, &database_type, mode)?;
    let database = database_name(draft, &database_type);
    let config = DbConnectionConfig {
        id: String::new(),
        database_type,
        name: name.clone(),
        host,
        port,
        username: draft.username.trim().to_string(),
        password: optional_text(&draft.password).unwrap_or_default(),
        credential_reference: None,
        database,
        service_name: None,
        sid: None,
        workspace_id: None,
        proxy: None,
        extra_params: extra_params(imported),
    };
    Ok(StoredConnection::from_db_connection(config))
}

fn native_external_connection(
    draft: &EditableImportDraft,
    imported: &DatabaseImportRecord,
    name: &str,
    mode: ConversionMode,
) -> Result<Option<StoredConnection>, String> {
    let ImportDatabaseType::External { id } = &imported.database_type else {
        return Ok(None);
    };

    match normalize_external_driver_id(id).as_str() {
        "mongodb" | "mongo" => Ok(Some(StoredConnection::new_mongodb(
            name.to_string(),
            mongodb_params(draft, mode)?,
            None,
        ))),
        "redis" => Ok(Some(StoredConnection::new_redis(
            name.to_string(),
            redis_params(draft, mode)?,
            None,
        ))),
        _ => Ok(None),
    }
}

fn normalize_external_driver_id(driver_id: &str) -> String {
    driver_id
        .trim()
        .to_ascii_lowercase()
        .replace(['_', '-', ' '], "")
}

fn mongodb_params(
    draft: &EditableImportDraft,
    mode: ConversionMode,
) -> Result<MongoDBParams, String> {
    Ok(MongoDBParams {
        driver_variant: Default::default(),
        connection_string: String::new(),
        host: host_for_native_external(draft, mode)?,
        port: Some(optional_port(&draft.port)?.unwrap_or(27017)),
        database: optional_text(&draft.database),
        username: optional_text(&draft.username),
        password: optional_text(&draft.password),
        credential_reference: None,
        auth_source: None,
        replica_set: None,
        read_preference: None,
        use_srv_record: false,
        direct_connection: false,
        use_tls: false,
        connect_timeout_seconds: None,
        application_name: None,
        ssh_tunnel: None,
    })
}

fn redis_params(draft: &EditableImportDraft, mode: ConversionMode) -> Result<RedisParams, String> {
    Ok(RedisParams {
        host: host_for_native_external(draft, mode)?,
        port: optional_port(&draft.port)?.unwrap_or(6379),
        password: optional_text(&draft.password),
        username: optional_text(&draft.username),
        credential_reference: None,
        db_index: redis_db_index(&draft.database),
        mode: RedisMode::Standalone,
        use_tls: false,
        connect_timeout: None,
        sentinel: None,
        cluster: None,
        ssh_tunnel: None,
    })
}

fn host_for_native_external(
    draft: &EditableImportDraft,
    mode: ConversionMode,
) -> Result<String, String> {
    match mode {
        ConversionMode::StrictSave => {
            required_text(&draft.host, t!("Home.ConnectionImport.field_host").as_ref())
        }
        ConversionMode::EditorPrefill => Ok(optional_text(&draft.host).unwrap_or_default()),
    }
}

fn redis_db_index(database: &str) -> u8 {
    database.trim().parse().unwrap_or_default()
}

pub(crate) fn database_duplicate_identity(
    draft: &EditableImportDraft,
    record: &ImportRecord,
) -> Result<String, String> {
    let imported = record
        .database
        .as_ref()
        .ok_or_else(|| t!("Home.ConnectionImport.database_config_missing").to_string())?;
    if let Some(identity) = native_external_duplicate_identity(draft, imported)? {
        return Ok(identity);
    }

    let database_type = storage_database_type(&imported.database_type);
    let port = optional_port(&draft.port)?
        .or_else(|| default_database_port(&database_type))
        .unwrap_or_default();
    let host = database_identity_host(draft, &database_type);
    let database = if is_file_database(&database_type) {
        ""
    } else {
        draft.database.as_str()
    };
    Ok(database_identity(DatabaseIdentity {
        database_type: &database_type,
        host: &host,
        port,
        username: &draft.username,
        database,
    }))
}

fn native_external_duplicate_identity(
    draft: &EditableImportDraft,
    imported: &DatabaseImportRecord,
) -> Result<Option<String>, String> {
    let ImportDatabaseType::External { id } = &imported.database_type else {
        return Ok(None);
    };

    match normalize_external_driver_id(id).as_str() {
        "mongodb" | "mongo" => Ok(Some(mongodb_identity(
            &draft.host,
            optional_port(&draft.port)?.unwrap_or(27017),
            &draft.username,
            &draft.database,
        ))),
        "redis" => Ok(Some(redis_identity(
            &draft.host,
            optional_port(&draft.port)?.unwrap_or(6379),
            &draft.username,
            redis_db_index(&draft.database),
        ))),
        _ => Ok(None),
    }
}

pub(crate) fn database_config_duplicate_identity(config: &DbConnectionConfig) -> String {
    database_identity(DatabaseIdentity {
        database_type: &config.database_type,
        host: &config.host,
        port: config.port,
        username: &config.username,
        database: config.database.as_deref().unwrap_or_default(),
    })
}

fn storage_database_type(database_type: &ImportDatabaseType) -> DatabaseType {
    match database_type {
        ImportDatabaseType::MySql => DatabaseType::MySQL,
        ImportDatabaseType::PostgreSql => DatabaseType::PostgreSQL,
        ImportDatabaseType::Sqlite => DatabaseType::SQLite,
        ImportDatabaseType::DuckDb => DatabaseType::DuckDB,
        ImportDatabaseType::SqlServer => DatabaseType::MSSQL,
        ImportDatabaseType::Oracle => DatabaseType::Oracle,
        ImportDatabaseType::ClickHouse => DatabaseType::ClickHouse,
        ImportDatabaseType::TDengine => DatabaseType::TDengine,
        ImportDatabaseType::External { id } => DatabaseType::External {
            driver_id: id.clone(),
        },
    }
}

fn database_identity(identity: DatabaseIdentity<'_>) -> String {
    format!(
        "db:{}:{}:{}:{}:{}",
        database_type_identity(identity.database_type),
        normalize_identity_part(identity.host),
        identity.port,
        normalize_identity_part(identity.username),
        normalize_identity_part(identity.database)
    )
}

fn database_type_identity(database_type: &DatabaseType) -> String {
    match database_type {
        DatabaseType::MySQL => "mysql".to_string(),
        DatabaseType::PostgreSQL => "postgresql".to_string(),
        DatabaseType::SQLite => "sqlite".to_string(),
        DatabaseType::DuckDB => "duckdb".to_string(),
        DatabaseType::MSSQL => "sqlserver".to_string(),
        DatabaseType::Oracle => "oracle".to_string(),
        DatabaseType::ClickHouse => "clickhouse".to_string(),
        DatabaseType::TDengine => "tdengine".to_string(),
        DatabaseType::External { driver_id } => format!("external:{driver_id}"),
    }
}

fn default_database_port(database_type: &DatabaseType) -> Option<u16> {
    match database_type {
        DatabaseType::MySQL => Some(3306),
        DatabaseType::PostgreSQL => Some(5432),
        DatabaseType::MSSQL => Some(1433),
        DatabaseType::Oracle => Some(1521),
        DatabaseType::ClickHouse => Some(8123),
        // TDengine 经 taosAdapter 的 WebSocket 端口。
        DatabaseType::TDengine => Some(6041),
        DatabaseType::SQLite | DatabaseType::DuckDB | DatabaseType::External { .. } => None,
    }
}

fn database_host(
    draft: &EditableImportDraft,
    database_type: &DatabaseType,
    mode: ConversionMode,
) -> Result<String, String> {
    if is_file_database(database_type) {
        return match file_database_path(draft) {
            Some(path) => Ok(path),
            None if matches!(mode, ConversionMode::EditorPrefill) => Ok(String::new()),
            None => Err(t!("Home.ConnectionImport.database_file_required").to_string()),
        };
    }
    match mode {
        ConversionMode::StrictSave => {
            required_text(&draft.host, t!("Home.ConnectionImport.field_host").as_ref())
        }
        ConversionMode::EditorPrefill => Ok(optional_text(&draft.host).unwrap_or_default()),
    }
}

fn database_name(draft: &EditableImportDraft, database_type: &DatabaseType) -> Option<String> {
    if is_file_database(database_type) {
        None
    } else {
        optional_text(&draft.database)
    }
}

fn database_identity_host(draft: &EditableImportDraft, database_type: &DatabaseType) -> String {
    if is_file_database(database_type) {
        file_database_path(draft).unwrap_or_default()
    } else {
        draft.host.clone()
    }
}

fn file_database_path(draft: &EditableImportDraft) -> Option<String> {
    optional_text(&draft.host).or_else(|| optional_text(&draft.database))
}

fn is_file_database(database_type: &DatabaseType) -> bool {
    matches!(database_type, DatabaseType::SQLite | DatabaseType::DuckDB)
}

fn extra_params(imported: &DatabaseImportRecord) -> HashMap<String, String> {
    imported
        .extra_params
        .iter()
        .map(|(key, value)| (key.clone(), value.clone()))
        .collect()
}
