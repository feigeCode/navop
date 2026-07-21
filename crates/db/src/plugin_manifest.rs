use crate::DbNodeType;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

pub const DATABASE_UI_MANIFEST_VERSION: u32 = 1;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DatabaseUiManifest {
    pub schema_version: u32,
    #[serde(default)]
    pub capabilities: DatabaseUiCapabilities,
    pub forms: Vec<DatabaseFormManifest>,
    #[serde(default)]
    pub actions: DatabaseActionManifest,
}

impl Default for DatabaseUiManifest {
    fn default() -> Self {
        Self {
            schema_version: DATABASE_UI_MANIFEST_VERSION,
            capabilities: DatabaseUiCapabilities::default(),
            forms: Vec::new(),
            actions: DatabaseActionManifest::default(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct DatabaseUiCapabilities {
    pub supports_schema: bool,
    pub uses_schema_as_database: bool,
    pub supports_views: bool,
    pub supports_indexes: bool,
    pub supports_users: bool,
    pub supports_user_create: bool,
    pub supports_user_edit: bool,
    pub supports_user_delete: bool,
    pub supports_user_privileges: bool,
    pub supports_sequences: bool,
    pub supports_functions: bool,
    pub supports_procedures: bool,
    pub supports_triggers: bool,
    pub supports_table_engine: bool,
    pub supports_table_charset: bool,
    pub supports_table_collation: bool,
    pub supports_auto_increment: bool,
    pub supports_tablespace: bool,
    pub supports_unsigned: bool,
    pub supports_enum_values: bool,
    pub show_charset_in_column_detail: bool,
    pub show_collation_in_column_detail: bool,
    pub table_engines: Vec<String>,
}

impl Default for DatabaseUiCapabilities {
    fn default() -> Self {
        Self {
            supports_schema: false,
            uses_schema_as_database: false,
            supports_views: true,
            supports_indexes: true,
            supports_users: false,
            supports_user_create: false,
            supports_user_edit: false,
            supports_user_delete: false,
            supports_user_privileges: false,
            supports_sequences: false,
            supports_functions: false,
            supports_procedures: false,
            supports_triggers: false,
            supports_table_engine: false,
            supports_table_charset: false,
            supports_table_collation: false,
            supports_auto_increment: false,
            supports_tablespace: false,
            supports_unsigned: false,
            supports_enum_values: false,
            show_charset_in_column_detail: false,
            show_collation_in_column_detail: false,
            table_engines: Vec::new(),
        }
    }
}

pub type DatabaseCapabilities = DatabaseUiCapabilities;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum DatabaseFormKind {
    Connection,
    CreateDatabase,
    EditDatabase,
    CreateSchema,
    CreateUser,
    EditUser,
    DeleteUser,
    UserPrivileges,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DatabaseFormManifest {
    pub kind: DatabaseFormKind,
    pub title_i18n_key: String,
    pub submit_i18n_key: String,
    pub tabs: Vec<DatabaseFormTab>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DatabaseFormTab {
    pub id: String,
    pub label_i18n_key: String,
    pub fields: Vec<DatabaseFormField>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DatabaseFormField {
    pub id: String,
    pub label_i18n_key: String,
    pub field_type: DatabaseFormFieldType,
    pub required: bool,
    pub default_value: Option<String>,
    pub placeholder_i18n_key: Option<String>,
    pub help_i18n_key: Option<String>,
    pub options: Vec<FormSelectOption>,
    pub options_source: Option<ReferenceDataKind>,
    pub visible_when: Vec<FormVisibilityRule>,
    pub default_when: Vec<FormDefaultRule>,
    pub disabled_when_editing: bool,
    pub rows: Option<u32>,
    pub min: Option<i64>,
    pub max: Option<i64>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum DatabaseFormFieldType {
    Text,
    Number,
    Password,
    TextArea,
    Select,
    Checkbox,
    FilePath,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FormSelectOption {
    pub value: String,
    pub label_i18n_key: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FormVisibilityRule {
    pub when_field: String,
    pub condition: FormValueCondition,
}

impl FormVisibilityRule {
    pub fn matches(&self, state: &HashMap<String, String>) -> bool {
        self.condition
            .matches(state.get(&self.when_field).map(String::as_str))
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum FormValueCondition {
    Equals(String),
    NotEquals(String),
    In(Vec<String>),
    NotEmpty,
}

impl FormValueCondition {
    pub fn matches(&self, value: Option<&str>) -> bool {
        match self {
            Self::Equals(expected) => value == Some(expected.as_str()),
            Self::NotEquals(expected) => value != Some(expected.as_str()),
            Self::In(candidates) => value
                .map(|current| candidates.iter().any(|candidate| candidate == current))
                .unwrap_or(false),
            Self::NotEmpty => value.map(|current| !current.is_empty()).unwrap_or(false),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FormDefaultRule {
    pub when_field_changes: String,
    pub via: ReferenceDataKind,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ReferenceDataKind {
    MySqlCharsets,
    MySqlCollations,
    MsSqlCharsets,
    MsSqlCollations,
    TableEngines,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct DatabaseActionManifest {
    pub actions: Vec<DatabaseActionDescriptor>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DatabaseActionDescriptor {
    pub id: DatabaseActionId,
    pub label_i18n_key: String,
    pub icon: Option<String>,
    pub targets: Vec<DatabaseActionTarget>,
    pub placement: DatabaseActionPlacement,
    pub requires_active_connection: bool,
    pub group: Option<String>,
    pub submenu_of: Option<DatabaseActionId>,
    pub toolbar_scope: Option<DatabaseActionToolbarScope>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum DatabaseActionId {
    CloseConnection,
    DeleteConnection,
    CreateDatabase,
    EditDatabase,
    CloseDatabase,
    DeleteDatabase,
    CreateSchema,
    DeleteSchema,
    OpenTableData,
    DesignTable,
    RenameTable,
    CopyTable,
    TruncateTable,
    DeleteTable,
    OpenViewData,
    DeleteView,
    CreateNewQuery,
    OpenNamedQuery,
    RenameQuery,
    DeleteQuery,
    RevealQueryInFileManager,
    RunSqlFile,
    ImportData,
    ExportData,
    DumpSqlStructure,
    DumpSqlData,
    DumpSqlStructureAndData,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DatabaseActionTarget {
    pub node_type: DbNodeType,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum DatabaseActionPlacement {
    ContextMenu,
    Toolbar,
    Both,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum DatabaseActionToolbarScope {
    CurrentNode,
    SelectedRow,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DatabaseFormSubmission {
    pub kind: DatabaseFormKind,
    pub field_values: HashMap<String, String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_user_capabilities_are_disabled() {
        let capabilities = DatabaseUiCapabilities::default();

        assert!(!capabilities.supports_users);
        assert!(!capabilities.supports_user_create);
        assert!(!capabilities.supports_user_edit);
        assert!(!capabilities.supports_user_delete);
        assert!(!capabilities.supports_user_privileges);
    }

    #[test]
    fn manifest_can_describe_user_operation_forms() {
        let kinds = [
            DatabaseFormKind::CreateUser,
            DatabaseFormKind::EditUser,
            DatabaseFormKind::DeleteUser,
            DatabaseFormKind::UserPrivileges,
        ];

        assert_eq!(4, kinds.len());
    }
}
