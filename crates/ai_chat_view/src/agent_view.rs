//! 可运行的 Agent 聊天视图。
//!
//! 本视图把 `agent_runtime` 的事件流、`AgentInput` 和通用消息列表接起来:
//! 提交用户输入后用 `run_turn_blocking` 驱动一轮任务,事件泵持续把
//! `RuntimeEvent` 归约进 `AgentTranscript`。
//!
//! 作为输入框的"上层"集成点:把 [`ResourceContext`] 映射为输入框展示用的
//! [`AgentComposerContext`],注入模型 / 工具 / 任务模式的下拉选项,并处理输入框
//! emit 的选择事件(目标轮换、模型 / 模式切换)。

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use agent_runtime::{
    AgentResourceScope, ResourceCatalog, ResourceContext, ResourceId, ResourceKind, ResourceRef,
    Runtime, RuntimeEvent, RuntimeEventReceiver, SessionId, TaskKind, ToolCallId,
    ToolExecutionMode, ToolRegistry, UserInput,
};
use gpui::prelude::FluentBuilder;
use gpui::{
    Anchor, App, AppContext, Context, Entity, EventEmitter, FontWeight, InteractiveElement,
    IntoElement, ParentElement, Render, ScrollHandle, SharedString, StatefulInteractiveElement,
    Styled, Subscription, Task, Window, div, px,
};
use gpui_component::{
    ActiveTheme, Disableable, Icon, IconName, Selectable, Sizable, WindowExt as _,
    button::{Button, ButtonVariants},
    dialog::DialogButtonProps,
    h_flex,
    input::{Input, InputState},
    menu::{DropdownMenu, PopupMenu, PopupMenuItem},
    popover::Popover,
    v_flex,
};
#[cfg(not(test))]
use one_core::gpui_tokio::Tokio;
use one_core::llm::{GlobalProviderState, LlmConnector, LlmProvider, ProviderConfig};
use one_core::sidebar_contribution::SidebarPlacement;
use rust_i18n::t;
use tokio::sync::broadcast::error::RecvError;

use crate::acp::{
    AcpAgentEntry, AcpConnectOutcome, AcpConnection, AcpError, AcpErrorKind, AcpPendingConnection,
    AcpPermissionEnvelope, AcpPermissionMessage, AcpPermissionOutcome, AcpPermissionProvider,
    AcpRecoveryAction, AcpSessionState, acp_permission_channel, acquire_acp_permission_grant,
    build_acp_agent_entries, current_acp_tool_mode, set_current_acp_tool_mode,
};
use crate::agent_cards::{
    ApproveToolCall, PlanCardData, RejectToolCall, SelectAcpPermissionOption, SubAgentCardData,
};
use crate::agent_skills::AgentSkillState;
use crate::agent_transcript::AgentTranscript;
use crate::bridge::build_runtime_from_llm_provider;
use crate::code_block::{CodeBlockAction, CodeBlockActionRegistry};
use crate::input::{
    AgentComposerContext, AgentInput, AgentInputEvent, ComposerAgentOption, ComposerMenuOption,
    ComposerModelOption, ComposerPlanItem, ComposerResourcePoolItem, ComposerResourcePoolSummary,
    ComposerResourceSourceOption, ComposerResourceTypeFilter, ComposerScope, ComposerSkillItem,
    ComposerSkillSummary, ComposerSubAgentItem, ComposerTarget, MentionItem,
};
use crate::message_view::{
    render_messages_with_code_actions, render_sidebar_messages_with_code_actions,
};
use crate::persistence;
use crate::resource_display::first_visible_alias;
use crate::session_sidebar::{self, SessionRowStyle, SessionSummary};
use crate::theme::{AgentChatTheme, resolve_agent_chat_theme};

mod acp_options;
mod acp_ui;

use acp_options::{agent_option_disabled, composer_agent_options, current_agent_label};

/// Agent 聊天视图事件。
#[derive(Clone, Debug)]
pub enum AgentChatViewEvent {
    /// 关闭面板。
    Close,
    /// 请求宿主把面板移动到指定位置。
    MoveTo(SidebarPlacement),
}

/// 根据模型选项构建对应运行时。
pub type AgentRuntimeFactory =
    Arc<dyn Fn(&ComposerModelOption) -> Arc<Runtime> + Send + Sync + 'static>;

/// 当前驱动后端:自研内核(One_Agent)或外部 ACP agent。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Backend {
    /// 自研内核(默认)。
    Local,
    /// 外部 ACP agent。
    Acp,
}

/// 运行时与当前模型 / 会话的绑定。
struct RuntimeBinding {
    runtime: Arc<Runtime>,
    session_id: SessionId,
    selected_model: Option<ComposerModelOption>,
    runtime_factory: Option<AgentRuntimeFactory>,
}

#[cfg(test)]
fn sidebar_mode_header_action_ids(show_frame_controls: bool) -> Vec<&'static str> {
    let mut ids = vec!["new", "history"];
    if show_frame_controls {
        ids.push("frame-options");
    }
    ids.push("close");
    ids
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct SidebarFrameMoveOption {
    placement: SidebarPlacement,
    disabled: bool,
}

fn sidebar_frame_move_options(current: SidebarPlacement) -> Vec<SidebarFrameMoveOption> {
    [
        SidebarPlacement::Left,
        SidebarPlacement::Right,
        SidebarPlacement::Bottom,
    ]
    .into_iter()
    .map(|placement| SidebarFrameMoveOption {
        placement,
        disabled: placement == current,
    })
    .collect()
}

fn sidebar_placement_label(placement: SidebarPlacement) -> &'static str {
    match placement {
        SidebarPlacement::Left => "Left",
        SidebarPlacement::Right => "Right",
        SidebarPlacement::Bottom => "Bottom",
    }
}

fn sidebar_placement_icon(placement: SidebarPlacement) -> IconName {
    match placement {
        SidebarPlacement::Left => IconName::PanelLeft,
        SidebarPlacement::Right => IconName::PanelRight,
        SidebarPlacement::Bottom => IconName::PanelBottom,
    }
}

fn build_sidebar_frame_options_menu(
    menu: PopupMenu,
    view: Entity<AgentChatView>,
    placement: SidebarPlacement,
    window: &mut Window,
    cx: &mut Context<PopupMenu>,
) -> PopupMenu {
    let move_view = view.clone();
    let close_view = view.clone();
    menu.min_w(px(220.0))
        .submenu_with_icon(
            Some(IconName::PanelRight.into()),
            "Move to",
            window,
            cx,
            move |submenu, _window, _cx| {
                sidebar_frame_move_options(placement).into_iter().fold(
                    submenu,
                    |submenu, option| {
                        let view = move_view.clone();
                        submenu.item(
                            PopupMenuItem::new(sidebar_placement_label(option.placement))
                                .icon(sidebar_placement_icon(option.placement))
                                .checked(option.disabled)
                                .disabled(option.disabled)
                                .on_click(move |_, _, cx| {
                                    view.update(cx, |_this, cx| {
                                        cx.emit(AgentChatViewEvent::MoveTo(option.placement));
                                    });
                                }),
                        )
                    },
                )
            },
        )
        .separator()
        .item(
            PopupMenuItem::new("Remove from Sidebar")
                .icon(IconName::Close)
                .on_click(move |_, _, cx| {
                    close_view.update(cx, |_this, cx| {
                        cx.emit(AgentChatViewEvent::Close);
                    });
                }),
        )
}

fn agent_history_title(show_archived: bool) -> &'static str {
    if show_archived {
        "已归档任务"
    } else {
        "历史任务"
    }
}

fn current_agent_task_title() -> &'static str {
    "当前 Agent 任务"
}

fn themed_session_row_style(theme: &AgentChatTheme) -> SessionRowStyle {
    SessionRowStyle {
        foreground: theme.foreground,
        muted_foreground: theme.muted_foreground,
        selected_background: theme.selection_background(),
        selected_foreground: theme.foreground,
        hover_background: theme.hover_background(),
    }
}

impl RuntimeBinding {
    fn new(
        runtime: Arc<Runtime>,
        resources: ResourceContext,
        selected_model: Option<ComposerModelOption>,
        runtime_factory: Option<AgentRuntimeFactory>,
    ) -> Self {
        let session = runtime.create_session(resources);
        Self {
            runtime,
            session_id: session.id().clone(),
            selected_model,
            runtime_factory,
        }
    }

    fn switch_model(&mut self, option: &ComposerModelOption, resources: &ResourceContext) -> bool {
        let Some(factory) = &self.runtime_factory else {
            return false;
        };
        self.runtime = factory(option);
        let session = self.runtime.create_session(resources.clone());
        self.session_id = session.id().clone();
        self.selected_model = Some(option.clone());
        true
    }
}

/// 创建 [`AgentChatView`] 所需的配置。
pub struct AgentChatViewConfig {
    pub runtime: Arc<Runtime>,
    pub resources: ResourceContext,
    pub available_resources: Vec<ResourceRef>,
    pub mentions: Vec<MentionItem>,
    pub model_options: Vec<ComposerModelOption>,
    pub selected_model_id: Option<SharedString>,
    pub runtime_factory: Option<AgentRuntimeFactory>,
    /// 以「侧边栏视图」(窄面板)模式渲染:头部走新建对话 / 历史记录 Popover,
    /// 不常驻左侧会话列表。默认 `false`(普通 tab 全宽视图)。
    ///
    /// **重要**：侧边栏模式下 ResourceContext 固定为当前连接，不支持切换。
    pub sidebar_mode: bool,
    /// 侧边栏模式是否渲染内部头部。嵌入到已有外层面板 frame 时可关闭。
    pub show_sidebar_header: bool,
    /// 侧边栏模式是否在内部头部显示宿主 frame 控制入口。
    pub show_sidebar_frame_controls: bool,
    /// 宿主 frame 当前所在位置,用于禁用移动菜单里的当前位置。
    pub sidebar_frame_placement: SidebarPlacement,
    /// 可接入的外部 ACP agent(自定义命令)。非空时头部显示后端切换控件。
    pub acp_agents: Vec<AcpAgentEntry>,
    /// 可选的局部聊天主题。用于终端侧边栏等嵌入场景,普通 Agent tab 保持应用主题。
    pub theme: Option<AgentChatTheme>,
}

impl AgentChatViewConfig {
    pub fn new(
        runtime: Arc<Runtime>,
        resources: ResourceContext,
        mentions: Vec<MentionItem>,
    ) -> Self {
        let option = static_runtime_model_option(&runtime);
        let available_resources = resources.resources.clone();
        Self {
            runtime,
            resources,
            available_resources,
            mentions,
            model_options: vec![option.clone()],
            selected_model_id: Some(option.id),
            runtime_factory: None,
            sidebar_mode: false,
            show_sidebar_header: true,
            show_sidebar_frame_controls: false,
            sidebar_frame_placement: SidebarPlacement::Right,
            acp_agents: Vec::new(),
            theme: None,
        }
    }

    pub fn new_with_scope(
        runtime: Arc<Runtime>,
        scope: AgentResourceScope,
        catalog: ResourceCatalog,
        mentions: Vec<MentionItem>,
    ) -> Self {
        let resources = scope.to_resource_context();
        let mut config = Self::new(runtime, resources, mentions);
        config.available_resources = catalog.resources;
        config
    }

    /// 切换为「侧边栏视图」(窄面板)模式。
    pub fn sidebar_mode(mut self, enabled: bool) -> Self {
        self.sidebar_mode = enabled;
        self
    }

    pub fn show_sidebar_header(mut self, visible: bool) -> Self {
        self.show_sidebar_header = visible;
        self
    }

    pub fn show_sidebar_frame_controls(
        mut self,
        visible: bool,
        placement: SidebarPlacement,
    ) -> Self {
        self.show_sidebar_frame_controls = visible;
        self.sidebar_frame_placement = placement;
        self
    }

    /// 注入可接入的外部 ACP agent 列表。
    pub fn with_acp_agents(mut self, agents: Vec<AcpAgentEntry>) -> Self {
        self.acp_agents = agents;
        self
    }

    /// 注入局部聊天主题。
    pub fn with_theme(mut self, theme: AgentChatTheme) -> Self {
        self.theme = Some(theme);
        self
    }

    pub fn with_available_resources(mut self, resources: Vec<ResourceRef>) -> Self {
        self.available_resources = resources;
        self
    }

    pub fn with_models(
        mut self,
        model_options: Vec<ComposerModelOption>,
        selected_model_id: Option<SharedString>,
        runtime_factory: AgentRuntimeFactory,
    ) -> Self {
        self.model_options = model_options;
        self.selected_model_id = selected_model_id;
        self.runtime_factory = Some(runtime_factory);
        self
    }

    /// 用正式 provider 配置创建 Agent tab 配置。
    ///
    /// 适用于普通 provider；`OnetCli` 这类需要 `GlobalProviderState` 的 provider 请使用
    /// [`AgentChatViewConfig::from_provider_state`]。
    pub fn from_provider_configs(
        resources: ResourceContext,
        mentions: Vec<MentionItem>,
        provider_configs: Vec<ProviderConfig>,
        registry: ToolRegistry,
    ) -> anyhow::Result<Self> {
        let specs = runtime_specs_from_provider_configs(provider_configs, registry)?;
        Self::from_runtime_specs(resources, mentions, specs)
    }

    /// 用 `GlobalProviderState` 创建 Agent tab 配置,支持 OnetCli provider。
    pub async fn from_provider_state(
        resources: ResourceContext,
        mentions: Vec<MentionItem>,
        provider_configs: Vec<ProviderConfig>,
        registry: ToolRegistry,
        provider_state: GlobalProviderState,
    ) -> anyhow::Result<Self> {
        let specs =
            runtime_specs_from_provider_state(provider_configs, registry, provider_state).await?;
        Self::from_runtime_specs(resources, mentions, specs)
    }

    fn from_runtime_specs(
        resources: ResourceContext,
        mentions: Vec<MentionItem>,
        specs: Vec<RuntimeBuildSpec>,
    ) -> anyhow::Result<Self> {
        let initial = specs
            .iter()
            .find(|spec| spec.is_default)
            .or_else(|| specs.first())
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("没有可用模型配置"))?;
        let runtime = initial.build();
        let selected_model_id = selected_provider_model_id(&specs);
        let model_options = specs.iter().map(|spec| spec.option.clone()).collect();
        let spec_map: Arc<HashMap<String, RuntimeBuildSpec>> = Arc::new(
            specs
                .into_iter()
                .map(|spec| (spec.option.id.to_string(), spec))
                .collect(),
        );
        let fallback = initial;
        let runtime_factory: AgentRuntimeFactory = Arc::new(move |option| {
            spec_map
                .get(option.id.as_ref())
                .unwrap_or(&fallback)
                .build()
        });

        Ok(Self::new(runtime, resources, mentions).with_models(
            model_options,
            selected_model_id,
            runtime_factory,
        ))
    }
}

#[derive(Clone)]
struct RuntimeBuildSpec {
    option: ComposerModelOption,
    provider: Arc<dyn LlmProvider>,
    model: String,
    registry: ToolRegistry,
    temperature: Option<f32>,
    max_tokens: Option<u32>,
    is_default: bool,
}

impl RuntimeBuildSpec {
    fn build(&self) -> Arc<Runtime> {
        build_runtime_from_llm_provider(
            self.provider.clone(),
            self.model.clone(),
            self.registry.clone(),
            self.temperature,
            self.max_tokens,
        )
    }
}

#[derive(Default)]
struct AutoScrollState {
    pending_bottom_scroll_frames: usize,
}

impl AutoScrollState {
    fn request(&mut self) {
        self.request_frames(2);
    }

    fn request_settle(&mut self) {
        self.request_frames(5);
    }

    fn request_frames(&mut self, frames: usize) {
        self.pending_bottom_scroll_frames = self.pending_bottom_scroll_frames.max(frames);
    }

    fn take_pending_for_render(&mut self) -> bool {
        if self.pending_bottom_scroll_frames == 0 {
            return false;
        }
        self.pending_bottom_scroll_frames -= 1;
        true
    }
}

/// Runtime 驱动的 Agent 聊天面板。
pub struct AgentChatView {
    runtime: Arc<Runtime>,
    session_id: SessionId,
    resources: ResourceContext,
    available_resources: Vec<ResourceRef>,
    transcript: AgentTranscript,
    input: Entity<AgentInput>,
    sessions: Vec<SessionSummary>,
    current_session: String,
    sidebar_collapsed: bool,
    /// 侧边栏是否显示「已归档」会话(否则显示活跃会话)。
    show_archived: bool,
    /// 侧边栏视图(窄面板)模式:头部走新建对话 / 历史记录紧凑布局,不常驻会话列表。
    sidebar_mode: bool,
    /// 侧边栏视图是否显示内部头部。
    show_sidebar_header: bool,
    /// 侧边栏视图是否显示宿主 frame 控制入口。
    show_sidebar_frame_controls: bool,
    /// 宿主 frame 当前所在位置。
    sidebar_frame_placement: SidebarPlacement,
    /// 侧边栏视图下「历史记录」Popover 的开合状态。
    history_popover_open: bool,
    /// 当前驱动后端(默认 One_Agent)。
    backend: Backend,
    /// 可接入的外部 ACP agent 列表。
    acp_agents: Vec<AcpAgentEntry>,
    /// 本地 Codex-style Skill 管理状态。
    skills: AgentSkillState,
    /// 已建立的 ACP 连接(backend == Acp 时存在)。
    acp: Option<AcpConnection>,
    /// 等待用户选择鉴权方式的 ACP 连接。
    acp_pending: Option<AcpPendingConnection>,
    /// 当前 pending 连接公布的鉴权方式。
    acp_auth_methods: Vec<String>,
    /// 当前选中的 ACP agent id(用于头部切换控件高亮)。
    current_acp_id: Option<SharedString>,
    /// 正在连接 ACP agent(拉起子进程中)。
    acp_connecting: bool,
    /// 正在连接的 ACP agent id,用于忽略已取消连接的异步回调。
    acp_connecting_id: Option<SharedString>,
    /// 当前 ACP 连接尚未响应的权限请求。
    pending_acp_permissions: HashMap<String, AcpPermissionEnvelope>,
    scroll_handle: ScrollHandle,
    auto_scroll: AutoScrollState,
    task_kind: TaskKind,
    /// 当前工具模式展示文案(本轮执行参数,占位)。
    selected_tool: SharedString,
    /// 当前模型。切换时通过 runtime_factory 重建 Runtime,影响后续提交。
    selected_model: Option<ComposerModelOption>,
    model_options: Vec<ComposerModelOption>,
    tool_options: Vec<ComposerMenuOption>,
    runtime_factory: Option<AgentRuntimeFactory>,
    is_running: bool,
    /// 系统提示词（可选，用于自定义 AI 行为）。
    system_instruction: Option<String>,
    /// 代码块操作注册表。
    code_block_actions: CodeBlockActionRegistry,
    /// 可选的局部聊天主题。
    theme: Option<AgentChatTheme>,
    /// 是否侧边栏模式。
    _subscriptions: Vec<Subscription>,
    _event_task: Task<()>,
    /// 当前 ACP 连接的权限请求泵；切换连接时丢弃以隔离旧连接请求。
    _acp_permission_task: Option<Task<()>>,
}

impl AgentChatView {
    pub fn refresh_models(
        &mut self,
        model_options: Vec<ComposerModelOption>,
        selected_model_id: Option<SharedString>,
        runtime_factory: Option<AgentRuntimeFactory>,
        cx: &mut Context<Self>,
    ) {
        let previous_id = self.selected_model.as_ref().map(|model| model.id.clone());
        let (selected, retained) = refreshed_model_selection(
            previous_id.as_ref(),
            selected_model_id.as_ref(),
            &model_options,
        );
        self.model_options = model_options;
        self.runtime_factory = runtime_factory;
        let model_options = self.model_options.clone();
        let tool_options = self.tool_options.clone();
        let task_options = default_task_options();
        self.input.update(cx, |input, cx| {
            input.set_menu_options(model_options, tool_options, task_options, cx);
        });
        if let Some(retained) = retained {
            self.selected_model = Some(retained);
            self.sync_composer(cx);
            cx.notify();
            return;
        }
        if let Some(selected) = selected {
            self.select_model(
                selected.id.as_ref(),
                selected.provider_id.as_ref(),
                selected.model.as_ref(),
                cx,
            );
        }
    }

    /// 创建视图实体。
    pub fn view(
        runtime: Arc<Runtime>,
        resources: ResourceContext,
        mentions: Vec<MentionItem>,
        window: &mut Window,
        cx: &mut App,
    ) -> Entity<Self> {
        Self::view_with_config(
            AgentChatViewConfig::new(runtime, resources, mentions),
            window,
            cx,
        )
    }

    /// 从配置创建视图实体。
    pub fn view_with_config(
        config: AgentChatViewConfig,
        window: &mut Window,
        cx: &mut App,
    ) -> Entity<Self> {
        cx.new(|cx| Self::new(config, window, cx))
    }

    pub(crate) fn new(
        config: AgentChatViewConfig,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let selected_model = selected_model_from_config(&config);
        let sidebar_mode = config.sidebar_mode;
        let show_sidebar_header = config.show_sidebar_header;
        let show_sidebar_frame_controls = config.show_sidebar_frame_controls;
        let sidebar_frame_placement = config.sidebar_frame_placement;
        let theme = config.theme;
        let acp_agents = config.acp_agents;
        let resources = config.resources;
        let available_resources = config.available_resources;
        let mentions = config.mentions;
        let model_options = config.model_options;
        let binding = RuntimeBinding::new(
            config.runtime,
            resources.clone(),
            selected_model,
            config.runtime_factory,
        );
        let runtime = binding.runtime;
        let session_id = binding.session_id;
        let selected_model = binding.selected_model;
        let runtime_factory = binding.runtime_factory;
        let input = cx.new(|cx| {
            AgentInput::with_mentions(mentions, "描述目标，输入 @ 引用资源…", window, cx)
        });
        Self::register_approval_actions(cx);
        if let Some(theme) = theme.clone() {
            input.update(cx, |input, cx| input.set_theme(Some(theme), cx));
        }
        if sidebar_mode {
            input.update(cx, |input, cx| input.set_edge_to_edge(true, cx));
        }

        let task_kind = default_task_kind();
        let selected_tool = default_tool_label();
        let tool_options = default_tool_options();
        let task_options = default_task_options();

        let skills = AgentSkillState::load_default();
        let init_ctx = build_composer_context(
            &resources,
            task_kind,
            &selected_tool,
            selected_model.as_ref(),
            None,
            &[],
            Backend::Local,
            &acp_agents,
            None,
            false,
            None,
            &available_resources,
            skills.summary(),
            skills.items(),
        );
        let target_options: Vec<ComposerTarget> = resources
            .resources
            .iter()
            .map(target_from_resource)
            .collect();
        input.update(cx, |inp, cx| {
            inp.set_target_options(target_options, cx);
            inp.set_menu_options(
                model_options.clone(),
                tool_options.clone(),
                task_options,
                cx,
            );
            inp.set_context(init_ctx, cx);
        });

        let subscriptions = vec![cx.subscribe_in(&input, window, Self::on_input_event)];
        let event_task = Self::spawn_event_pump(runtime.subscribe(), session_id.clone(), cx);
        let current_session = session_id.to_string();
        let mut transcript = AgentTranscript::new();
        transcript.set_resource_context(&resources);

        // 载入已持久化的会话列表。空的实时会话不作为历史占位展示。
        let sessions = persistence::list_summaries(cx);

        Self {
            runtime,
            session_id,
            resources,
            available_resources,
            transcript,
            input,
            sessions,
            current_session,
            sidebar_collapsed: false,
            show_archived: false,
            sidebar_mode,
            show_sidebar_header,
            show_sidebar_frame_controls,
            sidebar_frame_placement,
            history_popover_open: false,
            backend: Backend::Local,
            acp_agents,
            skills,
            acp: None,
            acp_pending: None,
            acp_auth_methods: Vec::new(),
            current_acp_id: None,
            acp_connecting: false,
            acp_connecting_id: None,
            pending_acp_permissions: HashMap::new(),
            scroll_handle: ScrollHandle::new(),
            auto_scroll: AutoScrollState::default(),
            task_kind,
            selected_tool,
            selected_model,
            model_options,
            tool_options,
            runtime_factory,
            is_running: false,
            system_instruction: None,
            theme,
            code_block_actions: CodeBlockActionRegistry::new(),
            _subscriptions: subscriptions,
            _event_task: event_task,
            _acp_permission_task: None,
        }
    }

    fn register_approval_actions(cx: &mut Context<Self>) {
        let view = cx.weak_entity();
        let app: &mut App = cx;
        app.on_action(move |action: &ApproveToolCall, cx: &mut App| {
            let call_id = action.call_id.clone();
            let handled = view
                .update(cx, |this, cx| {
                    this.resolve_pending_tool_action(call_id, true, cx)
                })
                .unwrap_or(false);
            if !handled {
                cx.propagate();
            }
        });

        let view = cx.weak_entity();
        let app: &mut App = cx;
        app.on_action(move |action: &RejectToolCall, cx: &mut App| {
            let call_id = action.call_id.clone();
            let handled = view
                .update(cx, |this, cx| {
                    this.resolve_pending_tool_action(call_id, false, cx)
                })
                .unwrap_or(false);
            if !handled {
                cx.propagate();
            }
        });

        let view = cx.weak_entity();
        let app: &mut App = cx;
        app.on_action(move |action: &SelectAcpPermissionOption, cx: &mut App| {
            let request_id = action.request_id.clone();
            let option_id = action.option_id.clone();
            let handled = view
                .update(cx, |this, cx| {
                    this.resolve_pending_acp_permission(request_id, option_id, cx)
                })
                .unwrap_or(false);
            if !handled {
                cx.propagate();
            }
        });
    }

    fn resolve_pending_tool_action(
        &mut self,
        call_id: String,
        approved: bool,
        cx: &mut Context<Self>,
    ) -> bool {
        if !self.transcript.has_pending_tool_confirm(&call_id) {
            return false;
        }
        self.resolve_tool_call(call_id, approved, cx);
        true
    }

    fn start_acp_permission_session(&mut self, cx: &mut Context<Self>) -> AcpPermissionProvider {
        self.reset_acp_permission_session(cx);
        let (provider, receiver) = acp_permission_channel();
        self._acp_permission_task = Some(Self::spawn_acp_permission_pump(receiver, cx));
        provider
    }

    fn spawn_acp_permission_pump(
        mut receiver: tokio::sync::mpsc::UnboundedReceiver<AcpPermissionMessage>,
        cx: &mut Context<Self>,
    ) -> Task<()> {
        cx.spawn(async move |this, cx| {
            while let Some(message) = receiver.recv().await {
                let updated = this.update(cx, |this, cx| match message {
                    AcpPermissionMessage::Requested(envelope) => {
                        this.receive_acp_permission(envelope, cx)
                    }
                    AcpPermissionMessage::Expired { request_id } => {
                        this.expire_acp_permission(&request_id, cx)
                    }
                });
                if updated.is_err() {
                    break;
                }
            }
        })
    }

    fn receive_acp_permission(&mut self, envelope: AcpPermissionEnvelope, cx: &mut Context<Self>) {
        let request = envelope.request().clone();
        if self
            .pending_acp_permissions
            .contains_key(&request.request_id)
        {
            envelope.resolve(AcpPermissionOutcome::Cancelled);
            return;
        }
        self.transcript.push_acp_permission(&request);
        self.pending_acp_permissions
            .insert(request.request_id, envelope);
        self.request_scroll_to_bottom();
        self.auto_scroll.request_settle();
        cx.notify();
    }

    fn resolve_pending_acp_permission(
        &mut self,
        request_id: String,
        option_id: String,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(envelope) = self.pending_acp_permissions.remove(&request_id) else {
            return false;
        };
        let Some(option) = envelope
            .request()
            .options
            .iter()
            .find(|option| option.option_id == option_id)
            .cloned()
        else {
            self.pending_acp_permissions.insert(request_id, envelope);
            return false;
        };
        let request = envelope.request().clone();
        let grant = acquire_acp_permission_grant(cx, &request, &option);
        let delivered = envelope.resolve(AcpPermissionOutcome::Selected {
            option_id: option.option_id.clone(),
        });
        if delivered {
            if let Some(grant) = grant {
                grant.commit();
            }
            self.transcript.resolve_acp_permission(&request_id, &option);
        } else {
            self.transcript.cancel_acp_permission(&request_id);
        }
        cx.notify();
        true
    }

    fn expire_acp_permission(&mut self, request_id: &str, cx: &mut Context<Self>) {
        if let Some(envelope) = self.pending_acp_permissions.remove(request_id) {
            envelope.resolve(AcpPermissionOutcome::Cancelled);
            self.transcript.cancel_acp_permission(request_id);
            cx.notify();
        }
    }

    fn cancel_pending_acp_permissions(&mut self, cx: &mut Context<Self>) {
        let pending = std::mem::take(&mut self.pending_acp_permissions);
        for (request_id, envelope) in pending {
            envelope.resolve(AcpPermissionOutcome::Cancelled);
            self.transcript.cancel_acp_permission(&request_id);
        }
        cx.notify();
    }

    fn reset_acp_permission_session(&mut self, cx: &mut Context<Self>) {
        self.cancel_pending_acp_permissions(cx);
        self._acp_permission_task = None;
    }

    fn spawn_event_pump(
        mut rx: RuntimeEventReceiver,
        session_id: SessionId,
        cx: &mut Context<Self>,
    ) -> Task<()> {
        cx.spawn(async move |this, cx| {
            loop {
                match rx.recv().await {
                    Ok(event) if event.session_id() == &session_id => {
                        if this
                            .update(cx, |this, cx| this.apply_runtime_event(event, cx))
                            .is_err()
                        {
                            break;
                        }
                    }
                    Ok(_) => {}
                    Err(RecvError::Lagged(_)) => {}
                    Err(RecvError::Closed) => break,
                }
            }
        })
    }

    fn on_input_event(
        &mut self,
        _input: &Entity<AgentInput>,
        event: &AgentInputEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event.clone() {
            AgentInputEvent::Submit {
                text,
                mentions,
                images,
            } => {
                self.submit(text, mentions, images, cx);
            }
            AgentInputEvent::Stop => self.stop(cx),
            AgentInputEvent::SelectTarget { id } => {
                if !self.is_running {
                    self.select_target(&id, cx);
                }
            }
            AgentInputEvent::AddResourceToPool { id } => {
                if !self.is_running {
                    self.add_resource_to_pool(&id, cx);
                }
            }
            AgentInputEvent::RemoveResourceFromPool { id } => {
                if !self.is_running {
                    self.remove_resource_from_pool(&id, cx);
                }
            }
            AgentInputEvent::SelectResourceSource { id } => {
                if !self.is_running {
                    self.select_resource_source(&id, cx);
                }
            }
            AgentInputEvent::ToggleSkill { id } => {
                if !self.is_running {
                    self.toggle_skill(&id, cx);
                }
            }
            AgentInputEvent::ImportSkill { path } => {
                if !self.is_running {
                    self.import_skill(&path, cx);
                }
            }
            AgentInputEvent::PickScope { key: _ } => {}
            AgentInputEvent::SelectModel {
                id,
                provider_id,
                model,
            } => {
                if !self.is_running {
                    self.select_model(&id, &provider_id, &model, cx);
                }
            }
            AgentInputEvent::SelectToolMode { id } => self.select_tool(&id, cx),
            AgentInputEvent::SelectTaskMode { id } => self.select_task(&id, cx),
            AgentInputEvent::SelectAgentBackend { id } => {
                if !self.is_running {
                    self.select_backend(id, cx);
                }
            }
        }
    }

    fn submit(
        &mut self,
        text: String,
        mentions: Vec<MentionItem>,
        images: Vec<crate::ImageAttachment>,
        cx: &mut Context<Self>,
    ) {
        if self.is_running {
            return;
        }
        // ACP 后端:直接把文本交给外部 agent;流式更新经事件泵回灌转录。
        if self.backend == Backend::Acp {
            self.sync_acp_tool_mode_from_provider(cx);
            if self.acp.is_none() {
                self.transcript.push_system("ACP agent 未连接");
                cx.notify();
                return;
            }
            self.transcript.push_user(&text, images.len());
            self.request_scroll_to_bottom();
            self.set_running(true, cx);
            if let Some(acp) = &self.acp {
                acp.prompt(self.skills.selected_context().wrap_user_prompt(&text));
            }
            cx.notify();
            return;
        }
        self.apply_mentions_to_resources(&mentions);
        self.transcript.push_user(&text, images.len());
        self.request_scroll_to_bottom();
        let input =
            UserInput::new(text).with_images(images.iter().map(|i| i.to_input_image()).collect());
        self.set_running(true, cx);

        let runtime = self.runtime.clone();
        let session_id = self.session_id.clone();
        let task_kind = self.task_kind;
        let tool_mode = tool_execution_mode_from_label(&self.selected_tool);
        cx.spawn(async move |this, cx| {
            #[cfg(test)]
            let result = runtime
                .run_turn_blocking_with_tool_mode(&session_id, input, task_kind, tool_mode)
                .await;

            #[cfg(not(test))]
            let result = {
                let task = Tokio::spawn(cx, async move {
                    runtime
                        .run_turn_blocking_with_tool_mode(&session_id, input, task_kind, tool_mode)
                        .await
                });
                match task.await {
                    Ok(result) => result,
                    Err(err) => {
                        let _ = this.update(cx, |this, cx| {
                            this.transcript.push_system(format!("任务执行失败:{err}"));
                            this.set_running(false, cx);
                        });
                        return;
                    }
                }
            };

            if let Err(err) = result {
                let _ = this.update(cx, |this, cx| {
                    this.transcript.push_system(format!("运行失败:{err}"));
                    this.set_running(false, cx);
                });
            }
        })
        .detach();
        cx.notify();
    }

    fn apply_mentions_to_resources(&mut self, mentions: &[MentionItem]) {
        if apply_mentioned_resources(&mut self.resources, &self.available_resources, mentions) {
            self.sync_session_resources();
        }
    }

    fn stop(&mut self, cx: &mut Context<Self>) {
        if self.backend == Backend::Acp {
            if let Some(acp) = &self.acp {
                acp.cancel();
            }
            self.set_running(false, cx);
            cx.notify();
            return;
        }
        if let Err(err) = self.runtime.interrupt(&self.session_id) {
            self.transcript.push_system(format!("停止失败:{err}"));
        }
        self.set_running(false, cx);
        cx.notify();
    }

    fn approve_tool_call(
        &mut self,
        action: &ApproveToolCall,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.resolve_pending_tool_action(action.call_id.clone(), true, cx);
    }

    fn reject_tool_call(
        &mut self,
        action: &RejectToolCall,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.resolve_pending_tool_action(action.call_id.clone(), false, cx);
    }

    fn resolve_tool_call(&mut self, call_id: String, approved: bool, cx: &mut Context<Self>) {
        if self.backend != Backend::Local {
            return;
        }
        self.set_running(true, cx);

        let runtime = self.runtime.clone();
        let session_id = self.session_id.clone();
        let call_id = ToolCallId::from_string(call_id);
        cx.spawn(async move |this, cx| {
            #[cfg(test)]
            let result = if approved {
                runtime.approve_pending_tool(&session_id, &call_id).await
            } else {
                runtime.reject_pending_tool(&session_id, &call_id).await
            };

            #[cfg(not(test))]
            let result = {
                let task = Tokio::spawn(cx, async move {
                    if approved {
                        runtime.approve_pending_tool(&session_id, &call_id).await
                    } else {
                        runtime.reject_pending_tool(&session_id, &call_id).await
                    }
                });
                match task.await {
                    Ok(result) => result,
                    Err(err) => {
                        let _ = this.update(cx, |this, cx| {
                            this.transcript.push_system(format!("工具审批失败:{err}"));
                            this.set_running(false, cx);
                        });
                        return;
                    }
                }
            };

            if let Err(err) = result {
                let _ = this.update(cx, |this, cx| {
                    this.transcript.push_system(format!("工具审批失败:{err}"));
                    this.set_running(false, cx);
                });
            }
        })
        .detach();
        cx.notify();
    }

    fn apply_runtime_event(&mut self, event: RuntimeEvent, cx: &mut Context<Self>) {
        let terminal = matches!(
            event,
            RuntimeEvent::TurnCompleted { .. }
                | RuntimeEvent::TurnCancelled { .. }
                | RuntimeEvent::TurnFailed { .. }
                | RuntimeEvent::NeedUserInput { .. }
        );
        let applied = match &event {
            RuntimeEvent::TurnFailed { reason, .. } if self.backend == Backend::Acp => {
                let error = self.acp_turn_error(reason);
                self.transcript.apply_acp_failure(&event, &error)
            }
            _ => self.transcript.apply(&event),
        };
        if !applied {
            return;
        }
        self.sync_composer(cx);
        // 跟随流式输出 / 新卡片自动滚到底。
        self.request_scroll_to_bottom();
        if terminal {
            if self.backend == Backend::Acp {
                self.cancel_pending_acp_permissions(cx);
            }
            self.auto_scroll.request_settle();
            self.set_running(false, cx);
            // 一轮结束:把会话快照落库(仅自研后端;ACP 会话由外部 agent 管理)。
            if self.backend == Backend::Local {
                self.persist_current(cx);
            }
        }
        cx.notify();
    }

    fn acp_turn_error(&self, reason: &str) -> AcpError {
        let agent_id = self
            .current_acp_id
            .clone()
            .unwrap_or_else(|| SharedString::from("acp"));
        let agent_name = self.acp_agent_name(&agent_id);
        if reason.contains("没有返回任何内容") {
            return AcpError::empty_response(agent_id.to_string(), agent_name.to_string());
        }
        AcpError::new(
            AcpErrorKind::PromptFailed,
            agent_id.to_string(),
            agent_name.to_string(),
            "ACP 请求失败",
        )
        .with_detail(reason)
        .with_recovery(AcpRecoveryAction::Retry)
    }

    fn request_scroll_to_bottom(&mut self) {
        self.auto_scroll.request();
        self.scroll_handle.scroll_to_bottom();
    }

    fn set_running(&mut self, running: bool, cx: &mut Context<Self>) {
        self.is_running = running;
        self.input
            .update(cx, |input, cx| input.set_running(running, cx));
    }

    /// 重建并把展示上下文推给输入框。
    fn sync_composer(&self, cx: &mut Context<Self>) {
        let ctx = build_composer_context(
            &self.resources,
            self.task_kind,
            &self.selected_tool,
            self.selected_model.as_ref(),
            self.transcript.latest_plan(),
            self.transcript.active_subagents(),
            self.backend,
            &self.acp_agents,
            self.current_acp_id.as_ref(),
            self.acp_connecting,
            self.acp.as_ref().map(|acp| acp.state()),
            &self.available_resources,
            self.skills.summary(),
            self.skills.items(),
        );
        self.input.update(cx, |inp, cx| inp.set_context(ctx, cx));
    }

    fn refresh_acp_agents(&mut self, cx: &mut Context<Self>) {
        match build_acp_agent_entries(cx) {
            Ok(agents) => self.refresh_acp_agents_from(agents, cx),
            Err(error) => {
                tracing::warn!(%error, "Failed to refresh ACP agent configs");
            }
        }
    }

    fn refresh_acp_agents_from(&mut self, agents: Vec<AcpAgentEntry>, cx: &mut Context<Self>) {
        self.acp_agents = agents;
        self.sync_composer(cx);
        cx.notify();
    }

    fn agent_switcher_options(&self) -> Vec<ComposerAgentOption> {
        composer_agent_options(
            self.backend,
            &self.acp_agents,
            self.current_acp_id.as_ref(),
            self.acp_connecting,
        )
    }

    /// 在目标下拉中选中某个资源:设为当前目标并同步给会话与输入框。
    fn select_target(&mut self, id: &str, cx: &mut Context<Self>) {
        let rid = ResourceId::new(id.to_string());
        if self.resources.get(&rid).is_none() {
            return;
        }
        self.resources.current = Some(rid);
        self.sync_session_resources();
        self.sync_resource_targets(cx);
        cx.notify();
    }

    fn add_resource_to_pool(&mut self, id: &str, cx: &mut Context<Self>) {
        if add_resource_to_pool(&mut self.resources, &self.available_resources, id) {
            self.sync_session_resources();
            self.sync_resource_targets(cx);
            cx.notify();
        }
    }

    fn remove_resource_from_pool(&mut self, id: &str, cx: &mut Context<Self>) {
        if remove_resource_from_pool(&mut self.resources, id) {
            self.sync_session_resources();
            self.sync_resource_targets(cx);
            cx.notify();
        }
    }

    fn toggle_skill(&mut self, id: &str, cx: &mut Context<Self>) {
        if self.skills.toggle(id) {
            self.sync_session_skills();
            self.sync_composer(cx);
            cx.notify();
        }
    }

    fn import_skill(&mut self, path: &std::path::Path, cx: &mut Context<Self>) {
        match self.skills.import_skill(path) {
            Ok(()) => {
                self.sync_session_skills();
                self.sync_composer(cx);
            }
            Err(error) => {
                self.transcript
                    .push_system(format!("导入 Skill 失败:{error}"));
            }
        }
        cx.notify();
    }

    fn sync_session_skills(&self) {
        if let Some(session) = self.runtime.session(&self.session_id) {
            session.set_skills(self.skills.selected_context());
        }
    }

    fn select_resource_source(&mut self, id: &str, cx: &mut Context<Self>) {
        if apply_resource_source(&mut self.resources, &self.available_resources, id) {
            self.sync_session_resources();
            self.sync_resource_targets(cx);
            cx.notify();
        }
    }

    fn select_model(&mut self, id: &str, provider_id: &str, model: &str, cx: &mut Context<Self>) {
        let Some(opt) = self.model_options.iter().find(|o| {
            o.id.as_ref() == id
                && o.provider_id.as_ref() == provider_id
                && o.model.as_ref() == model
        }) else {
            return;
        };
        let opt = opt.clone();
        // 切换模型会重建 Runtime 与会话,先保存当前会话。
        self.persist_current(cx);
        let mut binding = RuntimeBinding {
            runtime: self.runtime.clone(),
            session_id: self.session_id.clone(),
            selected_model: self.selected_model.clone(),
            runtime_factory: self.runtime_factory.clone(),
        };
        if binding.switch_model(&opt, &self.resources) {
            self.runtime = binding.runtime;
            self.session_id = binding.session_id;
            self.apply_system_instruction_to_current_session();
            self.sync_session_skills();
            self.selected_model = binding.selected_model;
            self.current_session = self.session_id.to_string();
            self.sessions.insert(
                0,
                SessionSummary::new(
                    self.current_session.clone(),
                    format!("{} / {}", opt.provider_label, opt.model),
                    now_secs(),
                ),
            );
            self._event_task =
                Self::spawn_event_pump(self.runtime.subscribe(), self.session_id.clone(), cx);
        } else if self
            .selected_model
            .as_ref()
            .is_some_and(|current| current.id == opt.id)
        {
            self.selected_model = Some(opt);
        } else {
            return;
        }
        self.sync_composer(cx);
        cx.notify();
    }

    fn select_tool(&mut self, id: &str, cx: &mut Context<Self>) {
        if self.is_running {
            return;
        }
        if let Some(opt) = self.tool_options.iter().find(|o| o.id.as_ref() == id) {
            let mode = tool_execution_mode_from_label(&opt.label);
            if self.backend == Backend::Acp
                && let Err(error) = set_current_acp_tool_mode(cx, mode)
            {
                let message = t!(
                    "AgentChat.acp_tool_mode_update_failed",
                    error = error.to_string()
                )
                .to_string();
                tracing::warn!(%error, "Failed to update ACP Public MCP permission mode");
                self.transcript.push_system(message);
                self.request_scroll_to_bottom();
                cx.notify();
                return;
            }
            self.selected_tool = opt.label.clone();
            self.sync_composer(cx);
            cx.notify();
        }
    }

    fn select_task(&mut self, id: &str, cx: &mut Context<Self>) {
        if self.is_running {
            return;
        }
        if let Some(kind) = task_kind_from_id(id) {
            self.set_task_kind(kind, cx);
            self.sync_composer(cx);
        }
    }

    fn new_session(&mut self, cx: &mut Context<Self>) {
        self.history_popover_open = false;
        // ACP 后端:会话由外部 agent 管理,这里仅做视觉重置(清空转录)。
        if self.backend == Backend::Acp {
            if self.is_running {
                self.stop(cx);
            }
            let Some(mut acp) = self.acp.take() else {
                self.transcript.clear();
                self.transcript.push_system("ACP agent 未连接");
                cx.notify();
                return;
            };
            self.transcript.clear();
            self.transcript.push_system("正在创建 ACP 新会话…");
            self.request_scroll_to_bottom();
            cx.notify();
            cx.spawn(async move |this, cx| {
                let cwd = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("/"));
                let result = acp.create_session(cwd).await;
                let _ = this.update(cx, |this, cx| {
                    this.acp = Some(acp);
                    this.transcript.clear();
                    match result {
                        Ok(_) => {}
                        Err(err) => {
                            this.transcript
                                .push_system(format!("创建 ACP 新会话失败:{err}"));
                        }
                    }
                    this.request_scroll_to_bottom();
                    this.sync_composer(cx);
                    cx.notify();
                });
            })
            .detach();
            return;
        }
        // 新建前先保存当前会话,避免内容丢失。
        self.persist_current(cx);
        self.show_archived = false;
        self.start_fresh_session(cx);
        self.reload_sessions(cx);
        cx.notify();
    }

    /// 切换驱动后端:`None` = One_Agent(自研);`Some(id)` = 对应 ACP agent。
    fn select_backend(&mut self, agent_id: Option<SharedString>, cx: &mut Context<Self>) {
        match agent_id {
            None => self.select_local_backend(cx),
            Some(id) => self.select_acp_backend(id, cx),
        }
    }

    /// 新建一个空会话并设为当前(仅运行时层面,不触碰持久化 / 列表)。
    fn start_fresh_session(&mut self, cx: &mut Context<Self>) {
        if self.is_running {
            self.stop(cx);
        }
        let session = self.runtime.create_session(self.resources.clone());
        self.session_id = session.id().clone();
        self.apply_system_instruction_to_current_session();
        self.sync_session_skills();
        self.current_session = self.session_id.to_string();
        self.transcript.clear();
        self._event_task =
            Self::spawn_event_pump(self.runtime.subscribe(), self.session_id.clone(), cx);
    }

    /// 从存储重载当前视图(活跃 / 已归档)的会话列表。
    fn reload_sessions(&mut self, cx: &mut Context<Self>) {
        let mut list = if self.show_archived {
            persistence::list_archived_summaries(cx)
        } else {
            persistence::list_summaries(cx)
        };
        // 活跃视图:仅当当前实时会话已有内容时才置顶,避免空会话在历史中凭空新增。
        if !self.show_archived
            && !list.iter().any(|s| s.id == self.current_session)
            && self.current_runtime_session_has_history()
        {
            let name = self
                .sessions
                .iter()
                .find(|s| s.id == self.current_session)
                .map(|s| s.name.clone())
                .unwrap_or_else(|| SharedString::from(current_agent_task_title()));
            list.insert(
                0,
                SessionSummary::new(self.current_session.clone(), name, now_secs()),
            );
        }
        self.sessions = list;
    }

    fn current_runtime_session_has_history(&self) -> bool {
        self.runtime
            .session(&self.session_id)
            .is_some_and(|session| !session.snapshot().history.is_empty())
    }

    /// 切换「活跃 / 已归档」视图。
    fn toggle_archived(&mut self, cx: &mut Context<Self>) {
        self.show_archived = !self.show_archived;
        self.reload_sessions(cx);
        cx.notify();
    }

    /// 归档(软删除)一个会话;归档当前会话时自动新建空会话顶上。
    fn apply_archive(&mut self, uid: &str, cx: &mut Context<Self>) {
        if !persistence::set_archived(cx, uid, true) {
            return;
        }
        if self.current_session == uid {
            self.start_fresh_session(cx);
        }
        self.reload_sessions(cx);
        cx.notify();
    }

    /// 从归档恢复一个会话(回到活跃列表)。
    fn apply_unarchive(&mut self, uid: &str, cx: &mut Context<Self>) {
        if persistence::set_archived(cx, uid, false) {
            self.reload_sessions(cx);
            cx.notify();
        }
    }

    /// 把当前会话快照写入持久化存储,并刷新其侧边栏摘要(空会话不落库)。
    fn persist_current(&mut self, cx: &mut Context<Self>) {
        let Some(session) = self.runtime.session(&self.session_id) else {
            return;
        };
        // 归档视图下不要把活跃会话塞进展示中的归档列表。
        if let Some((title, updated_at)) = persistence::save_session(cx, &session)
            && !self.show_archived
        {
            let uid = self.current_session.clone();
            self.update_summary(uid, title, updated_at);
        }
    }

    /// 更新(或新增)某会话的侧边栏摘要并置顶。
    fn update_summary(&mut self, uid: String, title: String, updated_at: i64) {
        self.sessions.retain(|s| s.id != uid);
        self.sessions
            .insert(0, SessionSummary::new(uid, title, updated_at));
    }

    /// 切换到另一个(已持久化的)会话:保存当前 → 加载快照恢复 → 重建转录。
    fn switch_session(&mut self, uid: &str, cx: &mut Context<Self>) {
        // 侧边栏视图:从历史 Popover 选择后随即收起。
        self.history_popover_open = false;
        if uid == self.current_session {
            cx.notify();
            return;
        }
        if self.is_running {
            self.stop(cx);
        }
        self.persist_current(cx);

        let should_use_ask_mode = persistence::should_use_ask_mode(cx, uid);
        let Some(snapshot) = persistence::load_snapshot(cx, uid) else {
            // 无快照(如尚未落库的实时会话):仅切换高亮。
            self.current_session = uid.to_string();
            cx.notify();
            return;
        };
        let plan = snapshot.plan.clone();
        let history = snapshot.history.clone();
        let restored = self.runtime.restore_session(snapshot);
        // 用当前视图环境覆盖会话资源,保证后续轮次使用现有连接。
        restored.set_resources(self.resources.clone());
        self.session_id = restored.id().clone();
        self.system_instruction = restored.system_instruction();
        if should_use_ask_mode {
            self.task_kind = TaskKind::Ask;
        }
        self.current_session = self.session_id.to_string();
        self.transcript.load_history(&history, plan.as_ref());
        self._event_task =
            Self::spawn_event_pump(self.runtime.subscribe(), self.session_id.clone(), cx);
        self.reload_sessions(cx);
        self.request_scroll_to_bottom();
        cx.notify();
    }

    fn toggle_sidebar(&mut self, cx: &mut Context<Self>) {
        self.sidebar_collapsed = !self.sidebar_collapsed;
        cx.notify();
    }

    /// 打开重命名对话框。
    fn start_rename(
        &mut self,
        uid: String,
        current_name: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let input_state = cx.new(|cx| {
            InputState::new(window, cx)
                .default_value(&current_name)
                .placeholder("会话名称")
        });
        let view = cx.entity();
        let input = input_state.clone();
        window.open_dialog(cx, move |dialog, _window, _cx| {
            let input_for_ok = input.clone();
            let view_for_ok = view.clone();
            let uid = uid.clone();
            dialog
                .title("重命名会话")
                .w(px(360.0))
                .confirm()
                .button_props(
                    DialogButtonProps::default()
                        .ok_text("保存")
                        .cancel_text("取消"),
                )
                .on_ok(move |_, _window, cx| {
                    let new_name = input_for_ok.read(cx).value().trim().to_string();
                    if !new_name.is_empty() {
                        view_for_ok.update(cx, |this, cx| this.apply_rename(&uid, new_name, cx));
                    }
                    true
                })
                .child(
                    v_flex()
                        .gap_2()
                        .child(div().text_sm().child("输入新的会话名称"))
                        .child(Input::new(&input).w_full()),
                )
        });
    }

    /// 提交重命名:更新存储与侧边栏摘要。
    fn apply_rename(&mut self, uid: &str, new_name: String, cx: &mut Context<Self>) {
        if persistence::rename_session(cx, uid, &new_name) {
            if let Some(summary) = self.sessions.iter_mut().find(|s| s.id == uid) {
                summary.name = new_name.into();
                summary.updated_at = now_secs();
            }
            cx.notify();
        }
    }

    /// 打开删除确认对话框。
    fn confirm_delete(
        &mut self,
        uid: String,
        name: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let view = cx.entity();
        window.open_dialog(cx, move |dialog, _window, _cx| {
            let view_for_ok = view.clone();
            let uid = uid.clone();
            dialog
                .title("删除会话")
                .w(px(360.0))
                .confirm()
                .button_props(
                    DialogButtonProps::default()
                        .ok_text("删除")
                        .cancel_text("取消"),
                )
                .on_ok(move |_, _window, cx| {
                    view_for_ok.update(cx, |this, cx| this.apply_delete(&uid, cx));
                    true
                })
                .child(
                    div()
                        .text_sm()
                        .child(format!("确定删除会话「{name}」?此操作不可撤销。")),
                )
        });
    }

    /// 提交删除:从存储与列表移除;若删的是当前会话,自动新建一个空会话。
    fn apply_delete(&mut self, uid: &str, cx: &mut Context<Self>) {
        persistence::delete_session(cx, uid);
        if self.current_session == uid {
            self.start_fresh_session(cx);
        }
        self.reload_sessions(cx);
        cx.notify();
    }

    fn set_task_kind(&mut self, task_kind: TaskKind, cx: &mut Context<Self>) {
        if !self.is_running {
            self.task_kind = task_kind;
            cx.notify();
        }
    }

    /// 从外部发送消息(兼容 sidebar 的 ask_ai 功能)。
    pub fn send_external_message(&mut self, message: String, cx: &mut Context<Self>) {
        if message.trim().is_empty() {
            return;
        }
        self.submit(message, Vec::new(), Vec::new(), cx);
    }

    /// 设置系统提示词（用于自定义 AI 行为）。
    pub fn set_system_instruction(&mut self, instruction: Option<String>, cx: &mut Context<Self>) {
        self.system_instruction = instruction.clone();
        self.apply_system_instruction_to_current_session();
        cx.notify();
    }

    fn apply_system_instruction_to_current_session(&self) {
        if let Some(session) = self.runtime.session(&self.session_id) {
            session.set_system_instruction(self.system_instruction.clone());
        }
    }

    fn sync_session_resources(&self) {
        if let Some(session) = self.runtime.session(&self.session_id) {
            session.set_resources(self.resources.clone());
        }
    }

    fn sync_resource_targets(&mut self, cx: &mut Context<Self>) {
        self.transcript.set_resource_context(&self.resources);
        let target_options: Vec<ComposerTarget> = self
            .resources
            .resources
            .iter()
            .map(target_from_resource)
            .collect();
        let ctx = build_composer_context(
            &self.resources,
            self.task_kind,
            &self.selected_tool,
            self.selected_model.as_ref(),
            self.transcript.latest_plan(),
            self.transcript.active_subagents(),
            self.backend,
            &self.acp_agents,
            self.current_acp_id.as_ref(),
            self.acp_connecting,
            self.acp.as_ref().map(|acp| acp.state()),
            &self.available_resources,
            self.skills.summary(),
            self.skills.items(),
        );
        self.input.update(cx, |input, cx| {
            input.set_target_options(target_options, cx);
            input.set_context(ctx, cx);
        });
    }

    /// 更新可操作资源上下文与 `@` 提及项。
    pub fn set_resource_context(
        &mut self,
        resources: ResourceContext,
        mentions: Vec<MentionItem>,
        cx: &mut Context<Self>,
    ) {
        let available_resources = resources.resources.clone();
        self.set_resource_context_with_catalog(resources, mentions, available_resources, cx);
    }

    pub fn set_resource_catalog(
        &mut self,
        mentions: Vec<MentionItem>,
        available_resources: Vec<ResourceRef>,
        cx: &mut Context<Self>,
    ) {
        self.available_resources = available_resources;
        let resource_metadata_changed =
            refresh_pool_resource_metadata(&mut self.resources, &self.available_resources);
        self.input
            .update(cx, |input, cx| input.set_mentions(mentions, cx));
        if resource_metadata_changed {
            self.sync_session_resources();
            self.sync_resource_targets(cx);
        } else {
            self.sync_composer(cx);
        }
        cx.notify();
    }

    pub fn set_resource_context_with_catalog(
        &mut self,
        resources: ResourceContext,
        mentions: Vec<MentionItem>,
        available_resources: Vec<ResourceRef>,
        cx: &mut Context<Self>,
    ) {
        self.available_resources = available_resources;
        self.resources = resources.clone();
        self.transcript.set_resource_context(&self.resources);
        self.sync_session_resources();
        let target_options: Vec<ComposerTarget> = self
            .resources
            .resources
            .iter()
            .map(target_from_resource)
            .collect();
        let ctx = build_composer_context(
            &self.resources,
            self.task_kind,
            &self.selected_tool,
            self.selected_model.as_ref(),
            self.transcript.latest_plan(),
            self.transcript.active_subagents(),
            self.backend,
            &self.acp_agents,
            self.current_acp_id.as_ref(),
            self.acp_connecting,
            self.acp.as_ref().map(|acp| acp.state()),
            &self.available_resources,
            self.skills.summary(),
            self.skills.items(),
        );
        self.input.update(cx, |input, cx| {
            input.set_mentions(mentions, cx);
            input.set_target_options(target_options, cx);
            input.set_context(ctx, cx);
        });
        cx.notify();
    }

    /// 注册代码块操作。
    pub fn register_code_block_action(&mut self, action: CodeBlockAction, _cx: &mut Context<Self>) {
        self.code_block_actions.register(action);
    }

    pub fn set_theme(&mut self, theme: Option<AgentChatTheme>, cx: &mut Context<Self>) {
        self.theme = theme.clone();
        self.input
            .update(cx, |input, cx| input.set_theme(theme, cx));
        cx.notify();
    }

    /// 渲染单个会话行:活跃视图可点击切换 + 重命名/归档/删除;归档视图为恢复/永久删除。
    fn render_session_row(
        &self,
        session: &SessionSummary,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let uid = session.id.clone();
        let name = session.name.to_string();
        let archived_view = self.show_archived;
        let selected = !archived_view && self.current_session == session.id;
        let group = SharedString::from(format!("agent-session-row-{uid}"));
        let theme = resolve_agent_chat_theme(self.theme.as_ref(), cx);
        let row_style = themed_session_row_style(&theme);

        // 标题区:活跃视图可点击切换;归档视图只读。
        let label = session_sidebar::session_row_with_style(session, selected, row_style);
        let label_area = if archived_view {
            div().flex_1().min_w_0().child(label).into_any_element()
        } else {
            let switch_uid = uid.clone();
            div()
                .id(SharedString::from(format!("agent-session-{uid}")))
                .flex_1()
                .min_w_0()
                .on_click(cx.listener(move |this, _, _, cx| this.switch_session(&switch_uid, cx)))
                .child(label)
                .into_any_element()
        };

        let mut actions = h_flex()
            .flex_shrink_0()
            .gap_0p5()
            .invisible()
            .group_hover(group.clone(), |this| this.visible());

        let delete_uid = uid.clone();
        let delete_name = name.clone();
        let delete_btn = Button::new(SharedString::from(format!("agent-delete-{uid}")))
            .icon(IconName::Delete)
            .ghost()
            .xsmall()
            .on_click(cx.listener(move |this, _, window, cx| {
                this.confirm_delete(delete_uid.clone(), delete_name.clone(), window, cx);
            }));

        if archived_view {
            let unarchive_uid = uid.clone();
            actions = actions
                .child(
                    Button::new(SharedString::from(format!("agent-unarchive-{uid}")))
                        .icon(IconName::WindowRestore)
                        .ghost()
                        .xsmall()
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.apply_unarchive(&unarchive_uid, cx);
                        })),
                )
                .child(delete_btn);
        } else {
            let rename_uid = uid.clone();
            let rename_name = name.clone();
            let archive_uid = uid.clone();
            actions = actions
                .child(
                    Button::new(SharedString::from(format!("agent-rename-{uid}")))
                        .icon(IconName::Edit)
                        .ghost()
                        .xsmall()
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.start_rename(rename_uid.clone(), rename_name.clone(), window, cx);
                        })),
                )
                .child(
                    Button::new(SharedString::from(format!("agent-archive-{uid}")))
                        .icon(IconName::Inbox)
                        .ghost()
                        .xsmall()
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.apply_archive(&archive_uid, cx);
                        })),
                )
                .child(delete_btn);
        }

        h_flex()
            .w_full()
            .items_center()
            .gap_0p5()
            .group(group)
            .child(label_area)
            .child(actions)
            .into_any_element()
    }

    fn render_sidebar(&mut self, cx: &mut Context<Self>) -> gpui::AnyElement {
        if self.sidebar_collapsed {
            return v_flex()
                .w(px(48.0))
                .h_full()
                .flex_shrink_0()
                .border_r_1()
                .border_color(cx.theme().border)
                .bg(cx.theme().muted)
                .items_center()
                .py_2()
                .gap_2()
                .child(
                    Button::new("agent-expand")
                        .icon(IconName::PanelLeftOpen)
                        .ghost()
                        .small()
                        .on_click(cx.listener(|this, _, _, cx| this.toggle_sidebar(cx))),
                )
                .child(
                    Button::new("agent-new-collapsed")
                        .icon(IconName::Plus)
                        .ghost()
                        .small()
                        .on_click(cx.listener(|this, _, _, cx| this.new_session(cx))),
                )
                .into_any_element();
        }

        let body = if self.backend == Backend::Acp {
            v_flex()
                .flex_1()
                .min_h_0()
                .p_3()
                .child(
                    div()
                        .text_sm()
                        .text_color(cx.theme().muted_foreground)
                        .child("ACP 任务由外部 agent 管理,不在此持久化。"),
                )
                .into_any_element()
        } else {
            let sessions = self.sessions.clone();
            let rows: Vec<gpui::AnyElement> = sessions
                .iter()
                .map(|session| self.render_session_row(session, cx))
                .collect();
            v_flex()
                .id("agent-session-list")
                .flex_1()
                .min_h_0()
                .overflow_y_scroll()
                .p_2()
                .gap_1()
                .children(rows)
                .into_any_element()
        };

        v_flex()
            .w(px(260.0))
            .h_full()
            .min_h_0()
            .flex_shrink_0()
            .border_r_1()
            .border_color(cx.theme().border)
            .bg(cx.theme().muted)
            .child(self.render_sidebar_header(cx))
            .child(body)
            .into_any_element()
    }

    fn render_sidebar_header(&self, cx: &mut Context<Self>) -> gpui::AnyElement {
        let title = agent_history_title(self.show_archived);
        h_flex()
            .w_full()
            .items_center()
            .justify_between()
            .px_3()
            .py_2()
            .border_b_1()
            .border_color(cx.theme().border)
            .child(
                h_flex()
                    .gap_2()
                    .items_center()
                    .child(
                        Button::new("agent-collapse")
                            .icon(IconName::PanelLeftClose)
                            .ghost()
                            .small()
                            .on_click(cx.listener(|this, _, _, cx| this.toggle_sidebar(cx))),
                    )
                    .child(
                        div()
                            .text_sm()
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(title),
                    ),
            )
            .child(
                h_flex()
                    .gap_1()
                    .items_center()
                    .child(
                        Button::new("agent-toggle-archived")
                            .icon(IconName::Inbox)
                            .ghost()
                            .small()
                            .selected(self.show_archived)
                            .on_click(cx.listener(|this, _, _, cx| this.toggle_archived(cx))),
                    )
                    .child(
                        Button::new("agent-new")
                            .icon(IconName::Plus)
                            .ghost()
                            .small()
                            .on_click(cx.listener(|this, _, _, cx| this.new_session(cx))),
                    ),
            )
            .into_any_element()
    }

    /// 侧边栏视图(窄面板)头部:标题 + 新建对话 + 历史记录(Popover)。
    fn render_sidebar_mode_header(&mut self, cx: &mut Context<Self>) -> gpui::AnyElement {
        let theme = resolve_agent_chat_theme(self.theme.as_ref(), cx);
        let border = theme.border;
        let muted = theme.muted;
        let history_open = self.history_popover_open;
        // 仅在打开时构建列表,避免每帧渲染全部会话行。
        let history_list = history_open.then(|| self.render_history_list(cx));

        h_flex()
            .flex_shrink_0()
            .w_full()
            .items_center()
            .justify_between()
            .px_3()
            .py_2()
            .border_b_1()
            .border_color(border)
            .bg(muted)
            .child(self.render_agent_switcher(cx))
            .child(
                h_flex()
                    .gap_1()
                    .items_center()
                    .child(
                        Button::new("agent-sidebar-new")
                            .icon(IconName::Plus)
                            .ghost()
                            .small()
                            .tooltip("新建任务")
                            .on_click(cx.listener(|this, _, _, cx| this.new_session(cx))),
                    )
                    .child(
                        Popover::new("agent-sidebar-history")
                            .anchor(Anchor::TopRight)
                            .p_0()
                            .open(history_open)
                            .on_open_change(cx.listener(|this, open: &bool, _window, cx| {
                                this.history_popover_open = *open;
                                if *open {
                                    this.reload_sessions(cx);
                                }
                                cx.notify();
                            }))
                            .trigger(
                                Button::new("agent-sidebar-history-btn")
                                    .icon(IconName::BookOpen)
                                    .ghost()
                                    .small()
                                    .tooltip("历史任务"),
                            )
                            .when_some(history_list, |popover, list| popover.child(list)),
                    )
                    .when(self.show_sidebar_frame_controls, |this| {
                        this.child(self.render_sidebar_frame_options(cx))
                    })
                    .child(
                        Button::new("agent-sidebar-close")
                            .icon(IconName::Close)
                            .ghost()
                            .small()
                            .tooltip("关闭面板")
                            .on_click(cx.listener(|_this, _, _, cx| {
                                cx.emit(AgentChatViewEvent::Close);
                            })),
                    ),
            )
            .into_any_element()
    }

    pub fn set_sidebar_header_visible(&mut self, visible: bool, cx: &mut Context<Self>) {
        if self.show_sidebar_header == visible {
            return;
        }
        self.show_sidebar_header = visible;
        cx.notify();
    }

    pub fn set_sidebar_frame_controls(
        &mut self,
        visible: bool,
        placement: SidebarPlacement,
        cx: &mut Context<Self>,
    ) {
        if self.show_sidebar_frame_controls == visible && self.sidebar_frame_placement == placement
        {
            return;
        }
        self.show_sidebar_frame_controls = visible;
        self.sidebar_frame_placement = placement;
        cx.notify();
    }

    /// 历史记录 Popover 内容:小标题 + 活跃/归档切换 + 会话行列表(复用行渲染)。
    fn render_history_list(&mut self, cx: &mut Context<Self>) -> gpui::AnyElement {
        let theme = resolve_agent_chat_theme(self.theme.as_ref(), cx);
        let border = theme.border;
        // ACP 模式:会话由外部 agent 管理,不展示本地列表。
        if self.backend == Backend::Acp {
            return v_flex()
                .w(px(300.0))
                .p_3()
                .bg(theme.background)
                .child(
                    div()
                        .text_sm()
                        .text_color(theme.muted_foreground)
                        .child("ACP 任务由外部 agent 管理,不在此持久化。"),
                )
                .into_any_element();
        }
        let title = agent_history_title(self.show_archived);
        let sessions = self.sessions.clone();
        let rows: Vec<gpui::AnyElement> = sessions
            .iter()
            .map(|session| self.render_session_row(session, cx))
            .collect();
        let show_archived = self.show_archived;

        v_flex()
            .w(px(300.0))
            .bg(theme.background)
            .text_color(theme.foreground)
            .child(
                h_flex()
                    .w_full()
                    .items_center()
                    .justify_between()
                    .px_2()
                    .py_1p5()
                    .border_b_1()
                    .border_color(border)
                    .child(
                        div()
                            .text_sm()
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(title),
                    )
                    .child(
                        h_flex()
                            .gap_1()
                            .items_center()
                            .child(
                                Button::new("agent-history-archived")
                                    .icon(IconName::Inbox)
                                    .ghost()
                                    .xsmall()
                                    .selected(show_archived)
                                    .tooltip("已归档")
                                    .on_click(
                                        cx.listener(|this, _, _, cx| this.toggle_archived(cx)),
                                    ),
                            )
                            .child(
                                Button::new("agent-history-new")
                                    .icon(IconName::Plus)
                                    .ghost()
                                    .xsmall()
                                    .tooltip("新建对话")
                                    .on_click(cx.listener(|this, _, _, cx| this.new_session(cx))),
                            ),
                    ),
            )
            .child(if rows.is_empty() {
                div()
                    .px_3()
                    .py_4()
                    .text_sm()
                    .text_color(theme.muted_foreground)
                    .child(if show_archived {
                        "暂无已归档会话"
                    } else {
                        "暂无历史会话"
                    })
                    .into_any_element()
            } else {
                v_flex()
                    .id("agent-history-list")
                    .max_h(px(360.0))
                    .overflow_y_scroll()
                    .p_1()
                    .gap_0p5()
                    .children(rows)
                    .into_any_element()
            })
            .into_any_element()
    }

    fn render_toolbar(&self, cx: &mut Context<Self>) -> gpui::AnyElement {
        h_flex()
            .w_full()
            .items_center()
            .justify_between()
            .px_4()
            .py_2()
            .border_b_1()
            .border_color(cx.theme().border)
            .child(self.render_agent_switcher(cx))
            .into_any_element()
    }

    fn render_sidebar_frame_options(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let view = cx.entity();
        let placement = self.sidebar_frame_placement;
        Button::new("agent-sidebar-frame-options")
            .icon(IconName::Ellipsis)
            .ghost()
            .small()
            .tooltip("面板选项")
            .dropdown_menu_with_anchor(Anchor::TopRight, move |menu, window, cx| {
                build_sidebar_frame_options_menu(menu, view.clone(), placement, window, cx)
            })
    }

    fn render_agent_switcher(&self, cx: &mut Context<Self>) -> gpui::AnyElement {
        let view = cx.entity();
        let theme = resolve_agent_chat_theme(self.theme.as_ref(), cx);
        let label = current_agent_label(
            self.backend,
            &self.acp_agents,
            self.current_acp_id.as_ref(),
            self.acp_connecting,
        );
        let trigger = Button::new("agent-header-switcher-btn")
            .small()
            .icon(current_agent_icon(self.backend))
            .label(compact_agent_label(label.as_ref(), 24))
            .outline()
            .dropdown_caret(true)
            .disabled(self.is_running)
            .bg(theme.panel)
            .border_color(theme.border)
            .text_color(theme.foreground);

        Popover::new("agent-header-switcher")
            .anchor(Anchor::TopLeft)
            .p_0()
            .on_open_change({
                let view = view.clone();
                move |open, _window, cx| {
                    if *open {
                        view.update(cx, |this, cx| this.refresh_acp_agents(cx));
                    }
                }
            })
            .trigger(trigger)
            .content({
                let theme = theme.clone();
                let view_for_content = view.clone();
                move |_state, _window, cx| {
                    let options = view_for_content.read(cx).agent_switcher_options();
                    render_agent_switcher_content(view.clone(), options.clone(), &theme, cx)
                }
            })
            .into_any_element()
    }
}

impl EventEmitter<AgentChatViewEvent> for AgentChatView {}

impl Render for AgentChatView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self.auto_scroll.take_pending_for_render() {
            self.scroll_handle.scroll_to_bottom();
        }
        let chat_theme = resolve_agent_chat_theme(self.theme.as_ref(), cx);
        let messages = if self.sidebar_mode {
            render_sidebar_messages_with_code_actions(
                &self.transcript.messages,
                &self.scroll_handle,
                Some(&self.code_block_actions),
                Some(&chat_theme),
                window,
                cx,
            )
        } else {
            render_messages_with_code_actions(
                &self.transcript.messages,
                &self.scroll_handle,
                Some(&self.code_block_actions),
                Some(&chat_theme),
                window,
                cx,
            )
        };
        let input_area = div()
            .debug_selector(|| "agent-input-area".to_string())
            .w_full()
            .min_w_0()
            .flex_shrink_0()
            .overflow_hidden()
            .border_t_1()
            .border_color(chat_theme.border)
            .bg(chat_theme.background)
            .child(
                v_flex()
                    .w_full()
                    .min_w_0()
                    .when(!self.sidebar_mode, |this| this.p_3())
                    .child(self.input.clone()),
            );
        let auth_actions = self.render_acp_auth_actions(cx);

        if self.sidebar_mode {
            // 侧边栏视图:紧凑头部(新建对话 / 历史记录) + 消息 + 输入。
            let header = self
                .show_sidebar_header
                .then(|| self.render_sidebar_mode_header(cx));
            div()
                .debug_selector(|| "agent-sidebar-root".to_string())
                .size_full()
                .min_w_0()
                .overflow_hidden()
                .text_color(chat_theme.foreground)
                .bg(chat_theme.background)
                .on_action(cx.listener(Self::approve_tool_call))
                .on_action(cx.listener(Self::reject_tool_call))
                .child(
                    v_flex()
                        .debug_selector(|| "agent-sidebar-stack".to_string())
                        .size_full()
                        .min_w_0()
                        .min_h_0()
                        .overflow_hidden()
                        .when_some(header, |this, header| this.child(header))
                        .child(messages)
                        .when_some(auth_actions, |this, actions| this.child(actions))
                        .child(input_area),
                )
        } else {
            // 普通全宽视图:常驻左侧会话栏 + 主区(标题 / 消息 / 输入)。
            let sidebar = self.render_sidebar(cx);
            let toolbar = self.render_toolbar(cx);
            div()
                .size_full()
                .text_color(chat_theme.foreground)
                .bg(chat_theme.background)
                .on_action(cx.listener(Self::approve_tool_call))
                .on_action(cx.listener(Self::reject_tool_call))
                .child(
                    h_flex().size_full().child(sidebar).child(
                        div().flex_1().h_full().min_w_0().child(
                            v_flex()
                                .size_full()
                                .child(toolbar)
                                .child(messages)
                                .when_some(auth_actions, |this, actions| this.child(actions))
                                .child(input_area),
                        ),
                    ),
                )
        }
    }
}

fn build_composer_context(
    resources: &ResourceContext,
    task_kind: TaskKind,
    tool_label: &SharedString,
    model: Option<&ComposerModelOption>,
    plan: Option<&PlanCardData>,
    subagents: &[SubAgentCardData],
    backend: Backend,
    acp_agents: &[AcpAgentEntry],
    current_acp_id: Option<&SharedString>,
    acp_connecting: bool,
    acp_state: Option<AcpSessionState>,
    available_resources: &[ResourceRef],
    skill_summary: ComposerSkillSummary,
    skill_items: Vec<ComposerSkillItem>,
) -> AgentComposerContext {
    let mut context = build_context(resources, task_kind, tool_label, model);
    context.resource_source_options = resource_source_options(resources, available_resources);
    context.resource_pool_items = resource_pool_items(resources, available_resources);
    context.skill_summary = skill_summary;
    context.skill_items = skill_items;
    context.plan_items = composer_plan_items(plan);
    context.subagent_items = composer_subagent_items(subagents);
    context.agent_options =
        composer_agent_options(backend, acp_agents, current_acp_id, acp_connecting);
    if backend == Backend::Acp {
        apply_acp_state_to_context(&mut context, acp_state.as_ref());
    }
    context
}

fn apply_acp_state_to_context(
    context: &mut AgentComposerContext,
    acp_state: Option<&AcpSessionState>,
) {
    context.target = Some(ComposerTarget::new(
        "acp-session",
        acp_state
            .and_then(AcpSessionState::title)
            .unwrap_or("ACP Session"),
        "AI",
        "ACP",
        "Agent Client Protocol",
    ));
    context.scopes = acp_state.map(acp_scopes).unwrap_or_default();
    context.capabilities = acp_state
        .map(acp_capabilities)
        .unwrap_or_else(|| vec![SharedString::from("ACP"), SharedString::from("Connecting")]);
}

fn acp_scopes(state: &AcpSessionState) -> Vec<ComposerScope> {
    let mut scopes = Vec::new();
    if let Some(mode) = acp_mode_label(state) {
        scopes.push(ComposerScope::new("acp-mode", "模式", mode));
    }
    if let Some(updated_at) = state.updated_at() {
        scopes.push(ComposerScope::new("acp-updated", "更新", updated_at));
    }
    if let Some(usage) = state.usage() {
        scopes.push(ComposerScope::new(
            "acp-usage",
            "用量",
            format!("{}/{} tokens", usage.used, usage.size),
        ));
    }
    scopes
}

fn acp_capabilities(state: &AcpSessionState) -> Vec<SharedString> {
    let mut labels = vec![SharedString::from("ACP")];
    labels.extend(acp_agent_capability_labels(state));
    if !state.available_commands().is_empty() {
        labels.push(SharedString::from(format!(
            "命令:{}",
            state.available_commands().len()
        )));
    }
    if !state.config_options().is_empty() {
        labels.push(SharedString::from(format!(
            "配置:{}",
            state.config_options().len()
        )));
    }
    labels
}

fn acp_agent_capability_labels(state: &AcpSessionState) -> Vec<SharedString> {
    let caps = state.agent_capabilities();
    let session = &caps.session_capabilities;
    let mut labels = Vec::new();
    if caps.load_session {
        labels.push(SharedString::from("Load Session"));
    }
    if session.list.is_some() {
        labels.push(SharedString::from("List Sessions"));
    }
    if session.resume.is_some() {
        labels.push(SharedString::from("Resume"));
    }
    if session.close.is_some() {
        labels.push(SharedString::from("Close"));
    }
    if session.delete.is_some() {
        labels.push(SharedString::from("Delete"));
    }
    labels
}

fn acp_mode_label(state: &AcpSessionState) -> Option<String> {
    let current = state.current_mode_id()?;
    Some(
        state
            .available_modes()
            .iter()
            .find(|mode| mode.id == *current)
            .map(|mode| mode.name.clone())
            .unwrap_or_else(|| current.0.to_string()),
    )
}

/// 由资源上下文构建输入框展示用上下文。
fn build_context(
    resources: &ResourceContext,
    task_kind: TaskKind,
    tool_label: &SharedString,
    model: Option<&ComposerModelOption>,
) -> AgentComposerContext {
    let current = resources.current();
    let target = current.map(target_from_resource);
    let scopes = current
        .map(|r| {
            r.scopes
                .iter()
                .map(|scope| ComposerScope::new(&scope.key, &scope.label, &scope.value))
                .collect()
        })
        .unwrap_or_default();
    let capabilities = current
        .map(|r| {
            vec![
                SharedString::from("目标"),
                SharedString::from(r.kind.as_str().to_string()),
            ]
        })
        .unwrap_or_default();
    AgentComposerContext {
        target,
        resource_pool: resource_pool_summary(resources),
        resource_type_filters: resource_type_filters(resources),
        resource_source_options: Vec::new(),
        resource_pool_items: Vec::new(),
        skill_summary: Default::default(),
        skill_items: Vec::new(),
        scopes,
        capabilities,
        plan_items: Vec::new(),
        subagent_items: Vec::new(),
        agent_options: Vec::new(),
        model: model.map(ComposerModelOption::to_composer_model),
        tool_label: tool_label.clone(),
        task_label: SharedString::from(task_kind_label(task_kind)),
    }
}

fn composer_plan_items(plan: Option<&PlanCardData>) -> Vec<ComposerPlanItem> {
    plan.map(|plan| {
        plan.steps
            .iter()
            .map(|step| {
                ComposerPlanItem::new(step.title.clone(), step.status.clone()).with_details(
                    step.description.clone(),
                    step.risk.clone(),
                    step.tool.clone().map(SharedString::from),
                )
            })
            .collect()
    })
    .unwrap_or_default()
}

fn composer_subagent_items(subagents: &[SubAgentCardData]) -> Vec<ComposerSubAgentItem> {
    subagents
        .iter()
        .map(|subagent| {
            ComposerSubAgentItem::new(
                subagent.subagent_id.clone(),
                subagent.name.clone(),
                subagent.task.clone(),
                subagent_status_for_composer(subagent),
            )
            .with_summary(subagent.summary.clone())
        })
        .collect()
}

fn subagent_status_for_composer(subagent: &SubAgentCardData) -> &'static str {
    if subagent.running {
        "running"
    } else if subagent.success == Some(false) {
        "failed"
    } else {
        "completed"
    }
}

fn current_agent_icon(backend: Backend) -> Icon {
    if backend == Backend::Acp {
        Icon::new(IconName::Bot)
    } else {
        Icon::new(IconName::AI).color()
    }
}

fn compact_agent_label(label: &str, max_chars: usize) -> SharedString {
    if label.chars().count() <= max_chars {
        return SharedString::from(label.to_string());
    }
    let mut s: String = label.chars().take(max_chars.saturating_sub(1)).collect();
    s.push_str("...");
    SharedString::from(s)
}

fn render_agent_switcher_content(
    view: Entity<AgentChatView>,
    agents: Vec<ComposerAgentOption>,
    theme: &AgentChatTheme,
    cx: &mut Context<gpui_component::popover::PopoverState>,
) -> gpui::AnyElement {
    let muted = theme.muted_foreground;
    let mut col = v_flex()
        .p_1()
        .gap(px(2.0))
        .min_w(px(300.0))
        .bg(theme.background)
        .text_color(theme.foreground);

    col = col.child(header_switcher_group_label("Agent", theme));
    if agents.is_empty() {
        return col
            .child(
                div()
                    .px_2()
                    .py_2()
                    .text_sm()
                    .text_color(muted)
                    .child("无可用 Agent"),
            )
            .into_any_element();
    }

    for agent in agents {
        col = col.child(header_agent_option_row(
            view.clone(),
            agent,
            muted,
            theme,
            cx,
        ));
    }
    col.into_any_element()
}

fn header_switcher_group_label(label: &'static str, theme: &AgentChatTheme) -> gpui::AnyElement {
    div()
        .px_2()
        .py_1()
        .text_xs()
        .text_color(theme.muted_foreground)
        .child(label)
        .into_any_element()
}

fn header_agent_option_row(
    view: Entity<AgentChatView>,
    agent: ComposerAgentOption,
    muted: gpui::Hsla,
    theme: &AgentChatTheme,
    cx: &mut Context<gpui_component::popover::PopoverState>,
) -> gpui::AnyElement {
    let hover_bg = theme.hover_background();
    let selected_bg = theme.selection_background();
    let selected_fg = theme.foreground;
    let icon_fg = if agent.selected { theme.accent } else { muted };
    let target = agent.id.clone();
    let disabled = agent_option_disabled(&agent);

    h_flex()
        .id(SharedString::from(format!(
            "agent-header-option-{}",
            agent.element_id()
        )))
        .w_full()
        .items_center()
        .gap_2()
        .px_2()
        .py_1p5()
        .rounded(cx.theme().radius)
        .when(agent.selected, |this| this.bg(selected_bg))
        .when(agent.selected, |this| this.text_color(selected_fg))
        .when(disabled, |this| this.opacity(0.5))
        .when(!disabled, |this| {
            this.cursor_pointer()
                .hover(move |this| this.bg(hover_bg))
                .on_click(move |_, _window, cx| {
                    let target = target.clone();
                    view.update(cx, |this, cx| {
                        if !this.is_running {
                            this.select_backend(target, cx);
                        }
                    });
                })
        })
        .child(
            Icon::new(current_agent_icon_for_option(&agent))
                .small()
                .text_color(icon_fg),
        )
        .child(
            v_flex()
                .flex_1()
                .min_w_0()
                .gap(px(1.0))
                .child(div().text_sm().truncate().child(agent.label))
                .child(div().text_xs().text_color(muted).child(agent.subtitle)),
        )
        .when(agent.selected, |this| {
            this.child(Icon::new(IconName::Check).xsmall().text_color(icon_fg))
        })
        .into_any_element()
}

fn current_agent_icon_for_option(agent: &ComposerAgentOption) -> Icon {
    if agent.id.is_some() {
        Icon::new(IconName::Bot)
    } else {
        Icon::new(IconName::AI).color()
    }
}

fn resource_pool_summary(resources: &ResourceContext) -> ComposerResourcePoolSummary {
    let current = resources.current();
    ComposerResourcePoolSummary::new(
        current.map(|resource| SharedString::from(resource.id.as_str().to_string())),
        current
            .map(|resource| resource.label.clone())
            .unwrap_or_else(|| "无默认目标".to_string()),
        resources.resources.len(),
    )
}

fn resource_type_filters(resources: &ResourceContext) -> Vec<ComposerResourceTypeFilter> {
    let mut counts = std::collections::BTreeMap::<String, usize>::new();
    for resource in &resources.resources {
        *counts
            .entry(resource.kind.as_str().to_string())
            .or_default() += 1;
    }

    let mut filters = vec![ComposerResourceTypeFilter::new(
        "all",
        "全部",
        resources.resources.len(),
        true,
    )];
    filters.extend(counts.into_iter().map(|(kind, count)| {
        ComposerResourceTypeFilter::new(kind.clone(), kind.to_uppercase(), count, false)
    }));
    filters
}

fn resource_source_options(
    pool: &ResourceContext,
    catalog: &[ResourceRef],
) -> Vec<ComposerResourceSourceOption> {
    let pool_ids = resource_id_set(&pool.resources);
    let catalog_ids = resource_id_set(catalog);
    let current_selected = pool.resources.len() == 1
        && pool
            .current
            .as_ref()
            .is_some_and(|current| Some(current) == pool.resources.first().map(|r| &r.id));
    let all_selected = !current_selected && !catalog_ids.is_empty() && pool_ids == catalog_ids;
    let ssh_ids = source_ids(catalog, |kind| matches!(kind, ResourceKind::Ssh));
    let db_ids = source_ids(catalog, is_database_kind);
    let redis_ids = source_ids(catalog, |kind| matches!(kind, ResourceKind::Redis));
    let terminal_ids = source_ids(catalog, |kind| matches!(kind, ResourceKind::Terminal));
    let source_selected = |ids: &std::collections::HashSet<ResourceId>| {
        !current_selected && !all_selected && !ids.is_empty() && pool_ids == *ids
    };
    let type_selected = source_selected(&ssh_ids)
        || source_selected(&db_ids)
        || source_selected(&redis_ids)
        || source_selected(&terminal_ids);
    let manual_selected = !current_selected && !all_selected && !type_selected;

    vec![
        ComposerResourceSourceOption::new("current", "当前", current_count(pool), current_selected),
        ComposerResourceSourceOption::new("pool", "资源池", pool.resources.len(), false),
        ComposerResourceSourceOption::new("all", "全部", catalog.len(), all_selected),
        ComposerResourceSourceOption::new("ssh", "SSH", ssh_ids.len(), source_selected(&ssh_ids)),
        ComposerResourceSourceOption::new("db", "DB", db_ids.len(), source_selected(&db_ids)),
        ComposerResourceSourceOption::new(
            "redis",
            "Redis",
            redis_ids.len(),
            source_selected(&redis_ids),
        ),
        ComposerResourceSourceOption::new(
            "terminal",
            "Terminal",
            terminal_ids.len(),
            source_selected(&terminal_ids),
        ),
        ComposerResourceSourceOption::new("manual", "手动", pool.resources.len(), manual_selected),
        ComposerResourceSourceOption::new("workspace", "工作区", 0, false)
            .disabled("暂无工作区资源来源"),
        ComposerResourceSourceOption::new("tag", "标签", 0, false).disabled("暂无标签资源来源"),
    ]
}

fn resource_id_set(resources: &[ResourceRef]) -> std::collections::HashSet<ResourceId> {
    resources
        .iter()
        .map(|resource| resource.id.clone())
        .collect()
}

fn source_ids(
    catalog: &[ResourceRef],
    predicate: fn(&ResourceKind) -> bool,
) -> std::collections::HashSet<ResourceId> {
    catalog
        .iter()
        .filter(|resource| predicate(&resource.kind))
        .map(|resource| resource.id.clone())
        .collect()
}

fn is_database_kind(kind: &ResourceKind) -> bool {
    matches!(
        kind,
        ResourceKind::Mysql | ResourceKind::Postgres | ResourceKind::Sqlite | ResourceKind::Mongo
    )
}

fn current_count(pool: &ResourceContext) -> usize {
    usize::from(pool.current().is_some())
}

fn resource_pool_items(
    pool: &ResourceContext,
    catalog: &[ResourceRef],
) -> Vec<ComposerResourcePoolItem> {
    let pool_ids = pool
        .resources
        .iter()
        .map(|resource| resource.id.clone())
        .collect::<std::collections::HashSet<_>>();
    let default_id = pool.current.clone();

    catalog
        .iter()
        .map(|resource| {
            let in_pool = pool_ids.contains(&resource.id);
            let is_default = default_id.as_ref() == Some(&resource.id);
            ComposerResourcePoolItem::new(
                resource.id.as_str().to_string(),
                resource.label.clone(),
                kind_icon(&resource.kind),
                resource.kind.as_str().to_string(),
                resource_primary_meta(resource),
                resource_pool_status(in_pool),
                resource_default_reason(is_default),
                resource.capabilities.len(),
                in_pool,
                is_default,
            )
        })
        .collect()
}

fn resource_primary_meta(resource: &ResourceRef) -> String {
    first_visible_alias(&resource.aliases)
        .or_else(|| {
            resource
                .scopes
                .first()
                .map(|scope| format!("{}: {}", scope.label, scope.value))
        })
        .unwrap_or_else(|| resource.kind.as_str().to_string())
}

fn resource_pool_status(in_pool: bool) -> &'static str {
    if in_pool { "已加入" } else { "可添加" }
}

fn resource_default_reason(is_default: bool) -> Option<&'static str> {
    is_default.then_some("默认目标")
}

fn refresh_pool_resource_metadata(pool: &mut ResourceContext, catalog: &[ResourceRef]) -> bool {
    let mut changed = false;
    for resource in &mut pool.resources {
        let Some(updated) = catalog
            .iter()
            .find(|candidate| candidate.id == resource.id)
            .cloned()
        else {
            continue;
        };
        if *resource != updated {
            *resource = updated;
            changed = true;
        }
    }
    changed
}

fn add_resource_to_pool(pool: &mut ResourceContext, catalog: &[ResourceRef], id: &str) -> bool {
    let rid = ResourceId::new(id.to_string());
    if pool.get(&rid).is_some() {
        return false;
    }
    let Some(resource) = catalog.iter().find(|resource| resource.id == rid).cloned() else {
        return false;
    };
    pool.resources.push(resource);
    if pool.current.is_none() {
        pool.current = Some(rid);
    }
    true
}

fn apply_mentioned_resources(
    pool: &mut ResourceContext,
    catalog: &[ResourceRef],
    mentions: &[MentionItem],
) -> bool {
    let mut changed = false;
    let mut first_mentioned_id: Option<ResourceId> = None;
    for mention in mentions {
        let rid = ResourceId::new(mention.id.clone());
        if first_mentioned_id.is_none() {
            first_mentioned_id = Some(rid.clone());
        }
        if pool.get(&rid).is_some() {
            continue;
        }
        if let Some(resource) = catalog.iter().find(|resource| resource.id == rid).cloned() {
            pool.resources.push(resource);
            changed = true;
        }
    }
    if let Some(id) = first_mentioned_id.filter(|id| pool.get(id).is_some()) {
        if pool.current.as_ref() != Some(&id) {
            pool.current = Some(id);
            changed = true;
        }
    }
    changed
}

fn remove_resource_from_pool(pool: &mut ResourceContext, id: &str) -> bool {
    let rid = ResourceId::new(id.to_string());
    let before = pool.resources.len();
    pool.resources.retain(|resource| resource.id != rid);
    if pool.resources.len() == before {
        return false;
    }
    if pool.current.as_ref() == Some(&rid) {
        pool.current = pool.resources.first().map(|resource| resource.id.clone());
    }
    true
}

fn apply_resource_source(pool: &mut ResourceContext, catalog: &[ResourceRef], id: &str) -> bool {
    let resources = match id {
        "current" => pool.current().cloned().map(|resource| vec![resource]),
        "all" => Some(catalog.to_vec()),
        "ssh" => Some(resources_matching(catalog, |kind| {
            matches!(kind, ResourceKind::Ssh)
        })),
        "db" => Some(resources_matching(catalog, is_database_kind)),
        "redis" => Some(resources_matching(catalog, |kind| {
            matches!(kind, ResourceKind::Redis)
        })),
        "terminal" => Some(resources_matching(catalog, |kind| {
            matches!(kind, ResourceKind::Terminal)
        })),
        "pool" | "manual" | "workspace" | "tag" => None,
        _ => None,
    };
    let Some(resources) = resources else {
        return false;
    };
    replace_pool_resources(pool, resources)
}

fn resources_matching(
    catalog: &[ResourceRef],
    predicate: fn(&ResourceKind) -> bool,
) -> Vec<ResourceRef> {
    catalog
        .iter()
        .filter(|resource| predicate(&resource.kind))
        .cloned()
        .collect()
}

fn replace_pool_resources(pool: &mut ResourceContext, resources: Vec<ResourceRef>) -> bool {
    if resources.is_empty() {
        return false;
    }
    let next_current = pool
        .current
        .clone()
        .filter(|id| resources.iter().any(|resource| resource.id == *id))
        .or_else(|| resources.first().map(|resource| resource.id.clone()));
    let changed = pool.resources != resources || pool.current != next_current;
    if changed {
        pool.resources = resources;
        pool.current = next_current;
    }
    changed
}

fn target_from_resource(r: &ResourceRef) -> ComposerTarget {
    ComposerTarget::new(
        r.id.as_str().to_string(),
        r.label.clone(),
        kind_icon(&r.kind),
        r.kind.as_str().to_string(),
        format!("{} · {}", r.kind.as_str(), r.id),
    )
}

fn kind_icon(kind: &ResourceKind) -> &'static str {
    match kind {
        ResourceKind::Mysql | ResourceKind::Postgres | ResourceKind::Sqlite => "DB",
        ResourceKind::Ssh => "SH",
        ResourceKind::Redis => "RD",
        ResourceKind::Mongo => "MG",
        ResourceKind::Terminal => "TM",
        ResourceKind::Other(kind) => match kind.as_str() {
            "rdp" => "RD",
            "vnc" => "VN",
            "port-forwarding" => "PF",
            _ => "OT",
        },
    }
}

fn task_kind_label(kind: TaskKind) -> &'static str {
    match kind {
        TaskKind::Agent => "Auto Mode",
        TaskKind::Ask => "Ask",
        TaskKind::Plan => "Plan",
    }
}

fn task_kind_from_id(id: &str) -> Option<TaskKind> {
    match id {
        "agent" => Some(TaskKind::Agent),
        "ask" => Some(TaskKind::Ask),
        "plan" => Some(TaskKind::Plan),
        _ => None,
    }
}

fn tool_execution_mode_from_label(label: &SharedString) -> ToolExecutionMode {
    match label.as_ref() {
        "只读" => ToolExecutionMode::ReadOnly,
        "手动确认" => ToolExecutionMode::Manual,
        _ => ToolExecutionMode::Auto,
    }
}

fn tool_execution_mode_label(mode: ToolExecutionMode) -> &'static str {
    match mode {
        ToolExecutionMode::Auto => "自动",
        ToolExecutionMode::ReadOnly => "只读",
        ToolExecutionMode::Manual => "手动确认",
    }
}

fn default_tool_label() -> SharedString {
    SharedString::from("手动确认")
}

fn static_runtime_model_option(runtime: &Runtime) -> ComposerModelOption {
    let model = runtime.services().model.model_name().to_string();
    ComposerModelOption::new("runtime:current", "runtime", "当前 Runtime", model)
        .with_hint("固定运行时")
}

fn selected_model_from_config(config: &AgentChatViewConfig) -> Option<ComposerModelOption> {
    config
        .selected_model_id
        .as_ref()
        .and_then(|id| config.model_options.iter().find(|m| &m.id == id))
        .cloned()
        .or_else(|| config.model_options.first().cloned())
}

fn refreshed_model_selection(
    previous_id: Option<&SharedString>,
    selected_model_id: Option<&SharedString>,
    model_options: &[ComposerModelOption],
) -> (Option<ComposerModelOption>, Option<ComposerModelOption>) {
    let retained = previous_id
        .and_then(|id| model_options.iter().find(|model| &model.id == id))
        .cloned();
    let selected = retained
        .clone()
        .or_else(|| {
            selected_model_id
                .and_then(|id| model_options.iter().find(|model| &model.id == id))
                .cloned()
        })
        .or_else(|| model_options.first().cloned());
    (selected, retained)
}

fn runtime_specs_from_provider_configs(
    provider_configs: Vec<ProviderConfig>,
    registry: ToolRegistry,
) -> anyhow::Result<Vec<RuntimeBuildSpec>> {
    let mut specs = Vec::new();
    for config in provider_configs.into_iter().filter(|config| config.enabled) {
        let provider: Arc<dyn LlmProvider> = Arc::new(LlmConnector::from_config(&config)?);
        specs.extend(runtime_specs_for_provider_config(
            &config,
            provider,
            registry.clone(),
        ));
    }
    Ok(specs)
}

async fn runtime_specs_from_provider_state(
    provider_configs: Vec<ProviderConfig>,
    registry: ToolRegistry,
    provider_state: GlobalProviderState,
) -> anyhow::Result<Vec<RuntimeBuildSpec>> {
    let mut specs = Vec::new();
    for config in provider_configs.into_iter().filter(|config| config.enabled) {
        let provider = provider_state.manager().get_provider(&config).await?;
        specs.extend(runtime_specs_for_provider_config(
            &config,
            provider,
            registry.clone(),
        ));
    }
    Ok(specs)
}

fn runtime_specs_for_provider_config(
    config: &ProviderConfig,
    provider: Arc<dyn LlmProvider>,
    registry: ToolRegistry,
) -> Vec<RuntimeBuildSpec> {
    provider_models(config)
        .into_iter()
        .map(|model| {
            let option = ComposerModelOption::new(
                provider_model_option_id(config.id, &model),
                config.id.to_string(),
                provider_label(config),
                model.clone(),
            )
            .with_hint(format!(
                "{} · 正式模型",
                config.provider_type.display_name()
            ));
            RuntimeBuildSpec {
                option,
                provider: provider.clone(),
                model: model.clone(),
                registry: registry.clone(),
                temperature: config.temperature,
                max_tokens: config.max_tokens.and_then(|v| u32::try_from(v).ok()),
                is_default: config.is_default && model == config.model,
            }
        })
        .collect()
}

fn provider_models(config: &ProviderConfig) -> Vec<String> {
    let mut models = Vec::new();
    if !config.model.is_empty() {
        models.push(config.model.clone());
    }
    for model in &config.models {
        if !model.is_empty() && !models.contains(model) {
            models.push(model.clone());
        }
    }
    models
}

fn provider_model_option_id(provider_id: i64, model: &str) -> String {
    format!("provider:{provider_id}:{model}")
}

fn provider_label(config: &ProviderConfig) -> String {
    if config.name.is_empty() {
        config.provider_type.display_name().to_string()
    } else {
        config.name.clone()
    }
}

fn selected_provider_model_id(specs: &[RuntimeBuildSpec]) -> Option<SharedString> {
    specs
        .iter()
        .find(|spec| spec.is_default)
        .or_else(|| specs.first())
        .map(|spec| spec.option.id.clone())
}

fn default_tool_options() -> Vec<ComposerMenuOption> {
    vec![
        ComposerMenuOption::new("auto", "自动"),
        ComposerMenuOption::new("readonly", "只读"),
        ComposerMenuOption::new("manual", "手动确认"),
    ]
}

fn default_task_kind() -> TaskKind {
    TaskKind::Agent
}

fn default_task_options() -> Vec<ComposerMenuOption> {
    vec![
        ComposerMenuOption::new("agent", "Auto Mode").with_hint("按需回答、规划或调用工具"),
        ComposerMenuOption::new("ask", "Ask").with_hint("直接问答"),
        ComposerMenuOption::new("plan", "Plan").with_hint("先规划再执行"),
    ]
}

fn now_secs() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent_cards::{
        ACP_PERMISSION_CARD, AcpPermissionCardData, AcpPermissionOptionData, TOOL_CARD,
        TOOL_CONFIRM_CARD, ToolCardData, ToolConfirmCardData,
    };
    use crate::{AcpAgentConfig, AcpConfigDiagnostic, AcpPermissionOption, AcpPermissionRequest};
    use agent_runtime::RuntimeServices;
    use agent_runtime::model::MockModelClient;
    use agent_runtime::model::function_tool_call;
    use agent_runtime::model::{ModelClient, ModelRequest, ModelResponse, ModelStream};
    use agent_runtime::tools::ToolInvocation;
    use agent_runtime::tools::builtin::EchoTool;
    use agent_runtime::{
        ObservationData, RiskLevel, Tool, ToolError, ToolName, ToolObservation, ToolRegistry,
        ToolRouter, ToolSpec,
    };
    use async_trait::async_trait;
    use gpui::{
        Entity, IntoElement, Modifiers, ParentElement, Render, ScrollDelta, ScrollWheelEvent,
        Styled, TestAppContext, TouchPhase, VisualTestContext, Window, div, point, px,
    };
    use one_core::llm::{ProviderConfig, ProviderType};
    use serde_json::json;
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct WriteTool;

    struct FixedSidebarHost {
        view: Entity<AgentChatView>,
    }

    fn test_acp_permission_request() -> AcpPermissionRequest {
        AcpPermissionRequest {
            request_id: "session:call".into(),
            session_id: "session".into(),
            tool_call_id: "call".into(),
            tool_name: "Write file".into(),
            summary: "ACP Agent 请求执行工具：Write file".into(),
            details: json!({"path": "/tmp/a"}),
            options: vec![
                AcpPermissionOption {
                    option_id: "reject".into(),
                    name: "拒绝".into(),
                    kind: "reject_once".into(),
                },
                AcpPermissionOption {
                    option_id: "allow".into(),
                    name: "仅本次允许".into(),
                    kind: "allow_once".into(),
                },
            ],
        }
    }

    impl FixedSidebarHost {
        fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
            let config =
                AgentChatViewConfig::new(test_runtime("m"), ResourceContext::new(), vec![])
                    .sidebar_mode(true);
            let view = cx.new(|cx| AgentChatView::new(config, window, cx));
            Self { view }
        }
    }

    impl Render for FixedSidebarHost {
        fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
            v_flex()
                .debug_selector(|| "fixed-sidebar-host".to_string())
                .w(px(420.0))
                .h(px(640.0))
                .overflow_hidden()
                .child(
                    div()
                        .debug_selector(|| "fixed-sidebar-header".to_string())
                        .h(px(34.0))
                        .w_full()
                        .flex_shrink_0(),
                )
                .child(
                    div()
                        .debug_selector(|| "fixed-sidebar-content-slot".to_string())
                        .flex_1()
                        .min_h_0()
                        .min_w_0()
                        .overflow_hidden()
                        .child(self.view.clone()),
                )
        }
    }

    #[async_trait]
    impl Tool for WriteTool {
        fn name(&self) -> ToolName {
            ToolName::new("write_data")
        }

        fn spec(&self, _resources: &ResourceContext) -> ToolSpec {
            ToolSpec::new(
                "write_data",
                "写入测试数据。",
                json!({
                    "type": "object",
                    "properties": {"value": {"type": "string"}},
                    "required": ["value"]
                }),
            )
            .with_risk(RiskLevel::Low)
        }

        async fn execute(&self, invocation: ToolInvocation) -> Result<ToolObservation, ToolError> {
            Ok(ToolObservation::success(
                invocation.call_id,
                invocation.tool_name,
                "write executed",
                ObservationData::Text("executed".into()),
            ))
        }
    }

    #[test]
    fn target_maps_label_kind_icon() {
        let r = ResourceRef::new("c1", ResourceKind::Redis, "prod-redis");
        let t = target_from_resource(&r);
        assert_eq!(t.label.as_ref(), "prod-redis");
        assert_eq!(t.kind.as_ref(), "redis");
        assert_eq!(t.icon.as_ref(), "RD");
    }

    #[test]
    fn auto_scroll_state_consumes_pending_scroll_in_render() {
        let mut state = AutoScrollState::default();

        assert!(!state.take_pending_for_render());
        state.request();
        assert!(state.take_pending_for_render());
        assert!(state.take_pending_for_render());
        assert!(!state.take_pending_for_render());
    }

    #[test]
    fn auto_scroll_state_terminal_request_spans_multiple_renders() {
        let mut state = AutoScrollState::default();

        state.request_settle();
        for _ in 0..5 {
            assert!(state.take_pending_for_render());
        }
        assert!(!state.take_pending_for_render());
    }

    #[test]
    fn build_context_without_target_is_empty() {
        let ctx = build_context(
            &ResourceContext::new(),
            TaskKind::Agent,
            &SharedString::from("自动"),
            None,
        );
        assert!(ctx.target.is_none());
        assert!(ctx.scopes.is_empty());
        assert!(ctx.capabilities.is_empty());
        assert_eq!(ctx.task_label.as_ref(), "Auto Mode");
    }

    #[test]
    fn build_context_with_target_fills_scopes_and_caps() {
        let resources = ResourceContext::new().with_resource(
            ResourceRef::new("c1", ResourceKind::Mysql, "prod-mysql")
                .with_scope(agent_runtime::ResourceScope::new(
                    "database", "Database", "ai_app",
                ))
                .with_scope(agent_runtime::ResourceScope::new(
                    "schema", "Schema", "public",
                )),
        );
        let ctx = build_context(
            &resources,
            TaskKind::Agent,
            &SharedString::from("只读"),
            Some(&ComposerModelOption::new(
                "openai:gpt-4.1",
                "openai",
                "OpenAI",
                "gpt-4.1",
            )),
        );
        assert_eq!(ctx.target.unwrap().label.as_ref(), "prod-mysql");
        assert_eq!(ctx.scopes.len(), 2);
        assert_eq!(ctx.scopes[0].value.as_ref(), "ai_app");
        assert_eq!(ctx.scopes[1].value.as_ref(), "public");
        assert_eq!(ctx.task_label.as_ref(), "Auto Mode");
        assert_eq!(ctx.tool_label.as_ref(), "只读");
    }

    #[test]
    fn build_context_marks_current_resource_as_default_target() {
        let resources = ResourceContext::new()
            .with_resource(ResourceRef::new("ssh-a", ResourceKind::Ssh, "prod-a"))
            .with_resource(ResourceRef::new("ssh-b", ResourceKind::Ssh, "prod-b"));

        let context = build_context(
            &resources,
            TaskKind::Agent,
            &SharedString::from("自动"),
            None,
        );

        assert_eq!(context.resource_pool.total_resources, 2);
        assert_eq!(
            context
                .resource_pool
                .default_target_id
                .as_ref()
                .map(|id| id.as_ref()),
            Some("ssh-a")
        );
        assert_eq!(context.resource_pool.default_label.as_ref(), "prod-a");
    }

    #[test]
    fn build_context_counts_resource_types_for_filters() {
        let resources = ResourceContext::new()
            .with_resource(ResourceRef::new("ssh-a", ResourceKind::Ssh, "prod-a"))
            .with_resource(ResourceRef::new("db-a", ResourceKind::Postgres, "prod-db"))
            .with_resource(ResourceRef::new("redis-a", ResourceKind::Redis, "cache"));

        let context = build_context(
            &resources,
            TaskKind::Agent,
            &SharedString::from("自动"),
            None,
        );

        let filters = context
            .resource_type_filters
            .iter()
            .map(|filter| (filter.id.as_ref(), filter.count))
            .collect::<Vec<_>>();

        assert_eq!(
            vec![("all", 3), ("postgres", 1), ("redis", 1), ("ssh", 1)],
            filters
        );
    }

    #[test]
    fn agent_config_defaults_available_resources_to_pool_resources() {
        let resources = ResourceContext::new().with_resource(ResourceRef::new(
            "ssh-a",
            ResourceKind::Ssh,
            "prod-a",
        ));

        let config = AgentChatViewConfig::new(test_runtime("m"), resources.clone(), Vec::new());

        assert_eq!(config.available_resources, resources.resources);
    }

    #[test]
    fn agent_config_can_start_with_empty_scope_and_non_empty_catalog() {
        let catalog = agent_runtime::ResourceCatalog::new(vec![
            ResourceRef::new("ssh-a", ResourceKind::Ssh, "prod-a"),
            ResourceRef::new("db-a", ResourceKind::Mysql, "prod-db"),
        ]);
        let scope = agent_runtime::AgentResourceScope::empty();

        let config = AgentChatViewConfig::new_with_scope(
            test_runtime("m"),
            scope,
            catalog.clone(),
            Vec::new(),
        );

        assert!(config.resources.is_empty());
        assert_eq!(catalog.resources, config.available_resources);
    }

    #[test]
    fn agent_config_accepts_available_resource_catalog() {
        let pool = ResourceContext::new().with_resource(ResourceRef::new(
            "ssh-a",
            ResourceKind::Ssh,
            "prod-a",
        ));
        let catalog = vec![
            ResourceRef::new("ssh-a", ResourceKind::Ssh, "prod-a"),
            ResourceRef::new("ssh-b", ResourceKind::Ssh, "prod-b"),
        ];

        let config = AgentChatViewConfig::new(test_runtime("m"), pool, Vec::new())
            .with_available_resources(catalog.clone());

        assert_eq!(config.available_resources, catalog);
    }

    #[gpui::test]
    fn gpui_refreshing_resource_catalog_preserves_current_scope(cx: &mut TestAppContext) {
        init_test_ui(cx);
        let pool = ResourceContext::new().with_resource(ResourceRef::new(
            "ssh-a",
            ResourceKind::Ssh,
            "prod-a",
        ));
        let initial_catalog = vec![ResourceRef::new("ssh-a", ResourceKind::Ssh, "prod-a")];
        let config = AgentChatViewConfig::new(test_runtime("m"), pool, Vec::new())
            .with_available_resources(initial_catalog);

        let (view, cx) =
            cx.add_window_view(move |window, cx| AgentChatView::new(config, window, cx));
        view.update(cx, |view, cx| {
            view.set_resource_catalog(
                vec![
                    MentionItem::new("ssh-a", "prod-a", "ssh", "ssh"),
                    MentionItem::new("db-a", "prod-db", "mysql", "mysql"),
                ],
                vec![
                    ResourceRef::new("ssh-a", ResourceKind::Ssh, "prod-a-renamed"),
                    ResourceRef::new("db-a", ResourceKind::Mysql, "prod-db"),
                ],
                cx,
            );
        });

        let (pool_labels, default_id, catalog_labels) = view.read_with(cx, |view, _| {
            (
                view.resources
                    .resources
                    .iter()
                    .map(|resource| resource.label.as_str().to_string())
                    .collect::<Vec<_>>(),
                view.resources
                    .current
                    .as_ref()
                    .map(|id| id.as_str().to_string()),
                view.available_resources
                    .iter()
                    .map(|resource| resource.label.as_str().to_string())
                    .collect::<Vec<_>>(),
            )
        });

        assert_eq!(vec!["prod-a-renamed"], pool_labels);
        assert_eq!(Some("ssh-a".to_string()), default_id);
        assert_eq!(vec!["prod-a-renamed", "prod-db"], catalog_labels);
    }

    #[gpui::test]
    fn local_stop_ack_immediately_clears_running_state(cx: &mut TestAppContext) {
        init_test_ui(cx);
        let config =
            AgentChatViewConfig::new(test_runtime("m"), ResourceContext::new(), Vec::new());
        let (view, cx) =
            cx.add_window_view(move |window, cx| AgentChatView::new(config, window, cx));

        view.update(cx, |view, cx| {
            view.set_running(true, cx);
            view.stop(cx);
        });

        assert!(!view.read_with(cx, |view, _| view.is_running));
    }

    #[test]
    fn applying_mentioned_resource_adds_from_catalog_and_sets_default() {
        let mut resources = ResourceContext::new();
        let catalog = vec![
            ResourceRef::new("ssh-a", ResourceKind::Ssh, "prod-a"),
            ResourceRef::new("db-a", ResourceKind::Mysql, "prod-db"),
        ];
        let mentions = vec![MentionItem::new("db-a", "prod-db", "mysql", "mysql")];

        assert!(apply_mentioned_resources(
            &mut resources,
            &catalog,
            &mentions
        ));

        assert_eq!(1, resources.resources.len());
        assert_eq!(
            Some("prod-db"),
            resources.current().map(|resource| resource.label.as_str())
        );
    }

    #[test]
    fn resource_pool_items_mark_pool_membership_and_default_target() {
        let pool = ResourceContext::new().with_resource(ResourceRef::new(
            "ssh-a",
            ResourceKind::Ssh,
            "prod-a",
        ));
        let catalog = vec![
            ResourceRef::new("ssh-a", ResourceKind::Ssh, "prod-a"),
            ResourceRef::new("ssh-b", ResourceKind::Ssh, "prod-b"),
        ];

        let items = resource_pool_items(&pool, &catalog);

        assert_eq!(2, items.len());
        assert_eq!(items[0].id.as_ref(), "ssh-a");
        assert!(items[0].in_pool);
        assert!(items[0].is_default);
        assert_eq!(items[1].id.as_ref(), "ssh-b");
        assert!(!items[1].in_pool);
        assert!(!items[1].is_default);
    }

    #[test]
    fn resource_pool_item_primary_meta_does_not_fallback_to_uuid() {
        let resource = ResourceRef::new(
            "fa9476d8-de90-4f7d-9b63-6f4783594211",
            ResourceKind::Other("rdp".into()),
            "a82 bi 服务",
        );

        assert_eq!(resource_primary_meta(&resource), "rdp");
    }

    #[test]
    fn resource_pool_item_primary_meta_skips_uuid_alias() {
        let resource = ResourceRef::new("rdp-a", ResourceKind::Other("rdp".into()), "a82 bi 服务")
            .with_alias("abfcee0a-2827-4588-9f6-587a7a95d1e9")
            .with_alias("10.1.131.181");

        assert_eq!(resource_primary_meta(&resource), "10.1.131.181");
    }

    #[test]
    fn resource_pool_item_uses_specific_icons_for_known_other_kinds() {
        assert_eq!(kind_icon(&ResourceKind::Other("rdp".into())), "RD");
        assert_eq!(kind_icon(&ResourceKind::Other("vnc".into())), "VN");
        assert_eq!(
            kind_icon(&ResourceKind::Other("port-forwarding".into())),
            "PF"
        );
    }

    #[test]
    fn resource_source_options_mark_all_when_pool_matches_catalog() {
        let pool = ResourceContext::new()
            .with_resource(ResourceRef::new("ssh-a", ResourceKind::Ssh, "prod-a"))
            .with_resource(ResourceRef::new("redis-a", ResourceKind::Redis, "cache"));
        let catalog = pool.resources.clone();

        let options = resource_source_options(&pool, &catalog);

        assert!(source_option(&options, "all").selected);
        assert_eq!(source_option(&options, "all").count, 2);
        assert!(!source_option(&options, "current").selected);
    }

    #[test]
    fn resource_source_options_mark_manual_for_mixed_subset() {
        let pool = ResourceContext::new()
            .with_resource(ResourceRef::new("ssh-a", ResourceKind::Ssh, "prod-a"))
            .with_resource(ResourceRef::new("redis-a", ResourceKind::Redis, "cache"));
        let catalog = vec![
            ResourceRef::new("ssh-a", ResourceKind::Ssh, "prod-a"),
            ResourceRef::new("ssh-b", ResourceKind::Ssh, "prod-b"),
            ResourceRef::new("redis-a", ResourceKind::Redis, "cache"),
        ];

        let options = resource_source_options(&pool, &catalog);

        assert!(source_option(&options, "manual").selected);
        assert_eq!(source_option(&options, "ssh").count, 2);
        assert_eq!(source_option(&options, "redis").count, 1);
    }

    fn source_option<'a>(
        options: &'a [ComposerResourceSourceOption],
        id: &str,
    ) -> &'a ComposerResourceSourceOption {
        options
            .iter()
            .find(|option| option.id.as_ref() == id)
            .unwrap()
    }

    #[test]
    fn apply_resource_source_all_replaces_pool_with_catalog() {
        let mut pool = ResourceContext::new().with_resource(ResourceRef::new(
            "ssh-a",
            ResourceKind::Ssh,
            "prod-a",
        ));
        let catalog = vec![
            ResourceRef::new("ssh-a", ResourceKind::Ssh, "prod-a"),
            ResourceRef::new("ssh-b", ResourceKind::Ssh, "prod-b"),
        ];

        assert!(apply_resource_source(&mut pool, &catalog, "all"));
        assert_eq!(2, pool.resources.len());
        assert_eq!(
            Some("prod-a"),
            pool.current().map(|resource| resource.label.as_str())
        );
    }

    #[test]
    fn apply_resource_source_ssh_selects_only_ssh_resources() {
        let mut pool = ResourceContext::new().with_resource(ResourceRef::new(
            "redis-a",
            ResourceKind::Redis,
            "cache",
        ));
        let catalog = vec![
            ResourceRef::new("ssh-a", ResourceKind::Ssh, "prod-a"),
            ResourceRef::new("redis-a", ResourceKind::Redis, "cache"),
        ];

        assert!(apply_resource_source(&mut pool, &catalog, "ssh"));
        assert_eq!(1, pool.resources.len());
        assert_eq!(
            Some("prod-a"),
            pool.current().map(|resource| resource.label.as_str())
        );
    }

    #[test]
    fn add_resource_to_pool_uses_catalog_resource() {
        let mut pool = ResourceContext::new().with_resource(ResourceRef::new(
            "ssh-a",
            ResourceKind::Ssh,
            "prod-a",
        ));
        let catalog = vec![
            ResourceRef::new("ssh-a", ResourceKind::Ssh, "prod-a"),
            ResourceRef::new("ssh-b", ResourceKind::Ssh, "prod-b"),
        ];

        assert!(add_resource_to_pool(&mut pool, &catalog, "ssh-b"));
        assert_eq!(2, pool.resources.len());
        assert_eq!(
            Some("prod-a"),
            pool.current().map(|resource| resource.label.as_str())
        );
    }

    #[test]
    fn mentioned_catalog_resources_are_added_to_pool_and_set_default() {
        let mut pool = ResourceContext::new().with_resource(ResourceRef::new(
            "db-a",
            ResourceKind::Mysql,
            "prod-db",
        ));
        let catalog = vec![
            ResourceRef::new("db-a", ResourceKind::Mysql, "prod-db"),
            ResourceRef::new("ssh-a", ResourceKind::Ssh, "prod-a"),
            ResourceRef::new("ssh-b", ResourceKind::Ssh, "prod-b"),
        ];
        let mentions = vec![
            MentionItem::new("ssh-a", "prod-a", "ssh", "ssh"),
            MentionItem::new("ssh-b", "prod-b", "ssh", "ssh"),
        ];

        assert!(apply_mentioned_resources(&mut pool, &catalog, &mentions));
        assert_eq!(3, pool.resources.len());
        assert_eq!(
            Some("prod-a"),
            pool.current().map(|resource| resource.label.as_str())
        );
        assert!(
            pool.resources
                .iter()
                .any(|resource| resource.label == "prod-b")
        );
    }

    #[test]
    fn remove_default_resource_reassigns_default_target() {
        let mut pool = ResourceContext::new()
            .with_resource(ResourceRef::new("ssh-a", ResourceKind::Ssh, "prod-a"))
            .with_resource(ResourceRef::new("ssh-b", ResourceKind::Ssh, "prod-b"));

        assert!(remove_resource_from_pool(&mut pool, "ssh-a"));
        assert_eq!(1, pool.resources.len());
        assert_eq!(
            Some("prod-b"),
            pool.current().map(|resource| resource.label.as_str())
        );
    }

    #[test]
    fn task_kind_round_trips() {
        assert_eq!(task_kind_from_id("agent"), Some(TaskKind::Agent));
        assert_eq!(task_kind_from_id("ask"), Some(TaskKind::Ask));
        assert_eq!(task_kind_from_id("plan"), Some(TaskKind::Plan));
        assert_eq!(task_kind_from_id("chat"), None);
        assert_eq!(task_kind_from_id("nope"), None);
    }

    #[test]
    fn tool_label_maps_to_runtime_execution_mode() {
        assert_eq!(
            ToolExecutionMode::Auto,
            tool_execution_mode_from_label(&SharedString::from("自动"))
        );
        assert_eq!(
            ToolExecutionMode::ReadOnly,
            tool_execution_mode_from_label(&SharedString::from("只读"))
        );
        assert_eq!(
            ToolExecutionMode::Manual,
            tool_execution_mode_from_label(&SharedString::from("手动确认"))
        );
    }

    #[test]
    fn default_tool_label_is_manual_confirmation() {
        assert_eq!("手动确认", default_tool_label().as_ref());
    }

    #[test]
    fn composer_defaults_to_auto_ask_plan_modes() {
        let options = default_task_options();

        assert_eq!(TaskKind::Agent, default_task_kind());
        assert_eq!(
            vec!["agent", "ask", "plan"],
            options
                .iter()
                .map(|option| option.id.as_ref())
                .collect::<Vec<_>>()
        );
        assert_eq!(options[0].label.as_ref(), "Auto Mode");
    }

    #[test]
    fn composer_context_includes_plan_items_for_local_and_acp_backends() {
        let plan = PlanCardData {
            goal: "上线检查".to_string(),
            status: "running".to_string(),
            steps: vec![crate::agent_cards::PlanStepData {
                title: "检查连接".to_string(),
                description: "确认服务可达".to_string(),
                status: "running".to_string(),
                risk: "只读".to_string(),
                tool: Some("ping".to_string()),
            }],
        };
        let acp_id = SharedString::from("codex");
        let acp_agents = vec![AcpAgentEntry::ready(AcpAgentConfig::new(
            acp_id.clone(),
            "Codex ACP",
            "codex",
        ))];

        let local = build_composer_context(
            &ResourceContext::new(),
            TaskKind::Agent,
            &SharedString::from("自动"),
            None,
            Some(&plan),
            &[],
            Backend::Local,
            &acp_agents,
            None,
            false,
            None,
            &[],
            ComposerSkillSummary::default(),
            Vec::new(),
        );
        let acp = build_composer_context(
            &ResourceContext::new(),
            TaskKind::Agent,
            &SharedString::from("自动"),
            None,
            Some(&plan),
            &[],
            Backend::Acp,
            &acp_agents,
            Some(&acp_id),
            false,
            None,
            &[],
            ComposerSkillSummary::default(),
            Vec::new(),
        );

        assert_eq!(local.plan_items, acp.plan_items);
        assert_eq!(local.plan_items[0].title.as_ref(), "检查连接");
        assert_eq!(local.plan_items[0].description.as_ref(), "确认服务可达");
        assert_eq!(local.plan_items[0].risk.as_ref(), "只读");
        assert_eq!(
            local.plan_items[0].tool.as_ref().map(|s| s.as_ref()),
            Some("ping")
        );
        assert!(local.agent_options[0].selected);
        assert!(acp.agent_options[1].selected);
    }

    #[test]
    fn local_backend_option_is_not_named_after_a_specific_cli() {
        let ctx = build_composer_context(
            &ResourceContext::new(),
            TaskKind::Agent,
            &SharedString::from("自动"),
            None,
            None,
            &[],
            Backend::Local,
            &[],
            None,
            false,
            None,
            &[],
            ComposerSkillSummary::default(),
            Vec::new(),
        );

        assert_eq!(ctx.agent_options[0].label.as_ref(), "One Agent");
    }

    #[test]
    fn composer_context_includes_running_subagents() {
        let subagents = vec![
            SubAgentCardData {
                subagent_id: "sub_1".into(),
                name: "reviewer".into(),
                task: "检查事件流".into(),
                running: true,
                success: None,
                summary: "正在读取事件".into(),
            },
            SubAgentCardData {
                subagent_id: "sub_2".into(),
                name: "done".into(),
                task: "已完成任务".into(),
                running: false,
                success: Some(true),
                summary: "完成".into(),
            },
        ];

        let ctx = build_composer_context(
            &ResourceContext::new(),
            TaskKind::Agent,
            &SharedString::from("自动"),
            None,
            None,
            &subagents,
            Backend::Local,
            &[],
            None,
            false,
            None,
            &[],
            ComposerSkillSummary::default(),
            Vec::new(),
        );

        assert_eq!(ctx.subagent_items.len(), 2);
        assert_eq!(ctx.subagent_items[0].name.as_ref(), "reviewer");
        assert_eq!(ctx.subagent_items[0].task.as_ref(), "检查事件流");
        assert_eq!(ctx.subagent_items[0].summary.as_ref(), "正在读取事件");
        assert_eq!(ctx.subagent_items[0].status.as_ref(), "running");
        assert_eq!(ctx.subagent_items[1].name.as_ref(), "done");
        assert_eq!(ctx.subagent_items[1].status.as_ref(), "completed");
    }

    #[test]
    fn header_agent_switcher_lists_and_labels_multiple_acp_agents() {
        let codex_id = SharedString::from("codex");
        let opencode_id = SharedString::from("opencode");
        let acp_agents = vec![
            AcpAgentEntry::ready(AcpAgentConfig::new(codex_id.clone(), "Codex", "codex")),
            AcpAgentEntry::ready(AcpAgentConfig::new(
                opencode_id.clone(),
                "OpenCode",
                "opencode",
            )),
        ];

        let options = composer_agent_options(Backend::Acp, &acp_agents, Some(&opencode_id), false);
        let labels = options
            .iter()
            .map(|option| option.label.as_ref())
            .collect::<Vec<_>>();

        assert_eq!(vec!["One Agent", "Codex", "OpenCode"], labels);
        assert!(options[2].selected);
        assert_eq!(
            "OpenCode",
            current_agent_label(Backend::Acp, &acp_agents, Some(&opencode_id), false).as_ref()
        );
    }

    #[test]
    fn invalid_acp_agent_remains_visible_but_disabled() {
        let diagnostic = AcpConfigDiagnostic::new("缺少环境变量 OPENAI_API_KEY");
        let entries = vec![AcpAgentEntry::invalid("codex", "Codex", diagnostic.clone())];

        let options = composer_agent_options(Backend::Local, &entries, None, false);

        assert_eq!(2, options.len());
        assert_eq!("Codex", options[1].label.as_ref());
        assert!(!options[1].enabled);
        assert_eq!(diagnostic.message, options[1].subtitle.as_ref());
        assert!(agent_option_disabled(&options[1]));
    }

    #[test]
    fn pending_acp_agent_is_treated_as_selected() {
        let selected = SharedString::from("codex");

        assert!(acp_options::agent_selection_is_active(
            Backend::Local,
            Some(&selected),
            true,
            &selected,
        ));
    }

    #[gpui::test]
    fn gpui_refresh_acp_agents_updates_header_switcher_options(cx: &mut TestAppContext) {
        init_test_ui(cx);
        let config = AgentChatViewConfig::new(test_runtime("m"), ResourceContext::new(), vec![])
            .with_acp_agents(vec![AcpAgentEntry::ready(AcpAgentConfig::new(
                "codex", "Codex", "codex",
            ))]);

        let (view, cx) =
            cx.add_window_view(move |window, cx| AgentChatView::new(config, window, cx));
        view.update(cx, |view, cx| {
            view.refresh_acp_agents_from(
                vec![
                    AcpAgentEntry::ready(AcpAgentConfig::new("codex", "Codex", "codex")),
                    AcpAgentEntry::ready(AcpAgentConfig::new("opencode", "OpenCode", "opencode")),
                ],
                cx,
            );
        });

        let labels = view.read_with(cx, |view, _| {
            composer_agent_options(
                view.backend,
                &view.acp_agents,
                view.current_acp_id.as_ref(),
                view.acp_connecting,
            )
            .iter()
            .map(|option| option.label.as_ref().to_string())
            .collect::<Vec<_>>()
        });

        assert_eq!(vec!["One Agent", "Codex", "OpenCode"], labels);
    }

    #[test]
    fn header_agent_switcher_keeps_local_available_while_acp_connects() {
        let acp_agents = vec![AcpAgentEntry::ready(AcpAgentConfig::new(
            "codex", "Codex", "codex",
        ))];
        let options = composer_agent_options(Backend::Local, &acp_agents, None, true);

        assert!(!agent_option_disabled(&options[0]));
        assert!(agent_option_disabled(&options[1]));
        assert_eq!(
            "连接中...",
            current_agent_label(Backend::Local, &acp_agents, None, true).as_ref()
        );
    }

    #[test]
    fn composer_context_maps_acp_state_to_visible_context() {
        use agent_client_protocol::schema::{
            AgentCapabilities, AvailableCommand, AvailableCommandsUpdate, CurrentModeUpdate,
            SessionInfoUpdate, SessionMode, SessionModeState, SessionUpdate, UsageUpdate,
        };

        let mut state = AcpSessionState::default();
        state.set_agent_capabilities(AgentCapabilities::new().load_session(true));
        state.apply_new_session_response(
            &agent_client_protocol::schema::NewSessionResponse::new("s1").modes(
                SessionModeState::new(
                    "ask",
                    vec![
                        SessionMode::new("ask", "Ask"),
                        SessionMode::new("code", "Code"),
                    ],
                ),
            ),
        );
        state.apply_session_update(&SessionUpdate::AvailableCommandsUpdate(
            AvailableCommandsUpdate::new(vec![AvailableCommand::new("plan", "Create plan")]),
        ));
        state.apply_session_update(&SessionUpdate::CurrentModeUpdate(CurrentModeUpdate::new(
            "code",
        )));
        state.apply_session_update(&SessionUpdate::SessionInfoUpdate(
            SessionInfoUpdate::new().title("ACP 工作会话"),
        ));
        state.apply_session_update(&SessionUpdate::UsageUpdate(UsageUpdate::new(42, 100)));

        let ctx = build_composer_context(
            &ResourceContext::new(),
            TaskKind::Ask,
            &SharedString::from("自动"),
            None,
            None,
            &[],
            Backend::Acp,
            &[],
            None,
            false,
            Some(state),
            &[],
            ComposerSkillSummary::default(),
            Vec::new(),
        );

        assert_eq!(ctx.target.unwrap().label.as_ref(), "ACP 工作会话");
        assert_eq!(ctx.scopes[0].value.as_ref(), "Code");
        assert_eq!(ctx.scopes[1].value.as_ref(), "42/100 tokens");
        assert!(ctx.capabilities.contains(&SharedString::from("ACP")));
        assert!(
            ctx.capabilities
                .contains(&SharedString::from("Load Session"))
        );
        assert!(ctx.capabilities.contains(&SharedString::from("命令:1")));
    }

    #[gpui::test]
    fn gpui_submit_ask_mode_does_not_pass_tools(cx: &mut TestAppContext) {
        init_test_ui(cx);
        let model = Arc::new(MockModelClient::new([ModelResponse::text("直接回答。")]));
        let runtime = test_runtime_with_model(model.clone());
        let config = AgentChatViewConfig::new(runtime, ResourceContext::new(), vec![]);

        let (view, cx) =
            cx.add_window_view(move |window, cx| AgentChatView::new(config, window, cx));
        view.update_in(cx, |view, window, cx| {
            view.select_task("ask", cx);
            let input = view.input.clone();
            view.on_input_event(
                &input,
                &AgentInputEvent::Submit {
                    text: "解释一下索引".into(),
                    mentions: Vec::new(),
                    images: Vec::new(),
                },
                window,
                cx,
            );
        });
        run_gpui_until(cx, || model.request_count() >= 1);

        let requests = model.received_requests();
        assert_eq!(1, requests.len());
        assert!(
            requests[0].tools.is_empty(),
            "Ask 模式完整 GPUI 提交链路不能向模型传 tools"
        );
        assert!(
            requests[0].tool_choice.is_none(),
            "Ask 模式完整 GPUI 提交链路不能向模型传 tool_choice"
        );
        assert!(
            !requests[0].messages[0]
                .content_as_text()
                .contains("update_plan")
        );
    }

    #[gpui::test]
    fn gpui_submit_readonly_tool_mode_filters_write_tools(cx: &mut TestAppContext) {
        init_test_ui(cx);
        let model = Arc::new(MockModelClient::new([ModelResponse::text("直接回答。")]));
        let runtime = test_runtime_with_model_and_write_tool(model.clone());
        let config = AgentChatViewConfig::new(runtime, ResourceContext::new(), vec![]);

        let (view, cx) =
            cx.add_window_view(move |window, cx| AgentChatView::new(config, window, cx));
        view.update_in(cx, |view, window, cx| {
            view.select_tool("readonly", cx);
            let input = view.input.clone();
            view.on_input_event(
                &input,
                &AgentInputEvent::Submit {
                    text: "只读分析".into(),
                    mentions: Vec::new(),
                    images: Vec::new(),
                },
                window,
                cx,
            );
        });
        run_gpui_until(cx, || model.request_count() >= 1);

        let requests = model.received_requests();
        let tool_names = requests[0]
            .tools
            .iter()
            .map(|tool| tool.function.name.as_str())
            .collect::<Vec<_>>();
        assert!(tool_names.contains(&"echo"));
        assert!(!tool_names.contains(&"write_data"));
    }

    #[gpui::test]
    fn gpui_tool_approval_click_is_not_blocked_by_running_flag(cx: &mut TestAppContext) {
        init_test_ui(cx);
        let model = Arc::new(MockModelClient::new([
            ModelResponse::tool_call(function_tool_call(
                "c_write",
                "write_data",
                json!({"value": "x"}).to_string(),
            )),
            ModelResponse::text("写入已完成。"),
        ]));
        let runtime = test_runtime_with_model_and_write_tool(model.clone());
        let config = AgentChatViewConfig::new(runtime, ResourceContext::new(), vec![]);

        let (view, cx) =
            cx.add_window_view(move |window, cx| AgentChatView::new(config, window, cx));
        view.update_in(cx, |view, window, cx| {
            let input = view.input.clone();
            view.on_input_event(
                &input,
                &AgentInputEvent::Submit {
                    text: "写入 x".into(),
                    mentions: Vec::new(),
                    images: Vec::new(),
                },
                window,
                cx,
            );
        });
        run_gpui_until(cx, || model.request_count() >= 1);
        cx.run_until_parked();

        view.update(cx, |view, cx| {
            view.is_running = true;
            view.resolve_tool_call("c_write".into(), true, cx);
        });
        run_gpui_until(cx, || model.request_count() >= 2);

        assert_eq!(2, model.request_count());
    }

    #[gpui::test]
    fn gpui_acp_permission_action_resolves_message_card_with_original_option_id(
        cx: &mut TestAppContext,
    ) {
        init_test_ui(cx);
        let config = AgentChatViewConfig::new(test_runtime("m"), ResourceContext::new(), vec![]);
        let (view, cx) =
            cx.add_window_view(move |window, cx| AgentChatView::new(config, window, cx));
        let (envelope, mut outcome_rx) = AcpPermissionEnvelope::new(test_acp_permission_request());

        view.update(cx, |view, cx| view.receive_acp_permission(envelope, cx));
        cx.dispatch_action(SelectAcpPermissionOption {
            request_id: "session:call".into(),
            option_id: "allow".into(),
        });
        cx.run_until_parked();

        assert_eq!(
            AcpPermissionOutcome::Selected {
                option_id: "allow".into(),
            },
            outcome_rx.try_recv().expect("ACP permission response")
        );
        let data = view.read_with(cx, |view, _| {
            let message = view
                .transcript
                .messages
                .iter()
                .find(|message| message.variant.card_kind() == Some(ACP_PERMISSION_CARD))
                .expect("ACP permission card");
            AcpPermissionCardData::from_json(&message.content).expect("card data")
        });
        assert_eq!("approved", data.status);
        assert_eq!("仅本次允许", data.selected_option_name);
    }

    #[gpui::test]
    fn gpui_acp_permission_button_resolves_without_opening_dialog(cx: &mut TestAppContext) {
        init_test_ui(cx);
        let granted = Arc::new(AtomicUsize::new(0));
        let revoked = Arc::new(AtomicUsize::new(0));
        cx.update({
            let granted = granted.clone();
            let revoked = revoked.clone();
            move |cx| {
                crate::set_acp_permission_grant_provider(cx, move |_request, option| {
                    if !option.kind.starts_with("allow") {
                        return None;
                    }
                    granted.fetch_add(1, Ordering::SeqCst);
                    let revoked = revoked.clone();
                    Some(crate::AcpPermissionGrant::new(move || {
                        revoked.fetch_add(1, Ordering::SeqCst);
                    }))
                });
            }
        });
        let config = AgentChatViewConfig::new(test_runtime("m"), ResourceContext::new(), vec![]);
        let (view, cx) =
            cx.add_window_view(move |window, cx| AgentChatView::new(config, window, cx));
        let (envelope, mut outcome_rx) = AcpPermissionEnvelope::new(test_acp_permission_request());

        view.update(cx, |view, cx| view.receive_acp_permission(envelope, cx));
        cx.run_until_parked();
        let allow = cx
            .debug_bounds("acp-permission-allow_once")
            .expect("ACP allow button should render in the message list");
        cx.simulate_click(allow.center(), Modifiers::default());
        cx.run_until_parked();

        assert_eq!(
            AcpPermissionOutcome::Selected {
                option_id: "allow".into(),
            },
            outcome_rx.try_recv().expect("ACP permission response")
        );
        assert_eq!(1, granted.load(Ordering::SeqCst));
        assert_eq!(0, revoked.load(Ordering::SeqCst));
    }

    #[gpui::test]
    fn gpui_failed_acp_permission_delivery_revokes_public_mcp_grant(cx: &mut TestAppContext) {
        init_test_ui(cx);
        let revoked = Arc::new(AtomicUsize::new(0));
        cx.update({
            let revoked = revoked.clone();
            move |cx| {
                crate::set_acp_permission_grant_provider(cx, move |_request, option| {
                    if !option.kind.starts_with("allow") {
                        return None;
                    }
                    let revoked = revoked.clone();
                    Some(crate::AcpPermissionGrant::new(move || {
                        revoked.fetch_add(1, Ordering::SeqCst);
                    }))
                });
            }
        });
        let config = AgentChatViewConfig::new(test_runtime("m"), ResourceContext::new(), vec![]);
        let (view, cx) =
            cx.add_window_view(move |window, cx| AgentChatView::new(config, window, cx));
        let (envelope, outcome_rx) = AcpPermissionEnvelope::new(test_acp_permission_request());
        drop(outcome_rx);

        view.update(cx, |view, cx| view.receive_acp_permission(envelope, cx));
        cx.run_until_parked();
        let allow = cx
            .debug_bounds("acp-permission-allow_once")
            .expect("ACP allow button should render in the message list");
        cx.simulate_click(allow.center(), Modifiers::default());
        cx.run_until_parked();

        assert_eq!(1, revoked.load(Ordering::SeqCst));
    }

    #[gpui::test]
    fn gpui_acp_permission_card_uses_full_width_details_and_compact_actions(
        cx: &mut TestAppContext,
    ) {
        init_test_ui(cx);
        let config = AgentChatViewConfig::new(test_runtime("m"), ResourceContext::new(), vec![])
            .sidebar_mode(true);
        let (view, cx) =
            cx.add_window_view(move |window, cx| AgentChatView::new(config, window, cx));
        view.update(cx, |view, cx| {
            view.transcript.messages.push(crate::ChatMessageUI::card(
                ACP_PERMISSION_CARD,
                AcpPermissionCardData {
                    request_id: "session:call-layout".into(),
                    session_id: "session".into(),
                    tool_call_id: "call-layout".into(),
                    tool_name: "ACP tool".into(),
                    summary: "ACP Agent 请求执行工具：ACP tool".into(),
                    details_json: r#"{
  "tool": "terminal.exec",
  "kind": "write",
  "scope": "session"
}"#
                    .into(),
                    options: vec![
                        AcpPermissionOptionData {
                            option_id: "allow".into(),
                            name: "Allow".into(),
                            kind: "allow_once".into(),
                        },
                        AcpPermissionOptionData {
                            option_id: "allow-session".into(),
                            name: "Allow for This Session".into(),
                            kind: "allow_for_session".into(),
                        },
                        AcpPermissionOptionData {
                            option_id: "allow-always".into(),
                            name: "Allow and Don't Ask Again".into(),
                            kind: "allow_always".into(),
                        },
                        AcpPermissionOptionData {
                            option_id: "decline".into(),
                            name: "Decline".into(),
                            kind: "reject_once".into(),
                        },
                    ],
                    status: "pending".into(),
                    selected_option_name: String::new(),
                }
                .to_json(),
            ));
            cx.notify();
        });
        let cx: &mut VisualTestContext = cx;

        let column = cx
            .debug_bounds("ai-chat-message-column")
            .expect("message column should render");
        let details = cx
            .debug_bounds("acp-permission-details")
            .expect("ACP details should render");
        let frame = cx
            .debug_bounds("agent-tool-json-frame")
            .expect("ACP details frame should render");
        let input = cx
            .debug_bounds("agent-tool-json-input-slot")
            .expect("ACP details input should render");
        for (name, bounds) in [("details", details), ("frame", frame), ("input", input)] {
            assert!(
                bounds.size.width > column.size.width * 0.75,
                "ACP {name} should use the available message width: column={column:?}, bounds={bounds:?}"
            );
        }

        let actions = cx
            .debug_bounds("acp-permission-actions")
            .expect("ACP actions should render");
        let allow = cx
            .debug_bounds("acp-permission-allow_once")
            .expect("allow button should render");
        let reject = cx
            .debug_bounds("acp-permission-reject_once")
            .expect("reject button should render");
        let more = cx
            .debug_bounds("acp-permission-more-options")
            .expect("more-options trigger should render");
        assert_eq!(allow.origin.y, reject.origin.y);
        assert_eq!(allow.origin.y, more.origin.y);
        assert!(actions.size.height <= allow.size.height + px(4.0));
    }

    #[gpui::test]
    fn gpui_resetting_acp_connection_cancels_pending_permission_card(cx: &mut TestAppContext) {
        init_test_ui(cx);
        let config = AgentChatViewConfig::new(test_runtime("m"), ResourceContext::new(), vec![]);
        let (view, cx) =
            cx.add_window_view(move |window, cx| AgentChatView::new(config, window, cx));
        let (envelope, mut outcome_rx) = AcpPermissionEnvelope::new(test_acp_permission_request());

        view.update(cx, |view, cx| {
            view.receive_acp_permission(envelope, cx);
            view.reset_acp_permission_session(cx);
        });

        assert_eq!(
            AcpPermissionOutcome::Cancelled,
            outcome_rx.try_recv().expect("cancelled ACP permission")
        );
        let data = view.read_with(cx, |view, _| {
            let message = view
                .transcript
                .messages
                .iter()
                .find(|message| message.variant.card_kind() == Some(ACP_PERMISSION_CARD))
                .expect("ACP permission card");
            AcpPermissionCardData::from_json(&message.content).expect("card data")
        });
        assert_eq!("cancelled", data.status);
        assert!(data.selected_option_name.is_empty());
    }

    #[gpui::test]
    fn gpui_tool_approval_action_dispatch_submits_approval(cx: &mut TestAppContext) {
        init_test_ui(cx);
        let model = Arc::new(MockModelClient::new([
            ModelResponse::tool_call(function_tool_call(
                "c_write",
                "write_data",
                json!({"value": "x"}).to_string(),
            )),
            ModelResponse::text("写入已完成。"),
        ]));
        let runtime = test_runtime_with_model_and_write_tool(model.clone());
        let config = AgentChatViewConfig::new(runtime, ResourceContext::new(), vec![]);

        let (view, cx) =
            cx.add_window_view(move |window, cx| AgentChatView::new(config, window, cx));
        view.update_in(cx, |view, window, cx| {
            let input = view.input.clone();
            view.on_input_event(
                &input,
                &AgentInputEvent::Submit {
                    text: "写入 x".into(),
                    mentions: Vec::new(),
                    images: Vec::new(),
                },
                window,
                cx,
            );
        });
        run_gpui_until(cx, || model.request_count() >= 1);
        cx.run_until_parked();

        cx.dispatch_action(ApproveToolCall {
            call_id: "c_write".into(),
        });
        run_gpui_until(cx, || model.request_count() >= 2);

        assert_eq!(2, model.request_count());
    }

    #[gpui::test]
    fn gpui_tool_approval_button_click_submits_approval(cx: &mut TestAppContext) {
        init_test_ui(cx);
        let model = Arc::new(MockModelClient::new([
            ModelResponse::tool_call(function_tool_call(
                "c_write",
                "write_data",
                json!({"value": "x"}).to_string(),
            )),
            ModelResponse::text("写入已完成。"),
        ]));
        let runtime = test_runtime_with_model_and_write_tool(model.clone());
        let config = AgentChatViewConfig::new(runtime, ResourceContext::new(), vec![]);

        let (view, cx) =
            cx.add_window_view(move |window, cx| AgentChatView::new(config, window, cx));
        view.update_in(cx, |view, window, cx| {
            let input = view.input.clone();
            view.on_input_event(
                &input,
                &AgentInputEvent::Submit {
                    text: "写入 x".into(),
                    mentions: Vec::new(),
                    images: Vec::new(),
                },
                window,
                cx,
            );
        });
        run_gpui_until(cx, || model.request_count() >= 1);
        cx.run_until_parked();

        let approve = cx
            .debug_bounds("agent-tool-approve")
            .expect("approval button should render");
        cx.simulate_click(approve.center(), Modifiers::default());
        run_gpui_until(cx, || model.request_count() >= 2);

        assert_eq!(2, model.request_count());
    }

    #[gpui::test]
    fn gpui_tool_approval_button_click_submits_after_scrolling(cx: &mut TestAppContext) {
        init_test_ui(cx);
        let model = Arc::new(MockModelClient::new([
            ModelResponse::tool_call(function_tool_call(
                "c_write",
                "write_data",
                json!({"value": "x"}).to_string(),
            )),
            ModelResponse::text("写入已完成。"),
        ]));
        let runtime = test_runtime_with_model_and_write_tool(model.clone());
        let config =
            AgentChatViewConfig::new(runtime, ResourceContext::new(), vec![]).sidebar_mode(true);

        let (view, cx) =
            cx.add_window_view(move |window, cx| AgentChatView::new(config, window, cx));
        view.update_in(cx, |view, window, cx| {
            for index in 0..16 {
                view.transcript.push_system(format!(
                    "滚动前置消息 {index}: 用于让确认卡进入可滚动区域。"
                ));
            }
            let input = view.input.clone();
            view.on_input_event(
                &input,
                &AgentInputEvent::Submit {
                    text: "写入 x".into(),
                    mentions: Vec::new(),
                    images: Vec::new(),
                },
                window,
                cx,
            );
        });
        run_gpui_until(cx, || model.request_count() >= 1);
        cx.run_until_parked();

        let approve_before_scroll = cx
            .debug_bounds("agent-tool-approve")
            .expect("approval button should render before scrolling");
        cx.simulate_event(ScrollWheelEvent {
            position: approve_before_scroll.center(),
            delta: ScrollDelta::Pixels(point(px(0.0), px(-280.0))),
            modifiers: Modifiers::default(),
            touch_phase: TouchPhase::Moved,
        });
        cx.run_until_parked();

        let approve = cx
            .debug_bounds("agent-tool-approve")
            .expect("approval button should render after scrolling");
        cx.simulate_click(approve.center(), Modifiers::default());
        run_gpui_until(cx, || model.request_count() >= 2);

        assert_eq!(2, model.request_count());
    }

    #[gpui::test]
    fn gpui_system_instruction_is_sent_to_runtime_prompt(cx: &mut TestAppContext) {
        init_test_ui(cx);
        let model = Arc::new(MockModelClient::new([ModelResponse::text("直接回答。")]));
        let runtime = test_runtime_with_model(model.clone());
        let config = AgentChatViewConfig::new(runtime, ResourceContext::new(), vec![]);

        let (view, cx) =
            cx.add_window_view(move |window, cx| AgentChatView::new(config, window, cx));
        view.update_in(cx, |view, window, cx| {
            view.set_system_instruction(Some("始终用 DBA 视角回答。".into()), cx);
            let input = view.input.clone();
            view.on_input_event(
                &input,
                &AgentInputEvent::Submit {
                    text: "解释一下索引".into(),
                    mentions: Vec::new(),
                    images: Vec::new(),
                },
                window,
                cx,
            );
        });
        run_gpui_until(cx, || model.request_count() >= 1);

        let requests = model.received_requests();
        assert_eq!(1, requests.len());
        assert!(
            requests[0].messages[0]
                .content_as_text()
                .contains("始终用 DBA 视角回答。")
        );
    }

    #[gpui::test]
    fn gpui_system_instruction_survives_new_local_session(cx: &mut TestAppContext) {
        init_test_ui(cx);
        let runtime = test_runtime("m");
        let config = AgentChatViewConfig::new(runtime.clone(), ResourceContext::new(), vec![]);

        let (view, cx) =
            cx.add_window_view(move |window, cx| AgentChatView::new(config, window, cx));
        let session_id = view.update(cx, |view, cx| {
            view.set_system_instruction(Some("只输出 SQL 审计建议。".into()), cx);
            view.start_fresh_session(cx);
            view.session_id.clone()
        });

        let session = runtime.session(&session_id).expect("session should exist");
        assert_eq!(
            session.system_instruction().as_deref(),
            Some("只输出 SQL 审计建议。")
        );
    }

    #[gpui::test]
    fn gpui_system_instruction_survives_model_switch(cx: &mut TestAppContext) {
        init_test_ui(cx);
        let first = ComposerModelOption::new("openai:gpt-a", "openai", "OpenAI", "gpt-a");
        let second = ComposerModelOption::new("ollama:qwen", "ollama", "Ollama", "qwen3:14b");
        let runtimes = Arc::new(std::sync::Mutex::new(Vec::<Arc<Runtime>>::new()));
        let factory_runtimes = runtimes.clone();
        let factory: AgentRuntimeFactory = Arc::new(move |option| {
            let runtime = test_runtime(option.model.as_ref());
            factory_runtimes.lock().unwrap().push(runtime.clone());
            runtime
        });
        let initial_runtime = test_runtime("gpt-a");
        let config = AgentChatViewConfig::new(initial_runtime, ResourceContext::new(), vec![])
            .with_models(
                vec![first, second],
                Some(SharedString::from("openai:gpt-a")),
                factory,
            );

        let (view, cx) =
            cx.add_window_view(move |window, cx| AgentChatView::new(config, window, cx));
        view.update(cx, |view, cx| {
            view.set_system_instruction(Some("只输出 SQL 审计建议。".into()), cx);
            view.select_model("ollama:qwen", "ollama", "qwen3:14b", cx);
        });

        let runtime = runtimes.lock().unwrap().last().cloned().unwrap();
        let session_id = view.read_with(cx, |view, _| view.session_id.clone());
        let session = runtime.session(&session_id).expect("session should exist");
        assert_eq!(
            session.system_instruction().as_deref(),
            Some("只输出 SQL 审计建议。")
        );
    }

    #[gpui::test]
    fn gpui_submit_agent_recovers_from_pseudo_tool_call(cx: &mut TestAppContext) {
        init_test_ui(cx);
        let model = Arc::new(MockModelClient::new([
            ModelResponse::tool_call(function_tool_call("c_bad", "tool", "db.schema")),
            ModelResponse::tool_call(function_tool_call(
                "c_plan",
                "update_plan",
                json!({
                    "plan": [
                        {"step": "创建计划清单", "status": "completed"},
                        {"step": "给出总结", "status": "in_progress"}
                    ]
                })
                .to_string(),
            )),
            ModelResponse::text("已创建计划清单。"),
        ]));
        let runtime = test_runtime_with_model(model.clone());
        let config = AgentChatViewConfig::new(runtime.clone(), ResourceContext::new(), vec![]);

        let (view, cx) =
            cx.add_window_view(move |window, cx| AgentChatView::new(config, window, cx));
        let session_id = view.read_with(cx, |view, _| view.session_id.clone());
        view.update_in(cx, |view, window, cx| {
            let input = view.input.clone();
            view.on_input_event(
                &input,
                &AgentInputEvent::Submit {
                    text: "先创建一个包含几个步骤的计划清单。".into(),
                    mentions: Vec::new(),
                    images: Vec::new(),
                },
                window,
                cx,
            );
        });
        run_gpui_until(cx, || model.request_count() >= 3);

        assert_eq!(3, model.request_count());
        let session = runtime.session(&session_id).expect("session should exist");
        let history = session.history_snapshot();
        assert!(history.items().iter().any(|item| {
            matches!(
                item,
                agent_runtime::HistoryItem::Observation(observation)
                    if !observation.success && observation.tool_name.as_str() == "tool"
            )
        }));
        assert!(
            session.current_plan().is_some(),
            "伪工具调用纠偏后应继续完成 update_plan"
        );
    }

    #[test]
    fn config_defaults_to_full_view_and_builder_enables_sidebar() {
        let config = AgentChatViewConfig::new(test_runtime("m"), ResourceContext::new(), vec![]);
        assert!(!config.sidebar_mode, "默认应为全宽视图");
        assert!(
            config.sidebar_mode(true).sidebar_mode,
            "builder 应开启侧边栏视图"
        );
    }

    #[test]
    fn sidebar_header_visibility_can_be_disabled_for_framed_hosts() {
        let config = AgentChatViewConfig::new(test_runtime("m"), ResourceContext::new(), vec![])
            .sidebar_mode(true);
        assert!(config.show_sidebar_header);
        assert!(!config.show_sidebar_frame_controls);

        let embedded = config.show_sidebar_header(false);
        assert!(!embedded.show_sidebar_header);
    }

    #[test]
    fn sidebar_mode_header_actions_include_close() {
        assert_eq!(
            vec!["new", "history", "close"],
            sidebar_mode_header_action_ids(false)
        );
    }

    #[test]
    fn sidebar_mode_header_actions_can_include_frame_options() {
        let config = AgentChatViewConfig::new(test_runtime("m"), ResourceContext::new(), vec![])
            .sidebar_mode(true)
            .show_sidebar_frame_controls(true, SidebarPlacement::Bottom);

        assert!(config.show_sidebar_frame_controls);
        assert_eq!(SidebarPlacement::Bottom, config.sidebar_frame_placement);
        assert_eq!(
            vec!["new", "history", "frame-options", "close"],
            sidebar_mode_header_action_ids(config.show_sidebar_frame_controls)
        );
    }

    #[test]
    fn agent_history_labels_use_task_language() {
        assert_eq!("历史任务", agent_history_title(false));
        assert_eq!("已归档任务", agent_history_title(true));
        assert_eq!("当前 Agent 任务", current_agent_task_title());
    }

    #[gpui::test]
    fn sidebar_mode_input_is_edge_to_edge(cx: &mut TestAppContext) {
        init_test_ui(cx);
        let config = AgentChatViewConfig::new(test_runtime("m"), ResourceContext::new(), vec![])
            .sidebar_mode(true);
        let (_, cx) = cx.add_window_view(move |window, cx| AgentChatView::new(config, window, cx));
        let cx: &mut VisualTestContext = cx;

        let area = cx
            .debug_bounds("agent-input-area")
            .expect("input area should render");
        let input = cx
            .debug_bounds("agent-input-root")
            .expect("input root should render");

        assert_eq!(
            area.size.width, input.size.width,
            "sidebar input should fill the bottom area: area={area:?}, input={input:?}"
        );
        assert_eq!(
            area.origin.x, input.origin.x,
            "sidebar input should not be inset: area={area:?}, input={input:?}"
        );
    }

    #[gpui::test]
    fn sidebar_mode_user_message_row_fills_message_column(cx: &mut TestAppContext) {
        init_test_ui(cx);
        let config = AgentChatViewConfig::new(test_runtime("m"), ResourceContext::new(), vec![])
            .sidebar_mode(true);
        let (view, cx) =
            cx.add_window_view(move |window, cx| AgentChatView::new(config, window, cx));
        view.update(cx, |view, cx| {
            view.transcript
                .messages
                .push(crate::ChatMessageUI::user("帮我看看内存占用"));
            cx.notify();
        });
        let cx: &mut VisualTestContext = cx;

        let column = cx
            .debug_bounds("ai-chat-message-column")
            .expect("message column should render");
        let scroll = cx
            .debug_bounds("ai-chat-messages-scroll")
            .expect("message scroll area should render");
        let user_row = cx
            .debug_bounds("ai-chat-user-row")
            .expect("user row should render");

        let expected_column_width = scroll.size.width - px(32.0);
        assert_eq!(
            expected_column_width, column.size.width,
            "sidebar message column should fill the padded scroll area: scroll={scroll:?}, column={column:?}"
        );
        assert_eq!(
            column.size.width, user_row.size.width,
            "user message row should fill the message column: column={column:?}, row={user_row:?}"
        );
        assert_eq!(
            column.origin.x, user_row.origin.x,
            "user message row should not drift horizontally: column={column:?}, row={user_row:?}"
        );
    }

    #[gpui::test]
    fn sidebar_mode_long_user_message_bubble_uses_available_column(cx: &mut TestAppContext) {
        init_test_ui(cx);
        let (host, cx) = cx.add_window_view(FixedSidebarHost::new);
        let chat = host.read_with(cx, |host, _| host.view.clone());
        chat.update(cx, |view, cx| {
            view.transcript.messages.push(crate::ChatMessageUI::user(
                "帮我看看这台服务器当前还有多少内存，并且顺便判断一下是否需要扩容或者清理缓存",
            ));
            cx.notify();
        });
        let cx: &mut VisualTestContext = cx;

        let column = cx
            .debug_bounds("ai-chat-message-column")
            .expect("message column should render");
        let bubble = cx
            .debug_bounds("ai-chat-user-bubble")
            .expect("user bubble should render");

        assert!(
            bubble.size.width > column.size.width * 0.7,
            "long user bubble should use the available sidebar column width: column={column:?}, bubble={bubble:?}"
        );
    }

    #[gpui::test]
    fn sidebar_mode_fills_fixed_host_frame(cx: &mut TestAppContext) {
        init_test_ui(cx);
        let (host, cx) = cx.add_window_view(FixedSidebarHost::new);
        let chat = host.read_with(cx, |host, _| host.view.clone());
        chat.update(cx, |view, cx| {
            view.transcript
                .messages
                .push(crate::ChatMessageUI::user("帮我检查终端侧边栏布局"));
            cx.notify();
        });
        let cx: &mut VisualTestContext = cx;

        let slot = cx
            .debug_bounds("fixed-sidebar-content-slot")
            .expect("fixed sidebar content slot should render");
        let root = cx
            .debug_bounds("agent-sidebar-root")
            .expect("sidebar root should render");
        let stack = cx
            .debug_bounds("agent-sidebar-stack")
            .expect("sidebar stack should render");
        let messages = cx
            .debug_bounds("ai-chat-messages")
            .expect("messages area should render");
        let input_area = cx
            .debug_bounds("agent-input-area")
            .expect("input area should render");
        let input = cx
            .debug_bounds("agent-input-root")
            .expect("input root should render");

        assert_eq!(slot.origin.x, root.origin.x);
        assert_eq!(slot.size.width, root.size.width);
        assert_eq!(root.origin.x, stack.origin.x);
        assert_eq!(root.size.width, stack.size.width);
        assert_eq!(root.origin.x, messages.origin.x);
        assert_eq!(root.size.width, messages.size.width);
        assert_eq!(root.origin.x, input_area.origin.x);
        assert_eq!(root.size.width, input_area.size.width);
        assert_eq!(input_area.origin.x, input.origin.x);
        assert_eq!(input_area.size.width, input.size.width);
    }

    #[gpui::test]
    fn sidebar_mode_tool_card_fills_message_column(cx: &mut TestAppContext) {
        init_test_ui(cx);
        let (host, cx) = cx.add_window_view(FixedSidebarHost::new);
        let chat = host.read_with(cx, |host, _| host.view.clone());
        chat.update(cx, |view, cx| {
            view.transcript.messages.push(crate::ChatMessageUI::card(
                TOOL_CARD,
                ToolCardData {
                    call_id: "call-layout".to_string(),
                    tool_name: "terminal.exec".to_string(),
                    target_id: Some("ssh-prod-with-a-very-long-target-id".to_string()),
                    target_label: Some("生产终端节点-很长的展示名称".to_string()),
                    input_summary: "ps aux | sort -nrk 3,3 | head -20".to_string(),
                    input_json: r#"{"command":"ps aux | sort -nrk 3,3 | head -20"}"#.to_string(),
                    running: true,
                    success: None,
                    summary: String::new(),
                    data_text: String::new(),
                }
                .to_json(),
            ));
            cx.notify();
        });
        let cx: &mut VisualTestContext = cx;

        let column = cx
            .debug_bounds("ai-chat-message-column")
            .expect("message column should render");
        let card = cx
            .debug_bounds("agent-tool-card")
            .expect("tool card should render");

        assert_eq!(column.origin.x, card.origin.x);
        assert_eq!(
            column.size.width, card.size.width,
            "tool card should fill sidebar message column: column={column:?}, card={card:?}"
        );
    }

    #[gpui::test]
    fn sidebar_mode_tool_confirm_actions_align_to_message_column(cx: &mut TestAppContext) {
        init_test_ui(cx);
        let (host, cx) = cx.add_window_view(FixedSidebarHost::new);
        let chat = host.read_with(cx, |host, _| host.view.clone());
        chat.update(cx, |view, cx| {
            view.transcript.messages.push(crate::ChatMessageUI::card(
                TOOL_CONFIRM_CARD,
                ToolConfirmCardData {
                    call_id: "call-confirm-layout".to_string(),
                    tool_name: "terminal_exec".to_string(),
                    items: Vec::new(),
                    input_summary: "free -h".to_string(),
                    input_json: r#"{"command":"free -h"}"#.to_string(),
                    question: "确认执行工具 terminal_exec 吗？".to_string(),
                    status: "pending".to_string(),
                }
                .to_json(),
            ));
            cx.notify();
        });
        let cx: &mut VisualTestContext = cx;

        let column = cx
            .debug_bounds("ai-chat-message-column")
            .expect("message column should render");
        let approve = cx
            .debug_bounds("agent-tool-approve")
            .expect("approval button should render");

        assert!(
            approve.right() > column.right() - px(96.0),
            "approval button should align near the message column right edge: column={column:?}, approve={approve:?}"
        );
    }

    #[gpui::test]
    fn sidebar_mode_tool_confirm_json_block_uses_available_column(cx: &mut TestAppContext) {
        init_test_ui(cx);
        let (host, cx) = cx.add_window_view(FixedSidebarHost::new);
        let chat = host.read_with(cx, |host, _| host.view.clone());
        chat.update(cx, |view, cx| {
            view.transcript.messages.push(crate::ChatMessageUI::card(
                TOOL_CONFIRM_CARD,
                ToolConfirmCardData {
                    call_id: "call-confirm-json-layout".to_string(),
                    tool_name: "terminal_exec".to_string(),
                    items: Vec::new(),
                    input_summary: "free -h".to_string(),
                    input_json: r#"{
  "target": "ssh-prod",
  "command": "free -h",
  "subprocess": true
}"#
                    .to_string(),
                    question: "确认执行工具 terminal_exec 吗？".to_string(),
                    status: "pending".to_string(),
                }
                .to_json(),
            ));
            cx.notify();
        });
        let cx: &mut VisualTestContext = cx;

        let column = cx
            .debug_bounds("ai-chat-message-column")
            .expect("message column should render");
        let json = cx
            .debug_bounds("agent-tool-json-block")
            .expect("tool json block should render");
        let frame = cx
            .debug_bounds("agent-tool-json-frame")
            .expect("tool json frame should render");
        let input = cx
            .debug_bounds("agent-tool-json-input-slot")
            .expect("tool json input slot should render");

        assert!(
            json.size.width > column.size.width * 0.75,
            "tool confirm json block should use the available sidebar column width: column={column:?}, json={json:?}"
        );
        assert!(
            frame.size.width > column.size.width * 0.75,
            "tool confirm json frame should use the available sidebar column width: column={column:?}, frame={frame:?}"
        );
        assert!(frame.right() <= json.right());
        assert!(
            input.size.width > column.size.width * 0.75,
            "tool confirm json input should use the available sidebar column width: column={column:?}, input={input:?}"
        );
    }

    #[test]
    fn runtime_binding_switches_runtime_from_structured_model_option() {
        let first = ComposerModelOption::new("openai:gpt-a", "openai", "OpenAI", "gpt-a");
        let second = ComposerModelOption::new("ollama:qwen", "ollama", "Ollama", "qwen3:14b");
        let calls = Arc::new(AtomicUsize::new(0));
        let factory_calls = calls.clone();
        let factory: AgentRuntimeFactory = Arc::new(move |option| {
            factory_calls.fetch_add(1, Ordering::SeqCst);
            test_runtime(option.model.as_ref())
        });

        let resources = ResourceContext::new();
        let initial_runtime = test_runtime(first.model.as_ref());
        let mut binding = RuntimeBinding::new(
            initial_runtime,
            resources.clone(),
            Some(first),
            Some(factory),
        );
        let old_session = binding.session_id.clone();

        assert!(binding.switch_model(&second, &resources));
        assert_ne!(binding.session_id, old_session);
        assert_eq!(binding.runtime.services().model.model_name(), "qwen3:14b");
        assert_eq!(
            binding
                .selected_model
                .as_ref()
                .unwrap()
                .provider_id
                .as_ref(),
            "ollama"
        );
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn provider_config_models_expand_to_structured_options() {
        let config = ProviderConfig {
            id: 7,
            name: "Local Ollama".to_string(),
            provider_type: ProviderType::Ollama,
            model: "qwen3:14b".to_string(),
            models: vec!["qwen3:14b".to_string(), "llama3.1".to_string()],
            is_default: true,
            ..Default::default()
        };

        let specs = runtime_specs_from_provider_configs(vec![config], ToolRegistry::new())
            .expect("ollama provider config should build without network");

        assert_eq!(specs.len(), 2);
        assert_eq!(specs[0].option.provider_id.as_ref(), "7");
        assert_eq!(specs[0].option.provider_label.as_ref(), "Local Ollama");
        assert_eq!(specs[0].option.model.as_ref(), "qwen3:14b");
        assert_eq!(specs[1].option.model.as_ref(), "llama3.1");
        assert_eq!(
            selected_provider_model_id(&specs),
            Some(SharedString::from("provider:7:qwen3:14b"))
        );
    }

    #[test]
    fn provider_config_initial_runtime_uses_default_model() {
        let first = ProviderConfig {
            id: 7,
            name: "First".to_string(),
            provider_type: ProviderType::Ollama,
            model: "first-model".to_string(),
            is_default: false,
            ..Default::default()
        };
        let second = ProviderConfig {
            id: 8,
            name: "Default".to_string(),
            provider_type: ProviderType::Ollama,
            model: "default-model".to_string(),
            is_default: true,
            ..Default::default()
        };

        let config = AgentChatViewConfig::from_provider_configs(
            ResourceContext::new(),
            vec![],
            vec![first, second],
            ToolRegistry::new(),
        )
        .expect("provider configs should build");

        assert_eq!(
            config.selected_model_id,
            Some(SharedString::from("provider:8:default-model"))
        );
        assert_eq!(
            config.runtime.services().model.model_name(),
            "default-model"
        );
    }

    #[test]
    fn provider_config_uses_type_label_when_name_is_empty() {
        let config = ProviderConfig {
            id: 8,
            provider_type: ProviderType::Ollama,
            model: "mistral".to_string(),
            ..Default::default()
        };

        let specs = runtime_specs_from_provider_configs(vec![config], ToolRegistry::new())
            .expect("ollama provider config should build without network");

        assert_eq!(specs.len(), 1);
        assert_eq!(specs[0].option.provider_label.as_ref(), "Ollama");
    }

    #[test]
    fn refreshed_models_keep_current_selection_when_it_still_exists() {
        let current = ComposerModelOption::new("p:old", "p", "Provider", "old");
        let added = ComposerModelOption::new("p:new", "p", "Provider", "new");
        let previous_id = current.id.clone();
        let default_id = added.id.clone();

        let (selected, retained) =
            refreshed_model_selection(Some(&previous_id), Some(&default_id), &[current, added]);

        assert_eq!(
            selected.as_ref().map(|model| model.id.as_ref()),
            Some("p:old")
        );
        assert!(retained.is_some());
    }

    #[test]
    fn refreshed_models_fall_back_when_current_selection_was_removed() {
        let fallback = ComposerModelOption::new("p:new", "p", "Provider", "new");
        let removed_id = SharedString::from("p:removed");
        let default_id = fallback.id.clone();

        let (selected, retained) =
            refreshed_model_selection(Some(&removed_id), Some(&default_id), &[fallback]);

        assert_eq!(
            selected.as_ref().map(|model| model.id.as_ref()),
            Some("p:new")
        );
        assert!(retained.is_none());
    }

    fn test_runtime(model_name: &str) -> Arc<Runtime> {
        let model = Arc::new(NamedModelClient(model_name.to_string()));
        let tools = Arc::new(ToolRouter::new(ToolRegistry::new()));
        Arc::new(Runtime::new(RuntimeServices::new(model, tools)))
    }

    fn test_runtime_with_model(model: Arc<MockModelClient>) -> Arc<Runtime> {
        let tools = Arc::new(ToolRouter::new(
            ToolRegistry::new().with_tool(Arc::new(EchoTool)),
        ));
        Arc::new(Runtime::new(RuntimeServices::new(model, tools)))
    }

    fn test_runtime_with_model_and_write_tool(model: Arc<MockModelClient>) -> Arc<Runtime> {
        let tools = Arc::new(ToolRouter::new(
            ToolRegistry::new()
                .with_tool(Arc::new(EchoTool))
                .with_tool(Arc::new(WriteTool)),
        ));
        Arc::new(Runtime::new(RuntimeServices::new(model, tools)))
    }

    fn init_test_ui(cx: &mut TestAppContext) {
        cx.update(|cx| {
            gpui_component::init(cx);
            crate::init(cx);
        });
    }

    fn run_gpui_until(cx: &mut VisualTestContext, condition: impl Fn() -> bool) {
        for _ in 0..20 {
            if condition() {
                return;
            }
            cx.run_until_parked();
        }
        assert!(condition(), "GPUI test condition was not reached");
    }

    struct NamedModelClient(String);

    #[async_trait]
    impl ModelClient for NamedModelClient {
        async fn complete(
            &self,
            _request: ModelRequest,
        ) -> Result<ModelResponse, agent_runtime::RuntimeError> {
            Ok(ModelResponse::text("ok"))
        }

        async fn complete_stream(
            &self,
            _request: ModelRequest,
        ) -> Result<ModelStream, agent_runtime::RuntimeError> {
            Ok(Box::pin(futures::stream::empty()))
        }

        fn model_name(&self) -> &str {
            &self.0
        }
    }
}
