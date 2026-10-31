//! 主实例侧的命名管道服务循环与单连接处理。

use std::io;

use super::super::{MAX_PAYLOAD_BYTES, StartupRequest, codec};
use super::forward::{ACKNOWLEDGEMENT_ACCEPTED, ACKNOWLEDGEMENT_REJECTED, REQUEST_IO_TIMEOUT};

pub(super) fn create_pipe_instance(
    pipe_name: &str,
    first: bool,
) -> io::Result<tokio::net::windows::named_pipe::NamedPipeServer> {
    use tokio::net::windows::named_pipe::ServerOptions;

    let mut options = ServerOptions::new();
    // `first_pipe_instance` 只允许出现在首个实例上：它就是"同一配置目录只有一个
    // 主实例"这条互斥语义的来源。服务循环补建的实例必须不带这个标志，否则会把
    // 自己也挡在门外。
    options.first_pipe_instance(first);
    options.create(pipe_name)
}

pub(super) async fn serve_forever(
    pipe_name: String,
    first_instance: tokio::net::windows::named_pipe::NamedPipeServer,
    on_request: std::sync::Arc<dyn Fn(StartupRequest) -> bool + Send + Sync>,
) {
    let mut server = Some(first_instance);

    while let Some(instance) = server.take() {
        if let Err(error) = instance.connect().await {
            tracing::warn!(%error, "failed to accept a Windows single-instance connection");
            // 把实例放回去：宁可不再服务新连接，也不能让名称变成无主。
            server = Some(instance);
            break;
        }

        // 先补建下一个实例、再移交当前连接：任何时刻都至少有一个实例持有这个
        // 名称，别的进程不会在这条缝隙里抢到主实例资格。
        let next_instance = match create_pipe_instance(&pipe_name, false) {
            Ok(instance) => instance,
            Err(error) => {
                tracing::warn!(
                    %error,
                    "failed to create the next Windows single-instance pipe instance"
                );
                server = Some(instance);
                break;
            }
        };

        // 每条连接独立处理：一个半开连接不能拖住后续启动请求。
        tokio::spawn(handle_connection(
            instance,
            std::sync::Arc::clone(&on_request),
        ));
        server = Some(next_instance);
    }

    // 服务循环停在这里，说明管道已经无法继续接受连接。此时必须**保持名称被
    // 占用**（fail-closed）：一旦放手，另一个进程就会把自己当主实例并开出第二个
    // GUI。宁可让后续启动请求转发失败（会明确报错退出），也不要破坏"同一配置
    // 目录只有一个主实例"这条语义。
    if server.is_some() {
        tracing::error!("Windows single-instance listener stopped; keeping the pipe name claimed");
        std::future::pending::<()>().await;
    }
}

async fn handle_connection(
    mut connection: tokio::net::windows::named_pipe::NamedPipeServer,
    on_request: std::sync::Arc<dyn Fn(StartupRequest) -> bool + Send + Sync>,
) {
    use tokio::io::AsyncWriteExt as _;
    use tokio::time::timeout;

    let request = match timeout(REQUEST_IO_TIMEOUT, read_request(&mut connection)).await {
        Ok(Ok(request)) => request,
        Ok(Err(error)) => {
            tracing::warn!(%error, "failed to read a Windows single-instance request");
            return;
        }
        Err(_) => {
            tracing::warn!(
                timeout_ms = REQUEST_IO_TIMEOUT.as_millis(),
                "timed out reading a Windows single-instance request"
            );
            return;
        }
    };

    // ACK 只表示"请求已经进入主实例的启动队列"，不代表文件已经打开；入队失败
    // 必须回拒绝，否则第二个进程会误判请求已被接管而静默退出。
    let acknowledgement = [if on_request(request) {
        ACKNOWLEDGEMENT_ACCEPTED
    } else {
        ACKNOWLEDGEMENT_REJECTED
    }];
    match timeout(REQUEST_IO_TIMEOUT, connection.write_all(&acknowledgement)).await {
        Ok(Ok(())) => {}
        Ok(Err(error)) => {
            tracing::warn!(%error, "failed to acknowledge a Windows single-instance request");
        }
        Err(_) => {
            tracing::warn!("timed out acknowledging a Windows single-instance request");
        }
    }
    if let Err(error) = connection.flush().await {
        tracing::warn!(%error, "failed to flush a Windows single-instance acknowledgement");
    }
}

async fn read_request(
    connection: &mut tokio::net::windows::named_pipe::NamedPipeServer,
) -> io::Result<StartupRequest> {
    use tokio::io::AsyncReadExt as _;

    let mut payload_length = [0; size_of::<u32>()];
    connection.read_exact(&mut payload_length).await?;
    let payload_length = u32::from_le_bytes(payload_length) as usize;
    if payload_length > MAX_PAYLOAD_BYTES {
        return Err(codec::invalid_data("startup request is too large"));
    }

    let mut payload = vec![0; payload_length];
    connection.read_exact(&mut payload).await?;
    codec::decode_request(&payload)
}
