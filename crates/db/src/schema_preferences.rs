use one_core::storage::{DatabaseType, DbConnectionConfig};

pub const DEFAULT_SCHEMA_PARAM: &str = "default_schema";
pub const SCHEMA_FILTER_MODE_PARAM: &str = "schema_filter_mode";
pub const SCHEMA_FILTER_INCLUDE_PARAM: &str = "schema_filter_include";
pub const SCHEMA_FILTER_EXCLUDE_PARAM: &str = "schema_filter_exclude";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SchemaFilterProfile {
    None,
    PostgreSql,
    Oracle,
    MsSql,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SchemaFilterMode {
    Auto,
    Include,
    Exclude,
    All,
}

pub fn schema_filter_profile_for_database_type(
    database_type: &DatabaseType,
) -> SchemaFilterProfile {
    match database_type {
        DatabaseType::PostgreSQL => SchemaFilterProfile::PostgreSql,
        DatabaseType::Oracle => SchemaFilterProfile::Oracle,
        DatabaseType::MSSQL => SchemaFilterProfile::MsSql,
        _ => SchemaFilterProfile::None,
    }
}

pub fn default_schema_from_config(config: &DbConnectionConfig) -> Option<String> {
    trimmed_param(config, DEFAULT_SCHEMA_PARAM)
}

pub fn schema_for_new_query(
    node_schema: Option<&str>,
    config: &DbConnectionConfig,
) -> Option<String> {
    node_schema
        .map(str::trim)
        .filter(|schema| !schema.is_empty())
        .map(ToOwned::to_owned)
        .or_else(|| default_schema_from_config(config))
}

pub fn filter_schemas(
    config: &DbConnectionConfig,
    profile: SchemaFilterProfile,
    schemas: Vec<String>,
) -> Vec<String> {
    let mode = filter_mode(config);
    let include_patterns = split_patterns(config.get_param(SCHEMA_FILTER_INCLUDE_PARAM));
    let exclude_patterns = split_patterns(config.get_param(SCHEMA_FILTER_EXCLUDE_PARAM));
    let default_schema = default_schema_from_config(config);

    schemas
        .into_iter()
        .filter(|schema| {
            keep_schema(
                schema,
                profile,
                mode,
                &include_patterns,
                &exclude_patterns,
                default_schema.as_deref(),
            )
        })
        .collect()
}

fn keep_schema(
    schema: &str,
    profile: SchemaFilterProfile,
    mode: SchemaFilterMode,
    include_patterns: &[String],
    exclude_patterns: &[String],
    default_schema: Option<&str>,
) -> bool {
    if default_schema.is_some_and(|default| schema_eq(default, schema)) {
        return true;
    }
    match mode {
        SchemaFilterMode::All => true,
        SchemaFilterMode::Include => {
            include_patterns.is_empty() || matches_any_pattern(include_patterns, schema)
        }
        SchemaFilterMode::Exclude => !matches_any_pattern(exclude_patterns, schema),
        SchemaFilterMode::Auto => {
            !is_system_schema(profile, schema) && !matches_any_pattern(exclude_patterns, schema)
        }
    }
}

fn filter_mode(config: &DbConnectionConfig) -> SchemaFilterMode {
    match config
        .get_param(SCHEMA_FILTER_MODE_PARAM)
        .map(String::as_str)
        .map(str::trim)
    {
        Some("all") => SchemaFilterMode::All,
        Some("include") => SchemaFilterMode::Include,
        Some("exclude") => SchemaFilterMode::Exclude,
        _ => SchemaFilterMode::Auto,
    }
}

fn trimmed_param(config: &DbConnectionConfig, key: &str) -> Option<String> {
    config
        .get_param(key)
        .map(String::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
}

fn split_patterns(value: Option<&String>) -> Vec<String> {
    value
        .map(String::as_str)
        .unwrap_or_default()
        .split([',', ';', '\n'])
        .map(str::trim)
        .filter(|pattern| !pattern.is_empty())
        .map(ToOwned::to_owned)
        .collect()
}

fn matches_any_pattern(patterns: &[String], schema: &str) -> bool {
    patterns
        .iter()
        .any(|pattern| matches_pattern(pattern, schema))
}

fn matches_pattern(pattern: &str, schema: &str) -> bool {
    let pattern = pattern.trim().to_ascii_lowercase();
    let schema = schema.trim().to_ascii_lowercase();
    if pattern == "*" {
        return true;
    }
    if !pattern.contains('*') {
        return pattern == schema;
    }

    match (pattern.strip_prefix('*'), pattern.strip_suffix('*')) {
        (Some(suffix), _) if !suffix.contains('*') => schema.ends_with(suffix),
        (_, Some(prefix)) if !prefix.contains('*') => schema.starts_with(prefix),
        _ => matches_multi_wildcard(&pattern, &schema),
    }
}

fn matches_multi_wildcard(pattern: &str, schema: &str) -> bool {
    let parts: Vec<&str> = pattern.split('*').filter(|part| !part.is_empty()).collect();
    if parts.is_empty() {
        return true;
    }
    let mut remainder = schema;
    for part in parts {
        let Some(index) = remainder.find(part) else {
            return false;
        };
        remainder = &remainder[index + part.len()..];
    }
    true
}

fn is_system_schema(profile: SchemaFilterProfile, schema: &str) -> bool {
    match profile {
        SchemaFilterProfile::None => false,
        SchemaFilterProfile::PostgreSql => is_postgres_system_schema(schema),
        SchemaFilterProfile::Oracle => is_oracle_system_schema(schema),
        SchemaFilterProfile::MsSql => is_mssql_system_schema(schema),
    }
}

fn is_postgres_system_schema(schema: &str) -> bool {
    let schema = schema.trim().to_ascii_lowercase();
    matches!(
        schema.as_str(),
        "pg_catalog" | "information_schema" | "pg_toast" | "sys_catalog"
    ) || schema.starts_with("pg_temp_")
        || schema.starts_with("pg_toast_temp_")
}

fn is_oracle_system_schema(schema: &str) -> bool {
    let schema = schema.trim().to_ascii_uppercase();
    matches!(
        schema.as_str(),
        "SYS"
            | "SYSTEM"
            | "SYSDBA"
            | "SYSAUDITOR"
            | "CTISYS"
            | "OUTLN"
            | "DBSNMP"
            | "WMSYS"
            | "XDB"
            | "MDSYS"
            | "ORDSYS"
            | "CTXSYS"
            | "DMSYS"
    ) || schema.starts_with("APEX_")
        || schema.starts_with("FLOWS_")
}

fn is_mssql_system_schema(schema: &str) -> bool {
    let schema = schema.trim().to_ascii_lowercase();
    matches!(
        schema.as_str(),
        "information_schema"
            | "sys"
            | "guest"
            | "db_owner"
            | "db_accessadmin"
            | "db_securityadmin"
            | "db_ddladmin"
            | "db_backupoperator"
            | "db_datareader"
            | "db_datawriter"
            | "db_denydatareader"
            | "db_denydatawriter"
    )
}

fn schema_eq(left: &str, right: &str) -> bool {
    left.trim().eq_ignore_ascii_case(right.trim())
}

#[cfg(test)]
#[path = "schema_preferences_tests.rs"]
mod tests;
