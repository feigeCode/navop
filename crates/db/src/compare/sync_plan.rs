use super::{CellValue, DataCompareResult, RowData};
use serde::{Deserialize, Serialize};

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

/// 从数据比较结果生成同步计划
///
/// # P0 安全保护
/// - DELETE 语句默认不选中（`selected_by_default = false`）
/// - 所有 DELETE 标记为破坏性操作（`destructive = true`）
pub fn build_data_sync_plan(result: &DataCompareResult) -> SyncPlan {
    let mut statements = Vec::new();
    let plan_id = uuid::Uuid::new_v4().to_string();

    // 生成 INSERT 语句（新增行）
    for row in &result.added {
        let stmt_id = uuid::Uuid::new_v4().to_string();
        let sql = generate_insert_sql(&result.target_table, row, &result.columns);

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
            &result.target_table,
            &modified.source_values,
            &modified.key_values,
            &modified.changes,
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
        let sql = generate_delete_sql(&result.target_table, &key_values);

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
            destructive: true,  // 破坏性操作
            transactional_safe: true,
            selected_by_default: false,  // P0: 默认不选中
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
    let sql_text = statements.iter().map(|s| s.sql.as_str()).collect::<Vec<_>>().join("\n");

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
fn generate_insert_sql(table: &str, row: &RowData, columns: &[String]) -> String {
    let cols = columns.join(", ");
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
) -> String {
    let set_clause = changes
        .keys()
        .map(|col| {
            let value = source_values.get(col).cloned().unwrap_or(CellValue::Null);
            format!("{} = {}", col, format_value(value))
        })
        .collect::<Vec<_>>()
        .join(", ");

    let where_clause = key_values
        .iter()
        .map(|(col, val)| format!("{} = {}", col, format_value(val.clone())))
        .collect::<Vec<_>>()
        .join(" AND ");

    format!("UPDATE {} SET {} WHERE {};", table, set_clause, where_clause)
}

/// 生成 DELETE SQL
fn generate_delete_sql(
    table: &str,
    key_values: &std::collections::HashMap<String, CellValue>,
) -> String {
    let where_clause = key_values
        .iter()
        .map(|(col, val)| format!("{} = {}", col, format_value(val.clone())))
        .collect::<Vec<_>>()
        .join(" AND ");

    format!("DELETE FROM {} WHERE {};", table, where_clause)
}

/// 格式化值为 SQL 字面量
fn format_value(value: CellValue) -> String {
    match value {
        CellValue::Null => "NULL".to_string(),
        CellValue::Bool(b) => if b { "TRUE" } else { "FALSE" }.to_string(),
        CellValue::Number(n) => n.to_string(),
        CellValue::String(s) => format!("'{}'", s.replace('\'', "''")),
        _ => format!("'{}'", serde_json::to_string(&value).unwrap_or_else(|_| "NULL".to_string()).replace('\'', "''")),
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
    if map.is_empty() {
        None
    } else {
        Some(map)
    }
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

        let sql = generate_insert_sql("users", &row, &columns);

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

        let sql = generate_update_sql("users", &source_values, &key_values, &changes);

        assert!(sql.contains("UPDATE users"));
        assert!(sql.contains("SET"));
        assert!(sql.contains("WHERE"));
        assert!(sql.contains("id = 1"));
    }

    #[test]
    fn test_generate_delete_sql() {
        let mut key_values = HashMap::new();
        key_values.insert("id".to_string(), json!(1));

        let sql = generate_delete_sql("users", &key_values);

        assert!(sql.contains("DELETE FROM users"));
        assert!(sql.contains("WHERE id = 1"));
    }
}
