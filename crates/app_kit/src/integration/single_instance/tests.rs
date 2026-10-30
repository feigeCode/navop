//! 单实例的跨平台测试：线上格式、产品隔离、主/次实例握手。

use std::path::PathBuf;

use super::{MAX_PATH_COUNT, MAX_PAYLOAD_BYTES, StartupRequest, codec};

#[test]
fn startup_request_round_trips_empty_and_unicode_paths() {
    for paths in [
        Vec::new(),
        vec![
            PathBuf::from(r"C:\工作区\连接.navop"),
            PathBuf::from("/tmp/工作区/demo.onetcli"),
        ],
    ] {
        let request = StartupRequest::new(paths);
        let encoded = codec::encode_request(&request).expect("request should encode");
        let decoded = codec::decode_request(&encoded).expect("request should decode");

        assert_eq!(request, decoded);
    }
}

#[test]
fn decoder_rejects_truncated_and_oversized_payloads() {
    assert!(codec::decode_request(&[]).is_err());
    assert!(codec::decode_request(&[1, 0, 0]).is_err());

    let mut oversized_count = Vec::new();
    oversized_count.extend_from_slice(&((MAX_PATH_COUNT as u32) + 1).to_le_bytes());
    assert!(codec::decode_request(&oversized_count).is_err());

    let oversized_payload = vec![0; MAX_PAYLOAD_BYTES + 1];
    assert!(codec::decode_request(&oversized_payload).is_err());
}

#[cfg(unix)]
mod unix_transport {
    use std::path::PathBuf;
    use std::sync::Mutex;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::Duration;

    use super::super::{SingleInstanceOutcome, StartupRequest, unix};

    static NEXT_ID: AtomicUsize = AtomicUsize::new(0);

    /// 每个测试用独立 `app_id`，避免共享同一个 socket 互相干扰。
    fn unique_app_id(label: &str) -> String {
        let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
        format!("navop-test-{label}-{}-{id}", std::process::id())
    }

    #[test]
    fn socket_paths_are_scoped_to_the_product_and_are_safe_filenames() {
        let app_ids = ["navop", "navop-shell", "navop-data"];
        let paths = app_ids.map(unix::socket_path);

        assert!(paths.iter().all(|path| {
            path.file_name()
                .is_some_and(|name| name.to_string_lossy().starts_with("navop-"))
        }));
        assert!(
            paths
                .iter()
                .all(|path| path.extension().is_some_and(|ext| ext == "sock"))
        );

        let unique = paths.iter().collect::<std::collections::HashSet<_>>().len();
        assert_eq!(
            unique,
            app_ids.len(),
            "三个产品必须使用各自的 socket：{paths:?}"
        );

        // 带路径分隔符 / 空格的 app_id 不能把 socket 带出运行时目录。
        let tricky = unix::socket_path("navop/evil app");
        assert_eq!(
            tricky.parent(),
            unix::socket_path("navop").parent(),
            "带分隔符的 app_id 不能把 socket 带出运行时目录"
        );
    }

    #[test]
    fn a_second_launch_forwards_its_request_to_the_running_instance() {
        let app_id = unique_app_id("forward");
        let (sender, receiver) = std::sync::mpsc::channel();
        let sender = Mutex::new(sender);

        let primary = super::super::acquire(&app_id, &StartupRequest::new(Vec::new()), move |_| {
            sender.lock().expect("request sink").send(()).is_ok()
        })
        .expect("first launch must become primary");
        assert_eq!(primary, SingleInstanceOutcome::Primary);

        let request = StartupRequest::new(vec![PathBuf::from("/tmp/工作区/再次启动.navop")]);
        let forwarded = super::super::acquire(&app_id, &request, |_| true)
            .expect("second launch must forward instead of starting a new instance");
        assert_eq!(forwarded, SingleInstanceOutcome::Forwarded);

        receiver
            .recv_timeout(Duration::from_secs(2))
            .expect("the running instance should receive the forwarded request");

        let _ = std::fs::remove_file(unix::socket_path(&app_id));
    }
}
