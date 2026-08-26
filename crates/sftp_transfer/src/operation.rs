use std::{path::Path, sync::Arc};

use anyhow::Result;
use gpui::SharedString;
use sftp::ProgressCallback;

use super::{
    SftpConnectionIdentity, SftpDeleteRemoteExecution, SftpDeleteRemoteRequest,
    SftpDownloadExecution, SftpDownloadRequest, SftpTransferId, SftpTransferOperation,
    SftpTransferProvider, SftpUploadExecution, SftpUploadRequest,
};

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
        TransferExecution::Upload(execution) => provider.upload(execution, progress).await,
        TransferExecution::Download(execution) => provider.download(execution, progress).await,
        TransferExecution::DeleteRemote(execution) => {
            provider.delete_remote(execution, progress).await
        }
    }
}
