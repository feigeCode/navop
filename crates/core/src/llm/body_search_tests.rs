//! `AgentSessionRepository::search_snapshots` 的行为测试。
//!
//! 正文活在快照 JSON 里，这套检索是「SQL 粗筛 + Rust 逐条找」，两半都容易
//! 出岔子（`LIKE` 通配符、JSON 转义、片段边界），所以这里按这两半分别钉。

use super::*;
use crate::storage::migration::run_migrations;
use std::sync::atomic::{AtomicU64, Ordering};

static DB_COUNTER: AtomicU64 = AtomicU64::new(0);

fn test_repository() -> AgentSessionRepository {
    let unique = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("time")
        .as_nanos();
    let counter = DB_COUNTER.fetch_add(1, Ordering::Relaxed);
    let db_path = std::env::temp_dir().join(format!(
        "navop-agent-body-search-{}-{unique}-{counter}.db",
        std::process::id(),
    ));
    let _ = std::fs::remove_file(&db_path);
    let conn = SqliteConnection::open_with_pool_size(&db_path, 1).expect("open sqlite");
    conn.with_connection(|conn| run_migrations(conn))
        .expect("run migrations");
    AgentSessionRepository::new(conn)
}

/// 拼一个最小快照：`history` 里的条目直接给 JSON 字面量。
fn snapshot(history: &str) -> String {
    format!(r#"{{"history":[{history}],"workspace_root":"/tmp/ws"}}"#)
}

fn user_item(text: &str) -> String {
    format!(r#"{{"User":{{"text":{text:?},"images":[]}}}}"#)
}

#[test]
fn finds_the_message_that_contains_the_query_and_reports_a_snippet() {
    let repo = test_repository();
    repo.save_snapshot(
        "sess_budget",
        "预算审批",
        &snapshot(&format!(
            "{},{}",
            user_item("帮我看看这个月的预算"),
            r#""助手回答说预算够用""#
        )),
    )
    .expect("save");
    repo.save_snapshot(
        "sess_other",
        "另一个会话",
        &snapshot(&format!("{},{}", user_item("今天天气不错"), r#""确实""#)),
    )
    .expect("save");

    let hits = repo.search_snapshots("预算", 10).expect("search");
    assert_eq!(1, hits.len(), "只该命中含查询词的那一条");
    assert_eq!("sess_budget", hits[0].uid);
    assert_eq!("预算审批", hits[0].title);
    assert!(
        hits[0].snippet.contains("预算"),
        "片段要带命中词，实际 {:?}",
        hits[0].snippet
    );
}

#[test]
fn assistant_replies_are_searchable_too() {
    let repo = test_repository();
    repo.save_snapshot(
        "sess_reply",
        "会话",
        &snapshot(&format!("{},\"结论是换个后端\"", user_item("问一下"))),
    )
    .expect("save");

    let hits = repo.search_snapshots("换个后端", 10).expect("search");
    assert_eq!(1, hits.len(), "裸字符串变体（助手 / 系统）也要能搜到");
}

#[test]
fn like_wildcards_in_the_query_stay_literal() {
    let repo = test_repository();
    repo.save_snapshot(
        "sess_literal",
        "会话",
        &snapshot(&format!("{},{}", user_item("a_b 这种下划线"), r#""ok""#)),
    )
    .expect("save");

    // `_` 在 LIKE 里是「任意一个字符」。要是没转义，`axb` 会假命中。
    assert_eq!(1, repo.search_snapshots("a_b", 10).expect("search").len());
    assert_eq!(
        0,
        repo.search_snapshots("axb", 10).expect("search").len(),
        "`_` 必须当字面量，不能当通配符"
    );
}

#[test]
fn tool_payloads_are_not_searched() {
    let repo = test_repository();
    // 工具调用参数里带着查询词，但没有任何消息文本提到它：这属于噪音，不算命中。
    repo.save_snapshot(
        "sess_tool",
        "会话",
        &snapshot(&format!(
            "{},{{\"ToolCall\":{{\"name\":\"read_file\",\"arguments\":{{\"path\":\"/secrets/api_key.txt\"}}}}}}",
            user_item("读个文件")
        )),
    )
    .expect("save");

    let hits = repo.search_snapshots("api_key", 10).expect("search");
    assert!(hits.is_empty(), "工具参数不该被当成正文命中，实际 {hits:?}");
}

#[test]
fn ascii_case_is_ignored() {
    let repo = test_repository();
    repo.save_snapshot(
        "sess_case",
        "会话",
        &snapshot(&format!("{},{}", user_item("Budget review"), r#""ok""#)),
    )
    .expect("save");

    let hits = repo.search_snapshots("budget", 10).expect("search");
    assert_eq!(1, hits.len());
}

#[test]
fn snippets_collapse_whitespace_and_keep_the_query_readable() {
    let repo = test_repository();
    repo.save_snapshot(
        "sess_multiline",
        "会话",
        &snapshot(&format!(
            "{},{}",
            user_item(&format!(
                "第一行\n\n第二行   带     大量空白 {padding_before} 关键结论 {padding_after}",
                // 命中点左右都留够长度：片段每个方向只取 48 个字符，两边都得超。
                padding_before = "铺".repeat(60),
                padding_after = "垫".repeat(60),
            )),
            r#""ok""#
        )),
    )
    .expect("save");

    let hits = repo.search_snapshots("关键结论", 10).expect("search");
    assert_eq!(1, hits.len());
    let snippet = &hits[0].snippet;
    assert!(
        !snippet.contains('\n') && !snippet.contains("  "),
        "片段要压成一行：{snippet:?}"
    );
    assert!(
        snippet.starts_with('…') && snippet.ends_with('…'),
        "两端都截断时都要有省略号：{snippet:?}"
    );
    assert!(snippet.contains("关键结论"));
}

#[test]
fn recent_sessions_come_first_and_results_are_capped() {
    let repo = test_repository();
    for index in 0..3 {
        repo.save_snapshot(
            &format!("sess_{index}"),
            &format!("会话 {index}"),
            &snapshot(&format!("{},{}", user_item("同样的内容"), r#""ok""#)),
        )
        .expect("save");
    }

    let hits = repo.search_snapshots("同样的内容", 2).expect("search");
    assert_eq!(2, hits.len(), "上限要生效");
    // `updated_at` 同级（同一秒内写入）时顺序不保证，但三条都该能各自被搜到。
    let all = repo.search_snapshots("同样的内容", 10).expect("search");
    assert_eq!(3, all.len());
}

#[test]
fn broken_snapshots_are_skipped_instead_of_failing_the_search() {
    let repo = test_repository();
    repo.save_snapshot("sess_broken", "坏快照", "{not json")
        .expect("save");
    repo.save_snapshot(
        "sess_good",
        "好快照",
        &snapshot(&format!("{},{}", user_item("能找到我"), r#""ok""#)),
    )
    .expect("save");

    let hits = repo.search_snapshots("能找到我", 10).expect("search");
    assert_eq!(1, hits.len(), "坏快照跳过，好的照常返回");
    assert_eq!("sess_good", hits[0].uid);
}

#[test]
fn empty_query_returns_nothing() {
    let repo = test_repository();
    repo.save_snapshot(
        "sess_a",
        "会话",
        &snapshot(&format!("{}", user_item("内容"))),
    )
    .expect("save");

    assert!(repo.search_snapshots("   ", 10).expect("search").is_empty());
    assert!(repo.search_snapshots("内容", 0).expect("search").is_empty());
}
