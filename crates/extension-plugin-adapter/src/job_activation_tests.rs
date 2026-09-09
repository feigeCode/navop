use extension_protocol::job::{JobStartResult, JobState, JobStatusResult};

use crate::job_activation::JobActivationManager;

fn started(job_id: &str, state: JobState) -> JobStartResult {
    JobStartResult {
        job_id: job_id.into(),
        state,
    }
}

fn status(job_id: &str, state: JobState) -> JobStatusResult {
    JobStatusResult {
        job_id: job_id.into(),
        state,
        progress_percent: None,
        message: None,
    }
}

#[test]
fn active_generation_registers_and_validates_job() {
    let manager = JobActivationManager::new();
    manager.mark_runtime_active("runtime", 3);
    let handle = manager
        .register_start(
            "extension",
            "runtime",
            3,
            &started("job-1", JobState::Queued),
        )
        .unwrap();

    assert_eq!(JobState::Queued, manager.validate(&handle).unwrap());
    assert_eq!(1, manager.active_count("runtime"));
}

#[test]
fn duplicate_exact_job_is_rejected_but_replacement_can_reuse_id() {
    let manager = JobActivationManager::new();
    manager.mark_runtime_active("runtime", 0);
    manager
        .register_start("extension", "runtime", 0, &started("job", JobState::Queued))
        .unwrap();
    assert!(
        manager
            .register_start(
                "extension",
                "runtime",
                0,
                &started("job", JobState::Running)
            )
            .is_err()
    );

    manager.retire_generation("runtime", 0);
    manager.mark_runtime_active("runtime", 1);
    let replacement = manager
        .register_start("extension", "runtime", 1, &started("job", JobState::Queued))
        .unwrap();
    assert_eq!(1, replacement.generation);
}

#[test]
fn stale_generation_and_terminal_regression_are_rejected() {
    let manager = JobActivationManager::new();
    manager.mark_runtime_active("runtime", 1);
    assert!(
        manager
            .register_start("extension", "runtime", 0, &started("job", JobState::Queued))
            .is_err()
    );
    let handle = manager
        .register_start(
            "extension",
            "runtime",
            1,
            &started("job", JobState::Running),
        )
        .unwrap();
    manager
        .update_status(&handle, &status("job", JobState::Succeeded))
        .unwrap();
    assert!(
        manager
            .update_status(&handle, &status("job", JobState::Running))
            .is_err()
    );
}

#[test]
fn close_is_idempotent_and_returns_job_owned_blobs() {
    let manager = JobActivationManager::new();
    manager.mark_runtime_active("runtime", 0);
    let handle = manager
        .register_start(
            "extension",
            "runtime",
            0,
            &started("job", JobState::Succeeded),
        )
        .unwrap();
    manager.attach_blob(&handle, "blob-b").unwrap();
    manager.attach_blob(&handle, "blob-a").unwrap();

    assert_eq!(
        vec!["blob-a".to_string(), "blob-b".to_string()],
        manager.close(&handle)
    );
    assert!(manager.close(&handle).is_empty());
    assert_eq!(0, manager.active_count("runtime"));
}

#[test]
fn generation_cleanup_does_not_touch_replacement_or_other_runtime() {
    let manager = JobActivationManager::new();
    manager.mark_runtime_active("runtime", 0);
    manager
        .register_start(
            "extension",
            "runtime",
            0,
            &started("old", JobState::Running),
        )
        .unwrap();
    manager.mark_runtime_active("other", 0);
    manager
        .register_start(
            "extension",
            "other",
            0,
            &started("other", JobState::Running),
        )
        .unwrap();

    let removed = manager.retire_generation("runtime", 0);
    assert_eq!(1, removed.len());
    assert_eq!(0, manager.active_count("runtime"));
    assert_eq!(1, manager.active_count("other"));
}

#[test]
fn restart_recovers_jobs_under_new_generation_and_retires_old_blobs() {
    let manager = JobActivationManager::new();
    manager.mark_runtime_active("runtime", 4);
    let old = manager
        .register_start(
            "extension",
            "runtime",
            4,
            &started("job", JobState::Running),
        )
        .unwrap();
    manager.attach_blob(&old, "blob-old").unwrap();

    let recovered = manager.recover_generation("runtime", 4, 5);

    assert_eq!(1, recovered.len());
    assert_eq!(old, recovered[0].previous_handle);
    assert_eq!(5, recovered[0].recovered_handle.generation);
    assert_eq!(vec!["blob-old".to_string()], recovered[0].retired_blob_ids);
    assert!(manager.validate(&old).is_err());
    assert_eq!(
        JobState::Running,
        manager.validate(&recovered[0].recovered_handle).unwrap()
    );
}

#[test]
fn recovery_only_moves_the_exact_active_generation() {
    let manager = JobActivationManager::new();
    manager.mark_runtime_active("runtime", 2);
    let current = manager
        .register_start(
            "extension",
            "runtime",
            2,
            &started("current", JobState::Queued),
        )
        .unwrap();

    assert!(manager.recover_generation("runtime", 1, 3).is_empty());
    assert_eq!(JobState::Queued, manager.validate(&current).unwrap());
}
