//! INSERT value hints 纯算法
//!
//! 在 INSERT 语句的 VALUES 列表内，为每个值槽位计算对应列名。典型用途是
//! 在编辑器里为 `VALUES (?, ?)` 的每个参数显示列名 hint。
//!
//! 支持：
//! - 显式列清单：`INSERT INTO t (a, b) VALUES (1, 2)`
//! - 多行 VALUES
//! - 嵌套表达式（括号内的子表达式不误判为值槽）
//! - 无显式列清单时，由调用方提供 ordinal 列名（`column_1` 等）

use super::sql_tokenizer::{SqlKeyword, SqlTokenKind, SqlTokenizer};

/// 单个 INSERT 值槽 hint。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SqlInsertValueHint {
    /// 值槽起点在目标 SQL 中的字节偏移（对应 `(` 之后的第一个值）。
    pub offset: usize,
    /// 该槽对应的列名（显式列清单或 ordinal 推导）。
    pub column: String,
    /// 行号（0-based；第一行 VALUES 为 0）。
    pub row_index: usize,
}

/// 在语句文本中计算 INSERT 值 hint。
///
/// `statement` 为单个 INSERT 语句文本（已去除分隔符）。
/// `ordinal_columns`：无显式列清单时按列顺序使用的列名列表；为空时使用
/// `column_1`、`column_2` … 占位。
pub fn insert_value_hints(statement: &str, ordinal_columns: &[String]) -> Vec<SqlInsertValueHint> {
    let tokens = SqlTokenizer::new(statement).tokenize();

    // 找到 INSERT 关键字之后的列清单与 VALUES 关键字。
    let Some(insert_index) = tokens
        .iter()
        .position(|token| matches!(token.kind, SqlTokenKind::Keyword(SqlKeyword::Insert)))
    else {
        return Vec::new();
    };

    // 显式列清单：INSERT INTO t (a, b) VALUES ...
    let columns = parse_explicit_columns(&tokens, insert_index);

    // VALUES 关键字位置。
    let Some(values_index) = tokens
        .iter()
        .position(|token| matches!(token.kind, SqlTokenKind::Keyword(SqlKeyword::Values)))
    else {
        return Vec::new();
    };

    // 每个值行是一个 LParen ... RParen 组。列名按行循环（少于列数时补空）。
    let mut hints = Vec::new();
    let mut row_index = 0usize;
    let mut position = values_index + 1;
    while let Some(open) = tokens[position..]
        .iter()
        .position(|token| matches!(token.kind, SqlTokenKind::LParen))
    {
        let open_abs = position + open;
        let open_token = &tokens[open_abs];
        let Some((slot_count, close_end)) = scan_value_row(&tokens, open_abs) else {
            break;
        };
        for slot in 0..slot_count {
            let column = resolve_column(&columns, ordinal_columns, row_index, slot);
            hints.push(SqlInsertValueHint {
                offset: value_slot_offset(statement, &tokens, open_abs + 1, open_token.end, slot),
                column,
                row_index,
            });
        }
        row_index += 1;
        position = close_end;
    }

    hints
}

/// 解析显式列清单。返回 Some(columns) 当且仅当 `INSERT INTO t (a, b)` 存在
/// 非空列清单。
fn parse_explicit_columns(
    tokens: &[crate::sql_editor::sql_tokenizer::SqlToken],
    insert_index: usize,
) -> Option<Vec<String>> {
    let after_insert = &tokens[insert_index + 1..];
    let lparen = after_insert
        .iter()
        .position(|token| matches!(token.kind, SqlTokenKind::LParen))?;
    let open_abs = insert_index + 1 + lparen;
    let mut columns = Vec::new();
    let mut position = open_abs + 1;
    while position < tokens.len() {
        let token = &tokens[position];
        match token.kind {
            SqlTokenKind::Ident => {
                columns.push(token.text.clone());
                position += 1;
            }
            SqlTokenKind::QuotedIdent => {
                columns.push(unquote_identifier(&token.text));
                position += 1;
            }
            SqlTokenKind::Comma => {
                position += 1;
            }
            SqlTokenKind::RParen => {
                return if columns.is_empty() {
                    None
                } else {
                    Some(columns)
                };
            }
            SqlTokenKind::Keyword(SqlKeyword::Values)
            | SqlTokenKind::Keyword(SqlKeyword::Select) => {
                // 列清单解析失败（VALUES 前还有别的东西），退回无显式列。
                return None;
            }
            _ => {
                position += 1;
            }
        }
    }
    if columns.is_empty() {
        None
    } else {
        Some(columns)
    }
}

/// 扫描单个 VALUES 行（`(...)`），返回 (值槽数量, 该行右括号后的 token
/// 下标)。嵌套括号被正确跳过。
fn scan_value_row(
    tokens: &[crate::sql_editor::sql_tokenizer::SqlToken],
    open_index: usize,
) -> Option<(usize, usize)> {
    // 值槽数量按“顶层逗号 + 1”计算。
    let mut depth = 0usize;
    let mut slot_count = 1usize;
    let mut position = open_index;
    while position < tokens.len() {
        let token = &tokens[position];
        match token.kind {
            SqlTokenKind::LParen => {
                depth += 1;
            }
            SqlTokenKind::RParen => {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    return Some((slot_count, position + 1));
                }
            }
            SqlTokenKind::Comma if depth == 1 => {
                slot_count += 1;
            }
            _ => {}
        }
        position += 1;
    }
    None
}

/// 计算第 `slot` 个值槽的起点字节偏移（相对于语句文本）。
fn value_slot_offset(
    statement: &str,
    tokens: &[crate::sql_editor::sql_tokenizer::SqlToken],
    row_content_index: usize,
    row_open_end: usize,
    slot: usize,
) -> usize {
    let mut depth = 0usize;
    let mut slot_count = 0usize;
    let mut position = row_content_index;
    while position < tokens.len() {
        let token = &tokens[position];
        match token.kind {
            SqlTokenKind::LParen => depth += 1,
            SqlTokenKind::RParen => {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    return row_open_end;
                }
            }
            SqlTokenKind::Comma if depth == 0 => {
                slot_count += 1;
                if slot_count == slot {
                    return token.end.min(statement.len());
                }
            }
            _ => {}
        }
        position += 1;
    }
    row_open_end
}

/// 解析列名。显式列清单按 slot 取；否则用 ordinal 列名（超出时用
/// `column_{slot+1}`）。
fn resolve_column(
    explicit: &Option<Vec<String>>,
    ordinal_columns: &[String],
    row_index: usize,
    slot: usize,
) -> String {
    if let Some(columns) = explicit {
        if let Some(name) = columns.get(slot) {
            return name.clone();
        }
        return format!("column_{}", slot + 1);
    }
    let _ = row_index;
    if let Some(name) = ordinal_columns.get(slot) {
        return name.clone();
    }
    format!("column_{}", slot + 1)
}

/// 去掉引用标识符的引号（支持 `"name"`、`` `name` ``、`[name]`）。
fn unquote_identifier(text: &str) -> String {
    let trimmed = text.trim();
    let bytes = trimmed.as_bytes();
    if bytes.len() >= 2 {
        let first = bytes[0];
        let last = bytes[bytes.len() - 1];
        if (first == b'"' && last == b'"')
            || (first == b'`' && last == b'`')
            || (first == b'[' && last == b']')
        {
            let inner = &trimmed[1..trimmed.len() - 1];
            return inner.replace("\"\"", "\"").replace("``", "`");
        }
    }
    trimmed.to_string()
}
