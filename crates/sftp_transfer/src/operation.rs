use std::{
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

use anyhow::{Result, anyhow};
use gpui::SharedString;
use sftp::{ProgressCallback, TransferCancelled, TransferProgress};

use super::{
    SftpConnectionIdentity, SftpDeleteRemoteExecution, SftpDeleteRemoteRequest,
    SftpDownloadExecution, SftpDownloadRequest, SftpTransferId, SftpTransferOperation,
    SftpTransferProvider, SftpUploadExecution, SftpUploadRequest,
};

const MAX_UPLOAD_ATTEMPTS: usize = 2;
const UPLOAD_RETRY_DELAY: Duration = Duration::from_millis(250);
const PERMANENT_UPLOAD_ERRORS: &[&str] = &[
    "permission denied",
    "authentication",
    "no such file",
    "not found",
    "disk quota",
    "no space left",
    "read-only file system",
];
const TRANSIENT_UPLOAD_ERRORS: &[&str] = &[
    "timeout",
    "timed out",
    "connection reset",
    "connection refused",
    "connection closed",
    "channel closed",
    "broken pipe",
    "unexpected eof",
    "network is unreachable",
    "temporarily unavailable",
    "try again",
];

pub(super) enum TransferRequest {
    Upload(SftpUploadRequest),
    Download(SftpDownloadRequest),
    DeleteRemote(SftpDeleteRemoteRequest),
}

pub(super) enum TransferExecution {
    Upload(SftpUploadExecution),
    Download(SftpDownloadExecution),
    DeleteRemote(SftpDeleteRemoteExecution),
}

impl TransferRequest {
    pub(super) fn connection(&self) -> &SftpConnectionIdentity {
        match self {
            Self::Upload(request) => &request.connection,
            Self::Download(request) => &request.connection,
            Self::DeleteRemote(request) => &request.connection,
        }
    }

    pub(super) fn operation(&self) -> SftpTransferOperation {
        match self {
            Self::Upload(_) => SftpTransferOperation::Upload,
            Self::Download(_) => SftpTransferOperation::Download,
            Self::DeleteRemote(_) => SftpTransferOperation::DeleteRemote,
        }
    }

    pub(super) fn local_path(&self) -> &Path {
        match self {
            Self::Upload(request) => &request.local_path,
            Self::Download(request) => &request.local_path,
            Self::DeleteRemote(_) => Path::new(""),
        }
    }

    pub(super) fn remote_path(&self) -> &str {
        match self {
            Self::Upload(request) => &request.remote_path,
            Self::Download(request) => &request.remote_path,
            Self::DeleteRemote(request) => &request.remote_dir,
        }
    }

    pub(super) fn display_name(&self) -> &str {
        match self {
            Self::Upload(request) => &request.display_name,
            Self::Download(request) => &request.display_name,
            Self::DeleteRemote(request) => &request.display_name,
        }
    }

    pub(super) fn title(&self) -> SharedString {
        match self {
            Self::Upload(request) => request.title.clone(),
            Self::Download(request) => request.title.clone(),
            Self::DeleteRemote(request) => request.title.clone(),
        }
    }

    pub(super) fn task_key(&self) -> Option<SharedString> {
        match self {
            Self::Upload(request) => request.task_key.clone(),
            Self::Download(request) => request.task_key.clone(),
            Self::DeleteRemote(request) => request.task_key.clone(),
        }
    }

    pub(super) fn task_group(&self) -> Option<SharedString> {
        match self {
            Self::Upload(request) => request.task_group.clone(),
            Self::Download(request) => request.task_group.clone(),
            Self::DeleteRemote(request) => request.task_group.clone(),
        }
    }

    pub(super) fn open_folder(&self) -> Option<PathBuf> {
        match self {
            Self::Download(request) if request.is_dir => Some(request.local_path.clone()),
            Self::Download(request) => Some(
                request
                    .local_path
                    .parent()
                    .filter(|path| !path.as_os_str().is_empty())
                    .unwrap_or_else(|| Path::new("."))
                    .to_path_buf(),
            ),
            Self::Upload(_) | Self::DeleteRemote(_) => None,
        }
    }

    pub(super) fn background_kind(&self) -> &'static str {
        match self {
            Self::Upload(_) => "sftp-upload",
            Self::Download(_) => "sftp-download",
            Self::DeleteRemote(_) => "sftp-delete-remote",
        }
    }

    pub(super) fn background_detail(&self) -> String {
        match self {
            Self::Upload(request) => request.remote_path.clone(),
            Self::Download(request) => request.local_path.to_string_lossy().into_owned(),
            Self::DeleteRemote(request) => request.remote_dir.clone(),
        }
    }

    pub(super) fn execution(
        &self,
        id: SftpTransferId,
        cancelled: Arc<std::sync::atomic::AtomicBool>,
    ) -> TransferExecution {
        match self {
            Self::Upload(request) => TransferExecution::Upload(SftpUploadExecution {
                id,
                connection_source: request.connection_source.clone(),
                local_path: request.local_path.clone(),
                remote_path: request.remote_path.clone(),
                is_dir: request.is_dir,
                directory_conflict_policy: request.directory_conflict_policy,
                cancelled,
            }),
            Self::Download(request) => TransferExecution::Download(SftpDownloadExecution {
                id,
                connection_source: request.connection_source.clone(),
                remote_path: request.remote_path.clone(),
                local_path: request.local_path.clone(),
                is_dir: request.is_dir,
                cancelled,
            }),
            Self::DeleteRemote(request) => {
                TransferExecution::DeleteRemote(SftpDeleteRemoteExecution {
                    id,
                    connection_source: request.connection_source.clone(),
                    entries: request.entries.clone(),
                    remote_dir: request.remote_dir.clone(),
                    cancelled,
                })
            }
        }
    }
}

pub(super) async fn execute_transfer(
    provider: Arc<dyn SftpTransferProvider>,
    execution: TransferExecution,
    progress: ProgressCallback,
) -> Result<()> {
    match execution {
        TransferExecution::Upload(execution) => {
            execute_upload_with_retry(provider, execution, progress).await
        }
        TransferExecution::Download(execution) => provider.download(execution, progress).await,
        TransferExecution::DeleteRemote(execution) => {
            provider.delete_remote(execution, progress).await
        }
    }
}

async fn execute_upload_with_retry(
    provider: Arc<dyn SftpTransferProvider>,
    execution: SftpUploadExecution,
    progress: ProgressCallback,
) -> Result<()> {
    let progress: Arc<dyn Fn(TransferProgress) + Send + Sync> = progress.into();
    for attempt in 1..=MAX_UPLOAD_ATTEMPTS {
        let result = provider
            .upload(execution.clone(), clone_progress_callback(&progress))
            .await;
        let Err(error) = result else {
            return Ok(());
        };
        if attempt == MAX_UPLOAD_ATTEMPTS || !is_retryable_upload_error(&error) {
            return final_upload_error(error, attempt);
        }
        ensure_upload_not_cancelled(&execution.cancelled)?;
        tracing::warn!(
            transfer_id = execution.id.as_u64(),
            attempt,
            max_attempts = MAX_UPLOAD_ATTEMPTS,
            error = %error,
            "retrying transient SFTP upload failure with a fresh SFTP channel"
        );
        tokio::time::sleep(UPLOAD_RETRY_DELAY).await;
        ensure_upload_not_cancelled(&execution.cancelled)?;
    }
    unreachable!("upload attempt loop must return")
}

fn clone_progress_callback(
    progress: &Arc<dyn Fn(TransferProgress) + Send + Sync>,
) -> ProgressCallback {
    let progress = Arc::clone(progress);
    Box::new(move |value| progress(value))
}

fn ensure_upload_not_cancelled(cancelled: &AtomicBool) -> Result<()> {
    if cancelled.load(Ordering::Relaxed) {
        Err(TransferCancelled.into())
    } else {
        Ok(())
    }
}

fn is_retryable_upload_error(error: &anyhow::Error) -> bool {
    if error.downcast_ref::<TransferCancelled>().is_some() {
        return false;
    }
    let message = format!("{error:#}").to_ascii_lowercase();
    !PERMANENT_UPLOAD_ERRORS
        .iter()
        .any(|candidate| message.contains(candidate))
        && TRANSIENT_UPLOAD_ERRORS
            .iter()
            .any(|candidate| message.contains(candidate))
}

fn final_upload_error(error: anyhow::Error, attempts: usize) -> Result<()> {
    if attempts == 1 {
        Err(error)
    } else {
        Err(anyhow!(
            "SFTP upload failed after {attempts} attempts; last error: {error:#}"
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::is_retryable_upload_error;
    use anyhow::anyhow;

    #[test]
    fn retry_classifier_accepts_timeout_and_transport_failures() {
        for message in [
            "Timeout",
            "connection reset by peer",
            "Failed to write remote file: broken pipe",
        ] {
            assert!(is_retryable_upload_error(&anyhow!(message)), "{message}");
        }
    }

    #[test]
    fn retry_classifier_rejects_permanent_failures() {
        for message in [
            "Permission denied",
            "No such file",
            "Authentication failed",
            "Timeout while reporting no space left on device",
        ] {
            assert!(!is_retryable_upload_error(&anyhow!(message)), "{message}");
        }
    }
}
