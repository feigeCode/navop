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
    ResourceContext, ResourceId, ResourceKind, ResourceRef, Runtime, RuntimeEvent,
    RuntimeEventReceiver, SessionId, TaskKind, ToolRegistry, UserInput,
};
use gpui::prelude::FluentBuilder;
use gpui::{
    Anchor, App, AppContext, Context, Entity, EventEmitter, FontWeight, InteractiveElement,
    IntoElement, ParentElement, Render, ScrollHandle, SharedString, StatefulInteractiveElement,
    Styled, Subscription, Task, Window, div, px,
};
use gpui_component::{
    ActiveTheme, IconName, Selectable, Sizable, WindowExt as _,
    button::{Button, ButtonVariants},
    dialog::DialogButtonProps,
    h_flex,
    input::{Input, InputState},
    popover::Popover,
    v_flex,
};
#[cfg(not(test))]
use one_core::gpui_tokio::Tokio;
use one_core::llm::{GlobalProviderState, LlmConnector, LlmProvider, ProviderConfig};
use tokio::sync::broadcast::error::RecvError;

use crate::acp::{AcpAgentConfig, AcpConnection, AcpSessionState};
use crate::agent_cards::PlanCardData;
use crate::agent_transcript::AgentTranscript;
use crate::bridge::build_runtime_from_llm_provider;
use crate::code_block::{CodeBlockAction, CodeBlockActionRegistry};
use crate::input::{
    AgentComposerContext, AgentInput, AgentInputEvent, ComposerAgentOption, ComposerMenuOption,
    ComposerModelOption, ComposerPlanItem, ComposerScope, ComposerTarget, MentionItem,
};
use crate::message_view::render_messages_with_code_actions;
use crate::persistence;
use crate::session_sidebar::{self, SessionSummary};

/// Agent 聊天视图事件。
#[derive(Clone, Debug)]
pub enum AgentChatViewEvent {
    /// 关闭面板。
    Close,
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
    pub mentions: Vec<MentionItem>,
    pub model_options: Vec<ComposerModelOption>,
    pub selected_model_id: Option<SharedString>,
    pub runtime_factory: Option<AgentRuntimeFactory>,
    /// 以「侧边栏视图」(窄面板)模式渲染:头部走新建对话 / 历史记录 Popover,
    /// 不常驻左侧会话列表。默认 `false`(普通 tab 全宽视图)。
    ///
    /// **重要**：侧边栏模式下 ResourceContext 固定为当前连接，不支持切换。
    pub sidebar_mode: bool,
    /// 可接入的外部 ACP agent(自定义命令)。非空时头部显示后端切换控件。
    pub acp_agents: Vec<AcpAgentConfig>,
}

impl AgentChatViewConfig {
    pub fn new(
        runtime: Arc<Runtime>,
        resources: ResourceContext,
        mentions: Vec<MentionItem>,
    ) -> Self {
        let option = static_runtime_model_option(&runtime);
        Self {
            runtime,
            resources,
            mentions,
            model_options: vec![option.clone()],
            selected_model_id: Some(option.id),
            runtime_factory: None,
            sidebar_mode: false,
            acp_agents: Vec::new(),
        }
    }

    /// 切换为「侧边栏视图」(窄面板)模式。
    pub fn sidebar_mode(mut self, enabled: bool) -> Self {
        self.sidebar_mode = enabled;
        self
    }

    /// 注入可接入的外部 ACP agent 列表。
    pub fn with_acp_agents(mut self, agents: Vec<AcpAgentConfig>) -> Self {
        self.acp_agents = agents;
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
        let first = specs
            .first()
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("没有可用模型配置"))?;
        let runtime = first.build();
        let selected_model_id = selected_provider_model_id(&specs);
        let model_options = specs.iter().map(|spec| spec.option.clone()).collect();
        let spec_map: Arc<HashMap<String, RuntimeBuildSpec>> = Arc::new(
            specs
                .into_iter()
                .map(|spec| (spec.option.id.to_string(), spec))
                .collect(),
        );
        let fallback = first;
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
    pending_bottom_scroll: bool,
}

impl AutoScrollState {
    fn request(&mut self) {
        self.pending_bottom_scroll = true;
    }

    fn take_pending_for_render(&mut self) -> bool {
        std::mem::take(&mut self.pending_bottom_scroll)
    }
}

/// Runtime 驱动的 Agent 聊天面板。
pub struct AgentChatView {
    runtime: Arc<Runtime>,
    session_id: SessionId,
    resources: ResourceContext,
    transcript: AgentTranscript,
    input: Entity<AgentInput>,
    sessions: Vec<SessionSummary>,
    current_session: String,
    sidebar_collapsed: bool,
    /// 侧边栏是否显示「已归档」会话(否则显示活跃会话)。
    show_archived: bool,
    /// 侧边栏视图(窄面板)模式:头部走新建对话 / 历史记录紧凑布局,不常驻会话列表。
    sidebar_mode: bool,
    /// 侧边栏视图下「历史记录」Popover 的开合状态。
    history_popover_open: bool,
    /// 当前驱动后端(默认 One_Agent)。
    backend: Backend,
    /// 可接入的外部 ACP agent 列表。
    acp_agents: Vec<AcpAgentConfig>,
    /// 已建立的 ACP 连接(backend == Acp 时存在)。
    acp: Option<AcpConnection>,
    /// 当前选中的 ACP agent id(用于头部切换控件高亮)。
    current_acp_id: Option<SharedString>,
    /// 正在连接 ACP agent(拉起子进程中)。
    acp_connecting: bool,
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
    /// 是否侧边栏模式。
    _subscriptions: Vec<Subscription>,
    _event_task: Task<()>,
}

impl AgentChatView {
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
        let acp_agents = config.acp_agents;
        let resources = config.resources;
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

        let task_kind = default_task_kind();
        let selected_tool = SharedString::from("自动");
        let tool_options = default_tool_options();
        let task_options = default_task_options();

        let init_ctx = build_composer_context(
            &resources,
            task_kind,
            &selected_tool,
            selected_model.as_ref(),
            None,
            Backend::Local,
            &acp_agents,
            None,
            false,
            None,
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

        // 载入已持久化的会话列表,并把当前实时会话置顶(尚无内容,落库前为占位)。
        let mut sessions = persistence::list_summaries(cx);
        if !sessions.iter().any(|s| s.id == current_session) {
            sessions.insert(
                0,
                SessionSummary::new(current_session.clone(), "当前 Agent 会话", now_secs()),
            );
        }

        Self {
            runtime,
            session_id,
            resources,
            transcript: AgentTranscript::new(),
            input,
            sessions,
            current_session,
            sidebar_collapsed: false,
            show_archived: false,
            sidebar_mode,
            history_popover_open: false,
            backend: Backend::Local,
            acp_agents,
            acp: None,
            current_acp_id: None,
            acp_connecting: false,
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
            code_block_actions: CodeBlockActionRegistry::new(),
            _subscriptions: subscriptions,
            _event_task: event_task,
        }
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
            if self.acp.is_none() {
                self.transcript.push_system("ACP agent 未连接");
                cx.notify();
                return;
            }
            self.transcript.push_user(&text, images.len());
            self.request_scroll_to_bottom();
            self.set_running(true, cx);
            if let Some(acp) = &self.acp {
                acp.prompt(text);
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
        cx.spawn(async move |this, cx| {
            #[cfg(test)]
            let result = runtime
                .run_turn_blocking(&session_id, input, task_kind)
                .await;

            #[cfg(not(test))]
            let result = {
                let task = Tokio::spawn(cx, async move {
                    runtime
                        .run_turn_blocking(&session_id, input, task_kind)
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
        let Some(first) = mentions.first() else {
            return;
        };
        let id = ResourceId::new(first.id.clone());
        if self.resources.get(&id).is_some() {
            self.resources.current = Some(id);
            if let Some(session) = self.runtime.session(&self.session_id) {
                session.set_resources(self.resources.clone());
            }
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
            self.set_running(false, cx);
        }
        cx.notify();
    }

    fn apply_runtime_event(&mut self, event: RuntimeEvent, cx: &mut Context<Self>) {
        let terminal = matches!(
            event,
            RuntimeEvent::TurnCompleted { .. }
                | RuntimeEvent::TurnFailed { .. }
                | RuntimeEvent::NeedUserInput { .. }
        );
        self.transcript.apply(&event);
        self.sync_composer(cx);
        // 跟随流式输出 / 新卡片自动滚到底。
        self.request_scroll_to_bottom();
        if terminal {
            self.set_running(false, cx);
            // 一轮结束:把会话快照落库(仅自研后端;ACP 会话由外部 agent 管理)。
            if self.backend == Backend::Local {
                self.persist_current(cx);
            }
        }
        cx.notify();
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
            self.backend,
            &self.acp_agents,
            self.current_acp_id.as_ref(),
            self.acp_connecting,
            self.acp.as_ref().map(|acp| acp.state()),
        );
        self.input.update(cx, |inp, cx| inp.set_context(ctx, cx));
    }

    /// 在目标下拉中选中某个资源:设为当前目标并同步给会话与输入框。
    fn select_target(&mut self, id: &str, cx: &mut Context<Self>) {
        let rid = ResourceId::new(id.to_string());
        if self.resources.get(&rid).is_none() {
            return;
        }
        self.resources.current = Some(rid);
        if let Some(session) = self.runtime.session(&self.session_id) {
            session.set_resources(self.resources.clone());
        }
        self.sync_composer(cx);
        cx.notify();
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
            // 切回自研 One_Agent。
            None => {
                if self.backend == Backend::Local {
                    return;
                }
                self.acp = None;
                self.current_acp_id = None;
                self.backend = Backend::Local;
                self.acp_connecting = false;
                self.transcript.clear();
                self.set_running(false, cx);
                self._event_task =
                    Self::spawn_event_pump(self.runtime.subscribe(), self.session_id.clone(), cx);
                self.sync_composer(cx);
                cx.notify();
            }
            // 切到某个 ACP agent(惰性拉起子进程)。
            Some(id) => {
                if self.acp_connecting {
                    return;
                }
                if self.backend == Backend::Acp && self.current_acp_id.as_ref() == Some(&id) {
                    return;
                }
                let Some(config) = self.acp_agents.iter().find(|a| a.id == id).cloned() else {
                    return;
                };
                self.acp_connecting = true;
                self.set_running(false, cx);
                self.transcript.clear();
                self.transcript
                    .push_system(format!("正在连接 ACP agent「{}」…", config.name));
                self.sync_composer(cx);
                cx.notify();

                cx.spawn(async move |this, cx| {
                    let connected = AcpConnection::connect(&config, cx).await;
                    let _ = this.update(cx, |this, cx| match connected {
                        Ok(conn) => {
                            let rx = conn.subscribe();
                            let sid = conn.session_id();
                            this.acp = Some(conn);
                            this.backend = Backend::Acp;
                            this.current_acp_id = Some(config.id.clone());
                            this.acp_connecting = false;
                            this.transcript.clear();
                            this._event_task = Self::spawn_event_pump(rx, sid, cx);
                            this.sync_composer(cx);
                            cx.notify();
                        }
                        Err(err) => {
                            this.acp_connecting = false;
                            this.backend = Backend::Local;
                            this.current_acp_id = None;
                            this.transcript
                                .push_system(format!("连接 ACP agent 失败:{err}"));
                            this.sync_composer(cx);
                            cx.notify();
                        }
                    });
                })
                .detach();
            }
        }
    }

    /// 新建一个空会话并设为当前(仅运行时层面,不触碰持久化 / 列表)。
    fn start_fresh_session(&mut self, cx: &mut Context<Self>) {
        if self.is_running {
            self.stop(cx);
        }
        let session = self.runtime.create_session(self.resources.clone());
        self.session_id = session.id().clone();
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
        // 活跃视图:把当前实时会话(可能尚未落库)置顶,保留其已有显示名。
        if !self.show_archived && !list.iter().any(|s| s.id == self.current_session) {
            let name = self
                .sessions
                .iter()
                .find(|s| s.id == self.current_session)
                .map(|s| s.name.clone())
                .unwrap_or_else(|| SharedString::from("当前 Agent 会话"));
            list.insert(
                0,
                SessionSummary::new(self.current_session.clone(), name, now_secs()),
            );
        }
        self.sessions = list;
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
        self.current_session = self.session_id.to_string();
        self.transcript.load_history(&history, plan.as_ref());
        self._event_task =
            Self::spawn_event_pump(self.runtime.subscribe(), self.session_id.clone(), cx);
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
    pub fn set_system_instruction(&mut self, instruction: Option<String>, _cx: &mut Context<Self>) {
        self.system_instruction = instruction;
        // TODO: 将系统提示词应用到 Runtime 会话中
    }

    /// 更新可操作资源上下文与 `@` 提及项。
    pub fn set_resource_context(
        &mut self,
        resources: ResourceContext,
        mentions: Vec<MentionItem>,
        cx: &mut Context<Self>,
    ) {
        self.resources = resources.clone();
        if let Some(session) = self.runtime.session(&self.session_id) {
            session.set_resources(resources.clone());
        }
        let target_options: Vec<ComposerTarget> = resources
            .resources
            .iter()
            .map(target_from_resource)
            .collect();
        let ctx = build_context(
            &resources,
            self.task_kind,
            &self.selected_tool,
            self.selected_model.as_ref(),
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

        // 标题区:活跃视图可点击切换;归档视图只读。
        let label = session_sidebar::session_row(session, selected, cx);
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
                        .child("ACP 会话由外部 agent 管理,不在此持久化。"),
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
        let title = if self.show_archived {
            "已归档会话"
        } else {
            "Agent 会话"
        };
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
        let border = cx.theme().border;
        let muted = cx.theme().muted;
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
            .child(
                div()
                    .text_sm()
                    .font_weight(FontWeight::SEMIBOLD)
                    .child("Agent"),
            )
            .child(
                h_flex()
                    .gap_1()
                    .items_center()
                    .child(
                        Button::new("agent-sidebar-new")
                            .icon(IconName::Plus)
                            .ghost()
                            .small()
                            .tooltip("新建对话")
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
                                    .tooltip("历史记录"),
                            )
                            .when_some(history_list, |popover, list| popover.child(list)),
                    ),
            )
            .into_any_element()
    }

    /// 历史记录 Popover 内容:小标题 + 活跃/归档切换 + 会话行列表(复用行渲染)。
    fn render_history_list(&mut self, cx: &mut Context<Self>) -> gpui::AnyElement {
        let border = cx.theme().border;
        // ACP 模式:会话由外部 agent 管理,不展示本地列表。
        if self.backend == Backend::Acp {
            return v_flex()
                .w(px(300.0))
                .p_3()
                .child(
                    div()
                        .text_sm()
                        .text_color(cx.theme().muted_foreground)
                        .child("ACP 会话由外部 agent 管理,不在此持久化。"),
                )
                .into_any_element();
        }
        let title = if self.show_archived {
            "已归档会话"
        } else {
            "历史记录"
        };
        let sessions = self.sessions.clone();
        let rows: Vec<gpui::AnyElement> = sessions
            .iter()
            .map(|session| self.render_session_row(session, cx))
            .collect();
        let show_archived = self.show_archived;

        v_flex()
            .w(px(300.0))
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
                    .text_color(cx.theme().muted_foreground)
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
        // 任务模式与 Agent/ACP 切换都下沉到输入框,这里保留精简标题。
        h_flex()
            .w_full()
            .items_center()
            .justify_between()
            .px_4()
            .py_2()
            .border_b_1()
            .border_color(cx.theme().border)
            .child(
                div()
                    .text_sm()
                    .font_weight(FontWeight::SEMIBOLD)
                    .child("Agent"),
            )
            .into_any_element()
    }
}

impl EventEmitter<AgentChatViewEvent> for AgentChatView {}

impl Render for AgentChatView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self.auto_scroll.take_pending_for_render() {
            self.scroll_handle.scroll_to_bottom();
        }
        let messages = render_messages_with_code_actions(
            &self.transcript.messages,
            &self.scroll_handle,
            Some(&self.code_block_actions),
            window,
            cx,
        );
        let input_area = div()
            .w_full()
            .flex_shrink_0()
            .border_t_1()
            .border_color(cx.theme().border)
            .bg(cx.theme().background)
            .child(v_flex().w_full().p_3().gap_2().child(self.input.clone()));

        if self.sidebar_mode {
            // 侧边栏视图:紧凑头部(新建对话 / 历史记录) + 消息 + 输入。
            let header = self.render_sidebar_mode_header(cx);
            div().size_full().bg(cx.theme().background).child(
                v_flex()
                    .size_full()
                    .child(header)
                    .child(messages)
                    .child(input_area),
            )
        } else {
            // 普通全宽视图:常驻左侧会话栏 + 主区(标题 / 消息 / 输入)。
            let sidebar = self.render_sidebar(cx);
            let toolbar = self.render_toolbar(cx);
            div().size_full().bg(cx.theme().background).child(
                h_flex().size_full().child(sidebar).child(
                    div().flex_1().h_full().min_w_0().child(
                        v_flex()
                            .size_full()
                            .child(toolbar)
                            .child(messages)
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
    backend: Backend,
    acp_agents: &[AcpAgentConfig],
    current_acp_id: Option<&SharedString>,
    acp_connecting: bool,
    acp_state: Option<AcpSessionState>,
) -> AgentComposerContext {
    let mut context = build_context(resources, task_kind, tool_label, model);
    context.plan_items = composer_plan_items(plan);
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
        scopes,
        capabilities,
        plan_items: Vec::new(),
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
            .map(|step| ComposerPlanItem::new(step.title.clone(), step.status.clone()))
            .collect()
    })
    .unwrap_or_default()
}

fn composer_agent_options(
    backend: Backend,
    acp_agents: &[AcpAgentConfig],
    current_acp_id: Option<&SharedString>,
    acp_connecting: bool,
) -> Vec<ComposerAgentOption> {
    let mut options = vec![ComposerAgentOption::local(
        "Codex CLI",
        backend == Backend::Local,
        acp_connecting,
    )];
    options.extend(acp_agents.iter().map(|agent| {
        ComposerAgentOption::acp(
            agent.id.clone(),
            agent.name.clone(),
            backend == Backend::Acp && current_acp_id == Some(&agent.id),
            acp_connecting,
        )
    }));
    options
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
        ResourceKind::Other(_) => "··",
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
    use agent_runtime::RuntimeServices;
    use agent_runtime::model::MockModelClient;
    use agent_runtime::model::function_tool_call;
    use agent_runtime::model::{ModelClient, ModelRequest, ModelResponse, ModelStream};
    use agent_runtime::tools::builtin::EchoTool;
    use agent_runtime::{ToolRegistry, ToolRouter};
    use async_trait::async_trait;
    use gpui::{TestAppContext, VisualTestContext};
    use one_core::llm::{ProviderConfig, ProviderType};
    use serde_json::json;
    use std::sync::atomic::{AtomicUsize, Ordering};

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
    fn task_kind_round_trips() {
        assert_eq!(task_kind_from_id("agent"), Some(TaskKind::Agent));
        assert_eq!(task_kind_from_id("ask"), Some(TaskKind::Ask));
        assert_eq!(task_kind_from_id("plan"), Some(TaskKind::Plan));
        assert_eq!(task_kind_from_id("chat"), None);
        assert_eq!(task_kind_from_id("nope"), None);
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
                description: String::new(),
                status: "running".to_string(),
                risk: String::new(),
                tool: None,
            }],
        };
        let acp_id = SharedString::from("codex");
        let acp_agents = vec![AcpAgentConfig::new(acp_id.clone(), "Codex ACP", "codex")];

        let local = build_composer_context(
            &ResourceContext::new(),
            TaskKind::Agent,
            &SharedString::from("自动"),
            None,
            Some(&plan),
            Backend::Local,
            &acp_agents,
            None,
            false,
            None,
        );
        let acp = build_composer_context(
            &ResourceContext::new(),
            TaskKind::Agent,
            &SharedString::from("自动"),
            None,
            Some(&plan),
            Backend::Acp,
            &acp_agents,
            Some(&acp_id),
            false,
            None,
        );

        assert_eq!(local.plan_items, acp.plan_items);
        assert_eq!(local.plan_items[0].title.as_ref(), "检查连接");
        assert!(local.agent_options[0].selected);
        assert!(acp.agent_options[1].selected);
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
            Backend::Acp,
            &[],
            None,
            false,
            Some(state),
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
