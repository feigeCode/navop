//! 剪贴板 TSV 编解码（issue #355）。
//!
//! 可编辑表格与查询结果共用同一套剪贴板格式：`\t` 分列、`\n` 分行。但单元格的值
//! 本身可能含 `\t`、`\n`、`\r` 或 `"`，原样写进剪贴板后，粘贴时会被当成分隔符
//! 切走——现象正是「剪贴板里的数据是全的，粘进去少了几个字段」。
//!
//! 所以编码时按 RFC 4180 的做法转义：字段含上述字符就用 `"` 包裹、内部 `"` 双写；
//! 解码对称还原。只有整段文本含 `\t`、换行或 `""` 时才按引号规则解转义，这样从
//! 别处复制一段单值（JSON 片段、带反斜杠的路径）粘进单元格仍然是原样写入。
//!
//! 分隔与换行沿用旧行为：`\r\n` 算一个换行，末尾换行不多产生一个空行。
//!
//! 已知遗留：值恰好等于字面量 `\N` 时，与 SQL NULL 的剪贴板标记仍无法区分
//! （复制侧把 NULL 写成 `\N`，粘贴侧没有 NULL 语义）。

/// 触发加引号转义的字符：列分隔、行分隔与引号本身。
const QUOTE_TRIGGERS: [char; 4] = ['\t', '\n', '\r', '"'];

/// 编码单个字段：含 `\t`、`\n`、`\r` 或 `"` 时加引号并把内部 `"` 双写。
pub fn escape_tsv_field(field: &str) -> String {
    if field.contains(QUOTE_TRIGGERS) {
        format!("\"{}\"", field.replace('"', "\"\""))
    } else {
        field.to_string()
    }
}

/// 编码一行字段（不含行尾换行）。
pub fn encode_tsv_row<S: AsRef<str>>(fields: impl IntoIterator<Item = S>) -> String {
    fields
        .into_iter()
        .map(|field| escape_tsv_field(field.as_ref()))
        .collect::<Vec<_>>()
        .join("\t")
}

/// 编码多行字段，行与行用 `\n` 连接——这就是写进剪贴板的内容。
pub fn encode_tsv_rows<S: AsRef<str>>(
    rows: impl IntoIterator<Item = impl IntoIterator<Item = S>>,
) -> String {
    rows.into_iter()
        .map(encode_tsv_row)
        .collect::<Vec<_>>()
        .join("\n")
}

/// 解码剪贴板文本，返回「行 → 字段」的二维值。
///
/// 空文本返回空列表（调用方按「没粘到东西」处理）。不含分隔符的单值原样返回，
/// 不会被当成引号包裹的字段而抹掉引号。
pub fn parse_tsv_rows(text: &str) -> Vec<Vec<String>> {
    if text.is_empty() {
        return Vec::new();
    }
    if !needs_quote_rules(text) {
        return vec![vec![text.to_string()]];
    }
    TsvParser::new(text).run()
}

/// 整段文本含列分隔、换行或 `""` 转义标记时，才需要按引号规则解转义。
fn needs_quote_rules(text: &str) -> bool {
    text.contains(['\t', '\n', '\r']) || text.contains("\"\"")
}

/// 逐字符解码：引号外的 `\t`/`\n` 是分隔符，引号内的原样保留。
struct TsvParser<'a> {
    chars: std::iter::Peekable<std::str::Chars<'a>>,
    ends_with_newline: bool,
    rows: Vec<Vec<String>>,
    row: Vec<String>,
    field: String,
    in_quotes: bool,
    at_field_start: bool,
}

impl<'a> TsvParser<'a> {
    fn new(text: &'a str) -> Self {
        Self {
            chars: text.chars().peekable(),
            ends_with_newline: text.ends_with('\n'),
            rows: Vec::new(),
            row: Vec::new(),
            field: String::new(),
            in_quotes: false,
            at_field_start: true,
        }
    }

    fn run(mut self) -> Vec<Vec<String>> {
        while let Some(ch) = self.chars.next() {
            self.push(ch);
        }
        self.finish()
    }

    fn push(&mut self, ch: char) {
        if self.in_quotes {
            self.push_quoted(ch);
            return;
        }
        match ch {
            // 只有字段开头的引号才开启引用，其余位置的 `"` 是普通字符。
            '"' if self.at_field_start => {
                self.in_quotes = true;
                self.at_field_start = false;
            }
            '\t' => self.end_field(),
            '\n' => self.end_row(),
            _ => {
                self.field.push(ch);
                self.at_field_start = false;
            }
        }
    }

    /// 引号内：`""` 还原成一个 `"`，其余字符（含 `\t`/`\n`）原样保留。
    fn push_quoted(&mut self, ch: char) {
        match ch {
            '"' if self.chars.peek() == Some(&'"') => {
                self.chars.next();
                self.field.push('"');
            }
            '"' => self.in_quotes = false,
            _ => self.field.push(ch),
        }
    }

    fn end_field(&mut self) {
        self.row.push(std::mem::take(&mut self.field));
        self.at_field_start = true;
    }

    fn end_row(&mut self) {
        // `\r\n` 算一个换行：行尾的 `\r` 丢掉，与 `str::lines()` 一致。
        if self.field.ends_with('\r') {
            self.field.pop();
        }
        self.end_field();
        self.rows.push(std::mem::take(&mut self.row));
    }

    fn finish(mut self) -> Vec<Vec<String>> {
        // 末尾没有换行时收尾最后一行；文本以换行结尾就不再补一个空行，
        // 否则「复制一行」粘回来会多出一行空数据。
        let has_trailing_line = self.in_quotes
            || !self.field.is_empty()
            || !self.row.is_empty()
            || !self.ends_with_newline;
        if has_trailing_line {
            self.row.push(std::mem::take(&mut self.field));
            self.rows.push(std::mem::take(&mut self.row));
        }
        self.rows
    }
}

#[cfg(test)]
#[path = "tsv_tests.rs"]
mod tests;
