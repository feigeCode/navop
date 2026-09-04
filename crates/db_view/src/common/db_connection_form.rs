use anyhow::Error;
use std::collections::HashMap;
use std::time::Instant;

use connection_form::team::{
    TeamSelectItem, connection_sync_controls_visible_in, create_team_select, refresh_team_options,
    refresh_teams_tooltip, replace_team_options, resolve_team_assignment, selected_team_id,
    team_label, team_management_enabled,
};
use connection_form::{
    SshAuthOption, SshConnectionSelectItem,
    credential::{
        CredentialCapabilities, CredentialPickerConfig, CredentialPickerEvent,
        CredentialReferencePicker, create_credential_picker, resolve_connection_for_runtime,
    },
    normalize_ssh_auth_type as normalized_ssh_auth_type,
};
use db::plugin_manifest::FormVisibilityRule;
use db::{
    DEFAULT_SCHEMA_PARAM, GlobalDbState, ORACLE_ROLE_PARAM, SCHEMA_FILTER_EXCLUDE_PARAM,
    SCHEMA_FILTER_INCLUDE_PARAM, SCHEMA_FILTER_MODE_PARAM, oracle,
};
use gpui::prelude::FluentBuilder;
use gpui::{
    App, AsyncApp, Axis, Context, Entity, EventEmitter, FocusHandle, Focusable, IntoElement,
    ParentElement, PathPromptOptions, Render, SharedString, Styled, Subscription, Window, div,
    prelude::*, px,
};
use gpui_component::{
    ActiveTheme, Disableable, Icon, IconName, IndexPath, Sizable, Size,
    button::{Button, ButtonVariants as _},
    checkbox::Checkbox,
    clipboard::Clipboard,
    form::{field, v_form},
    h_flex,
    input::{Input, InputEvent, InputState},
    popover::Popover,
    radio::Radio,
    scroll::ScrollableElement,
    select::{SearchableVec, Select, SelectEvent, SelectItem, SelectState},
    tab::{Tab, TabBar},
    v_flex,
};
use one_core::cloud_sync::TeamOption;
use one_core::gpui_tokio::Tokio;
use one_core::storage::traits::Repository;
use one_core::storage::{
    ConnectionRepository, ConnectionType, DatabaseType, DbConnectionConfig, GlobalStorageState,
    ProxyConfig, ProxyType, StoredConnection, Workspace, get_config_dir,
};
use rust_i18n::t;
use tracing::info;

use super::connection_proxy::{self, ProxyValidationError};

const ORACLE_GO_DRIVER_ID: &str = "oracle-go";

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum OracleDriverMode {
    #[default]
    Native,
    Go,
}

fn database_type_for_oracle_driver_mode(
    current: &DatabaseType,
    mode: OracleDriverMode,
) -> DatabaseType {
    match (current, mode) {
        (DatabaseType::Oracle, OracleDriverMode::Go) => DatabaseType::external(ORACLE_GO_DRIVER_ID),
        (DatabaseType::Oracle, OracleDriverMode::Native) => DatabaseType::Oracle,
        (DatabaseType::External { driver_id }, OracleDriverMode::Native)
            if driver_id == ORACLE_GO_DRIVER_ID =>
        {
            DatabaseType::Oracle
        }
        (DatabaseType::External { driver_id }, OracleDriverMode::Go)
            if driver_id == ORACLE_GO_DRIVER_ID =>
        {
            current.clone()
        }
        _ => current.clone(),
    }
}

fn oracle_driver_mode_for_database_type(database_type: &DatabaseType) -> OracleDriverMode {
    match database_type {
        DatabaseType::External { driver_id } if driver_id == ORACLE_GO_DRIVER_ID => {
            OracleDriverMode::Go
        }
        _ => OracleDriverMode::Native,
    }
}

/// Form select item for dropdown fields
#[derive(Clone, Debug)]
pub struct FormSelectItem {
    pub value: String,
    pub label: String,
}

impl FormSelectItem {
    pub fn new(value: impl Into<String>, label: impl Into<String>) -> Self {
        Self {
            value: value.into(),
            label: label.into(),
        }
    }
}

impl SelectItem for FormSelectItem {
    type Value = String;

    fn title(&self) -> SharedString {
        self.label.clone().into()
    }

    fn value(&self) -> &Self::Value {
        &self.value
    }
}

/// Workspace select item for dropdown
#[derive(Clone, Debug)]
pub struct WorkspaceSelectItem {
    pub id: Option<i64>,
    pub name: String,
}

impl WorkspaceSelectItem {
    pub fn none() -> Self {
        Self {
            id: None,
            name: t!("Common.none").to_string(),
        }
    }

    pub fn from_workspace(ws: &Workspace) -> Self {
        Self {
            id: ws.id,
            name: ws.name.clone(),
        }
    }
}

impl SelectItem for WorkspaceSelectItem {
    type Value = Option<i64>;

    fn title(&self) -> SharedString {
        self.name.clone().into()
    }

    fn value(&self) -> &Self::Value {
        &self.id
    }
}

/// Represents a tab group containing multiple fields
#[derive(Clone, Debug)]
pub struct TabGroup {
    pub name: String,
    pub label: String,
    pub fields: Vec<FormField>,
}

impl TabGroup {
    pub fn new(name: impl Into<String>, label: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            label: label.into(),
            fields: Vec::new(),
        }
    }

    pub fn field(mut self, field: FormField) -> Self {
        self.fields.push(field);
        self
    }

    pub fn fields(mut self, fields: Vec<FormField>) -> Self {
        self.fields = fields;
        self
    }
}

/// Represents a field in the connection form
#[derive(Clone, Debug)]
pub struct FormField {
    pub name: String,
    pub label: String,
    pub placeholder: String,
    pub field_type: FormFieldType,
    pub rows: usize,
    pub required: bool,
    pub default_value: String,
    pub options: Vec<(String, String)>,
    pub visible_when: Vec<FormVisibilityRule>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum FormFieldType {
    Text,
    Number,
    Password,
    TextArea,
    Select,
    Checkbox,
    FilePath,
}

impl FormField {
    pub fn new(
        name: impl Into<String>,
        label: impl Into<String>,
        field_type: FormFieldType,
    ) -> Self {
        let name = name.into();
        Self {
            placeholder: format!("Enter {}", name.to_lowercase()),
            name,
            label: label.into(),
            field_type,
            rows: 5,
            required: true,
            default_value: String::new(),
            options: Vec::new(),
            visible_when: Vec::new(),
        }
    }

    pub fn optional(mut self) -> Self {
        self.required = false;
        self
    }

    pub fn placeholder(mut self, placeholder: impl Into<String>) -> Self {
        self.placeholder = placeholder.into();
        self
    }

    pub fn default(mut self, value: impl Into<String>) -> Self {
        self.default_value = value.into();
        self
    }

    pub fn options(mut self, options: Vec<(String, String)>) -> Self {
        self.options = options;
        self
    }
    pub fn rows(mut self, rows: usize) -> Self {
        self.rows = rows;
        self
    }
}

/// Database connection form configuration for different database types
pub struct DbFormConfig {
    pub db_type: DatabaseType,
    pub title: String,
    pub tab_groups: Vec<TabGroup>,
    pub hidden_params: HashMap<String, String>,
}

impl DbFormConfig {
    fn schema_preference_fields() -> Vec<FormField> {
        vec![
            FormField::new(
                DEFAULT_SCHEMA_PARAM,
                t!("ConnectionForm.default_schema"),
                FormFieldType::Text,
            )
            .optional()
            .placeholder(t!("ConnectionForm.default_schema_placeholder")),
            FormField::new(
                SCHEMA_FILTER_MODE_PARAM,
                t!("ConnectionForm.schema_filter_mode"),
                FormFieldType::Select,
            )
            .optional()
            .default("auto")
            .options(vec![
                (
                    "auto".to_string(),
                    t!("ConnectionForm.schema_filter_mode_auto").to_string(),
                ),
                (
                    "include".to_string(),
                    t!("ConnectionForm.schema_filter_mode_include").to_string(),
                ),
                (
                    "exclude".to_string(),
                    t!("ConnectionForm.schema_filter_mode_exclude").to_string(),
                ),
                (
                    "all".to_string(),
                    t!("ConnectionForm.schema_filter_mode_all").to_string(),
                ),
            ]),
            FormField::new(
                SCHEMA_FILTER_INCLUDE_PARAM,
                t!("ConnectionForm.schema_filter_include"),
                FormFieldType::Text,
            )
            .optional()
            .placeholder(t!("ConnectionForm.schema_filter_include_placeholder")),
            FormField::new(
                SCHEMA_FILTER_EXCLUDE_PARAM,
                t!("ConnectionForm.schema_filter_exclude"),
                FormFieldType::Text,
            )
            .optional()
            .placeholder(t!("ConnectionForm.schema_filter_exclude_placeholder")),
        ]
    }

    fn with_schema_preference_fields(mut fields: Vec<FormField>) -> Vec<FormField> {
        fields.extend(Self::schema_preference_fields());
        fields
    }

    fn ssh_tab_group() -> TabGroup {
        TabGroup::new("ssh", t!("ConnectionForm.ssh")).fields(vec![
            FormField::new(
                "ssh_tunnel_enabled",
                t!("ConnectionForm.ssh_tunnel_enabled"),
                FormFieldType::Select,
            )
            .optional()
            .default("false")
            .options(vec![
                ("false".to_string(), t!("Common.no").to_string()),
                ("true".to_string(), t!("Common.yes").to_string()),
            ]),
            FormField::new(
                "ssh_connection_id",
                t!("ConnectionForm.ssh_connection_id"),
                FormFieldType::Text,
            )
            .optional(),
            FormField::new(
                "ssh_host",
                t!("ConnectionForm.ssh_host"),
                FormFieldType::Text,
            )
            .optional()
            .placeholder("jump.example.com"),
            FormField::new(
                "ssh_port",
                t!("ConnectionForm.ssh_port"),
                FormFieldType::Number,
            )
            .optional()
            .default("22")
            .placeholder("22"),
            FormField::new(
                "ssh_username",
                t!("ConnectionForm.ssh_username"),
                FormFieldType::Text,
            )
            .optional()
            .placeholder("root"),
            FormField::new(
                "ssh_auth_type",
                t!("ConnectionForm.ssh_auth_type"),
                FormFieldType::Select,
            )
            .optional()
            .default("password")
            .options(
                SshAuthOption::ALL
                    .iter()
                    .map(|option| (option.value().to_string(), option.label()))
                    .collect(),
            ),
            FormField::new(
                "ssh_password",
                t!("ConnectionForm.ssh_password"),
                FormFieldType::Password,
            )
            .optional()
            .placeholder(t!("ConnectionForm.enter_ssh_password").to_string()),
            FormField::new(
                "ssh_private_key_path",
                t!("ConnectionForm.ssh_private_key_path"),
                FormFieldType::Text,
            )
            .optional()
            .placeholder("~/.ssh/id_rsa"),
            FormField::new(
                "ssh_private_key_content",
                t!("ConnectionForm.ssh_private_key_content"),
                FormFieldType::TextArea,
            )
            .rows(5)
            .optional()
            .placeholder(t!("ConnectionForm.ssh_private_key_content_placeholder")),
            FormField::new(
                "ssh_private_key_passphrase",
                t!("ConnectionForm.ssh_private_key_passphrase"),
                FormFieldType::Password,
            )
            .optional()
            .placeholder(t!("ConnectionForm.enter_key_passphrase").to_string()),
            FormField::new(
                "ssh_target_host",
                t!("ConnectionForm.ssh_target_host"),
                FormFieldType::Text,
            )
            .optional()
            .placeholder("127.0.0.1"),
            FormField::new(
                "ssh_target_port",
                t!("ConnectionForm.ssh_target_port"),
                FormFieldType::Number,
            )
            .optional()
            .placeholder("3306"),
        ])
    }

    fn mysql_ssl_tab_group() -> TabGroup {
        TabGroup::new("ssl", t!("ConnectionForm.ssl")).fields(vec![
            FormField::new(
                "require_ssl",
                t!("ConnectionForm.require_ssl"),
                FormFieldType::Select,
            )
            .optional()
            .default("false")
            .options(vec![
                ("false".to_string(), t!("Common.no").to_string()),
                ("true".to_string(), t!("Common.yes").to_string()),
            ]),
            FormField::new(
                "verify_ca",
                t!("ConnectionForm.verify_ca"),
                FormFieldType::Select,
            )
            .optional()
            .default("true")
            .options(vec![
                ("true".to_string(), t!("Common.yes").to_string()),
                ("false".to_string(), t!("Common.no").to_string()),
            ]),
            FormField::new(
                "verify_identity",
                t!("ConnectionForm.verify_identity"),
                FormFieldType::Select,
            )
            .optional()
            .default("true")
            .options(vec![
                ("true".to_string(), t!("Common.yes").to_string()),
                ("false".to_string(), t!("Common.no").to_string()),
            ]),
            FormField::new(
                "ssl_root_cert_path",
                t!("ConnectionForm.ssl_root_cert_path"),
                FormFieldType::Text,
            )
            .optional()
            .placeholder(t!("ConnectionForm.ssl_root_cert_path_placeholder")),
            FormField::new(
                "tls_hostname_override",
                t!("ConnectionForm.tls_hostname_override"),
                FormFieldType::Text,
            )
            .optional()
            .placeholder(t!("ConnectionForm.tls_hostname_override_placeholder")),
        ])
    }

    fn postgres_ssl_tab_group() -> TabGroup {
        TabGroup::new("ssl", t!("ConnectionForm.ssl")).fields(vec![
            FormField::new(
                "ssl_mode",
                t!("ConnectionForm.ssl_mode"),
                FormFieldType::Select,
            )
            .optional()
            .default("prefer")
            .options(vec![
                (
                    "disable".to_string(),
                    t!("ConnectionForm.ssl_mode_disable").to_string(),
                ),
                (
                    "prefer".to_string(),
                    t!("ConnectionForm.ssl_mode_prefer").to_string(),
                ),
                (
                    "require".to_string(),
                    t!("ConnectionForm.ssl_mode_require").to_string(),
                ),
            ]),
            FormField::new(
                "ssl_root_cert_path",
                t!("ConnectionForm.ssl_root_cert_path"),
                FormFieldType::Text,
            )
            .optional()
            .placeholder(t!("ConnectionForm.ssl_root_cert_path_placeholder")),
            FormField::new(
                "ssl_accept_invalid_certs",
                t!("ConnectionForm.ssl_accept_invalid_certs"),
                FormFieldType::Select,
            )
            .optional()
            .default("false")
            .options(vec![
                ("false".to_string(), t!("Common.no").to_string()),
                ("true".to_string(), t!("Common.yes").to_string()),
            ]),
            FormField::new(
                "ssl_accept_invalid_hostnames",
                t!("ConnectionForm.ssl_accept_invalid_hostnames"),
                FormFieldType::Select,
            )
            .optional()
            .default("false")
            .options(vec![
                ("false".to_string(), t!("Common.no").to_string()),
                ("true".to_string(), t!("Common.yes").to_string()),
            ]),
        ])
    }

    fn mssql_ssl_tab_group() -> TabGroup {
        TabGroup::new("ssl", t!("ConnectionForm.ssl")).fields(vec![
            FormField::new(
                "encrypt",
                t!("ConnectionForm.encrypt"),
                FormFieldType::Select,
            )
            .optional()
            .default("off")
            .options(vec![
                (
                    "off".to_string(),
                    t!("ConnectionForm.encrypt_off").to_string(),
                ),
                (
                    "on".to_string(),
                    t!("ConnectionForm.encrypt_on").to_string(),
                ),
                (
                    "required".to_string(),
                    t!("ConnectionForm.encrypt_strict").to_string(),
                ),
            ]),
            FormField::new(
                "trust_cert",
                t!("ConnectionForm.trust_certificate"),
                FormFieldType::Select,
            )
            .optional()
            .default("true")
            .options(vec![
                ("true".to_string(), t!("Common.yes").to_string()),
                ("false".to_string(), t!("Common.no").to_string()),
            ]),
        ])
    }

    fn clickhouse_ssl_tab_group() -> TabGroup {
        TabGroup::new("ssl", t!("ConnectionForm.ssl")).fields(vec![
            FormField::new("schema", t!("ConnectionForm.schema"), FormFieldType::Select)
                .optional()
                .default("http")
                .options(vec![
                    (
                        "http".to_string(),
                        t!("ConnectionForm.schema_http").to_string(),
                    ),
                    (
                        "https".to_string(),
                        t!("ConnectionForm.schema_https").to_string(),
                    ),
                ]),
        ])
    }

    /// MySQL form configuration
    pub fn mysql() -> Self {
        Self {
            db_type: DatabaseType::MySQL,
            title: format!("{} (MySQL)", t!("Common.new")),
            hidden_params: HashMap::new(),
            tab_groups: vec![
                TabGroup::new("general", t!("ConnectionForm.general")).fields(vec![
                    FormField::new(
                        "name",
                        t!("ConnectionForm.connection_name"),
                        FormFieldType::Text,
                    )
                    .placeholder(
                        t!("ConnectionForm.connection_name_placeholder", kind = "MySQL")
                            .to_string(),
                    )
                    .default("Local MySQL"),
                    FormField::new("host", t!("ConnectionForm.host"), FormFieldType::Text)
                        .placeholder("localhost")
                        .default("localhost"),
                    FormField::new("port", t!("ConnectionForm.port"), FormFieldType::Number)
                        .placeholder("3306")
                        .default("3306"),
                    FormField::new(
                        "username",
                        t!("ConnectionForm.username"),
                        FormFieldType::Text,
                    )
                    .placeholder("root")
                    .default("root"),
                    FormField::new(
                        "password",
                        t!("ConnectionForm.password"),
                        FormFieldType::Password,
                    )
                    .placeholder(t!("ConnectionForm.enter_password").to_string()),
                    FormField::new(
                        "database",
                        t!("ConnectionForm.database"),
                        FormFieldType::Text,
                    )
                    .optional()
                    .placeholder(t!("ConnectionForm.database_optional").to_string()),
                ]),
                TabGroup::new("advanced", t!("ConnectionForm.advanced")).fields(vec![
                    FormField::new(
                        "connect_timeout",
                        t!("ConnectionForm.connect_timeout"),
                        FormFieldType::Number,
                    )
                    .optional()
                    .placeholder("30")
                    .default("30"),
                    FormField::new("charset", t!("ConnectionForm.charset"), FormFieldType::Text)
                        .optional()
                        .placeholder("gbk"),
                    FormField::new(
                        "collation",
                        t!("ConnectionForm.collation"),
                        FormFieldType::Text,
                    )
                    .optional()
                    .placeholder("gbk_chinese_ci"),
                    FormField::new(
                        "read_timeout",
                        t!("ConnectionForm.read_timeout"),
                        FormFieldType::Number,
                    )
                    .optional()
                    .placeholder("28800"),
                ]),
                Self::mysql_ssl_tab_group(),
                Self::ssh_tab_group(),
                TabGroup::new("notes", t!("ConnectionForm.notes")).fields(vec![
                    FormField::new(
                        "remark",
                        t!("ConnectionForm.remark"),
                        FormFieldType::TextArea,
                    )
                    .rows(14)
                    .optional()
                    .placeholder(t!("ConnectionForm.enter_remark"))
                    .default(""),
                ]),
            ],
        }
    }

    /// PostgreSQL form configuration
    pub fn postgres() -> Self {
        Self {
            db_type: DatabaseType::PostgreSQL,
            title: format!("{} (PostgreSQL)", t!("Common.new")),
            hidden_params: HashMap::new(),
            tab_groups: vec![
                TabGroup::new("general", t!("ConnectionForm.general")).fields(vec![
                    FormField::new(
                        "name",
                        t!("ConnectionForm.connection_name"),
                        FormFieldType::Text,
                    )
                    .placeholder(
                        t!(
                            "ConnectionForm.connection_name_placeholder",
                            kind = "PostgreSQL"
                        )
                        .to_string(),
                    )
                    .default("Local PostgreSQL"),
                    FormField::new("host", t!("ConnectionForm.host"), FormFieldType::Text)
                        .placeholder("localhost")
                        .default("localhost"),
                    FormField::new("port", t!("ConnectionForm.port"), FormFieldType::Number)
                        .placeholder("5432")
                        .default("5432"),
                    FormField::new(
                        "username",
                        t!("ConnectionForm.username"),
                        FormFieldType::Text,
                    )
                    .placeholder("postgres")
                    .default("postgres"),
                    FormField::new(
                        "password",
                        t!("ConnectionForm.password"),
                        FormFieldType::Password,
                    )
                    .placeholder(t!("ConnectionForm.enter_password").to_string()),
                    FormField::new(
                        "database",
                        t!("ConnectionForm.database"),
                        FormFieldType::Text,
                    )
                    .optional()
                    .placeholder(t!("ConnectionForm.database_optional").to_string()),
                ]),
                TabGroup::new("advanced", t!("ConnectionForm.advanced")).fields(
                    Self::with_schema_preference_fields(vec![
                        FormField::new(
                            "connect_timeout",
                            t!("ConnectionForm.connect_timeout"),
                            FormFieldType::Number,
                        )
                        .optional()
                        .placeholder("30")
                        .default("30"),
                        FormField::new(
                            "application_name",
                            t!("ConnectionForm.application_name"),
                            FormFieldType::Text,
                        )
                        .optional()
                        .placeholder(t!("ConnectionForm.application_name").to_string()),
                    ]),
                ),
                Self::postgres_ssl_tab_group(),
                Self::ssh_tab_group(),
                TabGroup::new("notes", t!("ConnectionForm.notes")).fields(vec![
                    FormField::new(
                        "remark",
                        t!("ConnectionForm.remark"),
                        FormFieldType::TextArea,
                    )
                    .rows(14)
                    .optional()
                    .placeholder(t!("ConnectionForm.enter_remark"))
                    .default(""),
                ]),
            ],
        }
    }

    /// MSSQL (SQL Server) form configuration
    pub fn mssql() -> Self {
        Self {
            db_type: DatabaseType::MSSQL,
            title: format!("{} (SQL Server)", t!("Common.new")),
            hidden_params: HashMap::new(),
            tab_groups: vec![
                TabGroup::new("general", t!("ConnectionForm.general")).fields(vec![
                    FormField::new(
                        "name",
                        t!("ConnectionForm.connection_name"),
                        FormFieldType::Text,
                    )
                    .placeholder(
                        t!(
                            "ConnectionForm.connection_name_placeholder",
                            kind = "SQL Server"
                        )
                        .to_string(),
                    )
                    .default("Local SQL Server"),
                    FormField::new("host", t!("ConnectionForm.host"), FormFieldType::Text)
                        .placeholder("localhost")
                        .default("localhost"),
                    FormField::new("port", t!("ConnectionForm.port"), FormFieldType::Number)
                        .placeholder("1433")
                        .default("1433"),
                    FormField::new(
                        "username",
                        t!("ConnectionForm.username"),
                        FormFieldType::Text,
                    )
                    .placeholder("sa")
                    .default("sa"),
                    FormField::new(
                        "password",
                        t!("ConnectionForm.password"),
                        FormFieldType::Password,
                    )
                    .placeholder(t!("ConnectionForm.enter_password").to_string()),
                    FormField::new(
                        "database",
                        t!("ConnectionForm.database"),
                        FormFieldType::Text,
                    )
                    .optional()
                    .placeholder(t!("ConnectionForm.database_optional").to_string()),
                ]),
                TabGroup::new("advanced", t!("ConnectionForm.advanced")).fields(
                    Self::with_schema_preference_fields(vec![
                        FormField::new(
                            "connect_timeout",
                            t!("ConnectionForm.connect_timeout"),
                            FormFieldType::Number,
                        )
                        .optional()
                        .placeholder("30")
                        .default("30"),
                        FormField::new(
                            "application_name",
                            t!("ConnectionForm.application_name"),
                            FormFieldType::Text,
                        )
                        .optional()
                        .placeholder(t!("ConnectionForm.application_name").to_string()),
                    ]),
                ),
                Self::mssql_ssl_tab_group(),
                Self::ssh_tab_group(),
                TabGroup::new("notes", t!("ConnectionForm.notes")).fields(vec![
                    FormField::new(
                        "remark",
                        t!("ConnectionForm.remark"),
                        FormFieldType::TextArea,
                    )
                    .rows(14)
                    .optional()
                    .placeholder(t!("ConnectionForm.enter_remark"))
                    .default(""),
                ]),
            ],
        }
    }

    /// Oracle form configuration
    pub fn oracle() -> Self {
        Self {
            db_type: DatabaseType::Oracle,
            title: format!("{} (Oracle)", t!("Common.new")),
            hidden_params: HashMap::new(),
            tab_groups: vec![
                TabGroup::new("general", t!("ConnectionForm.general")).fields(vec![
                    FormField::new(
                        "name",
                        t!("ConnectionForm.connection_name"),
                        FormFieldType::Text,
                    )
                    .placeholder(
                        t!(
                            "ConnectionForm.connection_name_placeholder",
                            kind = "Oracle"
                        )
                        .to_string(),
                    )
                    .default("Local Oracle"),
                    FormField::new("host", t!("ConnectionForm.host"), FormFieldType::Text)
                        .placeholder("localhost")
                        .default("localhost"),
                    FormField::new("port", t!("ConnectionForm.port"), FormFieldType::Number)
                        .placeholder("1521")
                        .default("1521"),
                    FormField::new(
                        "username",
                        t!("ConnectionForm.username"),
                        FormFieldType::Text,
                    )
                    .placeholder("system")
                    .default("system"),
                    FormField::new(
                        "password",
                        t!("ConnectionForm.password"),
                        FormFieldType::Password,
                    )
                    .placeholder(t!("ConnectionForm.enter_password").to_string()),
                    FormField::new("service_name", "Service Name", FormFieldType::Text)
                        .optional()
                        .placeholder(t!("ConnectionForm.service_name_placeholder").to_string()),
                    FormField::new("sid", "SID", FormFieldType::Text)
                        .optional()
                        .placeholder(t!("ConnectionForm.sid_placeholder").to_string()),
                    FormField::new(
                        ORACLE_ROLE_PARAM,
                        t!("ConnectionForm.oracle_role"),
                        FormFieldType::Select,
                    )
                    .optional()
                    .default("default")
                    .options(vec![
                        (
                            "default".to_string(),
                            t!("ConnectionForm.oracle_role_default").to_string(),
                        ),
                        (
                            "sysdba".to_string(),
                            t!("ConnectionForm.oracle_role_sysdba").to_string(),
                        ),
                        (
                            "sysoper".to_string(),
                            t!("ConnectionForm.oracle_role_sysoper").to_string(),
                        ),
                    ]),
                ]),
                TabGroup::new("advanced", t!("ConnectionForm.advanced")).fields(
                    Self::with_schema_preference_fields(vec![
                        FormField::new(
                            "connect_timeout",
                            t!("ConnectionForm.connect_timeout"),
                            FormFieldType::Number,
                        )
                        .optional()
                        .placeholder("30")
                        .default("30"),
                    ]),
                ),
                Self::ssh_tab_group(),
                TabGroup::new("notes", t!("ConnectionForm.notes")).fields(vec![
                    FormField::new(
                        "remark",
                        t!("ConnectionForm.remark"),
                        FormFieldType::TextArea,
                    )
                    .rows(14)
                    .optional()
                    .placeholder(t!("ConnectionForm.enter_remark"))
                    .default(""),
                ]),
            ],
        }
    }

    /// ClickHouse form configuration
    pub fn clickhouse() -> Self {
        Self {
            db_type: DatabaseType::ClickHouse,
            title: format!("{} (ClickHouse)", t!("Common.new")),
            hidden_params: HashMap::new(),
            tab_groups: vec![
                TabGroup::new("general", t!("ConnectionForm.general")).fields(vec![
                    FormField::new(
                        "name",
                        t!("ConnectionForm.connection_name"),
                        FormFieldType::Text,
                    )
                    .placeholder(
                        t!(
                            "ConnectionForm.connection_name_placeholder",
                            kind = "ClickHouse"
                        )
                        .to_string(),
                    )
                    .default("Local ClickHouse"),
                    FormField::new("host", t!("ConnectionForm.host"), FormFieldType::Text)
                        .placeholder("localhost")
                        .default("localhost"),
                    FormField::new("port", t!("ConnectionForm.port"), FormFieldType::Number)
                        .placeholder("8123 (HTTP port)")
                        .default("8123"),
                    FormField::new(
                        "username",
                        t!("ConnectionForm.username"),
                        FormFieldType::Text,
                    )
                    .placeholder("default")
                    .default("default"),
                    FormField::new(
                        "password",
                        t!("ConnectionForm.password"),
                        FormFieldType::Password,
                    )
                    .placeholder(t!("ConnectionForm.enter_password").to_string()),
                    FormField::new(
                        "database",
                        t!("ConnectionForm.database"),
                        FormFieldType::Text,
                    )
                    .optional()
                    .placeholder(t!("ConnectionForm.database_optional").to_string()),
                ]),
                TabGroup::new("advanced", t!("ConnectionForm.advanced")).fields(vec![
                    FormField::new(
                        "connect_timeout",
                        t!("ConnectionForm.connect_timeout"),
                        FormFieldType::Number,
                    )
                    .optional()
                    .placeholder("30")
                    .default("30"),
                    FormField::new(
                        "compression",
                        t!("ConnectionForm.compression"),
                        FormFieldType::Select,
                    )
                    .optional()
                    .default("lz4")
                    .options(vec![
                        ("none".to_string(), t!("Common.none").to_string()),
                        ("lz4".to_string(), "LZ4".to_string()),
                    ]),
                ]),
                Self::clickhouse_ssl_tab_group(),
                Self::ssh_tab_group(),
                TabGroup::new("notes", t!("ConnectionForm.notes")).fields(vec![
                    FormField::new(
                        "remark",
                        t!("ConnectionForm.remark"),
                        FormFieldType::TextArea,
                    )
                    .rows(14)
                    .optional()
                    .placeholder(t!("ConnectionForm.enter_remark"))
                    .default(""),
                ]),
            ],
        }
    }

    /// TDengine 连接表单配置(WebSocket 经 taosAdapter,默认端口 6041,默认用户 root)
    pub fn tdengine() -> Self {
        Self {
            db_type: DatabaseType::TDengine,
            title: format!("{} (TDengine)", t!("Common.new")),
            hidden_params: HashMap::new(),
            tab_groups: vec![
                TabGroup::new("general", t!("ConnectionForm.general")).fields(vec![
                    FormField::new(
                        "name",
                        t!("ConnectionForm.connection_name"),
                        FormFieldType::Text,
                    )
                    .placeholder(
                        t!(
                            "ConnectionForm.connection_name_placeholder",
                            kind = "TDengine"
                        )
                        .to_string(),
                    )
                    .default("Local TDengine"),
                    FormField::new("host", t!("ConnectionForm.host"), FormFieldType::Text)
                        .placeholder("localhost")
                        .default("localhost"),
                    FormField::new("port", t!("ConnectionForm.port"), FormFieldType::Number)
                        .placeholder("6041 (taosAdapter port)")
                        .default("6041"),
                    FormField::new(
                        "username",
                        t!("ConnectionForm.username"),
                        FormFieldType::Text,
                    )
                    .placeholder("root")
                    .default("root"),
                    FormField::new(
                        "password",
                        t!("ConnectionForm.password"),
                        FormFieldType::Password,
                    )
                    .placeholder("taosdata"),
                    FormField::new(
                        "database",
                        t!("ConnectionForm.database"),
                        FormFieldType::Text,
                    )
                    .optional()
                    .placeholder(t!("ConnectionForm.database_optional").to_string()),
                ]),
                TabGroup::new("advanced", t!("ConnectionForm.advanced")).fields(vec![
                    FormField::new(
                        "connect_timeout",
                        t!("ConnectionForm.connect_timeout"),
                        FormFieldType::Number,
                    )
                    .optional()
                    .placeholder("30")
                    .default("30"),
                ]),
                TabGroup::new("ssl", t!("ConnectionForm.ssl")).fields(vec![
                    FormField::new(
                        "schema",
                        t!("ConnectionForm.schema"),
                        FormFieldType::Select,
                    )
                    .optional()
                    .default("ws")
                    .options(vec![
                        ("ws".to_string(), t!("ConnectionForm.schema_ws").to_string()),
                        ("wss".to_string(), t!("ConnectionForm.schema_wss").to_string()),
                    ]),
                ]),
                Self::ssh_tab_group(),
                TabGroup::new("notes", t!("ConnectionForm.notes")).fields(vec![
                    FormField::new(
                        "remark",
                        t!("ConnectionForm.remark"),
                        FormFieldType::TextArea,
                    )
                    .rows(14)
                    .optional()
                    .placeholder(t!("ConnectionForm.enter_remark"))
                    .default(""),
                ]),
            ],
        }
    }

    /// SQLite form configuration
    pub fn sqlite() -> Self {
        let default_db_path = get_config_dir()
            .map(|p| p.join("onetcli_default.db").to_string_lossy().to_string())
            .unwrap_or_else(|_| "onetcli_default.db".to_string());

        Self {
            db_type: DatabaseType::SQLite,
            title: format!("{} (SQLite)", t!("Common.new")),
            hidden_params: HashMap::new(),
            tab_groups: vec![
                TabGroup::new("general", t!("ConnectionForm.general")).fields(vec![
                    FormField::new(
                        "name",
                        t!("ConnectionForm.connection_name"),
                        FormFieldType::Text,
                    )
                    .placeholder(
                        t!(
                            "ConnectionForm.connection_name_placeholder",
                            kind = "SQLite"
                        )
                        .to_string(),
                    )
                    .default("Local SQLite"),
                    FormField::new(
                        "host",
                        t!("ConnectionForm.database_file_path"),
                        FormFieldType::Text,
                    )
                    .placeholder("/path/to/database.db")
                    .default(default_db_path),
                ]),
                TabGroup::new("notes", t!("ConnectionForm.notes")).fields(vec![
                    FormField::new(
                        "remark",
                        t!("ConnectionForm.remark"),
                        FormFieldType::TextArea,
                    )
                    .rows(14)
                    .optional()
                    .placeholder(t!("ConnectionForm.enter_remark"))
                    .default(""),
                ]),
            ],
        }
    }

    /// DuckDB form configuration
    pub fn duckdb() -> Self {
        let default_db_path = get_config_dir()
            .map(|p| {
                p.join("onetcli_default.duckdb")
                    .to_string_lossy()
                    .to_string()
            })
            .unwrap_or_else(|_| "onetcli_default.duckdb".to_string());

        Self {
            db_type: DatabaseType::DuckDB,
            title: format!("{} (DuckDB)", t!("Common.new")),
            hidden_params: HashMap::new(),
            tab_groups: vec![
                TabGroup::new("general", t!("ConnectionForm.general")).fields(vec![
                    FormField::new(
                        "name",
                        t!("ConnectionForm.connection_name"),
                        FormFieldType::Text,
                    )
                    .placeholder(
                        t!(
                            "ConnectionForm.connection_name_placeholder",
                            kind = "DuckDB"
                        )
                        .to_string(),
                    )
                    .default("Local DuckDB"),
                    FormField::new(
                        "host",
                        t!("ConnectionForm.database_file_path"),
                        FormFieldType::Text,
                    )
                    .placeholder("/path/to/database.duckdb")
                    .default(default_db_path),
                ]),
                TabGroup::new("notes", t!("ConnectionForm.notes")).fields(vec![
                    FormField::new(
                        "remark",
                        t!("ConnectionForm.remark"),
                        FormFieldType::TextArea,
                    )
                    .rows(14)
                    .optional()
                    .placeholder(t!("ConnectionForm.enter_remark"))
                    .default(""),
                ]),
            ],
        }
    }
}

fn ssh_auth_requires_password(auth_type: &str) -> bool {
    normalized_ssh_auth_type(auth_type) == SshAuthOption::Password.value()
}

fn ssh_auth_requires_private_key(auth_type: &str) -> bool {
    normalized_ssh_auth_type(auth_type) == SshAuthOption::PrivateKey.value()
}

fn ssh_auth_requires_private_key_content(auth_type: &str) -> bool {
    normalized_ssh_auth_type(auth_type) == SshAuthOption::PrivateKeyContent.value()
}

const REQUIRED_HOST_SSH_FIELD_NAMES: &[&str] = &[
    "ssh_tunnel_enabled",
    "ssh_connection_id",
    "ssh_host",
    "ssh_port",
    "ssh_username",
    "ssh_auth_type",
    "ssh_password",
    "ssh_private_key_path",
    "ssh_private_key_passphrase",
    "ssh_target_host",
    "ssh_target_port",
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum HostSslTabKind {
    MySql,
    PostgreSql,
    Mssql,
}

fn has_field(fields: &[FormField], field_name: &str) -> bool {
    fields.iter().any(|field| field.name == field_name)
}

fn has_all_fields(fields: &[FormField], field_names: &[&str]) -> bool {
    field_names
        .iter()
        .all(|field_name| has_field(fields, field_name))
}

fn should_use_custom_ssh_tab(db_type: &DatabaseType, fields: &[FormField]) -> bool {
    !db_type.is_external() || has_all_fields(fields, REQUIRED_HOST_SSH_FIELD_NAMES)
}

fn host_ssl_tab_kind(db_type: &DatabaseType, fields: &[FormField]) -> Option<HostSslTabKind> {
    match db_type {
        DatabaseType::MySQL => Some(HostSslTabKind::MySql),
        DatabaseType::PostgreSQL => Some(HostSslTabKind::PostgreSql),
        DatabaseType::MSSQL => Some(HostSslTabKind::Mssql),
        _ if has_field(fields, "ssl_mode") => Some(HostSslTabKind::PostgreSql),
        _ if has_field(fields, "encrypt") => Some(HostSslTabKind::Mssql),
        _ if has_field(fields, "require_ssl") => Some(HostSslTabKind::MySql),
        _ => None,
    }
}

fn is_custom_ssl_enabled(
    kind: HostSslTabKind,
    require_ssl: bool,
    ssl_mode: Option<&str>,
    encrypt: Option<&str>,
) -> bool {
    match kind {
        HostSslTabKind::MySql => require_ssl,
        HostSslTabKind::PostgreSql => ssl_mode
            .map(|value| !value.trim().eq_ignore_ascii_case("disable"))
            .unwrap_or(false),
        HostSslTabKind::Mssql => encrypt
            .map(|value| !value.trim().eq_ignore_ascii_case("off"))
            .unwrap_or(false),
    }
}

fn field_visible_from_values(
    field: &FormField,
    mut value_for: impl FnMut(&str) -> Option<String>,
) -> bool {
    field.visible_when.iter().all(|rule| {
        let value = value_for(&rule.when_field);
        rule.condition.matches(value.as_deref())
    })
}

fn missing_ssh_tunnel_required_field(
    enabled: bool,
    ssh_host: &str,
    ssh_username: &str,
    auth_type: &str,
    ssh_private_key_path: &str,
    ssh_private_key_content: &str,
    ssh_password: &str,
) -> Option<&'static str> {
    if !enabled {
        return None;
    }

    if ssh_host.trim().is_empty() {
        return Some("ssh_host");
    }

    if ssh_username.trim().is_empty() {
        return Some("ssh_username");
    }

    if ssh_auth_requires_private_key(auth_type) && ssh_private_key_path.trim().is_empty() {
        return Some("ssh_private_key_path");
    }

    if ssh_auth_requires_private_key_content(auth_type) && ssh_private_key_content.trim().is_empty()
    {
        return Some("ssh_private_key_content");
    }

    if ssh_auth_requires_password(auth_type) && ssh_password.trim().is_empty() {
        return Some("ssh_password");
    }

    None
}

fn credential_capabilities_for_fields(
    config: &DbFormConfig,
    username_field: &str,
    password_field: &str,
) -> CredentialCapabilities {
    let has_field = |name: &str| {
        config
            .tab_groups
            .iter()
            .flat_map(|tab| tab.fields.iter())
            .any(|field| field.name == name)
    };

    match (has_field(username_field), has_field(password_field)) {
        (true, true) => CredentialCapabilities::login(),
        (true, false) => CredentialCapabilities::username_only(),
        (false, true) => CredentialCapabilities::password_only(),
        (false, false) => CredentialCapabilities::default(),
    }
}

/// Event emitted when a connection is saved successfully
#[derive(Clone, Debug)]
pub enum DbConnectionFormEvent {
    Saved(Box<StoredConnection>),
    SaveError(String),
}

/// Database connection form modal
pub struct DbConnectionForm {
    config: DbFormConfig,
    current_db_type: Entity<DatabaseType>,
    focus_handle: FocusHandle,
    active_tab: usize,
    field_values: Vec<(String, Entity<String>)>,
    field_inputs: Vec<Option<Entity<InputState>>>,
    field_selects: std::collections::HashMap<String, Entity<SelectState<Vec<FormSelectItem>>>>,
    credential_picker: Entity<CredentialReferencePicker>,
    proxy_credential_picker: Entity<CredentialReferencePicker>,
    is_testing: Entity<bool>,
    test_result: Entity<Option<Result<bool, String>>>,
    workspace_select: Entity<SelectState<Vec<WorkspaceSelectItem>>>,
    team_select: Entity<SelectState<Vec<TeamSelectItem>>>,
    ssh_connection_select: Entity<SelectState<SearchableVec<SshConnectionSelectItem>>>,
    selected_ssh_connection_id: Option<i64>,
    ssh_connections: Vec<StoredConnection>,
    pending_file_path: Entity<Option<(String, String)>>,
    editing_connection: Option<StoredConnection>,
    /// Whether cloud sync is enabled.
    sync_enabled: Entity<bool>,
    /// Oracle client detection status: Ok(version) / Err(error).
    oracle_client_status: Entity<Option<Result<String, String>>>,
    oracle_client_checking: Entity<bool>,
    oracle_driver_mode: OracleDriverMode,
    _subscriptions: Vec<Subscription>,
}

impl DbConnectionForm {
    pub fn new(config: DbFormConfig, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let config = connection_proxy::with_proxy_tab(config);
        let focus_handle = cx.focus_handle();
        let current_db_type = cx.new(|_| config.db_type.clone());
        let oracle_driver_mode = oracle_driver_mode_for_database_type(&config.db_type);

        // Initialize field values, inputs, and selects
        let mut field_values = Vec::new();
        let mut field_inputs = Vec::new();
        let mut field_selects = std::collections::HashMap::new();

        for tab_group in &config.tab_groups {
            for field in &tab_group.fields {
                let value = cx.new(|_| field.default_value.clone());
                field_values.push((field.name.clone(), value.clone()));

                if field.field_type == FormFieldType::Select {
                    // Create SelectState for Select fields
                    let items: Vec<FormSelectItem> = field
                        .options
                        .iter()
                        .map(|(v, l)| FormSelectItem::new(v.clone(), l.clone()))
                        .collect();
                    // Find the index of the default value
                    let selected_index = if field.default_value.is_empty() {
                        Some(IndexPath::new(0))
                    } else {
                        items
                            .iter()
                            .position(|i| i.value == field.default_value)
                            .map(IndexPath::new)
                    };
                    let field_name = field.name.clone();
                    let value_clone = value.clone();
                    let select = cx.new(|cx| SelectState::new(items, selected_index, window, cx));
                    // Subscribe to select changes
                    cx.subscribe_in(
                        &select,
                        window,
                        move |_form,
                              _select,
                              event: &SelectEvent<Vec<FormSelectItem>>,
                              _window,
                              cx| {
                            if let SelectEvent::Confirm(Some(val)) = event {
                                value_clone.update(cx, |v, cx| {
                                    *v = val.clone();
                                    cx.notify();
                                });
                            }
                        },
                    )
                    .detach();
                    field_selects.insert(field_name, select);
                    field_inputs.push(None);
                } else if field.field_type == FormFieldType::Checkbox {
                    field_inputs.push(None);
                } else {
                    // Create InputState for other field types
                    let input = cx.new(|cx| {
                        let mut input_state =
                            InputState::new(window, cx).placeholder(&field.placeholder);

                        if field.field_type == FormFieldType::Password {
                            input_state = input_state.masked(true);
                        }

                        input_state.set_value(field.default_value.clone(), window, cx);
                        input_state
                    });

                    // Subscribe to input changes
                    let value_clone = value.clone();
                    cx.subscribe_in(&input, window, move |_form, _input, event, _window, cx| {
                        if let InputEvent::Change = event {
                            value_clone.update(cx, |v, cx| {
                                *v = _input.read(cx).text().to_string();
                                cx.notify();
                            });
                        }
                    })
                    .detach();

                    field_inputs.push(Some(input));
                }
            }
        }

        let is_testing = cx.new(|_| false);
        let test_result = cx.new(|_| None);

        let workspace_items = vec![WorkspaceSelectItem::none()];
        let workspace_select =
            cx.new(|cx| SelectState::new(workspace_items, Some(Default::default()), window, cx));

        let team_select = create_team_select(&[], None, window, cx);

        let ssh_connection_items = SearchableVec::new(vec![SshConnectionSelectItem::none()]);
        let ssh_connection_select = cx.new(|cx| {
            SelectState::new(ssh_connection_items, Some(Default::default()), window, cx)
                .searchable(true)
        });
        cx.subscribe_in(
            &ssh_connection_select,
            window,
            move |form,
                  _select,
                  event: &SelectEvent<SearchableVec<SshConnectionSelectItem>>,
                  window,
                  cx| {
                let SelectEvent::Confirm(selected_value) = event;
                let selected_id = selected_value.as_ref().copied().flatten();
                form.selected_ssh_connection_id = selected_id;
                let value = selected_id.map(|id| id.to_string()).unwrap_or_default();
                form.set_field_value("ssh_connection_id", &value, window, cx);
            },
        )
        .detach();

        let pending_file_path = cx.new(|_| None);

        // Enable cloud sync by default.
        let sync_enabled = cx.new(|_| true);
        let oracle_client_status = cx.new(|_| None);
        let oracle_client_checking = cx.new(|_| false);
        let credential_picker = create_credential_picker(
            CredentialPickerConfig::new(
                "database-credential",
                credential_capabilities_for_fields(&config, "username", "password"),
            ),
            window,
            cx,
        );
        let proxy_credential_picker = create_credential_picker(
            CredentialPickerConfig::new(
                "database-proxy-credential",
                credential_capabilities_for_fields(&config, "proxy_username", "proxy_password"),
            ),
            window,
            cx,
        );
        let subscriptions = vec![
            cx.subscribe(&credential_picker, |_, _, _: &CredentialPickerEvent, cx| {
                cx.notify()
            }),
            cx.subscribe(
                &proxy_credential_picker,
                |_, _, _: &CredentialPickerEvent, cx| cx.notify(),
            ),
        ];
        let form = Self {
            config,
            current_db_type,
            focus_handle,
            active_tab: 0,
            field_values,
            field_inputs,
            field_selects,
            credential_picker,
            proxy_credential_picker,
            is_testing,
            test_result,
            workspace_select,
            team_select,
            ssh_connection_select,
            selected_ssh_connection_id: None,
            ssh_connections: Vec::new(),
            pending_file_path,
            editing_connection: None,
            sync_enabled,
            oracle_client_status,
            oracle_client_checking,
            oracle_driver_mode,
            _subscriptions: subscriptions,
        };

        form.refresh_oracle_client_status(cx);
        form
    }

    fn effective_database_type(&self, cx: &App) -> DatabaseType {
        database_type_for_oracle_driver_mode(self.current_db_type.read(cx), self.oracle_driver_mode)
    }

    fn refresh_oracle_client_status(&self, cx: &mut Context<Self>) {
        if self.effective_database_type(cx) != DatabaseType::Oracle {
            self.oracle_client_checking.update(cx, |checking, cx| {
                *checking = false;
                cx.notify();
            });
            self.oracle_client_status.update(cx, |status, cx| {
                *status = None;
                cx.notify();
            });
            return;
        }

        self.oracle_client_checking.update(cx, |checking, cx| {
            *checking = true;
            cx.notify();
        });

        let checking_handle = self.oracle_client_checking.clone();
        let status_handle = self.oracle_client_status.clone();

        cx.spawn(async move |_, cx: &mut AsyncApp| {
            let result = oracle::detect_local_client_version();
            let _ = cx.update(|cx| {
                checking_handle.update(cx, |checking, cx| {
                    *checking = false;
                    cx.notify();
                });
                status_handle.update(cx, |status, cx| {
                    *status = Some(result);
                    cx.notify();
                });
            });
        })
        .detach();
    }

    fn oracle_client_guide_text(&self, cx: &App) -> Option<String> {
        if self.effective_database_type(cx) != DatabaseType::Oracle {
            return None;
        }

        let has_error = matches!(self.oracle_client_status.read(cx).as_ref(), Some(Err(_)));
        if !has_error {
            return None;
        }

        #[cfg(target_os = "windows")]
        return Some(t!("ConnectionForm.oracle_client_guide_windows").to_string());
        #[cfg(target_os = "macos")]
        return Some(t!("ConnectionForm.oracle_client_guide_macos").to_string());
        #[cfg(target_os = "linux")]
        return Some(t!("ConnectionForm.oracle_client_guide_linux").to_string());
        #[cfg(not(any(target_os = "windows", target_os = "macos", target_os = "linux")))]
        return Some(t!("ConnectionForm.oracle_client_guide_other").to_string());
    }

    fn oracle_client_download_url(&self, cx: &App) -> Option<&'static str> {
        if self.effective_database_type(cx) != DatabaseType::Oracle {
            return None;
        }

        let has_error = matches!(self.oracle_client_status.read(cx).as_ref(), Some(Err(_)));
        if !has_error {
            return None;
        }

        Some("https://www.oracle.com/database/technologies/instant-client/downloads.html")
    }

    pub fn set_workspaces(
        &mut self,
        workspaces: Vec<Workspace>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let mut items = vec![WorkspaceSelectItem::none()];
        items.extend(workspaces.iter().map(WorkspaceSelectItem::from_workspace));

        self.workspace_select.update(cx, |select, cx| {
            select.set_items(items, window, cx);
        });
        cx.notify();
    }

    pub fn set_teams(
        &mut self,
        teams: Vec<TeamOption>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        replace_team_options(&self.team_select, &teams, window, cx);
        cx.notify();
    }

    fn request_team_sync(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        refresh_team_options(&self.team_select, window, cx);
    }

    pub fn set_ssh_connections(
        &mut self,
        connections: Vec<StoredConnection>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.selected_ssh_connection_id = self.current_ssh_connection_id(cx);
        self.ssh_connections = connections
            .into_iter()
            .filter(|connection| connection.connection_type == ConnectionType::SshSftp)
            .collect();
        let mut items = vec![SshConnectionSelectItem::none()];
        items.extend(
            self.ssh_connections
                .iter()
                .map(SshConnectionSelectItem::from_connection),
        );

        self.ssh_connection_select.update(cx, |select, cx| {
            select.set_items(SearchableVec::new(items), window, cx);
        });
        self.sync_ssh_connection_selection(window, cx);
        cx.notify();
    }

    pub fn load_connection(
        &mut self,
        connection: &StoredConnection,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.editing_connection = Some(connection.clone());
        self.selected_ssh_connection_id = None;
        self.set_field_value("ssh_connection_id", "", window, cx);
        self.set_field_value("name", &connection.name, window, cx);

        // Load the sync state.
        self.sync_enabled.update(cx, |sync, cx| {
            *sync = connection.sync_enabled;
            cx.notify();
        });

        if let Ok(params) = connection.to_db_connection() {
            self.oracle_driver_mode = oracle_driver_mode_for_database_type(&params.database_type);
            self.refresh_oracle_client_status(cx);

            self.credential_picker.update(cx, |picker, cx| {
                picker.set_reference(params.credential_reference.clone(), window, cx)
            });
            self.proxy_credential_picker.update(cx, |picker, cx| {
                picker.set_reference(
                    params
                        .proxy
                        .as_ref()
                        .and_then(|proxy| proxy.credential_reference.clone()),
                    window,
                    cx,
                )
            });
            self.selected_ssh_connection_id = params
                .extra_params
                .get("ssh_connection_id")
                .and_then(|value| value.parse::<i64>().ok());
            self.set_field_value("host", &params.host, window, cx);
            self.set_field_value("port", &params.port.to_string(), window, cx);
            self.set_field_value("username", &params.username, window, cx);
            self.set_field_value("password", &params.password, window, cx);
            if let Some(db) = &params.database {
                self.set_field_value("database", db, window, cx);
            }
            if let Some(sn) = &params.service_name {
                self.set_field_value("service_name", sn, window, cx);
            }
            if let Some(sid) = &params.sid {
                self.set_field_value("sid", sid, window, cx);
            }
            if let Some(proxy) = &params.proxy {
                self.set_field_value("proxy_enabled", "true", window, cx);
                self.set_field_value(
                    "proxy_type",
                    match proxy.proxy_type {
                        ProxyType::Socks5 => "socks5",
                        ProxyType::Http => "http",
                    },
                    window,
                    cx,
                );
                self.set_field_value("proxy_host", &proxy.host, window, cx);
                self.set_field_value("proxy_port", &proxy.port.to_string(), window, cx);
                if let Some(username) = &proxy.username {
                    self.set_field_value("proxy_username", username, window, cx);
                }
                if let Some(password) = &proxy.password {
                    self.set_field_value("proxy_password", password, window, cx);
                }
            }
            for (key, value) in &params.extra_params {
                self.set_field_value(key, value, window, cx);
            }
        }

        if let Some(remark) = &connection.remark {
            self.set_field_value("remark", remark, window, cx);
        }

        if let Some(ws_id) = connection.workspace_id {
            self.workspace_select.update(cx, |select, cx| {
                select.set_selected_value(&Some(ws_id), window, cx);
            });
        } else {
            self.workspace_select.update(cx, |select, cx| {
                select.set_selected_value(&None, window, cx);
            });
        }

        // Load team ownership.
        if let Some(ref team_id) = connection.team_id {
            self.team_select.update(cx, |select, cx| {
                select.set_selected_value(&Some(team_id.clone()), window, cx);
            });
        } else {
            self.team_select.update(cx, |select, cx| {
                select.set_selected_value(&None, window, cx);
            });
        }

        self.sync_ssh_connection_selection(window, cx);
    }

    /// Prefills a new connection without switching the form into update mode.
    pub fn load_initial_connection(
        &mut self,
        connection: &StoredConnection,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.load_connection(connection, window, cx);
        self.editing_connection = None;
    }

    fn current_ssh_connection_id(&self, cx: &App) -> Option<i64> {
        self.selected_ssh_connection_id.or_else(|| {
            self.get_field_value("ssh_connection_id", cx)
                .and_then(|value| value.parse::<i64>().ok())
        })
    }

    fn sync_ssh_connection_selection(&self, window: &mut Window, cx: &mut Context<Self>) {
        let selected_id = self.current_ssh_connection_id(cx);
        self.ssh_connection_select.update(cx, |select, cx| {
            select.set_selected_value(&selected_id, window, cx);
        });
    }

    fn set_field_value(
        &mut self,
        field_name: &str,
        value: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some((idx, _)) = self
            .field_values
            .iter()
            .enumerate()
            .find(|(_, (name, _))| name == field_name)
        {
            self.field_values[idx].1.update(cx, |v, cx| {
                *v = value.to_string();
                cx.notify();
            });
            // Update input or select based on field type
            if let Some(Some(input)) = self.field_inputs.get(idx) {
                input.update(cx, |input, cx| {
                    input.set_value(value.to_string(), window, cx);
                });
            } else if let Some(select) = self.field_selects.get(field_name) {
                select.update(cx, |select, cx| {
                    select.set_selected_value(&value.to_string(), window, cx);
                });
            }
        }
    }

    fn get_field_value(&self, field_name: &str, cx: &App) -> Option<String> {
        self.field_values
            .iter()
            .find(|(name, _)| name == field_name)
            .map(|(_, value)| value.read(cx).clone())
    }

    fn build_connection(&self, cx: &App) -> DbConnectionConfig {
        let workspace_id = self
            .workspace_select
            .read(cx)
            .selected_value()
            .cloned()
            .flatten();

        // Collect extra params (fields that are not basic connection fields)
        let basic_fields = [
            "name",
            "host",
            "port",
            "username",
            "password",
            "database",
            "remark",
            "service_name",
            "sid",
        ];
        let mut extra_params = self.config.hidden_params.clone();

        for (field_name, value_entity) in &self.field_values {
            if let Some(field) = self.find_field(field_name) {
                if !self.is_field_visible(field, cx) {
                    continue;
                }
            }
            if !basic_fields.contains(&field_name.as_str())
                && !connection_proxy::is_proxy_field(field_name)
            {
                let value = value_entity.read(cx).clone();
                if !value.is_empty() {
                    extra_params.insert(field_name.clone(), value);
                }
            }
        }

        let db_type = self.effective_database_type(cx);

        let port_str = self.get_field_value("port", cx);

        let mut port = 3306;

        if let Some(port_str) = port_str {
            port = port_str.parse().unwrap_or(3306);
        }
        DbConnectionConfig {
            id: String::new(),
            database_type: db_type,
            name: self.get_field_value("name", cx).unwrap_or_default(),
            host: self.get_field_value("host", cx).unwrap_or_default(),
            port,
            username: self.get_field_value("username", cx).unwrap_or_default(),
            password: self.get_field_value("password", cx).unwrap_or_default(),
            database: self.get_field_value("database", cx),
            service_name: self.get_field_value("service_name", cx),
            sid: self.get_field_value("sid", cx),
            workspace_id,
            proxy: self.proxy_config(cx).ok().flatten(),
            extra_params,
            credential_reference: self.credential_picker.read(cx).selected_reference(),
        }
    }

    fn resolve_referenced_ssh_connection(&self, cx: &App) -> Option<&StoredConnection> {
        let selected_id = self.current_ssh_connection_id(cx)?;

        self.ssh_connections
            .iter()
            .find(|connection| connection.id == Some(selected_id))
    }

    fn build_connection_with_referenced_ssh(&self, cx: &App) -> Result<DbConnectionConfig, String> {
        let mut connection = self.build_connection(cx);

        if let Some(ssh_connection) = self.resolve_referenced_ssh_connection(cx) {
            connection.extra_params.insert(
                "ssh_connection_id".to_string(),
                ssh_connection.id.unwrap().to_string(),
            );
        }

        Ok(connection)
    }

    fn validate(&self, cx: &App) -> Result<(), String> {
        for tab_group in &self.config.tab_groups {
            for field in &tab_group.fields {
                if !self.is_field_visible(field, cx) {
                    continue;
                }
                if field.required {
                    let value = self.get_field_value(&field.name, cx);
                    if value.is_none() {
                        return Err(format!("{} is required", field.label));
                    }
                }
            }
        }

        self.validate_oracle_client(cx)?;
        self.validate_ssh_tunnel(cx)?;
        self.validate_proxy(cx)?;
        Ok(())
    }

    fn proxy_config(&self, cx: &App) -> Result<Option<ProxyConfig>, ProxyValidationError> {
        connection_proxy::build_proxy_config(
            self.field_bool_value("proxy_enabled", cx),
            &self
                .get_field_value("proxy_type", cx)
                .unwrap_or_else(|| "socks5".to_string()),
            &self.get_field_value("proxy_host", cx).unwrap_or_default(),
            &self.get_field_value("proxy_port", cx).unwrap_or_default(),
            &self
                .get_field_value("proxy_username", cx)
                .unwrap_or_default(),
            &self
                .get_field_value("proxy_password", cx)
                .unwrap_or_default(),
            self.proxy_credential_picker.read(cx).selected_reference(),
        )
    }

    fn validate_proxy(&self, cx: &App) -> Result<(), String> {
        self.proxy_config(cx).map(|_| ()).map_err(|error| {
            let field = error.field().unwrap_or("proxy");
            format!(
                "{}: {}",
                t!("ConnectionForm.proxy_invalid"),
                self.field_label(field)
            )
        })
    }

    fn validate_ssh_tunnel(&self, cx: &App) -> Result<(), String> {
        let enabled = self
            .get_field_value("ssh_tunnel_enabled", cx)
            .map(|value| value == "true" || value == "1")
            .unwrap_or(false);
        let auth_type = self
            .get_field_value("ssh_auth_type", cx)
            .unwrap_or_else(|| "password".to_string());
        if self.resolve_referenced_ssh_connection(cx).is_some() {
            return Ok(());
        }
        let missing_field = missing_ssh_tunnel_required_field(
            enabled,
            &self.get_field_value("ssh_host", cx).unwrap_or_default(),
            &self.get_field_value("ssh_username", cx).unwrap_or_default(),
            &auth_type,
            &self
                .get_field_value("ssh_private_key_path", cx)
                .unwrap_or_default(),
            &self
                .get_field_value("ssh_private_key_content", cx)
                .unwrap_or_default(),
            &self.get_field_value("ssh_password", cx).unwrap_or_default(),
        );

        if let Some(field) = missing_field {
            return Err(format!(
                "{}: {}",
                t!("ConnectionForm.ssh_tunnel_invalid"),
                t!("ConnectionForm.ssh_missing_required", field = field)
            ));
        }

        Ok(())
    }

    fn validate_oracle_client(&self, cx: &App) -> Result<(), String> {
        if self.effective_database_type(cx) != DatabaseType::Oracle {
            return Ok(());
        }

        oracle::detect_local_client_version()
            .map(|_| ())
            .map_err(|error| t!("ConnectionForm.oracle_client_required", error = error).to_string())
    }

    fn simplify_connection_error_message(err: &Error) -> String {
        let mut message = err
            .chain()
            .find_map(|error| {
                let message = error.to_string();
                message.contains("SQLSTATE").then_some(message)
            })
            .or_else(|| err.chain().last().map(|error| error.to_string()))
            .unwrap_or_else(|| err.to_string());

        // Strip common wrapper prefixes and keep the most useful root-level message.
        let prefixes = [
            "connection error: ",
            "query error: ",
            "transaction error: ",
            "failed to connect: ",
            "failed to switch schema: ",
            "failed to query: ",
        ];

        loop {
            let mut changed = false;
            for prefix in prefixes {
                if let Some(rest) = message.strip_prefix(prefix) {
                    message = rest.trim().to_string();
                    changed = true;
                    break;
                }
            }
            if !changed {
                break;
            }
        }

        if let Some(pos) = message.find("ORA-") {
            return message[pos..].trim().to_string();
        }

        message.trim().to_string()
    }

    pub fn trigger_test_connection(&mut self, cx: &mut Context<Self>) {
        if let Err(e) = self.validate(cx) {
            self.test_result.update(cx, |result, cx| {
                *result = Some(Err(e));
                cx.notify();
            });
            return;
        }

        let connection = match self
            .build_connection_with_referenced_ssh(cx)
            .and_then(|connection| {
                resolve_connection_for_runtime(
                    StoredConnection::new_database(connection.name.clone(), connection, None),
                    cx,
                )
            })
            .and_then(|connection| {
                connection
                    .to_db_connection()
                    .map_err(|error| error.to_string())
            }) {
            Ok(connection) => connection,
            Err(error) => {
                self.test_result.update(cx, |result, cx| {
                    *result = Some(Err(error));
                    cx.notify();
                });
                return;
            }
        };
        let db_type = self.effective_database_type(cx);

        self.is_testing.update(cx, |testing, cx| {
            *testing = true;
            cx.notify();
        });

        let global_state = cx.global::<GlobalDbState>().clone();
        let test_result_handle = self.test_result.clone();
        let is_testing_handle = self.is_testing.clone();

        cx.spawn(async move |_, cx: &mut AsyncApp| {
            let manager = global_state.db_manager;

            let test_result = Tokio::spawn_result(cx, async move {
                let test_started = Instant::now();
                let db_plugin = manager.get_plugin(&db_type)?;
                match db_plugin.test_connection(connection).await {
                    Ok(()) => {
                        info!(
                            "[DB][Timing] test_connection total db_type={:?} elapsed={}ms",
                            db_type,
                            test_started.elapsed().as_millis()
                        );
                    }
                    Err(error) => {
                        info!(
                            "[DB][Timing] test_connection failed db_type={:?} elapsed={}ms error={}",
                            db_type,
                            test_started.elapsed().as_millis(),
                            error
                        );
                        return Err(Error::new(error));
                    }
                }
                Ok::<bool, Error>(true)
            })
            .await;

            let result_msg = match test_result {
                Ok(_) => Ok(true),
                Err(err) => {
                    let detail = Self::simplify_connection_error_message(&err);
                    Err(format!("{}: {}", t!("ConnectionForm.test_failed"), detail))
                }
            };

            let _ = cx.update(|cx| {
                is_testing_handle.update(cx, |testing, cx| {
                    *testing = false;
                    cx.notify();
                });
                test_result_handle.update(cx, |result, cx| {
                    *result = Some(result_msg);
                    cx.notify();
                });
            });
        })
        .detach();
    }

    pub fn build_stored_connection(&self, cx: &App) -> Result<(StoredConnection, bool), String> {
        self.validate(cx)?;

        let connection = self.build_connection_with_referenced_ssh(cx)?;
        let remark = self.get_field_value("remark", cx);
        let is_update = self.editing_connection.is_some();
        let sync_enabled = *self.sync_enabled.read(cx);
        let team_id = selected_team_id(&self.team_select, cx);

        let mut stored = match &self.editing_connection {
            Some(conn) => {
                let mut c = conn.clone();
                c.name = StoredConnection::from_db_connection(connection.clone()).name;
                c.workspace_id = connection.workspace_id;
                c.sync_enabled = sync_enabled;
                c.params = serde_json::to_string(&connection)
                    .map_err(|e| format!("{}: {}", t!("ConnectionForm.serialize_failed"), e))?;
                // Keep selected_databases aligned with the current database config.
                c.selected_databases = if let Some(database) = &connection.database {
                    Some(format!("[\"{}\"]", database))
                } else {
                    None
                };
                c
            }
            None => {
                let mut c = StoredConnection::from_db_connection(connection);
                c.sync_enabled = sync_enabled;
                c
            }
        };

        let assignment = resolve_team_assignment(team_id, is_update, stored.owner_id.clone(), cx)
            .map_err(|error| error.to_string())?;
        stored.team_id = assignment.team_id;
        stored.owner_id = assignment.owner_id;
        stored.remark = remark;
        Ok((stored, is_update))
    }

    pub fn set_save_error(&mut self, error: String, cx: &mut Context<Self>) {
        self.test_result.update(cx, |result, cx| {
            *result = Some(Err(error));
            cx.notify();
        });
    }

    pub fn trigger_cancel(&mut self, _cx: &mut Context<Self>) {
        self.editing_connection = None;
    }

    pub fn is_testing(&self, cx: &App) -> bool {
        *self.is_testing.read(cx)
    }

    /// Returns the display string for the test-connection result, or None if absent.
    pub fn test_result_msg(&self, cx: &App) -> Option<String> {
        self.test_result.read(cx).as_ref().map(|r| match r {
            Ok(true) => format!("✓ {}", t!("ConnectionForm.test_success")),
            Ok(false) => format!("✗ {}", t!("ConnectionForm.connection_failed")),
            Err(e) => format!("✗ {}", e),
        })
    }

    pub fn set_test_result(&mut self, result: Result<bool, String>, cx: &mut Context<Self>) {
        self.is_testing.update(cx, |testing, cx| {
            *testing = false;
            cx.notify();
        });
        self.test_result.update(cx, |test_result, cx| {
            *test_result = Some(result);
            cx.notify();
        });
    }

    pub fn clear_test_result(&mut self, cx: &mut Context<Self>) {
        self.test_result.update(cx, |test_result, cx| {
            *test_result = None;
            cx.notify();
        });
    }

    pub fn save_connection(&mut self, cx: &mut Context<Self>) {
        let (stored, is_update) = match self.build_stored_connection(cx) {
            Ok(data) => data,
            Err(e) => {
                self.set_save_error(e.clone(), cx);
                cx.emit(DbConnectionFormEvent::SaveError(e));
                return;
            }
        };

        let storage = cx.global::<GlobalStorageState>().storage.clone();

        cx.spawn(async move |this, cx: &mut AsyncApp| {
            let repo_op = storage.get::<ConnectionRepository>();
            if let Some(repo) = repo_op {
                let mut stored = stored;
                if is_update {
                    let re = repo.update(&stored);
                    match re {
                        Ok(..) => {
                            let _ = this.update(cx, |form, cx| {
                                form.editing_connection = None;
                                cx.emit(DbConnectionFormEvent::Saved(Box::new(stored)));
                            });
                        }
                        Err(e) => {
                            let error_msg = format!("{}: {}", t!("ConnectionForm.save_failed"), e);
                            let _ = this.update(cx, |form, cx| {
                                form.set_save_error(error_msg.clone(), cx);
                                cx.emit(DbConnectionFormEvent::SaveError(error_msg));
                            });
                        }
                    }
                } else {
                    let re = repo.insert(&mut stored);
                    match re {
                        Ok(id) => {
                            let _ = this.update(cx, |form, cx| {
                                form.editing_connection = None;
                                stored.id = Some(id);
                                cx.emit(DbConnectionFormEvent::Saved(Box::new(stored)));
                            });
                        }
                        Err(e) => {
                            let error_msg = format!("{}: {}", t!("ConnectionForm.save_failed"), e);
                            let _ = this.update(cx, |form, cx| {
                                form.set_save_error(error_msg.clone(), cx);
                                cx.emit(DbConnectionFormEvent::SaveError(error_msg));
                            });
                        }
                    }
                }
            }
        })
        .detach();
    }

    fn browse_file_path_for_field(&mut self, field_name: impl Into<String>, cx: &mut App) {
        let pending = self.pending_file_path.clone();
        let field_name = field_name.into();

        let future = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            multiple: false,
            directories: false,
            prompt: Some(t!("ConnectionForm.select_database_file").into()),
        });

        cx.spawn(async move |cx| {
            if let Ok(Ok(Some(paths))) = future.await {
                if let Some(path) = paths.first() {
                    let path_str = path.to_string_lossy().to_string();
                    let _ = cx.update(|cx| {
                        pending.update(cx, |p, cx| {
                            *p = Some((field_name, path_str));
                            cx.notify();
                        });
                    });
                }
            }
        })
        .detach();
    }

    fn get_input_by_name(&self, field_name: &str) -> Option<Entity<InputState>> {
        let mut idx = 0;
        for tab_group in &self.config.tab_groups {
            for field in &tab_group.fields {
                if field.name == field_name {
                    return self.field_inputs.get(idx).and_then(|opt| opt.clone());
                }
                idx += 1;
            }
        }
        None
    }

    fn find_field(&self, field_name: &str) -> Option<&FormField> {
        self.config
            .tab_groups
            .iter()
            .flat_map(|group| group.fields.iter())
            .find(|field| field.name == field_name)
    }

    fn is_field_visible(&self, field: &FormField, cx: &App) -> bool {
        field_visible_from_values(field, |name| self.get_field_value(name, cx))
    }

    fn field_label(&self, field_name: &str) -> String {
        self.find_field(field_name)
            .map(|field| field.label.clone())
            .unwrap_or_else(|| field_name.to_string())
    }

    fn field_bool_value(&self, field_name: &str, cx: &App) -> bool {
        self.get_field_value(field_name, cx)
            .map(|value| value == "true" || value == "1")
            .unwrap_or(false)
    }

    fn render_credential_picker_field(&self, proxy: bool) -> gpui_component::form::Field {
        let picker = if proxy {
            self.proxy_credential_picker.clone()
        } else {
            self.credential_picker.clone()
        };
        field()
            .label("钥匙串")
            .items_center()
            .child(div().w_full().child(picker))
    }

    fn set_bool_field_value(
        &mut self,
        field_name: &str,
        value: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.set_field_value(field_name, if value { "true" } else { "false" }, window, cx);
    }

    fn is_ssl_enabled(&self, kind: HostSslTabKind, cx: &App) -> bool {
        is_custom_ssl_enabled(
            kind,
            self.field_bool_value("require_ssl", cx),
            self.get_field_value("ssl_mode", cx).as_deref(),
            self.get_field_value("encrypt", cx).as_deref(),
        )
    }

    fn toggle_ssl_enabled(
        &mut self,
        kind: HostSslTabKind,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let next_enabled = !self.is_ssl_enabled(kind, cx);
        match kind {
            HostSslTabKind::MySql => {
                self.set_bool_field_value("require_ssl", next_enabled, window, cx);
            }
            HostSslTabKind::PostgreSql => {
                let next_mode = if next_enabled {
                    let current_mode = self
                        .get_field_value("ssl_mode", cx)
                        .unwrap_or_else(|| "prefer".to_string());
                    if current_mode.eq_ignore_ascii_case("disable") {
                        "prefer".to_string()
                    } else {
                        current_mode
                    }
                } else {
                    "disable".to_string()
                };
                self.set_field_value("ssl_mode", &next_mode, window, cx);
            }
            HostSslTabKind::Mssql => {
                let next_encrypt = if next_enabled {
                    let current_encrypt = self
                        .get_field_value("encrypt", cx)
                        .unwrap_or_else(|| "on".to_string());
                    if current_encrypt.eq_ignore_ascii_case("off") {
                        "on".to_string()
                    } else {
                        current_encrypt
                    }
                } else {
                    "off".to_string()
                };
                self.set_field_value("encrypt", &next_encrypt, window, cx);
            }
        }
    }

    fn render_field_by_name(
        &self,
        field_name: &str,
        cx: &mut Context<Self>,
    ) -> gpui_component::form::Field {
        let Some(field_info) = self.find_field(field_name) else {
            return field();
        };
        if !self.is_field_visible(field_info, cx) {
            return field();
        }

        let is_select = field_info.field_type == FormFieldType::Select;
        let is_checkbox = field_info.field_type == FormFieldType::Checkbox;
        let is_file_path = field_info.field_type == FormFieldType::FilePath;
        let is_password = field_info.field_type == FormFieldType::Password;
        let is_textarea = field_info.field_type == FormFieldType::TextArea;
        let field_name = field_info.name.clone();

        field()
            .label(field_info.label.clone())
            .required(field_info.required)
            .when(!is_textarea, |field| field.items_center())
            .when(is_textarea, |field| field.items_start())
            .child(
                h_flex()
                    .w_full()
                    .gap_2()
                    .when(is_textarea, |el| el.items_start())
                    .when(is_select, |el| {
                        if let Some(select_state) = self.field_selects.get(&field_name) {
                            el.child(Select::new(select_state).w_full())
                        } else {
                            el
                        }
                    })
                    .when(is_checkbox, |el| {
                        let checkbox_field = field_name.clone();
                        el.child(
                            Checkbox::new(format!("{checkbox_field}-checkbox"))
                                .checked(self.field_bool_value(&field_name, cx))
                                .on_click(cx.listener(move |this, _, window, cx| {
                                    let next = !this.field_bool_value(&checkbox_field, cx);
                                    this.set_bool_field_value(&checkbox_field, next, window, cx);
                                })),
                        )
                    })
                    .when(!is_select && !is_checkbox, |el| {
                        if let Some(input_state) = self.get_input_by_name(&field_name) {
                            let input = Input::new(&input_state).w_full();
                            let input = if is_password {
                                input.mask_toggle()
                            } else {
                                input
                            };
                            el.child(input)
                        } else {
                            el
                        }
                        .when(is_file_path, |el| {
                            let file_field = field_name.clone();
                            el.child(
                                Button::new(format!("{file_field}-browse-file"))
                                    .icon(IconName::FolderOpen)
                                    .ghost()
                                    .on_click(cx.listener(move |this, _, _window, cx| {
                                        this.browse_file_path_for_field(file_field.clone(), cx);
                                    })),
                            )
                        })
                    }),
            )
    }

    fn render_standard_tab_content(
        &self,
        current_tab_fields: &[FormField],
        field_input_offset: usize,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let visible_fields = current_tab_fields
            .iter()
            .enumerate()
            .filter(|(_, field)| {
                self.is_field_visible(field, cx)
                    && match field.name.as_str() {
                        "username" | "password" => self
                            .credential_picker
                            .read(cx)
                            .selected_reference()
                            .is_none(),
                        "proxy_username" | "proxy_password" => self
                            .proxy_credential_picker
                            .read(cx)
                            .selected_reference()
                            .is_none(),
                        _ => true,
                    }
            })
            .collect::<Vec<_>>();

        if visible_fields.is_empty() {
            return div()
                .flex()
                .items_center()
                .justify_center()
                .h_full()
                .text_color(cx.theme().muted_foreground)
                .child(t!("SqlEditor.no_settings").to_string())
                .into_any_element();
        }

        let is_general_tab = self.active_tab == 0;
        let db_type = self.config.db_type.clone();
        let is_builtin_oracle = db_type == DatabaseType::Oracle;
        let is_native_oracle = self.effective_database_type(cx) == DatabaseType::Oracle;
        let has_main_credentials = current_tab_fields
            .iter()
            .any(|field| matches!(field.name.as_str(), "username" | "password"));
        let has_proxy_credentials = self.field_bool_value("proxy_enabled", cx)
            && current_tab_fields
                .iter()
                .any(|field| matches!(field.name.as_str(), "proxy_username" | "proxy_password"));

        v_form()
            .layout(Axis::Horizontal)
            .with_size(Size::Medium)
            .columns(1)
            .label_width(px(100.))
            .when(has_main_credentials, |form| {
                form.child(self.render_credential_picker_field(false))
            })
            .when(has_proxy_credentials, |form| {
                form.child(self.render_credential_picker_field(true))
            })
            .children(visible_fields.into_iter().map(|(i, field_info)| {
                let input_idx = field_input_offset + i;
                let is_sqlite_path = matches!(db_type, DatabaseType::SQLite | DatabaseType::DuckDB)
                    && field_info.name == "host";
                let is_textarea = field_info.field_type == FormFieldType::TextArea;
                let is_select = field_info.field_type == FormFieldType::Select;
                let is_checkbox = field_info.field_type == FormFieldType::Checkbox;
                let is_file_path = field_info.field_type == FormFieldType::FilePath;
                let is_password = field_info.field_type == FormFieldType::Password;
                let field_name = field_info.name.clone();

                field()
                    .label(field_info.label.clone())
                    .required(field_info.required)
                    .when(!is_textarea, |f| f.items_center())
                    .when(is_textarea, |f| f.items_start())
                    .child(
                        h_flex()
                            .w_full()
                            .gap_2()
                            .when(is_textarea, |el| el.items_start())
                            .when(is_select, |el| {
                                if let Some(select_state) = self.field_selects.get(&field_name) {
                                    el.child(Select::new(select_state).w_full())
                                } else {
                                    el
                                }
                            })
                            .when(is_checkbox, |el| {
                                let checkbox_field = field_name.clone();
                                el.child(
                                    Checkbox::new(format!("{checkbox_field}-checkbox"))
                                        .checked(self.field_bool_value(&field_name, cx))
                                        .on_click(cx.listener(move |this, _, window, cx| {
                                            let next =
                                                !this.field_bool_value(&checkbox_field, cx);
                                            this.set_bool_field_value(
                                                &checkbox_field,
                                                next,
                                                window,
                                                cx,
                                            );
                                        })),
                                )
                            })
                            .when(!is_select && !is_checkbox, |el| {
                                if let Some(Some(input_state)) = self.field_inputs.get(input_idx) {
                                    let input = Input::new(input_state).w_full();
                                    let input = if is_password {
                                        input.mask_toggle()
                                    } else {
                                        input
                                    };
                                    el.child(input)
                                } else {
                                    el
                                }
                            })
                            .when(is_sqlite_path || is_file_path, |el| {
                                let file_field = if is_file_path {
                                    field_name.clone()
                                } else {
                                    "host".to_string()
                                };
                                el.child(
                                    Button::new(format!("{file_field}-browse-file"))
                                        .icon(IconName::FolderOpen)
                                        .ghost()
                                        .on_click(cx.listener(move |this, _, _window, cx| {
                                            this.browse_file_path_for_field(
                                                file_field.clone(),
                                                cx,
                                            );
                                        })),
                                )
                            }),
                    )
            }))
            .when(is_general_tab, |form| {
                let sync_enabled = self.sync_enabled.clone();
                let is_sync_checked = *self.sync_enabled.read(cx);
                let is_checking = *self.oracle_client_checking.read(cx);
                let oracle_client_status = self.oracle_client_status.read(cx).clone();
                let oracle_client_guide = self.oracle_client_guide_text(cx);
                let oracle_client_download_url = self.oracle_client_download_url(cx);

                form.child(
                    field()
                        .label(t!("ConnectionForm.workspace").to_string())
                        .items_center()
                        .child(Select::new(&self.workspace_select).w_full()),
                )
                .when(
                    connection_sync_controls_visible_in(cx) && team_management_enabled(cx),
                    |form| {
                    form.child(
                        field()
                            .label(team_label())
                            .items_center()
                            .child(
                                h_flex()
                                    .gap_2()
                                    .child(Select::new(&self.team_select).w_full())
                                    .child(
                                        Button::new("sync-db-teams")
                                            .icon(IconName::Refresh)
                                            .ghost()
                                            .tooltip(refresh_teams_tooltip())
                                            .on_click(cx.listener(|this, _, window, cx| {
                                                this.request_team_sync(window, cx);
                                            })),
                                    ),
                            ),
                    )
                    },
                )
                .when(connection_sync_controls_visible_in(cx), |form| {
                    form.child(
                        field()
                            .label(t!("ConnectionForm.cloud_sync").to_string())
                            .items_center()
                            .child(
                                h_flex()
                                    .gap_2()
                                    .child(
                                        Checkbox::new("sync-enabled")
                                            .checked(is_sync_checked)
                                            .on_click(move |_, _, cx| {
                                                sync_enabled.update(cx, |sync, cx| {
                                                    *sync = !*sync;
                                                    cx.notify();
                                                });
                                            }),
                                    )
                                    .child(
                                        div()
                                            .text_sm()
                                            .text_color(cx.theme().muted_foreground)
                                            .child(t!("ConnectionForm.cloud_sync_desc").to_string()),
                                    ),
                        ),
                    )
                })
                .when(is_builtin_oracle, |form| {
                    let is_native = self.oracle_driver_mode == OracleDriverMode::Native;
                    let is_go = self.oracle_driver_mode == OracleDriverMode::Go;

                    form.child(
                        field()
                            .label(t!("ConnectionForm.oracle_driver_mode").to_string())
                            .items_center()
                            .child(
                                h_flex()
                                    .w_full()
                                    .items_center()
                                    .flex_wrap()
                                    .gap_3()
                                    .child(
                                        h_flex()
                                            .items_center()
                                            .gap_1()
                                            .flex_shrink_0()
                                            .child(
                                                Radio::new("oracle-driver-mode-native")
                                                    .label(
                                                        t!(
                                                            "ConnectionForm.oracle_driver_native"
                                                        )
                                                        .to_string(),
                                                    )
                                                    .checked(is_native)
                                                    .on_click(cx.listener(
                                                        |this, _, _window, cx| {
                                                            this.oracle_driver_mode =
                                                                OracleDriverMode::Native;
                                                            this.refresh_oracle_client_status(cx);
                                                            cx.notify();
                                                        },
                                                    )),
                                            )
                                            .child(
                                                Popover::new("oracle-driver-native-help")
                                                    .trigger(
                                                        Button::new("oracle-driver-native-help-btn")
                                                            .icon(IconName::Info)
                                                            .ghost()
                                                            .xsmall()
                                                            .tooltip(
                                                                t!(
                                                                    "ConnectionForm.oracle_driver_native_desc"
                                                                )
                                                                .to_string(),
                                                            ),
                                                    )
                                                    .content(|_, _, _| {
                                                        div()
                                                            .text_sm()
                                                            .max_w(px(320.))
                                                            .child(
                                                                t!(
                                                                    "ConnectionForm.oracle_driver_native_desc"
                                                                )
                                                                .to_string(),
                                                            )
                                                    }),
                                            ),
                                    )
                                    .child(
                                        h_flex()
                                            .items_center()
                                            .gap_1()
                                            .flex_shrink_0()
                                            .child(
                                                Radio::new("oracle-driver-mode-go")
                                                    .label(
                                                        t!("ConnectionForm.oracle_driver_go")
                                                            .to_string(),
                                                    )
                                                    .checked(is_go)
                                                    .on_click(cx.listener(
                                                        |this, _, _window, cx| {
                                                            this.oracle_driver_mode =
                                                                OracleDriverMode::Go;
                                                            this.refresh_oracle_client_status(cx);
                                                            cx.notify();
                                                        },
                                                    )),
                                            )
                                            .child(
                                                Popover::new("oracle-driver-go-help")
                                                    .trigger(
                                                        Button::new("oracle-driver-go-help-btn")
                                                            .icon(IconName::Info)
                                                            .ghost()
                                                            .xsmall()
                                                            .tooltip(
                                                                t!(
                                                                    "ConnectionForm.oracle_driver_go_desc"
                                                                )
                                                                .to_string(),
                                                            ),
                                                    )
                                                    .content(|_, _, _| {
                                                        div()
                                                            .text_sm()
                                                            .max_w(px(320.))
                                                            .child(
                                                                t!(
                                                                    "ConnectionForm.oracle_driver_go_desc"
                                                                )
                                                                .to_string(),
                                                            )
                                                    }),
                                            ),
                                    ),
                            ),
                    )
                })
                .when(is_builtin_oracle && is_native_oracle, |form| {
                    let has_error = matches!(&oracle_client_status, Some(Err(_)));
                    let oracle_client_guide = oracle_client_guide.clone();

                    form.child(
                        field()
                            .label(t!("ConnectionForm.oracle_client_status").to_string())
                            .items_center()
                            .child(
                                h_flex()
                                    .w_full()
                                    .items_center()
                                    .gap_2()
                                    .child(
                                        div()
                                            .text_sm()
                                            .overflow_hidden()
                                            .text_ellipsis()
                                            .whitespace_nowrap()
                                            .flex_shrink_1()
                                            .min_w_0()
                                            .when(is_checking, |div| {
                                                div.text_color(cx.theme().muted_foreground).child(
                                                    t!("ConnectionForm.oracle_client_checking")
                                                        .to_string(),
                                                )
                                            })
                                            .when(!is_checking, |div| match &oracle_client_status {
                                                Some(Ok(version)) => div
                                                    .text_color(cx.theme().success)
                                                    .child(
                                                        t!(
                                                            "ConnectionForm.oracle_client_available",
                                                            version = version
                                                        )
                                                        .to_string(),
                                                    ),
                                                Some(Err(error)) => div
                                                    .text_color(cx.theme().danger)
                                                    .child(
                                                        t!(
                                                            "ConnectionForm.oracle_client_unavailable",
                                                            error = error
                                                        )
                                                        .to_string(),
                                                    ),
                                                None => div
                                                    .text_color(cx.theme().muted_foreground)
                                                    .child("-"),
                                            }),
                                    )
                                    .child(
                                        div().flex_shrink_0().child(
                                            Button::new("oracle-client-status-refresh")
                                                .small()
                                                .ghost()
                                                .icon(IconName::Refresh)
                                                .disabled(is_checking)
                                                .on_click(cx.listener(|this, _, _window, cx| {
                                                    this.refresh_oracle_client_status(cx);
                                                })),
                                        ),
                                    )
                                    .when(has_error, |this| {
                                        let guide = oracle_client_guide.clone();
                                        let download_url = oracle_client_download_url;
                                        this.child(
                                            div().flex_shrink_0().child(
                                                Popover::new("oracle-client-guide-popover")
                                                    .trigger(
                                                        Button::new("oracle-client-guide-btn")
                                                            .small()
                                                            .ghost()
                                                            .icon(IconName::Info)
                                                            .label(
                                                                t!(
                                                                    "ConnectionForm.oracle_client_guide_label"
                                                                )
                                                                .to_string(),
                                                            ),
                                                    )
                                                    .content(move |_state, _window, cx| {
                                                        v_flex()
                                                            .gap_2()
                                                            .max_w(px(360.))
                                                            .child(
                                                                h_flex()
                                                                    .items_center()
                                                                    .gap_1()
                                                                    .child(
                                                                        Icon::new(IconName::Info)
                                                                            .with_size(Size::Small)
                                                                            .text_color(
                                                                                cx.theme()
                                                                                    .muted_foreground,
                                                                            ),
                                                                    )
                                                                    .child(
                                                                        div()
                                                                            .text_sm()
                                                                            .font_weight(
                                                                                gpui::FontWeight::MEDIUM,
                                                                            )
                                                                            .child(
                                                                                t!(
                                                                                    "ConnectionForm.oracle_client_guide_title"
                                                                                )
                                                                                .to_string(),
                                                                            ),
                                                                    ),
                                                            )
                                                            .when_some(guide.clone(), |this, guide| {
                                                                this.child(
                                                                    div()
                                                                        .text_sm()
                                                                        .text_color(
                                                                            cx.theme()
                                                                                .muted_foreground,
                                                                        )
                                                                        .child(guide),
                                                                )
                                                            })
                                                            .when_some(download_url, |this, url| {
                                                                this.child(
                                                                    h_flex()
                                                                        .w_full()
                                                                        .justify_end()
                                                                        .gap_2()
                                                                        .child(
                                                                            Clipboard::new(
                                                                                "oracle-client-copy-url",
                                                                            )
                                                                            .value(
                                                                                SharedString::from(
                                                                                    url,
                                                                                ),
                                                                            ),
                                                                        )
                                                                        .child(
                                                                            Button::new(
                                                                                "oracle-client-download-page",
                                                                            )
                                                                            .small()
                                                                            .outline()
                                                                            .label(
                                                                                t!(
                                                                                    "ConnectionForm.oracle_client_open_download"
                                                                                )
                                                                                .to_string(),
                                                                            )
                                                                            .on_click(
                                                                                move |_, _window, cx| {
                                                                                    cx.open_url(url);
                                                                                },
                                                                            ),
                                                                        ),
                                                                )
                                                            })
                                                    }),
                                            ),
                                        )
                                    }),
                            ),
                    )
                })
            })
            .into_any_element()
    }

    fn render_ssh_tab_content(
        &self,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let ssh_enabled = self.field_bool_value("ssh_tunnel_enabled", cx);
        let using_ssh_reference = self.resolve_referenced_ssh_connection(cx).is_some();
        let ssh_auth_type = self
            .get_field_value("ssh_auth_type", cx)
            .unwrap_or_else(|| "password".to_string());
        let ssh_auth_type = normalized_ssh_auth_type(&ssh_auth_type).to_string();

        v_form()
            .layout(Axis::Horizontal)
            .with_size(Size::Medium)
            .columns(1)
            .label_width(px(100.))
            .child(
                field()
                    .label(self.field_label("ssh_tunnel_enabled"))
                    .items_center()
                    .child(
                        Checkbox::new("db-ssh-tunnel-enabled")
                            .checked(ssh_enabled)
                            .on_click(cx.listener(|this, _, window, cx| {
                                let next_enabled = !this.field_bool_value("ssh_tunnel_enabled", cx);
                                this.set_bool_field_value(
                                    "ssh_tunnel_enabled",
                                    next_enabled,
                                    window,
                                    cx,
                                );
                            })),
                    ),
            )
            .when(ssh_enabled, |form| {
                form.child(
                    field()
                        .label(t!("ConnectionForm.ssh_connection_id").to_string())
                        .items_center()
                        .child(
                            Select::new(&self.ssh_connection_select)
                                .placeholder(t!("ConnectionForm.ssh_connection_manual"))
                                .w_full(),
                        ),
                )
                .when(!using_ssh_reference, |form| {
                    form.child(self.render_field_by_name("ssh_host", cx))
                        .child(self.render_field_by_name("ssh_port", cx))
                        .child(self.render_field_by_name("ssh_username", cx))
                })
                .when(!using_ssh_reference, |form| {
                    form.child(
                        field()
                            .label(self.field_label("ssh_auth_type"))
                            .items_center()
                            .child(h_flex().w_full().flex_wrap().gap_4().children(
                                SshAuthOption::ALL.iter().copied().map(|option| {
                                    Radio::new(format!("db-ssh-auth-{}", option.value()))
                                        .label(option.label())
                                        .checked(ssh_auth_type == option.value())
                                        .on_click(cx.listener(move |this, _, window, cx| {
                                            this.set_field_value(
                                                "ssh_auth_type",
                                                option.value(),
                                                window,
                                                cx,
                                            );
                                        }))
                                }),
                            )),
                    )
                    .when(ssh_auth_type == SshAuthOption::Password.value(), |form| {
                        form.child(self.render_field_by_name("ssh_password", cx))
                    })
                    .when(ssh_auth_type == SshAuthOption::PrivateKey.value(), |form| {
                        form.child(self.render_field_by_name("ssh_private_key_path", cx))
                            .child(self.render_field_by_name("ssh_private_key_passphrase", cx))
                    })
                    .when(
                        ssh_auth_type == SshAuthOption::PrivateKeyContent.value(),
                        |form| {
                            form.child(self.render_field_by_name("ssh_private_key_content", cx))
                                .child(self.render_field_by_name("ssh_private_key_passphrase", cx))
                        },
                    )
                })
                .child(self.render_field_by_name("ssh_target_host", cx))
                .child(self.render_field_by_name("ssh_target_port", cx))
            })
            .into_any_element()
    }

    fn render_ssl_tab_content(
        &self,
        kind: HostSslTabKind,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let ssl_enabled = self.is_ssl_enabled(kind, cx);

        v_form()
            .layout(Axis::Horizontal)
            .with_size(Size::Medium)
            .columns(1)
            .label_width(px(100.))
            .child(
                field()
                    .label(t!("ConnectionForm.require_ssl").to_string())
                    .items_center()
                    .child(
                        Checkbox::new("db-ssl-enabled")
                            .checked(ssl_enabled)
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.toggle_ssl_enabled(kind, window, cx);
                            })),
                    ),
            )
            .when(ssl_enabled, |form| match kind {
                HostSslTabKind::MySql => form
                    .child(self.render_field_by_name("verify_ca", cx))
                    .child(self.render_field_by_name("verify_identity", cx))
                    .child(self.render_field_by_name("ssl_root_cert_path", cx))
                    .child(self.render_field_by_name("tls_hostname_override", cx)),
                HostSslTabKind::PostgreSql => form
                    .child(self.render_field_by_name("ssl_mode", cx))
                    .child(self.render_field_by_name("ssl_root_cert_path", cx))
                    .child(self.render_field_by_name("ssl_accept_invalid_certs", cx))
                    .child(self.render_field_by_name("ssl_accept_invalid_hostnames", cx)),
                HostSslTabKind::Mssql => form
                    .child(self.render_field_by_name("encrypt", cx))
                    .child(self.render_field_by_name("trust_cert", cx)),
            })
            .into_any_element()
    }
}

impl EventEmitter<DbConnectionFormEvent> for DbConnectionForm {}

impl Focusable for DbConnectionForm {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for DbConnectionForm {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // Check if there's a pending file path to apply
        if let Some((field_name, path)) = self.pending_file_path.read(cx).clone() {
            self.set_field_value(&field_name, &path, window, cx);
            self.pending_file_path.update(cx, |p, _| *p = None);
        }

        // Calculate field input indices for current tab
        let mut field_input_offset = 0;
        for (tab_idx, tab_group) in self.config.tab_groups.iter().enumerate() {
            if tab_idx < self.active_tab {
                field_input_offset += tab_group.fields.len();
            }
        }

        let current_tab_group = &self.config.tab_groups[self.active_tab];
        let current_tab_fields = &current_tab_group.fields;
        let current_tab_name = current_tab_group.name.as_str();
        let tab_content = if current_tab_name == "ssh"
            && should_use_custom_ssh_tab(&self.config.db_type, current_tab_fields)
        {
            self.render_ssh_tab_content(window, cx)
        } else if current_tab_name == "ssl" {
            match host_ssl_tab_kind(&self.config.db_type, current_tab_fields) {
                Some(kind) => self.render_ssl_tab_content(kind, window, cx),
                None => self.render_standard_tab_content(
                    current_tab_fields,
                    field_input_offset,
                    window,
                    cx,
                ),
            }
        } else {
            self.render_standard_tab_content(current_tab_fields, field_input_offset, window, cx)
        };

        v_flex()
            .gap_4()
            .size_full()
            .child(
                // Tab bar
                div().flex().justify_center().child(
                    TabBar::new("connection-tabs")
                        .with_size(Size::Large)
                        .underline()
                        .selected_index(self.active_tab)
                        .on_click(cx.listener(|this, ix: &usize, window, cx| {
                            this.active_tab = *ix;
                            if this
                                .config
                                .tab_groups
                                .get(*ix)
                                .is_some_and(|tab| tab.name == "ssh")
                            {
                                this.sync_ssh_connection_selection(window, cx);
                            }
                            cx.notify();
                        }))
                        .children(
                            self.config
                                .tab_groups
                                .iter()
                                .map(|tab| Tab::new().label(tab.label.clone())),
                        ),
                ),
            )
            .child(
                // Form fields for active tab
                div()
                    .flex_1()
                    .min_h(px(250.))
                    .overflow_y_scrollbar()
                    .child(tab_content),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use db::plugin_manifest::FormValueCondition;
    use gpui::{TestAppContext, VisualTestContext, WindowOptions};
    use one_core::settings::AppSettings;
    use one_core::storage::{SshAuthMethod, SshParams};

    fn field_names(tab_group: &TabGroup) -> Vec<&str> {
        tab_group
            .fields
            .iter()
            .map(|field| field.name.as_str())
            .collect()
    }

    fn field_by_name<'a>(tab_group: &'a TabGroup, field_name: &str) -> &'a FormField {
        tab_group
            .fields
            .iter()
            .find(|field| field.name == field_name)
            .expect("field should exist")
    }

    #[test]
    fn simplify_connection_error_preserves_postgres_sqlstate() {
        let source = std::io::Error::other("password authentication failed for user postgres");
        let error = Error::new(db::DbError::connection_with_source(
            "failed to connect: SQLSTATE 28P01",
            source,
        ));

        assert_eq!(
            "SQLSTATE 28P01: password authentication failed for user postgres",
            DbConnectionForm::simplify_connection_error_message(&error)
        );
    }

    fn stored_ssh_connection(id: i64, name: &str, host: &str) -> StoredConnection {
        let mut connection = StoredConnection::new_ssh(
            name.to_string(),
            SshParams {
                sftp_default_directory: None,
                disabled_jump_server: None,
                sftp_account: None,
                host: host.to_string(),
                port: 22,
                username: "root".to_string(),
                auth_method: SshAuthMethod::Agent,
                credential_reference: None,
                prompt_username: None,
                prompt_password: None,
                keyboard_interactive: None,
                terminal_encoding: Default::default(),
                terminal_type: Default::default(),
                connect_timeout: None,
                keepalive_interval: None,
                keepalive_max: None,
                default_directory: None,
                init_script: None,
                disable_shell_integration: None,
                x11_forwarding: None,
                allow_legacy_algorithms: None,
                jump_server: None,
                proxy: None,
                os_id: None,
                icon: None,
                icon_file_path: None,
                account_expect: Default::default(),
            },
            None,
        );
        connection.id = Some(id);
        connection
    }

    #[test]
    fn ssh_connection_select_item_label_shows_name_and_host_not_id() {
        let connection = stored_ssh_connection(42, "Prod SSH", "10.0.0.5");
        let item = SshConnectionSelectItem::from_connection(&connection);

        assert_eq!("Prod SSH (10.0.0.5)", item.title().as_ref());
        assert_eq!(&Some(42), item.value());
        assert!(item.matches("10.0.0.5"));
        assert!(!item.matches("42"));
    }

    #[gpui::test]
    fn initial_connection_prefills_without_entering_edit_mode(cx: &mut TestAppContext) {
        cx.update(|cx| {
            cx.set_global(AppSettings::default());
            gpui_component::init(cx);
        });
        let params = DbConnectionConfig {
            database_type: DatabaseType::MySQL,
            name: "Imported DB".to_string(),
            host: "db.example.test".to_string(),
            port: 3306,
            username: "imported".to_string(),
            password: String::new(),
            credential_reference: None,
            database: None,
            service_name: None,
            sid: None,
            workspace_id: None,
            proxy: None,
            extra_params: HashMap::new(),
            id: String::new(),
        };
        let connection = StoredConnection::from_db_connection(params);
        assert_eq!(None, connection.id);

        let window = cx.update(|cx| {
            cx.open_window(WindowOptions::default(), |window, cx| {
                cx.new(|cx| {
                    let mut form = DbConnectionForm::new(DbFormConfig::mysql(), window, cx);
                    form.load_initial_connection(&connection, window, cx);
                    form
                })
            })
            .expect("form window should open")
        });
        let mut cx = VisualTestContext::from_window(window.into(), cx);
        cx.run_until_parked();
        let form = window.root(&mut cx).expect("form should be mounted");

        assert!(form.read_with(&cx, |form, _| form.editing_connection.is_none()));
        assert_eq!(
            Some("db.example.test".to_string()),
            form.read_with(&cx, |form, cx| form.get_field_value("host", cx))
        );
        let (stored, is_update) = form
            .read_with(&cx, |form, cx| form.build_stored_connection(cx))
            .expect("prefilled connection should be valid");
        assert!(!is_update);
        assert_eq!(None, stored.id);
    }

    #[gpui::test]
    fn editing_connection_restores_referenced_ssh_selection(cx: &mut TestAppContext) {
        cx.update(gpui_component::init);
        let ssh = stored_ssh_connection(42, "Prod SSH", "10.0.0.5");
        let params = DbConnectionConfig {
            database_type: DatabaseType::MySQL,
            name: "App DB".to_string(),
            host: "db.internal".to_string(),
            port: 3306,
            username: "app".to_string(),
            password: String::new(),
            credential_reference: None,
            database: None,
            service_name: None,
            sid: None,
            workspace_id: None,
            proxy: None,
            extra_params: HashMap::from([
                ("ssh_tunnel_enabled".to_string(), "true".to_string()),
                ("ssh_connection_id".to_string(), "42".to_string()),
            ]),
            id: String::new(),
        };
        let connection = StoredConnection::from_db_connection(params);
        let window = cx.update(|cx| {
            cx.open_window(WindowOptions::default(), |window, cx| {
                cx.new(|cx| {
                    let mut form = DbConnectionForm::new(DbFormConfig::mysql(), window, cx);
                    form.set_ssh_connections(vec![ssh], window, cx);
                    form.load_connection(&connection, window, cx);
                    form.active_tab = 3;
                    form
                })
            })
            .expect("form window should open")
        });
        let mut cx = VisualTestContext::from_window(window.into(), cx);
        cx.run_until_parked();
        let form = window.root(&mut cx).expect("form should be mounted");

        let selected = form.read_with(&cx, |form, cx| {
            form.ssh_connection_select
                .read(cx)
                .selected_value()
                .cloned()
                .flatten()
        });

        assert_eq!(Some(42), selected);
    }

    #[gpui::test]
    fn ssh_selection_uses_confirmed_event_value(cx: &mut TestAppContext) {
        cx.update(|cx| {
            cx.set_global(AppSettings::default());
            gpui_component::init(cx);
        });
        let window = cx.update(|cx| {
            cx.open_window(WindowOptions::default(), |window, cx| {
                cx.new(|cx| DbConnectionForm::new(DbFormConfig::mysql(), window, cx))
            })
            .expect("form window should open")
        });
        let mut cx = VisualTestContext::from_window(window.into(), cx);
        let form = window.root(&mut cx).expect("form should be mounted");
        let select = form.read_with(&cx, |form, _| form.ssh_connection_select.clone());

        select.update(&mut cx, |_, cx| {
            cx.emit(SelectEvent::Confirm(Some(Some(42))));
        });
        cx.run_until_parked();

        let state = form.read_with(&cx, |form, cx| {
            (
                form.selected_ssh_connection_id,
                form.get_field_value("ssh_connection_id", cx),
            )
        });
        assert_eq!((Some(42), Some("42".to_string())), state);
    }

    #[test]
    fn mysql_ssl_tab_exposes_expected_fields() {
        let config = DbFormConfig::mysql();
        let ssl_tab = config
            .tab_groups
            .iter()
            .find(|group| group.name == "ssl")
            .expect("MySQL should include the SSL tab");

        assert_eq!(
            field_names(ssl_tab),
            vec![
                "require_ssl",
                "verify_ca",
                "verify_identity",
                "ssl_root_cert_path",
                "tls_hostname_override"
            ]
        );
    }

    #[test]
    fn mysql_advanced_tab_exposes_charset_fields() {
        let config = DbFormConfig::mysql();
        let advanced_tab = config
            .tab_groups
            .iter()
            .find(|group| group.name == "advanced")
            .expect("MySQL should include the advanced tab");

        assert_eq!(
            field_names(advanced_tab),
            vec!["connect_timeout", "charset", "collation", "read_timeout"]
        );
    }

    #[test]
    fn mysql_form_keeps_connection_name_default_but_not_database_default() {
        let config = DbFormConfig::mysql();
        let general_tab = config
            .tab_groups
            .iter()
            .find(|group| group.name == "general")
            .expect("MySQL should include the general tab");

        assert_eq!(
            "Local MySQL",
            field_by_name(general_tab, "name").default_value
        );
        assert_eq!("", field_by_name(general_tab, "database").default_value);
    }

    #[test]
    fn oracle_form_omits_empty_ssl_tab() {
        let config = DbFormConfig::oracle();

        assert!(config.tab_groups.iter().all(|group| group.name != "ssl"));
    }

    #[test]
    fn oracle_form_offers_connect_role_select() {
        let config = DbFormConfig::oracle();
        let field = config
            .tab_groups
            .iter()
            .flat_map(|group| group.fields.iter())
            .find(|field| field.name == ORACLE_ROLE_PARAM)
            .expect("Oracle form should contain the role field");

        assert_eq!(field.field_type, FormFieldType::Select);
        let values: Vec<&str> = field
            .options
            .iter()
            .map(|(value, _)| value.as_str())
            .collect();
        assert_eq!(values, vec!["default", "sysdba", "sysoper"]);
        assert_eq!(field.default_value, "default");
    }

    #[test]
    fn oracle_driver_mode_maps_to_expected_database_type() {
        assert_eq!(
            DatabaseType::Oracle,
            database_type_for_oracle_driver_mode(&DatabaseType::Oracle, OracleDriverMode::Native)
        );
        assert_eq!(
            DatabaseType::external(ORACLE_GO_DRIVER_ID),
            database_type_for_oracle_driver_mode(&DatabaseType::Oracle, OracleDriverMode::Go)
        );
        assert_eq!(
            DatabaseType::MySQL,
            database_type_for_oracle_driver_mode(&DatabaseType::MySQL, OracleDriverMode::Go)
        );
        assert_eq!(
            DatabaseType::Oracle,
            database_type_for_oracle_driver_mode(
                &DatabaseType::external(ORACLE_GO_DRIVER_ID),
                OracleDriverMode::Native
            )
        );
    }

    #[test]
    fn oracle_driver_mode_restores_from_database_type() {
        assert_eq!(
            OracleDriverMode::Native,
            oracle_driver_mode_for_database_type(&DatabaseType::Oracle)
        );
        assert_eq!(
            OracleDriverMode::Go,
            oracle_driver_mode_for_database_type(&DatabaseType::external(ORACLE_GO_DRIVER_ID))
        );
        assert_eq!(
            OracleDriverMode::Native,
            oracle_driver_mode_for_database_type(&DatabaseType::MySQL)
        );
    }

    #[test]
    fn external_driver_host_ssh_fields_use_custom_ssh_tab() {
        let ssh_tab = DbFormConfig::mysql()
            .tab_groups
            .into_iter()
            .find(|group| group.name == "ssh")
            .expect("MySQL should include the SSH tab");

        assert!(should_use_custom_ssh_tab(
            &DatabaseType::external("iotdb"),
            &ssh_tab.fields
        ));
    }

    #[test]
    fn external_driver_custom_ssh_fields_use_standard_tab() {
        let fields = vec![FormField::new(
            "driver_ssh_endpoint",
            "Driver SSH endpoint",
            FormFieldType::Text,
        )];

        assert!(!should_use_custom_ssh_tab(
            &DatabaseType::external("iotdb"),
            &fields
        ));
    }

    #[test]
    fn field_visibility_rules_follow_current_values() {
        let mut field = FormField::new("ssl_ca_file", "CA", FormFieldType::FilePath);
        field.visible_when = vec![FormVisibilityRule {
            when_field: "ssl_enabled".to_string(),
            condition: FormValueCondition::Equals("true".to_string()),
        }];

        assert!(!field_visible_from_values(&field, |_| Some(
            "false".to_string()
        )));
        assert!(field_visible_from_values(&field, |_| Some(
            "true".to_string()
        )));
    }

    #[test]
    fn ssh_field_group_keeps_expected_storage_keys() {
        let config = DbFormConfig::mysql();
        let ssh_tab = config
            .tab_groups
            .iter()
            .find(|group| group.name == "ssh")
            .expect("MySQL should include the SSH tab");

        assert_eq!(
            field_names(ssh_tab),
            vec![
                "ssh_tunnel_enabled",
                "ssh_connection_id",
                "ssh_host",
                "ssh_port",
                "ssh_username",
                "ssh_auth_type",
                "ssh_password",
                "ssh_private_key_path",
                "ssh_private_key_content",
                "ssh_private_key_passphrase",
                "ssh_target_host",
                "ssh_target_port"
            ]
        );
    }

    #[test]
    fn private_key_content_auth_requires_pasted_key_body() {
        assert_eq!(
            Some("ssh_private_key_content"),
            missing_ssh_tunnel_required_field(
                true,
                "jump.example.com",
                "root",
                "private_key_content",
                "",
                "",
                "",
            )
        );
        assert_eq!(
            None,
            missing_ssh_tunnel_required_field(
                true,
                "jump.example.com",
                "root",
                "private_key_content",
                "",
                "-----BEGIN OPENSSH PRIVATE KEY-----",
                "",
            )
        );
    }

    #[test]
    fn private_key_material_alias_uses_private_key_content_auth() {
        assert_eq!(
            "private_key_content",
            normalized_ssh_auth_type("private_key_material")
        );
    }

    #[test]
    fn pageant_auth_type_is_preserved() {
        assert_eq!("pageant", normalized_ssh_auth_type(" Pageant "));
    }

    #[test]
    fn custom_ssl_enabled_matches_database_semantics() {
        assert!(is_custom_ssl_enabled(
            HostSslTabKind::MySql,
            true,
            None,
            None
        ));
        assert!(!is_custom_ssl_enabled(
            HostSslTabKind::MySql,
            false,
            None,
            None
        ));

        assert!(is_custom_ssl_enabled(
            HostSslTabKind::PostgreSql,
            false,
            Some("prefer"),
            None,
        ));
        assert!(!is_custom_ssl_enabled(
            HostSslTabKind::PostgreSql,
            false,
            Some("disable"),
            None,
        ));

        assert!(is_custom_ssl_enabled(
            HostSslTabKind::Mssql,
            false,
            None,
            Some("required"),
        ));
        assert!(!is_custom_ssl_enabled(
            HostSslTabKind::Mssql,
            false,
            None,
            Some("off"),
        ));
    }

    #[test]
    fn external_driver_host_ssl_fields_use_custom_ssl_tab() {
        let ssl_tab = DbFormConfig::postgres()
            .tab_groups
            .into_iter()
            .find(|group| group.name == "ssl")
            .expect("PostgreSQL should include the SSL tab");

        assert_eq!(
            Some(HostSslTabKind::PostgreSql),
            host_ssl_tab_kind(&DatabaseType::external("opengauss"), &ssl_tab.fields)
        );
    }

    #[test]
    fn external_driver_custom_ssl_fields_use_standard_tab() {
        let fields = vec![FormField::new(
            "driver_ssl_profile",
            "Driver SSL profile",
            FormFieldType::Text,
        )];

        assert_eq!(
            None,
            host_ssl_tab_kind(&DatabaseType::external("opengauss"), &fields)
        );
    }

    #[test]
    fn ssh_agent_auth_does_not_require_password() {
        assert_eq!(
            missing_ssh_tunnel_required_field(
                true,
                "jump.example.com",
                "root",
                "agent",
                "",
                "",
                ""
            ),
            None
        );
    }

    #[test]
    fn ssh_pageant_auth_does_not_require_password() {
        assert_eq!(
            missing_ssh_tunnel_required_field(
                true,
                "jump.example.com",
                "root",
                "pageant",
                "",
                "",
                ""
            ),
            None
        );
    }

    #[test]
    fn ssh_password_auth_still_requires_password() {
        assert_eq!(
            missing_ssh_tunnel_required_field(
                true,
                "jump.example.com",
                "root",
                "password",
                "",
                "",
                "",
            ),
            Some("ssh_password")
        );
    }
}
