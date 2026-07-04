use super::*;
use one_core::storage::{DatabaseType, DbConnectionConfig};
use std::collections::HashMap;

fn config(extra_params: &[(&str, &str)]) -> DbConnectionConfig {
    DbConnectionConfig {
        id: "conn-1".to_string(),
        database_type: DatabaseType::PostgreSQL,
        name: "postgres".to_string(),
        host: "localhost".to_string(),
        port: 5432,
        username: "user".to_string(),
        password: "password".to_string(),
        database: Some("app".to_string()),
        service_name: None,
        sid: None,
        workspace_id: None,
        extra_params: extra_params
            .iter()
            .map(|(key, value)| (key.to_string(), value.to_string()))
            .collect::<HashMap<_, _>>(),
    }
}

fn names(values: &[&str]) -> Vec<String> {
    values.iter().map(|value| value.to_string()).collect()
}

#[test]
fn auto_mode_hides_postgres_system_schemas() {
    let filtered = filter_schemas(
        &config(&[]),
        SchemaFilterProfile::PostgreSql,
        names(&[
            "pg_catalog",
            "information_schema",
            "pg_toast",
            "pg_temp_7",
            "pg_toast_temp_7",
            "sys_catalog",
            "public",
            "app",
        ]),
    );

    assert_eq!(names(&["public", "app"]), filtered);
}

#[test]
fn all_mode_keeps_system_schemas_visible() {
    let filtered = filter_schemas(
        &config(&[(SCHEMA_FILTER_MODE_PARAM, "all")]),
        SchemaFilterProfile::PostgreSql,
        names(&["pg_catalog", "information_schema", "public"]),
    );

    assert_eq!(
        names(&["pg_catalog", "information_schema", "public"]),
        filtered
    );
}

#[test]
fn include_mode_uses_exact_and_wildcard_patterns() {
    let filtered = filter_schemas(
        &config(&[
            (SCHEMA_FILTER_MODE_PARAM, "include"),
            (SCHEMA_FILTER_INCLUDE_PARAM, "app, audit_*"),
        ]),
        SchemaFilterProfile::PostgreSql,
        names(&["public", "app", "audit_2026", "other"]),
    );

    assert_eq!(names(&["app", "audit_2026"]), filtered);
}

#[test]
fn exclude_mode_hides_user_patterns_without_auto_filtering() {
    let filtered = filter_schemas(
        &config(&[
            (SCHEMA_FILTER_MODE_PARAM, "exclude"),
            (SCHEMA_FILTER_EXCLUDE_PARAM, "tmp_*,scratch"),
        ]),
        SchemaFilterProfile::PostgreSql,
        names(&["pg_catalog", "public", "tmp_1", "scratch", "app"]),
    );

    assert_eq!(names(&["pg_catalog", "public", "app"]), filtered);
}

#[test]
fn configured_default_schema_stays_visible_in_auto_mode() {
    let filtered = filter_schemas(
        &config(&[(DEFAULT_SCHEMA_PARAM, "pg_catalog")]),
        SchemaFilterProfile::PostgreSql,
        names(&["pg_catalog", "public", "app"]),
    );

    assert_eq!(names(&["pg_catalog", "public", "app"]), filtered);
}

#[test]
fn auto_mode_hides_oracle_style_system_schemas() {
    let filtered = filter_schemas(
        &config(&[]),
        SchemaFilterProfile::Oracle,
        names(&["SYS", "SYSTEM", "SYSAUDITOR", "APEX_220100", "APP_USER"]),
    );

    assert_eq!(names(&["APP_USER"]), filtered);
}

#[test]
fn configured_default_schema_is_normalized() {
    let schema = default_schema_from_config(&config(&[(DEFAULT_SCHEMA_PARAM, " app ")]));

    assert_eq!(Some("app".to_string()), schema);
}

#[test]
fn new_query_uses_node_schema_before_configured_default_schema() {
    let config = config(&[(DEFAULT_SCHEMA_PARAM, "app")]);

    assert_eq!(
        Some("node_schema".to_string()),
        schema_for_new_query(Some("node_schema"), &config)
    );
    assert_eq!(Some("app".to_string()), schema_for_new_query(None, &config));
}
