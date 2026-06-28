//! ACP(Agent Client Protocol)接入:把外部 agent 作为 stdio 子进程驱动一轮对话。
//!
//! - [`AcpAgentConfig`]:通用「自定义命令」配置(path + args + env)。
//! - [`AcpConnection`]:已建立的交互式会话连接,事件以 `RuntimeEvent` 形式经
//!   `broadcast` 推出,与自研 `Runtime` 同型,从而复用 view 的事件泵与转录。
//!
//! 翻译层 [`translate`] 把 ACP `SessionUpdate` 映射为 `agent_runtime::RuntimeEvent`。

mod config;
mod connection;
mod permission;
mod provider;
mod state;
mod translate;

pub use config::{AcpAgentConfig, AcpTransport};
pub use connection::AcpConnection;
pub use permission::{
    AcpPermissionFuture, AcpPermissionOption, AcpPermissionOutcome, AcpPermissionProvider,
    AcpPermissionRequest, set_acp_permission_provider,
};
pub use provider::{build_acp_agent_configs, set_acp_agent_config_provider};
pub(crate) use state::AcpSessionState;
