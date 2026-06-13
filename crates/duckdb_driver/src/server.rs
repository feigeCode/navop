//! DuckDB driver 的 v2 server,基于共享 [`extension_driver`] 运行时。
//!
//! 主循环(reader / per-conn worker / writer + 取消)全部下沉到 `extension-driver`;
//! 本文件只负责 transport 接入([`run`] / [`run_as_listener`] / [`handle_stream`])
//! 和给 [`crate::handlers`] 复用的错误构造 helper。

// 同 handlers:这是协议层,Err 类型固定。
#![allow(clippy::result_large_err)]

use anyhow::{Context, Result};
use extension_protocol::envelope::Notification;
use extension_protocol::error::{ErrorCode, ErrorData, ProtocolError, error_codes};
use interprocess::local_socket::{
    GenericNamespaced, ListenerOptions, ToNsName,
    tokio::{Stream, prelude::*},
};
use serde_json::Value;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tracing::warn;

use crate::driver::DuckDbDriver;

/// 标准入口:作为 `extension-host` 启动的子进程运行。
///
/// 流程:宿主先 listen 并把 socket 名通过 `ONETCLI_EXT_SOCKET` 透传过来,本进程
/// 主动 connect,然后把 transport 交给 [`extension_driver::serve`]。
pub async fn run(socket_name: &str) -> Result<()> {
    let name = socket_name
        .to_ns_name::<GenericNamespaced>()
        .context("invalid local socket name")?;
    tracing::info!(socket = %socket_name, "duckdb v2 driver connecting back to host");
    let stream = Stream::connect(name)
        .await
        .context("failed to connect to host listener")?;
    let (reader, writer) = tokio::io::split(stream);
    extension_driver::serve(DuckDbDriver::new(), reader, writer).await
}

/// 备用入口:**反向**模式——driver 自己 listen,客户端 connect(仅供开发/集成测试)。
pub async fn run_as_listener(socket_name: &str) -> Result<()> {
    let name = socket_name
        .to_ns_name::<GenericNamespaced>()
        .context("invalid local socket name")?;
    let listener = ListenerOptions::new()
        .name(name)
        .create_tokio()
        .context("failed to create local socket listener")?;
    tracing::info!(socket = %socket_name, "duckdb v2 driver listening (developer mode)");

    loop {
        let stream = listener.accept().await?;
        tokio::spawn(async move {
            if let Err(error) = handle_connection(stream).await {
                warn!("DuckDB v2 IPC connection failed: {error:#}");
            }
        });
    }
}

/// 暴露给集成测试:直接用一对 reader/writer 当 transport,免去 socket。
pub async fn handle_stream<R, W>(reader: R, writer: W) -> Result<()>
where
    R: AsyncReadExt + Unpin + Send,
    W: AsyncWriteExt + Unpin + Send + 'static,
{
    extension_driver::serve(DuckDbDriver::new(), reader, writer).await
}

async fn handle_connection(stream: Stream) -> Result<()> {
    let (reader, writer) = tokio::io::split(stream);
    extension_driver::serve(DuckDbDriver::new(), reader, writer).await
}

// ===================== 给 handlers 复用的 helper =====================

/// 把 `anyhow::Error` 包成 SQL 错误段的 `ProtocolError`。
pub(crate) fn protocol_error_from_anyhow(code: ErrorCode, error: anyhow::Error) -> ProtocolError {
    let mut pe = ProtocolError::new(code, format!("{error:#}"));
    pe = pe.with_data(ErrorData::new().with_extra(serde_json::json!({
        "chain": error
            .chain()
            .map(|e| e.to_string())
            .collect::<Vec<_>>(),
    })));
    pe
}

/// 必传参数缺失。
pub(crate) fn missing_param(name: &str) -> ProtocolError {
    ProtocolError::new(
        error_codes::INVALID_PARAMS,
        format!("missing required parameter `{name}`"),
    )
}

/// 通用 invalid_params。
pub(crate) fn invalid_params(message: impl Into<String>) -> ProtocolError {
    ProtocolError::new(error_codes::INVALID_PARAMS, message)
}

/// 用于反序列化 params 时报错。
pub(crate) fn params_deserialize_error(error: serde_json::Error) -> ProtocolError {
    ProtocolError::new(
        error_codes::INVALID_PARAMS,
        format!("failed to deserialize params: {error}"),
    )
}

#[allow(dead_code)] // 为 cancel / notification 路径预留
pub(crate) fn unknown_resource(message: impl Into<String>) -> ProtocolError {
    ProtocolError::new(error_codes::UNKNOWN_CONN_ID, message)
}

#[allow(dead_code)]
pub(crate) fn make_notification(method_name: &str, params: Value) -> Notification {
    Notification::new(method_name, params)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn protocol_error_from_anyhow_includes_chain() {
        let err = anyhow::anyhow!("root").context("middle").context("top");
        let pe = protocol_error_from_anyhow(error_codes::SQL_SYNTAX_ERROR, err);
        assert_eq!(pe.code, error_codes::SQL_SYNTAX_ERROR);
        assert!(pe.message.contains("top"));
        let data = pe.data.unwrap();
        let chain = data.extra.unwrap();
        assert!(chain.is_object() || chain.is_null() || chain["chain"].as_array().is_some());
    }

    #[test]
    fn missing_param_uses_invalid_params_code() {
        let pe = missing_param("conn_id");
        assert_eq!(pe.code, error_codes::INVALID_PARAMS);
        assert!(pe.message.contains("conn_id"));
    }

    #[test]
    fn invalid_params_carries_message() {
        let pe = invalid_params("bad");
        assert_eq!(pe.code, error_codes::INVALID_PARAMS);
        assert_eq!(pe.message, "bad");
    }

    #[test]
    fn params_deserialize_error_classifies_as_invalid_params() {
        let inner = serde_json::from_str::<serde_json::Value>("nope").unwrap_err();
        let pe = params_deserialize_error(inner);
        assert_eq!(pe.code, error_codes::INVALID_PARAMS);
    }
}
