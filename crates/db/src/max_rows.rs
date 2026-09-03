use one_core::storage::DatabaseType;
use sqlparser::dialect::{
    ClickHouseDialect, DuckDbDialect, GenericDialect, MsSqlDialect, MySqlDialect, OracleDialect,
    PostgreSqlDialect, SQLiteDialect,
};
use sqlparser::tokenizer::{Location, Token, Tokenizer, Whitespace};

pub fn apply_query_max_rows(db_type: &DatabaseType, sql: &str, max_rows: usize) -> String {
    if is_oracle_database(db_type) || !might_start_query(sql) {
        return sql.to_string();
    }

    let Some(query) = query_tokens(db_type, sql) else {
        return sql.to_string();
    };

    match db_type {
        DatabaseType::MSSQL => {
            if has_mssql_row_limit(&query.tokens) {
                sql.to_string()
            } else {
                apply_mssql_top(sql, max_rows, &query.tokens)
            }
        }
        // Returned before tokenization; keep this arm for exhaustive matching.
        DatabaseType::Oracle => sql.to_string(),
        DatabaseType::MySQL
        | DatabaseType::SQLite
        | DatabaseType::ClickHouse
        | DatabaseType::TDengine => {
            if has_top_level_keyword(&query.tokens, &["LIMIT"]) {
                sql.to_string()
            } else {
                insert_limit(db_type, sql, max_rows, &query)
            }
        }
        DatabaseType::PostgreSQL | DatabaseType::DuckDB => {
            if has_top_level_keyword(&query.tokens, &["LIMIT", "FETCH"]) {
                sql.to_string()
            } else {
                insert_limit(db_type, sql, max_rows, &query)
            }
        }
        DatabaseType::External { .. } => {
            if has_top_level_keyword(&query.tokens, &["LIMIT"]) {
                sql.to_string()
            } else {
                insert_limit(db_type, sql, max_rows, &query)
            }
        }
    }
}

fn is_oracle_database(db_type: &DatabaseType) -> bool {
    match db_type {
        DatabaseType::Oracle => true,
        DatabaseType::External { driver_id } => driver_id
            .as_bytes()
            .windows(b"oracle".len())
            .any(|window| window.eq_ignore_ascii_case(b"oracle")),
        _ => false,
    }
}

fn might_start_query(sql: &str) -> bool {
    let remaining = sql.trim_start();
    if remaining.starts_with("--") || remaining.starts_with('#') || remaining.starts_with("/*") {
        // Preserve dialect-specific comment handling instead of duplicating a lexer here.
        return true;
    }

    let keyword_end = remaining
        .as_bytes()
        .iter()
        .position(|byte| !byte.is_ascii_alphanumeric() && *byte != b'_')
        .unwrap_or(remaining.len());

    if keyword_end == 0 {
        return false;
    }

    let keyword = &remaining[..keyword_end];
    keyword.eq_ignore_ascii_case("SELECT") || keyword.eq_ignore_ascii_case("WITH")
}

fn query_tokens(db_type: &DatabaseType, sql: &str) -> Option<TokenizedQuery> {
    let query = tokenize_query(db_type, sql)?;
    let first = query.tokens.first()?;
    let starts_query = first.depth == 0
        && (word_eq(&first.token, "SELECT")
            || (word_eq(&first.token, "WITH")
                && query
                    .tokens
                    .iter()
                    .any(|token| token.depth == 0 && word_eq(&token.token, "SELECT"))));
    starts_query.then_some(query)
}

fn has_mssql_row_limit(tokens: &[SqlToken]) -> bool {
    tokens.iter().enumerate().any(|(index, token)| {
        token.depth == 0
            && (is_mssql_top_modifier(tokens, index)
                || (word_eq(&token.token, "FETCH") || word_eq(&token.token, "OFFSET"))
                    && has_clause_argument(tokens, index))
    })
}

fn has_top_level_keyword(tokens: &[SqlToken], keywords: &[&str]) -> bool {
    tokens.iter().enumerate().any(|(index, token)| {
        token.depth == 0
            && keywords
                .iter()
                .any(|keyword| word_eq(&token.token, keyword) && has_clause_argument(tokens, index))
    })
}

fn is_mssql_top_modifier(tokens: &[SqlToken], index: usize) -> bool {
    if !word_eq(&tokens[index].token, "TOP") {
        return false;
    }

    let is_after_select_modifier = index
        .checked_sub(1)
        .and_then(|previous| tokens.get(previous))
        .is_some_and(|token| {
            token.depth == 0
                && (word_eq(&token.token, "SELECT")
                    || word_eq(&token.token, "ALL")
                    || word_eq(&token.token, "DISTINCT"))
        });
    is_after_select_modifier && has_clause_argument(tokens, index)
}

fn has_clause_argument(tokens: &[SqlToken], index: usize) -> bool {
    let Some(next) = tokens.get(index + 1) else {
        return false;
    };

    if matches!(
        next.token,
        Token::Comma | Token::SemiColon | Token::RParen | Token::EOF
    ) {
        return false;
    }

    !matches!(
        &next.token,
        Token::Word(word)
            if word.quote_style.is_none()
                && [
                    "AS",
                    "CONNECT",
                    "EXCEPT",
                    "FETCH",
                    "FOR",
                    "FORMAT",
                    "FROM",
                    "GROUP",
                    "HAVING",
                    "INTERSECT",
                    "JOIN",
                    "LIMIT",
                    "OFFSET",
                    "ORDER",
                    "QUALIFY",
                    "RETURNING",
                    "SETTINGS",
                    "UNION",
                    "WHERE",
                    "WINDOW",
                ]
                .iter()
                .any(|keyword| word.value.eq_ignore_ascii_case(keyword))
    )
}

fn is_for_suffix(tokens: &[SqlToken], index: usize) -> bool {
    let Some(next) = tokens.get(index + 1) else {
        return false;
    };

    matches!(
        &next.token,
        Token::Word(word)
            if word.quote_style.is_none()
                && [
                    "BROWSE", "FETCH", "JSON", "KEY", "NO", "OF", "READ", "SHARE", "UPDATE",
                    "XML",
                ]
                .iter()
                .any(|keyword| word.value.eq_ignore_ascii_case(keyword))
    )
}

fn is_clickhouse_suffix(tokens: &[SqlToken], index: usize, keyword: &str) -> bool {
    word_eq(&tokens[index].token, keyword)
        && has_clause_argument(tokens, index)
        && tokens
            .get(index + 1)
            .is_some_and(|next| matches!(&next.token, Token::Word(_)))
}

fn apply_mssql_top(sql: &str, max_rows: usize, tokens: &[SqlToken]) -> String {
    let Some(select) = tokens
        .iter()
        .position(|token| token.depth == 0 && word_eq(&token.token, "SELECT"))
    else {
        return sql.to_string();
    };
    let insert_after = if tokens.get(select + 1).is_some_and(|token| {
        token.depth == 0 && (word_eq(&token.token, "ALL") || word_eq(&token.token, "DISTINCT"))
    }) {
        select + 1
    } else {
        select
    };
    let index = tokens[insert_after].end;

    let mut rewritten = String::with_capacity(sql.len() + 16);
    rewritten.push_str(&sql[..index]);
    rewritten.push_str(&format!(" TOP ({max_rows})"));
    rewritten.push_str(&sql[index..]);
    rewritten
}

fn insert_limit(
    db_type: &DatabaseType,
    sql: &str,
    max_rows: usize,
    query: &TokenizedQuery,
) -> String {
    let insertion = query
        .tokens
        .iter()
        .enumerate()
        .find(|(index, token)| {
            token.depth == 0
                && (matches!(token.token, Token::SemiColon)
                    || (word_eq(&token.token, "OFFSET") || word_eq(&token.token, "FETCH"))
                        && has_clause_argument(&query.tokens, *index)
                    || (word_eq(&token.token, "FOR") && is_for_suffix(&query.tokens, *index))
                    || (matches!(db_type, DatabaseType::ClickHouse)
                        && (is_clickhouse_suffix(&query.tokens, *index, "SETTINGS")
                            || is_clickhouse_suffix(&query.tokens, *index, "FORMAT"))))
        })
        .map(|(_, token)| token.start)
        .or(query.trailing_line_comment_start);

    match insertion {
        Some(index) => insert_query_clause(sql, index, &format!("LIMIT {max_rows}")),
        None => append_query_clause(sql, &format!("LIMIT {max_rows}")),
    }
}

fn insert_query_clause(sql: &str, index: usize, clause: &str) -> String {
    let prefix = &sql[..index];
    let suffix = &sql[index..];
    let needs_leading_space = !prefix.chars().next_back().is_some_and(char::is_whitespace);
    let needs_trailing_space = !suffix.starts_with(';');

    let mut rewritten = String::with_capacity(sql.len() + clause.len() + 2);
    rewritten.push_str(prefix);
    if needs_leading_space {
        rewritten.push(' ');
    }
    rewritten.push_str(clause);
    if needs_trailing_space {
        rewritten.push(' ');
    }
    rewritten.push_str(suffix);
    rewritten
}

fn append_query_clause(sql: &str, clause: &str) -> String {
    let trimmed_end = sql.trim_end();
    let trailing_ws = &sql[trimmed_end.len()..];
    format!("{trimmed_end} {clause}{trailing_ws}")
}

#[derive(Debug)]
struct SqlToken {
    token: Token,
    depth: usize,
    start: usize,
    end: usize,
}

struct TokenizedQuery {
    tokens: Vec<SqlToken>,
    trailing_line_comment_start: Option<usize>,
}

fn tokenize_query(db_type: &DatabaseType, sql: &str) -> Option<TokenizedQuery> {
    let dialect = tokenizer_dialect(db_type);
    let mut tokenizer = Tokenizer::new(dialect.as_ref(), sql);
    let tokens = tokenizer.tokenize_with_location().ok()?;
    let mut depth = 0usize;
    let mut output = Vec::with_capacity(tokens.len());
    let mut trailing_line_comment_start = None;
    let mut location_cursor = LocationCursor::new(sql);

    for token in tokens {
        match token.token {
            Token::Whitespace(Whitespace::SingleLineComment { .. }) => {
                trailing_line_comment_start = Some(location_cursor.byte_index(token.span.start));
            }
            Token::Whitespace(_) | Token::EOF => {}
            Token::LParen => depth += 1,
            Token::RParen => depth = depth.saturating_sub(1),
            _ => {
                trailing_line_comment_start = None;
                output.push(SqlToken {
                    start: location_cursor.byte_index(token.span.start),
                    end: location_cursor.byte_index(token.span.end),
                    token: token.token,
                    depth,
                });
            }
        }
    }

    Some(TokenizedQuery {
        tokens: output,
        trailing_line_comment_start,
    })
}

fn tokenizer_dialect(db_type: &DatabaseType) -> Box<dyn sqlparser::dialect::Dialect> {
    match db_type {
        DatabaseType::MySQL => Box::new(MySqlDialect {}),
        DatabaseType::PostgreSQL => Box::new(PostgreSqlDialect {}),
        DatabaseType::SQLite => Box::new(SQLiteDialect {}),
        DatabaseType::DuckDB => Box::new(DuckDbDialect {}),
        DatabaseType::MSSQL => Box::new(MsSqlDialect {}),
        DatabaseType::Oracle => Box::new(OracleDialect {}),
        DatabaseType::ClickHouse => Box::new(ClickHouseDialect {}),
        // TDengine 方言与 MySQL 一致(反引号引用、LIMIT)。
        DatabaseType::TDengine => Box::new(MySqlDialect {}),
        DatabaseType::External { .. } => Box::new(GenericDialect {}),
    }
}

fn word_eq(token: &Token, expected: &str) -> bool {
    matches!(
        token,
        Token::Word(word)
            if word.quote_style.is_none() && word.value.eq_ignore_ascii_case(expected)
    )
}

struct LocationCursor<'a> {
    sql: &'a str,
    byte_index: usize,
    line: u64,
    column: u64,
}

impl<'a> LocationCursor<'a> {
    fn new(sql: &'a str) -> Self {
        Self {
            sql,
            byte_index: 0,
            line: 1,
            column: 1,
        }
    }

    fn byte_index(&mut self, target: Location) -> usize {
        if target.line == 0 || target.column == 0 {
            return self.sql.len();
        }

        while self.byte_index < self.sql.len()
            && (self.line < target.line
                || (self.line == target.line && self.column < target.column))
        {
            let ch = self.sql[self.byte_index..]
                .chars()
                .next()
                .expect("byte index must point to a character");
            self.byte_index += ch.len_utf8();
            if ch == '\n' {
                self.line += 1;
                self.column = 1;
            } else {
                self.column += 1;
            }
        }

        if self.line == target.line && self.column == target.column {
            self.byte_index
        } else {
            self.sql.len()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn adds_limit_for_limit_dialects() {
        assert_eq!(
            apply_query_max_rows(&DatabaseType::MySQL, "SELECT * FROM users", 10),
            "SELECT * FROM users LIMIT 10"
        );
    }

    #[test]
    fn keeps_existing_top_level_row_limits() {
        for sql in [
            "SELECT * FROM users LIMIT 5",
            "SELECT * FROM users FETCH FIRST 5 ROWS ONLY",
        ] {
            assert_eq!(
                apply_query_max_rows(&DatabaseType::PostgreSQL, sql, 10),
                sql
            );
        }
    }

    #[test]
    fn ignores_limit_inside_string_comment_and_cte() {
        for sql in [
            "SELECT 'limit 1' AS text FROM users",
            "SELECT * FROM users /* LIMIT 1 */",
            "WITH recent AS (SELECT * FROM users LIMIT 1) SELECT * FROM recent",
        ] {
            assert_eq!(
                apply_query_max_rows(&DatabaseType::PostgreSQL, sql, 25),
                format!("{sql} LIMIT 25")
            );
        }
    }

    #[test]
    fn does_not_treat_rownum_identifier_as_a_limit_clause() {
        assert_eq!(
            apply_query_max_rows(
                &DatabaseType::MySQL,
                "SELECT rownum FROM imported_oracle_data",
                25
            ),
            "SELECT rownum FROM imported_oracle_data LIMIT 25"
        );
    }

    #[test]
    fn limits_cte_query_but_not_cte_update() {
        let query = "WITH recent AS (SELECT * FROM users) SELECT * FROM recent";
        assert_eq!(
            apply_query_max_rows(&DatabaseType::PostgreSQL, query, 25),
            format!("{query} LIMIT 25")
        );

        let update = "WITH recent AS (SELECT * FROM users) UPDATE users SET active = true";
        assert_eq!(
            apply_query_max_rows(&DatabaseType::PostgreSQL, update, 25),
            update
        );
    }

    #[test]
    fn inserts_limit_before_suffix_clauses() {
        assert_eq!(
            apply_query_max_rows(
                &DatabaseType::PostgreSQL,
                "SELECT * FROM users OFFSET 5",
                10
            ),
            "SELECT * FROM users LIMIT 10 OFFSET 5"
        );
        assert_eq!(
            apply_query_max_rows(
                &DatabaseType::PostgreSQL,
                "SELECT * FROM users FOR UPDATE",
                10
            ),
            "SELECT * FROM users LIMIT 10 FOR UPDATE"
        );
        assert_eq!(
            apply_query_max_rows(
                &DatabaseType::ClickHouse,
                "SELECT * FROM events FORMAT JSON",
                10
            ),
            "SELECT * FROM events LIMIT 10 FORMAT JSON"
        );
        assert_eq!(
            apply_query_max_rows(
                &DatabaseType::ClickHouse,
                "SELECT * FROM events SETTINGS max_threads = 2",
                10
            ),
            "SELECT * FROM events LIMIT 10 SETTINGS max_threads = 2"
        );
    }

    #[test]
    fn does_not_treat_mysql_format_function_as_clickhouse_suffix() {
        assert_eq!(
            apply_query_max_rows(
                &DatabaseType::MySQL,
                "SELECT FORMAT(price, 2) FROM sales",
                10
            ),
            "SELECT FORMAT(price, 2) FROM sales LIMIT 10"
        );
    }

    #[test]
    fn does_not_treat_clause_keywords_used_as_identifiers_as_suffixes() {
        for sql in [
            "SELECT offset FROM users",
            "SELECT * FROM users ORDER BY offset",
            "SELECT fetch FROM users",
            "SELECT limit FROM users",
        ] {
            assert_eq!(
                apply_query_max_rows(&DatabaseType::PostgreSQL, sql, 10),
                format!("{sql} LIMIT 10")
            );
        }

        for sql in ["SELECT format FROM events", "SELECT settings FROM events"] {
            assert_eq!(
                apply_query_max_rows(&DatabaseType::ClickHouse, sql, 10),
                format!("{sql} LIMIT 10")
            );
        }

        for sql in ["SELECT offset FROM users", "SELECT fetch FROM users"] {
            assert_eq!(
                apply_query_max_rows(&DatabaseType::MSSQL, sql, 10),
                sql.replacen("SELECT", "SELECT TOP (10)", 1)
            );
        }
    }

    #[test]
    fn preserves_semicolon_and_trailing_whitespace() {
        assert_eq!(
            apply_query_max_rows(&DatabaseType::SQLite, "SELECT * FROM users;  \n", 10),
            "SELECT * FROM users LIMIT 10;  \n"
        );
    }

    #[test]
    fn inserts_limit_before_trailing_line_comment() {
        assert_eq!(
            apply_query_max_rows(
                &DatabaseType::PostgreSQL,
                "SELECT * FROM users -- keep this comment\n",
                10
            ),
            "SELECT * FROM users LIMIT 10 -- keep this comment\n"
        );
    }

    #[test]
    fn adds_mssql_top_after_select_modifier() {
        assert_eq!(
            apply_query_max_rows(&DatabaseType::MSSQL, "SELECT * FROM users", 10),
            "SELECT TOP (10) * FROM users"
        );
        assert_eq!(
            apply_query_max_rows(&DatabaseType::MSSQL, "SELECT DISTINCT id FROM users", 10),
            "SELECT DISTINCT TOP (10) id FROM users"
        );
        assert_eq!(
            apply_query_max_rows(&DatabaseType::MSSQL, "SELECT ALL id FROM users", 10),
            "SELECT ALL TOP (10) id FROM users"
        );
    }

    #[test]
    fn adds_mssql_top_to_cte_main_query() {
        let sql = "WITH recent AS (SELECT * FROM users) SELECT DISTINCT id FROM recent";
        assert_eq!(
            apply_query_max_rows(&DatabaseType::MSSQL, sql, 10),
            "WITH recent AS (SELECT * FROM users) SELECT DISTINCT TOP (10) id FROM recent"
        );
    }

    #[test]
    fn keeps_mssql_existing_limit_forms() {
        for sql in [
            "SELECT TOP (5) * FROM users",
            "SELECT * FROM users ORDER BY id OFFSET 5 ROWS",
            "SELECT * FROM users ORDER BY id OFFSET 5 ROWS FETCH NEXT 10 ROWS ONLY",
        ] {
            assert_eq!(apply_query_max_rows(&DatabaseType::MSSQL, sql, 10), sql);
        }
    }

    #[test]
    fn leaves_oracle_and_non_queries_unchanged() {
        let select = "SELECT * FROM users";
        assert_eq!(
            apply_query_max_rows(&DatabaseType::Oracle, select, 10),
            select
        );

        let delete = "DELETE FROM users";
        assert_eq!(
            apply_query_max_rows(&DatabaseType::MySQL, delete, 10),
            delete
        );
    }

    #[test]
    fn skips_tokenization_for_non_query_statements() {
        let sql = "UPDATE users SET name = 'unterminated";
        assert_eq!(
            apply_query_max_rows(&DatabaseType::PostgreSQL, sql, 10),
            sql
        );
    }

    #[test]
    fn preserves_unicode_and_multiline_sql_when_adding_limit() {
        let sql = "/* 用户查询 */\nSELECT\n  用户名,\n  '限制 LIMIT 1' AS 文本\nFROM 用户表\nORDER BY 用户名";
        assert_eq!(
            apply_query_max_rows(&DatabaseType::PostgreSQL, sql, 10),
            format!("{sql} LIMIT 10")
        );
    }

    #[test]
    fn keeps_external_driver_behavior() {
        let postgres = DatabaseType::External {
            driver_id: "postgres".to_string(),
        };
        assert_eq!(
            apply_query_max_rows(&postgres, "SELECT * FROM external_table", 100),
            "SELECT * FROM external_table LIMIT 100"
        );

        let oracle = DatabaseType::External {
            driver_id: "oracle-go".to_string(),
        };
        let sql = "SELECT * FROM external_table";
        assert_eq!(apply_query_max_rows(&oracle, sql, 100), sql);
    }
}
