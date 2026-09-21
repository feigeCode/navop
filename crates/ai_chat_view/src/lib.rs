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
mod agent_tool_config;
mod agent_tool_input;
mod agent_transcript;
mod agent_view;
mod ask_ai;
mod bridge;
mod card;
mod cards;
mod chart_json;
mod chat_state;
mod chat_view;
mod code_block;
mod code_block_parse;
mod connection_selector;
mod default_panel;
#[cfg(test)]
mod default_panel_tests;
mod expansion_state;
pub mod find_shortcut;
mod html_code_block;
mod input;
mod message;
mod message_code_actions;
mod message_tool_group;
mod message_turn_view;
mod message_view;
mod model_settings;
mod pending_decision;
mod pending_submission;
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
mod transcript_scroll;
mod transcript_search;
mod turn;
mod workbench;

pub use acp::{
    AcpAgentConfig, AcpAgentEntry, AcpAuthConfig, AcpAuthMethodConfig, AcpConfigDiagnostic,
    AcpConnectOutcome, AcpConnection, AcpConnectionPhase, AcpError, AcpErrorKind,
    AcpPendingConnection, AcpPermissionFuture, AcpPermissionGrant, AcpPermissionOption,
    AcpPermissionOutcome, AcpPermissionProvider, AcpPermissionRequest, AcpPromptStartError,
    AcpPublicMcpApprovalFuture, AcpPublicMcpApprovalOutcome, AcpPublicMcpApprovalProvider,
    AcpPublicMcpApprovalRequest, AcpRecoveryAction, AcpTimeoutConfig, AcpTransport,
    build_acp_agent_configs, build_acp_agent_entries, current_acp_tool_mode,
    set_acp_agent_config_provider, set_acp_permission_grant_provider, set_acp_tool_mode_provider,
    set_current_acp_tool_mode,
};
pub use agent_cards::{PlanCardData, PlanStepData, SubAgentCardData, ToolCardData};
pub use agent_tool_config::emit_agent_tool_config_changed;
pub use agent_transcript::AgentTranscript;
pub use agent_view::{AgentChatView, AgentChatViewConfig, AgentChatViewEvent, AgentRuntimeFactory};
// 工作台外壳也要画同一份 ACP 会话行，所以把行构造器和模型提到 crate 可见。
pub(crate) use agent_view::acp_sessions::{
    AcpSessionListModel, acp_session_placeholder, acp_session_row, acp_session_section_header,
};
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
pub use chat_view::{ChatView, chat_task_sidebar_title};
pub use code_block::{
    CodeBlockAction, CodeBlockActionBuilder, CodeBlockActionCallback, CodeBlockActionPreview,
    CodeBlockActionRegistry, FencedCodeBlock, LanguageMatcher, extract_fenced_code_blocks,
};
pub use connection_selector::{ConnectionSelector, ConnectionSelectorEvent};
pub use default_panel::{DefaultAgentChatPanel, DefaultAgentChatPanelEvent};
pub use input::{
    AgentComposerContext, AgentInput, AgentInputEvent, ComposerAgentOption, ComposerMenuOption,
    ComposerModel, ComposerModelOption, ComposerPlanItem, ComposerScope, ComposerTarget,
    ImageAttachment, MentionCompletionProvider, MentionItem, SlashCommandItem,
};
pub use message::{
    ChatMessageUI, ChatMessageUIGeneric, ChatRole, MESSAGE_RENDER_LIMIT, MESSAGE_RENDER_STEP,
    MessageExtension, MessageVariant, NoExtension,
};
pub use message_turn_view::{
    MessageListAction, MessageListActionHandler, MessageListContext, render_message_list,
};
pub use message_view::{
    MessageListLayout, render_assistant_text, render_messages, render_messages_with_code_actions,
    render_messages_with_code_actions_and_activity, render_running_activity,
    render_sidebar_messages_with_code_actions,
    render_sidebar_messages_with_code_actions_and_activity, render_status_message,
    render_system_message, render_thinking, render_user_message,
};
pub use model_settings::{
    ModelSettings, ModelSettingsEvent, ModelSettingsLabels, ModelSettingsPanel,
};
pub use pending_decision::{
    DecisionAuthority, DecisionCardSource, DecisionOption, DecisionOptionKind, PendingDecision,
    pending_decisions,
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
pub use expansion_state::ExpansionState;
pub use find_shortcut::{
    AI_CHAT_COMPOSER_CONTEXT, AI_CHAT_FINDBAR_CONTEXT, AI_CHAT_SEARCH_CONTEXT, CloseTranscriptFind,
    FIND_MACOS, FIND_NEXT_MACOS, FIND_NEXT_OTHER, FIND_OTHER, FIND_PREVIOUS_MACOS,
    FIND_PREVIOUS_OTHER, FindNextInTranscript, FindPreviousInTranscript, ToggleTranscriptFind,
    find_defaults_for_platform,
};
pub use theme::AgentChatTheme;
pub use transcript_scroll::{FollowState, TranscriptScrollState};
pub use transcript_search::{
    MAX_SEARCH_HITS, SearchHit, TranscriptSearch, count_matches, message_search_text, turn_texts,
};
pub use turn::{
    TurnProjection, TurnTiming, TurnTimings, breakdown_text, is_pending_decision, is_risk_message,
    project_turns,
};
pub use workbench::{
    WorkbenchDockLayout, WorkbenchPanelEntry, WorkbenchPanelKind, WorkbenchShell,
    WorkbenchShellConfig, WorkbenchState, dock_region_width,
};

/// 初始化 `ai_chat_view`:确保全局卡片注册表存在。
///
/// 各业务模块可在自身 `init` 之后,通过 [`CardRegistry::register_global`] 把
/// 自己的卡片注册进来。
pub fn init(cx: &mut App) {
    agent_tool_config::init(cx);
    CardRegistry::init_global(cx);
    cards::register_builtin_cards(cx);
    agent_cards::register_agent_cards(cx);
    // 本 crate 自己的快捷键随 `init` 一起注册，而不是留给宿主单独调用：
    // 漏调不会报错、只会「快捷键静默不生效」，这种失效模式不值得赌。
    // 设置变更后的重绑定仍由宿主显式调 `find_shortcut::refresh_keybindings`。
    find_shortcut::init(cx);
}
