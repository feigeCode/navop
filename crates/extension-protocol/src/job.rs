//! 可取消长任务协议。
//!
//! 请求级 `$/cancelRequest` 只取消一次 in-flight RPC；provider 创建的长任务
//! 使用独立 job 生命周期，以便轮询状态、读取结果和显式释放资源。

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{resource::ResourceId, result_ref::ResultRef};

pub type JobId = String;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "u8", into = "u8")]
pub struct ProgressPercent(u8);

impl ProgressPercent {
    pub fn new(value: u8) -> Result<Self, ProgressPercentError> {
        Self::try_from(value)
    }

    pub fn get(self) -> u8 {
        self.0
    }
}

impl TryFrom<u8> for ProgressPercent {
    type Error = ProgressPercentError;

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        if value <= 100 {
            Ok(Self(value))
        } else {
            Err(ProgressPercentError { value })
        }
    }
}

impl From<ProgressPercent> for u8 {
    fn from(value: ProgressPercent) -> Self {
        value.0
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
#[error("progress percent must be between 0 and 100, got {value}")]
pub struct ProgressPercentError {
    pub value: u8,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum JobState {
    Queued,
    Running,
    Succeeded,
    Failed,
    Cancelled,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct JobStartParams {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resource_id: Option<ResourceId>,
    pub method: String,
    #[serde(default)]
    pub params: Value,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct JobStartResult {
    pub job_id: JobId,
    pub state: JobState,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct JobStatusParams {
    pub job_id: JobId,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct JobStatusResult {
    pub job_id: JobId,
    pub state: JobState,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub progress_percent: Option<ProgressPercent>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct JobCancelParams {
    pub job_id: JobId,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct JobResultParams {
    pub job_id: JobId,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct JobResultResult {
    pub result: ResultRef,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct JobCloseParams {
    pub job_id: JobId,
}
