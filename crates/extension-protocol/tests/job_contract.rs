use extension_protocol::{
    job::{
        JobCancelParams, JobCloseParams, JobResultParams, JobResultResult, JobStartParams,
        JobStartResult, JobState, JobStatusParams, JobStatusResult, ProgressPercent,
    },
    method,
    result_ref::ResultRef,
};

#[test]
fn job_methods_are_stable_public_contracts() {
    assert_eq!("job/start", method::JOB_START);
    assert_eq!("job/status", method::JOB_STATUS);
    assert_eq!("job/cancel", method::JOB_CANCEL);
    assert_eq!("job/result", method::JOB_RESULT);
    assert_eq!("job/close", method::JOB_CLOSE);
    assert!(method::is_known(method::JOB_START));
    assert!(method::is_known(method::JOB_CLOSE));
}

#[test]
fn job_contract_supports_progress_cancellation_and_results() {
    let start = JobStartParams {
        resource_id: Some("resource-1".into()),
        method: "kubernetes/resource/export".into(),
        params: serde_json::json!({"namespace": "default"}),
    };
    assert_eq!(
        start,
        serde_json::from_value(serde_json::to_value(&start).unwrap()).unwrap()
    );

    let started = JobStartResult {
        job_id: "job-1".into(),
        state: JobState::Queued,
    };
    assert_eq!("queued", serde_json::to_value(started).unwrap()["state"]);

    let status = JobStatusResult {
        job_id: "job-1".into(),
        state: JobState::Running,
        progress_percent: Some(ProgressPercent::new(42).unwrap()),
        message: Some("exporting resources".into()),
    };
    let status_json = serde_json::to_value(&status).unwrap();
    assert_eq!(42, status_json["progress_percent"]);
    assert_eq!(
        status,
        serde_json::from_value::<JobStatusResult>(status_json).unwrap()
    );

    for value in [
        serde_json::to_value(JobStatusParams {
            job_id: "job-1".into(),
        })
        .unwrap(),
        serde_json::to_value(JobCancelParams {
            job_id: "job-1".into(),
        })
        .unwrap(),
        serde_json::to_value(JobResultParams {
            job_id: "job-1".into(),
        })
        .unwrap(),
        serde_json::to_value(JobCloseParams {
            job_id: "job-1".into(),
        })
        .unwrap(),
    ] {
        assert_eq!("job-1", value["job_id"]);
    }

    let result = JobResultResult {
        result: ResultRef::Blob {
            id: "blob-export".into(),
        },
    };
    assert_eq!(
        "blob",
        serde_json::to_value(result).unwrap()["result"]["kind"]
    );
}

#[test]
fn job_state_rejects_unknown_wire_values() {
    assert!(serde_json::from_value::<JobState>(serde_json::json!("paused")).is_err());
}

#[test]
fn job_progress_rejects_values_above_one_hundred() {
    assert!(ProgressPercent::new(101).is_err());
    assert!(
        serde_json::from_value::<JobStatusResult>(serde_json::json!({
            "job_id": "job-1",
            "state": "running",
            "progress_percent": 101
        }))
        .is_err()
    );
}
