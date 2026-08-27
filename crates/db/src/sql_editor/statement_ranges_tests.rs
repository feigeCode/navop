use super::statement_ranges::*;

fn ranges(sql: &str, dialect: SqlDialect) -> Vec<(usize, usize, usize)> {
    split_sql_statement_ranges(sql, dialect)
        .into_iter()
        .map(|range| {
            (
                range.sql_range.start_byte,
                range.sql_range.end_byte,
                range.hit_start_byte,
            )
        })
        .collect()
}

#[test]
fn splits_top_level_statements_and_ignores_delimiters_in_literals() {
    let sql = "SELECT 1;\nSELECT ';', \"a;b\", `c;d`;";
    assert_eq!(
        ranges(sql, SqlDialect::MySql),
        vec![(0, 8, 0), (10, 34, 10)]
    );
}

#[test]
fn supports_trailing_statement_without_delimiter() {
    let sql = "-- prepare\nSELECT 1;\nSELECT 2";
    assert_eq!(
        ranges(sql, SqlDialect::Standard),
        vec![(11, 19, 11), (21, 29, 21)]
    );
}

#[test]
fn ignores_semicolons_in_comments_and_dollar_quotes() {
    let sql = "SELECT /* ; */ 1;\nSELECT $$\n;\n$$";
    assert_eq!(
        ranges(sql, SqlDialect::PostgreSql),
        vec![(0, 16, 0), (18, 32, 18)]
    );
}

#[test]
fn supports_mysql_delimiter_directive() {
    let sql = "DELIMITER $$\nCREATE PROCEDURE p() BEGIN SELECT 1; END$$\nDELIMITER ;\nSELECT 2;";
    let result = split_sql_statement_ranges(sql, SqlDialect::MySql);
    assert_eq!(result.len(), 2);
    assert_eq!(
        &sql[result[0].sql_range.to_range()],
        "CREATE PROCEDURE p() BEGIN SELECT 1; END"
    );
    assert_eq!(result[0].kind, SqlStatementKind::Procedure);
    assert_eq!(&sql[result[1].sql_range.to_range()], "SELECT 2");
}

#[test]
fn supports_sql_server_go_and_bracket_identifiers() {
    let sql = "SELECT [a;b]\nGO\nSELECT 2";
    let result = split_sql_statement_ranges(sql, SqlDialect::SqlServer);
    assert_eq!(result.len(), 2);
    assert_eq!(&sql[result[0].sql_range.to_range()], "SELECT [a;b]");
    assert_eq!(result[0].delimiter_range.unwrap().start_byte, 13);
    assert_eq!(&sql[result[1].sql_range.to_range()], "SELECT 2");
}

#[test]
fn supports_sql_server_go_repeat_count() {
    let sql = "SELECT 1;\nSELECT 2\nGO 3\nSELECT 4";
    let result = split_sql_statement_ranges(sql, SqlDialect::SqlServer);

    assert_eq!(result.len(), 3);
    assert_eq!(result[0].batch_index, 0);
    assert_eq!(result[1].batch_index, 0);
    assert_eq!(result[0].batch_repeat_count, 3);
    assert_eq!(result[1].batch_repeat_count, 3);
    assert_eq!(result[2].batch_index, 1);
    assert_eq!(result[2].batch_repeat_count, 1);
    assert_eq!(&sql[result[1].delimiter_range.unwrap().to_range()], "GO 3");
}

#[test]
fn supports_sql_server_proc_routines() {
    for sql in [
        "CREATE PROC p AS\nBEGIN\n  SELECT 1;\n  SELECT 2;\nEND\nGO\nSELECT 3;",
        "CREATE OR ALTER PROC p AS\nBEGIN\n  SELECT 1;\n  SELECT 2;\nEND\nGO\nSELECT 3;",
    ] {
        let result = split_sql_statement_ranges(sql, SqlDialect::SqlServer);
        assert_eq!(result.len(), 2, "{sql}");
        assert_eq!(result[0].kind, SqlStatementKind::Procedure, "{sql}");
        assert_eq!(
            &sql[result[0].sql_range.to_range()],
            sql.split("\nGO").next().unwrap(),
            "{sql}"
        );
        assert_eq!(&sql[result[1].sql_range.to_range()], "SELECT 3", "{sql}");
    }
}

#[test]
fn supports_oracle_slash_boundaries_and_begin_blocks() {
    let sql = "BEGIN\n  SELECT 1;\nEND\n/\nSELECT 2;";
    let result = split_sql_statement_ranges(sql, SqlDialect::Oracle);
    assert_eq!(result.len(), 2);
    assert_eq!(
        &sql[result[0].sql_range.to_range()],
        "BEGIN\n  SELECT 1;\nEND"
    );
    assert_eq!(result[0].kind, SqlStatementKind::AnonymousBlock);
    assert_eq!(&sql[result[1].sql_range.to_range()], "SELECT 2");
}

#[test]
fn transaction_begin_is_not_treated_as_anonymous_block() {
    let sql = "BEGIN;\nSELECT 1;\nCOMMIT;\nSELECT 2;";
    for dialect in [
        SqlDialect::Standard,
        SqlDialect::MySql,
        SqlDialect::PostgreSql,
        SqlDialect::SqlServer,
    ] {
        let result = split_sql_statement_ranges(sql, dialect);
        let statements = result
            .iter()
            .map(|statement| &sql[statement.sql_range.to_range()])
            .collect::<Vec<_>>();
        assert_eq!(
            statements,
            vec!["BEGIN", "SELECT 1", "COMMIT", "SELECT 2"],
            "{dialect:?}"
        );
        assert_eq!(result[0].kind, SqlStatementKind::Sql);
    }
}

#[test]
fn supports_mysql_standalone_begin_blocks_without_swallowing_transactions() {
    let sql = "BEGIN;\nSELECT 1;\nCOMMIT;\nBEGIN\n  SELECT 2;\n  SELECT 3;\nEND;\nSELECT 4;";
    let result = split_sql_statement_ranges(sql, SqlDialect::MySql);
    let statements = result
        .iter()
        .map(|statement| &sql[statement.sql_range.to_range()])
        .collect::<Vec<_>>();

    assert_eq!(
        statements,
        vec![
            "BEGIN",
            "SELECT 1",
            "COMMIT",
            "BEGIN\n  SELECT 2;\n  SELECT 3;\nEND",
            "SELECT 4",
        ]
    );
}

#[test]
fn classifies_create_or_replace_routines() {
    for (sql, expected) in [
        (
            "CREATE OR REPLACE PROCEDURE p AS BEGIN NULL; END;\n/",
            SqlStatementKind::Procedure,
        ),
        (
            "CREATE OR REPLACE FUNCTION f RETURN NUMBER AS BEGIN RETURN 1; END;\n/",
            SqlStatementKind::Function,
        ),
        (
            "CREATE OR REPLACE TRIGGER t BEFORE INSERT ON x BEGIN NULL; END;\n/",
            SqlStatementKind::Trigger,
        ),
    ] {
        let result = split_sql_statement_ranges(sql, SqlDialect::Oracle);
        assert_eq!(result.len(), 1, "{sql}");
        assert_eq!(result[0].kind, expected, "{sql}");
    }
}

#[test]
fn cursor_uses_line_boundaries_for_whitespace_ownership() {
    let sql = "SELECT 1;\n  SELECT 2;";
    let snapshot = SqlStatementSnapshot::new(sql, SqlDialect::Standard);
    assert_eq!(snapshot.statement_at_cursor(9).unwrap().start_line, 0);
    assert_eq!(snapshot.statement_at_cursor(10).unwrap().start_line, 1);
    assert_eq!(snapshot.statement_at_cursor(12).unwrap().start_line, 1);
    assert_eq!(snapshot.statement_at_cursor(8).unwrap().start_line, 0);
}

#[test]
fn blank_or_comment_only_document_has_no_statement() {
    assert!(split_sql_statement_ranges("\n  -- comment\n/* ; */", SqlDialect::Standard).is_empty());
}

#[test]
fn cursor_between_statements_ignores_blank_and_comment_only_lines() {
    let sql = "select 1;\n\n-- explain next batch\nselect 2;";
    let snapshot = SqlStatementSnapshot::new(sql, SqlDialect::Standard);

    let blank = sql.find('\n').expect("statement exists") + 1;
    let comment = sql.find("--").expect("comment exists");
    assert!(snapshot.statement_at_cursor(blank).is_none());
    assert!(snapshot.statement_at_cursor(comment).is_none());
}

#[test]
fn cursor_after_delimiter_owns_same_line_gap_but_not_next_statement() {
    // Spec §5.5 rule 4: whitespace / delimiter gap on the same line as a
    // completed delimiter belongs to that statement. A following blank or
    // comment-only line yields None (rule 5), and a same-line batch never
    // steals the next statement's hit range.
    let sql = "SELECT 1; -- trailing\nSELECT 2;";
    let snapshot = SqlStatementSnapshot::new(sql, SqlDialect::Standard);
    // The gap between `;` and the trailing comment is owned by statement 1.
    assert_eq!(snapshot.statement_at_cursor(9).unwrap().start_line, 0);
    assert_eq!(snapshot.statement_at_cursor(10).unwrap().start_line, 0);
    // The comment-only rest of line 1 is still statement 1's line.
    assert_eq!(snapshot.statement_at_cursor(20).unwrap().start_line, 0);
    // Statement 2's line is not swallowed.
    assert_eq!(snapshot.statement_at_cursor(22).unwrap().start_line, 1);

    // Same-line batch: the gap between `;` and the next statement belongs to
    // the next statement, never to the previous one.
    let same_line = "SELECT 1; SELECT 2;";
    let snapshot = SqlStatementSnapshot::new(same_line, SqlDialect::Standard);
    assert_eq!(
        snapshot
            .statement_at_cursor(8)
            .unwrap()
            .sql_range
            .start_byte,
        0
    );
    assert_eq!(
        snapshot
            .statement_at_cursor(9)
            .unwrap()
            .sql_range
            .start_byte,
        10
    );
    assert_eq!(
        snapshot
            .statement_at_cursor(10)
            .unwrap()
            .sql_range
            .start_byte,
        10
    );
}

#[test]
fn mysql_hash_comment_is_statement_delimiter_safe() {
    let sql = "select 1;\n# select ;\nselect 2;";
    let snapshot = SqlStatementSnapshot::new(sql, SqlDialect::MySql);

    assert_eq!(snapshot.statement_ranges().len(), 2);
    assert_eq!(
        snapshot.statement_text(snapshot.statement_ranges().last().unwrap()),
        "select 2"
    );
}

#[test]
fn multibyte_text_keeps_utf8_ranges() {
    let sql = "SELECT '中文';\nSELECT '🙂';";
    let result = split_sql_statement_ranges(sql, SqlDialect::Standard);
    assert_eq!(result.len(), 2);
    assert_eq!(&sql[result[0].sql_range.to_range()], "SELECT '中文'");
    assert_eq!(&sql[result[1].sql_range.to_range()], "SELECT '🙂'");
    assert!(sql.is_char_boundary(result[0].sql_range.end_byte));
}

#[test]
fn finds_statement_starting_on_a_logical_line() {
    let sql = "SELECT 1;\nSELECT 2;";
    let result = split_sql_statement_ranges(sql, SqlDialect::Standard);
    assert_eq!(
        statement_starting_on_line(&result, 1).unwrap().start_line,
        1
    );
    assert!(statement_starting_on_line(&result, 2).is_none());
}
