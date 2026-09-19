use std::sync::{Arc, atomic::Ordering};

use anyhow::{Result, anyhow};
use async_trait::async_trait;
use ftp::FtpClient;
use sftp::{
    ProgressCallback, RemoteFileClient, RusshSftpClient, SftpClient, TransferCancelled,
    TransferProgress,
};

use super::{
    SftpDeleteRemoteExecution, SftpDownloadExecution, SftpUploadConnection, SftpUploadExecution,
};

#[async_trait]
pub trait SftpTransferProvider: Send + Sync {
    async fn upload(
        &self,
        execution: SftpUploadExecution,
        progress: ProgressCallback,
    ) -> Result<()>;

    async fn download(
        &self,
        execution: SftpDownloadExecution,
        progress: ProgressCallback,
    ) -> Result<()>;

    async fn delete_remote(
        &self,
        execution: SftpDeleteRemoteExecution,
        progress: ProgressCallback,
    ) -> Result<()>;
}

pub struct RusshSftpTransferProvider;

/// 按连接来源建立协议无关的远程文件客户端。
///
/// FTP 来源走 `FtpClient`，SSH 来源走 `RusshSftpClient`；provider
/// 内部只依赖 `RemoteFileClient`，不感知底层协议。
pub(crate) async fn connect_remote(
    source: SftpUploadConnection,
) -> Result<Box<dyn RemoteFileClient>> {
    match source {
        SftpUploadConnection::SessionManager(session_manager) => {
            let shared_client = session_manager.client().await?;
            Ok(Box::new(
                RusshSftpClient::connect_with_client(shared_client).await?,
            ))
        }
        SftpUploadConnection::Config(config) => {
            Ok(Box::new(RusshSftpClient::connect(config).await?))
        }
        SftpUploadConnection::Ftp(config) => Ok(Box::new(FtpClient::connect(config).await?)),
    }
}

#[async_trait]
impl SftpTransferProvider for RusshSftpTransferProvider {
    async fn upload(
        &self,
        execution: SftpUploadExecution,
        progress: ProgressCallback,
    ) -> Result<()> {
        let mut client = connect_remote(execution.connection_source).await?;
        let local_path = execution.local_path.to_string_lossy().into_owned();
        tracing::debug!(
            transfer_id = execution.id.as_u64(),
            local_path = %local_path,
            remote_path = %execution.remote_path,
            is_dir = execution.is_dir,
            "starting remote file upload"
        );

        if execution.is_dir {
            client
                .upload_dir_with_progress(
                    &local_path,
                    &execution.remote_path,
                    execution.directory_conflict_policy,
                    execution.cancelled,
                    progress,
                )
                .await
        } else {
            client
                .upload_with_progress(
                    &local_path,
                    &execution.remote_path,
                    execution.cancelled,
                    progress,
                )
                .await
        }
    }

    async fn download(
        &self,
        execution: SftpDownloadExecution,
        progress: ProgressCallback,
    ) -> Result<()> {
        let mut client = connect_remote(execution.connection_source).await?;
        let local_path = execution.local_path.to_string_lossy().into_owned();
        tracing::debug!(
            transfer_id = execution.id.as_u64(),
            local_path = %local_path,
            remote_path = %execution.remote_path,
            is_dir = execution.is_dir,
            "starting remote file download"
        );

        if execution.is_dir {
            client
                .download_dir_with_progress(
                    &execution.remote_path,
                    &local_path,
                    execution.cancelled,
                    progress,
                )
                .await
        } else {
            client
                .download_with_progress(
                    &execution.remote_path,
                    &local_path,
                    execution.cancelled,
                    progress,
                )
                .await
        }
    }

    async fn delete_remote(
        &self,
        execution: SftpDeleteRemoteExecution,
        progress: ProgressCallback,
    ) -> Result<()> {
        let mut client = connect_remote(execution.connection_source.clone()).await?;
        tracing::debug!(
            transfer_id = execution.id.as_u64(),
            remote_dir = %execution.remote_dir,
            entry_count = execution.entries.len(),
            "starting remote file delete"
        );
        delete_remote_entries(&mut *client, &execution, progress).await
    }
}

async fn delete_remote_entries(
    client: &mut dyn RemoteFileClient,
    execution: &SftpDeleteRemoteExecution,
    progress: ProgressCallback,
) -> Result<()> {
    let progress: Arc<dyn Fn(TransferProgress) + Send + Sync> = progress.into();
    let mut errors = Vec::new();
    for (index, entry) in execution.entries.iter().enumerate() {
        check_cancelled(execution)?;
        let result = delete_remote_entry(client, execution, entry, index, progress.clone()).await;
        if let Err(error) = result {
            if error.downcast_ref::<TransferCancelled>().is_some() {
                return Err(error);
            }
            tracing::error!("Failed to delete {}: {}", entry.remote_path, error);
            errors.push(format!("{}: {error}", entry.remote_path));
        }
        check_cancelled(execution)?;
        report_delete_entry_progress(&progress, execution, entry, index);
    }
    if errors.is_empty() {
        Ok(())
    } else {
        Err(anyhow!(
            "failed to delete {} item(s): {}",
            errors.len(),
            errors.join("; ")
        ))
    }
}

async fn delete_remote_entry(
    client: &mut dyn RemoteFileClient,
    execution: &SftpDeleteRemoteExecution,
    entry: &super::SftpRemoteDeleteEntry,
    index: usize,
    progress: Arc<dyn Fn(TransferProgress) + Send + Sync>,
) -> Result<()> {
    if entry.is_dir {
        let total = execution.entries.len() as u64;
        client
            .delete_recursive(
                &entry.remote_path,
                execution.cancelled.clone(),
                Box::new(move |value| {
                    progress(TransferProgress {
                        transferred: index as u64,
                        total,
                        speed: value.speed,
                        current_file: value.current_file,
                        current_file_transferred: value.current_file_transferred,
                        current_file_total: value.current_file_total,
                    })
                }),
            )
            .await
    } else {
        client.delete(&entry.remote_path, false).await
    }
}

fn check_cancelled(execution: &SftpDeleteRemoteExecution) -> Result<()> {
    if execution.cancelled.load(Ordering::Relaxed) {
        Err(anyhow::Error::from(TransferCancelled))
    } else {
        Ok(())
    }
}

fn report_delete_entry_progress(
    progress: &Arc<dyn Fn(TransferProgress) + Send + Sync>,
    execution: &SftpDeleteRemoteExecution,
    entry: &super::SftpRemoteDeleteEntry,
    index: usize,
) {
    progress(TransferProgress {
        transferred: (index + 1) as u64,
        total: execution.entries.len() as u64,
        speed: 0.0,
        current_file: Some(entry.remote_path.clone()),
        current_file_transferred: 1,
        current_file_total: 1,
    });
}

#[cfg(test)]
mod tests {
    use super::{connect_remote, SftpUploadConnection};
    use ftp::FtpConnectConfig;

    #[tokio::test]
    async fn transfer_provider_routes_ftp_source_to_ftp_client() {
        // passive_mode=false 会让 FtpClient::connect 在发起任何网络请求前
        // 直接拒绝主动模式；错误信息匹配即证明 FTP 来源被路由到了
        // FtpClient，而不是 SSH 路径或 "not implemented" 兜底。
        let source = SftpUploadConnection::Ftp(FtpConnectConfig {
            host: "127.0.0.1".to_string(),
            port: 21,
            username: "user".to_string(),
            password: String::new(),
            passive_mode: false,
            use_tls: false,
            connect_timeout: Some(1),
        });
        let result = connect_remote(source).await;
        let Err(error) = result else {
            panic!("active mode should be rejected before any network I/O");
        };
        assert!(error.to_string().contains("active mode"));
    }
}
