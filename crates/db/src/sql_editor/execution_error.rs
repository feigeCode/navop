//! 执行错误位置解析与源码映射
//!
//! 纯算法模块：把数据库驱动返回的错误文本中的 line/column 解析成结构化位置，
//! 再把位置映射回 SQL 编辑器文档中的字节范围。
//!
//! 支持解析：
//! - PostgreSQL `POSITION n` / `LINE n: ...` / `at character n`
//! - MySQL `at line n`
//! - SQL Server `Line n`
//! - Oracle `ORA-06550: line n, column m`
//! - SQLite `near "..."`
//! - 结构化错误文本中的 `(line n, column m)` 形式

use one_core::storage::DatabaseType;

use super::execution::SqlExecutionResultSource;
use super::statement_ranges::SqlTextRange;

/// 驱动错误中的位置信息（1-based line/column）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SqlExecutionErrorLocation {
    pub line: Option<u32>,
    pub column: Option<u32>,
    /// 直接给出的 byte offset（若驱动提供）。
    pub byte_offset: Option<usize>,
}

/// 从驱动错误消息中提取位置。
///
/// 不同数据库的格式不同；无法识别时返回 `None`（不猜测）。
pub fn extract_error_location(
    database_type: &DatabaseType,
    message: &str,
) -> Option<SqlExecutionErrorLocation> {
    match database_type {
        DatabaseType::PostgreSQL => parse_postgres_error(message),
        DatabaseType::MySQL => parse_mysql_error(message),
        DatabaseType::MSSQL => parse_sql_server_error(message),
        DatabaseType::Oracle => parse_oracle_error(message),
        DatabaseType::SQLite => parse_sqlite_error(message),
        DatabaseType::DuckDB => parse_postgres_error(message).or_else(|| parse_generic(message)),
        DatabaseType::ClickHouse => parse_generic(message),
        // TDengine 错误消息无标准位置格式,走通用解析。
        DatabaseType::TDengine => parse_generic(message),
        DatabaseType::External { .. } => parse_generic(message),
    }
    .or_else(|| parse_generic(message))
}

fn parse_postgres_error(message: &str) -> Option<SqlExecutionErrorLocation> {
    // `LINE n: ...` 形式。
    if let Some(line) = capture_number_after(message, "LINE ") {
        return Some(SqlExecutionErrorLocation {
            line: Some(line),
            column: None,
            byte_offset: None,
        });
    }
    // `at character n`（1-based）。
    if let Some(position) = capture_number_after(message, "at character ") {
        return Some(SqlExecutionErrorLocation {
            line: None,
            column: None,
            byte_offset: Some(position.saturating_sub(1) as usize),
        });
    }
    // `POSITION n`（有时用大写）。
    if let Some(position) = capture_number_after_nocase(message, "position ") {
        return Some(SqlExecutionErrorLocation {
            line: None,
            column: None,
            byte_offset: Some(position.saturating_sub(1) as usize),
        });
    }
    None
}

fn parse_mysql_error(message: &str) -> Option<SqlExecutionErrorLocation> {
    // MySQL 通常没有精确位置；`at line n` 表示语句内行号。
    capture_number_after(message, "at line ").map(|line| SqlExecutionErrorLocation {
        line: Some(line),
        column: None,
        byte_offset: None,
    })
}

fn parse_sql_server_error(message: &str) -> Option<SqlExecutionErrorLocation> {
    // `Line n`、`State 1`、`Message ...`。
    capture_number_after(message, "Line ").map(|line| SqlExecutionErrorLocation {
        line: Some(line),
        column: None,
        byte_offset: None,
    })
}

fn parse_oracle_error(message: &str) -> Option<SqlExecutionErrorLocation> {
    // `ORA-06550: line n, column m`。
    let line = capture_number_after_nocase(message, "line ")?;
    let rest = after_text_nocase(message, "line ")?;
    let column = capture_number_after_nocase(rest, "column ")?;
    Some(SqlExecutionErrorLocation {
        line: Some(line),
        column: Some(column),
        byte_offset: None,
    })
}

fn parse_sqlite_error(_message: &str) -> Option<SqlExecutionErrorLocation> {
    // SQLite 报错通常只给 `near "x"`；无行列号，返回 None。
    None
}

fn parse_generic(message: &str) -> Option<SqlExecutionErrorLocation> {
    // `(line n, column m)` 结构形式（IPC driver 结构化错误）。
    let line = capture_number_after_nocase(message, "(line ")?;
    let rest = after_text_nocase(message, "(line ")?;
    let column = capture_number_after_nocase(rest, "column ");
    Some(SqlExecutionErrorLocation {
        line: Some(line),
        column,
        byte_offset: None,
    })
}

/// 提取 `标记` 之后紧跟的第一个正整数。
fn capture_number_after(haystack: &str, marker: &str) -> Option<u32> {
    let index = haystack.find(marker)?;
    let rest = &haystack[index + marker.len()..];
    read_unsigned(rest)
}

fn capture_number_after_nocase(haystack: &str, marker: &str) -> Option<u32> {
    let lower = haystack.to_ascii_lowercase();
    let marker_lower = marker.to_ascii_lowercase();
    let index = lower.find(&marker_lower)?;
    let rest = &haystack[index + marker.len()..];
    read_unsigned(rest)
}

fn read_unsigned(text: &str) -> Option<u32> {
    let digit_start = text.chars().position(|ch| ch.is_ascii_digit())?;
    let digits = text[digit_start..]
        .chars()
        .take_while(|ch| ch.is_ascii_digit())
        .collect::<String>();
    if digits.is_empty() {
        return None;
    }
    digits.parse().ok()
}

fn after_text_nocase<'a>(haystack: &'a str, marker: &str) -> Option<&'a str> {
    let lower = haystack.to_ascii_lowercase();
    let marker_lower = marker.to_ascii_lowercase();
    let index = lower.find(&marker_lower)?;
    Some(&haystack[index + marker.len()..])
}

/// 把执行错误位置映射回文档中的字节范围。
///
/// 规则：
/// - 驱动 line/column 为 1-based
/// - 先在 executed SQL 内转换为 UTF-8 byte offset
/// - 再叠加 source range 的起始字节
/// - 结果必须落在 source range 内
/// - document revision 必须与 source 相同
/// - sql_fingerprint 必须匹配（防止用旧语句的位置）
pub fn map_execution_error_range(
    source: &SqlExecutionResultSource,
    executed_sql: &str,
    location: SqlExecutionErrorLocation,
    document: &str,
    document_revision: u64,
) -> Option<SqlTextRange> {
    if source.document_revision != document_revision {
        return None;
    }
    let Some(source_range) = source.source_range else {
        return None;
    };

    // fingerprint 校验：driver 位置的 SQL 必须是本次执行的 SQL。
    let fingerprint = super::execution::sql_fingerprint(executed_sql);
    if source.sql_fingerprint != fingerprint {
        return None;
    }

    let local_offset = location_offset(executed_sql, location)?;

    // 叠加 base offset。
    let byte = source_range.start_byte.saturating_add(local_offset);
    if byte < source_range.start_byte || byte > source_range.end_byte {
        return None;
    }
    if byte >= document.len() {
        return None;
    }

    // 截取一个 token 的宽度（到下一个空白 / 行尾），保证 range 非空。
    let end = range_end(document, byte, source_range.end_byte);
    if end <= byte {
        return None;
    }

    Some(SqlTextRange {
        start_byte: byte,
        end_byte: end,
    })
}

/// 在 executed SQL 内把 line/column 或 byte_offset 转成 UTF-8 byte offset。
fn location_offset(sql: &str, location: SqlExecutionErrorLocation) -> Option<usize> {
    if let Some(offset) = location.byte_offset {
        return Some(clamp_utf8(sql, offset.min(sql.len())));
    }
    let line = location.line?;
    // 1-based line -> 第 (line-1) 行起点。
    let mut line_index = line.saturating_sub(1) as usize;
    let mut offset = 0usize;
    for chunk in sql.split_inclusive('\n') {
        if line_index == 0 {
            break;
        }
        offset += chunk.len();
        line_index -= 1;
    }
    if line_index > 0 {
        // 行号超出 SQL 行数。
        return None;
    }
    let column = location.column.unwrap_or(1).saturating_sub(1) as usize;
    // 列号是字符数还是字节数未知；按字节尽量向前推进并收敛到 UTF-8 边界。
    let target = offset.saturating_add(column).min(sql.len());
    Some(clamp_utf8(sql, target))
}

fn clamp_utf8(text: &str, mut offset: usize) -> usize {
    offset = offset.min(text.len());
    while offset < text.len() && !text.is_char_boundary(offset) {
        offset += 1;
    }
    offset
}

/// 从 byte 位置向后取一个 range 终点（到行尾或 source range 上限）。
fn range_end(document: &str, start: usize, upper: usize) -> usize {
    let line_end = document[start..]
        .find('\n')
        .map(|index| start + index)
        .unwrap_or(document.len());
    line_end.min(upper)
}
