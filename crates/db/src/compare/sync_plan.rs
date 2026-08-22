use super::{
    CellValue, ColumnSchema, DataCompareBatchResult, DataCompareBatchWarningKind,
    DataCompareResult, DataCompareTableDependency, DatabaseFamily, DiffStatus, RowData,
    SchemaCompareResult, SchemaObjectType, SchemaTypeMappingContext, TableSchema,
    TypeCompatibility, binary_cell_bytes, column_types_equivalent, database_family,
    map_column_type_with_overrides,
};
use crate::plugin::DatabasePlugin;
use crate::types::{ColumnDefinition, ForeignKeyDefinition, IndexDefinition, TableDesign};
use one_core::storage::DatabaseType;
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

#[derive(Debug, Clone, Default)]
pub struct SchemaSyncPlanOptions {
    pub compare_column_order: bool,
    /// User-defined type mapping overrides for sync plan generation.
    pub type_mapping_overrides: super::TypeMappingOverrides,
}

trait SyncSqlDialect {
    fn quote_identifier(&self, identifier: &str) -> String;
    fn format_table_reference(&self, database: &str, schema: Option<&str>, table: &str) -> String;
    fn drop_table(&self, database: &str, schema: Option<&str>, table: &str) -> String;
    fn build_column_def(&self, column: &ColumnDefinition) -> String;
    fn build_create_table_sql(&self, design: &TableDesign) -> String;

    /// Build CREATE TABLE with the exact qualified reference used by the compare plan.
    ///
    /// `TableDesign` currently carries no schema field and every supported dialect
    /// emits `CREATE TABLE <quoted design.table_name>` as its prefix. Replacing only
    /// that leading token keeps table-designer DDL generation unchanged while
    /// preventing compare plans from creating a table in a different schema than
    /// their subsequent INSERT/ALTER statements target.
    fn build_create_table_sql_with_reference(
        &self,
        design: &TableDesign,
        table_reference: &str,
    ) -> String {
        let sql = self.build_create_table_sql(design);
        let unqualified_prefix =
            format!("CREATE TABLE {}", self.quote_identifier(&design.table_name));
        let qualified_prefix = format!("CREATE TABLE {table_reference}");
        match sql.strip_prefix(&unqualified_prefix) {
            Some(suffix) => format!("{qualified_prefix}{suffix}"),
            None => {
                debug_assert!(
                    false,
                    "CREATE TABLE SQL does not start with the expected table token: {sql}"
                );
                sql
            }
        }
    }

    fn build_alter_table_sql(&self, original: &TableDesign, new: &TableDesign) -> Option<String>;
    fn needs_raw_comment_statements(&self) -> bool;

    /// 按目标数据库类型和列类型格式化 SQL 字面量。
    fn format_literal(&self, value: &CellValue, data_type: Option<&str>) -> String {
        format_value_for_database(value, data_type, None)
    }
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

    fn needs_raw_comment_statements(&self) -> bool {
        false
    }

    fn format_literal(&self, value: &CellValue, data_type: Option<&str>) -> String {
        format_value_for_database(value, data_type, Some(self.name()))
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

    fn needs_raw_comment_statements(&self) -> bool {
        false
    }

    fn format_literal(&self, value: &CellValue, data_type: Option<&str>) -> String {
        format_value_for_database(value, data_type, Some(self.0.name()))
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

    fn needs_raw_comment_statements(&self) -> bool {
        true
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

/// Build one deterministic sync plan for all successfully compared tables.
///
/// This is intentionally kept in the database layer: dependency ordering,
/// statement grouping, destructive defaults, and dialect-specific SQL plans
/// are database concerns. UI callers should only provide the compare result
/// and, for dialect-aware SQL, the target plugin.
pub fn build_data_sync_batch_plan(result: &DataCompareBatchResult) -> SyncPlan {
    build_data_sync_batch_plan_with_plans(
        result,
        result
            .table_results
            .iter()
            .map(build_data_sync_plan)
            .collect(),
    )
}

/// Dialect-aware counterpart of [`build_data_sync_batch_plan`].
pub fn build_data_sync_batch_plan_with_plugin(
    result: &DataCompareBatchResult,
    target_database: &str,
    target_schema: Option<&str>,
    plugin: &dyn DatabasePlugin,
) -> SyncPlan {
    build_data_sync_batch_plan_with_plans(
        result,
        result
            .table_results
            .iter()
            .map(|table_result| {
                build_data_sync_plan_with_plugin(
                    table_result,
                    target_database,
                    target_schema,
                    plugin,
                )
            })
            .collect(),
    )
}

fn build_data_sync_batch_plan_with_plans(
    result: &DataCompareBatchResult,
    plans: Vec<SyncPlan>,
) -> SyncPlan {
    if result.is_sync_sql_blocked() {
        return blocked_data_sync_batch_plan(result);
    }

    let plan_tables = plans
        .iter()
        .map(|plan| plan.target_table.clone())
        .collect::<HashSet<_>>();
    let external_dependency_warnings =
        external_dependency_warnings_by_table(&plan_tables, &result.table_dependencies);
    let target_table = match plans.as_slice() {
        [] => String::new(),
        [plan] => plan.target_table.clone(),
        _ => format!("{} tables", plans.len()),
    };
    let summary = plans.iter().fold(
        SyncPlanSummary {
            insert_count: 0,
            update_count: 0,
            delete_count: 0,
            ddl_count: 0,
            total_count: 0,
        },
        |mut summary, plan| {
            summary.insert_count += plan.summary.insert_count;
            summary.update_count += plan.summary.update_count;
            summary.delete_count += plan.summary.delete_count;
            summary.ddl_count += plan.summary.ddl_count;
            summary.total_count += plan.summary.total_count;
            summary
        },
    );
    let mut warnings = plans
        .iter()
        .flat_map(|plan| plan.warnings.iter().cloned())
        .collect::<Vec<_>>();
    warnings.extend(
        external_dependency_warnings
            .values()
            .flat_map(|warnings| warnings.iter().cloned()),
    );
    warnings.extend(data_compare_batch_failure_warnings(result));
    warnings.extend(data_compare_batch_warnings(result));

    let (mut statements, dependency_cycle) =
        ordered_sync_statements(plans, &result.table_dependencies);
    if let Some(cycle) = dependency_cycle {
        let mut blocked = blocked_data_sync_batch_plan(result);
        blocked.warnings.push(format!(
            "Foreign-key dependency cycle detected among tables ({}); sync SQL generation is disabled because no safe execution order exists.",
            cycle.join(" → ")
        ));
        return blocked;
    }
    apply_external_dependency_warnings(&mut statements, &external_dependency_warnings);
    let sql_text = statements
        .iter()
        .map(|statement| statement.sql.as_str())
        .collect::<Vec<_>>()
        .join("\n");

    SyncPlan {
        id: uuid::Uuid::new_v4().to_string(),
        target_table,
        statements,
        summary,
        warnings,
        sql_text,
    }
}

fn blocked_data_sync_batch_plan(result: &DataCompareBatchResult) -> SyncPlan {
    let target_table = match result.table_results.as_slice() {
        [] => String::new(),
        [table_result] => table_result.target_table.clone(),
        _ => format!("{} tables", result.table_results.len()),
    };
    let mut warnings = Vec::new();
    if result.has_truncated_tables() {
        warnings
            .push("Data compare result is truncated; sync SQL generation is disabled.".to_string());
    }
    if result.has_incomplete_dependency_metadata() {
        warnings.push(
            "Dependency metadata is incomplete; sync SQL generation is disabled because statement ordering cannot be guaranteed."
                .to_string(),
        );
    }
    if result.has_inconsistent_snapshot_risk() {
        warnings.push(
            "A consistent read snapshot was unavailable; sync SQL generation is disabled because paged results may represent different database states."
                .to_string(),
        );
    }
    warnings.extend(data_compare_batch_failure_warnings(result));
    warnings.extend(data_compare_batch_warnings(result));
    SyncPlan {
        id: uuid::Uuid::new_v4().to_string(),
        target_table,
        statements: vec![],
        summary: empty_sync_plan_summary(),
        warnings,
        sql_text: String::new(),
    }
}

fn data_compare_batch_failure_warnings(result: &DataCompareBatchResult) -> Vec<String> {
    result
        .table_failures
        .iter()
        .map(data_compare_table_failure_warning)
        .collect()
}

pub fn data_compare_table_failure_warning(failure: &super::DataCompareTableFailure) -> String {
    format!(
        "Table `{}` failed to compare and was excluded from the sync plan: {}",
        failure.table, failure.error
    )
}

fn data_compare_batch_warnings(result: &DataCompareBatchResult) -> Vec<String> {
    result
        .batch_warnings
        .iter()
        .map(|warning| match warning.kind {
            DataCompareBatchWarningKind::TargetTableMetadataUnavailable => format!(
                "Target table metadata could not be loaded; dependency ordering is incomplete: {}",
                warning.error
            ),
            DataCompareBatchWarningKind::ForeignKeyMetadataUnavailable => format!(
                "Foreign key metadata for table `{}` could not be loaded; dependency ordering is incomplete: {}",
                warning.table.as_deref().unwrap_or("<unknown>"),
                warning.error
            ),
            DataCompareBatchWarningKind::ConsistentSnapshotUnavailable => format!(
                "Consistent read snapshot unavailable; comparison is best-effort and cannot be used to generate sync SQL: {}",
                warning.error
            ),
        })
        .collect()
}

fn empty_sync_plan_summary() -> SyncPlanSummary {
    SyncPlanSummary {
        insert_count: 0,
        update_count: 0,
        delete_count: 0,
        ddl_count: 0,
        total_count: 0,
    }
}

fn external_dependency_warnings_by_table(
    plan_tables: &HashSet<String>,
    dependencies: &[DataCompareTableDependency],
) -> HashMap<String, Vec<String>> {
    let mut warnings_by_table = HashMap::new();
    let mut seen = HashSet::new();
    for dependency in dependencies {
        let child_in_plan = plan_tables.contains(&dependency.table);
        let parent_in_plan = plan_tables.contains(&dependency.referenced_table);
        if !child_in_plan || parent_in_plan {
            continue;
        }
        let warning = format!(
            "Table `{}` has a foreign key to `{}`, but `{}` is not included in this data compare. Insert/update SQL is skipped by default to avoid foreign key failures.",
            dependency.table, dependency.referenced_table, dependency.referenced_table
        );
        if seen.insert((dependency.table.clone(), warning.clone())) {
            warnings_by_table
                .entry(dependency.table.clone())
                .or_insert_with(Vec::new)
                .push(warning);
        }
    }
    warnings_by_table
}

fn apply_external_dependency_warnings(
    statements: &mut [SyncStatement],
    warnings_by_table: &HashMap<String, Vec<String>>,
) {
    for statement in statements {
        let Some(table) = statement.object_name.as_ref() else {
            continue;
        };
        let Some(warnings) = warnings_by_table.get(table) else {
            continue;
        };
        if matches!(
            statement.kind,
            SyncStatementKind::Insert | SyncStatementKind::Update
        ) {
            statement.selected_by_default = false;
            statement.warnings.extend(warnings.iter().cloned());
        }
    }
}

fn ordered_sync_statements(
    plans: Vec<SyncPlan>,
    dependencies: &[DataCompareTableDependency],
) -> (Vec<SyncStatement>, Option<Vec<String>>) {
    let tables = plans
        .iter()
        .map(|plan| plan.target_table.clone())
        .collect::<Vec<_>>();
    let table_set = tables.iter().cloned().collect::<HashSet<_>>();
    let mut indegree: HashMap<String, usize> =
        tables.iter().map(|table| (table.clone(), 0)).collect();
    let mut children_by_parent: HashMap<String, Vec<String>> = HashMap::new();
    let mut seen_edges = HashSet::new();
    for dependency in dependencies {
        if !table_set.contains(&dependency.table)
            || !table_set.contains(&dependency.referenced_table)
            || !seen_edges.insert((
                dependency.table.clone(),
                dependency.referenced_table.clone(),
            ))
        {
            continue;
        }
        children_by_parent
            .entry(dependency.referenced_table.clone())
            .or_default()
            .push(dependency.table.clone());
        *indegree.entry(dependency.table.clone()).or_insert(0) += 1;
    }
    let mut ready = tables
        .iter()
        .filter(|table| indegree.get(*table).copied().unwrap_or(0) == 0)
        .cloned()
        .collect::<Vec<_>>();
    let mut table_order = Vec::with_capacity(tables.len());
    while let Some(table) = ready.first().cloned() {
        ready.remove(0);
        table_order.push(table.clone());
        if let Some(children) = children_by_parent.get(&table) {
            for child in children {
                let Some(value) = indegree.get_mut(child) else {
                    continue;
                };
                *value = value.saturating_sub(1);
                if *value == 0 {
                    ready.push(child.clone());
                }
            }
        }
    }
    let ordered_set = table_order.iter().cloned().collect::<HashSet<_>>();
    if ordered_set.len() != tables.len() {
        let mut cycle = tables
            .iter()
            .filter(|table| !ordered_set.contains(*table))
            .cloned()
            .collect::<Vec<_>>();
        cycle.sort();
        return (Vec::new(), Some(cycle));
    }
    let table_rank = table_order
        .iter()
        .enumerate()
        .map(|(index, table)| (table.clone(), index))
        .collect::<HashMap<_, _>>();
    let fallback_rank = table_order.len();
    let mut statements = plans
        .into_iter()
        .flat_map(|plan| plan.statements.into_iter())
        .enumerate()
        .collect::<Vec<_>>();
    statements.sort_by(|(left_index, left), (right_index, right)| {
        sync_statement_sort_key(left, &table_rank, fallback_rank, *left_index).cmp(
            &sync_statement_sort_key(right, &table_rank, fallback_rank, *right_index),
        )
    });
    (
        statements
            .into_iter()
            .map(|(_, statement)| statement)
            .collect(),
        None,
    )
}

fn sync_statement_sort_key(
    statement: &SyncStatement,
    table_rank: &HashMap<String, usize>,
    fallback_rank: usize,
    original_index: usize,
) -> (usize, usize, usize) {
    let group = match &statement.kind {
        SyncStatementKind::CreateTable => 0,
        SyncStatementKind::Insert => 1,
        SyncStatementKind::Update => 2,
        SyncStatementKind::Delete => 3,
        _ => 4,
    };
    let rank = statement
        .object_name
        .as_ref()
        .and_then(|table| table_rank.get(table))
        .copied()
        .unwrap_or(fallback_rank);
    let rank = if group == 3 && rank < fallback_rank {
        fallback_rank - rank
    } else {
        rank
    };
    (group, rank, original_index)
}

fn build_data_sync_plan_with_dialect(
    result: &DataCompareResult,
    target_database: &str,
    target_schema: Option<&str>,
    dialect: &dyn SyncSqlDialect,
) -> SyncPlan {
    let mut statements = Vec::new();
    let mut warnings = Vec::new();
    let mut ddl_count = 0usize;
    let plan_id = uuid::Uuid::new_v4().to_string();
    let target_table_ref = compare_table_reference(
        dialect,
        target_database,
        target_schema,
        &result.target_table,
    );

    // 目标表不存在时，在 INSERT 前生成 CREATE TABLE（对齐 dbx 的 pre_sync_statements）
    if let Some(schema) = result.missing_target_schema.as_ref() {
        let sql =
            missing_target_create_table_sql(target_database, schema, &target_table_ref, dialect);
        statements.push(SyncStatement {
            id: uuid::Uuid::new_v4().to_string(),
            sql,
            kind: SyncStatementKind::CreateTable,
            object_name: Some(result.target_table.clone()),
            row_key: None,
            destructive: false,
            transactional_safe: true,
            selected_by_default: true,
            warnings: vec![],
        });
        ddl_count += 1;
        warnings.push(format!(
            "目标表 `{}` 不存在，已按目标方言在同步计划开头生成 CREATE TABLE 语句",
            result.target_table
        ));
    } else if result.target_table_missing {
        warnings.push(format!(
            "目标表 `{}` 不存在，但缺少源表结构信息，无法生成 CREATE TABLE 语句",
            result.target_table
        ));
    }

    // 生成 INSERT 语句（新增行）
    for row in &result.added {
        let stmt_id = uuid::Uuid::new_v4().to_string();
        let sql = generate_insert_sql(
            &target_table_ref,
            row,
            &result.columns,
            &result.column_types,
            dialect,
        );

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
            &result.columns,
            &result.column_types,
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
        let sql = generate_delete_sql(
            &target_table_ref,
            &key_values,
            &result.column_types,
            dialect,
        );

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
    let total_count = insert_count + update_count + delete_count + ddl_count;

    let summary = SyncPlanSummary {
        insert_count,
        update_count,
        delete_count,
        ddl_count,
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
        warnings,
        sql_text,
    }
}

/// 生成缺失目标表的建表语句：只带列和主键，索引/外键由结构同步单独处理。
fn missing_target_create_table_sql(
    target_database: &str,
    schema: &TableSchema,
    target_table_ref: &str,
    dialect: &dyn SyncSqlDialect,
) -> String {
    let mut design = table_schema_to_design(target_database, schema);
    design.indexes.clear();
    design.foreign_keys.clear();
    dialect.build_create_table_sql_with_reference(&design, target_table_ref)
}

fn compare_table_reference(
    dialect: &dyn SyncSqlDialect,
    database: &str,
    schema: Option<&str>,
    table: &str,
) -> String {
    let schema = schema.map(str::trim).filter(|schema| !schema.is_empty());
    if database.trim().is_empty() {
        return schema.map_or_else(
            || dialect.quote_identifier(table),
            |schema| {
                format!(
                    "{}.{}",
                    dialect.quote_identifier(schema),
                    dialect.quote_identifier(table)
                )
            },
        );
    }
    dialect.format_table_reference(database, schema, table)
}

/// 生成 INSERT SQL
fn generate_insert_sql(
    table: &str,
    row: &RowData,
    columns: &[String],
    column_types: &HashMap<String, String>,
    dialect: &dyn SyncSqlDialect,
) -> String {
    let cols = columns
        .iter()
        .map(|column| dialect.quote_identifier(column))
        .collect::<Vec<_>>()
        .join(", ");
    let values = columns
        .iter()
        .map(|col| {
            let value = row.get(col).cloned().unwrap_or(CellValue::Null);
            dialect.format_literal(&value, column_type_for(col, column_types))
        })
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
    column_order: &[String],
    column_types: &HashMap<String, String>,
    dialect: &dyn SyncSqlDialect,
) -> String {
    let set_clause = ordered_change_columns(changes, column_order)
        .into_iter()
        .map(|col| {
            let value = source_values.get(col).cloned().unwrap_or(CellValue::Null);
            format!(
                "{} = {}",
                dialect.quote_identifier(col),
                dialect.format_literal(&value, column_type_for(col, column_types))
            )
        })
        .collect::<Vec<_>>()
        .join(", ");

    let where_clause = sorted_key_values(key_values)
        .into_iter()
        .map(|(col, val)| format_where_condition(col, val, column_types, dialect))
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
    column_types: &HashMap<String, String>,
    dialect: &dyn SyncSqlDialect,
) -> String {
    let where_clause = sorted_key_values(key_values)
        .into_iter()
        .map(|(col, val)| format_where_condition(col, val, column_types, dialect))
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
    target_db_type: &DatabaseType,
    table_name: &str,
    index_name: &str,
    dialect: &dyn SyncSqlDialect,
) -> String {
    let index_name = dialect.quote_identifier(index_name);
    if is_mysql_family(target_db_type) {
        format!("ALTER TABLE {} DROP INDEX {};", table_name, index_name)
    } else if is_sqlserver_family(target_db_type) {
        format!("DROP INDEX {} ON {};", index_name, table_name)
    } else if is_oracle_family(target_db_type) {
        format!("DROP INDEX {};", index_name)
    } else if is_clickhouse_family(target_db_type) {
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
    target_db_type: &DatabaseType,
    table_name: &str,
    primary_key_name: &str,
    dialect: &dyn SyncSqlDialect,
) -> String {
    if is_mysql_family(target_db_type) {
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
    source_database: Option<&str>,
    source_schema: Option<&str>,
    dialect: &dyn SyncSqlDialect,
) -> String {
    let columns = foreign_key
        .columns
        .iter()
        .map(|column| dialect.quote_identifier(column))
        .collect::<Vec<_>>()
        .join(", ");
    let referenced_schema = map_foreign_key_reference_schema(
        foreign_key.ref_schema.as_deref(),
        source_database,
        source_schema,
        target_schema,
    );
    let ref_table =
        dialect.format_table_reference(target_database, referenced_schema, &foreign_key.ref_table);
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

fn map_foreign_key_reference_schema<'a>(
    referenced_schema: Option<&'a str>,
    source_database: Option<&str>,
    source_schema: Option<&str>,
    target_schema: Option<&'a str>,
) -> Option<&'a str> {
    let Some(referenced_schema) = referenced_schema else {
        return target_schema;
    };

    let is_source_namespace = source_database
        .is_some_and(|source| referenced_schema.eq_ignore_ascii_case(source))
        || source_schema.is_some_and(|source| referenced_schema.eq_ignore_ascii_case(source));
    if is_source_namespace {
        target_schema
    } else {
        Some(referenced_schema)
    }
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

type SyncDatabaseKind = DatabaseFamily;

fn sync_database_kind(database_type: &DatabaseType) -> SyncDatabaseKind {
    database_family(database_type)
}

fn is_mysql_family(database_type: &DatabaseType) -> bool {
    matches!(sync_database_kind(database_type), SyncDatabaseKind::MySql)
}

fn is_postgres_family(database_type: &DatabaseType) -> bool {
    matches!(
        sync_database_kind(database_type),
        SyncDatabaseKind::PostgreSql
    )
}

fn is_sqlserver_family(database_type: &DatabaseType) -> bool {
    matches!(
        sync_database_kind(database_type),
        SyncDatabaseKind::SqlServer
    )
}

fn is_oracle_family(database_type: &DatabaseType) -> bool {
    matches!(sync_database_kind(database_type), SyncDatabaseKind::Oracle)
}

fn is_clickhouse_family(database_type: &DatabaseType) -> bool {
    matches!(
        sync_database_kind(database_type),
        SyncDatabaseKind::ClickHouse
    )
}

fn is_sqlite_family(database_type: &DatabaseType) -> bool {
    matches!(sync_database_kind(database_type), SyncDatabaseKind::Sqlite)
}

fn sync_supports_foreign_keys(target_db_type: &DatabaseType) -> bool {
    !is_clickhouse_family(target_db_type)
}

fn generate_drop_foreign_key_sql(
    target_db_type: &DatabaseType,
    table_name: &str,
    foreign_key_name: &str,
    dialect: &dyn SyncSqlDialect,
) -> String {
    let foreign_key_name = dialect.quote_identifier(foreign_key_name);
    if is_mysql_family(target_db_type) {
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

fn escaped_comment(comment: &str) -> String {
    comment.replace('\'', "''")
}

fn comment_literal(target_db_type: &DatabaseType, comment: Option<&str>) -> String {
    let comment = comment.unwrap_or_default();
    if comment.is_empty() && is_postgres_family(target_db_type) {
        "NULL".to_string()
    } else {
        format!("'{}'", escaped_comment(comment))
    }
}

fn raw_table_comment_sql(
    target_db_type: &DatabaseType,
    table_ref: &str,
    table_name: &str,
    original_comment: Option<&str>,
    new_comment: Option<&str>,
) -> Option<String> {
    if original_comment.unwrap_or_default() == new_comment.unwrap_or_default() {
        return None;
    }
    let new_comment = new_comment.unwrap_or_default();
    if is_mysql_family(target_db_type) {
        Some(format!(
            "ALTER TABLE {} COMMENT='{}';",
            table_ref,
            escaped_comment(new_comment)
        ))
    } else if is_sqlserver_family(target_db_type) {
        Some(mssql_comment_property_sql(
            &["SCHEMA", "dbo", "TABLE", table_name],
            original_comment,
            new_comment,
        ))
    } else {
        Some(format!(
            "COMMENT ON TABLE {} IS {};",
            table_ref,
            comment_literal(target_db_type, Some(new_comment))
        ))
    }
}

fn raw_column_comment_sql(
    target_db_type: &DatabaseType,
    table_ref: &str,
    table_name: &str,
    source: &ColumnSchema,
    target: Option<&ColumnSchema>,
    source_definition: Option<&ColumnDefinition>,
    dialect: &dyn SyncSqlDialect,
) -> Option<String> {
    let original_comment = target.and_then(|column| column.comment.as_deref());
    let new_comment = source.comment.as_deref();
    if original_comment.unwrap_or_default() == new_comment.unwrap_or_default() {
        return None;
    }
    let new_comment = new_comment.unwrap_or_default();
    if is_mysql_family(target_db_type) {
        let mut column = source_definition
            .cloned()
            .unwrap_or_else(|| column_schema_to_definition(source));
        column.comment = new_comment.to_string();
        let mut definition = dialect.build_column_def(&column);
        if !new_comment.is_empty() {
            definition.push_str(&format!(" COMMENT '{}'", escaped_comment(new_comment)));
        }
        Some(format!(
            "ALTER TABLE {} MODIFY COLUMN {};",
            table_ref, definition
        ))
    } else if is_sqlserver_family(target_db_type) {
        Some(mssql_comment_property_sql(
            &["SCHEMA", "dbo", "TABLE", table_name, "COLUMN", &source.name],
            original_comment,
            new_comment,
        ))
    } else {
        Some(format!(
            "COMMENT ON COLUMN {}.{} IS {};",
            table_ref,
            dialect.quote_identifier(&source.name),
            comment_literal(target_db_type, Some(new_comment))
        ))
    }
}

fn mssql_comment_property_sql(
    levels: &[&str],
    original: Option<&str>,
    new_comment: &str,
) -> String {
    let operation = if new_comment.is_empty() {
        "drop"
    } else if original.unwrap_or_default().is_empty() {
        "add"
    } else {
        "update"
    };
    let mut sql = format!("EXEC sp_{operation}extendedproperty @name=N'MS_Description'");
    if !new_comment.is_empty() {
        sql.push_str(&format!(", @value=N'{}'", escaped_comment(new_comment)));
    }
    for (idx, pair) in levels.chunks(2).enumerate() {
        if let [level_type, level_name] = pair {
            sql.push_str(&format!(
                ", @level{idx}type=N'{}', @level{idx}name=N'{}'",
                escaped_comment(level_type),
                escaped_comment(level_name)
            ));
        }
    }
    sql.push(';');
    sql
}

fn raw_column_definition_changed(
    source: &ColumnSchema,
    target: &ColumnSchema,
    source_db_type: &DatabaseType,
    target_db_type: &DatabaseType,
    overrides: Option<&super::TypeMappingOverrides>,
) -> bool {
    !column_types_equivalent(
        &source.data_type,
        &target.data_type,
        match overrides {
            Some(ov) => {
                SchemaTypeMappingContext::with_overrides(source_db_type, target_db_type, ov)
            }
            None => SchemaTypeMappingContext::new(source_db_type, target_db_type),
        },
    ) || source.nullable != target.nullable
        || source.default_value != target.default_value
        || source.charset != target.charset
        || source.collation != target.collation
}

struct TableDesignerSyncOutcome {
    statement: Option<SyncStatement>,
    warnings: Vec<String>,
}

fn table_designer_sync_statement(
    table_diff: &super::TableDiff,
    target_database: &str,
    source_db_type: &DatabaseType,
    target_db_type: &DatabaseType,
    dialect: &dyn SyncSqlDialect,
    compare_column_order: bool,
    overrides: Option<&super::TypeMappingOverrides>,
) -> TableDesignerSyncOutcome {
    let Some(source) = table_diff.source.as_ref() else {
        return TableDesignerSyncOutcome {
            statement: None,
            warnings: vec![],
        };
    };
    let Some(target) = table_diff.target.as_ref() else {
        return TableDesignerSyncOutcome {
            statement: None,
            warnings: vec![],
        };
    };
    let original = table_schema_to_design(target_database, target);
    let mapped = match mapped_table_schema_to_design(
        target_database,
        source,
        Some(target),
        source_db_type,
        target_db_type,
        overrides,
    ) {
        Ok(mapped) => mapped,
        Err(warnings) => {
            return TableDesignerSyncOutcome {
                statement: None,
                warnings,
            };
        }
    };
    let mut new = mapped.design;
    if !compare_column_order {
        new.columns = compare_sync_columns_ignoring_order(new.columns, target);
    }
    let Some(sql) = dialect.build_alter_table_sql(&original, &new) else {
        return TableDesignerSyncOutcome {
            statement: None,
            warnings: vec![],
        };
    };
    if !has_executable_sync_sql(&sql) {
        return TableDesignerSyncOutcome {
            statement: None,
            warnings: vec![],
        };
    }
    let safety =
        table_designer_statement_safety(table_diff, source_db_type, target_db_type, overrides);
    let mut statement_warnings = safety.warnings;
    statement_warnings.extend(mapped.warnings);
    TableDesignerSyncOutcome {
        statement: Some(SyncStatement {
            id: uuid::Uuid::new_v4().to_string(),
            sql,
            kind: SyncStatementKind::AlterTable,
            object_name: Some(table_diff.name.clone()),
            row_key: None,
            destructive: safety.destructive,
            transactional_safe: true,
            selected_by_default: safety.selected_by_default && mapped.selected_by_default,
            warnings: statement_warnings,
        }),
        warnings: vec![],
    }
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
    source_db_type: &DatabaseType,
    target_db_type: &DatabaseType,
    overrides: Option<&super::TypeMappingOverrides>,
) -> SyncStatementSafety {
    let destructive_column_diff = table_diff
        .column_diffs
        .iter()
        .any(|diff| match diff.status {
            DiffStatus::Removed => true,
            DiffStatus::Added => false,
            DiffStatus::Modified => match (&diff.source, &diff.target) {
                (Some(source), Some(target)) => raw_column_definition_changed(
                    source,
                    target,
                    source_db_type,
                    target_db_type,
                    overrides,
                ),
                _ => true,
            },
        });
    let destructive = destructive_column_diff
        || table_diff.index_diffs.iter().any(|diff| {
            matches!(diff.status, DiffStatus::Removed | DiffStatus::Modified)
                && diff.target.as_ref().is_some_and(is_primary_index)
        });
    let rebuilds_sqlite_table = destructive && is_sqlite_family(target_db_type);
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

/// 格式化值为 SQL 字面量（无类型信息，兼容旧测试与 Raw 方言）。
#[cfg(test)]
fn format_value(value: CellValue) -> String {
    format_value_for_database(&value, None, None)
}

/// 按目标数据库和列类型格式化 SQL 字面量。
///
/// 与 dbx `format_grid_sql_literal` 对齐的核心路径：
/// - MySQL BIT 列输出裸位值或 `b'...'`
/// - 数组输出 PostgreSQL `'{...}'` / ClickHouse `[...]`
/// - MySQL 时间列把 RFC3339 归一化为 `YYYY-MM-DD HH:MM:SS[.ffffff]`
/// - SQL Server 字符串使用 `N'...'`
pub(super) fn format_value_for_database(
    value: &CellValue,
    data_type: Option<&str>,
    database_type: Option<DatabaseType>,
) -> String {
    if value.is_null() {
        return "NULL".to_string();
    }
    if let Some(bytes) = binary_cell_bytes(value) {
        return format_binary_literal_for_database(&bytes, database_type.as_ref());
    }
    if is_mysql_bit_literal_column(database_type.as_ref(), data_type) {
        if let Some(value) = value.as_bool() {
            return if value { "1" } else { "0" }.to_string();
        }
        if let Some(number) = value.as_number() {
            return number.to_string();
        }
        if let Some(text) = value.as_str().and_then(format_mysql_bit_literal_text) {
            return text;
        }
    }
    if let Some(value) = value.as_bool() {
        return if value { "TRUE" } else { "FALSE" }.to_string();
    }
    if let Some(number) = value.as_number() {
        return number.to_string();
    }
    if let Some(array) = value.as_array() {
        if database_type.as_ref().is_some_and(is_clickhouse_family) {
            return format_ch_array_sql_literal(array);
        }
        return format_pg_array_sql_literal(array);
    }
    let text = value
        .as_str()
        .map_or_else(|| value.to_string(), ToString::to_string);
    if text.is_empty() {
        return if database_type.as_ref().is_some_and(is_sqlserver_family) {
            "N''".to_string()
        } else {
            "''".to_string()
        };
    }
    let literal_text = if is_mysql_datetime_literal_database(database_type.as_ref())
        && data_type.map(is_temporal_column_type).unwrap_or(true)
    {
        format_mysql_temporal_literal_text(&text, data_type)
    } else {
        text
    };
    let escaped_text = literal_text.replace('\'', "''");
    let escaped_text = if database_type.as_ref().is_some_and(|database_type| {
        is_mysql_family(database_type) || is_clickhouse_family(database_type)
    }) {
        escaped_text.replace('\\', "\\\\")
    } else {
        escaped_text
    };
    let escaped = format!("'{escaped_text}'");
    if database_type.as_ref().is_some_and(is_sqlserver_family) {
        format!("N{escaped}")
    } else {
        escaped
    }
}

fn format_binary_literal_for_database(
    bytes: &[u8],
    database_type: Option<&DatabaseType>,
) -> String {
    let hex = encode_hex(bytes);
    if database_type.is_some_and(is_postgres_family) {
        format!("decode('{hex}', 'hex')")
    } else if database_type.is_some_and(is_sqlserver_family) {
        format!("0x{hex}")
    } else if database_type.is_some_and(is_oracle_family) {
        format!("hextoraw('{hex}')")
    } else if database_type.is_some_and(is_clickhouse_family) {
        format!("unhex('{hex}')")
    } else {
        // MySQL, SQLite, DuckDB, unknown external drivers, and the
        // type-less compatibility path all accept the SQL hex literal form.
        format!("X'{hex}'")
    }
}

fn encode_hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn is_mysql_bit_literal_column(
    database_type: Option<&DatabaseType>,
    data_type: Option<&str>,
) -> bool {
    is_mysql_datetime_literal_database(database_type) && data_type.is_some_and(is_bit_column_type)
}

fn is_bit_column_type(data_type: &str) -> bool {
    data_type
        .to_ascii_lowercase()
        .split(|ch: char| !ch.is_ascii_alphanumeric())
        .any(|token| token == "bit")
}

fn format_mysql_bit_literal_text(text: &str) -> Option<String> {
    let trimmed = text.trim();
    if trimmed.eq_ignore_ascii_case("true") {
        return Some("1".to_string());
    }
    if trimmed.eq_ignore_ascii_case("false") {
        return Some("0".to_string());
    }
    if !trimmed.is_empty() && trimmed.chars().all(|ch| ch.is_ascii_digit()) {
        return Some(if trimmed.len() == 1 {
            trimmed.to_string()
        } else if trimmed.chars().all(|ch| matches!(ch, '0' | '1')) {
            format!("b'{trimmed}'")
        } else {
            trimmed.to_string()
        });
    }
    let lower = trimmed.to_ascii_lowercase();
    if lower.starts_with("b'") && trimmed.ends_with('\'') {
        let bits = &trimmed[2..trimmed.len() - 1];
        if !bits.is_empty() && bits.chars().all(|ch| matches!(ch, '0' | '1')) {
            return Some(format!("b'{bits}'"));
        }
    }
    None
}

fn is_mysql_datetime_literal_database(database_type: Option<&DatabaseType>) -> bool {
    database_type.is_some_and(is_mysql_family)
}

fn is_temporal_column_type(data_type: &str) -> bool {
    temporal_column_kind(Some(data_type)).is_some()
}

fn temporal_column_kind(data_type: Option<&str>) -> Option<&'static str> {
    let normalized = data_type.unwrap_or("").trim().to_ascii_lowercase();
    let base = normalized
        .split(|ch: char| !ch.is_ascii_alphanumeric())
        .next()
        .unwrap_or("");
    match base {
        "date" => Some("date"),
        "time" => Some("time"),
        "datetime" | "timestamp" => Some("datetime"),
        _ => None,
    }
}

fn format_mysql_temporal_literal_text(text: &str, data_type: Option<&str>) -> String {
    let Some(parts) = parse_rfc3339_like_timestamp(text) else {
        return text.to_string();
    };
    match temporal_column_kind(data_type) {
        Some("date") => parts.date,
        Some("time") => format!(
            "{}{}",
            parts.time,
            normalize_mysql_fractional_seconds(parts.fraction.as_deref())
        ),
        _ => format!(
            "{} {}{}",
            parts.date,
            parts.time,
            normalize_mysql_fractional_seconds(parts.fraction.as_deref())
        ),
    }
}

struct Rfc3339TimestampParts {
    date: String,
    time: String,
    fraction: Option<String>,
}

fn parse_rfc3339_like_timestamp(text: &str) -> Option<Rfc3339TimestampParts> {
    let bytes = text.as_bytes();
    if bytes.len() < 20 || bytes.get(4) != Some(&b'-') || bytes.get(7) != Some(&b'-') {
        return None;
    }
    let separator = *bytes.get(10)?;
    if separator != b'T' && separator != b' ' {
        return None;
    }
    if bytes.get(13) != Some(&b':') || bytes.get(16) != Some(&b':') {
        return None;
    }
    let date = text.get(0..10)?.to_string();
    let time = text.get(11..19)?.to_string();
    let rest = text.get(19..)?;
    let (fraction, zone) = if let Some(rest) = rest.strip_prefix('.') {
        let digit_count = rest.chars().take_while(|ch| ch.is_ascii_digit()).count();
        if digit_count == 0 || digit_count > 9 {
            return None;
        }
        (
            Some(format!(".{}", &rest[..digit_count])),
            rest.get(digit_count..)?,
        )
    } else {
        (None, rest)
    };
    if zone == "Z" || zone == "z" || is_timezone_offset(zone) {
        Some(Rfc3339TimestampParts {
            date,
            time,
            fraction,
        })
    } else {
        None
    }
}

fn is_timezone_offset(value: &str) -> bool {
    let bytes = value.as_bytes();
    bytes.len() == 6
        && matches!(bytes[0], b'+' | b'-')
        && bytes[3] == b':'
        && bytes[1].is_ascii_digit()
        && bytes[2].is_ascii_digit()
        && bytes[4].is_ascii_digit()
        && bytes[5].is_ascii_digit()
}

fn normalize_mysql_fractional_seconds(fraction: Option<&str>) -> String {
    match fraction {
        Some(fraction) if fraction.len() > 7 => fraction[..7].to_string(),
        Some(fraction) => fraction.to_string(),
        None => String::new(),
    }
}

fn format_pg_array_sql_literal(array: &[CellValue]) -> String {
    if array.is_empty() {
        return "'{}'".to_string();
    }
    let elements = array
        .iter()
        .map(format_pg_array_element)
        .collect::<Vec<_>>();
    let inner = format!("{{{}}}", elements.join(","));
    format!("'{}'", inner.replace('\\', "\\\\").replace('\'', "''"))
}

fn format_pg_array_element(value: &CellValue) -> String {
    match value {
        CellValue::Null => "NULL".to_string(),
        CellValue::Object(_) if binary_cell_bytes(value).is_some() => {
            let bytes = binary_cell_bytes(value).expect("binary tag checked above");
            let hex = encode_hex(&bytes);
            format!(r"\x{hex}")
        }
        CellValue::Array(array) => {
            if array.is_empty() {
                return "{}".to_string();
            }
            let elements = array
                .iter()
                .map(format_pg_array_element)
                .collect::<Vec<_>>();
            format!("{{{}}}", elements.join(","))
        }
        CellValue::String(text) => {
            format!("\"{}\"", text.replace('\\', "\\\\").replace('"', "\\\""))
        }
        CellValue::Number(number) => number.to_string(),
        CellValue::Bool(value) => {
            if *value {
                "true".to_string()
            } else {
                "false".to_string()
            }
        }
        CellValue::Object(object) => {
            let json = serde_json::to_string(object).unwrap_or_default();
            format!("\"{}\"", json.replace('\\', "\\\\").replace('"', "\\\""))
        }
    }
}

fn format_ch_array_sql_literal(array: &[CellValue]) -> String {
    if array.is_empty() {
        return "[]".to_string();
    }
    let elements = array
        .iter()
        .map(format_ch_array_element)
        .collect::<Vec<_>>();
    format!("[{}]", elements.join(","))
}

fn format_ch_array_element(value: &CellValue) -> String {
    match value {
        CellValue::Null => "NULL".to_string(),
        CellValue::Object(_) if binary_cell_bytes(value).is_some() => {
            let bytes = binary_cell_bytes(value).expect("binary tag checked above");
            let hex = encode_hex(&bytes);
            format!("unhex('{hex}')")
        }
        CellValue::Array(array) => {
            if array.is_empty() {
                return "[]".to_string();
            }
            let elements = array
                .iter()
                .map(format_ch_array_element)
                .collect::<Vec<_>>();
            format!("[{}]", elements.join(","))
        }
        CellValue::String(text) => {
            format!("'{}'", text.replace('\\', "\\\\").replace('\'', "''"))
        }
        CellValue::Number(number) => number.to_string(),
        CellValue::Bool(value) => {
            if *value {
                "true".to_string()
            } else {
                "false".to_string()
            }
        }
        CellValue::Object(object) => {
            let json = serde_json::to_string(object).unwrap_or_default();
            format!("'{}'", json.replace('\\', "\\\\").replace('\'', "''"))
        }
    }
}

fn column_type_for<'a>(column: &str, column_types: &'a HashMap<String, String>) -> Option<&'a str> {
    if let Some(data_type) = column_types.get(column) {
        return Some(data_type);
    }
    column_types
        .iter()
        .find(|(name, _)| name.eq_ignore_ascii_case(column))
        .map(|(_, data_type)| data_type.as_str())
}

fn ordered_change_columns<'a>(
    changes: &'a HashMap<String, (CellValue, CellValue)>,
    column_order: &[String],
) -> Vec<&'a String> {
    let order = column_order
        .iter()
        .enumerate()
        .map(|(index, column)| (column.to_ascii_lowercase(), index))
        .collect::<HashMap<_, _>>();
    let mut columns = changes.keys().collect::<Vec<_>>();
    columns.sort_by(|left, right| {
        let left_order = order.get(&left.to_ascii_lowercase());
        let right_order = order.get(&right.to_ascii_lowercase());
        match (left_order, right_order) {
            (Some(left), Some(right)) => left.cmp(right),
            (Some(_), None) => std::cmp::Ordering::Less,
            (None, Some(_)) => std::cmp::Ordering::Greater,
            (None, None) => left.cmp(right),
        }
    });
    columns
}

fn sorted_key_values(
    key_values: &std::collections::HashMap<String, CellValue>,
) -> Vec<(&String, &CellValue)> {
    let mut values = key_values.iter().collect::<Vec<_>>();
    values.sort_by(|left, right| left.0.cmp(right.0));
    values
}

fn format_where_condition(
    column: &str,
    value: &CellValue,
    column_types: &HashMap<String, String>,
    dialect: &dyn SyncSqlDialect,
) -> String {
    let quoted_column = dialect.quote_identifier(column);
    if value.is_null() {
        format!("{quoted_column} IS NULL")
    } else {
        format!(
            "{} = {}",
            quoted_column,
            dialect.format_literal(value, column_type_for(column, column_types))
        )
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
pub fn build_schema_sync_plan_with_plugin(
    result: &SchemaCompareResult,
    target_database: &str,
    target_schema: Option<&str>,
    plugin: &dyn DatabasePlugin,
) -> SyncPlan {
    build_schema_sync_plan_with_plugin_options(
        result,
        target_database,
        target_schema,
        plugin,
        SchemaSyncPlanOptions::default(),
    )
}

pub fn build_schema_sync_plan_with_plugin_options(
    result: &SchemaCompareResult,
    target_database: &str,
    target_schema: Option<&str>,
    plugin: &dyn DatabasePlugin,
    options: SchemaSyncPlanOptions,
) -> SyncPlan {
    let target_db_type = plugin.name();
    build_schema_sync_plan_with_dialect(
        result,
        target_database,
        target_schema,
        None,
        None,
        &target_db_type,
        &target_db_type,
        &PluginSyncSqlDialect(plugin),
        options,
    )
}

/// Build a schema sync plan while mapping source column types into the target
/// plugin's database dialect.
pub fn build_schema_sync_plan_with_plugin_for_source(
    result: &SchemaCompareResult,
    target_database: &str,
    target_schema: Option<&str>,
    source_db_type: &DatabaseType,
    plugin: &dyn DatabasePlugin,
) -> SyncPlan {
    build_schema_sync_plan_with_plugin_options_for_source(
        result,
        target_database,
        target_schema,
        source_db_type,
        plugin,
        SchemaSyncPlanOptions::default(),
    )
}

/// Option-aware counterpart of
/// [`build_schema_sync_plan_with_plugin_for_source`].
pub fn build_schema_sync_plan_with_plugin_options_for_source(
    result: &SchemaCompareResult,
    target_database: &str,
    target_schema: Option<&str>,
    source_db_type: &DatabaseType,
    plugin: &dyn DatabasePlugin,
    options: SchemaSyncPlanOptions,
) -> SyncPlan {
    let target_db_type = plugin.name();
    build_schema_sync_plan_with_dialect(
        result,
        target_database,
        target_schema,
        None,
        None,
        source_db_type,
        &target_db_type,
        &PluginSyncSqlDialect(plugin),
        options,
    )
}

/// Option-aware schema sync plan builder that also maps source-qualified
/// foreign-key references into the target database/schema namespace.
pub fn build_schema_sync_plan_with_plugin_options_for_source_namespace(
    result: &SchemaCompareResult,
    target_database: &str,
    target_schema: Option<&str>,
    source_database: Option<&str>,
    source_schema: Option<&str>,
    source_db_type: &DatabaseType,
    plugin: &dyn DatabasePlugin,
    options: SchemaSyncPlanOptions,
) -> SyncPlan {
    let target_db_type = plugin.name();
    build_schema_sync_plan_with_dialect(
        result,
        target_database,
        target_schema,
        source_database,
        source_schema,
        source_db_type,
        &target_db_type,
        &PluginSyncSqlDialect(plugin),
        options,
    )
}

fn build_schema_sync_plan_with_dialect(
    result: &SchemaCompareResult,
    target_database: &str,
    target_schema: Option<&str>,
    source_database: Option<&str>,
    source_schema: Option<&str>,
    source_db_type: &DatabaseType,
    target_db_type: &DatabaseType,
    dialect: &dyn SyncSqlDialect,
    options: SchemaSyncPlanOptions,
) -> SyncPlan {
    if result.has_failed_tables() {
        return blocked_schema_sync_plan(result);
    }

    let overrides = if options.type_mapping_overrides.overrides.is_empty() {
        None
    } else {
        Some(&options.type_mapping_overrides)
    };
    let mut foreign_key_drops = Vec::new();
    let mut statements = Vec::new();
    let mut deferred_foreign_key_adds = Vec::new();
    let mut warnings = Vec::new();
    let plan_id = uuid::Uuid::new_v4().to_string();

    if !result.routine_diffs.is_empty() {
        warnings.push(format!(
            "Skipped {} function/procedure changes: routine synchronization is not implemented.",
            result.routine_diffs.len()
        ));
    }
    if !result.trigger_diffs.is_empty() {
        warnings.push(format!(
            "Skipped {} trigger changes: trigger synchronization is not implemented.",
            result.trigger_diffs.len()
        ));
    }

    for table_diff in &result.table_diffs {
        if table_diff_involves_view(table_diff) {
            warnings.push(format!(
                "Skipped view `{}`: view synchronization is not implemented.",
                table_diff.name
            ));
            continue;
        }

        match table_diff.status {
            DiffStatus::Added => {
                if let Some(source_table) = &table_diff.source {
                    let stmt_id = uuid::Uuid::new_v4().to_string();
                    let mapped = match mapped_table_schema_to_design(
                        target_database,
                        source_table,
                        None,
                        source_db_type,
                        target_db_type,
                        overrides,
                    ) {
                        Ok(mapped) => mapped,
                        Err(mapping_warnings) => {
                            warnings.extend(mapping_warnings);
                            continue;
                        }
                    };
                    let mut design = mapped.design;
                    design.foreign_keys.clear();
                    let selected_by_default = mapped.selected_by_default;
                    let mapping_warnings = mapped.warnings;
                    let table_ref = compare_table_reference(
                        dialect,
                        target_database,
                        target_schema,
                        &source_table.name,
                    );
                    statements.push(SyncStatement {
                        id: stmt_id,
                        sql: dialect.build_create_table_sql_with_reference(&design, &table_ref),
                        kind: SyncStatementKind::CreateTable,
                        object_name: Some(table_diff.name.clone()),
                        row_key: None,
                        destructive: false,
                        transactional_safe: true,
                        selected_by_default,
                        warnings: mapping_warnings.clone(),
                    });

                    if dialect.needs_raw_comment_statements() {
                        let table_ref = dialect.format_table_reference(
                            target_database,
                            target_schema,
                            &source_table.name,
                        );
                        if let Some(sql) = raw_table_comment_sql(
                            target_db_type,
                            &table_ref,
                            &source_table.name,
                            None,
                            source_table.comment.as_deref(),
                        ) {
                            statements.push(SyncStatement {
                                id: uuid::Uuid::new_v4().to_string(),
                                sql,
                                kind: SyncStatementKind::Comment,
                                object_name: Some(table_diff.name.clone()),
                                row_key: None,
                                destructive: false,
                                transactional_safe: true,
                                selected_by_default,
                                warnings: mapping_warnings.clone(),
                            });
                        }
                        for column in &source_table.columns {
                            let mapped_column = match mapped_column_schema_to_definition(
                                &source_table.name,
                                column,
                                None,
                                source_db_type,
                                target_db_type,
                                overrides,
                            ) {
                                Ok(mapped_column) => mapped_column,
                                Err(mapping_warning) => {
                                    warnings.push(mapping_warning);
                                    continue;
                                }
                            };
                            if let Some(sql) = raw_column_comment_sql(
                                target_db_type,
                                &table_ref,
                                &source_table.name,
                                column,
                                None,
                                Some(&mapped_column.definition),
                                dialect,
                            ) {
                                statements.push(SyncStatement {
                                    id: uuid::Uuid::new_v4().to_string(),
                                    sql,
                                    kind: SyncStatementKind::Comment,
                                    object_name: Some(table_diff.name.clone()),
                                    row_key: None,
                                    destructive: false,
                                    transactional_safe: true,
                                    selected_by_default,
                                    warnings: mapping_warnings.clone(),
                                });
                            }
                        }
                    }

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
                                    source_database,
                                    source_schema,
                                    dialect,
                                ),
                                kind: SyncStatementKind::AlterTable,
                                object_name: Some(foreign_key.name.clone()),
                                row_key: None,
                                destructive: false,
                                transactional_safe: true,
                                selected_by_default,
                                warnings: mapping_warnings.clone(),
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
                let table_designer_outcome = table_designer_sync_statement(
                    table_diff,
                    target_database,
                    source_db_type,
                    target_db_type,
                    dialect,
                    options.compare_column_order,
                    overrides,
                );
                warnings.extend(table_designer_outcome.warnings);
                let table_designer_statement = table_designer_outcome.statement;
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
                                    let mapped_column = match mapped_column_schema_to_definition(
                                        &table_diff.name,
                                        src,
                                        None,
                                        source_db_type,
                                        target_db_type,
                                        overrides,
                                    ) {
                                        Ok(mapped_column) => mapped_column,
                                        Err(mapping_warning) => {
                                            warnings.push(mapping_warning);
                                            continue;
                                        }
                                    };
                                    statements.push(SyncStatement {
                                        id: stmt_id,
                                        sql: format!(
                                            "ALTER TABLE {} ADD COLUMN {};",
                                            table_ref,
                                            dialect.build_column_def(&mapped_column.definition)
                                        ),
                                        kind: SyncStatementKind::AlterTable,
                                        object_name: Some(table_diff.name.clone()),
                                        row_key: None,
                                        destructive: false,
                                        transactional_safe: true,
                                        selected_by_default: mapped_column.selected_by_default,
                                        warnings: mapped_column.warnings.clone(),
                                    });
                                    if dialect.needs_raw_comment_statements() {
                                        if let Some(sql) = raw_column_comment_sql(
                                            target_db_type,
                                            &table_ref,
                                            &table_diff.name,
                                            src,
                                            None,
                                            Some(&mapped_column.definition),
                                            dialect,
                                        ) {
                                            statements.push(SyncStatement {
                                                id: uuid::Uuid::new_v4().to_string(),
                                                sql,
                                                kind: SyncStatementKind::Comment,
                                                object_name: Some(table_diff.name.clone()),
                                                row_key: None,
                                                destructive: false,
                                                transactional_safe: true,
                                                selected_by_default: mapped_column
                                                    .selected_by_default,
                                                warnings: mapped_column.warnings,
                                            });
                                        }
                                    }
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
                                if let (Some(src), Some(target)) =
                                    (&col_diff.source, &col_diff.target)
                                {
                                    let mapped_column = match mapped_column_schema_to_definition(
                                        &table_diff.name,
                                        src,
                                        Some(target),
                                        source_db_type,
                                        target_db_type,
                                        overrides,
                                    ) {
                                        Ok(mapped_column) => mapped_column,
                                        Err(mapping_warning) => {
                                            warnings.push(mapping_warning);
                                            continue;
                                        }
                                    };
                                    if !raw_column_definition_changed(
                                        src,
                                        target,
                                        source_db_type,
                                        target_db_type,
                                        overrides,
                                    ) {
                                        if dialect.needs_raw_comment_statements() {
                                            if let Some(sql) = raw_column_comment_sql(
                                                target_db_type,
                                                &table_ref,
                                                &table_diff.name,
                                                src,
                                                Some(target),
                                                Some(&mapped_column.definition),
                                                dialect,
                                            ) {
                                                statements.push(SyncStatement {
                                                    id: stmt_id,
                                                    sql,
                                                    kind: SyncStatementKind::Comment,
                                                    object_name: Some(table_diff.name.clone()),
                                                    row_key: None,
                                                    destructive: false,
                                                    transactional_safe: true,
                                                    selected_by_default: mapped_column
                                                        .selected_by_default,
                                                    warnings: mapped_column.warnings,
                                                });
                                            }
                                        }
                                        continue;
                                    }
                                    let sql = if is_mysql_family(target_db_type) {
                                        format!(
                                            "ALTER TABLE {} MODIFY COLUMN {};",
                                            table_ref,
                                            dialect.build_column_def(&mapped_column.definition)
                                        )
                                    } else {
                                        format!(
                                            "ALTER TABLE {} ALTER COLUMN {} TYPE {};",
                                            table_ref,
                                            dialect.quote_identifier(&src.name),
                                            mapped_column.definition.data_type
                                        )
                                    };

                                    let mut statement_warnings = vec![
                                        "此操作可能导致数据类型转换失败或数据丢失".to_string(),
                                    ];
                                    statement_warnings.extend(mapped_column.warnings.clone());
                                    statements.push(SyncStatement {
                                        id: stmt_id,
                                        sql,
                                        kind: SyncStatementKind::AlterTable,
                                        object_name: Some(table_diff.name.clone()),
                                        row_key: None,
                                        destructive: true,
                                        transactional_safe: true,
                                        selected_by_default: false,
                                        warnings: statement_warnings.clone(),
                                    });
                                    if dialect.needs_raw_comment_statements() {
                                        if let Some(sql) = raw_column_comment_sql(
                                            target_db_type,
                                            &table_ref,
                                            &table_diff.name,
                                            src,
                                            Some(target),
                                            Some(&mapped_column.definition),
                                            dialect,
                                        ) {
                                            statements.push(SyncStatement {
                                                id: uuid::Uuid::new_v4().to_string(),
                                                sql,
                                                kind: SyncStatementKind::Comment,
                                                object_name: Some(table_diff.name.clone()),
                                                row_key: None,
                                                destructive: false,
                                                transactional_safe: true,
                                                selected_by_default: false,
                                                warnings: statement_warnings,
                                            });
                                        }
                                    }
                                }
                            }
                        }
                    }

                    if dialect.needs_raw_comment_statements() && table_diff.comment_changed {
                        let source_comment = table_diff
                            .source
                            .as_ref()
                            .and_then(|table| table.comment.as_deref());
                        let target_comment = table_diff
                            .target
                            .as_ref()
                            .and_then(|table| table.comment.as_deref());
                        if let Some(sql) = raw_table_comment_sql(
                            target_db_type,
                            &table_ref,
                            &table_diff.name,
                            target_comment,
                            source_comment,
                        ) {
                            statements.push(SyncStatement {
                                id: uuid::Uuid::new_v4().to_string(),
                                sql,
                                kind: SyncStatementKind::Comment,
                                object_name: Some(table_diff.name.clone()),
                                row_key: None,
                                destructive: false,
                                transactional_safe: true,
                                selected_by_default: true,
                                warnings: vec![],
                            });
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
                                        source_database,
                                        source_schema,
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
        warnings,
        sql_text,
    }
}

pub(super) fn blocked_schema_sync_plan(result: &SchemaCompareResult) -> SyncPlan {
    let warnings = std::iter::once(
        "Schema compare result is incomplete; sync SQL generation is disabled.".to_string(),
    )
    .chain(
        result
            .table_failures
            .iter()
            .map(schema_compare_table_failure_warning),
    )
    .collect();

    SyncPlan {
        id: uuid::Uuid::new_v4().to_string(),
        target_table: String::new(),
        statements: vec![],
        summary: empty_sync_plan_summary(),
        warnings,
        sql_text: String::new(),
    }
}

pub fn schema_compare_table_failure_warning(failure: &super::SchemaCompareTableFailure) -> String {
    format!(
        "{:?} table `{}` failed to compare and was excluded: {}",
        failure.side, failure.table, failure.error
    )
}

fn table_diff_involves_view(table_diff: &super::TableDiff) -> bool {
    table_diff.object_type == SchemaObjectType::View
        || table_diff
            .source
            .as_ref()
            .is_some_and(|table| table.object_type == SchemaObjectType::View)
        || table_diff
            .target
            .as_ref()
            .is_some_and(|table| table.object_type == SchemaObjectType::View)
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
            ref_schema: foreign_key.ref_schema.clone(),
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

struct MappedColumnDefinition {
    definition: ColumnDefinition,
    selected_by_default: bool,
    warnings: Vec<String>,
}

struct MappedTableDesign {
    design: TableDesign,
    selected_by_default: bool,
    warnings: Vec<String>,
}

fn mapped_column_schema_to_definition(
    table_name: &str,
    source: &ColumnSchema,
    target: Option<&ColumnSchema>,
    source_db_type: &DatabaseType,
    target_db_type: &DatabaseType,
    overrides: Option<&super::TypeMappingOverrides>,
) -> Result<MappedColumnDefinition, String> {
    let mapping = map_column_type_with_overrides(
        &source.data_type,
        source_db_type,
        target_db_type,
        overrides,
    );
    let mapping_warning = mapping.warning.as_deref().map(|warning| {
        format!(
            "字段 `{}.{}`：{}",
            table_name.trim(),
            source.name.trim(),
            warning
        )
    });

    if mapping.compatibility == TypeCompatibility::Unsupported {
        return Err(mapping_warning.unwrap_or_else(|| {
            format!(
                "字段 `{}.{}` 的类型 `{}` 无法安全映射到目标数据库，已跳过相关同步 SQL",
                table_name.trim(),
                source.name.trim(),
                source.data_type
            )
        }));
    }

    let mut definition = column_schema_to_definition(source);
    let mapping_context = match overrides {
        Some(overrides) => {
            SchemaTypeMappingContext::with_overrides(source_db_type, target_db_type, overrides)
        }
        None => SchemaTypeMappingContext::new(source_db_type, target_db_type),
    };
    definition.data_type = if mapping.compatibility == TypeCompatibility::Exact {
        source.data_type.clone()
    } else if target.is_some_and(|target| {
        column_types_equivalent(
            &source.data_type,
            &target.data_type,
            mapping_context.clone(),
        )
    }) {
        target
            .map(|target| target.data_type.clone())
            .unwrap_or(mapping.target_type)
    } else {
        mapping.target_type
    };

    Ok(MappedColumnDefinition {
        definition,
        selected_by_default: mapping.compatibility.is_safe_for_automatic_sync(),
        warnings: mapping_warning.into_iter().collect(),
    })
}

fn mapped_table_schema_to_design(
    database: &str,
    source: &TableSchema,
    target: Option<&TableSchema>,
    source_db_type: &DatabaseType,
    target_db_type: &DatabaseType,
    overrides: Option<&super::TypeMappingOverrides>,
) -> Result<MappedTableDesign, Vec<String>> {
    let mut design = table_schema_to_design(database, source);
    let mut selected_by_default = true;
    let mut warnings = Vec::new();
    let mut unsupported_warnings = Vec::new();

    for (definition, source_column) in design.columns.iter_mut().zip(&source.columns) {
        let target_column = target.and_then(|target| {
            target
                .columns
                .iter()
                .find(|column| column.name.trim() == source_column.name.trim())
                .or_else(|| {
                    target
                        .columns
                        .iter()
                        .find(|column| column.name.eq_ignore_ascii_case(&source_column.name))
                })
        });
        match mapped_column_schema_to_definition(
            &source.name,
            source_column,
            target_column,
            source_db_type,
            target_db_type,
            overrides,
        ) {
            Ok(mapped) => {
                definition.data_type = mapped.definition.data_type;
                selected_by_default &= mapped.selected_by_default;
                warnings.extend(mapped.warnings);
            }
            Err(warning) => unsupported_warnings.push(warning),
        }
    }

    if !unsupported_warnings.is_empty() {
        return Err(unsupported_warnings);
    }

    Ok(MappedTableDesign {
        design,
        selected_by_default,
        warnings,
    })
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

    fn data_result_with_insert_and_delete(
        table: &str,
        insert_id: i64,
        delete_id: i64,
    ) -> DataCompareResult {
        DataCompareResult {
            source_table: table.to_string(),
            target_table: table.to_string(),
            key_columns: vec!["id".to_string()],
            columns: vec!["id".to_string()],
            added: vec![create_row(&[("id", json!(insert_id))])],
            removed: vec![create_row(&[("id", json!(delete_id))])],
            ..Default::default()
        }
    }

    fn postgres_schema_plan(result: &SchemaCompareResult) -> SyncPlan {
        let plugin = crate::postgresql::PostgresPlugin::new();
        build_schema_sync_plan_with_plugin(result, "", None, &plugin)
    }

    fn schema_column(name: &str, data_type: &str) -> ColumnSchema {
        ColumnSchema {
            name: name.to_string(),
            data_type: data_type.to_string(),
            nullable: false,
            ..Default::default()
        }
    }

    fn added_table_result(table: &str, columns: Vec<ColumnSchema>) -> SchemaCompareResult {
        let source = TableSchema {
            name: table.to_string(),
            columns,
            ..Default::default()
        };
        SchemaCompareResult {
            table_diffs: vec![super::super::TableDiff {
                name: table.to_string(),
                status: DiffStatus::Added,
                object_type: Default::default(),
                changes: vec![],
                source: Some(source),
                target: None,
                column_diffs: vec![],
                index_diffs: vec![],
                foreign_key_diffs: vec![],
                comment_changed: false,
                table_options_changed: false,
            }],
            added_count: 1,
            ..Default::default()
        }
    }

    #[test]
    fn cross_database_create_table_maps_mysql_integer_types_to_postgres() {
        let result = added_table_result(
            "accounts",
            vec![
                schema_column("id", "INT"),
                schema_column("balance", "BIGINT UNSIGNED"),
            ],
        );
        let plugin = crate::postgresql::PostgresPlugin::new();

        let plan = build_schema_sync_plan_with_plugin_for_source(
            &result,
            "",
            Some("public"),
            &DatabaseType::MySQL,
            &plugin,
        );

        assert_eq!(1, plan.statements.len());
        let statement = &plan.statements[0];
        assert!(statement.sql.contains("\"id\" INTEGER"));
        assert!(statement.sql.contains("\"balance\" NUMERIC(20,0)"));
        assert!(!statement.sql.contains("BIGINT UNSIGNED"));
        assert!(statement.selected_by_default);
    }

    #[test]
    fn cross_database_create_table_maps_binary_and_uuid_types() {
        let postgres_result =
            added_table_result("documents", vec![schema_column("payload", "BYTEA")]);
        let mysql_plugin = crate::mysql::MySqlPlugin::new();
        let mysql_plan = build_schema_sync_plan_with_plugin_for_source(
            &postgres_result,
            "app",
            None,
            &DatabaseType::PostgreSQL,
            &mysql_plugin,
        );

        assert_eq!(1, mysql_plan.statements.len());
        assert!(mysql_plan.statements[0].sql.contains("`payload` LONGBLOB"));
        assert!(!mysql_plan.statements[0].sql.contains("BYTEA"));
        assert!(mysql_plan.statements[0].selected_by_default);

        let mssql_result =
            added_table_result("sessions", vec![schema_column("session_id", "UUID")]);
        let mssql_plugin = crate::mssql::MsSqlPlugin::new();
        let mssql_plan = build_schema_sync_plan_with_plugin_for_source(
            &mssql_result,
            "app",
            Some("dbo"),
            &DatabaseType::PostgreSQL,
            &mssql_plugin,
        );

        assert_eq!(1, mssql_plan.statements.len());
        assert!(
            mssql_plan.statements[0]
                .sql
                .contains("[session_id] UNIQUEIDENTIFIER")
        );
        assert!(!mssql_plan.statements[0].sql.contains(" UUID"));
    }

    #[test]
    fn lossy_cross_database_mapping_is_valid_but_not_selected() {
        let result = added_table_result(
            "events",
            vec![schema_column("occurred_at", "TIMESTAMPTZ(9)")],
        );
        let plugin = crate::mysql::MySqlPlugin::new();

        let plan = build_schema_sync_plan_with_plugin_for_source(
            &result,
            "app",
            None,
            &DatabaseType::PostgreSQL,
            &plugin,
        );

        assert_eq!(1, plan.statements.len());
        let statement = &plan.statements[0];
        assert!(statement.sql.contains("`occurred_at` DATETIME(6)"));
        assert!(!statement.sql.contains("TIMESTAMPTZ"));
        assert!(!statement.selected_by_default);
        assert!(
            statement
                .warnings
                .iter()
                .any(|warning| warning.contains("时区语义"))
        );
        assert!(
            statement
                .warnings
                .iter()
                .any(|warning| warning.contains("精度"))
        );
    }

    #[test]
    fn unsupported_cross_database_mapping_skips_invalid_target_ddl() {
        let result = added_table_result("events", vec![schema_column("payload", "Array(Int32)")]);
        let plugin = crate::postgresql::PostgresPlugin::new();

        let plan = build_schema_sync_plan_with_plugin_for_source(
            &result,
            "",
            Some("public"),
            &DatabaseType::ClickHouse,
            &plugin,
        );

        assert!(plan.statements.is_empty());
        assert!(plan.sql_text.is_empty());
        assert!(
            plan.warnings
                .iter()
                .any(|warning| warning.contains("events.payload")
                    && warning.contains("Array(Int32)"))
        );
    }

    #[test]
    fn mapped_table_design_preserves_equivalent_target_type_declaration() {
        let source = TableSchema {
            name: "users".to_string(),
            columns: vec![schema_column("id", "INT4")],
            ..Default::default()
        };
        let target = TableSchema {
            name: "users".to_string(),
            columns: vec![schema_column("id", "INTEGER")],
            ..Default::default()
        };

        let mapped = mapped_table_schema_to_design(
            "app",
            &source,
            Some(&target),
            &DatabaseType::PostgreSQL,
            &DatabaseType::MySQL,
            None,
        )
        .expect("equivalent cross-database type should be mapped");

        assert_eq!("INTEGER", mapped.design.columns[0].data_type);
        assert!(mapped.selected_by_default);
    }

    #[test]
    fn schema_sync_override_does_not_alter_an_already_equivalent_target_column() {
        use super::super::{
            ColumnDiff, ColumnSchema, DiffStatus, SchemaCompareResult, TableDiff, TableSchema,
            TypeMappingOverride, TypeMappingOverrides,
        };

        let source_column = ColumnSchema {
            name: "display_name".to_string(),
            data_type: "VARCHAR(255)".to_string(),
            nullable: true,
            ..Default::default()
        };
        let target_column = ColumnSchema {
            name: "display_name".to_string(),
            data_type: "TEXT".to_string(),
            nullable: true,
            ..Default::default()
        };
        let result = SchemaCompareResult {
            table_diffs: vec![TableDiff {
                name: "users".to_string(),
                status: DiffStatus::Modified,
                source: Some(TableSchema {
                    name: "users".to_string(),
                    columns: vec![source_column.clone()],
                    ..Default::default()
                }),
                target: Some(TableSchema {
                    name: "users".to_string(),
                    columns: vec![target_column.clone()],
                    ..Default::default()
                }),
                column_diffs: vec![ColumnDiff {
                    name: "display_name".to_string(),
                    status: DiffStatus::Modified,
                    changes: vec!["type changed".to_string()],
                    source: Some(source_column),
                    target: Some(target_column),
                }],
                index_diffs: vec![],
                foreign_key_diffs: vec![],
                comment_changed: false,
                object_type: Default::default(),
                changes: vec![],
                table_options_changed: false,
            }],
            modified_count: 1,
            ..Default::default()
        };
        let mut overrides = TypeMappingOverrides::new();
        overrides.upsert(TypeMappingOverride {
            source_type: "varchar(255)".to_string(),
            target_database: "PostgreSQL".to_string(),
            target_type: "TEXT".to_string(),
            enabled: true,
            note: None,
        });
        let plugin = crate::postgresql::PostgresPlugin::new();

        let plan = build_schema_sync_plan_with_plugin_options_for_source(
            &result,
            "app",
            Some("public"),
            &DatabaseType::MySQL,
            &plugin,
            SchemaSyncPlanOptions {
                compare_column_order: false,
                type_mapping_overrides: overrides,
            },
        );

        assert!(plan.statements.is_empty());
        assert!(plan.sql_text.is_empty());
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
            ..Default::default()
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
    fn data_sync_batch_plan_orders_parent_before_child_and_deletes_in_reverse() {
        let result = DataCompareBatchResult {
            // Deliberately put the child first so dependency ordering, rather
            // than input order, determines the generated plan.
            table_results: vec![
                data_result_with_insert_and_delete("order_items", 11, 12),
                data_result_with_insert_and_delete("orders", 1, 2),
            ],
            table_dependencies: vec![
                DataCompareTableDependency {
                    table: "order_items".to_string(),
                    referenced_table: "orders".to_string(),
                },
                // Duplicate metadata rows must not corrupt indegree counts.
                DataCompareTableDependency {
                    table: "order_items".to_string(),
                    referenced_table: "orders".to_string(),
                },
            ],
            ..Default::default()
        };

        let plan = build_data_sync_batch_plan(&result);
        let operations = plan
            .statements
            .iter()
            .map(|statement| {
                (
                    statement_kind_code_for_test(&statement.kind),
                    statement.object_name.as_deref().unwrap_or_default(),
                )
            })
            .collect::<Vec<_>>();

        assert_eq!(
            operations,
            vec![
                ("insert", "orders"),
                ("insert", "order_items"),
                ("delete", "order_items"),
                ("delete", "orders"),
            ]
        );
        assert_eq!(plan.summary.insert_count, 2);
        assert_eq!(plan.summary.delete_count, 2);
        assert_eq!(plan.summary.total_count, 4);
    }

    #[test]
    fn data_sync_batch_plan_marks_external_fk_writes_unselected() {
        let result = DataCompareBatchResult {
            table_results: vec![DataCompareResult {
                source_table: "order_items".to_string(),
                target_table: "order_items".to_string(),
                key_columns: vec!["id".to_string()],
                columns: vec!["id".to_string()],
                added: vec![create_row(&[("id", json!(1))])],
                ..Default::default()
            }],
            table_dependencies: vec![DataCompareTableDependency {
                table: "order_items".to_string(),
                referenced_table: "orders".to_string(),
            }],
            ..Default::default()
        };

        let plan = build_data_sync_batch_plan(&result);
        let insert = plan
            .statements
            .iter()
            .find(|statement| matches!(statement.kind, SyncStatementKind::Insert))
            .unwrap();

        assert!(!insert.selected_by_default);
        assert!(
            insert
                .warnings
                .iter()
                .any(|warning| warning.contains("not included"))
        );
        assert!(
            plan.warnings
                .iter()
                .any(|warning| warning.contains("not included"))
        );
    }

    #[test]
    fn data_sync_batch_plan_blocks_foreign_key_dependency_cycles() {
        let result = DataCompareBatchResult {
            table_results: vec![
                data_result_with_insert_and_delete("a", 1, 0),
                data_result_with_insert_and_delete("b", 2, 0),
            ],
            table_dependencies: vec![
                DataCompareTableDependency {
                    table: "a".to_string(),
                    referenced_table: "b".to_string(),
                },
                DataCompareTableDependency {
                    table: "b".to_string(),
                    referenced_table: "a".to_string(),
                },
            ],
            ..Default::default()
        };

        let plan = build_data_sync_batch_plan(&result);
        assert!(plan.statements.is_empty());
        assert!(plan.sql_text.is_empty());
        assert!(
            plan.warnings
                .iter()
                .any(|warning| warning.contains("dependency cycle"))
        );
    }

    #[test]
    fn data_sync_batch_plan_keeps_successful_tables_when_another_table_failed() {
        let result = DataCompareBatchResult {
            table_results: vec![DataCompareResult {
                source_table: "users".to_string(),
                target_table: "users".to_string(),
                key_columns: vec!["id".to_string()],
                columns: vec!["id".to_string()],
                added: vec![create_row(&[("id", json!(1))])],
                ..Default::default()
            }],
            table_failures: vec![super::super::DataCompareTableFailure {
                table: "audit_log".to_string(),
                error: "permission denied".to_string(),
            }],
            ..Default::default()
        };

        let plan = build_data_sync_batch_plan(&result);

        assert_eq!(plan.summary.insert_count, 1);
        assert_eq!(plan.statements.len(), 1);
        assert!(
            plan.warnings
                .iter()
                .any(|warning| warning.contains("audit_log")
                    && warning.contains("permission denied"))
        );
    }

    #[test]
    fn data_sync_batch_plan_blocks_sql_when_dependency_metadata_is_incomplete() {
        let result = DataCompareBatchResult {
            table_results: vec![DataCompareResult {
                source_table: "users".to_string(),
                target_table: "users".to_string(),
                key_columns: vec!["id".to_string()],
                columns: vec!["id".to_string()],
                added: vec![create_row(&[("id", json!(1))])],
                ..Default::default()
            }],
            batch_warnings: vec![super::super::DataCompareBatchWarning {
                table: None,
                kind: DataCompareBatchWarningKind::TargetTableMetadataUnavailable,
                error: "metadata timeout".to_string(),
            }],
            ..Default::default()
        };

        let plan = build_data_sync_batch_plan(&result);

        assert!(plan.statements.is_empty());
        assert!(plan.sql_text.is_empty());
        assert_eq!(plan.summary.total_count, 0);
        assert!(
            plan.warnings
                .iter()
                .any(|warning| warning.contains("metadata timeout"))
        );
    }

    #[test]
    fn data_sync_batch_plan_blocks_sql_without_a_consistent_snapshot() {
        let result = DataCompareBatchResult {
            table_results: vec![DataCompareResult {
                source_table: "users".to_string(),
                target_table: "users".to_string(),
                key_columns: vec!["id".to_string()],
                columns: vec!["id".to_string()],
                added: vec![create_row(&[("id", json!(1))])],
                ..Default::default()
            }],
            batch_warnings: vec![super::super::DataCompareBatchWarning {
                table: None,
                kind: DataCompareBatchWarningKind::ConsistentSnapshotUnavailable,
                error: "external driver has no snapshot contract".to_string(),
            }],
            ..Default::default()
        };

        let plan = build_data_sync_batch_plan(&result);

        assert!(result.has_inconsistent_snapshot_risk());
        assert!(result.is_sync_sql_blocked());
        assert!(plan.statements.is_empty());
        assert!(plan.sql_text.is_empty());
        assert_eq!(plan.summary.total_count, 0);
        assert!(plan.warnings.iter().any(|warning| {
            warning.contains("consistent read snapshot")
                || warning.contains("Consistent read snapshot")
        }));
        assert!(
            plan.warnings
                .iter()
                .any(|warning| warning.contains("external driver has no snapshot contract"))
        );
    }

    fn statement_kind_code_for_test(kind: &SyncStatementKind) -> &'static str {
        match kind {
            SyncStatementKind::Insert => "insert",
            SyncStatementKind::Delete => "delete",
            _ => "other",
        }
    }

    #[test]
    fn test_format_value_handles_sql_injection() {
        let value = CellValue::String("'; DROP TABLE users; --".to_string());
        let formatted = format_value(value);
        assert_eq!(formatted, "'''; DROP TABLE users; --'");
        assert!(formatted.contains("''"));
    }

    #[test]
    fn test_data_sync_plan_prepends_create_table_for_missing_target() {
        use super::super::{IndexSchema, TableSchema};

        let schema = TableSchema {
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
                    data_type: "text".to_string(),
                    nullable: true,
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
            foreign_keys: vec![],
            comment: None,
            ..Default::default()
        };
        let result = DataCompareResult {
            source_table: "users".to_string(),
            target_table: "users".to_string(),
            key_columns: vec!["id".to_string()],
            columns: vec!["id".to_string(), "name".to_string()],
            added: vec![create_row(&[("id", json!(1)), ("name", json!("Ada"))])],
            removed: vec![],
            modified: vec![],
            target_table_missing: true,
            missing_target_schema: Some(schema),
            ..Default::default()
        };
        let plugin = crate::postgresql::PostgresPlugin::new();

        let plan = build_data_sync_plan_with_plugin(&result, "app", Some("public"), &plugin);

        assert_eq!(plan.summary.ddl_count, 1);
        assert_eq!(plan.summary.total_count, 2);
        assert!(
            matches!(
                plan.statements.first().map(|statement| &statement.kind),
                Some(SyncStatementKind::CreateTable)
            ),
            "missing-target 同步计划的第一条语句必须是 CREATE TABLE"
        );
        let create_table = plan.statements.first().unwrap();
        assert!(
            create_table
                .sql
                .starts_with("CREATE TABLE \"public\".\"users\""),
            "missing-target CREATE TABLE must use the same qualified target reference as DML: {}",
            create_table.sql
        );
        assert!(create_table.selected_by_default);
        assert!(!create_table.destructive);
        assert!(
            plan.warnings
                .iter()
                .any(|warning| warning.contains("CREATE TABLE"))
        );
        assert!(plan.sql_text.contains("INSERT INTO"));
    }

    #[test]
    fn test_format_value_for_database_is_type_and_dialect_aware() {
        // MySQL BIT 列不能写成带引号的字符串
        assert_eq!(
            format_value_for_database(&json!(true), Some("bit(1)"), Some(DatabaseType::MySQL)),
            "1"
        );
        assert_eq!(
            format_value_for_database(
                &json!("10101010"),
                Some("bit(8)"),
                Some(DatabaseType::MySQL)
            ),
            "b'10101010'"
        );
        assert_eq!(
            format_value_for_database(&json!("0"), Some("bit(1)"), Some(DatabaseType::PostgreSQL)),
            "'0'"
        );

        // PostgreSQL / ClickHouse 数组
        assert_eq!(
            format_value_for_database(
                &json!([1, "a"]),
                Some("integer[]"),
                Some(DatabaseType::PostgreSQL)
            ),
            "'{1,\"a\"}'"
        );
        assert_eq!(
            format_value_for_database(
                &json!([1, "a"]),
                Some("Array(Int64)"),
                Some(DatabaseType::ClickHouse)
            ),
            "[1,'a']"
        );

        // MySQL 时间列把 RFC3339 归一化；SQL Server 使用 N 前缀并转义单引号
        assert_eq!(
            format_value_for_database(
                &json!("2026-05-12T00:00:00Z"),
                Some("datetime"),
                Some(DatabaseType::MySQL)
            ),
            "'2026-05-12 00:00:00'"
        );
        assert_eq!(
            format_value_for_database(
                &json!("O'Brien"),
                Some("nvarchar(64)"),
                Some(DatabaseType::MSSQL)
            ),
            "N'O''Brien'"
        );
        assert_eq!(
            format_value_for_database(&CellValue::Null, None, None),
            "NULL"
        );
        let binary = super::super::binary_cell_value(&[0x01, 0x02, 0xab]);
        assert_eq!(
            format_value_for_database(&binary, Some("blob"), Some(DatabaseType::MySQL)),
            "X'0102ab'"
        );
        assert_eq!(
            format_value_for_database(&binary, Some("bytea"), Some(DatabaseType::PostgreSQL)),
            "decode('0102ab', 'hex')"
        );
        assert_eq!(
            format_value_for_database(&binary, Some("varbinary"), Some(DatabaseType::MSSQL)),
            "0x0102ab"
        );
        assert_eq!(
            format_value_for_database(&binary, Some("raw"), Some(DatabaseType::Oracle)),
            "hextoraw('0102ab')"
        );
        assert_eq!(
            format_value_for_database(&binary, Some("String"), Some(DatabaseType::ClickHouse)),
            "unhex('0102ab')"
        );
        assert_eq!(
            format_value_for_database(
                &binary,
                Some("blob"),
                Some(DatabaseType::External {
                    driver_id: "postgresql".to_string(),
                })
            ),
            "decode('0102ab', 'hex')"
        );
        assert_eq!(
            format_value_for_database(
                &binary,
                Some("blob"),
                Some(DatabaseType::External {
                    driver_id: "mssql".to_string(),
                })
            ),
            "0x0102ab"
        );
        assert_eq!(
            format_value_for_database(
                &binary,
                Some("blob"),
                Some(DatabaseType::External {
                    driver_id: "clickhouse".to_string(),
                })
            ),
            "unhex('0102ab')"
        );
        assert_eq!(
            format_value_for_database(
                &binary,
                Some("blob"),
                Some(DatabaseType::External {
                    driver_id: "mariadb".to_string(),
                })
            ),
            "X'0102ab'"
        );

        assert_eq!(
            format_value_for_database(
                &json!([super::super::binary_cell_value(&[0xab, 0xcd])]),
                Some("bytea[]"),
                Some(DatabaseType::PostgreSQL)
            ),
            r"'{\\xabcd}'"
        );
        assert_eq!(
            format_value_for_database(
                &json!([super::super::binary_cell_value(&[0xab, 0xcd])]),
                Some("Array(String)"),
                Some(DatabaseType::ClickHouse)
            ),
            "[unhex('abcd')]"
        );
        assert_eq!(
            format_value_for_database(&json!(r"a\b"), None, Some(DatabaseType::PostgreSQL)),
            r"'a\b'"
        );
        assert_eq!(
            format_value_for_database(&json!(r"a\b"), None, Some(DatabaseType::SQLite)),
            r"'a\b'"
        );
        assert_eq!(
            format_value_for_database(
                &json!(r"a\b"),
                Some("nvarchar(64)"),
                Some(DatabaseType::MSSQL)
            ),
            r"N'a\b'"
        );
        assert_eq!(
            format_value_for_database(&json!(r"a\b"), None, Some(DatabaseType::Oracle)),
            r"'a\b'"
        );
        assert_eq!(
            format_value_for_database(&json!(r"a\b"), None, Some(DatabaseType::MySQL)),
            r"'a\\b'"
        );
        assert_eq!(
            format_value_for_database(&json!(r"a\b"), None, Some(DatabaseType::ClickHouse)),
            r"'a\\b'"
        );
        assert_eq!(
            format_value_for_database(&json!(r"a\b"), None, None),
            r"'a\b'"
        );

        // External drivers must share the same family-specific literal behavior.
        assert_eq!(
            format_value_for_database(
                &json!("10101010"),
                Some("bit(8)"),
                Some(DatabaseType::External {
                    driver_id: "mariadb".to_string(),
                })
            ),
            "b'10101010'"
        );
        assert_eq!(
            format_value_for_database(
                &json!("2026-05-12T00:00:00Z"),
                Some("datetime"),
                Some(DatabaseType::External {
                    driver_id: "mariadb".to_string(),
                })
            ),
            "'2026-05-12 00:00:00'"
        );
        assert_eq!(
            format_value_for_database(
                &json!("O'Brien"),
                Some("nvarchar(64)"),
                Some(DatabaseType::External {
                    driver_id: "mssql".to_string(),
                })
            ),
            "N'O''Brien'"
        );
        assert_eq!(
            format_value_for_database(
                &json!([1, "a"]),
                Some("Array(Int64)"),
                Some(DatabaseType::External {
                    driver_id: "clickhouse".to_string(),
                })
            ),
            "[1,'a']"
        );
    }

    #[test]
    fn test_generate_update_sql_uses_result_column_order() {
        let source_values = create_row(&[
            ("id", json!(1)),
            ("name", json!("Alice")),
            ("email", json!("alice@example.com")),
        ]);
        let mut key_values = HashMap::new();
        key_values.insert("id".to_string(), json!(1));
        let mut changes = HashMap::new();
        changes.insert(
            "email".to_string(),
            (json!("new@example.com"), json!("old@example.com")),
        );
        changes.insert("name".to_string(), (json!("Alicia"), json!("Alice")));
        let columns = vec!["id".to_string(), "name".to_string(), "email".to_string()];
        let column_types = HashMap::new();
        let dialect = RawSyncSqlDialect;

        let sql = generate_update_sql(
            "users",
            &source_values,
            &key_values,
            &changes,
            &columns,
            &column_types,
            &dialect,
        );

        let name_index = sql.find("name =").expect("SET 应包含 name 列");
        let email_index = sql.find("email =").expect("SET 应包含 email 列");
        assert!(
            name_index < email_index,
            "SET 子句应遵循 result.columns 顺序: {sql}"
        );
    }

    #[test]
    fn test_generate_insert_sql() {
        let row = create_row(&[("id", json!(1)), ("name", json!("Alice"))]);
        let columns = vec!["id".to_string(), "name".to_string()];
        let column_types = HashMap::new();
        let dialect = RawSyncSqlDialect;

        let sql = generate_insert_sql("users", &row, &columns, &column_types, &dialect);

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
        let columns = vec!["id".to_string(), "name".to_string()];
        let column_types = HashMap::new();
        let dialect = RawSyncSqlDialect;

        let sql = generate_update_sql(
            "users",
            &source_values,
            &key_values,
            &changes,
            &columns,
            &column_types,
            &dialect,
        );

        assert!(sql.contains("UPDATE users"));
        assert!(sql.contains("SET"));
        assert!(sql.contains("WHERE"));
        assert!(sql.contains("id = 1"));
    }

    #[test]
    fn test_generate_delete_sql() {
        let mut key_values = HashMap::new();
        key_values.insert("id".to_string(), json!(1));
        let column_types = HashMap::new();
        let dialect = RawSyncSqlDialect;

        let sql = generate_delete_sql("users", &key_values, &column_types, &dialect);

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
            routine_diffs: vec![],
            trigger_diffs: vec![],
            table_failures: vec![],
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
                            changes: vec![],
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
                            changes: vec![],
                        },
                    ],
                    index_diffs: vec![],
                    foreign_key_diffs: vec![],
                    comment_changed: false,
                    object_type: Default::default(),
                    changes: vec![],
                    table_options_changed: false,
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
                    object_type: Default::default(),
                    changes: vec![],
                    table_options_changed: false,
                },
            ],
            added_count: 0,
            removed_count: 1,
            modified_count: 1,
        };

        let plan = postgres_schema_plan(&result);

        // 检查新增列（应该默认选中）
        let add_col = plan
            .statements
            .iter()
            .find(|s| s.sql.contains("ADD COLUMN \"email\""))
            .unwrap();
        assert!(add_col.selected_by_default);
        assert!(!add_col.destructive);

        // 检查删除列（P0: 应该默认不选中）
        let drop_col = plan
            .statements
            .iter()
            .find(|s| s.sql.contains("DROP COLUMN \"legacy\""))
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
    fn test_build_schema_sync_plan_with_plugin_adds_comments_for_new_table() {
        use super::super::{ColumnSchema, DiffStatus, SchemaCompareResult, TableDiff, TableSchema};

        let table = TableSchema {
            name: "users".to_string(),
            columns: vec![ColumnSchema {
                name: "name".to_string(),
                data_type: "text".to_string(),
                nullable: true,
                default_value: None,
                comment: Some("Display name".to_string()),
                ..Default::default()
            }],
            indexes: vec![],
            foreign_keys: vec![],
            comment: Some("User table".to_string()),
            ..Default::default()
        };
        let result = SchemaCompareResult {
            routine_diffs: vec![],
            trigger_diffs: vec![],
            table_failures: vec![],
            table_diffs: vec![TableDiff {
                name: "users".to_string(),
                status: DiffStatus::Added,
                source: Some(table),
                target: None,
                column_diffs: vec![],
                index_diffs: vec![],
                foreign_key_diffs: vec![],
                comment_changed: false,
                object_type: Default::default(),
                changes: vec![],
                table_options_changed: false,
            }],
            added_count: 1,
            removed_count: 0,
            modified_count: 0,
        };

        let plan = postgres_schema_plan(&result);

        assert!(plan.sql_text.contains("CREATE TABLE \"users\""));
        assert!(
            plan.sql_text
                .contains("COMMENT ON TABLE \"users\" IS 'User table';")
        );
        assert!(
            plan.sql_text
                .contains("COMMENT ON COLUMN \"users\".\"name\" IS 'Display name';")
        );
        assert_eq!(1, plan.summary.ddl_count);
    }

    #[test]
    fn test_build_schema_sync_plan_with_plugin_comment_only_column_is_not_destructive() {
        use super::super::{
            ColumnDiff, ColumnSchema, DiffStatus, SchemaCompareResult, TableDiff, TableSchema,
        };

        let target_column = ColumnSchema {
            name: "name".to_string(),
            data_type: "text".to_string(),
            nullable: true,
            default_value: None,
            comment: None,
            ..Default::default()
        };
        let source_column = ColumnSchema {
            comment: Some("Display name".to_string()),
            ..target_column.clone()
        };
        let result = SchemaCompareResult {
            routine_diffs: vec![],
            trigger_diffs: vec![],
            table_failures: vec![],
            table_diffs: vec![TableDiff {
                name: "users".to_string(),
                status: DiffStatus::Modified,
                source: Some(TableSchema {
                    name: "users".to_string(),
                    columns: vec![source_column.clone()],
                    indexes: vec![],
                    foreign_keys: vec![],
                    comment: Some("User table".to_string()),
                    ..Default::default()
                }),
                target: Some(TableSchema {
                    name: "users".to_string(),
                    columns: vec![target_column.clone()],
                    indexes: vec![],
                    foreign_keys: vec![],
                    comment: None,
                    ..Default::default()
                }),
                column_diffs: vec![ColumnDiff {
                    name: "name".to_string(),
                    status: DiffStatus::Modified,
                    source: Some(source_column),
                    target: Some(target_column),
                    changes: vec![],
                }],
                index_diffs: vec![],
                foreign_key_diffs: vec![],
                comment_changed: true,
                object_type: Default::default(),
                changes: vec![],
                table_options_changed: false,
            }],
            added_count: 0,
            removed_count: 0,
            modified_count: 1,
        };

        let plan = postgres_schema_plan(&result);

        assert_eq!(1, plan.statements.len());
        assert!(!plan.statements[0].destructive);
        assert!(plan.statements[0].selected_by_default);
        assert!(
            plan.sql_text
                .contains("COMMENT ON COLUMN \"users\".\"name\" IS 'Display name';")
        );
        assert!(
            plan.sql_text
                .contains("COMMENT ON TABLE \"users\" IS 'User table';")
        );
        assert!(!plan.sql_text.contains("ALTER COLUMN name TYPE"));
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
            routine_diffs: vec![],
            trigger_diffs: vec![],
            table_failures: vec![],
            table_diffs: vec![TableDiff {
                name: "users".to_string(),
                status: DiffStatus::Added,
                source: Some(source),
                target: None,
                column_diffs: vec![],
                index_diffs: vec![],
                foreign_key_diffs: vec![],
                comment_changed: false,
                object_type: Default::default(),
                changes: vec![],
                table_options_changed: false,
            }],
            added_count: 1,
            removed_count: 0,
            modified_count: 0,
        };

        let plugin = crate::postgresql::PostgresPlugin::new();
        let plan = build_schema_sync_plan_with_plugin(&result, "app", Some("audit"), &plugin);

        let statement = plan.statements.first().unwrap();
        assert!(matches!(statement.kind, SyncStatementKind::CreateTable));
        assert_eq!(
            statement.sql,
            "CREATE TABLE \"audit\".\"users\" (\n  \"id\" int NOT NULL,\n  \"name\" varchar(64) DEFAULT 'anonymous'\n);"
        );
        assert!(statement.selected_by_default);
        assert!(!statement.destructive);
    }

    #[test]
    fn test_mssql_schema_sync_plan_qualifies_added_table_create() {
        use super::super::{ColumnSchema, DiffStatus, SchemaCompareResult, TableDiff, TableSchema};

        let result = SchemaCompareResult {
            routine_diffs: vec![],
            trigger_diffs: vec![],
            table_failures: vec![],
            table_diffs: vec![TableDiff {
                name: "users".to_string(),
                status: DiffStatus::Added,
                source: Some(TableSchema {
                    name: "users".to_string(),
                    columns: vec![ColumnSchema {
                        name: "id".to_string(),
                        data_type: "bigint".to_string(),
                        nullable: false,
                        ..Default::default()
                    }],
                    ..Default::default()
                }),
                target: None,
                column_diffs: vec![],
                index_diffs: vec![],
                foreign_key_diffs: vec![],
                comment_changed: false,
                object_type: Default::default(),
                changes: vec![],
                table_options_changed: false,
            }],
            added_count: 1,
            removed_count: 0,
            modified_count: 0,
        };
        let plugin = crate::mssql::MsSqlPlugin::new();

        let with_schema =
            build_schema_sync_plan_with_plugin(&result, "app", Some("sales"), &plugin);
        assert!(
            with_schema.statements[0]
                .sql
                .starts_with("CREATE TABLE [app].[sales].[users]")
        );

        let without_schema = build_schema_sync_plan_with_plugin(&result, "app", None, &plugin);
        assert!(
            without_schema.statements[0]
                .sql
                .starts_with("CREATE TABLE [app]..[users]")
        );
    }

    #[test]
    fn test_where_clause_uses_is_null_for_null_key_values() {
        let mut key_values = HashMap::new();
        key_values.insert("tenant_id".to_string(), CellValue::Null);
        key_values.insert("id".to_string(), json!(1));
        let column_types = HashMap::new();
        let dialect = RawSyncSqlDialect;

        let sql = generate_delete_sql("users", &key_values, &column_types, &dialect);

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
            ..Default::default()
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
            routine_diffs: vec![],
            trigger_diffs: vec![],
            table_failures: vec![],
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
                    changes: vec![],
                }],
                foreign_key_diffs: vec![],
                comment_changed: false,
                object_type: Default::default(),
                changes: vec![],
                table_options_changed: false,
            }],
            added_count: 0,
            removed_count: 0,
            modified_count: 1,
        };

        let plan = postgres_schema_plan(&result);

        assert!(
            plan.statements
                .first()
                .unwrap()
                .sql
                .contains("CREATE UNIQUE INDEX \"idx_users_email\"")
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
            routine_diffs: vec![],
            trigger_diffs: vec![],
            table_failures: vec![],
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
                    changes: vec![],
                }],
                foreign_key_diffs: vec![],
                comment_changed: false,
                object_type: Default::default(),
                changes: vec![],
                table_options_changed: false,
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
    fn test_mysql_schema_sync_plan_uses_foreign_key_referenced_schema() {
        use super::super::{
            DiffStatus, ForeignKeyDiff, ForeignKeySchema, SchemaCompareResult, TableDiff,
            TableSchema,
        };

        let source_fk = ForeignKeySchema {
            name: "fk_order_items_order".to_string(),
            columns: vec!["order_id".to_string()],
            ref_table: "orders".to_string(),
            ref_schema: Some("audit".to_string()),
            ref_columns: vec!["id".to_string()],
            on_delete: None,
            on_update: None,
        };
        let result = SchemaCompareResult {
            routine_diffs: vec![],
            trigger_diffs: vec![],
            table_failures: vec![],
            table_diffs: vec![TableDiff {
                name: "order_items".to_string(),
                status: DiffStatus::Modified,
                source: Some(TableSchema {
                    name: "order_items".to_string(),
                    foreign_keys: vec![source_fk.clone()],
                    ..Default::default()
                }),
                target: Some(TableSchema {
                    name: "order_items".to_string(),
                    ..Default::default()
                }),
                column_diffs: vec![],
                index_diffs: vec![],
                foreign_key_diffs: vec![ForeignKeyDiff {
                    name: source_fk.name.clone(),
                    status: DiffStatus::Added,
                    changes: vec![],
                    source: Some(source_fk),
                    target: None,
                }],
                comment_changed: false,
                object_type: Default::default(),
                changes: vec![],
                table_options_changed: false,
            }],
            added_count: 0,
            removed_count: 0,
            modified_count: 1,
        };
        let plugin = crate::mysql::MySqlPlugin::new();
        let plan = build_schema_sync_plan_with_plugin(&result, "app", Some("public"), &plugin);
        assert!(
            plan.statements
                .iter()
                .any(|statement| { statement.sql.contains("REFERENCES `audit`.`orders`") })
        );
    }

    #[test]
    fn test_mysql_schema_sync_plan_maps_source_database_foreign_key_to_target_database() {
        use super::super::{
            DiffStatus, ForeignKeySchema, SchemaCompareResult, TableDiff, TableSchema,
        };

        let source_fk = ForeignKeySchema {
            name: "qrtz_blob_triggers_ibfk_1".to_string(),
            columns: vec!["TRIGGER_NAME".to_string()],
            ref_table: "QRTZ_TRIGGERS".to_string(),
            ref_schema: Some("comi_app_test".to_string()),
            ref_columns: vec!["TRIGGER_NAME".to_string()],
            on_delete: None,
            on_update: None,
        };
        let result = SchemaCompareResult {
            routine_diffs: vec![],
            trigger_diffs: vec![],
            table_failures: vec![],
            table_diffs: vec![TableDiff {
                name: "QRTZ_BLOB_TRIGGERS".to_string(),
                status: DiffStatus::Added,
                source: Some(TableSchema {
                    name: "QRTZ_BLOB_TRIGGERS".to_string(),
                    foreign_keys: vec![source_fk],
                    ..Default::default()
                }),
                target: None,
                column_diffs: vec![],
                index_diffs: vec![],
                foreign_key_diffs: vec![],
                comment_changed: false,
                object_type: Default::default(),
                changes: vec![],
                table_options_changed: false,
            }],
            added_count: 1,
            removed_count: 0,
            modified_count: 0,
        };
        let plugin = crate::mysql::MySqlPlugin::new();
        let plan = build_schema_sync_plan_with_plugin_options_for_source_namespace(
            &result,
            "sync_test",
            None,
            Some("comi_app_test"),
            None,
            &DatabaseType::MySQL,
            &plugin,
            SchemaSyncPlanOptions::default(),
        );

        assert!(plan.statements.iter().any(|statement| {
            statement
                .sql
                .contains("REFERENCES `sync_test`.`QRTZ_TRIGGERS`")
        }));
        assert!(
            plan.statements
                .iter()
                .all(|statement| !statement.sql.contains("REFERENCES `comi_app_test`."))
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
            routine_diffs: vec![],
            trigger_diffs: vec![],
            table_failures: vec![],
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
                    changes: vec![],
                }],
                foreign_key_diffs: vec![],
                comment_changed: false,
                object_type: Default::default(),
                changes: vec![],
                table_options_changed: false,
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
            routine_diffs: vec![],
            trigger_diffs: vec![],
            table_failures: vec![],
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
                    changes: vec![],
                }],
                foreign_key_diffs: vec![],
                comment_changed: false,
                object_type: Default::default(),
                changes: vec![],
                table_options_changed: false,
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
            routine_diffs: vec![],
            trigger_diffs: vec![],
            table_failures: vec![],
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
                    changes: vec![],
                }],
                index_diffs: vec![],
                foreign_key_diffs: vec![],
                comment_changed: false,
                object_type: Default::default(),
                changes: vec![],
                table_options_changed: false,
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
            routine_diffs: vec![],
            trigger_diffs: vec![],
            table_failures: vec![],
            table_diffs: vec![TableDiff {
                name: "users".to_string(),
                status: DiffStatus::Modified,
                source: Some(source),
                target: Some(target),
                column_diffs: vec![],
                index_diffs: vec![],
                foreign_key_diffs: vec![],
                comment_changed: true,
                object_type: Default::default(),
                changes: vec![],
                table_options_changed: false,
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
    fn test_mysql_schema_sync_plan_can_compare_existing_column_order_changes() {
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
            comment: None,
            ..Default::default()
        };
        let target = TableSchema {
            name: "users".to_string(),
            columns: vec![column("id"), column("name")],
            indexes: vec![],
            foreign_keys: vec![],
            comment: None,
            ..Default::default()
        };
        let result = SchemaCompareResult {
            routine_diffs: vec![],
            trigger_diffs: vec![],
            table_failures: vec![],
            table_diffs: vec![TableDiff {
                name: "users".to_string(),
                status: DiffStatus::Modified,
                source: Some(source),
                target: Some(target),
                column_diffs: vec![],
                index_diffs: vec![],
                foreign_key_diffs: vec![],
                comment_changed: false,
                object_type: Default::default(),
                changes: vec![],
                table_options_changed: false,
            }],
            added_count: 0,
            removed_count: 0,
            modified_count: 1,
        };
        let plugin = crate::mysql::MySqlPlugin::new();

        let plan = build_schema_sync_plan_with_plugin_options(
            &result,
            "app",
            None,
            &plugin,
            SchemaSyncPlanOptions {
                compare_column_order: true,
                ..Default::default()
            },
        );

        assert_eq!(1, plan.statements.len());
        assert!(plan.sql_text.contains("MODIFY COLUMN"));
        assert!(plan.sql_text.contains(" FIRST") || plan.sql_text.contains(" AFTER "));
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
            routine_diffs: vec![],
            trigger_diffs: vec![],
            table_failures: vec![],
            table_diffs: vec![TableDiff {
                name: "users".to_string(),
                status: DiffStatus::Modified,
                source: Some(source),
                target: Some(target),
                column_diffs: vec![],
                index_diffs: vec![],
                foreign_key_diffs: vec![],
                comment_changed: false,
                object_type: Default::default(),
                changes: vec![],
                table_options_changed: false,
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
            routine_diffs: vec![],
            trigger_diffs: vec![],
            table_failures: vec![],
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
                    changes: vec![],
                }],
                index_diffs: vec![],
                foreign_key_diffs: vec![],
                comment_changed: false,
                object_type: Default::default(),
                changes: vec![],
                table_options_changed: false,
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
            routine_diffs: vec![],
            trigger_diffs: vec![],
            table_failures: vec![],
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
                    changes: vec![],
                }],
                index_diffs: vec![],
                foreign_key_diffs: vec![],
                comment_changed: false,
                object_type: Default::default(),
                changes: vec![],
                table_options_changed: false,
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
                ref_schema: None,
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
                ref_schema: None,
                ref_columns: vec!["id".to_string()],
                on_delete: None,
                on_update: None,
            }],
            comment: None,
            ..Default::default()
        };
        let result = SchemaCompareResult {
            routine_diffs: vec![],
            trigger_diffs: vec![],
            table_failures: vec![],
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
                            ref_schema: None,
                            ref_columns: vec!["id".to_string()],
                            on_delete: None,
                            on_update: None,
                        }),
                        target: None,
                        changes: vec![],
                    },
                    ForeignKeyDiff {
                        name: "fk_order_items_legacy".to_string(),
                        status: DiffStatus::Removed,
                        source: None,
                        target: Some(ForeignKeySchema {
                            name: "fk_order_items_legacy".to_string(),
                            columns: vec!["legacy_order_id".to_string()],
                            ref_table: "orders".to_string(),
                            ref_schema: None,
                            ref_columns: vec!["id".to_string()],
                            on_delete: None,
                            on_update: None,
                        }),
                        changes: vec![],
                    },
                ],
                comment_changed: false,
                object_type: Default::default(),
                changes: vec![],
                table_options_changed: false,
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
            routine_diffs: vec![],
            trigger_diffs: vec![],
            table_failures: vec![],
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
                    object_type: Default::default(),
                    changes: vec![],
                    table_options_changed: false,
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
                            ref_schema: None,
                            ref_columns: vec!["id".to_string()],
                            on_delete: None,
                            on_update: None,
                        }),
                        changes: vec![],
                    }],
                    comment_changed: false,
                    object_type: Default::default(),
                    changes: vec![],
                    table_options_changed: false,
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
            ColumnSchema, DiffStatus, ForeignKeySchema, IndexSchema, SchemaCompareOptions,
            TableSchema, compare_schemas,
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
            indexes: vec![
                IndexSchema {
                    name: "PRIMARY".to_string(),
                    columns: vec!["id".to_string()],
                    unique: true,
                },
                IndexSchema {
                    name: "idx_order_items_order".to_string(),
                    columns: vec!["order_id".to_string()],
                    unique: false,
                },
            ],
            foreign_keys: vec![ForeignKeySchema {
                name: "fk_order_items_order".to_string(),
                columns: vec!["order_id".to_string()],
                ref_table: "orders".to_string(),
                ref_schema: None,
                ref_columns: vec!["id".to_string()],
                on_delete: Some("CASCADE".to_string()),
                on_update: Some("RESTRICT".to_string()),
            }],
            comment: None,
            ..Default::default()
        };
        let result =
            compare_schemas(vec![source], vec![], SchemaCompareOptions::default()).unwrap();
        let table_diff = &result.table_diffs[0];
        assert_eq!(table_diff.status, DiffStatus::Added);
        assert_eq!(table_diff.index_diffs.len(), 2);
        assert_eq!(table_diff.foreign_key_diffs.len(), 1);
        let plugin = crate::mysql::MySqlPlugin::new();

        let plan = build_schema_sync_plan_with_plugin(&result, "app", None, &plugin);
        let sql = plan
            .statements
            .iter()
            .map(|statement| statement.sql.as_str())
            .collect::<Vec<_>>();

        assert_eq!(2, sql.len());
        assert!(sql[0].starts_with("CREATE TABLE `app`.`order_items`"));
        assert!(sql[0].contains("PRIMARY KEY (`id`)"));
        assert!(!sql[0].contains("UNIQUE INDEX `PRIMARY`"));
        assert!(sql[0].contains("INDEX `idx_order_items_order` (`order_id`)"));
        assert_eq!(
            sql[1],
            "ALTER TABLE `app`.`order_items` ADD CONSTRAINT `fk_order_items_order` FOREIGN KEY (`order_id`) REFERENCES `app`.`orders` (`id`) ON DELETE CASCADE ON UPDATE RESTRICT;"
        );
    }

    #[test]
    fn test_mysql_schema_sync_plan_defers_all_foreign_keys_until_all_added_tables_exist() {
        use super::super::{
            ColumnSchema, DiffStatus, ForeignKeySchema, IndexSchema, SchemaCompareResult,
            TableDiff, TableSchema,
        };

        let table =
            |name: &str, reference_column: &str, foreign_key_name: &str, reference_table: &str| {
                TableSchema {
                    name: name.to_string(),
                    columns: vec![
                        ColumnSchema {
                            name: "id".to_string(),
                            data_type: "bigint".to_string(),
                            nullable: false,
                            ..Default::default()
                        },
                        ColumnSchema {
                            name: reference_column.to_string(),
                            data_type: "bigint".to_string(),
                            nullable: false,
                            ..Default::default()
                        },
                    ],
                    indexes: vec![IndexSchema {
                        name: "PRIMARY".to_string(),
                        columns: vec!["id".to_string()],
                        unique: true,
                    }],
                    foreign_keys: vec![ForeignKeySchema {
                        name: foreign_key_name.to_string(),
                        columns: vec![reference_column.to_string()],
                        ref_table: reference_table.to_string(),
                        ref_schema: None,
                        ref_columns: vec!["id".to_string()],
                        on_delete: None,
                        on_update: None,
                    }],
                    ..Default::default()
                }
            };
        let added_diff = |source: TableSchema| TableDiff {
            name: source.name.clone(),
            status: DiffStatus::Added,
            source: Some(source),
            target: None,
            column_diffs: Vec::new(),
            index_diffs: Vec::new(),
            foreign_key_diffs: Vec::new(),
            comment_changed: false,
            object_type: Default::default(),
            changes: Vec::new(),
            table_options_changed: false,
        };
        let result = SchemaCompareResult {
            table_diffs: vec![
                added_diff(table("alpha", "beta_id", "fk_alpha_beta", "beta")),
                added_diff(table("beta", "alpha_id", "fk_beta_alpha", "alpha")),
            ],
            added_count: 2,
            ..Default::default()
        };
        let plugin = crate::mysql::MySqlPlugin::new();

        let plan = build_schema_sync_plan_with_plugin(&result, "app", None, &plugin);
        let create_positions = plan
            .statements
            .iter()
            .enumerate()
            .filter_map(|(index, statement)| {
                matches!(statement.kind, SyncStatementKind::CreateTable).then_some(index)
            })
            .collect::<Vec<_>>();
        let foreign_key_positions = plan
            .statements
            .iter()
            .enumerate()
            .filter_map(|(index, statement)| {
                statement.sql.contains(" ADD CONSTRAINT ").then_some(index)
            })
            .collect::<Vec<_>>();

        assert_eq!(create_positions.len(), 2);
        assert_eq!(foreign_key_positions.len(), 2);
        assert!(
            create_positions.iter().max().unwrap() < foreign_key_positions.iter().min().unwrap()
        );
        for position in create_positions {
            assert!(!plan.statements[position].sql.contains("FOREIGN KEY"));
        }
        assert!(plan.sql_text.contains("fk_alpha_beta"));
        assert!(plan.sql_text.contains("fk_beta_alpha"));
    }

    #[test]
    fn schema_sync_plan_skips_routines_and_triggers_without_ddl() {
        use super::super::{
            DiffStatus, RoutineDiff, RoutineKind, RoutineSchema, SchemaCompareResult, TriggerDiff,
            TriggerSchema,
        };

        let result = SchemaCompareResult {
            routine_diffs: vec![RoutineDiff {
                name: "calculate_total".to_string(),
                kind: RoutineKind::Function,
                status: DiffStatus::Added,
                changes: Vec::new(),
                source: Some(RoutineSchema {
                    kind: RoutineKind::Function,
                    name: "calculate_total".to_string(),
                    schema: Some("public".to_string()),
                    ..Default::default()
                }),
                target: None,
            }],
            trigger_diffs: vec![TriggerDiff {
                name: "audit_orders".to_string(),
                status: DiffStatus::Added,
                changes: Vec::new(),
                source: Some(TriggerSchema {
                    name: "audit_orders".to_string(),
                    schema: Some("public".to_string()),
                    table_name: "orders".to_string(),
                    event: "INSERT".to_string(),
                    timing: "AFTER".to_string(),
                    definition: None,
                }),
                target: None,
            }],
            added_count: 2,
            ..Default::default()
        };

        let plan = postgres_schema_plan(&result);

        assert!(plan.statements.is_empty());
        assert!(plan.sql_text.is_empty());
        assert_eq!(plan.summary.ddl_count, 0);
        assert!(
            plan.warnings
                .iter()
                .any(|warning| warning.contains("function/procedure"))
        );
        assert!(
            plan.warnings
                .iter()
                .any(|warning| warning.contains("trigger"))
        );
    }

    #[test]
    fn test_sync_database_kind_maps_external_driver_aliases() {
        assert_eq!(
            sync_database_kind(&DatabaseType::External {
                driver_id: "mariadb".to_string()
            }),
            SyncDatabaseKind::MySql
        );
        assert_eq!(
            sync_database_kind(&DatabaseType::External {
                driver_id: "SQL Server".to_string()
            }),
            SyncDatabaseKind::SqlServer
        );
        assert_eq!(
            sync_database_kind(&DatabaseType::External {
                driver_id: "oceanbase".to_string()
            }),
            SyncDatabaseKind::MySql
        );
        assert_eq!(
            sync_database_kind(&DatabaseType::External {
                driver_id: "kingbase".to_string()
            }),
            SyncDatabaseKind::PostgreSql
        );
        assert_eq!(
            sync_database_kind(&DatabaseType::External {
                driver_id: "openGauss".to_string()
            }),
            SyncDatabaseKind::PostgreSql
        );
        assert_eq!(
            sync_database_kind(&DatabaseType::External {
                driver_id: "dm".to_string()
            }),
            SyncDatabaseKind::Oracle
        );
        assert_eq!(
            sync_database_kind(&DatabaseType::External {
                driver_id: "oracle-go".to_string()
            }),
            SyncDatabaseKind::Oracle
        );
        assert!(is_mysql_family(&DatabaseType::External {
            driver_id: "mysql".to_string()
        }));
        assert!(is_sqlserver_family(&DatabaseType::External {
            driver_id: "mssql".to_string()
        }));
        assert!(is_clickhouse_family(&DatabaseType::External {
            driver_id: "clickhouse".to_string()
        }));
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
                ref_schema: None,
                ref_columns: vec!["id".to_string()],
                on_delete: Some("CASCADE".to_string()),
                on_update: None,
            }],
            comment: None,
            ..Default::default()
        };
        let result = SchemaCompareResult {
            routine_diffs: vec![],
            trigger_diffs: vec![],
            table_failures: vec![],
            table_diffs: vec![TableDiff {
                name: "events".to_string(),
                status: DiffStatus::Added,
                source: Some(source),
                target: None,
                column_diffs: vec![],
                index_diffs: vec![],
                foreign_key_diffs: vec![],
                comment_changed: false,
                object_type: Default::default(),
                changes: vec![],
                table_options_changed: false,
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
        assert!(sql[0].contains("CREATE TABLE `analytics`.`events`"));
        assert!(!plan.sql_text.contains("FOREIGN KEY"));
        assert!(!plan.sql_text.contains("fk_events_order"));
    }

    #[test]
    fn schema_sync_plan_skips_views_without_table_ddl() {
        use super::super::{
            DiffStatus, SchemaCompareResult, SchemaObjectType, TableDiff, TableSchema,
        };

        let mut added_view = TableSchema {
            name: "active_users".to_string(),
            object_type: SchemaObjectType::View,
            ..Default::default()
        };
        let removed_view = TableSchema {
            name: "legacy_users".to_string(),
            object_type: SchemaObjectType::View,
            ..Default::default()
        };
        let mut source_table = TableSchema {
            name: "kind_changed".to_string(),
            object_type: SchemaObjectType::View,
            ..Default::default()
        };
        let target_table = TableSchema {
            name: "kind_changed".to_string(),
            object_type: SchemaObjectType::Table,
            ..Default::default()
        };
        // Keep the fixture explicit: an added view must be recognized from both
        // the diff and the source model, not merely from a default enum value.
        added_view.object_type = SchemaObjectType::View;
        source_table.object_type = SchemaObjectType::View;

        let result = SchemaCompareResult {
            routine_diffs: vec![],
            trigger_diffs: vec![],
            table_failures: vec![],
            table_diffs: vec![
                TableDiff {
                    name: "active_users".to_string(),
                    status: DiffStatus::Added,
                    object_type: SchemaObjectType::View,
                    changes: vec![],
                    source: Some(added_view),
                    target: None,
                    column_diffs: vec![],
                    index_diffs: vec![],
                    foreign_key_diffs: vec![],
                    comment_changed: false,
                    table_options_changed: false,
                },
                TableDiff {
                    name: "legacy_users".to_string(),
                    status: DiffStatus::Removed,
                    object_type: SchemaObjectType::View,
                    changes: vec![],
                    source: None,
                    target: Some(removed_view),
                    column_diffs: vec![],
                    index_diffs: vec![],
                    foreign_key_diffs: vec![],
                    comment_changed: false,
                    table_options_changed: false,
                },
                TableDiff {
                    name: "kind_changed".to_string(),
                    status: DiffStatus::Modified,
                    object_type: SchemaObjectType::Table,
                    changes: vec!["object_type: view → table".to_string()],
                    source: Some(source_table),
                    target: Some(target_table),
                    column_diffs: vec![],
                    index_diffs: vec![],
                    foreign_key_diffs: vec![],
                    comment_changed: false,
                    table_options_changed: false,
                },
            ],
            added_count: 1,
            removed_count: 1,
            modified_count: 1,
        };

        let plan = postgres_schema_plan(&result);

        assert!(plan.statements.is_empty());
        assert!(plan.sql_text.is_empty());
        assert_eq!(plan.summary.ddl_count, 0);
        assert_eq!(plan.warnings.len(), 3);
        assert!(
            plan.warnings
                .iter()
                .all(|warning| warning.contains("view synchronization is not implemented"))
        );
    }

    #[test]
    fn schema_sync_plan_is_blocked_when_table_metadata_failed() {
        use super::super::{CompareSchemaSide, SchemaCompareResult, SchemaCompareTableFailure};

        let result = SchemaCompareResult {
            routine_diffs: vec![],
            trigger_diffs: vec![],
            table_diffs: vec![],
            table_failures: vec![SchemaCompareTableFailure {
                side: CompareSchemaSide::Target,
                table: "orders".to_string(),
                error: "permission denied".to_string(),
            }],
            added_count: 0,
            removed_count: 0,
            modified_count: 0,
        };

        let plan = postgres_schema_plan(&result);

        assert!(plan.statements.is_empty());
        assert!(plan.sql_text.is_empty());
        assert_eq!(plan.summary.total_count, 0);
        assert!(
            plan.warnings
                .iter()
                .any(|warning| warning.contains("incomplete"))
        );
        assert!(
            plan.warnings
                .iter()
                .any(|warning| warning.contains("orders") && warning.contains("permission denied"))
        );
    }
}
