//! Agent 输入框:顶部目标 Context Bar + 多行输入 + `@` 提及 + 图片附件 + 底部执行参数工具栏。
//!
//! 布局参考 `agent-composer-design.html`:
//! - **顶部 Context Bar**:主目标 chip + 动态数量的派生上下文(scope)chip;
//! - **附件条**:图片缩略图(粘贴 / 附加);
//! - **编辑器**:基于 [`InputState`] 的多行自增高输入,注入 [`MentionCompletionProvider`] 实现 `@` 提及;
//! - **底部工具栏**:`+附件` / `@引用` / `工具模式▾` / `任务模式▾` / `模型▾` / `发送`。
//!
//! 设计原则:输入框是"哑组件",只接收 [`AgentComposerContext`] 做展示并在交互时 emit
//! [`AgentInputEvent`];目标用上层注入的列表渲染内置 popover(选中 emit `SelectTarget`),
//! scope 仅 emit `PickScope` 交上层;模型 / 工具 / 任务模式同样用注入选项渲染内置下拉。

use std::rc::Rc;
use std::sync::Arc;

use gpui::{
    App, AppContext, Context, Entity, EventEmitter, FocusHandle, Focusable, InteractiveElement,
    IntoElement, KeyDownEvent, ParentElement, PathPromptOptions, Render, SharedString,
    StatefulInteractiveElement, Styled, Subscription, Window, div, img, px,
};
use gpui_component::button::{Button, ButtonVariants};
use gpui_component::input::{Input, InputEvent, InputState};
use gpui_component::popover::Popover;
use gpui_component::{ActiveTheme, Disableable, Icon, IconName, Sizable, h_flex, v_flex};

use crate::input::attachment::ImageAttachment;
use crate::input::context::{
    AgentComposerContext, ComposerMenuOption, ComposerModelOption, ComposerScope, ComposerTarget,
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
}

/// 内置下拉的种类(用于受控开合状态)。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ComposerMenuKind {
    Target,
    Model,
    Tool,
    Task,
}

fn menu_event(kind: ComposerMenuKind, id: SharedString) -> AgentInputEvent {
    match kind {
        ComposerMenuKind::Target => AgentInputEvent::SelectTarget { id },
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

    /// 「@引用」按钮:聚焦编辑器并在光标处插入 `@` 以触发提及补全。
    fn insert_mention_trigger(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.focus_input(window, cx);
        self.input_state
            .update(cx, |state, cx| state.insert("@", window, cx));
    }

    fn remove_attachment(&mut self, id: &str, cx: &mut Context<Self>) {
        self.attachments.retain(|a| a.id != id);
        cx.notify();
    }

    fn render_context_bar(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let mut target_row = h_flex()
            .w_full()
            .items_center()
            .flex_wrap()
            .gap_2()
            .px_3()
            .py_2()
            .child(self.render_target_chip(cx));

        let scopes = self.context.scopes.clone();
        for scope in &scopes {
            target_row = target_row.child(self.render_scope_chip(scope, cx));
        }

        v_flex()
            .w_full()
            .border_b_1()
            .border_color(cx.theme().border)
            .child(target_row)
    }

    fn render_target_chip(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let view = cx.entity();
        let is_open = self.open_menu == Some(ComposerMenuKind::Target);
        let (has_target, value) = match &self.context.target {
            Some(t) => (true, t.label.clone()),
            None => (false, SharedString::from("选择目标")),
        };
        let options = self.target_options.clone();

        let trigger = {
            let button = Button::new("agent-target-chip")
                .small()
                .child(
                    h_flex()
                        .items_center()
                        .gap(px(6.0))
                        .child(div().text_xs().child(value)),
                )
                .dropdown_caret(true)
                .disabled(self.is_running);
            if has_target {
                button.outline()
            } else {
                button.ghost()
            }
        };

        Popover::new("agent-target-popover")
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
            .trigger(trigger)
            .content({
                let view = view.clone();
                move |_state, _window, cx| {
                    let muted = cx.theme().muted_foreground;
                    let hover_bg = cx.theme().list_hover;
                    let radius = cx.theme().radius;
                    let mut col = v_flex().p_1().gap(px(2.0)).min_w(px(280.0));
                    if options.is_empty() {
                        col = col.child(
                            div()
                                .p_2()
                                .text_xs()
                                .text_color(muted)
                                .child("（无可用目标）"),
                        );
                    }
                    for opt in &options {
                        let view = view.clone();
                        let sel = opt.id.clone();
                        col = col.child(
                            h_flex()
                                .id(SharedString::from(format!("target-opt-{}", opt.id)))
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
                                        .child(opt.icon.clone()),
                                )
                                .child(
                                    v_flex()
                                        .flex_1()
                                        .min_w_0()
                                        .gap(px(1.0))
                                        .child(div().text_sm().child(opt.label.clone()))
                                        .child(
                                            div()
                                                .text_xs()
                                                .text_color(muted)
                                                .child(opt.subtitle.clone()),
                                        ),
                                )
                                .child(div().text_xs().text_color(muted).child(opt.kind.clone()))
                                .on_click(move |_, _window, cx| {
                                    let sel = sel.clone();
                                    view.update(cx, |this, cx| {
                                        this.open_menu = None;
                                        cx.emit(menu_event(ComposerMenuKind::Target, sel));
                                        cx.notify();
                                    });
                                }),
                        );
                    }
                    col
                }
            })
    }

    fn render_scope_chip(
        &self,
        scope: &ComposerScope,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let key = scope.key.clone();
        let muted = cx.theme().muted_foreground;
        let fg = cx.theme().foreground;
        let border = cx.theme().border;
        let radius = cx.theme().radius;
        let hover_bg = cx.theme().list_hover;

        h_flex()
            .id(SharedString::from(format!("agent-scope-{key}")))
            .items_center()
            .h(px(28.0))
            .px(px(9.0))
            .gap(px(6.0))
            .rounded(radius)
            .border_1()
            .border_color(border)
            .bg(cx.theme().background)
            .cursor_pointer()
            .hover(move |s| s.bg(hover_bg))
            .child(div().text_xs().text_color(muted).child(scope.label.clone()))
            .child(div().text_xs().text_color(fg).child(scope.value.clone()))
            .child(Icon::new(IconName::ChevronDown).xsmall().text_color(muted))
            .on_click(cx.listener(move |this, _, _, cx| {
                if !this.is_running {
                    cx.emit(AgentInputEvent::PickScope { key: key.clone() });
                }
            }))
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
            .label(compact_label(trigger_label.as_ref(), 22))
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

    fn render_toolbar(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let running = self.is_running;
        let task_label = if self.context.task_label.is_empty() {
            SharedString::from("Ask")
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

        h_flex()
            .w_full()
            .items_center()
            .gap_1()
            .px_3()
            .pb_2()
            .pt_1()
            .child(
                Button::new("agent-attach")
                    .icon(IconName::Plus)
                    .ghost()
                    .small()
                    .tooltip("附加图片")
                    .on_click(cx.listener(|this, _, window, cx| this.open_file_picker(window, cx))),
            )
            .child(
                Button::new("agent-mention")
                    .label("@")
                    .ghost()
                    .small()
                    .tooltip("引用资源")
                    .on_click(
                        cx.listener(|this, _, window, cx| this.insert_mention_trigger(window, cx)),
                    ),
            )
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
            ))
            .child(div().flex_1().min_w_0())
            .child(
                div()
                    .max_w(px(220.0))
                    .min_w(px(120.0))
                    .overflow_hidden()
                    .child(self.render_model_menu(model_label, self.model_options.clone(), cx)),
            )
            .child(if running {
                Button::new("agent-stop")
                    .icon(IconName::Close)
                    .label("停止")
                    .danger()
                    .small()
                    .on_click(cx.listener(|this, _, _, cx| this.stop(cx)))
            } else {
                Button::new("agent-send")
                    .icon(IconName::ArrowUp)
                    .label("发送")
                    .primary()
                    .small()
                    .on_click(cx.listener(|this, _, window, cx| this.submit(window, cx)))
            })
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

impl Focusable for AgentInput {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for AgentInput {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let context_bar = self.render_context_bar(cx);
        let attachments = self.render_attachments(cx);
        let toolbar = self.render_toolbar(cx);

        v_flex()
            .track_focus(&self.focus_handle)
            .w_full()
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
            // 顶部：上下文目标选择栏
            .child(context_bar)
            // 附件预览（如果有）
            .children(attachments)
            // 中部：多行输入框
            .child(
                div()
                    .w_full()
                    .px_3()
                    .pt_2()
                    .max_h(px(240.0))
                    .overflow_hidden()
                    .child(Input::new(&self.input_state).size_full()),
            )
            // 底部：工具栏（附件、引用、模式、发送按钮）
            .child(toolbar)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
