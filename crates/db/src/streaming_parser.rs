use crate::executor::SqlSource;
use one_core::storage::DatabaseType;
use std::collections::VecDeque;
use std::fs::File;
use std::io::{self, BufRead, BufReader, Cursor, Read};
use std::path::PathBuf;

/// SQL Server `GO n` 的最大重复次数，防止错误脚本产生近乎无限的执行任务。
const MAX_MSSQL_GO_REPEAT: usize = 1000;

/// 统一的 SQL 读取器，支持字符串和文件两种来源
enum SqlReader {
    Memory(Cursor<Vec<u8>>),
    File(BufReader<File>),
}

/// 返回去掉单个行尾后的“前缀 + 最后一行”，用于识别必须独占一行的客户端指令。
fn split_last_line(buffer: &str) -> Option<(&str, &str)> {
    let without_newline = buffer
        .strip_suffix("\r\n")
        .or_else(|| buffer.strip_suffix('\n'))
        .or_else(|| buffer.strip_suffix('\r'))
        .unwrap_or(buffer);
    if let Some((prefix, line)) = without_newline.rsplit_once('\n') {
        return Some((prefix, line.trim_end_matches('\r')));
    }
    Some(("", without_newline.trim_end_matches('\r')))
}

/// 判断文本是否以完整关键字开头，避免把 `BEGINNER` 误识别为 `BEGIN`。
///
/// 参数 `text` 应为待检查的 SQL 前缀，`keyword` 为不区分大小写的关键字；
/// 返回值仅表示前缀匹配，不会跳过 SQL 注释或字符串内容。
fn starts_with_keyword(text: &str, keyword: &str) -> bool {
    text.get(..keyword.len())
        .is_some_and(|prefix| prefix.eq_ignore_ascii_case(keyword))
        && text
            .get(keyword.len()..)
            .and_then(|rest| rest.chars().next())
            .map_or(true, char::is_whitespace)
}

/// 判断指定数据库是否支持标准允许的嵌套块注释语法。
///
/// PostgreSQL 和 SQL Server 的块注释可以嵌套；其他数据库仍按单层注释处理，
/// 这样不会把其方言中普通文本误扩展为额外的注释层级。
fn supports_nested_block_comments(db_type: &DatabaseType) -> bool {
    matches!(db_type, DatabaseType::PostgreSQL | DatabaseType::MSSQL)
}

/// 将 Oracle `q'...'` 引用的起始分隔符映射为对应结束分隔符。
///
/// 方括号、圆括号、大括号和尖括号使用成对分隔符，其他字符按 Oracle 规则
/// 使用自身作为结束分隔符。
fn matching_oracle_quote_delimiter(opening: char) -> char {
    match opening {
        '[' => ']',
        '(' => ')',
        '{' => '}',
        '<' => '>',
        other => other,
    }
}

/// 判断缓冲区末尾是否刚好形成 Oracle 替代引用的 `q`/`Q` 前缀。
///
/// 只有前一个字符不是标识符字符时才算前缀，避免把普通标识符中的字母 `q`
/// 误当作替代引用起始标记。
fn has_oracle_q_prefix(buffer: &str) -> bool {
    let mut chars = buffer.chars().rev();
    let Some(prefix) = chars.next() else {
        return false;
    };
    (prefix == 'q' || prefix == 'Q')
        && chars.next().map_or(true, |previous| {
            !(previous == '_' || previous.is_alphanumeric())
        })
}

/// 判断字符串起始引号前是否存在 PostgreSQL 的 `E`/`e` 转义字符串前缀。
///
/// 该判断只控制反斜杠转义，不改变原始 SQL 文本，也不对其他数据库启用该规则。
fn has_postgresql_escape_prefix(buffer: &str) -> bool {
    let mut chars = buffer.chars().rev();
    let Some(prefix) = chars.next() else {
        return false;
    };
    (prefix == 'e' || prefix == 'E')
        && chars.next().map_or(true, |previous| {
            !(previous == '_' || previous.is_alphanumeric())
        })
}

/// 去除 Oracle 程序块识别所需的前导空白和注释，保留 SQL 原文不变。
///
/// 该函数仅用于判断语句类型；未闭合块注释不会被强行跳过，以免将残缺 SQL
/// 错误判断为可由独占行 `/` 结束的 PL/SQL 块。
fn strip_leading_oracle_comments(mut sql: &str) -> &str {
    loop {
        sql = sql.trim_start();
        if let Some(rest) = sql.strip_prefix("--") {
            sql = rest.split_once('\n').map(|(_, tail)| tail).unwrap_or("");
            continue;
        }
        if sql.starts_with("/*") {
            if let Some((_, tail)) = sql.split_once("*/") {
                sql = tail;
                continue;
            }
        }
        return sql;
    }
}

/// Oracle 程序块只能由独占行 `/` 结束，内部和结尾分号都属于 PL/SQL 文本。
fn is_oracle_plsql_block(sql: &str) -> bool {
    let upper = strip_leading_oracle_comments(sql).to_ascii_uppercase();
    let normalized = upper.trim_start();
    if starts_with_keyword(normalized, "BEGIN") || starts_with_keyword(normalized, "DECLARE") {
        return true;
    }
    let mut words = normalized.split_whitespace();
    if words.next() != Some("CREATE") {
        return false;
    }
    let mut word = words.next().unwrap_or_default();
    if word == "OR" {
        if !matches!(words.next(), Some("REPLACE" | "ALTER")) {
            return false;
        }
        word = words.next().unwrap_or_default();
    }
    while matches!(word, "EDITIONABLE" | "NONEDITIONABLE") {
        word = words.next().unwrap_or_default();
    }
    matches!(
        word,
        "PROCEDURE" | "FUNCTION" | "TRIGGER" | "PACKAGE" | "TYPE"
    )
}

impl Read for SqlReader {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        match self {
            SqlReader::Memory(cursor) => cursor.read(buf),
            SqlReader::File(reader) => reader.read(buf),
        }
    }
}

impl BufRead for SqlReader {
    fn fill_buf(&mut self) -> io::Result<&[u8]> {
        match self {
            SqlReader::Memory(cursor) => cursor.fill_buf(),
            SqlReader::File(reader) => reader.fill_buf(),
        }
    }

    fn consume(&mut self, amt: usize) {
        match self {
            SqlReader::Memory(cursor) => cursor.consume(amt),
            SqlReader::File(reader) => reader.consume(amt),
        }
    }
}

/// 流式 SQL 解析器
/// 从 BufRead 流中按需读取并解析 SQL 语句，避免一次性加载整个文件
pub struct StreamingSqlParser {
    reader: SqlReader,
    db_type: DatabaseType,
    buffer: String,
    bytes_read: u64,
    total_size: u64,

    in_string: bool,
    string_char: char,
    string_backslash_escape: bool,
    escape_next: bool,
    prev_was_string_char: bool,
    oracle_alt_quote_waiting_delimiter: bool,
    oracle_alt_quote_closing: Option<char>,
    oracle_alt_quote_seen_closing: bool,
    in_line_comment: bool,
    in_block_comment: bool,
    block_comment_depth: usize,
    dollar_quote: Option<String>,

    paren_depth: i32,
    begin_depth: i32,
    pending_end_word: bool,
    last_checked_len: usize,
    delimiter: String,

    /// 上次返回语句时尚未消费的字符，必须逐个弹出，禁止整体 drain 后丢失尾部。
    pending_chars: VecDeque<char>,
    pending_repeated_statement: Option<(String, usize)>,
    eof: bool,
    terminated: bool,
    at_start: bool,
}

impl StreamingSqlParser {
    /// 从 SqlSource 创建解析器
    pub fn from_source(source: SqlSource, db_type: DatabaseType) -> io::Result<Self> {
        let (reader, total_size) = match source {
            SqlSource::Script(script) => {
                let size = script.len() as u64;
                (SqlReader::Memory(Cursor::new(script.into_bytes())), size)
            }
            SqlSource::File(path) => {
                let file = File::open(&path)?;
                let size = file.metadata()?.len();
                (SqlReader::File(BufReader::new(file)), size)
            }
        };

        Ok(Self {
            reader,
            db_type,
            buffer: String::new(),
            bytes_read: 0,
            total_size,
            in_string: false,
            string_char: '\0',
            string_backslash_escape: false,
            escape_next: false,
            prev_was_string_char: false,
            oracle_alt_quote_waiting_delimiter: false,
            oracle_alt_quote_closing: None,
            oracle_alt_quote_seen_closing: false,
            in_line_comment: false,
            in_block_comment: false,
            block_comment_depth: 0,
            dollar_quote: None,
            paren_depth: 0,
            begin_depth: 0,
            pending_end_word: false,
            last_checked_len: 0,
            delimiter: ";".to_string(),
            pending_chars: VecDeque::new(),
            pending_repeated_statement: None,
            eof: false,
            terminated: false,
            at_start: true,
        })
    }

    /// 从文件路径创建解析器
    pub fn from_file(path: PathBuf, db_type: DatabaseType) -> io::Result<Self> {
        Self::from_source(SqlSource::File(path), db_type)
    }

    /// 从脚本字符串创建解析器
    pub fn from_script(script: String, db_type: DatabaseType) -> io::Result<Self> {
        Self::from_source(SqlSource::Script(script), db_type)
    }

    pub fn bytes_read(&self) -> u64 {
        self.bytes_read
    }

    pub fn total_size(&self) -> u64 {
        self.total_size
    }

    /// 进度百分比
    pub fn progress_percent(&self) -> f32 {
        if self.total_size > 0 {
            (self.bytes_read as f64 / self.total_size as f64 * 100.0).clamp(0.0, 100.0) as f32
        } else {
            0.0
        }
    }

    fn read_next_statement(&mut self) -> io::Result<Option<String>> {
        if self.terminated {
            return Ok(None);
        }
        if let Some(statement) = self.take_pending_statement() {
            return Ok(Some(statement));
        }
        if self.eof && self.buffer.is_empty() && self.pending_chars.is_empty() {
            return Ok(None);
        }

        let mut line_buf = String::new();

        loop {
            // 逐个消费待处理字符，遇到边界时队列中的剩余字符会保留到下一次调用。
            while let Some(ch) = self.pending_chars.pop_front() {
                if let Some(stmt) = self.process_char(ch)? {
                    return Ok(Some(stmt));
                }
            }

            // Then read new line if not EOF
            if !self.eof {
                line_buf.clear();
                match self.reader.read_line(&mut line_buf) {
                    Ok(0) => {
                        self.eof = true;
                    }
                    Ok(n) => {
                        self.bytes_read += n as u64;
                    }
                    Err(e) => return Err(e),
                }
            }

            if !line_buf.is_empty() {
                let mut chars = line_buf.chars();
                while let Some(ch) = chars.next() {
                    if let Some(stmt) = self.process_char(ch)? {
                        self.pending_chars.extend(chars);
                        return Ok(Some(stmt));
                    }
                }
            }

            if self.eof {
                if let Some(statement) = self.take_mssql_go_batch()? {
                    return Ok(statement.or_else(|| self.take_pending_statement()));
                }
                if let Some(statement) = self.take_oracle_slash_block() {
                    return Ok(statement);
                }
                self.finish_terminal_quote_state();
                self.validate_eof_state()?;
                let trimmed = self.buffer.trim();
                if let Some(stmt) = self.finalize_statement(trimmed) {
                    self.buffer.clear();
                    self.last_checked_len = 0;
                    self.pending_end_word = false;
                    self.terminated = true;
                    return Ok(Some(stmt));
                }
                self.buffer.clear();
                self.last_checked_len = 0;
                self.pending_end_word = false;
                self.terminated = true;
                return Ok(None);
            }
        }
    }

    fn take_pending_statement(&mut self) -> Option<String> {
        let (statement, remaining) = self.pending_repeated_statement.take()?;
        if remaining > 1 {
            self.pending_repeated_statement = Some((statement.clone(), remaining - 1));
        }
        Some(statement)
    }

    /// 从当前缓冲区消费末尾独占行 `GO [count]`，并按指定次数生成 SQL Server 批次。
    ///
    /// 外层 `Option` 表示是否识别到合法的 GO 行，内层 `Option` 表示 GO 前是否存在
    /// 可执行批次；非法次数和多余参数返回 `InvalidData`，避免无界任务或静默误解析。
    fn take_mssql_go_batch(&mut self) -> io::Result<Option<Option<String>>> {
        if self.db_type != DatabaseType::MSSQL
            || self.in_string
            || self.in_line_comment
            || self.in_block_comment
            || self.dollar_quote.is_some()
        {
            return Ok(None);
        }

        let Some((batch, last_line)) = split_last_line(&self.buffer) else {
            return Ok(None);
        };
        let Some(repeat_count) = parse_go_repeat_count(last_line)? else {
            return Ok(None);
        };
        let statement = batch.trim().to_string();

        self.buffer.clear();
        self.last_checked_len = 0;
        self.pending_end_word = false;
        if statement.is_empty() {
            return Ok(Some(None));
        }
        if repeat_count > 1 {
            self.pending_repeated_statement = Some((statement.clone(), repeat_count - 1));
        }
        Ok(Some(Some(statement)))
    }

    /// 识别 Oracle 客户端独占行 `/`；该行只终止前面的 PL/SQL 单元，不发送给 JDBC。
    fn take_oracle_slash_block(&mut self) -> Option<Option<String>> {
        if self.db_type != DatabaseType::Oracle
            || self.in_string
            || self.in_line_comment
            || self.in_block_comment
            || self.dollar_quote.is_some()
        {
            return None;
        }
        let (block, last_line) = split_last_line(&self.buffer)?;
        if last_line.trim() != "/" {
            return None;
        }
        let statement = block.trim().to_string();
        self.buffer.clear();
        self.last_checked_len = 0;
        self.paren_depth = 0;
        self.begin_depth = 0;
        self.pending_end_word = false;
        Some((!statement.is_empty()).then_some(statement))
    }

    /// EOF 可以正常结束行注释以及已经读取到右引号的延迟关闭状态。
    fn finish_terminal_quote_state(&mut self) {
        if self.in_string && self.prev_was_string_char {
            self.in_string = false;
            self.prev_was_string_char = false;
            self.string_backslash_escape = false;
        }
        self.in_line_comment = false;
    }

    /// EOF 时拒绝未闭合的词法和结构状态，避免执行残缺 SQL。
    fn validate_eof_state(&mut self) -> io::Result<()> {
        let error = if self.in_string {
            Some("SQL 文件结束时字符串或引用标识符未闭合")
        } else if self.oracle_alt_quote_waiting_delimiter || self.oracle_alt_quote_closing.is_some()
        {
            Some("SQL 文件结束时 Oracle q 引用未闭合")
        } else if self.in_block_comment {
            Some("SQL 文件结束时块注释未闭合")
        } else if self.dollar_quote.is_some() {
            Some("SQL 文件结束时 PostgreSQL 美元引用未闭合")
        } else if self.paren_depth != 0 {
            Some("SQL 文件结束时圆括号未闭合")
        } else if self.db_type == DatabaseType::Oracle
            && is_oracle_plsql_block(&self.buffer)
            && !self.buffer.trim().is_empty()
        {
            Some("Oracle/PLSQL 块缺少独占行 / 终止符")
        } else if self.begin_depth != 0 {
            Some("SQL 文件结束时 BEGIN/END 块未闭合")
        } else {
            None
        };
        if let Some(message) = error {
            self.terminated = true;
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("{message}，已读取字节数: {}", self.bytes_read),
            ));
        }
        Ok(())
    }

    /// 消费单个 Unicode 字符并推进词法与语句边界状态。
    ///
    /// 返回 `Some` 表示形成一个完整语句；未闭合或不匹配的结构返回 `InvalidData`。
    /// 该方法会修改内部缓冲区、注释/引号状态和块深度，但不会执行 SQL。
    fn process_char(&mut self, ch: char) -> io::Result<Option<String>> {
        if self.at_start {
            self.at_start = false;
            if ch == '\u{feff}' {
                return Ok(None);
            }
        }

        if self.oracle_alt_quote_waiting_delimiter {
            self.buffer.push(ch);
            self.oracle_alt_quote_waiting_delimiter = false;
            self.oracle_alt_quote_closing = Some(matching_oracle_quote_delimiter(ch));
            return Ok(None);
        }
        if let Some(closing) = self.oracle_alt_quote_closing {
            self.buffer.push(ch);
            if self.oracle_alt_quote_seen_closing && ch == '\'' {
                self.oracle_alt_quote_seen_closing = false;
                self.oracle_alt_quote_closing = None;
            } else {
                self.oracle_alt_quote_seen_closing = ch == closing;
            }
            return Ok(None);
        }

        if self.in_line_comment {
            self.buffer.push(ch);
            if ch == '\n' {
                self.in_line_comment = false;
            }
            return Ok(None);
        }

        if self.in_block_comment {
            let previous = self.buffer.chars().next_back();
            self.buffer.push(ch);
            if supports_nested_block_comments(&self.db_type) && previous == Some('/') && ch == '*' {
                self.block_comment_depth += 1;
            } else if previous == Some('*') && ch == '/' {
                self.block_comment_depth = self.block_comment_depth.saturating_sub(1);
                if self.block_comment_depth == 0 {
                    self.in_block_comment = false;
                }
            }
            return Ok(None);
        }

        if let Some(ref tag) = self.dollar_quote.clone() {
            self.buffer.push(ch);
            if ch == '$' {
                if self.buffer.ends_with(tag.as_str()) {
                    self.dollar_quote = None;
                }
            }
            return Ok(None);
        }

        if self.in_string {
            if self.escape_next {
                self.buffer.push(ch);
                self.escape_next = false;
                self.prev_was_string_char = false;
                return Ok(None);
            }

            if ch == '\\' && self.string_backslash_escape {
                self.buffer.push(ch);
                self.escape_next = true;
                self.prev_was_string_char = false;
                return Ok(None);
            }

            if ch == self.string_char {
                self.buffer.push(ch);
                if self.prev_was_string_char {
                    // This is '' escape - two quotes represent one escaped quote
                    self.prev_was_string_char = false;
                } else {
                    // Might be end of string or start of '' escape
                    self.prev_was_string_char = true;
                }
                return Ok(None);
            }

            // Non-quote, non-escape character
            if self.prev_was_string_char {
                // Previous quote was end of string, process this char normally
                self.in_string = false;
                self.prev_was_string_char = false;
                self.string_backslash_escape = false;
                // Fall through to normal character processing
            } else {
                self.buffer.push(ch);
                return Ok(None);
            }
        }

        if self.pending_end_word && !ch.is_whitespace() {
            if ch == ';' || self.delimiter.starts_with(ch) {
                self.begin_depth = (self.begin_depth - 1).max(0);
            }
            self.pending_end_word = false;
        }

        if self.should_start_line_comment(ch) {
            self.buffer.push(ch);
            self.in_line_comment = true;
            return Ok(None);
        }

        if ch == '-' {
            self.buffer.push(ch);
            return Ok(None);
        }

        if ch == '#' && self.db_type == DatabaseType::MySQL {
            self.buffer.push(ch);
            self.in_line_comment = true;
            return Ok(None);
        }

        if ch == '*' && self.buffer.ends_with('/') {
            self.buffer.push(ch);
            self.in_block_comment = true;
            self.block_comment_depth = 1;
            return Ok(None);
        }

        if ch == '$' && self.db_type == DatabaseType::PostgreSQL {
            self.buffer.push(ch);
            if let Some(tag) = self.try_extract_dollar_quote() {
                self.dollar_quote = Some(tag);
            }
            return Ok(None);
        }

        if ch == '\'' && self.db_type == DatabaseType::Oracle && has_oracle_q_prefix(&self.buffer) {
            self.buffer.push(ch);
            self.oracle_alt_quote_waiting_delimiter = true;
            return Ok(None);
        }
        if ch == '\'' || ch == '"' {
            self.in_string = true;
            self.string_char = ch;
            self.string_backslash_escape = self.db_type == DatabaseType::MySQL
                || (self.db_type == DatabaseType::PostgreSQL
                    && ch == '\''
                    && has_postgresql_escape_prefix(&self.buffer));
            self.buffer.push(ch);
            return Ok(None);
        }

        if ch == '`' && self.db_type == DatabaseType::MySQL {
            self.in_string = true;
            self.string_char = ch;
            self.string_backslash_escape = true;
            self.buffer.push(ch);
            return Ok(None);
        }

        if ch == '[' && self.db_type == DatabaseType::MSSQL {
            self.in_string = true;
            self.string_char = ']';
            self.string_backslash_escape = false;
            self.buffer.push(ch);
            return Ok(None);
        }

        if ch == '(' {
            self.paren_depth += 1;
            self.buffer.push(ch);
            return Ok(None);
        }

        if ch == ')' {
            if self.paren_depth == 0 {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "SQL 出现未匹配的右圆括号",
                ));
            }
            self.paren_depth -= 1;
            self.buffer.push(ch);
            return Ok(None);
        }

        self.buffer.push(ch);

        if ch.is_whitespace() || ch == ';' || ch == '$' {
            self.update_begin_depth();
            if self.pending_end_word && (ch == ';' || self.delimiter.starts_with(ch)) {
                self.begin_depth = (self.begin_depth - 1).max(0);
                self.pending_end_word = false;
            }
        }

        if self.db_type == DatabaseType::MySQL && ch == '\n' {
            if let Some(new_delim) = self.try_parse_delimiter() {
                self.delimiter = new_delim;
                let prefix = split_last_line(&self.buffer)
                    .map(|(prefix, _)| prefix)
                    .unwrap_or_default();
                self.buffer = prefix.to_string();
                self.last_checked_len = 0;
                return Ok(None);
            }
        }

        if self.db_type == DatabaseType::MSSQL && ch == '\n' {
            if let Some(statement) = self.take_mssql_go_batch()? {
                return Ok(statement);
            }
        }

        if self.db_type == DatabaseType::Oracle && ch == '\n' {
            if let Some(statement) = self.take_oracle_slash_block() {
                return Ok(statement);
            }
        }

        if self.paren_depth == 0 && self.begin_depth == 0 {
            let trimmed_current = self.buffer.trim_end();
            if self.db_type != DatabaseType::MSSQL
                && !(self.db_type == DatabaseType::Oracle && is_oracle_plsql_block(trimmed_current))
                && trimmed_current.ends_with(&self.delimiter)
            {
                let stmt = trimmed_current
                    .strip_suffix(&self.delimiter)
                    .unwrap_or(trimmed_current)
                    .trim();

                if let Some(result) = self.finalize_statement(stmt) {
                    self.buffer.clear();
                    self.last_checked_len = 0;
                    self.pending_end_word = false;
                    return Ok(Some(result));
                }
                self.buffer.clear();
                self.last_checked_len = 0;
                self.pending_end_word = false;
            }
        }

        Ok(None)
    }

    fn should_start_line_comment(&self, ch: char) -> bool {
        match self.db_type {
            DatabaseType::MySQL => {
                self.buffer.ends_with("--") && (ch.is_whitespace() || ch.is_control())
            }
            _ => ch == '-' && self.buffer.ends_with('-'),
        }
    }

    fn finalize_statement(&self, stmt: &str) -> Option<String> {
        let normalized = self.strip_leading_ignorable_lines(stmt).trim();
        let normalized = normalized
            .strip_suffix(self.delimiter.as_str())
            .unwrap_or(normalized)
            .trim();
        if normalized.is_empty()
            || starts_with_keyword(normalized, "DELIMITER")
            || self.is_pure_comment(normalized)
        {
            return None;
        }
        Some(normalized.to_string())
    }

    fn strip_leading_ignorable_lines<'a>(&self, stmt: &'a str) -> &'a str {
        let mut remaining = stmt;

        loop {
            let trimmed_start = remaining.trim_start();
            if trimmed_start.is_empty() {
                return trimmed_start;
            }

            let line_end = trimmed_start.find('\n').unwrap_or(trimmed_start.len());
            let line = trimmed_start[..line_end].trim_end_matches('\r');

            if !self.is_ignorable_leading_line(line) {
                return trimmed_start;
            }

            if line_end == trimmed_start.len() {
                return "";
            }

            remaining = &trimmed_start[line_end + 1..];
        }
    }

    fn is_ignorable_leading_line(&self, line: &str) -> bool {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            return true;
        }

        if self.is_separator_line(trimmed) {
            return true;
        }

        match self.db_type {
            DatabaseType::MySQL => {
                if let Some(rest) = trimmed.strip_prefix("--") {
                    return rest.is_empty()
                        || rest
                            .chars()
                            .next()
                            .is_some_and(|ch| ch.is_whitespace() || ch.is_control());
                }
                trimmed.starts_with('#')
            }
            _ => trimmed.starts_with("--"),
        }
    }

    fn is_separator_line(&self, line: &str) -> bool {
        const MIN_SEPARATOR_LEN: usize = 3;

        let mut chars = line.chars();
        let Some(first) = chars.next() else {
            return false;
        };

        if !matches!(first, '-' | '=' | '*' | '/' | '#') {
            return false;
        }

        line.chars().count() >= MIN_SEPARATOR_LEN && chars.all(|ch| ch == first)
    }

    fn try_extract_dollar_quote(&self) -> Option<String> {
        let last_dollar_pos = self.buffer.rfind('$')?;
        if last_dollar_pos == 0 {
            return None;
        }

        let before_last = &self.buffer[..last_dollar_pos];
        let prev_dollar_pos = before_last.rfind('$')?;

        let tag = &self.buffer[prev_dollar_pos..=last_dollar_pos];
        let inner = &tag[1..tag.len() - 1];
        let valid_tag = inner.is_empty()
            || inner
                .chars()
                .next()
                .is_some_and(|first| first == '_' || first.is_ascii_alphabetic())
                && inner
                    .chars()
                    .skip(1)
                    .all(|c| c == '_' || c.is_ascii_alphanumeric());
        if valid_tag {
            Some(tag.to_string())
        } else {
            None
        }
    }

    fn try_parse_delimiter(&self) -> Option<String> {
        let (_, last_line) = split_last_line(&self.buffer)?;
        let trimmed = last_line.trim();
        if !starts_with_keyword(trimmed, "DELIMITER") {
            return None;
        }
        let mut parts = trimmed.split_whitespace();
        parts.next()?;
        let delimiter = parts.next()?;
        (parts.next().is_none() && !delimiter.is_empty()).then(|| delimiter.to_string())
    }

    /// 检查字符串是否为纯注释（只包含注释和空白字符）
    fn is_pure_comment(&self, s: &str) -> bool {
        let mut chars = s.trim().chars().peekable();

        while let Some(ch) = chars.next() {
            match ch {
                // 空白字符跳过
                c if c.is_whitespace() => continue,

                // 行注释 --
                '-' => {
                    if chars.peek() == Some(&'-') {
                        chars.next();
                        // 跳过直到换行
                        for c in chars.by_ref() {
                            if c == '\n' {
                                break;
                            }
                        }
                    } else {
                        return false;
                    }
                }

                // MySQL 的 # 注释
                '#' if self.db_type == DatabaseType::MySQL => {
                    // 跳过直到换行
                    for c in chars.by_ref() {
                        if c == '\n' {
                            break;
                        }
                    }
                }

                // 块注释 /* */
                '/' => {
                    if chars.peek() == Some(&'*') {
                        chars.next();
                        if self.db_type == DatabaseType::MySQL && chars.peek() == Some(&'!') {
                            return false;
                        }
                        // 跳过直到 */
                        let mut prev = ' ';
                        let mut closed = false;
                        for c in chars.by_ref() {
                            if prev == '*' && c == '/' {
                                closed = true;
                                break;
                            }
                            prev = c;
                        }
                        if !closed {
                            return false;
                        }
                    } else {
                        return false;
                    }
                }

                // 其他非空白字符表示不是纯注释
                _ => return false,
            }
        }

        true
    }

    fn update_begin_depth(&mut self) {
        let buffer_len = self.buffer.len();

        if buffer_len <= self.last_checked_len {
            return;
        }

        let buffer_bytes = self.buffer.as_bytes();
        let mut end = buffer_len;

        while end > 0 {
            let ch = buffer_bytes[end - 1];
            if ch.is_ascii_whitespace() || ch == b';' || ch == b',' || ch == b'$' {
                end -= 1;
            } else {
                break;
            }
        }

        if end == 0 {
            self.last_checked_len = buffer_len;
            return;
        }

        if end <= self.last_checked_len {
            self.last_checked_len = buffer_len;
            return;
        }

        let mut start = end;
        while start > 0 && buffer_bytes[start - 1].is_ascii_alphabetic() {
            start -= 1;
        }

        let last_word = &self.buffer[start..end];
        let last_word_upper = last_word.to_uppercase();

        if last_word_upper == "BEGIN" && self.should_track_begin_depth() {
            self.begin_depth += 1;
        } else if last_word_upper == "END" {
            self.pending_end_word = true;
        }

        self.last_checked_len = end;
    }

    fn should_track_begin_depth(&self) -> bool {
        let normalized = self
            .strip_leading_ignorable_lines(&self.buffer)
            .trim_start();
        let upper = normalized.to_ascii_uppercase();
        match self.db_type {
            DatabaseType::Oracle => {
                upper.starts_with("BEGIN") || starts_with_create_routine(&upper)
            }
            DatabaseType::MSSQL => starts_with_create_routine(&upper),
            DatabaseType::SQLite => starts_with_create_routine(&upper),
            DatabaseType::MySQL => {
                starts_with_create_routine(&upper)
                    || starts_with_mysql_standalone_begin_block(normalized)
            }
            _ => false,
        }
    }
}

/// 判断 MySQL 文本是否是独立复合 `BEGIN ... END` 块，而不是事务控制 `BEGIN;`。
fn starts_with_mysql_standalone_begin_block(sql: &str) -> bool {
    // 事务 BEGIN 必须立即输出；独立复合块以单独 BEGIN 行开头，内部语句需保留至配对 END。
    sql.contains('\n')
        && sql
            .lines()
            .next()
            .is_some_and(|line| line.trim().eq_ignore_ascii_case("begin"))
}

/// 判断规范化 SQL 是否以存储过程、函数或触发器定义开头。
///
/// 该判断兼容 `CREATE OR REPLACE/ALTER` 和 MySQL DEFINER 等修饰符，仅用于决定
/// 是否跟踪 `BEGIN/END` 深度，不负责校验完整 DDL 语法。
fn starts_with_create_routine(sql: &str) -> bool {
    let mut words = sql.split_whitespace();
    if words.next() != Some("CREATE") {
        return false;
    }
    let mut object_type = words.next().unwrap_or_default();
    if object_type == "OR" {
        let modifier = words.next().unwrap_or_default();
        if matches!(modifier, "REPLACE" | "ALTER") {
            object_type = words.next().unwrap_or_default();
        }
    }
    while matches!(object_type, "DEFINER" | "TEMP" | "TEMPORARY")
        || object_type.starts_with("DEFINER=")
    {
        object_type = words.next().unwrap_or_default();
    }
    matches!(object_type, "PROCEDURE" | "PROC" | "FUNCTION" | "TRIGGER")
}

/// 解析 SQL Server 独占行 `GO [count]`。
///
/// 非 GO 行返回 `Ok(None)`；合法 GO 返回正整数次数；零值、非数字、多余参数和
/// 超过安全上限的值返回 `InvalidData`，调用方应终止当前解析器。
fn parse_go_repeat_count(line: &str) -> io::Result<Option<usize>> {
    let mut parts = line.split_whitespace();
    if !parts
        .next()
        .is_some_and(|word| word.eq_ignore_ascii_case("go"))
    {
        return Ok(None);
    }
    let repeat_count = match parts.next() {
        Some(count) => count
            .parse::<usize>()
            .ok()
            .filter(|count| *count > 0)
            .ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::InvalidData,
                    "SQL Server GO 重复次数必须为正整数",
                )
            })?,
        None => 1,
    };
    if parts.next().is_some() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "SQL Server GO 指令包含多余参数",
        ));
    }
    if repeat_count > MAX_MSSQL_GO_REPEAT {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!(
                "SQL Server GO 重复次数 {} 超过安全上限 {}",
                repeat_count, MAX_MSSQL_GO_REPEAT
            ),
        ));
    }
    Ok(Some(repeat_count))
}

impl Iterator for StreamingSqlParser {
    type Item = io::Result<String>;

    fn next(&mut self) -> Option<Self::Item> {
        match self.read_next_statement() {
            Ok(Some(stmt)) => Some(Ok(stmt)),
            Ok(None) => None,
            Err(e) => {
                self.terminated = true;
                Some(Err(e))
            }
        }
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use one_core::storage::DatabaseType;

    fn parse_all(source: SqlSource, db_type: DatabaseType) -> Vec<String> {
        StreamingSqlParser::from_source(source, db_type)
            .unwrap()
            .collect::<io::Result<Vec<_>>>()
            .expect("测试脚本必须完整解析成功")
    }

    #[test]
    fn test_basic_statements() {
        let sql = "SELECT * FROM users;\nINSERT INTO users VALUES (1, 'test');\nUPDATE users SET name = 'new';";
        let statements = parse_all(SqlSource::Script(sql.to_string()), DatabaseType::MySQL);

        assert_eq!(statements.len(), 3);
        assert_eq!(statements[0], "SELECT * FROM users");
        assert_eq!(statements[1], "INSERT INTO users VALUES (1, 'test')");
        assert_eq!(statements[2], "UPDATE users SET name = 'new'");
    }

    #[test]
    fn test_data_compare_snapshot_statements_remain_single_commands() {
        for (database_type, sql) in [
            (
                DatabaseType::PostgreSQL,
                "BEGIN TRANSACTION ISOLATION LEVEL REPEATABLE READ READ ONLY",
            ),
            (
                DatabaseType::MySQL,
                "SET TRANSACTION ISOLATION LEVEL REPEATABLE READ",
            ),
            (DatabaseType::MySQL, "START TRANSACTION READ ONLY"),
            (DatabaseType::SQLite, "BEGIN"),
            (DatabaseType::PostgreSQL, "ROLLBACK"),
            (DatabaseType::MySQL, "ROLLBACK"),
            (DatabaseType::SQLite, "ROLLBACK"),
        ] {
            assert_eq!(
                vec![sql.to_string()],
                parse_all(SqlSource::Script(sql.to_string()), database_type),
                "snapshot command must pass through the script splitter unchanged"
            );
        }
    }

    #[test]
    fn test_string_with_backslash_escape() {
        let sql =
            "INSERT INTO t VALUES ('it\\'s good');\nINSERT INTO t VALUES ('path\\\\to\\\\file');";
        let statements = parse_all(SqlSource::Script(sql.to_string()), DatabaseType::MySQL);

        assert_eq!(statements.len(), 2);
        assert_eq!(statements[0], "INSERT INTO t VALUES ('it\\'s good')");
        assert_eq!(statements[1], "INSERT INTO t VALUES ('path\\\\to\\\\file')");
    }

    #[test]
    fn test_string_with_double_quote_escape() {
        let sql =
            "INSERT INTO t VALUES ('it''s good');\nINSERT INTO t VALUES ('quote''test''here');";
        let statements = parse_all(SqlSource::Script(sql.to_string()), DatabaseType::MySQL);

        assert_eq!(statements.len(), 2);
        assert_eq!(statements[0], "INSERT INTO t VALUES ('it''s good')");
        assert_eq!(statements[1], "INSERT INTO t VALUES ('quote''test''here')");
    }

    #[test]
    fn test_mixed_escapes() {
        let sql = "INSERT INTO t VALUES ('test\\'s', 'he''s', 'path\\\\x');\nSELECT * FROM t;";
        let statements = parse_all(SqlSource::Script(sql.to_string()), DatabaseType::MySQL);

        assert_eq!(statements.len(), 2);
        assert!(statements[0].contains("'test\\'s'"));
        assert!(statements[0].contains("'he''s'"));
        assert!(statements[0].contains("'path\\\\x'"));
    }

    #[test]
    fn test_line_comments() {
        let sql = "-- This is a comment\nSELECT * FROM users; -- inline comment\n-- Another comment\nINSERT INTO t VALUES (1);";
        let statements = parse_all(SqlSource::Script(sql.to_string()), DatabaseType::MySQL);

        assert_eq!(statements.len(), 2);
        assert!(statements[0].contains("SELECT * FROM users"));
        assert!(statements[1].contains("INSERT INTO t VALUES (1)"));
    }

    #[test]
    fn test_mysql_hash_comments() {
        let sql = "# MySQL comment\nSELECT * FROM users; # inline\nINSERT INTO t VALUES (1);";
        let statements = parse_all(SqlSource::Script(sql.to_string()), DatabaseType::MySQL);

        assert_eq!(statements.len(), 2);
    }

    #[test]
    fn test_block_comments() {
        let sql = "/* This is a block comment */\nSELECT * FROM users; /* inline */ DELETE FROM t;";
        let statements = parse_all(SqlSource::Script(sql.to_string()), DatabaseType::MySQL);

        assert_eq!(statements.len(), 2);
        assert!(statements[0].contains("SELECT"));
        assert!(statements[1].contains("DELETE"));
    }

    #[test]
    fn test_delimiter_change() {
        let sql =
            "DELIMITER $$\nCREATE PROCEDURE p() BEGIN SELECT 1; END$$\nDELIMITER ;\nSELECT 2;";
        let statements = parse_all(SqlSource::Script(sql.to_string()), DatabaseType::MySQL);

        assert_eq!(statements.len(), 2);
        assert!(statements[0].contains("CREATE PROCEDURE"));
        assert!(statements[0].contains("BEGIN"));
        assert!(statements[0].contains("END"));
        assert_eq!(statements[1], "SELECT 2");
    }

    #[test]
    fn test_mysql_procedure_replacement_script() {
        let sql = r#"-- Running this script replaces the existing procedure.
-- MySQL executes DROP/CREATE as non-atomic DDL; keep a backup before running.
DROP PROCEDURE IF EXISTS `app_db`.`sync_orders`;

DELIMITER $$
CREATE DEFINER=`root`@`%` PROCEDURE `sync_orders`()
BEGIN
  SELECT 'value;inside';
  BEGIN
    SELECT 2;
  END;
END$$
DELIMITER ;

-- Add arguments as needed before running:
-- CALL `app_db`.`sync_orders`();
"#;
        let statements = parse_all(SqlSource::Script(sql.to_string()), DatabaseType::MySQL);

        assert_eq!(statements.len(), 2);
        assert_eq!(
            statements[0],
            "DROP PROCEDURE IF EXISTS `app_db`.`sync_orders`"
        );
        assert!(statements[1].starts_with("CREATE DEFINER="));
        assert!(statements[1].contains("SELECT 'value;inside';"));
        assert!(statements[1].contains("BEGIN\n    SELECT 2;\n  END;"));
        assert!(statements[1].ends_with("END"));
        assert!(
            !statements
                .iter()
                .any(|statement| statement.contains("DELIMITER"))
        );
        assert!(
            !statements
                .iter()
                .any(|statement| statement.contains("CALL"))
        );
    }

    #[test]
    fn test_mysql_function_replacement_script() {
        let sql = r#"-- Running this script replaces the existing function.
-- MySQL executes DROP/CREATE as non-atomic DDL; keep a backup before running.
DROP FUNCTION IF EXISTS `app_db`.`calculate_total`;

DELIMITER $$
CREATE DEFINER=`root`@`%` FUNCTION `calculate_total`(amount INT)
RETURNS INT
DETERMINISTIC
BEGIN
  DECLARE result_value INT;
  SET result_value = amount + 1;
  RETURN result_value;
END$$
DELIMITER ;

-- Add arguments as needed before running:
-- SELECT `app_db`.`calculate_total`(1);
"#;
        let statements = parse_all(SqlSource::Script(sql.to_string()), DatabaseType::MySQL);

        assert_eq!(statements.len(), 2);
        assert_eq!(
            statements[0],
            "DROP FUNCTION IF EXISTS `app_db`.`calculate_total`"
        );
        assert!(statements[1].starts_with("CREATE DEFINER="));
        assert!(statements[1].contains("FUNCTION `calculate_total`"));
        assert!(statements[1].contains("RETURNS INT"));
        assert!(statements[1].contains("SET result_value = amount + 1;"));
        assert!(statements[1].ends_with("END"));
        assert!(
            !statements
                .iter()
                .any(|statement| statement.contains("DELIMITER"))
        );
        assert!(
            !statements
                .iter()
                .any(|statement| statement.contains("-- SELECT"))
        );
    }

    #[test]
    fn test_transaction_begin_does_not_swallow_following_statements() {
        let sql = "BEGIN;\nSELECT 1;\nCOMMIT;\nSELECT 2;";
        for database_type in [
            DatabaseType::MySQL,
            DatabaseType::PostgreSQL,
            DatabaseType::SQLite,
        ] {
            let statements = parse_all(SqlSource::Script(sql.to_string()), database_type.clone());
            assert_eq!(
                statements,
                vec!["BEGIN", "SELECT 1", "COMMIT", "SELECT 2"],
                "{database_type:?}"
            );
        }

        // SQL Server's client-side execution unit is a GO-delimited batch.
        // Semicolons inside a batch do not split it into independent requests.
        assert_eq!(
            parse_all(SqlSource::Script(sql.to_string()), DatabaseType::MSSQL),
            vec!["BEGIN;\nSELECT 1;\nCOMMIT;\nSELECT 2"]
        );
    }

    #[test]
    fn test_nested_parentheses() {
        let sql = "SELECT * FROM t WHERE id IN (SELECT id FROM u WHERE (status = 1 AND (flag = 0)));\nINSERT INTO t VALUES (1);";
        let statements = parse_all(SqlSource::Script(sql.to_string()), DatabaseType::MySQL);

        assert_eq!(statements.len(), 2);
        assert!(statements[0].contains("IN (SELECT"));
    }

    #[test]
    fn test_postgresql_dollar_quote() {
        let sql = "CREATE FUNCTION f() RETURNS void AS $$\nBEGIN\n  SELECT 'test;here';\nEND;\n$$ LANGUAGE plpgsql;\nSELECT 1;";
        let statements = parse_all(SqlSource::Script(sql.to_string()), DatabaseType::PostgreSQL);

        assert_eq!(statements.len(), 2);
        assert!(statements[0].contains("$$"));
        assert!(statements[0].contains("'test;here'"));
        assert_eq!(statements[1], "SELECT 1");
    }

    #[test]
    fn test_postgresql_tagged_dollar_quote() {
        let sql = "CREATE FUNCTION f() RETURNS text AS $body$\nSELECT 'test; with semicolon';\n$body$ LANGUAGE sql;\nSELECT 2;";
        let statements = parse_all(SqlSource::Script(sql.to_string()), DatabaseType::PostgreSQL);

        assert_eq!(statements.len(), 2);
        assert!(statements[0].contains("$body$"));
        assert!(statements[0].contains("semicolon"));
    }

    #[test]
    fn test_mssql_go_separator() {
        let sql = "CREATE TABLE t (id INT);\nGO\nINSERT INTO t VALUES (1);\nGO\nSELECT * FROM t;";
        let statements = parse_all(SqlSource::Script(sql.to_string()), DatabaseType::MSSQL);

        assert_eq!(statements.len(), 3);
        assert!(statements[0].contains("CREATE TABLE"));
        assert!(statements[1].contains("INSERT"));
        assert!(statements[2].contains("SELECT"));
    }

    #[test]
    fn test_mssql_go_repeat_count() {
        let sql = "SELECT 1;\nGO 3\nSELECT 2;\nGO\n";
        let statements = parse_all(SqlSource::Script(sql.to_string()), DatabaseType::MSSQL);

        assert_eq!(
            statements,
            vec!["SELECT 1;", "SELECT 1;", "SELECT 1;", "SELECT 2;"]
        );
    }

    #[test]
    fn test_mssql_go_separator_at_eof_without_newline() {
        let sql = "SELECT 1;\nGO";
        let statements = parse_all(SqlSource::Script(sql.to_string()), DatabaseType::MSSQL);

        assert_eq!(statements, vec!["SELECT 1;"]);
    }

    #[test]
    fn test_mssql_go_repeat_count_at_eof_without_newline() {
        let sql = "SELECT 1;\nGO 3";
        let statements = parse_all(SqlSource::Script(sql.to_string()), DatabaseType::MSSQL);

        assert_eq!(statements, vec!["SELECT 1;", "SELECT 1;", "SELECT 1;"]);
    }

    #[test]
    fn test_mssql_large_go_repeat_count_is_lazy() {
        let sql = format!("SELECT 1;\nGO {}", MAX_MSSQL_GO_REPEAT + 1);
        let mut parser =
            StreamingSqlParser::from_source(SqlSource::Script(sql), DatabaseType::MSSQL).unwrap();

        let first = parser.next().expect("超限 GO 必须返回错误");
        assert!(first.is_err());
        assert!(parser.next().is_none(), "解析错误后不应重复返回错误");
    }

    #[test]
    fn test_same_line_statements_do_not_drop_remaining_characters() {
        let sql = "SELECT 1;SELECT 2;SELECT 3;SELECT 4;";
        let statements = parse_all(SqlSource::Script(sql.to_string()), DatabaseType::MySQL);

        assert_eq!(
            statements,
            vec!["SELECT 1", "SELECT 2", "SELECT 3", "SELECT 4"]
        );
    }

    #[test]
    fn test_oracle_plsql_block_uses_slash_as_terminator() {
        let sql = "DECLARE\n  v_count NUMBER;\nBEGIN\n  v_count := 1;\n  DBMS_OUTPUT.PUT_LINE(v_count);\nEND;\n/\nSELECT 1;";
        let statements = parse_all(SqlSource::Script(sql.to_string()), DatabaseType::Oracle);

        assert_eq!(statements.len(), 2);
        assert!(statements[0].contains("v_count := 1;"));
        assert!(statements[0].ends_with("END;"));
        assert_eq!(statements[1], "SELECT 1");
    }

    #[test]
    fn test_oracle_slash_after_regular_sql_is_ignored() {
        let sql = "CREATE TABLE t (id NUMBER);\n/\nSELECT 1;";
        let statements = parse_all(SqlSource::Script(sql.to_string()), DatabaseType::Oracle);

        assert_eq!(statements, vec!["CREATE TABLE t (id NUMBER)", "SELECT 1"]);
    }

    #[test]
    fn test_unclosed_lexical_state_returns_error() {
        for (database_type, sql) in [
            (DatabaseType::MySQL, "SELECT 'unterminated"),
            (DatabaseType::MySQL, "SELECT /* unterminated"),
            (DatabaseType::PostgreSQL, "SELECT $$unterminated"),
            (DatabaseType::MySQL, "SELECT (1"),
        ] {
            let mut parser =
                StreamingSqlParser::from_source(SqlSource::Script(sql.to_string()), database_type)
                    .unwrap();
            assert!(parser.next().expect("未闭合结构必须报错").is_err());
            assert!(parser.next().is_none());
        }
    }

    #[test]
    fn test_oracle_plsql_without_slash_returns_error() {
        let mut parser = StreamingSqlParser::from_source(
            SqlSource::Script("BEGIN\n  NULL;\nEND;".to_string()),
            DatabaseType::Oracle,
        )
        .unwrap();

        let error = parser.next().expect("缺少 Oracle / 必须报错").unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
        assert!(parser.next().is_none());
    }

    #[test]
    fn test_mysql_conditional_comment_is_preserved() {
        let sql = "/*!40101 SET @saved_cs_client = @@character_set_client */;\nSELECT 1;";
        let statements = parse_all(SqlSource::Script(sql.to_string()), DatabaseType::MySQL);

        assert_eq!(statements.len(), 2);
        assert!(statements[0].starts_with("/*!40101 SET"));
        assert_eq!(statements[1], "SELECT 1");
    }

    #[test]
    fn test_mssql_bracket_identifier_can_contain_semicolon() {
        let sql = "SELECT [column;name] FROM [table;name];";
        let statements = parse_all(SqlSource::Script(sql.to_string()), DatabaseType::MSSQL);

        assert_eq!(statements, vec!["SELECT [column;name] FROM [table;name]"]);
    }

    #[test]
    fn test_postgresql_escape_string_and_unicode_before_dollar_tag() {
        // 中文内容用于覆盖 UTF-8 多字节字符；合法 dollar tag 仍遵循 PostgreSQL 标识符规则。
        let sql = "SELECT E'line\\nvalue;中文';SELECT '中文', $body$body;inside$body$;";
        let statements = parse_all(SqlSource::Script(sql.to_string()), DatabaseType::PostgreSQL);

        assert_eq!(statements.len(), 2);
        assert!(statements[0].contains("E'line\\nvalue;中文'"));
        assert!(statements[1].contains("$body$body;inside$body$"));
    }

    #[test]
    fn test_oracle_q_quote_can_contain_semicolon() {
        let sql = "SELECT q'[value;with;semicolon]' FROM dual;";
        let statements = parse_all(SqlSource::Script(sql.to_string()), DatabaseType::Oracle);

        assert_eq!(
            statements,
            vec!["SELECT q'[value;with;semicolon]' FROM dual"]
        );
    }

    #[test]
    fn test_nested_block_comments_are_supported() {
        let postgresql = "SELECT 1 /* outer /* inner; */ still outer */;SELECT 2;";
        assert_eq!(
            parse_all(
                SqlSource::Script(postgresql.to_string()),
                DatabaseType::PostgreSQL
            ),
            vec!["SELECT 1 /* outer /* inner; */ still outer */", "SELECT 2"]
        );

        // SQL Server 以 GO 为客户端批次边界，因此使用两个 GO 批次验证嵌套注释状态。
        let mssql = "SELECT 1 /* outer /* inner; */ still outer */;\nGO\nSELECT 2;\nGO\n";
        assert_eq!(
            parse_all(SqlSource::Script(mssql.to_string()), DatabaseType::MSSQL),
            vec![
                "SELECT 1 /* outer /* inner; */ still outer */;",
                "SELECT 2;"
            ]
        );
    }

    #[test]
    fn test_invalid_mssql_go_arguments_fail_once() {
        for go_line in ["GO 0", "GO abc", "GO 2 extra"] {
            let sql = format!("SELECT 1;\n{go_line}\n");
            let mut parser =
                StreamingSqlParser::from_source(SqlSource::Script(sql), DatabaseType::MSSQL)
                    .unwrap();
            assert!(parser.next().expect("非法 GO 必须报错").is_err());
            assert!(parser.next().is_none());
        }
    }

    #[test]
    fn test_unmatched_right_parenthesis_returns_error() {
        let mut parser = StreamingSqlParser::from_source(
            SqlSource::Script("SELECT 1);".to_string()),
            DatabaseType::MySQL,
        )
        .unwrap();

        let error = parser.next().expect("未匹配右括号必须报错").unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
    }

    #[test]
    fn test_utf8_bom_is_ignored() {
        let sql = "\u{feff}SELECT 1;";
        let statements = parse_all(SqlSource::Script(sql.to_string()), DatabaseType::MySQL);

        assert_eq!(statements, vec!["SELECT 1"]);
    }

    #[test]
    fn test_oracle_slash_separator() {
        let sql = "CREATE TABLE t (id NUMBER);\n/\nINSERT INTO t VALUES (1);\n/\nSELECT * FROM t;";
        let statements = parse_all(SqlSource::Script(sql.to_string()), DatabaseType::Oracle);

        assert!(statements.len() >= 2);
        assert!(statements[0].contains("CREATE TABLE"));
    }

    #[test]
    fn test_unicode_content() {
        let sql = "INSERT INTO t VALUES ('中文测试');\nINSERT INTO t VALUES ('日本語');\nINSERT INTO t VALUES ('한글');";
        let statements = parse_all(SqlSource::Script(sql.to_string()), DatabaseType::MySQL);

        assert_eq!(statements.len(), 3);
        assert!(statements[0].contains("中文测试"));
        assert!(statements[1].contains("日本語"));
        assert!(statements[2].contains("한글"));
    }

    #[test]
    fn test_unicode_with_escapes() {
        let sql = "INSERT INTO t VALUES ('测试\\'引号');\nINSERT INTO t VALUES ('test''测试');";
        let statements = parse_all(SqlSource::Script(sql.to_string()), DatabaseType::MySQL);

        assert_eq!(statements.len(), 2);
        assert!(statements[0].contains("测试\\'引号"));
        assert!(statements[1].contains("test''测试"));
    }

    #[test]
    fn test_multiline_statement() {
        let sql = "INSERT INTO users (\n  id,\n  name,\n  email\n)\nVALUES (\n  1,\n  'test',\n  'test@example.com'\n);\nSELECT 1;";
        let statements = parse_all(SqlSource::Script(sql.to_string()), DatabaseType::MySQL);

        assert_eq!(statements.len(), 2);
        assert!(statements[0].contains("INSERT INTO users"));
        assert!(statements[0].contains("test@example.com"));
    }

    #[test]
    fn test_empty_statements() {
        let sql = ";;;\nSELECT 1;\n;\n;;SELECT 2;";
        let statements = parse_all(SqlSource::Script(sql.to_string()), DatabaseType::MySQL);

        assert_eq!(statements.len(), 2);
        assert_eq!(statements[0], "SELECT 1");
        assert_eq!(statements[1], "SELECT 2");
    }

    #[test]
    fn test_complex_real_world_dump() {
        let sql = r#"
-- MySQL dump example
DROP TABLE IF EXISTS `users`;
/*!40101 SET @saved_cs_client     = @@character_set_client */;
CREATE TABLE `users` (
  `id` int(11) NOT NULL AUTO_INCREMENT,
  `name` varchar(100) DEFAULT NULL,
  `email` varchar(100) DEFAULT 'test@example.com',
  PRIMARY KEY (`id`)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4;

LOCK TABLES `users` WRITE;
INSERT INTO `users` VALUES (1,'O\'Reilly','test@mail.com'),(2,'It''s fine','user@test.com');
UNLOCK TABLES;
"#;
        let statements = parse_all(SqlSource::Script(sql.to_string()), DatabaseType::MySQL);

        assert!(statements.len() >= 5);
        assert!(statements.iter().any(|s| s.contains("DROP TABLE")));
        assert!(statements.iter().any(|s| s.contains("CREATE TABLE")));
        assert!(statements.iter().any(|s| s.contains("O\\'Reilly")));
        assert!(statements.iter().any(|s| s.contains("It''s fine")));
    }

    #[test]
    fn test_string_with_semicolon_inside() {
        let sql = "INSERT INTO t VALUES ('SELECT * FROM users; DELETE FROM t;');\nSELECT 1;";
        let statements = parse_all(SqlSource::Script(sql.to_string()), DatabaseType::MySQL);

        assert_eq!(statements.len(), 2);
        assert!(statements[0].contains("SELECT * FROM users; DELETE FROM t;"));
        assert_eq!(statements[1], "SELECT 1");
    }

    #[test]
    fn test_backtick_identifiers() {
        let sql = "SELECT `id`, `name` FROM `users`;\nINSERT INTO `table` VALUES (1, 'test;here');";
        let statements = parse_all(SqlSource::Script(sql.to_string()), DatabaseType::MySQL);

        assert_eq!(statements.len(), 2);
        assert!(statements[0].contains("`id`"));
        assert!(statements[1].contains("'test;here'"));
    }

    #[test]
    fn test_double_quote_identifiers() {
        let sql = r#"SELECT "id", "name" FROM "users";"#;
        let statements = parse_all(SqlSource::Script(sql.to_string()), DatabaseType::PostgreSQL);

        assert_eq!(statements.len(), 1);
        assert!(statements[0].contains(r#""id""#));
    }

    #[test]
    fn test_mixed_quotes() {
        let sql = r#"SELECT 'single', "double", `backtick` FROM t WHERE x = 'it''s' AND y = "col""name";"#;
        let statements = parse_all(SqlSource::Script(sql.to_string()), DatabaseType::MySQL);

        assert_eq!(statements.len(), 1);
        assert!(statements[0].contains("'single'"));
        assert!(statements[0].contains("'it''s'"));
    }

    #[test]
    fn test_progress_tracking() {
        let sql = "SELECT 1;\nSELECT 2;\nSELECT 3;";
        let mut parser = StreamingSqlParser::from_source(
            SqlSource::Script(sql.to_string()),
            DatabaseType::MySQL,
        )
        .unwrap();

        assert_eq!(parser.progress_percent(), 0.0);

        let _ = parser.next();
        assert!(parser.progress_percent() > 0.0);
        assert!(parser.progress_percent() < 100.0);

        while parser.next().is_some() {}
        assert_eq!(parser.progress_percent(), 100.0);
    }

    #[test]
    fn test_pure_comment_after_statement() {
        // 测试语句后跟纯注释的情况，纯注释不应该被当作独立语句
        let sql = "SELECT id, username, create_by FROM login_user; -- ❌ 列不存在";
        let statements = parse_all(SqlSource::Script(sql.to_string()), DatabaseType::MySQL);

        assert_eq!(statements.len(), 1);
        assert_eq!(
            statements[0],
            "SELECT id, username, create_by FROM login_user"
        );
    }

    #[test]
    fn test_pure_comment_only() {
        // 测试只有纯注释的情况
        let sql = "-- 这是一个注释";
        let statements = parse_all(SqlSource::Script(sql.to_string()), DatabaseType::MySQL);

        assert_eq!(statements.len(), 0);
    }

    #[test]
    fn test_multiple_pure_comments() {
        // 测试多个纯注释
        let sql = "-- 注释1\n-- 注释2\n/* 块注释 */";
        let statements = parse_all(SqlSource::Script(sql.to_string()), DatabaseType::MySQL);

        assert_eq!(statements.len(), 0);
    }

    #[test]
    fn test_mixed_comments_and_statements() {
        // 测试混合场景
        let sql = "SELECT 1; -- 注释\n-- 纯注释\nSELECT 2; /* 行尾注释 */";
        let statements = parse_all(SqlSource::Script(sql.to_string()), DatabaseType::MySQL);

        assert_eq!(statements.len(), 2);
        assert!(statements[0].contains("SELECT 1"));
        assert!(statements[1].contains("SELECT 2"));
    }

    #[test]
    fn test_nested_begin_end() {
        let sql = "BEGIN\n  BEGIN\n    SELECT 1;\n  END;\n  SELECT 2;\nEND;\nSELECT 3;";
        let statements = parse_all(SqlSource::Script(sql.to_string()), DatabaseType::MySQL);

        assert_eq!(statements.len(), 2);
        assert!(statements[0].contains("BEGIN"));
        assert!(statements[0].contains("SELECT 1"));
        assert!(statements[0].contains("SELECT 2"));
    }

    #[test]
    fn test_create_procedure_with_complex_body() {
        let sql = r#"DELIMITER $$
CREATE PROCEDURE complex_proc(IN param INT)
BEGIN
    DECLARE var VARCHAR(100);
    SET var = 'test;value';

    -- Comment with semicolon;
    IF param > 0 THEN
        SELECT * FROM users WHERE name = 'O''Reilly';
    ELSE
        INSERT INTO log VALUES ('Error; occurred');
    END IF;
END$$
DELIMITER ;
SELECT 'done';"#;
        let statements = parse_all(SqlSource::Script(sql.to_string()), DatabaseType::MySQL);

        assert_eq!(statements.len(), 2);
        assert!(statements[0].contains("CREATE PROCEDURE"));
        assert!(statements[0].contains("'test;value'"));
        assert!(statements[0].contains("'O''Reilly'"));
        assert_eq!(statements[1], "SELECT 'done'");
    }

    #[test]
    fn test_mysql_separator_comment_block_should_not_be_executed() {
        let sql = r#"------------------------------------------------------------------------
-- 模块: 用户表
------------------------------------------------------------------------
SELECT 1;
"#;
        let statements = parse_all(SqlSource::Script(sql.to_string()), DatabaseType::MySQL);

        assert_eq!(statements, vec!["SELECT 1"]);
    }

    #[test]
    fn test_mysql_block_comment_with_separator_only_should_not_be_executed() {
        let sql = r#"/**
 sql脚本文件命名规则:
 V: 前缀
 1: 自增长序列，新增的以 2 开始
 readme: 本文档的 版本号等说明
 版本号 V8.0SP2
 */

 ------------------------------------------------------------------------
"#;
        let statements = parse_all(SqlSource::Script(sql.to_string()), DatabaseType::MySQL);

        assert!(statements.is_empty());
    }
}
