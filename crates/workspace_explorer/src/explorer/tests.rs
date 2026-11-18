use super::*;
use crate::explorer::load::load_workspace;
use gpui::TestAppContext;

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
fn a_new_session_has_no_diff_baseline_even_when_another_session_has_one() {
    let mut baselines = HashMap::new();
    baselines.insert("session-a".to_string(), "commit-a".to_string());

    assert_eq!(
        Some("commit-a".to_string()),
        turn_baseline(&baselines, "session-a")
    );
    // 新会话没有上一轮：拿 session-a 的现场当基线，会把它的改动算成「本轮改动」，
    // 于是什么都没改也会弹出审阅。
    assert_eq!(None, turn_baseline(&baselines, "session-b"));
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
fn workspace_header_exposes_hidden_files_toggle() {
    let source = include_str!("header.rs");
    assert!(
        source.contains("\"workspace-toggle-hidden\""),
        "工作区侧边栏顶部操作区必须有隐藏文件开关"
    );
    assert!(
        source.contains("toggle_show_hidden"),
        "顶部开关必须复用现有隐藏文件切换逻辑"
    );
    assert!(
        source.contains("show_hidden_files") && source.contains("hide_hidden_files"),
        "开关提示必须明确当前点击后会显示还是隐藏文件"
    );
}

#[test]
fn blank_workspace_files_area_opens_a_directory_context_menu() {
    let source = include_str!("render.rs");
    assert!(
        source.contains("build_files_context_menu"),
        "工作区文件区应复用目录级菜单，而不是文件项菜单"
    );
    assert!(
        source.contains(".context_menu("),
        "目录级菜单必须接在文件区容器上，空白处才能响应右键"
    );
    assert!(
        source.contains("min_h(px(80.0))"),
        "空目录仍要保留可右键的空白区域"
    );
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
fn only_turns_with_a_real_diff_offer_a_review_entry() {
    let reviews = vec![
        review("session-a", "turn-1", "   \n"),
        review("session-a", "turn-2", "diff-for-2"),
        review("session-b", "turn-1", "diff-for-b-1"),
        review("session-a", "turn-3", "\t"),
    ];

    assert_eq!(
        vec!["turn-2".to_string()],
        reviewed_turn_ids_of(&reviews, "session-a"),
        "首轮没基线、或这一轮没改文件时 diff 是空的：入口摆出来点开只有一片空白，不如不摆"
    );
    assert_eq!(
        vec!["turn-1".to_string()],
        reviewed_turn_ids_of(&reviews, "session-b"),
        "轮次 id 按会话隔离"
    );
    assert!(reviewed_turn_ids_of(&reviews, "session-unknown").is_empty());
}

#[test]
fn opening_a_turn_review_never_falls_back_to_another_turn() {
    let reviews = vec![
        review("session-a", "turn-1", ""),
        review("session-a", "turn-2", "diff-for-2"),
    ];

    assert_eq!(
        Some("diff-for-2"),
        openable_review(&reviews, "session-a", "turn-2").map(|r| r.diff.as_str()),
        "点的那一轮有 diff 就打开它"
    );
    assert_eq!(
        None,
        openable_review(&reviews, "session-a", "turn-1").map(|r| r.diff.as_str()),
        "那一轮的 diff 是空的就认下「没有」，不能拿第 2 轮冒充：页脚写的是「本轮改动」"
    );
    assert!(
        openable_review(&reviews, "session-a", "turn-9").is_none(),
        "压根没有的轮次更不该退到最近一轮（单文件的 `review_for_open` 才允许退）"
    );
    assert!(
        openable_review(&reviews, "session-b", "turn-2").is_none(),
        "别的会话的同号轮次不算数"
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

#[gpui::test]
fn a_new_session_reviews_nothing_after_a_restart(cx: &mut TestAppContext) {
    use std::cell::RefCell;
    use std::rc::Rc;

    cx.update(gpui_component::init);
    let root = initialized_git_repository();
    // 仓库里本来就有未提交改动（用户自己的 WIP 或上一个会话留下的）。
    // 复现的正是这个现场：重启后新开一个会话，只发一句「你好」。
    std::fs::write(root.join("main.rs"), "fn main() { /* earlier */ }\n").unwrap();

    // 上一个会话在重启前结束过一轮，并把自己的基线锚进了仓库。
    let earlier = open_explorer(cx, &root);
    capture_turn(cx, &earlier, "session-earlier", "turn_1");
    assert!(
        earlier.read_with(cx, |explorer, _| explorer.repository().is_some()),
        "测试仓库应被发现"
    );
    drop(earlier);

    // 上一个会话结束之后仓库又变了：期间有人提交、用户自己也改了文件。
    // 这正是「啥都没改」的现场要靠基线判断的地方——那份旧现场不能算成新会话的本轮改动。
    std::fs::write(
        root.join("main.rs"),
        "fn main() { /* changed while nobody was looking */ }\n",
    )
    .unwrap();

    // 重启：同一仓库上重新加载，内存里的基线全丢，只剩仓库里的 ref。
    let explorer = open_explorer(cx, &root);
    let (events, subscription) = cx.update(|cx| {
        let events = Rc::new(RefCell::new(Vec::new()));
        let collected = events.clone();
        let subscription = cx.subscribe(
            &explorer,
            move |_, event: &WorkspaceExplorerEvent, _| {
                if let WorkspaceExplorerEvent::TurnReviewCaptured { session_id, .. } = event {
                    collected.borrow_mut().push(session_id.clone());
                }
            },
        );
        (events, subscription)
    });

    capture_turn(cx, &explorer, "session-new", "turn_1");
    capture_turn(cx, &explorer, "session-new", "turn_2");

    explorer.read_with(cx, |explorer, _| {
        assert!(
            explorer.reviewed_turn_ids("session-new").is_empty(),
            "新会话没有上一轮，不该算出本轮改动: {:?}",
            explorer
                .turn_reviews
                .iter()
                .map(|review| (review.session_id.clone(), review.diff.len()))
                .collect::<Vec<_>>()
        );
        assert_eq!(
            2,
            explorer
                .turn_reviews
                .iter()
                .filter(|review| review.session_id == "session-new")
                .filter(|review| review.diff.trim().is_empty())
                .count(),
            "快照照拍，只是没有内容可审阅"
        );
    });
    assert!(
        events.borrow().is_empty(),
        "没有改动就不该通知宿主弹出审阅: {:?}",
        events.borrow()
    );

    // 重启恢复仍然有效：同一个会话接着跑，期间发生的改动要算进它的本轮。
    drop(subscription);
    capture_turn(cx, &explorer, "session-earlier", "turn_2");
    explorer.read_with(cx, |explorer, _| {
        assert!(
            explorer.reviewed_turn_ids("session-new").is_empty(),
            "新会话不该被旧基线带出改动"
        );
        let diff = explorer
            .turn_reviews
            .iter()
            .find(|review| review.session_id == "session-earlier" && review.turn_id == "turn_2")
            .map(|review| review.diff.clone())
            .expect("重启后的同一会话要能接着算 diff");
        assert!(diff.contains("changed while nobody was looking"), "{diff}");
    });

    let _ = std::fs::remove_dir_all(&root);
}

fn open_explorer(
    cx: &mut TestAppContext,
    root: &std::path::Path,
) -> Entity<WorkspaceExplorer> {
    let root = root.to_path_buf();
    let explorer = cx.update(|cx| {
        let theme = test_theme();
        let editor = cx.new(|_| crate::WorkspaceEditor::new(theme));
        cx.new(|cx| {
            WorkspaceExplorer::new(
                WorkspaceExplorerConfig {
                    root,
                    editor,
                    theme,
                    show_frame_controls: false,
                    backend: None,
                },
                cx,
            )
        })
    });
    // 仓库发现与变更加载都是异步的，等它落定再捕获，否则 `repository` 还是 None。
    cx.run_until_parked();
    explorer
}

fn capture_turn(
    cx: &mut TestAppContext,
    explorer: &Entity<WorkspaceExplorer>,
    session_id: &str,
    turn_id: &str,
) {
    cx.update(|cx| {
        explorer.update(cx, |explorer, cx| {
            explorer.capture_turn_finished(session_id.to_string(), turn_id.to_string(), true, cx)
        })
    });
    cx.run_until_parked();
}

fn test_theme() -> WorkspaceTheme {
    WorkspaceTheme {
        background: gpui::rgb(0x111111).into(),
        foreground: gpui::rgb(0xffffff).into(),
        muted: gpui::rgb(0x222222).into(),
        muted_foreground: gpui::rgb(0x999999).into(),
        border: gpui::rgb(0x333333).into(),
        accent: gpui::rgb(0x444444).into(),
        accent_foreground: gpui::rgb(0xffffff).into(),
        selection: gpui::rgb(0x55a0fc).into(),
        caret: gpui::rgb(0xffffff).into(),
        danger: gpui::rgb(0xff0000).into(),
        warning: gpui::rgb(0xffaa00).into(),
        success: gpui::rgb(0x00aa00).into(),
    }
}

fn initialized_git_repository() -> PathBuf {
    let root = std::env::temp_dir().join(format!(
        "workspace-explorer-review-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    run_test_git(&root, &["init", "-q"]);
    // 固定初始分支名：本机 `init.defaultBranch` 是 main，CI 上未必。
    run_test_git(&root, &["symbolic-ref", "HEAD", "refs/heads/master"]);
    run_test_git(&root, &["config", "core.autocrlf", "false"]);
    run_test_git(&root, &["config", "user.name", "Workspace Explorer Test"]);
    run_test_git(
        &root,
        &["config", "user.email", "workspace-explorer@example.invalid"],
    );
    std::fs::write(root.join("main.rs"), "fn main() {}\n").unwrap();
    run_test_git(&root, &["add", "main.rs"]);
    run_test_git(&root, &["commit", "-q", "-m", "initial"]);
    root
}

fn run_test_git(root: &std::path::Path, args: &[&str]) {
    let output = std::process::Command::new("git")
        .args(args)
        .current_dir(root)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_SYSTEM", "/dev/null")
        .output()
        .expect("git should be installed");
    assert!(
        output.status.success(),
        "git {args:?} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}
