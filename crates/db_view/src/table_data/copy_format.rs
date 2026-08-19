use db::{ColumnInfo, DatabasePlugin};
use gpui::SharedString;

#[path = "copy_sql_format.rs"]
mod copy_sql_format;

/// 复制格式枚举
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CopyFormat {
    /// Tab 分隔值（默认，与 Excel 兼容）
    Tsv,
    /// 逗号分隔值
    Csv,
    /// JSON 数组格式
    Json,
    /// Markdown 表格
    Markdown,
    /// SQL INSERT 语句
    SqlInsert,
    /// SQL UPDATE 语句（需要主键）
    SqlUpdate,
    /// SQL DELETE 语句（需要主键）
    SqlDelete,
    /// SQL IN 子句（单列时）
    SqlIn,
}

/// 表格元数据（用于生成 SQL）
#[derive(Debug, Clone, Default)]
pub struct TableMetadata {
    /// 表名
    pub table_name: SharedString,
    /// 列名列表
    pub column_names: Vec<SharedString>,
    /// 与列名按索引对齐的数据库列元数据
    pub column_meta: Vec<Option<ColumnInfo>>,
    /// 主键列索引（可能有多个复合主键）
    pub primary_key_indices: Vec<usize>,
}

impl TableMetadata {
    pub fn new(table_name: impl Into<SharedString>) -> Self {
        Self {
            table_name: table_name.into(),
            ..Self::default()
        }
    }

    pub fn with_columns(mut self, columns: Vec<impl Into<SharedString>>) -> Self {
        self.column_names = columns.into_iter().map(Into::into).collect();
        self
    }

    pub fn with_column_meta(mut self, columns: Vec<ColumnInfo>) -> Self {
        self.column_meta = columns.into_iter().map(Some).collect();
        self
    }

    pub fn with_primary_keys(mut self, indices: Vec<usize>) -> Self {
        self.primary_key_indices = indices;
        self
    }

    pub fn select_columns(&self, indices: &[usize]) -> Self {
        let primary_key_indices = indices
            .iter()
            .enumerate()
            .filter_map(|(local, original)| {
                self.primary_key_indices.contains(original).then_some(local)
            })
            .collect();
        Self {
            table_name: self.table_name.clone(),
            column_names: select_items(&self.column_names, indices),
            column_meta: select_items(&self.column_meta, indices),
            primary_key_indices,
        }
    }

    pub fn for_columns(&self, columns: &[SharedString]) -> Self {
        let mut used = vec![false; self.column_names.len()];
        let indices = columns
            .iter()
            .map(|column| find_column_index(&self.column_names, column, &mut used))
            .collect::<Vec<_>>();
        let column_meta = indices
            .iter()
            .map(|index| index.and_then(|index| self.column_meta.get(index).cloned().flatten()))
            .collect();
        let primary_key_indices = indices
            .iter()
            .enumerate()
            .filter_map(|(local, original)| {
                original
                    .filter(|index| self.primary_key_indices.contains(index))
                    .map(|_| local)
            })
            .collect();
        Self {
            table_name: self.table_name.clone(),
            column_names: columns.to_vec(),
            column_meta,
            primary_key_indices,
        }
    }
}

fn select_items<T: Clone>(items: &[T], indices: &[usize]) -> Vec<T> {
    indices
        .iter()
        .filter_map(|index| items.get(*index).cloned())
        .collect()
}

fn find_column_index(
    source: &[SharedString],
    target: &SharedString,
    used: &mut [bool],
) -> Option<usize> {
    let exact = source
        .iter()
        .enumerate()
        .position(|(index, name)| !used[index] && name == target);
    let index = exact.or_else(|| {
        source.iter().enumerate().position(|(index, name)| {
            !used[index] && name.as_ref().eq_ignore_ascii_case(target.as_ref())
        })
    })?;
    used[index] = true;
    Some(index)
}

#[derive(Clone, Copy)]
pub struct CopyFormatContext<'a> {
    pub data: &'a [Vec<Option<String>>],
    pub columns: &'a [SharedString],
    pub metadata: &'a TableMetadata,
    pub plugin: Option<&'a dyn DatabasePlugin>,
}

impl<'a> CopyFormatContext<'a> {
    pub fn new(
        data: &'a [Vec<Option<String>>],
        columns: &'a [SharedString],
        metadata: &'a TableMetadata,
    ) -> Self {
        Self {
            data,
            columns,
            metadata,
            plugin: None,
        }
    }

    pub fn with_plugin(mut self, plugin: &'a dyn DatabasePlugin) -> Self {
        self.plugin = Some(plugin);
        self
    }
}

/// 格式化器
pub struct CopyFormatter;

impl CopyFormatter {
    /// 格式化数据为指定格式
    pub fn format(format: CopyFormat, context: CopyFormatContext<'_>) -> String {
        match format {
            CopyFormat::Tsv => Self::format_tsv(context.data),
            CopyFormat::Csv => Self::format_csv(context.data),
            CopyFormat::Json => Self::format_json(context.data, context.columns),
            CopyFormat::Markdown => Self::format_markdown(context.data, context.columns),
            _ => copy_sql_format::format(format, context),
        }
    }

    fn format_tsv(data: &[Vec<Option<String>>]) -> String {
        data.iter()
            .map(|row| {
                row.iter()
                    .map(|cell| cell.as_deref().unwrap_or("\\N"))
                    .collect::<Vec<_>>()
                    .join("\t")
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn format_csv(data: &[Vec<Option<String>>]) -> String {
        data.iter()
            .map(|row| {
                row.iter()
                    .map(|cell| match cell {
                        Some(cell) => Self::escape_csv_field(cell),
                        None => "\\N".to_string(),
                    })
                    .collect::<Vec<_>>()
                    .join(",")
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn format_json(data: &[Vec<Option<String>>], columns: &[SharedString]) -> String {
        let rows = data
            .iter()
            .map(|row| {
                let fields = row
                    .iter()
                    .enumerate()
                    .map(|(index, value)| {
                        let name = columns.get(index).map_or("_", SharedString::as_ref);
                        format!("    \"{}\": {}", name, Self::to_json_value(value))
                    })
                    .collect::<Vec<_>>();
                format!("  {{\n{}\n  }}", fields.join(",\n"))
            })
            .collect::<Vec<_>>();
        format!("[\n{}\n]", rows.join(",\n"))
    }

    fn format_markdown(data: &[Vec<Option<String>>], columns: &[SharedString]) -> String {
        let Some(first_row) = data.first() else {
            return String::new();
        };
        let header = (0..first_row.len())
            .map(|index| columns.get(index).map_or("-", SharedString::as_ref))
            .collect::<Vec<_>>();
        let separator = vec!["---"; first_row.len()];
        let mut lines = vec![
            format!("| {} |", header.join(" | ")),
            format!("| {} |", separator.join(" | ")),
        ];
        lines.extend(data.iter().map(|row| {
            let values = row
                .iter()
                .map(|cell| cell.as_deref().unwrap_or("\\N").replace('|', "\\|"))
                .collect::<Vec<_>>();
            format!("| {} |", values.join(" | "))
        }));
        lines.join("\n")
    }

    fn escape_csv_field(field: &str) -> String {
        if field.is_empty() || field == "\\N" || field.contains([',', '"', '\n', '\r']) {
            format!("\"{}\"", field.replace('"', "\"\""))
        } else {
            field.to_string()
        }
    }

    fn to_json_value(value: &Option<String>) -> String {
        let Some(value) = value else {
            return "null".to_string();
        };
        if value.parse::<i64>().is_ok() || value.parse::<f64>().is_ok() {
            return value.to_string();
        }
        if matches!(value.as_str(), "true" | "false") {
            return value.to_string();
        }
        let escaped = value
            .replace('\\', "\\\\")
            .replace('"', "\\\"")
            .replace('\n', "\\n")
            .replace('\r', "\\r")
            .replace('\t', "\\t");
        format!("\"{escaped}\"")
    }
}

#[cfg(test)]
#[path = "copy_format_tests.rs"]
mod tests;
