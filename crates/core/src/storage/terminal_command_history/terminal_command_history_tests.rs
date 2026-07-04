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
fn suggestions_match_case_insensitively_and_prefer_db_ranking() {
    let repo = test_repository();
    let local = TerminalHistoryScope::local();

    repo.record_success(&local, "Git Status", None, Some(0))
        .unwrap();
    repo.record_success(&local, "git stash", None, Some(0))
        .unwrap();
    repo.record_success(&local, "git stash", None, Some(0))
        .unwrap();

    let suggestions = repo.suggestions(&local, "GIT S", 5).unwrap();

    assert_eq!(
        vec!["git stash".to_string(), "Git Status".to_string()],
        suggestions
    );
}

fn commands(items: Vec<TerminalCommandHistory>) -> Vec<String> {
    items.into_iter().map(|item| item.command).collect()
}
