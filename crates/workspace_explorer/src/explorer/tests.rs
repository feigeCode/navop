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
