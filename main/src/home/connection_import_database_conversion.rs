use std::collections::HashMap;

use connection_import_protocol::{DatabaseImportRecord, ImportDatabaseType, ImportRecord};
use one_core::storage::{DatabaseType, DbConnectionConfig, StoredConnection};

use super::connection_import_draft::EditableImportDraft;
use super::connection_import_draft_conversion::{
    ConversionMode, normalize_identity_part, optional_port, optional_text, required_text,
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
        .ok_or_else(|| "数据库导入记录缺少数据库配置".to_string())?;
    let name = required_text(&draft.name, "连接名称")?;
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
        database,
        service_name: None,
        sid: None,
        workspace_id: None,
        extra_params: extra_params(imported),
    };
    Ok(StoredConnection::from_db_connection(config))
}

pub(crate) fn database_duplicate_identity(
    draft: &EditableImportDraft,
    record: &ImportRecord,
) -> Result<String, String> {
    let imported = record
        .database
        .as_ref()
        .ok_or_else(|| "数据库导入记录缺少数据库配置".to_string())?;
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
            None => Err("数据库文件路径不能为空".to_string()),
        };
    }
    match mode {
        ConversionMode::StrictSave => required_text(&draft.host, "主机"),
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
