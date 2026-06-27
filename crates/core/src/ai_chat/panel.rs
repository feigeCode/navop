//! AI Chat Panel - 数据库 AI 助手对话面板

use super::acp::config::AcpAgentConfig;
use super::acp::connection::AcpConnection;
use super::acp::provider::build_acp_agent_configs;
use crate::cloud_sync::GlobalCloudUser;
use crate::llm::{
    chat_history::{MessageRepository, SessionRepository},
    storage::ProviderRepository,
};
use crate::storage::{GlobalStorageState, traits::Repository};
use agent_runtime::ToolRegistry;
use gpui::{
    App, AppContext, AsyncApp, Context, Entity, EventEmitter, FocusHandle, Focusable, Hsla,
    IntoElement, ParentElement, Render, Styled, Subscription, Window, div, px,
};
use gpui_component::{
    ActiveTheme, WindowExt as _,
    dialog::DialogButtonProps,
    input::{Input, InputEvent, InputState},
    list::ListState,
    v_flex,
};
use rust_i18n::t;
use tracing::warn;
// 使用引擎和渲染器
use super::engine::ChatEngine;
use super::runtime_bridge::PlanRuntimeController;
use super::types::{AiChatMode, AiChatPlanBackend};
// 使用共享组件
use super::components::{
    ModelSettings, ModelSettingsEvent, ModelSettingsPanel, ProviderItem, ProviderSelectEvent,
    ProviderSelectState, SessionData, SessionListConfig, SessionListDelegate, SessionListHost,
};

mod ask;
pub mod code_block;
mod helpers;
mod plan_backend;
mod rendering;

pub use code_block::*;

/// AI 聊天面板事件
#[derive(Clone, Debug)]
pub enum AiChatPanelEvent {
    Close,
    ExecuteSql {
        sql: String,
        connection_id: String,
        database: Option<String>,
        schema: Option<String>,
    },
}

/// AI 聊天面板
pub struct AiChatPanel {
    focus_handle: FocusHandle,

    /// 共享业务逻辑引擎
    engine: ChatEngine,

    ai_input_state: Entity<InputState>,
    provider_select_state: ProviderSelectState,

    _subscriptions: Vec<Subscription>,
    connection_name: Option<String>,
    database: Option<String>,
    history_popover_open: bool,
    session_list: Option<Entity<ListState<SessionListDelegate<AiChatPanel>>>>,
    /// 可选的自定义颜色（用于终端等需要自定义主题的场景）
    custom_colors: Option<AiChatColors>,
    /// 模型设置面板
    settings_panel: Entity<ModelSettingsPanel>,
    is_logged_in: bool,
    /// 场景专属系统提示词，仅在发送消息时前置注入
    system_instruction: Option<String>,
    /// Plan 模式可调用的工具注册表，由上层注入当前 MCP/function-calling 工具。
    plan_tool_registry: ToolRegistry,
    plan_controller: Option<PlanRuntimeController>,
    plan_runtime_key: Option<String>,
    plan_backend: AiChatPlanBackend,
    plan_backend_popover_open: bool,
    acp_agent_configs: Vec<AcpAgentConfig>,
    acp_agent_config: Option<AcpAgentConfig>,
    acp_connection: Option<AcpConnection>,
}

impl AiChatPanel {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let focus_handle = cx.focus_handle();

        // 创建引擎
        let global_state = cx.global::<GlobalStorageState>();
        let engine = ChatEngine::new(global_state.storage.clone());

        // Agent 模式输入框
        let agent_input_state = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder(t!("AiChat.input_placeholder").to_string())
                .auto_grow(2, 6)
                .default_value("")
        });

        // Provider/Model 选择器（回调直接接收 &mut Self，避免重复借用）
        let provider_select_state =
            ProviderSelectState::new(window, cx, |event, this, window, cx| match event {
                ProviderSelectEvent::ProviderChanged { provider_id, .. } => {
                    this.engine.provider_id = Some(provider_id.clone());
                    this.engine.selected_model = this
                        .provider_select_state
                        .update_models_for_provider(&provider_id, window, cx);
                    cx.notify();
                }
                ProviderSelectEvent::ModelChanged { model } => {
                    this.engine.selected_model = Some(model.clone());
                    cx.notify();
                }
            });

        let mut subscriptions = Vec::new();

        // 订阅 Agent 输入事件
        subscriptions.push(cx.subscribe_in(
            &agent_input_state,
            window,
            |this, _state, event, window, cx| {
                if let InputEvent::PressEnter { secondary } = event {
                    if !secondary {
                        this.submit(window, cx);
                    }
                }
            },
        ));

        // 创建模型设置面板
        let model_settings = ModelSettings::default();
        let settings_panel =
            cx.new(|cx| ModelSettingsPanel::new(model_settings.clone(), window, cx));

        // 订阅模型设置事件
        subscriptions.push(cx.subscribe_in(
            &settings_panel,
            window,
            |this, _panel, event: &ModelSettingsEvent, _window, cx| match event {
                ModelSettingsEvent::Changed(settings) => {
                    this.engine.model_settings = settings.clone();
                    cx.notify();
                }
            },
        ));

        let mut panel = Self {
            focus_handle,
            engine,
            ai_input_state: agent_input_state,
            provider_select_state,
            _subscriptions: subscriptions,
            connection_name: None,
            database: None,
            history_popover_open: false,
            session_list: None,
            custom_colors: None,
            settings_panel,
            is_logged_in: GlobalCloudUser::is_logged_in(cx),
            system_instruction: None,
            plan_tool_registry: ToolRegistry::default(),
            plan_controller: None,
            plan_runtime_key: None,
            plan_backend: AiChatPlanBackend::LocalRuntime,
            plan_backend_popover_open: false,
            acp_agent_configs: Vec::new(),
            acp_agent_config: None,
            acp_connection: None,
        };

        // 加载 providers
        panel.load_providers(cx);
        panel
    }

    fn load_providers(&mut self, cx: &mut Context<Self>) {
        let global_state = cx.global::<GlobalStorageState>();
        let storage_manager = global_state.storage.clone();
        let is_logged_in = GlobalCloudUser::is_logged_in(cx);
        self.is_logged_in = is_logged_in;

        cx.spawn(async move |this, cx: &mut AsyncApp| {
            let providers = {
                let repo = match storage_manager.get::<ProviderRepository>() {
                    Some(r) => r,
                    None => return,
                };
                let mut list = match repo.list() {
                    Ok(all) => all.into_iter().filter(|p| p.enabled).collect::<Vec<_>>(),
                    Err(_) => Vec::new(),
                };
                if is_logged_in {
                    if let Ok(onet) = repo.ensure_onetcli_provider() {
                        if !list.iter().any(|p| p.id == onet.id) {
                            list.insert(0, onet);
                        }
                    }
                } else {
                    list.retain(|p| !p.is_builtin());
                }
                list
            };

            let _ = cx.update(|cx| {
                if let Some(window_id) = cx.active_window() {
                    let _ = cx.update_window(window_id, |_, window, cx| {
                        if let Some(entity) = this.upgrade() {
                            entity.update(cx, |panel, cx| {
                                panel.engine.provider_configs = providers.clone();
                                let items: Vec<_> =
                                    providers.iter().map(ProviderItem::from_config).collect();
                                panel.provider_select_state.set_providers(items, window, cx);
                                panel.engine.provider_id =
                                    panel.provider_select_state.selected_provider().cloned();
                                panel.engine.selected_model =
                                    panel.provider_select_state.selected_model().cloned();
                                cx.notify();
                            });
                        }
                    });
                }
            });
        })
        .detach();
    }

    pub fn set_connection_info(
        &mut self,
        connection_name: Option<String>,
        database: Option<String>,
    ) {
        self.connection_name = connection_name;
        self.database = database;
    }

    /// 设置场景专属系统提示词
    pub fn set_system_instruction(&mut self, instruction: Option<String>, cx: &mut Context<Self>) {
        self.system_instruction = instruction.and_then(|instruction| {
            let trimmed = instruction.trim();
            (!trimmed.is_empty()).then(|| trimmed.to_string())
        });
        cx.notify();
    }

    /// 设置 Plan 模式使用的工具注册表。
    pub fn set_plan_tool_registry(&mut self, registry: ToolRegistry, cx: &mut Context<Self>) {
        self.plan_tool_registry = registry;
        self.plan_controller = None;
        self.plan_runtime_key = None;
        cx.notify();
    }

    pub fn set_plan_backend(&mut self, backend: AiChatPlanBackend, cx: &mut Context<Self>) {
        if self.plan_backend == backend {
            self.plan_backend_popover_open = false;
            cx.notify();
            return;
        }
        self.interrupt_plan_backends();
        self.plan_backend = backend;
        self.plan_backend_popover_open = false;
        self.engine.cancel_current_operation();
        cx.notify();
    }

    pub fn set_acp_agent_config(&mut self, config: Option<AcpAgentConfig>, cx: &mut Context<Self>) {
        self.interrupt_plan_backends();
        self.acp_agent_config = config;
        self.acp_connection = None;
        self.plan_backend_popover_open = false;
        cx.notify();
    }

    /// 切换 Ask / Plan 模式。
    pub fn set_mode(&mut self, mode: AiChatMode, cx: &mut Context<Self>) {
        if self.engine.mode() == mode {
            return;
        }
        self.interrupt_plan_backends();
        self.engine.set_mode(mode);
        cx.notify();
    }

    /// 设置自定义颜色（用于终端等需要自定义主题的场景）
    pub fn set_colors(&mut self, colors: AiChatColors, cx: &mut Context<Self>) {
        self.custom_colors = Some(colors);
        cx.notify();
    }

    /// 获取背景色（自定义或默认主题）
    fn background(&self, cx: &App) -> Hsla {
        self.custom_colors
            .as_ref()
            .map(|c| c.background)
            .unwrap_or_else(|| cx.theme().background)
    }

    /// 获取前景色（自定义或默认主题）
    fn foreground(&self, cx: &App) -> Hsla {
        self.custom_colors
            .as_ref()
            .map(|c| c.foreground)
            .unwrap_or_else(|| cx.theme().foreground)
    }

    /// 获取次要背景色（自定义或默认主题）
    fn muted(&self, cx: &App) -> Hsla {
        self.custom_colors
            .as_ref()
            .map(|c| c.muted)
            .unwrap_or_else(|| cx.theme().muted)
    }

    /// 获取边框色（自定义或默认主题）
    fn border(&self, cx: &App) -> Hsla {
        self.custom_colors
            .as_ref()
            .map(|c| c.border)
            .unwrap_or_else(|| cx.theme().border)
    }

    pub fn set_provider_id(&mut self, provider_id: String, cx: &mut Context<Self>) {
        self.engine.provider_id = Some(provider_id.clone());
        if let Some(config) = self
            .engine
            .provider_configs
            .iter()
            .find(|provider| provider.id.to_string() == provider_id)
        {
            let models = ProviderSelectState::build_model_list_from_config(config);
            self.engine.selected_model =
                ProviderSelectState::resolve_default_model_from_config(config, &models);
        }
        cx.notify();
    }

    /// 注册代码块操作
    pub fn register_code_block_action(&mut self, action: CodeBlockAction, cx: &mut Context<Self>) {
        self.engine.code_block_actions.register(action);
        cx.notify();
    }

    /// 批量注册代码块操作
    pub fn register_code_block_actions(
        &mut self,
        actions: Vec<CodeBlockAction>,
        cx: &mut Context<Self>,
    ) {
        for action in actions {
            self.engine.code_block_actions.register(action);
        }
        cx.notify();
    }

    /// 获取代码块操作注册表的引用（用于外部查询）
    pub fn code_block_actions(&self) -> &CodeBlockActionRegistry {
        &self.engine.code_block_actions
    }

    /// 从外部发送消息到AI聊天
    pub fn send_external_message(&mut self, message: String, cx: &mut Context<Self>) {
        if !message.trim().is_empty() {
            self.send_message(message, cx);
        }
    }

    // 创建新会话 - 同步返回，异步保存
    pub fn start_new_session(&mut self, cx: &mut Context<Self>) {
        self.engine.start_new_session();
        cx.notify();
    }

    /// 确保会话存在，如果不存在则创建新会话
    fn ensure_session_id(&mut self, provider_id: &str, cx: &mut Context<Self>) -> Option<i64> {
        let result = self
            .engine
            .ensure_session_id(provider_id, t!("AiChat.new_session_name").as_ref());
        if result.is_some() && self.engine.is_new_session {
            // 新创建的会话，需要刷新历史列表（ensure_session_id 已经设置了 is_new_session）
            self.load_history_sessions(cx);
        }
        result
    }

    /// 持久化用户消息，并在新会话时更新标题
    fn persist_user_message(&mut self, session_id: i64, content: &str, cx: &mut Context<Self>) {
        let was_new = self.engine.is_new_session;
        self.engine.persist_user_message(session_id, content);
        if was_new {
            self.load_history_sessions(cx);
        }
    }

    /// 取消当前操作
    pub fn cancel_current_operation(&mut self, cx: &mut Context<Self>) {
        self.interrupt_plan_backends();
        self.engine.cancel_current_operation();
        cx.notify();
    }

    /// 是否可以取消
    pub fn can_cancel(&self) -> bool {
        self.engine.can_cancel()
    }

    fn refresh_acp_agent_configs(&mut self, cx: &mut Context<Self>) {
        let configs = match build_acp_agent_configs(cx) {
            Ok(configs) => configs,
            Err(error) => {
                warn!("Failed to load ACP agent configs: {}", error);
                return;
            }
        };
        let selected_id = self
            .acp_agent_config
            .as_ref()
            .map(|config| config.id.to_string());
        if let Some(selected_id) = selected_id {
            match configs
                .iter()
                .find(|config| config.id.as_ref() == selected_id)
                .cloned()
            {
                Some(config) => self.acp_agent_config = Some(config),
                None => {
                    self.acp_agent_config = None;
                    self.acp_connection = None;
                    if self.plan_backend == AiChatPlanBackend::AcpAgent {
                        self.plan_backend = AiChatPlanBackend::LocalRuntime;
                    }
                }
            }
        }
        self.acp_agent_configs = configs;
    }

    fn interrupt_plan_backends(&mut self) {
        if let Some(controller) = self.plan_controller.as_ref()
            && let Err(error) = controller.interrupt()
        {
            warn!("Failed to interrupt plan runtime: {}", error);
        }
        if let Some(connection) = self.acp_connection.as_ref() {
            connection.cancel();
        }
    }

    fn update_session_list(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let sessions_data: Vec<SessionData> = self
            .engine
            .history_sessions
            .iter()
            .map(|s| SessionData::new(s.id, s.name.clone(), s.updated_at))
            .collect();
        let panel = cx.entity();

        if let Some(session_list) = &self.session_list {
            session_list.update(cx, |state, _| {
                let delegate = state.delegate_mut();
                delegate.update_sessions(sessions_data);
            });
        } else {
            self.session_list = Some(cx.new(|cx| {
                ListState::new(
                    SessionListDelegate::new(panel, sessions_data, SessionListConfig::default()),
                    window,
                    cx,
                )
                .searchable(true)
            }));
        }
    }

    fn load_history_sessions(&mut self, cx: &mut Context<Self>) {
        let global_state = cx.global::<GlobalStorageState>();
        let storage_manager = global_state.storage.clone();

        cx.spawn(async move |this, cx: &mut AsyncApp| {
            let sessions = {
                let session_repo = match storage_manager.get::<SessionRepository>() {
                    Some(r) => r,
                    None => return,
                };
                match session_repo.list() {
                    Ok(s) => s,
                    Err(_) => return,
                }
            };

            if let Some(entity) = this.upgrade() {
                let _ = cx.update(|cx| {
                    if let Some(window_id) = cx.active_window() {
                        let _ = cx.update_window(window_id, |_, window, cx| {
                            entity.update(cx, |this, cx| {
                                this.engine.history_sessions = sessions;
                                this.update_session_list(window, cx);
                                cx.notify();
                            });
                        });
                    }
                });
            }
        })
        .detach();
    }

    fn delete_session(&mut self, session_id: i64, cx: &mut Context<Self>) {
        let global_state = cx.global::<GlobalStorageState>();
        let storage_manager = global_state.storage.clone();

        cx.spawn(async move |this, cx: &mut AsyncApp| {
            let delete_ok = {
                let session_repo = match storage_manager.get::<SessionRepository>() {
                    Some(r) => r,
                    None => return,
                };
                let message_repo = match storage_manager.get::<MessageRepository>() {
                    Some(r) => r,
                    None => return,
                };
                message_repo.delete_by_session(session_id).is_ok()
                    && session_repo.delete(session_id).is_ok()
            };

            if delete_ok {
                if let Some(entity) = this.upgrade() {
                    let _ = cx.update(|cx| {
                        entity.update(cx, |this, cx| {
                            if this.engine.session_id == Some(session_id) {
                                this.engine.session_id = None;
                                this.engine.messages.clear();
                            }
                            this.engine.history_sessions.retain(|s| s.id != session_id);
                            cx.notify();
                        });
                    });
                }
            }
        })
        .detach();
    }

    fn start_rename_session(
        &mut self,
        session_id: i64,
        current_name: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let input_state = cx.new(|cx| {
            InputState::new(window, cx)
                .multi_line(true)
                .rows(1)
                .default_value(&current_name)
                .placeholder(t!("AiChat.session_name_placeholder").to_string())
        });

        let panel_entity = cx.entity();
        let input_for_dialog = input_state.clone();

        window.open_dialog(cx, move |dialog, _window, _cx| {
            let input_for_ok = input_for_dialog.clone();
            let panel_for_ok = panel_entity.clone();

            dialog
                .title(t!("AiChat.rename_session_title").to_string())
                .w(px(360.0))
                .confirm()
                .button_props(
                    DialogButtonProps::default()
                        .ok_text(t!("Common.save").to_string())
                        .cancel_text(t!("Common.cancel").to_string()),
                )
                .on_ok(move |_, _window, cx| {
                    let new_name = input_for_ok.read(cx).value().to_string();
                    if !new_name.trim().is_empty() {
                        panel_for_ok.update(cx, |this, cx| {
                            this.rename_session(session_id, new_name, cx);
                        });
                    }
                    true
                })
                .child(
                    v_flex()
                        .gap_2()
                        .child(
                            div()
                                .text_sm()
                                .child(t!("AiChat.rename_session_prompt").to_string()),
                        )
                        .child(Input::new(&input_for_dialog).w_full()),
                )
        });
    }

    fn rename_session(&mut self, session_id: i64, new_name: String, cx: &mut Context<Self>) {
        let global_state = cx.global::<GlobalStorageState>();
        let storage_manager = global_state.storage.clone();

        cx.spawn(async move |this, cx: &mut AsyncApp| {
            let renamed = {
                let session_repo = match storage_manager.get::<SessionRepository>() {
                    Some(r) => r,
                    None => return,
                };
                if let Ok(Some(mut session)) = session_repo.get(session_id) {
                    session.name = new_name;
                    session_repo.update(&session).is_ok()
                } else {
                    false
                }
            };

            if renamed {
                if let Some(entity) = this.upgrade() {
                    let _ = cx.update(|cx| {
                        entity.update(cx, |this, cx| {
                            this.load_history_sessions(cx);
                        });
                    });
                }
            }
        })
        .detach();
    }

    fn load_session(&mut self, session_id: i64, cx: &mut Context<Self>) {
        let global_state = cx.global::<GlobalStorageState>();
        let storage_manager = global_state.storage.clone();

        cx.spawn(async move |this, cx: &mut AsyncApp| {
            let messages = {
                let message_repo = match storage_manager.get::<MessageRepository>() {
                    Some(r) => r,
                    None => return,
                };
                match message_repo.list_by_session(session_id) {
                    Ok(m) => m,
                    Err(_) => return,
                }
            };

            if let Some(entity) = this.upgrade() {
                let _ = cx.update(|cx| {
                    entity.update(cx, |this, cx| {
                        this.engine.session_id = Some(session_id);
                        this.engine.messages = ChatEngine::messages_from_history(&messages);
                        this.history_popover_open = false;
                        cx.notify();
                    });
                });
            }
        })
        .detach();
    }

    fn send_message(&mut self, content: String, cx: &mut Context<Self>) {
        match self.engine.mode() {
            AiChatMode::Ask => self.send_ask_message(content, cx),
            AiChatMode::Plan => self.send_plan_message(content, cx),
        }
    }
}

impl EventEmitter<AiChatPanelEvent> for AiChatPanel {}

impl Focusable for AiChatPanel {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl SessionListHost for AiChatPanel {
    fn on_session_select(&mut self, session_id: i64, cx: &mut Context<Self>) {
        self.load_session(session_id, cx);
    }

    fn on_session_edit(
        &mut self,
        session_id: i64,
        name: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.start_rename_session(session_id, name, window, cx);
    }

    fn on_session_delete(&mut self, session_id: i64, cx: &mut Context<Self>) {
        self.delete_session(session_id, cx);
    }

    fn is_current_session(&self, session_id: i64) -> bool {
        self.engine.session_id == Some(session_id)
    }

    fn on_session_list_confirm(&mut self, cx: &mut Context<Self>) {
        self.history_popover_open = false;
        cx.notify();
    }

    fn on_session_list_cancel(&mut self, cx: &mut Context<Self>) {
        self.history_popover_open = false;
        cx.notify();
    }
}

impl Render for AiChatPanel {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.refresh_acp_agent_configs(cx);
        let is_logged_in = GlobalCloudUser::is_logged_in(cx);
        if is_logged_in != self.is_logged_in {
            self.is_logged_in = is_logged_in;
            self.load_providers(cx);
        }

        let bg_color = self.background(cx);
        let fg_color = self.foreground(cx);

        div().size_full().child(
            v_flex()
                .size_full()
                .bg(bg_color)
                .text_color(fg_color)
                .child(self.render_header(window, cx))
                .child(self.render_messages(window, cx))
                .child(self.render_input(window, cx)),
        )
    }
}
