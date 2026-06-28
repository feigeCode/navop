//! Agent 输入框:顶部模式入口 + 多行输入 + `@` 提及 + 图片附件 + 底部执行参数工具栏。
//!
//! 布局参考 `agent-composer-design.html`:
//! - **顶部模式入口**:计划 / 子代理 / 上下文;
//! - **附件条**:编辑器顶部的附件入口 + 图片缩略图(粘贴 / 附加);
//! - **编辑器**:基于 [`InputState`] 的多行自增高输入,注入 [`MentionCompletionProvider`] 实现 `@` 提及;
//! - **底部工具栏**:执行设置 / 任务模式▾ / 工具模式▾ / 模型▾ / 发送。
//!
//! 设计原则:输入框是"哑组件",只接收 [`AgentComposerContext`] 做展示并在交互时 emit
//! [`AgentInputEvent`];目标用上层注入的列表渲染内置 popover(选中 emit `SelectTarget`),
//! scope 仅 emit `PickScope` 交上层;模型 / 工具 / 任务模式同样用注入选项渲染内置下拉。

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
    AgentComposerContext, ComposerAgentOption, ComposerMenuOption, ComposerModelOption,
    ComposerPlanItem, ComposerScope, ComposerTarget,
};
use crate::input::mention::{MentionCompletionProvider, MentionItem};

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
    /// 在顶部「子代理」面板中选择内置 Agent 或 ACP Agent。
    SelectAgentBackend { id: Option<SharedString> },
}

/// 内置下拉的种类(用于受控开合状态)。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ComposerMenuKind {
    Target,
    Plan,
    Agent,
    Model,
    Tool,
    Task,
}

fn menu_event(kind: ComposerMenuKind, id: SharedString) -> AgentInputEvent {
    match kind {
        ComposerMenuKind::Target => AgentInputEvent::SelectTarget { id },
        ComposerMenuKind::Plan | ComposerMenuKind::Agent => {
            unreachable!("顶部能力面板使用专用事件")
        }
        ComposerMenuKind::Model => unreachable!("模型菜单使用结构化事件"),
        ComposerMenuKind::Tool => AgentInputEvent::SelectToolMode { id },
        ComposerMenuKind::Task => AgentInputEvent::SelectTaskMode { id },
    }
}

fn compact_label(label: &str, max_chars: usize) -> SharedString {
    if label.chars().count() <= max_chars {
        return SharedString::from(label.to_string());
    }
    let mut s: String = label.chars().take(max_chars.saturating_sub(1)).collect();
    s.push('…');
    SharedString::from(s)
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
    /// 当前展开的下拉(受控开合)。
    open_menu: Option<ComposerMenuKind>,
    /// 是否折叠顶部计划 / 子代理 / 上下文能力区。
    top_capabilities_collapsed: bool,
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
            open_menu: None,
            top_capabilities_collapsed: false,
            _subscriptions: vec![enter_sub, paste_sub],
        }
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
        let border = cx.theme().border;

        v_flex()
            .w_full()
            .flex_shrink_0()
            .when(!self.top_capabilities_collapsed, |this| {
                this.border_b_1()
                    .border_color(border)
                    .child(self.render_mode_tabs(cx))
            })
    }

    fn render_mode_tabs(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        v_flex().w_full().px_3().pt_2().pb_1p5().child(
            h_flex()
                .w_full()
                .h(px(38.0))
                .items_center()
                .rounded(cx.theme().radius)
                .border_1()
                .border_color(cx.theme().border)
                .bg(cx.theme().muted)
                .child(self.render_plan_mode_tab(cx))
                .child(self.render_mode_separator(cx))
                .child(self.render_agent_mode_tab(cx))
                .child(self.render_mode_separator(cx))
                .child(self.render_context_mode_tab(cx)),
        )
    }

    fn render_plan_mode_tab(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let view = cx.entity();
        let is_open = self.open_menu == Some(ComposerMenuKind::Plan);
        let plan_items = self.context.plan_items.clone();

        Popover::new("agent-plan-popover")
            .p_0()
            .open(is_open)
            .on_open_change({
                let view = view.clone();
                move |open, _window, cx| {
                    let open = *open;
                    view.update(cx, |this, cx| {
                        this.open_menu = if open && !this.is_running {
                            Some(ComposerMenuKind::Plan)
                        } else {
                            None
                        };
                        cx.notify();
                    });
                }
            })
            .trigger(self.render_capability_trigger(
                "agent-plan-trigger",
                "计划",
                IconName::Check,
                cx,
            ))
            .content(move |_state, _window, cx| render_plan_mode_content(plan_items.clone(), cx))
    }

    fn render_agent_mode_tab(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let view = cx.entity();
        let is_open = self.open_menu == Some(ComposerMenuKind::Agent);
        let agents = self.context.agent_options.clone();

        Popover::new("agent-subagent-popover")
            .p_0()
            .open(is_open)
            .on_open_change({
                let view = view.clone();
                move |open, _window, cx| {
                    let open = *open;
                    view.update(cx, |this, cx| {
                        this.open_menu = if open && !this.is_running {
                            Some(ComposerMenuKind::Agent)
                        } else {
                            None
                        };
                        cx.notify();
                    });
                }
            })
            .trigger(self.render_capability_trigger(
                "agent-subagent-trigger",
                "子代理",
                IconName::Bot,
                cx,
            ))
            .content({
                let view = view.clone();
                move |_state, _window, cx| {
                    render_agent_mode_content(view.clone(), agents.clone(), cx)
                }
            })
    }

    fn render_capability_trigger(
        &self,
        id: &'static str,
        label: &'static str,
        icon: IconName,
        cx: &mut Context<Self>,
    ) -> Button {
        Button::new(id)
            .debug_selector(move || id.to_string())
            .flex_1()
            .min_w_0()
            .h_full()
            .ghost()
            .small()
            .disabled(self.is_running)
            .child(
                h_flex()
                    .min_w_0()
                    .items_center()
                    .justify_center()
                    .gap_1()
                    .text_color(cx.theme().muted_foreground)
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

        Popover::new("agent-context-mode-popover")
            .p_0()
            .open(is_open)
            .on_open_change({
                let view = view.clone();
                move |open, _window, cx| {
                    let open = *open;
                    view.update(cx, |this, cx| {
                        this.open_menu = if open && !this.is_running {
                            Some(ComposerMenuKind::Target)
                        } else {
                            None
                        };
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
                        cx,
                    )
                }
            })
    }

    fn render_context_mode_trigger(&self, cx: &mut Context<Self>) -> Button {
        let fg = if self.context.target.is_some() {
            cx.theme().foreground
        } else {
            cx.theme().muted_foreground
        };
        Button::new("agent-context-mode")
            .flex_1()
            .min_w_0()
            .h_full()
            .ghost()
            .small()
            .disabled(self.is_running)
            .child(
                h_flex()
                    .min_w_0()
                    .items_center()
                    .justify_center()
                    .gap_1()
                    .text_color(fg)
                    .child(Icon::new(IconName::File).xsmall())
                    .child(div().text_sm().truncate().child("上下文")),
            )
    }

    fn render_mode_separator(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        div().h(px(20.0)).w(px(1.0)).bg(cx.theme().border)
    }

    fn render_menu(
        &self,
        kind: ComposerMenuKind,
        id: &'static str,
        trigger_label: SharedString,
        options: Vec<ComposerMenuOption>,
        accent: bool,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let view = cx.entity();
        let is_open = self.open_menu == Some(kind);

        let trigger = {
            let button = Button::new(id)
                .small()
                .label(trigger_label)
                .dropdown_caret(true)
                .disabled(self.is_running);
            if accent {
                button.outline()
            } else {
                button.ghost()
            }
        };

        Popover::new(SharedString::from(format!("{id}-popover")))
            .p_0()
            .open(is_open)
            .on_open_change({
                let view = view.clone();
                move |open, _window, cx| {
                    let open = *open;
                    view.update(cx, |this, cx| {
                        this.open_menu = if open { Some(kind) } else { None };
                        cx.notify();
                    });
                }
            })
            .trigger(trigger)
            .content({
                let view = view.clone();
                move |_state, _window, cx| {
                    let muted = cx.theme().muted_foreground;
                    let hover_bg = cx.theme().list_hover;
                    let radius = cx.theme().radius;
                    let mut col = v_flex().p_1().gap(px(2.0)).min_w(px(200.0));
                    for opt in &options {
                        let view = view.clone();
                        let sel = opt.id.clone();
                        let mut inner = v_flex()
                            .gap(px(1.0))
                            .child(div().text_sm().child(opt.label.clone()));
                        if let Some(hint) = &opt.hint {
                            inner =
                                inner.child(div().text_xs().text_color(muted).child(hint.clone()));
                        }
                        col = col.child(
                            h_flex()
                                .id(SharedString::from(format!("{id}-opt-{sel}")))
                                .w_full()
                                .px_2()
                                .py_1()
                                .rounded(radius)
                                .cursor_pointer()
                                .hover(move |s| s.bg(hover_bg))
                                .child(inner)
                                .on_click(move |_, _window, cx| {
                                    let sel = sel.clone();
                                    view.update(cx, |this, cx| {
                                        this.open_menu = None;
                                        cx.emit(menu_event(kind, sel));
                                        cx.notify();
                                    });
                                }),
                        );
                    }
                    col
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

        let trigger = Button::new("agent-model")
            .small()
            .label(compact_label(trigger_label.as_ref(), 18))
            .outline()
            .dropdown_caret(true)
            .disabled(self.is_running);

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
                move |_state, _window, cx| {
                    let muted = cx.theme().muted_foreground;
                    let hover_bg = cx.theme().list_hover;
                    let radius = cx.theme().radius;
                    let mut col = v_flex().p_1().gap(px(2.0)).min_w(px(240.0));
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
                            .border_color(cx.theme().border),
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
        let muted = cx.theme().muted_foreground;
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
                    .child(div().h(px(18.0)).w(px(1.0)).bg(cx.theme().border)),
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
        let running = self.is_running;
        let task_label = if self.context.task_label.is_empty() {
            SharedString::from("Auto Mode")
        } else {
            self.context.task_label.clone()
        };
        let tool_label = if self.context.tool_label.is_empty() {
            SharedString::from("工具: 自动")
        } else {
            SharedString::from(format!("工具: {}", self.context.tool_label))
        };
        let model_label = match &self.context.model {
            Some(m) => SharedString::from(format!("{} / {}", m.provider, m.model)),
            None => SharedString::from("选择模型"),
        };

        let left_controls = h_flex()
            .flex_1()
            .min_w_0()
            .items_center()
            .flex_wrap()
            .gap_1()
            .child(self.render_menu(
                ComposerMenuKind::Task,
                "agent-task-mode",
                task_label,
                self.task_options.clone(),
                true,
                cx,
            ))
            .child(self.render_menu(
                ComposerMenuKind::Tool,
                "agent-tool-mode",
                tool_label,
                self.tool_options.clone(),
                false,
                cx,
            ));

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
            .gap_2()
            .px_3()
            .pb_2()
            .pt_1()
            .flex_shrink_0()
            .child(left_controls)
            .child(
                h_flex()
                    .items_center()
                    .gap_1()
                    .flex_shrink_0()
                    .child(
                        div()
                            .w(px(168.0))
                            .max_w(px(168.0))
                            .overflow_hidden()
                            .child(self.render_model_menu(
                                model_label,
                                self.model_options.clone(),
                                cx,
                            ))
                            .debug_selector(|| "agent-input-model-control".to_string()),
                    )
                    .child(
                        div()
                            .w(px(34.0))
                            .h(px(32.0))
                            .flex_shrink_0()
                            .debug_selector(|| "agent-input-send-control".to_string())
                            .child(run_button),
                    ),
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

fn render_plan_mode_content(
    items: Vec<ComposerPlanItem>,
    cx: &mut Context<gpui_component::popover::PopoverState>,
) -> gpui::AnyElement {
    let muted = cx.theme().muted_foreground;
    let border = cx.theme().border;
    let mut col = v_flex().p_1().gap(px(2.0)).min_w(px(320.0));

    col = col.child(context_group_label("计划 Todo", cx));
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

    for item in items {
        col = col.child(plan_item_row(item, muted, border, cx));
    }
    col.into_any_element()
}

fn plan_item_row(
    item: ComposerPlanItem,
    muted: gpui::Hsla,
    border: gpui::Hsla,
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

    h_flex()
        .w_full()
        .items_center()
        .gap_2()
        .px_2()
        .py_1()
        .border_b_1()
        .border_color(border)
        .child(Icon::new(icon).xsmall().text_color(icon_color))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .text_sm()
                .truncate()
                .child(item.title),
        )
        .child(
            div()
                .text_xs()
                .text_color(muted)
                .child(plan_status_label(item.status.as_ref())),
        )
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

fn render_agent_mode_content(
    view: Entity<AgentInput>,
    agents: Vec<ComposerAgentOption>,
    cx: &mut Context<gpui_component::popover::PopoverState>,
) -> gpui::AnyElement {
    let muted = cx.theme().muted_foreground;
    let mut col = v_flex().p_1().gap(px(2.0)).min_w(px(300.0));

    col = col.child(context_group_label("Agent", cx));
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
        col = col.child(agent_option_row(view.clone(), agent, muted, cx));
    }
    col.into_any_element()
}

fn agent_option_row(
    view: Entity<AgentInput>,
    agent: ComposerAgentOption,
    muted: gpui::Hsla,
    cx: &mut Context<gpui_component::popover::PopoverState>,
) -> gpui::AnyElement {
    let hover_bg = cx.theme().list_hover;
    let selected_bg = cx.theme().accent;
    let icon_fg = if agent.selected {
        cx.theme().accent_foreground
    } else {
        muted
    };
    let id = agent.element_id();
    let target = agent.id.clone();
    let disabled = agent.connecting;

    h_flex()
        .id(id)
        .w_full()
        .items_center()
        .gap_2()
        .px_2()
        .py_1p5()
        .rounded(cx.theme().radius)
        .when(agent.selected, |this| this.bg(selected_bg))
        .when(disabled, |this| this.opacity(0.5))
        .when(!disabled, |this| {
            this.cursor_pointer()
                .hover(move |this| this.bg(hover_bg))
                .on_click(move |_, _window, cx| {
                    let target = target.clone();
                    view.update(cx, |this, cx| {
                        if this.is_running {
                            return;
                        }
                        this.open_menu = None;
                        cx.emit(AgentInputEvent::SelectAgentBackend { id: target });
                        cx.notify();
                    });
                })
        })
        .child(
            Icon::new(if agent.id.is_some() {
                IconName::Bot
            } else {
                IconName::AI
            })
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

fn render_context_mode_content(
    view: Entity<AgentInput>,
    options: Vec<ComposerTarget>,
    current: Option<ComposerTarget>,
    scopes: Vec<ComposerScope>,
    cx: &mut Context<gpui_component::popover::PopoverState>,
) -> gpui::AnyElement {
    let muted = cx.theme().muted_foreground;
    let border = cx.theme().border;
    let mut col = v_flex().p_1().gap(px(2.0)).min_w(px(300.0));

    if let Some(target) = current {
        col = col
            .child(context_group_label("当前上下文", cx))
            .child(context_summary_row(
                target.label,
                target.subtitle,
                muted,
                cx,
            ));
    }
    if !scopes.is_empty() {
        col = col.child(context_group_label("作用域", cx));
        for scope in scopes {
            col = col.child(context_scope_row(view.clone(), scope, muted, border, cx));
        }
    }
    col = col.child(context_group_label("选择目标", cx));
    if options.is_empty() {
        col = col.child(div().p_2().text_xs().text_color(muted).child("无可用目标"));
    }
    for opt in options {
        col = col.child(context_target_option(view.clone(), opt, muted, cx));
    }

    col.into_any_element()
}

fn context_group_label(
    label: &'static str,
    cx: &mut Context<gpui_component::popover::PopoverState>,
) -> gpui::AnyElement {
    div()
        .px_2()
        .pt_1()
        .text_xs()
        .text_color(cx.theme().muted_foreground)
        .child(label)
        .into_any_element()
}

fn context_summary_row(
    label: SharedString,
    subtitle: SharedString,
    muted: gpui::Hsla,
    cx: &mut Context<gpui_component::popover::PopoverState>,
) -> gpui::AnyElement {
    v_flex()
        .px_2()
        .py_1()
        .rounded(cx.theme().radius)
        .bg(cx.theme().list_hover)
        .child(div().text_sm().child(label))
        .child(div().text_xs().text_color(muted).child(subtitle))
        .into_any_element()
}

fn context_scope_row(
    view: Entity<AgentInput>,
    scope: ComposerScope,
    muted: gpui::Hsla,
    border: gpui::Hsla,
    cx: &mut Context<gpui_component::popover::PopoverState>,
) -> gpui::AnyElement {
    let key = scope.key.clone();
    let hover_bg = cx.theme().list_hover;
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
    cx: &mut Context<gpui_component::popover::PopoverState>,
) -> gpui::AnyElement {
    let sel = opt.id.clone();
    let hover_bg = cx.theme().list_hover;
    let radius = cx.theme().radius;

    h_flex()
        .id(SharedString::from(format!("context-target-opt-{}", opt.id)))
        .w_full()
        .items_center()
        .gap(px(8.0))
        .px_2()
        .py_1()
        .rounded(radius)
        .cursor_pointer()
        .hover(move |s| s.bg(hover_bg))
        .child(
            h_flex()
                .items_center()
                .justify_center()
                .size(px(24.0))
                .rounded(radius)
                .bg(hover_bg)
                .text_xs()
                .child(opt.icon),
        )
        .child(
            v_flex()
                .flex_1()
                .min_w_0()
                .gap(px(1.0))
                .child(div().text_sm().child(opt.label))
                .child(div().text_xs().text_color(muted).child(opt.subtitle)),
        )
        .child(div().text_xs().text_color(muted).child(opt.kind))
        .on_click(move |_, _window, cx| {
            let sel = sel.clone();
            view.update(cx, |this, cx| {
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
        let context_bar = self.render_context_bar(cx);
        let attachments = self.render_attachments(cx);
        let editor_top_bar = self.render_editor_top_bar(cx);
        let toolbar = self.render_toolbar(cx);

        v_flex()
            .track_focus(&self.focus_handle)
            .w_full()
            .flex_shrink_0()
            .bg(cx.theme().background)
            .rounded_lg()
            .border_1()
            .border_color(cx.theme().border)
            .shadow_sm()
            // 捕获阶段拦截 cmd/ctrl-v:把剪贴板图片作为附件(不阻断文本粘贴)。
            .capture_key_down(cx.listener(|this, ev: &KeyDownEvent, _window, cx| {
                if ev.keystroke.key == "v" && ev.keystroke.modifiers.secondary() {
                    this.paste_images_from_clipboard(cx);
                }
            }))
            // 顶部：计划 / 子代理 / 上下文入口
            .child(context_bar)
            // 附件预览（如果有）
            .children(attachments)
            .child(editor_top_bar)
            // 中部：多行输入框
            .child(
                div()
                    .w_full()
                    .px_3()
                    .pt_2()
                    .max_h(px(220.0))
                    .overflow_hidden()
                    .child(Input::new(&self.input_state).size_full()),
            )
            // 底部：执行参数、模型和发送按钮
            .child(toolbar)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::input::context::ComposerModel;
    use gpui::{TestAppContext, VisualTestContext};

    struct AgentInputLayoutRoot {
        input: Entity<AgentInput>,
    }

    impl AgentInputLayoutRoot {
        fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
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
                        tool_label: SharedString::from("自动"),
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
                    vec![ComposerMenuOption::new("auto", "自动")],
                    vec![ComposerMenuOption::new("agent", "Auto Mode")],
                    cx,
                );
            });
            Self { input }
        }
    }

    impl Render for AgentInputLayoutRoot {
        fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
            div().w(px(360.0)).h(px(220.0)).child(self.input.clone())
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
    fn top_capability_triggers_replace_bottom_settings(cx: &mut TestAppContext) {
        cx.update(|cx| {
            gpui_component::init(cx);
            crate::init(cx);
        });
        let (_, cx) = cx.add_window_view(AgentInputLayoutRoot::new);
        let cx: &mut VisualTestContext = cx;

        assert!(
            cx.debug_bounds("agent-plan-trigger").is_some(),
            "plan should be a top capability trigger, not a task-mode switch"
        );
        assert!(
            cx.debug_bounds("agent-subagent-trigger").is_some(),
            "subagent should be a top capability trigger for local/ACP agents"
        );
        assert!(
            cx.debug_bounds("agent-settings").is_none(),
            "execution settings no longer belongs in the bottom toolbar"
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
