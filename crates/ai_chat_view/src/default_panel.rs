//! 默认 Agent 聊天面板。
//!
//! 给侧边栏提供一个同步可创建的包装层:内部异步读取 provider 配置,
//! 构建 [`AgentChatView`],并暴露 close event 与外部消息入口。

use gpui::{
    AnyWindowHandle, App, AppContext, AsyncApp, Context, Entity, EventEmitter, FocusHandle,
    Focusable, InteractiveElement, IntoElement, ParentElement, Render, SharedString, Styled,
    Subscription, Window, div, prelude::FluentBuilder,
};
use gpui_component::{ActiveTheme, Icon, IconName, Sizable, Size, v_flex};
use one_core::{
    connection_notifier::{ConnectionDataEvent, get_notifier},
    llm::{
        GlobalProviderState, ProviderConfig,
        notifier::{ProviderConfigEvent, get_notifier as get_provider_notifier},
        storage::ProviderRepository,
    },
    sidebar_contribution::SidebarPlacement,
    storage::{ConnectionRepository, GlobalStorageState, StoredConnection, traits::Repository},
    tab_container::{TabContent, TabContentEvent},
};
use rust_i18n::t;

use crate::{
    AcpAgentEntry, AgentChatTheme, AgentChatView, AgentChatViewConfig, AgentChatViewEvent,
    CodeBlockAction, MentionItem,
    agent_tool_config::{AgentToolConfigEvent, get_notifier as get_agent_tool_config_notifier},
    build_acp_agent_entries, build_plan_tool_registry,
};

#[derive(Clone, Debug)]
pub enum DefaultAgentChatPanelEvent {
    Close,
    MoveTo(SidebarPlacement),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum DefaultAgentChatPanelMode {
    Sidebar,
    Workbench,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ProviderRefreshAction {
    RebuildView,
    RefreshModels,
}

pub(crate) fn provider_refresh_action(has_view: bool) -> ProviderRefreshAction {
    if has_view {
        ProviderRefreshAction::RefreshModels
    } else {
        ProviderRefreshAction::RebuildView
    }
}

pub(crate) fn should_refresh_resource_catalog(event: &ConnectionDataEvent) -> bool {
    matches!(
        event,
        ConnectionDataEvent::ConnectionCreated { .. }
            | ConnectionDataEvent::ConnectionUpdated { .. }
            | ConnectionDataEvent::ConnectionDeleted { .. }
    )
}

fn list_connections(cx: &mut Context<DefaultAgentChatPanel>) -> Option<Vec<StoredConnection>> {
    let storage_state = cx.try_global::<GlobalStorageState>()?;
    Some(
        storage_state
            .storage
            .get::<ConnectionRepository>()
            .and_then(|repo| repo.list().ok())
            .unwrap_or_default(),
    )
}

pub struct DefaultAgentChatPanel {
    focus_handle: FocusHandle,
    mode: DefaultAgentChatPanelMode,
    view: Option<Entity<AgentChatView>>,
    view_subscription: Option<Subscription>,
    pending_message: Option<String>,
    pending_system_instruction: Option<String>,
    pending_resource_context: Option<(
        agent_runtime::ResourceContext,
        Vec<MentionItem>,
        Vec<agent_runtime::ResourceRef>,
    )>,
    pending_resource_catalog: Option<(Vec<MentionItem>, Vec<agent_runtime::ResourceRef>)>,
    pending_code_block_actions: Vec<CodeBlockAction>,
    connection_subscription: Option<Subscription>,
    provider_subscription: Option<Subscription>,
    agent_tool_config_subscription: Option<Subscription>,
    provider_refresh_generation: u64,
    window_handle: AnyWindowHandle,
    theme: Option<AgentChatTheme>,
    show_sidebar_header: bool,
    show_sidebar_frame_controls: bool,
    sidebar_frame_placement: SidebarPlacement,
    tab_closeable: bool,
    error: Option<String>,
}

impl DefaultAgentChatPanel {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        Self::new_with_context(
            agent_runtime::ResourceContext::new(),
            Vec::new(),
            window,
            cx,
        )
    }

    pub fn new_with_context(
        resources: agent_runtime::ResourceContext,
        mentions: Vec<MentionItem>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let available_resources = resources.resources.clone();
        Self::new_with_context_and_catalog(resources, mentions, available_resources, window, cx)
    }

    pub fn new_with_context_and_catalog(
        resources: agent_runtime::ResourceContext,
        mentions: Vec<MentionItem>,
        available_resources: Vec<agent_runtime::ResourceRef>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        Self::new_with_context_and_catalog_for_mode(
            resources,
            mentions,
            available_resources,
            DefaultAgentChatPanelMode::Sidebar,
            window,
            cx,
        )
    }

    pub fn new_sidebar_with_scope_and_catalog(
        scope: agent_runtime::AgentResourceScope,
        catalog: agent_runtime::ResourceCatalog,
        mentions: Vec<MentionItem>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        Self::new_with_context_and_catalog(
            scope.to_resource_context(),
            mentions,
            catalog.resources,
            window,
            cx,
        )
    }

    pub fn new_workbench(window: &mut Window, cx: &mut Context<Self>) -> Self {
        Self::new_workbench_with_context(
            agent_runtime::ResourceContext::new(),
            Vec::new(),
            window,
            cx,
        )
    }

    pub fn new_workbench_with_context(
        resources: agent_runtime::ResourceContext,
        mentions: Vec<MentionItem>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let available_resources = resources.resources.clone();
        Self::new_workbench_with_context_and_catalog(
            resources,
            mentions,
            available_resources,
            window,
            cx,
        )
    }

    pub fn new_workbench_with_context_and_catalog(
        resources: agent_runtime::ResourceContext,
        mentions: Vec<MentionItem>,
        available_resources: Vec<agent_runtime::ResourceRef>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        Self::new_with_context_and_catalog_for_mode(
            resources,
            mentions,
            available_resources,
            DefaultAgentChatPanelMode::Workbench,
            window,
            cx,
        )
    }

    pub fn new_workbench_with_scope_and_catalog(
        scope: agent_runtime::AgentResourceScope,
        catalog: agent_runtime::ResourceCatalog,
        mentions: Vec<MentionItem>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        Self::new_workbench_with_context_and_catalog(
            scope.to_resource_context(),
            mentions,
            catalog.resources,
            window,
            cx,
        )
    }

    fn new_with_context_and_catalog_for_mode(
        resources: agent_runtime::ResourceContext,
        mentions: Vec<MentionItem>,
        available_resources: Vec<agent_runtime::ResourceRef>,
        mode: DefaultAgentChatPanelMode,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let mut panel = Self {
            focus_handle: cx.focus_handle(),
            mode,
            view: None,
            view_subscription: None,
            pending_message: None,
            pending_system_instruction: None,
            pending_resource_context: Some((
                resources.clone(),
                mentions.clone(),
                available_resources.clone(),
            )),
            pending_resource_catalog: None,
            pending_code_block_actions: Vec::new(),
            connection_subscription: None,
            provider_subscription: None,
            agent_tool_config_subscription: None,
            provider_refresh_generation: 0,
            window_handle: window.window_handle(),
            theme: None,
            show_sidebar_header: true,
            show_sidebar_frame_controls: false,
            sidebar_frame_placement: SidebarPlacement::Right,
            tab_closeable: false,
            error: None,
        };
        panel.subscribe_connection_events(cx);
        panel.subscribe_provider_events(cx);
        panel.subscribe_agent_tool_config_events(cx);
        panel.spawn_build_view(cx);
        panel
    }

    /// Configure whether this panel can be closed when hosted in a regular tab.
    pub fn with_tab_closeable(mut self, closeable: bool) -> Self {
        self.tab_closeable = closeable;
        self
    }

    fn subscribe_provider_events(&mut self, cx: &mut Context<Self>) {
        let Some(notifier) = get_provider_notifier(cx) else {
            return;
        };
        self.provider_subscription = Some(
            cx.subscribe(&notifier, |this, _, _: &ProviderConfigEvent, cx| {
                this.refresh_provider_models(cx)
            }),
        );
    }

    fn subscribe_agent_tool_config_events(&mut self, cx: &mut Context<Self>) {
        let Some(notifier) = get_agent_tool_config_notifier(cx) else {
            return;
        };
        self.agent_tool_config_subscription = Some(
            cx.subscribe(&notifier, |this, _, _: &AgentToolConfigEvent, cx| {
                this.refresh_provider_models(cx)
            }),
        );
    }

    fn refresh_provider_models(&mut self, cx: &mut Context<Self>) {
        self.provider_refresh_generation = self.provider_refresh_generation.wrapping_add(1);
        match provider_refresh_action(self.view.is_some()) {
            ProviderRefreshAction::RebuildView => {
                self.spawn_build_view(cx);
                return;
            }
            ProviderRefreshAction::RefreshModels => {}
        }
        let view = self.view.clone().expect("view checked above");
        let generation = self.provider_refresh_generation;
        let storage = cx.global::<GlobalStorageState>().storage.clone();
        let provider_state = cx.global::<GlobalProviderState>().clone();
        let registry = build_plan_tool_registry(cx).unwrap_or_else(|error| {
            tracing::warn!(%error, "Failed to rebuild plan tool registry");
            agent_runtime::ToolRegistry::new()
        });
        cx.spawn(async move |this, cx: &mut AsyncApp| {
            let configs = load_enabled_provider_configs(&storage);
            let result = AgentChatViewConfig::from_provider_state(
                agent_runtime::ResourceContext::new(),
                Vec::new(),
                configs,
                registry,
                provider_state,
            )
            .await;
            let Some(panel) = this.upgrade() else {
                return;
            };
            if cx.read_entity(&panel, |panel, _| panel.provider_refresh_generation) != generation {
                return;
            }
            let _ = view.update(cx, |view, cx| match result {
                Ok(config) => {
                    let tool_registry = config.runtime.services().tools.registry();
                    view.refresh_models(
                        config.model_options,
                        config.selected_model_id,
                        config.runtime_factory,
                        tool_registry,
                        cx,
                    )
                }
                Err(error) => tracing::warn!(%error, "Failed to refresh provider models"),
            });
        })
        .detach();
    }

    fn subscribe_connection_events(&mut self, cx: &mut Context<Self>) {
        let Some(notifier) = get_notifier(cx) else {
            return;
        };
        self.connection_subscription = Some(cx.subscribe(
            &notifier,
            |this, _, event: &ConnectionDataEvent, cx| {
                if should_refresh_resource_catalog(event) {
                    this.refresh_resource_catalog(cx);
                }
            },
        ));
    }

    fn refresh_resource_catalog(&mut self, cx: &mut Context<Self>) {
        let Some(connections) = list_connections(cx) else {
            return;
        };
        let (_, catalog, mentions) = crate::build_workbench_resource_state(&connections);
        self.set_resource_catalog(mentions, catalog.resources, cx);
    }

    pub fn send_external_message(&mut self, message: String, cx: &mut Context<Self>) {
        if let Some(view) = &self.view {
            view.update(cx, |view, cx| view.send_external_message(message, cx));
        } else {
            self.pending_message = Some(message);
            cx.notify();
        }
    }

    pub fn set_system_instruction(&mut self, instruction: Option<String>, cx: &mut Context<Self>) {
        if let Some(view) = &self.view {
            view.update(cx, |view, cx| view.set_system_instruction(instruction, cx));
        } else {
            self.pending_system_instruction = instruction;
            cx.notify();
        }
    }

    pub fn set_resource_context(
        &mut self,
        resources: agent_runtime::ResourceContext,
        mentions: Vec<MentionItem>,
        cx: &mut Context<Self>,
    ) {
        let available_resources = resources.resources.clone();
        self.set_resource_context_with_catalog(resources, mentions, available_resources, cx);
    }

    pub fn set_resource_context_with_catalog(
        &mut self,
        resources: agent_runtime::ResourceContext,
        mentions: Vec<MentionItem>,
        available_resources: Vec<agent_runtime::ResourceRef>,
        cx: &mut Context<Self>,
    ) {
        if let Some(view) = &self.view {
            view.update(cx, |view, cx| {
                view.set_resource_context_with_catalog(
                    resources,
                    mentions,
                    available_resources,
                    cx,
                );
            });
        } else {
            self.pending_resource_context = Some((resources, mentions, available_resources));
            self.pending_resource_catalog = None;
            cx.notify();
        }
    }

    pub fn set_resource_catalog(
        &mut self,
        mentions: Vec<MentionItem>,
        available_resources: Vec<agent_runtime::ResourceRef>,
        cx: &mut Context<Self>,
    ) {
        if let Some(view) = &self.view {
            view.update(cx, |view, cx| {
                view.set_resource_catalog(mentions, available_resources, cx);
            });
        } else if let Some((_, pending_mentions, pending_available_resources)) =
            self.pending_resource_context.as_mut()
        {
            *pending_mentions = mentions;
            *pending_available_resources = available_resources;
            cx.notify();
        } else {
            self.pending_resource_catalog = Some((mentions, available_resources));
            cx.notify();
        }
    }

    pub fn register_code_block_action(&mut self, action: CodeBlockAction, cx: &mut Context<Self>) {
        if let Some(view) = &self.view {
            view.update(cx, |view, cx| view.register_code_block_action(action, cx));
        } else {
            self.pending_code_block_actions.push(action);
            cx.notify();
        }
    }

    pub fn set_sidebar_header_visible(&mut self, visible: bool, cx: &mut Context<Self>) {
        self.show_sidebar_header = visible;
        if let Some(view) = &self.view {
            view.update(cx, |view, cx| {
                view.set_sidebar_header_visible(visible, cx);
            });
        }
        cx.notify();
    }

    pub fn set_sidebar_frame_controls(
        &mut self,
        visible: bool,
        placement: SidebarPlacement,
        cx: &mut Context<Self>,
    ) {
        self.show_sidebar_frame_controls = visible;
        self.sidebar_frame_placement = placement;
        if let Some(view) = &self.view {
            view.update(cx, |view, cx| {
                view.set_sidebar_frame_controls(visible, placement, cx);
            });
        }
        cx.notify();
    }

    pub fn set_theme(&mut self, theme: Option<AgentChatTheme>, cx: &mut Context<Self>) {
        self.theme = theme.clone();
        if let Some(view) = &self.view {
            view.update(cx, |view, cx| view.set_theme(theme, cx));
        }
        cx.notify();
    }

    fn spawn_build_view(&mut self, cx: &mut Context<Self>) {
        let Some((resources, mentions, available_resources)) =
            self.pending_resource_context.clone()
        else {
            return;
        };
        let mode = self.mode;
        let generation = self.provider_refresh_generation;
        let storage = cx.global::<GlobalStorageState>().storage.clone();
        let provider_state = cx.global::<GlobalProviderState>().clone();
        let registry = build_plan_tool_registry(cx).unwrap_or_else(|error| {
            tracing::warn!(%error, "Failed to build plan tool registry");
            agent_runtime::ToolRegistry::new()
        });
        let acp_agents = build_acp_agent_entries(cx).unwrap_or_else(|error| {
            tracing::warn!(%error, "Failed to build ACP agent configs");
            Vec::new()
        });
        let window_handle = self.window_handle.clone();

        cx.spawn(async move |this, cx: &mut AsyncApp| {
            let provider_configs = load_enabled_provider_configs(&storage);
            let config = AgentChatViewConfig::from_provider_state(
                resources,
                mentions,
                provider_configs,
                registry,
                provider_state,
            )
            .await
            .map(|config| {
                let config = config.with_available_resources(available_resources);
                match mode {
                    DefaultAgentChatPanelMode::Sidebar => build_sidebar_config(config, acp_agents),
                    DefaultAgentChatPanelMode::Workbench => {
                        build_workbench_config(config, acp_agents)
                    }
                }
            });

            let _ = cx.update_window(window_handle, |_, window, cx| {
                if let Some(panel) = this.upgrade() {
                    if panel.read(cx).provider_refresh_generation != generation {
                        return;
                    }
                    panel.update(cx, |panel, cx| match config {
                        Ok(mut config) => {
                            if let Some(theme) = panel.theme.clone() {
                                config = config.with_theme(theme);
                            }
                            config = config.show_sidebar_header(panel.show_sidebar_header);
                            config = config.show_sidebar_frame_controls(
                                panel.show_sidebar_frame_controls,
                                panel.sidebar_frame_placement,
                            );
                            let view = AgentChatView::view_with_config(config, window, cx);
                            let view_subscription =
                                cx.subscribe(&view, |_, _, event: &AgentChatViewEvent, cx| {
                                    match event {
                                        AgentChatViewEvent::Close => {
                                            cx.emit(DefaultAgentChatPanelEvent::Close);
                                        }
                                        AgentChatViewEvent::MoveTo(placement) => {
                                            cx.emit(DefaultAgentChatPanelEvent::MoveTo(*placement));
                                        }
                                    }
                                });
                            if let Some(instruction) = panel.pending_system_instruction.clone() {
                                view.update(cx, |view, cx| {
                                    view.set_system_instruction(Some(instruction), cx);
                                });
                            }
                            if let Some((resources, mentions, available_resources)) =
                                panel.pending_resource_context.take()
                            {
                                view.update(cx, |view, cx| {
                                    view.set_resource_context_with_catalog(
                                        resources,
                                        mentions,
                                        available_resources,
                                        cx,
                                    );
                                });
                            }
                            if let Some((mentions, available_resources)) =
                                panel.pending_resource_catalog.take()
                            {
                                view.update(cx, |view, cx| {
                                    view.set_resource_catalog(mentions, available_resources, cx);
                                });
                            }
                            for action in panel.pending_code_block_actions.drain(..) {
                                view.update(cx, |view, cx| {
                                    view.register_code_block_action(action, cx);
                                });
                            }
                            if let Some(message) = panel.pending_message.take() {
                                view.update(cx, |view, cx| {
                                    view.send_external_message(message, cx);
                                });
                            }
                            panel.view = Some(view);
                            panel.view_subscription = Some(view_subscription);
                            panel.error = None;
                            cx.notify();
                        }
                        Err(error) => {
                            panel.error = Some(error.to_string());
                            cx.notify();
                        }
                    });
                }
            });
        })
        .detach();
    }
}

impl EventEmitter<DefaultAgentChatPanelEvent> for DefaultAgentChatPanel {}
impl EventEmitter<TabContentEvent> for DefaultAgentChatPanel {}

impl Focusable for DefaultAgentChatPanel {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for DefaultAgentChatPanel {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let muted_foreground = self
            .theme
            .as_ref()
            .map(|theme| theme.muted_foreground)
            .unwrap_or(cx.theme().muted_foreground);
        let background = self
            .theme
            .as_ref()
            .map(|theme| theme.background)
            .unwrap_or(cx.theme().background);
        div()
            .track_focus(&self.focus_handle)
            .size_full()
            .bg(background)
            .when_some(self.view.clone(), |this, view| this.child(view))
            .when(self.view.is_none(), |this| {
                this.child(
                    v_flex()
                        .size_full()
                        .items_center()
                        .justify_center()
                        .gap_2()
                        .text_color(muted_foreground)
                        .child(
                            self.error
                                .clone()
                                .unwrap_or_else(|| "Loading AI chat...".to_string()),
                        ),
                )
            })
    }
}

impl TabContent for DefaultAgentChatPanel {
    fn content_key(&self) -> &'static str {
        "DefaultAgentChat"
    }

    fn title(&self, _cx: &App) -> SharedString {
        SharedString::from(panel_title_for_mode(self.mode))
    }

    fn icon(&self, _cx: &App) -> Option<Icon> {
        Some(IconName::AI.color().with_size(Size::Medium))
    }

    fn closeable(&self, _cx: &App) -> bool {
        self.tab_closeable
    }

    fn dump(&self, _cx: &App) -> serde_json::Value {
        serde_json::json!({
            "version": 1,
        })
    }
}

fn load_enabled_provider_configs(
    storage: &one_core::storage::StorageManager,
) -> Vec<ProviderConfig> {
    let Some(repo) = storage.get::<ProviderRepository>() else {
        return Vec::new();
    };
    match repo.list() {
        Ok(configs) => enabled_provider_configs(configs),
        Err(error) => {
            tracing::warn!(%error, "Failed to load AI provider configs");
            Vec::new()
        }
    }
}

pub(crate) fn enabled_provider_configs(configs: Vec<ProviderConfig>) -> Vec<ProviderConfig> {
    configs
        .into_iter()
        .filter(|config| config.enabled)
        .collect()
}

pub(crate) fn build_sidebar_config(
    config: AgentChatViewConfig,
    acp_agents: Vec<AcpAgentEntry>,
) -> AgentChatViewConfig {
    config.sidebar_mode(true).with_acp_agents(acp_agents)
}

pub(crate) fn build_workbench_config(
    config: AgentChatViewConfig,
    acp_agents: Vec<AcpAgentEntry>,
) -> AgentChatViewConfig {
    config.sidebar_mode(false).with_acp_agents(acp_agents)
}

pub(crate) fn panel_title_for_mode(mode: DefaultAgentChatPanelMode) -> String {
    match mode {
        DefaultAgentChatPanelMode::Sidebar => t!("AgentUi.ai_chat").to_string(),
        DefaultAgentChatPanelMode::Workbench => t!("AgentUi.workbench").to_string(),
    }
}
