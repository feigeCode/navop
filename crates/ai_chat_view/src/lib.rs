//! `ai_chat_view` —— 通用 AI 聊天 UI 渲染层 + 可扩展卡片渲染机制。
//!
//! 本 crate 刻意**不依赖任何具体业务**(数据库 / SSH 等),只提供通用、可扩展
//! 的聊天界面机制,供各业务模块在其上注册自己的卡片渲染器。
//!
//! 现阶段提供:
//! - 卡片注册机制:[`CardRegistry`] / [`ChatCard`] / [`CardMessage`]
//!
//! 规划中(逐步补齐):通用消息列表、可折叠会话侧边栏、通用聊天视图 `ChatView`、
//! 内置示例卡片。
//!
//! 数据模型(泛型消息、`MessageVariant::Card { kind }`)由本 crate 提供。

rust_i18n::i18n!("locales", fallback = "en");

use gpui::App;

pub use agent_runtime::{
    AgentResourceScope, DefaultTargetReason, ResourceCatalog, ResourceContext, ResourceId,
    ResourceScope,
};

mod acp;
mod agent_cards;
mod agent_skills;
mod agent_tab;
mod agent_tool_input;
mod agent_transcript;
mod agent_view;
mod ask_ai;
mod bridge;
mod card;
mod cards;
mod chart_json;
mod chat_state;
mod chat_tab;
mod chat_view;
mod code_block;
mod code_block_parse;
mod connection_selector;
mod default_panel;
#[cfg(test)]
mod default_panel_tests;
mod html_code_block;
mod input;
mod message;
mod message_code_actions;
mod message_tool_group;
mod message_view;
mod model_settings;
mod persistence;
mod plan_tools;
mod provider;
mod reasoning;
mod resource_builder;
#[cfg(test)]
mod resource_builder_tests;
mod resource_display;
mod send_button;
mod session_service;
mod session_sidebar;
mod theme;
#[cfg(test)]
mod workbench_tab_tests;

pub use acp::{
    AcpAgentConfig, AcpAgentEntry, AcpAuthConfig, AcpAuthMethodConfig, AcpConfigDiagnostic,
    AcpConnectOutcome, AcpConnection, AcpConnectionPhase, AcpError, AcpErrorKind,
    AcpPendingConnection, AcpPermissionFuture, AcpPermissionGrant, AcpPermissionOption,
    AcpPermissionOutcome, AcpPermissionProvider, AcpPermissionRequest, AcpRecoveryAction,
    AcpTimeoutConfig, AcpTransport, build_acp_agent_configs, build_acp_agent_entries,
    current_acp_tool_mode, set_acp_agent_config_provider, set_acp_permission_grant_provider,
    set_acp_tool_mode_provider, set_current_acp_tool_mode,
};
pub use agent_cards::{PlanCardData, PlanStepData, SubAgentCardData, ToolCardData};
pub use agent_tab::{AGENT_TAB_CONTENT_KEY, AgentTabContent};
pub use agent_transcript::AgentTranscript;
pub use agent_view::{AgentChatView, AgentChatViewConfig, AgentChatViewEvent, AgentRuntimeFactory};
pub use ask_ai::{
    AskAiButton, AskAiEvent, AskAiNotifier, emit_ask_ai_event, emit_ask_ai_event_app,
    format_ask_ai_message, get_ask_ai_notifier, init_ask_ai_notifier,
};
pub use bridge::{
    LlmModelClient, build_runtime, build_runtime_from_llm_provider,
    build_runtime_from_provider_config, build_runtime_from_provider_state,
};
pub use card::{CardMessage, CardRegistry, ChatCard};
pub use cards::JsonCard;
pub use chart_json::{
    ChartJsonBlock, ChartPiePoint, ChartType, ChartXYPoint, parse_chart_json_block,
};
pub use chat_state::ChatViewState;
pub use chat_tab::{CHAT_TAB_CONTENT_KEY, ChatTabContent};
pub use chat_view::{CHAT_TASK_SIDEBAR_TITLE, ChatView};
pub use code_block::{
    CodeBlockAction, CodeBlockActionBuilder, CodeBlockActionCallback, CodeBlockActionPreview,
    CodeBlockActionRegistry, FencedCodeBlock, LanguageMatcher, extract_fenced_code_blocks,
};
pub use connection_selector::{ConnectionSelector, ConnectionSelectorEvent};
pub use default_panel::{DefaultAgentChatPanel, DefaultAgentChatPanelEvent};
pub use input::{
    AgentComposerContext, AgentInput, AgentInputEvent, ComposerAgentOption, ComposerMenuOption,
    ComposerModel, ComposerModelOption, ComposerPlanItem, ComposerScope, ComposerTarget,
    ImageAttachment, MentionCompletionProvider, MentionItem,
};
pub use message::{
    ChatMessageUI, ChatMessageUIGeneric, ChatRole, MESSAGE_RENDER_LIMIT, MESSAGE_RENDER_STEP,
    MessageExtension, MessageVariant, NoExtension,
};
pub use message_view::{
    render_assistant_text, render_messages, render_messages_with_code_actions,
    render_status_message, render_system_message, render_thinking, render_user_message,
};
pub use model_settings::{
    ModelSettings, ModelSettingsEvent, ModelSettingsLabels, ModelSettingsPanel,
};
pub use plan_tools::{
    PlanToolRegistryProvider, build_plan_tool_registry, set_plan_tool_registry_provider,
};
pub use provider::ProviderItem;
pub use reasoning::render_reasoning_block;
pub use resource_builder::{
    build_agent_context_all, build_agent_context_single, build_agent_context_single_with_catalog,
    build_mentions_from_connections, build_mentions_single, build_resource_catalog,
    build_resource_context_all, build_resource_context_single, build_sidebar_resource_state,
    build_workbench_agent_context, build_workbench_resource_state,
};
pub use send_button::{SendButton, SendButtonEvent, SendButtonState};
pub use session_service::{SessionError, SessionService, extract_session_name};
pub use session_sidebar::{SessionSummary, format_timestamp, session_row};
pub use theme::AgentChatTheme;

/// 初始化 `ai_chat_view`:确保全局卡片注册表存在。
///
/// 各业务模块可在自身 `init` 之后,通过 [`CardRegistry::register_global`] 把
/// 自己的卡片注册进来。
pub fn init(cx: &mut App) {
    CardRegistry::init_global(cx);
    cards::register_builtin_cards(cx);
    agent_cards::register_agent_cards(cx);
}
