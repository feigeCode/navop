//! onetcli 扩展协议 crate。
//!
//! 本 crate 定义宿主与扩展子进程 / WASM 实例之间的 wire protocol,
//! 包含 JSON-RPC envelope、业务消息类型和最小 framed JSON 编解码。
//! 具体的进程管理 / socket 握手 / WASM runtime 集成在另外的 crate 中
//! (`extension-host` / `db/ipc/client.rs`)。
//!
//! 协议层概览(详见 `docs/design/extensions/api-database.md`):
//!
//! - 物理层: framed (4-byte LE length prefix) + MessagePack(或 JSON,
//!   开发模式调试用)
//! - 消息层: JSON-RPC 2.0 兼容结构(`method` / `params` / `id` /
//!   `result` / `error`)
//! - 业务层: 按命名空间分组的方法(`conn/*` / `schema/*` / `query/*` ...)
//!
//! ## 模块布局
//!
//! - [`envelope`]: JSON-RPC 2.0 Request / Response / Notification
//! - [`error`]: 错误码常量 + ProtocolError + ErrorData
//! - [`framing`]: 4-byte LE length-prefix JSON 帧收发
//! - [`row`]: 统一行/值表示(Row IR)
//! - [`method`][]: 所有方法名常量,避免拼写错误
//! - [`lifecycle`]: init / shutdown / Capability
//! - [`conn`][]: 连接管理
//! - [`schema`]: schema 内省
//! - [`query`][]: 查询、游标、执行、事务
//! - [`sql`]: SQL 工具(parse / format / explain / completion / lint)
//! - [`ddl`]: DDL 构造
//! - [`data`][]: 数据导入导出
//! - [`host`]: 反向 Host API(扩展 → 宿主)
//! - [`event`][]: 单向事件通知

pub mod conn;
pub mod data;
pub mod ddl;
pub mod envelope;
pub mod error;
pub mod event;
pub mod framing;
pub mod host;
pub mod lifecycle;
pub mod method;
pub mod query;
pub mod row;
pub mod schema;
pub mod sql;

pub use envelope::{Notification, Request, RequestId, Response, ResponseBody, RpcMessage};
pub use error::{ErrorCode, ErrorData, ProtocolError, error_codes};
pub use row::{CellValue, ColumnSpec, ColumnTypeKind, Row};

/// 当前 crate 提供的 wire protocol 主版本。
///
/// 与 `extension.json` 的 `api.extension` 字段无关——那是 *扩展协议本身*
/// 的契约版本(详见 [`docs/design/extensions/versioning.md`])。
pub const WIRE_PROTOCOL_VERSION: &str = "1.0";

/// JSON-RPC 协议固定标识。
pub const JSONRPC_VERSION: &str = "2.0";
