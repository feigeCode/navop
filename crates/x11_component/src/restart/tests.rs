use std::time::{Duration, Instant};

use super::RestartBudget;

#[test]
fn restart_budget_allows_three_crashes_per_ten_minutes() {
    let start = Instant::now();
    let mut budget = RestartBudget::default();
    assert!(budget.record_crash(start));
    assert!(budget.record_crash(start + Duration::from_secs(1)));
    assert!(budget.record_crash(start + Duration::from_secs(2)));
    assert!(!budget.record_crash(start + Duration::from_secs(3)));
    assert!(!budget.record_crash(start + Duration::from_secs(599)));
    assert!(budget.record_crash(start + Duration::from_secs(601)));
}
