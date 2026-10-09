use super::*;
use crate::explorer::load::load_workspace;

#[test]
fn non_repository_workspace_snapshot_keeps_the_requested_root() {
    let temp = std::env::temp_dir().join(format!(
        "workspace-explorer-non-repo-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&temp);
    std::fs::create_dir_all(&temp).unwrap();
    std::fs::write(temp.join("readme.txt"), "hello").unwrap();

    let snapshot = load_workspace(temp.clone(), false, false, crate::local_backend()).unwrap();

    assert_eq!(temp.canonicalize().unwrap(), snapshot.root);
    assert!(snapshot.repository.is_none());
    assert_eq!(1, snapshot.entries.len());
    let _ = std::fs::remove_dir_all(temp);
}

#[test]
fn stale_git_results_are_rejected_after_workspace_changes() {
    let current = PathBuf::from("/workspace/current");
    let old = PathBuf::from("/workspace/old");

    let current_identity = identity(4, Some(&current));
    assert!(accepts_git_result(
        current_identity,
        identity(4, Some(&current))
    ));
    assert!(!accepts_git_result(
        current_identity,
        identity(3, Some(&current))
    ));
    assert!(!accepts_git_result(
        current_identity,
        identity(4, Some(&old))
    ));
    assert!(!accepts_git_result(current_identity, identity(4, None)));
}

#[test]
fn repository_root_stays_stable_when_terminal_moves_into_a_subdirectory() {
    let root = PathBuf::from("/workspace/repository");
    let child = root.join("src");
    let sibling = PathBuf::from("/workspace/other");

    assert!(!should_update_root(&root, &root, true));
    assert!(!should_update_root(&root, &child, true));
    assert!(should_update_root(&root, &sibling, true));
    assert!(should_update_root(&root, &child, false));
}

#[test]
fn terminal_root_sync_stops_after_manual_root_selection() {
    let current = PathBuf::from("/workspace/manual");
    let requested = PathBuf::from("/workspace/terminal");

    assert!(!should_sync_terminal_root(
        false, &current, &requested, false
    ));
    assert!(should_sync_terminal_root(true, &current, &requested, false));
}

fn identity<'a>(generation: u64, repository: Option<&'a PathBuf>) -> GitResultIdentity<'a> {
    GitResultIdentity {
        generation,
        repository: repository.map(PathBuf::as_path),
    }
}

fn checkpoint(session_id: &str, turn_id: &str) -> TurnCheckpoint {
    TurnCheckpoint {
        session_id: session_id.to_string(),
        turn_id: turn_id.to_string(),
        commit: format!("commit-{turn_id}"),
        success: true,
    }
}

fn session_turns(
    checkpoints: &HashMap<String, Vec<TurnCheckpoint>>,
    session_id: &str,
) -> Vec<String> {
    checkpoints
        .get(session_id)
        .map(|turns| turns.iter().map(|turn| turn.turn_id.clone()).collect())
        .unwrap_or_default()
}

fn review(session_id: &str, turn_id: &str, diff: &str) -> WorktreeReviewSnapshot {
    WorktreeReviewSnapshot {
        session_id: session_id.to_string(),
        turn_id: turn_id.to_string(),
        diff: diff.to_string(),
        success: true,
    }
}

#[test]
fn a_review_is_found_by_the_turn_it_belongs_to() {
    let reviews = vec![
        review("session-a", "turn-1", "diff-for-1"),
        review("session-b", "turn-1", "diff-for-b-1"),
        review("session-a", "turn-2", "diff-for-2"),
    ];

    assert_eq!(
        Some("diff-for-2"),
        review_for_turn(&reviews, "session-a", "turn-2").map(|review| review.diff.as_str()),
        "第 2 轮要拿第 2 轮的 diff —— 拿最近一轮冒充就是答错"
    );
    assert_eq!(
        Some("diff-for-b-1"),
        review_for_turn(&reviews, "session-b", "turn-1").map(|review| review.diff.as_str()),
        "轮次 id 按会话隔离：别的会话的 turn-1 不能串过来"
    );
    assert_eq!(
        None,
        review_for_turn(&reviews, "session-a", "turn-9"),
        "没有那一轮就直说没有，由调用方决定退到哪一档"
    );
}

#[test]
fn the_review_entry_prefers_the_turn_it_was_asked_for() {
    let reviews = vec![
        review("session-a", "turn-1", "diff-for-1"),
        review("session-a", "turn-2", "diff-for-2"),
        review("session-a", "turn-3", "diff-for-3"),
    ];

    assert_eq!(
        Some("diff-for-1"),
        review_for_open(&reviews, "session-a", Some("turn-1")).map(|r| r.diff.as_str()),
        "点第 1 轮的文件就要第 1 轮的 diff —— 这是「点历史轮看到的是最近一轮」那个 bug 的本体"
    );
    assert_eq!(
        Some("diff-for-3"),
        review_for_open(&reviews, "session-a", None).map(|r| r.diff.as_str()),
        "给不出轮次时才退到最近一轮"
    );
    assert_eq!(
        Some("diff-for-3"),
        review_for_open(&reviews, "session-a", Some("turn-9")).map(|r| r.diff.as_str()),
        "那一轮没留下快照（非 Git 目录、捕获失败）也退到最近一轮，而不是什么都不开"
    );
}

#[test]
fn restoring_a_turn_drops_only_later_reviews_of_the_same_session() {
    let mut reviews = vec![
        review("session-a", "turn-1", "a-1"),
        review("session-b", "turn-1", "b-1"),
        review("session-a", "turn-2", "a-2"),
        review("session-a", "turn-3", "a-3"),
        review("session-b", "turn-2", "b-2"),
    ];

    drop_reviews_after(&mut reviews, "session-a", "turn-2");

    let remaining: Vec<(&str, &str)> = reviews
        .iter()
        .map(|review| (review.session_id.as_str(), review.turn_id.as_str()))
        .collect();
    assert_eq!(
        vec![
            ("session-a", "turn-1"),
            ("session-b", "turn-1"),
            ("session-a", "turn-2"),
            ("session-b", "turn-2"),
        ],
        remaining,
        "回到第 2 轮：第 2 轮自己留着（它就是这次恢复出来的改动），\
         第 3 轮丢掉；别的会话与这条时间线无关，原地保留"
    );
}

#[test]
fn dropping_reviews_for_an_unknown_turn_is_a_no_op() {
    let mut reviews = vec![review("session-a", "turn-1", "a-1")];

    drop_reviews_after(&mut reviews, "session-a", "turn-unknown");
    drop_reviews_after(&mut reviews, "session-b", "turn-1");

    assert_eq!(1, reviews.len(), "误报式的截断不该吃掉任何快照");
}

#[test]
fn truncating_after_a_restored_turn_keeps_that_turn_and_drops_the_rest() {
    let mut checkpoints: HashMap<String, Vec<TurnCheckpoint>> = HashMap::new();
    checkpoints.insert(
        "session-a".into(),
        vec![
            checkpoint("session-a", "turn-1"),
            checkpoint("session-a", "turn-2"),
            checkpoint("session-a", "turn-3"),
        ],
    );

    let dropped = truncate_checkpoints_after(&mut checkpoints, "session-a", "turn-2");

    assert_eq!(
        vec!["turn-3".to_string()],
        dropped,
        "只有目标轮之后的轮次失效，返回给调用方去删 ref"
    );
    assert_eq!(
        vec!["turn-1".to_string(), "turn-2".to_string()],
        session_turns(&checkpoints, "session-a"),
        "回到第 2 轮之后，第 2 轮本身仍是可回滚的"
    );
}

#[test]
fn truncating_an_unknown_session_or_turn_is_a_no_op() {
    let mut checkpoints: HashMap<String, Vec<TurnCheckpoint>> = HashMap::new();
    checkpoints.insert("session-a".into(), vec![checkpoint("session-a", "turn-1")]);

    assert!(truncate_checkpoints_after(&mut checkpoints, "session-b", "turn-1").is_empty());
    assert!(truncate_checkpoints_after(&mut checkpoints, "session-a", "turn-unknown").is_empty());
    assert!(
        truncate_checkpoints_after(&mut checkpoints, "session-a", "turn-1").is_empty(),
        "最后一轮后面没有别的轮次可丢"
    );
    assert_eq!(
        vec!["turn-1".to_string()],
        session_turns(&checkpoints, "session-a"),
        "误报式的截断不该吃掉任何记录"
    );
}
