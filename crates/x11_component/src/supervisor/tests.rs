use std::sync::Mutex;
use std::time::{Duration, Instant};

use crate::X11ComponentError;
use crate::launch::LaunchSpec;
use crate::lock::ComponentLock;
use crate::restart::RestartBudget;
use crate::runtime::RuntimeIdentity;
use crate::state::{
    embedded_disable_reason, reset_restart_notifications_for_test, take_restart_notification,
};
use x11_forwarding::MagicCookie;

#[test]
fn initial_probe_failure_rotates_endpoint_but_crash_restart_does_not() {
    let occupied = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let port = occupied.local_addr().unwrap().port();
    let spec = LaunchSpec {
        root: std::env::temp_dir(),
        executable: "/unused/yserver".into(),
        version: "manual".into(),
        min_vulkan: "1.3.0".into(),
        identity: RuntimeIdentity::loopback(port, MagicCookie::from_slice(&[7; 16]).unwrap()),
    };
    let decision = super::retry_decision(&spec, None, false, true);
    assert_eq!(decision, super::RetryDecision::Rotate);
    let decision = super::retry_decision(&spec, Some("crash"), false, true);
    assert_eq!(decision, super::RetryDecision::Stop);
}

#[test]
fn active_other_instance_is_never_a_port_conflict() {
    let error = X11ComponentError::ActiveOtherInstance("running".into());
    assert!(matches!(
        super::spawn_retry_decision(&error, true),
        super::RetryDecision::Stop
    ));
    let error = X11ComponentError::Process("bind failed".into());
    assert_eq!(
        super::spawn_retry_decision(&error, true),
        super::RetryDecision::Rotate
    );
}

#[test]
fn endpoint_rotation_keeps_competitors_outside_the_component_lock() {
    let root = std::env::temp_dir().join(format!(
        "x11-rotation-lock-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let identity = RuntimeIdentity::loopback(6010, MagicCookie::from_slice(&[3; 16]).unwrap());
    identity.write(&root.join("runtime.json")).unwrap();
    let mut spec = LaunchSpec {
        root: root.clone(),
        executable: root.join("yserver"),
        version: "manual".into(),
        min_vulkan: "1.3.0".into(),
        identity,
    };
    let lock = ComponentLock::acquire(&root).unwrap();
    let (started_tx, started_rx) = std::sync::mpsc::channel();
    let (locked_tx, locked_rx) = std::sync::mpsc::channel();
    let competitor_root = root.clone();
    let competitor = std::thread::spawn(move || {
        started_tx.send(()).unwrap();
        let _lock = ComponentLock::acquire(&competitor_root).unwrap();
        locked_tx.send(()).unwrap();
    });
    started_rx.recv().unwrap();
    super::rotate_endpoint_locked(&mut spec, &lock).unwrap();
    assert!(
        locked_rx.recv_timeout(Duration::from_millis(100)).is_err(),
        "rotation 完成前另一实例不得进入启动临界区"
    );
    drop(lock);
    locked_rx.recv_timeout(Duration::from_secs(2)).unwrap();
    competitor.join().unwrap();
    std::fs::remove_dir_all(root).unwrap();
}

#[tokio::test]
async fn crash_is_notified_once_when_permanently_disabled() {
    let _guard = crate::test_support::global_state_lock_async().await;
    reset_restart_notifications_for_test();
    let root = std::env::temp_dir().join(format!(
        "x11-restart-disable-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&root).unwrap();
    let spec = LaunchSpec::manual(root.clone(), root.join("yserver")).unwrap();
    let mut budget = RestartBudget::default();
    let now = Instant::now();
    for offset in 0..3 {
        assert!(budget.record_crash(now + Duration::from_secs(offset)));
    }
    let budget = Mutex::new(budget);
    let mut running = None;
    let context = super::RecoveryContext {
        budget: &budget,
        running: &mut running,
    };
    let error = super::recover(context, spec, Some("crash".into()))
        .await
        .unwrap_err();
    assert!(matches!(error, X11ComponentError::EmbeddedDisabled(_)));
    assert!(embedded_disable_reason().is_some());
    let notice = take_restart_notification().expect("永久禁用也必须通知崩溃结果");
    assert!(notice.reason.contains("crash"));
    assert!(notice.reason.contains("永久禁用"));
    assert!(
        take_restart_notification().is_none(),
        "同一次崩溃不得重复通知"
    );
    std::fs::remove_dir_all(root).unwrap();
}
