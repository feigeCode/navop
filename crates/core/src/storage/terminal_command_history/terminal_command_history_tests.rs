use super::{
    TerminalCommandHistory, TerminalCommandHistoryRepository, TerminalCommandHistorySort,
    TerminalHistoryScope,
};
use crate::storage::connection::SqliteConnection;
use crate::storage::migration::run_migrations;
use std::sync::atomic::{AtomicU64, Ordering};

static DB_COUNTER: AtomicU64 = AtomicU64::new(0);

fn test_repository() -> TerminalCommandHistoryRepository {
    let unique = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("time")
        .as_nanos();
    let counter = DB_COUNTER.fetch_add(1, Ordering::Relaxed);
    let db_path = std::env::temp_dir().join(format!(
        "onetcli-terminal-command-history-{}-{unique}-{counter}.db",
        std::process::id(),
    ));
    let _ = std::fs::remove_file(&db_path);
    let conn = SqliteConnection::open_with_pool_size(&db_path, 1).expect("open sqlite");
    conn.with_connection(|conn| run_migrations(conn))
        .expect("run migrations");
    TerminalCommandHistoryRepository::new(conn)
}

#[test]
fn successful_command_upsert_deduplicates_and_counts_by_scope() {
    let repo = test_repository();
    let local = TerminalHistoryScope::local();
    let ssh = TerminalHistoryScope::ssh(42);

    let first = repo
        .record_success(&local, " git status ", Some("/repo"), Some(0))
        .unwrap()
        .expect("successful command should be stored");
    let second = repo
        .record_success(&local, "git status", Some("/repo/app"), Some(0))
        .unwrap()
        .expect("duplicate command should be updated");
    repo.record_success(&ssh, "git status", Some("/srv"), Some(0))
        .unwrap();

    assert_eq!(first.id, second.id);
    assert_eq!(2, second.use_count);
    assert_eq!(Some("/repo/app".to_string()), second.cwd);
    assert_eq!(
        1,
        repo.list(&local, TerminalCommandHistorySort::Latest, None, 20)
            .unwrap()
            .len()
    );
    assert_eq!(
        1,
        repo.list(&ssh, TerminalCommandHistorySort::Latest, None, 20)
            .unwrap()
            .len()
    );
}

#[test]
fn record_success_rejects_blank_and_non_success_exit_codes() {
    let repo = test_repository();
    let local = TerminalHistoryScope::local();

    assert!(
        repo.record_success(&local, "   ", None, Some(0))
            .unwrap()
            .is_none()
    );
    assert!(
        repo.record_success(&local, "missing-command", None, Some(127))
            .unwrap()
            .is_none()
    );
    assert!(
        repo.record_success(&local, "unknown-status", None, None)
            .unwrap()
            .is_none()
    );

    assert_eq!(
        0,
        repo.list(&local, TerminalCommandHistorySort::Latest, None, 20)
            .unwrap()
            .len()
    );
}

#[test]
fn delete_command_removes_history_row() {
    let repo = test_repository();
    let local = TerminalHistoryScope::local();

    let item = repo
        .record_success(&local, "git status", None, Some(0))
        .unwrap()
        .expect("successful command should be stored");

    repo.delete_command(item.id.expect("id")).unwrap();

    assert_eq!(
        0,
        repo.list(&local, TerminalCommandHistorySort::Latest, None, 20)
            .unwrap()
            .len()
    );
}

#[test]
fn favorite_survives_upsert_and_pins_sorting() {
    let repo = test_repository();
    let local = TerminalHistoryScope::local();

    let cargo = repo
        .record_success(&local, "cargo test", None, Some(0))
        .unwrap()
        .unwrap();
    repo.record_success(&local, "git status", None, Some(0))
        .unwrap();
    assert!(repo.toggle_favorite(cargo.id.expect("id")).unwrap());
    repo.record_success(&local, "cargo test", Some("/repo"), Some(0))
        .unwrap();

    let latest = repo
        .list(&local, TerminalCommandHistorySort::Latest, None, 20)
        .unwrap();
    assert_eq!("cargo test", latest[0].command);
    assert!(latest[0].favorite);
    assert_eq!(2, latest[0].use_count);
}

#[test]
fn list_supports_most_used_latest_query_and_limit() {
    let repo = test_repository();
    let local = TerminalHistoryScope::local();

    repo.record_success(&local, "git status", None, Some(0))
        .unwrap();
    repo.record_success(&local, "git status", None, Some(0))
        .unwrap();
    repo.record_success(&local, "cargo test", None, Some(0))
        .unwrap();
    repo.record_success(&local, "cargo test", None, Some(0))
        .unwrap();
    repo.record_success(&local, "git commit", None, Some(0))
        .unwrap();

    let most_used = repo
        .list(&local, TerminalCommandHistorySort::MostUsed, Some("git"), 1)
        .unwrap();
    assert_eq!(vec!["git status".to_string()], commands(most_used));

    let latest = repo
        .list(&local, TerminalCommandHistorySort::Latest, Some("git"), 20)
        .unwrap();
    assert_eq!("git commit", latest[0].command);
}

#[test]
fn suggestions_only_return_exact_prefix_completions_and_prefer_db_ranking() {
    let repo = test_repository();
    let local = TerminalHistoryScope::local();

    repo.record_success(&local, "netstat -an", None, Some(0))
        .unwrap();
    repo.record_success(&local, "netstat -rn", None, Some(0))
        .unwrap();
    repo.record_success(&local, "netstat -rn", None, Some(0))
        .unwrap();
    repo.record_success(
        &local,
        "wget https://example.com/netsta/archive",
        None,
        Some(0),
    )
    .unwrap();
    repo.record_success(&local, "find /var/lib -name netstat", None, Some(0))
        .unwrap();
    repo.record_success(&local, "NETSTAT -an", None, Some(0))
        .unwrap();
    repo.record_success(&local, "netsta", None, Some(0))
        .unwrap();

    let suggestions = repo.suggestions(&local, "netsta", 10).unwrap();

    assert_eq!(
        vec!["netstat -rn".to_string(), "netstat -an".to_string()],
        suggestions
    );
}

fn commands(items: Vec<TerminalCommandHistory>) -> Vec<String> {
    items.into_iter().map(|item| item.command).collect()
}

#[test]
fn import_shell_history_only_adds_unknown_commands() {
    // issue #570：从远端 shell 历史文件导入时，只补库里没有的命令，
    // 已存在的条目不能被改写（否则每次连接都会虚增 use_count）。
    let repo = test_repository();
    let scope = TerminalHistoryScope::ssh(7);

    let existing = repo
        .record_success(&scope, "git status", Some("/repo"), Some(0))
        .unwrap()
        .expect("known command is stored");
    assert_eq!(existing.use_count, 1);

    let imported = vec![
        "git status".to_string(),
        "cargo test".to_string(),
        "  ".to_string(),
    ];
    let inserted = repo.import_shell_history(&scope, &imported).unwrap();
    assert_eq!(inserted, 1, "只有 cargo test 是新的");

    let commands = repo
        .list(&scope, TerminalCommandHistorySort::Latest, None, 20)
        .unwrap();
    assert_eq!(commands.len(), 2);

    let git_status = commands
        .iter()
        .find(|entry| entry.command == "git status")
        .expect("existing command stays");
    assert_eq!(git_status.use_count, 1, "导入不虚增计数");
    assert_eq!(git_status.first_used_at, existing.first_used_at);

    assert!(
        commands.iter().any(|entry| entry.command == "cargo test"),
        "新命令被导入"
    );

    // 再次导入同样内容不再插入。
    assert_eq!(repo.import_shell_history(&scope, &imported).unwrap(), 0);
}
