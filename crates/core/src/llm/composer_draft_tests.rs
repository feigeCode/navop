use super::*;
use crate::storage::migration::run_migrations;
use std::sync::atomic::{AtomicU64, Ordering};

static DB_COUNTER: AtomicU64 = AtomicU64::new(0);

fn test_repository() -> ComposerDraftRepository {
    let unique = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("time")
        .as_nanos();
    let counter = DB_COUNTER.fetch_add(1, Ordering::Relaxed);
    let db_path = std::env::temp_dir().join(format!(
        "onetcli-composer-draft-{}-{unique}-{counter}.db",
        std::process::id(),
    ));
    let _ = std::fs::remove_file(&db_path);
    let conn = SqliteConnection::open_with_pool_size(&db_path, 1).expect("open sqlite");
    conn.with_connection(run_migrations)
        .expect("run migrations");
    ComposerDraftRepository::new(conn)
}

#[test]
fn save_and_load_round_trips_text_and_attachments() {
    let repo = test_repository();
    let draft = ComposerDraft::new(
        "sess_a",
        "还没发送的半句话",
        r#"[{"id":"1"}]"#,
        1_700_000_000,
    );

    repo.save(&draft).expect("save");

    let loaded = repo.load("sess_a").expect("load").expect("draft");
    assert_eq!(draft, loaded);
}

#[test]
fn saving_twice_overwrites_instead_of_appending() {
    let repo = test_repository();
    repo.save(&ComposerDraft::new("sess_a", "第一版", "[]", 1))
        .expect("save");
    repo.save(&ComposerDraft::new("sess_a", "第二版", "[]", 2))
        .expect("save");

    let loaded = repo.load("sess_a").expect("load").expect("draft");
    assert_eq!("第二版", loaded.text);
    assert_eq!(2, loaded.updated_at);
    assert_eq!(1, repo.drafted_uids().expect("uids").len());
}

#[test]
fn an_empty_draft_deletes_the_row() {
    let repo = test_repository();
    repo.save(&ComposerDraft::new("sess_a", "半句话", "[]", 1))
        .expect("save");

    // 清空输入框：文字空 + 没有附件 → 不留行。
    repo.save(&ComposerDraft::new("sess_a", "   ", "[]", 2))
        .expect("save");

    assert_eq!(None, repo.load("sess_a").expect("load"));
    assert!(repo.drafted_uids().expect("uids").is_empty());
}

#[test]
fn attachments_alone_are_still_a_draft() {
    let repo = test_repository();
    // 只粘了一张图，一个字没打：这也是草稿，标记要亮。
    repo.save(&ComposerDraft::new(
        "sess_a",
        "",
        r#"[{"id":"1","name":"a.png"}]"#,
        1,
    ))
    .expect("save");

    assert_eq!(
        vec!["sess_a".to_string()],
        repo.drafted_uids().expect("uids")
    );
}

#[test]
fn drafted_uids_lists_recent_edits_first_and_ignores_other_sessions() {
    let repo = test_repository();
    repo.save(&ComposerDraft::new("sess_old", "旧的", "[]", 10))
        .expect("save");
    repo.save(&ComposerDraft::new("sess_new", "新的", "[]", 20))
        .expect("save");
    repo.save(&ComposerDraft::new("sess_cleared", "清掉的", "[]", 30))
        .expect("save");
    repo.clear("sess_cleared").expect("clear");

    assert_eq!(
        vec!["sess_new".to_string(), "sess_old".to_string()],
        repo.drafted_uids().expect("uids")
    );
}

#[test]
fn clear_is_idempotent() {
    let repo = test_repository();
    repo.clear("sess_missing").expect("clear missing");
    assert_eq!(None, repo.load("sess_missing").expect("load"));
}
