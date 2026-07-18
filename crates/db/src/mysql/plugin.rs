use std::collections::HashMap;
use std::sync::LazyLock;

use crate::types::ObjectViewColumn as Column;
use anyhow::Result;
use one_core::storage::{DatabaseType, DbConnectionConfig};
use rust_i18n::t;

use crate::connection::{DbConnection, DbError};
use crate::executor::SqlResult;
use crate::import_export::{
    ExportConfig, ExportProgressSender, ExportResult, ImportConfig, ImportProgressSender,
    ImportResult,
};
use crate::mysql::connection::MysqlDbConnection;
use crate::plugin::{DatabasePlugin, DatabaseUserOperationRequest, SqlCompletionInfo};
use crate::plugin_manifest::{
    DatabaseActionDescriptor, DatabaseActionId, DatabaseActionManifest, DatabaseActionPlacement,
    DatabaseActionTarget, DatabaseActionToolbarScope, DatabaseCapabilities, DatabaseFormField,
    DatabaseFormFieldType, DatabaseFormKind, DatabaseFormManifest, DatabaseFormTab,
    DatabaseUiCapabilities, DatabaseUiManifest, FormDefaultRule, FormSelectOption,
    FormValueCondition, FormVisibilityRule, ReferenceDataKind,
};
use crate::types::*;

/// MySQL data types (name, description)
pub const MYSQL_DATA_TYPES: &[(&str, &str)] = &[
    ("TINYINT", "Very small integer (-128 to 127)"),
    ("SMALLINT", "Small integer (-32768 to 32767)"),
    ("MEDIUMINT", "Medium integer (-8388608 to 8388607)"),
    ("INT", "Standard integer (-2147483648 to 2147483647)"),
    ("BIGINT", "Large integer"),
    ("DECIMAL", "Fixed-point number"),
    ("FLOAT", "Single-precision floating-point"),
    ("DOUBLE", "Double-precision floating-point"),
    ("BIT", "Bit field"),
    ("CHAR", "Fixed-length string"),
    ("VARCHAR", "Variable-length string"),
    ("TINYTEXT", "Very small text (255 bytes)"),
    ("TEXT", "Text (65KB)"),
    ("MEDIUMTEXT", "Medium text (16MB)"),
    ("LONGTEXT", "Large text (4GB)"),
    ("BINARY", "Fixed-length binary"),
    ("VARBINARY", "Variable-length binary"),
    ("TINYBLOB", "Very small BLOB (255 bytes)"),
    ("BLOB", "BLOB (65KB)"),
    ("MEDIUMBLOB", "Medium BLOB (16MB)"),
    ("LONGBLOB", "Large BLOB (4GB)"),
    ("DATE", "Date (YYYY-MM-DD)"),
    ("TIME", "Time (HH:MM:SS)"),
    ("DATETIME", "Date and time"),
    ("TIMESTAMP", "Timestamp with timezone"),
    ("YEAR", "Year (1901-2155)"),
    ("BOOLEAN", "Boolean (TINYINT(1))"),
    ("JSON", "JSON document"),
    ("ENUM", "Enumeration"),
    ("SET", "Set of values"),
];

/// MySQL database plugin implementation (stateless)
pub struct MySqlPlugin;

const MYSQL_ENGINES: &[&str] = &[
    "InnoDB",
    "MyISAM",
    "MEMORY",
    "CSV",
    "ARCHIVE",
    "BLACKHOLE",
    "FEDERATED",
];

static MYSQL_UI_MANIFEST: LazyLock<DatabaseUiManifest> = LazyLock::new(build_mysql_ui_manifest);

fn executable_mysql_option(value: Option<&str>) -> Option<&str> {
    let value = value?.trim();
    if value.is_empty() || value.to_ascii_lowercase().starts_with("default ") {
        return None;
    }
    value
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || ch == '_' || ch == '$')
        .then_some(value)
}

impl MySqlPlugin {
    pub fn new() -> Self {
        Self
    }

    fn foreign_key_action_sql(action: &str) -> Option<String> {
        let action = action.trim();
        if action.is_empty() {
            return None;
        }
        let action = action
            .split_whitespace()
            .map(str::to_ascii_uppercase)
            .collect::<Vec<_>>()
            .join(" ");
        match action.as_str() {
            "CASCADE" | "RESTRICT" | "NO ACTION" | "SET NULL" | "SET DEFAULT" => Some(action),
            _ => None,
        }
    }

    fn foreign_key_changed(left: &ForeignKeyDefinition, right: &ForeignKeyDefinition) -> bool {
        left.columns != right.columns
            || left.ref_table != right.ref_table
            || left.ref_columns != right.ref_columns
            || Self::foreign_key_action_sql(&left.on_delete)
                != Self::foreign_key_action_sql(&right.on_delete)
            || Self::foreign_key_action_sql(&left.on_update)
                != Self::foreign_key_action_sql(&right.on_update)
    }

    fn column_change_reasons(
        original: &ColumnDefinition,
        new: &ColumnDefinition,
    ) -> Vec<&'static str> {
        let mut reasons = Vec::new();

        if original.data_type.to_uppercase() != new.data_type.to_uppercase() {
            reasons.push("data_type");
        }
        if original.length != new.length {
            reasons.push("length");
        }
        if original.precision != new.precision {
            reasons.push("precision");
        }
        if original.scale != new.scale {
            reasons.push("scale");
        }
        if original.is_nullable != new.is_nullable {
            reasons.push("is_nullable");
        }
        if original.is_auto_increment != new.is_auto_increment {
            reasons.push("is_auto_increment");
        }
        if original.is_unsigned != new.is_unsigned {
            reasons.push("is_unsigned");
        }
        if original.default_value != new.default_value {
            reasons.push("default_value");
        }
        if original.comment != new.comment {
            reasons.push("comment");
        }
        if original.charset != new.charset {
            reasons.push("charset");
        }
        if original.collation != new.collation {
            reasons.push("collation");
        }

        reasons
    }
}

fn build_mysql_ui_manifest() -> DatabaseUiManifest {
    let mut forms = vec![
        mysql_connection_form(),
        mysql_database_form(false),
        mysql_database_form(true),
    ];
    forms.extend(mysql_user_forms());

    DatabaseUiManifest {
        capabilities: DatabaseUiCapabilities {
            supports_schema: false,
            uses_schema_as_database: false,
            supports_views: true,
            supports_indexes: true,
            supports_users: true,
            supports_user_create: true,
            supports_user_edit: true,
            supports_user_delete: true,
            supports_user_privileges: true,
            supports_sequences: false,
            supports_functions: true,
            supports_procedures: true,
            supports_triggers: true,
            supports_table_engine: true,
            supports_table_charset: true,
            supports_table_collation: true,
            supports_auto_increment: true,
            supports_tablespace: false,
            supports_unsigned: true,
            supports_enum_values: true,
            show_charset_in_column_detail: true,
            show_collation_in_column_detail: true,
            table_engines: mysql_engine_names(),
        },
        forms,
        actions: mysql_action_manifest(),
        ..DatabaseUiManifest::default()
    }
}

fn mysql_engine_names() -> Vec<String> {
    MYSQL_ENGINES
        .iter()
        .map(|engine| (*engine).to_string())
        .collect()
}

fn mysql_foreign_keys_sql(database: &str, table: &str) -> String {
    format!(
        "SELECT k.CONSTRAINT_NAME, k.COLUMN_NAME, k.REFERENCED_TABLE_NAME, \
         k.REFERENCED_COLUMN_NAME, rc.DELETE_RULE, rc.UPDATE_RULE \
         FROM INFORMATION_SCHEMA.KEY_COLUMN_USAGE k \
         LEFT JOIN INFORMATION_SCHEMA.REFERENTIAL_CONSTRAINTS rc \
           ON rc.CONSTRAINT_SCHEMA = k.CONSTRAINT_SCHEMA \
          AND rc.CONSTRAINT_NAME = k.CONSTRAINT_NAME \
          AND rc.TABLE_NAME = k.TABLE_NAME \
         WHERE k.TABLE_SCHEMA = '{}' AND k.TABLE_NAME = '{}' \
           AND k.REFERENCED_TABLE_NAME IS NOT NULL \
         ORDER BY k.CONSTRAINT_NAME, k.ORDINAL_POSITION",
        database, table
    )
}

fn mysql_table_triggers_sql(database: &str, table: &str) -> String {
    format!(
        "SELECT TRIGGER_NAME, EVENT_OBJECT_TABLE, EVENT_MANIPULATION, ACTION_TIMING, \
         ACTION_STATEMENT \
         FROM INFORMATION_SCHEMA.TRIGGERS \
         WHERE TRIGGER_SCHEMA = '{}' AND EVENT_OBJECT_TABLE = '{}' \
         ORDER BY TRIGGER_NAME",
        database, table
    )
}

fn row_value(row: &[Option<String>], index: usize) -> String {
    row.get(index).and_then(|v| v.clone()).unwrap_or_default()
}

fn parse_mysql_foreign_keys(rows: Vec<Vec<Option<String>>>) -> Vec<ForeignKeyDefinition> {
    let mut foreign_keys = Vec::new();
    let mut positions = HashMap::new();

    for row in rows {
        let name = row_value(&row, 0);
        if name.is_empty() {
            continue;
        }

        let index = *positions.entry(name.clone()).or_insert_with(|| {
            foreign_keys.push(ForeignKeyDefinition {
                name: name.clone(),
                columns: Vec::new(),
                ref_table: row_value(&row, 2),
                ref_columns: Vec::new(),
                on_delete: row_value(&row, 4),
                on_update: row_value(&row, 5),
            });
            foreign_keys.len() - 1
        });

        foreign_keys[index].columns.push(row_value(&row, 1));
        foreign_keys[index].ref_columns.push(row_value(&row, 3));
    }

    foreign_keys
}

fn parse_mysql_triggers(rows: Vec<Vec<Option<String>>>) -> Vec<TriggerInfo> {
    rows.into_iter()
        .map(|row| TriggerInfo {
            name: row_value(&row, 0),
            table_name: row_value(&row, 1),
            event: row_value(&row, 2),
            timing: row_value(&row, 3),
            definition: row.get(4).and_then(|v| v.clone()),
        })
        .collect()
}

fn mysql_string_literal(value: &str) -> String {
    format!("'{}'", value.replace('\'', "''"))
}

fn mysql_user_host(request: &DatabaseUserOperationRequest) -> &str {
    request
        .field_values
        .get("host")
        .map(String::as_str)
        .or(request.host.as_deref())
        .filter(|host| !host.trim().is_empty())
        .unwrap_or("%")
}

fn mysql_user_account(request: &DatabaseUserOperationRequest) -> String {
    format!(
        "{}@{}",
        mysql_string_literal(&request.user_name),
        mysql_string_literal(mysql_user_host(request))
    )
}

fn mysql_user_password(request: &DatabaseUserOperationRequest) -> &str {
    request
        .field_values
        .get("password")
        .map(String::as_str)
        .filter(|password| !password.is_empty())
        .unwrap_or("change_me")
}

fn mysql_user_privileges(request: &DatabaseUserOperationRequest) -> &str {
    match request.field_values.get("privileges").map(String::as_str) {
        Some("SELECT") => "SELECT",
        Some("INSERT") => "INSERT",
        Some("UPDATE") => "UPDATE",
        Some("DELETE") => "DELETE",
        Some("ALL PRIVILEGES") => "ALL PRIVILEGES",
        _ => "SELECT",
    }
}

fn mysql_user_forms() -> Vec<DatabaseFormManifest> {
    vec![
        mysql_user_form(DatabaseFormKind::CreateUser, true, true, false),
        mysql_user_form(DatabaseFormKind::EditUser, true, true, false),
        mysql_user_form(DatabaseFormKind::DeleteUser, true, false, false),
        mysql_user_form(DatabaseFormKind::UserPrivileges, true, false, true),
    ]
}

fn mysql_user_form(
    kind: DatabaseFormKind,
    include_host: bool,
    include_password: bool,
    include_privileges: bool,
) -> DatabaseFormManifest {
    let mut fields = vec![field(
        "name",
        "DatabaseUser.name",
        DatabaseFormFieldType::Text,
    )];
    if include_host {
        fields.push(
            field("host", "DatabaseUser.host", DatabaseFormFieldType::Text)
                .optional()
                .with_default("%"),
        );
    }
    if include_password {
        fields.push(field(
            "password",
            "DatabaseUser.password",
            DatabaseFormFieldType::Password,
        ));
    }
    if include_privileges {
        fields.push(field(
            "database",
            "DatabaseUser.database",
            DatabaseFormFieldType::Text,
        ));
        fields.push(
            field(
                "privileges",
                "DatabaseUser.privileges",
                DatabaseFormFieldType::Select,
            )
            .with_default("SELECT")
            .with_options(mysql_user_privilege_options()),
        );
    }
    DatabaseFormManifest {
        kind,
        title_i18n_key: user_form_title_key(kind).into(),
        submit_i18n_key: "Common.save".into(),
        tabs: vec![tab("user", "DatabaseUser.user_tab", fields)],
    }
}

fn mysql_user_privilege_options() -> Vec<FormSelectOption> {
    vec![
        option("SELECT", "DatabaseUser.privilege_select"),
        option("INSERT", "DatabaseUser.privilege_insert"),
        option("UPDATE", "DatabaseUser.privilege_update"),
        option("DELETE", "DatabaseUser.privilege_delete"),
        option("ALL PRIVILEGES", "DatabaseUser.privilege_all"),
    ]
}

fn user_form_title_key(kind: DatabaseFormKind) -> &'static str {
    match kind {
        DatabaseFormKind::CreateUser => "DatabaseUser.create_title",
        DatabaseFormKind::EditUser => "DatabaseUser.edit_title",
        DatabaseFormKind::DeleteUser => "DatabaseUser.delete_title",
        DatabaseFormKind::UserPrivileges => "DatabaseUser.privileges_title",
        _ => "DatabaseUser.user_title",
    }
}

fn mysql_connection_form() -> DatabaseFormManifest {
    DatabaseFormManifest {
        kind: DatabaseFormKind::Connection,
        title_i18n_key: "Common.new".into(),
        submit_i18n_key: "Common.save".into(),
        tabs: vec![
            tab(
                "general",
                "ConnectionForm.general",
                vec![
                    field(
                        "name",
                        "ConnectionForm.connection_name",
                        DatabaseFormFieldType::Text,
                    )
                    .with_placeholder("My MySQL Database")
                    .with_default("Local MySQL"),
                    field("host", "ConnectionForm.host", DatabaseFormFieldType::Text)
                        .with_placeholder("localhost")
                        .with_default("localhost"),
                    field("port", "ConnectionForm.port", DatabaseFormFieldType::Number)
                        .with_placeholder("3306")
                        .with_default("3306"),
                    field(
                        "username",
                        "ConnectionForm.username",
                        DatabaseFormFieldType::Text,
                    )
                    .with_placeholder("root")
                    .with_default("root"),
                    field(
                        "password",
                        "ConnectionForm.password",
                        DatabaseFormFieldType::Password,
                    )
                    .with_placeholder("Enter password"),
                    field(
                        "database",
                        "ConnectionForm.database",
                        DatabaseFormFieldType::Text,
                    )
                    .optional()
                    .with_placeholder("database name (optional)"),
                ],
            ),
            tab(
                "advanced",
                "ConnectionForm.advanced",
                vec![
                    field(
                        "connect_timeout",
                        "ConnectionForm.connect_timeout",
                        DatabaseFormFieldType::Number,
                    )
                    .optional()
                    .with_placeholder("30")
                    .with_default("30"),
                    field(
                        "charset",
                        "ConnectionForm.charset",
                        DatabaseFormFieldType::Text,
                    )
                    .optional()
                    .with_placeholder("gbk"),
                    field(
                        "collation",
                        "ConnectionForm.collation",
                        DatabaseFormFieldType::Text,
                    )
                    .optional()
                    .with_placeholder("gbk_chinese_ci"),
                    field(
                        "read_timeout",
                        "ConnectionForm.read_timeout",
                        DatabaseFormFieldType::Number,
                    )
                    .optional()
                    .with_placeholder("28800"),
                ],
            ),
            tab(
                "ssl",
                "ConnectionForm.ssl",
                vec![
                    field(
                        "require_ssl",
                        "ConnectionForm.require_ssl",
                        DatabaseFormFieldType::Select,
                    )
                    .optional()
                    .with_default("false")
                    .with_options(yes_no_options()),
                    field(
                        "verify_ca",
                        "ConnectionForm.verify_ca",
                        DatabaseFormFieldType::Select,
                    )
                    .optional()
                    .with_default("true")
                    .with_options(yes_no_options()),
                    field(
                        "verify_identity",
                        "ConnectionForm.verify_identity",
                        DatabaseFormFieldType::Select,
                    )
                    .optional()
                    .with_default("true")
                    .with_options(yes_no_options()),
                    field(
                        "ssl_root_cert_path",
                        "ConnectionForm.ssl_root_cert_path",
                        DatabaseFormFieldType::Text,
                    )
                    .optional()
                    .with_placeholder("ConnectionForm.ssl_root_cert_path_placeholder"),
                    field(
                        "tls_hostname_override",
                        "ConnectionForm.tls_hostname_override",
                        DatabaseFormFieldType::Text,
                    )
                    .optional()
                    .with_placeholder("ConnectionForm.tls_hostname_override_placeholder"),
                ],
            ),
            tab(
                "ssh",
                "ConnectionForm.ssh",
                vec![
                    field(
                        "ssh_tunnel_enabled",
                        "ConnectionForm.ssh_tunnel_enabled",
                        DatabaseFormFieldType::Select,
                    )
                    .optional()
                    .with_default("false")
                    .with_options(yes_no_options()),
                    ssh_field("ssh_host", "ConnectionForm.ssh_host")
                        .with_placeholder("jump.example.com"),
                    ssh_number_field("ssh_port", "ConnectionForm.ssh_port")
                        .with_default("22")
                        .with_placeholder("22"),
                    ssh_field("ssh_username", "ConnectionForm.ssh_username")
                        .with_placeholder("root"),
                    field(
                        "ssh_auth_type",
                        "ConnectionForm.ssh_auth_type",
                        DatabaseFormFieldType::Select,
                    )
                    .optional()
                    .with_default("password")
                    .with_options(vec![
                        option("password", "ConnectionForm.ssh_auth_password"),
                        option("private_key", "ConnectionForm.ssh_auth_private_key"),
                        option("agent", "ConnectionForm.ssh_auth_agent"),
                    ])
                    .with_visibility(ssh_enabled_rules()),
                    ssh_password_field(
                        "ssh_password",
                        "ConnectionForm.ssh_password",
                        "Enter SSH password",
                    )
                    .with_visibility(ssh_auth_rules("password")),
                    ssh_field(
                        "ssh_private_key_path",
                        "ConnectionForm.ssh_private_key_path",
                    )
                    .with_placeholder("~/.ssh/id_rsa")
                    .with_visibility(ssh_auth_rules("private_key")),
                    ssh_password_field(
                        "ssh_private_key_passphrase",
                        "ConnectionForm.ssh_private_key_passphrase",
                        "Enter key passphrase",
                    )
                    .with_visibility(ssh_auth_rules("private_key")),
                    ssh_field("ssh_target_host", "ConnectionForm.ssh_target_host")
                        .with_placeholder("127.0.0.1"),
                    ssh_number_field("ssh_target_port", "ConnectionForm.ssh_target_port")
                        .with_placeholder("3306"),
                ],
            ),
            tab(
                "notes",
                "ConnectionForm.notes",
                vec![
                    field(
                        "remark",
                        "ConnectionForm.remark",
                        DatabaseFormFieldType::TextArea,
                    )
                    .optional()
                    .with_rows(14)
                    .with_placeholder("ConnectionForm.enter_remark"),
                ],
            ),
        ],
    }
}

fn mysql_database_form(is_edit_mode: bool) -> DatabaseFormManifest {
    DatabaseFormManifest {
        kind: if is_edit_mode {
            DatabaseFormKind::EditDatabase
        } else {
            DatabaseFormKind::CreateDatabase
        },
        title_i18n_key: if is_edit_mode {
            "Database.edit_database".into()
        } else {
            "Database.new_database".into()
        },
        submit_i18n_key: if is_edit_mode {
            "Common.save".into()
        } else {
            "Common.create".into()
        },
        tabs: vec![tab(
            "general",
            "ConnectionForm.general",
            vec![
                field(
                    "name",
                    "Database.database_name",
                    DatabaseFormFieldType::Text,
                )
                .with_placeholder("Database.enter_database_name")
                .disabled_when_editing(is_edit_mode),
                field(
                    "charset",
                    "ConnectionForm.charset",
                    DatabaseFormFieldType::Select,
                )
                .optional()
                .with_default("utf8mb4")
                .with_options_source(ReferenceDataKind::MySqlCharsets),
                field(
                    "collation",
                    "ConnectionForm.collation",
                    DatabaseFormFieldType::Select,
                )
                .optional()
                .with_default("utf8mb4_general_ci")
                .with_options_source(ReferenceDataKind::MySqlCollations)
                .with_default_rules(vec![FormDefaultRule {
                    when_field_changes: "charset".into(),
                    via: ReferenceDataKind::MySqlCollations,
                }]),
            ],
        )],
    }
}

fn mysql_action_manifest() -> DatabaseActionManifest {
    DatabaseActionManifest {
        actions: vec![
            action(
                DatabaseActionId::RunSqlFile,
                "ImportExport.run_sql_file",
                vec![DbNodeType::Connection, DbNodeType::Database],
                DatabaseActionPlacement::ContextMenu,
            ),
            action_with_scope(
                DatabaseActionId::CloseConnection,
                "Connection.close_connection",
                vec![DbNodeType::Connection],
                DatabaseActionPlacement::Both,
                false,
                Some(DatabaseActionToolbarScope::SelectedRow),
            ),
            action_with_scope(
                DatabaseActionId::DeleteConnection,
                "Connection.delete_connection",
                vec![DbNodeType::Connection],
                DatabaseActionPlacement::Both,
                false,
                Some(DatabaseActionToolbarScope::SelectedRow),
            ),
            action_with_scope(
                DatabaseActionId::CreateDatabase,
                "Database.new_database",
                vec![DbNodeType::Connection],
                DatabaseActionPlacement::Both,
                true,
                Some(DatabaseActionToolbarScope::CurrentNode),
            ),
            action_with_scope(
                DatabaseActionId::EditDatabase,
                "Database.edit_database",
                vec![DbNodeType::Database],
                DatabaseActionPlacement::Both,
                true,
                Some(DatabaseActionToolbarScope::SelectedRow),
            ),
            action(
                DatabaseActionId::CloseDatabase,
                "Database.close_database",
                vec![DbNodeType::Database],
                DatabaseActionPlacement::ContextMenu,
            )
            .always_enabled(),
            action_with_scope(
                DatabaseActionId::DeleteDatabase,
                "Database.delete_database",
                vec![DbNodeType::Database],
                DatabaseActionPlacement::Both,
                false,
                Some(DatabaseActionToolbarScope::SelectedRow),
            ),
            action(
                DatabaseActionId::DesignTable,
                "Table.new_table",
                vec![DbNodeType::Database, DbNodeType::TablesFolder],
                DatabaseActionPlacement::Both,
            )
            .with_toolbar_scope(DatabaseActionToolbarScope::CurrentNode),
            action(
                DatabaseActionId::DesignTable,
                "Table.design_table",
                vec![DbNodeType::Table],
                DatabaseActionPlacement::Both,
            )
            .with_toolbar_scope(DatabaseActionToolbarScope::CurrentNode),
            action_with_scope(
                DatabaseActionId::OpenTableData,
                "Table.view_data",
                vec![DbNodeType::Table],
                DatabaseActionPlacement::Both,
                true,
                Some(DatabaseActionToolbarScope::SelectedRow),
            ),
            action_with_scope(
                DatabaseActionId::OpenTableData,
                "Table.view_data",
                vec![DbNodeType::Table],
                DatabaseActionPlacement::Toolbar,
                true,
                Some(DatabaseActionToolbarScope::CurrentNode),
            ),
            action(
                DatabaseActionId::RenameTable,
                "Table.rename_table",
                vec![DbNodeType::Table],
                DatabaseActionPlacement::ContextMenu,
            ),
            action(
                DatabaseActionId::CopyTable,
                "Table.copy_table",
                vec![DbNodeType::Table],
                DatabaseActionPlacement::ContextMenu,
            ),
            action(
                DatabaseActionId::TruncateTable,
                "Table.truncate_table",
                vec![DbNodeType::Table],
                DatabaseActionPlacement::ContextMenu,
            ),
            action_with_scope(
                DatabaseActionId::DeleteTable,
                "Table.delete_table",
                vec![DbNodeType::Table],
                DatabaseActionPlacement::Both,
                true,
                Some(DatabaseActionToolbarScope::SelectedRow),
            ),
            action_with_scope(
                DatabaseActionId::DeleteTable,
                "Table.delete_table",
                vec![DbNodeType::Table],
                DatabaseActionPlacement::Toolbar,
                true,
                Some(DatabaseActionToolbarScope::CurrentNode),
            ),
            action(
                DatabaseActionId::DumpSqlStructure,
                "ImportExport.export_structure",
                vec![DbNodeType::Database, DbNodeType::Table],
                DatabaseActionPlacement::ContextMenu,
            ),
            action(
                DatabaseActionId::DumpSqlData,
                "ImportExport.export_data",
                vec![DbNodeType::Database, DbNodeType::Table],
                DatabaseActionPlacement::ContextMenu,
            ),
            action(
                DatabaseActionId::DumpSqlStructureAndData,
                "ImportExport.export_structure_and_data",
                vec![DbNodeType::Database, DbNodeType::Table],
                DatabaseActionPlacement::ContextMenu,
            ),
            action(
                DatabaseActionId::ImportData,
                "ImportExport.import_data",
                vec![DbNodeType::Table],
                DatabaseActionPlacement::ContextMenu,
            ),
            action(
                DatabaseActionId::ExportData,
                "ImportExport.export_table",
                vec![DbNodeType::Table],
                DatabaseActionPlacement::ContextMenu,
            ),
            action_with_scope(
                DatabaseActionId::OpenViewData,
                "View.view_data",
                vec![DbNodeType::View],
                DatabaseActionPlacement::Both,
                true,
                Some(DatabaseActionToolbarScope::SelectedRow),
            ),
            action_with_scope(
                DatabaseActionId::OpenViewData,
                "View.view_data",
                vec![DbNodeType::View],
                DatabaseActionPlacement::Toolbar,
                true,
                Some(DatabaseActionToolbarScope::CurrentNode),
            ),
            action_with_scope(
                DatabaseActionId::DeleteView,
                "View.delete_view",
                vec![DbNodeType::View],
                DatabaseActionPlacement::Both,
                true,
                Some(DatabaseActionToolbarScope::SelectedRow),
            ),
            action_with_scope(
                DatabaseActionId::DeleteView,
                "View.delete_view",
                vec![DbNodeType::View],
                DatabaseActionPlacement::Toolbar,
                true,
                Some(DatabaseActionToolbarScope::CurrentNode),
            ),
            action(
                DatabaseActionId::CreateNewQuery,
                "Query.new_query",
                vec![DbNodeType::Database, DbNodeType::QueriesFolder],
                DatabaseActionPlacement::ContextMenu,
            ),
            action_with_scope(
                DatabaseActionId::CreateNewQuery,
                "Query.new_query",
                vec![
                    DbNodeType::QueriesFolder,
                    DbNodeType::NamedQuery,
                    DbNodeType::Schema,
                ],
                DatabaseActionPlacement::Toolbar,
                true,
                Some(DatabaseActionToolbarScope::CurrentNode),
            ),
            action(
                DatabaseActionId::OpenNamedQuery,
                "Query.open_query",
                vec![DbNodeType::NamedQuery],
                DatabaseActionPlacement::Both,
            )
            .with_toolbar_scope(DatabaseActionToolbarScope::SelectedRow),
            action(
                DatabaseActionId::RenameQuery,
                "Query.rename_query",
                vec![DbNodeType::NamedQuery],
                DatabaseActionPlacement::Both,
            )
            .with_toolbar_scope(DatabaseActionToolbarScope::SelectedRow),
            action(
                DatabaseActionId::DeleteQuery,
                "Query.delete_query",
                vec![DbNodeType::NamedQuery],
                DatabaseActionPlacement::Both,
            )
            .with_toolbar_scope(DatabaseActionToolbarScope::SelectedRow),
        ],
    }
}

fn tab(id: &str, label_i18n_key: &str, fields: Vec<ManifestFieldBuilder>) -> DatabaseFormTab {
    DatabaseFormTab {
        id: id.into(),
        label_i18n_key: label_i18n_key.into(),
        fields: fields.into_iter().map(Into::into).collect(),
    }
}

fn field(
    id: &str,
    label_i18n_key: &str,
    field_type: DatabaseFormFieldType,
) -> ManifestFieldBuilder {
    ManifestFieldBuilder::new(id, label_i18n_key, field_type)
}

fn ssh_field(id: &str, label_i18n_key: &str) -> ManifestFieldBuilder {
    field(id, label_i18n_key, DatabaseFormFieldType::Text)
        .optional()
        .with_visibility(ssh_enabled_rules())
}

fn ssh_number_field(id: &str, label_i18n_key: &str) -> ManifestFieldBuilder {
    field(id, label_i18n_key, DatabaseFormFieldType::Number)
        .optional()
        .with_visibility(ssh_enabled_rules())
}

fn ssh_password_field(id: &str, label_i18n_key: &str, placeholder: &str) -> ManifestFieldBuilder {
    field(id, label_i18n_key, DatabaseFormFieldType::Password)
        .optional()
        .with_placeholder(placeholder)
        .with_visibility(ssh_enabled_rules())
}

fn yes_no_options() -> Vec<FormSelectOption> {
    vec![option("false", "Common.no"), option("true", "Common.yes")]
}

fn option(value: &str, label_i18n_key: &str) -> FormSelectOption {
    FormSelectOption {
        value: value.into(),
        label_i18n_key: label_i18n_key.into(),
    }
}

fn action(
    id: DatabaseActionId,
    label_i18n_key: &str,
    targets: Vec<DbNodeType>,
    placement: DatabaseActionPlacement,
) -> DatabaseActionDescriptor {
    action_with_scope(id, label_i18n_key, targets, placement, true, None)
}

fn action_with_scope(
    id: DatabaseActionId,
    label_i18n_key: &str,
    targets: Vec<DbNodeType>,
    placement: DatabaseActionPlacement,
    requires_active_connection: bool,
    toolbar_scope: Option<DatabaseActionToolbarScope>,
) -> DatabaseActionDescriptor {
    DatabaseActionDescriptor {
        id,
        label_i18n_key: label_i18n_key.into(),
        icon: None,
        targets: targets.into_iter().map(target).collect(),
        placement,
        requires_active_connection,
        group: None,
        submenu_of: None,
        toolbar_scope,
    }
}

impl DatabaseActionDescriptor {
    fn always_enabled(mut self) -> Self {
        self.requires_active_connection = false;
        self
    }

    fn with_toolbar_scope(mut self, toolbar_scope: DatabaseActionToolbarScope) -> Self {
        self.toolbar_scope = Some(toolbar_scope);
        self
    }
}

fn target(node_type: DbNodeType) -> DatabaseActionTarget {
    DatabaseActionTarget { node_type }
}

fn equals_rule(field: &str, value: &str) -> FormVisibilityRule {
    FormVisibilityRule {
        when_field: field.into(),
        condition: FormValueCondition::Equals(value.into()),
    }
}

fn ssh_enabled_rules() -> Vec<FormVisibilityRule> {
    vec![equals_rule("ssh_tunnel_enabled", "true")]
}

fn ssh_auth_rules(expected_auth_type: &str) -> Vec<FormVisibilityRule> {
    vec![
        equals_rule("ssh_tunnel_enabled", "true"),
        equals_rule("ssh_auth_type", expected_auth_type),
    ]
}

#[derive(Clone)]
struct ManifestFieldBuilder {
    field: DatabaseFormField,
}

impl ManifestFieldBuilder {
    fn new(id: &str, label_i18n_key: &str, field_type: DatabaseFormFieldType) -> Self {
        Self {
            field: DatabaseFormField {
                id: id.into(),
                label_i18n_key: label_i18n_key.into(),
                field_type,
                required: true,
                default_value: None,
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
            },
        }
    }

    fn optional(mut self) -> Self {
        self.field.required = false;
        self
    }

    fn with_default(mut self, value: &str) -> Self {
        self.field.default_value = Some(value.into());
        self
    }

    fn with_placeholder(mut self, value: &str) -> Self {
        self.field.placeholder_i18n_key = Some(value.into());
        self
    }

    fn with_options(mut self, options: Vec<FormSelectOption>) -> Self {
        self.field.options = options;
        self
    }

    fn with_options_source(mut self, source: ReferenceDataKind) -> Self {
        self.field.options_source = Some(source);
        self
    }

    fn with_visibility(mut self, rules: Vec<FormVisibilityRule>) -> Self {
        self.field.visible_when = rules;
        self
    }

    fn with_default_rules(mut self, rules: Vec<FormDefaultRule>) -> Self {
        self.field.default_when = rules;
        self
    }

    fn disabled_when_editing(mut self, disabled: bool) -> Self {
        self.field.disabled_when_editing = disabled;
        self
    }

    fn with_rows(mut self, rows: u32) -> Self {
        self.field.rows = Some(rows);
        self
    }
}

impl From<ManifestFieldBuilder> for DatabaseFormField {
    fn from(value: ManifestFieldBuilder) -> Self {
        value.field
    }
}

#[async_trait::async_trait]
impl DatabasePlugin for MySqlPlugin {
    fn name(&self) -> DatabaseType {
        DatabaseType::MySQL
    }

    fn quote_identifier(&self, identifier: &str) -> String {
        format!("`{}`", identifier.replace("`", "``"))
    }

    fn get_completion_info(&self) -> SqlCompletionInfo {
        SqlCompletionInfo {
            keywords: vec![
                // MySQL-specific keywords only
                ("AUTO_INCREMENT", "Auto-increment column attribute"),
                ("ENGINE", "Storage engine specification"),
                ("CHARSET", "Character set specification"),
                ("COLLATE", "Collation specification"),
                ("UNSIGNED", "Unsigned integer attribute"),
                ("ZEROFILL", "Zero-fill display attribute"),
                ("BINARY", "Binary string comparison"),
                ("IGNORE", "Ignore errors during operation"),
                ("REPLACE", "Replace existing rows"),
                ("DUPLICATE KEY UPDATE", "On duplicate key update"),
                ("STRAIGHT_JOIN", "Force join order"),
                ("SQL_CALC_FOUND_ROWS", "Calculate total rows"),
                ("HIGH_PRIORITY", "High priority query"),
                ("LOW_PRIORITY", "Low priority query"),
                ("DELAYED", "Delayed insert"),
                ("FORCE INDEX", "Force index usage"),
                ("USE INDEX", "Suggest index usage"),
                ("IGNORE INDEX", "Ignore index"),
            ],
            functions: vec![
                // MySQL-specific functions only (standard SQL functions are added via with_standard_sql())
                ("CONCAT_WS(sep, str1, str2, ...)", "Concatenate with separator"),
                ("CHAR_LENGTH(str)", "String length in characters"),
                ("LPAD(str, len, pad)", "Left pad string"),
                ("RPAD(str, len, pad)", "Right pad string"),
                ("LOCATE(substr, str)", "Find substring position"),
                ("INSTR(str, substr)", "Find substring position"),
                ("REPEAT(str, count)", "Repeat string"),
                ("SPACE(n)", "Generate spaces"),
                ("FORMAT(num, decimals)", "Format number"),
                ("TRUNCATE(x, d)", "Truncate to d decimal places"),
                ("POW(x, y)", "Power function"),
                ("RAND()", "Random number 0-1"),
                ("CURDATE()", "Current date"),
                ("CURTIME()", "Current time"),
                ("DATE(expr)", "Extract date part"),
                ("TIME(expr)", "Extract time part"),
                ("YEAR(date)", "Extract year"),
                ("MONTH(date)", "Extract month"),
                ("DAY(date)", "Extract day"),
                ("HOUR(time)", "Extract hour"),
                ("MINUTE(time)", "Extract minute"),
                ("SECOND(time)", "Extract second"),
                ("DAYOFWEEK(date)", "Day of week (1=Sunday)"),
                ("DAYOFMONTH(date)", "Day of month"),
                ("DAYOFYEAR(date)", "Day of year"),
                ("WEEK(date)", "Week number"),
                ("WEEKDAY(date)", "Weekday (0=Monday)"),
                ("DATE_ADD(date, INTERVAL)", "Add interval to date"),
                ("DATE_SUB(date, INTERVAL)", "Subtract interval from date"),
                ("DATEDIFF(date1, date2)", "Difference in days"),
                ("TIMESTAMPDIFF(unit, dt1, dt2)", "Difference in specified unit"),
                ("DATE_FORMAT(date, format)", "Format date"),
                ("STR_TO_DATE(str, format)", "Parse string to date"),
                ("UNIX_TIMESTAMP()", "Current Unix timestamp"),
                ("FROM_UNIXTIME(ts)", "Convert Unix timestamp"),
                ("GROUP_CONCAT(col)", "Concatenate group values"),
                ("IF(cond, then, else)", "Conditional expression"),
                ("IFNULL(expr, alt)", "Return alt if expr is NULL"),
                ("JSON_EXTRACT(doc, path)", "Extract JSON value"),
                ("JSON_UNQUOTE(json)", "Unquote JSON string"),
                ("JSON_OBJECT(key, val, ...)", "Create JSON object"),
                ("JSON_ARRAY(val, ...)", "Create JSON array"),
                ("JSON_CONTAINS(doc, val)", "Check if JSON contains value"),
                ("JSON_LENGTH(doc)", "JSON document length"),
                ("CONVERT(expr, type)", "Type conversion"),
                ("UUID()", "Generate UUID"),
                ("LAST_INSERT_ID()", "Last auto-increment ID"),
                ("FOUND_ROWS()", "Rows found by previous query"),
                ("ROW_COUNT()", "Affected rows count"),
                ("DATABASE()", "Current database name"),
                ("USER()", "Current user"),
                ("VERSION()", "MySQL version"),
            ],
            operators: vec![
                ("REGEXP", "Regular expression match"),
                ("RLIKE", "Regular expression match (alias)"),
                ("SOUNDS LIKE", "Soundex comparison"),
                ("<=>", "NULL-safe equal"),
                ("DIV", "Integer division"),
                ("XOR", "Logical XOR"),
                (":=", "Assignment operator"),
            ],
            data_types: MYSQL_DATA_TYPES.to_vec(),
            snippets: vec![
                ("crt", "CREATE TABLE $1 (\n  id INT AUTO_INCREMENT PRIMARY KEY,\n  $2\n) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4", "Create table"),
                ("idx", "CREATE INDEX $1 ON $2 ($3)", "Create index"),
                ("alt", "ALTER TABLE $1 ADD COLUMN $2", "Add column"),
                ("jn", "JOIN $1 ON $2.$3 = $4.$5", "Join clause"),
                ("lj", "LEFT JOIN $1 ON $2.$3 = $4.$5", "Left join clause"),
            ],
        }.with_standard_sql()
    }

    async fn create_connection(
        &self,
        config: DbConnectionConfig,
    ) -> Result<Box<dyn DbConnection + Send + Sync>, DbError> {
        let mut conn = MysqlDbConnection::new(config);
        conn.connect().await?;
        Ok(Box::new(conn))
    }

    async fn list_databases(&self, connection: &dyn DbConnection) -> Result<Vec<String>> {
        let result = connection
            .query("SELECT SCHEMA_NAME FROM INFORMATION_SCHEMA.SCHEMATA ORDER BY SCHEMA_NAME")
            .await
            .map_err(|e| anyhow::anyhow!("Failed to list databases: {}", e))?;

        if let SqlResult::Query(query_result) = result {
            Ok(query_result
                .rows
                .iter()
                .filter_map(|row| row.first().and_then(|v| v.clone()))
                .collect())
        } else {
            Err(anyhow::anyhow!("Unexpected result type"))
        }
    }

    async fn list_databases_view(&self, connection: &dyn DbConnection) -> Result<ObjectView> {
        let databases = self.list_databases_detailed(connection).await?;

        let columns = vec![
            Column::localized("name", "ObjectView.columns.name").width(180.0),
            Column::localized("charset", "ObjectView.columns.charset").width(120.0),
            Column::localized("collation", "ObjectView.columns.collation").width(180.0),
            Column::localized("size", "ObjectView.columns.size")
                .width(100.0)
                .text_right(),
            Column::localized("tables", "ObjectView.columns.tables")
                .width(80.0)
                .text_right(),
            Column::localized("comment", "ObjectView.columns.comment").width(250.0),
        ];

        let rows: Vec<Vec<String>> = databases
            .iter()
            .map(|db| {
                vec![
                    db.name.clone(),
                    db.charset.as_deref().unwrap_or("-").to_string(),
                    db.collation.as_deref().unwrap_or("-").to_string(),
                    db.size.as_deref().unwrap_or("-").to_string(),
                    db.table_count
                        .map(|n| n.to_string())
                        .unwrap_or_else(|| "-".to_string()),
                    db.comment.as_deref().unwrap_or("").to_string(),
                ]
            })
            .collect();

        Ok(ObjectView {
            db_node_type: DbNodeType::Database,
            title: t!("ObjectView.counts.databases", count = databases.len()).to_string(),
            columns,
            rows,
        })
    }

    async fn list_databases_detailed(
        &self,
        connection: &dyn DbConnection,
    ) -> Result<Vec<DatabaseInfo>> {
        let result = connection
            .query(
                "SELECT
                s.SCHEMA_NAME as name,
                s.DEFAULT_CHARACTER_SET_NAME as charset,
                s.DEFAULT_COLLATION_NAME as collation,
                COUNT(t.TABLE_NAME) as table_count
            FROM INFORMATION_SCHEMA.SCHEMATA s
            LEFT JOIN INFORMATION_SCHEMA.TABLES t
                ON s.SCHEMA_NAME = t.TABLE_SCHEMA AND t.TABLE_TYPE = 'BASE TABLE'
            GROUP BY s.SCHEMA_NAME, s.DEFAULT_CHARACTER_SET_NAME, s.DEFAULT_COLLATION_NAME
            ORDER BY s.SCHEMA_NAME",
            )
            .await
            .map_err(|e| anyhow::anyhow!("Failed to list databases: {}", e))?;

        if let SqlResult::Query(query_result) = result {
            let databases: Vec<DatabaseInfo> = query_result
                .rows
                .iter()
                .filter_map(|row| {
                    let name = row.first().and_then(|v| v.clone())?;
                    let charset = row.get(1).and_then(|v| v.clone());
                    let collation = row.get(2).and_then(|v| v.clone());
                    let table_count = row
                        .get(3)
                        .and_then(|v| v.clone())
                        .and_then(|s| s.parse::<i64>().ok());

                    Some(DatabaseInfo {
                        name,
                        charset,
                        collation,
                        size: None,
                        table_count,
                        comment: None,
                    })
                })
                .collect();
            Ok(databases)
        } else {
            Err(anyhow::anyhow!("Unexpected result type"))
        }
    }

    // === Database/Schema Level Operations ===

    fn sql_dialect(&self) -> Box<dyn sqlparser::dialect::Dialect> {
        Box::new(sqlparser::dialect::MySqlDialect {})
    }

    async fn list_tables(
        &self,
        connection: &dyn DbConnection,
        database: &str,
        _schema: Option<String>,
    ) -> Result<Vec<TableInfo>> {
        // Query to get all tables with their description/metadata
        let sql = format!(
            "SELECT \
                TABLE_NAME, \
                TABLE_COMMENT, \
                ENGINE, \
                TABLE_ROWS, \
                CREATE_TIME, \
                TABLE_COLLATION \
             FROM INFORMATION_SCHEMA.TABLES \
             WHERE TABLE_SCHEMA = '{}' AND TABLE_TYPE IN ('BASE TABLE','SYSTEM VIEW') \
             ORDER BY TABLE_NAME",
            database
        );

        let result = connection
            .query(&sql)
            .await
            .map_err(|e| anyhow::anyhow!("Failed to list tables: {}", e))?;

        if let SqlResult::Query(query_result) = result {
            let tables: Vec<TableInfo> = query_result
                .rows
                .iter()
                .map(|row| {
                    let collation = row.get(5).and_then(|v| v.clone());
                    // Extract charset from collation (e.g., "utf8mb4_general_ci" -> "utf8mb4")
                    let charset = collation
                        .as_ref()
                        .and_then(|c| c.split('_').next().map(|s| s.to_string()));

                    // Parse row count
                    let row_count = row
                        .get(3)
                        .and_then(|v| v.clone())
                        .and_then(|s| s.parse::<i64>().ok());

                    TableInfo {
                        name: row.first().and_then(|v| v.clone()).unwrap_or_default(),
                        schema: None,
                        comment: row.get(1).and_then(|v| v.clone()).filter(|s| !s.is_empty()),
                        engine: row.get(2).and_then(|v| v.clone()),
                        row_count,
                        create_time: row.get(4).and_then(|v| v.clone()),
                        charset,
                        collation,
                    }
                })
                .collect();

            Ok(tables)
        } else {
            Err(anyhow::anyhow!("Unexpected result type"))
        }
    }

    async fn list_tables_view(
        &self,
        connection: &dyn DbConnection,
        database: &str,
        _schema: Option<String>,
    ) -> Result<ObjectView> {
        let tables = self.list_tables(connection, database, None).await?;

        let columns = vec![
            Column::localized("name", "ObjectView.columns.name").width(200.0),
            Column::localized("engine", "ObjectView.columns.engine").width(150.0),
            Column::localized("rows", "ObjectView.columns.rows")
                .width(100.0)
                .text_right(),
            Column::localized("created", "ObjectView.columns.created").width(180.0),
            Column::localized("comment", "ObjectView.columns.comment").width(300.0),
        ];

        let rows: Vec<Vec<String>> = tables
            .iter()
            .map(|table| {
                vec![
                    table.name.clone(),
                    table.engine.as_deref().unwrap_or("-").to_string(),
                    table
                        .row_count
                        .map(|n| n.to_string())
                        .unwrap_or_else(|| "-".to_string()),
                    table.create_time.as_deref().unwrap_or("-").to_string(),
                    table.comment.as_deref().unwrap_or("").to_string(),
                ]
            })
            .collect();

        Ok(ObjectView {
            db_node_type: DbNodeType::Table,
            title: t!("ObjectView.counts.tables", count = tables.len()).to_string(),
            columns,
            rows,
        })
    }

    // === Table Operations ===

    async fn list_columns(
        &self,
        connection: &dyn DbConnection,
        database: &str,
        _schema: Option<String>,
        table: &str,
    ) -> Result<Vec<ColumnInfo>> {
        let sql = format!(
            "SELECT COLUMN_NAME, COLUMN_TYPE, IS_NULLABLE, COLUMN_KEY, COLUMN_DEFAULT, COLUMN_COMMENT, \
             CHARACTER_SET_NAME, COLLATION_NAME \
             FROM INFORMATION_SCHEMA.COLUMNS \
             WHERE TABLE_SCHEMA = '{}' AND TABLE_NAME = '{}' \
             ORDER BY ORDINAL_POSITION",
            database, table
        );

        let result = connection
            .query(&sql)
            .await
            .map_err(|e| anyhow::anyhow!("Failed to list columns: {}", e))?;

        if let SqlResult::Query(query_result) = result {
            Ok(query_result
                .rows
                .iter()
                .map(|row| ColumnInfo {
                    name: row.first().and_then(|v| v.clone()).unwrap_or_default(),
                    data_type: row.get(1).and_then(|v| v.clone()).unwrap_or_default(),
                    is_nullable: row
                        .get(2)
                        .and_then(|v| v.clone())
                        .map(|v| v == "YES")
                        .unwrap_or(true),
                    is_primary_key: row
                        .get(3)
                        .and_then(|v| v.clone())
                        .map(|v| v == "PRI")
                        .unwrap_or(false),
                    default_value: row.get(4).and_then(|v| v.clone()),
                    comment: row.get(5).and_then(|v| v.clone()),
                    charset: row.get(6).and_then(|v| v.clone()),
                    collation: row.get(7).and_then(|v| v.clone()),
                })
                .collect())
        } else {
            Err(anyhow::anyhow!("Unexpected result type"))
        }
    }

    async fn list_columns_view(
        &self,
        connection: &dyn DbConnection,
        database: &str,
        schema: Option<String>,
        table: &str,
    ) -> Result<ObjectView> {
        let columns_data = self
            .list_columns(connection, database, schema, table)
            .await?;

        let columns = vec![
            Column::localized("name", "ObjectView.columns.name").width(180.0),
            Column::localized("type", "ObjectView.columns.type").width(150.0),
            Column::localized("nullable", "ObjectView.columns.nullable").width(80.0),
            Column::localized("key", "ObjectView.columns.key").width(80.0),
            Column::localized("default", "ObjectView.columns.default").width(120.0),
            Column::localized("comment", "ObjectView.columns.comment").width(250.0),
        ];

        let rows: Vec<Vec<String>> = columns_data
            .iter()
            .map(|col| {
                vec![
                    col.name.clone(),
                    col.data_type.clone(),
                    if col.is_nullable { "YES" } else { "NO" }.to_string(),
                    if col.is_primary_key { "PRI" } else { "" }.to_string(),
                    col.default_value.as_deref().unwrap_or("").to_string(),
                    col.comment.as_deref().unwrap_or("").to_string(),
                ]
            })
            .collect();

        Ok(ObjectView {
            db_node_type: DbNodeType::Column,
            title: t!("ObjectView.counts.columns", count = columns_data.len()).to_string(),
            columns,
            rows,
        })
    }

    async fn list_indexes(
        &self,
        connection: &dyn DbConnection,
        database: &str,
        _schema: Option<String>,
        table: &str,
    ) -> Result<Vec<IndexInfo>> {
        let sql = format!(
            "SELECT INDEX_NAME, COLUMN_NAME, NON_UNIQUE, INDEX_TYPE \
             FROM INFORMATION_SCHEMA.STATISTICS \
             WHERE TABLE_SCHEMA = '{}' AND TABLE_NAME = '{}' AND INDEX_NAME != 'PRIMARY' \
             ORDER BY INDEX_NAME, SEQ_IN_INDEX",
            database, table
        );

        let result = connection
            .query(&sql)
            .await
            .map_err(|e| anyhow::anyhow!("Failed to list indexes: {}", e))?;

        if let SqlResult::Query(query_result) = result {
            let mut indexes: HashMap<String, IndexInfo> = HashMap::new();

            for row in query_result.rows {
                let index_name = row.first().and_then(|v| v.clone()).unwrap_or_default();
                let column_name = row.get(1).and_then(|v| v.clone()).unwrap_or_default();
                let is_unique = row
                    .get(2)
                    .and_then(|v| v.clone())
                    .map(|v| v == "0")
                    .unwrap_or(false);
                let index_type = row.get(3).and_then(|v| v.clone());

                indexes
                    .entry(index_name.clone())
                    .or_insert_with(|| IndexInfo {
                        name: index_name,
                        columns: Vec::new(),
                        is_unique,
                        is_primary: false,
                        index_type: index_type.clone(),
                    })
                    .columns
                    .push(column_name);
            }

            Ok(indexes.into_values().collect())
        } else {
            Err(anyhow::anyhow!("Unexpected result type"))
        }
    }

    async fn list_indexes_view(
        &self,
        connection: &dyn DbConnection,
        database: &str,
        schema: Option<&str>,
        table: &str,
    ) -> Result<ObjectView> {
        let indexes = self
            .list_indexes(connection, database, schema.map(|s| s.to_string()), table)
            .await?;

        let columns = vec![
            Column::localized("name", "ObjectView.columns.name").width(180.0),
            Column::localized("columns", "ObjectView.columns.columns").width(250.0),
            Column::localized("unique", "ObjectView.columns.unique").width(80.0),
            Column::localized("type", "ObjectView.columns.type").width(120.0),
        ];

        let rows: Vec<Vec<String>> = indexes
            .iter()
            .map(|idx| {
                vec![
                    idx.name.clone(),
                    idx.columns.join(", "),
                    if idx.is_unique { "YES" } else { "NO" }.to_string(),
                    idx.index_type.as_deref().unwrap_or("-").to_string(),
                ]
            })
            .collect();

        Ok(ObjectView {
            db_node_type: DbNodeType::Index,
            title: t!("ObjectView.counts.indexes", count = indexes.len()).to_string(),
            columns,
            rows,
        })
    }

    async fn list_foreign_keys(
        &self,
        connection: &dyn DbConnection,
        database: &str,
        _schema: Option<String>,
        table: &str,
    ) -> Result<Vec<ForeignKeyDefinition>> {
        let sql = mysql_foreign_keys_sql(database, table);
        let result = connection
            .query(&sql)
            .await
            .map_err(|e| anyhow::anyhow!("Failed to list foreign keys: {}", e))?;

        if let SqlResult::Query(query_result) = result {
            Ok(parse_mysql_foreign_keys(query_result.rows))
        } else {
            Err(anyhow::anyhow!("Unexpected result type"))
        }
    }

    async fn list_table_triggers(
        &self,
        connection: &dyn DbConnection,
        database: &str,
        _schema: Option<String>,
        table: &str,
    ) -> Result<Vec<TriggerInfo>> {
        let sql = mysql_table_triggers_sql(database, table);
        let result = connection
            .query(&sql)
            .await
            .map_err(|e| anyhow::anyhow!("Failed to list table triggers: {}", e))?;

        if let SqlResult::Query(query_result) = result {
            Ok(parse_mysql_triggers(query_result.rows))
        } else {
            Err(anyhow::anyhow!("Unexpected result type"))
        }
    }

    async fn list_table_checks(
        &self,
        _connection: &dyn DbConnection,
        _database: &str,
        _schema: Option<String>,
        _table: &str,
    ) -> Result<Vec<CheckInfo>> {
        let sql = format!(
            "SELECT cc.CONSTRAINT_NAME, tc.TABLE_NAME, cc.CHECK_CLAUSE \
             FROM INFORMATION_SCHEMA.CHECK_CONSTRAINTS cc \
             JOIN INFORMATION_SCHEMA.TABLE_CONSTRAINTS tc \
                ON cc.CONSTRAINT_SCHEMA = tc.CONSTRAINT_SCHEMA \
                AND cc.CONSTRAINT_NAME = tc.CONSTRAINT_NAME \
             WHERE tc.CONSTRAINT_SCHEMA = '{}' AND tc.TABLE_NAME = '{}' \
               AND tc.CONSTRAINT_TYPE = 'CHECK' \
             ORDER BY cc.CONSTRAINT_NAME",
            _database, _table
        );

        let result = _connection
            .query(&sql)
            .await
            .map_err(|e| anyhow::anyhow!("Failed to list check constraints: {}", e))?;

        if let SqlResult::Query(query_result) = result {
            Ok(query_result
                .rows
                .iter()
                .map(|row| CheckInfo {
                    name: row.first().and_then(|v| v.clone()).unwrap_or_default(),
                    table_name: row.get(1).and_then(|v| v.clone()).unwrap_or_default(),
                    definition: row.get(2).and_then(|v| v.clone()),
                })
                .collect())
        } else {
            Err(anyhow::anyhow!("Unexpected result type"))
        }
    }

    // === View Operations ===

    async fn list_views(
        &self,
        connection: &dyn DbConnection,
        database: &str,
        _schema: Option<String>,
    ) -> Result<Vec<ViewInfo>> {
        let sql = format!(
            "SELECT TABLE_NAME, VIEW_DEFINITION \
             FROM INFORMATION_SCHEMA.VIEWS \
             WHERE TABLE_SCHEMA = '{}' \
             ORDER BY TABLE_NAME",
            database
        );

        let result = connection
            .query(&sql)
            .await
            .map_err(|e| anyhow::anyhow!("Failed to list views: {}", e))?;

        if let SqlResult::Query(query_result) = result {
            Ok(query_result
                .rows
                .iter()
                .map(|row| ViewInfo {
                    name: row.first().and_then(|v| v.clone()).unwrap_or_default(),
                    schema: None,
                    definition: row.get(1).and_then(|v| v.clone()),
                    comment: None,
                })
                .collect())
        } else {
            Err(anyhow::anyhow!("Unexpected result type"))
        }
    }

    async fn list_views_view(
        &self,
        connection: &dyn DbConnection,
        database: &str,
    ) -> Result<ObjectView> {
        let views = self.list_views(connection, database, None).await?;

        let columns = vec![
            Column::localized("name", "ObjectView.columns.name").width(200.0),
            Column::localized("definition", "ObjectView.columns.definition").width(400.0),
        ];

        let rows: Vec<Vec<String>> = views
            .iter()
            .map(|view| {
                vec![
                    view.name.clone(),
                    view.definition.as_deref().unwrap_or("").to_string(),
                ]
            })
            .collect();

        Ok(ObjectView {
            db_node_type: DbNodeType::View,
            title: t!("ObjectView.counts.views", count = views.len()).to_string(),
            columns,
            rows,
        })
    }

    // === Function Operations ===

    async fn list_functions(
        &self,
        connection: &dyn DbConnection,
        database: &str,
    ) -> Result<Vec<FunctionInfo>> {
        let sql = format!(
            "SELECT ROUTINE_NAME, DTD_IDENTIFIER \
             FROM INFORMATION_SCHEMA.ROUTINES \
             WHERE ROUTINE_SCHEMA = '{}' AND ROUTINE_TYPE = 'FUNCTION' \
             ORDER BY ROUTINE_NAME",
            database
        );

        let result = connection
            .query(&sql)
            .await
            .map_err(|e| anyhow::anyhow!("Failed to list functions: {}", e))?;

        if let SqlResult::Query(query_result) = result {
            Ok(query_result
                .rows
                .iter()
                .map(|row| FunctionInfo {
                    name: row.first().and_then(|v| v.clone()).unwrap_or_default(),
                    return_type: row.get(1).and_then(|v| v.clone()),
                    parameters: Vec::new(),
                    definition: None,
                    comment: None,
                })
                .collect())
        } else {
            Err(anyhow::anyhow!("Unexpected result type"))
        }
    }

    async fn list_functions_view(
        &self,
        connection: &dyn DbConnection,
        database: &str,
    ) -> Result<ObjectView> {
        let functions = self.list_functions(connection, database).await?;

        let columns = vec![
            Column::localized("name", "ObjectView.columns.name").width(200.0),
            Column::localized("return_type", "ObjectView.columns.return_type").width(150.0),
        ];

        let rows: Vec<Vec<String>> = functions
            .iter()
            .map(|func| {
                vec![
                    func.name.clone(),
                    func.return_type.as_deref().unwrap_or("-").to_string(),
                ]
            })
            .collect();

        Ok(ObjectView {
            db_node_type: DbNodeType::Function,
            title: t!("ObjectView.counts.functions", count = functions.len()).to_string(),
            columns,
            rows,
        })
    }

    // === Procedure Operations ===

    fn capabilities(&self) -> DatabaseCapabilities {
        DatabaseUiCapabilities {
            supports_functions: true,
            supports_procedures: true,
            supports_triggers: true,
            supports_users: true,
            supports_user_create: true,
            supports_user_edit: true,
            supports_user_delete: true,
            supports_user_privileges: true,
            supports_table_engine: true,
            supports_table_charset: true,
            supports_table_collation: true,
            supports_auto_increment: true,
            supports_unsigned: true,
            supports_enum_values: true,
            show_charset_in_column_detail: true,
            show_collation_in_column_detail: true,
            table_engines: self.engines(),
            ..DatabaseUiCapabilities::default()
        }
    }

    fn ui_manifest(&self) -> DatabaseUiManifest {
        MYSQL_UI_MANIFEST.clone()
    }

    // === Trigger Operations ===

    fn resolve_reference_data(
        &self,
        kind: ReferenceDataKind,
        context: &HashMap<String, String>,
    ) -> Vec<FormSelectOption> {
        match kind {
            ReferenceDataKind::MySqlCharsets => self
                .get_charsets()
                .into_iter()
                .map(|charset| FormSelectOption {
                    value: charset.name.clone(),
                    label_i18n_key: format!("{} - {}", charset.name, charset.description),
                })
                .collect(),
            ReferenceDataKind::MySqlCollations => {
                let charset = context
                    .get("charset")
                    .map(String::as_str)
                    .unwrap_or("utf8mb4");
                self.get_collations(charset)
                    .into_iter()
                    .map(|collation| FormSelectOption {
                        value: collation.name.clone(),
                        label_i18n_key: if collation.is_default {
                            format!("{} (default)", collation.name)
                        } else {
                            collation.name
                        },
                    })
                    .collect()
            }
            ReferenceDataKind::TableEngines => self
                .engines()
                .into_iter()
                .map(|engine| FormSelectOption {
                    value: engine.clone(),
                    label_i18n_key: engine,
                })
                .collect(),
            _ => Vec::new(),
        }
    }

    async fn list_procedures(
        &self,
        connection: &dyn DbConnection,
        database: &str,
    ) -> Result<Vec<FunctionInfo>> {
        let sql = format!(
            "SELECT ROUTINE_NAME \
             FROM INFORMATION_SCHEMA.ROUTINES \
             WHERE ROUTINE_SCHEMA = '{}' AND ROUTINE_TYPE = 'PROCEDURE' \
             ORDER BY ROUTINE_NAME",
            database
        );

        let result = connection
            .query(&sql)
            .await
            .map_err(|e| anyhow::anyhow!("Failed to list procedures: {}", e))?;

        if let SqlResult::Query(query_result) = result {
            Ok(query_result
                .rows
                .iter()
                .map(|row| FunctionInfo {
                    name: row.first().and_then(|v| v.clone()).unwrap_or_default(),
                    return_type: None,
                    parameters: Vec::new(),
                    definition: None,
                    comment: None,
                })
                .collect())
        } else {
            Err(anyhow::anyhow!("Unexpected result type"))
        }
    }

    async fn list_procedures_view(
        &self,
        connection: &dyn DbConnection,
        database: &str,
    ) -> Result<ObjectView> {
        let procedures = self.list_procedures(connection, database).await?;

        let columns = vec![Column::localized("name", "ObjectView.columns.name").width(200.0)];

        let rows: Vec<Vec<String>> = procedures
            .iter()
            .map(|proc| vec![proc.name.clone()])
            .collect();

        Ok(ObjectView {
            db_node_type: DbNodeType::Procedure,
            title: t!("ObjectView.counts.procedures", count = procedures.len()).to_string(),
            columns,
            rows,
        })
    }

    async fn list_triggers(
        &self,
        connection: &dyn DbConnection,
        database: &str,
    ) -> Result<Vec<TriggerInfo>> {
        let sql = format!(
            "SELECT TRIGGER_NAME, EVENT_OBJECT_TABLE, EVENT_MANIPULATION, ACTION_TIMING \
             FROM INFORMATION_SCHEMA.TRIGGERS \
             WHERE TRIGGER_SCHEMA = '{}' \
             ORDER BY TRIGGER_NAME",
            database
        );

        let result = connection
            .query(&sql)
            .await
            .map_err(|e| anyhow::anyhow!("Failed to list triggers: {}", e))?;

        if let SqlResult::Query(query_result) = result {
            Ok(query_result
                .rows
                .iter()
                .map(|row| TriggerInfo {
                    name: row.first().and_then(|v| v.clone()).unwrap_or_default(),
                    table_name: row.get(1).and_then(|v| v.clone()).unwrap_or_default(),
                    event: row.get(2).and_then(|v| v.clone()).unwrap_or_default(),
                    timing: row.get(3).and_then(|v| v.clone()).unwrap_or_default(),
                    definition: None,
                })
                .collect())
        } else {
            Err(anyhow::anyhow!("Unexpected result type"))
        }
    }

    // === Sequence Operations ===
    // MySQL doesn't support sequences natively (until MySQL 8.0 which has AUTO_INCREMENT only)
    // Return empty results

    async fn list_triggers_view(
        &self,
        connection: &dyn DbConnection,
        database: &str,
    ) -> Result<ObjectView> {
        let triggers = self.list_triggers(connection, database).await?;

        let columns = vec![
            Column::localized("name", "ObjectView.columns.name").width(180.0),
            Column::localized("table", "ObjectView.columns.table").width(150.0),
            Column::localized("event", "ObjectView.columns.event").width(100.0),
            Column::localized("timing", "ObjectView.columns.timing").width(100.0),
        ];

        let rows: Vec<Vec<String>> = triggers
            .iter()
            .map(|trigger| {
                vec![
                    trigger.name.clone(),
                    trigger.table_name.clone(),
                    trigger.event.clone(),
                    trigger.timing.clone(),
                ]
            })
            .collect();

        Ok(ObjectView {
            db_node_type: DbNodeType::Trigger,
            title: t!("ObjectView.counts.triggers", count = triggers.len()).to_string(),
            columns,
            rows,
        })
    }

    async fn list_sequences(
        &self,
        _connection: &dyn DbConnection,
        _database: &str,
        _schema: Option<String>,
    ) -> Result<Vec<SequenceInfo>> {
        Ok(Vec::new())
    }

    async fn list_sequences_view(
        &self,
        _connection: &dyn DbConnection,
        _database: &str,
    ) -> Result<ObjectView> {
        let columns = vec![Column::localized("name", "ObjectView.columns.name").width(200.0)];

        Ok(ObjectView {
            db_node_type: DbNodeType::Sequence,
            title: t!("ObjectView.counts.sequences", count = 0).to_string(),
            columns,
            rows: vec![],
        })
    }

    fn build_column_definition(&self, column: &ColumnInfo, include_name: bool) -> String {
        let mut def = String::new();

        if include_name {
            def.push_str(&self.quote_identifier(&column.name));
            def.push(' ');
        }

        def.push_str(&column.data_type);

        if !column.is_nullable {
            def.push_str(" NOT NULL");
        }

        if let Some(default) = &column.default_value {
            def.push_str(&format!(" DEFAULT {}", default));
        }

        if column.is_primary_key {
            def.push_str(" PRIMARY KEY");
        }

        if let Some(comment) = &column.comment {
            def.push_str(&format!(" COMMENT '{}'", comment.replace("'", "''")));
        }

        def
    }

    // === Database Management Operations ===
    fn build_list_users_sql(&self, _database: Option<&str>) -> Option<String> {
        Some(
            r#"SELECT
  User,
  Host,
  plugin AS authentication_plugin,
  account_locked,
  password_expired,
  password_last_changed,
  password_lifetime,
  max_questions,
  max_updates,
  max_connections,
  max_user_connections,
  ssl_type,
  ssl_cipher,
  x509_issuer,
  x509_subject,
  Select_priv,
  Insert_priv,
  Update_priv,
  Delete_priv,
  Create_priv,
  Drop_priv,
  Grant_priv
FROM mysql.user
ORDER BY User, Host;"#
                .to_string(),
        )
    }

    fn user_list_columns(&self) -> Vec<Column> {
        vec![
            Column::localized("user", "DatabaseUser.columns.user").width(180.0),
            Column::localized("host", "DatabaseUser.columns.host").width(160.0),
            Column::localized(
                "authentication_plugin",
                "DatabaseUser.columns.authentication_plugin",
            )
            .width(220.0),
            Column::localized("account_locked", "DatabaseUser.columns.account_locked").width(120.0),
            Column::localized("password_expired", "DatabaseUser.columns.password_expired")
                .width(130.0),
            Column::localized(
                "password_last_changed",
                "DatabaseUser.columns.password_last_changed",
            )
            .width(180.0),
            Column::localized(
                "password_lifetime",
                "DatabaseUser.columns.password_lifetime",
            )
            .width(150.0),
            Column::localized("max_questions", "DatabaseUser.columns.max_questions")
                .width(120.0)
                .text_right(),
            Column::localized("max_updates", "DatabaseUser.columns.max_updates")
                .width(120.0)
                .text_right(),
            Column::localized("max_connections", "DatabaseUser.columns.max_connections")
                .width(140.0)
                .text_right(),
            Column::localized(
                "max_user_connections",
                "DatabaseUser.columns.max_user_connections",
            )
            .width(160.0)
            .text_right(),
            Column::localized("ssl_type", "DatabaseUser.columns.ssl_type").width(120.0),
            Column::localized("ssl_cipher", "DatabaseUser.columns.ssl_cipher").width(160.0),
            Column::localized("x509_issuer", "DatabaseUser.columns.x509_issuer").width(240.0),
            Column::localized("x509_subject", "DatabaseUser.columns.x509_subject").width(240.0),
            Column::localized("select_priv", "DatabaseUser.columns.select_priv").width(110.0),
            Column::localized("insert_priv", "DatabaseUser.columns.insert_priv").width(110.0),
            Column::localized("update_priv", "DatabaseUser.columns.update_priv").width(110.0),
            Column::localized("delete_priv", "DatabaseUser.columns.delete_priv").width(110.0),
            Column::localized("create_priv", "DatabaseUser.columns.create_priv").width(110.0),
            Column::localized("drop_priv", "DatabaseUser.columns.drop_priv").width(110.0),
            Column::localized("grant_priv", "DatabaseUser.columns.grant_priv").width(110.0),
        ]
    }

    fn build_create_user_sql(&self, request: &DatabaseUserOperationRequest) -> Option<String> {
        Some(format!(
            "CREATE USER {} IDENTIFIED BY {};",
            mysql_user_account(request),
            mysql_string_literal(mysql_user_password(request))
        ))
    }

    fn build_modify_user_sql(&self, request: &DatabaseUserOperationRequest) -> Option<String> {
        Some(format!(
            "ALTER USER {} IDENTIFIED BY {};",
            mysql_user_account(request),
            mysql_string_literal(mysql_user_password(request))
        ))
    }

    fn build_drop_user_sql(&self, request: &DatabaseUserOperationRequest) -> Option<String> {
        Some(format!("DROP USER {};", mysql_user_account(request)))
    }

    fn build_user_privileges_sql(&self, request: &DatabaseUserOperationRequest) -> Option<String> {
        let database = request
            .field_values
            .get("database")
            .map(String::as_str)
            .or(request.database.as_deref())
            .filter(|database| !database.trim().is_empty());
        let scope = database
            .map(|database| format!("{}.*", self.quote_identifier(database)))
            .unwrap_or_else(|| "*.*".to_string());
        Some(format!(
            "GRANT {} ON {} TO {};",
            mysql_user_privileges(request),
            scope,
            mysql_user_account(request)
        ))
    }

    fn build_create_database_sql(
        &self,
        request: &crate::plugin::DatabaseOperationRequest,
    ) -> String {
        let db_name = self.quote_identifier(&request.database_name);
        let charset = request
            .field_values
            .get("charset")
            .map(|s| s.as_str())
            .unwrap_or("utf8mb4");
        let collation = request
            .field_values
            .get("collation")
            .map(|s| s.as_str())
            .unwrap_or("utf8mb4_general_ci");

        format!(
            "CREATE DATABASE {} CHARACTER SET {} COLLATE {};",
            db_name, charset, collation
        )
    }

    fn build_modify_database_sql(
        &self,
        request: &crate::plugin::DatabaseOperationRequest,
    ) -> String {
        let db_name = self.quote_identifier(&request.database_name);
        let charset = request
            .field_values
            .get("charset")
            .map(|s| s.as_str())
            .unwrap_or("utf8mb4");
        let collation = request
            .field_values
            .get("collation")
            .map(|s| s.as_str())
            .unwrap_or("utf8mb4_general_ci");

        format!(
            "ALTER DATABASE {} CHARACTER SET {} COLLATE {};",
            db_name, charset, collation
        )
    }

    fn build_drop_database_sql(&self, database_name: &str) -> String {
        format!("DROP DATABASE {};", self.quote_identifier(database_name))
    }

    fn build_limit_clause(&self) -> String {
        " LIMIT 1".to_string()
    }

    fn build_where_and_limit_clause(
        &self,
        request: &TableSaveRequest,
        original_data: &[String],
    ) -> (String, String) {
        let where_clause = self.build_table_change_where_clause(request, original_data);
        (where_clause, self.build_limit_clause())
    }

    async fn export_table_create_sql(
        &self,
        connection: &dyn DbConnection,
        database: &str,
        _schema: Option<&str>,
        table: &str,
    ) -> Result<String> {
        let table_ref = self.format_table_reference(database, None, table);
        let show_create = format!("SHOW CREATE TABLE {}", table_ref);
        let result = connection
            .query(&show_create)
            .await
            .map_err(|e| anyhow::anyhow!("Query failed: {}", e))?;

        if let SqlResult::Query(query_result) = result {
            if let Some(row) = query_result.rows.first() {
                if let Some(Some(create_sql)) = row.get(1) {
                    return Ok(create_sql.clone());
                }
            }
        }
        Ok(String::new())
    }

    fn get_charsets(&self) -> Vec<CharsetInfo> {
        vec![
            CharsetInfo {
                name: "utf8mb4".into(),
                description: "UTF-8 Unicode (4 bytes)".into(),
                default_collation: "utf8mb4_general_ci".into(),
            },
            CharsetInfo {
                name: "utf8mb3".into(),
                description: "UTF-8 Unicode (3 bytes)".into(),
                default_collation: "utf8mb3_general_ci".into(),
            },
            CharsetInfo {
                name: "utf8".into(),
                description: "UTF-8 Unicode (alias for utf8mb3)".into(),
                default_collation: "utf8_general_ci".into(),
            },
            CharsetInfo {
                name: "latin1".into(),
                description: "West European (ISO 8859-1)".into(),
                default_collation: "latin1_swedish_ci".into(),
            },
            CharsetInfo {
                name: "latin2".into(),
                description: "Central European (ISO 8859-2)".into(),
                default_collation: "latin2_general_ci".into(),
            },
            CharsetInfo {
                name: "ascii".into(),
                description: "US ASCII".into(),
                default_collation: "ascii_general_ci".into(),
            },
            CharsetInfo {
                name: "gbk".into(),
                description: "GBK Simplified Chinese".into(),
                default_collation: "gbk_chinese_ci".into(),
            },
            CharsetInfo {
                name: "gb2312".into(),
                description: "GB2312 Simplified Chinese".into(),
                default_collation: "gb2312_chinese_ci".into(),
            },
            CharsetInfo {
                name: "gb18030".into(),
                description: "GB18030 Chinese".into(),
                default_collation: "gb18030_chinese_ci".into(),
            },
            CharsetInfo {
                name: "big5".into(),
                description: "Big5 Traditional Chinese".into(),
                default_collation: "big5_chinese_ci".into(),
            },
            CharsetInfo {
                name: "sjis".into(),
                description: "Shift-JIS Japanese".into(),
                default_collation: "sjis_japanese_ci".into(),
            },
            CharsetInfo {
                name: "euckr".into(),
                description: "EUC-KR Korean".into(),
                default_collation: "euckr_korean_ci".into(),
            },
            CharsetInfo {
                name: "greek".into(),
                description: "ISO 8859-7 Greek".into(),
                default_collation: "greek_general_ci".into(),
            },
            CharsetInfo {
                name: "hebrew".into(),
                description: "ISO 8859-8 Hebrew".into(),
                default_collation: "hebrew_general_ci".into(),
            },
            CharsetInfo {
                name: "cp1251".into(),
                description: "Windows Cyrillic".into(),
                default_collation: "cp1251_general_ci".into(),
            },
            CharsetInfo {
                name: "cp1256".into(),
                description: "Windows Arabic".into(),
                default_collation: "cp1256_general_ci".into(),
            },
            CharsetInfo {
                name: "binary".into(),
                description: "Binary pseudo charset".into(),
                default_collation: "binary".into(),
            },
        ]
    }

    fn get_collations(&self, charset: &str) -> Vec<CollationInfo> {
        match charset {
            "utf8mb4" => vec![
                CollationInfo {
                    name: "utf8mb4_general_ci".into(),
                    charset: "utf8mb4".into(),
                    is_default: true,
                },
                CollationInfo {
                    name: "utf8mb4_unicode_ci".into(),
                    charset: "utf8mb4".into(),
                    is_default: false,
                },
                CollationInfo {
                    name: "utf8mb4_unicode_520_ci".into(),
                    charset: "utf8mb4".into(),
                    is_default: false,
                },
                CollationInfo {
                    name: "utf8mb4_bin".into(),
                    charset: "utf8mb4".into(),
                    is_default: false,
                },
                CollationInfo {
                    name: "utf8mb4_0900_ai_ci".into(),
                    charset: "utf8mb4".into(),
                    is_default: false,
                },
                CollationInfo {
                    name: "utf8mb4_0900_as_ci".into(),
                    charset: "utf8mb4".into(),
                    is_default: false,
                },
                CollationInfo {
                    name: "utf8mb4_0900_as_cs".into(),
                    charset: "utf8mb4".into(),
                    is_default: false,
                },
                CollationInfo {
                    name: "utf8mb4_zh_0900_as_cs".into(),
                    charset: "utf8mb4".into(),
                    is_default: false,
                },
                CollationInfo {
                    name: "utf8mb4_ja_0900_as_cs".into(),
                    charset: "utf8mb4".into(),
                    is_default: false,
                },
            ],
            "utf8mb3" => vec![
                CollationInfo {
                    name: "utf8mb3_general_ci".into(),
                    charset: "utf8mb3".into(),
                    is_default: true,
                },
                CollationInfo {
                    name: "utf8mb3_unicode_ci".into(),
                    charset: "utf8mb3".into(),
                    is_default: false,
                },
                CollationInfo {
                    name: "utf8mb3_bin".into(),
                    charset: "utf8mb3".into(),
                    is_default: false,
                },
            ],
            "utf8" => vec![
                CollationInfo {
                    name: "utf8_general_ci".into(),
                    charset: "utf8".into(),
                    is_default: true,
                },
                CollationInfo {
                    name: "utf8_unicode_ci".into(),
                    charset: "utf8".into(),
                    is_default: false,
                },
                CollationInfo {
                    name: "utf8_bin".into(),
                    charset: "utf8".into(),
                    is_default: false,
                },
            ],
            "latin1" => vec![
                CollationInfo {
                    name: "latin1_swedish_ci".into(),
                    charset: "latin1".into(),
                    is_default: true,
                },
                CollationInfo {
                    name: "latin1_general_ci".into(),
                    charset: "latin1".into(),
                    is_default: false,
                },
                CollationInfo {
                    name: "latin1_general_cs".into(),
                    charset: "latin1".into(),
                    is_default: false,
                },
                CollationInfo {
                    name: "latin1_bin".into(),
                    charset: "latin1".into(),
                    is_default: false,
                },
            ],
            "latin2" => vec![
                CollationInfo {
                    name: "latin2_general_ci".into(),
                    charset: "latin2".into(),
                    is_default: true,
                },
                CollationInfo {
                    name: "latin2_bin".into(),
                    charset: "latin2".into(),
                    is_default: false,
                },
            ],
            "ascii" => vec![
                CollationInfo {
                    name: "ascii_general_ci".into(),
                    charset: "ascii".into(),
                    is_default: true,
                },
                CollationInfo {
                    name: "ascii_bin".into(),
                    charset: "ascii".into(),
                    is_default: false,
                },
            ],
            "gbk" => vec![
                CollationInfo {
                    name: "gbk_chinese_ci".into(),
                    charset: "gbk".into(),
                    is_default: true,
                },
                CollationInfo {
                    name: "gbk_bin".into(),
                    charset: "gbk".into(),
                    is_default: false,
                },
            ],
            "gb2312" => vec![
                CollationInfo {
                    name: "gb2312_chinese_ci".into(),
                    charset: "gb2312".into(),
                    is_default: true,
                },
                CollationInfo {
                    name: "gb2312_bin".into(),
                    charset: "gb2312".into(),
                    is_default: false,
                },
            ],
            "gb18030" => vec![
                CollationInfo {
                    name: "gb18030_chinese_ci".into(),
                    charset: "gb18030".into(),
                    is_default: true,
                },
                CollationInfo {
                    name: "gb18030_bin".into(),
                    charset: "gb18030".into(),
                    is_default: false,
                },
                CollationInfo {
                    name: "gb18030_unicode_520_ci".into(),
                    charset: "gb18030".into(),
                    is_default: false,
                },
            ],
            "big5" => vec![
                CollationInfo {
                    name: "big5_chinese_ci".into(),
                    charset: "big5".into(),
                    is_default: true,
                },
                CollationInfo {
                    name: "big5_bin".into(),
                    charset: "big5".into(),
                    is_default: false,
                },
            ],
            "sjis" => vec![
                CollationInfo {
                    name: "sjis_japanese_ci".into(),
                    charset: "sjis".into(),
                    is_default: true,
                },
                CollationInfo {
                    name: "sjis_bin".into(),
                    charset: "sjis".into(),
                    is_default: false,
                },
            ],
            "euckr" => vec![
                CollationInfo {
                    name: "euckr_korean_ci".into(),
                    charset: "euckr".into(),
                    is_default: true,
                },
                CollationInfo {
                    name: "euckr_bin".into(),
                    charset: "euckr".into(),
                    is_default: false,
                },
            ],
            "greek" => vec![
                CollationInfo {
                    name: "greek_general_ci".into(),
                    charset: "greek".into(),
                    is_default: true,
                },
                CollationInfo {
                    name: "greek_bin".into(),
                    charset: "greek".into(),
                    is_default: false,
                },
            ],
            "hebrew" => vec![
                CollationInfo {
                    name: "hebrew_general_ci".into(),
                    charset: "hebrew".into(),
                    is_default: true,
                },
                CollationInfo {
                    name: "hebrew_bin".into(),
                    charset: "hebrew".into(),
                    is_default: false,
                },
            ],
            "cp1251" => vec![
                CollationInfo {
                    name: "cp1251_general_ci".into(),
                    charset: "cp1251".into(),
                    is_default: true,
                },
                CollationInfo {
                    name: "cp1251_bin".into(),
                    charset: "cp1251".into(),
                    is_default: false,
                },
            ],
            "cp1256" => vec![
                CollationInfo {
                    name: "cp1256_general_ci".into(),
                    charset: "cp1256".into(),
                    is_default: true,
                },
                CollationInfo {
                    name: "cp1256_bin".into(),
                    charset: "cp1256".into(),
                    is_default: false,
                },
            ],
            "binary" => vec![CollationInfo {
                name: "binary".into(),
                charset: "binary".into(),
                is_default: true,
            }],
            _ => vec![],
        }
    }

    fn engines(&self) -> Vec<String> {
        mysql_engine_names()
    }

    fn get_data_types(&self) -> &[(&'static str, &'static str)] {
        MYSQL_DATA_TYPES
    }

    fn parse_column_type(&self, type_str: &str) -> ParsedColumnType {
        let upper = type_str.to_uppercase();
        let is_unsigned = upper.contains("UNSIGNED");
        let is_auto_increment = upper.contains("AUTO_INCREMENT");

        let base_upper = upper.split_whitespace().next().unwrap_or(&upper);
        if base_upper.starts_with("ENUM") || base_upper.starts_with("SET") {
            if let Some(start) = type_str.find('(') {
                if let Some(end) = type_str.rfind(')') {
                    let base_type = type_str[..start].trim().to_string();
                    let enum_values = type_str[start + 1..end].to_string();
                    return ParsedColumnType {
                        base_type,
                        length: None,
                        scale: None,
                        enum_values: Some(enum_values),
                        is_unsigned,
                        is_auto_increment,
                    };
                }
            }
        }

        if let Some(start) = type_str.find('(') {
            if let Some(end) = type_str.find(')') {
                let base_type = type_str[..start].trim().to_string();
                let params = &type_str[start + 1..end];

                if let Some(comma) = params.find(',') {
                    let length = params[..comma].trim().parse().ok();
                    let scale = params[comma + 1..].trim().parse().ok();
                    return ParsedColumnType {
                        base_type,
                        length,
                        scale,
                        enum_values: None,
                        is_unsigned,
                        is_auto_increment,
                    };
                }

                let length = params.trim().parse().ok();
                return ParsedColumnType {
                    base_type,
                    length,
                    scale: None,
                    enum_values: None,
                    is_unsigned,
                    is_auto_increment,
                };
            }
        }

        ParsedColumnType {
            base_type: type_str
                .split_whitespace()
                .next()
                .unwrap_or(type_str)
                .to_string(),
            length: None,
            scale: None,
            enum_values: None,
            is_unsigned,
            is_auto_increment,
        }
    }

    fn is_enum_type(&self, type_name: &str) -> bool {
        let upper = type_name.to_uppercase();
        upper.starts_with("ENUM") || upper.starts_with("SET")
    }

    fn rename_table(&self, _database: &str, old_name: &str, new_name: &str) -> String {
        format!(
            "RENAME TABLE {} TO {}",
            self.quote_identifier(old_name),
            self.quote_identifier(new_name)
        )
    }

    fn build_backup_table_sql(
        &self,
        database: &str,
        _schema: Option<&str>,
        source_table: &str,
        target_table: &str,
    ) -> String {
        let source = format!(
            "{}.{}",
            self.quote_identifier(database),
            self.quote_identifier(source_table)
        );
        let target = format!(
            "{}.{}",
            self.quote_identifier(database),
            self.quote_identifier(target_table)
        );
        format!(
            "CREATE TABLE {} LIKE {};\nINSERT INTO {} SELECT * FROM {};",
            target, source, target, source
        )
    }

    fn build_column_def(&self, col: &ColumnDefinition) -> String {
        let mut def = String::new();
        def.push_str(&self.quote_identifier(&col.name));
        def.push(' ');

        let type_str = self.build_type_string(col);
        def.push_str(&type_str);

        if col.is_unsigned {
            def.push_str(" UNSIGNED");
        }

        if !col.is_nullable {
            def.push_str(" NOT NULL");
        }

        if col.is_auto_increment {
            def.push_str(" AUTO_INCREMENT");
        }

        if let Some(default) = &col.default_value {
            if !default.is_empty() {
                def.push_str(&format!(" DEFAULT {}", default));
            }
        }

        if !col.comment.is_empty() {
            def.push_str(&format!(" COMMENT '{}'", col.comment.replace("'", "''")));
        }

        def
    }

    fn build_create_table_sql(&self, design: &TableDesign) -> String {
        let mut sql = String::new();
        sql.push_str("CREATE TABLE ");
        sql.push_str(&self.quote_identifier(&design.table_name));
        sql.push_str(" (\n");

        let mut definitions: Vec<String> = Vec::new();

        for col in &design.columns {
            definitions.push(format!("  {}", self.build_column_def(col)));
        }

        let pk_columns: Vec<&str> = design
            .columns
            .iter()
            .filter(|c| c.is_primary_key)
            .map(|c| c.name.as_str())
            .collect();
        if !pk_columns.is_empty() {
            let pk_cols: Vec<String> = pk_columns
                .iter()
                .map(|c| self.quote_identifier(c))
                .collect();
            definitions.push(format!("  PRIMARY KEY ({})", pk_cols.join(", ")));
        }

        for idx in &design.indexes {
            if idx.is_primary {
                continue;
            }
            let idx_cols: Vec<String> = idx
                .columns
                .iter()
                .map(|c| self.quote_identifier(c))
                .collect();
            let idx_type = if idx.is_unique {
                "UNIQUE INDEX"
            } else {
                "INDEX"
            };
            definitions.push(format!(
                "  {} {} ({})",
                idx_type,
                self.quote_identifier(&idx.name),
                idx_cols.join(", ")
            ));
        }

        for foreign_key in &design.foreign_keys {
            definitions.push(format!("  {}", self.build_foreign_key_def(foreign_key)));
        }

        sql.push_str(&definitions.join(",\n"));
        sql.push_str("\n)");

        if let Some(engine) = executable_mysql_option(design.options.engine.as_deref()) {
            sql.push_str(&format!(" ENGINE={}", engine));
        }
        if let Some(charset) = executable_mysql_option(design.options.charset.as_deref()) {
            sql.push_str(&format!(" DEFAULT CHARSET={}", charset));
        }
        if let Some(collation) = executable_mysql_option(design.options.collation.as_deref()) {
            sql.push_str(&format!(" COLLATE={}", collation));
        }
        if !design.options.comment.is_empty() {
            sql.push_str(&format!(
                " COMMENT='{}'",
                design.options.comment.replace("'", "''")
            ));
        }

        sql.push(';');
        sql
    }

    fn build_alter_table_sql(&self, original: &TableDesign, new: &TableDesign) -> String {
        let mut statements: Vec<String> = Vec::new();
        let table_name = self.quote_identifier(&new.table_name);

        let original_cols: HashMap<&str, &ColumnDefinition> = original
            .columns
            .iter()
            .map(|c| (c.name.as_str(), c))
            .collect();
        let new_cols: HashMap<&str, &ColumnDefinition> =
            new.columns.iter().map(|c| (c.name.as_str(), c)).collect();
        let original_order: HashMap<&str, usize> = original
            .columns
            .iter()
            .enumerate()
            .map(|(idx, col)| (col.name.as_str(), idx))
            .collect();
        let original_existing: Vec<&str> = original
            .columns
            .iter()
            .map(|col| col.name.as_str())
            .collect();
        let new_existing: Vec<&str> = new
            .columns
            .iter()
            .filter(|col| original_cols.contains_key(col.name.as_str()))
            .map(|col| col.name.as_str())
            .collect();
        let order_changed = original_existing != new_existing;
        let new_existing_positions: HashMap<&str, usize> = new_existing
            .iter()
            .enumerate()
            .map(|(idx, name)| (*name, idx))
            .collect();

        if order_changed {
            tracing::warn!(
                target: "table_designer_diag",
                table = %new.table_name,
                ?original_existing,
                ?new_existing,
                "[table_designer_diag][mysql] detected existing-column order change"
            );
        }

        let original_foreign_keys: HashMap<&str, &ForeignKeyDefinition> = original
            .foreign_keys
            .iter()
            .map(|foreign_key| (foreign_key.name.as_str(), foreign_key))
            .collect();
        let new_foreign_keys: HashMap<&str, &ForeignKeyDefinition> = new
            .foreign_keys
            .iter()
            .map(|foreign_key| (foreign_key.name.as_str(), foreign_key))
            .collect();

        for (name, original_foreign_key) in &original_foreign_keys {
            match new_foreign_keys.get(name) {
                Some(new_foreign_key)
                    if !Self::foreign_key_changed(original_foreign_key, new_foreign_key) => {}
                _ => {
                    statements.push(self.build_drop_foreign_key_sql(&new.table_name, name));
                }
            }
        }

        for name in original_cols.keys() {
            if !new_cols.contains_key(name) {
                statements.push(format!(
                    "ALTER TABLE {} DROP COLUMN {};",
                    table_name,
                    self.quote_identifier(name)
                ));
            }
        }

        for (idx, col) in new.columns.iter().enumerate() {
            if let Some(orig_col) = original_cols.get(col.name.as_str()) {
                let changed_fields = Self::column_change_reasons(orig_col, col);

                if !changed_fields.is_empty() {
                    let col_def = self.build_column_def(col);
                    let position = if idx == 0 {
                        " FIRST".to_string()
                    } else {
                        format!(
                            " AFTER {}",
                            self.quote_identifier(&new.columns[idx - 1].name)
                        )
                    };
                    tracing::warn!(
                        target: "table_designer_diag",
                        table = %new.table_name,
                        column = %col.name,
                        ?changed_fields,
                        original = ?orig_col,
                        current = ?col,
                        position = %position,
                        "[table_designer_diag][mysql] generating MODIFY COLUMN because column fields changed"
                    );
                    statements.push(format!(
                        "ALTER TABLE {} MODIFY COLUMN {}{};",
                        table_name, col_def, position
                    ));
                } else if order_changed {
                    let original_idx = original_order.get(col.name.as_str());
                    let new_idx = new_existing_positions.get(col.name.as_str());
                    if let (Some(original_idx), Some(new_idx)) = (original_idx, new_idx) {
                        if original_idx != new_idx {
                            let col_def = self.build_column_def(col);
                            let position = if idx == 0 {
                                " FIRST".to_string()
                            } else {
                                format!(
                                    " AFTER {}",
                                    self.quote_identifier(&new.columns[idx - 1].name)
                                )
                            };
                            tracing::warn!(
                                target: "table_designer_diag",
                                table = %new.table_name,
                                column = %col.name,
                                original_index = *original_idx,
                                new_existing_index = *new_idx,
                                position = %position,
                                "[table_designer_diag][mysql] generating MODIFY COLUMN because existing-column order changed"
                            );
                            statements.push(format!(
                                "ALTER TABLE {} MODIFY COLUMN {}{};",
                                table_name, col_def, position
                            ));
                        }
                    }
                }
            } else {
                let col_def = self.build_column_def(col);
                let position = if idx == 0 {
                    " FIRST".to_string()
                } else {
                    format!(
                        " AFTER {}",
                        self.quote_identifier(&new.columns[idx - 1].name)
                    )
                };

                statements.push(format!(
                    "ALTER TABLE {} ADD COLUMN {}{};",
                    table_name, col_def, position
                ));
            }
        }

        let original_indexes: HashMap<&str, &IndexDefinition> = original
            .indexes
            .iter()
            .map(|i| (i.name.as_str(), i))
            .collect();
        let new_indexes: HashMap<&str, &IndexDefinition> =
            new.indexes.iter().map(|i| (i.name.as_str(), i)).collect();

        for (name, idx) in &original_indexes {
            if !new_indexes.contains_key(name) {
                if idx.is_primary {
                    statements.push(format!("ALTER TABLE {} DROP PRIMARY KEY;", table_name));
                } else {
                    statements.push(format!(
                        "ALTER TABLE {} DROP INDEX {};",
                        table_name,
                        self.quote_identifier(name)
                    ));
                }
            }
        }

        for (name, idx) in &new_indexes {
            if !original_indexes.contains_key(name) {
                let idx_cols: Vec<String> = idx
                    .columns
                    .iter()
                    .map(|c| self.quote_identifier(c))
                    .collect();

                if idx.is_primary {
                    statements.push(format!(
                        "ALTER TABLE {} ADD PRIMARY KEY ({});",
                        table_name,
                        idx_cols.join(", ")
                    ));
                } else {
                    let idx_type = if idx.is_unique {
                        "UNIQUE INDEX"
                    } else {
                        "INDEX"
                    };
                    statements.push(format!(
                        "ALTER TABLE {} ADD {} {} ({});",
                        table_name,
                        idx_type,
                        self.quote_identifier(name),
                        idx_cols.join(", ")
                    ));
                }
            }
        }

        for (name, new_foreign_key) in &new_foreign_keys {
            match original_foreign_keys.get(name) {
                Some(original_foreign_key)
                    if !Self::foreign_key_changed(original_foreign_key, new_foreign_key) => {}
                _ => {
                    statements
                        .push(self.build_add_foreign_key_sql(&new.table_name, new_foreign_key));
                }
            }
        }

        let mut options_changed = false;
        let mut option_parts: Vec<String> = Vec::new();

        if original.options.engine != new.options.engine
            && original.options.engine.is_some()
            && new.options.engine.is_some()
        {
            if let Some(engine) = executable_mysql_option(new.options.engine.as_deref()) {
                option_parts.push(format!("ENGINE={}", engine));
                options_changed = true;
            }
        }

        if original.options.charset != new.options.charset
            && original.options.charset.is_some()
            && new.options.charset.is_some()
        {
            if let Some(charset) = executable_mysql_option(new.options.charset.as_deref()) {
                option_parts.push(format!("DEFAULT CHARSET={}", charset));
                options_changed = true;
            }
        }

        if original.options.collation != new.options.collation
            && original.options.collation.is_some()
            && new.options.collation.is_some()
        {
            if let Some(collation) = executable_mysql_option(new.options.collation.as_deref()) {
                option_parts.push(format!("COLLATE={}", collation));
                options_changed = true;
            }
        }

        if original.options.comment != new.options.comment {
            option_parts.push(format!(
                "COMMENT='{}'",
                new.options.comment.replace("'", "''")
            ));
            options_changed = true;
        }

        if options_changed && !option_parts.is_empty() {
            statements.push(format!(
                "ALTER TABLE {} {};",
                table_name,
                option_parts.join(" ")
            ));
        }

        if statements.is_empty() {
            "-- No changes detected".to_string()
        } else {
            statements.join("\n")
        }
    }

    fn build_drop_foreign_key_sql(&self, table_name: &str, foreign_key_name: &str) -> String {
        format!(
            "ALTER TABLE {} DROP FOREIGN KEY {};",
            self.quote_identifier(table_name),
            self.quote_identifier(foreign_key_name)
        )
    }

    /// MySQL 使用 CHANGE COLUMN 语法进行列重命名，需要完整列定义。
    fn build_column_rename_sql(
        &self,
        table_name: &str,
        old_name: &str,
        new_name: &str,
        new_column: Option<&ColumnDefinition>,
    ) -> String {
        let quoted_table = self.quote_identifier(table_name);
        let quoted_old = self.quote_identifier(old_name);
        if let Some(col) = new_column {
            let col_def = self.build_column_def(col);
            format!(
                "ALTER TABLE {} CHANGE COLUMN {} {};",
                quoted_table, quoted_old, col_def
            )
        } else {
            let quoted_new = self.quote_identifier(new_name);
            format!(
                "ALTER TABLE {} RENAME COLUMN {} TO {};",
                quoted_table, quoted_old, quoted_new
            )
        }
    }

    async fn import_data_with_progress(
        &self,
        connection: &dyn DbConnection,
        config: &ImportConfig,
        data: &str,
        file_name: &str,
        progress_tx: Option<ImportProgressSender>,
    ) -> Result<ImportResult> {
        crate::plugin::default_import_data_with_progress(
            self,
            connection,
            config,
            data,
            file_name,
            progress_tx,
        )
        .await
    }

    async fn export_data_with_progress(
        &self,
        connection: &dyn DbConnection,
        config: &ExportConfig,
        progress_tx: Option<ExportProgressSender>,
    ) -> Result<ExportResult> {
        crate::plugin::default_export_data_with_progress(self, connection, config, progress_tx)
            .await
    }
}

impl Default for MySqlPlugin {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plugin::DatabasePlugin;
    use crate::types::{ColumnDefinition, IndexDefinition, TableDesign, TableOptions};
    use crate::{DatabaseActionId, DatabaseFormKind, FormValueCondition, ReferenceDataKind};

    fn create_plugin() -> MySqlPlugin {
        MySqlPlugin::new()
    }

    fn cell(value: &str) -> Option<String> {
        Some(value.to_string())
    }

    fn row(values: &[&str]) -> Vec<Option<String>> {
        values.iter().map(|value| cell(value)).collect()
    }

    fn user_request(
        user_name: &str,
        host: Option<&str>,
        database: Option<&str>,
        values: &[(&str, &str)],
    ) -> crate::plugin::DatabaseUserOperationRequest {
        crate::plugin::DatabaseUserOperationRequest {
            user_name: user_name.to_string(),
            host: host.map(str::to_string),
            database: database.map(str::to_string),
            field_values: values
                .iter()
                .map(|(key, value)| ((*key).to_string(), (*value).to_string()))
                .collect(),
        }
    }

    // ==================== Basic Plugin Info Tests ====================

    #[test]
    fn test_plugin_name() {
        let plugin = create_plugin();
        assert_eq!(plugin.name(), DatabaseType::MySQL);
    }

    #[test]
    fn test_quote_identifier() {
        let plugin = create_plugin();
        assert_eq!(plugin.quote_identifier("table_name"), "`table_name`");
        assert_eq!(plugin.quote_identifier("column"), "`column`");
        assert_eq!(plugin.quote_identifier("col`umn"), "`col``umn`");
    }

    #[test]
    fn test_capabilities_support_users() {
        let capabilities = create_plugin().capabilities();

        assert!(capabilities.supports_users);
        assert!(capabilities.supports_user_create);
        assert!(capabilities.supports_user_edit);
        assert!(capabilities.supports_user_delete);
        assert!(capabilities.supports_user_privileges);
    }

    #[test]
    fn test_mysql_foreign_keys_sql_targets_table_metadata() {
        let sql = mysql_foreign_keys_sql("app", "order_items");

        assert!(sql.contains("INFORMATION_SCHEMA.KEY_COLUMN_USAGE"));
        assert!(sql.contains("INFORMATION_SCHEMA.REFERENTIAL_CONSTRAINTS"));
        assert!(sql.contains("k.TABLE_SCHEMA = 'app'"));
        assert!(sql.contains("k.TABLE_NAME = 'order_items'"));
        assert!(sql.contains("k.REFERENCED_TABLE_NAME IS NOT NULL"));
    }

    #[test]
    fn test_parse_mysql_foreign_keys_groups_composite_columns() {
        let foreign_keys = parse_mysql_foreign_keys(vec![
            row(&[
                "fk_order_items_order",
                "tenant_id",
                "orders",
                "tenant_id",
                "CASCADE",
                "RESTRICT",
            ]),
            row(&[
                "fk_order_items_order",
                "order_id",
                "orders",
                "id",
                "CASCADE",
                "RESTRICT",
            ]),
        ]);

        assert_eq!(1, foreign_keys.len());
        assert_eq!("fk_order_items_order", foreign_keys[0].name);
        assert_eq!(
            vec!["tenant_id".to_string(), "order_id".to_string()],
            foreign_keys[0].columns
        );
        assert_eq!("orders", foreign_keys[0].ref_table);
        assert_eq!(
            vec!["tenant_id".to_string(), "id".to_string()],
            foreign_keys[0].ref_columns
        );
        assert_eq!("CASCADE", foreign_keys[0].on_delete);
        assert_eq!("RESTRICT", foreign_keys[0].on_update);
    }

    #[test]
    fn test_mysql_table_triggers_sql_filters_table() {
        let sql = mysql_table_triggers_sql("app", "orders");

        assert!(sql.contains("INFORMATION_SCHEMA.TRIGGERS"));
        assert!(sql.contains("TRIGGER_SCHEMA = 'app'"));
        assert!(sql.contains("EVENT_OBJECT_TABLE = 'orders'"));
        assert!(sql.contains("ACTION_STATEMENT"));
    }

    #[test]
    fn test_parse_mysql_triggers_maps_table_event_timing_and_definition() {
        let triggers = parse_mysql_triggers(vec![row(&[
            "orders_before_insert",
            "orders",
            "INSERT",
            "BEFORE",
            "SET NEW.created_at = NOW()",
        ])]);

        assert_eq!(1, triggers.len());
        assert_eq!("orders_before_insert", triggers[0].name);
        assert_eq!("orders", triggers[0].table_name);
        assert_eq!("INSERT", triggers[0].event);
        assert_eq!("BEFORE", triggers[0].timing);
        assert_eq!(
            Some("SET NEW.created_at = NOW()".to_string()),
            triggers[0].definition
        );
    }

    // ==================== DDL SQL Generation Tests ====================

    #[test]
    fn test_drop_database() {
        let plugin = create_plugin();
        let sql = plugin.drop_database("test_db");
        assert!(sql.contains("DROP DATABASE"));
        assert!(sql.contains("`test_db`"));
    }

    #[test]
    fn test_drop_table() {
        let plugin = create_plugin();
        let sql = plugin.drop_table("test_db", None, "users");
        assert!(sql.contains("DROP TABLE"));
        assert!(sql.contains("`test_db`"));
        assert!(sql.contains("`users`"));
    }

    #[test]
    fn test_truncate_table() {
        let plugin = create_plugin();
        let sql = plugin.truncate_table("test_db", "users");
        assert!(sql.contains("TRUNCATE TABLE"));
        assert!(sql.contains("`users`"));
    }

    #[test]
    fn test_rename_table() {
        let plugin = create_plugin();
        let sql = plugin.rename_table("test_db", "old_name", "new_name");
        assert!(sql.contains("RENAME TABLE"));
        assert!(sql.contains("`old_name`"));
        assert!(sql.contains("`new_name`"));
    }

    #[test]
    fn test_build_backup_table_sql() {
        let plugin = create_plugin();
        let sql = plugin.build_backup_table_sql("test_db", None, "orders", "orders_bak");
        assert!(sql.contains("CREATE TABLE `test_db`.`orders_bak` LIKE `test_db`.`orders`;"));
        assert!(
            sql.contains("INSERT INTO `test_db`.`orders_bak` SELECT * FROM `test_db`.`orders`;")
        );
    }

    #[test]
    fn test_drop_view() {
        let plugin = create_plugin();
        let sql = plugin.drop_view("test_db", "my_view");
        assert!(sql.contains("DROP VIEW"));
        assert!(sql.contains("`my_view`"));
    }

    #[test]
    fn test_build_list_users_sql() {
        let plugin = create_plugin();
        let sql = plugin
            .build_list_users_sql(Some("appdb"))
            .expect("MySQL supports user listing");

        assert!(sql.contains("FROM mysql.user"));
        assert!(sql.contains("User"));
        assert!(sql.contains("Host"));
        assert!(sql.contains("account_locked"));
    }

    #[test]
    fn test_build_mysql_user_operation_sql_escapes_user_host_and_password() {
        let plugin = create_plugin();
        let request = user_request(
            "app'user",
            Some("10.%"),
            Some("app`db"),
            &[("password", "pa'ss"), ("privileges", "SELECT")],
        );

        assert_eq!(
            Some("CREATE USER 'app''user'@'10.%' IDENTIFIED BY 'pa''ss';".to_string()),
            plugin.build_create_user_sql(&request)
        );
        assert_eq!(
            Some("ALTER USER 'app''user'@'10.%' IDENTIFIED BY 'pa''ss';".to_string()),
            plugin.build_modify_user_sql(&request)
        );
        assert_eq!(
            Some("DROP USER 'app''user'@'10.%';".to_string()),
            plugin.build_drop_user_sql(&request)
        );
        assert_eq!(
            Some("GRANT SELECT ON `app``db`.* TO 'app''user'@'10.%';".to_string()),
            plugin.build_user_privileges_sql(&request)
        );
    }

    // ==================== Database Operations Tests ====================

    #[test]
    fn test_build_create_database_sql() {
        let plugin = create_plugin();
        let mut field_values = HashMap::new();
        field_values.insert("charset".to_string(), "utf8mb4".to_string());
        field_values.insert("collation".to_string(), "utf8mb4_unicode_ci".to_string());

        let request = crate::plugin::DatabaseOperationRequest {
            database_name: "new_db".to_string(),
            field_values,
        };

        let sql = plugin.build_create_database_sql(&request);
        assert!(sql.contains("CREATE DATABASE"));
        assert!(sql.contains("`new_db`"));
        assert!(sql.contains("utf8mb4"));
        assert!(sql.contains("utf8mb4_unicode_ci"));
    }

    #[test]
    fn test_build_create_database_sql_escapes_identifier() {
        let plugin = create_plugin();
        let mut field_values = HashMap::new();
        field_values.insert("charset".to_string(), "utf8mb4".to_string());
        field_values.insert("collation".to_string(), "utf8mb4_general_ci".to_string());

        let request = crate::plugin::DatabaseOperationRequest {
            database_name: "new`db".to_string(),
            field_values,
        };

        let sql = plugin.build_create_database_sql(&request);
        assert!(sql.contains("CREATE DATABASE"));
        assert!(sql.contains("`new``db`"));
    }

    #[test]
    fn test_build_modify_database_sql() {
        let plugin = create_plugin();
        let mut field_values = HashMap::new();
        field_values.insert("charset".to_string(), "utf8mb4".to_string());
        field_values.insert("collation".to_string(), "utf8mb4_bin".to_string());

        let request = crate::plugin::DatabaseOperationRequest {
            database_name: "my_db".to_string(),
            field_values,
        };

        let sql = plugin.build_modify_database_sql(&request);
        assert!(sql.contains("ALTER DATABASE"));
        assert!(sql.contains("`my_db`"));
        assert!(sql.contains("utf8mb4_bin"));
    }

    #[test]
    fn test_build_drop_database_sql() {
        let plugin = create_plugin();
        let sql = plugin.build_drop_database_sql("old_db");
        assert_eq!(sql, "DROP DATABASE `old_db`;");
    }

    #[test]
    fn test_build_drop_database_sql_escapes_identifier() {
        let plugin = create_plugin();
        let sql = plugin.build_drop_database_sql("old`db");
        assert_eq!(sql, "DROP DATABASE `old``db`;");
    }

    // ==================== Column Definition Tests ====================

    #[test]
    fn test_build_column_def_simple() {
        let plugin = create_plugin();
        let col = ColumnDefinition::new("id")
            .data_type("INT")
            .nullable(false)
            .primary_key(true)
            .auto_increment(true);

        let def = plugin.build_column_def(&col);
        assert!(def.contains("`id`"));
        assert!(def.contains("INT"));
        assert!(def.contains("NOT NULL"));
        assert!(def.contains("AUTO_INCREMENT"));
    }

    #[test]
    fn test_build_column_def_with_length() {
        let plugin = create_plugin();
        let col = ColumnDefinition::new("name")
            .data_type("VARCHAR")
            .length(255)
            .nullable(true);

        let def = plugin.build_column_def(&col);
        assert!(def.contains("`name`"));
        assert!(def.contains("VARCHAR(255)"));
        assert!(!def.contains("NOT NULL"));
    }

    #[test]
    fn test_build_column_def_with_default() {
        let plugin = create_plugin();
        let mut col = ColumnDefinition::new("status")
            .data_type("INT")
            .default_value("0");
        col.is_nullable = false;

        let def = plugin.build_column_def(&col);
        assert!(def.contains("DEFAULT 0"));
        assert!(def.contains("NOT NULL"));
    }

    #[test]
    fn test_build_column_def_with_comment() {
        let plugin = create_plugin();
        let col = ColumnDefinition::new("email")
            .data_type("VARCHAR")
            .length(100)
            .comment("User email address");

        let def = plugin.build_column_def(&col);
        assert!(def.contains("COMMENT 'User email address'"));
    }

    #[test]
    fn test_build_column_def_unsigned() {
        let plugin = create_plugin();
        let mut col = ColumnDefinition::new("age").data_type("INT");
        col.is_unsigned = true;
        col.is_nullable = false;

        let def = plugin.build_column_def(&col);
        assert!(def.contains("UNSIGNED"));
    }

    #[test]
    fn test_build_column_def_decimal() {
        let plugin = create_plugin();
        let mut col = ColumnDefinition::new("price").data_type("DECIMAL");
        col.length = Some(10);

        let def = plugin.build_column_def(&col);
        assert!(def.contains("DECIMAL(10)"));
    }

    // ==================== CREATE TABLE Tests ====================

    #[test]
    fn test_build_create_table_sql_simple() {
        let plugin = create_plugin();
        let design = TableDesign {
            database_name: "test_db".to_string(),
            table_name: "users".to_string(),
            columns: vec![
                ColumnDefinition::new("id")
                    .data_type("INT")
                    .nullable(false)
                    .primary_key(true)
                    .auto_increment(true),
                ColumnDefinition::new("name")
                    .data_type("VARCHAR")
                    .length(100),
            ],
            indexes: vec![],
            foreign_keys: vec![],
            options: TableOptions::default(),
        };

        let sql = plugin.build_create_table_sql(&design);
        assert!(sql.contains("CREATE TABLE `users`"));
        assert!(sql.contains("`id`"));
        assert!(sql.contains("INT"));
        assert!(sql.contains("AUTO_INCREMENT"));
        assert!(sql.contains("`name`"));
        assert!(sql.contains("VARCHAR(100)"));
        assert!(sql.contains("PRIMARY KEY"));
    }

    #[test]
    fn test_build_create_table_sql_with_options() {
        let plugin = create_plugin();
        let design = TableDesign {
            database_name: "test_db".to_string(),
            table_name: "products".to_string(),
            columns: vec![
                ColumnDefinition::new("id")
                    .data_type("INT")
                    .nullable(false)
                    .primary_key(true),
            ],
            indexes: vec![],
            foreign_keys: vec![],
            options: TableOptions {
                engine: Some("InnoDB".to_string()),
                charset: Some("utf8mb4".to_string()),
                collation: Some("utf8mb4_unicode_ci".to_string()),
                comment: "Product table".to_string(),
                auto_increment: None,
            },
        };

        let sql = plugin.build_create_table_sql(&design);
        assert!(sql.contains("ENGINE=InnoDB"));
        assert!(sql.contains("DEFAULT CHARSET=utf8mb4"));
        assert!(sql.contains("COLLATE=utf8mb4_unicode_ci"));
        assert!(sql.contains("COMMENT='Product table'"));
    }

    #[test]
    fn test_build_create_table_sql_skips_display_labels_in_options() {
        let plugin = create_plugin();
        let design = TableDesign {
            database_name: "test_db".to_string(),
            table_name: "products".to_string(),
            columns: vec![ColumnDefinition::new("id").data_type("INT").nullable(false)],
            indexes: vec![],
            foreign_keys: vec![],
            options: TableOptions {
                engine: Some("Default InnoDB".to_string()),
                charset: Some("utf8mb4 - UTF-8 Unicode (4 bytes)".to_string()),
                collation: Some("Default UTF-8".to_string()),
                comment: String::new(),
                auto_increment: None,
            },
        };

        let sql = plugin.build_create_table_sql(&design);
        assert!(!sql.contains("Default InnoDB"));
        assert!(!sql.contains("UTF-8"));
        assert!(!sql.contains("DEFAULT CHARSET"));
        assert!(!sql.contains("COLLATE="));
    }

    #[test]
    fn test_build_create_table_sql_with_indexes() {
        let plugin = create_plugin();
        let design = TableDesign {
            database_name: "test_db".to_string(),
            table_name: "orders".to_string(),
            columns: vec![
                ColumnDefinition::new("id")
                    .data_type("INT")
                    .nullable(false)
                    .primary_key(true),
                ColumnDefinition::new("user_id")
                    .data_type("INT")
                    .nullable(false),
                ColumnDefinition::new("email")
                    .data_type("VARCHAR")
                    .length(100),
            ],
            indexes: vec![
                IndexDefinition::new("idx_user_id")
                    .columns(vec!["user_id".to_string()])
                    .unique(false),
                IndexDefinition::new("idx_email")
                    .columns(vec!["email".to_string()])
                    .unique(true),
            ],
            foreign_keys: vec![],
            options: TableOptions::default(),
        };

        let sql = plugin.build_create_table_sql(&design);
        assert!(sql.contains("INDEX `idx_user_id`"));
        assert!(sql.contains("UNIQUE INDEX `idx_email`"));
    }

    #[test]
    fn test_build_create_table_sql_with_foreign_keys() {
        let plugin = create_plugin();
        let design = TableDesign {
            database_name: "test_db".to_string(),
            table_name: "order_items".to_string(),
            columns: vec![
                ColumnDefinition::new("id").data_type("INT").nullable(false),
                ColumnDefinition::new("order_id")
                    .data_type("INT")
                    .nullable(false),
            ],
            indexes: vec![],
            foreign_keys: vec![ForeignKeyDefinition {
                name: "fk_order_items_order".to_string(),
                columns: vec!["order_id".to_string()],
                ref_table: "orders".to_string(),
                ref_columns: vec!["id".to_string()],
                on_delete: "CASCADE".to_string(),
                on_update: "RESTRICT".to_string(),
            }],
            options: TableOptions::default(),
        };

        let sql = plugin.build_create_table_sql(&design);

        assert!(sql.contains(
            "CONSTRAINT `fk_order_items_order` FOREIGN KEY (`order_id`) REFERENCES `orders` (`id`) ON DELETE CASCADE ON UPDATE RESTRICT"
        ));
    }

    // ==================== ALTER TABLE Tests ====================

    #[test]
    fn test_build_alter_table_sql_add_column() {
        let plugin = create_plugin();

        let original = TableDesign {
            database_name: "test_db".to_string(),
            table_name: "users".to_string(),
            columns: vec![ColumnDefinition::new("id").data_type("INT")],
            indexes: vec![],
            foreign_keys: vec![],
            options: TableOptions::default(),
        };

        let new = TableDesign {
            database_name: "test_db".to_string(),
            table_name: "users".to_string(),
            columns: vec![
                ColumnDefinition::new("id").data_type("INT"),
                ColumnDefinition::new("email")
                    .data_type("VARCHAR")
                    .length(100),
            ],
            indexes: vec![],
            foreign_keys: vec![],
            options: TableOptions::default(),
        };

        let sql = plugin.build_alter_table_sql(&original, &new);
        assert!(sql.contains("ADD COLUMN"));
        assert!(sql.contains("`email`"));
    }

    #[test]
    fn test_build_alter_table_sql_skips_display_labels_in_options() {
        let plugin = create_plugin();

        let original = TableDesign {
            database_name: "test_db".to_string(),
            table_name: "users".to_string(),
            columns: vec![ColumnDefinition::new("id").data_type("INT")],
            indexes: vec![],
            foreign_keys: vec![],
            options: TableOptions {
                engine: Some("InnoDB".to_string()),
                charset: Some("utf8mb4".to_string()),
                collation: Some("utf8mb4_general_ci".to_string()),
                comment: String::new(),
                auto_increment: None,
            },
        };
        let new = TableDesign {
            options: TableOptions {
                engine: Some("Default InnoDB".to_string()),
                charset: Some("utf8mb4 - UTF-8 Unicode (4 bytes)".to_string()),
                collation: Some("Default UTF-8".to_string()),
                comment: String::new(),
                auto_increment: None,
            },
            ..original.clone()
        };

        let sql = plugin.build_alter_table_sql(&original, &new);
        assert!(!sql.contains("Default InnoDB"));
        assert!(!sql.contains("UTF-8"));
        assert!(!sql.contains("DEFAULT CHARSET"));
        assert!(!sql.contains("COLLATE="));
    }

    #[test]
    fn test_build_alter_table_sql_adds_table_comment_from_empty() {
        let plugin = create_plugin();

        let original = TableDesign {
            database_name: "test_db".to_string(),
            table_name: "users".to_string(),
            columns: vec![ColumnDefinition::new("id").data_type("INT")],
            indexes: vec![],
            foreign_keys: vec![],
            options: TableOptions::default(),
        };
        let new = TableDesign {
            options: TableOptions {
                comment: "User table".to_string(),
                ..TableOptions::default()
            },
            ..original.clone()
        };

        let sql = plugin.build_alter_table_sql(&original, &new);
        assert!(sql.contains("ALTER TABLE `users` COMMENT='User table';"));
    }

    #[test]
    fn test_build_alter_table_sql_add_column_no_reorder() {
        let plugin = create_plugin();

        let original = TableDesign {
            database_name: "test_db".to_string(),
            table_name: "users".to_string(),
            columns: vec![
                ColumnDefinition::new("id").data_type("INT"),
                ColumnDefinition::new("name")
                    .data_type("VARCHAR")
                    .length(50),
            ],
            indexes: vec![],
            foreign_keys: vec![],
            options: TableOptions::default(),
        };

        let new = TableDesign {
            database_name: "test_db".to_string(),
            table_name: "users".to_string(),
            columns: vec![
                ColumnDefinition::new("id").data_type("INT"),
                ColumnDefinition::new("email")
                    .data_type("VARCHAR")
                    .length(100),
                ColumnDefinition::new("name")
                    .data_type("VARCHAR")
                    .length(50),
            ],
            indexes: vec![],
            foreign_keys: vec![],
            options: TableOptions::default(),
        };

        let sql = plugin.build_alter_table_sql(&original, &new);
        assert!(sql.contains("ADD COLUMN"));
        assert!(sql.contains("`email`"));
        assert!(!sql.contains("MODIFY COLUMN `name`"));
    }

    #[test]
    fn test_build_alter_table_sql_drop_column() {
        let plugin = create_plugin();

        let original = TableDesign {
            database_name: "test_db".to_string(),
            table_name: "users".to_string(),
            columns: vec![
                ColumnDefinition::new("id").data_type("INT"),
                ColumnDefinition::new("old_column")
                    .data_type("VARCHAR")
                    .length(50),
            ],
            indexes: vec![],
            foreign_keys: vec![],
            options: TableOptions::default(),
        };

        let new = TableDesign {
            database_name: "test_db".to_string(),
            table_name: "users".to_string(),
            columns: vec![ColumnDefinition::new("id").data_type("INT")],
            indexes: vec![],
            foreign_keys: vec![],
            options: TableOptions::default(),
        };

        let sql = plugin.build_alter_table_sql(&original, &new);
        assert!(sql.contains("DROP COLUMN"));
        assert!(sql.contains("`old_column`"));
    }

    #[test]
    fn test_build_alter_table_sql_modify_column() {
        let plugin = create_plugin();

        let original = TableDesign {
            database_name: "test_db".to_string(),
            table_name: "users".to_string(),
            columns: vec![
                ColumnDefinition::new("name")
                    .data_type("VARCHAR")
                    .length(50),
            ],
            indexes: vec![],
            foreign_keys: vec![],
            options: TableOptions::default(),
        };

        let new = TableDesign {
            database_name: "test_db".to_string(),
            table_name: "users".to_string(),
            columns: vec![
                ColumnDefinition::new("name")
                    .data_type("VARCHAR")
                    .length(100),
            ],
            indexes: vec![],
            foreign_keys: vec![],
            options: TableOptions::default(),
        };

        let sql = plugin.build_alter_table_sql(&original, &new);
        assert!(sql.contains("MODIFY COLUMN"));
        assert!(sql.contains("`name`"));
        assert!(sql.contains("VARCHAR(100)"));
    }

    #[test]
    fn test_build_alter_table_sql_add_and_drop_foreign_keys() {
        let plugin = create_plugin();

        let original = TableDesign {
            database_name: "test_db".to_string(),
            table_name: "order_items".to_string(),
            columns: vec![
                ColumnDefinition::new("id").data_type("INT"),
                ColumnDefinition::new("order_id").data_type("INT"),
                ColumnDefinition::new("legacy_order_id").data_type("INT"),
            ],
            indexes: vec![],
            foreign_keys: vec![ForeignKeyDefinition {
                name: "fk_order_items_legacy".to_string(),
                columns: vec!["legacy_order_id".to_string()],
                ref_table: "orders".to_string(),
                ref_columns: vec!["id".to_string()],
                on_delete: String::new(),
                on_update: String::new(),
            }],
            options: TableOptions::default(),
        };
        let new = TableDesign {
            database_name: "test_db".to_string(),
            table_name: "order_items".to_string(),
            columns: vec![
                ColumnDefinition::new("id").data_type("INT"),
                ColumnDefinition::new("order_id").data_type("INT"),
            ],
            indexes: vec![],
            foreign_keys: vec![ForeignKeyDefinition {
                name: "fk_order_items_order".to_string(),
                columns: vec!["order_id".to_string()],
                ref_table: "orders".to_string(),
                ref_columns: vec!["id".to_string()],
                on_delete: "CASCADE".to_string(),
                on_update: "RESTRICT".to_string(),
            }],
            options: TableOptions::default(),
        };

        let sql = plugin.build_alter_table_sql(&original, &new);

        assert!(
            sql.contains("ALTER TABLE `order_items` DROP FOREIGN KEY `fk_order_items_legacy`;")
        );
        assert!(
            sql.find("DROP FOREIGN KEY `fk_order_items_legacy`")
                .unwrap()
                < sql.find("DROP COLUMN `legacy_order_id`").unwrap()
        );
        assert!(sql.contains(
            "ALTER TABLE `order_items` ADD CONSTRAINT `fk_order_items_order` FOREIGN KEY (`order_id`) REFERENCES `orders` (`id`) ON DELETE CASCADE ON UPDATE RESTRICT;"
        ));
    }

    #[test]
    fn test_build_alter_table_sql_reorder_columns() {
        let plugin = create_plugin();

        let original = TableDesign {
            database_name: "test_db".to_string(),
            table_name: "users".to_string(),
            columns: vec![
                ColumnDefinition::new("id").data_type("INT"),
                ColumnDefinition::new("name")
                    .data_type("VARCHAR")
                    .length(50),
                ColumnDefinition::new("age").data_type("INT"),
            ],
            indexes: vec![],
            foreign_keys: vec![],
            options: TableOptions::default(),
        };

        let new = TableDesign {
            database_name: "test_db".to_string(),
            table_name: "users".to_string(),
            columns: vec![
                ColumnDefinition::new("name")
                    .data_type("VARCHAR")
                    .length(50),
                ColumnDefinition::new("id").data_type("INT"),
                ColumnDefinition::new("age").data_type("INT"),
            ],
            indexes: vec![],
            foreign_keys: vec![],
            options: TableOptions::default(),
        };

        let sql = plugin.build_alter_table_sql(&original, &new);
        assert!(sql.contains("MODIFY COLUMN"));
        assert!(sql.contains("`name`"));
        assert!(sql.contains(" AFTER `id`") || sql.contains(" FIRST"));
    }

    #[test]
    fn test_build_alter_table_sql_reorder_with_modify_column() {
        let plugin = create_plugin();

        let original = TableDesign {
            database_name: "test_db".to_string(),
            table_name: "users".to_string(),
            columns: vec![
                ColumnDefinition::new("id").data_type("INT"),
                ColumnDefinition::new("name")
                    .data_type("VARCHAR")
                    .length(50),
                ColumnDefinition::new("age").data_type("INT"),
            ],
            indexes: vec![],
            foreign_keys: vec![],
            options: TableOptions::default(),
        };

        let new = TableDesign {
            database_name: "test_db".to_string(),
            table_name: "users".to_string(),
            columns: vec![
                ColumnDefinition::new("age").data_type("INT"),
                ColumnDefinition::new("id").data_type("INT"),
                ColumnDefinition::new("name")
                    .data_type("VARCHAR")
                    .length(120),
            ],
            indexes: vec![],
            foreign_keys: vec![],
            options: TableOptions::default(),
        };

        let sql = plugin.build_alter_table_sql(&original, &new);
        let modify_count = sql.matches("MODIFY COLUMN `name`").count();
        assert_eq!(modify_count, 1);
        assert!(sql.contains("VARCHAR(120)"));
        assert!(sql.contains(" AFTER `id`") || sql.contains(" FIRST"));
    }

    #[test]
    fn test_build_alter_table_sql_no_changes_with_text_metadata() {
        let plugin = create_plugin();

        let column = ColumnDefinition {
            name: "session_id".to_string(),
            data_type: "varchar".to_string(),
            length: Some(255),
            is_nullable: false,
            comment: "会话ID".to_string(),
            charset: Some("utf8mb4".to_string()),
            collation: Some("utf8mb4_general_ci".to_string()),
            ..Default::default()
        };
        let original = TableDesign {
            database_name: "test_db".to_string(),
            table_name: "task_execution_record".to_string(),
            columns: vec![column.clone()],
            indexes: vec![],
            foreign_keys: vec![],
            options: TableOptions::default(),
        };
        let new = TableDesign {
            database_name: "test_db".to_string(),
            table_name: "task_execution_record".to_string(),
            columns: vec![column],
            indexes: vec![],
            foreign_keys: vec![],
            options: TableOptions::default(),
        };

        let sql = plugin.build_alter_table_sql(&original, &new);
        assert_eq!(sql, "-- No changes detected");
    }

    // ==================== Charset & Collation Tests ====================

    #[test]
    fn test_get_charsets() {
        let plugin = create_plugin();
        let charsets = plugin.get_charsets();

        assert!(!charsets.is_empty());
        assert!(charsets.iter().any(|c| c.name == "utf8mb4"));
        assert!(charsets.iter().any(|c| c.name == "latin1"));
        assert!(charsets.iter().any(|c| c.name == "gbk"));
    }

    #[test]
    fn test_get_collations_utf8mb4() {
        let plugin = create_plugin();
        let collations = plugin.get_collations("utf8mb4");

        assert!(!collations.is_empty());
        assert!(collations.iter().any(|c| c.name == "utf8mb4_general_ci"));
        assert!(collations.iter().any(|c| c.name == "utf8mb4_unicode_ci"));
        assert!(collations.iter().any(|c| c.name == "utf8mb4_bin"));
    }

    #[test]
    fn test_get_collations_utf8mb3_uses_utf8mb3_names() {
        let plugin = create_plugin();
        let collations = plugin.get_collations("utf8mb3");

        assert!(!collations.is_empty());
        assert!(collations.iter().any(|c| c.name == "utf8mb3_general_ci"));
        assert!(collations.iter().all(|c| c.charset == "utf8mb3"));
    }

    #[test]
    fn test_get_collations_latin1() {
        let plugin = create_plugin();
        let collations = plugin.get_collations("latin1");

        assert!(!collations.is_empty());
        assert!(collations.iter().any(|c| c.name == "latin1_swedish_ci"));
    }

    #[test]
    fn test_get_collations_unknown() {
        let plugin = create_plugin();
        let collations = plugin.get_collations("unknown_charset");
        assert!(collations.is_empty());
    }

    #[test]
    fn test_mysql_manifest_exposes_engine_list() {
        let plugin = create_plugin();
        let manifest = plugin.ui_manifest();

        assert_eq!(
            manifest.capabilities.table_engines,
            vec![
                "InnoDB".to_string(),
                "MyISAM".to_string(),
                "MEMORY".to_string(),
                "CSV".to_string(),
                "ARCHIVE".to_string(),
                "BLACKHOLE".to_string(),
                "FEDERATED".to_string(),
            ]
        );
    }

    #[test]
    fn test_mysql_manifest_contains_expected_forms_and_tabs() {
        let plugin = create_plugin();
        let manifest = plugin.ui_manifest();

        assert_eq!(manifest.forms.len(), 7);
        assert_eq!(
            manifest
                .forms
                .iter()
                .map(|form| form.kind)
                .collect::<Vec<_>>(),
            vec![
                DatabaseFormKind::Connection,
                DatabaseFormKind::CreateDatabase,
                DatabaseFormKind::EditDatabase,
                DatabaseFormKind::CreateUser,
                DatabaseFormKind::EditUser,
                DatabaseFormKind::DeleteUser,
                DatabaseFormKind::UserPrivileges,
            ]
        );

        let connection_form = manifest
            .forms
            .iter()
            .find(|form| form.kind == DatabaseFormKind::Connection)
            .unwrap();
        assert_eq!(
            connection_form
                .tabs
                .iter()
                .map(|tab| tab.id.as_str())
                .collect::<Vec<_>>(),
            vec!["general", "advanced", "ssl", "ssh", "notes"]
        );

        let general_tab = connection_form
            .tabs
            .iter()
            .find(|tab| tab.id == "general")
            .unwrap();
        let name_field = general_tab
            .fields
            .iter()
            .find(|field| field.id == "name")
            .unwrap();
        let database_field = general_tab
            .fields
            .iter()
            .find(|field| field.id == "database")
            .unwrap();
        assert_eq!(name_field.default_value.as_deref(), Some("Local MySQL"));
        assert_eq!(database_field.default_value, None);

        let ssh_host = connection_form
            .tabs
            .iter()
            .find(|tab| tab.id == "ssh")
            .and_then(|tab| tab.fields.iter().find(|field| field.id == "ssh_host"))
            .unwrap();
        assert_eq!(ssh_host.visible_when.len(), 1);
        assert_eq!(ssh_host.visible_when[0].when_field, "ssh_tunnel_enabled");
        assert_eq!(
            ssh_host.visible_when[0].condition,
            FormValueCondition::Equals("true".into())
        );
    }

    #[test]
    fn test_mysql_database_form_uses_reference_data_sources() {
        let plugin = create_plugin();
        let manifest = plugin.ui_manifest();
        let create_form = manifest
            .forms
            .iter()
            .find(|form| form.kind == DatabaseFormKind::CreateDatabase)
            .unwrap();

        let charset = create_form
            .tabs
            .iter()
            .flat_map(|tab| tab.fields.iter())
            .find(|field| field.id == "charset")
            .unwrap();
        assert_eq!(
            charset.options_source,
            Some(ReferenceDataKind::MySqlCharsets)
        );

        let collation = create_form
            .tabs
            .iter()
            .flat_map(|tab| tab.fields.iter())
            .find(|field| field.id == "collation")
            .unwrap();
        assert_eq!(
            collation.options_source,
            Some(ReferenceDataKind::MySqlCollations)
        );
        assert_eq!(collation.default_when.len(), 1);
        assert_eq!(collation.default_when[0].when_field_changes, "charset");
        assert_eq!(
            collation.default_when[0].via,
            ReferenceDataKind::MySqlCollations
        );
    }

    #[test]
    fn test_mysql_manifest_declares_context_menu_and_toolbar_actions() {
        let plugin = create_plugin();
        let manifest = plugin.ui_manifest();
        let ids = manifest
            .actions
            .actions
            .iter()
            .map(|action| action.id)
            .collect::<Vec<_>>();

        for action_id in [
            DatabaseActionId::RunSqlFile,
            DatabaseActionId::CloseConnection,
            DatabaseActionId::DeleteConnection,
            DatabaseActionId::CreateDatabase,
            DatabaseActionId::EditDatabase,
            DatabaseActionId::DeleteDatabase,
            DatabaseActionId::OpenTableData,
            DatabaseActionId::DesignTable,
            DatabaseActionId::RenameTable,
            DatabaseActionId::CopyTable,
            DatabaseActionId::TruncateTable,
            DatabaseActionId::DeleteTable,
            DatabaseActionId::ImportData,
            DatabaseActionId::ExportData,
            DatabaseActionId::OpenViewData,
            DatabaseActionId::DeleteView,
            DatabaseActionId::CreateNewQuery,
            DatabaseActionId::OpenNamedQuery,
            DatabaseActionId::RenameQuery,
            DatabaseActionId::DeleteQuery,
            DatabaseActionId::DumpSqlStructure,
            DatabaseActionId::DumpSqlData,
            DatabaseActionId::DumpSqlStructureAndData,
        ] {
            assert!(
                ids.contains(&action_id),
                "missing MySQL action descriptor: {:?}",
                action_id
            );
        }
    }

    // ==================== Data Types Tests ====================

    #[test]
    fn test_get_data_types() {
        let plugin = create_plugin();
        let types = plugin.get_data_types();

        assert!(!types.is_empty());
        assert!(types.iter().any(|t| t.0 == "INT"));
        assert!(types.iter().any(|t| t.0 == "VARCHAR"));
        assert!(types.iter().any(|t| t.0 == "TEXT"));
        assert!(types.iter().any(|t| t.0 == "DATETIME"));
        assert!(types.iter().any(|t| t.0 == "JSON"));
    }

    // ==================== Completion Info Tests ====================

    #[test]
    fn test_get_completion_info() {
        let plugin = create_plugin();
        let info = plugin.get_completion_info();

        assert!(!info.keywords.is_empty());
        assert!(!info.functions.is_empty());
        assert!(!info.operators.is_empty());
        assert!(!info.data_types.is_empty());
        assert!(!info.snippets.is_empty());

        assert!(info.keywords.iter().any(|(k, _)| *k == "AUTO_INCREMENT"));
        assert!(
            info.functions
                .iter()
                .any(|(f, _)| f.starts_with("GROUP_CONCAT"))
        );
        assert!(info.operators.iter().any(|(o, _)| *o == "REGEXP"));
    }
}
