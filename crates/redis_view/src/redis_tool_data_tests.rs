use crate::RedisValue;
use crate::redis_tool_data::{
    build_publish_command, parse_info, quote_command_arg, rows_from_pubsub_values,
    rows_from_slowlog_config_value, rows_from_slowlog_len_value, rows_from_slowlog_value,
    slowlog_reset_command,
};
use crate::redis_tool_pages::build_table_rows;

#[test]
fn parse_info_groups_rows_by_section() {
    let rows = parse_info("# Server\r\nredis_version:8.0\r\n\r\n# Clients\nconnected_clients:2");
    assert_eq!(2, rows.len());
    assert_eq!("Server", rows[0].category);
    assert_eq!("redis_version", rows[0].key);
    assert_eq!("Clients", rows[1].category);
}

#[test]
fn slowlog_rows_extract_id_command_and_detail_fields() {
    let rows = rows_from_slowlog_value(RedisValue::Bulk(vec![RedisValue::Bulk(vec![
        RedisValue::Integer(12),
        RedisValue::Integer(1_700_000_000),
        RedisValue::Integer(42),
        RedisValue::Bulk(vec![
            RedisValue::String("GET".into()),
            RedisValue::String("user:1".into()),
        ]),
        RedisValue::String("127.0.0.1:6379".into()),
    ])]));

    assert_eq!(1, rows.len());
    assert_eq!("12", rows[0].category);
    assert_eq!("GET user:1", rows[0].key);
    assert_eq!("1700000000|42|127.0.0.1:6379", rows[0].value);
}

#[test]
fn pubsub_rows_pair_channels_with_subscriber_counts() {
    let rows = rows_from_pubsub_values(
        RedisValue::Bulk(vec![RedisValue::String("news".into())]),
        RedisValue::Bulk(vec![
            RedisValue::String("news".into()),
            RedisValue::Integer(3),
        ]),
        RedisValue::Integer(1),
        RedisValue::Bulk(vec![RedisValue::String("shard".into())]),
        RedisValue::Bulk(vec![
            RedisValue::String("shard".into()),
            RedisValue::Integer(2),
        ]),
    );

    assert_eq!(3, rows.len());
    assert_eq!("channel", rows[0].category);
    assert_eq!("news", rows[0].key);
    assert_eq!("3", rows[0].value);
    assert_eq!("pattern", rows[1].category);
    assert_eq!("1", rows[1].value);
    assert_eq!("shard_channel", rows[2].category);
    assert_eq!("2", rows[2].value);
}

#[test]
fn quote_command_arg_preserves_safe_values_and_quotes_spaces() {
    assert_eq!("news:1", quote_command_arg("news:1"));
    assert_eq!("\"a b\"", quote_command_arg("a b"));
}

#[test]
fn build_publish_command_quotes_channel_and_message() {
    assert_eq!(
        "PUBLISH \"news room\" \"hello \\\"redis\\\"\"",
        build_publish_command("news room", "hello \"redis\"")
    );
}

#[test]
fn slowlog_reset_command_is_explicit() {
    assert_eq!("SLOWLOG RESET", slowlog_reset_command());
}

#[test]
fn slowlog_len_and_config_values_become_summary_rows() {
    let len_rows = rows_from_slowlog_len_value(RedisValue::Integer(7));
    let config_rows = rows_from_slowlog_config_value(RedisValue::Bulk(vec![
        RedisValue::String("slowlog-log-slower-than".into()),
        RedisValue::String("10000".into()),
        RedisValue::String("slowlog-max-len".into()),
        RedisValue::String("128".into()),
    ]));

    assert_eq!("summary", len_rows[0].category);
    assert_eq!("slowlog_len", len_rows[0].key);
    assert_eq!("7", len_rows[0].value);
    assert_eq!("config", config_rows[0].category);
    assert_eq!("slowlog-log-slower-than", config_rows[0].key);
}

#[test]
fn quote_command_arg_escapes_newlines_and_tabs() {
    // 含空格才会走引号分支
    let quoted = quote_command_arg("a b\nc\td");
    assert_eq!("\"a b\\nc\\td\"", quoted);
}

#[test]
fn quote_command_arg_escapes_carriage_return() {
    let quoted = quote_command_arg("foo\rbar baz");
    assert_eq!("\"foo\\rbar baz\"", quoted);
}

#[test]
fn parse_command_args_decodes_newline_escape_round_trip() {
    use crate::connection::tests::parse_command_args_for_test;
    let original = "hello\nworld\t!";
    let quoted = quote_command_arg(original);
    let cmd = format!("PUBLISH ch {quoted}");
    let parts = parse_command_args_for_test(&cmd);
    assert_eq!(parts.len(), 3);
    assert_eq!(parts[0], "PUBLISH");
    assert_eq!(parts[1], "ch");
    assert_eq!(parts[2], original);
}

#[test]
fn pubsub_table_rows_include_pattern_row() {
    use crate::redis_tool_data::{RedisToolKind, ToolRow};
    let rows = vec![
        ToolRow {
            category: "channel".to_string(),
            key: "news".to_string(),
            value: "3".to_string(),
        },
        ToolRow {
            category: "pattern".to_string(),
            key: "numpat".to_string(),
            value: "1".to_string(),
        },
        ToolRow {
            category: "shard_channel".to_string(),
            key: "shard".to_string(),
            value: "2".to_string(),
        },
    ];
    let cells = build_table_rows(RedisToolKind::PubSub, &rows);
    assert_eq!(cells.len(), 3, "pattern row must be preserved in table");
    assert_eq!(cells[1][0].as_ref(), "pattern");
    assert_eq!(cells[1][1].as_ref(), "numpat");
    assert_eq!(cells[1][2].as_ref(), "1");
}
