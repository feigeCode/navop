//! Signature help 纯算法
//!
//! 给定 SQL 文本与光标偏移，计算光标所在的函数调用及其 active parameter 序号，
//! 供 signature help popover 使用。
//!
//! 规则：
//! - 定位光标所在的函数调用（含嵌套函数）
//! - 字符串 / 注释内逗号不计参数
//! - 正确计算 active parameter（光标所在参数序号）
//! - 支持 overload（同名多签名）
//! - 光标离开参数列表（`)` 之后）返回 None

use super::sql_tokenizer::{SqlToken, SqlTokenKind, SqlTokenizer};

/// 例程签名。
#[derive(Clone, Debug, PartialEq)]
pub struct SqlRoutineSignature {
    pub identity: String,
    pub label: String,
    pub parameters: Vec<String>,
    pub return_type: Option<String>,
    pub documentation: Option<String>,
}

/// Signature help 结果。
#[derive(Clone, Debug, PartialEq)]
pub struct SqlSignatureHelp {
    /// 函数名调用区间（`name(` 中的 name 起，到调用右括号止，若无则到光标）。
    pub call_range: std::ops::Range<usize>,
    /// active parameter 序号（0-based）。
    pub active_parameter: usize,
    /// 候选签名（同名的所有 overload）。
    pub signatures: Vec<SqlRoutineSignature>,
}

/// 在当前 SQL 文本中计算 signature help。
///
/// `cursor_byte` 为光标字节偏移（应在函数调用参数列表内或紧邻其后）。
/// `routines` 为候选签名列表（按名字匹配，大小写不敏感）。
pub fn signature_help(
    sql: &str,
    cursor_byte: usize,
    routines: &[SqlRoutineSignature],
) -> Option<SqlSignatureHelp> {
    let cursor = cursor_byte.min(sql.len());
    let tokens = SqlTokenizer::new(sql).tokenize();

    // 找到光标位置所属的最近一个未闭合的 `(`（即正在输入参数的调用）。
    let Some(call) = find_active_call(&tokens, cursor) else {
        return None;
    };

    // 函数名 = 该 LParen 之前最近的 Ident / QuotedIdent token。
    let name_token = tokens[..call.open_index]
        .iter()
        .rev()
        .find(|token| matches!(token.kind, SqlTokenKind::Ident | SqlTokenKind::QuotedIdent))?;

    let function_name = unquote_identifier(&name_token.text);
    let matching: Vec<SqlRoutineSignature> = routines
        .iter()
        .filter(|routine| routine.identity.eq_ignore_ascii_case(&function_name))
        .cloned()
        .collect();
    if matching.is_empty() {
        return None;
    }

    let active_parameter = count_active_parameter(&tokens, call.open_index, cursor);

    let call_start = name_token.start;
    let call_end = tokens[call.open_index..]
        .iter()
        .find(|token| matches!(token.kind, SqlTokenKind::RParen))
        .map(|token| token.end)
        .unwrap_or(cursor);

    Some(SqlSignatureHelp {
        call_range: call_start..call_end,
        active_parameter,
        signatures: matching,
    })
}

/// 找到光标处正在输入参数的调用。
///
/// 返回最内层（深度最大）的、未被右括号闭合的 LParen。若光标在 `)` 之后，
/// 表示参数已输入完毕，返回 None。
struct ActiveCall {
    open_index: usize,
}

fn find_active_call(tokens: &[SqlToken], cursor: usize) -> Option<ActiveCall> {
    // depth_stack[index] = 该深度最近一个 LParen 的 token 下标。
    let mut depth_stack: Vec<usize> = Vec::new();

    for (index, token) in tokens.iter().enumerate() {
        if token.start >= cursor {
            break;
        }
        match token.kind {
            SqlTokenKind::LParen => depth_stack.push(index),
            SqlTokenKind::RParen => {
                depth_stack.pop();
            }
            _ => {}
        }
    }

    // 光标前未闭合的最内层调用（深度最大）。`(` 前必须有函数名才算调用。
    let &open_index = depth_stack.last()?;
    let has_function_name = tokens[..open_index]
        .iter()
        .rev()
        .any(|token| matches!(token.kind, SqlTokenKind::Ident | SqlTokenKind::QuotedIdent));

    if !has_function_name {
        return None;
    }

    // 验证光标确实还在该调用参数列表内（未越过对应右括号）。
    let mut check_depth = 0usize;
    for token in &tokens[open_index..] {
        match token.kind {
            SqlTokenKind::LParen => check_depth += 1,
            SqlTokenKind::RParen => {
                check_depth = check_depth.saturating_sub(1);
                if check_depth == 0 {
                    if token.end <= cursor {
                        return None;
                    }
                    break;
                }
            }
            _ => {}
        }
    }

    Some(ActiveCall { open_index })
}

/// 计算 active parameter：调用 LParen 之后、光标之前、位于该调用深度层的
/// 逗号数量。
fn count_active_parameter(tokens: &[SqlToken], open_index: usize, cursor: usize) -> usize {
    let mut depth = 0usize;
    let mut commas = 0usize;
    for token in &tokens[open_index + 1..] {
        if token.start >= cursor {
            break;
        }
        match token.kind {
            SqlTokenKind::LParen => depth += 1,
            SqlTokenKind::RParen => {
                if depth > 0 {
                    depth -= 1;
                }
            }
            SqlTokenKind::Comma if depth == 0 => commas += 1,
            _ => {}
        }
    }
    commas
}

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