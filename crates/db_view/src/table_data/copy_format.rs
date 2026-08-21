use db::{ColumnInfo, DatabasePlugin, binary_value::format_binary_input, executor::BinaryCell};
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
    pub binary_cells: &'a [BinaryCell],
    pub columns: &'a [SharedString],
    pub metadata: &'a TableMetadata,
    pub plugin: Option<&'a dyn DatabasePlugin>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum CopyCell<'a> {
    Null,
    Text(&'a str),
    Binary(&'a [u8]),
}

impl<'a> CopyFormatContext<'a> {
    pub fn new(
        data: &'a [Vec<Option<String>>],
        columns: &'a [SharedString],
        metadata: &'a TableMetadata,
    ) -> Self {
        Self {
            data,
            binary_cells: &[],
            columns,
            metadata,
            plugin: None,
        }
    }

    pub fn with_binary_cells(mut self, binary_cells: &'a [BinaryCell]) -> Self {
        self.binary_cells = binary_cells;
        self
    }

    pub fn with_plugin(mut self, plugin: &'a dyn DatabasePlugin) -> Self {
        self.plugin = Some(plugin);
        self
    }

    pub(super) fn cell(self, row_index: usize, column_index: usize) -> CopyCell<'a> {
        if let Some(bytes) = self
            .binary_cells
            .iter()
            .find(|cell| cell.row_index == row_index && cell.column_index == column_index)
            .map(|cell| cell.bytes.as_slice())
        {
            return CopyCell::Binary(bytes);
        }

        match self
            .data
            .get(row_index)
            .and_then(|row| row.get(column_index))
            .and_then(Option::as_deref)
        {
            Some(value) => CopyCell::Text(value),
            None => CopyCell::Null,
        }
    }
}

/// 格式化器
pub struct CopyFormatter;

impl CopyFormatter {
    /// 格式化数据为指定格式
    pub fn format(format: CopyFormat, context: CopyFormatContext<'_>) -> String {
        match format {
            CopyFormat::Tsv => Self::format_tsv(context),
            CopyFormat::Csv => Self::format_csv(context),
            CopyFormat::Json => Self::format_json(context),
            CopyFormat::Markdown => Self::format_markdown(context),
            _ => copy_sql_format::format(format, context),
        }
    }

    fn format_tsv(context: CopyFormatContext<'_>) -> String {
        context
            .data
            .iter()
            .enumerate()
            .map(|(row_index, row)| {
                row.iter()
                    .enumerate()
                    .map(|(column_index, _)| {
                        Self::plain_cell(context.cell(row_index, column_index))
                    })
                    .collect::<Vec<_>>()
                    .join("\t")
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn format_csv(context: CopyFormatContext<'_>) -> String {
        context
            .data
            .iter()
            .enumerate()
            .map(|(row_index, row)| {
                row.iter()
                    .enumerate()
                    .map(
                        |(column_index, _)| match context.cell(row_index, column_index) {
                            CopyCell::Null => "\\N".to_string(),
                            CopyCell::Text(value) => Self::escape_csv_field(value),
                            CopyCell::Binary(bytes) => {
                                Self::escape_csv_field(&format_binary_input(bytes))
                            }
                        },
                    )
                    .collect::<Vec<_>>()
                    .join(",")
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn format_json(context: CopyFormatContext<'_>) -> String {
        let rows = context
            .data
            .iter()
            .enumerate()
            .map(|(row_index, row)| {
                let fields = row
                    .iter()
                    .enumerate()
                    .map(|(column_index, _)| {
                        let name = context
                            .columns
                            .get(column_index)
                            .map_or("_", SharedString::as_ref);
                        format!(
                            "    {}: {}",
                            Self::json_string(name),
                            Self::to_json_value(context.cell(row_index, column_index))
                        )
                    })
                    .collect::<Vec<_>>();
                format!("  {{\n{}\n  }}", fields.join(",\n"))
            })
            .collect::<Vec<_>>();
        format!("[\n{}\n]", rows.join(",\n"))
    }

    fn format_markdown(context: CopyFormatContext<'_>) -> String {
        let Some(first_row) = context.data.first() else {
            return String::new();
        };
        let header = (0..first_row.len())
            .map(|index| context.columns.get(index).map_or("-", SharedString::as_ref))
            .collect::<Vec<_>>();
        let separator = vec!["---"; first_row.len()];
        let mut lines = vec![
            format!("| {} |", header.join(" | ")),
            format!("| {} |", separator.join(" | ")),
        ];
        lines.extend(context.data.iter().enumerate().map(|(row_index, row)| {
            let values = row
                .iter()
                .enumerate()
                .map(|(column_index, _)| {
                    Self::plain_cell(context.cell(row_index, column_index)).replace('|', "\\|")
                })
                .collect::<Vec<_>>();
            format!("| {} |", values.join(" | "))
        }));
        lines.join("\n")
    }

    fn plain_cell(cell: CopyCell<'_>) -> String {
        match cell {
            CopyCell::Null => "\\N".to_string(),
            CopyCell::Text(value) => value.to_string(),
            CopyCell::Binary(bytes) => format_binary_input(bytes),
        }
    }

    fn escape_csv_field(field: &str) -> String {
        if field.is_empty() || field == "\\N" || field.contains([',', '"', '\n', '\r']) {
            format!("\"{}\"", field.replace('"', "\"\""))
        } else {
            field.to_string()
        }
    }

    fn to_json_value(cell: CopyCell<'_>) -> String {
        let value = match cell {
            CopyCell::Null => return "null".to_string(),
            CopyCell::Binary(bytes) => return Self::json_string(&format_binary_input(bytes)),
            CopyCell::Text(value) => value,
        };
        if matches!(
            serde_json::from_str::<serde_json::Value>(value),
            Ok(serde_json::Value::Number(_))
        ) {
            return value.to_string();
        }
        if matches!(value, "true" | "false") {
            return value.to_string();
        }
        Self::json_string(value)
    }

    fn json_string(value: &str) -> String {
        serde_json::to_string(value).expect("serializing a string to JSON cannot fail")
    }
}

#[cfg(test)]
#[path = "copy_format_tests.rs"]
mod tests;
