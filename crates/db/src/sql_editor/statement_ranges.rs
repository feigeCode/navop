use std::ops::Range;

use one_core::storage::DatabaseType;

use super::sql_tokenizer::{SqlTokenKind, SqlTokenizer};

/// The SQL dialect behavior required by the editor statement range engine.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SqlDialect {
    Standard,
    MySql,
    PostgreSql,
    SqlServer,
    Oracle,
}

impl From<&DatabaseType> for SqlDialect {
    fn from(value: &DatabaseType) -> Self {
        match value {
            DatabaseType::MySQL => Self::MySql,
            // TDengine 语句划分规则与 MySQL 方言一致。
            DatabaseType::TDengine => Self::MySql,
            DatabaseType::PostgreSQL => Self::PostgreSql,
            DatabaseType::MSSQL => Self::SqlServer,
            DatabaseType::Oracle => Self::Oracle,
            DatabaseType::SQLite
            | DatabaseType::DuckDB
            | DatabaseType::ClickHouse
            | DatabaseType::External { .. } => Self::Standard,
        }
    }
}

/// A UTF-8 byte range in the editor document.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct SqlTextRange {
    pub start_byte: usize,
    pub end_byte: usize,
}

impl SqlTextRange {
    fn new(start_byte: usize, end_byte: usize) -> Self {
        Self {
            start_byte,
            end_byte,
        }
    }

    pub fn to_range(self) -> Range<usize> {
        self.start_byte..self.end_byte
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SqlStatementKind {
    Sql,
    Procedure,
    Function,
    Trigger,
    AnonymousBlock,
}

/// One executable SQL statement and the delimiter which terminated it.
///
/// All ranges use UTF-8 byte offsets. `sql_range` excludes leading trivia and
/// the terminating delimiter. `hit_start_byte` is the start of the line which
/// contains the first SQL token, which makes cursor ownership deterministic in
/// whitespace between statements.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SqlStatementRange {
    pub hit_start_byte: usize,
    pub sql_range: SqlTextRange,
    pub delimiter_range: Option<SqlTextRange>,
    pub start_line: usize,
    pub kind: SqlStatementKind,
    pub batch_index: usize,
    pub batch_repeat_count: usize,
}

/// A text/dialect/ranges snapshot which callers can share during one operation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SqlStatementSnapshot {
    text: String,
    dialect: SqlDialect,
    ranges: Vec<SqlStatementRange>,
}

impl SqlStatementSnapshot {
    pub fn new(text: impl Into<String>, dialect: SqlDialect) -> Self {
        let text = text.into();
        let ranges = split_sql_statement_ranges(&text, dialect);
        Self {
            text,
            dialect,
            ranges,
        }
    }

    pub fn text(&self) -> &str {
        &self.text
    }

    pub fn dialect(&self) -> SqlDialect {
        self.dialect
    }

    pub fn statement_ranges(&self) -> &[SqlStatementRange] {
        &self.ranges
    }

    pub fn statement_at_cursor(&self, cursor_byte: usize) -> Option<&SqlStatementRange> {
        statement_at_cursor_inner(&self.ranges, &self.text, 0, self.text.len(), cursor_byte)
    }

    pub fn statement_text(&self, statement: &SqlStatementRange) -> &str {
        &self.text[statement.sql_range.start_byte..statement.sql_range.end_byte]
    }
}

/// A statement scan over a window (slice) of the full document.
///
/// Produced for large documents where scanning the whole text on every edit is
/// too expensive: only the rows around the viewport are tokenized. All ranges
/// are shifted back into full-document coordinates (bytes and lines), so
/// windowed results can be keyed and compared exactly like full-document
/// results produced by [`SqlStatementSnapshot`].
///
/// Statements whose `hit_start_byte` lies before the window base may be
/// missing entirely (the scanner never saw their start); call sites must
/// refresh the window when the viewport leaves the analyzed row range.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WindowedStatementScan {
    base_byte: usize,
    base_line: usize,
    text: String,
    ranges: Vec<SqlStatementRange>,
}

impl WindowedStatementScan {
    /// Scan `text` (a slice of the document starting at `base_byte` /
    /// `base_line`) and shift every range into document coordinates.
    ///
    /// `base_byte` must be a UTF-8 boundary of the original document and
    /// `base_line` the 0-based document line of `base_byte`; callers are
    /// expected to pick a statement boundary (see [`line_scans_neutral`]) so
    /// the window never starts inside a string/comment/dollar-quoted body.
    pub fn scan(text: String, dialect: SqlDialect, base_byte: usize, base_line: usize) -> Self {
        let mut ranges = Scanner::new(&text, dialect).scan();
        for range in &mut ranges {
            range.hit_start_byte += base_byte;
            range.sql_range.start_byte += base_byte;
            range.sql_range.end_byte += base_byte;
            if let Some(delimiter) = &mut range.delimiter_range {
                delimiter.start_byte += base_byte;
                delimiter.end_byte += base_byte;
            }
            range.start_line += base_line;
        }
        Self {
            base_byte,
            base_line,
            text,
            ranges,
        }
    }

    /// Byte offset of the window start in the full document.
    pub fn base_byte(&self) -> usize {
        self.base_byte
    }

    /// 0-based document line of the window start.
    pub fn base_line(&self) -> usize {
        self.base_line
    }

    /// The window text (a slice of the document), not the full document.
    pub fn text(&self) -> &str {
        &self.text
    }

    pub fn statement_ranges(&self) -> &[SqlStatementRange] {
        &self.ranges
    }

    /// Resolve the statement owning `cursor_byte` (a document offset).
    ///
    /// A cursor outside the window clamps to the window edges, which yields
    /// `None` for cursors before the first in-window statement, matching the
    /// full-document behavior for whitespace before the first statement.
    pub fn statement_at_cursor(&self, cursor_byte: usize) -> Option<&SqlStatementRange> {
        self.statement_at_cursor_doc_len(cursor_byte, self.base_byte + self.text.len())
    }

    /// Same as [`Self::statement_at_cursor`], but the caller provides the full
    /// document length so the last in-window statement only claims trailing
    /// whitespace ownership when the window actually reaches the document end.
    /// A truncated trailing statement (more text below the window) reports no
    /// ownership past its own sql range instead of swallowing everything to
    /// the window end.
    pub fn statement_at_cursor_doc_len(
        &self,
        cursor_byte: usize,
        doc_len: usize,
    ) -> Option<&SqlStatementRange> {
        statement_at_cursor_inner(
            &self.ranges,
            &self.text,
            self.base_byte,
            doc_len,
            cursor_byte,
        )
    }
}

/// Whether tokenizing this line in isolation ends in a neutral state: no
/// unterminated string literal, quoted identifier, or block comment.
///
/// Used to pick window start lines for [`WindowedStatementScan`]: combined
/// with a line that ends a statement (trailing `;`), a neutral line is a safe
/// re-synchronization point because no quoting context leaks across it. This
/// mirrors the closed-token checks in the parser diagnostics (`String`,
/// `QuotedIdent` quote parity, `BlockComment` ending in `*/`).
///
/// Known limitation: PostgreSQL dollar-quoted bodies (`$$ ... $$`) are not
/// tracked by the tokenizer, so a neutral line may still sit inside a
/// dollar-quoted function body; the windowed scan can then mis-split until the
/// window is recomputed. Display-only consumers (gutter markers, statement
/// frame) tolerate this; execution paths always rescan the full document.
pub fn line_scans_neutral(line: &str) -> bool {
    SqlTokenizer::new(line)
        .tokenize()
        .iter()
        .all(|token| match token.kind {
            SqlTokenKind::String => token.text.matches('\'').count() % 2 == 0,
            SqlTokenKind::QuotedIdent => token.text.matches('"').count() % 2 == 0,
            SqlTokenKind::BlockComment => token.text.ends_with("*/"),
            _ => true,
        })
}

pub fn split_sql_statement_ranges(sql: &str, dialect: SqlDialect) -> Vec<SqlStatementRange> {
    Scanner::new(sql, dialect).scan()
}

/// Read-only statement index shared by full-document snapshots and windowed
/// scans, so display consumers (gutter markers, current-statement frame) can
/// work against either source.
pub trait StatementIndex {
    fn statement_ranges(&self) -> &[SqlStatementRange];

    /// Resolve the statement owning `cursor_byte` (a document offset).
    ///
    /// `doc_len` is the full document length in bytes; windowed indexes use it
    /// to decide whether their window reaches the document end, full-document
    /// indexes ignore it.
    fn statement_at_cursor(&self, cursor_byte: usize, doc_len: usize)
    -> Option<&SqlStatementRange>;
}

impl StatementIndex for SqlStatementSnapshot {
    fn statement_ranges(&self) -> &[SqlStatementRange] {
        &self.ranges
    }

    fn statement_at_cursor(
        &self,
        cursor_byte: usize,
        _doc_len: usize,
    ) -> Option<&SqlStatementRange> {
        self.statement_at_cursor(cursor_byte)
    }
}

impl StatementIndex for WindowedStatementScan {
    fn statement_ranges(&self) -> &[SqlStatementRange] {
        &self.ranges
    }

    fn statement_at_cursor(
        &self,
        cursor_byte: usize,
        doc_len: usize,
    ) -> Option<&SqlStatementRange> {
        self.statement_at_cursor_doc_len(cursor_byte, doc_len)
    }
}

pub fn statement_at_cursor<'a>(
    statements: &'a [SqlStatementRange],
    sql: &str,
    cursor_byte: usize,
) -> Option<&'a SqlStatementRange> {
    statement_at_cursor_inner(statements, sql, 0, sql.len(), cursor_byte)
}

fn statement_at_cursor_inner<'a>(
    statements: &'a [SqlStatementRange],
    window: &str,
    base_byte: usize,
    doc_len: usize,
    cursor_byte: usize,
) -> Option<&'a SqlStatementRange> {
    // A cursor before the window belongs to a statement the scan never saw;
    // report nothing and let the caller recompute the window.
    if cursor_byte < base_byte {
        return None;
    }
    // Clamp the document-space cursor into the window, then back into
    // document coordinates; a cursor before the window lands on the window
    // start, which no statement owns unless one starts exactly there.
    let cursor = base_byte + clamp_utf8_boundary(window, cursor_byte.saturating_sub(base_byte));

    for (index, statement) in statements.iter().enumerate() {
        if cursor < statement.hit_start_byte {
            break;
        }
        if cursor < statement.sql_range.start_byte {
            return Some(statement);
        }
        if cursor < statement.sql_range.end_byte {
            return Some(statement);
        }
        if let Some(delimiter) = statement.delimiter_range {
            if cursor < delimiter.end_byte {
                return Some(statement);
            }

            // Spec §5.5 rule 4: a cursor in whitespace or the delimiter gap on
            // the same line as a completed delimiter still belongs to that
            // statement. Own up to the start of the next line, but never past a
            // subsequent statement's hit range (same-line batches) and never
            // into a following blank/comment-only line, which yields `None`
            // (spec §5.5 rule 5).
            let line_end = window[delimiter.end_byte - base_byte..]
                .find('\n')
                .map(|index| delimiter.end_byte + index + 1)
                .unwrap_or(base_byte + window.len() + 1);
            let next_hit_start = statements
                .get(index + 1)
                .map(|next| next.hit_start_byte)
                .unwrap_or(line_end);
            if cursor < line_end.min(next_hit_start) {
                return Some(statement);
            }
            continue;
        }

        // When the window does not reach the document end, the trailing
        // statement may be truncated: more of it exists below the window.
        // Claim ownership only inside its sql range so cursor ownership past
        // the window is answered by a freshly recomputed window instead.
        if base_byte + window.len() < doc_len {
            return None;
        }

        // The final statement has no delimiter. Trailing whitespace can still be
        // an implicit cursor position for that statement.
        return Some(statement);
    }

    None
}

pub fn statement_starting_on_line(
    statements: &[SqlStatementRange],
    row: usize,
) -> Option<&SqlStatementRange> {
    statements
        .iter()
        .find(|statement| statement.start_line == row)
}

fn clamp_utf8_boundary(text: &str, mut offset: usize) -> usize {
    offset = offset.min(text.len());
    while offset < text.len() && !text.is_char_boundary(offset) {
        offset += 1;
    }
    offset
}

fn line_index(text: &str, offset: usize) -> usize {
    text[..offset].bytes().filter(|byte| *byte == b'\n').count()
}

fn is_identifier_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'_' || byte >= 0x80
}

fn statement_kind(text: &str, range: Range<usize>, dialect: SqlDialect) -> SqlStatementKind {
    let mut rest = text[range].trim_start();
    while let Some(comment_end) = leading_comment_end(rest) {
        rest = rest[comment_end..].trim_start();
    }

    let mut words = rest.split_whitespace();
    let first = words.next().unwrap_or_default().to_ascii_lowercase();
    if first != "create" {
        return if first == "begin" && dialect == SqlDialect::Oracle {
            SqlStatementKind::AnonymousBlock
        } else {
            SqlStatementKind::Sql
        };
    }

    let mut object_type = words.next().unwrap_or_default().to_ascii_lowercase();
    if object_type == "or" {
        let modifier = words.next().unwrap_or_default().to_ascii_lowercase();
        if matches!(modifier.as_str(), "replace" | "alter") {
            object_type = words.next().unwrap_or_default().to_ascii_lowercase();
        }
    }

    match object_type.as_str() {
        "procedure" | "proc" => SqlStatementKind::Procedure,
        "function" => SqlStatementKind::Function,
        "trigger" => SqlStatementKind::Trigger,
        _ => SqlStatementKind::Sql,
    }
}

fn leading_comment_end(text: &str) -> Option<usize> {
    if let Some(rest) = text.strip_prefix("--") {
        rest.find('\n').map(|index| index + 2).or(Some(text.len()))
    } else if text.starts_with("/*") {
        text[2..].find("*/").map(|index| index + 4)
    } else {
        None
    }
}

struct Scanner<'a> {
    text: &'a str,
    bytes: &'a [u8],
    dialect: SqlDialect,
    position: usize,
    current_start: Option<usize>,
    current_code_end: Option<usize>,
    current_hit_start: usize,
    begin_depth: usize,
    tracks_block_depth: bool,
    routine_mode: bool,
    delimiter: Vec<u8>,
    current_batch_index: usize,
    current_batch_start: usize,
    statements: Vec<SqlStatementRange>,
}

impl<'a> Scanner<'a> {
    fn new(text: &'a str, dialect: SqlDialect) -> Self {
        Self {
            text,
            bytes: text.as_bytes(),
            dialect,
            position: 0,
            current_start: None,
            current_code_end: None,
            current_hit_start: 0,
            begin_depth: 0,
            tracks_block_depth: false,
            routine_mode: false,
            delimiter: b";".to_vec(),
            current_batch_index: 0,
            current_batch_start: 0,
            statements: Vec::new(),
        }
    }

    fn scan(mut self) -> Vec<SqlStatementRange> {
        while self.position < self.bytes.len() {
            let offset = self.position;

            if self.skip_trivia() {
                continue;
            }

            let byte = self.bytes[self.position];
            let content_start = self.position;
            if byte == b'\'' {
                self.scan_single_quoted();
                self.mark_content(content_start, self.position);
                continue;
            }
            if byte == b'"' {
                self.scan_double_quoted();
                self.mark_content(content_start, self.position);
                continue;
            }
            if byte == b'`' {
                self.scan_backquoted();
                self.mark_content(content_start, self.position);
                continue;
            }
            if byte == b'[' && self.dialect == SqlDialect::SqlServer {
                self.scan_bracket_quoted();
                self.mark_content(content_start, self.position);
                continue;
            }
            if byte == b'$' && self.dialect == SqlDialect::PostgreSql {
                if let Some(end) = self.dollar_quote_end(self.position) {
                    self.position = end;
                    self.mark_content(content_start, self.position);
                    continue;
                }
            }

            if self.is_at_custom_delimiter() {
                let delimiter_start = self.position;
                self.position += self.delimiter.len();
                self.finish_statement(Some(delimiter_start..self.position));
                continue;
            }

            if byte == b';' && self.delimiter == b";" && self.begin_depth == 0 && !self.routine_mode
            {
                self.position += 1;
                self.finish_statement(Some(offset..self.position));
                continue;
            }

            if is_identifier_byte(byte) {
                let start = self.position;
                self.scan_word();
                self.handle_word(start..self.position);
                continue;
            }

            if self.try_scan_line_boundary(byte) {
                continue;
            }

            self.position += 1;
            self.mark_content(content_start, self.position);
        }

        self.finish_statement(None);
        self.statements
    }

    /// Skip whitespace and comments. Returns true if anything was skipped.
    fn skip_trivia(&mut self) -> bool {
        let start = self.position;
        while self.position < self.bytes.len() {
            let byte = self.bytes[self.position];
            if byte.is_ascii_whitespace() {
                if byte == b'\n' && self.current_start.is_none() {
                    self.current_hit_start = self.position + 1;
                }
                self.position += 1;
            } else if self.is_line_comment_start() {
                let line_end = self.text[self.position..]
                    .find('\n')
                    .map(|index| self.position + index + 1)
                    .unwrap_or(self.bytes.len());
                self.position = line_end;
                if self.current_start.is_none() {
                    self.current_hit_start = self.position;
                }
            } else if self.bytes[self.position..].starts_with(b"/*") {
                let end = self.text[self.position..]
                    .find("*/")
                    .map(|index| self.position + index + 2)
                    .unwrap_or(self.bytes.len());
                self.position = end;
                if self.current_start.is_none() {
                    self.current_hit_start = self.line_after(self.position);
                }
            } else {
                break;
            }
        }

        self.position > start
    }

    fn is_line_comment_start(&self) -> bool {
        if self.bytes[self.position..].starts_with(b"--") {
            return self.dialect != SqlDialect::MySql
                || self
                    .bytes
                    .get(self.position + 2)
                    .is_none_or(|byte| byte.is_ascii_whitespace());
        }
        self.dialect == SqlDialect::MySql
            && self.bytes[self.position] == b'#'
            && self.bytes.get(self.position + 1) != Some(&b'{')
    }

    fn line_after(&self, offset: usize) -> usize {
        let mut result = offset;
        if result > 0 && self.bytes.get(result - 1) == Some(&b'\n') {
            result -= 1;
        }
        result
    }

    fn scan_single_quoted(&mut self) {
        self.position += 1;
        while self.position < self.bytes.len() {
            let byte = self.bytes[self.position];
            if byte == b'\\' && self.dialect == SqlDialect::MySql {
                self.position += 2;
                continue;
            }
            if byte == b'\'' {
                if self.bytes.get(self.position + 1) == Some(&b'\'') {
                    self.position += 2;
                    continue;
                }
                self.position += 1;
                return;
            }
            self.position += 1;
        }
    }

    fn scan_double_quoted(&mut self) {
        self.position += 1;
        while self.position < self.bytes.len() {
            if self.bytes[self.position] == b'"' {
                if self.bytes.get(self.position + 1) == Some(&b'"') {
                    self.position += 2;
                    continue;
                }
                self.position += 1;
                return;
            }
            self.position += 1;
        }
    }

    fn scan_backquoted(&mut self) {
        self.position += 1;
        while self.position < self.bytes.len() {
            if self.bytes[self.position] == b'`' {
                if self.bytes.get(self.position + 1) == Some(&b'`') {
                    self.position += 2;
                    continue;
                }
                self.position += 1;
                return;
            }
            self.position += 1;
        }
    }

    fn scan_bracket_quoted(&mut self) {
        self.position += 1;
        while self.position < self.bytes.len() {
            if self.bytes[self.position] == b']' {
                if self.bytes.get(self.position + 1) == Some(&b']') {
                    self.position += 2;
                    continue;
                }
                self.position += 1;
                return;
            }
            self.position += 1;
        }
    }

    fn dollar_quote_end(&self, start: usize) -> Option<usize> {
        let mut tag_end = start + 1;
        while tag_end < self.bytes.len()
            && (self.bytes[tag_end].is_ascii_alphanumeric() || self.bytes[tag_end] == b'_')
        {
            tag_end += 1;
        }
        if tag_end >= self.bytes.len() || self.bytes[tag_end] != b'$' {
            return None;
        }
        let tag = &self.bytes[start..=tag_end];
        let body_start = tag_end + 1;
        let mut offset = body_start;
        while offset < self.bytes.len() {
            if self.bytes[offset..].starts_with(tag) {
                return Some(offset + tag.len());
            }
            offset += 1;
        }
        None
    }

    fn scan_word(&mut self) {
        self.position += 1;
        while self.position < self.bytes.len() && is_identifier_byte(self.bytes[self.position]) {
            self.position += 1;
        }
    }

    fn handle_word(&mut self, range: Range<usize>) {
        let word = self.text[range.clone()].to_ascii_lowercase();

        if self.dialect == SqlDialect::MySql
            && self.current_start.is_none()
            && word == "delimiter"
            && self.is_at_line_start_before(range.start)
        {
            self.scan_delimiter_directive(range.end);
            return;
        }

        if self.dialect == SqlDialect::SqlServer && word == "go" {
            if let Some((delimiter_end, repeat_count)) = self.parse_go_separator(&range) {
                self.position = delimiter_end;
                self.finish_statement(Some(range.start..delimiter_end));
                self.finish_batch(repeat_count);
                return;
            }
        }

        self.update_block_tracking(&word);
        if self.tracks_block_depth {
            match word.as_str() {
                "begin" | "case" => self.begin_depth += 1,
                "end" => self.begin_depth = self.begin_depth.saturating_sub(1),
                _ => {}
            }
        }
        self.mark_content(range.start, self.position);
    }

    fn parse_go_separator(&self, range: &Range<usize>) -> Option<(usize, usize)> {
        if !self.is_at_line_start_before(range.start) {
            return None;
        }

        let line_end = self.text[range.end..]
            .find('\n')
            .map(|offset| range.end + offset)
            .unwrap_or(self.bytes.len());
        let suffix = self.text[range.end..line_end].trim();
        let repeat_count = if suffix.is_empty() {
            1
        } else {
            suffix.parse::<usize>().ok().filter(|count| *count > 0)?
        };
        Some((line_end, repeat_count))
    }

    fn update_block_tracking(&mut self, word: &str) {
        if self.current_start.is_none() {
            self.tracks_block_depth = word == "begin"
                && (self.dialect == SqlDialect::Oracle
                    || self.starts_mysql_standalone_begin_block());
            return;
        }

        if !self.routine_mode && self.current_statement_is_routine() {
            self.routine_mode = matches!(self.dialect, SqlDialect::Oracle | SqlDialect::SqlServer);
            self.tracks_block_depth = self.routine_mode;
        }
    }

    fn current_statement_is_routine(&self) -> bool {
        let Some(start) = self.current_start else {
            return false;
        };
        matches!(
            statement_kind(self.text, start..self.position, self.dialect),
            SqlStatementKind::Procedure | SqlStatementKind::Function | SqlStatementKind::Trigger
        )
    }

    fn starts_mysql_standalone_begin_block(&self) -> bool {
        if self.dialect != SqlDialect::MySql {
            return false;
        }

        let Some(line_end_offset) = self.text[self.position..].find('\n') else {
            return false;
        };
        let line_end = self.position + line_end_offset;
        self.text[self.position..line_end].trim().is_empty()
    }

    fn scan_delimiter_directive(&mut self, keyword_end: usize) {
        while self
            .bytes
            .get(self.position)
            .is_some_and(|byte| byte.is_ascii_whitespace() && *byte != b'\n')
        {
            self.position += 1;
        }

        let delimiter_start = self.position;
        while self
            .bytes
            .get(self.position)
            .is_some_and(|byte| !byte.is_ascii_whitespace())
        {
            self.position += 1;
        }
        let delimiter_end = self.position;
        if delimiter_end > delimiter_start {
            self.delimiter = self.bytes[delimiter_start..delimiter_end].to_vec();
        }
        self.skip_to_next_line();
        self.current_hit_start = self.position;
        let _ = keyword_end;
    }

    fn try_scan_line_boundary(&mut self, byte: u8) -> bool {
        if byte != b'/' || self.dialect != SqlDialect::Oracle {
            return false;
        }
        let start = self.position;
        if !self.is_at_line_start_before(start) || !self.is_end_of_line(start + 1) {
            return false;
        }
        self.position += 1;
        self.finish_statement(Some(start..self.position));
        true
    }

    fn is_at_line_start_before(&self, offset: usize) -> bool {
        let mut index = offset;
        while index > 0 {
            index -= 1;
            let byte = self.bytes[index];
            if byte == b'\n' {
                return true;
            }
            if byte != b' ' && byte != b'\t' {
                return false;
            }
        }
        true
    }

    fn is_end_of_line(&self, offset: usize) -> bool {
        self.bytes
            .get(offset..)
            .is_some_and(|rest| rest.is_empty() || rest[0].is_ascii_whitespace())
    }

    fn skip_to_next_line(&mut self) {
        while self
            .bytes
            .get(self.position)
            .is_some_and(|byte| *byte != b'\n')
        {
            self.position += 1;
        }
        if self.position < self.bytes.len() {
            self.position += 1;
        }
    }

    fn is_at_custom_delimiter(&self) -> bool {
        self.delimiter != b";"
            && self
                .bytes
                .get(self.position..)
                .is_some_and(|rest| rest.starts_with(&self.delimiter))
    }

    fn mark_content(&mut self, start: usize, end: usize) {
        if self.current_start.is_none() {
            self.current_start = Some(start);
        }
        self.current_code_end = Some(end);
    }

    fn finish_statement(&mut self, delimiter: Option<Range<usize>>) {
        let Some(start) = self.current_start else {
            self.reset_after_boundary(delimiter);
            return;
        };
        let Some(end) = self.current_code_end else {
            self.reset_after_boundary(delimiter);
            return;
        };
        let range = SqlTextRange::new(start, end.max(start));
        self.statements.push(SqlStatementRange {
            hit_start_byte: self.current_hit_start,
            sql_range: range,
            delimiter_range: delimiter.as_ref().map(|range| SqlTextRange {
                start_byte: range.start,
                end_byte: range.end,
            }),
            start_line: line_index(self.text, start),
            kind: statement_kind(self.text, range.start_byte..range.end_byte, self.dialect),
            batch_index: self.current_batch_index,
            batch_repeat_count: 1,
        });
        self.reset_after_boundary(delimiter);
    }

    fn finish_batch(&mut self, repeat_count: usize) {
        for statement in &mut self.statements[self.current_batch_start..] {
            statement.batch_repeat_count = repeat_count;
        }
        self.current_batch_index = self.current_batch_index.saturating_add(1);
        self.current_batch_start = self.statements.len();
    }

    fn reset_after_boundary(&mut self, delimiter: Option<Range<usize>>) {
        self.current_start = None;
        self.current_code_end = None;
        self.begin_depth = 0;
        self.tracks_block_depth = false;
        self.routine_mode = false;
        self.current_hit_start = delimiter
            .map(|range| self.line_after(range.end))
            .unwrap_or(self.bytes.len());
    }
}
