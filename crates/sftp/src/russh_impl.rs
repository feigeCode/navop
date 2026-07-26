use crate::server_copy::CopyFileRequest;
use crate::{
    FileEntry, PathMetadata, ProgressCallback, SftpClient, TransferCancelled, TransferProgress,
    validate_read_size,
};
use anyhow::{Result, anyhow};
use async_trait::async_trait;
use russh::client::{self, Handle};
use russh::keys::PublicKey;
use russh_sftp::client::RawSftpSession;
use russh_sftp::client::SftpSession;
use russh_sftp::client::error::Error as SftpError;
use russh_sftp::client::fs::File as SftpFile;
use russh_sftp::client::rawsession::Limits;
use russh_sftp::protocol::{FileAttributes, OpenFlags, StatusCode};
use rust_i18n::t;
use ssh::{
    AuthFailureMessages, HostKeyAcceptance, HostKeyDetails, HostKeyIdentity, HostKeyVerifier,
    ProxyConnectConfig, ProxyType, RusshClient, SshConnectConfig, authenticate_with_strategy,
    defaults,
};
use std::collections::BTreeMap;
use std::future::Future;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tokio::fs::{self, File, OpenOptions};
use tokio::io::{AsyncReadExt, AsyncWriteExt, BufReader, BufWriter};
use tokio::net::TcpStream;
use tokio::sync::Mutex;
use tokio::task::JoinSet;

const BUFFER_SIZE: usize = 256 * 1024; // 256 KB
const PIPELINE_CHUNK_SIZE: u32 = 61440; // 60 KB per read request (within 65535 packet limit)
const MAX_INFLIGHT_REQUESTS: usize = 64; // 最多 64 个并发请求
const PIPELINE_THRESHOLD: u64 = 512 * 1024; // 超过 512 KB 的文件才走流水线

/// A downloaded file is written beside its destination and becomes visible
/// only after the complete byte range has been verified and synced.
struct LocalDownloadTemp {
    path: PathBuf,
    committed: bool,
}

impl LocalDownloadTemp {
    async fn create(target: &Path) -> Result<(Self, File)> {
        let parent = target.parent().unwrap_or_else(|| Path::new("."));
        let name = target
            .file_name()
            .and_then(|name| name.to_str())
            .filter(|name| !name.is_empty())
            .unwrap_or("download");

        // `create_new` prevents a concurrent transfer from accidentally
        // opening another transfer's partial file.
        for _ in 0..8 {
            let path = parent.join(format!(".{name}.navop-part-{}", uuid::Uuid::new_v4()));
            match OpenOptions::new()
                .write(true)
                .read(true)
                .create_new(true)
                .open(&path)
                .await
            {
                Ok(file) => {
                    return Ok((
                        Self {
                            path,
                            committed: false,
                        },
                        file,
                    ));
                }
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(error) => {
                    return Err(anyhow!(
                        "Failed to create temporary local file beside {}: {}",
                        target.display(),
                        error
                    ));
                }
            }
        }

        Err(anyhow!(
            "Failed to allocate a unique temporary local file beside {}",
            target.display()
        ))
    }

    async fn commit(mut self, target: &Path) -> Result<()> {
        fs::rename(&self.path, target).await.map_err(|error| {
            anyhow!(
                "Failed to atomically commit downloaded file {}: {}",
                target.display(),
                error
            )
        })?;
        self.committed = true;
        Ok(())
    }
}

impl Drop for LocalDownloadTemp {
    fn drop(&mut self) {
        if !self.committed {
            let _ = std::fs::remove_file(&self.path);
        }
    }
}

/// A remote upload is staged beside its destination and published with one
/// SFTP rename request. The target is never opened with TRUNCATE.
struct RemoteReplaceTemp {
    path: String,
    committed: bool,
    cleaned: bool,
}

impl RemoteReplaceTemp {
    fn path_for(target: &str) -> Result<String> {
        if target.is_empty() || target.ends_with('/') || target.contains('\0') {
            return Err(anyhow!(
                "Remote replace target must be a non-empty file path without a trailing slash"
            ));
        }

        let (parent, name) = match target.rfind('/') {
            Some(index) => {
                let parent = if index == 0 { "/" } else { &target[..index] };
                (parent, &target[index + 1..])
            }
            None => (".", target),
        };
        if name.is_empty() || name == "." || name == ".." {
            return Err(anyhow!(
                "Remote replace target must identify a regular file name"
            ));
        }
        let temporary_name = format!(".{name}.navop-part-{}", uuid::Uuid::new_v4());

        if parent == "/" {
            Ok(format!("/{temporary_name}"))
        } else {
            Ok(format!("{parent}/{temporary_name}"))
        }
    }

    async fn create(sftp: &SftpSession, target: &str) -> Result<(Self, SftpFile)> {
        let path = Self::path_for(target)?;
        let file = sftp
            .open_with_flags(
                &path,
                OpenFlags::CREATE | OpenFlags::EXCLUDE | OpenFlags::WRITE,
            )
            .await
            .map_err(|error| {
                anyhow!(
                    "Failed to create remote temporary file beside {}: {}",
                    target,
                    error
                )
            })?;

        Ok((
            Self {
                path,
                committed: false,
                cleaned: false,
            },
            file,
        ))
    }

    async fn cleanup(&mut self, sftp: &SftpSession) {
        if self.committed || self.cleaned {
            return;
        }

        match sftp.remove_file(&self.path).await {
            Ok(()) => self.cleaned = true,
            Err(SftpError::Status(status)) if status.status_code == StatusCode::NoSuchFile => {
                self.cleaned = true;
            }
            Err(error) => {
                tracing::debug!(
                    path = %self.path,
                    error = %error,
                    "failed to remove abandoned remote temporary file"
                );
            }
        }
    }

    async fn commit(mut self, sftp: &SftpSession, target: &str) -> Result<()> {
        match sftp.rename(&self.path, target).await {
            Ok(()) => {
                self.committed = true;
                Ok(())
            }
            Err(error) => {
                self.cleanup(sftp).await;
                Err(anyhow!(
                    "Failed to atomically replace remote file {} (the original was not removed): {}",
                    target,
                    error
                ))
            }
        }
    }
}

impl Drop for RemoteReplaceTemp {
    fn drop(&mut self) {
        if !self.committed && !self.cleaned {
            tracing::warn!(
                path = %self.path,
                "remote temporary file may have been abandoned"
            );
        }
    }
}

/// Run a complete remote replace transaction. The closure owns the temporary
/// handle and returns it after writing so this helper can flush, verify the
/// exact byte count, close the handle, and only then publish the new name.
async fn with_remote_replace<F, Fut>(
    sftp: &SftpSession,
    target: &str,
    expected_size: u64,
    operation: F,
) -> Result<u64>
where
    F: FnOnce(SftpFile) -> Fut,
    Fut: Future<Output = Result<(SftpFile, u64)>>,
{
    let (mut temporary, file) = RemoteReplaceTemp::create(sftp, target).await?;
    let (mut file, written) = match operation(file).await {
        Ok(result) => result,
        Err(error) => {
            temporary.cleanup(sftp).await;
            return Err(error);
        }
    };

    let validation = async {
        if written != expected_size {
            return Err(anyhow!(
                "Remote replace for {} wrote {} bytes, expected {}",
                target,
                written,
                expected_size
            ));
        }

        file.flush()
            .await
            .map_err(|error| anyhow!("Failed to flush remote temporary file: {}", error))?;
        file.sync_all()
            .await
            .map_err(|error| anyhow!("Failed to sync remote temporary file: {}", error))?;

        let actual_size = file
            .metadata()
            .await
            .map_err(|error| anyhow!("Failed to verify remote temporary file: {}", error))?
            .size
            .ok_or_else(|| {
                anyhow!(
                    "Remote temporary file {} has no reported size",
                    temporary.path
                )
            })?;
        if actual_size != expected_size {
            return Err(anyhow!(
                "Remote replace for {} has remote size {}, expected {}",
                target,
                actual_size,
                expected_size
            ));
        }

        file.shutdown()
            .await
            .map_err(|error| anyhow!("Failed to close remote temporary file: {}", error))?;
        Ok(())
    }
    .await;

    if let Err(error) = validation {
        let _ = file.shutdown().await;
        temporary.cleanup(sftp).await;
        return Err(error);
    }

    temporary.commit(sftp, target).await?;
    Ok(written)
}

fn expected_chunk_len(offset: u64, total_size: u64) -> Result<usize> {
    if offset >= total_size {
        return Err(anyhow!(
            "SFTP pipeline scheduled an out-of-range chunk at offset {} for {} bytes",
            offset,
            total_size
        ));
    }
    Ok(std::cmp::min(PIPELINE_CHUNK_SIZE as u64, total_size - offset) as usize)
}

fn validate_chunk_len(offset: u64, total_size: u64, actual_len: usize) -> Result<()> {
    let expected = expected_chunk_len(offset, total_size)?;
    if actual_len != expected {
        return Err(anyhow!(
            "SFTP short read at offset {}: received {} bytes, expected {}",
            offset,
            actual_len,
            expected
        ));
    }
    Ok(())
}

async fn abort_pipeline_reads<T: 'static>(reads: &mut JoinSet<T>) {
    reads.abort_all();
    while reads.join_next().await.is_some() {}
}

fn ensure_not_cancelled(cancelled: &AtomicBool) -> Result<()> {
    if cancelled.load(Ordering::Relaxed) {
        return Err(TransferCancelled.into());
    }
    Ok(())
}

struct SftpHandler {
    identity: HostKeyIdentity,
    host_key_verifier: HostKeyVerifier,
}

impl SftpHandler {
    fn new(identity: HostKeyIdentity, host_key_verifier: HostKeyVerifier) -> Self {
        Self {
            identity,
            host_key_verifier,
        }
    }
}

impl client::Handler for SftpHandler {
    type Error = anyhow::Error;

    async fn check_server_key(
        &mut self,
        server_public_key: &PublicKey,
    ) -> Result<bool, Self::Error> {
        match self
            .host_key_verifier
            .verify(&self.identity, server_public_key)
        {
            Ok(HostKeyAcceptance::Known) => Ok(true),
            Ok(HostKeyAcceptance::AcceptedNew) => {
                let details = HostKeyDetails::from_public_key(server_public_key);
                tracing::warn!(
                    target: "ssh.host_key",
                    identity = %self.identity,
                    algorithm = %details.algorithm,
                    fingerprint = %details.fingerprint,
                    "accepted and persisted a new SFTP SSH host key"
                );
                Ok(true)
            }
            Ok(HostKeyAcceptance::Insecure) => {
                let details = HostKeyDetails::from_public_key(server_public_key);
                tracing::warn!(
                    target: "ssh.host_key",
                    identity = %self.identity,
                    algorithm = %details.algorithm,
                    fingerprint = %details.fingerprint,
                    "accepted an SFTP SSH host key using explicit insecure mode"
                );
                Ok(true)
            }
            Err(rejection) => Err(rejection.into()),
        }
    }
}

fn sftp_auth_failure_messages() -> AuthFailureMessages {
    AuthFailureMessages {
        password_failed: t!("Sftp.auth_password_failed").to_string(),
        certificate_failed: t!("Sftp.auth_certificate_failed").to_string(),
        public_key_failed: t!("Sftp.auth_public_key_failed").to_string(),
        agent_connect_failed: t!("Sftp.auth_agent_connect_failed").to_string(),
        agent_no_identities: t!("Sftp.auth_agent_no_identities").to_string(),
        agent_auth_failed: t!("Sftp.auth_agent_failed").to_string(),
        auto_publickey_failed: t!("Sftp.auth_auto_publickey_failed").to_string(),
        no_local_identity: t!("Sftp.auth_no_local_identity").to_string(),
        auto_publickey_next_step: t!("Sftp.auth_auto_publickey_next_step").to_string(),
        keyboard_interactive_required: t!("Sftp.auth_keyboard_interactive_required").to_string(),
        keyboard_interactive_failed: t!("Sftp.auth_keyboard_interactive_failed").to_string(),
        keyboard_interactive_cancelled: t!("Sftp.auth_keyboard_interactive_cancelled").to_string(),
    }
}

/// 通过代理建立TCP连接
async fn sftp_connect_via_proxy(
    proxy: &ProxyConnectConfig,
    target_host: &str,
    target_port: u16,
) -> Result<TcpStream> {
    let proxy_addr = format!("{}:{}", proxy.host, proxy.port);

    match proxy.proxy_type {
        ProxyType::Socks5 => {
            use tokio_socks::tcp::Socks5Stream;

            let stream = if let (Some(username), Some(password)) =
                (&proxy.username, &proxy.password)
            {
                Socks5Stream::connect_with_password(
                    proxy_addr.as_str(),
                    (target_host, target_port),
                    username,
                    password,
                )
                .await
                .map_err(|e| {
                    anyhow::anyhow!(t!("Sftp.socks5_proxy_connect_failed", error = e).to_string())
                })?
            } else {
                Socks5Stream::connect(proxy_addr.as_str(), (target_host, target_port))
                    .await
                    .map_err(|e| {
                        anyhow::anyhow!(
                            t!("Sftp.socks5_proxy_connect_failed", error = e).to_string()
                        )
                    })?
            };

            Ok(stream.into_inner())
        }
        ProxyType::Http => {
            let stream = TcpStream::connect(&proxy_addr).await.map_err(|e| {
                anyhow::anyhow!(t!("Sftp.http_proxy_connect_failed", error = e).to_string())
            })?;

            let connect_request = if let (Some(username), Some(password)) =
                (&proxy.username, &proxy.password)
            {
                let credentials = format!("{}:{}", username, password);
                let encoded = base64_encode(&credentials);
                format!(
                    "CONNECT {}:{} HTTP/1.1\r\nHost: {}:{}\r\nProxy-Authorization: Basic {}\r\n\r\n",
                    target_host, target_port, target_host, target_port, encoded
                )
            } else {
                format!(
                    "CONNECT {}:{} HTTP/1.1\r\nHost: {}:{}\r\n\r\n",
                    target_host, target_port, target_host, target_port
                )
            };

            use tokio::io::{AsyncBufReadExt, BufReader};

            let (reader, mut writer) = stream.into_split();
            writer.write_all(connect_request.as_bytes()).await?;

            let mut reader = BufReader::new(reader);
            let mut response_line = String::new();
            reader.read_line(&mut response_line).await?;

            if !response_line.contains("200") {
                anyhow::bail!(t!(
                    "Sftp.http_proxy_connection_failed",
                    response = response_line.trim()
                ));
            }

            loop {
                let mut line = String::new();
                reader.read_line(&mut line).await?;
                if line == "\r\n" || line.is_empty() {
                    break;
                }
            }

            Ok(reader.into_inner().reunite(writer)?)
        }
    }
}

/// 简单的Base64编码
fn base64_encode(input: &str) -> String {
    const ALPHABET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

    let bytes = input.as_bytes();
    let mut result = Vec::new();

    for chunk in bytes.chunks(3) {
        let b0 = chunk[0] as u32;
        let b1 = chunk.get(1).copied().unwrap_or(0) as u32;
        let b2 = chunk.get(2).copied().unwrap_or(0) as u32;

        let n = (b0 << 16) | (b1 << 8) | b2;

        result.push(ALPHABET[((n >> 18) & 0x3F) as usize]);
        result.push(ALPHABET[((n >> 12) & 0x3F) as usize]);

        if chunk.len() > 1 {
            result.push(ALPHABET[((n >> 6) & 0x3F) as usize]);
        } else {
            result.push(b'=');
        }

        if chunk.len() > 2 {
            result.push(ALPHABET[(n & 0x3F) as usize]);
        } else {
            result.push(b'=');
        }
    }

    String::from_utf8(result).unwrap()
}

enum SessionOwner {
    Owned {
        session: Handle<SftpHandler>,
        _jump_session: Option<Handle<SftpHandler>>,
    },
    Shared {
        client: Arc<Mutex<RusshClient>>,
    },
}

pub struct RusshSftpClient {
    sftp: SftpSession,
    owner: SessionOwner,
    /// 懒初始化的原始 SFTP 会话，用于流水线下载
    raw_sftp: Option<Arc<RawSftpSession>>,
}

impl RusshSftpClient {
    pub async fn connect_with_client(client: Arc<Mutex<RusshClient>>) -> Result<Self> {
        let channel = {
            let mut guard = client.lock().await;
            guard.open_raw_channel().await?
        };
        channel.request_subsystem(true, "sftp").await?;
        let sftp = SftpSession::new(channel.into_stream()).await?;

        Ok(Self {
            sftp,
            owner: SessionOwner::Shared { client },
            raw_sftp: None,
        })
    }

    /// 在已有 SSH 连接上创建一个新的 RawSftpSession 用于流水线操作
    async fn get_or_create_raw_session(&mut self) -> Result<Arc<RawSftpSession>> {
        if let Some(ref raw) = self.raw_sftp {
            return Ok(Arc::clone(raw));
        }

        let channel = match &self.owner {
            SessionOwner::Owned { session, .. } => {
                let channel = session.channel_open_session().await?;
                channel.request_subsystem(true, "sftp").await?;
                channel
            }
            SessionOwner::Shared { client } => {
                let channel = {
                    let mut guard = client.lock().await;
                    guard.open_raw_channel().await?
                };
                channel.request_subsystem(true, "sftp").await?;
                channel
            }
        };

        let mut raw = RawSftpSession::new(channel.into_stream());
        raw.init()
            .await
            .map_err(|e| anyhow!("Failed to init raw SFTP session: {}", e))?;

        // 尝试查询 limits@openssh.com 扩展并设置限制
        if let Ok(limits_ext) = raw.limits().await {
            let limits: Limits = limits_ext.into();
            raw.set_limits(limits);
        }

        raw.set_timeout(300);

        let raw = Arc::new(raw);
        self.raw_sftp = Some(Arc::clone(&raw));
        Ok(raw)
    }

    /// Read a remote file in bounded parallel chunks and write only contiguous,
    /// exactly-sized chunks to the caller's temporary file.
    async fn pipelined_read_into_writer<F>(
        raw_session: Arc<RawSftpSession>,
        remote_path: &str,
        total_size: u64,
        cancelled: &AtomicBool,
        writer: &mut BufWriter<File>,
        mut on_progress: F,
    ) -> Result<u64>
    where
        F: FnMut(u64),
    {
        let handle_result = raw_session
            .open(remote_path, OpenFlags::READ, FileAttributes::default())
            .await
            .map_err(|error| anyhow!("Failed to open remote file {}: {}", remote_path, error))?;
        let file_handle = handle_result.handle;
        let chunk_size = PIPELINE_CHUNK_SIZE as u64;
        let total_chunks = total_size.div_ceil(chunk_size);
        let mut reads = JoinSet::new();
        let mut next_request = 0u64;
        let mut pending: BTreeMap<u64, Vec<u8>> = BTreeMap::new();
        let mut next_offset = 0u64;
        let mut transferred = 0u64;
        let mut result = Ok(());

        while next_request < total_chunks || !reads.is_empty() {
            if let Err(error) = ensure_not_cancelled(cancelled) {
                result = Err(error);
                break;
            }

            while next_request < total_chunks && reads.len() < MAX_INFLIGHT_REQUESTS {
                let offset = next_request * chunk_size;
                let expected_len = expected_chunk_len(offset, total_size)?;
                let raw = Arc::clone(&raw_session);
                let handle = file_handle.clone();
                reads.spawn(async move {
                    match raw
                        .read(handle, offset, expected_len as u32)
                        .await
                    {
                        Ok(data) => {
                            validate_chunk_len(offset, total_size, data.data.len())?;
                            Ok::<_, anyhow::Error>((offset, data.data))
                        }
                        Err(SftpError::Status(status))
                            if status.status_code == StatusCode::Eof =>
                        {
                            Err(anyhow!(
                                "Unexpected EOF while reading remote file at offset {}: expected {} bytes",
                                offset,
                                expected_len
                            ))
                        }
                        Err(error) => Err(anyhow!(
                            "SFTP read failed at offset {} ({} bytes): {}",
                            offset,
                            expected_len,
                            error
                        )),
                    }
                });
                next_request += 1;
            }

            let Some(joined) = reads.join_next().await else {
                result = Err(anyhow!(
                    "SFTP pipeline ended before all chunks were received"
                ));
                break;
            };
            let chunk = match joined {
                Ok(Ok(chunk)) => chunk,
                Ok(Err(error)) => {
                    result = Err(error);
                    break;
                }
                Err(error) => {
                    result = Err(anyhow!("SFTP read task failed: {}", error));
                    break;
                }
            };

            if pending.insert(chunk.0, chunk.1).is_some() {
                result = Err(anyhow!(
                    "SFTP pipeline returned duplicate chunk at offset {}",
                    chunk.0
                ));
                break;
            }

            let drain_result: Result<()> = async {
                while let Some(data) = pending.remove(&next_offset) {
                    validate_chunk_len(next_offset, total_size, data.len())?;
                    writer
                        .write_all(&data)
                        .await
                        .map_err(|error| anyhow!("Failed to write to local file: {}", error))?;
                    let bytes = data.len() as u64;
                    transferred += bytes;
                    next_offset += bytes;
                    on_progress(transferred);
                }
                Ok(())
            }
            .await;
            if let Err(error) = drain_result {
                result = Err(error);
                break;
            }
        }

        if result.is_ok() {
            if next_offset != total_size || transferred != total_size || !pending.is_empty() {
                result = Err(anyhow!(
                    "SFTP pipeline did not cover the complete remote file: wrote {} of {} bytes (next offset {}, pending chunks {})",
                    transferred,
                    total_size,
                    next_offset,
                    pending.len()
                ));
            }
        }

        if result.is_err() {
            abort_pipeline_reads(&mut reads).await;
        }

        let close_result = raw_session.close(file_handle).await;
        match (result, close_result) {
            (Err(error), _) => Err(error),
            (Ok(()), Err(error)) => Err(anyhow!("Failed to close remote file: {}", error)),
            (Ok(()), Ok(_)) => Ok(transferred),
        }
    }

    /// 流水线下载：通过 RawSftpSession 发起多个并发读请求
    async fn pipelined_download(
        raw_session: Arc<RawSftpSession>,
        remote_path: &str,
        local_path: &str,
        total_size: u64,
        cancelled: &AtomicBool,
        progress: &(dyn Fn(TransferProgress) + Send + Sync),
    ) -> Result<()> {
        let target = Path::new(local_path);
        let (temporary, local_file) = LocalDownloadTemp::create(target).await?;
        let mut writer = BufWriter::with_capacity(BUFFER_SIZE, local_file);
        let start_time = Instant::now();
        let mut last_update = Instant::now();

        let result = Self::pipelined_read_into_writer(
            raw_session,
            remote_path,
            total_size,
            cancelled,
            &mut writer,
            |transferred| {
                let now = Instant::now();
                if now.duration_since(last_update).as_millis() >= 100 {
                    let elapsed = start_time.elapsed().as_secs_f64();
                    progress(TransferProgress {
                        transferred,
                        total: total_size,
                        speed: if elapsed > 0.0 {
                            transferred as f64 / elapsed
                        } else {
                            0.0
                        },
                        current_file: None,
                        current_file_transferred: 0,
                        current_file_total: 0,
                    });
                    last_update = now;
                }
            },
        )
        .await;

        let transferred = result?;
        progress(TransferProgress {
            transferred,
            total: total_size,
            speed: 0.0,
            current_file: None,
            current_file_transferred: 0,
            current_file_total: 0,
        });
        writer
            .flush()
            .await
            .map_err(|error| anyhow!("Failed to flush local file: {}", error))?;
        writer
            .into_inner()
            .sync_all()
            .await
            .map_err(|error| anyhow!("Failed to sync local file: {}", error))?;
        temporary.commit(target).await
    }

    /// 流水线下载（目录内文件），带 current_file 进度信息
    #[allow(clippy::too_many_arguments)]
    async fn pipelined_download_with_file_progress(
        raw_session: Arc<RawSftpSession>,
        remote_path: &str,
        local_path: &str,
        total_size: u64,
        file_name: &str,
        file_total: u64,
        dir_transferred: &mut u64,
        dir_total: u64,
        start_time: Instant,
        cancelled: &AtomicBool,
        progress: &(dyn Fn(TransferProgress) + Send + Sync),
    ) -> Result<()> {
        let target = Path::new(local_path);
        let (temporary, local_file) = LocalDownloadTemp::create(target).await?;
        let mut writer = BufWriter::with_capacity(BUFFER_SIZE, local_file);
        let base_transferred = *dir_transferred;
        let mut current_file_transferred = 0u64;

        let result = Self::pipelined_read_into_writer(
            raw_session,
            remote_path,
            total_size,
            cancelled,
            &mut writer,
            |transferred| {
                current_file_transferred = transferred;
                let total_transferred = base_transferred + transferred;
                let elapsed = start_time.elapsed().as_secs_f64();
                progress(TransferProgress {
                    transferred: total_transferred,
                    total: dir_total,
                    speed: if elapsed > 0.0 {
                        total_transferred as f64 / elapsed
                    } else {
                        0.0
                    },
                    current_file: Some(file_name.to_owned()),
                    current_file_transferred,
                    current_file_total: file_total,
                });
            },
        )
        .await;

        let transferred = result?;
        writer
            .flush()
            .await
            .map_err(|error| anyhow!("Failed to flush local file: {}", error))?;
        writer
            .into_inner()
            .sync_all()
            .await
            .map_err(|error| anyhow!("Failed to sync local file: {}", error))?;
        temporary.commit(target).await?;
        *dir_transferred = base_transferred + transferred;
        Ok(())
    }

    /// 串行下载（小文件或 raw session 不可用时的后备）
    async fn serial_download_file(
        &mut self,
        remote_path: &str,
        local_path: &str,
        total_size: u64,
        cancelled: Arc<AtomicBool>,
        progress: ProgressCallback,
    ) -> Result<()> {
        let mut remote_file = self
            .sftp
            .open_with_flags(remote_path, OpenFlags::READ)
            .await
            .map_err(|e| anyhow!("Failed to open remote file {}: {}", remote_path, e))?;

        let target = Path::new(local_path);
        let (temporary, local_file) = LocalDownloadTemp::create(target).await?;
        let mut local_file = BufWriter::with_capacity(BUFFER_SIZE, local_file);

        let mut buffer = vec![0u8; BUFFER_SIZE];
        let mut transferred = 0u64;
        let mut last_update = Instant::now();
        let mut speed_samples: Vec<f64> = Vec::new();

        loop {
            ensure_not_cancelled(&cancelled)?;
            let bytes_read = remote_file
                .read(&mut buffer)
                .await
                .map_err(|e| anyhow!("Failed to read from remote file: {}", e))?;

            if bytes_read == 0 {
                break;
            }

            let remaining = total_size.saturating_sub(transferred);
            if bytes_read as u64 > remaining {
                return Err(anyhow!(
                    "Remote file {} changed during download: received {} bytes beyond expected size {}",
                    remote_path,
                    bytes_read,
                    total_size
                ));
            }

            local_file
                .write_all(&buffer[..bytes_read])
                .await
                .map_err(|e| anyhow!("Failed to write to local file: {}", e))?;

            transferred += bytes_read as u64;

            let now = Instant::now();
            let elapsed = now.duration_since(last_update).as_secs_f64();

            if elapsed >= 0.1 {
                let speed = bytes_read as f64 / elapsed;
                speed_samples.push(speed);
                if speed_samples.len() > 10 {
                    speed_samples.remove(0);
                }

                let avg_speed = speed_samples.iter().sum::<f64>() / speed_samples.len() as f64;

                progress(TransferProgress {
                    transferred,
                    total: total_size,
                    speed: avg_speed,
                    current_file: None,
                    current_file_transferred: 0,
                    current_file_total: 0,
                });

                last_update = now;
            }
        }

        if transferred != total_size {
            return Err(anyhow!(
                "Unexpected EOF while downloading {}: received {} of {} bytes",
                remote_path,
                transferred,
                total_size
            ));
        }

        progress(TransferProgress {
            transferred,
            total: total_size,
            speed: 0.0,
            current_file: None,
            current_file_transferred: 0,
            current_file_total: 0,
        });

        local_file
            .flush()
            .await
            .map_err(|e| anyhow!("Failed to flush local file: {}", e))?;
        local_file
            .into_inner()
            .sync_all()
            .await
            .map_err(|e| anyhow!("Failed to sync local file: {}", e))?;

        temporary.commit(target).await
    }

    pub(crate) async fn copy_file_to(
        &mut self,
        target: &mut Self,
        request: CopyFileRequest<'_>,
    ) -> Result<()> {
        let source_path = request.source_path.to_owned();
        let target_path = request.target_path.to_owned();
        let file_size = request.file_size;
        let completed = request.completed;
        let total = request.total;
        let cancelled = request.cancelled;
        let progress = request.progress;
        let source_path_for_error = source_path.clone();
        let target_path_for_error = target_path.clone();
        let target_path_for_write = target_path.clone();

        let mut source_file = self
            .sftp
            .open_with_flags(&source_path, OpenFlags::READ)
            .await
            .map_err(|error| anyhow!("Failed to open {}: {}", source_path, error))?;
        let file_name = request
            .source_path
            .rsplit('/')
            .next()
            .unwrap_or(request.source_path)
            .to_string();
        let started_at = Instant::now();
        let result = with_remote_replace(
            &target.sftp,
            &target_path,
            file_size,
            move |mut target_file| async move {
                let mut buffer = vec![0u8; BUFFER_SIZE];
                let mut file_transferred = 0u64;

                loop {
                    ensure_not_cancelled(&cancelled)?;
                    let read = source_file
                        .read(&mut buffer)
                        .await
                        .map_err(|error| anyhow!("Failed to read {}: {}", source_path, error))?;
                    if read == 0 {
                        break;
                    }
                    target_file
                        .write_all(&buffer[..read])
                        .await
                        .map_err(|error| {
                            anyhow!("Failed to write {}: {}", target_path_for_write, error)
                        })?;
                    file_transferred += read as u64;
                    let elapsed = started_at.elapsed().as_secs_f64();
                    (progress)(TransferProgress {
                        transferred: completed + file_transferred,
                        total,
                        speed: if elapsed > 0.0 {
                            file_transferred as f64 / elapsed
                        } else {
                            0.0
                        },
                        current_file: Some(file_name.clone()),
                        current_file_transferred: file_transferred,
                        current_file_total: file_size,
                    });
                }

                Ok((target_file, file_transferred))
            },
        )
        .await;

        result.map(|_| ()).map_err(|error| {
            anyhow!(
                "Failed to copy {} to {} without replacing the original: {}",
                source_path_for_error,
                target_path_for_error,
                error
            )
        })
    }
}

#[async_trait]
impl SftpClient for RusshSftpClient {
    async fn connect(ssh_config: SshConnectConfig) -> Result<Self> {
        let config = Arc::new(client::Config {
            inactivity_timeout: ssh_config.timeout.or(Some(defaults::INACTIVITY_TIMEOUT)),
            keepalive_interval: ssh_config
                .keepalive_interval
                .or(Some(defaults::KEEPALIVE_INTERVAL)),
            keepalive_max: ssh_config.keepalive_max.unwrap_or(defaults::KEEPALIVE_MAX),
            window_size: 16 * 1024 * 1024, // 16 MB
            maximum_packet_size: 0xFFFF,   // 65535, max allowed by russh
            nodelay: true,
            ..<_>::default()
        });
        let target_host_key_identity = ssh_config.target_host_key_identity();
        let jump_host_key_identity = ssh_config.jump_host_key_identity();
        let host_key_verifier = ssh_config.host_key_verifier.clone();

        let (mut session, jump_session) = if let Some(ref jump) = ssh_config.jump_server {
            tracing::info!("SFTP: 通过跳板机 {}:{} 连接", jump.host, jump.port);
            let jump_identity = jump_host_key_identity
                .clone()
                .expect("jump identity must exist when jump server is configured");

            // 连接跳板机
            let mut jump_session = if let Some(ref proxy) = ssh_config.proxy {
                tracing::info!("SFTP: 通过代理 {}:{} 连接跳板机", proxy.host, proxy.port);
                let stream = sftp_connect_via_proxy(proxy, &jump.host, jump.port).await?;
                let handler = SftpHandler::new(jump_identity, host_key_verifier.clone());
                client::connect_stream(config.clone(), stream, handler).await?
            } else {
                let handler = SftpHandler::new(jump_identity, host_key_verifier.clone());
                client::connect(config.clone(), (jump.host.as_str(), jump.port), handler).await?
            };

            // 认证跳板机
            authenticate_with_strategy(
                &mut jump_session,
                &jump.username,
                &jump.auth,
                sftp_auth_failure_messages(),
            )
            .await?;

            // 通过跳板机转发到目标服务器
            let forwarded_channel = jump_session
                .channel_open_direct_tcpip(&ssh_config.host, ssh_config.port as u32, "127.0.0.1", 0)
                .await?;

            let handler =
                SftpHandler::new(target_host_key_identity.clone(), host_key_verifier.clone());
            let session =
                client::connect_stream(config, forwarded_channel.into_stream(), handler).await?;

            (session, Some(jump_session))
        } else if let Some(ref proxy) = ssh_config.proxy {
            tracing::info!("SFTP: 通过代理 {}:{} 连接", proxy.host, proxy.port);
            let stream = sftp_connect_via_proxy(proxy, &ssh_config.host, ssh_config.port).await?;
            let handler =
                SftpHandler::new(target_host_key_identity.clone(), host_key_verifier.clone());
            let session = client::connect_stream(config, stream, handler).await?;
            (session, None)
        } else {
            let handler =
                SftpHandler::new(target_host_key_identity.clone(), host_key_verifier.clone());
            let session =
                client::connect(config, (ssh_config.host.as_str(), ssh_config.port), handler)
                    .await?;
            (session, None)
        };

        // 认证目标服务器
        authenticate_with_strategy(
            &mut session,
            &ssh_config.username,
            &ssh_config.auth,
            sftp_auth_failure_messages(),
        )
        .await?;

        let channel = session.channel_open_session().await?;
        channel.request_subsystem(true, "sftp").await?;

        let sftp = SftpSession::new(channel.into_stream()).await?;

        Ok(Self {
            sftp,
            owner: SessionOwner::Owned {
                session,
                _jump_session: jump_session,
            },
            raw_sftp: None,
        })
    }

    async fn list_dir(&mut self, path: &str) -> Result<Vec<FileEntry>> {
        let dir_entries = self
            .sftp
            .read_dir(path)
            .await
            .map_err(|e| anyhow!("Failed to read directory {}: {}", path, e))?;

        let mut entries = Vec::new();

        for entry in dir_entries {
            let file_name = entry.file_name();

            if file_name == "." || file_name == ".." {
                continue;
            }

            let metadata = entry.metadata();
            let size = metadata.size.unwrap_or(0);
            let is_dir = metadata.is_dir();
            let permissions = metadata.permissions.unwrap_or(0);

            let modified = metadata
                .mtime
                .and_then(|mtime| UNIX_EPOCH.checked_add(Duration::from_secs(mtime as u64)))
                .unwrap_or_else(SystemTime::now);

            entries.push(FileEntry {
                name: file_name.clone(),
                path: file_name,
                size,
                modified,
                is_dir,
                permissions,
            });
        }

        entries.sort_by(|a, b| {
            if a.is_dir == b.is_dir {
                a.name.to_lowercase().cmp(&b.name.to_lowercase())
            } else if a.is_dir {
                std::cmp::Ordering::Less
            } else {
                std::cmp::Ordering::Greater
            }
        });

        Ok(entries)
    }

    async fn stat(&mut self, path: &str) -> Result<Option<PathMetadata>> {
        let metadata = match self.sftp.metadata(path).await {
            Ok(metadata) => metadata,
            Err(SftpError::Status(status)) if status.status_code == StatusCode::NoSuchFile => {
                return Ok(None);
            }
            Err(error) => return Err(anyhow!("Failed to get remote metadata {}: {}", path, error)),
        };

        let modified = metadata
            .mtime
            .and_then(|mtime| UNIX_EPOCH.checked_add(Duration::from_secs(mtime as u64)))
            .unwrap_or_else(SystemTime::now);

        Ok(Some(PathMetadata {
            size: metadata.size.unwrap_or(0),
            modified,
            is_dir: metadata.is_dir(),
            permissions: metadata.permissions.unwrap_or(0),
        }))
    }

    async fn download_with_progress(
        &mut self,
        remote_path: &str,
        local_path: &str,
        cancelled: Arc<AtomicBool>,
        progress: ProgressCallback,
    ) -> Result<()> {
        let metadata = self
            .sftp
            .metadata(remote_path)
            .await
            .map_err(|e| anyhow!("Failed to get remote file metadata: {}", e))?;

        let total_size = metadata.size.unwrap_or(0);

        // 大文件走流水线下载
        if total_size > PIPELINE_THRESHOLD {
            let raw_session = match self.get_or_create_raw_session().await {
                Ok(raw) => raw,
                Err(e) => {
                    tracing::warn!(
                        "Failed to create raw SFTP session, falling back to serial: {}",
                        e
                    );
                    self.raw_sftp = None;
                    return self
                        .serial_download_file(
                            remote_path,
                            local_path,
                            total_size,
                            cancelled,
                            progress,
                        )
                        .await;
                }
            };

            let result = Self::pipelined_download(
                raw_session,
                remote_path,
                local_path,
                total_size,
                &cancelled,
                &progress,
            )
            .await;

            if result.is_err() {
                // raw session 出错时置空，下次重建
                self.raw_sftp = None;
            }

            return result;
        }

        self.serial_download_file(remote_path, local_path, total_size, cancelled, progress)
            .await
    }

    async fn upload_with_progress(
        &mut self,
        local_path: &str,
        remote_path: &str,
        cancelled: Arc<AtomicBool>,
        progress: ProgressCallback,
    ) -> Result<()> {
        let local_file = File::open(local_path)
            .await
            .map_err(|e| anyhow!("Failed to open local file {}: {}", local_path, e))?;

        let metadata = local_file
            .metadata()
            .await
            .map_err(|e| anyhow!("Failed to get local file metadata: {}", e))?;

        let total_size = metadata.len();

        let mut local_file = BufReader::with_capacity(BUFFER_SIZE, local_file);

        let result = with_remote_replace(
            &self.sftp,
            remote_path,
            total_size,
            |mut remote_file| async {
                let mut buffer = vec![0u8; BUFFER_SIZE];
                let mut transferred = 0u64;
                let mut last_update = Instant::now();
                let mut speed_samples: Vec<f64> = Vec::new();

                loop {
                    ensure_not_cancelled(&cancelled)?;
                    let bytes_read = local_file
                        .read(&mut buffer)
                        .await
                        .map_err(|e| anyhow!("Failed to read from local file: {}", e))?;

                    if bytes_read == 0 {
                        break;
                    }

                    remote_file
                        .write_all(&buffer[..bytes_read])
                        .await
                        .map_err(|e| anyhow!("Failed to write to remote file: {}", e))?;

                    transferred += bytes_read as u64;

                    let now = Instant::now();
                    let elapsed = now.duration_since(last_update).as_secs_f64();

                    if elapsed >= 0.1 {
                        let speed = bytes_read as f64 / elapsed;
                        speed_samples.push(speed);
                        if speed_samples.len() > 10 {
                            speed_samples.remove(0);
                        }

                        let avg_speed =
                            speed_samples.iter().sum::<f64>() / speed_samples.len() as f64;

                        progress(TransferProgress {
                            transferred,
                            total: total_size,
                            speed: avg_speed,
                            current_file: None,
                            current_file_transferred: 0,
                            current_file_total: 0,
                        });

                        last_update = now;
                    }
                }

                ensure_not_cancelled(&cancelled)?;
                Ok((remote_file, transferred))
            },
        )
        .await;

        let transferred = result.map_err(|error| {
            anyhow!(
                "Failed to upload {} without replacing the original: {}",
                remote_path,
                error
            )
        })?;

        progress(TransferProgress {
            transferred,
            total: total_size,
            speed: 0.0,
            current_file: None,
            current_file_transferred: 0,
            current_file_total: 0,
        });

        Ok(())
    }

    async fn delete(&mut self, path: &str, is_dir: bool) -> Result<()> {
        if is_dir {
            self.sftp
                .remove_dir(path)
                .await
                .map_err(|e| anyhow!("Failed to remove directory {}: {}", path, e))?;
        } else {
            self.sftp
                .remove_file(path)
                .await
                .map_err(|e| anyhow!("Failed to remove file {}: {}", path, e))?;
        }
        Ok(())
    }

    async fn delete_recursive(
        &mut self,
        path: &str,
        cancelled: Arc<AtomicBool>,
        progress: ProgressCallback,
    ) -> Result<()> {
        let entries = self.list_dir_recursive(path, cancelled.clone()).await?;

        // 计算总数：文件数 + 目录数 + 根目录本身
        let file_count = entries.iter().filter(|e| !e.is_dir).count();
        let dir_count = entries.iter().filter(|e| e.is_dir).count();
        let total = (file_count + dir_count + 1) as u64;
        let mut deleted: u64 = 0;

        // 先删除所有文件
        for entry in &entries {
            ensure_not_cancelled(&cancelled)?;
            if !entry.is_dir {
                progress(TransferProgress {
                    transferred: deleted,
                    total,
                    speed: 0.0,
                    current_file: Some(entry.name.clone()),
                    current_file_transferred: 0,
                    current_file_total: 1,
                });

                self.sftp
                    .remove_file(&entry.path)
                    .await
                    .map_err(|e| anyhow!("Failed to remove file {}: {}", entry.path, e))?;

                deleted += 1;
                progress(TransferProgress {
                    transferred: deleted,
                    total,
                    speed: 0.0,
                    current_file: Some(entry.name.clone()),
                    current_file_transferred: 1,
                    current_file_total: 1,
                });
            }
        }

        // 按路径深度倒序删除目录（先删子目录）
        let mut dirs: Vec<&FileEntry> = entries.iter().filter(|e| e.is_dir).collect();
        dirs.sort_by_key(|dir| std::cmp::Reverse(dir.path.len()));
        for dir in dirs {
            ensure_not_cancelled(&cancelled)?;
            progress(TransferProgress {
                transferred: deleted,
                total,
                speed: 0.0,
                current_file: Some(dir.name.clone()),
                current_file_transferred: 0,
                current_file_total: 1,
            });

            self.sftp
                .remove_dir(&dir.path)
                .await
                .map_err(|e| anyhow!("Failed to remove directory {}: {}", dir.path, e))?;

            deleted += 1;
            progress(TransferProgress {
                transferred: deleted,
                total,
                speed: 0.0,
                current_file: Some(dir.name.clone()),
                current_file_transferred: 1,
                current_file_total: 1,
            });
        }

        // 最后删除根目录本身
        let root_name = path.rsplit('/').next().unwrap_or(path).to_string();
        ensure_not_cancelled(&cancelled)?;
        progress(TransferProgress {
            transferred: deleted,
            total,
            speed: 0.0,
            current_file: Some(root_name.clone()),
            current_file_transferred: 0,
            current_file_total: 1,
        });

        self.sftp
            .remove_dir(path)
            .await
            .map_err(|e| anyhow!("Failed to remove directory {}: {}", path, e))?;

        deleted += 1;
        progress(TransferProgress {
            transferred: deleted,
            total,
            speed: 0.0,
            current_file: Some(root_name),
            current_file_transferred: 1,
            current_file_total: 1,
        });

        Ok(())
    }

    async fn mkdir(&mut self, path: &str) -> Result<()> {
        self.sftp
            .create_dir(path)
            .await
            .map_err(|e| anyhow!("Failed to create directory {}: {}", path, e))?;
        Ok(())
    }

    async fn rename(&mut self, old_path: &str, new_path: &str) -> Result<()> {
        self.sftp
            .rename(old_path, new_path)
            .await
            .map_err(|e| anyhow!("Failed to rename {} to {}: {}", old_path, new_path, e))?;
        Ok(())
    }

    async fn chmod(&mut self, _path: &str, _mode: u32) -> Result<()> {
        anyhow::bail!("chmod not yet supported")
    }

    async fn read_file(&mut self, path: &str, max_bytes: usize) -> Result<Vec<u8>> {
        let metadata = self
            .sftp
            .metadata(path)
            .await
            .map_err(|e| anyhow!("Failed to get remote file metadata {}: {}", path, e))?;

        let total_size = metadata.size.unwrap_or(0) as usize;
        validate_read_size(total_size, max_bytes)?;

        let mut remote_file = self
            .sftp
            .open_with_flags(path, OpenFlags::READ)
            .await
            .map_err(|e| anyhow!("Failed to open remote file {}: {}", path, e))?;

        let capacity = total_size.min(max_bytes);
        let mut content = Vec::with_capacity(capacity);
        let mut buffer = vec![0u8; BUFFER_SIZE];

        loop {
            let bytes_read = remote_file
                .read(&mut buffer)
                .await
                .map_err(|e| anyhow!("Failed to read remote file {}: {}", path, e))?;

            if bytes_read == 0 {
                break;
            }

            content.extend_from_slice(&buffer[..bytes_read]);
            validate_read_size(content.len(), max_bytes)?;
        }

        if content.len() != total_size {
            return Err(anyhow!(
                "Unexpected EOF while reading remote file {}: received {} of {} bytes",
                path,
                content.len(),
                total_size
            ));
        }

        Ok(content)
    }

    async fn write_file(&mut self, path: &str, content: &[u8]) -> Result<()> {
        with_remote_replace(
            &self.sftp,
            path,
            content.len() as u64,
            |mut remote_file| async {
                if !content.is_empty() {
                    remote_file
                        .write_all(content)
                        .await
                        .map_err(|e| anyhow!("Failed to write to remote file {}: {}", path, e))?;
                }

                Ok((remote_file, content.len() as u64))
            },
        )
        .await
        .map(|_| ())
        .map_err(|error| {
            anyhow!(
                "Failed to save {} without replacing the original: {}",
                path,
                error
            )
        })
    }

    async fn list_dir_recursive(
        &mut self,
        path: &str,
        cancelled: Arc<AtomicBool>,
    ) -> Result<Vec<FileEntry>> {
        let mut all_entries = Vec::new();
        let mut dirs_to_process = vec![path.to_string()];

        while let Some(current_dir) = dirs_to_process.pop() {
            ensure_not_cancelled(&cancelled)?;
            let entries = self.list_dir(&current_dir).await?;

            for entry in entries {
                ensure_not_cancelled(&cancelled)?;
                let full_path = if current_dir == "/" {
                    format!("/{}", entry.name)
                } else {
                    format!("{}/{}", current_dir, entry.name)
                };

                if entry.is_dir {
                    dirs_to_process.push(full_path.clone());
                }

                all_entries.push(FileEntry {
                    name: entry.name,
                    path: full_path,
                    size: entry.size,
                    modified: entry.modified,
                    is_dir: entry.is_dir,
                    permissions: entry.permissions,
                });
            }
        }

        Ok(all_entries)
    }

    async fn download_dir_with_progress(
        &mut self,
        remote_path: &str,
        local_path: &str,
        cancelled: Arc<AtomicBool>,
        progress: ProgressCallback,
    ) -> Result<()> {
        let entries = self
            .list_dir_recursive(remote_path, cancelled.clone())
            .await?;

        let total_size: u64 = entries.iter().filter(|e| !e.is_dir).map(|e| e.size).sum();
        let mut transferred: u64 = 0;

        let base_remote = remote_path.trim_end_matches('/');
        let base_local = std::path::Path::new(local_path);

        std::fs::create_dir_all(base_local)
            .map_err(|e| anyhow!("Failed to create local directory {}: {}", local_path, e))?;

        let mut dirs: Vec<&FileEntry> = entries.iter().filter(|e| e.is_dir).collect();
        dirs.sort_by_key(|dir| dir.path.len());
        for dir_entry in dirs {
            ensure_not_cancelled(&cancelled)?;
            let relative = dir_entry
                .path
                .strip_prefix(base_remote)
                .unwrap_or(&dir_entry.path);
            let relative = relative.trim_start_matches('/');
            if relative.is_empty() {
                continue;
            }
            let local_dir = base_local.join(relative);
            std::fs::create_dir_all(&local_dir)
                .map_err(|e| anyhow!("Failed to create directory {:?}: {}", local_dir, e))?;
        }

        let files: Vec<&FileEntry> = entries.iter().filter(|e| !e.is_dir).collect();
        let start_time = Instant::now();

        // 检查是否有大文件需要流水线下载
        let has_large_files = files.iter().any(|f| f.size > PIPELINE_THRESHOLD);
        let raw_session = if has_large_files {
            match self.get_or_create_raw_session().await {
                Ok(raw) => Some(raw),
                Err(e) => {
                    tracing::warn!(
                        "Failed to create raw SFTP session, falling back to serial: {}",
                        e
                    );
                    self.raw_sftp = None;
                    None
                }
            }
        } else {
            None
        };

        for file_entry in files {
            ensure_not_cancelled(&cancelled)?;
            let relative = file_entry
                .path
                .strip_prefix(base_remote)
                .unwrap_or(&file_entry.path);
            let relative = relative.trim_start_matches('/');
            let local_file = base_local.join(relative);

            let current_file_name = file_entry.name.clone();
            let current_file_total = file_entry.size;

            if let Some(parent) = local_file.parent() {
                std::fs::create_dir_all(parent).map_err(|e| {
                    anyhow!("Failed to create parent directory {:?}: {}", parent, e)
                })?;
            }

            let local_file_str = local_file.to_string_lossy().to_string();

            // 大文件走流水线
            if file_entry.size > PIPELINE_THRESHOLD {
                if let Some(ref raw) = raw_session {
                    let result = Self::pipelined_download_with_file_progress(
                        Arc::clone(raw),
                        &file_entry.path,
                        &local_file_str,
                        file_entry.size,
                        &current_file_name,
                        current_file_total,
                        &mut transferred,
                        total_size,
                        start_time,
                        &cancelled,
                        &progress,
                    )
                    .await;

                    if result.is_err() {
                        self.raw_sftp = None;
                        return result;
                    }
                    continue;
                }
            }

            // 小文件或没有 raw session 时走串行下载
            let mut remote_file = self
                .sftp
                .open_with_flags(&file_entry.path, OpenFlags::READ)
                .await
                .map_err(|e| anyhow!("Failed to open remote file {}: {}", file_entry.path, e))?;

            let (temporary, local_file_handle) = LocalDownloadTemp::create(&local_file).await?;
            let mut local_file_handle = BufWriter::with_capacity(BUFFER_SIZE, local_file_handle);

            let mut buffer = vec![0u8; BUFFER_SIZE];
            let mut current_file_transferred: u64 = 0;

            loop {
                ensure_not_cancelled(&cancelled)?;
                let bytes_read = remote_file
                    .read(&mut buffer)
                    .await
                    .map_err(|e| anyhow!("Failed to read from remote file: {}", e))?;

                if bytes_read == 0 {
                    break;
                }

                let remaining = file_entry.size.saturating_sub(current_file_transferred);
                if bytes_read as u64 > remaining {
                    return Err(anyhow!(
                        "Remote file {} changed during download: received bytes beyond expected size {}",
                        file_entry.path,
                        file_entry.size
                    ));
                }

                local_file_handle
                    .write_all(&buffer[..bytes_read])
                    .await
                    .map_err(|e| anyhow!("Failed to write to local file: {}", e))?;

                transferred += bytes_read as u64;
                current_file_transferred += bytes_read as u64;

                let elapsed = start_time.elapsed().as_secs_f64();
                let speed = if elapsed > 0.0 {
                    transferred as f64 / elapsed
                } else {
                    0.0
                };

                progress(TransferProgress {
                    transferred,
                    total: total_size,
                    speed,
                    current_file: Some(current_file_name.clone()),
                    current_file_transferred,
                    current_file_total,
                });
            }

            if current_file_transferred != file_entry.size {
                return Err(anyhow!(
                    "Unexpected EOF while downloading {}: received {} of {} bytes",
                    file_entry.path,
                    current_file_transferred,
                    file_entry.size
                ));
            }

            local_file_handle
                .flush()
                .await
                .map_err(|e| anyhow!("Failed to flush local file: {}", e))?;
            local_file_handle
                .into_inner()
                .sync_all()
                .await
                .map_err(|e| anyhow!("Failed to sync local file: {}", e))?;
            temporary.commit(&local_file).await?;
        }

        progress(TransferProgress {
            transferred,
            total: total_size,
            speed: 0.0,
            current_file: None,
            current_file_transferred: 0,
            current_file_total: 0,
        });

        Ok(())
    }

    async fn upload_dir_with_progress(
        &mut self,
        local_path: &str,
        remote_path: &str,
        cancelled: Arc<AtomicBool>,
        progress: ProgressCallback,
    ) -> Result<()> {
        let local_base = std::path::Path::new(local_path);
        if !local_base.is_dir() {
            anyhow::bail!("Local path is not a directory: {}", local_path);
        }

        let mut entries: Vec<(std::path::PathBuf, bool, u64)> = Vec::new();
        let mut dirs_to_scan = vec![local_base.to_path_buf()];

        while let Some(dir) = dirs_to_scan.pop() {
            ensure_not_cancelled(&cancelled)?;
            let read_dir = std::fs::read_dir(&dir)
                .map_err(|e| anyhow!("Failed to read directory {:?}: {}", dir, e))?;

            for entry in read_dir {
                let entry = entry.map_err(|e| anyhow!("Failed to read entry: {}", e))?;
                let path = entry.path();
                let metadata = entry
                    .metadata()
                    .map_err(|e| anyhow!("Failed to get metadata for {:?}: {}", path, e))?;

                if metadata.is_dir() {
                    entries.push((path.clone(), true, 0));
                    dirs_to_scan.push(path);
                } else {
                    entries.push((path, false, metadata.len()));
                }
            }
        }

        let total_size: u64 = entries
            .iter()
            .filter(|(_, is_dir, _)| !is_dir)
            .map(|(_, _, size)| size)
            .sum();
        let mut transferred: u64 = 0;

        let _ = self.sftp.create_dir(remote_path).await;

        let mut dirs: Vec<_> = entries.iter().filter(|(_, is_dir, _)| *is_dir).collect();
        dirs.sort_by_key(|dir| dir.0.as_os_str().len());

        for (dir_path, _, _) in dirs {
            ensure_not_cancelled(&cancelled)?;
            let relative = dir_path
                .strip_prefix(local_base)
                .map_err(|e| anyhow!("Failed to strip prefix: {}", e))?;
            let relative_str = relative.to_string_lossy();
            if relative_str.is_empty() {
                continue;
            }
            let remote_dir = format!(
                "{}/{}",
                remote_path.trim_end_matches('/'),
                relative_str.replace('\\', "/")
            );
            let _ = self.sftp.create_dir(&remote_dir).await;
        }

        let files: Vec<_> = entries.iter().filter(|(_, is_dir, _)| !*is_dir).collect();
        let start_time = Instant::now();

        for (file_path, _, file_size) in files {
            ensure_not_cancelled(&cancelled)?;
            let relative = file_path
                .strip_prefix(local_base)
                .map_err(|e| anyhow!("Failed to strip prefix: {}", e))?;
            let relative_str = relative.to_string_lossy().replace('\\', "/");
            let remote_file_path =
                format!("{}/{}", remote_path.trim_end_matches('/'), relative_str);

            let current_file_name = file_path
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_default();

            let local_file = File::open(file_path)
                .await
                .map_err(|e| anyhow!("Failed to open local file {:?}: {}", file_path, e))?;
            let mut local_file = BufReader::with_capacity(BUFFER_SIZE, local_file);
            let committed_before = transferred;
            let expected_size = *file_size;
            let current_file_transferred = with_remote_replace(
                &self.sftp,
                &remote_file_path,
                expected_size,
                |mut remote_file| async {
                    let mut buffer = vec![0u8; BUFFER_SIZE];
                    let mut current_file_transferred: u64 = 0;

                    loop {
                        ensure_not_cancelled(&cancelled)?;
                        let bytes_read = local_file
                            .read(&mut buffer)
                            .await
                            .map_err(|e| anyhow!("Failed to read from local file: {}", e))?;

                        if bytes_read == 0 {
                            break;
                        }

                        remote_file
                            .write_all(&buffer[..bytes_read])
                            .await
                            .map_err(|e| anyhow!("Failed to write to remote file: {}", e))?;

                        current_file_transferred += bytes_read as u64;

                        let elapsed = start_time.elapsed().as_secs_f64();
                        let displayed_transferred = committed_before + current_file_transferred;
                        let speed = if elapsed > 0.0 {
                            displayed_transferred as f64 / elapsed
                        } else {
                            0.0
                        };

                        progress(TransferProgress {
                            transferred: displayed_transferred,
                            total: total_size,
                            speed,
                            current_file: Some(current_file_name.clone()),
                            current_file_transferred,
                            current_file_total: expected_size,
                        });
                    }

                    ensure_not_cancelled(&cancelled)?;
                    Ok((remote_file, current_file_transferred))
                },
            )
            .await
            .map_err(|error| {
                anyhow!(
                    "Failed to upload {} without replacing the original: {}",
                    remote_file_path,
                    error
                )
            })?;
            transferred = committed_before + current_file_transferred;
        }

        progress(TransferProgress {
            transferred,
            total: total_size,
            speed: 0.0,
            current_file: None,
            current_file_transferred: 0,
            current_file_total: 0,
        });

        Ok(())
    }

    async fn disconnect(&mut self) -> Result<()> {
        Ok(())
    }

    async fn realpath(&mut self, path: &str) -> Result<String> {
        let real_path = self
            .sftp
            .canonicalize(path)
            .await
            .map_err(|e| anyhow!("Failed to get realpath for {}: {}", path, e))?;
        Ok(real_path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pipeline_chunk_length_covers_full_and_final_partial_chunks() {
        let chunk_size = PIPELINE_CHUNK_SIZE as u64;
        let total_size = chunk_size + 17;

        assert_eq!(
            expected_chunk_len(0, total_size).expect("first chunk must be valid"),
            PIPELINE_CHUNK_SIZE as usize
        );
        assert_eq!(
            expected_chunk_len(chunk_size, total_size).expect("final chunk must be valid"),
            17
        );
    }

    #[test]
    fn pipeline_chunk_validation_rejects_short_and_out_of_range_reads() {
        let short_read = validate_chunk_len(
            0,
            PIPELINE_CHUNK_SIZE as u64,
            PIPELINE_CHUNK_SIZE as usize - 1,
        )
        .expect_err("short reads must fail");
        assert!(short_read.to_string().contains("SFTP short read"));

        let out_of_range =
            expected_chunk_len(10, 10).expect_err("offset at EOF must not be scheduled");
        assert!(out_of_range.to_string().contains("out-of-range chunk"));
    }

    #[test]
    fn remote_replace_temp_stays_beside_relative_target() {
        let root_relative =
            RemoteReplaceTemp::path_for("notes.txt").expect("relative file target must be valid");
        assert!(
            root_relative.starts_with("./.notes.txt.navop-part-"),
            "unexpected temporary path: {root_relative}"
        );

        let nested = RemoteReplaceTemp::path_for("documents/notes.txt")
            .expect("nested relative file target must be valid");
        assert!(
            nested.starts_with("documents/.notes.txt.navop-part-"),
            "unexpected temporary path: {nested}"
        );
    }

    #[test]
    fn remote_replace_temp_stays_beside_absolute_target() {
        let root =
            RemoteReplaceTemp::path_for("/notes.txt").expect("root file target must be valid");
        assert!(
            root.starts_with("/.notes.txt.navop-part-"),
            "unexpected temporary path: {root}"
        );

        let nested = RemoteReplaceTemp::path_for("/srv/documents/notes.txt")
            .expect("nested absolute file target must be valid");
        assert!(
            nested.starts_with("/srv/documents/.notes.txt.navop-part-"),
            "unexpected temporary path: {nested}"
        );
    }

    #[test]
    fn remote_replace_temp_rejects_non_file_targets() {
        for invalid in ["", "/", "documents/", ".", "..", "/srv/."] {
            assert!(
                RemoteReplaceTemp::path_for(invalid).is_err(),
                "{invalid:?} must not be accepted as a remote file target"
            );
        }
    }

    #[tokio::test]
    async fn local_download_temp_preserves_target_until_commit() {
        let directory = tempfile::tempdir().expect("temporary directory must be created");
        let target = directory.path().join("download.txt");
        fs::write(&target, b"old")
            .await
            .expect("existing target must be written");

        let (temporary, mut file) = LocalDownloadTemp::create(&target)
            .await
            .expect("download temporary file must be created");
        file.write_all(b"partial")
            .await
            .expect("partial data must be writable");
        file.flush().await.expect("partial data must flush");

        assert_eq!(
            fs::read(&target)
                .await
                .expect("existing target must remain readable"),
            b"old"
        );

        let temporary_path = temporary.path.clone();
        drop(file);
        drop(temporary);

        assert_eq!(
            fs::read(&target)
                .await
                .expect("existing target must remain after failure"),
            b"old"
        );
        assert!(
            fs::metadata(temporary_path).await.is_err(),
            "abandoned temporary file must be removed"
        );
    }

    #[tokio::test]
    async fn local_download_temp_atomically_replaces_existing_target() {
        let directory = tempfile::tempdir().expect("temporary directory must be created");
        let target = directory.path().join("download.txt");
        fs::write(&target, b"old")
            .await
            .expect("existing target must be written");

        let (temporary, mut file) = LocalDownloadTemp::create(&target)
            .await
            .expect("download temporary file must be created");
        let temporary_path = temporary.path.clone();
        file.write_all(b"complete")
            .await
            .expect("complete data must be writable");
        file.flush().await.expect("complete data must flush");
        file.sync_all().await.expect("complete data must sync");
        drop(file);

        temporary
            .commit(&target)
            .await
            .expect("verified download must replace target");

        assert_eq!(
            fs::read(&target)
                .await
                .expect("committed target must be readable"),
            b"complete"
        );
        assert!(
            fs::metadata(temporary_path).await.is_err(),
            "temporary name must disappear after commit"
        );
    }
}
