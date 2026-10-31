//! Windows 命名管道传输的测试（只能在 Windows 上编译运行）。

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use super::forward::REQUEST_IO_TIMEOUT;
use super::{claim_or_forward, pipe_name_for_config_dir};
use crate::integration::single_instance::{
    SingleInstanceError, SingleInstanceOutcome, StartupRequest,
};

/// 记录主实例收到的启动请求。
type RequestLog = Arc<Mutex<Vec<StartupRequest>>>;

static NEXT_DIR: AtomicUsize = AtomicUsize::new(0);

/// 代替 `tempfile`：app_kit 没有这个 dev-dependency，用进程号 + 计数器造唯一目录。
fn temporary_config_dir() -> PathBuf {
    let id = NEXT_DIR.fetch_add(1, Ordering::Relaxed);
    let dir =
        std::env::temp_dir().join(format!("navop-single-instance-{}-{id}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("temporary config directory");
    dir
}

fn recording_primary(config_dir: &Path, accepted: bool) -> (RequestLog, SingleInstanceOutcome) {
    let received: RequestLog = Arc::new(Mutex::new(Vec::new()));
    let sink = Arc::clone(&received);
    let outcome = claim_or_forward(
        config_dir,
        &StartupRequest::new(Vec::new()),
        move |request| {
            sink.lock().expect("request log lock").push(request);
            accepted
        },
    )
    .expect("primary claim should succeed");
    (received, outcome)
}

fn claim(
    config_dir: &Path,
    paths: Vec<PathBuf>,
) -> Result<SingleInstanceOutcome, SingleInstanceError> {
    claim_or_forward(config_dir, &StartupRequest::new(paths), |_| true)
}

#[test]
fn pipe_names_are_scoped_to_the_product() {
    let app_ids = ["navop", "navop-shell", "navop-data"];
    let names = app_ids.map(super::pipe_name);

    assert!(names.iter().all(|name| name.starts_with(r"\\.\pipe\")));
    let unique = names.iter().collect::<std::collections::HashSet<_>>().len();
    assert_eq!(
        unique,
        app_ids.len(),
        "三个产品必须使用各自的管道：{names:?}"
    );
}

#[test]
fn first_claim_becomes_primary_and_second_one_forwards() {
    let config_dir = temporary_config_dir();
    let (received, outcome) = recording_primary(&config_dir, true);
    assert_eq!(outcome, SingleInstanceOutcome::Primary);

    let forwarded = claim(
        &config_dir,
        vec![PathBuf::from(r"C:\工作区\第二个实例.navop")],
    )
    .expect("second claim must forward instead of failing");

    assert_eq!(forwarded, SingleInstanceOutcome::Forwarded);
    assert_eq!(
        received.lock().expect("request log lock").as_slice(),
        [StartupRequest::new(vec![PathBuf::from(
            r"C:\工作区\第二个实例.navop"
        )])]
    );
    let _ = std::fs::remove_dir_all(&config_dir);
}

#[test]
fn a_rejected_enqueue_must_not_be_acknowledged_as_success() {
    let config_dir = temporary_config_dir();
    // 主实例的启动队列不可用：入队回调一律返回 false。
    let (_, outcome) = recording_primary(&config_dir, false);
    assert_eq!(outcome, SingleInstanceOutcome::Primary);

    let forwarded = claim(&config_dir, Vec::new());

    assert!(
        matches!(forwarded, Err(SingleInstanceError::Forward(_))),
        "a request that never entered the startup queue must fail instead of \
         reporting success: {forwarded:?}"
    );
    let _ = std::fs::remove_dir_all(&config_dir);
}

#[test]
fn half_written_request_is_dropped_without_blocking_later_requests() {
    use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
    use tokio::net::windows::named_pipe::ClientOptions;

    let config_dir = temporary_config_dir();
    let (received, outcome) = recording_primary(&config_dir, true);
    assert_eq!(outcome, SingleInstanceOutcome::Primary);

    let pipe_name = pipe_name_for_config_dir(&config_dir);
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("test runtime");

    // 只发出长度头的一半就停住，主实例的 read_exact 会一直等下去。
    let mut stalled = runtime.block_on(async {
        let mut connection = ClientOptions::new()
            .open(&pipe_name)
            .expect("stalled client should connect");
        connection
            .write_all(&[0u8, 0])
            .await
            .expect("partial header should be writable");
        connection
            .flush()
            .await
            .expect("partial header should flush");
        connection
    });

    // 后续启动请求必须立刻被服务，而不是排在半开连接后面。
    let started = std::time::Instant::now();
    let forwarded = claim(&config_dir, vec![PathBuf::from(r"C:\工作区\later.navop")])
        .expect("later request should still be served");
    let waited = started.elapsed();

    assert_eq!(forwarded, SingleInstanceOutcome::Forwarded);
    assert!(
        waited < REQUEST_IO_TIMEOUT,
        "later request waited for the stalled connection: {waited:?}"
    );
    assert_eq!(received.lock().expect("request log lock").len(), 1);

    // 半开连接最终必须被超时断开；对端关闭时读到 EOF 或连接类错误都算正常。
    let mut buffer = [0u8; 1];
    let dropped = runtime.block_on(async {
        tokio::time::timeout(REQUEST_IO_TIMEOUT * 4, stalled.read(&mut buffer)).await
    });
    match dropped {
        Ok(Ok(0)) => {}
        Ok(Ok(_)) => panic!("primary instance sent unexpected data on a stalled connection"),
        Ok(Err(error)) => assert!(
            matches!(
                error.kind(),
                std::io::ErrorKind::BrokenPipe
                    | std::io::ErrorKind::NotConnected
                    | std::io::ErrorKind::ConnectionAborted
                    | std::io::ErrorKind::ConnectionReset
                    | std::io::ErrorKind::UnexpectedEof
            ),
            "a dropped connection must not surface as {error:?}"
        ),
        Err(_) => panic!("primary instance never timed out the stalled connection"),
    }
    let _ = std::fs::remove_dir_all(&config_dir);
}

#[test]
fn concurrent_claims_produce_exactly_one_primary_instance() {
    let config_dir = temporary_config_dir();

    let outcomes = std::thread::scope(|scope| {
        let handles = (0..4)
            .map(|_| {
                let config_dir = config_dir.clone();
                scope.spawn(move || claim(&config_dir, Vec::new()))
            })
            .collect::<Vec<_>>();

        handles
            .into_iter()
            .map(|handle| handle.join().expect("claim thread"))
            .collect::<Vec<_>>()
    });

    let primaries = outcomes
        .iter()
        .filter(|outcome| matches!(outcome, Ok(SingleInstanceOutcome::Primary)))
        .count();
    let forwarded = outcomes
        .iter()
        .filter(|outcome| matches!(outcome, Ok(SingleInstanceOutcome::Forwarded)))
        .count();

    assert_eq!(
        primaries, 1,
        "exactly one claim may win the primary instance: {outcomes:?}"
    );
    assert_eq!(forwarded, 3, "every other claim must forward: {outcomes:?}");
    let _ = std::fs::remove_dir_all(&config_dir);
}

#[test]
fn claims_are_scoped_to_the_configuration_directory() {
    let first_dir = temporary_config_dir();
    let second_dir = temporary_config_dir();

    assert_eq!(
        claim(&first_dir, Vec::new()).expect("first config directory"),
        SingleInstanceOutcome::Primary
    );
    assert_eq!(
        claim(&second_dir, Vec::new()).expect("second config directory"),
        SingleInstanceOutcome::Primary
    );
    let _ = std::fs::remove_dir_all(&first_dir);
    let _ = std::fs::remove_dir_all(&second_dir);
}
