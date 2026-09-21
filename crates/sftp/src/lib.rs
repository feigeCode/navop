rust_i18n::i18n!("locales", fallback = "en");

mod file_operations;
mod remote_exec;
mod remote_file_command;
mod russh_impl;
mod server_copy;
mod server_copy_auth;
mod server_copy_command;
mod server_copy_direct;
mod upload_batch;

use anyhow::{Result, anyhow};
use async_trait::async_trait;
use ssh::SshConnectConfig;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::time::SystemTime;

pub use file_operations::{
    calculate_directory_size, remote_path_is_same_or_descendant, total_file_size,
};
pub use remote_file_command::{RemoteFileOperation, build_remote_file_command};
pub use russh_impl::RusshSftpClient;
pub use server_copy::{
    CopyPlanEntry, DirectCopyApproval, DirectCopyApprovalFuture, DirectCopyDecision,
    DirectCopyPreview, DirectCopyStrategy, ServerCopyItem, ServerCopyRequest, copy_between_servers,
    join_copy_path, relay_copy,
};

#[derive(Debug, Clone)]
pub struct FileEntry {
    pub name: String,
    pub path: String,
    pub size: u64,
    pub modified: SystemTime,
    pub is_dir: bool,
    pub permissions: u32,
    pub uid: Option<u32>,
    pub gid: Option<u32>,
    pub user: Option<String>,
    pub group: Option<String>,
}

impl FileEntry {
    /// Returns the owner reported by the remote server.
    ///
    /// SFTP v3 servers commonly expose only a numeric UID. When the remote user
    /// name is available, prefer it and keep the UID only as a fallback.
    pub fn owner_display(&self) -> Option<String> {
        match (&self.user, self.uid) {
            (Some(user), _) if !user.is_empty() => Some(user.clone()),
            (_, Some(uid)) => Some(uid.to_string()),
            _ => None,
        }
    }
}

#[derive(Debug, Clone)]
pub struct PathMetadata {
    pub size: u64,
    pub modified: SystemTime,
    pub is_dir: bool,
    pub permissions: u32,
}

#[derive(Debug, Clone)]
pub struct TransferItem {
    pub local_path: String,
    pub remote_path: String,
    pub size: u64,
    pub is_dir: bool,
}

#[derive(Debug, Clone, Default)]
pub struct TransferProgress {
    pub transferred: u64,
    pub total: u64,
    pub speed: f64,
    pub current_file: Option<String>,
    pub current_file_transferred: u64,
    pub current_file_total: u64,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum DirectoryConflictPolicy {
    #[default]
    Merge,
    Replace,
}

#[derive(Debug)]
pub struct TransferCancelled;

impl std::fmt::Display for TransferCancelled {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Cancelled")
    }
}

impl std::error::Error for TransferCancelled {}

pub type ProgressCallback = Box<dyn Fn(TransferProgress) + Send + Sync + 'static>;

pub(crate) fn validate_read_size(file_size: usize, max_bytes: usize) -> Result<()> {
    if file_size > max_bytes {
        return Err(anyhow!(
            "Remote file size {} exceeds max readable size {}",
            file_size,
            max_bytes
        ));
    }
    Ok(())
}

/// 协议无关的远程文件客户端能力。
///
/// 只描述文件操作（浏览/传输/编辑所需），不约束底层连接方式，
/// 因此 SFTP 与 FTP 客户端都可以实现；SSH 专属能力仍留在 [`SftpClient`]。
#[async_trait]
pub trait RemoteFileClient: Send + Sync {
    async fn list_dir(&mut self, path: &str) -> Result<Vec<FileEntry>>;

    async fn stat(&mut self, path: &str) -> Result<Option<PathMetadata>>;

    async fn download_with_progress(
        &mut self,
        remote_path: &str,
        local_path: &str,
        cancelled: Arc<AtomicBool>,
        progress: ProgressCallback,
    ) -> Result<()>;

    async fn upload_with_progress(
        &mut self,
        local_path: &str,
        remote_path: &str,
        cancelled: Arc<AtomicBool>,
        progress: ProgressCallback,
    ) -> Result<()>;

    async fn delete(&mut self, path: &str, is_dir: bool) -> Result<()>;

    /// 递归删除目录及其所有内容，带进度回调
    async fn delete_recursive(
        &mut self,
        path: &str,
        cancelled: Arc<AtomicBool>,
        progress: ProgressCallback,
    ) -> Result<()>;

    async fn mkdir(&mut self, path: &str) -> Result<()>;

    async fn rename(&mut self, old_path: &str, new_path: &str) -> Result<()>;

    async fn chmod(&mut self, path: &str, mode: u32) -> Result<()>;

    /// 读取远程文件内容，超过 max_bytes 时返回错误。
    async fn read_file(&mut self, path: &str, max_bytes: usize) -> Result<Vec<u8>>;

    /// 写入文件内容（用于创建新文件或覆盖文件）
    async fn write_file(&mut self, path: &str, content: &[u8]) -> Result<()>;

    async fn list_dir_recursive(
        &mut self,
        path: &str,
        cancelled: Arc<AtomicBool>,
    ) -> Result<Vec<FileEntry>>;

    async fn download_dir_with_progress(
        &mut self,
        remote_path: &str,
        local_path: &str,
        cancelled: Arc<AtomicBool>,
        progress: ProgressCallback,
    ) -> Result<()>;

    async fn upload_dir_with_progress(
        &mut self,
        local_path: &str,
        remote_path: &str,
        conflict_policy: DirectoryConflictPolicy,
        cancelled: Arc<AtomicBool>,
        progress: ProgressCallback,
    ) -> Result<()>;

    async fn disconnect(&mut self) -> Result<()>;

    /// 获取路径的真实绝对路径
    async fn realpath(&mut self, path: &str) -> Result<String>;

    /// 连接当前是否允许复用。
    ///
    /// 传输被取消、连接失步或已中断后，协议状态未知，连接必须被
    /// 上层连接池丢弃而不是归还复用。默认 `true`（SFTP 等无状态
    /// 失效语义的客户端无需处理）；FTP 在置毒后返回 `false`。
    fn is_reusable(&self) -> bool {
        true
    }
}

/// SSH 专属的远程文件客户端：在 [`RemoteFileClient`] 之上增加基于
/// `SshConnectConfig` 的连接入口（host key 校验、跳板机、代理均在此路径内）。
#[async_trait]
pub trait SftpClient: RemoteFileClient {
    async fn connect(ssh_config: SshConnectConfig) -> Result<Self>
    where
        Self: Sized;
}

/// 跨模块共享的协议无关远程文件客户端句柄。
///
/// SFTP 视图、远程文件编辑器、图片预览等只依赖该别名，
/// 不再硬编码 `RusshSftpClient`。
pub type SharedRemoteFileClient = std::sync::Arc<tokio::sync::Mutex<Box<dyn RemoteFileClient>>>;

#[async_trait]
impl<T: RemoteFileClient + ?Sized + Send + Sync> RemoteFileClient for Box<T> {
    async fn list_dir(&mut self, path: &str) -> Result<Vec<FileEntry>> {
        (**self).list_dir(path).await
    }
    async fn stat(&mut self, path: &str) -> Result<Option<PathMetadata>> {
        (**self).stat(path).await
    }
    async fn download_with_progress(
        &mut self,
        remote_path: &str,
        local_path: &str,
        cancelled: Arc<AtomicBool>,
        progress: ProgressCallback,
    ) -> Result<()> {
        (**self)
            .download_with_progress(remote_path, local_path, cancelled, progress)
            .await
    }
    async fn upload_with_progress(
        &mut self,
        local_path: &str,
        remote_path: &str,
        cancelled: Arc<AtomicBool>,
        progress: ProgressCallback,
    ) -> Result<()> {
        (**self)
            .upload_with_progress(local_path, remote_path, cancelled, progress)
            .await
    }
    async fn delete(&mut self, path: &str, is_dir: bool) -> Result<()> {
        (**self).delete(path, is_dir).await
    }
    async fn delete_recursive(
        &mut self,
        path: &str,
        cancelled: Arc<AtomicBool>,
        progress: ProgressCallback,
    ) -> Result<()> {
        (**self).delete_recursive(path, cancelled, progress).await
    }
    async fn mkdir(&mut self, path: &str) -> Result<()> {
        (**self).mkdir(path).await
    }
    async fn rename(&mut self, old_path: &str, new_path: &str) -> Result<()> {
        (**self).rename(old_path, new_path).await
    }
    async fn chmod(&mut self, path: &str, mode: u32) -> Result<()> {
        (**self).chmod(path, mode).await
    }
    async fn read_file(&mut self, path: &str, max_bytes: usize) -> Result<Vec<u8>> {
        (**self).read_file(path, max_bytes).await
    }
    async fn write_file(&mut self, path: &str, content: &[u8]) -> Result<()> {
        (**self).write_file(path, content).await
    }
    async fn list_dir_recursive(
        &mut self,
        path: &str,
        cancelled: Arc<AtomicBool>,
    ) -> Result<Vec<FileEntry>> {
        (**self).list_dir_recursive(path, cancelled).await
    }
    async fn download_dir_with_progress(
        &mut self,
        remote_path: &str,
        local_path: &str,
        cancelled: Arc<AtomicBool>,
        progress: ProgressCallback,
    ) -> Result<()> {
        (**self)
            .download_dir_with_progress(remote_path, local_path, cancelled, progress)
            .await
    }
    async fn upload_dir_with_progress(
        &mut self,
        local_path: &str,
        remote_path: &str,
        conflict_policy: DirectoryConflictPolicy,
        cancelled: Arc<AtomicBool>,
        progress: ProgressCallback,
    ) -> Result<()> {
        (**self)
            .upload_dir_with_progress(
                local_path,
                remote_path,
                conflict_policy,
                cancelled,
                progress,
            )
            .await
    }
    async fn disconnect(&mut self) -> Result<()> {
        (**self).disconnect().await
    }
    async fn realpath(&mut self, path: &str) -> Result<String> {
        (**self).realpath(path).await
    }
    fn is_reusable(&self) -> bool {
        (**self).is_reusable()
    }
}

#[cfg(test)]
mod tests {
    use super::{FileEntry, validate_read_size};
    use std::time::SystemTime;

    fn file_entry(user: Option<&str>, uid: Option<u32>) -> FileEntry {
        FileEntry {
            name: "file".to_string(),
            path: "/file".to_string(),
            size: 0,
            modified: SystemTime::UNIX_EPOCH,
            is_dir: false,
            permissions: 0,
            uid,
            gid: None,
            user: user.map(str::to_string),
            group: None,
        }
    }

    #[test]
    fn validate_read_size_allows_files_within_limit() {
        assert!(validate_read_size(1024, 1024).is_ok());
        assert!(validate_read_size(512, 1024).is_ok());
    }

    #[test]
    fn validate_read_size_rejects_files_larger_than_limit() {
        let error = validate_read_size(1025, 1024).expect_err("应拒绝超限文件");
        assert!(error.to_string().contains("exceeds max readable size"));
    }

    #[test]
    fn owner_display_prefers_server_user_name() {
        assert_eq!(
            Some("deploy".to_string()),
            file_entry(Some("deploy"), Some(1001)).owner_display()
        );
    }

    #[test]
    fn owner_display_falls_back_to_numeric_uid() {
        assert_eq!(
            Some("1001".to_string()),
            file_entry(None, Some(1001)).owner_display()
        );
        assert_eq!(None, file_entry(None, None).owner_display());
    }
}
