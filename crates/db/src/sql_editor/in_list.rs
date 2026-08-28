//! Paste-as-IN-list 纯算法
//!
//! 把剪贴板内容解析为 SQL IN 列表值，供 `PasteAsInList` action 使用。
//! 限制：最大 1 MiB、最多 10_000 个值；支持逗号 / Tab / 换行分隔；NULL 与
//! 数字不加引号，其余转义为 SQL 字符串字面量。

/// 单条 IN 列表值。
#[derive(Clone, Debug, PartialEq)]
pub enum SqlInListValue {
    /// 不加引号的原样片段（数字、NULL、TRUE/FALSE 等）。
    Raw(String),
    /// 需要转义为字符串字面量的值。
    String(String),
}

impl SqlInListValue {
    /// 生成 SQL 片段（不含外层括号）。
    pub fn to_sql_fragment(&self) -> String {
        match self {
            SqlInListValue::Raw(text) => text.clone(),
            SqlInListValue::String(text) => {
                let escaped = text.replace('\'', "''");
                format!("'{}'", escaped)
            }
        }
    }
}

/// 解析失败原因。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum InListParseError {
    /// 内容为空。
    Empty,
    /// 超过 1 MiB 上限。
    TooLarge,
    /// 超过 10_000 个值上限。
    TooManyValues,
    /// 单个普通文本（缺少分隔符 / 看起来像一句话）拒绝转换。
    SinglePlainText,
    /// 类似 URL / 日期 / 绝对路径（包含 `/` 与分隔符），不误判为列表。
    LooksLikeSingleText,
}

pub const IN_LIST_MAX_BYTES: usize = 1024 * 1024;
pub const IN_LIST_MAX_VALUES: usize = 10_000;

/// 解析剪贴板内容为 IN 列表值。
pub fn parse_in_list(text: &str) -> Result<Vec<SqlInListValue>, InListParseError> {
    if text.is_empty() || text.trim().is_empty() {
        return Err(InListParseError::Empty);
    }
    if text.len() > IN_LIST_MAX_BYTES {
        return Err(InListParseError::TooLarge);
    }

    // 分割：逗号、Tab、换行、分号（行内）都算分隔符；多个连续分隔符允许。
    let raw_parts = split_values(text);

    // 过滤空片段。
    let parts: Vec<&str> = raw_parts.iter().map(|s| s.trim()).filter(|s| !s.is_empty()).collect();

    if parts.is_empty() {
        return Err(InListParseError::Empty);
    }
    if parts.len() > IN_LIST_MAX_VALUES {
        return Err(InListParseError::TooManyValues);
    }

    // 单元素且不含逗号/Tab/换行：可能是“一句话”，拒绝转换，避免误伤。
    if parts.len() == 1 {
        let single = parts[0];
        let has_delimiter = text.contains(',') || text.contains('\t') || text.contains('\n');
        if !has_delimiter {
            if looks_like_single_text(single) {
                return Err(InListParseError::SinglePlainText);
            }
        }
        // 单个值也允许（例如复制了一个数字），只要不是自由文本。
    }

    Ok(parts.into_iter().map(classify_value).collect())
}

/// 生成 SQL IN 列表（不含 `IN` 关键字，只含括号内容）。
pub fn build_in_list_clause(values: &[SqlInListValue]) -> String {
    values
        .iter()
        .map(SqlInListValue::to_sql_fragment)
        .collect::<Vec<_>>()
        .join(", ")
}

fn split_values(text: &str) -> Vec<String> {
    let mut parts = Vec::new();
    let mut current = String::new();
    for ch in text.chars() {
        if ch == ',' || ch == '\t' || ch == '\n' || ch == '\r' || ch == ';' {
            parts.push(std::mem::take(&mut current));
        } else {
            current.push(ch);
        }
    }
    parts.push(current);
    parts
}

/// 判断值是否应该不加引号输出（NULL、数字、布尔等）。
fn classify_value(value: &str) -> SqlInListValue {
    let upper = value.to_ascii_uppercase();
    if upper == "NULL" || upper == "TRUE" || upper == "FALSE" {
        return SqlInListValue::Raw(value.to_string());
    }
    if is_number(value) {
        return SqlInListValue::Raw(value.to_string());
    }
    SqlInListValue::String(value.to_string())
}

fn is_number(value: &str) -> bool {
    if value.is_empty() {
        return false;
    }
    let mut has_digit = false;
    let mut chars = value.chars();
    if let Some(first) = chars.next() {
        if first == '+' || first == '-' {
            // continue
        }
    }
    for ch in value.chars() {
        if ch == '.' || ch == 'e' || ch == 'E' || ch == '+' || ch == '-' {
            continue;
        }
        if ch.is_ascii_digit() {
            has_digit = true;
            continue;
        }
        return false;
    }
    has_digit
}

/// 判断一段无分隔符的文本是否“看起来像自由文本”（而非单值）。
///
/// 规则：包含 `/`（URL / 日期 / 绝对路径）或包含空格且长度较长时，视为
/// 单段文本而非单个值。空行之外，避免把复制的一句话误转为 `'...'`。
fn looks_like_single_text(value: &str) -> bool {
    let trimmed = value.trim();
    if trimmed.contains('/') || trimmed.contains('\\') {
        return true;
    }
    trimmed.split_whitespace().count() > 1
}