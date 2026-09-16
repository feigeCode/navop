//! FTP 传输实现：安全文件下载/上传与递归目录操作。
//!
//! 文件安全策略：
//! - 远端下载：先写同目录**独占创建**的临时文件，完成并 flush 后
//!   通过 rename 原子替换目标（Unix rename / Windows MoveFileEx
//!   REPLACE_EXISTING）；失败删除临时文件。绝不先删除旧目标，
//!   避免"删除成功而 rename 失败"丢失原文件。
//! - 本地上传：先上传到远端临时路径，成功后远端 rename；失败清理临时文件，
//!   不直接截断已有文件。编辑器保存（write_file）复用同一契约。
//! - LIST 返回的名称视为不可信输入：单层名称校验 + 本地目标逐级
//!   符号链接检查，防止恶意服务端把写入导出用户所选目录之外。
//! - 取消/停滞：数据读写用 select! 轮询取消标志并可唤醒；
//!   连接超时覆盖完整建连流程；取消或中途断线后连接置为不可复用。
//! - 传输状态保护：`RETR`/`STOR` 一旦开始（数据连接已建立），任何
//!   退出路径——数据读取失败、本地写入失败、超限退出、终态响应
//!   不确定——都必须将连接置毒，只有完整消费了终态响应才允许复用。

use anyhow::{Result, anyhow};
use async_ftp::FtpStream;
use sftp::{DirectoryConflictPolicy, FileEntry, TransferCancelled, TransferProgress};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{Duration, SystemTime};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

use crate::listing::is_valid_remote_name;

/// 数据连接读取分块大小。
const CHUNK_SIZE: usize = 256 * 1024;

/// 取消标志轮询间隔：停滞的 I/O 最多在这个时间内被取消唤醒。
const CANCEL_POLL_INTERVAL: Duration = Duration::from_millis(200);
/// I/O 空闲超时：数据连接超过该时长没有任何数据视为停滞。
const IO_IDLE_TIMEOUT: Duration = Duration::from_secs(30);
/// 单条命令/收尾响应的超时上限。
pub(super) const COMMAND_TIMEOUT: Duration = Duration::from_secs(15);
/// 断开连接（QUIT）的超时上限。
pub(super) const QUIT_TIMEOUT: Duration = Duration::from_secs(5);

/// put 传输的结果；取消时以 Cancelled 标记（future 中途丢弃，协议状态未知）。
enum PutOutcome {
    Done(std::result::Result<(), async_ftp::types::FtpError>),
    Cancelled,
}

/// 数据连接停滞（空闲超时）标记；用于区分"可重试的路径错误"与"连接已坏"。
#[derive(Debug)]
struct ConnectionStalled;

impl std::fmt::Display for ConnectionStalled {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("connection stalled")
    }
}

impl std::error::Error for ConnectionStalled {}

pub(super) fn ensure_not_cancelled(cancelled: &AtomicBool) -> Result<()> {
    if cancelled.load(Ordering::Relaxed) {
        Err(TransferCancelled.into())
    } else {
        Ok(())
    }
}

pub(super) fn child_path(parent: &str, name: &str) -> String {
    format!("{}/{}", parent.trim_end_matches('/'), name)
}

static TEMP_SEQUENCE: AtomicU64 = AtomicU64::new(0);

/// 生成真正唯一的临时名后缀：进程 ID + 进程内序号 + 纳秒时间。
/// 只带进程 ID 的临时名在同进程并发传输时会互相冲突。
pub(super) fn unique_temp_suffix() -> String {
    let nanos = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|value| value.as_nanos() as u64)
        .unwrap_or_default();
    format!(
        "{}-{}-{}",
        std::process::id(),
        TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed),
        nanos % 1_000_000_000
    )
}

/// 生成远端临时路径（上传均先写临时目标，成功后再改名）。
pub(super) fn temp_path(path: &str) -> String {
    format!(
        "{}.navop-tmp-{}",
        path.trim_end_matches('/'),
        unique_temp_suffix()
    )
}

/// 校验远端相对路径：每个分量都必须是"单层名称"。
///
/// LIST 名称来自可能被攻陷的服务端；拒绝 `..`、`.`、空分量、
/// 包含 `\` 或 NUL 的分量，防止拼接出逃逸目标目录的路径。
pub(super) fn validate_relative_path(relative: &str) -> Result<Vec<&str>> {
    let components: Vec<&str> = relative.split('/').collect();
    if components.is_empty() {
        return Err(anyhow!("Refusing empty remote entry path"));
    }
    for component in &components {
        if !is_valid_remote_name(component) {
            return Err(anyhow!("Refusing unsafe remote entry name: {relative:?}"));
        }
    }
    Ok(components)
}

/// 确保本地目录存在；已存在的路径若是符号链接则拒绝穿过，
/// 防止预置符号链接把写入导向所选目录之外。
async fn ensure_dir_checked(dir: &std::path::Path) -> Result<()> {
    match tokio::fs::symlink_metadata(dir).await {
        Ok(metadata) => {
            if metadata.file_type().is_symlink() {
                return Err(anyhow!(
                    "Refusing to traverse local symlink: {}",
                    dir.display()
                ));
            }
            if metadata.is_dir() {
                return Ok(());
            }
            Err(anyhow!("Local path is not a directory: {}", dir.display()))
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            tokio::fs::create_dir(dir).await?;
            Ok(())
        }
        Err(error) => Err(error.into()),
    }
}

/// 等待一个传输准备阶段（如 SIZE、RETR、LIST）：取消标志（如提供）在
/// 轮询间隔内可唤醒，且带操作级超时兜底（服务端对单条命令不响应时
/// 不会无限挂起）。
///
/// 返回 `Err` 表示被取消或超时：命令 future 已被丢弃，控制连接上可能
/// 残留未消费的响应，协议状态未知，调用方必须将连接置毒。
/// 命令本身的服务端拒绝（如 550，响应已被完整消费）以 `Ok(Err(..))`
/// 形态返回，连接仍可用。
pub(super) async fn await_stage<T>(
    future: impl std::future::Future<Output = T>,
    cancelled: Option<&AtomicBool>,
    what: &str,
) -> Result<T> {
    let stage = async {
        let mut future = std::pin::pin!(future);
        loop {
            tokio::select! {
                value = future.as_mut() => return Ok::<T, anyhow::Error>(value),
                _ = tokio::time::sleep(CANCEL_POLL_INTERVAL) => {
                    if cancelled.is_some_and(|cancelled| cancelled.load(Ordering::Relaxed)) {
                        return Err(TransferCancelled.into());
                    }
                }
            }
        }
    };
    match tokio::time::timeout(COMMAND_TIMEOUT, stage).await {
        Ok(result) => result,
        Err(_) => Err(anyhow!("Timed out waiting for FTP {what}")),
    }
}

/// async_ftp 把非预期应答包装成 InvalidResponse("Expected code …, got response: 550 …")。
/// 从中提取服务端状态码；连接层错误（ConnectionError 等）返回 `None`。
pub(super) fn ftp_status_code(error: &async_ftp::types::FtpError) -> Option<u32> {
    let text = match error {
        async_ftp::types::FtpError::InvalidResponse(text) => text.as_str(),
        _ => return None,
    };
    let marker = "got response: ";
    let index = text.find(marker)? + marker.len();
    text[index..].get(0..3)?.parse().ok()
}

/// 可取消、带 I/O 空闲超时的数据块读取。
///
/// 用 select! 轮询取消标志：即使读操作正等待数据，取消标志一置位
/// 也会在 `CANCEL_POLL_INTERVAL` 内被唤醒；超过 `IO_IDLE_TIMEOUT`
/// 没有任何数据则判定连接停滞并中止。
pub(super) async fn read_chunk<R: tokio::io::AsyncRead + Unpin>(
    reader: &mut R,
    chunk: &mut [u8],
    cancelled: Option<&AtomicBool>,
    last_data: &mut tokio::time::Instant,
) -> Result<usize> {
    loop {
        if let Some(cancelled) = cancelled {
            if cancelled.load(Ordering::Relaxed) {
                return Err(TransferCancelled.into());
            }
        }
        tokio::select! {
            read = reader.read(chunk) => {
                let read = read?;
                if read > 0 {
                    *last_data = tokio::time::Instant::now();
                }
                return Ok(read);
            }
            _ = tokio::time::sleep(CANCEL_POLL_INTERVAL) => {
                if last_data.elapsed() > IO_IDLE_TIMEOUT {
                    return Err(anyhow::Error::new(ConnectionStalled).context(format!(
                        "FTP data connection idle for over {}s",
                        IO_IDLE_TIMEOUT.as_secs()
                    )));
                }
            }
        }
    }
}

impl crate::FtpClient {
    /// 目录内容列表。
    ///
    /// LIST 本身有取消与超时保护（`cancelled` 可为 `None`，此时仅超时
    /// 兜底）；取消或超时时命令 future 被丢弃，控制连接状态未知 → 置毒。
    ///
    /// 解析区分"可忽略行"（`total` 汇总、`.`/`..`、空行）与"异常行"：
    /// 只有出现异常行且没有任何有效条目时才报错——
    /// 合法的空目录（只含汇总行或 `.`/`..`）必须返回空列表，
    /// 否则递归下载/删除会在空子目录上中断。
    pub(super) async fn list_entries(
        &mut self,
        path: &str,
        cancelled: Option<&AtomicBool>,
    ) -> Result<Vec<FileEntry>> {
        let stage = await_stage(
            self.stream.list((!path.is_empty()).then_some(path)),
            cancelled,
            "directory listing",
        )
        .await;
        let lines = match stage {
            Ok(Ok(lines)) => lines,
            // 服务端明确拒绝 LIST（响应已被完整消费）：连接仍可用。
            Ok(Err(error)) => return Err(error.into()),
            // 取消或超时：LIST 响应可能未被消费，连接状态未知。
            Err(error) => {
                self.poison();
                return Err(error);
            }
        };
        let mut entries = Vec::new();
        let mut unparsable = 0usize;
        let mut sample: Option<&str> = None;
        for line in &lines {
            match crate::listing::parse_list_line(line) {
                crate::listing::ListLine::Entry(entry) => entries.push(entry),
                crate::listing::ListLine::Ignorable => {}
                crate::listing::ListLine::Unparsable => {
                    unparsable += 1;
                    if sample.is_none() {
                        sample = Some(line);
                    }
                    tracing::warn!("FTP LIST: skipping unparsable line: {:?}", line);
                }
            }
        }
        if entries.is_empty() && unparsable > 0 {
            return Err(anyhow!(
                "Failed to parse any entries from FTP LIST output ({unparsable} unparsable lines, sample: {:?})",
                sample.map(str::trim).unwrap_or("")
            ));
        }
        Ok(entries)
    }

    /// 递归枚举目录下所有文件与子目录（不含根目录本身）。
    pub(super) async fn collect_recursive_entries(
        &mut self,
        path: &str,
        cancelled: &AtomicBool,
    ) -> Result<Vec<FileEntry>> {
        let mut result = Vec::new();
        self.collect_recursive_entries_inner(path, cancelled, &mut result)
            .await?;
        Ok(result)
    }

    fn collect_recursive_entries_inner<'a>(
        &'a mut self,
        path: &'a str,
        cancelled: &'a AtomicBool,
        result: &'a mut Vec<FileEntry>,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<()>> + Send + 'a>> {
        Box::pin(async move {
            for entry in self.list_entries(path, Some(cancelled)).await? {
                ensure_not_cancelled(cancelled)?;
                let child = child_path(path, &entry.name);
                if entry.is_dir {
                    self.collect_recursive_entries_inner(&child, cancelled, result)
                        .await?;
                }
                result.push(FileEntry {
                    path: child.clone(),
                    ..entry
                });
            }
            Ok(())
        })
    }

    /// 安全下载：RETR 到独占创建的同目录临时文件 → flush → 原子 rename。
    pub(super) async fn download_file(
        &mut self,
        remote_path: &str,
        local_path: &str,
        cancelled: &AtomicBool,
        progress: impl Fn(TransferProgress) + Send + Sync,
    ) -> Result<()> {
        self.ensure_usable()?;
        // SIZE 探测（可选信息）：取消/超时时响应可能未被消费，连接置毒；
        // 服务端拒绝（550 等）响应已被完整消费，忽略即可。
        let stage = await_stage(self.stream.size(remote_path), Some(cancelled), "size query").await;
        let total = match stage {
            Ok(size) => size.unwrap_or(None).unwrap_or(0) as u64,
            Err(error) => {
                self.poison();
                return Err(error);
            }
        };
        if cancelled.load(Ordering::Relaxed) {
            return Err(TransferCancelled.into());
        }
        let temp_local = format!("{}.navop-tmp-{}", local_path, unique_temp_suffix());
        let result = self
            .download_to_file(remote_path, &temp_local, total, cancelled, &progress)
            .await;
        if let Err(error) = result {
            // 置毒决策由 download_to_file 按"传输是否已开始"处理；
            // 这里只负责清理本地临时文件。
            let _ = tokio::fs::remove_file(&temp_local).await;
            return Err(error);
        }
        let file = tokio::fs::File::open(&temp_local).await?;
        file.sync_all().await?;
        drop(file);
        // 不先删除旧目标：rename 在 Unix 上原子替换已有文件，
        // Windows 上经 MoveFileEx REPLACE_EXISTING 也能替换；
        // 先删后改名会让 rename 失败时丢失原文件。
        match tokio::fs::rename(&temp_local, local_path).await {
            Ok(()) => Ok(()),
            Err(error) => {
                let _ = tokio::fs::remove_file(&temp_local).await;
                Err(anyhow!(
                    "Failed to move downloaded file into place: {error}"
                ))
            }
        }
    }

    async fn download_to_file(
        &mut self,
        remote_path: &str,
        temp_local: &str,
        total: u64,
        cancelled: &AtomicBool,
        progress: &(impl Fn(TransferProgress) + Send + Sync + ?Sized),
    ) -> Result<()> {
        // 先创建本地临时文件，再发起 RETR：本地 I/O 失败时传输尚未开始，
        // 命令交换完整、连接仍可用，直接报错即可（消除"传输已开始
        // 但本地文件创建失败"留下的失步窗口）。
        let mut file = tokio::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(temp_local)
            .await?;
        let stage = await_stage(
            self.stream.get(remote_path),
            Some(cancelled),
            "download start",
        )
        .await;
        let mut reader = match stage {
            Ok(Ok(reader)) => reader,
            // 服务端明确拒绝 RETR：响应已被完整消费，连接仍可用。
            Ok(Err(error)) => return Err(error.into()),
            // 取消或超时：RETR 响应可能未被消费，连接状态未知。
            Err(error) => {
                self.poison();
                return Err(error);
            }
        };
        // 传输已开始：数据读取失败（取消/停滞/断线）或本地写入失败，
        // 数据流都没有正常收尾，协议状态未知 → 连接置毒，拒绝复用。
        let pumped: Result<()> = async {
            let mut transferred: u64 = 0;
            let mut chunk = vec![0u8; CHUNK_SIZE];
            let mut last_data = tokio::time::Instant::now();
            let start = std::time::Instant::now();
            loop {
                ensure_not_cancelled(cancelled)?;
                let read =
                    read_chunk(&mut reader, &mut chunk, Some(cancelled), &mut last_data).await?;
                if read == 0 {
                    break;
                }
                file.write_all(&chunk[..read]).await?;
                transferred += read as u64;
                progress(TransferProgress {
                    transferred,
                    total,
                    speed: transferred as f64 / start.elapsed().as_secs_f64().max(0.001),
                    current_file: Some(remote_path.to_string()),
                    current_file_transferred: transferred,
                    current_file_total: total,
                });
            }
            file.flush().await?;
            Ok(())
        }
        .await;
        if let Err(error) = pumped {
            self.poison();
            return Err(error);
        }
        drop(reader);
        // 数据流结束后必须消费 226 完成响应，否则后续命令会读到残留应答。
        // 等待响应加超时上限并响应取消，避免服务端停滞时挂死或取消失效。
        // 保守策略：传输开始后，终态响应只要不是"成功且完整消费"——
        // 服务端报错（可能是 426 后还跟着 226 的残留序列）、超时、取消——
        // 都无法证明控制通道已恢复同步，一律置毒禁止复用。
        let outcome = {
            let mut response = std::pin::pin!(tokio::time::timeout(
                COMMAND_TIMEOUT,
                self.stream
                    .read_response(async_ftp::status::CLOSING_DATA_CONNECTION),
            ));
            loop {
                tokio::select! {
                    result = response.as_mut() => break Some(result),
                    _ = tokio::time::sleep(CANCEL_POLL_INTERVAL) => {
                        if cancelled.load(Ordering::Relaxed) {
                            break None;
                        }
                    }
                }
            }
        };
        match outcome {
            None => {
                self.poison();
                return Err(TransferCancelled.into());
            }
            Some(Ok(Ok(_))) => {}
            Some(Ok(Err(error))) => {
                self.poison();
                return Err(error.into());
            }
            Some(Err(_)) => {
                self.poison();
                return Err(anyhow!(
                    "Timed out waiting for FTP transfer completion response"
                ));
            }
        }
        Ok(())
    }

    /// 安全上传：上传到远端临时路径 → 远端 rename；失败清理临时文件。
    pub(super) async fn upload_file(
        &mut self,
        local_path: &str,
        remote_path: &str,
        cancelled: &AtomicBool,
        progress: impl Fn(TransferProgress) + Send + Sync,
    ) -> Result<()> {
        self.ensure_usable()?;
        let total = tokio::fs::metadata(local_path).await?.len();
        let temp_remote = temp_path(remote_path);
        let mut file = tokio::fs::File::open(local_path).await?;
        let mut reader = ProgressReader {
            inner: &mut file,
            transferred: 0,
            total,
            cancelled,
            progress: &progress,
            current_file: remote_path.to_string(),
        };
        // put 过程用 select! 轮询取消标志：停滞的上传也能被取消唤醒。
        // 取消时丢弃 put future（数据连接中途断开，协议状态未知，连接置毒）。
        // pin! 的 future 借用持续到作用域结束，因此整个 select 循环
        // 放在独立的块作用域中，块结束后才能再次使用 self.stream。
        let outcome = {
            let mut put = std::pin::pin!(self.stream.put(&temp_remote, &mut reader));
            loop {
                tokio::select! {
                    result = put.as_mut() => break PutOutcome::Done(result),
                    _ = tokio::time::sleep(CANCEL_POLL_INTERVAL) => {
                        if cancelled.load(Ordering::Relaxed) {
                            break PutOutcome::Cancelled;
                        }
                    }
                }
            }
        };
        drop(reader);
        match outcome {
            PutOutcome::Cancelled => {
                // 连接已置毒：不要在状态不明的连接上同步等待清理命令
                // （STOR 之后的响应可能仍未消费，再发 DELE 只会读到失步应答）。
                // 远端临时文件留待重连后的清理或服务端垃圾回收。
                self.poison();
                return Err(TransferCancelled.into());
            }
            PutOutcome::Done(Err(error)) => {
                // 保守策略：无法区分错误发生在 STOR 被拒（响应已消费、
                // 连接同步）还是数据传输已开始后的中途失败（可能残留
                // 226/426 序列）——一律置毒，且不在该连接上继续清理。
                self.poison();
                return Err(error.into());
            }
            PutOutcome::Done(Ok(())) => {}
        }
        match tokio::time::timeout(
            COMMAND_TIMEOUT,
            self.stream.rename(&temp_remote, remote_path),
        )
        .await
        {
            Ok(Ok(())) => {}
            Ok(Err(error)) => {
                // 服务端明确拒绝 rename（如权限不足）：命令已完整收发，协议状态正常。
                let _ = tokio::time::timeout(COMMAND_TIMEOUT, self.stream.rm(&temp_remote)).await;
                return Err(anyhow!("Failed to move uploaded file into place: {error}"));
            }
            Err(_) => {
                // RNFR 可能已发出，协议状态未知：连接置毒。
                self.poison();
                return Err(anyhow!("Timed out renaming uploaded file into place"));
            }
        }
        progress(TransferProgress {
            transferred: total,
            total,
            speed: 0.0,
            current_file: Some(remote_path.to_string()),
            current_file_transferred: total,
            current_file_total: total,
        });
        Ok(())
    }

    /// 编辑器保存等小内容写入：与上传相同的"远端临时文件 → rename"契约，
    /// 保存失败时不破坏远端原文件。
    pub(super) async fn write_file_safe(&mut self, path: &str, content: &[u8]) -> Result<()> {
        self.ensure_usable()?;
        let temp_remote = temp_path(path);
        let mut cursor = std::io::Cursor::new(content);
        let result =
            tokio::time::timeout(COMMAND_TIMEOUT, self.stream.put(&temp_remote, &mut cursor)).await;
        match result {
            Ok(Ok(())) => {}
            Ok(Err(error)) => {
                // 与上传相同的保守策略：无法区分"STOR 被拒"与"传输中途
                // 失败"，一律置毒并跳过该连接上的清理命令。
                self.poison();
                return Err(error.into());
            }
            Err(_) => {
                self.poison();
                return Err(anyhow!("Timed out uploading editor save"));
            }
        }
        match tokio::time::timeout(COMMAND_TIMEOUT, self.stream.rename(&temp_remote, path)).await {
            Ok(Ok(())) => {}
            Ok(Err(error)) => {
                let _ = tokio::time::timeout(COMMAND_TIMEOUT, self.stream.rm(&temp_remote)).await;
                return Err(anyhow!("Failed to move saved file into place: {error}"));
            }
            Err(_) => {
                self.poison();
                return Err(anyhow!("Timed out renaming saved file into place"));
            }
        }
        Ok(())
    }

    /// 递归删除：文件先行，目录按路径深度降序（深者先删），最后删除根目录。
    pub(super) async fn delete_tree(
        &mut self,
        path: &str,
        cancelled: &AtomicBool,
        progress: impl Fn(TransferProgress) + Send + Sync,
    ) -> Result<()> {
        self.ensure_usable()?;
        let entries = self.collect_recursive_entries(path, cancelled).await?;
        let (files, dirs) = deletion_order(&entries);
        let total = (files.len() + dirs.len()) as u64;
        let mut done: u64 = 0;
        for file in &files {
            ensure_not_cancelled(cancelled)?;
            self.stream.rm(file).await?;
            done += 1;
            progress(TransferProgress {
                transferred: done,
                total,
                speed: 0.0,
                current_file: Some(file.clone()),
                current_file_transferred: done,
                current_file_total: total,
            });
        }
        for dir in &dirs {
            ensure_not_cancelled(cancelled)?;
            self.stream.rmdir(dir).await?;
            done += 1;
            progress(TransferProgress {
                transferred: done,
                total,
                speed: 0.0,
                current_file: Some(dir.clone()),
                current_file_transferred: done,
                current_file_total: total,
            });
        }
        self.stream.rmdir(path).await?;
        Ok(())
    }

    /// 递归上传目录（带冲突策略）。
    pub(super) async fn upload_dir(
        &mut self,
        local_path: &str,
        remote_path: &str,
        conflict_policy: DirectoryConflictPolicy,
        cancelled: &AtomicBool,
        progress: impl Fn(TransferProgress) + Send + Sync,
    ) -> Result<()> {
        self.ensure_usable()?;
        let local_meta = tokio::fs::metadata(local_path).await?;
        if !local_meta.is_dir() {
            return Err(anyhow!("Local path is not a directory: {local_path}"));
        }

        // 冲突策略：Replace 时先删除远端已有目录。
        if conflict_policy == DirectoryConflictPolicy::Replace
            && self.path_exists(remote_path).await?
        {
            self.delete_tree(remote_path, cancelled, |_| {}).await?;
        }

        ensure_dir(&mut self.stream, remote_path).await?;
        let mut pending = vec![(local_path.to_string(), remote_path.to_string())];
        while let Some((current_local, current_remote)) = pending.pop() {
            ensure_not_cancelled(cancelled)?;
            let mut entries = tokio::fs::read_dir(&current_local).await?;
            while let Some(entry) = entries.next_entry().await? {
                ensure_not_cancelled(cancelled)?;
                let child_local = entry.path();
                let child_name = entry.file_name().to_string_lossy().to_string();
                let child_remote = child_path(&current_remote, &child_name);
                if entry.file_type().await?.is_dir() {
                    ensure_dir(&mut self.stream, &child_remote).await?;
                    pending.push((child_local.to_string_lossy().to_string(), child_remote));
                } else {
                    self.upload_file(
                        child_local.to_string_lossy().as_ref(),
                        &child_remote,
                        cancelled,
                        |file_progress| {
                            progress(TransferProgress {
                                transferred: file_progress.current_file_transferred,
                                total: file_progress.current_file_total,
                                speed: file_progress.speed,
                                current_file: Some(child_name.clone()),
                                current_file_transferred: file_progress.current_file_transferred,
                                current_file_total: file_progress.current_file_total,
                            })
                        },
                    )
                    .await?;
                }
            }
        }
        Ok(())
    }

    /// 递归下载目录（带冲突策略）。
    pub(super) async fn download_dir(
        &mut self,
        remote_path: &str,
        local_path: &str,
        conflict_policy: DirectoryConflictPolicy,
        cancelled: &AtomicBool,
        progress: impl Fn(TransferProgress) + Send + Sync,
    ) -> Result<()> {
        self.ensure_usable()?;
        // 用 LIST 探测目录可访问性：SIZE 对目录普遍返回 550，
        // 用 SIZE 判定会把可列举的目录误报为不可访问。
        self.list_entries(remote_path, Some(cancelled))
            .await
            .map_err(|error| {
                anyhow!("Remote directory is not accessible: {remote_path}: {error}")
            })?;

        if conflict_policy == DirectoryConflictPolicy::Replace {
            let _ = tokio::fs::remove_dir_all(local_path).await;
        }
        tokio::fs::create_dir_all(local_path).await?;

        let entries = self
            .collect_recursive_entries(remote_path, cancelled)
            .await?;
        let total: u64 = entries.iter().map(|entry| entry.size).sum();
        let mut transferred: u64 = 0;
        for entry in &entries {
            ensure_not_cancelled(cancelled)?;
            let relative = entry
                .path
                .strip_prefix(remote_path.trim_end_matches('/'))
                .unwrap_or(&entry.path)
                .trim_start_matches('/')
                .to_string();
            let components = validate_relative_path(&relative)?;
            let mut target = PathBuf::from(local_path);
            let last = components.len() - 1;
            for (index, component) in components.iter().enumerate() {
                target.push(component);
                // 逐级创建并做符号链接检查；目录条目的最后一级也建目录。
                if index < last || entry.is_dir {
                    ensure_dir_checked(&target).await?;
                }
            }
            if entry.is_dir {
                continue;
            }
            // 目标文件若是预置符号链接，拒绝覆盖（防止写出目录之外）。
            if let Ok(metadata) = tokio::fs::symlink_metadata(&target).await {
                if metadata.file_type().is_symlink() {
                    return Err(anyhow!(
                        "Refusing to overwrite local symlink: {}",
                        target.display()
                    ));
                }
            }
            self.download_file(
                &entry.path,
                target.to_string_lossy().as_ref(),
                cancelled,
                |file_progress| {
                    progress(TransferProgress {
                        transferred: transferred + file_progress.current_file_transferred,
                        total,
                        speed: file_progress.speed,
                        current_file: Some(entry.path.clone()),
                        current_file_transferred: file_progress.current_file_transferred,
                        current_file_total: file_progress.current_file_total,
                    })
                },
            )
            .await?;
            transferred += entry.size;
        }
        Ok(())
    }

    /// 判断远程路径是否存在：SIZE 对目录通常失败，因此对目录用 CWD 探测。
    pub(super) async fn path_exists(&mut self, path: &str) -> Result<bool> {
        if self.stream.size(path).await.unwrap_or(None).is_some() {
            return Ok(true);
        }
        self.path_is_dir(path).await
    }

    /// CWD 探测目录并恢复原工作目录。
    pub(super) async fn path_is_dir(&mut self, path: &str) -> Result<bool> {
        let previous = match self.stream.pwd().await {
            Ok(previous) => Some(previous),
            Err(_) => None,
        };
        if self.stream.cwd(path).await.is_ok() {
            if let Some(previous) = previous {
                let _ = self.stream.cwd(&previous).await;
            } else {
                let _ = self.stream.cdup().await;
            }
            Ok(true)
        } else {
            Ok(false)
        }
    }

    /// stat 探测：CWD 判目录 → SIZE 判文件 → 父目录 LIST 兜底。
    ///
    /// 语义：路径不存在返回 `Ok(None)`；连接中断或协议异常返回 `Err`；
    /// 命令不被支持时走父目录 LIST 的兼容路径。
    pub(super) async fn stat_path(&mut self, path: &str) -> Result<Option<sftp::PathMetadata>> {
        // 1) CWD 探测目录；服务端明确拒绝（550 等）才继续文件探测。
        let previous = self.stream.pwd().await.ok();
        match self.stream.cwd(path).await {
            Ok(()) => {
                if let Some(previous) = previous {
                    let _ = self.stream.cwd(&previous).await;
                } else {
                    let _ = self.stream.cdup().await;
                }
                return Ok(Some(sftp::PathMetadata {
                    size: 0,
                    modified: self.mdtm_or_epoch(path).await,
                    is_dir: true,
                    // FTP 不提供可靠的数值权限；填 0，UI 不展示伪造权限。
                    permissions: 0,
                }));
            }
            Err(error) => {
                if ftp_status_code(&error).is_none() {
                    // 连接中断或协议异常：必须与"路径不存在"区分。
                    return Err(error.into());
                }
            }
        }
        // 2) SIZE 探测文件。
        match self.stream.size(path).await {
            Ok(Some(size)) => {
                return Ok(Some(sftp::PathMetadata {
                    size: size as u64,
                    modified: self.mdtm_or_epoch(path).await,
                    is_dir: false,
                    permissions: 0,
                }));
            }
            Ok(None) => {}
            Err(error) => {
                if ftp_status_code(&error).is_none() {
                    return Err(error.into());
                }
            }
        }
        // 3) 兼容路径：SIZE/CWD 都无法确认时，用父目录 LIST 判定存在性。
        self.stat_via_parent_listing(path).await
    }

    /// 兼容路径：列出父目录查找条目；列不出父目录返回 Err（无法区分时保守报错）。
    async fn stat_via_parent_listing(&mut self, path: &str) -> Result<Option<sftp::PathMetadata>> {
        let (parent, name) = split_remote_parent(path);
        if name.is_empty() {
            return Ok(None);
        }
        let entries = self.list_entries(&parent, None).await.map_err(|error| {
            anyhow!("Failed to stat remote path {path}: cannot list parent directory: {error}")
        })?;
        Ok(entries
            .into_iter()
            .find(|entry| entry.name == name)
            .map(|entry| sftp::PathMetadata {
                size: if entry.is_dir { 0 } else { entry.size },
                modified: entry.modified,
                is_dir: entry.is_dir,
                permissions: 0,
            }))
    }

    /// MDTM 最佳努力获取修改时间；不支持或失败时回退 Unix 纪元。
    async fn mdtm_or_epoch(&mut self, path: &str) -> SystemTime {
        self.stream
            .mdtm(path)
            .await
            .unwrap_or(None)
            .map(|value| {
                SystemTime::UNIX_EPOCH + Duration::from_secs(value.timestamp().max(0) as u64)
            })
            .unwrap_or(SystemTime::UNIX_EPOCH)
    }
}

/// 拆分远程路径为（父目录, 文件名），区分三种父目录形态：
/// - `file.txt`      → ("", "file.txt")   父目录是当前工作目录（`LIST` 不带参数）
/// - `/root.txt`     → ("/", "root.txt")  父目录是根目录，必须显式 `LIST /`
/// - `/dir/file.txt` → ("/dir", "file.txt")
///
/// "相对文件名的父目录为空"与"绝对路径的父目录是根目录"不是同一种情况：
/// 对后者发不带参数的 `LIST` 会列出当前工作目录而非根目录。
pub(super) fn split_remote_parent(path: &str) -> (String, String) {
    let trimmed = path.trim_end_matches('/');
    match trimmed.rsplit_once('/') {
        // 绝对路径直接位于根目录下：父目录必须显式为 "/"。
        Some(("", name)) => ("/".to_string(), name.to_string()),
        Some((parent, name)) => (parent.to_string(), name.to_string()),
        // 相对文件名：父目录是当前工作目录（空字符串 → LIST 不带参数）。
        None => (String::new(), trimmed.to_string()),
    }
}

/// 递归删除执行顺序（纯函数，便于回归测试）：
/// 文件先行；目录按路径深度降序（最深目录先删），与遍历收集顺序解耦。
pub(super) fn deletion_order(entries: &[FileEntry]) -> (Vec<String>, Vec<String>) {
    let mut files = Vec::new();
    let mut dirs: Vec<&FileEntry> = entries.iter().filter(|entry| entry.is_dir).collect();
    dirs.sort_by(|a, b| remote_path_depth(&b.path).cmp(&remote_path_depth(&a.path)));
    for entry in entries {
        if !entry.is_dir {
            files.push(entry.path.clone());
        }
    }
    (
        files,
        dirs.into_iter().map(|entry| entry.path.clone()).collect(),
    )
}

fn remote_path_depth(path: &str) -> usize {
    path.matches('/').count()
}

/// 确保远端目录存在（mkdir 失败但目录已存在时视为成功）。
async fn ensure_dir(client: &mut FtpStream, path: &str) -> Result<()> {
    if client.mkdir(path).await.is_err() {
        // 目录可能已存在：用 CWD 验证。
        let previous = client.pwd().await.ok();
        if client.cwd(path).await.is_ok() {
            if let Some(previous) = previous {
                let _ = client.cwd(&previous).await;
            }
            return Ok(());
        }
        return Err(anyhow!("Failed to create remote directory: {path}"));
    }
    Ok(())
}

/// 包装本地文件读取器，边读边上报进度并响应取消。
struct ProgressReader<'a, P> {
    inner: &'a mut tokio::fs::File,
    transferred: u64,
    total: u64,
    cancelled: &'a AtomicBool,
    progress: &'a P,
    current_file: String,
}

impl<P> tokio::io::AsyncRead for ProgressReader<'_, P>
where
    P: Fn(TransferProgress) + Send + Sync,
{
    fn poll_read(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &mut tokio::io::ReadBuf<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        if self.cancelled.load(Ordering::Relaxed) {
            return std::task::Poll::Ready(Err(std::io::Error::other(TransferCancelled)));
        }
        let before = buf.filled().len();
        let pinned = std::pin::Pin::new(&mut *self.inner);
        match pinned.poll_read(cx, buf) {
            std::task::Poll::Ready(Ok(())) => {
                let read = (buf.filled().len() - before) as u64;
                self.transferred += read;
                (self.progress)(TransferProgress {
                    transferred: self.transferred,
                    total: self.total,
                    speed: 0.0,
                    current_file: Some(self.current_file.clone()),
                    current_file_transferred: self.transferred,
                    current_file_total: self.total,
                });
                std::task::Poll::Ready(Ok(()))
            }
            other => other,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{deletion_order, ftp_status_code, split_remote_parent, validate_relative_path};
    use crate::FtpClient;
    use sftp::{FileEntry, TransferCancelled};
    use std::sync::atomic::Ordering;
    use std::sync::{Arc, Mutex};
    use std::time::SystemTime;

    fn entry(path: &str, is_dir: bool) -> FileEntry {
        FileEntry {
            name: path.rsplit('/').next().unwrap_or(path).to_string(),
            path: path.to_string(),
            size: 0,
            modified: SystemTime::UNIX_EPOCH,
            is_dir,
            permissions: 0,
            uid: None,
            gid: None,
            user: None,
            group: None,
        }
    }

    #[test]
    fn deletion_order_deletes_files_first_and_deepest_dirs_first() {
        // 树：/tree/sub/{file.txt,note.md}、/tree/empty、/tree/top.txt
        // 收集顺序（先子后父）曾配合 .rev() 导致先删非空子目录；
        // deletion_order 必须与收集顺序无关。
        let entries = vec![
            entry("/tree/sub/file.txt", false),
            entry("/tree/sub/note.md", false),
            entry("/tree/sub", true),
            entry("/tree/empty", true),
            entry("/tree/top.txt", false),
        ];
        let (files, dirs) = deletion_order(&entries);
        // 所有文件先于任何目录删除。
        assert_eq!(files.len(), 3);
        // 目录按深度降序：深目录（sub）在浅目录（empty）之前。
        assert_eq!(
            dirs,
            vec!["/tree/sub".to_string(), "/tree/empty".to_string()]
        );
    }

    #[test]
    fn deletion_order_handles_two_level_nesting() {
        let entries = vec![entry("/tree/a/b", true), entry("/tree/a", true)];
        let (_, dirs) = deletion_order(&entries);
        assert_eq!(dirs, vec!["/tree/a/b".to_string(), "/tree/a".to_string()]);
    }

    #[test]
    fn validate_relative_path_rejects_escape_attempts() {
        assert!(validate_relative_path("file.txt").is_ok());
        assert!(validate_relative_path("sub/file.txt").is_ok());
        assert!(validate_relative_path("../escaped.txt").is_err());
        assert!(validate_relative_path("sub/../../escaped.txt").is_err());
        assert!(validate_relative_path("a\\b.txt").is_err());
        assert!(validate_relative_path("").is_err());
        assert!(validate_relative_path("sub//file").is_err());
        assert!(validate_relative_path(".").is_err());
    }

    #[test]
    fn ftp_status_code_parses_server_replies_only() {
        let reply = async_ftp::types::FtpError::InvalidResponse(
            "Expected code [213], got response: 550 Could not get file size.".to_string(),
        );
        assert_eq!(ftp_status_code(&reply), Some(550));
        let connection =
            async_ftp::types::FtpError::ConnectionError(std::io::Error::other("connection reset"));
        assert_eq!(ftp_status_code(&connection), None);
    }

    #[test]
    fn split_remote_parent_distinguishes_relative_and_root_paths() {
        // 相对文件名：父目录是当前工作目录（空 → LIST 不带参数）。
        assert_eq!(
            split_remote_parent("file.txt"),
            (String::new(), "file.txt".to_string())
        );
        // 绝对路径直接位于根目录：父目录必须显式为 "/"。
        assert_eq!(
            split_remote_parent("/root.txt"),
            ("/".to_string(), "root.txt".to_string())
        );
        // 普通绝对路径。
        assert_eq!(
            split_remote_parent("/dir/file.txt"),
            ("/dir".to_string(), "file.txt".to_string())
        );
        // 带尾斜杠的目录路径。
        assert_eq!(
            split_remote_parent("/dir/"),
            ("/".to_string(), "dir".to_string())
        );
        // 根目录本身没有名称成分。
        assert_eq!(split_remote_parent("/").1, "");
    }

    /// 极简被动模式假 FTP 服务端：应答登录/TYPE/PWD/CWD/SIZE/PASV/LIST/RETR，
    /// 记录收到的命令供断言使用。LIST/RETR 时通过预绑定的数据连接发送
    /// `listing` 内容；RETR 后按 `retr_tail` 依次发送收尾应答
    /// （正常为 `&["226 done\r\n"]`，异常场景可发送 426→226 或留空模拟停滞）。
    async fn spawn_fake_ftp_server(
        recorded: Arc<Mutex<Vec<String>>>,
        listing: &'static str,
        retr_tail: &'static [&'static str],
    ) -> std::net::SocketAddr {
        use tokio::io::AsyncWriteExt;
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            let data_listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let data_port = data_listener.local_addr().unwrap().port();
            let (stream, _) = listener.accept().await.unwrap();
            let (reader, mut writer) = tokio::io::split(stream);
            let mut lines = tokio::io::BufReader::new(reader);
            writer.write_all(b"220 fake ftp ready\r\n").await.unwrap();
            let mut line = String::new();
            loop {
                line.clear();
                if tokio::io::AsyncBufReadExt::read_line(&mut lines, &mut line)
                    .await
                    .unwrap_or(0)
                    == 0
                {
                    break;
                }
                let command = line.trim_end().to_string();
                recorded.lock().unwrap().push(command.clone());
                let reply: &[u8] = if command.starts_with("USER") {
                    b"331 need password\r\n"
                } else if command.starts_with("PASS") {
                    b"230 logged in\r\n"
                } else if command.starts_with("TYPE") {
                    b"200 type set\r\n"
                } else if command.starts_with("PWD") {
                    b"257 \"/home\" is current directory\r\n"
                } else if command.starts_with("CWD") {
                    b"550 no such directory\r\n"
                } else if command.starts_with("SIZE") {
                    b"550 SIZE not allowed\r\n"
                } else if command.starts_with("PASV") {
                    let (p1, p2) = (data_port / 256, data_port % 256);
                    let reply = format!("227 Entering Passive Mode (127,0,0,1,{p1},{p2})\r\n");
                    writer.write_all(reply.as_bytes()).await.unwrap();
                    continue;
                } else if command.starts_with("LIST") {
                    writer
                        .write_all(b"150 opening data connection\r\n")
                        .await
                        .unwrap();
                    let (mut data, _) = data_listener.accept().await.unwrap();
                    data.write_all(listing.as_bytes()).await.unwrap();
                    drop(data);
                    writer.write_all(b"226 done\r\n").await.unwrap();
                    continue;
                } else if command.starts_with("RETR") {
                    writer
                        .write_all(b"150 opening data connection\r\n")
                        .await
                        .unwrap();
                    let (mut data, _) = data_listener.accept().await.unwrap();
                    data.write_all(listing.as_bytes()).await.unwrap();
                    drop(data);
                    for reply in retr_tail {
                        writer.write_all(reply.as_bytes()).await.unwrap();
                    }
                    continue;
                } else if command.starts_with("QUIT") {
                    writer.write_all(b"221 bye\r\n").await.unwrap();
                    break;
                } else {
                    b"502 not implemented\r\n"
                };
                writer.write_all(reply).await.unwrap();
            }
        });
        addr
    }

    async fn connect_to_fake_server(port: u16) -> FtpClient {
        FtpClient::connect(crate::FtpConnectConfig {
            host: "127.0.0.1".to_string(),
            port,
            username: "user".to_string(),
            password: "pass".to_string(),
            passive_mode: true,
            use_tls: false,
            connect_timeout: Some(5),
        })
        .await
        .expect("connect to fake server")
    }

    fn recorded_commands(recorded: &Mutex<Vec<String>>) -> Vec<String> {
        recorded.lock().unwrap().clone()
    }

    /// 回归：根目录绝对路径文件的 stat 兼容回退必须显式 `LIST /`，
    /// 而不是发不带参数的 LIST（那会列举登录后的当前工作目录）。
    #[tokio::test]
    async fn stat_root_level_file_lists_root_directory_explicitly() {
        let recorded = Arc::new(Mutex::new(Vec::new()));
        let addr = spawn_fake_ftp_server(
            recorded.clone(),
            "-rw-r--r-- 1 u g 4 Jan 01 2025 root.txt\r\n",
            &[],
        )
        .await;
        let mut client = connect_to_fake_server(addr.port()).await;

        // CWD 与 SIZE 都被假服务端拒绝（550），必须走父目录 LIST 兼容路径。
        let stat = client
            .stat_path("/root.txt")
            .await
            .expect("stat should not error")
            .expect("file exists");
        assert!(!stat.is_dir);
        assert_eq!(stat.size, 4);

        let commands = recorded_commands(&recorded);
        assert!(
            commands.iter().any(|command| command == "LIST /"),
            "expected explicit 'LIST /', got: {commands:?}"
        );
        assert!(
            !commands.iter().any(|command| command == "LIST"),
            "bare 'LIST' would list the wrong directory, got: {commands:?}"
        );
    }

    /// 回归：相对文件名的 stat 回退应使用不带参数的 LIST（当前工作目录）。
    #[tokio::test]
    async fn stat_relative_file_lists_current_directory() {
        let recorded = Arc::new(Mutex::new(Vec::new()));
        let addr = spawn_fake_ftp_server(
            recorded.clone(),
            "-rw-r--r-- 1 u g 9 Jan 01 2025 local.txt\r\n",
            &[],
        )
        .await;
        let mut client = connect_to_fake_server(addr.port()).await;

        let stat = client
            .stat_path("local.txt")
            .await
            .expect("stat should not error")
            .expect("file exists");
        assert!(!stat.is_dir);

        let commands = recorded_commands(&recorded);
        assert!(
            commands.iter().any(|command| command == "LIST"),
            "relative name should use bare 'LIST', got: {commands:?}"
        );
    }

    /// 回归：只有 `total 0`（或 `.`/`..`）的合法空目录不得被误判为解析失败。
    #[tokio::test]
    async fn legal_empty_directory_listing_returns_no_entries() {
        for listing in [
            "total 0\r\n",
            "drwxr-xr-x 1 u g 0 Jan 01 2025 .\r\ndrwxr-xr-x 1 u g 0 Jan 01 2025 ..\r\n",
        ] {
            let recorded = Arc::new(Mutex::new(Vec::new()));
            let addr = spawn_fake_ftp_server(recorded.clone(), listing, &[]).await;
            let mut client = connect_to_fake_server(addr.port()).await;
            let entries = client
                .list_entries("/empty", None)
                .await
                .expect("empty directory must not error");
            assert!(
                entries.is_empty(),
                "listing {listing:?} should yield no entries"
            );
        }
    }

    /// 回归：数据流正常 EOF 后服务端发 426 → 226 残留序列时，
    /// 连接必须被置毒——后续操作不得在失步连接上继续（读到残留 226）。
    #[tokio::test]
    async fn download_with_426_then_226_poisons_connection() {
        use sftp::RemoteFileClient;
        use std::sync::atomic::AtomicBool;

        let recorded = Arc::new(Mutex::new(Vec::new()));
        let addr = spawn_fake_ftp_server(
            recorded.clone(),
            "partial data\r\n",
            &["426 connection closed prematurely\r\n", "226 done\r\n"],
        )
        .await;
        let mut client = connect_to_fake_server(addr.port()).await;

        let cancelled = Arc::new(AtomicBool::new(false));
        let dir = std::env::temp_dir().join(format!("navop-ftp-426-test-{}", std::process::id()));
        tokio::fs::create_dir_all(&dir).await.unwrap();
        let target = dir.join("out.bin");
        let result = client
            .download_file("/root.txt", target.to_str().unwrap(), &cancelled, |_| {})
            .await;
        assert!(result.is_err(), "426 closing must fail the download");
        let _ = tokio::fs::remove_dir_all(&dir).await;

        // 连接已被置毒：realpath 的 ensure_usable 直接拒绝，
        // 而不是在失步连接上发 PWD 读到残留的 226。
        let error = client
            .realpath(".")
            .await
            .expect_err("poisoned connection must be rejected");
        assert!(
            error.to_string().contains("unusable"),
            "expected unusable-connection error, got: {error}"
        );
        assert!(!client.is_reusable());
    }

    /// 回归：服务端对 LIST 不应答时，取消标志必须在轮询间隔内唤醒等待。
    #[tokio::test]
    async fn list_entries_cancel_wakes_stalled_listing() {
        use sftp::RemoteFileClient;
        use std::sync::atomic::AtomicBool;

        // 精简假服务端：完成登录后对 PASV/LIST 一概不应答。
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            use tokio::io::AsyncWriteExt;
            let (stream, _) = listener.accept().await.unwrap();
            let (reader, mut writer) = tokio::io::split(stream);
            let mut lines = tokio::io::BufReader::new(reader);
            writer.write_all(b"220 stalled\r\n").await.unwrap();
            let mut line = String::new();
            loop {
                line.clear();
                if tokio::io::AsyncBufReadExt::read_line(&mut lines, &mut line)
                    .await
                    .unwrap_or(0)
                    == 0
                {
                    break;
                }
                if line.starts_with("QUIT") {
                    break;
                }
                if line.starts_with("USER") {
                    writer.write_all(b"331 ok\r\n").await.unwrap();
                } else if line.starts_with("PASS") {
                    writer.write_all(b"230 ok\r\n").await.unwrap();
                } else if line.starts_with("TYPE") {
                    writer.write_all(b"200 ok\r\n").await.unwrap();
                }
                // PASV/LIST：故意不应答。
            }
        });
        let mut client = connect_to_fake_server(addr.port()).await;

        let cancelled = Arc::new(AtomicBool::new(false));
        let waker = {
            let cancelled = cancelled.clone();
            tokio::spawn(async move {
                tokio::time::sleep(std::time::Duration::from_millis(150)).await;
                cancelled.store(true, Ordering::Relaxed);
            })
        };
        let start = std::time::Instant::now();
        let result = client.list_entries("/tree", Some(&cancelled)).await;
        waker.await.unwrap();
        let elapsed = start.elapsed();
        let error = result.expect_err("stalled LIST must fail after cancel");
        assert!(
            error.is::<TransferCancelled>(),
            "expected TransferCancelled, got: {error}"
        );
        assert!(
            elapsed < std::time::Duration::from_secs(2),
            "cancel must wake the stalled LIST promptly, took {elapsed:?}"
        );
        assert!(
            !client.is_reusable(),
            "cancelled LIST must poison the connection"
        );
    }

    /// 回归：数据接收完毕、服务端迟迟不发 226 时，等待完成响应阶段必须响应取消。
    #[tokio::test]
    async fn download_completion_response_respects_cancellation() {
        use std::sync::atomic::AtomicBool;

        // retr_tail 为空：数据发送完即停滞，不发 226。
        let recorded = Arc::new(Mutex::new(Vec::new()));
        let addr = spawn_fake_ftp_server(recorded.clone(), "file body\r\n", &[]).await;
        let mut client = connect_to_fake_server(addr.port()).await;

        let cancelled = Arc::new(AtomicBool::new(false));
        let waker = {
            let cancelled = cancelled.clone();
            tokio::spawn(async move {
                tokio::time::sleep(std::time::Duration::from_millis(150)).await;
                cancelled.store(true, Ordering::Relaxed);
            })
        };
        let dir = std::env::temp_dir().join(format!("navop-ftp-final-test-{}", std::process::id()));
        tokio::fs::create_dir_all(&dir).await.unwrap();
        let target = dir.join("out.bin");
        let start = std::time::Instant::now();
        let result = client
            .download_file("/root.txt", target.to_str().unwrap(), &cancelled, |_| {})
            .await;
        waker.await.unwrap();
        let elapsed = start.elapsed();
        let _ = tokio::fs::remove_dir_all(&dir).await;
        let error = result.expect_err("stalled completion response must fail after cancel");
        assert!(
            error.is::<TransferCancelled>(),
            "expected TransferCancelled, got: {error}"
        );
        assert!(
            elapsed < std::time::Duration::from_secs(2),
            "cancel must wake the completion wait promptly, took {elapsed:?}"
        );
    }
}
