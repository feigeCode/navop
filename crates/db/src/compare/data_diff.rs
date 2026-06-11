use super::{
    CellValue, DataCompareError, DataCompareModifiedRow, DataCompareResult, KeyValues, RowData,
};
use std::collections::HashMap;

/// 数据比较选项
#[derive(Debug, Clone)]
pub struct DataCompareOptions {
    /// 源表名
    pub source_table: String,
    /// 目标表名
    pub target_table: String,
    /// 键列名列表（用于唯一定位行）
    pub key_columns: Vec<String>,
    /// 要比较的列名列表
    pub columns: Vec<String>,
}

/// 比较两侧数据行，返回差异结果
///
/// # 参数
/// - `source_rows`: 源端数据行（每行是列名到值的映射）
/// - `target_rows`: 目标端数据行
/// - `options`: 比较选项（包含键列和比较列）
///
/// # 返回
/// - `Ok(DataCompareResult)`: 比较结果，包含新增、删除、修改行
/// - `Err(DataCompareError)`: 错误，包括键列为空、键列不存在、重复键等
pub fn compare_data_rows(
    source_rows: Vec<RowData>,
    target_rows: Vec<RowData>,
    options: DataCompareOptions,
) -> Result<DataCompareResult, DataCompareError> {
    // 校验键列非空
    if options.key_columns.is_empty() {
        return Err(DataCompareError::EmptyKeyColumns);
    }

    // 校验键列存在于比较列中
    for key_col in &options.key_columns {
        if !options.columns.contains(key_col) {
            return Err(DataCompareError::KeyColumnNotFound(key_col.clone()));
        }
    }

    // 构建源端和目标端的键 -> 行数据映射
    let source_map = build_key_map(&source_rows, &options.key_columns, true)?;
    let target_map = build_key_map(&target_rows, &options.key_columns, false)?;

    // 识别新增、删除、修改行
    let mut added = Vec::new();
    let mut removed = Vec::new();
    let mut modified = Vec::new();

    // 遍历源端，找新增和修改
    for (key, source_row) in &source_map {
        if let Some(target_row) = target_map.get(key) {
            // 两端都存在，比较非键列值
            if let Some(modified_row) =
                compare_row_values(source_row, target_row, &options.key_columns, &options.columns)
            {
                modified.push(modified_row);
            }
        } else {
            // 目标端不存在，新增行
            added.push(source_row.clone());
        }
    }

    // 遍历目标端，找删除
    for (key, target_row) in &target_map {
        if !source_map.contains_key(key) {
            // 源端不存在，删除行
            removed.push(target_row.clone());
        }
    }

    Ok(DataCompareResult {
        source_table: options.source_table,
        target_table: options.target_table,
        key_columns: options.key_columns,
        columns: options.columns,
        added,
        removed,
        modified,
        source_truncated: false,
        target_truncated: false,
    })
}

/// 构建键 -> 行数据的映射
///
/// 键的生成规则：
/// - 单个键列：该值的 JSON 序列化
/// - 多个键列：多个 JSON 字符串用 U+001F (Unit Separator) 拼接
fn build_key_map(
    rows: &[RowData],
    key_columns: &[String],
    is_source: bool,
) -> Result<HashMap<String, RowData>, DataCompareError> {
    let mut map = HashMap::new();

    for row in rows {
        let key = generate_row_key(row, key_columns)?;

        // 检测重复键
        if map.contains_key(&key) {
            return if is_source {
                Err(DataCompareError::DuplicateSourceKey(key))
            } else {
                Err(DataCompareError::DuplicateTargetKey(key))
            };
        }

        map.insert(key, row.clone());
    }

    Ok(map)
}

/// 为一行数据生成唯一键
fn generate_row_key(row: &RowData, key_columns: &[String]) -> Result<String, DataCompareError> {
    let key_values: Vec<String> = key_columns
        .iter()
        .map(|col| {
            row.get(col)
                .map(|v| serde_json::to_string(v).unwrap_or_else(|_| "null".to_string()))
                .unwrap_or_else(|| "null".to_string())
        })
        .collect();

    if key_columns.len() == 1 {
        Ok(key_values[0].clone())
    } else {
        // 多键用 U+001F 分隔
        Ok(key_values.join("\u{001F}"))
    }
}

/// 比较两行的非键列值，返回修改详情
fn compare_row_values(
    source_row: &RowData,
    target_row: &RowData,
    key_columns: &[String],
    columns: &[String],
) -> Option<DataCompareModifiedRow> {
    let mut changes = HashMap::new();
    let mut key_values = KeyValues::new();

    // 提取键列值
    for key_col in key_columns {
        if let Some(val) = source_row.get(key_col) {
            key_values.insert(key_col.clone(), val.clone());
        }
    }

    // 比较非键列
    for col in columns {
        if key_columns.contains(col) {
            continue; // 跳过键列
        }

        let source_val = source_row.get(col).cloned().unwrap_or(CellValue::Null);
        let target_val = target_row.get(col).cloned().unwrap_or(CellValue::Null);

        // 使用 serde_json::Value 的等值语义比较
        if source_val != target_val {
            changes.insert(col.clone(), (source_val, target_val));
        }
    }

    // 有变化才返回修改行
    if changes.is_empty() {
        None
    } else {
        Some(DataCompareModifiedRow {
            key_values,
            source_values: source_row.clone(),
            target_values: target_row.clone(),
            changes,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn create_row(data: &[(&str, serde_json::Value)]) -> RowData {
        data.iter()
            .map(|(k, v)| (k.to_string(), v.clone()))
            .collect()
    }

    #[test]
    fn test_empty_key_columns_error() {
        let result = compare_data_rows(
            vec![],
            vec![],
            DataCompareOptions {
                source_table: "test".to_string(),
                target_table: "test".to_string(),
                key_columns: vec![],
                columns: vec!["id".to_string()],
            },
        );

        assert!(matches!(result, Err(DataCompareError::EmptyKeyColumns)));
    }

    #[test]
    fn test_key_column_not_found() {
        let result = compare_data_rows(
            vec![],
            vec![],
            DataCompareOptions {
                source_table: "test".to_string(),
                target_table: "test".to_string(),
                key_columns: vec!["missing".to_string()],
                columns: vec!["id".to_string()],
            },
        );

        assert!(matches!(
            result,
            Err(DataCompareError::KeyColumnNotFound(_))
        ));
    }

    #[test]
    fn test_added_rows() {
        let source = vec![
            create_row(&[("id", json!(1)), ("name", json!("Alice"))]),
            create_row(&[("id", json!(2)), ("name", json!("Bob"))]),
        ];
        let target = vec![create_row(&[("id", json!(1)), ("name", json!("Alice"))])];

        let result = compare_data_rows(
            source,
            target,
            DataCompareOptions {
                source_table: "users".to_string(),
                target_table: "users".to_string(),
                key_columns: vec!["id".to_string()],
                columns: vec!["id".to_string(), "name".to_string()],
            },
        )
        .unwrap();

        assert_eq!(result.added.len(), 1);
        assert_eq!(result.added[0].get("id"), Some(&json!(2)));
        assert_eq!(result.removed.len(), 0);
        assert_eq!(result.modified.len(), 0);
    }

    #[test]
    fn test_removed_rows() {
        let source = vec![create_row(&[("id", json!(1)), ("name", json!("Alice"))])];
        let target = vec![
            create_row(&[("id", json!(1)), ("name", json!("Alice"))]),
            create_row(&[("id", json!(2)), ("name", json!("Bob"))]),
        ];

        let result = compare_data_rows(
            source,
            target,
            DataCompareOptions {
                source_table: "users".to_string(),
                target_table: "users".to_string(),
                key_columns: vec!["id".to_string()],
                columns: vec!["id".to_string(), "name".to_string()],
            },
        )
        .unwrap();

        assert_eq!(result.added.len(), 0);
        assert_eq!(result.removed.len(), 1);
        assert_eq!(result.removed[0].get("id"), Some(&json!(2)));
        assert_eq!(result.modified.len(), 0);
    }

    #[test]
    fn test_modified_rows() {
        let source = vec![create_row(&[("id", json!(1)), ("name", json!("Alice"))])];
        let target = vec![create_row(&[("id", json!(1)), ("name", json!("Alicia"))])];

        let result = compare_data_rows(
            source,
            target,
            DataCompareOptions {
                source_table: "users".to_string(),
                target_table: "users".to_string(),
                key_columns: vec!["id".to_string()],
                columns: vec!["id".to_string(), "name".to_string()],
            },
        )
        .unwrap();

        assert_eq!(result.added.len(), 0);
        assert_eq!(result.removed.len(), 0);
        assert_eq!(result.modified.len(), 1);
        assert_eq!(result.modified[0].changes.len(), 1);
        assert!(result.modified[0].changes.contains_key("name"));
    }

    #[test]
    fn test_composite_key() {
        let source = vec![create_row(&[
            ("user_id", json!(1)),
            ("order_id", json!(100)),
            ("amount", json!(50.0)),
        ])];
        let target = vec![create_row(&[
            ("user_id", json!(1)),
            ("order_id", json!(100)),
            ("amount", json!(75.0)),
        ])];

        let result = compare_data_rows(
            source,
            target,
            DataCompareOptions {
                source_table: "orders".to_string(),
                target_table: "orders".to_string(),
                key_columns: vec!["user_id".to_string(), "order_id".to_string()],
                columns: vec![
                    "user_id".to_string(),
                    "order_id".to_string(),
                    "amount".to_string(),
                ],
            },
        )
        .unwrap();

        assert_eq!(result.modified.len(), 1);
        assert!(result.modified[0].changes.contains_key("amount"));
    }

    #[test]
    fn test_duplicate_key_error() {
        let source = vec![
            create_row(&[("id", json!(1)), ("name", json!("Alice"))]),
            create_row(&[("id", json!(1)), ("name", json!("Bob"))]),
        ];
        let target = vec![];

        let result = compare_data_rows(
            source,
            target,
            DataCompareOptions {
                source_table: "users".to_string(),
                target_table: "users".to_string(),
                key_columns: vec!["id".to_string()],
                columns: vec!["id".to_string(), "name".to_string()],
            },
        );

        assert!(matches!(
            result,
            Err(DataCompareError::DuplicateSourceKey(_))
        ));
    }
}
