//! Host-owned lifecycle and ownership registry for provider jobs.

use std::{collections::BTreeSet, sync::Arc};

use extension_host::{HostError, HostResult};
use extension_protocol::job::{JobStartResult, JobState, JobStatusResult};
use parking_lot::Mutex;

use crate::job_activation_state::{
    JobActivationState, JobRecord, matching_handles, recover_job, retire_matching,
};

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct JobActivationHandle {
    pub extension_id: String,
    pub runtime_id: String,
    pub generation: u64,
    pub job_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RetiredJob {
    pub handle: JobActivationHandle,
    pub blob_ids: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecoveredJob {
    pub previous_handle: JobActivationHandle,
    pub recovered_handle: JobActivationHandle,
    pub retired_blob_ids: Vec<String>,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum JobActivationError {
    #[error("runtime `{runtime_id}` generation `{generation}` is not active")]
    StaleGeneration { runtime_id: String, generation: u64 },
    #[error("job `{job_id}` is already registered")]
    DuplicateJob { job_id: String },
    #[error("job `{job_id}` is not owned by this runtime generation")]
    UnknownJob { job_id: String },
    #[error("provider returned job `{actual}` while `{expected}` was requested")]
    JobIdMismatch { expected: String, actual: String },
    #[error("terminal job `{job_id}` cannot transition from `{from:?}` to `{to:?}`")]
    TerminalRegression {
        job_id: String,
        from: JobState,
        to: JobState,
    },
}

#[derive(Default)]
pub struct JobActivationManager {
    state: Mutex<JobActivationState>,
}

impl JobActivationManager {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn shared(self) -> Arc<Self> {
        Arc::new(self)
    }

    pub fn mark_runtime_active(&self, runtime_id: &str, generation: u64) {
        self.state
            .lock()
            .runtime_generations
            .insert(runtime_id.into(), generation);
    }

    pub fn register_start(
        &self,
        extension_id: &str,
        runtime_id: &str,
        generation: u64,
        result: &JobStartResult,
    ) -> HostResult<JobActivationHandle> {
        let handle = JobActivationHandle {
            extension_id: extension_id.into(),
            runtime_id: runtime_id.into(),
            generation,
            job_id: result.job_id.clone(),
        };
        let mut state = self.state.lock();
        ensure_generation(&state, runtime_id, generation)?;
        if state.jobs.contains_key(&handle) {
            return Err(job_error(JobActivationError::DuplicateJob {
                job_id: result.job_id.clone(),
            }));
        }
        state.jobs.insert(
            handle.clone(),
            JobRecord {
                state: result.state,
                result_observed: false,
                blob_ids: BTreeSet::new(),
            },
        );
        Ok(handle)
    }

    pub fn validate(&self, handle: &JobActivationHandle) -> HostResult<JobState> {
        let state = self.state.lock();
        ensure_generation(&state, &handle.runtime_id, handle.generation)?;
        state
            .jobs
            .get(handle)
            .map(|record| record.state)
            .ok_or_else(|| unknown_job(&handle.job_id))
    }

    pub fn handle(
        &self,
        extension_id: &str,
        runtime_id: &str,
        generation: u64,
        job_id: &str,
    ) -> HostResult<JobActivationHandle> {
        let handle = JobActivationHandle {
            extension_id: extension_id.into(),
            runtime_id: runtime_id.into(),
            generation,
            job_id: job_id.into(),
        };
        self.validate(&handle)?;
        Ok(handle)
    }

    pub fn update_status(
        &self,
        handle: &JobActivationHandle,
        result: &JobStatusResult,
    ) -> HostResult<()> {
        if result.job_id != handle.job_id {
            return Err(job_error(JobActivationError::JobIdMismatch {
                expected: handle.job_id.clone(),
                actual: result.job_id.clone(),
            }));
        }
        let mut state = self.state.lock();
        ensure_generation(&state, &handle.runtime_id, handle.generation)?;
        let record = state
            .jobs
            .get_mut(handle)
            .ok_or_else(|| unknown_job(&handle.job_id))?;
        if is_terminal(record.state) && record.state != result.state {
            return Err(job_error(JobActivationError::TerminalRegression {
                job_id: handle.job_id.clone(),
                from: record.state,
                to: result.state,
            }));
        }
        record.state = result.state;
        Ok(())
    }

    pub fn mark_result_observed(&self, handle: &JobActivationHandle) -> HostResult<()> {
        let mut state = self.state.lock();
        ensure_generation(&state, &handle.runtime_id, handle.generation)?;
        let record = state
            .jobs
            .get_mut(handle)
            .ok_or_else(|| unknown_job(&handle.job_id))?;
        record.result_observed = true;
        Ok(())
    }

    pub fn attach_blob(&self, handle: &JobActivationHandle, blob_id: &str) -> HostResult<()> {
        let mut state = self.state.lock();
        ensure_generation(&state, &handle.runtime_id, handle.generation)?;
        let record = state
            .jobs
            .get_mut(handle)
            .ok_or_else(|| unknown_job(&handle.job_id))?;
        record.blob_ids.insert(blob_id.into());
        Ok(())
    }

    pub fn close(&self, handle: &JobActivationHandle) -> Vec<String> {
        self.state
            .lock()
            .jobs
            .remove(handle)
            .map(|record| record.blob_ids.into_iter().collect())
            .unwrap_or_default()
    }

    pub fn retire_generation(&self, runtime_id: &str, generation: u64) -> Vec<RetiredJob> {
        let mut state = self.state.lock();
        if state.runtime_generations.get(runtime_id) == Some(&generation) {
            state.runtime_generations.remove(runtime_id);
        }
        retire_matching(&mut state, |handle| {
            handle.runtime_id == runtime_id && handle.generation == generation
        })
    }

    pub fn recover_generation(
        &self,
        runtime_id: &str,
        previous_generation: u64,
        recovered_generation: u64,
    ) -> Vec<RecoveredJob> {
        let mut state = self.state.lock();
        if state.runtime_generations.get(runtime_id) != Some(&previous_generation) {
            return Vec::new();
        }
        let previous_handles = matching_handles(&state, |handle| {
            handle.runtime_id == runtime_id && handle.generation == previous_generation
        });
        state
            .runtime_generations
            .insert(runtime_id.into(), recovered_generation);
        previous_handles
            .into_iter()
            .filter_map(|previous_handle| {
                recover_job(&mut state, previous_handle, recovered_generation)
            })
            .collect()
    }

    pub fn remove_runtime(&self, runtime_id: &str) -> Vec<RetiredJob> {
        let mut state = self.state.lock();
        state.runtime_generations.remove(runtime_id);
        retire_matching(&mut state, |handle| handle.runtime_id == runtime_id)
    }

    pub fn remove_extension(&self, extension_id: &str) -> Vec<RetiredJob> {
        let mut state = self.state.lock();
        let runtime_ids = state
            .jobs
            .keys()
            .filter(|handle| handle.extension_id == extension_id)
            .map(|handle| handle.runtime_id.clone())
            .collect::<BTreeSet<_>>();
        for runtime_id in runtime_ids {
            state.runtime_generations.remove(&runtime_id);
        }
        retire_matching(&mut state, |handle| handle.extension_id == extension_id)
    }

    pub fn active_count(&self, runtime_id: &str) -> usize {
        self.state
            .lock()
            .jobs
            .keys()
            .filter(|handle| handle.runtime_id == runtime_id)
            .count()
    }
}

fn ensure_generation(
    state: &JobActivationState,
    runtime_id: &str,
    generation: u64,
) -> HostResult<()> {
    if state.runtime_generations.get(runtime_id) == Some(&generation) {
        Ok(())
    } else {
        Err(job_error(JobActivationError::StaleGeneration {
            runtime_id: runtime_id.into(),
            generation,
        }))
    }
}

fn is_terminal(state: JobState) -> bool {
    matches!(
        state,
        JobState::Succeeded | JobState::Failed | JobState::Cancelled
    )
}

fn unknown_job(job_id: &str) -> HostError {
    job_error(JobActivationError::UnknownJob {
        job_id: job_id.into(),
    })
}

fn job_error(error: JobActivationError) -> HostError {
    use extension_protocol::error::error_codes;
    let code = match error {
        JobActivationError::StaleGeneration { .. } => error_codes::PERMISSION_DENIED,
        JobActivationError::UnknownJob { .. } | JobActivationError::JobIdMismatch { .. } => {
            error_codes::INVALID_PARAMS
        }
        JobActivationError::DuplicateJob { .. } | JobActivationError::TerminalRegression { .. } => {
            error_codes::RESOURCE_BUSY
        }
    };
    HostError::protocol(extension_protocol::ProtocolError::new(
        code,
        error.to_string(),
    ))
}
