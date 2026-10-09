use super::*;
use crate::storage::migration::run_migrations;
use std::sync::atomic::{AtomicU64, Ordering};

static DB_COUNTER: AtomicU64 = AtomicU64::new(0);

fn test_repository() -> AgentUsageRepository {
    let unique = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("time")
        .as_nanos();
    let counter = DB_COUNTER.fetch_add(1, Ordering::Relaxed);
    let db_path = std::env::temp_dir().join(format!(
        "onetcli-agent-usage-history-{}-{unique}-{counter}.db",
        std::process::id(),
    ));
    let _ = std::fs::remove_file(&db_path);
    let conn = SqliteConnection::open_with_pool_size(&db_path, 1).expect("open sqlite");
    conn.with_connection(|conn| run_migrations(conn))
        .expect("run migrations");
    AgentUsageRepository::new(conn)
}

fn sample(uid: &str, used: u64, recorded_at: i64) -> AgentUsageSample {
    AgentUsageSample {
        id: None,
        uid: uid.to_string(),
        used,
        window: Some(200_000),
        model: Some("claude-sonnet-4-5".to_string()),
        recorded_at,
    }
}

#[test]
fn record_and_read_back_preserves_every_field() {
    let repo = test_repository();
    let expected = AgentUsageSample {
        id: None,
        uid: "sess_a".to_string(),
        used: 42_000,
        window: Some(200_000),
        model: Some("gpt-5".to_string()),
        recorded_at: 1_700_000_000,
    };
    let id = repo.record(&expected).expect("record sample");

    let samples = repo.list_since(0).expect("list samples");
    assert_eq!(1, samples.len());
    assert_eq!(Some(id), samples[0].id);
    let read_back = samples[0].clone();
    assert_eq!(expected.uid, read_back.uid);
    assert_eq!(expected.used, read_back.used);
    assert_eq!(expected.window, read_back.window);
    assert_eq!(expected.model, read_back.model);
    assert_eq!(expected.recorded_at, read_back.recorded_at);
}

#[test]
fn window_and_model_round_trip_as_null() {
    let repo = test_repository();
    // 窗口未知时不写 0 也不写假值——读回来必须还是「没有」。
    let mut expected = sample("sess_a", 1_000, 10);
    expected.window = None;
    expected.model = None;
    repo.record(&expected).expect("record sample");

    let samples = repo.list_since(0).expect("list samples");
    assert_eq!(1, samples.len());
    assert_eq!(None, samples[0].window);
    assert_eq!(None, samples[0].model);
}

#[test]
fn list_since_filters_and_orders_by_time() {
    let repo = test_repository();
    repo.record(&sample("sess_a", 10, 100)).expect("t100");
    repo.record(&sample("sess_a", 20, 300)).expect("t300");
    repo.record(&sample("sess_b", 30, 200)).expect("t200");

    let all = repo.list_since(0).expect("list all");
    assert_eq!(
        vec![100, 200, 300],
        all.iter().map(|s| s.recorded_at).collect::<Vec<_>>(),
        "同一批读回来应按时间正序"
    );

    let recent = repo.list_since(200).expect("list recent");
    assert_eq!(
        vec![200, 300],
        recent.iter().map(|s| s.recorded_at).collect::<Vec<_>>(),
        "边界值 `since` 本身要算在内"
    );
}

#[test]
fn latest_for_uid_picks_the_newest_sample() {
    let repo = test_repository();
    repo.record(&sample("sess_a", 10, 100)).expect("old");
    repo.record(&sample("sess_a", 20, 300)).expect("new");
    repo.record(&sample("sess_b", 30, 500))
        .expect("other session");

    let latest = repo.latest_for_uid("sess_a").expect("latest");
    assert_eq!(Some(300), latest.map(|s| s.recorded_at));
    assert_eq!(
        None,
        repo.latest_for_uid("sess_missing").expect("missing"),
        "没有采样的会话应给 None"
    );
}

#[test]
fn latest_per_session_keeps_one_row_per_uid_newest_first() {
    let repo = test_repository();
    repo.record(&sample("sess_a", 10, 100)).expect("a old");
    repo.record(&sample("sess_a", 20, 300)).expect("a new");
    repo.record(&sample("sess_b", 30, 200)).expect("b");
    repo.record(&sample("sess_c", 40, 400)).expect("c");

    let latest = repo.latest_per_session().expect("latest per session");
    assert_eq!(3, latest.len(), "一个会话只留一行");
    assert_eq!(
        vec!["sess_c", "sess_a", "sess_b"],
        latest.iter().map(|s| s.uid.as_str()).collect::<Vec<_>>(),
        "按最近采样时间倒序"
    );
    assert_eq!(300, latest[1].recorded_at, "每个会话取自己最新的那条");
}

#[test]
fn record_prunes_samples_beyond_the_per_session_cap() {
    let repo = test_repository();
    let total = MAX_SAMPLES_PER_SESSION + 20;
    for index in 0..total {
        repo.record(&sample("sess_a", index as u64, index as i64))
            .expect("record sample");
    }
    repo.record(&sample("sess_b", 1, 1)).expect("other session");

    let samples = repo.list_since(0).expect("list samples");
    let sess_a: Vec<_> = samples.iter().filter(|s| s.uid == "sess_a").collect();
    assert_eq!(MAX_SAMPLES_PER_SESSION, sess_a.len(), "单会话条数要封顶");
    assert_eq!(
        (total - MAX_SAMPLES_PER_SESSION) as i64,
        sess_a[0].recorded_at,
        "裁掉的是最旧的，保留最近的一批"
    );
    assert_eq!(
        1,
        samples.iter().filter(|s| s.uid == "sess_b").count(),
        "裁剪只能作用于被写入的那个会话"
    );
}

#[test]
fn clear_uid_removes_only_that_session() {
    let repo = test_repository();
    repo.record(&sample("sess_a", 10, 100)).expect("a");
    repo.record(&sample("sess_b", 20, 200)).expect("b");

    repo.clear_uid("sess_a").expect("clear");
    let samples = repo.list_since(0).expect("list samples");
    assert_eq!(
        vec!["sess_b"],
        samples.iter().map(|s| s.uid.as_str()).collect::<Vec<_>>()
    );
}
