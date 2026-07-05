use super::{
    CellValue, ColumnSchema, DataCompareResult, DiffStatus, RowData, SchemaCompareResult,
    TableSchema,
};
use crate::plugin::DatabasePlugin;
use crate::types::{ColumnDefinition, ForeignKeyDefinition, IndexDefinition, TableDesign};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};

/// 同步计划类型
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SyncStatementKind {
    CreateTable,
    DropTable,
    AlterTable,
    CreateIndex,
    DropIndex,
    Insert,
    Update,
    Delete,
    Comment,
    Unknown,
}

/// 单条同步语句
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncStatement {
    /// 语句唯一标识
    pub id: String,
    /// SQL 语句
    pub sql: String,
    /// 语句类型
    pub kind: SyncStatementKind,
    /// 对象名（表名、索引名等）
    pub object_name: Option<String>,
    /// 行键（数据同步时）
    pub row_key: Option<serde_json::Map<String, serde_json::Value>>,
    /// 是否破坏性操作（DROP、DELETE、类型修改等）
    pub destructive: bool,
    /// 是否事务安全
    pub transactional_safe: bool,
    /// 默认是否选中（破坏性操作默认不选）
    pub selected_by_default: bool,
    /// 警告信息
    pub warnings: Vec<String>,
}

/// 同步计划摘要
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncPlanSummary {
    /// INSERT 语句数
    pub insert_count: usize,
    /// UPDATE 语句数
    pub update_count: usize,
    /// DELETE 语句数
    pub delete_count: usize,
    /// 其他 DDL 语句数
    pub ddl_count: usize,
    /// 总语句数
    pub total_count: usize,
}

/// 同步计划
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncPlan {
    /// 计划唯一标识
    pub id: String,
    /// 目标表名
    pub target_table: String,
    /// 同步语句列表
    pub statements: Vec<SyncStatement>,
    /// 摘要统计
    pub summary: SyncPlanSummary,
    /// 全局警告
    pub warnings: Vec<String>,
    /// 完整 SQL 文本（所有语句拼接）
    pub sql_text: String,
}

trait SyncSqlDialect {
    fn quote_identifier(&self, identifier: &str) -> String;
    fn format_table_reference(&self, database: &str, schema: Option<&str>, table: &str) -> String;
    fn drop_table(&self, database: &str, schema: Option<&str>, table: &str) -> String;
    fn build_column_def(&self, column: &ColumnDefinition) -> String;
    fn build_create_table_sql(&self, design: &TableDesign) -> String;
    fn build_alter_table_sql(&self, original: &TableDesign, new: &TableDesign) -> Option<String>;
}

impl<T> SyncSqlDialect for T
where
    T: DatabasePlugin + ?Sized,
{
    fn quote_identifier(&self, identifier: &str) -> String {
        DatabasePlugin::quote_identifier(self, identifier)
    }

    fn format_table_reference(&self, database: &str, schema: Option<&str>, table: &str) -> String {
        DatabasePlugin::format_table_reference(self, database, schema, table)
    }

    fn drop_table(&self, database: &str, schema: Option<&str>, table: &str) -> String {
        DatabasePlugin::drop_table(self, database, schema, table)
    }

    fn build_column_def(&self, column: &ColumnDefinition) -> String {
        DatabasePlugin::build_column_def(self, column)
    }

    fn build_create_table_sql(&self, design: &TableDesign) -> String {
        DatabasePlugin::build_create_table_sql(self, design)
    }

    fn build_alter_table_sql(&self, original: &TableDesign, new: &TableDesign) -> Option<String> {
        Some(DatabasePlugin::build_alter_table_sql(self, original, new))
    }
}

struct RawSyncSqlDialect;

struct PluginSyncSqlDialect<'a>(&'a dyn DatabasePlugin);

impl SyncSqlDialect for PluginSyncSqlDialect<'_> {
    fn quote_identifier(&self, identifier: &str) -> String {
        self.0.quote_identifier(identifier)
    }

    fn format_table_reference(&self, database: &str, schema: Option<&str>, table: &str) -> String {
        self.0.format_table_reference(database, schema, table)
    }

    fn drop_table(&self, database: &str, schema: Option<&str>, table: &str) -> String {
        self.0.drop_table(database, schema, table)
    }

    fn build_column_def(&self, column: &ColumnDefinition) -> String {
        self.0.build_column_def(column)
    }

    fn build_create_table_sql(&self, design: &TableDesign) -> String {
        self.0.build_create_table_sql(design)
    }

    fn build_alter_table_sql(&self, original: &TableDesign, new: &TableDesign) -> Option<String> {
        Some(self.0.build_alter_table_sql(original, new))
    }
}

impl SyncSqlDialect for RawSyncSqlDialect {
    fn quote_identifier(&self, identifier: &str) -> String {
        identifier.to_string()
    }

    fn format_table_reference(&self, _database: &str, schema: Option<&str>, table: &str) -> String {
        match schema {
            Some(schema) if !schema.trim().is_empty() => format!("{schema}.{table}"),
            _ => table.to_string(),
        }
    }

    fn drop_table(&self, _database: &str, schema: Option<&str>, table: &str) -> String {
        format!(
            "DROP TABLE IF EXISTS {}",
            self.format_table_reference("", schema, table)
        )
    }

    fn build_column_def(&self, column: &ColumnDefinition) -> String {
        let nullable = if column.is_nullable {
            "NULL"
        } else {
            "NOT NULL"
        };
        let mut definition = format!("{} {} {}", column.name, column.data_type, nullable);
        if let Some(default_value) = &column.default_value {
            definition.push_str(" DEFAULT ");
            definition.push_str(default_value);
        }
        definition
    }

    fn build_create_table_sql(&self, design: &TableDesign) -> String {
        let columns = design
            .columns
            .iter()
            .map(|column| format!("  {}", self.build_column_def(column)))
            .collect::<Vec<_>>()
            .join(",\n");
        format!("CREATE TABLE {} (\n{}\n);", design.table_name, columns)
    }

    fn build_alter_table_sql(&self, _original: &TableDesign, _new: &TableDesign) -> Option<String> {
        None
    }
}

/// 从数据比较结果生成同步计划
///
/// # P0 安全保护
/// - DELETE 语句默认不选中（`selected_by_default = false`）
/// - 所有 DELETE 标记为破坏性操作（`destructive = true`）
pub fn build_data_sync_plan(result: &DataCompareResult) -> SyncPlan {
    build_data_sync_plan_with_dialect(result, "", None, &RawSyncSqlDialect)
}

pub fn build_data_sync_plan_with_plugin(
    result: &DataCompareResult,
    target_database: &str,
    target_schema: Option<&str>,
    plugin: &dyn DatabasePlugin,
) -> SyncPlan {
    build_data_sync_plan_with_dialect(
        result,
        target_database,
        target_schema,
        &PluginSyncSqlDialect(plugin),
    )
}

fn build_data_sync_plan_with_dialect(
    result: &DataCompareResult,
    target_database: &str,
    target_schema: Option<&str>,
    dialect: &dyn SyncSqlDialect,
) -> SyncPlan {
    let mut statements = Vec::new();
    let plan_id = uuid::Uuid::new_v4().to_string();
    let target_table_ref =
        dialect.format_table_reference(target_database, target_schema, &result.target_table);

    // 生成 INSERT 语句（新增行）
    for row in &result.added {
        let stmt_id = uuid::Uuid::new_v4().to_string();
        let sql = generate_insert_sql(&target_table_ref, row, &result.columns, dialect);

        statements.push(SyncStatement {
            id: stmt_id,
            sql,
            kind: SyncStatementKind::Insert,
            object_name: Some(result.target_table.clone()),
            row_key: extract_row_key(row, &result.key_columns),
            destructive: false,
            transactional_safe: true,
            selected_by_default: true,
            warnings: vec![],
        });
    }

    // 生成 UPDATE 语句（修改行）
    for modified in &result.modified {
        let stmt_id = uuid::Uuid::new_v4().to_string();
        let sql = generate_update_sql(
            &target_table_ref,
            &modified.source_values,
            &modified.key_values,
            &modified.changes,
            dialect,
        );

        statements.push(SyncStatement {
            id: stmt_id,
            sql,
            kind: SyncStatementKind::Update,
            object_name: Some(result.target_table.clone()),
            row_key: Some(
                modified
                    .key_values
                    .iter()
                    .map(|(k, v)| (k.clone(), v.clone()))
                    .collect(),
            ),
            destructive: false,
            transactional_safe: true,
            selected_by_default: true,
            warnings: vec![],
        });
    }

    // 生成 DELETE 语句（删除行）
    // P0 安全保护：默认不选中，标记为破坏性操作
    for row in &result.removed {
        let stmt_id = uuid::Uuid::new_v4().to_string();
        let key_values = extract_key_values_from_row(row, &result.key_columns);
        let sql = generate_delete_sql(&target_table_ref, &key_values, dialect);

        statements.push(SyncStatement {
            id: stmt_id,
            sql,
            kind: SyncStatementKind::Delete,
            object_name: Some(result.target_table.clone()),
            row_key: Some(
                key_values
                    .iter()
                    .map(|(k, v)| (k.clone(), v.clone()))
                    .collect(),
            ),
            destructive: true, // 破坏性操作
            transactional_safe: true,
            selected_by_default: false, // P0: 默认不选中
            warnings: vec!["此操作将删除目标表中的数据".to_string()],
        });
    }

    // 生成摘要
    let insert_count = result.added.len();
    let update_count = result.modified.len();
    let delete_count = result.removed.len();
    let total_count = insert_count + update_count + delete_count;

    let summary = SyncPlanSummary {
        insert_count,
        update_count,
        delete_count,
        ddl_count: 0,
        total_count,
    };

    // 生成完整 SQL 文本
    let sql_text = statements
        .iter()
        .map(|s| s.sql.as_str())
        .collect::<Vec<_>>()
        .join("\n");

    SyncPlan {
        id: plan_id,
        target_table: result.target_table.clone(),
        statements,
        summary,
        warnings: vec![],
        sql_text,
    }
}

/// 生成 INSERT SQL
fn generate_insert_sql(
    table: &str,
    row: &RowData,
    columns: &[String],
    dialect: &dyn SyncSqlDialect,
) -> String {
    let cols = columns
        .iter()
        .map(|column| dialect.quote_identifier(column))
        .collect::<Vec<_>>()
        .join(", ");
    let values = columns
        .iter()
        .map(|col| format_value(row.get(col).cloned().unwrap_or(CellValue::Null)))
        .collect::<Vec<_>>()
        .join(", ");

    format!("INSERT INTO {} ({}) VALUES ({});", table, cols, values)
}

/// 生成 UPDATE SQL
fn generate_update_sql(
    table: &str,
    source_values: &RowData,
    key_values: &std::collections::HashMap<String, CellValue>,
    changes: &std::collections::HashMap<String, (CellValue, CellValue)>,
    dialect: &dyn SyncSqlDialect,
) -> String {
    let set_clause = changes
        .keys()
        .map(|col| {
            let value = source_values.get(col).cloned().unwrap_or(CellValue::Null);
            format!(
                "{} = {}",
                dialect.quote_identifier(col),
                format_value(value)
            )
        })
        .collect::<Vec<_>>()
        .join(", ");

    let where_clause = sorted_key_values(key_values)
        .into_iter()
        .map(|(col, val)| format_where_condition(col, val, dialect))
        .collect::<Vec<_>>()
        .join(" AND ");

    format!(
        "UPDATE {} SET {} WHERE {};",
        table, set_clause, where_clause
    )
}

/// 生成 DELETE SQL
fn generate_delete_sql(
    table: &str,
    key_values: &std::collections::HashMap<String, CellValue>,
    dialect: &dyn SyncSqlDialect,
) -> String {
    let where_clause = sorted_key_values(key_values)
        .into_iter()
        .map(|(col, val)| format_where_condition(col, val, dialect))
        .collect::<Vec<_>>()
        .join(" AND ");

    format!("DELETE FROM {} WHERE {};", table, where_clause)
}

fn generate_create_index_sql(
    table_name: &str,
    index: &super::IndexSchema,
    dialect: &dyn SyncSqlDialect,
) -> String {
    let unique = if index.unique { "UNIQUE " } else { "" };
    format!(
        "CREATE {}INDEX {} ON {} ({});",
        unique,
        dialect.quote_identifier(&index.name),
        table_name,
        index
            .columns
            .iter()
            .map(|column| dialect.quote_identifier(column))
            .collect::<Vec<_>>()
            .join(", ")
    )
}

fn generate_drop_index_sql(
    target_db_type: &str,
    table_name: &str,
    index_name: &str,
    dialect: &dyn SyncSqlDialect,
) -> String {
    let index_name = dialect.quote_identifier(index_name);
    let db_type = target_db_type.to_lowercase();
    if db_type.contains("mysql") || db_type.contains("mariadb") {
        format!("ALTER TABLE {} DROP INDEX {};", table_name, index_name)
    } else if db_type.contains("mssql") || db_type.contains("sql server") {
        format!("DROP INDEX {} ON {};", index_name, table_name)
    } else if db_type.contains("oracle") {
        format!("DROP INDEX {};", index_name)
    } else if db_type.contains("clickhouse") {
        format!("ALTER TABLE {} DROP INDEX {};", table_name, index_name)
    } else {
        format!("DROP INDEX IF EXISTS {};", index_name)
    }
}

fn is_primary_index_name(index_name: &str) -> bool {
    index_name.eq_ignore_ascii_case("PRIMARY")
}

fn is_primary_index(index: &super::IndexSchema) -> bool {
    is_primary_index_name(&index.name)
}

fn generate_add_primary_key_sql(
    table_name: &str,
    index: &super::IndexSchema,
    dialect: &dyn SyncSqlDialect,
) -> String {
    let columns = index
        .columns
        .iter()
        .map(|column| dialect.quote_identifier(column))
        .collect::<Vec<_>>()
        .join(", ");
    format!("ALTER TABLE {} ADD PRIMARY KEY ({});", table_name, columns)
}

fn generate_drop_primary_key_sql(
    target_db_type: &str,
    table_name: &str,
    primary_key_name: &str,
    dialect: &dyn SyncSqlDialect,
) -> String {
    let db_type = target_db_type.to_lowercase();
    if db_type.contains("mysql") || db_type.contains("mariadb") {
        format!("ALTER TABLE {} DROP PRIMARY KEY;", table_name)
    } else {
        format!(
            "ALTER TABLE {} DROP CONSTRAINT {};",
            table_name,
            dialect.quote_identifier(primary_key_name)
        )
    }
}

fn generate_add_foreign_key_sql(
    table_name: &str,
    foreign_key: &super::ForeignKeySchema,
    target_database: &str,
    target_schema: Option<&str>,
    dialect: &dyn SyncSqlDialect,
) -> String {
    let columns = foreign_key
        .columns
        .iter()
        .map(|column| dialect.quote_identifier(column))
        .collect::<Vec<_>>()
        .join(", ");
    let ref_table =
        dialect.format_table_reference(target_database, target_schema, &foreign_key.ref_table);
    let ref_columns = foreign_key
        .ref_columns
        .iter()
        .map(|column| dialect.quote_identifier(column))
        .collect::<Vec<_>>()
        .join(", ");

    let mut sql = format!(
        "ALTER TABLE {} ADD CONSTRAINT {} FOREIGN KEY ({}) REFERENCES {} ({})",
        table_name,
        dialect.quote_identifier(&foreign_key.name),
        columns,
        ref_table,
        ref_columns
    );
    if let Some(on_delete) = foreign_key_action_sql(foreign_key.on_delete.as_deref()) {
        sql.push_str(&format!(" ON DELETE {on_delete}"));
    }
    if let Some(on_update) = foreign_key_action_sql(foreign_key.on_update.as_deref()) {
        sql.push_str(&format!(" ON UPDATE {on_update}"));
    }
    sql.push(';');
    sql
}

fn foreign_key_action_sql(value: Option<&str>) -> Option<String> {
    let value = value?.trim();
    if value.is_empty() {
        return None;
    }
    let action = value
        .split_whitespace()
        .map(str::to_ascii_uppercase)
        .collect::<Vec<_>>()
        .join(" ");
    match action.as_str() {
        "CASCADE" | "RESTRICT" | "NO ACTION" | "SET NULL" | "SET DEFAULT" => Some(action),
        _ => None,
    }
}

fn sync_supports_foreign_keys(target_db_type: &str) -> bool {
    !target_db_type.to_lowercase().contains("clickhouse")
}

fn generate_drop_foreign_key_sql(
    target_db_type: &str,
    table_name: &str,
    foreign_key_name: &str,
    dialect: &dyn SyncSqlDialect,
) -> String {
    let foreign_key_name = dialect.quote_identifier(foreign_key_name);
    let db_type = target_db_type.to_lowercase();
    if db_type.contains("mysql") || db_type.contains("mariadb") {
        format!(
            "ALTER TABLE {} DROP FOREIGN KEY {};",
            table_name, foreign_key_name
        )
    } else {
        format!(
            "ALTER TABLE {} DROP CONSTRAINT {};",
            table_name, foreign_key_name
        )
    }
}

fn table_designer_sync_statement(
    table_diff: &super::TableDiff,
    target_database: &str,
    target_db_type: &str,
    dialect: &dyn SyncSqlDialect,
) -> Option<SyncStatement> {
    let source = table_diff.source.as_ref()?;
    let target = table_diff.target.as_ref()?;
    let original = table_schema_to_design(target_database, target);
    let new = table_schema_to_compare_design(target_database, source, target);
    let sql = dialect.build_alter_table_sql(&original, &new)?;
    if !has_executable_sync_sql(&sql) {
        return None;
    }
    let safety = table_designer_statement_safety(table_diff, target_db_type);
    Some(SyncStatement {
        id: uuid::Uuid::new_v4().to_string(),
        sql,
        kind: SyncStatementKind::AlterTable,
        object_name: Some(table_diff.name.clone()),
        row_key: None,
        destructive: safety.destructive,
        transactional_safe: true,
        selected_by_default: safety.selected_by_default,
        warnings: safety.warnings,
    })
}

fn has_executable_sync_sql(sql: &str) -> bool {
    let sql = sql.trim();
    !sql.is_empty() && !sql.starts_with("-- No changes detected")
}

struct SyncStatementSafety {
    destructive: bool,
    selected_by_default: bool,
    warnings: Vec<String>,
}

fn table_designer_statement_safety(
    table_diff: &super::TableDiff,
    target_db_type: &str,
) -> SyncStatementSafety {
    let destructive = table_diff
        .column_diffs
        .iter()
        .any(|diff| matches!(diff.status, DiffStatus::Removed | DiffStatus::Modified))
        || table_diff.index_diffs.iter().any(|diff| {
            matches!(diff.status, DiffStatus::Removed | DiffStatus::Modified)
                && diff
                    .target
                    .as_ref()
                    .is_some_and(|index| is_primary_index(index))
        });
    let rebuilds_sqlite_table = destructive && target_db_type.to_lowercase().contains("sqlite");
    let modified_indexes = table_diff
        .index_diffs
        .iter()
        .any(|diff| matches!(diff.status, DiffStatus::Modified));
    let selected_by_default = !destructive && !modified_indexes;
    let warnings = if rebuilds_sqlite_table {
        vec!["SQLite 将通过重建表来同步此结构变更，请先确认数据备份".to_string()]
    } else if destructive {
        vec!["此操作会修改或删除目标表结构，请先确认数据备份".to_string()]
    } else if modified_indexes {
        vec!["此操作会重建目标索引，请先确认现有索引可被替换".to_string()]
    } else {
        vec![]
    };

    SyncStatementSafety {
        destructive,
        selected_by_default,
        warnings,
    }
}

/// 格式化值为 SQL 字面量
fn format_value(value: CellValue) -> String {
    match value {
        CellValue::Null => "NULL".to_string(),
        CellValue::Bool(b) => if b { "TRUE" } else { "FALSE" }.to_string(),
        CellValue::Number(n) => n.to_string(),
        CellValue::String(s) => format!("'{}'", s.replace('\'', "''")),
        _ => format!(
            "'{}'",
            serde_json::to_string(&value)
                .unwrap_or_else(|_| "NULL".to_string())
                .replace('\'', "''")
        ),
    }
}

fn sorted_key_values(
    key_values: &std::collections::HashMap<String, CellValue>,
) -> Vec<(&String, &CellValue)> {
    let mut values = key_values.iter().collect::<Vec<_>>();
    values.sort_by(|left, right| left.0.cmp(right.0));
    values
}

fn format_where_condition(column: &str, value: &CellValue, dialect: &dyn SyncSqlDialect) -> String {
    let column = dialect.quote_identifier(column);
    if value.is_null() {
        format!("{column} IS NULL")
    } else {
        format!("{} = {}", column, format_value(value.clone()))
    }
}

/// 提取行键
fn extract_row_key(
    row: &RowData,
    key_columns: &[String],
) -> Option<serde_json::Map<String, serde_json::Value>> {
    let mut map = serde_json::Map::new();
    for col in key_columns {
        if let Some(val) = row.get(col) {
            map.insert(col.clone(), val.clone());
        }
    }
    if map.is_empty() { None } else { Some(map) }
}

/// 从行中提取键值对
fn extract_key_values_from_row(
    row: &RowData,
    key_columns: &[String],
) -> std::collections::HashMap<String, CellValue> {
    key_columns
        .iter()
        .filter_map(|col| row.get(col).map(|v| (col.clone(), v.clone())))
        .collect()
}

/// 从结构比较结果生成同步计划
///
/// # P0 安全保护
/// - DROP TABLE 默认不选中
/// - DROP COLUMN 默认不选中
/// - 列类型修改默认不选中（可能导致数据丢失）
pub fn build_schema_sync_plan(result: &SchemaCompareResult, target_db_type: &str) -> SyncPlan {
    build_schema_sync_plan_with_dialect(result, "", None, target_db_type, &RawSyncSqlDialect)
}

pub fn build_schema_sync_plan_with_plugin(
    result: &SchemaCompareResult,
    target_database: &str,
    target_schema: Option<&str>,
    plugin: &dyn DatabasePlugin,
) -> SyncPlan {
    let target_db_type = format!("{:?}", plugin.name()).to_lowercase();
    build_schema_sync_plan_with_dialect(
        result,
        target_database,
        target_schema,
        &target_db_type,
        &PluginSyncSqlDialect(plugin),
    )
}

fn build_schema_sync_plan_with_dialect(
    result: &SchemaCompareResult,
    target_database: &str,
    target_schema: Option<&str>,
    target_db_type: &str,
    dialect: &dyn SyncSqlDialect,
) -> SyncPlan {
    let mut foreign_key_drops = Vec::new();
    let mut statements = Vec::new();
    let mut deferred_foreign_key_adds = Vec::new();
    let plan_id = uuid::Uuid::new_v4().to_string();

    for table_diff in &result.table_diffs {
        match table_diff.status {
            DiffStatus::Added => {
                if let Some(source_table) = &table_diff.source {
                    let stmt_id = uuid::Uuid::new_v4().to_string();
                    let mut design = table_schema_to_design(target_database, source_table);
                    design.foreign_keys.clear();
                    statements.push(SyncStatement {
                        id: stmt_id,
                        sql: dialect.build_create_table_sql(&design),
                        kind: SyncStatementKind::CreateTable,
                        object_name: Some(table_diff.name.clone()),
                        row_key: None,
                        destructive: false,
                        transactional_safe: true,
                        selected_by_default: true,
                        warnings: vec![],
                    });

                    if sync_supports_foreign_keys(target_db_type) {
                        let table_ref = dialect.format_table_reference(
                            target_database,
                            target_schema,
                            &source_table.name,
                        );
                        for foreign_key in &source_table.foreign_keys {
                            deferred_foreign_key_adds.push(SyncStatement {
                                id: uuid::Uuid::new_v4().to_string(),
                                sql: generate_add_foreign_key_sql(
                                    &table_ref,
                                    foreign_key,
                                    target_database,
                                    target_schema,
                                    dialect,
                                ),
                                kind: SyncStatementKind::AlterTable,
                                object_name: Some(foreign_key.name.clone()),
                                row_key: None,
                                destructive: false,
                                transactional_safe: true,
                                selected_by_default: true,
                                warnings: vec![],
                            });
                        }
                    }
                }
            }
            DiffStatus::Removed => {
                // 删除表 - P0 安全保护：默认不选中
                let stmt_id = uuid::Uuid::new_v4().to_string();
                let mut sql = dialect.drop_table(target_database, target_schema, &table_diff.name);
                if !sql.trim_end().ends_with(';') {
                    sql.push(';');
                }
                statements.push(SyncStatement {
                    id: stmt_id,
                    sql,
                    kind: SyncStatementKind::DropTable,
                    object_name: Some(table_diff.name.clone()),
                    row_key: None,
                    destructive: true,
                    transactional_safe: false,
                    selected_by_default: false,
                    warnings: vec!["此操作将删除表及其所有数据".to_string()],
                });
            }
            DiffStatus::Modified => {
                // 修改表
                let table_ref = dialect.format_table_reference(
                    target_database,
                    target_schema,
                    &table_diff.name,
                );
                let table_designer_statement = table_designer_sync_statement(
                    table_diff,
                    target_database,
                    target_db_type,
                    dialect,
                );
                let table_designer_handles_table = table_designer_statement.is_some();

                if !table_designer_handles_table && sync_supports_foreign_keys(target_db_type) {
                    for fk_diff in &table_diff.foreign_key_diffs {
                        if matches!(fk_diff.status, DiffStatus::Removed | DiffStatus::Modified) {
                            let fk_name = fk_diff
                                .target
                                .as_ref()
                                .map(|foreign_key| foreign_key.name.as_str())
                                .unwrap_or(&fk_diff.name);
                            foreign_key_drops.push(SyncStatement {
                                id: uuid::Uuid::new_v4().to_string(),
                                sql: generate_drop_foreign_key_sql(
                                    target_db_type,
                                    &table_ref,
                                    fk_name,
                                    dialect,
                                ),
                                kind: SyncStatementKind::AlterTable,
                                object_name: Some(fk_diff.name.clone()),
                                row_key: None,
                                destructive: false,
                                transactional_safe: true,
                                selected_by_default: fk_diff.status == DiffStatus::Removed,
                                warnings: if fk_diff.status == DiffStatus::Modified {
                                    vec![
                                        "此操作会重建目标外键，请先确认现有外键可被替换"
                                            .to_string(),
                                    ]
                                } else {
                                    vec![]
                                },
                            });
                        }
                    }
                }

                if let Some(statement) = table_designer_statement {
                    statements.push(statement);
                } else {
                    for col_diff in &table_diff.column_diffs {
                        let stmt_id = uuid::Uuid::new_v4().to_string();

                        match col_diff.status {
                            DiffStatus::Added => {
                                if let Some(src) = &col_diff.source {
                                    let column = column_schema_to_definition(src);
                                    statements.push(SyncStatement {
                                        id: stmt_id,
                                        sql: format!(
                                            "ALTER TABLE {} ADD COLUMN {};",
                                            table_ref,
                                            dialect.build_column_def(&column)
                                        ),
                                        kind: SyncStatementKind::AlterTable,
                                        object_name: Some(table_diff.name.clone()),
                                        row_key: None,
                                        destructive: false,
                                        transactional_safe: true,
                                        selected_by_default: true,
                                        warnings: vec![],
                                    });
                                }
                            }
                            DiffStatus::Removed => {
                                // 删除列 - P0 安全保护：默认不选中
                                statements.push(SyncStatement {
                                    id: stmt_id,
                                    sql: format!(
                                        "ALTER TABLE {} DROP COLUMN {};",
                                        table_ref,
                                        dialect.quote_identifier(&col_diff.name)
                                    ),
                                    kind: SyncStatementKind::AlterTable,
                                    object_name: Some(table_diff.name.clone()),
                                    row_key: None,
                                    destructive: true,
                                    transactional_safe: true,
                                    selected_by_default: false,
                                    warnings: vec!["此操作将删除列及其所有数据".to_string()],
                                });
                            }
                            DiffStatus::Modified => {
                                // 修改列 - P0 安全保护：默认不选中（可能导致数据丢失）
                                if let Some(src) = &col_diff.source {
                                    let sql = if target_db_type == "mysql"
                                        || target_db_type == "mariadb"
                                    {
                                        let column = column_schema_to_definition(src);
                                        format!(
                                            "ALTER TABLE {} MODIFY COLUMN {};",
                                            table_ref,
                                            dialect.build_column_def(&column)
                                        )
                                    } else {
                                        format!(
                                            "ALTER TABLE {} ALTER COLUMN {} TYPE {};",
                                            table_ref,
                                            dialect.quote_identifier(&src.name),
                                            src.data_type
                                        )
                                    };

                                    statements.push(SyncStatement {
                                        id: stmt_id,
                                        sql,
                                        kind: SyncStatementKind::AlterTable,
                                        object_name: Some(table_diff.name.clone()),
                                        row_key: None,
                                        destructive: true,
                                        transactional_safe: true,
                                        selected_by_default: false,
                                        warnings: vec![
                                            "此操作可能导致数据类型转换失败或数据丢失".to_string(),
                                        ],
                                    });
                                }
                            }
                        }
                    }

                    // 索引差异
                    for idx_diff in &table_diff.index_diffs {
                        let stmt_id = uuid::Uuid::new_v4().to_string();

                        match idx_diff.status {
                            DiffStatus::Added => {
                                if let Some(index) = &idx_diff.source {
                                    let table_ref = dialect.format_table_reference(
                                        target_database,
                                        target_schema,
                                        &table_diff.name,
                                    );
                                    if is_primary_index(index) {
                                        statements.push(SyncStatement {
                                            id: stmt_id,
                                            sql: generate_add_primary_key_sql(
                                                &table_ref, index, dialect,
                                            ),
                                            kind: SyncStatementKind::AlterTable,
                                            object_name: Some(idx_diff.name.clone()),
                                            row_key: None,
                                            destructive: false,
                                            transactional_safe: true,
                                            selected_by_default: true,
                                            warnings: vec![],
                                        });
                                    } else {
                                        statements.push(SyncStatement {
                                            id: stmt_id,
                                            sql: generate_create_index_sql(
                                                &table_ref, index, dialect,
                                            ),
                                            kind: SyncStatementKind::CreateIndex,
                                            object_name: Some(idx_diff.name.clone()),
                                            row_key: None,
                                            destructive: false,
                                            transactional_safe: true,
                                            selected_by_default: true,
                                            warnings: vec![],
                                        });
                                    }
                                }
                            }
                            DiffStatus::Removed => {
                                let table_ref = dialect.format_table_reference(
                                    target_database,
                                    target_schema,
                                    &table_diff.name,
                                );
                                let index_name = idx_diff
                                    .target
                                    .as_ref()
                                    .map(|index| index.name.as_str())
                                    .unwrap_or(&idx_diff.name);
                                if is_primary_index_name(index_name) {
                                    statements.push(SyncStatement {
                                        id: stmt_id,
                                        sql: generate_drop_primary_key_sql(
                                            target_db_type,
                                            &table_ref,
                                            index_name,
                                            dialect,
                                        ),
                                        kind: SyncStatementKind::AlterTable,
                                        object_name: Some(idx_diff.name.clone()),
                                        row_key: None,
                                        destructive: false,
                                        transactional_safe: true,
                                        selected_by_default: false,
                                        warnings: vec![
                                            "此操作会删除目标主键约束，请先确认依赖关系"
                                                .to_string(),
                                        ],
                                    });
                                } else {
                                    statements.push(SyncStatement {
                                        id: stmt_id,
                                        sql: generate_drop_index_sql(
                                            target_db_type,
                                            &table_ref,
                                            index_name,
                                            dialect,
                                        ),
                                        kind: SyncStatementKind::DropIndex,
                                        object_name: Some(idx_diff.name.clone()),
                                        row_key: None,
                                        destructive: false,
                                        transactional_safe: true,
                                        selected_by_default: true,
                                        warnings: vec![],
                                    });
                                }
                            }
                            DiffStatus::Modified => {
                                if let Some(index) = &idx_diff.source {
                                    let drop_index_name = idx_diff
                                        .target
                                        .as_ref()
                                        .map(|index| index.name.as_str())
                                        .unwrap_or(&idx_diff.name);
                                    if is_primary_index(index)
                                        || is_primary_index_name(drop_index_name)
                                    {
                                        statements.push(SyncStatement {
                                            id: stmt_id,
                                            sql: generate_drop_primary_key_sql(
                                                target_db_type,
                                                &table_ref,
                                                drop_index_name,
                                                dialect,
                                            ),
                                            kind: SyncStatementKind::AlterTable,
                                            object_name: Some(idx_diff.name.clone()),
                                            row_key: None,
                                            destructive: false,
                                            transactional_safe: true,
                                            selected_by_default: false,
                                            warnings: vec![
                                                "此操作会重建目标主键约束，请先确认依赖关系"
                                                    .to_string(),
                                            ],
                                        });
                                        statements.push(SyncStatement {
                                            id: uuid::Uuid::new_v4().to_string(),
                                            sql: generate_add_primary_key_sql(
                                                &table_ref, index, dialect,
                                            ),
                                            kind: SyncStatementKind::AlterTable,
                                            object_name: Some(idx_diff.name.clone()),
                                            row_key: None,
                                            destructive: false,
                                            transactional_safe: true,
                                            selected_by_default: false,
                                            warnings: vec![
                                                "此操作会重建目标主键约束，请先确认依赖关系"
                                                    .to_string(),
                                            ],
                                        });
                                    } else {
                                        statements.push(SyncStatement {
                                            id: stmt_id,
                                            sql: generate_drop_index_sql(
                                                target_db_type,
                                                &table_ref,
                                                drop_index_name,
                                                dialect,
                                            ),
                                            kind: SyncStatementKind::DropIndex,
                                            object_name: Some(idx_diff.name.clone()),
                                            row_key: None,
                                            destructive: false,
                                            transactional_safe: true,
                                            selected_by_default: false,
                                            warnings: vec![
                                                "此操作会重建目标索引，请先确认现有索引可被替换"
                                                    .to_string(),
                                            ],
                                        });
                                        statements.push(SyncStatement {
                                            id: uuid::Uuid::new_v4().to_string(),
                                            sql: generate_create_index_sql(
                                                &table_ref, index, dialect,
                                            ),
                                            kind: SyncStatementKind::CreateIndex,
                                            object_name: Some(idx_diff.name.clone()),
                                            row_key: None,
                                            destructive: false,
                                            transactional_safe: true,
                                            selected_by_default: false,
                                            warnings: vec![
                                                "此操作会重建目标索引，请先确认现有索引可被替换"
                                                    .to_string(),
                                            ],
                                        });
                                    }
                                }
                            }
                        }
                    }
                }

                if !table_designer_handles_table && sync_supports_foreign_keys(target_db_type) {
                    for fk_diff in &table_diff.foreign_key_diffs {
                        if matches!(fk_diff.status, DiffStatus::Added | DiffStatus::Modified) {
                            if let Some(foreign_key) = &fk_diff.source {
                                deferred_foreign_key_adds.push(SyncStatement {
                                    id: uuid::Uuid::new_v4().to_string(),
                                    sql: generate_add_foreign_key_sql(
                                        &table_ref,
                                        foreign_key,
                                        target_database,
                                        target_schema,
                                        dialect,
                                    ),
                                    kind: SyncStatementKind::AlterTable,
                                    object_name: Some(fk_diff.name.clone()),
                                    row_key: None,
                                    destructive: false,
                                    transactional_safe: true,
                                    selected_by_default: fk_diff.status == DiffStatus::Added,
                                    warnings: if fk_diff.status == DiffStatus::Modified {
                                        vec![
                                            "此操作会重建目标外键，请先确认现有外键可被替换"
                                                .to_string(),
                                        ]
                                    } else {
                                        vec![]
                                    },
                                });
                            }
                        }
                    }
                }
            }
        }
    }

    if !foreign_key_drops.is_empty() {
        let mut ordered_statements = foreign_key_drops;
        ordered_statements.extend(statements);
        statements = ordered_statements;
    }
    statements.extend(deferred_foreign_key_adds);

    let ddl_count = statements.len();
    let sql_text = statements
        .iter()
        .map(|s| s.sql.as_str())
        .collect::<Vec<_>>()
        .join("\n");

    SyncPlan {
        id: plan_id,
        target_table: "".to_string(),
        statements,
        summary: SyncPlanSummary {
            insert_count: 0,
            update_count: 0,
            delete_count: 0,
            ddl_count,
            total_count: ddl_count,
        },
        warnings: vec![],
        sql_text,
    }
}

fn table_schema_to_design(database: &str, table: &TableSchema) -> TableDesign {
    let mut design = TableDesign::new(database, table.name.clone());
    let primary_columns = table
        .indexes
        .iter()
        .find(|index| is_primary_index(index))
        .map(|index| index.columns.clone())
        .unwrap_or_default();
    design.columns = table
        .columns
        .iter()
        .map(|column| {
            let mut definition = column_schema_to_definition(column);
            if primary_columns
                .iter()
                .any(|primary_column| primary_column.eq_ignore_ascii_case(&column.name))
            {
                definition = definition.primary_key(true);
            }
            definition
        })
        .collect();
    design.indexes = table
        .indexes
        .iter()
        .filter(|index| !is_primary_index(index))
        .map(|index| {
            IndexDefinition::new(index.name.clone())
                .columns(index.columns.clone())
                .unique(index.unique)
        })
        .collect();
    design.foreign_keys = table
        .foreign_keys
        .iter()
        .map(|foreign_key| ForeignKeyDefinition {
            name: foreign_key.name.clone(),
            columns: foreign_key.columns.clone(),
            ref_table: foreign_key.ref_table.clone(),
            ref_columns: foreign_key.ref_columns.clone(),
            on_delete: foreign_key.on_delete.clone().unwrap_or_default(),
            on_update: foreign_key.on_update.clone().unwrap_or_default(),
        })
        .collect();
    design.options.comment = table.comment.clone().unwrap_or_default();
    design.options.engine = table.engine.clone();
    design.options.charset = table.charset.clone();
    design.options.collation = table.collation.clone();
    design
}

fn table_schema_to_compare_design(
    database: &str,
    source: &TableSchema,
    target: &TableSchema,
) -> TableDesign {
    let mut design = table_schema_to_design(database, source);
    design.columns = compare_sync_columns_ignoring_order(design.columns, target);
    design
}

fn compare_sync_columns_ignoring_order(
    source_columns: Vec<ColumnDefinition>,
    target: &TableSchema,
) -> Vec<ColumnDefinition> {
    let source_by_exact_name = source_columns
        .iter()
        .enumerate()
        .map(|(index, column)| (column.name.trim().to_string(), index))
        .collect::<HashMap<_, _>>();
    let source_by_folded_name = folded_source_column_map(&source_columns);
    let mut used_indexes = HashSet::new();
    let mut ordered = Vec::with_capacity(source_columns.len());
    for target_column in &target.columns {
        let index = source_by_exact_name
            .get(target_column.name.trim())
            .copied()
            .or_else(|| {
                source_by_folded_name
                    .get(&sync_identifier_key(&target_column.name))
                    .and_then(|indexes| {
                        indexes
                            .iter()
                            .copied()
                            .find(|index| !used_indexes.contains(index))
                    })
            });
        if let Some(index) = index {
            used_indexes.insert(index);
            ordered.push(source_columns[index].clone());
        }
    }
    for (index, column) in source_columns.into_iter().enumerate() {
        if !used_indexes.contains(&index) {
            ordered.push(column);
        }
    }
    ordered
}

fn folded_source_column_map(source_columns: &[ColumnDefinition]) -> HashMap<String, Vec<usize>> {
    let mut source_by_key = HashMap::<String, Vec<usize>>::new();
    for (index, column) in source_columns.iter().enumerate() {
        source_by_key
            .entry(sync_identifier_key(&column.name))
            .or_default()
            .push(index);
    }
    source_by_key
}

fn sync_identifier_key(value: &str) -> String {
    value.trim().to_lowercase()
}

fn column_schema_to_definition(column: &ColumnSchema) -> ColumnDefinition {
    let mut definition = ColumnDefinition::new(column.name.clone())
        .data_type(column.data_type.clone())
        .nullable(column.nullable);
    if let Some(default_value) = executable_column_default(column) {
        definition = definition.default_value(default_value.clone());
    }
    if let Some(comment) = &column.comment {
        definition = definition.comment(comment.clone());
    }
    definition.charset = column.charset.clone();
    definition.collation = column.collation.clone();
    definition
}

fn executable_column_default(column: &ColumnSchema) -> Option<String> {
    let default = column.default_value.as_deref()?.trim();
    if default.is_empty() {
        return Some(String::new());
    }
    if is_raw_sql_default(default) {
        return Some(default.to_string());
    }
    Some(format!("'{}'", default.replace('\'', "''")))
}

fn is_raw_sql_default(default: &str) -> bool {
    let upper = default.to_ascii_uppercase();
    is_quoted_literal(default)
        || is_numeric_literal(default)
        || is_known_default_keyword(&upper)
        || default.starts_with('(')
        || default.contains("::")
        || default.contains('(')
}

fn is_quoted_literal(default: &str) -> bool {
    (default.starts_with('\'') && default.ends_with('\''))
        || (default.starts_with('"') && default.ends_with('"'))
        || default.starts_with("b'")
        || default.starts_with("B'")
        || default.starts_with("x'")
        || default.starts_with("X'")
}

fn is_numeric_literal(default: &str) -> bool {
    default.parse::<i64>().is_ok() || default.parse::<f64>().is_ok()
}

fn is_known_default_keyword(upper: &str) -> bool {
    matches!(
        upper,
        "NULL"
            | "TRUE"
            | "FALSE"
            | "CURRENT_TIMESTAMP"
            | "CURRENT_DATE"
            | "CURRENT_TIME"
            | "LOCALTIMESTAMP"
            | "LOCALTIME"
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::collections::HashMap;

    fn create_row(data: &[(&str, serde_json::Value)]) -> RowData {
        data.iter()
            .map(|(k, v)| (k.to_string(), v.clone()))
            .collect()
    }

    #[test]
    fn test_build_data_sync_plan_with_all_types() {
        let mut changes = HashMap::new();
        changes.insert("name".to_string(), (json!("Alice"), json!("Alicia")));

        let result = DataCompareResult {
            source_table: "users".to_string(),
            target_table: "users".to_string(),
            key_columns: vec!["id".to_string()],
            columns: vec!["id".to_string(), "name".to_string()],
            added: vec![create_row(&[("id", json!(2)), ("name", json!("Bob"))])],
            removed: vec![create_row(&[("id", json!(3)), ("name", json!("Charlie"))])],
            modified: vec![super::super::DataCompareModifiedRow {
                key_values: {
                    let mut m = HashMap::new();
                    m.insert("id".to_string(), json!(1));
                    m
                },
                source_values: create_row(&[("id", json!(1)), ("name", json!("Alice"))]),
                target_values: create_row(&[("id", json!(1)), ("name", json!("Alicia"))]),
                changes,
            }],
            source_truncated: false,
            target_truncated: false,
        };

        let plan = build_data_sync_plan(&result);

        assert_eq!(plan.summary.insert_count, 1);
        assert_eq!(plan.summary.update_count, 1);
        assert_eq!(plan.summary.delete_count, 1);
        assert_eq!(plan.summary.total_count, 3);

        // 检查 INSERT 语句
        let insert_stmt = plan
            .statements
            .iter()
            .find(|s| matches!(s.kind, SyncStatementKind::Insert))
            .unwrap();
        assert!(insert_stmt.sql.contains("INSERT INTO users"));
        assert!(insert_stmt.selected_by_default);
        assert!(!insert_stmt.destructive);

        // 检查 DELETE 语句（P0 安全保护）
        let delete_stmt = plan
            .statements
            .iter()
            .find(|s| matches!(s.kind, SyncStatementKind::Delete))
            .unwrap();
        assert!(delete_stmt.sql.contains("DELETE FROM users"));
        assert!(!delete_stmt.selected_by_default); // P0: 默认不选中
        assert!(delete_stmt.destructive); // P0: 标记为破坏性
        assert!(!delete_stmt.warnings.is_empty()); // P0: 有警告信息
    }

    #[test]
    fn test_format_value_handles_sql_injection() {
        let value = CellValue::String("'; DROP TABLE users; --".to_string());
        let formatted = format_value(value);
        assert_eq!(formatted, "'''; DROP TABLE users; --'");
        assert!(formatted.contains("''"));
    }

    #[test]
    fn test_generate_insert_sql() {
        let row = create_row(&[("id", json!(1)), ("name", json!("Alice"))]);
        let columns = vec!["id".to_string(), "name".to_string()];
        let dialect = RawSyncSqlDialect;

        let sql = generate_insert_sql("users", &row, &columns, &dialect);

        assert!(sql.contains("INSERT INTO users"));
        assert!(sql.contains("id, name"));
        assert!(sql.contains("VALUES"));
    }

    #[test]
    fn test_generate_update_sql() {
        let source_values = create_row(&[("id", json!(1)), ("name", json!("Alice"))]);
        let mut key_values = HashMap::new();
        key_values.insert("id".to_string(), json!(1));
        let mut changes = HashMap::new();
        changes.insert("name".to_string(), (json!("Alicia"), json!("Alice")));
        let dialect = RawSyncSqlDialect;

        let sql = generate_update_sql("users", &source_values, &key_values, &changes, &dialect);

        assert!(sql.contains("UPDATE users"));
        assert!(sql.contains("SET"));
        assert!(sql.contains("WHERE"));
        assert!(sql.contains("id = 1"));
    }

    #[test]
    fn test_generate_delete_sql() {
        let mut key_values = HashMap::new();
        key_values.insert("id".to_string(), json!(1));
        let dialect = RawSyncSqlDialect;

        let sql = generate_delete_sql("users", &key_values, &dialect);

        assert!(sql.contains("DELETE FROM users"));
        assert!(sql.contains("WHERE id = 1"));
    }

    #[test]
    fn test_build_schema_sync_plan_marks_destructive_unselected() {
        use super::super::{ColumnSchema, DiffStatus, SchemaCompareResult, TableDiff, TableSchema};

        let users = TableSchema {
            name: "users".to_string(),
            columns: vec![ColumnSchema {
                name: "id".to_string(),
                data_type: "int".to_string(),
                nullable: false,
                default_value: None,
                comment: None,
                ..Default::default()
            }],
            indexes: vec![],
            foreign_keys: vec![],
            comment: None,
            ..Default::default()
        };

        let result = SchemaCompareResult {
            table_diffs: vec![
                TableDiff {
                    name: "users".to_string(),
                    status: DiffStatus::Modified,
                    source: Some(users.clone()),
                    target: Some(users),
                    column_diffs: vec![
                        super::super::ColumnDiff {
                            name: "email".to_string(),
                            status: DiffStatus::Added,
                            source: Some(ColumnSchema {
                                name: "email".to_string(),
                                data_type: "text".to_string(),
                                nullable: true,
                                default_value: None,
                                comment: None,
                                ..Default::default()
                            }),
                            target: None,
                        },
                        super::super::ColumnDiff {
                            name: "legacy".to_string(),
                            status: DiffStatus::Removed,
                            source: None,
                            target: Some(ColumnSchema {
                                name: "legacy".to_string(),
                                data_type: "text".to_string(),
                                nullable: true,
                                default_value: None,
                                comment: None,
                                ..Default::default()
                            }),
                        },
                    ],
                    index_diffs: vec![],
                    foreign_key_diffs: vec![],
                    comment_changed: false,
                },
                TableDiff {
                    name: "audit".to_string(),
                    status: DiffStatus::Removed,
                    source: None,
                    target: Some(TableSchema {
                        name: "audit".to_string(),
                        columns: vec![],
                        indexes: vec![],
                        foreign_keys: vec![],
                        comment: None,
                        ..Default::default()
                    }),
                    column_diffs: vec![],
                    index_diffs: vec![],
                    foreign_key_diffs: vec![],
                    comment_changed: false,
                },
            ],
            added_count: 0,
            removed_count: 1,
            modified_count: 1,
        };

        let plan = build_schema_sync_plan(&result, "postgresql");

        // 检查新增列（应该默认选中）
        let add_col = plan
            .statements
            .iter()
            .find(|s| s.sql.contains("ADD COLUMN email"))
            .unwrap();
        assert!(add_col.selected_by_default);
        assert!(!add_col.destructive);

        // 检查删除列（P0: 应该默认不选中）
        let drop_col = plan
            .statements
            .iter()
            .find(|s| s.sql.contains("DROP COLUMN legacy"))
            .unwrap();
        assert!(!drop_col.selected_by_default);
        assert!(drop_col.destructive);
        assert!(!drop_col.warnings.is_empty());

        // 检查删除表（P0: 应该默认不选中）
        let drop_table = plan
            .statements
            .iter()
            .find(|s| s.sql.contains("DROP TABLE"))
            .unwrap();
        assert!(!drop_table.selected_by_default);
        assert!(drop_table.destructive);
        assert!(!drop_table.warnings.is_empty());
    }

    #[test]
    fn test_schema_sync_plan_generates_create_table_for_added_table() {
        use super::super::{ColumnSchema, DiffStatus, SchemaCompareResult, TableDiff, TableSchema};

        let source = TableSchema {
            name: "users".to_string(),
            columns: vec![
                ColumnSchema {
                    name: "id".to_string(),
                    data_type: "int".to_string(),
                    nullable: false,
                    default_value: None,
                    comment: None,
                    ..Default::default()
                },
                ColumnSchema {
                    name: "name".to_string(),
                    data_type: "varchar(64)".to_string(),
                    nullable: true,
                    default_value: Some("'anonymous'".to_string()),
                    comment: None,
                    ..Default::default()
                },
            ],
            indexes: vec![],
            foreign_keys: vec![],
            comment: None,
            ..Default::default()
        };
        let result = SchemaCompareResult {
            table_diffs: vec![TableDiff {
                name: "users".to_string(),
                status: DiffStatus::Added,
                source: Some(source),
                target: None,
                column_diffs: vec![],
                index_diffs: vec![],
                foreign_key_diffs: vec![],
                comment_changed: false,
            }],
            added_count: 1,
            removed_count: 0,
            modified_count: 0,
        };

        let plan = build_schema_sync_plan(&result, "postgresql");

        let statement = plan.statements.first().unwrap();
        assert!(matches!(statement.kind, SyncStatementKind::CreateTable));
        assert_eq!(
            statement.sql,
            "CREATE TABLE users (\n  id int NOT NULL,\n  name varchar(64) NULL DEFAULT 'anonymous'\n);"
        );
        assert!(statement.selected_by_default);
        assert!(!statement.destructive);
    }

    #[test]
    fn test_where_clause_uses_is_null_for_null_key_values() {
        let mut key_values = HashMap::new();
        key_values.insert("tenant_id".to_string(), CellValue::Null);
        key_values.insert("id".to_string(), json!(1));
        let dialect = RawSyncSqlDialect;

        let sql = generate_delete_sql("users", &key_values, &dialect);

        assert!(sql.contains("id = 1"));
        assert!(sql.contains("tenant_id IS NULL"));
        assert!(!sql.contains("tenant_id = NULL"));
    }

    #[test]
    fn test_data_sync_plan_with_plugin_quotes_keyword_identifiers() {
        let result = DataCompareResult {
            source_table: "select".to_string(),
            target_table: "select".to_string(),
            key_columns: vec!["from".to_string()],
            columns: vec!["from".to_string(), "order".to_string()],
            added: vec![create_row(&[("from", json!(1)), ("order", json!("new"))])],
            removed: vec![],
            modified: vec![super::super::DataCompareModifiedRow {
                key_values: {
                    let mut values = HashMap::new();
                    values.insert("from".to_string(), json!(2));
                    values
                },
                source_values: create_row(&[("from", json!(2)), ("order", json!("updated"))]),
                target_values: create_row(&[("from", json!(2)), ("order", json!("old"))]),
                changes: {
                    let mut changes = HashMap::new();
                    changes.insert("order".to_string(), (json!("updated"), json!("old")));
                    changes
                },
            }],
            source_truncated: false,
            target_truncated: false,
        };
        let plugin = crate::mysql::MySqlPlugin::new();

        let plan = build_data_sync_plan_with_plugin(&result, "app", None, &plugin);

        assert!(plan.sql_text.contains("INSERT INTO `app`.`select`"));
        assert!(plan.sql_text.contains("(`from`, `order`)"));
        assert!(plan.sql_text.contains("UPDATE `app`.`select`"));
        assert!(plan.sql_text.contains("`order` = 'updated'"));
        assert!(plan.sql_text.contains("WHERE `from` = 2"));
    }

    #[test]
    fn test_schema_sync_plan_uses_index_details_for_create_index() {
        use super::super::{
            DiffStatus, IndexDiff, IndexSchema, SchemaCompareResult, TableDiff, TableSchema,
        };

        let table = TableSchema {
            name: "users".to_string(),
            columns: vec![],
            indexes: vec![],
            foreign_keys: vec![],
            comment: None,
            ..Default::default()
        };
        let result = SchemaCompareResult {
            table_diffs: vec![TableDiff {
                name: "users".to_string(),
                status: DiffStatus::Modified,
                source: Some(table.clone()),
                target: Some(table),
                column_diffs: vec![],
                index_diffs: vec![IndexDiff {
                    name: "idx_users_email".to_string(),
                    status: DiffStatus::Added,
                    source: Some(IndexSchema {
                        name: "idx_users_email".to_string(),
                        columns: vec!["email".to_string()],
                        unique: true,
                    }),
                    target: None,
                }],
                foreign_key_diffs: vec![],
                comment_changed: false,
            }],
            added_count: 0,
            removed_count: 0,
            modified_count: 1,
        };

        let plan = build_schema_sync_plan(&result, "postgresql");

        assert_eq!(
            plan.statements.first().unwrap().sql,
            "CREATE UNIQUE INDEX idx_users_email ON users (email);"
        );
    }

    #[test]
    fn test_mysql_schema_sync_plan_drops_index_with_table_name() {
        use super::super::{
            DiffStatus, IndexDiff, IndexSchema, SchemaCompareResult, TableDiff, TableSchema,
        };

        let table = TableSchema {
            name: "users".to_string(),
            columns: vec![],
            indexes: vec![],
            foreign_keys: vec![],
            comment: None,
            ..Default::default()
        };
        let result = SchemaCompareResult {
            table_diffs: vec![TableDiff {
                name: "users".to_string(),
                status: DiffStatus::Modified,
                source: Some(table.clone()),
                target: Some(table),
                column_diffs: vec![],
                index_diffs: vec![IndexDiff {
                    name: "UK89mj1edq3g8hfxqea6pts53lr".to_string(),
                    status: DiffStatus::Removed,
                    source: None,
                    target: Some(IndexSchema {
                        name: "UK89mj1edq3g8hfxqea6pts53lr".to_string(),
                        columns: vec!["email".to_string()],
                        unique: true,
                    }),
                }],
                foreign_key_diffs: vec![],
                comment_changed: false,
            }],
            added_count: 0,
            removed_count: 0,
            modified_count: 1,
        };
        let plugin = crate::mysql::MySqlPlugin::new();

        let plan = build_schema_sync_plan_with_plugin(&result, "app", None, &plugin);

        assert_eq!(
            plan.statements.first().unwrap().sql,
            "ALTER TABLE `app`.`users` DROP INDEX `UK89mj1edq3g8hfxqea6pts53lr`;"
        );
    }

    #[test]
    fn test_mysql_schema_sync_plan_rebuilds_modified_index() {
        use super::super::{
            DiffStatus, IndexDiff, IndexSchema, SchemaCompareResult, TableDiff, TableSchema,
        };

        let table = TableSchema {
            name: "users".to_string(),
            columns: vec![],
            indexes: vec![],
            foreign_keys: vec![],
            comment: None,
            ..Default::default()
        };
        let result = SchemaCompareResult {
            table_diffs: vec![TableDiff {
                name: "users".to_string(),
                status: DiffStatus::Modified,
                source: Some(table.clone()),
                target: Some(table),
                column_diffs: vec![],
                index_diffs: vec![IndexDiff {
                    name: "idx_users_email".to_string(),
                    status: DiffStatus::Modified,
                    source: Some(IndexSchema {
                        name: "idx_users_email".to_string(),
                        columns: vec!["email".to_string()],
                        unique: true,
                    }),
                    target: Some(IndexSchema {
                        name: "idx_users_email".to_string(),
                        columns: vec!["legacy_email".to_string()],
                        unique: false,
                    }),
                }],
                foreign_key_diffs: vec![],
                comment_changed: false,
            }],
            added_count: 0,
            removed_count: 0,
            modified_count: 1,
        };
        let plugin = crate::mysql::MySqlPlugin::new();

        let plan = build_schema_sync_plan_with_plugin(&result, "app", None, &plugin);
        let sql = plan
            .statements
            .iter()
            .map(|statement| statement.sql.as_str())
            .collect::<Vec<_>>();

        assert_eq!(
            sql,
            vec![
                "ALTER TABLE `app`.`users` DROP INDEX `idx_users_email`;",
                "CREATE UNIQUE INDEX `idx_users_email` ON `app`.`users` (`email`);",
            ]
        );
        assert!(
            plan.statements
                .iter()
                .all(|statement| !statement.selected_by_default)
        );
    }

    #[test]
    fn test_mysql_schema_sync_plan_rebuilds_modified_primary_key() {
        use super::super::{
            DiffStatus, IndexDiff, IndexSchema, SchemaCompareResult, TableDiff, TableSchema,
        };

        let table = TableSchema {
            name: "users".to_string(),
            columns: vec![],
            indexes: vec![],
            foreign_keys: vec![],
            comment: None,
            ..Default::default()
        };
        let result = SchemaCompareResult {
            table_diffs: vec![TableDiff {
                name: "users".to_string(),
                status: DiffStatus::Modified,
                source: Some(table.clone()),
                target: Some(table),
                column_diffs: vec![],
                index_diffs: vec![IndexDiff {
                    name: "PRIMARY".to_string(),
                    status: DiffStatus::Modified,
                    source: Some(IndexSchema {
                        name: "PRIMARY".to_string(),
                        columns: vec!["uuid".to_string()],
                        unique: true,
                    }),
                    target: Some(IndexSchema {
                        name: "PRIMARY".to_string(),
                        columns: vec!["id".to_string()],
                        unique: true,
                    }),
                }],
                foreign_key_diffs: vec![],
                comment_changed: false,
            }],
            added_count: 0,
            removed_count: 0,
            modified_count: 1,
        };
        let plugin = crate::mysql::MySqlPlugin::new();

        let plan = build_schema_sync_plan_with_plugin(&result, "app", None, &plugin);
        let sql = plan
            .statements
            .iter()
            .map(|statement| statement.sql.as_str())
            .collect::<Vec<_>>();

        assert_eq!(
            sql,
            vec![
                "ALTER TABLE `app`.`users` DROP PRIMARY KEY;",
                "ALTER TABLE `app`.`users` ADD PRIMARY KEY (`uuid`);",
            ]
        );
        assert!(
            plan.statements
                .iter()
                .all(|statement| !statement.selected_by_default)
        );
    }

    #[test]
    fn test_mysql_schema_sync_plan_reuses_table_designer_alter_sql() {
        use super::super::{
            ColumnDiff, ColumnSchema, DiffStatus, SchemaCompareResult, TableDiff, TableSchema,
        };

        let target = TableSchema {
            name: "users".to_string(),
            columns: vec![ColumnSchema {
                name: "id".to_string(),
                data_type: "int".to_string(),
                nullable: false,
                default_value: None,
                comment: None,
                ..Default::default()
            }],
            indexes: vec![],
            foreign_keys: vec![],
            comment: None,
            ..Default::default()
        };
        let source = TableSchema {
            name: "users".to_string(),
            columns: vec![
                ColumnSchema {
                    name: "id".to_string(),
                    data_type: "int".to_string(),
                    nullable: false,
                    default_value: None,
                    comment: None,
                    ..Default::default()
                },
                ColumnSchema {
                    name: "email".to_string(),
                    data_type: "varchar(255)".to_string(),
                    nullable: true,
                    default_value: None,
                    comment: None,
                    ..Default::default()
                },
            ],
            indexes: vec![],
            foreign_keys: vec![],
            comment: None,
            ..Default::default()
        };
        let result = SchemaCompareResult {
            table_diffs: vec![TableDiff {
                name: "users".to_string(),
                status: DiffStatus::Modified,
                source: Some(source),
                target: Some(target),
                column_diffs: vec![ColumnDiff {
                    name: "email".to_string(),
                    status: DiffStatus::Added,
                    source: Some(ColumnSchema {
                        name: "email".to_string(),
                        data_type: "varchar(255)".to_string(),
                        nullable: true,
                        default_value: None,
                        comment: None,
                        ..Default::default()
                    }),
                    target: None,
                }],
                index_diffs: vec![],
                foreign_key_diffs: vec![],
                comment_changed: false,
            }],
            added_count: 0,
            removed_count: 0,
            modified_count: 1,
        };
        let plugin = crate::mysql::MySqlPlugin::new();

        let plan = build_schema_sync_plan_with_plugin(&result, "app", None, &plugin);

        assert_eq!(1, plan.statements.len());
        assert_eq!(
            plan.statements.first().unwrap().sql,
            "ALTER TABLE `users` ADD COLUMN `email` varchar(255) AFTER `id`;"
        );
        assert!(plan.statements.first().unwrap().selected_by_default);
    }

    #[test]
    fn test_mysql_schema_sync_plan_ignores_existing_column_order_changes() {
        use super::super::{ColumnSchema, DiffStatus, SchemaCompareResult, TableDiff, TableSchema};

        fn column(name: &str) -> ColumnSchema {
            ColumnSchema {
                name: name.to_string(),
                data_type: "varchar(64)".to_string(),
                nullable: true,
                default_value: None,
                comment: None,
                ..Default::default()
            }
        }

        let source = TableSchema {
            name: "users".to_string(),
            columns: vec![column("name"), column("id")],
            indexes: vec![],
            foreign_keys: vec![],
            comment: Some("source comment".to_string()),
            ..Default::default()
        };
        let target = TableSchema {
            name: "users".to_string(),
            columns: vec![column("id"), column("name")],
            indexes: vec![],
            foreign_keys: vec![],
            comment: Some("target comment".to_string()),
            ..Default::default()
        };
        let result = SchemaCompareResult {
            table_diffs: vec![TableDiff {
                name: "users".to_string(),
                status: DiffStatus::Modified,
                source: Some(source),
                target: Some(target),
                column_diffs: vec![],
                index_diffs: vec![],
                foreign_key_diffs: vec![],
                comment_changed: true,
            }],
            added_count: 0,
            removed_count: 0,
            modified_count: 1,
        };
        let plugin = crate::mysql::MySqlPlugin::new();

        let plan = build_schema_sync_plan_with_plugin(&result, "app", None, &plugin);

        assert_eq!(1, plan.statements.len());
        assert_eq!(
            plan.statements.first().unwrap().sql,
            "ALTER TABLE `users` COMMENT='source comment';"
        );
        assert!(!plan.sql_text.contains("MODIFY COLUMN"));
        assert!(!plan.sql_text.contains(" AFTER "));
        assert!(!plan.sql_text.contains(" FIRST"));
    }

    #[test]
    fn test_mysql_schema_sync_plan_quotes_string_metadata_defaults() {
        use super::super::{ColumnSchema, DiffStatus, SchemaCompareResult, TableDiff, TableSchema};

        let target = TableSchema {
            name: "users".to_string(),
            columns: vec![ColumnSchema {
                name: "id".to_string(),
                data_type: "int".to_string(),
                nullable: false,
                default_value: None,
                comment: None,
                ..Default::default()
            }],
            indexes: vec![],
            foreign_keys: vec![],
            comment: None,
            ..Default::default()
        };
        let source = TableSchema {
            name: "users".to_string(),
            columns: vec![
                ColumnSchema {
                    name: "id".to_string(),
                    data_type: "int".to_string(),
                    nullable: false,
                    default_value: None,
                    comment: None,
                    ..Default::default()
                },
                ColumnSchema {
                    name: "status".to_string(),
                    data_type: "varchar(20)".to_string(),
                    nullable: false,
                    default_value: Some("active".to_string()),
                    comment: None,
                    ..Default::default()
                },
            ],
            indexes: vec![],
            foreign_keys: vec![],
            comment: None,
            ..Default::default()
        };
        let result = SchemaCompareResult {
            table_diffs: vec![TableDiff {
                name: "users".to_string(),
                status: DiffStatus::Modified,
                source: Some(source),
                target: Some(target),
                column_diffs: vec![],
                index_diffs: vec![],
                foreign_key_diffs: vec![],
                comment_changed: false,
            }],
            added_count: 0,
            removed_count: 0,
            modified_count: 1,
        };
        let plugin = crate::mysql::MySqlPlugin::new();

        let plan = build_schema_sync_plan_with_plugin(&result, "app", None, &plugin);

        assert!(plan.sql_text.contains("DEFAULT 'active'"));
        assert!(!plan.sql_text.contains("DEFAULT active"));
    }

    #[test]
    fn test_sqlite_schema_sync_plan_reuses_table_designer_recreate_sql() {
        use super::super::{
            ColumnDiff, ColumnSchema, DiffStatus, SchemaCompareResult, TableDiff, TableSchema,
        };

        let source = TableSchema {
            name: "users".to_string(),
            columns: vec![ColumnSchema {
                name: "id".to_string(),
                data_type: "INTEGER".to_string(),
                nullable: false,
                default_value: None,
                comment: None,
                ..Default::default()
            }],
            indexes: vec![],
            foreign_keys: vec![],
            comment: None,
            ..Default::default()
        };
        let target = TableSchema {
            name: "users".to_string(),
            columns: vec![
                ColumnSchema {
                    name: "id".to_string(),
                    data_type: "INTEGER".to_string(),
                    nullable: false,
                    default_value: None,
                    comment: None,
                    ..Default::default()
                },
                ColumnSchema {
                    name: "legacy".to_string(),
                    data_type: "TEXT".to_string(),
                    nullable: true,
                    default_value: None,
                    comment: None,
                    ..Default::default()
                },
            ],
            indexes: vec![],
            foreign_keys: vec![],
            comment: None,
            ..Default::default()
        };
        let result = SchemaCompareResult {
            table_diffs: vec![TableDiff {
                name: "users".to_string(),
                status: DiffStatus::Modified,
                source: Some(source),
                target: Some(target),
                column_diffs: vec![ColumnDiff {
                    name: "legacy".to_string(),
                    status: DiffStatus::Removed,
                    source: None,
                    target: Some(ColumnSchema {
                        name: "legacy".to_string(),
                        data_type: "TEXT".to_string(),
                        nullable: true,
                        default_value: None,
                        comment: None,
                        ..Default::default()
                    }),
                }],
                index_diffs: vec![],
                foreign_key_diffs: vec![],
                comment_changed: false,
            }],
            added_count: 0,
            removed_count: 0,
            modified_count: 1,
        };
        let plugin = crate::sqlite::SqlitePlugin::new();

        let plan = build_schema_sync_plan_with_plugin(&result, "main", None, &plugin);

        assert_eq!(1, plan.statements.len());
        let statement = plan.statements.first().unwrap();
        assert!(statement.sql.contains("create table \"users_dg_tmp\""));
        assert!(
            statement
                .sql
                .contains("insert into \"users_dg_tmp\"(\"id\")")
        );
        assert!(statement.sql.contains("drop table \"users\";"));
        assert!(statement.sql.contains("rename to \"users\";"));
        assert!(!statement.selected_by_default);
        assert!(statement.destructive);
    }

    #[test]
    fn test_sqlite_schema_sync_plan_keeps_simple_add_column_selected() {
        use super::super::{
            ColumnDiff, ColumnSchema, DiffStatus, SchemaCompareResult, TableDiff, TableSchema,
        };

        let target = TableSchema {
            name: "users".to_string(),
            columns: vec![ColumnSchema {
                name: "id".to_string(),
                data_type: "INTEGER".to_string(),
                nullable: false,
                default_value: None,
                comment: None,
                ..Default::default()
            }],
            indexes: vec![],
            foreign_keys: vec![],
            comment: None,
            ..Default::default()
        };
        let source = TableSchema {
            name: "users".to_string(),
            columns: vec![
                ColumnSchema {
                    name: "id".to_string(),
                    data_type: "INTEGER".to_string(),
                    nullable: false,
                    default_value: None,
                    comment: None,
                    ..Default::default()
                },
                ColumnSchema {
                    name: "email".to_string(),
                    data_type: "TEXT".to_string(),
                    nullable: true,
                    default_value: None,
                    comment: None,
                    ..Default::default()
                },
            ],
            indexes: vec![],
            foreign_keys: vec![],
            comment: None,
            ..Default::default()
        };
        let result = SchemaCompareResult {
            table_diffs: vec![TableDiff {
                name: "users".to_string(),
                status: DiffStatus::Modified,
                source: Some(source),
                target: Some(target),
                column_diffs: vec![ColumnDiff {
                    name: "email".to_string(),
                    status: DiffStatus::Added,
                    source: Some(ColumnSchema {
                        name: "email".to_string(),
                        data_type: "TEXT".to_string(),
                        nullable: true,
                        default_value: None,
                        comment: None,
                        ..Default::default()
                    }),
                    target: None,
                }],
                index_diffs: vec![],
                foreign_key_diffs: vec![],
                comment_changed: false,
            }],
            added_count: 0,
            removed_count: 0,
            modified_count: 1,
        };
        let plugin = crate::sqlite::SqlitePlugin::new();

        let plan = build_schema_sync_plan_with_plugin(&result, "main", None, &plugin);

        assert_eq!(1, plan.statements.len());
        let statement = plan.statements.first().unwrap();
        assert_eq!(
            statement.sql,
            "ALTER TABLE \"users\" ADD COLUMN \"email\" TEXT;"
        );
        assert!(statement.selected_by_default);
        assert!(!statement.destructive);
    }

    #[test]
    fn test_mysql_schema_sync_plan_generates_foreign_key_sync_sql() {
        use super::super::{
            DiffStatus, ForeignKeyDiff, ForeignKeySchema, SchemaCompareResult, TableDiff,
            TableSchema,
        };

        let source_table = TableSchema {
            name: "order_items".to_string(),
            columns: vec![],
            indexes: vec![],
            foreign_keys: vec![ForeignKeySchema {
                name: "fk_order_items_order".to_string(),
                columns: vec!["order_id".to_string()],
                ref_table: "orders".to_string(),
                ref_columns: vec!["id".to_string()],
                on_delete: None,
                on_update: None,
            }],
            comment: None,
            ..Default::default()
        };
        let target_table = TableSchema {
            name: "order_items".to_string(),
            columns: vec![],
            indexes: vec![],
            foreign_keys: vec![ForeignKeySchema {
                name: "fk_order_items_legacy".to_string(),
                columns: vec!["legacy_order_id".to_string()],
                ref_table: "orders".to_string(),
                ref_columns: vec!["id".to_string()],
                on_delete: None,
                on_update: None,
            }],
            comment: None,
            ..Default::default()
        };
        let result = SchemaCompareResult {
            table_diffs: vec![TableDiff {
                name: "order_items".to_string(),
                status: DiffStatus::Modified,
                source: Some(source_table),
                target: Some(target_table),
                column_diffs: vec![],
                index_diffs: vec![],
                foreign_key_diffs: vec![
                    ForeignKeyDiff {
                        name: "fk_order_items_order".to_string(),
                        status: DiffStatus::Added,
                        source: Some(ForeignKeySchema {
                            name: "fk_order_items_order".to_string(),
                            columns: vec!["order_id".to_string()],
                            ref_table: "orders".to_string(),
                            ref_columns: vec!["id".to_string()],
                            on_delete: None,
                            on_update: None,
                        }),
                        target: None,
                    },
                    ForeignKeyDiff {
                        name: "fk_order_items_legacy".to_string(),
                        status: DiffStatus::Removed,
                        source: None,
                        target: Some(ForeignKeySchema {
                            name: "fk_order_items_legacy".to_string(),
                            columns: vec!["legacy_order_id".to_string()],
                            ref_table: "orders".to_string(),
                            ref_columns: vec!["id".to_string()],
                            on_delete: None,
                            on_update: None,
                        }),
                    },
                ],
                comment_changed: false,
            }],
            added_count: 0,
            removed_count: 0,
            modified_count: 1,
        };
        let plugin = crate::mysql::MySqlPlugin::new();

        let plan = build_schema_sync_plan_with_plugin(&result, "app", None, &plugin);
        let sql = plan
            .statements
            .iter()
            .map(|statement| statement.sql.as_str())
            .collect::<Vec<_>>();

        assert_eq!(
            sql,
            vec![
                "ALTER TABLE `order_items` DROP FOREIGN KEY `fk_order_items_legacy`;\nALTER TABLE `order_items` ADD CONSTRAINT `fk_order_items_order` FOREIGN KEY (`order_id`) REFERENCES `orders` (`id`);",
            ]
        );
    }

    #[test]
    fn test_mysql_schema_sync_plan_drops_foreign_keys_before_tables() {
        use super::super::{
            DiffStatus, ForeignKeyDiff, ForeignKeySchema, SchemaCompareResult, TableDiff,
            TableSchema,
        };

        let orders = TableSchema {
            name: "orders".to_string(),
            columns: vec![],
            indexes: vec![],
            foreign_keys: vec![],
            comment: None,
            ..Default::default()
        };
        let order_items = TableSchema {
            name: "order_items".to_string(),
            columns: vec![],
            indexes: vec![],
            foreign_keys: vec![],
            comment: None,
            ..Default::default()
        };
        let result = SchemaCompareResult {
            table_diffs: vec![
                TableDiff {
                    name: "orders".to_string(),
                    status: DiffStatus::Removed,
                    source: None,
                    target: Some(orders),
                    column_diffs: vec![],
                    index_diffs: vec![],
                    foreign_key_diffs: vec![],
                    comment_changed: false,
                },
                TableDiff {
                    name: "order_items".to_string(),
                    status: DiffStatus::Modified,
                    source: Some(order_items.clone()),
                    target: Some(order_items),
                    column_diffs: vec![],
                    index_diffs: vec![],
                    foreign_key_diffs: vec![ForeignKeyDiff {
                        name: "fk_order_items_order".to_string(),
                        status: DiffStatus::Removed,
                        source: None,
                        target: Some(ForeignKeySchema {
                            name: "fk_order_items_order".to_string(),
                            columns: vec!["order_id".to_string()],
                            ref_table: "orders".to_string(),
                            ref_columns: vec!["id".to_string()],
                            on_delete: None,
                            on_update: None,
                        }),
                    }],
                    comment_changed: false,
                },
            ],
            added_count: 0,
            removed_count: 1,
            modified_count: 1,
        };
        let plugin = crate::mysql::MySqlPlugin::new();

        let plan = build_schema_sync_plan_with_plugin(&result, "app", None, &plugin);
        let sql = plan
            .statements
            .iter()
            .map(|statement| statement.sql.as_str())
            .collect::<Vec<_>>();

        assert_eq!(
            sql,
            vec![
                "ALTER TABLE `app`.`order_items` DROP FOREIGN KEY `fk_order_items_order`;",
                "DROP TABLE IF EXISTS `app`.`orders`;",
            ]
        );
    }

    #[test]
    fn test_mysql_schema_sync_plan_adds_foreign_keys_after_added_table() {
        use super::super::{
            ColumnSchema, DiffStatus, ForeignKeySchema, IndexSchema, SchemaCompareResult,
            TableDiff, TableSchema,
        };

        let source = TableSchema {
            name: "order_items".to_string(),
            columns: vec![
                ColumnSchema {
                    name: "id".to_string(),
                    data_type: "bigint".to_string(),
                    nullable: false,
                    default_value: None,
                    comment: None,
                    ..Default::default()
                },
                ColumnSchema {
                    name: "order_id".to_string(),
                    data_type: "bigint".to_string(),
                    nullable: false,
                    default_value: None,
                    comment: None,
                    ..Default::default()
                },
            ],
            indexes: vec![IndexSchema {
                name: "PRIMARY".to_string(),
                columns: vec!["id".to_string()],
                unique: true,
            }],
            foreign_keys: vec![ForeignKeySchema {
                name: "fk_order_items_order".to_string(),
                columns: vec!["order_id".to_string()],
                ref_table: "orders".to_string(),
                ref_columns: vec!["id".to_string()],
                on_delete: Some("CASCADE".to_string()),
                on_update: Some("RESTRICT".to_string()),
            }],
            comment: None,
            ..Default::default()
        };
        let result = SchemaCompareResult {
            table_diffs: vec![TableDiff {
                name: "order_items".to_string(),
                status: DiffStatus::Added,
                source: Some(source),
                target: None,
                column_diffs: vec![],
                index_diffs: vec![],
                foreign_key_diffs: vec![],
                comment_changed: false,
            }],
            added_count: 1,
            removed_count: 0,
            modified_count: 0,
        };
        let plugin = crate::mysql::MySqlPlugin::new();

        let plan = build_schema_sync_plan_with_plugin(&result, "app", None, &plugin);
        let sql = plan
            .statements
            .iter()
            .map(|statement| statement.sql.as_str())
            .collect::<Vec<_>>();

        assert_eq!(2, sql.len());
        assert!(sql[0].starts_with("CREATE TABLE `order_items`"));
        assert!(sql[0].contains("PRIMARY KEY (`id`)"));
        assert!(!sql[0].contains("UNIQUE INDEX `PRIMARY`"));
        assert_eq!(
            sql[1],
            "ALTER TABLE `app`.`order_items` ADD CONSTRAINT `fk_order_items_order` FOREIGN KEY (`order_id`) REFERENCES `app`.`orders` (`id`) ON DELETE CASCADE ON UPDATE RESTRICT;"
        );
    }

    #[test]
    fn test_clickhouse_schema_sync_plan_ignores_foreign_keys() {
        use super::super::{
            ColumnSchema, DiffStatus, ForeignKeySchema, SchemaCompareResult, TableDiff, TableSchema,
        };

        let source = TableSchema {
            name: "events".to_string(),
            columns: vec![
                ColumnSchema {
                    name: "id".to_string(),
                    data_type: "UInt64".to_string(),
                    nullable: false,
                    default_value: None,
                    comment: None,
                    ..Default::default()
                },
                ColumnSchema {
                    name: "order_id".to_string(),
                    data_type: "UInt64".to_string(),
                    nullable: false,
                    default_value: None,
                    comment: None,
                    ..Default::default()
                },
            ],
            indexes: vec![],
            foreign_keys: vec![ForeignKeySchema {
                name: "fk_events_order".to_string(),
                columns: vec!["order_id".to_string()],
                ref_table: "orders".to_string(),
                ref_columns: vec!["id".to_string()],
                on_delete: Some("CASCADE".to_string()),
                on_update: None,
            }],
            comment: None,
            ..Default::default()
        };
        let result = SchemaCompareResult {
            table_diffs: vec![TableDiff {
                name: "events".to_string(),
                status: DiffStatus::Added,
                source: Some(source),
                target: None,
                column_diffs: vec![],
                index_diffs: vec![],
                foreign_key_diffs: vec![],
                comment_changed: false,
            }],
            added_count: 1,
            removed_count: 0,
            modified_count: 0,
        };
        let plugin = crate::clickhouse::ClickHousePlugin::new();

        let plan = build_schema_sync_plan_with_plugin(&result, "analytics", None, &plugin);
        let sql = plan
            .statements
            .iter()
            .map(|statement| statement.sql.as_str())
            .collect::<Vec<_>>();

        assert_eq!(1, sql.len());
        assert!(sql[0].contains("CREATE TABLE `events`"));
        assert!(!plan.sql_text.contains("FOREIGN KEY"));
        assert!(!plan.sql_text.contains("fk_events_order"));
    }
}
