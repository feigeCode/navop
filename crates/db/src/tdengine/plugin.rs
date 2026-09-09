//! TDengine 插件实现:SHOW DATABASES / SHOW TABLES + SHOW STABLES / DESCRIBE 元数据,
//! SQL 方言与 MySQL 对齐(反引号标识符、LIMIT n OFFSET m)。
//!
//! 库/表列表的元数据展示:
//! - `SHOW DATABASES` 在 3.x 返回多列元数据(ntables/vgroups/replica/keep0/keep1/precision/status 等,
//!   不同版本列集有差异),按列名探测映射,缺失列容忍为空;
//! - 表列表优先走 `information_schema.INS_TABLES`/`INS_STABLES`(含表类型/列数/标签数),
//!   查询报错时降级为 `SHOW TABLES`/`SHOW STABLES` 方案。

use crate::types::ObjectViewColumn as Column;
use anyhow::Result;
use one_core::storage::{DatabaseType, DbConnectionConfig};
use rust_i18n::t;
use std::collections::{HashMap, HashSet};
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

    /// 执行查询并返回列名与全部行;失败时返回 anyhow 错误。
    async fn query_table(
        connection: &dyn DbConnection,
        sql: &str,
        context: &str,
    ) -> Result<(Vec<String>, Vec<Vec<Option<String>>>)> {
        match connection.query(sql).await? {
            SqlResult::Query(query_result) => Ok((query_result.columns, query_result.rows)),
            SqlResult::Error(error) => {
                Err(anyhow::anyhow!("Failed to {}: {}", context, error.message))
            }
            SqlResult::Exec(_) => Err(anyhow::anyhow!("{} did not return a result set", context)),
        }
    }

    /// 执行查询并返回所有行;失败时返回 anyhow 错误。
    async fn query_rows(
        connection: &dyn DbConnection,
        sql: &str,
        context: &str,
    ) -> Result<Vec<Vec<Option<String>>>> {
        Ok(Self::query_table(connection, sql, context).await?.1)
    }

    /// 查询 `SHOW DATABASES` 并映射为库摘要(过滤内置系统库)。
    async fn database_summaries(
        &self,
        connection: &dyn DbConnection,
    ) -> Result<Vec<TdengineDatabaseSummary>> {
        let (columns, rows) =
            Self::query_table(connection, "SHOW DATABASES", "list databases").await?;
        Ok(summarize_databases(&columns, &rows))
    }

    /// 查询表列表摘要:优先 information_schema,失败时降级 SHOW 方案。
    async fn table_summaries(
        &self,
        connection: &dyn DbConnection,
        database: &str,
    ) -> Result<Vec<TdengineTableSummary>> {
        // information_schema.INS_TABLES/INS_STABLES 含表类型/列数/标签数等富元数据,
        // 老版本或权限不足时查询报错,整体降级到 SHOW 方案。
        if let Ok(summaries) =
            Self::table_summaries_from_information_schema(connection, database).await
        {
            return Ok(summaries);
        }
        self.table_summaries_from_show(connection, database).await
    }

    /// information_schema 路径:INS_TABLES(普通表/子表,部分版本含超级表行)
    /// + INS_STABLES(超级表,含列数/标签数);任一查询报错即返回 Err 交给上层降级。
    async fn table_summaries_from_information_schema(
        connection: &dyn DbConnection,
        database: &str,
    ) -> Result<Vec<TdengineTableSummary>> {
        let literal = escape_single_quoted(database);
        let (tables_columns, tables_rows) = Self::query_table(
            connection,
            &format!("SELECT * FROM information_schema.INS_TABLES WHERE db_name = '{literal}'"),
            "list tables from information_schema",
        )
        .await?;
        let (stables_columns, stables_rows) = Self::query_table(
            connection,
            &format!("SELECT * FROM information_schema.INS_STABLES WHERE db_name = '{literal}'"),
            "list stables from information_schema",
        )
        .await?;

        let tables_index = probe_table_columns(&tables_columns);
        let stables_index = probe_table_columns(&stables_columns);
        let tables = map_table_rows(&tables_index, &tables_rows, TdengineTableKind::Normal);
        let stables = map_table_rows(&stables_index, &stables_rows, TdengineTableKind::Super);

        // 超级表以 INS_STABLES 为准,创建时间/列数缺失时从 INS_TABLES 的超级表行回填。
        let mut summaries: Vec<TdengineTableSummary> = Vec::new();
        let mut seen_supers: HashSet<String> = HashSet::new();
        for mut stable in stables {
            if let Some(row) = tables
                .iter()
                .find(|t| t.kind == TdengineTableKind::Super && t.name == stable.name)
            {
                if stable.create_time.is_none() {
                    stable.create_time = row.create_time.clone();
                }
                if stable.column_count.is_none() {
                    stable.column_count = row.column_count;
                }
            }
            seen_supers.insert(stable.name.clone());
            summaries.push(stable);
        }
        // INS_TABLES 中出现而 INS_STABLES 未覆盖的超级表行(理论上少见)也保留。
        for table in &tables {
            if table.kind == TdengineTableKind::Super && !seen_supers.contains(&table.name) {
                seen_supers.insert(table.name.clone());
                summaries.push(table.clone());
            }
        }
        // 普通表/子表。
        summaries.extend(
            tables
                .into_iter()
                .filter(|t| t.kind != TdengineTableKind::Super),
        );

        summaries.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(summaries)
    }

    /// SHOW 降级路径:普通表/子表来自 SHOW {db}.TABLES,超级表来自 SHOW {db}.STABLES。
    async fn table_summaries_from_show(
        &self,
        connection: &dyn DbConnection,
        database: &str,
    ) -> Result<Vec<TdengineTableSummary>> {
        let db = self.quote_identifier(database);
        let (tables_columns, tables_rows) =
            Self::query_table(connection, &format!("SHOW {db}.TABLES"), "list tables").await?;
        let (stables_columns, stables_rows) =
            Self::query_table(connection, &format!("SHOW {db}.STABLES"), "list stables").await?;

        let mut summaries = map_table_rows(
            &probe_table_columns(&tables_columns),
            &tables_rows,
            TdengineTableKind::Normal,
        );
        summaries.extend(map_table_rows(
            &probe_table_columns(&stables_columns),
            &stables_rows,
            TdengineTableKind::Super,
        ));

        summaries.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(summaries)
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
        Ok(self
            .database_summaries(connection)
            .await?
            .into_iter()
            .map(|db| db.name)
            .collect())
    }

    async fn list_databases_view(&self, connection: &dyn DbConnection) -> Result<ObjectView> {
        let (columns, rows) =
            Self::query_table(connection, "SHOW DATABASES", "list databases").await?;
        let index = probe_database_columns(&columns);
        let databases = summarize_databases(&columns, &rows);

        // TDengine 特有属性列:按查询结果的列探测情况动态加入,缺失列整体省略。
        let keep_available = index.keep.is_some() || index.keep0.is_some() || index.keep1.is_some();
        let mut view_columns =
            vec![Column::localized("name", "ObjectView.columns.name").width(220.0)];
        if index.ntables.is_some() {
            view_columns.push(
                Column::localized("tables", "ObjectView.columns.tables")
                    .width(100.0)
                    .text_right(),
            );
        }
        if index.precision.is_some() {
            view_columns.push(
                Column::localized("precision", "ObjectView.columns.precision")
                    .width(90.0)
                    .text_center(),
            );
        }
        if index.replica.is_some() {
            view_columns.push(
                Column::localized("replica", "ObjectView.columns.replica")
                    .width(80.0)
                    .text_right(),
            );
        }
        if index.vgroups.is_some() {
            view_columns.push(
                Column::localized("vgroups", "ObjectView.columns.vgroups")
                    .width(90.0)
                    .text_right(),
            );
        }
        if keep_available {
            view_columns.push(Column::localized("keep", "ObjectView.columns.keep").width(150.0));
        }
        if index.status.is_some() {
            view_columns.push(Column::localized("status", "ObjectView.columns.status").width(90.0));
        }
        if index.create_time.is_some() {
            view_columns
                .push(Column::localized("create_time", "ObjectView.columns.created").width(180.0));
        }

        let view_rows: Vec<Vec<String>> = databases
            .iter()
            .map(|db| {
                // 与上方动态列一一对应,缺失值统一展示 "-"。
                let mut cells = vec![db.name.clone()];
                if index.ntables.is_some() {
                    cells.push(format_i64_cell(db.table_count));
                }
                if index.precision.is_some() {
                    cells.push(db.precision.clone().unwrap_or_else(|| "-".to_string()));
                }
                if index.replica.is_some() {
                    cells.push(format_i64_cell(db.replica));
                }
                if index.vgroups.is_some() {
                    cells.push(format_i64_cell(db.vgroups));
                }
                if keep_available {
                    cells.push(db.keep.clone().unwrap_or_else(|| "-".to_string()));
                }
                if index.status.is_some() {
                    cells.push(db.status.clone().unwrap_or_else(|| "-".to_string()));
                }
                if index.create_time.is_some() {
                    cells.push(db.create_time.clone().unwrap_or_else(|| "-".to_string()));
                }
                cells
            })
            .collect();

        Ok(ObjectView {
            db_node_type: DbNodeType::Database,
            title: t!("ObjectView.titles.databases").to_string(),
            columns: view_columns,
            rows: view_rows,
        })
    }

    async fn list_databases_detailed(
        &self,
        connection: &dyn DbConnection,
    ) -> Result<Vec<DatabaseInfo>> {
        // SHOW DATABASES 3.x 返回 name/ntables/precision/replica/vgroups/keep0/keep1/status
        // 等多列,按列名探测填充 ntables → table_count,缺失列容忍为 None。
        Ok(self
            .database_summaries(connection)
            .await?
            .into_iter()
            .map(|db| DatabaseInfo {
                name: db.name,
                // TDengine 无库引擎概念,统一展示为 TDengine。
                charset: Some("TDengine".to_string()),
                collation: None,
                size: None,
                table_count: db.table_count,
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
        // 优先 information_schema(含表类型/列数/标签数/所属超级表),降级 SHOW 方案。
        let summaries = self.table_summaries(connection, database).await?;
        Ok(table_infos_from_summaries(summaries))
    }

    async fn list_tables_view(
        &self,
        connection: &dyn DbConnection,
        database: &str,
        _schema: Option<String>,
    ) -> Result<ObjectView> {
        let tables = self.table_summaries(connection, database).await?;

        // 属性列按数据可用性动态加入:任一行有值才展示对应列(缺失列整体省略)。
        let has_column_count = tables.iter().any(|t| t.column_count.is_some());
        let has_tag_count = tables.iter().any(|t| t.tag_count.is_some());
        let has_stable = tables.iter().any(|t| t.stable_name.is_some());
        let has_create_time = tables.iter().any(|t| t.create_time.is_some());

        let mut columns = vec![
            Column::localized("name", "ObjectView.columns.name").width(220.0),
            Column::localized("type", "ObjectView.columns.type").width(110.0),
        ];
        if has_column_count {
            columns.push(
                Column::localized("column_count", "ObjectView.columns.column_count")
                    .width(90.0)
                    .text_right(),
            );
        }
        if has_tag_count {
            columns.push(
                Column::localized("tag_count", "ObjectView.columns.tag_count")
                    .width(90.0)
                    .text_right(),
            );
        }
        if has_stable {
            columns.push(
                Column::localized("stable_name", "ObjectView.columns.stable_name").width(220.0),
            );
        }
        if has_create_time {
            columns
                .push(Column::localized("create_time", "ObjectView.columns.created").width(180.0));
        }

        // 类型列展示本地化文本:超级表/子表/普通表。
        let rows: Vec<Vec<String>> = tables
            .iter()
            .map(|table| {
                let mut cells = vec![table.name.clone(), t!(table.kind.i18n_key()).to_string()];
                if has_column_count {
                    cells.push(format_i64_cell(table.column_count));
                }
                if has_tag_count {
                    cells.push(format_i64_cell(table.tag_count));
                }
                if has_stable {
                    cells.push(table.stable_name.clone().unwrap_or_else(|| "-".to_string()));
                }
                if has_create_time {
                    cells.push(table.create_time.clone().unwrap_or_else(|| "-".to_string()));
                }
                cells
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

// === 库/表列表元数据的纯映射逻辑(输入列名+行,输出展示摘要,便于单元测试) ===

/// TDengine 表种类:超级表 / 子表 / 普通表。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
enum TdengineTableKind {
    /// 普通表。
    #[default]
    Normal,
    /// 超级表。
    Super,
    /// 子表(依超级表创建)。
    Child,
}

impl TdengineTableKind {
    /// 类型展示文案的 i18n key。
    fn i18n_key(self) -> &'static str {
        match self {
            Self::Normal => "Tdengine.table_kind.normal",
            Self::Super => "Tdengine.table_kind.stable",
            Self::Child => "Tdengine.table_kind.child",
        }
    }
}

/// 库列表摘要(`SHOW DATABASES` 按列名探测的结果)。
#[derive(Debug, Default, Clone, PartialEq)]
struct TdengineDatabaseSummary {
    name: String,
    /// 表数量(ntables)。
    table_count: Option<i64>,
    /// 时间戳精度(precision,如 ms/us/ns)。
    precision: Option<String>,
    /// 副本数(replica)。
    replica: Option<i64>,
    /// VGroup 数(vgroups)。
    vgroups: Option<i64>,
    /// 保留策略(keep,或 keep0/keep1 合并,如 "3650d,3650d")。
    keep: Option<String>,
    /// 库状态(status,如 ready)。
    status: Option<String>,
    /// 创建时间(create_time)。
    create_time: Option<String>,
}

/// `SHOW DATABASES` 结果的列下标探测结果(缺失列为 None)。
#[derive(Debug, Default, Clone, Copy)]
struct TdengineDatabaseColumnIndex {
    name: Option<usize>,
    ntables: Option<usize>,
    precision: Option<usize>,
    replica: Option<usize>,
    vgroups: Option<usize>,
    keep: Option<usize>,
    keep0: Option<usize>,
    keep1: Option<usize>,
    status: Option<usize>,
    create_time: Option<usize>,
}

/// 表列表摘要(INS_TABLES/INS_STABLES/SHOW 派生)。
#[derive(Debug, Default, Clone, PartialEq)]
struct TdengineTableSummary {
    name: String,
    kind: TdengineTableKind,
    /// 列数(不含标签)。
    column_count: Option<i64>,
    /// 标签数(仅超级表有)。
    tag_count: Option<i64>,
    /// 所属超级表(仅子表有)。
    stable_name: Option<String>,
    create_time: Option<String>,
}

/// 表查询结果(INS_TABLES/INS_STABLES/SHOW TABLES/SHOW STABLES)的列下标探测结果。
#[derive(Debug, Default, Clone, Copy)]
struct TdengineTableColumnIndex {
    name: Option<usize>,
    /// 表类型列(INS_TABLES 的 type,如 SUPER_TABLE/CHILD_TABLE/NORMAL_TABLE)。
    table_type: Option<usize>,
    /// 列数(候选名 columns/col_count)。
    column_count: Option<usize>,
    /// 标签数(候选名 tags/tag_columns)。
    tag_count: Option<usize>,
    /// 所属超级表(候选名 stable_name/stb_name)。
    stable_name: Option<usize>,
    /// 创建时间(候选名 create_time/created_time)。
    create_time: Option<usize>,
}

/// 在列名列表中按候选名查找列下标(忽略大小写与首尾空白,候选名按优先级排序)。
fn probe_column(columns: &[String], candidates: &[&str]) -> Option<usize> {
    candidates.iter().find_map(|candidate| {
        columns
            .iter()
            .position(|name| name.trim().eq_ignore_ascii_case(candidate))
    })
}

/// 探测 `SHOW DATABASES` 结果的列下标;不同版本列集有差异,缺失列保持 None。
fn probe_database_columns(columns: &[String]) -> TdengineDatabaseColumnIndex {
    TdengineDatabaseColumnIndex {
        name: probe_column(columns, &["name"]),
        ntables: probe_column(columns, &["ntables"]),
        precision: probe_column(columns, &["precision"]),
        replica: probe_column(columns, &["replica"]),
        vgroups: probe_column(columns, &["vgroups"]),
        keep: probe_column(columns, &["keep"]),
        keep0: probe_column(columns, &["keep0"]),
        keep1: probe_column(columns, &["keep1"]),
        status: probe_column(columns, &["status"]),
        // 3.x 为 create_time,老版本为 created_time。
        create_time: probe_column(columns, &["create_time", "created_time"]),
    }
}

/// 探测表查询结果的列下标;缺失列保持 None。
fn probe_table_columns(columns: &[String]) -> TdengineTableColumnIndex {
    TdengineTableColumnIndex {
        // INS_TABLES/SHOW TABLES 为 table_name,SHOW STABLES 为 name,
        // INS_STABLES 部分版本为 stable_name/stb_name,按优先级探测。
        name: probe_column(columns, &["table_name", "name", "stable_name", "stb_name"]),
        table_type: probe_column(columns, &["type"]),
        column_count: probe_column(columns, &["columns", "col_count"]),
        tag_count: probe_column(columns, &["tags", "tag_columns"]),
        stable_name: probe_column(columns, &["stable_name", "stb_name"]),
        create_time: probe_column(columns, &["create_time", "created_time"]),
    }
}

/// 取行中指定下标的文本值。
fn cell_text(row: &[Option<String>], index: Option<usize>) -> Option<String> {
    index
        .and_then(|i| row.get(i))
        .and_then(|value| value.clone())
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

/// 取行中指定下标的整数值。
fn cell_i64(row: &[Option<String>], index: Option<usize>) -> Option<i64> {
    cell_text(row, index).and_then(|value| value.parse::<i64>().ok())
}

/// 将单行 `SHOW DATABASES` 结果映射为库摘要;名称缺失时返回 None。
fn map_database_row(
    index: &TdengineDatabaseColumnIndex,
    row: &[Option<String>],
) -> Option<TdengineDatabaseSummary> {
    Some(TdengineDatabaseSummary {
        name: cell_text(row, index.name)?,
        table_count: cell_i64(row, index.ntables),
        precision: cell_text(row, index.precision),
        replica: cell_i64(row, index.replica),
        vgroups: cell_i64(row, index.vgroups),
        keep: merge_keep_text(
            cell_text(row, index.keep).as_deref(),
            cell_text(row, index.keep0).as_deref(),
            cell_text(row, index.keep1).as_deref(),
        ),
        status: cell_text(row, index.status),
        create_time: cell_text(row, index.create_time),
    })
}

/// `SHOW DATABASES` 查询结果 → 库摘要列表(含内置系统库过滤)。
fn summarize_databases(
    columns: &[String],
    rows: &[Vec<Option<String>>],
) -> Vec<TdengineDatabaseSummary> {
    let index = probe_database_columns(columns);
    rows.iter()
        .filter_map(|row| map_database_row(&index, row))
        .filter(|db| !is_system_database(&db.name))
        .collect()
}

/// 判断是否为 TDengine 内置系统库(库列表中隐藏)。
fn is_system_database(name: &str) -> bool {
    TDENGINE_SYSTEM_DATABASES
        .iter()
        .any(|system| name.eq_ignore_ascii_case(system))
}

/// 合并保留策略文本:优先单列 keep,否则拼接 keep0/keep1(如 "3650d,3650d")。
fn merge_keep_text(keep: Option<&str>, keep0: Option<&str>, keep1: Option<&str>) -> Option<String> {
    if let Some(keep) = keep {
        return Some(keep.to_string());
    }
    let parts: Vec<&str> = [keep0, keep1]
        .into_iter()
        .flatten()
        .filter(|value| !value.is_empty())
        .collect();
    if parts.is_empty() {
        None
    } else {
        Some(parts.join(","))
    }
}

/// 依据类型文本推断表种类(INS_TABLES 的 type 列,如 SUPER_TABLE/CHILD_TABLE)。
fn table_kind_from_type(type_text: Option<&str>) -> Option<TdengineTableKind> {
    let text = type_text?.to_ascii_uppercase();
    if text.contains("SUPER") {
        Some(TdengineTableKind::Super)
    } else if text.contains("CHILD") {
        Some(TdengineTableKind::Child)
    } else if text.contains("NORMAL") {
        Some(TdengineTableKind::Normal)
    } else {
        None
    }
}

/// 将表查询行映射为表摘要列表;无类型列时按 default_kind 与所属超级表推断种类
/// (SHOW TABLES 默认普通表,所属超级表非空即为子表;SHOW STABLES 默认超级表)。
fn map_table_rows(
    index: &TdengineTableColumnIndex,
    rows: &[Vec<Option<String>>],
    default_kind: TdengineTableKind,
) -> Vec<TdengineTableSummary> {
    rows.iter()
        .filter_map(|row| {
            let name = cell_text(row, index.name)?;
            let stable_name = cell_text(row, index.stable_name);
            let kind = table_kind_from_type(cell_text(row, index.table_type).as_deref()).unwrap_or(
                match default_kind {
                    TdengineTableKind::Super => TdengineTableKind::Super,
                    _ if stable_name.is_some() => TdengineTableKind::Child,
                    _ => TdengineTableKind::Normal,
                },
            );
            Some(TdengineTableSummary {
                name,
                kind,
                column_count: cell_i64(row, index.column_count),
                tag_count: cell_i64(row, index.tag_count),
                // 超级表无所属超级表;个别数据源(如 INS_STABLES 的名称列)会与
                // 所属超级表列同名,这里统一清除避免自引用。
                stable_name: if kind == TdengineTableKind::Super {
                    None
                } else {
                    stable_name
                },
                create_time: cell_text(row, index.create_time),
            })
        })
        .collect()
}

/// 表摘要 → 树视图 TableInfo;超级表以 engine=STABLE 标记(既有约定,树视图依赖)。
fn table_infos_from_summaries(summaries: Vec<TdengineTableSummary>) -> Vec<TableInfo> {
    summaries
        .into_iter()
        .map(|table| TableInfo {
            name: table.name,
            object_type: TableObjectType::Table,
            schema: None,
            create_time: table.create_time,
            charset: None,
            collation: None,
            engine: if table.kind == TdengineTableKind::Super {
                Some("STABLE".to_string())
            } else {
                None
            },
            comment: None,
        })
        .collect()
}

/// 数值单元格文本:缺失时展示 "-"。
fn format_i64_cell(value: Option<i64>) -> String {
    value
        .map(|n| n.to_string())
        .unwrap_or_else(|| "-".to_string())
}

/// 转义 SQL 字符串字面量中的单引号(以成对单引号表示)。
fn escape_single_quoted(value: &str) -> String {
    value.replace('\'', "''")
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

    // === 库/表列表元数据映射测试(模拟 SHOW / information_schema 输出) ===

    /// 按列数构造一行模拟数据:与列一一对应,空字符串表示 NULL。
    fn mock_row(columns: &[&str], cells: &[&str]) -> Vec<Option<String>> {
        assert_eq!(columns.len(), cells.len(), "模拟行与列数不一致");
        columns
            .iter()
            .zip(cells)
            .map(|(_, value)| (!value.is_empty()).then(|| value.to_string()))
            .collect()
    }

    #[test]
    fn test_probe_column_ignores_case_and_prioritizes_candidates() {
        let columns = vec!["Name".to_string(), "NTABLES".to_string()];
        assert_eq!(probe_column(&columns, &["name"]), Some(0));
        assert_eq!(probe_column(&columns, &["ntables"]), Some(1));
        assert_eq!(probe_column(&columns, &["keep"]), None);
        // 候选名按优先级取第一个命中的列。
        assert_eq!(probe_column(&columns, &["keep", "name"]), Some(0));
    }

    #[test]
    fn test_summarize_databases_full_columns() {
        // TDengine 3.x 的 SHOW DATABASES 完整列集(截取映射关心的列)。
        let columns: Vec<String> = [
            "name",
            "create_time",
            "ntables",
            "vgroups",
            "replica",
            "quorum",
            "days",
            "keep0",
            "keep1",
            "cache",
            "blocks",
            "minrows",
            "maxrows",
            "wal",
            "wal_level",
            "comp",
            "cachemodel",
            "precision",
            "status",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect();
        let values = [
            "power_db",
            "2024-05-01 10:00:00.000",
            "24",
            "4",
            "1",
            "1",
            "10",
            "3650d",
            "3650d",
            "16",
            "12",
            "100",
            "4096",
            "1",
            "1",
            "2",
            "none",
            "ms",
            "ready",
        ];
        let row: Vec<Option<String>> = values.iter().map(|v| Some(v.to_string())).collect();

        let databases = summarize_databases(&columns, &[row]);
        assert_eq!(databases.len(), 1);
        let db = &databases[0];
        assert_eq!(db.name, "power_db");
        assert_eq!(db.table_count, Some(24));
        assert_eq!(db.precision.as_deref(), Some("ms"));
        assert_eq!(db.replica, Some(1));
        assert_eq!(db.vgroups, Some(4));
        // keep0/keep1 合并展示。
        assert_eq!(db.keep.as_deref(), Some("3650d,3650d"));
        assert_eq!(db.status.as_deref(), Some("ready"));
        assert_eq!(db.create_time.as_deref(), Some("2024-05-01 10:00:00.000"));
    }

    #[test]
    fn test_summarize_databases_tolerates_missing_columns() {
        // 列缺失变体:仅 name 一列,其余属性保持 None。
        let columns = vec!["name".to_string()];
        let row = vec![Some("log_db".to_string())];

        let databases = summarize_databases(&columns, &[row]);
        assert_eq!(databases.len(), 1);
        let db = &databases[0];
        assert_eq!(db.name, "log_db");
        assert_eq!(db.table_count, None);
        assert_eq!(db.precision, None);
        assert_eq!(db.replica, None);
        assert_eq!(db.vgroups, None);
        assert_eq!(db.keep, None);
        assert_eq!(db.status, None);
        assert_eq!(db.create_time, None);
    }

    #[test]
    fn test_summarize_databases_legacy_keep_and_created_time() {
        // 老版本变体:单 keep 列 + created_time 列名。
        let columns: Vec<String> = ["name", "created_time", "ntables", "keep"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        let row: Vec<Option<String>> = ["old_db", "2023-01-01 00:00:00", "12", "3650d"]
            .iter()
            .map(|v| Some(v.to_string()))
            .collect();

        let databases = summarize_databases(&columns, &[row]);
        assert_eq!(databases.len(), 1);
        let db = &databases[0];
        assert_eq!(db.table_count, Some(12));
        // 单 keep 列直接使用原始值。
        assert_eq!(db.keep.as_deref(), Some("3650d"));
        assert_eq!(db.create_time.as_deref(), Some("2023-01-01 00:00:00"));
    }

    #[test]
    fn test_summarize_databases_filters_system_databases() {
        let columns = vec!["name".to_string(), "ntables".to_string()];
        let rows = vec![
            vec![
                Some("information_schema".to_string()),
                Some("17".to_string()),
            ],
            vec![
                Some("performance_schema".to_string()),
                Some("9".to_string()),
            ],
            vec![Some("power_db".to_string()), Some("3".to_string())],
        ];

        let databases = summarize_databases(&columns, &rows);
        // 内置系统库隐藏,仅保留用户库。
        assert_eq!(databases.len(), 1);
        assert_eq!(databases[0].name, "power_db");
        assert_eq!(databases[0].table_count, Some(3));
    }

    #[test]
    fn test_merge_keep_text_variants() {
        // 优先单 keep 列。
        assert_eq!(
            merge_keep_text(Some("3650d"), Some("3650d"), Some("3650d")),
            Some("3650d".to_string())
        );
        // keep0/keep1 拼接。
        assert_eq!(
            merge_keep_text(None, Some("3650d"), Some("1825d")),
            Some("3650d,1825d".to_string())
        );
        // 仅 keep0。
        assert_eq!(
            merge_keep_text(None, Some("3650d"), None),
            Some("3650d".to_string())
        );
        // 全部缺失。
        assert_eq!(merge_keep_text(None, None, None), None);
    }

    #[test]
    fn test_map_table_rows_from_ins_tables() {
        // information_schema.INS_TABLES 风格:含类型列与标签数列。
        let cols = [
            "vgroup_id",
            "db_name",
            "table_name",
            "create_time",
            "columns",
            "tag_columns",
            "type",
            "stable_name",
        ];
        let columns: Vec<String> = cols.iter().map(|s| s.to_string()).collect();
        let rows = vec![
            mock_row(
                &cols,
                &[
                    "2",
                    "power_db",
                    "meters",
                    "2024-05-01 10:00:00",
                    "4",
                    "2",
                    "SUPER_TABLE",
                    "",
                ],
            ),
            mock_row(
                &cols,
                &[
                    "2",
                    "power_db",
                    "d1001",
                    "2024-05-01 10:01:00",
                    "4",
                    "2",
                    "CHILD_TABLE",
                    "meters",
                ],
            ),
            mock_row(
                &cols,
                &[
                    "3",
                    "power_db",
                    "alarm",
                    "2024-05-02 08:00:00",
                    "2",
                    "0",
                    "NORMAL_TABLE",
                    "",
                ],
            ),
        ];

        let index = probe_table_columns(&columns);
        let tables = map_table_rows(&index, &rows, TdengineTableKind::Normal);
        assert_eq!(tables.len(), 3);

        assert_eq!(tables[0].name, "meters");
        assert_eq!(tables[0].kind, TdengineTableKind::Super);
        assert_eq!(tables[0].column_count, Some(4));
        assert_eq!(tables[0].tag_count, Some(2));
        assert_eq!(tables[0].stable_name, None);

        assert_eq!(tables[1].name, "d1001");
        assert_eq!(tables[1].kind, TdengineTableKind::Child);
        assert_eq!(tables[1].stable_name.as_deref(), Some("meters"));

        assert_eq!(tables[2].name, "alarm");
        assert_eq!(tables[2].kind, TdengineTableKind::Normal);
    }

    #[test]
    fn test_map_table_rows_from_show_tables_infers_child_by_stable() {
        // SHOW TABLES 风格:无类型列,所属超级表非空即推断为子表。
        let columns: Vec<String> = ["table_name", "stable_name", "created_time"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        let rows = vec![
            mock_row(
                &["table_name", "stable_name", "created_time"],
                &["d1001", "meters", "2024-05-01 10:01:00"],
            ),
            mock_row(
                &["table_name", "stable_name", "created_time"],
                &["alarm", "", "2024-05-02 08:00:00"],
            ),
        ];

        let index = probe_table_columns(&columns);
        let tables = map_table_rows(&index, &rows, TdengineTableKind::Normal);
        assert_eq!(tables.len(), 2);
        assert_eq!(tables[0].kind, TdengineTableKind::Child);
        assert_eq!(tables[0].stable_name.as_deref(), Some("meters"));
        assert_eq!(tables[1].kind, TdengineTableKind::Normal);
        assert_eq!(tables[1].stable_name, None);
        assert_eq!(
            tables[1].create_time.as_deref(),
            Some("2024-05-02 08:00:00")
        );
    }

    #[test]
    fn test_map_table_rows_from_show_stables() {
        // SHOW STABLES 风格:默认超级表,含列数/标签数。
        let columns: Vec<String> = ["name", "created_time", "columns", "tags"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        let rows = vec![mock_row(
            &["name", "created_time", "columns", "tags"],
            &["meters", "2024-05-01 10:00:00", "4", "2"],
        )];

        let index = probe_table_columns(&columns);
        let tables = map_table_rows(&index, &rows, TdengineTableKind::Super);
        assert_eq!(tables.len(), 1);
        assert_eq!(tables[0].name, "meters");
        assert_eq!(tables[0].kind, TdengineTableKind::Super);
        assert_eq!(tables[0].column_count, Some(4));
        assert_eq!(tables[0].tag_count, Some(2));
    }

    #[test]
    fn test_map_table_rows_from_ins_stables_with_stable_name_column() {
        // INS_STABLES 列名变体:名称列为 stable_name,与所属超级表候选同名,
        // 名称探测需命中该列且超级表的所属超级表清空(避免自引用)。
        let cols = ["stable_name", "db_name", "create_time", "columns", "tags"];
        let columns: Vec<String> = cols.iter().map(|s| s.to_string()).collect();
        let rows = vec![mock_row(
            &cols,
            &["meters", "power_db", "2024-05-01 10:00:00", "4", "2"],
        )];

        let index = probe_table_columns(&columns);
        let tables = map_table_rows(&index, &rows, TdengineTableKind::Super);
        assert_eq!(tables.len(), 1);
        assert_eq!(tables[0].name, "meters");
        assert_eq!(tables[0].kind, TdengineTableKind::Super);
        assert_eq!(tables[0].tag_count, Some(2));
        assert_eq!(tables[0].stable_name, None);
    }

    #[test]
    fn test_table_kind_from_type() {
        assert_eq!(
            table_kind_from_type(Some("SUPER_TABLE")),
            Some(TdengineTableKind::Super)
        );
        assert_eq!(
            table_kind_from_type(Some("child_table")),
            Some(TdengineTableKind::Child)
        );
        assert_eq!(
            table_kind_from_type(Some("NORMAL_TABLE")),
            Some(TdengineTableKind::Normal)
        );
        // 无法识别或缺失时交由调用方推断。
        assert_eq!(table_kind_from_type(Some("VIEW")), None);
        assert_eq!(table_kind_from_type(None), None);
    }

    #[test]
    fn test_table_infos_keep_stable_engine_convention() {
        // 树视图依赖 engine=STABLE 标记超级表,其余表 engine 为 None。
        let summaries = vec![
            TdengineTableSummary {
                name: "meters".to_string(),
                kind: TdengineTableKind::Super,
                column_count: Some(4),
                tag_count: Some(2),
                stable_name: None,
                create_time: Some("2024-05-01 10:00:00".to_string()),
            },
            TdengineTableSummary {
                name: "d1001".to_string(),
                kind: TdengineTableKind::Child,
                column_count: Some(4),
                tag_count: None,
                stable_name: Some("meters".to_string()),
                create_time: None,
            },
        ];

        let infos = table_infos_from_summaries(summaries);
        assert_eq!(infos.len(), 2);
        assert_eq!(infos[0].engine.as_deref(), Some("STABLE"));
        assert_eq!(infos[0].create_time.as_deref(), Some("2024-05-01 10:00:00"));
        assert_eq!(infos[1].engine, None);
        assert_eq!(infos[1].create_time, None);
    }

    #[test]
    fn test_escape_single_quoted() {
        assert_eq!(escape_single_quoted("power_db"), "power_db");
        assert_eq!(escape_single_quoted("a'b"), "a''b");
    }
}
