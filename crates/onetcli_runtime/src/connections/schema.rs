use super::input::required_str;
use db::ipc::{IpcDriverManifest, IpcDriverRegistry};
use db::plugin::DatabasePlugin;
use db::plugin_manifest::{
    DatabaseFormField, DatabaseFormFieldType, DatabaseFormKind, DatabaseFormManifest,
    DatabaseFormTab, DatabaseUiManifest,
};
use one_core::storage::DatabaseType;
use serde_json::{Value, json};
use tool_runtime::ToolError;

pub(super) fn list_kinds() -> Value {
    list_kinds_with_registry(&IpcDriverRegistry::load_default())
}

pub(super) fn list_kinds_with_registry(registry: &IpcDriverRegistry) -> Value {
    json!({
        "kinds": [
            { "kind": "database", "database_types": database_type_keys(registry) },
            { "kind": "ssh_sftp" },
            { "kind": "redis" },
            { "kind": "mongodb" },
            { "kind": "serial" },
            { "kind": "port_forwarding" },
            { "kind": "rdp" },
            { "kind": "vnc" }
        ]
    })
}

pub(super) fn schema_for(input: Value) -> Result<Value, ToolError> {
    schema_for_with_registry(input, &IpcDriverRegistry::load_default())
}

pub(super) fn schema_for_with_registry(
    input: Value,
    registry: &IpcDriverRegistry,
) -> Result<Value, ToolError> {
    let kind = required_str(&input, "kind")?;
    let fields = match kind {
        "database" => database_schema(&input, registry)?,
        "ssh_sftp" => ssh_schema(),
        "redis" => redis_schema(),
        "mongodb" => mongodb_schema(),
        "serial" => serial_schema(),
        "port_forwarding" => port_forwarding_schema(),
        "rdp" => remote_desktop_schema(3389, true),
        "vnc" => remote_desktop_schema(5900, false),
        other => {
            return Err(ToolError::Failed {
                message: format!("unknown connection kind: {other}"),
            });
        }
    };
    Ok(json!({ "schema_version": 1, "kind": kind, "fields": fields }))
}

fn database_type_keys(registry: &IpcDriverRegistry) -> Vec<String> {
    let mut keys = DatabaseType::builtin_all()
        .iter()
        .map(DatabaseType::storage_key)
        .collect::<Vec<_>>();
    keys.extend(
        registry
            .drivers()
            .iter()
            .map(|driver| DatabaseType::external(driver.id.clone()).storage_key()),
    );
    keys
}

fn database_schema(input: &Value, registry: &IpcDriverRegistry) -> Result<Value, ToolError> {
    let database_type = parse_database_type(
        input
            .get("database_type")
            .and_then(Value::as_str)
            .unwrap_or("MySQL"),
    )?;
    let form = database_connection_form(&database_type, registry)?;
    Ok(Value::Array(
        form.tabs
            .iter()
            .flat_map(|tab| tab.fields.iter().map(schema_field))
            .collect(),
    ))
}

fn database_connection_form(
    database_type: &DatabaseType,
    registry: &IpcDriverRegistry,
) -> Result<DatabaseFormManifest, ToolError> {
    match database_type {
        DatabaseType::External { driver_id } => external_connection_form(driver_id, registry),
        DatabaseType::DuckDB => duckdb_connection_form(registry),
        _ => built_in_connection_form(database_type),
    }
}

fn built_in_connection_form(
    database_type: &DatabaseType,
) -> Result<DatabaseFormManifest, ToolError> {
    let manifest = built_in_database_ui_manifest(database_type)?;
    connection_form(&manifest).ok_or_else(|| ToolError::Failed {
        message: format!("missing connection form for database type: {database_type:?}"),
    })
}

fn built_in_database_ui_manifest(
    database_type: &DatabaseType,
) -> Result<DatabaseUiManifest, ToolError> {
    Ok(match database_type {
        DatabaseType::MySQL => db::mysql::MySqlPlugin::new().ui_manifest(),
        DatabaseType::PostgreSQL => db::postgresql::PostgresPlugin::new().ui_manifest(),
        DatabaseType::SQLite => db::sqlite::SqlitePlugin::new().ui_manifest(),
        DatabaseType::MSSQL => db::mssql::MsSqlPlugin::new().ui_manifest(),
        DatabaseType::Oracle => db::oracle::OraclePlugin::new().ui_manifest(),
        DatabaseType::ClickHouse => db::clickhouse::ClickHousePlugin::new().ui_manifest(),
        DatabaseType::TDengine => db::tdengine::TdenginePlugin::new().ui_manifest(),
        DatabaseType::DuckDB | DatabaseType::External { .. } => {
            return Err(ToolError::Failed {
                message: format!(
                    "database type is not a built-in manifest source: {database_type:?}"
                ),
            });
        }
    })
}

#[cfg(feature = "builtin-duckdb")]
fn duckdb_connection_form(
    _registry: &IpcDriverRegistry,
) -> Result<DatabaseFormManifest, ToolError> {
    let manifest = db::duckdb::DuckDbPlugin::new().ui_manifest();
    connection_form(&manifest).ok_or_else(|| ToolError::Failed {
        message: "missing connection form for database type: DuckDB".to_string(),
    })
}

#[cfg(not(feature = "builtin-duckdb"))]
fn duckdb_connection_form(registry: &IpcDriverRegistry) -> Result<DatabaseFormManifest, ToolError> {
    external_connection_form("duckdb", registry).map_err(|_| ToolError::Failed {
        message: "DuckDB requires the duckdb IPC driver when builtin-duckdb is disabled"
            .to_string(),
    })
}

fn external_connection_form(
    driver_id: &str,
    registry: &IpcDriverRegistry,
) -> Result<DatabaseFormManifest, ToolError> {
    let driver = registry.find(driver_id).ok_or_else(|| ToolError::Failed {
        message: format!("unknown external database driver: {driver_id}"),
    })?;
    if let Some(manifest) = driver.ui.form.as_ref() {
        if let Some(mut form) = connection_form(manifest) {
            apply_external_driver_defaults(&mut form, &driver);
            apply_external_driver_empty_tab_defaults(&mut form, &driver, registry);
            return Ok(form);
        }
    }
    Ok(default_external_connection_form(&driver))
}

fn connection_form(manifest: &DatabaseUiManifest) -> Option<DatabaseFormManifest> {
    manifest
        .forms
        .iter()
        .find(|form| form.kind == DatabaseFormKind::Connection)
        .cloned()
}

fn apply_external_driver_defaults(form: &mut DatabaseFormManifest, driver: &IpcDriverManifest) {
    for tab in &mut form.tabs {
        for field in &mut tab.fields {
            if field.id != "name" {
                continue;
            }
            if field
                .default_value
                .as_deref()
                .unwrap_or_default()
                .is_empty()
            {
                field.default_value = Some(driver.name.clone());
            }
            return;
        }
    }
}

fn apply_external_driver_empty_tab_defaults(
    form: &mut DatabaseFormManifest,
    driver: &IpcDriverManifest,
    registry: &IpcDriverRegistry,
) {
    for tab in &mut form.tabs {
        if !tab.fields.is_empty() {
            continue;
        }
        if let Some(default_tab) = external_driver_default_tab(driver, &tab.id, registry) {
            tab.fields = default_tab.fields;
        }
    }
}

fn external_driver_default_tab(
    driver: &IpcDriverManifest,
    tab_id: &str,
    registry: &IpcDriverRegistry,
) -> Option<DatabaseFormTab> {
    match tab_id {
        "general" => find_tab(default_external_connection_form(driver), "general"),
        "ssh" => find_tab(
            connection_form_for_default_tab(&DatabaseType::MySQL, registry).ok()?,
            "ssh",
        ),
        "ssl" => find_tab(
            external_driver_compatible_host_form(driver, registry)?,
            "ssl",
        )
        .or_else(|| {
            find_tab(
                connection_form_for_default_tab(&DatabaseType::MySQL, registry).ok()?,
                "ssl",
            )
        }),
        "notes" | "remark" => find_tab(
            connection_form_for_default_tab(&DatabaseType::MySQL, registry).ok()?,
            "notes",
        )
        .map(|mut tab| {
            tab.id = tab_id.to_string();
            tab
        }),
        _ => None,
    }
}

fn external_driver_compatible_host_form(
    driver: &IpcDriverManifest,
    registry: &IpcDriverRegistry,
) -> Option<DatabaseFormManifest> {
    let database_type = driver
        .dialect
        .compatible_database_type
        .as_ref()
        .unwrap_or(&DatabaseType::MySQL);
    connection_form_for_default_tab(database_type, registry)
        .ok()
        .or_else(|| connection_form_for_default_tab(&DatabaseType::MySQL, registry).ok())
}

fn connection_form_for_default_tab(
    database_type: &DatabaseType,
    registry: &IpcDriverRegistry,
) -> Result<DatabaseFormManifest, ToolError> {
    match database_type {
        DatabaseType::DuckDB => duckdb_connection_form(registry),
        DatabaseType::External { driver_id } => external_connection_form(driver_id, registry),
        _ => built_in_connection_form(database_type),
    }
}

fn find_tab(form: DatabaseFormManifest, tab_id: &str) -> Option<DatabaseFormTab> {
    form.tabs.into_iter().find(|tab| tab.id == tab_id)
}

fn default_external_connection_form(driver: &IpcDriverManifest) -> DatabaseFormManifest {
    DatabaseFormManifest {
        kind: DatabaseFormKind::Connection,
        title_i18n_key: "Common.new".to_string(),
        submit_i18n_key: "Common.save".to_string(),
        tabs: vec![DatabaseFormTab {
            id: "general".to_string(),
            label_i18n_key: "ConnectionForm.general".to_string(),
            fields: vec![
                form_field(
                    "name",
                    DatabaseFormFieldType::Text,
                    true,
                    Some(&driver.name),
                ),
                form_field("host", DatabaseFormFieldType::Text, true, Some("localhost")),
                form_field(
                    "port",
                    DatabaseFormFieldType::Number,
                    true,
                    Some(&driver.ui.default_port.unwrap_or_default().to_string()),
                ),
                form_field("username", DatabaseFormFieldType::Text, false, None),
                form_field("password", DatabaseFormFieldType::Password, false, None),
                form_field("database", DatabaseFormFieldType::Text, false, None),
            ],
        }],
    }
}

fn form_field(
    id: &str,
    field_type: DatabaseFormFieldType,
    required: bool,
    default_value: Option<&str>,
) -> DatabaseFormField {
    DatabaseFormField {
        id: id.to_string(),
        label_i18n_key: format!("ConnectionForm.{id}"),
        field_type,
        required,
        default_value: default_value.map(str::to_string),
        placeholder_i18n_key: None,
        help_i18n_key: None,
        options: Vec::new(),
        options_source: None,
        visible_when: Vec::new(),
        default_when: Vec::new(),
        disabled_when_editing: false,
        rows: None,
        min: None,
        max: None,
    }
}

fn schema_field(field: &DatabaseFormField) -> Value {
    let default = field
        .default_value
        .as_deref()
        .map(|value| default_value(value, field.field_type))
        .unwrap_or(Value::Null);
    let mut schema = json!({
        "name": field.id,
        "type": field_type(field.field_type),
        "required": field.required,
        "default": default
    });
    if field.field_type == DatabaseFormFieldType::Password {
        schema["secret"] = json!(true);
    }
    if !field.options.is_empty() {
        schema["enum"] = Value::Array(
            field
                .options
                .iter()
                .map(|option| Value::String(option.value.clone()))
                .collect(),
        );
    }
    if let Some(rows) = field.rows {
        schema["rows"] = json!(rows);
    }
    if let Some(min) = field.min {
        schema["min"] = json!(min);
    }
    if let Some(max) = field.max {
        schema["max"] = json!(max);
    }
    schema
}

fn field_type(field_type: DatabaseFormFieldType) -> &'static str {
    match field_type {
        DatabaseFormFieldType::Number => "integer",
        DatabaseFormFieldType::Password
        | DatabaseFormFieldType::TextArea
        | DatabaseFormFieldType::Select
        | DatabaseFormFieldType::FilePath
        | DatabaseFormFieldType::Text => "string",
        DatabaseFormFieldType::Checkbox => "boolean",
    }
}

fn default_value(value: &str, field_type: DatabaseFormFieldType) -> Value {
    if value.is_empty() {
        return Value::Null;
    }
    match field_type {
        DatabaseFormFieldType::Number => value
            .parse::<i64>()
            .map(Value::from)
            .unwrap_or_else(|_| Value::String(value.to_string())),
        DatabaseFormFieldType::Checkbox => value
            .parse::<bool>()
            .map(Value::from)
            .unwrap_or_else(|_| Value::String(value.to_string())),
        _ => Value::String(value.to_string()),
    }
}

fn parse_database_type(value: &str) -> Result<DatabaseType, ToolError> {
    DatabaseType::from_storage_key(value).ok_or_else(|| ToolError::Failed {
        message: format!("unknown database type: {value}"),
    })
}

fn ssh_schema() -> Value {
    json!([
        field("name", "string", true, Value::Null),
        field("host", "string", true, Value::Null),
        field("port", "integer", false, json!(22)),
        field("username", "string", true, Value::Null),
        secret_field("password"),
        field("default_directory", "string", false, Value::Null)
    ])
}

fn redis_schema() -> Value {
    json!([
        field("name", "string", true, Value::Null),
        field("host", "string", true, Value::Null),
        field("port", "integer", false, json!(6379)),
        field("username", "string", false, Value::Null),
        secret_field("password"),
        field("db_index", "integer", false, json!(0))
    ])
}

fn mongodb_schema() -> Value {
    json!([
        field("name", "string", true, Value::Null),
        enum_field(
            "driver_variant",
            &["modern", "legacy", "legacy_3_2"],
            false,
            json!("modern")
        ),
        field("connection_string", "string", false, Value::Null),
        field("host", "string", false, Value::Null),
        field("port", "integer", false, json!(27017)),
        field("username", "string", false, Value::Null),
        secret_field("password"),
        field("database", "string", false, Value::Null)
    ])
}

fn serial_schema() -> Value {
    json!([
        field("name", "string", true, Value::Null),
        field("port_name", "string", true, Value::Null),
        field("baud_rate", "integer", false, json!(115200)),
        field("data_bits", "integer", false, json!(8)),
        field("stop_bits", "integer", false, json!(1)),
        enum_field("parity", &["None", "Odd", "Even"], false, json!("None")),
        enum_field(
            "flow_control",
            &["None", "Software", "Hardware"],
            false,
            json!("None"),
        )
    ])
}

fn port_forwarding_schema() -> Value {
    json!([
        field("name", "string", true, Value::Null),
        field("ssh_connection_id", "integer", true, Value::Null),
        enum_field(
            "kind",
            &["Local", "Remote", "Dynamic"],
            false,
            json!("Local"),
        ),
        field("bind_host", "string", false, json!("127.0.0.1")),
        field("bind_port", "integer", true, Value::Null),
        field("target_host", "string", false, Value::Null),
        field("target_port", "integer", false, Value::Null)
    ])
}

fn remote_desktop_schema(default_port: u16, supports_audio_playback: bool) -> Value {
    let mut fields = vec![
        field("name", "string", true, Value::Null),
        field("host", "string", true, Value::Null),
        field("port", "integer", false, json!(default_port)),
        field("username", "string", false, Value::Null),
        secret_field("password"),
        field("domain", "string", false, Value::Null),
        field("read_only", "boolean", false, json!(false)),
    ];
    if supports_audio_playback {
        fields.push(field("audio_playback", "boolean", false, json!(false)));
    }
    Value::Array(fields)
}

fn field(name: &str, field_type: &str, required: bool, default: Value) -> Value {
    json!({ "name": name, "type": field_type, "required": required, "default": default })
}

fn enum_field(name: &str, values: &[&str], required: bool, default: Value) -> Value {
    json!({
        "name": name,
        "type": "string",
        "required": required,
        "enum": values,
        "default": default
    })
}

fn secret_field(name: &str) -> Value {
    json!({ "name": name, "type": "string", "required": false, "secret": true })
}
