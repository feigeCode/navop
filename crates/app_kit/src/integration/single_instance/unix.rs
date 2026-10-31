//! macOS/Linux 单实例后端：`UnixListener` + `SHOW` 握手。
//!
//! 首次启动绑定 `<runtime>/navop-<app_id>.sock` 并起一个常驻监听线程；二次启动先
//! `connect` 把请求写进去（成功即 `Forwarded`），连不上说明 socket 是上次崩溃留下的
//! 陈旧文件，删掉重新绑定即可。

use std::io::{Read as _, Write as _};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::time::Duration;

use super::codec;
use super::{MAX_PAYLOAD_BYTES, SingleInstanceError, SingleInstanceOutcome, StartupRequest};

/// 握手魔数：读到它才说明对端是本产品的单实例客户端，而不是恰好连上来的什么东西。
const MAGIC: &[u8; 4] = b"SHOW";
const IO_TIMEOUT: Duration = Duration::from_secs(2);
const ACKNOWLEDGEMENT_ACCEPTED: u8 = 1;
const ACKNOWLEDGEMENT_REJECTED: u8 = 0;

/// 产品专属 socket 路径：文件名必须带 `app_id`，三个产品因此互不干扰。
///
/// 优先 `XDG_RUNTIME_DIR`（Linux 登录会话私有、退出即清理），否则退回系统临时目录
/// （macOS 上就是 `$TMPDIR` 的每用户私有目录）。
pub(super) fn socket_path(app_id: &str) -> PathBuf {
    runtime_dir().join(format!("navop-{}.sock", sanitize(app_id)))
}

/// 纯函数便于单测：把 `app_id` 收敛成安全的文件名片段。
pub(super) fn sanitize(app_id: &str) -> String {
    app_id
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '-'
            }
        })
        .collect()
}

fn runtime_dir() -> PathBuf {
    if let Some(dir) = std::env::var_os("XDG_RUNTIME_DIR").map(PathBuf::from) {
        if dir.is_dir() {
            return dir;
        }
    }
    std::env::temp_dir()
}

pub(super) fn claim_or_forward(
    app_id: &str,
    request: &StartupRequest,
    on_request: impl Fn(StartupRequest) -> bool + Send + Sync + 'static,
) -> Result<SingleInstanceOutcome, SingleInstanceError> {
    let path = socket_path(app_id);
    let payload = codec::encode_request(request).map_err(SingleInstanceError::Forward)?;

    match forward(&path, &payload) {
        Ok(true) => return Ok(SingleInstanceOutcome::Forwarded),
        Ok(false) => {
            return Err(SingleInstanceError::Forward(std::io::Error::other(
                "正在运行的 Navop 实例拒绝了启动请求",
            )));
        }
        Err(error) => {
            // NotFound / ConnectionRefused：没有活着的实例在监听，socket 是陈旧文件。
            let stale = matches!(
                error.kind(),
                std::io::ErrorKind::NotFound | std::io::ErrorKind::ConnectionRefused
            );
            if !stale {
                return Err(SingleInstanceError::Forward(error));
            }
            let _ = std::fs::remove_file(&path);
        }
    }

    let listener = bind(&path).map_err(SingleInstanceError::Listen)?;
    spawn_listener(listener, on_request).map_err(SingleInstanceError::Listen)?;
    Ok(SingleInstanceOutcome::Primary)
}

/// 绑定 socket。万一和另一个正在退出的实例抢同一个路径（`AddrInUse`），清掉陈旧文件
/// 再试一次；第二次仍失败就如实报错，不制造第二个实例。
fn bind(path: &Path) -> std::io::Result<UnixListener> {
    match UnixListener::bind(path) {
        Ok(listener) => {
            restrict_permissions(path);
            Ok(listener)
        }
        Err(error) if path.exists() => {
            let _ = std::fs::remove_file(path);
            let listener = UnixListener::bind(path)?;
            restrict_permissions(path);
            tracing::warn!(%error, "清理了陈旧的单实例 socket 并重新绑定");
            Ok(listener)
        }
        Err(error) => Err(error),
    }
}

fn restrict_permissions(path: &Path) {
    use std::os::unix::fs::PermissionsExt as _;

    if let Err(error) = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600)) {
        tracing::warn!(%error, "收紧单实例 socket 权限失败");
    }
}

fn spawn_listener(
    listener: UnixListener,
    on_request: impl Fn(StartupRequest) -> bool + Send + Sync + 'static,
) -> std::io::Result<()> {
    std::thread::Builder::new()
        .name("navop-single-instance".to_owned())
        .spawn(move || {
            for stream in listener.incoming() {
                let stream = match stream {
                    Ok(stream) => stream,
                    Err(error) => {
                        // 单个连接失败不能弄死监听线程：下一次启动还要靠它。
                        tracing::warn!(%error, "接受单实例连接失败");
                        continue;
                    }
                };
                let mut reader = match stream.try_clone() {
                    Ok(reader) => reader,
                    Err(error) => {
                        tracing::warn!(%error, "克隆单实例连接失败");
                        continue;
                    }
                };
                let Some(request) = read_request(&mut reader) else {
                    continue;
                };
                let accepted = on_request(request);
                acknowledge(stream, accepted);
            }
            tracing::debug!("单实例监听线程退出");
        })
        .map(|_| ())
}

/// 读取 `SHOW` + 请求负载；任何畸形输入都丢弃整条连接（绝不 panic）。
fn read_request(stream: &mut UnixStream) -> Option<StartupRequest> {
    let _ = stream.set_read_timeout(Some(IO_TIMEOUT));

    let mut magic = [0u8; 4];
    stream.read_exact(&mut magic).ok()?;
    if &magic != MAGIC {
        tracing::warn!("单实例连接使用了未知协议，已忽略");
        return None;
    }

    let mut length = [0u8; 4];
    stream.read_exact(&mut length).ok()?;
    let length = u32::from_le_bytes(length) as usize;
    if length > MAX_PAYLOAD_BYTES {
        tracing::warn!(length, "单实例请求过大，已忽略");
        return None;
    }

    let mut payload = vec![0u8; length];
    stream.read_exact(&mut payload).ok()?;
    match codec::decode_request(&payload) {
        Ok(request) => Some(request),
        Err(error) => {
            tracing::warn!(%error, "单实例请求解码失败，已忽略");
            None
        }
    }
}

fn acknowledge(mut stream: UnixStream, accepted: bool) {
    let ack = if accepted {
        ACKNOWLEDGEMENT_ACCEPTED
    } else {
        ACKNOWLEDGEMENT_REJECTED
    };
    if let Err(error) = stream.write_all(&[ack]).and_then(|()| stream.flush()) {
        tracing::warn!(%error, "回写单实例确认失败");
    }
}

/// 把请求交给已有实例；`Ok(true)` 表示主实例已收下，`Ok(false)` 表示明确拒绝。
fn forward(path: &Path, payload: &[u8]) -> std::io::Result<bool> {
    let mut stream = UnixStream::connect(path)?;
    stream.set_write_timeout(Some(IO_TIMEOUT))?;
    stream.set_read_timeout(Some(IO_TIMEOUT))?;

    stream.write_all(MAGIC)?;
    let length = u32::try_from(payload.len())
        .map_err(|_| std::io::Error::other("startup request is too large"))?;
    stream.write_all(&length.to_le_bytes())?;
    stream.write_all(payload)?;
    stream.flush()?;

    let mut ack = [0u8; 1];
    match stream.read_exact(&mut ack) {
        Ok(()) => Ok(ack[0] == ACKNOWLEDGEMENT_ACCEPTED),
        // 请求字节已经写出去了，主实例很可能已经收下，只是回执慢。此时按「已交付」
        // 处理比让两个实例同时跑起来安全。
        Err(error) => {
            tracing::warn!(%error, "主实例未回确认，按已交付处理");
            Ok(true)
        }
    }
}
