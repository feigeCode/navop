//! Agent 输入框:顶部能力区 + 多行输入 + `@` 提及 + 图片附件 + 底部发送栏。
//!
//! 布局参考 `agent-composer-design.html`:
//! - **顶部能力区**:计划 / Agent / 上下文;
//! - **附件条**:编辑器顶部的附件入口 + 图片缩略图(粘贴 / 附加);
//! - **编辑器**:基于 [`InputState`] 的多行自增高输入,注入 [`MentionCompletionProvider`] 实现 `@` 提及;
//! - **底部发送栏**:模型▾ / 发送。
//!
//! 设计原则:输入框是"哑组件",只接收 [`AgentComposerContext`] 做展示并在交互时 emit
//! [`AgentInputEvent`];目标用上层注入的列表渲染内置 popover(选中 emit `SelectTarget`),
//! scope 仅 emit `PickScope` 交上层;模型 / 工具 / 任务模式同样用注入选项渲染内置下拉。

use std::collections::HashSet;
use std::rc::Rc;
use std::sync::Arc;

use gpui::{
    App, AppContext, Context, Entity, EventEmitter, FocusHandle, Focusable, InteractiveElement,
    IntoElement, KeyDownEvent, ParentElement, PathPromptOptions, Render, SharedString,
    StatefulInteractiveElement, Styled, Subscription, Window, div, img, prelude::FluentBuilder, px,
};
use gpui_component::button::{Button, ButtonVariants};
use gpui_component::input::{Input, InputEvent, InputState};
use gpui_component::popover::Popover;
use gpui_component::{ActiveTheme, Disableable, Icon, IconName, Sizable, h_flex, v_flex};

use crate::input::attachment::ImageAttachment;
use crate::input::context::{
    AgentComposerContext, ComposerMenuOption, ComposerModelOption, ComposerPlanItem,
    ComposerResourcePoolItem, ComposerResourceSourceOption, ComposerResourceTypeFilter,
    ComposerScope, ComposerSubAgentItem, ComposerTarget,
};
use crate::input::mention::{MentionCompletionProvider, MentionItem};
use crate::theme::{AgentChatTheme, active_agent_chat_theme};

/// AgentInput 对外事件。
#[derive(Clone, Debug)]
pub enum AgentInputEvent {
    /// 用户提交一条消息。
    Submit {
        /// 文本内容(含 `@提及` 原文)。
        text: String,
        /// 文本中被引用到的提及条目。
        mentions: Vec<MentionItem>,
        /// 附带的图片。
        images: Vec<ImageAttachment>,
    },
    /// 用户请求停止当前运行。
    Stop,
    /// 在顶部目标下拉中选择了目标。
    SelectTarget { id: SharedString },
    /// 将资源加入本会话资源池。
    AddResourceToPool { id: SharedString },
    /// 将资源移出本会话资源池。
    RemoveResourceFromPool { id: SharedString },
    /// 选择资源池来源预设。
    SelectResourceSource { id: SharedString },
    /// 点击某个派生上下文 chip —— 上层据 `key` 弹出对应选择器。
    PickScope { key: SharedString },
    /// 在内置下拉中选择了模型。
    SelectModel {
        id: SharedString,
        provider_id: SharedString,
        model: SharedString,
    },
    /// 在内置下拉中选择了工具模式。
    SelectToolMode { id: SharedString },
    /// 在内置下拉中选择了任务模式。
    SelectTaskMode { id: SharedString },
    /// 在顶部「Agent」面板中选择内置 Agent 或 ACP Agent。
    SelectAgentBackend { id: Option<SharedString> },
}

/// 内置下拉的种类(用于受控开合状态)。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ComposerMenuKind {
    Target,
    Plan,
    SubAgent,
    Mode,
    Model,
}

const CONTEXT_POPOVER_WIDTH: f32 = 400.0;
const CONTEXT_TARGET_LIST_MAX_HEIGHT: f32 = 320.0;
const CONTEXT_KIND_MAX_WIDTH: f32 = 92.0;

fn current_task_label(label: &SharedString) -> SharedString {
    if label.is_empty() {
        SharedString::from("Auto Mode")
    } else {
        label.clone()
    }
}

fn toolbar_button_label(label: SharedString) -> impl IntoElement {
    h_flex()
        .w_full()
        .min_w_0()
        .items_center()
        .gap_1()
        .child(div().flex_1().min_w_0().truncate().child(label.to_string()))
        .child(Icon::new(IconName::ChevronDown).xsmall().flex_shrink_0())
}

fn menu_state_after_open_change(
    requested_open: bool,
    menu: ComposerMenuKind,
) -> Option<ComposerMenuKind> {
    requested_open.then_some(menu)
}

fn current_tool_label(label: &SharedString) -> SharedString {
    if label.is_empty() {
        SharedString::from("手动确认")
    } else {
        label.clone()
    }
}

fn execution_mode_trigger_label(
    task_label: &SharedString,
    tool_label: &SharedString,
) -> SharedString {
    SharedString::from(format!("{task_label} · {tool_label}"))
}

/// Agent 输入框组件。
pub struct AgentInput {
    focus_handle: FocusHandle,
    input_state: Entity<InputState>,
    /// 可被 `@` 引用的条目(同时用于补全 provider 与提交时解析)。
    mentions: Arc<Vec<MentionItem>>,
    /// 当前图片附件。
    attachments: Vec<ImageAttachment>,
    /// 是否正在运行(运行中显示「停止」)。
    is_running: bool,
    /// 上层注入的展示上下文(目标 / scope / 能力 / 模型 / 模式文案)。
    context: AgentComposerContext,
    /// 目标下拉选项(上层注入)。
    target_options: Vec<ComposerTarget>,
    /// 模型下拉选项(上层注入)。
    model_options: Vec<ComposerModelOption>,
    /// 工具模式下拉选项(上层注入)。
    tool_options: Vec<ComposerMenuOption>,
    /// 任务模式下拉选项(上层注入)。
    task_options: Vec<ComposerMenuOption>,
    /// 上下文面板的目标搜索框(与顶部输入框分离,避免抢焦点 / 拦截回车提交)。
    context_search_input: Entity<InputState>,
    /// 上下文面板当前搜索关键字(每次打开面板时重置为空)。
    context_search_query: SharedString,
    /// 当前资源类型筛选。`all` 表示显示全部资源。
    selected_resource_kind_filter: SharedString,
    /// 打开上下文面板时置位,下次 render 时据此清空搜索框(需 &mut Window)。
    context_search_needs_reset: bool,
    /// 当前展开的下拉(受控开合)。
    open_menu: Option<ComposerMenuKind>,
    /// 顶部计划面板中已展开的只读计划项。
    expanded_plan_items: HashSet<String>,
    /// 是否折叠顶部计划 / Agent / 上下文能力区。
    top_capabilities_collapsed: bool,
    /// 可选的局部聊天主题。终端侧边栏会注入终端主题,普通 Agent tab 继续使用应用主题。
    theme: Option<AgentChatTheme>,
    edge_to_edge: bool,
    _subscriptions: Vec<Subscription>,
}

impl AgentInput {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        Self::with_mentions(
            Vec::new(),
            "给 Agent 下达目标，输入 @ 引用资源…",
            window,
            cx,
        )
    }

    pub fn with_mentions(
        mentions: Vec<MentionItem>,
        placeholder: impl Into<SharedString>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let mentions = Arc::new(mentions);
        let placeholder = placeholder.into();
        let provider_items = (*mentions).clone();
        let input_state = cx.new(|cx| {
            let mut state = InputState::new(window, cx)
                .multi_line(true)
                .auto_grow(3, 10)
                .placeholder(placeholder);
            state.lsp.completion_provider =
                Some(Rc::new(MentionCompletionProvider::new(provider_items)));
            state
        });

        let enter_sub = cx.subscribe_in(&input_state, window, |this, _state, event, window, cx| {
            if let InputEvent::PressEnter { secondary } = event
                && !secondary
            {
                this.submit(window, cx);
            }
        });

        // 编辑器聚焦时,cmd/ctrl-v 会被 InputState 的 Paste action 先消费,外层
        // capture_key_down 不会触发(action 在 key_down 监听之前分发)。keystroke
        // 拦截器在 action 之前运行,用它在聚焦时把剪贴板图片补进附件(文本粘贴仍交给
        // InputState,二者互不影响)。
        let weak = cx.entity().downgrade();
        let paste_sub = cx.intercept_keystrokes(move |ev, window, cx| {
            if ev.keystroke.key != "v" || !ev.keystroke.modifiers.secondary() {
                return;
            }
            let Some(this) = weak.upgrade() else {
                return;
            };
            let input_state = this.read(cx).input_state.clone();
            if !input_state.read(cx).focus_handle(cx).is_focused(window) {
                return;
            }
            let atts = ImageAttachment::from_clipboard(cx);
            if atts.is_empty() {
                return;
            }
            this.update(cx, |this, cx| this.add_attachments(atts, cx));
        });

        let context_search_input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("搜索目标…")
                .clean_on_escape()
        });
        let context_search_sub = cx.subscribe_in(
            &context_search_input,
            window,
            |this: &mut Self,
             input: &Entity<InputState>,
             event: &InputEvent,
             _window,
             cx: &mut Context<Self>| {
                if let InputEvent::Change = event {
                    let query = input.read(cx).text().to_string();
                    this.context_search_query = SharedString::from(query);
                    cx.notify();
                }
            },
        );

        Self {
            focus_handle: cx.focus_handle(),
            input_state,
            mentions,
            attachments: Vec::new(),
            is_running: false,
            context: AgentComposerContext::default(),
            target_options: Vec::new(),
            model_options: Vec::new(),
            tool_options: Vec::new(),
            task_options: Vec::new(),
            context_search_input,
            context_search_query: SharedString::default(),
            selected_resource_kind_filter: SharedString::from("all"),
            context_search_needs_reset: false,
            open_menu: None,
            expanded_plan_items: HashSet::new(),
            top_capabilities_collapsed: false,
            theme: None,
            edge_to_edge: false,
            _subscriptions: vec![enter_sub, paste_sub, context_search_sub],
        }
    }

    pub fn set_theme(&mut self, theme: Option<AgentChatTheme>, cx: &mut Context<Self>) {
        self.theme = theme;
        cx.notify();
    }

    fn local_theme(&self, cx: &App) -> AgentChatTheme {
        self.theme
            .clone()
            .unwrap_or_else(|| AgentChatTheme::from_app(cx))
    }

    pub fn set_edge_to_edge(&mut self, enabled: bool, cx: &mut Context<Self>) {
        self.edge_to_edge = enabled;
        cx.notify();
    }

    /// 更新可引用的提及条目(同时刷新补全 provider)。
    pub fn set_mentions(&mut self, mentions: Vec<MentionItem>, cx: &mut Context<Self>) {
        self.mentions = Arc::new(mentions);
        let items = (*self.mentions).clone();
        self.input_state.update(cx, |state, _| {
            state.lsp.completion_provider = Some(Rc::new(MentionCompletionProvider::new(items)));
        });
    }

    /// 注入展示上下文(顶部 Context Bar + 底部模式/模型当前文案)。
    pub fn set_context(&mut self, context: AgentComposerContext, cx: &mut Context<Self>) {
        self.context = context;
        cx.notify();
    }

    /// 注入顶部目标下拉的选项。
    pub fn set_target_options(&mut self, options: Vec<ComposerTarget>, cx: &mut Context<Self>) {
        if options.is_empty() {
            self.selected_resource_kind_filter = SharedString::from("all");
        }
        self.target_options = options;
        cx.notify();
    }

    /// 注入底部三个内置下拉的选项。
    pub fn set_menu_options(
        &mut self,
        model_options: Vec<ComposerModelOption>,
        tool_options: Vec<ComposerMenuOption>,
        task_options: Vec<ComposerMenuOption>,
        cx: &mut Context<Self>,
    ) {
        self.model_options = model_options;
        self.tool_options = tool_options;
        self.task_options = task_options;
        cx.notify();
    }

    /// 设置运行状态(决定显示「发送」还是「停止」)。
    pub fn set_running(&mut self, running: bool, cx: &mut Context<Self>) {
        if self.is_running != running {
            self.is_running = running;
            if running {
                self.open_menu = None;
            }
            cx.notify();
        }
    }

    /// 聚焦输入框。
    pub fn focus_input(&self, window: &mut Window, cx: &mut App) {
        let handle = self.input_state.read(cx).focus_handle(cx);
        handle.focus(window, cx);
    }

    fn submit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.is_running {
            return;
        }
        let text = self.input_state.read(cx).value().to_string();
        let trimmed = text.trim();
        if trimmed.is_empty() && self.attachments.is_empty() {
            return;
        }

        let mentions = self.referenced_mentions(trimmed);
        let images = std::mem::take(&mut self.attachments);

        cx.emit(AgentInputEvent::Submit {
            text: trimmed.to_string(),
            mentions,
            images,
        });

        self.input_state.update(cx, |state, cx| {
            state.set_value("", window, cx);
        });
        cx.notify();
    }

    fn stop(&mut self, cx: &mut Context<Self>) {
        cx.emit(AgentInputEvent::Stop);
    }

    /// 文本中实际引用到的提及条目(其 `@label` 出现在文本里)。
    fn referenced_mentions(&self, text: &str) -> Vec<MentionItem> {
        referenced_mentions_in_text(text, self.mentions.as_ref())
    }

    fn add_attachments(&mut self, mut atts: Vec<ImageAttachment>, cx: &mut Context<Self>) {
        if atts.is_empty() {
            return;
        }
        self.attachments.append(&mut atts);
        cx.notify();
    }

    /// 从剪贴板读取图片并加入附件(cmd/ctrl-v 与按钮共用)。
    fn paste_images_from_clipboard(&mut self, cx: &mut Context<Self>) {
        let atts = ImageAttachment::from_clipboard(cx);
        self.add_attachments(atts, cx);
    }

    /// 打开系统文件对话框选择图片。
    fn open_file_picker(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        let rx = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: true,
            prompt: Some("选择图片".into()),
        });
        cx.spawn(async move |this, cx| {
            let Ok(Ok(Some(paths))) = rx.await else {
                return;
            };
            let atts: Vec<ImageAttachment> = paths
                .iter()
                .filter_map(|p| ImageAttachment::from_path(p))
                .collect();
            let _ = this.update(cx, |this, cx| this.add_attachments(atts, cx));
        })
        .detach();
    }

    fn remove_attachment(&mut self, id: &str, cx: &mut Context<Self>) {
        self.attachments.retain(|a| a.id != id);
        cx.notify();
    }

    fn render_context_bar(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let theme = self.local_theme(cx);

        v_flex()
            .w_full()
            .flex_shrink_0()
            .when(!self.top_capabilities_collapsed, |this| {
                this.border_b_1()
                    .border_color(theme.border)
                    .child(self.render_mode_tabs(cx))
            })
    }

    fn render_mode_tabs(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let theme = self.local_theme(cx);
        v_flex().w_full().px_3().pt_2().pb_1p5().child(
            h_flex()
                .w_full()
                .h(px(38.0))
                .items_center()
                .rounded(cx.theme().radius)
                .border_1()
                .border_color(theme.border)
                .bg(theme.panel)
                .child(self.render_plan_mode_tab(cx))
                .child(self.render_mode_separator(cx))
                .child(self.render_subagent_mode_tab(cx))
                .child(self.render_mode_separator(cx))
                .child(self.render_context_mode_tab(cx)),
        )
    }

    fn render_plan_mode_tab(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let view = cx.entity();
        let is_open = self.open_menu == Some(ComposerMenuKind::Plan);
        let plan_items = self.context.plan_items.clone();
        let expanded_items = self.expanded_plan_items.clone();
        let trigger_label = plan_trigger_label(&plan_items);

        Popover::new("agent-plan-popover")
            .p_0()
            .open(is_open)
            .on_open_change({
                let view = view.clone();
                move |open, _window, cx| {
                    let open = *open;
                    view.update(cx, |this, cx| {
                        this.open_menu = menu_state_after_open_change(open, ComposerMenuKind::Plan);
                        cx.notify();
                    });
                }
            })
            .trigger(self.render_capability_trigger(
                "agent-plan-trigger",
                trigger_label,
                IconName::Check,
                cx,
            ))
            .content({
                let view = view.clone();
                move |_state, _window, cx| {
                    render_plan_mode_content(
                        view.clone(),
                        plan_items.clone(),
                        expanded_items.clone(),
                        cx,
                    )
                }
            })
    }

    fn render_subagent_mode_tab(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let view = cx.entity();
        let is_open = self.open_menu == Some(ComposerMenuKind::SubAgent);
        let subagents = self.context.subagent_items.clone();
        let trigger_label = subagent_trigger_label(&subagents);

        Popover::new("agent-subagents-popover")
            .p_0()
            .open(is_open)
            .on_open_change({
                let view = view.clone();
                move |open, _window, cx| {
                    let open = *open;
                    view.update(cx, |this, cx| {
                        this.open_menu =
                            menu_state_after_open_change(open, ComposerMenuKind::SubAgent);
                        cx.notify();
                    });
                }
            })
            .trigger(self.render_capability_trigger(
                "agent-subagents-trigger",
                trigger_label,
                IconName::Bot,
                cx,
            ))
            .content({
                move |_state, _window, cx| render_subagent_mode_content(subagents.clone(), cx)
            })
    }

    fn render_capability_trigger(
        &self,
        id: &'static str,
        label: impl Into<SharedString>,
        icon: IconName,
        cx: &mut Context<Self>,
    ) -> Button {
        let label = label.into();
        let theme = self.local_theme(cx);
        Button::new(id)
            .debug_selector(move || id.to_string())
            .flex_1()
            .min_w_0()
            .h_full()
            .ghost()
            .small()
            .child(
                h_flex()
                    .min_w_0()
                    .items_center()
                    .justify_center()
                    .gap_1()
                    .text_color(theme.muted_foreground)
                    .child(Icon::new(icon).xsmall())
                    .child(div().text_sm().truncate().child(label)),
            )
    }

    fn render_context_mode_tab(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let view = cx.entity();
        let is_open = self.open_menu == Some(ComposerMenuKind::Target);
        let options = self.target_options.clone();
        let current = self.context.target.clone();
        let scopes = self.context.scopes.clone();
        let pool_items = self.context.resource_pool_items.clone();
        let source_options = self.context.resource_source_options.clone();
        let search_input = self.context_search_input.clone();
        let search_query = self.context_search_query.clone();
        let selected_kind = self.selected_resource_kind_filter.clone();
        let filters = self
            .context
            .resource_type_filters
            .iter()
            .cloned()
            .map(|mut filter| {
                filter.selected = filter.id == selected_kind;
                filter
            })
            .collect::<Vec<_>>();

        Popover::new("agent-context-mode-popover")
            .p_0()
            .open(is_open)
            .on_open_change({
                let view = view.clone();
                move |open, _window, cx| {
                    let open = *open;
                    view.update(cx, |this, cx| {
                        let became_open =
                            menu_state_after_open_change(open, ComposerMenuKind::Target);
                        this.open_menu = became_open;
                        // 标记在下次 render 时重置搜索框(render 持有 &mut Window,可安全 set_value)。
                        if became_open.is_some() {
                            this.context_search_needs_reset = true;
                        }
                        cx.notify();
                    });
                }
            })
            .trigger(self.render_context_mode_trigger(cx))
            .content({
                let view = view.clone();
                move |_state, _window, cx| {
                    render_context_mode_content(
                        view.clone(),
                        options.clone(),
                        current.clone(),
                        scopes.clone(),
                        pool_items.clone(),
                        source_options.clone(),
                        filters.clone(),
                        selected_kind.clone(),
                        search_input.clone(),
                        search_query.clone(),
                        cx,
                    )
                }
            })
    }

    fn render_context_mode_trigger(&self, cx: &mut Context<Self>) -> Button {
        let theme = self.local_theme(cx);
        let fg = if self.context.target.is_some() {
            theme.foreground
        } else {
            theme.muted_foreground
        };
        Button::new("agent-context-mode")
            .flex_1()
            .min_w_0()
            .h_full()
            .ghost()
            .small()
            .child(
                h_flex()
                    .min_w_0()
                    .items_center()
                    .justify_center()
                    .gap_1()
                    .text_color(fg)
                    .child(Icon::new(IconName::File).xsmall())
                    .child(
                        div()
                            .text_sm()
                            .truncate()
                            .child(resource_pool_trigger_label(&self.context)),
                    ),
            )
    }

    fn render_mode_separator(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let theme = self.local_theme(cx);
        div().h(px(20.0)).w(px(1.0)).bg(theme.border)
    }

    fn render_execution_mode_menu(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let view = cx.entity();
        let is_open = self.open_menu == Some(ComposerMenuKind::Mode);
        let task_label = current_task_label(&self.context.task_label);
        let tool_label = current_tool_label(&self.context.tool_label);
        let trigger_label = execution_mode_trigger_label(&task_label, &tool_label);
        let data = ModeContentData {
            task_label: task_label.clone(),
            tool_label,
            task_options: self.task_options.clone(),
            tool_options: self.tool_options.clone(),
        };

        let theme = self.local_theme(cx);
        let trigger = themed_outline_button(
            Button::new("agent-task-mode")
                .debug_selector(|| "agent-task-mode".to_string())
                .small()
                .w_full()
                .h_full()
                .justify_between()
                .outline()
                .disabled(self.is_running)
                .child(toolbar_button_label(trigger_label)),
            &theme,
        );

        Popover::new("agent-mode-popover")
            .p_0()
            .open(is_open)
            .on_open_change({
                let view = view.clone();
                move |open, _window, cx| {
                    let open = *open;
                    view.update(cx, |this, cx| {
                        this.open_menu = if open && !this.is_running {
                            Some(ComposerMenuKind::Mode)
                        } else {
                            None
                        };
                        cx.notify();
                    });
                }
            })
            .trigger(trigger)
            .content({
                let view = view.clone();
                let theme = theme.clone();
                move |_state, _window, cx| {
                    render_mode_content(view.clone(), data.clone(), &theme, cx)
                }
            })
    }

    fn render_model_menu(
        &self,
        trigger_label: SharedString,
        options: Vec<ComposerModelOption>,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let view = cx.entity();
        let is_open = self.open_menu == Some(ComposerMenuKind::Model);

        let theme = self.local_theme(cx);
        let trigger = themed_outline_button(
            Button::new("agent-model")
                .small()
                .w_full()
                .h_full()
                .justify_between()
                .outline()
                .disabled(self.is_running)
                .child(toolbar_button_label(trigger_label)),
            &theme,
        );

        Popover::new("agent-model-popover")
            .p_0()
            .open(is_open)
            .on_open_change({
                let view = view.clone();
                move |open, _window, cx| {
                    let open = *open;
                    view.update(cx, |this, cx| {
                        this.open_menu = if open && !this.is_running {
                            Some(ComposerMenuKind::Model)
                        } else {
                            None
                        };
                        cx.notify();
                    });
                }
            })
            .trigger(trigger)
            .content({
                let view = view.clone();
                let theme = theme.clone();
                move |_state, _window, cx| {
                    let muted = theme.muted_foreground;
                    let hover_bg = theme.hover_background();
                    let radius = cx.theme().radius;
                    let mut col = v_flex()
                        .p_1()
                        .gap(px(2.0))
                        .min_w(px(240.0))
                        .bg(theme.background)
                        .text_color(theme.foreground);
                    for opt in &options {
                        let view = view.clone();
                        let id = opt.id.clone();
                        let provider_id = opt.provider_id.clone();
                        let model = opt.model.clone();
                        let mut inner = v_flex()
                            .gap(px(1.0))
                            .child(div().text_sm().child(opt.display_label()));
                        if let Some(hint) = &opt.hint {
                            inner =
                                inner.child(div().text_xs().text_color(muted).child(hint.clone()));
                        }
                        col = col.child(
                            h_flex()
                                .id(SharedString::from(format!("agent-model-opt-{id}")))
                                .w_full()
                                .px_2()
                                .py_1()
                                .rounded(radius)
                                .cursor_pointer()
                                .hover(move |s| s.bg(hover_bg))
                                .child(inner)
                                .on_click(move |_, _window, cx| {
                                    let id = id.clone();
                                    let provider_id = provider_id.clone();
                                    let model = model.clone();
                                    view.update(cx, |this, cx| {
                                        if this.is_running {
                                            return;
                                        }
                                        this.open_menu = None;
                                        cx.emit(AgentInputEvent::SelectModel {
                                            id,
                                            provider_id,
                                            model,
                                        });
                                        cx.notify();
                                    });
                                }),
                        );
                    }
                    col
                }
            })
    }

    fn render_attachments(&self, cx: &mut Context<Self>) -> Option<gpui::AnyElement> {
        if self.attachments.is_empty() {
            return None;
        }
        let theme = self.local_theme(cx);
        let thumbs: Vec<gpui::AnyElement> = self
            .attachments
            .iter()
            .map(|att| {
                let id = att.id.clone();
                div()
                    .relative()
                    .child(
                        img(att.image.clone())
                            .w(px(56.0))
                            .h(px(56.0))
                            .rounded(cx.theme().radius)
                            .border_1()
                            .border_color(theme.border),
                    )
                    .child(
                        div().absolute().top_0().right_0().child(
                            Button::new(SharedString::from(format!("rm-att-{id}")))
                                .icon(IconName::Close)
                                .ghost()
                                .xsmall()
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.remove_attachment(&id, cx);
                                })),
                        ),
                    )
                    .into_any_element()
            })
            .collect();

        Some(
            h_flex()
                .w_full()
                .flex_wrap()
                .gap_2()
                .px_3()
                .pt_2()
                .children(thumbs)
                .into_any_element(),
        )
    }

    fn render_editor_top_bar(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let theme = self.local_theme(cx);
        let muted = theme.muted_foreground;
        let count = self.attachments.len();

        h_flex()
            .w_full()
            .items_center()
            .justify_between()
            .px_3()
            .pt_2()
            .pb_1()
            .child(
                h_flex()
                    .items_center()
                    .gap_1()
                    .child(
                        Button::new("agent-attach")
                            .icon(IconName::File)
                            .ghost()
                            .small()
                            .tooltip("附加图片")
                            .on_click(
                                cx.listener(|this, _, window, cx| {
                                    this.open_file_picker(window, cx)
                                }),
                            ),
                    )
                    .child(Icon::new(IconName::LoaderCircle).xsmall().text_color(muted))
                    .child(
                        div()
                            .text_xs()
                            .text_color(muted)
                            .child(format!("{count} 个附件")),
                    )
                    .child(div().h(px(18.0)).w(px(1.0)).bg(theme.border)),
            )
            .child(
                h_flex()
                    .items_center()
                    .gap_0p5()
                    .child(
                        Button::new("agent-editor-menu")
                            .icon(if self.top_capabilities_collapsed {
                                IconName::ChevronUp
                            } else {
                                IconName::ChevronDown
                            })
                            .ghost()
                            .small()
                            .tooltip("折叠能力区")
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.top_capabilities_collapsed = !this.top_capabilities_collapsed;
                                if this.top_capabilities_collapsed {
                                    this.open_menu = None;
                                }
                                cx.notify();
                            })),
                    )
                    .child(
                        Button::new("agent-editor-undo")
                            .icon(IconName::Undo)
                            .ghost()
                            .small()
                            .tooltip("撤销"),
                    ),
            )
    }

    fn render_toolbar(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let theme = self.local_theme(cx);
        let running = self.is_running;
        let model_label = match &self.context.model {
            Some(m) => SharedString::from(format!("{} / {}", m.provider, m.model)),
            None => SharedString::from("选择模型"),
        };
        let run_button = if running {
            Button::new("agent-stop")
                .icon(IconName::Close)
                .danger()
                .small()
                .tooltip("停止")
                .on_click(cx.listener(|this, _, _, cx| this.stop(cx)))
        } else {
            Button::new("agent-send")
                .icon(IconName::ArrowUp)
                .primary()
                .small()
                .tooltip("发送")
                .on_click(cx.listener(|this, _, window, cx| this.submit(window, cx)))
        };

        h_flex()
            .w_full()
            .items_center()
            .text_color(theme.foreground)
            .gap_2()
            .px_3()
            .py_2()
            .flex_shrink_0()
            .child(
                div()
                    .w(px(124.0))
                    .h(px(32.0))
                    .flex_shrink_0()
                    .overflow_hidden()
                    .child(self.render_execution_mode_menu(cx)),
            )
            .child(
                div()
                    .flex_1()
                    .min_w(px(150.0))
                    .h(px(32.0))
                    .overflow_hidden()
                    .child(self.render_model_menu(model_label, self.model_options.clone(), cx))
                    .debug_selector(|| "agent-input-model-control".to_string()),
            )
            .child(
                div()
                    .w(px(34.0))
                    .h(px(32.0))
                    .flex_shrink_0()
                    .debug_selector(|| "agent-input-send-control".to_string())
                    .child(run_button),
            )
    }
}

fn referenced_mentions_in_text(text: &str, mentions: &[MentionItem]) -> Vec<MentionItem> {
    mentions
        .iter()
        .filter(|mention| text_contains_mention(text, &mention.label))
        .cloned()
        .collect()
}

fn themed_outline_button(button: Button, theme: &AgentChatTheme) -> Button {
    button
        .bg(theme.panel)
        .border_color(theme.border)
        .text_color(theme.foreground)
}

fn text_contains_mention(text: &str, label: &str) -> bool {
    mention_match_end(text, &format!("@{label}"))
        || text.contains(&format!("@`{label}`"))
        || text.contains(&format!("@\"{label}\""))
}

fn mention_match_end(text: &str, needle: &str) -> bool {
    let mut start = 0;
    while let Some(offset) = text[start..].find(needle) {
        let end = start + offset + needle.len();
        if text[end..]
            .chars()
            .next()
            .is_none_or(|ch| !is_mention_name_char(ch))
        {
            return true;
        }
        start = end;
    }
    false
}

fn is_mention_name_char(ch: char) -> bool {
    ch.is_alphanumeric() || matches!(ch, '_' | '-')
}

impl EventEmitter<AgentInputEvent> for AgentInput {}

#[derive(Clone)]
struct ModeContentData {
    task_label: SharedString,
    tool_label: SharedString,
    task_options: Vec<ComposerMenuOption>,
    tool_options: Vec<ComposerMenuOption>,
}

struct ModeOptionRow {
    id_prefix: &'static str,
    option: ComposerMenuOption,
    selected: bool,
    event: ModeOptionEvent,
}

#[derive(Clone, Copy)]
enum ModeOptionEvent {
    Task,
    Tool,
}

fn render_mode_content(
    view: Entity<AgentInput>,
    data: ModeContentData,
    theme: &AgentChatTheme,
    cx: &mut Context<gpui_component::popover::PopoverState>,
) -> gpui::AnyElement {
    let mut col = v_flex()
        .p_1()
        .gap(px(2.0))
        .min_w(px(320.0))
        .bg(theme.background)
        .text_color(theme.foreground);

    col = col.child(context_group_label("响应模式", theme));
    for option in data.task_options {
        let selected = option.label == data.task_label;
        col = col.child(mode_option_row(
            view.clone(),
            ModeOptionRow {
                id_prefix: "agent-task-mode",
                option,
                selected,
                event: ModeOptionEvent::Task,
            },
            theme,
            cx,
        ));
    }

    col = col.child(context_group_label("工具执行确认", theme));
    for option in data.tool_options {
        let selected = option.label == data.tool_label;
        col = col.child(mode_option_row(
            view.clone(),
            ModeOptionRow {
                id_prefix: "agent-tool-mode",
                option,
                selected,
                event: ModeOptionEvent::Tool,
            },
            theme,
            cx,
        ));
    }

    col.into_any_element()
}

fn mode_option_row(
    view: Entity<AgentInput>,
    row: ModeOptionRow,
    theme: &AgentChatTheme,
    cx: &mut Context<gpui_component::popover::PopoverState>,
) -> gpui::AnyElement {
    let muted = theme.muted_foreground;
    let hover_bg = theme.hover_background();
    let selected_bg = theme.selection_background();
    let selected_fg = theme.accent;
    let id = row.option.id.clone();
    let row_id = SharedString::from(format!("{}-opt-{id}", row.id_prefix));
    let mut inner = v_flex()
        .flex_1()
        .min_w_0()
        .gap(px(1.0))
        .child(div().text_sm().truncate().child(row.option.label));
    if let Some(hint) = row.option.hint {
        inner = inner.child(div().text_xs().text_color(muted).child(hint));
    }

    h_flex()
        .id(row_id)
        .w_full()
        .items_center()
        .gap_2()
        .px_2()
        .py_1p5()
        .rounded(cx.theme().radius)
        .cursor_pointer()
        .when(row.selected, |this| {
            this.bg(selected_bg).text_color(theme.foreground)
        })
        .hover(move |this| this.bg(hover_bg))
        .child(inner)
        .when(row.selected, |this| {
            this.child(Icon::new(IconName::Check).xsmall().text_color(selected_fg))
        })
        .on_click(move |_, _window, cx| {
            let id = id.clone();
            view.update(cx, |this, cx| {
                if this.is_running {
                    return;
                }
                this.open_menu = None;
                cx.emit(mode_option_event(row.event, id));
                cx.notify();
            });
        })
        .into_any_element()
}

fn mode_option_event(event: ModeOptionEvent, id: SharedString) -> AgentInputEvent {
    match event {
        ModeOptionEvent::Task => AgentInputEvent::SelectTaskMode { id },
        ModeOptionEvent::Tool => AgentInputEvent::SelectToolMode { id },
    }
}

fn render_plan_mode_content(
    view: Entity<AgentInput>,
    items: Vec<ComposerPlanItem>,
    expanded_items: HashSet<String>,
    cx: &mut Context<gpui_component::popover::PopoverState>,
) -> gpui::AnyElement {
    let theme = active_agent_chat_theme(cx);
    let muted = theme.muted_foreground;
    let border = theme.border;
    let mut col = v_flex().p_1().gap(px(2.0)).min_w(px(320.0));

    col = col.child(context_group_label("计划 Todo", &theme));
    if items.is_empty() {
        return col
            .child(
                div()
                    .px_2()
                    .py_2()
                    .text_sm()
                    .text_color(muted)
                    .child("暂无计划"),
            )
            .into_any_element();
    }

    for (ix, item) in items.into_iter().enumerate() {
        let key = plan_item_key(ix, &item);
        let expanded = expanded_items.contains(&key);
        col = col.child(plan_item_row(
            view.clone(),
            item,
            key,
            expanded,
            muted,
            border,
            &theme,
            cx,
        ));
    }
    col.into_any_element()
}

fn plan_trigger_label(items: &[ComposerPlanItem]) -> SharedString {
    if items.is_empty() {
        return SharedString::from("计划");
    }
    let total = items.len();
    let completed = items
        .iter()
        .filter(|item| is_completed_plan_status(item.status.as_ref()))
        .count();
    let label = if completed == total {
        "完成"
    } else if items
        .iter()
        .any(|item| is_running_plan_status(item.status.as_ref()))
    {
        "进行中"
    } else {
        "待执行"
    };
    SharedString::from(format!("{completed}/{total} {label}"))
}

fn is_completed_plan_status(status: &str) -> bool {
    status == "completed"
}

fn is_running_plan_status(status: &str) -> bool {
    matches!(status, "running" | "in_progress")
}

fn plan_item_row(
    view: Entity<AgentInput>,
    item: ComposerPlanItem,
    key: String,
    expanded: bool,
    muted: gpui::Hsla,
    border: gpui::Hsla,
    theme: &AgentChatTheme,
    cx: &mut Context<gpui_component::popover::PopoverState>,
) -> gpui::AnyElement {
    let icon = match item.status.as_ref() {
        "completed" => IconName::CircleCheck,
        "running" | "in_progress" => IconName::LoaderCircle,
        _ => IconName::CircleCheck,
    };
    let icon_color = match item.status.as_ref() {
        "completed" => cx.theme().success,
        "running" | "in_progress" => cx.theme().warning,
        _ => muted,
    };

    let has_details = item.has_details();
    let hover_bg = theme.hover_background();
    let radius = cx.theme().radius;
    let row_id = SharedString::from(format!("agent-plan-item-{key}"));
    let row = h_flex()
        .id(row_id)
        .w_full()
        .items_center()
        .gap_2()
        .px_2()
        .py_1()
        .border_b_1()
        .border_color(border)
        .when(has_details, |this| {
            this.cursor_pointer()
                .rounded(radius)
                .hover(move |s| s.bg(hover_bg))
                .on_click(move |_, _window, cx| {
                    let key = key.clone();
                    view.update(cx, |this, cx| {
                        if !this.expanded_plan_items.insert(key.clone()) {
                            this.expanded_plan_items.remove(&key);
                        }
                        cx.notify();
                    });
                })
        })
        .child(Icon::new(icon).xsmall().text_color(icon_color))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .text_sm()
                .truncate()
                .child(item.title.clone()),
        )
        .child(
            div()
                .text_xs()
                .text_color(muted)
                .child(plan_status_label(item.status.as_ref())),
        )
        .when(has_details, |this| {
            this.child(
                Icon::new(if expanded {
                    IconName::ChevronDown
                } else {
                    IconName::ChevronRight
                })
                .xsmall()
                .text_color(muted),
            )
        });

    let mut wrapper = v_flex().w_full().child(row);
    if expanded {
        wrapper = wrapper.child(plan_item_details(item, muted, cx));
    }
    wrapper.into_any_element()
}

fn plan_item_key(index: usize, item: &ComposerPlanItem) -> String {
    format!("{index}:{}:{}", item.title, item.status)
}

fn plan_item_details(
    item: ComposerPlanItem,
    muted: gpui::Hsla,
    cx: &mut Context<gpui_component::popover::PopoverState>,
) -> gpui::AnyElement {
    let mut details = v_flex()
        .w_full()
        .gap(px(2.0))
        .px_2()
        .pb_2()
        .pl_7()
        .border_b_1()
        .border_color(cx.theme().border);

    if !item.description.is_empty() {
        details = details.child(
            div()
                .text_xs()
                .text_color(cx.theme().foreground)
                .child(item.description),
        );
    }
    if !item.risk.is_empty() {
        details = details.child(plan_detail_line("风险", item.risk, muted));
    }
    if let Some(tool) = item.tool {
        details = details.child(plan_detail_line("工具", tool, muted));
    }
    details.into_any_element()
}

fn plan_detail_line(
    label: &'static str,
    value: SharedString,
    muted: gpui::Hsla,
) -> gpui::AnyElement {
    h_flex()
        .w_full()
        .min_w_0()
        .gap_1()
        .text_xs()
        .child(div().flex_shrink_0().text_color(muted).child(label))
        .child(div().flex_1().min_w_0().truncate().child(value))
        .into_any_element()
}

fn plan_status_label(status: &str) -> &'static str {
    match status {
        "completed" => "已完成",
        "running" | "in_progress" => "进行中",
        "failed" => "失败",
        "cancelled" => "已取消",
        _ => "待执行",
    }
}

fn render_subagent_mode_content(
    subagents: Vec<ComposerSubAgentItem>,
    cx: &mut Context<gpui_component::popover::PopoverState>,
) -> gpui::AnyElement {
    let theme = active_agent_chat_theme(cx);
    let muted = theme.muted_foreground;
    let mut col = v_flex().p_1().gap(px(2.0)).min_w(px(300.0));

    col = col.child(context_group_label("子代理", &theme));
    if subagents.is_empty() {
        col = col.child(
            div()
                .px_2()
                .py_2()
                .text_sm()
                .text_color(muted)
                .child("暂无子代理"),
        );
    }
    for subagent in subagents {
        col = col.child(subagent_item_row(subagent, muted, cx));
    }
    col.into_any_element()
}

fn subagent_trigger_label(subagents: &[ComposerSubAgentItem]) -> SharedString {
    if subagents.is_empty() {
        SharedString::from("子代理")
    } else {
        SharedString::from(format!("子代理 · {}", subagents.len()))
    }
}

fn resource_pool_trigger_label(context: &AgentComposerContext) -> SharedString {
    if context.resource_pool.total_resources == 0 {
        return SharedString::from("资源池");
    }
    SharedString::from(format!(
        "资源池 · {}",
        context.resource_pool.total_resources
    ))
}

fn subagent_item_row(
    item: ComposerSubAgentItem,
    muted: gpui::Hsla,
    cx: &mut Context<gpui_component::popover::PopoverState>,
) -> gpui::AnyElement {
    let (icon, icon_color) = subagent_item_status_style(item.status.as_ref(), muted, cx);
    h_flex()
        .id(SharedString::from(format!(
            "agent-running-subagent-{}",
            item.id
        )))
        .w_full()
        .items_center()
        .gap_2()
        .px_2()
        .py_1p5()
        .child(Icon::new(icon).xsmall().text_color(icon_color))
        .child(
            v_flex()
                .flex_1()
                .min_w_0()
                .gap(px(1.0))
                .child(div().text_sm().truncate().child(item.name))
                .child(
                    div()
                        .text_xs()
                        .text_color(muted)
                        .truncate()
                        .child(format!("用途: {}", item.task)),
                )
                .when(!item.summary.is_empty(), |this| {
                    this.child(
                        div()
                            .text_xs()
                            .text_color(muted)
                            .truncate()
                            .child(format!("进展: {}", item.summary)),
                    )
                }),
        )
        .child(
            div()
                .text_xs()
                .text_color(muted)
                .child(plan_status_label(item.status.as_ref())),
        )
        .into_any_element()
}

fn subagent_item_status_style(
    status: &str,
    muted: gpui::Hsla,
    cx: &mut Context<gpui_component::popover::PopoverState>,
) -> (IconName, gpui::Hsla) {
    match status {
        "completed" => (IconName::CircleCheck, cx.theme().success),
        "failed" => (IconName::CircleX, cx.theme().danger),
        "running" | "in_progress" => (IconName::LoaderCircle, cx.theme().warning),
        _ => (IconName::Bot, muted),
    }
}

fn render_context_mode_content(
    view: Entity<AgentInput>,
    options: Vec<ComposerTarget>,
    current: Option<ComposerTarget>,
    scopes: Vec<ComposerScope>,
    pool_items: Vec<ComposerResourcePoolItem>,
    source_options: Vec<ComposerResourceSourceOption>,
    filters: Vec<ComposerResourceTypeFilter>,
    selected_kind: SharedString,
    search_input: Entity<InputState>,
    search_query: SharedString,
    cx: &mut Context<gpui_component::popover::PopoverState>,
) -> gpui::AnyElement {
    let theme = active_agent_chat_theme(cx);
    let muted = theme.muted_foreground;
    let border = theme.border;
    let mut col = v_flex()
        .debug_selector(|| "agent-context-popover-content".to_string())
        .w(px(CONTEXT_POPOVER_WIDTH))
        .min_w(px(CONTEXT_POPOVER_WIDTH))
        .overflow_x_hidden();

    if let Some(target) = current {
        col = col
            .px_1()
            .pt_1()
            .child(context_group_label("默认目标", &theme))
            .child(context_summary_row(target, muted, &theme));
    }
    let has_database_scope = scopes.iter().any(|scope| scope.key.as_ref() == "database");
    if !scopes.is_empty() {
        col = col.px_1().child(context_group_label("作用域", &theme));
        for scope in scopes {
            col = col.child(context_scope_row(
                view.clone(),
                scope,
                muted,
                border,
                &theme,
                cx,
            ));
        }
        if has_database_scope {
            col = col.child(context_database_hint(muted, cx));
        }
    }

    if !source_options.is_empty() {
        col = col.child(render_resource_source_options(
            view.clone(),
            source_options,
            &theme,
            cx,
        ));
    }

    if !filters.is_empty() {
        col = col.child(render_resource_type_filters(
            view.clone(),
            filters,
            &theme,
            cx,
        ));
    }

    // 搜索框:固定在列表上方,不参与滚动,避免长列表里输入框被滚出可视区。
    col = col.child(
        div()
            .debug_selector(|| "context-target-search".to_string())
            .w_full()
            .min_w_0()
            .px_1()
            .pb_1()
            .child(
                Input::new(&search_input)
                    .prefix(Icon::new(IconName::Search).text_color(muted))
                    .cleanable(true)
                    .small()
                    .w_full(),
            ),
    );

    // 按类型和关键字过滤资源(label / subtitle / kind 不区分大小写)。
    let needle = normalize_target_search_query(search_query.as_ref());
    let filtered_pool_items = filter_pool_items(pool_items, selected_kind.as_ref(), &needle);
    let filtered_targets = if filtered_pool_items.is_empty() {
        let kind_filtered = filter_targets_by_kind(options, selected_kind.as_ref());
        kind_filtered
            .into_iter()
            .filter(|opt| needle.is_empty() || target_matches(opt, &needle))
            .collect::<Vec<_>>()
    } else {
        Vec::new()
    };
    let filtered_count = if filtered_pool_items.is_empty() {
        filtered_targets.len()
    } else {
        filtered_pool_items.len()
    };

    col = col.child(
        div()
            .w_full()
            .min_w_0()
            .px_1()
            .text_xs()
            .text_color(muted)
            .child(filter_result_label(filtered_count, &search_query)),
    );

    // 列表区:限定最大高度并内部滚动,避免目标多时撑爆 popover。
    let mut list = v_flex()
        .id("context-target-list")
        .w_full()
        .px_1()
        .pb_1()
        .gap(px(2.0))
        .max_h(px(CONTEXT_TARGET_LIST_MAX_HEIGHT))
        .overflow_x_hidden()
        .overflow_y_scroll();
    if filtered_pool_items.is_empty() && filtered_targets.is_empty() {
        list = list.child(div().px_2().py_2().text_sm().text_color(muted).child(
            if search_query.is_empty() {
                "资源池为空"
            } else {
                "未匹配到资源"
            },
        ));
    }
    if filtered_pool_items.is_empty() {
        for opt in filtered_targets {
            list = list.child(context_target_option(view.clone(), opt, muted, &theme, cx));
        }
    } else {
        for item in filtered_pool_items {
            list = list.child(resource_pool_item_row(
                view.clone(),
                item,
                muted,
                &theme,
                cx,
            ));
        }
    }

    col = col.child(list);
    col.into_any_element()
}

fn render_resource_source_options(
    view: Entity<AgentInput>,
    options: Vec<ComposerResourceSourceOption>,
    theme: &AgentChatTheme,
    _cx: &mut Context<gpui_component::popover::PopoverState>,
) -> gpui::AnyElement {
    let selected_bg = theme.selection_background();
    let selected_fg = theme.foreground;
    let muted = theme.muted_foreground;
    let hover_bg = theme.hover_background();
    let mut row = h_flex().w_full().px_1().pb_1().gap(px(4.0)).flex_wrap();

    for option in options.into_iter().filter(|option| option.enabled) {
        let id = option.id.clone();
        let selected = option.selected;
        let enabled = option.enabled;
        let label = resource_source_option_label(&option);
        let view = view.clone();
        row = row.child(
            h_flex()
                .id(option.element_id())
                .items_center()
                .gap(px(4.0))
                .px_2()
                .py_1()
                .rounded_sm()
                .text_xs()
                .when(selected, |this| {
                    this.bg(selected_bg).text_color(selected_fg)
                })
                .when(!selected, |this| this.text_color(muted))
                .when(enabled && !selected, |this| {
                    this.cursor_pointer().hover(move |s| s.bg(hover_bg))
                })
                .when(!enabled, |this| this.opacity(0.5))
                .child(label)
                .when(enabled, |this| {
                    this.on_click(move |_, _window, cx| {
                        let id = id.clone();
                        view.update(cx, |this, cx| {
                            if this.is_running {
                                return;
                            }
                            cx.emit(AgentInputEvent::SelectResourceSource { id });
                            cx.notify();
                        });
                    })
                }),
        );
    }

    row.into_any_element()
}

fn render_resource_type_filters(
    view: Entity<AgentInput>,
    filters: Vec<ComposerResourceTypeFilter>,
    theme: &AgentChatTheme,
    _cx: &mut Context<gpui_component::popover::PopoverState>,
) -> gpui::AnyElement {
    let hover_bg = theme.hover_background();
    let selected_bg = theme.selection_background();
    let selected_fg = theme.foreground;
    let muted = theme.muted_foreground;
    let mut row = h_flex().w_full().px_1().pb_1().gap(px(4.0));

    for filter in filters {
        let id = filter.id.clone();
        let selected = filter.selected;
        let view = view.clone();
        row = row.child(
            h_flex()
                .id(filter.element_id())
                .items_center()
                .gap(px(4.0))
                .px_2()
                .py_1()
                .rounded_sm()
                .text_xs()
                .when(selected, |this| {
                    this.bg(selected_bg).text_color(selected_fg)
                })
                .when(!selected, |this| {
                    this.text_color(muted).hover(move |s| s.bg(hover_bg))
                })
                .child(filter.label)
                .child(format!("{}", filter.count))
                .on_click(move |_, _window, cx| {
                    let id = id.clone();
                    view.update(cx, |this, cx| {
                        this.selected_resource_kind_filter = id;
                        cx.notify();
                    });
                }),
        );
    }

    row.into_any_element()
}

fn resource_pool_item_row(
    view: Entity<AgentInput>,
    item: ComposerResourcePoolItem,
    muted: gpui::Hsla,
    theme: &AgentChatTheme,
    cx: &mut Context<gpui_component::popover::PopoverState>,
) -> gpui::AnyElement {
    let hover_bg = theme.hover_background();
    let radius = cx.theme().radius;
    let id = item.id.clone();
    let action_id = item.id.clone();
    let action_label = resource_pool_action_label(&item);
    let action_view = view.clone();
    let add_action_id = action_id.clone();
    let add_action_view = action_view.clone();
    let in_pool = item.in_pool;
    let is_default = item.is_default;
    let action_button = Button::new(SharedString::from(format!(
        "resource-pool-action-{}",
        action_id
    )))
    .ghost()
    .xsmall()
    .label(action_label)
    .disabled(is_default);
    let action_button = if in_pool && !is_default {
        action_button.on_click(move |_, _window, cx| {
            let id = action_id.clone();
            action_view.update(cx, |this, cx| {
                if this.is_running {
                    return;
                }
                cx.emit(AgentInputEvent::RemoveResourceFromPool { id });
                cx.notify();
            });
        })
    } else if !in_pool {
        action_button.on_click(move |_, _window, cx| {
            let id = add_action_id.clone();
            add_action_view.update(cx, |this, cx| {
                if this.is_running {
                    return;
                }
                cx.emit(AgentInputEvent::AddResourceToPool { id });
                cx.notify();
            });
        })
    } else {
        action_button
    };

    h_flex()
        .id(item.element_id())
        .w_full()
        .min_w_0()
        .items_center()
        .gap(px(8.0))
        .px_2()
        .py_1()
        .rounded(radius)
        .cursor_pointer()
        .hover(move |s| s.bg(hover_bg))
        .on_click(move |_, _window, cx| {
            let id = id.clone();
            view.update(cx, |this, cx| {
                if this.is_running {
                    return;
                }
                if in_pool {
                    this.open_menu = None;
                    cx.emit(AgentInputEvent::SelectTarget { id });
                } else {
                    cx.emit(AgentInputEvent::AddResourceToPool { id });
                }
                cx.notify();
            });
        })
        .child(
            h_flex()
                .flex_shrink_0()
                .items_center()
                .justify_center()
                .size(px(24.0))
                .rounded(radius)
                .bg(hover_bg)
                .text_xs()
                .child(item.icon),
        )
        .child(
            v_flex()
                .flex_1()
                .min_w_0()
                .gap(px(1.0))
                .child(
                    div()
                        .w_full()
                        .min_w_0()
                        .text_sm()
                        .truncate()
                        .child(item.label),
                )
                .child(
                    div()
                        .w_full()
                        .min_w_0()
                        .text_xs()
                        .text_color(muted)
                        .truncate()
                        .child(item.subtitle),
                ),
        )
        .child(
            div()
                .flex_shrink_0()
                .max_w(px(CONTEXT_KIND_MAX_WIDTH))
                .text_xs()
                .text_color(muted)
                .truncate()
                .child(item.kind),
        )
        .child(action_button)
        .into_any_element()
}

/// 目标是否匹配搜索关键字(子串匹配,忽略大小写)。
fn target_matches(opt: &ComposerTarget, needle: &str) -> bool {
    let needle = normalize_target_search_query(needle);
    opt.label.to_lowercase().contains(&needle)
        || opt.subtitle.to_lowercase().contains(&needle)
        || opt.kind.to_lowercase().contains(&needle)
}

fn normalize_target_search_query(query: &str) -> String {
    query.trim().to_lowercase()
}

fn filter_targets_by_kind(targets: Vec<ComposerTarget>, kind: &str) -> Vec<ComposerTarget> {
    if kind == "all" {
        return targets;
    }
    targets
        .into_iter()
        .filter(|target| target.kind.as_ref() == kind)
        .collect()
}

fn filter_pool_items(
    items: Vec<ComposerResourcePoolItem>,
    kind: &str,
    needle: &str,
) -> Vec<ComposerResourcePoolItem> {
    items
        .into_iter()
        .filter(|item| kind == "all" || item.kind.as_ref() == kind)
        .filter(|item| {
            needle.is_empty()
                || item.label.to_lowercase().contains(needle)
                || item.subtitle.to_lowercase().contains(needle)
                || item.kind.to_lowercase().contains(needle)
        })
        .collect()
}

fn resource_pool_action_label(item: &ComposerResourcePoolItem) -> &'static str {
    if item.is_default {
        "默认"
    } else if item.in_pool {
        "-"
    } else {
        "+"
    }
}

fn resource_source_option_label(option: &ComposerResourceSourceOption) -> SharedString {
    if !option.enabled {
        return option
            .hint
            .as_ref()
            .map(|hint| format!("{} · {}", option.label, hint))
            .unwrap_or_else(|| option.label.to_string())
            .into();
    }
    SharedString::from(format!("{} {}", option.label, option.count))
}

/// 列表结果计数文案:有关键字时展示匹配数,无关键字时展示总数。
fn filter_result_label(filtered_count: usize, query: &SharedString) -> SharedString {
    if query.is_empty() {
        return SharedString::default();
    }
    SharedString::from(format!("匹配到 {filtered_count} 个资源"))
}

fn context_database_hint(
    muted: gpui::Hsla,
    _cx: &mut Context<gpui_component::popover::PopoverState>,
) -> gpui::AnyElement {
    div()
        .px_2()
        .py_1()
        .text_xs()
        .text_color(muted)
        .child("如需切换数据库,请在数据库侧边栏中点击目标数据库。")
        .into_any_element()
}

fn context_group_label(label: &'static str, theme: &AgentChatTheme) -> gpui::AnyElement {
    div()
        .px_2()
        .pt_1()
        .text_xs()
        .text_color(theme.muted_foreground)
        .child(label)
        .into_any_element()
}

fn context_summary_row(
    target: ComposerTarget,
    muted: gpui::Hsla,
    theme: &AgentChatTheme,
) -> gpui::AnyElement {
    context_target_row(target, muted, theme, true)
        .debug_selector(|| "context-current-summary".to_string())
        .into_any_element()
}

fn context_target_row(
    opt: ComposerTarget,
    muted: gpui::Hsla,
    theme: &AgentChatTheme,
    selected: bool,
) -> gpui::Div {
    let hover_bg = theme.hover_background();
    let selected_bg = theme.selection_background();

    h_flex()
        .w_full()
        .min_w_0()
        .items_center()
        .gap(px(8.0))
        .px_2()
        .py_1()
        .rounded(px(6.0))
        .when(selected, |this| this.bg(selected_bg))
        .child(
            h_flex()
                .flex_shrink_0()
                .items_center()
                .justify_center()
                .size(px(24.0))
                .rounded(px(6.0))
                .bg(hover_bg)
                .text_xs()
                .child(opt.icon),
        )
        .child(
            v_flex()
                .flex_1()
                .min_w_0()
                .gap(px(1.0))
                .child(
                    div()
                        .w_full()
                        .min_w_0()
                        .text_sm()
                        .truncate()
                        .child(opt.label),
                )
                .child(
                    div()
                        .w_full()
                        .min_w_0()
                        .text_xs()
                        .text_color(muted)
                        .truncate()
                        .child(opt.subtitle),
                ),
        )
        .child(
            div()
                .flex_shrink_0()
                .max_w(px(CONTEXT_KIND_MAX_WIDTH))
                .text_xs()
                .text_color(muted)
                .truncate()
                .child(opt.kind),
        )
}

fn context_scope_row(
    view: Entity<AgentInput>,
    scope: ComposerScope,
    muted: gpui::Hsla,
    border: gpui::Hsla,
    theme: &AgentChatTheme,
    cx: &mut Context<gpui_component::popover::PopoverState>,
) -> gpui::AnyElement {
    let key = scope.key.clone();
    let hover_bg = theme.hover_background();
    let radius = cx.theme().radius;

    h_flex()
        .id(SharedString::from(format!("context-scope-{key}")))
        .items_center()
        .justify_between()
        .gap_2()
        .px_2()
        .py_1()
        .rounded(radius)
        .border_b_1()
        .border_color(border)
        .cursor_pointer()
        .hover(move |s| s.bg(hover_bg))
        .child(div().text_xs().text_color(muted).child(scope.label))
        .child(div().text_xs().child(scope.value))
        .on_click(move |_, _window, cx| {
            let key = key.clone();
            view.update(cx, |this, cx| {
                if this.is_running {
                    return;
                }
                this.open_menu = None;
                cx.emit(AgentInputEvent::PickScope { key });
                cx.notify();
            });
        })
        .into_any_element()
}

fn context_target_option(
    view: Entity<AgentInput>,
    opt: ComposerTarget,
    muted: gpui::Hsla,
    theme: &AgentChatTheme,
    _cx: &mut Context<gpui_component::popover::PopoverState>,
) -> gpui::AnyElement {
    let sel = opt.id.clone();
    let row_id = SharedString::from(format!("context-target-opt-{}", opt.id));
    let hover_bg = theme.hover_background();

    context_target_row(opt, muted, theme, false)
        .id(row_id)
        .cursor_pointer()
        .hover(move |s| s.bg(hover_bg))
        .on_click(move |_, _window, cx| {
            let sel = sel.clone();
            view.update(cx, |this, cx| {
                if this.is_running {
                    return;
                }
                this.open_menu = None;
                cx.emit(AgentInputEvent::SelectTarget { id: sel });
                cx.notify();
            });
        })
        .into_any_element()
}

impl Focusable for AgentInput {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for AgentInput {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // 打开上下文面板时,在 render 时(持有 &mut Window)清空搜索框。
        if self.context_search_needs_reset {
            self.context_search_input.update(cx, |state, cx| {
                state.set_value("", _window, cx);
            });
            self.context_search_query = SharedString::default();
            self.context_search_needs_reset = false;
        }
        let context_bar = self.render_context_bar(cx);
        let attachments = self.render_attachments(cx);
        let editor_top_bar = self.render_editor_top_bar(cx);
        let toolbar = self.render_toolbar(cx);
        let theme = self.local_theme(cx);
        let input_focused = self
            .input_state
            .read(cx)
            .focus_handle(cx)
            .is_focused(_window);

        v_flex()
            .debug_selector(|| "agent-input-root".to_string())
            .track_focus(&self.focus_handle)
            .w_full()
            .flex_shrink_0()
            .bg(theme.background)
            .text_color(theme.foreground)
            .when(!self.edge_to_edge, |this| {
                this.rounded_lg()
                    .border_1()
                    .border_color(theme.border)
                    .shadow_sm()
            })
            // 捕获阶段拦截 cmd/ctrl-v:把剪贴板图片作为附件(不阻断文本粘贴)。
            .capture_key_down(cx.listener(|this, ev: &KeyDownEvent, _window, cx| {
                if ev.keystroke.key == "v" && ev.keystroke.modifiers.secondary() {
                    this.paste_images_from_clipboard(cx);
                }
            }))
            // 顶部：计划 / Agent / 上下文入口
            .child(context_bar)
            // 附件预览（如果有）
            .children(attachments)
            .child(editor_top_bar)
            // 中部：多行输入框
            .child(
                div()
                    .w_full()
                    .px_3()
                    .pt_1()
                    .max_h(px(220.0))
                    .overflow_hidden()
                    .child(
                        div()
                            .w_full()
                            .border_1()
                            .rounded(cx.theme().radius)
                            .border_color(if input_focused {
                                theme.accent
                            } else {
                                theme.border
                            })
                            .bg(theme.background)
                            .child(
                                Input::new(&self.input_state)
                                    .size_full()
                                    .appearance(false)
                                    .text_color(theme.foreground)
                                    .caret_color(theme.foreground),
                            ),
                    ),
            )
            // 底部：执行参数、模型和发送按钮
            .child(toolbar)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::input::context::ComposerModel;
    use gpui::{Pixels, TestAppContext, VisualTestContext};

    struct AgentInputLayoutRoot {
        input: Entity<AgentInput>,
        width: Pixels,
    }

    impl AgentInputLayoutRoot {
        fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
            Self::with_width(px(360.0), window, cx)
        }

        fn wide(window: &mut Window, cx: &mut Context<Self>) -> Self {
            Self::with_width(px(900.0), window, cx)
        }

        fn with_width(width: Pixels, window: &mut Window, cx: &mut Context<Self>) -> Self {
            let input = cx.new(|cx| {
                AgentInput::with_mentions(Vec::new(), "描述目标，输入 @ 引用资源…", window, cx)
            });
            input.update(cx, |input, cx| {
                input.set_context(
                    AgentComposerContext {
                        model: Some(ComposerModel::new(
                            "Very Long Provider Name",
                            "extremely-long-model-name-with-large-context",
                        )),
                        tool_label: SharedString::from("手动确认"),
                        task_label: SharedString::from("Auto Mode"),
                        ..AgentComposerContext::default()
                    },
                    cx,
                );
                input.set_menu_options(
                    vec![ComposerModelOption::new(
                        "long-model",
                        "provider",
                        "Very Long Provider Name",
                        "extremely-long-model-name-with-large-context",
                    )],
                    vec![
                        ComposerMenuOption::new("auto", "自动"),
                        ComposerMenuOption::new("readonly", "只读"),
                        ComposerMenuOption::new("manual", "手动确认"),
                    ],
                    vec![ComposerMenuOption::new("agent", "Auto Mode")],
                    cx,
                );
            });
            Self { input, width }
        }
    }

    impl Render for AgentInputLayoutRoot {
        fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
            div().w(self.width).h(px(220.0)).child(self.input.clone())
        }
    }

    #[gpui::test]
    fn narrow_layout_keeps_model_and_send_controls_visible(cx: &mut TestAppContext) {
        cx.update(|cx| {
            gpui_component::init(cx);
            crate::init(cx);
        });
        let (_, cx) = cx.add_window_view(AgentInputLayoutRoot::new);
        let cx: &mut VisualTestContext = cx;

        let model = cx
            .debug_bounds("agent-input-model-control")
            .expect("model control should be rendered");
        let send = cx
            .debug_bounds("agent-input-send-control")
            .expect("send control should be rendered");

        assert!(model.size.width >= px(150.0));
        assert!(send.size.width >= px(28.0));
        assert!(
            model.origin.x + model.size.width <= send.origin.x,
            "model and send controls must not overlap: model={model:?}, send={send:?}"
        );
    }

    #[gpui::test]
    fn wide_layout_expands_model_control(cx: &mut TestAppContext) {
        cx.update(|cx| {
            gpui_component::init(cx);
            crate::init(cx);
        });
        let (_, cx) = cx.add_window_view(AgentInputLayoutRoot::wide);
        let cx: &mut VisualTestContext = cx;

        let model = cx
            .debug_bounds("agent-input-model-control")
            .expect("model control should be rendered");
        let send = cx
            .debug_bounds("agent-input-send-control")
            .expect("send control should be rendered");

        assert!(
            model.size.width >= px(280.0),
            "model control should use available toolbar width: model={model:?}"
        );
        assert_eq!(
            model.size.height, send.size.height,
            "model and send controls should align vertically: model={model:?}, send={send:?}"
        );
    }

    #[gpui::test]
    fn plan_and_subagent_triggers_stay_and_tool_mode_merges_into_task_menu(
        cx: &mut TestAppContext,
    ) {
        cx.update(|cx| {
            gpui_component::init(cx);
            crate::init(cx);
        });
        let (_, cx) = cx.add_window_view(AgentInputLayoutRoot::new);
        let cx: &mut VisualTestContext = cx;

        assert!(
            cx.debug_bounds("agent-plan-trigger").is_some(),
            "plan should stay as a top capability trigger"
        );
        assert!(
            cx.debug_bounds("agent-subagents-trigger").is_some(),
            "subagents should be a top capability trigger"
        );
        assert!(
            cx.debug_bounds("agent-task-mode").is_some(),
            "task mode should stay as the merged bottom mode control"
        );
        assert!(
            cx.debug_bounds("agent-tool-mode").is_none(),
            "tool mode should not render as a separate bottom toolbar control"
        );
    }

    #[test]
    fn merged_mode_menu_keeps_all_tool_confirmation_options() {
        let options = vec![
            ComposerMenuOption::new("auto", "自动"),
            ComposerMenuOption::new("readonly", "只读"),
            ComposerMenuOption::new("manual", "手动确认"),
        ];

        assert_eq!(
            vec!["自动", "只读", "手动确认"],
            options
                .iter()
                .map(|option| option.label.as_ref())
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn empty_tool_label_defaults_to_manual_confirmation() {
        assert_eq!(
            "手动确认",
            current_tool_label(&SharedString::from("")).as_ref()
        );
    }

    #[test]
    fn mode_trigger_label_includes_task_and_tool_confirmation_modes() {
        assert_eq!(
            "Auto Mode · 手动确认",
            execution_mode_trigger_label(
                &SharedString::from("Auto Mode"),
                &SharedString::from("手动确认")
            )
            .as_ref()
        );
    }

    #[test]
    fn plan_trigger_label_is_default_when_empty() {
        assert_eq!(plan_trigger_label(&[]).as_ref(), "计划");
    }

    #[test]
    fn plan_trigger_label_shows_running_progress() {
        let items = vec![
            ComposerPlanItem::new("完成项", "completed"),
            ComposerPlanItem::new("执行项", "running"),
            ComposerPlanItem::new("待执行项", "pending"),
        ];

        assert_eq!(plan_trigger_label(&items).as_ref(), "1/3 进行中");
    }

    #[test]
    fn plan_trigger_label_shows_completed_progress() {
        let items = vec![
            ComposerPlanItem::new("一", "completed"),
            ComposerPlanItem::new("二", "completed"),
        ];

        assert_eq!(plan_trigger_label(&items).as_ref(), "2/2 完成");
    }

    #[test]
    fn plan_trigger_label_shows_pending_progress() {
        let items = vec![
            ComposerPlanItem::new("一", "pending"),
            ComposerPlanItem::new("二", "failed"),
        ];

        assert_eq!(plan_trigger_label(&items).as_ref(), "0/2 待执行");
    }

    #[test]
    fn subagent_trigger_label_shows_running_subagent_count() {
        assert_eq!(subagent_trigger_label(&[]).as_ref(), "子代理");

        let items = vec![ComposerSubAgentItem::new(
            "sub_1",
            "reviewer",
            "检查事件流",
            "running",
        )];

        assert_eq!(subagent_trigger_label(&items).as_ref(), "子代理 · 1");
    }

    #[test]
    fn resource_pool_trigger_label_uses_pool_wording() {
        let context = AgentComposerContext {
            resource_pool: crate::input::context::ComposerResourcePoolSummary::new(
                Some(SharedString::from("ssh-a")),
                "prod-a",
                3,
            ),
            ..AgentComposerContext::default()
        };

        assert_eq!(resource_pool_trigger_label(&context).as_ref(), "资源池 · 3");
    }

    #[test]
    fn resource_pool_trigger_label_handles_empty_pool() {
        assert_eq!(
            resource_pool_trigger_label(&AgentComposerContext::default()).as_ref(),
            "资源池"
        );
    }

    #[test]
    fn target_search_matches_label_subtitle_and_kind_case_insensitively() {
        let opt = ComposerTarget::new(
            "prod-pg",
            "Prod PostgreSQL",
            "DB",
            "database",
            "PostgreSQL · 10.0.0.8:5432",
        );

        assert!(target_matches(&opt, "prod"));
        assert!(target_matches(&opt, "postgresql"));
        assert!(target_matches(&opt, "DATABASE"));
    }

    #[test]
    fn target_search_query_ignores_surrounding_whitespace() {
        let opt = ComposerTarget::new(
            "prod-pg",
            "Prod PostgreSQL",
            "DB",
            "database",
            "PostgreSQL · 10.0.0.8:5432",
        );

        assert!(target_matches(&opt, "  prod  "));
    }

    #[test]
    fn resource_type_filter_keeps_all_resources_for_all() {
        let targets = vec![
            ComposerTarget::new("ssh-a", "prod-a", "SH", "ssh", "ssh · ssh-a"),
            ComposerTarget::new("db-a", "prod-db", "DB", "postgres", "postgres · db-a"),
        ];

        let filtered = filter_targets_by_kind(targets.clone(), "all");

        assert_eq!(filtered, targets);
    }

    #[test]
    fn resource_type_filter_matches_target_kind() {
        let targets = vec![
            ComposerTarget::new("ssh-a", "prod-a", "SH", "ssh", "ssh · ssh-a"),
            ComposerTarget::new("db-a", "prod-db", "DB", "postgres", "postgres · db-a"),
        ];

        let filtered = filter_targets_by_kind(targets, "ssh");

        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].id.as_ref(), "ssh-a");
    }

    #[test]
    fn resource_pool_action_labels_match_membership() {
        let add = crate::input::context::ComposerResourcePoolItem::new(
            "ssh-b",
            "prod-b",
            "SH",
            "ssh",
            "ssh · ssh-b",
            false,
            false,
        );
        let remove = crate::input::context::ComposerResourcePoolItem::new(
            "ssh-a",
            "prod-a",
            "SH",
            "ssh",
            "ssh · ssh-a",
            true,
            false,
        );
        let default = crate::input::context::ComposerResourcePoolItem::new(
            "ssh-a",
            "prod-a",
            "SH",
            "ssh",
            "ssh · ssh-a",
            true,
            true,
        );

        assert_eq!(resource_pool_action_label(&add), "+");
        assert_eq!(resource_pool_action_label(&remove), "-");
        assert_eq!(resource_pool_action_label(&default), "默认");
    }

    #[test]
    fn resource_source_option_label_includes_count_or_disabled_hint() {
        let enabled =
            crate::input::context::ComposerResourceSourceOption::new("all", "全部", 3, true);
        let disabled = crate::input::context::ComposerResourceSourceOption::new(
            "workspace",
            "工作区",
            0,
            false,
        )
        .disabled("暂无工作区资源来源");

        assert_eq!(resource_source_option_label(&enabled).as_ref(), "全部 3");
        assert_eq!(
            resource_source_option_label(&disabled).as_ref(),
            "工作区 · 暂无工作区资源来源"
        );
    }

    #[test]
    fn top_capability_menus_can_open_while_agent_is_running() {
        assert_eq!(
            Some(ComposerMenuKind::Plan),
            menu_state_after_open_change(true, ComposerMenuKind::Plan)
        );
        assert_eq!(
            Some(ComposerMenuKind::SubAgent),
            menu_state_after_open_change(true, ComposerMenuKind::SubAgent)
        );
        assert_eq!(
            None,
            menu_state_after_open_change(false, ComposerMenuKind::Target)
        );
    }

    #[test]
    fn referenced_mentions_do_not_match_label_prefixes() {
        let mentions = vec![
            MentionItem::new("short", "prod", "mysql", "mysql"),
            MentionItem::new("long", "prod-db", "mysql", "mysql"),
        ];

        let got = referenced_mentions_in_text("分析 @`prod-db` 慢查询", &mentions);

        assert_eq!(
            vec!["long"],
            got.iter().map(|item| item.id.as_str()).collect::<Vec<_>>()
        );
    }

    #[test]
    fn referenced_mentions_match_quoted_names_with_spaces() {
        let mentions = vec![MentionItem::new("c1", "prod db", "postgres", "postgres")];

        let got = referenced_mentions_in_text("请检查 @`prod db` 的连接数", &mentions);

        assert_eq!(
            vec!["c1"],
            got.iter().map(|item| item.id.as_str()).collect::<Vec<_>>()
        );
    }

    #[test]
    fn referenced_mentions_respect_simple_name_boundaries() {
        let mentions = vec![MentionItem::new("c1", "prod", "mysql", "mysql")];

        let got = referenced_mentions_in_text("请检查 @production", &mentions);

        assert!(got.is_empty());
    }
}
