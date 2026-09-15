//! FTP / FTPS 远程文件客户端。
//!
//! 实现 `sftp::RemoteFileClient`，与 SFTP 共享同一套文件操作抽象；
//! 不实现 `sftp::SftpClient`（那是 SSH 专属入口），FTP 也不参与
//! host key、跳板机或 SSH server copy。
//!
//! 会话状态：传输被取消或连接中断后协议状态未知（`poisoned`），
//! 此时拒绝继续复用该连接，上层需要重新连接。

mod listing;
mod transfer;

use anyhow::{Result, anyhow};
use async_trait::async_trait;
use sftp::{DirectoryConflictPolicy, FileEntry, PathMetadata, ProgressCallback, RemoteFileClient};
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::time::Duration;
use suppaftp::tokio::{AsyncRustlsConnector, AsyncRustlsFtpStream};
use suppaftp::types::FileType;

/// FTP 连接参数（与 `one_core::storage::FtpParams` 的运行时形态对应）。
#[derive(Clone, Debug)]
pub struct FtpConnectConfig {
    pub host: String,
    pub port: u16,
    pub username: String,
    pub password: String,
    pub passive_mode: bool,
    /// 显式 FTPS（AUTH TLS）。
    pub use_tls: bool,
    /// 连接超时（秒）；`None` 表示不限制。
    pub connect_timeout: Option<u64>,
}

pub struct FtpClient {
    stream: AsyncRustlsFtpStream,
    /// 取消/停滞/连接中断后协议状态未知；置位后拒绝继续复用。
    poisoned: bool,
}

impl FtpClient {
    /// 建立连接：TCP → 可选显式 TLS → 登录 → 二进制传输模式。
    ///
    /// 超时覆盖**完整**建连流程，而非仅 TCP/欢迎响应阶段。
    /// 仅支持被动模式；主动模式在 NAT/防火墙后通常不可用，直接拒绝。
    pub async fn connect(config: FtpConnectConfig) -> Result<Self> {
        if !config.passive_mode {
            return Err(anyhow!("FTP active mode is not supported"));
        }
        let establish = async {
            let mut stream =
                AsyncRustlsFtpStream::connect((config.host.as_str(), config.port)).await?;
            if config.use_tls {
                let tls = tls_client_config()?;
                // suppaftp 内部用 rustls-pki-types 解析主机名：IP 与 DNS 名均可，
                // 且 rustls 0.23 的 webpki 原生校验 IP SAN（rustls 0.20 不支持，
                // 这正是从 async_ftp 迁移到 suppaftp 的原因）。
                stream = stream.into_secure(tls, &config.host).await?;
            }
            stream.login(&config.username, &config.password).await?;
            stream.transfer_type(FileType::Binary).await?;
            Ok::<_, anyhow::Error>(stream)
        };
        let stream = match config.connect_timeout {
            Some(seconds) => tokio::time::timeout(Duration::from_secs(seconds), establish)
                .await
                .map_err(|_| anyhow!("FTP connect timed out after {seconds}s"))??,
            None => establish.await?,
        };
        Ok(Self {
            stream,
            poisoned: false,
        })
    }

    /// 取消/中断后连接处于未知协议状态，必须拒绝复用并要求重连。
    fn ensure_usable(&self) -> Result<()> {
        if self.poisoned {
            return Err(anyhow!(
                "FTP connection is unusable after a cancelled or interrupted transfer; reconnect required"
            ));
        }
        Ok(())
    }

    fn poison(&mut self) {
        self.poisoned = true;
    }
}
/// 系统钥匙串信任链 → TLS connector。
///
/// rustls 0.23 与 rustls-native-certs 0.8 共享 pki-types，证书无需转换。
///
/// 显式安装 ring CryptoProvider：本仓依赖图中 rustls 同时被启用了
/// ring（suppaftp）与 aws-lc-rs（tiberius 等），进程级默认 provider
/// 无法自动判定，必须手动指定。
fn tls_client_config() -> Result<AsyncRustlsConnector> {
    use tokio_rustls::rustls::ClientConfig;

    let _ = tokio_rustls::rustls::crypto::ring::default_provider().install_default();

    let mut roots = tokio_rustls::rustls::RootCertStore::empty();
    let loaded = rustls_native_certs::load_native_certs();
    for error in &loaded.errors {
        tracing::warn!("FTP TLS: loading system certificates warned: {error}");
    }
    let mut added = 0;
    for cert in loaded.certs {
        if roots.add(cert).is_ok() {
            added += 1;
        }
    }
    if added == 0 {
        return Err(anyhow!("No usable system certificates for FTPS"));
    }
    let config = ClientConfig::builder()
        .with_root_certificates(roots)
        .with_no_client_auth();
    let connector: tokio_rustls::TlsConnector = Arc::new(config).into();
    Ok(AsyncRustlsConnector::from(connector))
}

#[async_trait]
impl RemoteFileClient for FtpClient {
    /// 置毒后的连接不允许被上层连接池归还复用。
    fn is_reusable(&self) -> bool {
        !self.poisoned
    }

    async fn list_dir(&mut self, path: &str) -> Result<Vec<FileEntry>> {
        self.ensure_usable()?;
        self.list_entries(path, None).await
    }

    async fn stat(&mut self, path: &str) -> Result<Option<PathMetadata>> {
        self.ensure_usable()?;
        self.stat_path(path).await
    }

    async fn download_with_progress(
        &mut self,
        remote_path: &str,
        local_path: &str,
        cancelled: Arc<AtomicBool>,
        progress: ProgressCallback,
    ) -> Result<()> {
        self.download_file(&remote_path, local_path, &cancelled, progress)
            .await
    }

    async fn upload_with_progress(
        &mut self,
        local_path: &str,
        remote_path: &str,
        cancelled: Arc<AtomicBool>,
        progress: ProgressCallback,
    ) -> Result<()> {
        self.upload_file(local_path, &remote_path, &cancelled, progress)
            .await
    }

    async fn delete(&mut self, path: &str, is_dir: bool) -> Result<()> {
        self.ensure_usable()?;
        if is_dir {
            self.stream.rmdir(path).await?;
        } else {
            self.stream.rm(path).await?;
        }
        Ok(())
    }

    async fn delete_recursive(
        &mut self,
        path: &str,
        cancelled: Arc<AtomicBool>,
        progress: ProgressCallback,
    ) -> Result<()> {
        self.delete_tree(path, &cancelled, progress).await
    }

    async fn mkdir(&mut self, path: &str) -> Result<()> {
        self.ensure_usable()?;
        self.stream.mkdir(path).await?;
        Ok(())
    }

    async fn rename(&mut self, old_path: &str, new_path: &str) -> Result<()> {
        self.ensure_usable()?;
        self.stream.rename(old_path, new_path).await?;
        Ok(())
    }

    /// FTP 协议无权限修改能力；UI 应隐藏该入口，此处返回明确错误兜底。
    async fn chmod(&mut self, _path: &str, _mode: u32) -> Result<()> {
        Err(anyhow!("FTP does not support chmod"))
    }

    async fn read_file(&mut self, path: &str, max_bytes: usize) -> Result<Vec<u8>> {
        self.ensure_usable()?;
        // 流式读取并限制字节数，避免大文件撑爆内存。
        // get() 成功即传输已开始：此后任何退出路径——读取失败、
        // 本地缓冲错误、超限退出——数据流都没有正常收尾，
        // 协议状态未知 → 连接置毒，拒绝复用（残留应答会让后续
        // 命令读到上一次传输的 226/426，造成命令失步）。
        let mut reader = self.stream.retr_as_stream(path).await?;
        let mut content: Vec<u8> = Vec::with_capacity(max_bytes.min(4 * 1024 * 1024));
        let mut chunk = vec![0u8; 64 * 1024];
        let mut last_data = tokio::time::Instant::now();
        loop {
            let read =
                match crate::transfer::read_chunk(&mut reader, &mut chunk, None, &mut last_data)
                    .await
                {
                    Ok(read) => read,
                    Err(error) => {
                        self.poison();
                        return Err(error);
                    }
                };
            if read == 0 {
                break;
            }
            if content.len() + read > max_bytes {
                // 数据连接上仍有未读数据，终态响应无法安全消费：置毒。
                drop(reader);
                self.poison();
                return Err(anyhow!(
                    "Remote file size exceeds max readable size {max_bytes}"
                ));
            }
            content.extend_from_slice(&chunk[..read]);
        }
        // 数据流 EOF 后由 TransferStream::finish() 消费 226 完成响应；
        // finish 同时关闭数据连接并读取终态，任何失败都视为失步 → 置毒。
        match tokio::time::timeout(crate::transfer::COMMAND_TIMEOUT, reader.finish()).await {
            Ok(Ok(_)) => {}
            Ok(Err(error)) => {
                self.poison();
                return Err(error.into());
            }
            Err(_) => {
                self.poison();
                return Err(anyhow!(
                    "Timed out waiting for FTP read completion response"
                ));
            }
        }
        Ok(content)
    }

    async fn write_file(&mut self, path: &str, content: &[u8]) -> Result<()> {
        // 编辑器保存与普通上传共用"远端临时文件 → rename"安全契约，
        // 保存失败时不破坏远端原文件。
        self.write_file_safe(path, content).await
    }

    async fn list_dir_recursive(
        &mut self,
        path: &str,
        cancelled: Arc<AtomicBool>,
    ) -> Result<Vec<FileEntry>> {
        self.ensure_usable()?;
        self.collect_recursive_entries(path, &cancelled).await
    }

    async fn download_dir_with_progress(
        &mut self,
        remote_path: &str,
        local_path: &str,
        cancelled: Arc<AtomicBool>,
        progress: ProgressCallback,
    ) -> Result<()> {
        self.download_dir(
            remote_path,
            local_path,
            DirectoryConflictPolicy::Merge,
            &cancelled,
            progress,
        )
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
        self.upload_dir(
            local_path,
            remote_path,
            conflict_policy,
            &cancelled,
            progress,
        )
        .await
    }

    async fn disconnect(&mut self) -> Result<()> {
        // 连接可能已失效（对端关闭、TLS 中断）；quit 加超时上限，失败不视为错误。
        let _ = tokio::time::timeout(crate::transfer::QUIT_TIMEOUT, self.stream.quit()).await;
        Ok(())
    }

    /// FTP 无 realpath；空路径或 `.` 返回当前工作目录，
    /// 相对路径基于当前工作目录拼接，绝对路径原样返回。
    async fn realpath(&mut self, path: &str) -> Result<String> {
        self.ensure_usable()?;
        if path.is_empty() || path == "." {
            return self.stream.pwd().await.map_err(Into::into);
        }
        if path.starts_with('/') {
            return Ok(path.to_string());
        }
        let pwd = self.stream.pwd().await?;
        Ok(crate::transfer::child_path(&pwd, path))
    }
}
