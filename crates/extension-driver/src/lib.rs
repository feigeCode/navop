//! 驱动子进程共享运行时。
//!
//! 解决「reader 与同步执行耦合」这个根因:把每个数据库驱动重构成
//!
//! - **reader 任务**:只读帧、永不阻塞在 DB 上;按 `conn_id` 把请求派发给 worker,
//!   即时处理 `$/cancelRequest` / `shutdown` / `$/ping`。
//! - **per-conn worker 线程**:独占一个 [`DriverConnection`](数据库连接),FIFO 串行
//!   执行阻塞调用(DuckDB 等同步 API 在专属线程上跑,不毒化 async runtime)。
//! - **pump 任务**:把 worker 产出的响应回收、从 in-flight 表摘除、写回 writer。
//!
//! 这样跨连接天然并发、连接内天然串行;长查询执行期间 reader 依然空闲,
//! 因此取消 / ping / shutdown 能被即时处理。
//!
//! 驱动作者只需实现 [`Driver`](控制面/工厂)+ [`DriverConnection`](每连接),
//! 把现有的同步 handler 包进来,并发 / 取消 / writer 串行化全部由 [`serve`] 负责。

// ProtocolError 作为 Err 类型较大(~248 bytes),协议层固定如此,统一 allow。
#![allow(clippy::result_large_err)]

mod async_runtime;
mod runtime;

pub use async_runtime::{
    AsyncDriverConnection, AsyncNativeDriver, AsyncOpenedConnection, serve_async,
};
pub use runtime::serve;

use std::sync::Arc;

use extension_protocol::ProtocolError;
use extension_protocol::conn::ConnId;
use serde_json::Value;

/// Connect a sidecar to the host-created local socket named by `env_var`, then
/// run the Tokio-first driver runtime until shutdown or EOF.
pub async fn serve_async_from_env<D>(driver: D, env_var: &str) -> anyhow::Result<()>
where
    D: AsyncNativeDriver,
{
    use interprocess::local_socket::{
        GenericNamespaced, ToNsName,
        tokio::{Stream, prelude::*},
    };

    let socket_name = std::env::var(env_var)
        .map_err(|_| anyhow::anyhow!("required local socket env `{env_var}` is not set"))?;
    let name = socket_name
        .clone()
        .to_ns_name::<GenericNamespaced>()
        .map_err(|error| anyhow::anyhow!("invalid local socket name `{socket_name}`: {error}"))?;
    let stream = Stream::connect(name).await?;
    let (reader, writer) = tokio::io::split(stream);
    serve_async(driver, reader, writer).await
}

/// 硬取消钩子:包裹驱动私有的中断机制(例如 DuckDB 的 `InterruptHandle::interrupt`)。
///
/// 由 reader 在 worker 线程**正阻塞于查询**时从另一线程调用,要求实现 `Send + Sync`。
/// 返回 `None` 表示该驱动暂不支持硬中断(取消只能等当前调用自然结束后归一为
/// `REQUEST_CANCELLED`)。
pub type InterruptHook = Arc<dyn Fn() + Send + Sync>;

/// 一个已打开的数据库连接。跑在专属 worker 线程上,因此只要求 `Send`,无需 `Sync`。
pub trait DriverConnection: Send {
    /// 在 worker 线程上同步执行一个连接内方法(`query/start` / `cursor/*` /
    /// `exec/run` / `schema/*` / `conn/ping` / `conn/use` 等)。
    fn call(&mut self, method: &str, params: &Value) -> Result<Value, ProtocolError>;

    /// 返回硬取消钩子(若支持)。在连接打开后、移入 worker 线程前由运行时取一次。
    fn interrupt_hook(&self) -> Option<InterruptHook> {
        None
    }

    /// `conn/close` 或进程 shutdown 时调用,做连接级清理。
    fn close(&mut self) {}
}

/// `open_connection` 的产物:连接 id、回给宿主的 `conn/open` result、连接本体。
pub struct OpenedConnection {
    pub conn_id: ConnId,
    /// 作为 `conn/open` 响应 result 原样回给宿主(通常含 `server_info`)。
    pub open_result: Value,
    pub connection: Box<dyn DriverConnection>,
}

/// 驱动控制面 / 工厂,每个驱动进程一个,被运行时共享(`Send + Sync`)。
pub trait Driver: Send + Sync + 'static {
    /// `init` 握手。返回 result 原样回给宿主。
    fn init(&self, params: &Value) -> Result<Value, ProtocolError>;

    /// `conn/open`:建立一个数据库连接。可能做网络 I/O,运行时会放到
    /// `spawn_blocking` 里执行,因此**允许阻塞**。
    fn open_connection(&self, params: &Value) -> Result<OpenedConnection, ProtocolError>;

    /// 不依赖具体连接的方法(`ddl/build_*` 等纯方法)。运行时只在请求 `params`
    /// 不含 `conn_id` 时走这里。
    fn call_connless(&self, method: &str, params: &Value) -> Result<Value, ProtocolError>;

    /// `shutdown` 时调用,做进程级清理(可选)。
    fn shutdown(&self) {}
}
