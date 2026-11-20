use super::{MetricEvent, metrics, record};

#[test]
fn metric_events_update_the_public_snapshot() {
    let _guard = crate::test_support::global_state_lock();
    let before = metrics();
    record(MetricEvent::EmbeddedStart);
    record(MetricEvent::Crash);
    record(MetricEvent::Restart);
    record(MetricEvent::AutoExternalFallback);
    record(MetricEvent::EmbeddedFailure);
    let after = crate::metrics();
    assert_eq!(after.embedded_starts, before.embedded_starts + 1);
    assert_eq!(after.crashes, before.crashes + 1);
    assert_eq!(after.restarts, before.restarts + 1);
    assert_eq!(
        after.auto_external_fallbacks,
        before.auto_external_fallbacks + 1
    );
    assert_eq!(after.embedded_failures, before.embedded_failures + 1);
}
