//! ACP(Agent Client Protocol)接入:把外部 agent 作为 stdio 子进程驱动一轮对话。
//!
//! - [`AcpAgentConfig`]:通用「自定义命令」配置(path + args + env)。
//! - [`AcpConnection`]:已建立的交互式会话连接,事件以 `RuntimeEvent` 形式经
//!   `broadcast` 推出,与自研 `Runtime` 同型,从而复用 view 的事件泵与转录。
//!
//! 翻译层 [`translate`] 把 ACP `SessionUpdate` 映射为 `agent_runtime::RuntimeEvent`。

mod auth;
#[cfg(test)]
mod auth_tests;
mod client;
mod config;
mod connection;
mod error;
#[cfg(test)]
mod error_tests;
mod permission;
mod provider;
mod public_mcp_approval;
mod state;
mod translate;
mod turn;
#[cfg(test)]
mod turn_tests;

pub use config::{
    AcpAgentConfig, AcpAgentEntry, AcpAuthConfig, AcpAuthMethodConfig, AcpConfigDiagnostic,
    AcpTimeoutConfig, AcpTransport,
};
pub use connection::{AcpConnectOutcome, AcpConnection, AcpPendingConnection, AcpPromptStartError};
pub use error::{AcpError, AcpErrorKind, AcpRecoveryAction};
pub(crate) use permission::{AcpPermissionEnvelope, AcpPermissionMessage, acp_permission_channel};
pub use permission::{
    AcpPermissionFuture, AcpPermissionOption, AcpPermissionOutcome, AcpPermissionProvider,
    AcpPermissionRequest,
};
pub(crate) use provider::acquire_acp_permission_grant;
pub use provider::{
    AcpPermissionGrant, build_acp_agent_configs, build_acp_agent_entries, current_acp_tool_mode,
    set_acp_agent_config_provider, set_acp_permission_grant_provider, set_acp_tool_mode_provider,
    set_current_acp_tool_mode,
};
pub(crate) use public_mcp_approval::{
    AcpPublicMcpApprovalEnvelope, AcpPublicMcpApprovalMessage, acp_public_mcp_approval_channel,
};
pub use public_mcp_approval::{
    AcpPublicMcpApprovalFuture, AcpPublicMcpApprovalOutcome, AcpPublicMcpApprovalProvider,
    AcpPublicMcpApprovalRequest,
};
pub use state::AcpConnectionPhase;
pub(crate) use state::AcpSessionState;
