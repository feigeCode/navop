//! TDengine 插件实现:SHOW DATABASES / SHOW TABLES + SHOW STABLES / DESCRIBE 元数据,
//! SQL 方言与 MySQL 对齐(反引号标识符、LIMIT n OFFSET m)。

use crate::types::ObjectViewColumn as Column;
use anyhow::Result;
use one_core::storage::{DatabaseType, DbConnectionConfig};
use rust_i18n::t;
use std::collections::HashMap;
use std::sync::LazyLock;

use crate::connection::{DbConnection, DbError};
use crate::executor::SqlResult;
use crate::import_export::{
    ExportConfig, ExportProgressSender, ExportResult, ImportConfig, ImportProgressSender,
    ImportResult,
};
use crate::manifest_helpers::{
    DatabaseActionDescriptorExt, action, action_with_scope, field, option, ssh_auth_options,
    ssh_auth_rules, ssh_enabled_rules, ssh_field, ssh_number_field, ssh_password_field, tab,
    yes_no_options,
};
use crate::plugin::{DatabaseOperationRequest, DatabasePlugin, SqlCompletionInfo};
use crate::plugin_manifest::{
    DatabaseActionId, DatabaseActionManifest, DatabaseActionPlacement, DatabaseActionToolbarScope,
    DatabaseCapabilities, DatabaseFormFieldType, DatabaseFormKind, DatabaseFormManifest,
    DatabaseUiCapabilities, DatabaseUiManifest,
};
use crate::tdengine::connection::TdengineDbConnection;
use crate::types::*;

/// TDengine 数据类型(名称, 描述),用于补全与表设计器。
pub const TDENGINE_DATA_TYPES: &[(&str, &str)] = &[
    ("TIMESTAMP", "时间戳类型(时间序列主键列)"),
    ("INT", "32 位有符号整数"),
    ("INT UNSIGNED", "32 位无符号整数"),
    ("BIGINT", "64 位有符号整数"),
    ("BIGINT UNSIGNED", "64 位无符号整数"),
    ("SMALLINT", "16 位有符号整数"),
    ("SMALLINT UNSIGNED", "16 位无符号整数"),
    ("TINYINT", "8 位有符号整数"),
    ("TINYINT UNSIGNED", "8 位无符号整数"),
    ("FLOAT", "32 位单精度浮点数"),
    ("DOUBLE", "64 位双精度浮点数"),
    ("BOOL", "布尔类型"),
    ("BINARY(n)", "变长字节串(原生字节)"),
    ("VARCHAR(n)", "变长字符串(BINARY 别名)"),
    ("NCHAR(n)", "变长 Unicode 字符串"),
    ("JSON", "JSON 标签类型(仅超级表标签)"),
    ("VARBINARY(n)", "变长二进制数据"),
];

/// TDengine 内置数据库,在库列表中隐藏。
const TDENGINE_SYSTEM_DATABASES: &[&str] = &["information_schema", "performance_schema"];

/// TDengine 数据库插件(无状态)。
#[derive(Default)]
pub struct TdenginePlugin;

static TDENGINE_UI_MANIFEST: LazyLock<DatabaseUiManifest> =
    LazyLock::new(build_tdengine_ui_manifest);

impl TdenginePlugin {
    pub fn new() -> Self {
        Self
    }

    /// 执行查询并返回所有行;失败时返回 anyhow 错误。
    async fn query_rows(
        connection: &dyn DbConnection,
        sql: &str,
        context: &str,
    ) -> Result<Vec<Vec<Option<String>>>> {
        match connection.query(sql).await? {
            SqlResult::Query(query_result) => Ok(query_result.rows),
            SqlResult::Error(error) => {
                Err(anyhow::anyhow!("Failed to {}: {}", context, error.message))
            }
            SqlResult::Exec(_) => Err(anyhow::anyhow!("{} did not return a result set", context)),
        }
    }

    /// 执行查询并提取首列文本;失败时返回 anyhow 错误。
    async fn first_column_values(
        connection: &dyn DbConnection,
        sql: &str,
        context: &str,
    ) -> Result<Vec<String>> {
        let rows = Self::query_rows(connection, sql, context).await?;
        Ok(rows
            .into_iter()
            .filter_map(|row| row.first().and_then(|value| value.clone()))
            .collect())
    }
}

fn build_tdengine_ui_manifest() -> DatabaseUiManifest {
    DatabaseUiManifest {
        capabilities: DatabaseUiCapabilities {
            // TDengine 不支持视图/二级索引/存储过程/函数/触发器/序列等对象,
            // 这些能力统一关闭,避免树中出现空目录。
            supports_views: false,
            supports_indexes: false,
            supports_functions: false,
            supports_procedures: false,
            supports_triggers: false,
            supports_sequences: false,
            ..DatabaseUiCapabilities::default()
        },
        forms: vec![
            tdengine_connection_form(),
            tdengine_database_form(false),
            tdengine_database_form(true),
        ],
        actions: tdengine_action_manifest(),
        ..DatabaseUiManifest::default()
    }
}

fn tdengine_connection_form() -> DatabaseFormManifest {
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
                    .with_placeholder("My TDengine Database")
                    .with_default("Local TDengine"),
                    field("host", "ConnectionForm.host", DatabaseFormFieldType::Text)
                        .with_placeholder("localhost")
                        .with_default("localhost"),
                    field("port", "ConnectionForm.port", DatabaseFormFieldType::Number)
                        .with_placeholder("6041 (taosAdapter port)")
                        .with_default("6041"),
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
                    .with_placeholder("taosdata"),
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
                ],
            ),
            tab(
                "ssl",
                "ConnectionForm.ssl",
                vec![
                    field(
                        "schema",
                        "ConnectionForm.schema",
                        DatabaseFormFieldType::Select,
                    )
                    .optional()
                    .with_default("ws")
                    .with_options(vec![
                        option("ws", "ConnectionForm.schema_ws"),
                        option("wss", "ConnectionForm.schema_wss"),
                    ]),
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
                    .with_options(ssh_auth_options())
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
                        .with_placeholder("6041"),
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
                    .with_placeholder("ConnectionForm.enter_remark")
                    .with_default(""),
                ],
            ),
        ],
    }
}

fn tdengine_database_form(is_edit_mode: bool) -> DatabaseFormManifest {
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
            ],
        )],
    }
}

fn tdengine_action_manifest() -> DatabaseActionManifest {
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
                DatabaseActionId::DeleteDatabase,
                "Database.delete_database",
                vec![DbNodeType::Database],
                DatabaseActionPlacement::Both,
                false,
                Some(DatabaseActionToolbarScope::SelectedRow),
            ),
            action(
                DatabaseActionId::CloseDatabase,
                "Database.close_database",
                vec![DbNodeType::Database],
                DatabaseActionPlacement::ContextMenu,
            )
            .always_enabled(),
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
            action(
                DatabaseActionId::CreateNewQuery,
                "Query.new_query",
                vec![DbNodeType::Database, DbNodeType::QueriesFolder],
                DatabaseActionPlacement::ContextMenu,
            ),
            action_with_scope(
                DatabaseActionId::CreateNewQuery,
                "Query.new_query",
                vec![DbNodeType::QueriesFolder, DbNodeType::NamedQuery],
                DatabaseActionPlacement::Toolbar,
                true,
                Some(DatabaseActionToolbarScope::CurrentNode),
            ),
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
            action(
                DatabaseActionId::RevealQueryInFileManager,
                "Query.reveal_in_file_manager",
                vec![DbNodeType::NamedQuery],
                DatabaseActionPlacement::Both,
            )
            .with_toolbar_scope(DatabaseActionToolbarScope::SelectedRow),
        ],
    }
}

#[async_trait::async_trait]
impl DatabasePlugin for TdenginePlugin {
    fn name(&self) -> DatabaseType {
        DatabaseType::TDengine
    }

    fn quote_identifier(&self, identifier: &str) -> String {
        format!("`{}`", identifier.replace('`', "``"))
    }

    fn capabilities(&self) -> DatabaseCapabilities {
        DatabaseUiCapabilities {
            supports_views: false,
            supports_indexes: false,
            supports_functions: false,
            supports_procedures: false,
            table_engines: self.engines(),
            ..DatabaseUiCapabilities::default()
        }
    }

    fn ui_manifest(&self) -> DatabaseUiManifest {
        TDENGINE_UI_MANIFEST.clone()
    }

    fn get_completion_info(&self) -> SqlCompletionInfo {
        SqlCompletionInfo {
            keywords: vec![
                ("STABLE", "超级表"),
                ("TAGS", "超级表标签定义"),
                ("USING", "按超级表创建子表"),
                ("INTERVAL", "时间窗口聚合间隔"),
                ("SLIDING", "窗口滑动步长"),
                ("FILL", "窗口空值填充策略"),
                ("PARTITION BY", "按标签/时间分区"),
                ("SESSION", "会话窗口"),
                ("STATE_WINDOW", "状态窗口"),
                ("EVENT_WINDOW", "事件窗口"),
                ("ORDER BY", "结果排序"),
                ("SLIMIT", "分组分页"),
                ("SOFFSET", "分组分页偏移"),
                ("KEEP", "数据保留时长"),
                ("PRECISION", "时间戳精度"),
            ],
            functions: vec![
                ("NOW()", "当前时间戳"),
                ("TODAY()", "今日零点"),
                ("TIMEZONE()", "当前时区"),
                ("SERVER_VERSION()", "服务端版本"),
                ("SERVER_STATUS()", "服务端状态"),
                ("DATABASE()", "当前数据库"),
                ("FIRST(col)", "时间序列最早值"),
                ("LAST(col)", "时间序列最新值"),
                ("LAST_ROW(col)", "最后一行(非缓存)"),
                ("TWA(col)", "时间加权平均"),
                ("IRATE(col)", "瞬时速率"),
                ("DERIVATIVE(col)", "一阶导数"),
                ("DIFF(col)", "相邻差值"),
                ("TAIL(col, k)", "最后 k 行"),
                ("UNIQUE(col)", "去重值"),
                ("STATECOUNT(col, ...)", "连续满足条件时长计数"),
                ("DURATION(col, ...)", "连续满足条件时长"),
                ("ELAPSED(col, ...)", "覆盖时长"),
                ("CSUM(col)", "累计求和"),
                ("MLEN(col, k)", "滑动最小值"),
                ("ROUND(col, d)", "四舍五入"),
                ("TO_TIMESTAMP(ms)", "毫秒转时间戳"),
                ("TO_ISO8601(ts)", "时间戳转 ISO8601"),
                ("CAST(expr AS type)", "类型转换"),
            ],
            operators: vec![
                ("IN", "集合匹配"),
                ("NOT IN", "集合排除"),
                ("LIKE", "通配符匹配"),
                ("MATCH", "正则匹配"),
                ("NMATCH", "正则不匹配"),
                ("CONTAINS", "包含子串"),
            ],
            data_types: TDENGINE_DATA_TYPES.to_vec(),
            snippets: vec![
                (
                    "stb",
                    "CREATE STABLE $1 (\n  ts TIMESTAMP,\n  $2\n) TAGS ($3)",
                    "创建超级表",
                ),
                (
                    "ctb",
                    "CREATE TABLE $1 USING $2 TAGS ($3)",
                    "按超级表创建子表",
                ),
                (
                    "win",
                    "SELECT _wstart, COUNT(*) FROM $1\nWHERE ts >= $2\nINTERVAL($3)",
                    "时间窗口聚合",
                ),
            ],
        }
        .with_standard_sql()
    }

    async fn create_connection(
        &self,
        config: DbConnectionConfig,
    ) -> Result<Box<dyn DbConnection + Send + Sync>, DbError> {
        let mut conn = TdengineDbConnection::new(config);
        conn.connect().await?;
        Ok(Box::new(conn))
    }

    // === 库级操作 ===

    async fn list_databases(&self, connection: &dyn DbConnection) -> Result<Vec<String>> {
        let rows = Self::query_rows(connection, "SHOW DATABASES", "list databases").await?;
        Ok(rows
            .into_iter()
            .filter_map(|row| row.first().and_then(|value| value.clone()))
            .filter(|name| {
                !TDENGINE_SYSTEM_DATABASES
                    .iter()
                    .any(|system| name.eq_ignore_ascii_case(system))
            })
            .collect())
    }

    async fn list_databases_view(&self, connection: &dyn DbConnection) -> Result<ObjectView> {
        let databases = self.list_databases_detailed(connection).await?;

        let columns = vec![
            Column::localized("name", "ObjectView.columns.name").width(220.0),
            Column::localized("engine", "ObjectView.columns.engine").width(120.0),
        ];

        let rows: Vec<Vec<String>> = databases
            .iter()
            .map(|db| {
                vec![
                    db.name.clone(),
                    db.charset.as_deref().unwrap_or("-").to_string(),
                ]
            })
            .collect();

        Ok(ObjectView {
            db_node_type: DbNodeType::Database,
            title: t!("ObjectView.titles.databases").to_string(),
            columns,
            rows,
        })
    }

    async fn list_databases_detailed(
        &self,
        connection: &dyn DbConnection,
    ) -> Result<Vec<DatabaseInfo>> {
        let rows = Self::query_rows(connection, "SHOW DATABASES", "list databases").await?;
        Ok(rows
            .into_iter()
            .filter_map(|row| row.first().and_then(|value| value.clone()))
            .filter(|name| {
                !TDENGINE_SYSTEM_DATABASES
                    .iter()
                    .any(|system| name.eq_ignore_ascii_case(system))
            })
            .map(|name| DatabaseInfo {
                name,
                // TDengine 无库引擎概念,统一展示为 TDengine。
                charset: Some("TDengine".to_string()),
                collation: None,
                size: None,
                table_count: None,
                comment: None,
            })
            .collect())
    }

    fn sql_dialect(&self) -> Box<dyn sqlparser::dialect::Dialect> {
        // TDengine SQL 语法与 MySQL 方言最接近(反引号引用、LIMIT n OFFSET m)。
        Box::new(sqlparser::dialect::MySqlDialect {})
    }

    // === 表操作 ===

    async fn list_tables(
        &self,
        connection: &dyn DbConnection,
        database: &str,
        _schema: Option<String>,
    ) -> Result<Vec<TableInfo>> {
        let db = self.quote_identifier(database);
        let mut tables = Vec::new();

        // 普通表(含子表)。
        let sql = format!("SHOW {db}.TABLES");
        for name in Self::first_column_values(connection, &sql, "list tables").await? {
            tables.push(TableInfo {
                name,
                object_type: TableObjectType::Table,
                schema: None,
                create_time: None,
                charset: None,
                collation: None,
                engine: None,
                comment: None,
            });
        }

        // 超级表,以 engine 字段区分展示。
        let sql = format!("SHOW {db}.STABLES");
        for name in Self::first_column_values(connection, &sql, "list stables").await? {
            tables.push(TableInfo {
                name,
                object_type: TableObjectType::Table,
                schema: None,
                create_time: None,
                charset: None,
                collation: None,
                engine: Some("STABLE".to_string()),
                comment: None,
            });
        }

        tables.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(tables)
    }

    async fn list_tables_view(
        &self,
        connection: &dyn DbConnection,
        database: &str,
        schema: Option<String>,
    ) -> Result<ObjectView> {
        let tables = self.list_tables(connection, database, schema).await?;

        let columns = vec![
            Column::localized("name", "ObjectView.columns.name").width(220.0),
            Column::localized("engine", "ObjectView.columns.engine").width(140.0),
        ];

        let rows: Vec<Vec<String>> = tables
            .iter()
            .map(|table| {
                vec![
                    table.name.clone(),
                    table.engine.clone().unwrap_or_else(|| "-".to_string()),
                ]
            })
            .collect();

        Ok(ObjectView {
            db_node_type: DbNodeType::Table,
            title: t!("ObjectView.titles.tables").to_string(),
            columns,
            rows,
        })
    }

    async fn list_columns(
        &self,
        connection: &dyn DbConnection,
        database: &str,
        _schema: Option<String>,
        table: &str,
    ) -> Result<Vec<ColumnInfo>> {
        // DESCRIBE 返回 field/type/length/note,note 为 TAG 时表示超级表标签列。
        let sql = format!(
            "DESCRIBE {}.{}",
            self.quote_identifier(database),
            self.quote_identifier(table)
        );
        let rows = Self::query_rows(connection, &sql, "list columns").await?;

        let mut columns = Vec::new();
        for row in rows {
            let Some(name) = row.first().and_then(|value| value.clone()) else {
                continue;
            };
            let raw_type = row
                .get(1)
                .and_then(|value| value.clone())
                .unwrap_or_default();
            let length = row
                .get(2)
                .and_then(|value| value.clone())
                .and_then(|value| value.trim().parse::<u32>().ok())
                .unwrap_or(0);
            let note = row.get(3).and_then(|value| value.clone());

            // 变长类型补上宽度,例如 BINARY(16),与 DESCRIBE 语义保持一致。
            let data_type = if length > 0 && tdengine_type_takes_width(&raw_type) {
                format!("{}({})", raw_type.to_uppercase(), length)
            } else {
                raw_type.to_uppercase()
            };

            columns.push(ColumnInfo {
                name,
                data_type,
                // TDengine 普通列均可为 NULL(时间戳列除外),按可空处理。
                is_nullable: true,
                is_primary_key: false,
                default_value: None,
                // note 列为 TAG 时标记为标签列。
                comment: note,
                charset: None,
                collation: None,
            });
        }

        Ok(columns)
    }

    async fn list_columns_view(
        &self,
        connection: &dyn DbConnection,
        database: &str,
        schema: Option<String>,
        table: &str,
    ) -> Result<ObjectView> {
        let columns = self
            .list_columns(connection, database, schema, table)
            .await?;

        let column_defs = vec![
            Column::localized("name", "ObjectView.columns.name").width(180.0),
            Column::localized("type", "ObjectView.columns.type").width(180.0),
            Column::localized("comment", "ObjectView.columns.comment").width(160.0),
        ];

        let rows: Vec<Vec<String>> = columns
            .iter()
            .map(|col| {
                vec![
                    col.name.clone(),
                    col.data_type.clone(),
                    col.comment.as_deref().unwrap_or("").to_string(),
                ]
            })
            .collect();

        Ok(ObjectView {
            db_node_type: DbNodeType::Column,
            title: t!("ObjectView.titles.columns").to_string(),
            columns: column_defs,
            rows,
        })
    }

    async fn list_indexes(
        &self,
        _connection: &dyn DbConnection,
        _database: &str,
        _schema: Option<String>,
        _table: &str,
    ) -> Result<Vec<IndexInfo>> {
        // TDengine 经典模型无二级索引。
        Ok(Vec::new())
    }

    async fn list_indexes_view(
        &self,
        _connection: &dyn DbConnection,
        _database: &str,
        _schema: Option<&str>,
        _table: &str,
    ) -> Result<ObjectView> {
        Ok(ObjectView {
            db_node_type: DbNodeType::Index,
            title: t!("ObjectView.counts.indexes", count = 0).to_string(),
            columns: vec![Column::localized("name", "ObjectView.columns.name").width(200.0)],
            rows: Vec::new(),
        })
    }

    // === 视图/函数/存储过程/触发器/序列:TDengine 不支持,统一返回空 ===

    async fn list_views(
        &self,
        _connection: &dyn DbConnection,
        _database: &str,
        _schema: Option<String>,
    ) -> Result<Vec<ViewInfo>> {
        Ok(Vec::new())
    }

    async fn list_views_view(
        &self,
        _connection: &dyn DbConnection,
        _database: &str,
    ) -> Result<ObjectView> {
        Ok(ObjectView {
            db_node_type: DbNodeType::View,
            title: t!("ObjectView.titles.views").to_string(),
            columns: vec![Column::localized("name", "ObjectView.columns.name").width(200.0)],
            rows: Vec::new(),
        })
    }

    async fn list_functions(
        &self,
        _connection: &dyn DbConnection,
        _database: &str,
    ) -> Result<Vec<FunctionInfo>> {
        Ok(Vec::new())
    }

    async fn list_functions_view(
        &self,
        _connection: &dyn DbConnection,
        _database: &str,
    ) -> Result<ObjectView> {
        Ok(ObjectView {
            db_node_type: DbNodeType::Function,
            title: t!("ObjectView.titles.functions").to_string(),
            columns: vec![Column::localized("name", "ObjectView.columns.name").width(200.0)],
            rows: Vec::new(),
        })
    }

    async fn list_procedures(
        &self,
        _connection: &dyn DbConnection,
        _database: &str,
    ) -> Result<Vec<FunctionInfo>> {
        Ok(Vec::new())
    }

    async fn list_procedures_view(
        &self,
        _connection: &dyn DbConnection,
        _database: &str,
    ) -> Result<ObjectView> {
        Ok(ObjectView {
            db_node_type: DbNodeType::Procedure,
            title: t!("ObjectView.titles.procedures").to_string(),
            columns: vec![Column::localized("name", "ObjectView.columns.name").width(200.0)],
            rows: Vec::new(),
        })
    }

    async fn list_triggers(
        &self,
        _connection: &dyn DbConnection,
        _database: &str,
    ) -> Result<Vec<TriggerInfo>> {
        Ok(Vec::new())
    }

    async fn list_triggers_view(
        &self,
        _connection: &dyn DbConnection,
        _database: &str,
    ) -> Result<ObjectView> {
        Ok(ObjectView {
            db_node_type: DbNodeType::Trigger,
            title: t!("ObjectView.titles.triggers").to_string(),
            columns: vec![Column::localized("name", "ObjectView.columns.name").width(200.0)],
            rows: Vec::new(),
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
        Ok(ObjectView {
            db_node_type: DbNodeType::Sequence,
            title: t!("ObjectView.titles.sequences").to_string(),
            columns: vec![Column::localized("name", "ObjectView.columns.name").width(200.0)],
            rows: Vec::new(),
        })
    }

    fn build_column_definition(&self, column: &ColumnInfo, include_name: bool) -> String {
        let mut def = String::new();
        if include_name {
            def.push_str(&self.quote_identifier(&column.name));
            def.push(' ');
        }
        // TDengine 不支持列注释/默认值语法,仅输出名称 + 类型。
        def.push_str(&column.data_type.to_uppercase());
        def
    }

    // === 库管理 ===

    fn build_create_database_sql(&self, request: &DatabaseOperationRequest) -> String {
        format!(
            "CREATE DATABASE {}",
            self.quote_identifier(&request.database_name)
        )
    }

    fn build_modify_database_sql(&self, request: &DatabaseOperationRequest) -> String {
        // TDengine 的 ALTER DATABASE 需要显式选项(KEEP/PRECISION 等),表单未收集,
        // 这里输出注释提示手动调整。
        format!(
            "-- TDengine: use `ALTER DATABASE {} ...` to adjust options",
            request.database_name
        )
    }

    fn build_drop_database_sql(&self, database_name: &str) -> String {
        format!(
            "DROP DATABASE IF EXISTS {}",
            self.quote_identifier(database_name)
        )
    }

    async fn load_table_children(
        &self,
        connection: &dyn DbConnection,
        node: &DbNode,
        id: &str,
    ) -> Result<Vec<DbNode>> {
        let database = &*node
            .get_database_name()
            .ok_or_else(|| anyhow::anyhow!("Database name not found"))?;
        let schema = node.get_schema_name();
        let table = &*node
            .get_table_name()
            .ok_or_else(|| anyhow::anyhow!("Table name not found"))?;

        let mut folder_metadata: HashMap<String, String> = node.metadata.clone();
        folder_metadata.insert("table".to_string(), table.to_string());

        // TDengine 表下仅有列目录(无索引/外键/触发器/约束目录)。
        let columns = self
            .list_columns(connection, database, schema, table)
            .await?;

        Ok(vec![
            self.build_table_subfolder(
                node,
                id,
                "columns_folder",
                "DbTree.Columns",
                DbNodeType::ColumnsFolder,
                &folder_metadata,
                columns
                    .into_iter()
                    .map(|column| {
                        (column.name.clone(), DbNodeType::Column, {
                            let mut metadata = folder_metadata.clone();
                            metadata.insert("type".to_string(), column.data_type);
                            metadata.insert("is_nullable".to_string(), "true".to_string());
                            metadata.insert("is_primary_key".to_string(), "false".to_string());
                            metadata
                        })
                    })
                    .collect(),
            ),
        ])
    }

    fn build_limit_clause(&self) -> String {
        " LIMIT 1".to_string()
    }

    fn build_where_and_limit_clause(
        &self,
        request: &TableSaveRequest,
        original_data: &[TableCellValue],
    ) -> (String, String) {
        let where_clause = self.build_table_change_where_clause(request, original_data);
        (where_clause, self.build_limit_clause())
    }

    fn get_data_types(&self) -> &[(&'static str, &'static str)] {
        TDENGINE_DATA_TYPES
    }

    fn rename_table(&self, database: &str, old_name: &str, new_name: &str) -> String {
        format!(
            "ALTER TABLE {}.{} RENAME TO {}",
            self.quote_identifier(database),
            self.quote_identifier(old_name),
            self.quote_identifier(new_name)
        )
    }

    fn build_backup_table_sql(
        &self,
        _database: &str,
        _schema: Option<&str>,
        source_table: &str,
        _target_table: &str,
    ) -> String {
        // TDengine 不支持 CREATE TABLE ... AS SELECT 形式的整表备份。
        format!(
            "-- TDengine does not support one-statement table backup for '{source_table}', create the target table first and then INSERT INTO ... SELECT"
        )
    }

    fn build_column_def(&self, col: &ColumnDefinition) -> String {
        let mut def = String::new();
        def.push_str(&self.quote_identifier(&col.name));
        def.push(' ');

        let mut type_str = self.build_type_string(col).to_uppercase();
        if col.is_unsigned && !type_str.contains(" UNSIGNED") {
            type_str.push_str(" UNSIGNED");
        }
        def.push_str(&type_str);

        def
    }

    fn build_create_table_sql(&self, design: &TableDesign) -> String {
        let mut sql = String::new();
        sql.push_str("CREATE TABLE ");
        sql.push_str(&self.quote_identifier(&design.table_name));
        sql.push_str(" (\n");

        let definitions: Vec<String> = design
            .columns
            .iter()
            .map(|col| format!("  {}", self.build_column_def(col)))
            .collect();
        sql.push_str(&definitions.join(",\n"));
        sql.push_str("\n);");

        sql
    }

    fn build_alter_table_sql(&self, original: &TableDesign, new: &TableDesign) -> String {
        let mut statements: Vec<String> = Vec::new();
        let table_name = self.quote_identifier(&new.table_name);

        let original_cols: HashMap<&str, &ColumnDefinition> = original
            .columns
            .iter()
            .map(|col| (col.name.as_str(), col))
            .collect();
        let new_cols: HashMap<&str, &ColumnDefinition> = new
            .columns
            .iter()
            .map(|col| (col.name.as_str(), col))
            .collect();

        for name in original_cols.keys() {
            if !new_cols.contains_key(name) {
                statements.push(format!(
                    "ALTER TABLE {} DROP COLUMN {};",
                    table_name,
                    self.quote_identifier(name)
                ));
            }
        }

        for col in new.columns.iter() {
            if let Some(orig_col) = original_cols.get(col.name.as_str()) {
                if self.column_changed(orig_col, col) {
                    // TDengine 仅修改变长列的宽度,统一输出 MODIFY COLUMN。
                    let type_str = self.build_type_string(col).to_uppercase();
                    statements.push(format!(
                        "ALTER TABLE {} MODIFY COLUMN {} {};",
                        table_name,
                        self.quote_identifier(&col.name),
                        type_str
                    ));
                }
            } else {
                let col_def = self.build_column_def(col);
                statements.push(format!(
                    "ALTER TABLE {} ADD COLUMN {};",
                    table_name, col_def
                ));
            }
        }

        if statements.is_empty() {
            "-- No changes detected".to_string()
        } else {
            statements.join("\n")
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

/// 判断 DESCRIBE 输出的类型是否需要附带宽度展示。
fn tdengine_type_takes_width(raw_type: &str) -> bool {
    let base = raw_type
        .split('(')
        .next()
        .unwrap_or(raw_type)
        .trim()
        .to_ascii_uppercase();
    matches!(
        base.as_str(),
        "BINARY" | "VARCHAR" | "NCHAR" | "VARBINARY" | "GEOMETRY"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_plugin_name() {
        let plugin = TdenginePlugin::new();
        assert_eq!(plugin.name(), DatabaseType::TDengine);
    }

    #[test]
    fn test_quote_identifier() {
        let plugin = TdenginePlugin::new();
        assert_eq!(plugin.quote_identifier("orders"), "`orders`");
        assert_eq!(plugin.quote_identifier("my`db"), "`my``db`");
    }

    #[test]
    fn test_build_limit_clause() {
        let plugin = TdenginePlugin::new();
        assert_eq!(plugin.build_limit_clause(), " LIMIT 1");
    }

    #[test]
    fn test_format_pagination_uses_limit_offset() {
        // 默认实现即 LIMIT n OFFSET m,与 TDengine 语法一致。
        let plugin = TdenginePlugin::new();
        assert_eq!(plugin.format_pagination(10, 20, ""), " LIMIT 10 OFFSET 20");
    }

    #[test]
    fn test_ui_manifest_default_port_and_username() {
        let manifest = TdenginePlugin::new().ui_manifest();
        let connection_form = manifest
            .forms
            .iter()
            .find(|form| form.kind == DatabaseFormKind::Connection)
            .expect("connection form should exist");
        let general = connection_form
            .tabs
            .iter()
            .find(|tab| tab.id == "general")
            .expect("general tab should exist");

        let field_default = |field_id: &str| {
            general
                .fields
                .iter()
                .find(|field| field.id == field_id)
                .and_then(|field| field.default_value.clone())
        };

        // 端口默认 6041(taosAdapter),用户名默认 root。
        assert_eq!(field_default("port").as_deref(), Some("6041"));
        assert_eq!(field_default("username").as_deref(), Some("root"));
    }

    #[test]
    fn test_capabilities_disable_unsupported_objects() {
        let capabilities = TdenginePlugin::new().capabilities();
        assert!(!capabilities.supports_views);
        assert!(!capabilities.supports_indexes);
        assert!(!capabilities.supports_functions);
        assert!(!capabilities.supports_procedures);
    }

    #[test]
    fn test_drop_database_sql() {
        let plugin = TdenginePlugin::new();
        assert_eq!(
            plugin.build_drop_database_sql("log_db"),
            "DROP DATABASE IF EXISTS `log_db`"
        );
    }

    #[test]
    fn test_create_database_sql() {
        let plugin = TdenginePlugin::new();
        let request = DatabaseOperationRequest {
            database_name: "metrics".to_string(),
            field_values: HashMap::new(),
        };
        assert_eq!(
            plugin.build_create_database_sql(&request),
            "CREATE DATABASE `metrics`"
        );
    }

    #[test]
    fn test_rename_table_sql() {
        let plugin = TdenginePlugin::new();
        assert_eq!(
            plugin.rename_table("db1", "t1", "t2"),
            "ALTER TABLE `db1`.`t1` RENAME TO `t2`"
        );
    }

    #[test]
    fn test_build_column_def_appends_unsigned() {
        let plugin = TdenginePlugin::new();
        let mut col = ColumnDefinition::new("value");
        col.data_type = "BIGINT".to_string();
        col.is_unsigned = true;
        assert_eq!(plugin.build_column_def(&col), "`value` BIGINT UNSIGNED");
    }

    #[test]
    fn test_build_create_table_sql() {
        let plugin = TdenginePlugin::new();
        let mut design = TableDesign::new("metrics", "meters");
        let mut ts = ColumnDefinition::new("ts");
        ts.data_type = "TIMESTAMP".to_string();
        let mut current = ColumnDefinition::new("current");
        current.data_type = "FLOAT".to_string();
        design.add_column(ts);
        design.add_column(current);

        assert_eq!(
            plugin.build_create_table_sql(&design),
            "CREATE TABLE `meters` (\n  `ts` TIMESTAMP,\n  `current` FLOAT\n);"
        );
    }

    #[test]
    fn test_describe_type_takes_width() {
        assert!(tdengine_type_takes_width("BINARY"));
        assert!(tdengine_type_takes_width("nchar"));
        assert!(!tdengine_type_takes_width("INT"));
        assert!(!tdengine_type_takes_width("TIMESTAMP"));
    }
}
