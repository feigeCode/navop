use std::collections::{BTreeMap, BTreeSet};

use extension_protocol::job::JobState;

use crate::job_activation::{JobActivationHandle, RecoveredJob, RetiredJob};

pub(crate) struct JobRecord {
    pub(crate) state: JobState,
    pub(crate) result_observed: bool,
    pub(crate) blob_ids: BTreeSet<String>,
}

#[derive(Default)]
pub(crate) struct JobActivationState {
    pub(crate) runtime_generations: BTreeMap<String, u64>,
    pub(crate) jobs: BTreeMap<JobActivationHandle, JobRecord>,
}

pub(crate) fn retire_matching(
    state: &mut JobActivationState,
    predicate: impl Fn(&JobActivationHandle) -> bool,
) -> Vec<RetiredJob> {
    matching_handles(state, predicate)
        .into_iter()
        .filter_map(|handle| {
            state.jobs.remove(&handle).map(|record| RetiredJob {
                handle,
                blob_ids: record.blob_ids.into_iter().collect(),
            })
        })
        .collect()
}

pub(crate) fn matching_handles(
    state: &JobActivationState,
    predicate: impl Fn(&JobActivationHandle) -> bool,
) -> Vec<JobActivationHandle> {
    state
        .jobs
        .keys()
        .filter(|handle| predicate(handle))
        .cloned()
        .collect()
}

pub(crate) fn recover_job(
    state: &mut JobActivationState,
    previous_handle: JobActivationHandle,
    recovered_generation: u64,
) -> Option<RecoveredJob> {
    let mut record = state.jobs.remove(&previous_handle)?;
    let recovered_handle = JobActivationHandle {
        generation: recovered_generation,
        ..previous_handle.clone()
    };
    let retired_blob_ids = std::mem::take(&mut record.blob_ids).into_iter().collect();
    state.jobs.insert(recovered_handle.clone(), record);
    Some(RecoveredJob {
        previous_handle,
        recovered_handle,
        retired_blob_ids,
    })
}
