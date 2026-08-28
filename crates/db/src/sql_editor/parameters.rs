//! SQL 参数检测与替换
//!
//! 纯算法模块：在 `crates/db` 内部，不依赖 UI。负责识别文档中的 SQL
//! 占位符（`?`、`:name`、`$name`、`${name}`、`@name`、MyBatis `#{name}` /
//! `${name}`），把它们从字符串、注释、引用标识符中区分开，并支持占位符到
//! SQL 字面量的替换与预览。

use one_core::storage::DatabaseType;

use super::sql_tokenizer::{SqlTokenKind, SqlTokenizer};

/// 占位符类型。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SqlParameterKind {
    /// `?`（JDBC 匿名参数）
    QuestionMark,
    /// `:name`
    Colon,
    /// `$name` 或 `${name}`
    Dollar,
    /// `@name`
    At,
    /// MyBatis `#{name}`（预编译参数）
    MyBatisHash,
    /// MyBatis `${name}`（原样拼接）
    MyBatisDollar,
}

impl SqlParameterKind {
    /// 参数是否表示“值”（需要占位符替换），`Raw` 为原样拼接。
    pub fn is_value(self) -> bool {
        !matches!(self, SqlParameterKind::MyBatisDollar)
    }
}

/// 参数值类型，用于对话框与字面量生成。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SqlParameterValueType {
    String,
    Number,
    Boolean,
    Null,
    Raw,
}

/// 单个占位符出现。`start`/`end` 为文档字节偏移。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SqlParameterOccurrence {
    pub kind: SqlParameterKind,
    /// `?` 无名字；其他类型为参数名（不含前缀符号）。
    pub name: Option<String>,
    pub start: usize,
    pub end: usize,
}

impl SqlParameterOccurrence {
    /// 参数名（匿名返回 None）。
    pub fn name(&self) -> Option<&str> {
        self.name.as_deref()
    }
}

/// 按名字聚合的参数描述，供参数对话框使用。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SqlParameterDescriptor {
    pub name: String,
    pub kind: SqlParameterKind,
    /// 该参数在语句中出现的次数。
    pub occurrences: usize,
    /// 用户提供的值类型（未填时为默认值）。
    pub value_type: SqlParameterValueType,
}

/// 一次绑定的参数值。
#[derive(Clone, Debug, PartialEq)]
pub struct SqlParameterBinding {
    pub occurrence: SqlParameterOccurrence,
    /// 替换后的 SQL 字面量（已转义）。
    pub literal: String,
}

/// 收集语句中的全部参数出现。
///
/// 识别规则：
/// - `?` 匿名参数
/// - `:name`
/// - `$name` / `${name}`
/// - `@name`（`@@xxx` 系统变量不识别）
/// - MyBatis `#{name}` / `${name}`
///
/// 字符串、注释、引用标识符内部不识别。`::` 操作符不会误判为参数。
pub fn collect_parameters(sql: &str) -> Vec<SqlParameterOccurrence> {
    let tokens = SqlTokenizer::new(sql).tokenize();
    let mut occurrences = Vec::new();

    for (index, token) in tokens.iter().enumerate() {
        if !matches!(token.kind, SqlTokenKind::Unknown) {
            continue;
        }
        match token.text.as_str() {
            "?" => occurrences.push(SqlParameterOccurrence {
                kind: SqlParameterKind::QuestionMark,
                name: None,
                start: token.start,
                end: token.end,
            }),
            ":" => {
                let is_doubled = index > 0
                    && tokens[index - 1].text == ":"
                    || tokens
                        .get(index + 1)
                        .is_some_and(|next| next.text == ":");
                if !is_doubled {
                    if let Some(name) = next_identifier(&tokens, index) {
                        occurrences.push(SqlParameterOccurrence {
                            kind: SqlParameterKind::Colon,
                            name: Some(name.text.clone()),
                            start: token.start,
                            end: name.end,
                        });
                    }
                }
            }
            "@" => {
                let is_doubled = index > 0
                    && tokens[index - 1].text == "@"
                    || tokens
                        .get(index + 1)
                        .is_some_and(|next| next.text == "@");
                if !is_doubled {
                    if let Some(name) = next_identifier(&tokens, index) {
                        occurrences.push(SqlParameterOccurrence {
                            kind: SqlParameterKind::At,
                            name: Some(name.text.clone()),
                            start: token.start,
                            end: name.end,
                        });
                    }
                }
            }
            "$" => {
                // `$name` / `${name}`（也可能是 `$1` 序号占位符，同样作为参数）
                if let Some(next) = tokens.get(index + 1) {
                    if next.text == "{" {
                        if let Some(name) = tokens.get(index + 2).filter(|t| {
                            matches!(t.kind, SqlTokenKind::Ident) || is_number_text(&t.text)
                        }) {
                            if let Some(close) = tokens.get(index + 3) {
                                if close.text == "}" {
                                    occurrences.push(SqlParameterOccurrence {
                                        kind: SqlParameterKind::MyBatisDollar,
                                        name: Some(name.text.clone()),
                                        start: token.start,
                                        end: close.end,
                                    });
                                }
                            }
                        }
                    } else if matches!(next.kind, SqlTokenKind::Ident) || is_number_text(&next.text) {
                        occurrences.push(SqlParameterOccurrence {
                            kind: SqlParameterKind::Dollar,
                            name: Some(next.text.clone()),
                            start: token.start,
                            end: next.end,
                        });
                    }
                }
            }
            "#" => {
                if let Some(open) = tokens.get(index + 1) {
                    if open.text == "{" {
                        if let Some(name) = tokens.get(index + 2).filter(|t| {
                            matches!(t.kind, SqlTokenKind::Ident) || is_number_text(&t.text)
                        }) {
                            if let Some(close) = tokens.get(index + 3) {
                                if close.text == "}" {
                                    occurrences.push(SqlParameterOccurrence {
                                        kind: SqlParameterKind::MyBatisHash,
                                        name: Some(name.text.clone()),
                                        start: token.start,
                                        end: close.end,
                                    });
                                }
                            }
                        }
                    }
                }
            }
            _ => {}
        }
    }

    occurrences
}

/// 聚合为参数描述列表（按出现顺序去重，相同名字合并计数）。
pub fn parameter_descriptors(
    occurrences: &[SqlParameterOccurrence],
) -> Vec<SqlParameterDescriptor> {
    let mut descriptors: Vec<SqlParameterDescriptor> = Vec::new();
    for occurrence in occurrences {
        let name = occurrence.name().unwrap_or("?").to_string();
        if let Some(existing) = descriptors.iter_mut().find(|d| d.name == name) {
            existing.occurrences += 1;
        } else {
            descriptors.push(SqlParameterDescriptor {
                name,
                kind: occurrence.kind,
                occurrences: 1,
                value_type: SqlParameterValueType::String,
            });
        }
    }
    descriptors
}

/// 用绑定值替换占位符，返回替换后的 SQL。
///
/// `bindings` 中的 occurrence 必须来自 `collect_parameters(sql)`；替换按
/// 字节偏移从后往前进行，保证偏移不回退。
pub fn substitute_parameters(sql: &str, bindings: &[SqlParameterBinding]) -> String {
    let mut sorted: Vec<&SqlParameterBinding> = bindings.iter().collect();
    sorted.sort_by_key(|binding| binding.occurrence.start);
    sorted.reverse();

    let mut result = sql.to_string();
    for binding in sorted {
        let range = binding.occurrence.start..binding.occurrence.end;
        if range.end > result.len() {
            continue;
        }
        result.replace_range(range, &binding.literal);
    }
    result
}

/// 生成参数预览 SQL：每个占位符替换为规范形式（`?` 保持 `?`，具名参数为
/// `:name`，MyBatis 保持原样）。用于参数对话框中的 SQL 预览。
pub fn preview_sql(sql: &str, occurrences: &[SqlParameterOccurrence]) -> String {
    let mut sorted: Vec<&SqlParameterOccurrence> = occurrences.iter().collect();
    sorted.sort_by_key(|o| o.start);
    sorted.reverse();

    let mut result = sql.to_string();
    for occurrence in sorted {
        let replacement = match occurrence.kind {
            SqlParameterKind::QuestionMark => "?".to_string(),
            SqlParameterKind::Colon => format!(":{}", occurrence.name().unwrap_or("")),
            SqlParameterKind::At => format!("@{}", occurrence.name().unwrap_or("")),
            SqlParameterKind::Dollar => format!("$({})", occurrence.name().unwrap_or("")),
            SqlParameterKind::MyBatisHash => format!("#{{{}}}", occurrence.name().unwrap_or("")),
            SqlParameterKind::MyBatisDollar => format!("${{{}}}", occurrence.name().unwrap_or("")),
        };
        if occurrence.end > result.len() {
            continue;
        }
        result.replace_range(occurrence.start..occurrence.end, &replacement);
    }
    result
}

/// 将值转成 SQL 字面量片段。
///
/// - String：单引号包裹并转义（数据库相关转义）
/// - Number：原样
/// - Boolean：`TRUE` / `FALSE`
/// - Null：`NULL`
/// - Raw：原样（不转义）
pub fn build_parameter_literal(
    value_type: SqlParameterValueType,
    value: &str,
    database_type: &DatabaseType,
) -> String {
    match value_type {
        SqlParameterValueType::String => {
            let escaped = escape_string_literal(value, database_type);
            format!("'{}'", escaped)
        }
        SqlParameterValueType::Number => value.trim().to_string(),
        SqlParameterValueType::Boolean => match value.trim().to_ascii_lowercase().as_str() {
            "1" | "true" | "yes" | "on" => "TRUE".to_string(),
            _ => "FALSE".to_string(),
        },
        SqlParameterValueType::Null => "NULL".to_string(),
        SqlParameterValueType::Raw => value.to_string(),
    }
}

/// 转义字符串字面量中的单引号（以及 MySQL 的反斜杠）。
pub fn escape_string_literal(value: &str, database_type: &DatabaseType) -> String {
    let mut escaped = String::with_capacity(value.len());
    for ch in value.chars() {
        if ch == '\'' {
            escaped.push('\'');
            escaped.push('\'');
        } else if ch == '\\' && is_backslash_escape(database_type) {
            escaped.push('\\');
            escaped.push('\\');
        } else {
            escaped.push(ch);
        }
    }
    escaped
}

fn is_backslash_escape(database_type: &DatabaseType) -> bool {
    matches!(database_type, DatabaseType::MySQL)
}

fn next_identifier(
    tokens: &[crate::sql_editor::sql_tokenizer::SqlToken],
    index: usize,
) -> Option<&crate::sql_editor::sql_tokenizer::SqlToken> {
    tokens.get(index + 1).filter(|token| {
        matches!(token.kind, SqlTokenKind::Ident) || is_number_text(&token.text)
    })
}

fn is_number_text(text: &str) -> bool {
    !text.is_empty()
        && text
            .bytes()
            .all(|byte| byte.is_ascii_digit() || byte == b'.' || byte == b'-' || byte == b'+')
}
