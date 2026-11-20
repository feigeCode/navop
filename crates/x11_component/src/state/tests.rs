use super::{
    note_restart, reset_restart_notifications_for_test, resolution_generation, set_configured_mode,
    take_restart_notification, take_restart_notification_for,
};
use crate::X11ServerMode;

#[test]
fn restart_notification_is_consumed_once() {
    let _guard = crate::test_support::global_state_lock();
    reset_restart_notifications_for_test();
    note_restart("yserver 退出码 1".into());
    let notice = take_restart_notification().expect("应产生结构化通知");
    assert!(notice.generation > 0);
    assert!(notice.reason.contains("退出码 1"));
    assert!(take_restart_notification().is_none());
}

#[test]
fn mode_change_and_restart_advance_resolution_generation() {
    let _guard = crate::test_support::global_state_lock();
    let initial = resolution_generation();
    set_configured_mode(X11ServerMode::External);
    let changed = resolution_generation();
    assert!(changed > initial);
    set_configured_mode(X11ServerMode::External);
    assert_eq!(changed, resolution_generation());
    note_restart("测试 sidecar 重启".into());
    assert!(resolution_generation() > changed);
    let _ = take_restart_notification();
}

#[test]
fn restart_notification_reaches_each_settings_consumer_once() {
    let _guard = crate::test_support::global_state_lock();
    reset_restart_notifications_for_test();
    note_restart("测试 sidecar 崩溃".into());
    let _ = take_restart_notification();
    let first = take_restart_notification_for(1).expect("第一个窗口应收到");
    let second = take_restart_notification_for(2).expect("第二个窗口也应收到");
    assert_eq!(first, second);
    assert!(take_restart_notification_for(1).is_none());
    assert!(take_restart_notification_for(2).is_none());
    reset_restart_notifications_for_test();
}
