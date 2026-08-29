//! SQL `@set` 变量系统
//!
//! 支持：
//! ```sql
//! @set ids = (1, 2, 3);
//! SELECT * FROM users WHERE id IN ${ids};
//! ```
//!
//! 执行顺序契约（与参数系统衔接）：
//! 1. 解析执行目标（document + target range）
//! 2. 从全文收集 `@set` 声明
//! 3. 展开声明过的 `${name}` / `$name` 占位符
//! 4. 移除执行目标内部的 `@set` 声明
//! 5. 收集未解析的参数交给参数系统
//! 6. 参数替换
//!
//! 规则：
//! - 变量名大小写不敏感
//! - `@@version` 等原生系统变量不替换
//! - 字符串 / 注释 / 引用标识符内不替换
//! - 未声明的 `${name}` 占位符保留给参数系统

use std::ops::Range;

use super::parameters::{SqlParameterOccurrence, collect_parameters};
use super::sql_tokenizer::{SqlToken, SqlTokenKind, SqlTokenizer};
use super::statement_ranges::SqlStatementSnapshot;

/// `@set name = value;` 声明。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SqlVariableDeclaration {
    /// 变量名（大小写不敏感，原始大小写保留）。
    pub name: String,
    /// 声明的值文本（含括号等，如 `(1, 2, 3)`）。
    pub value: String,
    /// 声明在文档中的字节范围（整个 `@set ...;` 语句，不含分隔符）。
    pub range: Range<usize>,
}

/// 变量展开结果。
#[derive(Clone, Debug, PartialEq)]
pub struct SqlVariableExpansion {
    /// 展开后的目标 SQL（变量已替换、声明已移除）。
    pub target_sql: String,
    /// 目标中剩余、需要交给参数系统的占位符。
    pub unresolved: Vec<SqlParameterOccurrence>,
}

/// 在 `document` 的 `target_range` 范围内执行变量展开。
///
/// `target_range` 是从 document 中提取出来的执行目标字节范围（例如当前语句
/// 或选区）。声明总是在**全文**中收集，但只从目标内部移除。
pub fn expand_variables(document: &str, target_range: Range<usize>) -> SqlVariableExpansion {
    let declarations = collect_declarations(document);
    let target_sql = &document[target_range.clone()];

    let variable_values: Vec<(String, String)> = declarations
        .iter()
        .map(|declaration| {
            (
                declaration.name.to_ascii_lowercase(),
                declaration.value.clone(),
            )
        })
        .collect();

    let expanded = replace_variables(target_sql, &variable_values);
    let cleaned = remove_declarations_in_range(&expanded, &declarations, target_range.start);
    let unresolved = collect_parameters(&cleaned);

    SqlVariableExpansion {
        target_sql: cleaned,
        unresolved,
    }
}

/// 从全文中收集所有 `@set` 声明。
pub fn collect_declarations(document: &str) -> Vec<SqlVariableDeclaration> {
    let snapshot = SqlStatementSnapshot::new(
        document.to_string(),
        super::statement_ranges::SqlDialect::Standard,
    );
    let mut declarations = Vec::new();
    for statement in snapshot.statement_ranges() {
        let sql = snapshot.statement_text(statement);
        let trimmed = sql.trim_start();
        if !trimmed.to_ascii_lowercase().starts_with("@set ") {
            continue;
        }
        let Some((name, value)) = parse_set_declaration(trimmed) else {
            continue;
        };
        let mut end = statement.sql_range.end_byte;
        if let Some(delimiter) = statement.delimiter_range {
            end = delimiter.end_byte;
        }
        declarations.push(SqlVariableDeclaration {
            name,
            value,
            range: statement.sql_range.start_byte..end,
        });
    }
    declarations
}

/// 解析 `@set name = value` 文本（trimmed 后以 `@set ` 开头）。
/// 返回 (变量名, 值文本)。
fn parse_set_declaration(text: &str) -> Option<(String, String)> {
    let rest = text["@set ".len()..].trim_start();
    let (name, rest) = read_identifier(rest)?;
    let rest = rest.trim_start();
    if !rest.starts_with('=') {
        return None;
    }
    let value_start = rest[1..].trim_start();
    let value_end = value_start.find(';').unwrap_or(value_start.len());
    let value = value_start[..value_end].trim().to_string();
    Some((name, value))
}

fn read_identifier(text: &str) -> Option<(String, &str)> {
    let end = text
        .char_indices()
        .find(|(_, ch)| !(ch.is_alphanumeric() || *ch == '_' || *ch == '.'))
        .map(|(index, _)| index)
        .unwrap_or(text.len());
    if end == 0 {
        return None;
    }
    Some((text[..end].to_string(), &text[end..]))
}

/// 将 `${name}` / `$name` 占位符替换为变量的值。
///
/// `variable_values` 为 (小写变量名, 值) 列表。字符串、注释、引用标识符内不
/// 替换；`@@` 系统变量不替换。
fn replace_variables(sql: &str, variable_values: &[(String, String)]) -> String {
    let tokens = SqlTokenizer::new(sql).tokenize();
    let mut replacements = Vec::new();
    for (index, token) in tokens.iter().enumerate() {
        if !matches!(token.kind, SqlTokenKind::Unknown) || token.text != "$" {
            continue;
        }
        if let Some(next) = tokens.get(index + 1) {
            if next.text == "{" {
                if let Some(name_token) = tokens.get(index + 2) {
                    if matches!(name_token.kind, SqlTokenKind::Ident) {
                        if let Some(close) = tokens.get(index + 3) {
                            if close.text == "}" {
                                if let Some(value) = lookup_variable(name_token, variable_values) {
                                    replacements.push((token.start, close.end, value));
                                }
                            }
                        }
                    }
                }
            } else if matches!(next.kind, SqlTokenKind::Ident) {
                if let Some(value) = lookup_variable(next, variable_values) {
                    replacements.push((token.start, next.end, value));
                }
            }
        }
    }
    apply_replacements(sql, replacements)
}

fn lookup_variable(name_token: &SqlToken, variable_values: &[(String, String)]) -> Option<String> {
    let lower = name_token.text.to_ascii_lowercase();
    variable_values
        .iter()
        .find(|(name, _)| *name == lower)
        .map(|(_, value)| value.clone())
}

fn apply_replacements(sql: &str, replacements: Vec<(usize, usize, String)>) -> String {
    let mut sorted = replacements;
    sorted.sort_by(|a, b| b.0.cmp(&a.0));
    let mut result = sql.to_string();
    for (start, end, replacement) in sorted {
        if end > result.len() {
            continue;
        }
        result.replace_range(start..end, &replacement);
    }
    result
}

/// 从目标 SQL 中移除落在目标范围内的 `@set` 声明。
///
/// `declarations` 使用文档坐标，`target_base` 为目标在文档中的起始字节。
fn remove_declarations_in_range(
    target_sql: &str,
    declarations: &[SqlVariableDeclaration],
    target_base: usize,
) -> String {
    let target_end = target_base + target_sql.len();
    let local_ranges = declarations
        .iter()
        .filter(|decl| decl.range.start >= target_base && decl.range.end <= target_end)
        .map(|decl| (decl.range.start - target_base, decl.range.end - target_base))
        .collect::<Vec<_>>();
    apply_removals(target_sql, local_ranges)
}

fn apply_removals(sql: &str, ranges: Vec<(usize, usize)>) -> String {
    let mut sorted = ranges;
    sorted.sort_by(|a, b| b.0.cmp(&a.0));
    let mut result = sql.to_string();
    for (start, end) in sorted {
        if end > result.len() {
            continue;
        }
        result.replace_range(start..end, "");
    }
    result
}
