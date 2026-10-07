//! Agent 输入框:顶部能力区 + 多行输入 + `@` 提及 + 图片附件 + 底部发送栏。
//!
//! 布局参考 `agent-composer-design.html`:
//! - **顶部能力区**:计划 / Agent / 上下文;
//! - **附件条**:编辑器顶部的附件入口 + 图片缩略图(粘贴 / 附加);
//! - **编辑器**:基于 [`EditorState`] 的多行输入,注入 [`MentionCompletionProvider`] 实现 `@` 提及;
//! - **底部发送栏**:模型▾ / 发送。
//!
//! 设计原则:输入框是"哑组件",只接收 [`AgentComposerContext`] 做展示并在交互时 emit
//! [`AgentInputEvent`];目标用上层注入的列表渲染内置 popover(选中 emit `SelectTarget`),
//! scope 仅 emit `PickScope` 交上层;模型 / 执行模式同样用注入选项渲染内置下拉。

use std::collections::{BTreeMap, HashSet};
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;

use gpui::{
    App, AppContext, Context, Entity, EntityInputHandler, EventEmitter, FocusHandle, Focusable,
    InteractiveElement, IntoElement, Modifiers, ParentElement, PathPromptOptions, Pixels, Render,
    SharedString, StatefulInteractiveElement, StyleRefinement, Styled, Subscription, Window, div,
    img, prelude::FluentBuilder, px,
};
use gpui_component::button::{Button, ButtonVariants};
use gpui_component::checkbox::Checkbox;
use gpui_component::input::{Editor, EditorState, Input, InputEvent, InputState};
use gpui_component::popover::Popover;
use gpui_component::searchable_list::SearchableVec;
use gpui_component::select::{Select, SelectEvent, SelectGroup, SelectState};
use gpui_component::switch::Switch;
use gpui_component::{ActiveTheme, Disableable, Icon, Sizable, h_flex, v_flex};
use one_assets::IconName;
use rust_i18n::t;
use serde_json::Value;

use crate::acp::{
    AcpElicitationField, AcpElicitationFieldKind, AcpElicitationMode, AcpElicitationRequest,
};
use crate::input::PromptHistory;
use crate::input::attachment::ImageAttachment;
use crate::input::context::{
    AgentComposerContext, ComposerBranchOption, ComposerMenuOption, ComposerModelOption,
    ComposerPlanItem, ComposerResourcePoolItem, ComposerResourceSourceOption,
    ComposerResourceTypeFilter, ComposerScope, ComposerSubAgentItem, ComposerTarget,
    ComposerWorkspaceOption,
};
use crate::input::completion::ComposerCompletionProvider;
use crate::input::mention::{MentionCompletionProvider, MentionItem};
use crate::input::slash::{SlashCommandItem, SlashCompletionProvider};
use crate::input::model_picker::{ModelChoice, model_groups, selected_model_index};
use crate::input::skill::{render_skill_mode_content, skill_trigger_label};
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
    /// 切换某个 Skill 是否注入本会话。
    ToggleSkill { id: SharedString },
    /// 从本地目录导入一个 Codex-style Skill。
    ImportSkill { path: PathBuf },
    /// 点击某个派生上下文 chip —— 上层据 `key` 弹出对应选择器。
    PickScope { key: SharedString },
    /// 在内置下拉中选择了模型。
    SelectModel {
        id: SharedString,
        provider_id: SharedString,
        model: SharedString,
    },
    /// 在内置下拉中选择了工具执行模式。
    SelectExecutionMode { id: SharedString },
    /// 在底部上下文栏的工作区下拉中选择了工作区。
    SelectWorkspace { path: SharedString },
    /// 点击底部上下文栏工作区下拉里的「选择其它目录」，由宿主弹出目录选择器。
    BrowseWorkspace,
    /// 在底部上下文栏的分支下拉中选择了分支。
    SelectBranch { name: SharedString },
    /// 切换底部上下文栏的 Worktree 开关。
    ToggleWorktree { enabled: bool },
    /// 在顶部「Agent」面板中选择内置 Agent 或 ACP Agent。
    SelectAgentBackend { id: Option<SharedString> },
    /// 删除队列中第 `index` 条待执行提交。
    RemoveQueued { index: usize },
    /// 把队列中第 `index` 条拉回输入框继续编辑（由上层负责从队列移除）。
    EditQueued { index: usize },
    /// 回答 agent 的提问：`content` 已按属性名收集好，直接送回 ACP。
    SubmitElicitation { content: BTreeMap<String, Value> },
    /// 明确拒绝回答 agent 的提问。
    DeclineElicitation,
    /// 关掉这条提问（等价于取消，agent 会走降级路径）。
    CancelElicitation,
}

/// 内置下拉的种类(用于受控开合状态)。
///
/// 模型下拉不在这里:它交给组件库的 `Select`,开合由组件自己管。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ComposerMenuKind {
    Target,
    Skill,
    Plan,
    SubAgent,
    Mode,
    /// 底部上下文栏的工作区下拉。
    Workspace,
    /// 底部上下文栏的分支下拉。
    Branch,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum HistoryDirection {
    Previous,
    Next,
}

fn history_direction(key: &str, modifiers: &Modifiers) -> Option<HistoryDirection> {
    if modifiers.modified() {
        return None;
    }
    match key {
        "up" => Some(HistoryDirection::Previous),
        "down" => Some(HistoryDirection::Next),
        _ => None,
    }
}

fn cursor_is_at_history_boundary(direction: HistoryDirection, text: &str, cursor: usize) -> bool {
    match direction {
        HistoryDirection::Previous => cursor == 0,
        HistoryDirection::Next => cursor == text.len(),
    }
}

/// 当前会话中等待下一轮执行的提交摘要。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct QueuedPromptPreview {
    pub text: SharedString,
    pub image_count: usize,
}

impl QueuedPromptPreview {
    pub fn new(text: impl Into<SharedString>, image_count: usize) -> Self {
        Self {
            text: text.into(),
            image_count,
        }
    }
}

const CONTEXT_POPOVER_WIDTH: f32 = 400.0;
const CONTEXT_TARGET_LIST_MAX_HEIGHT: f32 = 320.0;
const CONTEXT_KIND_MAX_WIDTH: f32 = 92.0;
const COMPOSER_EDITOR_MIN_ROWS: usize = 3;
const COMPOSER_EDITOR_MAX_ROWS: usize = 10;
const COMPOSER_EDITOR_FALLBACK_LINE_HEIGHT: f32 = 20.0;
const COMPOSER_EDITOR_VERTICAL_PADDING: f32 = 12.0;
/// 底部工具栏操作按钮(发送 / 排队 / 停止)的边长,同时也是工具栏控件高度。
const TOOLBAR_ACTION_BUTTON_SIZE: f32 = 32.0;
/// 底部那一行里上下文档位 chip 的固定高度。
///
/// 与发送 / 附件按钮同高：整行只有一种控件高度，7 项并排时不会参差。
const COMPOSER_CHIP_HEIGHT: f32 = TOOLBAR_ACTION_BUTTON_SIZE;
/// 上下文档位 chip 收缩到的最小宽度（标签走省略号）。
///
/// 侧栏被拖窄时 7 项仍要在同一行：宽度不足靠 `flex_shrink` + `truncate` 消化，
/// 每项的完整内容在点击后弹出的菜单 / 弹层里。
const COMPOSER_CHIP_MIN_WIDTH: f32 = 44.0;
/// 上下文档位 chip 的自然宽度上限。
///
/// 工作区目录名、分支名都可能很长；不设上限的话一项就能吃掉整行，
/// 把其余 chip 挤到只剩省略号。
const COMPOSER_CHIP_MAX_WIDTH: f32 = 220.0;
/// 下拉触发器里除文字外的固定占位:左右内边距 + 箭头 + 间距。
const TRIGGER_CHROME_WIDTH: f32 = 48.0;

fn composer_editor_height(state: &EditorState) -> Pixels {
    let rows = state
        .text()
        .to_string()
        .split('\n')
        .count()
        .clamp(COMPOSER_EDITOR_MIN_ROWS, COMPOSER_EDITOR_MAX_ROWS);
    let line_height = state
        .line_height()
        .unwrap_or(px(COMPOSER_EDITOR_FALLBACK_LINE_HEIGHT));
    line_height * rows as f32 + px(COMPOSER_EDITOR_VERTICAL_PADDING)
}

/// 底部那一行里单个 chip 的内容:`[图标] [可截断标签] [⌄]`。
///
/// 这个函数只管内容;承载它的容器(直接可点的 chip / 下拉触发器)由调用方决定,
/// 但都必须给足宽度约束(容器 `min_w_0` + 这里 `flex_1 min_w_0`)标签才会走省略号。
fn composer_chip_label(icon: IconName, label: SharedString) -> impl IntoElement {
    h_flex()
        .w_full()
        .min_w_0()
        .items_center()
        .gap_1()
        .child(Icon::new(icon).xsmall().flex_shrink_0())
        .child(div().flex_1().min_w_0().truncate().text_xs().child(label))
        .child(Icon::new(IconName::ChevronDown).xsmall().flex_shrink_0())
}

/// 上下文档位 chip 的自然宽度:按标签估算后夹在区间内。
///
/// 这一行固定 7 项(`[附件][工作区][分支][模型][Worktree][权限][发送]`),
/// 谁都不能无条件铺满:这里给的是「内容想要的宽度」,空间不够时由容器的
/// `flex_shrink` + 标签 `truncate` 消化,而不是让某一项独占整行。
fn composer_chip_width(label: &str) -> Pixels {
    px((estimated_label_width(label) + TRIGGER_CHROME_WIDTH)
        .clamp(COMPOSER_CHIP_MIN_WIDTH, COMPOSER_CHIP_MAX_WIDTH))
}

fn menu_state_after_open_change(
    requested_open: bool,
    menu: ComposerMenuKind,
) -> Option<ComposerMenuKind> {
    requested_open.then_some(menu)
}

fn current_execution_mode_label(label: &SharedString) -> SharedString {
    if label.is_empty() {
        SharedString::from(t!("AgentUi.auto").to_string())
    } else {
        label.clone()
    }
}

/// 执行模式下拉触发器的宽度。
///
/// 执行模式只承载「自动 / 只读 / 手动确认」这类短标签,固定 124px 会在窄侧边栏里
/// 吃掉模型选择的空间,把发送 / 停止按钮挤成细条。这里按标签估算宽度并夹在区间内:
/// 中文短标签收窄,英文长标签仍保留原来的可用宽度上限。
fn execution_trigger_width(label: &str, queue_mode: bool) -> Pixels {
    let (min, max) = if queue_mode {
        (72.0, 88.0)
    } else {
        (80.0, 124.0)
    };
    px((estimated_label_width(label) + TRIGGER_CHROME_WIDTH).clamp(min, max))
}

/// 估算一段文案的渲染宽度:CJK 等全角字符按 13px,其余按 7.5px。
fn estimated_label_width(label: &str) -> f32 {
    label
        .chars()
        .map(|ch| if is_full_width_char(ch) { 13.0 } else { 7.5 })
        .sum()
}

/// 常见全角字符区间(中日韩文字、全角标点等)。
fn is_full_width_char(ch: char) -> bool {
    matches!(
        ch as u32,
        0x1100..=0x115F
            | 0x2E80..=0x303E
            | 0x3041..=0x33FF
            | 0x3400..=0x4DBF
            | 0x4E00..=0x9FFF
            | 0xA000..=0xA4CF
            | 0xAC00..=0xD7A3
            | 0xF900..=0xFAFF
            | 0xFE30..=0xFE4F
            | 0xFF00..=0xFF60
            | 0xFFE0..=0xFFE6
            | 0x20000..=0x3FFFD
    )
}

/// Agent 输入框组件。
pub struct AgentInput {
    focus_handle: FocusHandle,
    input_state: Entity<EditorState>,
    /// 可被 `@` 引用的条目(同时用于补全 provider 与提交时解析)。
    mentions: Arc<Vec<MentionItem>>,
    /// 可被 `/` 补全的命令；来源是 ACP agent 推送的 `available_commands`。
    slash_commands: Vec<SlashCommandItem>,
    /// 当前图片附件。
    attachments: Vec<ImageAttachment>,
    /// 已提交过的文本历史，用于在输入框中通过上下键浏览。
    history: PromptHistory,
    /// 当前会话等待下一轮执行的提交摘要。
    queued_submissions: Vec<QueuedPromptPreview>,
    /// 队列因 ACP 连接或会话过渡失败而暂停，可由用户显式停止并清空。
    pending_queue_blocked: bool,
    /// 当前挂着的「agent 提问」。由上层在收到 ACP elicitation 时推入，答完清空。
    pending_elicitation: Option<PendingElicitation>,
    /// 是否正在运行(运行中显示「停止」)。
    is_running: bool,
    /// 上层注入的展示上下文(目标 / scope / 能力 / 模型 / 模式文案)。
    context: AgentComposerContext,
    /// 目标下拉选项(上层注入)。
    target_options: Vec<ComposerTarget>,
    /// 模型下拉选项(上层注入)。
    model_options: Vec<ComposerModelOption>,
    /// 模型下拉的组件库状态。
    ///
    /// 选项的注入点只有 `Context`、拿不到窗口，而 `SelectState` 的更新需要窗口；
    /// 所以注入时只置脏标记，真正的同步在 `render`(持有窗口)时做 ——
    /// 与 `context_search_needs_reset` 同一个套路。
    model_select: Entity<SelectState<SearchableVec<SelectGroup<ModelChoice>>>>,
    /// 见 `model_select`:注入新选项后置位,`render` 消费后清掉。
    model_select_dirty: bool,
    /// 工具执行模式下拉选项(上层注入)。
    execution_mode_options: Vec<ComposerMenuOption>,
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
            t!("AgentUi.input_placeholder").to_string(),
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
            let mut state = EditorState::new(window, cx)
                .line_number(false)
                .soft_wrap(true)
                .submit_on_enter(true)
                .placeholder(placeholder);
            state.lsp_mut().completion_provider = Some(Rc::new(
                ComposerCompletionProvider::new(provider_items, Vec::new()),
            ));
            state
        });

        let enter_sub = cx.subscribe_in(&input_state, window, |this, _state, event, window, cx| {
            if let InputEvent::PressEnter {
                secondary, shift, ..
            } = event
                && !secondary
                && !shift
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

        let weak = cx.entity().downgrade();
        let history_sub = cx.intercept_keystrokes(move |ev, window, cx| {
            let Some(direction) =
                history_direction(ev.keystroke.key.as_ref(), &ev.keystroke.modifiers)
            else {
                return;
            };
            let Some(this) = weak.upgrade() else {
                return;
            };
            let input_state = this.read(cx).input_state.clone();
            let can_navigate = input_state.update(cx, |state, cx| {
                let text = state.text().to_string();
                state.focus_handle(cx).is_focused(window)
                    && state.selected_range().is_empty()
                    && state.marked_text_range(window, cx).is_none()
                    && MentionCompletionProvider::extract_mention_query(&text, state.cursor())
                        .is_none()
                    && SlashCompletionProvider::extract_slash_query(&text, state.cursor()).is_none()
                    && cursor_is_at_history_boundary(direction, &text, state.cursor())
            });
            if !can_navigate {
                return;
            }

            let handled = this.update(cx, |this, cx| this.navigate_history(direction, window, cx));
            if handled {
                cx.stop_propagation();
            }
        });

        let context_search_input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder(t!("AgentUi.search_targets").to_string())
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

        // 模型下拉交给组件库的 `Select`(自带搜索框、虚拟滚动与键盘导航)。
        // 选项这里先留空:它由 `set_menu_options` 置脏、`render` 里带窗口灌进去。
        let model_select: Entity<SelectState<SearchableVec<SelectGroup<ModelChoice>>>> =
            cx.new(|cx| {
                SelectState::<SearchableVec<SelectGroup<ModelChoice>>>::new(
                    SearchableVec::new(Vec::new()),
                    None,
                    window,
                    cx,
                )
                .searchable(true)
            });
        let model_select_sub = cx.subscribe(
            &model_select,
            |_this: &mut Self,
             _state: Entity<SelectState<SearchableVec<SelectGroup<ModelChoice>>>>,
             event: &SelectEvent<SearchableVec<SelectGroup<ModelChoice>>>,
             cx: &mut Context<Self>| {
                // 用户确认了一项:把完整选项回报给上层(它据此重建运行时)。
                let SelectEvent::Confirm(Some(option)) = event else {
                    return;
                };
                cx.emit(AgentInputEvent::SelectModel {
                    id: option.id.clone(),
                    provider_id: option.provider_id.clone(),
                    model: option.model.clone(),
                });
            },
        );

        Self {
            focus_handle: cx.focus_handle(),
            input_state,
            mentions,
            slash_commands: Vec::new(),
            attachments: Vec::new(),
            history: PromptHistory::default(),
            queued_submissions: Vec::new(),
            pending_queue_blocked: false,
            pending_elicitation: None,
            is_running: false,
            context: AgentComposerContext::default(),
            target_options: Vec::new(),
            model_options: Vec::new(),
            model_select,
            model_select_dirty: false,
            execution_mode_options: Vec::new(),
            context_search_input,
            context_search_query: SharedString::default(),
            selected_resource_kind_filter: SharedString::from("all"),
            context_search_needs_reset: false,
            open_menu: None,
            expanded_plan_items: HashSet::new(),
            top_capabilities_collapsed: false,
            theme: None,
            edge_to_edge: false,
            _subscriptions: vec![
                enter_sub,
                paste_sub,
                history_sub,
                context_search_sub,
                model_select_sub,
            ],
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
        self.refresh_completion_provider(cx);
    }

    /// 更新 `/` 命令补全来源(Acp agent 推送的 `available_commands`)。
    pub fn set_slash_commands(
        &mut self,
        commands: Vec<SlashCommandItem>,
        cx: &mut Context<Self>,
    ) {
        // 推送可能很频繁(每次 `session/update` 都可能带),内容没变就不重建 provider。
        if self.slash_commands == commands {
            return;
        }
        self.slash_commands = commands;
        self.refresh_completion_provider(cx);
    }

    fn refresh_completion_provider(&self, cx: &mut Context<Self>) {
        let mentions = (*self.mentions).clone();
        let commands = self.slash_commands.clone();
        self.input_state.update(cx, |state, _| {
            state.lsp_mut().completion_provider = Some(Rc::new(
                ComposerCompletionProvider::new(mentions, commands),
            ));
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

    /// 注入底部模型与工具执行模式下拉的选项。
    pub fn set_menu_options(
        &mut self,
        model_options: Vec<ComposerModelOption>,
        execution_mode_options: Vec<ComposerMenuOption>,
        cx: &mut Context<Self>,
    ) {
        if self.model_options != model_options {
            self.model_options = model_options;
            // `SelectState` 的更新要窗口,而这里只有 `Context`;置脏,`render` 时同步。
            self.model_select_dirty = true;
        }
        self.execution_mode_options = execution_mode_options;
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

    /// 更新当前会话等待下一轮执行的提交摘要。
    pub fn set_queued_submissions(
        &mut self,
        submissions: Vec<QueuedPromptPreview>,
        cx: &mut Context<Self>,
    ) {
        if self.queued_submissions != submissions {
            self.queued_submissions = submissions;
            cx.notify();
        }
    }

    /// 设置当前会话的待执行队列是否处于阻塞状态。
    pub fn set_pending_queue_blocked(&mut self, blocked: bool, cx: &mut Context<Self>) {
        if self.pending_queue_blocked != blocked {
            self.pending_queue_blocked = blocked;
            cx.notify();
        }
    }

    /// 聚焦输入框。
    pub fn focus_input(&self, window: &mut Window, cx: &mut App) {
        let handle = self.input_state.read(cx).focus_handle(cx);
        handle.focus(window, cx);
    }

    /// 当前输入框文本。
    pub fn composer_text(&self, cx: &App) -> String {
        self.input_state.read(cx).value().to_string()
    }

    /// 把一段排队提交放回输入框并聚焦（供「编辑队列项」使用）。
    ///
    /// 文本与附件一起回填，避免「编辑」变成静默丢附件。
    /// **从队列中移除由上层完成**：队列事实源在 `AgentChatView::pending_submissions`，
    /// 输入框只是渲染副本。
    pub fn restore_to_composer(
        &mut self,
        text: &str,
        images: Vec<ImageAttachment>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.input_state.update(cx, |state, cx| {
            state.set_value(text, window, cx);
        });
        if !images.is_empty() {
            self.add_attachments(images, cx);
        }
        self.focus_input(window, cx);
        cx.notify();
    }

    /// 设置输入框文本，不动附件、不抢焦点（会话草稿恢复专用）。
    ///
    /// 与 [`Self::restore_to_composer`] 的区别：那是「用户显式编辑」——要聚焦、
    /// 要带回附件；这里是「切了个会话，输入框该显示那个会话自己的草稿」——
    /// 只是换内容，用户正在别处打字的话不该被拽走焦点。
    pub fn set_composer_text(&mut self, text: &str, window: &mut Window, cx: &mut Context<Self>) {
        self.input_state.update(cx, |state, cx| {
            state.set_value(text, window, cx);
        });
        cx.notify();
    }

    /// 当前输入框挂着的图片附件（会话草稿捕获用，只读）。
    pub fn composer_attachments(&self) -> &[ImageAttachment] {
        &self.attachments
    }

    /// 整体替换输入框的图片附件（会话草稿恢复专用，与 [`Self::set_composer_text`] 同一套纪律）。
    ///
    /// **整体替换**是防串台的关键：切到一个没有附件草稿的会话时，上一会话挂在
    /// 输入框里的图片必须被清掉，而不是留下来跟着新会话一起发出去。
    /// 不聚焦、不追加：只是换内容。
    pub fn set_composer_attachments(
        &mut self,
        images: Vec<ImageAttachment>,
        cx: &mut Context<Self>,
    ) {
        self.attachments = images;
        cx.notify();
    }

    fn submit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let text = self.input_state.read(cx).value().to_string();
        if text.trim().is_empty() && self.attachments.is_empty() {
            return;
        }

        self.history.record(&text);
        let mentions = self.referenced_mentions(&text);
        let images = std::mem::take(&mut self.attachments);

        cx.emit(AgentInputEvent::Submit {
            text,
            mentions,
            images,
        });

        self.input_state.update(cx, |state, cx| {
            state.set_value("", window, cx);
        });
        cx.notify();
    }

    fn navigate_history(
        &mut self,
        direction: HistoryDirection,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let current = self.input_state.read(cx).text().to_string();
        let was_browsing = self.history.is_browsing();
        let replacement = match direction {
            HistoryDirection::Previous => self.history.previous(&current),
            HistoryDirection::Next => self.history.next(),
        };
        let Some(replacement) = replacement else {
            return was_browsing;
        };

        self.input_state.update(cx, |state, cx| {
            state.replace_all(replacement, window, cx);
            let new_len = state.text().len();
            state.set_selected_range(new_len..new_len, cx);
        });
        true
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

    /// 打开系统文件对话框选择图片。
    fn open_file_picker(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        let rx = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: true,
            prompt: Some(t!("AgentUi.choose_images").to_string().into()),
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

    pub(super) fn toggle_skill(&mut self, id: SharedString, cx: &mut Context<Self>) {
        if !self.is_running {
            cx.emit(AgentInputEvent::ToggleSkill { id });
        }
    }

    pub(crate) fn is_running(&self) -> bool {
        self.is_running
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
                .child(self.render_context_mode_tab(cx))
                .child(self.render_mode_separator(cx))
                .child(self.render_skill_mode_tab(cx)),
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
            .debug_selector(|| "agent-context-mode".to_string())
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

    fn render_skill_mode_tab(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let view = cx.entity();
        let is_open = self.open_menu == Some(ComposerMenuKind::Skill);
        let summary = self.context.skill_summary.clone();
        let items = self.context.skill_items.clone();

        Popover::new("agent-skill-popover")
            .p_0()
            .open(is_open)
            .on_open_change({
                let view = view.clone();
                move |open, _window, cx| {
                    let open = *open;
                    view.update(cx, |this, cx| {
                        this.open_menu =
                            menu_state_after_open_change(open, ComposerMenuKind::Skill);
                        cx.notify();
                    });
                }
            })
            .trigger(self.render_capability_trigger(
                "agent-skill-trigger",
                skill_trigger_label(&summary),
                IconName::BookOpen,
                cx,
            ))
            .content({
                move |_state, _window, cx| {
                    render_skill_mode_content(view.clone(), summary.clone(), items.clone(), cx)
                }
            })
    }

    fn render_mode_separator(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let theme = self.local_theme(cx);
        div().h(px(20.0)).w(px(1.0)).bg(theme.border)
    }

    /// 底部那一行的「权限级别」下拉。
    ///
    /// 承载的仍是 ACP 工具执行模式(自动 / 只读 / 手动确认):状态、选项与回调都与
    /// 原先工具栏里的那个下拉一致,只是收成了 chip 形态。
    fn render_permission_menu(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let view = cx.entity();
        let is_open = self.open_menu == Some(ComposerMenuKind::Mode);
        let label = current_execution_mode_label(&self.context.execution_mode_label);
        let data = ModeContentData {
            execution_mode_label: label.clone(),
            options: self.execution_mode_options.clone(),
        };

        let theme = self.local_theme(cx);
        let queue_mode = self.is_running || self.pending_queue_blocked;
        let chip_width = execution_trigger_width(&label, queue_mode);
        let trigger = themed_outline_button(
            Button::new("agent-permission")
                .debug_selector(|| "agent-input-permission".to_string())
                .small()
                .w_full()
                .h(px(COMPOSER_CHIP_HEIGHT))
                .justify_between()
                .outline()
                .disabled(self.is_running)
                .child(composer_chip_label(IconName::Key, label)),
            &theme,
        );

        div()
            .w(chip_width)
            .min_w(px(COMPOSER_CHIP_MIN_WIDTH))
            .flex_shrink(1.0)
            .h(px(COMPOSER_CHIP_HEIGHT))
            .overflow_hidden()
            .child(
                Popover::new("agent-permission-popover")
                    .p_0()
                    .open(is_open)
                    .trigger_style(StyleRefinement::default().w_full())
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
                    }),
            )
    }

    /// 底部那一行的「分支」下拉。
    ///
    /// 选项由上层注入(见 [`AgentComposerContext::branch_options`]);选中后只 emit
    /// [`AgentInputEvent::SelectBranch`],真正的切分支动作交给宿主。
    fn render_branch_menu(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let view = cx.entity();
        let is_open = self.open_menu == Some(ComposerMenuKind::Branch);
        let current = self
            .context
            .branch_options
            .iter()
            .find(|branch| branch.current)
            .map(|branch| branch.name.clone())
            .unwrap_or_else(|| {
                SharedString::from(t!("AgentUi.composer_branch").to_string())
            });
        let options = self.context.branch_options.clone();

        let theme = self.local_theme(cx);
        let trigger = themed_outline_button(
            Button::new("agent-branch")
                .debug_selector(|| "agent-input-branch".to_string())
                .small()
                .w_full()
                .h(px(COMPOSER_CHIP_HEIGHT))
                .justify_between()
                .outline()
                .disabled(self.is_running)
                .child(composer_chip_label(IconName::GitBranch, current.clone())),
            &theme,
        );

        div()
            .w(composer_chip_width(&current))
            .min_w(px(COMPOSER_CHIP_MIN_WIDTH))
            .flex_shrink(1.0)
            .h(px(COMPOSER_CHIP_HEIGHT))
            .overflow_hidden()
            .child(
                Popover::new("agent-branch-popover")
                    .p_0()
                    .open(is_open)
                    .trigger_style(StyleRefinement::default().w_full())
                    .on_open_change({
                        let view = view.clone();
                        move |open, _window, cx| {
                            let open = *open;
                            view.update(cx, |this, cx| {
                                this.open_menu = if open && !this.is_running {
                                    Some(ComposerMenuKind::Branch)
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
                            render_branch_content(view.clone(), options.clone(), &theme, cx)
                        }
                    }),
            )
    }

    /// 底部那一行的「工作区」下拉。
    ///
    /// 候选由上层注入(见 [`AgentComposerContext::workspace_options`])，与左侧会话
    /// 导航里的工作区下拉同源。这里不自行实现选择逻辑：选中只 emit
    /// [`AgentInputEvent::SelectWorkspace`]，「会话已有消息则在新工作区开新对话」的
    /// 判定留在宿主。
    fn render_workspace_menu(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let view = cx.entity();
        let is_open = self.open_menu == Some(ComposerMenuKind::Workspace);
        let label = if self.context.workspace.label.is_empty() {
            SharedString::from(t!("AgentUi.composer_pick_workspace").to_string())
        } else {
            self.context.workspace.label.clone()
        };
        let options = self.context.workspace_options.clone();
        let current_path = self.context.workspace.path.clone();

        let theme = self.local_theme(cx);
        let trigger = themed_outline_button(
            Button::new("agent-workspace")
                .debug_selector(|| "agent-input-workspace".to_string())
                .small()
                .w_full()
                .h(px(COMPOSER_CHIP_HEIGHT))
                .justify_between()
                .outline()
                .disabled(self.is_running)
                .tooltip(
                    current_path
                        .clone()
                        .map(|path| path.to_string())
                        .unwrap_or_else(|| t!("AgentUi.composer_pick_workspace").to_string()),
                )
                .child(composer_chip_label(IconName::Workspace, label.clone())),
            &theme,
        );

        div()
            .w(composer_chip_width(&label))
            .min_w(px(COMPOSER_CHIP_MIN_WIDTH))
            .flex_shrink(1.0)
            .h(px(COMPOSER_CHIP_HEIGHT))
            .overflow_hidden()
            .child(
                Popover::new("agent-workspace-popover")
                    .p_0()
                    .open(is_open)
                    .trigger_style(StyleRefinement::default().w_full())
                    .on_open_change({
                        let view = view.clone();
                        move |open, _window, cx| {
                            let open = *open;
                            view.update(cx, |this, cx| {
                                this.open_menu = if open && !this.is_running {
                                    Some(ComposerMenuKind::Workspace)
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
                            render_workspace_content(
                                view.clone(),
                                options.clone(),
                                current_path.clone(),
                                &theme,
                                cx,
                            )
                        }
                    }),
            )
    }

    /// 底部那一行的「Worktree」勾选。
    ///
    /// 勾选态来自 [`ComposerWorktreeState::enabled`];切换只 emit
    /// [`AgentInputEvent::ToggleWorktree`],建/切 worktree 由宿主完成。
    ///
    /// 勾选后不会立刻创建:宿主只记下意图,等本会话第一次发送时才建
    /// (见 `ComposerWorktreeState::pending`)。所以待创建阶段的 tooltip
    /// 要改成「首次对话时创建」,否则用户会以为勾完就已经建好了。
    fn render_worktree_toggle(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let view = cx.entity();
        let label = self
            .context
            .worktree
            .label
            .clone()
            .unwrap_or_else(|| SharedString::from(t!("AgentUi.composer_worktree").to_string()));
        let checked = self.context.worktree.enabled;
        let hint = if self.context.worktree.pending {
            t!("AgentUi.composer_worktree_pending").to_string()
        } else {
            t!("AgentUi.composer_worktree_hint").to_string()
        };

        div()
            .debug_selector(|| "agent-input-worktree".to_string())
            .w(composer_chip_width(&label))
            .min_w(px(COMPOSER_CHIP_MIN_WIDTH))
            .flex_shrink(1.0)
            .h(px(COMPOSER_CHIP_HEIGHT))
            .overflow_hidden()
            .flex()
            .items_center()
            .child(
                Checkbox::new("agent-worktree-toggle")
                    .checked(checked)
                    .disabled(self.is_running)
                    .tooltip(hint)
                    .label(label)
                    .on_click(move |checked, _window, cx| {
                        let enabled = *checked;
                        view.update(cx, |this, cx| {
                            this.open_menu = None;
                            cx.emit(AgentInputEvent::ToggleWorktree { enabled });
                            cx.notify();
                        });
                    }),
            )
    }

    /// 底部那一行里「附件 / 上下文档位 / 操作按钮」三段中的中间段。
    ///
    /// 顺序:`[工作区][分支][模型][Worktree][权限]`(非 Git 工作区里分支与 Worktree 隐藏)。
    /// 这一组独占剩余宽度:空间不足时先在组内 `flex_shrink` + `truncate` 消化,
    /// 由本组自己的 `overflow_hidden` 兜底,绝不把右侧的发送按钮挤出可视区。
    fn render_context_group(&self, cx: &mut Context<Self>, row_gap: Pixels) -> impl IntoElement + use<> {
        let is_git_repo =
            self.context.workspace.is_git_repo || !self.context.branch_options.is_empty();
        let model_label = match &self.context.model {
            Some(m) if m.model_only => m.model.clone(),
            Some(m) => SharedString::from(format!("{} / {}", m.provider, m.model)),
            None => SharedString::from(t!("AgentUi.select_model").to_string()),
        };
        let model_min_width = if self.is_running || self.pending_queue_blocked {
            96.0
        } else {
            150.0
        };
        let action_button_size = px(TOOLBAR_ACTION_BUTTON_SIZE);

        let mut group = h_flex()
            .debug_selector(|| "agent-input-context-group".to_string())
            .flex_1()
            .min_w_0()
            .overflow_hidden()
            .items_center()
            .gap(row_gap)
            .text_color(self.local_theme(cx).foreground)
            .child(self.render_workspace_menu(cx));
        if is_git_repo {
            group = group.child(self.render_branch_menu(cx));
        }
        // 模型留在这一行中间:它是唯一需要「搜索 + 滚动 + 键盘导航」的控件,
        // 用 `flex_1` 吸收整行余量,其余项保持内容宽度。
        group = group.child(
            div()
                .flex_1()
                .min_w(px(model_min_width))
                .h(action_button_size)
                .overflow_hidden()
                .child(self.render_model_menu(model_label))
                .debug_selector(|| "agent-input-model-control".to_string()),
        );
        if is_git_repo {
            group = group.child(self.render_worktree_toggle(cx));
        }
        group.child(self.render_permission_menu(cx))
    }

    /// 模型下拉:交给组件库的 `Select` —— 搜索框、滚动、键盘导航都由它提供。
    ///
    /// 选项不从这里传:它们在 `set_menu_options` 时置脏、`render` 里灌进 `model_select`。
    fn render_model_menu(&self, placeholder: SharedString) -> impl IntoElement + use<> {
        Select::new(&self.model_select)
            .id("agent-model")
            .w_full()
            .h(px(32.0))
            .small()
            .placeholder(placeholder)
            .search_placeholder(t!("AgentUi.search_model").to_string())
            .menu_max_h(px(320.0))
            .disabled(self.is_running)
    }

    /// 把当前选项与选中项灌进组件库的 `Select`。
    ///
    /// 只在 `model_select_dirty` 时调用:`set_items` 会触发重渲染,必须靠脏标记保证
    /// 「一次同步即收敛」,否则渲染 → 通知 → 渲染会自己转起来。
    fn sync_model_select(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let groups = model_groups(&self.model_options);
        let selected = selected_model_index(&self.model_options, self.context.model.as_ref());
        self.model_select.update(cx, |state, cx| {
            state.set_items(groups, window, cx);
            state.set_selected_index(selected, window, cx);
        });
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

    fn render_queued_submissions(&self, cx: &mut Context<Self>) -> Option<gpui::AnyElement> {
        if self.queued_submissions.is_empty() {
            return None;
        }

        let theme = self.local_theme(cx);
        let danger = cx.theme().danger;
        let count = self.queued_submissions.len();
        let mut items = v_flex().w_full().gap_1();
        for (index, submission) in self.queued_submissions.iter().enumerate() {
            let text = if submission.text.trim().is_empty() {
                t!("AgentUi.attachment_count", count = submission.image_count).to_string()
            } else if submission.image_count == 0 {
                submission.text.to_string()
            } else {
                format!(
                    "{} · {}",
                    submission.text,
                    t!("AgentUi.attachment_count", count = submission.image_count)
                )
            };
            items = items.child(
                h_flex()
                    .debug_selector(|| "agent-input-queued-item".to_string())
                    .w_full()
                    .min_w_0()
                    .items_center()
                    .gap_1()
                    .px_2()
                    .py_1()
                    .rounded(cx.theme().radius)
                    .bg(theme.panel)
                    .text_xs()
                    .text_color(theme.muted_foreground)
                    .child(
                        div()
                            .flex_shrink_0()
                            .text_xs()
                            .text_color(theme.muted_foreground)
                            .child(format!("{}", index + 1)),
                    )
                    .child(div().flex_1().min_w_0().truncate().child(text))
                    .child(
                        Button::new(SharedString::from(format!("agent-queued-edit-{index}")))
                            .icon(IconName::Edit)
                            .ghost()
                            .small()
                            .tooltip(t!("AgentUi.edit_queued_item").to_string())
                            .on_click(cx.listener(move |_this, _, window, cx| {
                                cx.emit(AgentInputEvent::EditQueued { index });
                                // 焦点在此刻转移由上层回填文本后再落定；
                                // 这里只负责把窗口从按钮上摘下来。
                                window.refresh();
                                cx.notify();
                            })),
                    )
                    .child(
                        Button::new(SharedString::from(format!("agent-queued-remove-{index}")))
                            .icon(IconName::Delete)
                            .ghost()
                            .small()
                            .text_color(danger)
                            .tooltip(t!("AgentUi.remove_queued_item").to_string())
                            .on_click(cx.listener(move |_this, _, _window, cx| {
                                cx.emit(AgentInputEvent::RemoveQueued { index });
                                cx.notify();
                            })),
                    ),
            );
        }

        Some(
            v_flex()
                .debug_selector(|| "agent-input-queued".to_string())
                .w_full()
                .min_w_0()
                .gap_1()
                .px_3()
                .pt_1()
                .child(
                    div()
                        .text_xs()
                        .text_color(theme.foreground)
                        .child(t!("AgentUi.queued_for_next_turn", count = count).to_string()),
                )
                .child(items)
                .into_any_element(),
        )
    }

    fn render_editor_top_bar(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        // Finch 式改版：附件入口迁到底部 chips 行（render_toolbar），顶栏只留
        // 能力折叠与撤销，靠右排布。
        h_flex()
            .w_full()
            .items_center()
            .justify_end()
            .px_3()
            .pt_2()
            .pb_1()
            .child(
                Button::new("agent-editor-menu")
                    .icon(if self.top_capabilities_collapsed {
                        IconName::ChevronUp
                    } else {
                        IconName::ChevronDown
                    })
                    .ghost()
                    .small()
                    .tooltip(t!("AgentUi.collapse_capabilities").to_string())
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
                    .tooltip(t!("AgentUi.undo").to_string()),
            )
    }

    fn render_toolbar(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let theme = self.local_theme(cx);
        let running = self.is_running;
        let queue_mode = running || self.pending_queue_blocked;
        let row_gap = if queue_mode { px(4.0) } else { px(8.0) };
        let action_button_size = px(TOOLBAR_ACTION_BUTTON_SIZE);
        let attach_count = self.attachments.len();

        // 左段:附件入口 + 附件计数(固定宽度,不参与收缩)。
        let attach_group = h_flex()
            .flex_shrink_0()
            .items_center()
            .gap(row_gap)
            .child(
                Button::new("agent-attach")
                    .icon(IconName::File)
                    .ghost()
                    .small()
                    .tooltip(t!("AgentUi.attach_images").to_string())
                    .on_click(
                        cx.listener(|this, _, window, cx| this.open_file_picker(window, cx)),
                    ),
            )
            .child(
                div()
                    .text_xs()
                    .text_color(theme.muted_foreground)
                    .child(t!("AgentUi.attachment_count", count = attach_count).to_string()),
            )
            .child(div().h(px(18.0)).w(px(1.0)).bg(theme.border));

        // 中段:工作区 / 分支 / 模型 / Worktree / 权限。
        let context_group = self.render_context_group(cx, row_gap);

        let mut toolbar = h_flex()
            .debug_selector(|| "agent-input-toolbar".to_string())
            .w_full()
            .min_w_0()
            .items_center()
            .text_color(theme.foreground)
            .gap(row_gap)
            .px_3()
            .py_2()
            // 极端窄宽度下宁可裁掉中段的尾巴,也不让内容画到输入卡片外面。
            .overflow_hidden()
            .flex_shrink_0()
            .child(attach_group)
            .child(context_group);

        if queue_mode {
            toolbar = toolbar
                .child(
                    div()
                        .w(action_button_size)
                        .h(action_button_size)
                        .flex_shrink_0()
                        .debug_selector(|| "agent-input-queue-send".to_string())
                        .child(
                            Button::new("agent-queue-send")
                                .debug_selector(|| "agent-queue-send-button".to_string())
                                .icon(IconName::ArrowUp)
                                .primary()
                                .small()
                                .size(action_button_size)
                                .tooltip(t!("AgentUi.queue_for_next_turn").to_string())
                                .on_click(
                                    cx.listener(|this, _, window, cx| this.submit(window, cx)),
                                ),
                        ),
                )
                .child(
                    div()
                        .w(action_button_size)
                        .h(action_button_size)
                        .flex_shrink_0()
                        .debug_selector(|| "agent-input-stop".to_string())
                        .child(
                            Button::new("agent-stop")
                                .debug_selector(|| "agent-stop-button".to_string())
                                .icon(IconName::Close)
                                .danger()
                                .small()
                                .size(action_button_size)
                                .tooltip(t!("AgentUi.stop").to_string())
                                .on_click(cx.listener(|this, _, _, cx| this.stop(cx))),
                        ),
                );
        } else {
            toolbar = toolbar.child(
                div()
                    .w(action_button_size)
                    .h(action_button_size)
                    .flex_shrink_0()
                    .debug_selector(|| "agent-input-send-control".to_string())
                    .child(
                        Button::new("agent-send")
                            .debug_selector(|| "agent-send-button".to_string())
                            .icon(IconName::ArrowUp)
                            .primary()
                            .small()
                            .size(action_button_size)
                            .tooltip(t!("AgentUi.send").to_string())
                            .on_click(cx.listener(|this, _, window, cx| this.submit(window, cx))),
                    ),
            );
        }

        toolbar
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
    execution_mode_label: SharedString,
    options: Vec<ComposerMenuOption>,
}

struct ModeOptionRow {
    option: ComposerMenuOption,
    selected: bool,
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

    col = col.child(context_group_label(t!("AgentUi.mode").to_string(), theme));
    for option in data.options {
        let selected = option.label == data.execution_mode_label;
        col = col.child(mode_option_row(
            view.clone(),
            ModeOptionRow { option, selected },
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
    let row_id = SharedString::from(format!("agent-execution-mode-opt-{id}"));
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
                cx.emit(AgentInputEvent::SelectExecutionMode { id });
                cx.notify();
            });
        })
        .into_any_element()
}

/// 工作区下拉的菜单内容。
fn render_workspace_content(
    view: Entity<AgentInput>,
    options: Vec<ComposerWorkspaceOption>,
    current_path: Option<SharedString>,
    theme: &AgentChatTheme,
    cx: &mut Context<gpui_component::popover::PopoverState>,
) -> gpui::AnyElement {
    let mut col = v_flex()
        .p_1()
        .gap(px(2.0))
        .min_w(px(260.0))
        .bg(theme.background)
        .text_color(theme.foreground);

    col = col.child(context_group_label(
        t!("AgentUi.workspace").to_string(),
        theme,
    ));

    if options.is_empty() {
        col = col.child(
            div()
                .px_2()
                .py_2()
                .text_sm()
                .text_color(theme.muted_foreground)
                .child(t!("AgentUi.composer_workspace_empty").to_string()),
        );
    } else {
        for option in options {
            let current = current_path.as_deref() == Some(option.path.as_ref());
            col = col.child(workspace_option_row(
                view.clone(),
                option,
                current,
                theme,
                cx,
            ));
        }
    }

    // 与候选列表之间加一条分隔线：下面这项是「打开新目录」，不是同级候选。
    col.child(
        div()
            .my_1()
            .h(px(1.0))
            .w_full()
            .bg(theme.border),
    )
    .child(browse_workspace_row(view, theme, cx))
    .into_any_element()
}

/// 工作区下拉里的一行候选。
fn workspace_option_row(
    view: Entity<AgentInput>,
    option: ComposerWorkspaceOption,
    current: bool,
    theme: &AgentChatTheme,
    cx: &mut Context<gpui_component::popover::PopoverState>,
) -> gpui::AnyElement {
    let muted = theme.muted_foreground;
    let hover_bg = theme.hover_background();
    let selected_bg = theme.selection_background();
    let accent = theme.accent;

    h_flex()
        .id(option.element_id())
        .w_full()
        .min_w_0()
        .items_center()
        .gap_2()
        .px_2()
        .py_1p5()
        .rounded(cx.theme().radius)
        .cursor_pointer()
        .when(current, |this| this.bg(selected_bg))
        .hover(move |this| this.bg(hover_bg))
        .child(Icon::new(IconName::Folder).xsmall().flex_shrink_0())
        .child(
            v_flex()
                .flex_1()
                .min_w_0()
                .gap(px(1.0))
                .child(div().text_sm().truncate().child(option.label.clone()))
                // 完整路径作为副标题：重名目录靠它区分。
                .child(
                    div()
                        .text_xs()
                        .text_color(muted)
                        .truncate()
                        .child(option.path.clone()),
                ),
        )
        .when(current, |this| {
            this.child(Icon::new(IconName::Check).xsmall().text_color(accent))
        })
        .on_click(move |_, _, cx| {
            let path = option.path.clone();
            view.update(cx, |this, cx| {
                if this.is_running {
                    return;
                }
                this.open_menu = None;
                cx.emit(AgentInputEvent::SelectWorkspace { path });
                cx.notify();
            });
        })
        .into_any_element()
}

/// 工作区下拉里的「选择其它目录…」：交给宿主弹目录选择器。
fn browse_workspace_row(
    view: Entity<AgentInput>,
    theme: &AgentChatTheme,
    cx: &mut Context<gpui_component::popover::PopoverState>,
) -> gpui::AnyElement {
    let hover_bg = theme.hover_background();

    h_flex()
        .id("composer-workspace-browse")
        .w_full()
        .min_w_0()
        .items_center()
        .gap_2()
        .px_2()
        .py_1p5()
        .rounded(cx.theme().radius)
        .cursor_pointer()
        .hover(move |this| this.bg(hover_bg))
        .child(Icon::new(IconName::Plus).xsmall().flex_shrink_0())
        .child(
            div()
                .flex_1()
                .min_w_0()
                .text_sm()
                .truncate()
                .child(t!("AgentUi.composer_browse_workspace").to_string()),
        )
        .on_click(move |_, _, cx| {
            view.update(cx, |this, cx| {
                this.open_menu = None;
                cx.emit(AgentInputEvent::BrowseWorkspace);
                cx.notify();
            });
        })
        .into_any_element()
}

/// 分支下拉的菜单内容。
fn render_branch_content(
    view: Entity<AgentInput>,
    options: Vec<ComposerBranchOption>,
    theme: &AgentChatTheme,
    cx: &mut Context<gpui_component::popover::PopoverState>,
) -> gpui::AnyElement {
    let mut col = v_flex()
        .p_1()
        .gap(px(2.0))
        .min_w(px(260.0))
        .bg(theme.background)
        .text_color(theme.foreground);

    col = col.child(context_group_label(
        t!("AgentUi.composer_branch").to_string(),
        theme,
    ));

    if options.is_empty() {
        return col
            .child(
                div()
                    .px_2()
                    .py_2()
                    .text_sm()
                    .text_color(theme.muted_foreground)
                    .child(t!("AgentUi.composer_branch_empty").to_string()),
            )
            .into_any_element();
    }

    for option in options {
        col = col.child(branch_option_row(view.clone(), option, theme, cx));
    }

    col.into_any_element()
}

/// 分支下拉里的一行。
fn branch_option_row(
    view: Entity<AgentInput>,
    option: ComposerBranchOption,
    theme: &AgentChatTheme,
    cx: &mut Context<gpui_component::popover::PopoverState>,
) -> gpui::AnyElement {
    let muted = theme.muted_foreground;
    let hover_bg = theme.hover_background();
    let selected_bg = theme.selection_background();
    let selected_fg = theme.accent;
    let current = option.current;
    let remote = option.remote;
    let name = option.name.clone();
    let row_id = option.element_id();

    let mut inner = v_flex()
        .flex_1()
        .min_w_0()
        .gap(px(1.0))
        .child(div().text_sm().truncate().child(name.clone()));
    if remote {
        inner = inner.child(
            div()
                .text_xs()
                .text_color(muted)
                .child(t!("AgentUi.composer_branch_remote").to_string()),
        );
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
        .when(current, |this| {
            this.bg(selected_bg).text_color(theme.foreground)
        })
        .hover(move |this| this.bg(hover_bg))
        .child(inner)
        .when(current, |this| {
            this.child(Icon::new(IconName::Check).xsmall().text_color(selected_fg))
        })
        .on_click(move |_, _window, cx| {
            let name = name.clone();
            view.update(cx, |this, cx| {
                if this.is_running {
                    return;
                }
                this.open_menu = None;
                cx.emit(AgentInputEvent::SelectBranch { name });
                cx.notify();
            });
        })
        .into_any_element()
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

    col = col.child(context_group_label(
        t!("AgentUi.plan_todo").to_string(),
        &theme,
    ));
    if items.is_empty() {
        return col
            .child(
                div()
                    .px_2()
                    .py_2()
                    .text_sm()
                    .text_color(muted)
                    .child(t!("AgentUi.no_plan").to_string()),
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
        return SharedString::from(t!("AgentUi.plan").to_string());
    }
    let total = items.len();
    let completed = items
        .iter()
        .filter(|item| is_completed_plan_status(item.status.as_ref()))
        .count();
    let label = if completed == total {
        t!("AgentUi.completed").to_string()
    } else if items
        .iter()
        .any(|item| is_running_plan_status(item.status.as_ref()))
    {
        t!("AgentUi.in_progress").to_string()
    } else {
        t!("AgentUi.pending").to_string()
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
        details = details.child(plan_detail_line(
            t!("AgentUi.risk").to_string(),
            item.risk,
            muted,
        ));
    }
    if let Some(tool) = item.tool {
        details = details.child(plan_detail_line(
            t!("AgentUi.tool").to_string(),
            tool,
            muted,
        ));
    }
    details.into_any_element()
}

fn plan_detail_line(
    label: impl Into<SharedString>,
    value: SharedString,
    muted: gpui::Hsla,
) -> gpui::AnyElement {
    h_flex()
        .w_full()
        .min_w_0()
        .gap_1()
        .text_xs()
        .child(div().flex_shrink_0().text_color(muted).child(label.into()))
        .child(div().flex_1().min_w_0().truncate().child(value))
        .into_any_element()
}

fn plan_status_label(status: &str) -> String {
    match status {
        "completed" => t!("AgentUi.completed_state").to_string(),
        "running" | "in_progress" => t!("AgentUi.in_progress").to_string(),
        "failed" => t!("AgentUi.failed").to_string(),
        "cancelled" => t!("AgentUi.cancelled").to_string(),
        _ => t!("AgentUi.pending").to_string(),
    }
}

fn render_subagent_mode_content(
    subagents: Vec<ComposerSubAgentItem>,
    cx: &mut Context<gpui_component::popover::PopoverState>,
) -> gpui::AnyElement {
    let theme = active_agent_chat_theme(cx);
    let muted = theme.muted_foreground;
    let mut col = v_flex().p_1().gap(px(2.0)).min_w(px(300.0));

    col = col.child(context_group_label(
        t!("AgentUi.subagents").to_string(),
        &theme,
    ));
    if subagents.is_empty() {
        col = col.child(
            div()
                .px_2()
                .py_2()
                .text_sm()
                .text_color(muted)
                .child(t!("AgentUi.no_subagents").to_string()),
        );
    }
    for subagent in subagents {
        col = col.child(subagent_item_row(subagent, muted, cx));
    }
    col.into_any_element()
}

fn subagent_trigger_label(subagents: &[ComposerSubAgentItem]) -> SharedString {
    if subagents.is_empty() {
        SharedString::from(t!("AgentUi.subagents").to_string())
    } else {
        SharedString::from(t!("AgentUi.subagent_count", count = subagents.len()).to_string())
    }
}

fn resource_pool_trigger_label(context: &AgentComposerContext) -> SharedString {
    if context.resource_pool.total_resources == 0 {
        return SharedString::from(t!("AgentUi.resource_pool").to_string());
    }
    SharedString::from(
        t!(
            "AgentUi.resource_pool_count",
            count = context.resource_pool.total_resources
        )
        .to_string(),
    )
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
                        .child(t!("AgentUi.purpose", value = item.task).to_string()),
                )
                .when(!item.summary.is_empty(), |this| {
                    this.child(
                        div()
                            .text_xs()
                            .text_color(muted)
                            .truncate()
                            .child(t!("AgentUi.progress", value = item.summary).to_string()),
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
            .child(context_group_label(
                t!("AgentUi.default_target").to_string(),
                &theme,
            ))
            .child(context_summary_row(target, muted, &theme));
    }
    let has_database_scope = scopes.iter().any(|scope| scope.key.as_ref() == "database");
    if !scopes.is_empty() {
        col = col
            .px_1()
            .child(context_group_label(t!("AgentUi.scope").to_string(), &theme));
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
        let empty_message = if search_query.is_empty() {
            t!("AgentUi.resource_pool_empty").to_string()
        } else {
            t!("AgentUi.no_matching_resources").to_string()
        };
        list = list.child(
            div()
                .px_2()
                .py_2()
                .text_sm()
                .text_color(muted)
                .child(empty_message),
        );
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
    let action_button = Button::new(SharedString::from(format!(
        "resource-pool-action-{}",
        action_id
    )))
    .ghost()
    .xsmall()
    .label(action_label)
    .disabled(resource_pool_action_disabled(&item));
    let action_button = if in_pool {
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
        .debug_selector(|| "agent-resource-pool-row".to_string())
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
                .gap(px(2.0))
                .child(
                    h_flex()
                        .w_full()
                        .min_w_0()
                        .items_center()
                        .gap(px(4.0))
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .text_sm()
                                .truncate()
                                .child(item.label),
                        )
                        .when(item.is_default, |this| {
                            this.child(resource_pool_badge(
                                SharedString::from(t!("AgentUi.default").to_string()),
                                theme.selection_background(),
                                theme.foreground,
                            ))
                        })
                        .child(resource_pool_badge(item.status.clone(), theme.panel, muted)),
                )
                .child(
                    div()
                        .w_full()
                        .min_w_0()
                        .text_xs()
                        .text_color(muted)
                        .truncate()
                        .child(item.primary_meta),
                ),
        )
        .child(action_button)
        .into_any_element()
}

fn resource_pool_badge(
    label: SharedString,
    background: gpui::Hsla,
    foreground: gpui::Hsla,
) -> impl IntoElement {
    div()
        .flex_shrink_0()
        .max_w(px(CONTEXT_KIND_MAX_WIDTH))
        .px_1()
        .py(px(1.0))
        .rounded_sm()
        .bg(background)
        .text_xs()
        .text_color(foreground)
        .truncate()
        .child(label)
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
                || item.primary_meta.to_lowercase().contains(needle)
                || item.kind.to_lowercase().contains(needle)
        })
        .collect()
}

fn resource_pool_action_label(item: &ComposerResourcePoolItem) -> &'static str {
    if item.in_pool { "-" } else { "+" }
}

fn resource_pool_action_disabled(_item: &ComposerResourcePoolItem) -> bool {
    false
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
    SharedString::from(t!("AgentUi.matching_resources", count = filtered_count).to_string())
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
        .child(t!("AgentUi.switch_database_hint").to_string())
        .into_any_element()
}

fn context_group_label(label: impl Into<SharedString>, theme: &AgentChatTheme) -> gpui::AnyElement {
    div()
        .px_2()
        .pt_1()
        .text_xs()
        .text_color(theme.muted_foreground)
        .child(label.into())
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

/// composer 上挂着的「agent 提问」。
///
/// 这是纯展示状态：`AgentInput` 只管渲染表单与收集答案，答完把内容通过
/// [`AgentInputEvent::SubmitElicitation`] 交回上层，由上层送回 ACP——输入框不认识协议。
struct PendingElicitation {
    request_id: String,
    message: String,
    form: Option<PendingElicitationForm>,
    /// URL 模式：用户去外部页面完成，面板只提供入口与取消。
    url: Option<String>,
    /// 协议新增、本客户端不认识的方式：只提示，不猜渲染。
    unsupported_mode: Option<String>,
    /// 校验提示（必填未填 / 数字解析失败）。
    error: Option<String>,
}

struct PendingElicitationForm {
    title: Option<String>,
    description: Option<String>,
    fields: Vec<PendingElicitationField>,
}

struct PendingElicitationField {
    name: String,
    title: String,
    description: Option<String>,
    required: bool,
    control: PendingElicitationControl,
}

enum PendingElicitationControl {
    /// 文本 / 整数 / 小数共用一个输入框，提交时按 `kind` 解析。
    Text {
        kind: ElicitationTextKind,
        /// 输入框在首次渲染时创建——`InputState::new` 需要 `&mut Window`。
        input: Option<Entity<InputState>>,
        placeholder: String,
    },
    Boolean {
        value: bool,
    },
    SingleSelect {
        options: Vec<PendingElicitationChoice>,
        selected: Option<String>,
    },
    MultiSelect {
        options: Vec<PendingElicitationChoice>,
        selected: Vec<String>,
    },
    Unsupported {
        type_name: String,
    },
}

impl PendingElicitationControl {
    /// 有没有能真正收集输入的控件。协议新增的类型不算。
    fn is_editable(&self) -> bool {
        !matches!(self, Self::Unsupported { .. })
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ElicitationTextKind {
    Plain,
    Integer,
    Number,
}

#[derive(Clone)]
struct PendingElicitationChoice {
    value: String,
    title: String,
}

impl PendingElicitation {
    fn from_request(request: &AcpElicitationRequest) -> Self {
        let mut pending = Self {
            request_id: request.request_id.clone(),
            message: request.message.clone(),
            form: None,
            url: None,
            unsupported_mode: None,
            error: None,
        };
        match &request.mode {
            AcpElicitationMode::Form(form) => {
                pending.form = Some(PendingElicitationForm {
                    title: form.title.clone(),
                    description: form.description.clone(),
                    fields: form.fields.iter().map(PendingElicitationField::new).collect(),
                });
            }
            AcpElicitationMode::Url { url } => pending.url = Some(url.clone()),
            AcpElicitationMode::Unsupported { mode } => {
                pending.unsupported_mode = Some(mode.clone());
            }
        }
        pending
    }

    /// 能不能给出结构化答案。URL 模式要靠外部页面完成，这里只能取消。
    fn can_submit(&self) -> bool {
        self.form
            .as_ref()
            .is_some_and(|form| form.fields.iter().any(|field| field.control.is_editable()))
    }

    /// 收集答案。必填项缺失或数字解析失败时返回可直接展示的提示文案。
    fn content(&self, cx: &App) -> Result<BTreeMap<String, Value>, String> {
        let Some(form) = &self.form else {
            return Ok(BTreeMap::new());
        };
        let mut content = BTreeMap::new();
        for field in &form.fields {
            match &field.control {
                PendingElicitationControl::Text { kind, input, .. } => {
                    let raw = input
                        .as_ref()
                        .map(|input| input.read(cx).value().to_string())
                        .unwrap_or_default();
                    let raw = raw.trim();
                    if raw.is_empty() {
                        if field.required {
                            return Err(required_hint(field));
                        }
                        continue;
                    }
                    let value = match kind {
                        ElicitationTextKind::Plain => Value::String(raw.to_string()),
                        ElicitationTextKind::Integer => {
                            raw.parse::<i64>().map(Value::from).map_err(|_| {
                                t!("AgentUi.elicitation_not_integer", field = field.title.clone())
                                    .to_string()
                            })?
                        }
                        ElicitationTextKind::Number => {
                            raw.parse::<f64>().map(Value::from).map_err(|_| {
                                t!("AgentUi.elicitation_not_number", field = field.title.clone())
                                    .to_string()
                            })?
                        }
                    };
                    content.insert(field.name.clone(), value);
                }
                PendingElicitationControl::Boolean { value } => {
                    // 布尔字段永远给值：不填与「否」在这里没有区别，给 false 更符合直觉。
                    content.insert(field.name.clone(), Value::Bool(*value));
                }
                PendingElicitationControl::SingleSelect { selected, .. } => match selected {
                    Some(value) => {
                        content.insert(field.name.clone(), Value::String(value.clone()));
                    }
                    None if field.required => return Err(required_hint(field)),
                    None => {}
                },
                PendingElicitationControl::MultiSelect { selected, .. } => {
                    if selected.is_empty() {
                        if field.required {
                            return Err(required_hint(field));
                        }
                    } else {
                        content.insert(
                            field.name.clone(),
                            Value::Array(selected.iter().cloned().map(Value::String).collect()),
                        );
                    }
                }
                PendingElicitationControl::Unsupported { .. } => {}
            }
        }
        Ok(content)
    }
}

fn required_hint(field: &PendingElicitationField) -> String {
    t!("AgentUi.elicitation_required", field = field.title.clone()).to_string()
}

impl PendingElicitationField {
    fn new(field: &AcpElicitationField) -> Self {
        let control = match &field.kind {
            AcpElicitationFieldKind::Text { format } => PendingElicitationControl::Text {
                kind: ElicitationTextKind::Plain,
                input: None,
                placeholder: format
                    .clone()
                    .map(|format| t!("AgentUi.elicitation_format", format = format).to_string())
                    .unwrap_or_default(),
            },
            AcpElicitationFieldKind::Integer => PendingElicitationControl::Text {
                kind: ElicitationTextKind::Integer,
                input: None,
                placeholder: String::new(),
            },
            AcpElicitationFieldKind::Number => PendingElicitationControl::Text {
                kind: ElicitationTextKind::Number,
                input: None,
                placeholder: String::new(),
            },
            AcpElicitationFieldKind::Boolean => PendingElicitationControl::Boolean {
                value: field.default.as_ref().and_then(Value::as_bool).unwrap_or(false),
            },
            AcpElicitationFieldKind::SingleSelect { options } => {
                PendingElicitationControl::SingleSelect {
                    options: choices(options),
                    selected: None,
                }
            }
            AcpElicitationFieldKind::MultiSelect { options } => PendingElicitationControl::MultiSelect {
                options: choices(options),
                selected: Vec::new(),
            },
            AcpElicitationFieldKind::Unsupported { type_name } => {
                PendingElicitationControl::Unsupported {
                    type_name: type_name.clone(),
                }
            }
        };
        Self {
            name: field.name.clone(),
            title: field.title.clone(),
            description: field.description.clone(),
            required: field.required,
            control,
        }
    }
}

fn choices(options: &[crate::acp::AcpElicitationOption]) -> Vec<PendingElicitationChoice> {
    options
        .iter()
        .map(|option| PendingElicitationChoice {
            value: option.value.clone(),
            title: option.title.clone(),
        })
        .collect()
}

impl AgentInput {
    /// 推入 / 清空「agent 提问」。传 `None` 即收起面板。
    pub fn set_pending_elicitation(
        &mut self,
        request: Option<&AcpElicitationRequest>,
        cx: &mut Context<Self>,
    ) {
        self.pending_elicitation = request.map(PendingElicitation::from_request);
        cx.notify();
    }

    /// 有没有挂着的提问（上层据此决定是否吞掉回车等提交动作）。
    pub fn has_pending_elicitation(&self) -> bool {
        self.pending_elicitation.is_some()
    }

    /// 文本输入框得在拿到 `&mut Window` 之后才能建，因此在渲染前补建一次。
    fn ensure_elicitation_inputs(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(form) = self
            .pending_elicitation
            .as_mut()
            .and_then(|pending| pending.form.as_mut())
        else {
            return;
        };
        for field in &mut form.fields {
            let PendingElicitationControl::Text {
                input, placeholder, ..
            } = &mut field.control
            else {
                continue;
            };
            if input.is_some() {
                continue;
            }
            let placeholder = placeholder.clone();
            *input = Some(cx.new(|cx| {
                InputState::new(window, cx).placeholder(placeholder.clone())
            }));
        }
    }

    fn submit_elicitation(&mut self, cx: &mut Context<Self>) {
        let Some(pending) = self.pending_elicitation.as_ref() else {
            return;
        };
        match pending.content(cx) {
            Ok(content) => cx.emit(AgentInputEvent::SubmitElicitation { content }),
            Err(error) => {
                if let Some(pending) = self.pending_elicitation.as_mut() {
                    pending.error = Some(error);
                }
                cx.notify();
            }
        }
    }

    fn render_pending_elicitation(
        &self,
        cx: &mut Context<Self>,
    ) -> Option<gpui::AnyElement> {
        let pending = self.pending_elicitation.as_ref()?;
        let theme = self.local_theme(cx);
        let mut body = v_flex()
            .debug_selector(|| "agent-input-elicitation".to_string())
            .w_full()
            .min_w_0()
            .gap_2()
            .px_3()
            .py_2()
            .border_t_1()
            .border_color(theme.border)
            .bg(theme.panel);

        body = body.child(
            div()
                .text_xs()
                .text_color(theme.accent)
                .child(t!("AgentUi.elicitation_heading").to_string()),
        );
        if let Some(title) = pending.form.as_ref().and_then(|form| form.title.clone()) {
            body = body.child(div().text_sm().child(title));
        }
        body = body.child(
            div()
                .text_xs()
                .text_color(theme.muted_foreground)
                .child(pending.message.clone()),
        );

        if let Some(form) = &pending.form {
            if let Some(description) = &form.description {
                body = body.child(
                    div()
                        .text_xs()
                        .text_color(theme.muted_foreground)
                        .child(description.clone()),
                );
            }
            body = body.children(
                form.fields
                    .iter()
                    .map(|field| self.render_elicitation_field(field, cx)),
            );
        }
        if let Some(url) = &pending.url {
            body = body.child(
                h_flex()
                    .w_full()
                    .min_w_0()
                    .items_center()
                    .gap_2()
                    .child(
                        Button::new(SharedString::from(format!(
                            "elicitation-open-{}",
                            pending.request_id
                        )))
                        .small()
                        .debug_selector(|| "elicitation-open-url".to_string())
                        .label(t!("AgentUi.elicitation_open_url").to_string())
                        .on_click({
                            let url = url.clone();
                            move |_, _, cx| cx.open_url(&url)
                        }),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .text_xs()
                            .text_color(theme.muted_foreground)
                            .child(url.clone()),
                    ),
            );
        }
        if let Some(mode) = &pending.unsupported_mode {
            body = body.child(
                div()
                    .text_xs()
                    .text_color(theme.muted_foreground)
                    .child(
                        t!("AgentUi.elicitation_unsupported_mode", mode = mode.clone()).to_string(),
                    ),
            );
        }
        if let Some(error) = &pending.error {
            body = body.child(
                div()
                    .text_xs()
                    .text_color(cx.theme().danger)
                    .child(error.clone()),
            );
        }

        let mut actions = h_flex().w_full().min_w_0().justify_end().gap_2().child(
            Button::new(SharedString::from(format!(
                "elicitation-cancel-{}",
                pending.request_id
            )))
            .small()
            .debug_selector(|| "elicitation-cancel".to_string())
            .label(t!("AgentUi.elicitation_cancel").to_string())
            .on_click(cx.listener(|_, _, _, cx| {
                cx.emit(AgentInputEvent::CancelElicitation);
            })),
        );
        if pending.can_submit() {
            actions = actions
                .child(
                    Button::new(SharedString::from(format!(
                        "elicitation-decline-{}",
                        pending.request_id
                    )))
                    .small()
                    .debug_selector(|| "elicitation-decline".to_string())
                    .label(t!("AgentUi.elicitation_decline").to_string())
                    .on_click(cx.listener(|_, _, _, cx| {
                        cx.emit(AgentInputEvent::DeclineElicitation);
                    })),
                )
                .child(
                    Button::new(SharedString::from(format!(
                        "elicitation-submit-{}",
                        pending.request_id
                    )))
                    .small()
                    .primary()
                    .debug_selector(|| "elicitation-submit".to_string())
                    .label(t!("AgentUi.elicitation_submit").to_string())
                    .on_click(cx.listener(|this, _, _, cx| this.submit_elicitation(cx))),
                );
        }
        body = body.child(actions);

        Some(body.into_any_element())
    }

    fn render_elicitation_field(
        &self,
        field: &PendingElicitationField,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let theme = self.local_theme(cx);
        let mut column = v_flex().w_full().min_w_0().gap_1();
        let mut label = h_flex().items_center().gap_1().child(
            div()
                .text_xs()
                .text_color(theme.foreground)
                .child(field.title.clone()),
        );
        if field.required {
            label = label.child(
                div()
                    .text_xs()
                    .text_color(cx.theme().danger)
                    .child("*"),
            );
        }
        column = column.child(label);
        if let Some(description) = &field.description {
            column = column.child(
                div()
                    .text_xs()
                    .text_color(theme.muted_foreground)
                    .child(description.clone()),
            );
        }

        let name = field.name.clone();
        match &field.control {
            PendingElicitationControl::Text { input, .. } => {
                if let Some(input) = input {
                    column = column.child(
                        Input::new(input).cleanable(true).small().w_full(),
                    );
                }
            }
            PendingElicitationControl::Boolean { value } => {
                let checked = *value;
                column = column.child(
                    Switch::new(SharedString::from(format!("elicitation-bool-{name}")))
                        .small()
                        .checked(checked)
                        .on_click(cx.listener({
                            let name = name.clone();
                            move |this, checked, _, cx| {
                                this.toggle_elicitation_boolean(&name, *checked, cx)
                            }
                        })),
                );
            }
            PendingElicitationControl::SingleSelect { options, selected } => {
                let mut row = h_flex().w_full().min_w_0().flex_wrap().gap_1();
                for option in options {
                    let is_selected = selected.as_deref() == Some(option.value.as_str());
                    row = row.child(
                        Button::new(SharedString::from(format!(
                            "elicitation-radio-{name}-{}",
                            option.value
                        )))
                        .small()
                        .debug_selector({
                            let selector = format!("elicitation-radio-{name}-{}", option.value);
                            move || selector.clone()
                        })
                        .when(is_selected, |this| this.primary())
                        .label(option.title.clone())
                        .on_click(cx.listener({
                            let name = name.clone();
                            let value = option.value.clone();
                            move |this, _, _, cx| {
                                this.select_elicitation_option(&name, &value, cx)
                            }
                        })),
                    );
                }
                column = column.child(row);
            }
            PendingElicitationControl::MultiSelect { options, selected } => {
                let mut row = h_flex().w_full().min_w_0().flex_wrap().gap_1();
                for option in options {
                    let is_selected = selected.contains(&option.value);
                    row = row.child(
                        Button::new(SharedString::from(format!(
                            "elicitation-checkbox-{name}-{}",
                            option.value
                        )))
                        .small()
                        .debug_selector({
                            let selector = format!("elicitation-checkbox-{name}-{}", option.value);
                            move || selector.clone()
                        })
                        .when(is_selected, |this| this.primary())
                        .label(option.title.clone())
                        .on_click(cx.listener({
                            let name = name.clone();
                            let value = option.value.clone();
                            move |this, _, _, cx| {
                                this.toggle_elicitation_option(&name, &value, cx)
                            }
                        })),
                    );
                }
                column = column.child(row);
            }
            PendingElicitationControl::Unsupported { type_name } => {
                column = column.child(
                    div()
                        .text_xs()
                        .text_color(theme.muted_foreground)
                        .child(
                            t!("AgentUi.elicitation_unsupported_field", kind = type_name.clone())
                                .to_string(),
                        ),
                );
            }
        }
        column.into_any_element()
    }

    fn with_elicitation_field(
        &mut self,
        name: &str,
        edit: impl FnOnce(&mut PendingElicitationControl),
        cx: &mut Context<Self>,
    ) {
        let Some(form) = self
            .pending_elicitation
            .as_mut()
            .and_then(|pending| pending.form.as_mut())
        else {
            return;
        };
        let Some(field) = form.fields.iter_mut().find(|field| field.name == name) else {
            return;
        };
        edit(&mut field.control);
        if let Some(pending) = self.pending_elicitation.as_mut() {
            pending.error = None;
        }
        cx.notify();
    }

    fn toggle_elicitation_boolean(
        &mut self,
        name: &str,
        value: bool,
        cx: &mut Context<Self>,
    ) {
        self.with_elicitation_field(
            name,
            |control| {
                if let PendingElicitationControl::Boolean { value: current } = control {
                    *current = value;
                }
            },
            cx,
        );
    }

    fn select_elicitation_option(&mut self, name: &str, value: &str, cx: &mut Context<Self>) {
        let value = value.to_string();
        self.with_elicitation_field(
            name,
            move |control| {
                if let PendingElicitationControl::SingleSelect { selected, .. } = control {
                    // 再点一次已选项即取消选择（不必让用户去猜怎么撤销）。
                    if selected.as_deref() == Some(value.as_str()) {
                        *selected = None;
                    } else {
                        *selected = Some(value);
                    }
                }
            },
            cx,
        );
    }

    fn toggle_elicitation_option(&mut self, name: &str, value: &str, cx: &mut Context<Self>) {
        let value = value.to_string();
        self.with_elicitation_field(
            name,
            move |control| {
                if let PendingElicitationControl::MultiSelect { selected, .. } = control {
                    match selected.iter().position(|current| current == &value) {
                        Some(index) => {
                            selected.remove(index);
                        }
                        None => selected.push(value),
                    }
                }
            },
            cx,
        );
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
        // 模型下拉换了选项 / 选中项:同一套路,趁手里有窗口时灌进组件库的 state。
        if self.model_select_dirty {
            self.sync_model_select(_window, cx);
            self.model_select_dirty = false;
        }
        let context_bar = self.render_context_bar(cx);
        let attachments = self.render_attachments(cx);
        let editor_top_bar = self.render_editor_top_bar(cx);
        let queued_submissions = self.render_queued_submissions(cx);
        self.ensure_elicitation_inputs(_window, cx);
        let pending_elicitation = self.render_pending_elicitation(cx);
        let toolbar = self.render_toolbar(cx);
        let theme = self.local_theme(cx);
        let input_state = self.input_state.read(cx);
        let input_focused = input_state.focus_handle(cx).is_focused(_window);
        let editor_height = composer_editor_height(input_state);

        // 输入卡片：带边框的那一块（工作区 / 分支 / 模型 / Worktree / 权限
        // 与附件、发送按钮同处卡片底部的**同一行**）。
        let card = v_flex()
            .debug_selector(|| "agent-input-card".to_string())
            .w_full()
            .min_w_0()
            .bg(theme.background)
            .text_color(theme.foreground)
            .when(!self.edge_to_edge, |this| {
                this.rounded_lg()
                    .border_1()
                    .border_color(theme.border)
                    .shadow_sm()
            })
            .when(self.edge_to_edge, |this| this.min_h_0())
            // 顶部：计划 / Agent / 上下文入口
            .child(context_bar)
            // 附件预览（如果有）
            .children(attachments)
            .child(editor_top_bar)
            .when_some(queued_submissions, |this, queued| this.child(queued))
            // agent 提问：放在输入框正上方，答完才收起
            .when_some(pending_elicitation, |this, pending| this.child(pending))
            // 中部：多行输入框
            .child(
                div()
                    .debug_selector(|| "agent-input-editor".to_string())
                    .w_full()
                    .min_w_0()
                    .px_3()
                    .pt_1()
                    .max_h(px(220.0))
                    .child(
                        div()
                            .w_full()
                            .min_w_0()
                            .border_1()
                            .rounded(cx.theme().radius)
                            .border_color(if input_focused {
                                theme.accent
                            } else {
                                theme.border
                            })
                            .bg(theme.background)
                            .child(
                                Editor::new(&self.input_state)
                                    .w_full()
                                    .h(editor_height)
                                    .appearance(false)
                                    .bordered(false)
                                    .text_color(theme.foreground),
                            ),
                    ),
            )
            // 底部：附件 / 工作区 / 分支 / 模型 / Worktree / 权限 / 发送，一整行
            .child(toolbar);

        v_flex()
            .id("agent-input-root")
            .debug_selector(|| "agent-input-root".to_string())
            .track_focus(&self.focus_handle)
            .w_full()
            .min_w_0()
            .when(self.edge_to_edge, |this| this.min_h_0().flex_shrink_0())
            .when(!self.edge_to_edge, |this| this.flex_shrink_0())
            .bg(theme.background)
            .text_color(theme.foreground)
            .child(card)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::input::context::{ComposerModel, ComposerWorkspaceInfo, ComposerWorktreeState};
    use gpui::{Modifiers, Pixels, TestAppContext, VisualTestContext};

    /// 一行 7 项全部放下、不触发裁剪的最小宽度。
    ///
    /// 底部那一行固定是 `[附件][计数][工作区][分支][模型][Worktree][权限][发送]`：
    /// 附件组与发送按钮不参与收缩，中段 5 项各自还有可读性下限，加起来约 520px；
    /// 再窄就轮到中段的 `overflow_hidden` 裁尾巴了（发送按钮始终保留）。
    /// 布局类测试一律以它为「窄」的下限，低于它就不再声称「都看得到」。
    const NARROW_USABLE_WIDTH: f32 = 620.0;

    struct AgentInputLayoutRoot {
        input: Entity<AgentInput>,
        width: Pixels,
        height: Pixels,
    }

    impl AgentInputLayoutRoot {
        fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
            Self::with_width(px(NARROW_USABLE_WIDTH), window, cx)
        }

        fn wide(window: &mut Window, cx: &mut Context<Self>) -> Self {
            Self::with_width(px(900.0), window, cx)
        }

        fn with_resource_pool(window: &mut Window, cx: &mut Context<Self>) -> Self {
            let root = Self::with_width(px(420.0), window, cx);
            root.input.update(cx, |input, cx| {
                input.set_context(
                    AgentComposerContext {
                        resource_pool_items: vec![
                            ComposerResourcePoolItem::new(
                                "ssh-a",
                                "production-ssh-with-a-long-readable-name",
                                "SH",
                                "ssh",
                                "10.2.4.54",
                                "已加入",
                                Some("默认目标"),
                                3,
                                true,
                                true,
                            ),
                            ComposerResourcePoolItem::new(
                                "db-a",
                                "analytics-postgres-primary",
                                "DB",
                                "postgres",
                                "Database: ai_app",
                                "可添加",
                                None::<&str>,
                                4,
                                false,
                                false,
                            ),
                        ],
                        ..AgentComposerContext::default()
                    },
                    cx,
                );
            });
            root
        }

        fn running_with_queue(window: &mut Window, cx: &mut Context<Self>) -> Self {
            let mut root = Self::with_width(px(NARROW_USABLE_WIDTH), window, cx);
            root.height = px(320.0);
            root.input.update(cx, |input, cx| {
                input.set_running(true, cx);
                input.set_queued_submissions(
                    vec![
                        QueuedPromptPreview::new("检查数据库连接状态", 0),
                        QueuedPromptPreview::new("", 2),
                    ],
                    cx,
                );
            });
            root
        }

        fn blocked_with_queue(window: &mut Window, cx: &mut Context<Self>) -> Self {
            let mut root = Self::with_width(px(NARROW_USABLE_WIDTH), window, cx);
            root.height = px(320.0);
            root.input.update(cx, |input, cx| {
                input.set_pending_queue_blocked(true, cx);
                input.set_queued_submissions(
                    vec![QueuedPromptPreview::new("等待 ACP 恢复后执行", 0)],
                    cx,
                );
            });
            root
        }

        /// 注入工作区 / 分支 / Worktree 数据，用于底栏渲染与截断测试。
        fn with_composer_context(
            width: Pixels,
            window: &mut Window,
            cx: &mut Context<Self>,
        ) -> Self {
            const LONG_WORKSPACE: &str = "an-extremely-long-workspace-directory-name";
            let root = Self::with_width(width, window, cx);
            root.input.update(cx, |input, cx| {
                input.set_context(
                    AgentComposerContext {
                        model: Some(ComposerModel::new(
                            "Very Long Provider Name",
                            "extremely-long-model-name-with-large-context",
                        )),
                        execution_mode_label: SharedString::from("允许完全访问"),
                        workspace: ComposerWorkspaceInfo::new(
                            LONG_WORKSPACE,
                            Some("/Users/someone/projects/an-extremely-long-workspace-directory-name"),
                        )
                        .with_git_repo(true),
                        workspace_options: vec![
                            ComposerWorkspaceOption::new(
                                LONG_WORKSPACE,
                                "/Users/someone/projects/an-extremely-long-workspace-directory-name",
                                true,
                            ),
                            ComposerWorkspaceOption::new("navop", "/Users/someone/navop", false),
                        ],
                        branch_options: vec![
                            ComposerBranchOption::new(
                                "feature/composer-footer-relayout",
                                true,
                                false,
                            ),
                            ComposerBranchOption::new("main", false, false),
                            ComposerBranchOption::new("origin/main", false, true),
                        ],
                        worktree: ComposerWorktreeState::new(true, Some("navop-1a2b3c4d")),
                        ..AgentComposerContext::default()
                    },
                    cx,
                );
            });
            root
        }

        fn with_width(width: Pixels, window: &mut Window, cx: &mut Context<Self>) -> Self {            let input = cx.new(|cx| {
                AgentInput::with_mentions(Vec::new(), "描述目标，输入 @ 引用资源…", window, cx)
            });
            input.update(cx, |input, cx| {
                input.set_context(
                    AgentComposerContext {
                        model: Some(ComposerModel::new(
                            "Very Long Provider Name",
                            "extremely-long-model-name-with-large-context",
                        )),
                        execution_mode_label: SharedString::from("自动"),
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
                    cx,
                );
            });
            Self {
                input,
                width,
                height: px(220.0),
            }
        }
    }

    impl Render for AgentInputLayoutRoot {
        fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
            div().w(self.width).h(self.height).child(self.input.clone())
        }
    }

    /// 七项都在输入卡片底部的**同一行**，并按 工作区→分支→模型→Worktree→权限→发送 排布。
    #[gpui::test]
    fn composer_row_keeps_context_chips_on_one_line(cx: &mut TestAppContext) {
        cx.update(|cx| {
            gpui_component::init(cx);
            crate::init(cx);
        });
        let (_, cx) = cx.add_window_view(|window, cx| {
            AgentInputLayoutRoot::with_composer_context(px(1100.0), window, cx)
        });
        let cx: &mut VisualTestContext = cx;

        let card = cx
            .debug_bounds("agent-input-card")
            .expect("input card should render");
        let group = cx
            .debug_bounds("agent-input-context-group")
            .expect("context group should render");
        let workspace = cx
            .debug_bounds("agent-input-workspace")
            .expect("workspace chip should render");
        let branch = cx
            .debug_bounds("agent-input-branch")
            .expect("branch chip should render");
        let model = cx
            .debug_bounds("agent-input-model-control")
            .expect("model control should render");
        let worktree = cx
            .debug_bounds("agent-input-worktree")
            .expect("worktree toggle should render");
        let permission = cx
            .debug_bounds("agent-input-permission")
            .expect("permission chip should render");
        let send = cx
            .debug_bounds("agent-input-send-control")
            .expect("send control should render");

        assert!(
            workspace.origin.x < branch.origin.x
                && branch.origin.x < model.origin.x
                && model.origin.x < worktree.origin.x
                && worktree.origin.x < permission.origin.x
                && permission.origin.x < send.origin.x,
            "chips must be ordered workspace → branch → model → worktree → permission → send: \
             workspace={workspace:?}, branch={branch:?}, model={model:?}, \
             worktree={worktree:?}, permission={permission:?}, send={send:?}"
        );
        // 七项共处一行：纵向偏差只可能来自边框/内边距的取整。
        let row_y = workspace.origin.y;
        for bounds in [branch, model, worktree, permission, send] {
            assert!(
                (bounds.origin.y - row_y).abs() <= px(2.0),
                "all controls must share one row: rows differ at {bounds:?} (row_y={row_y:?})"
            );
        }
        // 而且这一行在输入卡片内部，不再有卡片外的第二行。
        for bounds in [workspace, branch, model, worktree, permission, send] {
            assert!(
                bounds.origin.y >= card.origin.y
                    && bounds.origin.y + bounds.size.height
                        <= card.origin.y + card.size.height,
                "every control must stay inside the input card: {bounds:?}, card={card:?}"
            );
        }
        assert!(
            group.origin.x + group.size.width <= send.origin.x,
            "the context group must not overlap the send button: group={group:?}, send={send:?}"
        );
    }

    /// 侧栏被拖窄时七项仍留在同一行：靠收缩 + 截断消化，绝不把发送按钮挤出卡片。
    #[gpui::test]
    fn composer_row_truncates_instead_of_pushing_the_send_button_out(cx: &mut TestAppContext) {
        cx.update(|cx| {
            gpui_component::init(cx);
            crate::init(cx);
        });
        const NARROW: f32 = 620.0;
        let (_, cx) = cx.add_window_view(|window, cx| {
            AgentInputLayoutRoot::with_composer_context(px(NARROW), window, cx)
        });
        let cx: &mut VisualTestContext = cx;

        let card = cx
            .debug_bounds("agent-input-card")
            .expect("input card should render");
        let workspace = cx
            .debug_bounds("agent-input-workspace")
            .expect("workspace chip should render");
        let branch = cx
            .debug_bounds("agent-input-branch")
            .expect("branch chip should render");
        let worktree = cx
            .debug_bounds("agent-input-worktree")
            .expect("worktree toggle should render");
        let permission = cx
            .debug_bounds("agent-input-permission")
            .expect("permission chip should render");
        let send = cx
            .debug_bounds("agent-input-send-control")
            .expect("send control should render");

        // 发送按钮是这一行的锚点：任何收缩都不能把它推出卡片。
        assert!(
            send.origin.x + send.size.width <= card.origin.x + card.size.width,
            "send control must stay inside the card: send={send:?}, card={card:?}"
        );
        assert!(
            workspace.size.width < px(COMPOSER_CHIP_MAX_WIDTH),
            "a long workspace name must shrink instead of hogging the row: workspace={workspace:?}"
        );
        for bounds in [workspace, branch, worktree, permission] {
            assert!(
                bounds.size.width >= px(COMPOSER_CHIP_MIN_WIDTH),
                "chips must not shrink below the readable floor: {bounds:?}"
            );
        }
        // 收缩之后仍然是同一行。
        let row_y = workspace.origin.y;
        for bounds in [branch, worktree, permission, send] {
            assert!(
                (bounds.origin.y - row_y).abs() <= px(2.0),
                "chips must stay on one row at narrow width: {bounds:?} (row_y={row_y:?})"
            );
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
    fn running_toolbar_keeps_queue_stop_and_model_controls_visible(cx: &mut TestAppContext) {
        cx.update(|cx| {
            gpui_component::init(cx);
            crate::init(cx);
        });
        let (_, cx) = cx.add_window_view(AgentInputLayoutRoot::running_with_queue);
        let cx: &mut VisualTestContext = cx;

        let model = cx
            .debug_bounds("agent-input-model-control")
            .expect("model control should be rendered");
        let queue = cx
            .debug_bounds("agent-input-queue-send")
            .expect("queue send control should be rendered");
        let stop = cx
            .debug_bounds("agent-input-stop")
            .expect("stop control should be rendered");

        assert!(model.size.width >= px(96.0));
        assert!(
            model.origin.x + model.size.width <= queue.origin.x,
            "model and queue controls must not overlap: model={model:?}, queue={queue:?}"
        );
        assert!(
            queue.origin.x + queue.size.width <= stop.origin.x,
            "queue and stop controls must not overlap: queue={queue:?}, stop={stop:?}"
        );
    }

    #[test]
    fn execution_trigger_width_tracks_label_length() {
        // 中文短标签收窄到下限,给模型选择与操作按钮让出空间。
        assert_eq!(execution_trigger_width("自动", false), px(80.0));
        assert_eq!(execution_trigger_width("只读", false), px(80.0));
        // 四字标签按内容展开,仍明显窄于旧的固定 124px。
        assert_eq!(execution_trigger_width("手动确认", false), px(100.0));
        // 英文长标签不突破上限,保留原有的可用宽度。
        assert_eq!(
            execution_trigger_width("Manual Confirmation", false),
            px(124.0)
        );
        // 运行中还要容纳排队与停止两个按钮,上限更低。
        let running = execution_trigger_width("自动", true);
        assert!(
            running < px(80.0),
            "running trigger should stay narrower: {running:?}"
        );
    }

    /// 工具栏一行内同时有「附件 + 上下文档位 + 发送」；发送按钮仍是 32x32。
    #[gpui::test]
    fn toolbar_keeps_action_button_square_and_permission_in_the_same_row(
        cx: &mut TestAppContext,
    ) {
        cx.update(|cx| {
            gpui_component::init(cx);
            crate::init(cx);
        });
        let (_, cx) = cx.add_window_view(AgentInputLayoutRoot::new);
        let cx: &mut VisualTestContext = cx;

        let send = cx
            .debug_bounds("agent-send-button")
            .expect("send button should be rendered");
        assert_eq!(
            (send.size.width, send.size.height),
            (
                px(TOOLBAR_ACTION_BUTTON_SIZE),
                px(TOOLBAR_ACTION_BUTTON_SIZE)
            ),
            "send button must stay a 32x32 tap target: send={send:?}"
        );

        let toolbar = cx
            .debug_bounds("agent-input-toolbar")
            .expect("toolbar should render");
        let permission = cx
            .debug_bounds("agent-input-permission")
            .expect("permission chip should render in the toolbar");
        assert!(
            permission.origin.y >= toolbar.origin.y
                && permission.origin.y + permission.size.height
                    <= toolbar.origin.y + toolbar.size.height,
            "permission chip must share the toolbar row, not sit in a second row: \
             permission={permission:?}, toolbar={toolbar:?}"
        );
        assert!(
            permission.size.width <= px(124.0),
            "permission chip should hug its short label: permission={permission:?}"
        );
        assert!(
            permission.size.height <= px(TOOLBAR_ACTION_BUTTON_SIZE),
            "permission chip must not outgrow the toolbar buttons: permission={permission:?}"
        );
    }

    #[gpui::test]
    fn running_toolbar_keeps_action_buttons_square(cx: &mut TestAppContext) {
        cx.update(|cx| {
            gpui_component::init(cx);
            crate::init(cx);
        });
        let (_, cx) = cx.add_window_view(AgentInputLayoutRoot::running_with_queue);
        let cx: &mut VisualTestContext = cx;

        for id in ["agent-queue-send-button", "agent-stop-button"] {
            let bounds = cx
                .debug_bounds(id)
                .unwrap_or_else(|| panic!("{id} should be rendered"));
            assert_eq!(
                (bounds.size.width, bounds.size.height),
                (
                    px(TOOLBAR_ACTION_BUTTON_SIZE),
                    px(TOOLBAR_ACTION_BUTTON_SIZE)
                ),
                "{id} must stay a 32x32 tap target: {bounds:?}"
            );
        }
    }

    #[gpui::test]
    fn blocked_queue_keeps_queue_and_stop_controls_without_marking_turn_running(
        cx: &mut TestAppContext,
    ) {
        cx.update(|cx| {
            gpui_component::init(cx);
            crate::init(cx);
        });
        let (root, cx) = cx.add_window_view(AgentInputLayoutRoot::blocked_with_queue);
        let cx: &mut VisualTestContext = cx;

        assert!(cx.debug_bounds("agent-input-queue-send").is_some());
        assert!(cx.debug_bounds("agent-input-stop").is_some());
        assert!(cx.debug_bounds("agent-input-send-control").is_none());
        root.read_with(cx, |root, cx| {
            assert!(!root.input.read(cx).is_running());
        });
    }

    #[gpui::test]
    fn queued_submission_preview_renders_above_editor(cx: &mut TestAppContext) {
        cx.update(|cx| {
            gpui_component::init(cx);
            crate::init(cx);
        });
        let (_, cx) = cx.add_window_view(AgentInputLayoutRoot::running_with_queue);
        let cx: &mut VisualTestContext = cx;

        let queue = cx
            .debug_bounds("agent-input-queued")
            .expect("queued preview should be rendered");
        let item = cx
            .debug_bounds("agent-input-queued-item")
            .expect("queued preview item should be rendered");
        let editor = cx
            .debug_bounds("agent-input-editor")
            .expect("editor should be rendered");

        assert!(item.size.height > px(0.0));
        assert!(
            queue.origin.y + queue.size.height <= editor.origin.y,
            "queued preview must stay above the editor: queue={queue:?}, editor={editor:?}"
        );
    }

    #[gpui::test]
    fn plan_and_subagent_triggers_stay_and_permission_is_a_single_footer_control(
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
            cx.debug_bounds("agent-input-permission").is_some(),
            "permission level should render as the single control in the composer footer"
        );
        assert!(
            cx.debug_bounds("agent-task-mode").is_none(),
            "task mode should no longer render"
        );
        assert!(
            cx.debug_bounds("agent-tool-mode").is_none(),
            "tool mode should not render as a separate legacy toolbar control"
        );
    }

    #[gpui::test]
    fn resource_pool_rows_keep_stable_compact_height(cx: &mut TestAppContext) {
        cx.update(|cx| {
            gpui_component::init(cx);
            crate::init(cx);
        });
        let (_, cx) = cx.add_window_view(AgentInputLayoutRoot::with_resource_pool);
        let cx: &mut VisualTestContext = cx;

        let trigger = cx
            .debug_bounds("agent-context-mode")
            .expect("context trigger should render");
        cx.simulate_click(trigger.center(), Modifiers::default());

        let root = cx
            .debug_bounds("agent-input-root")
            .expect("input root should render");
        let row = cx
            .debug_bounds("agent-resource-pool-row")
            .expect("resource row should render");

        assert!(
            row.size.width <= root.size.width,
            "resource row should stay within input width: row={row:?}, root={root:?}"
        );
    }

    #[gpui::test]
    fn mention_completion_popup_stays_above_bottom_toolbar(cx: &mut TestAppContext) {
        cx.update(|cx| {
            gpui_component::init(cx);
            crate::init(cx);
        });
        let (_, cx) = cx.add_window_view(AgentInputLayoutRoot::new);
        let cx: &mut VisualTestContext = cx;

        let input = cx
            .debug_bounds("agent-input-root")
            .expect("input root should render");
        let toolbar = cx
            .debug_bounds("agent-input-toolbar")
            .expect("toolbar should render");

        assert!(
            input.origin.y < toolbar.origin.y,
            "input editor should have vertical room above toolbar for completion popup"
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
    fn empty_execution_mode_label_defaults_to_auto() {
        assert_eq!(
            t!("AgentUi.auto").as_ref(),
            current_execution_mode_label(&SharedString::from("")).as_ref()
        );
    }

    #[test]
    fn execution_mode_trigger_uses_single_label() {
        assert_eq!(
            "只读",
            current_execution_mode_label(&SharedString::from("只读")).as_ref()
        );
    }

    #[test]
    fn plan_trigger_label_is_default_when_empty() {
        assert_eq!(
            plan_trigger_label(&[]).as_ref(),
            t!("AgentUi.plan").as_ref()
        );
    }

    #[test]
    fn plan_trigger_label_shows_running_progress() {
        let items = vec![
            ComposerPlanItem::new("完成项", "completed"),
            ComposerPlanItem::new("执行项", "running"),
            ComposerPlanItem::new("待执行项", "pending"),
        ];

        assert_eq!(
            plan_trigger_label(&items).as_ref(),
            format!("1/3 {}", t!("AgentUi.in_progress"))
        );
    }

    #[test]
    fn plan_trigger_label_shows_completed_progress() {
        let items = vec![
            ComposerPlanItem::new("一", "completed"),
            ComposerPlanItem::new("二", "completed"),
        ];

        assert_eq!(
            plan_trigger_label(&items).as_ref(),
            format!("2/2 {}", t!("AgentUi.completed"))
        );
    }

    #[test]
    fn plan_trigger_label_shows_pending_progress() {
        let items = vec![
            ComposerPlanItem::new("一", "pending"),
            ComposerPlanItem::new("二", "failed"),
        ];

        assert_eq!(
            plan_trigger_label(&items).as_ref(),
            format!("0/2 {}", t!("AgentUi.pending"))
        );
    }

    #[test]
    fn subagent_trigger_label_shows_running_subagent_count() {
        assert_eq!(
            subagent_trigger_label(&[]).as_ref(),
            t!("AgentUi.subagents").as_ref()
        );

        let items = vec![ComposerSubAgentItem::new(
            "sub_1",
            "reviewer",
            "检查事件流",
            "running",
        )];

        assert_eq!(
            subagent_trigger_label(&items).as_ref(),
            t!("AgentUi.subagent_count", count = 1).as_ref()
        );
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

        assert_eq!(
            resource_pool_trigger_label(&context).as_ref(),
            t!("AgentUi.resource_pool_count", count = 3).as_ref()
        );
    }

    #[test]
    fn resource_pool_trigger_label_handles_empty_pool() {
        assert_eq!(
            resource_pool_trigger_label(&AgentComposerContext::default()).as_ref(),
            t!("AgentUi.resource_pool").as_ref()
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
    fn history_navigation_only_uses_unmodified_vertical_keys() {
        assert_eq!(
            Some(HistoryDirection::Previous),
            history_direction("up", &Modifiers::default())
        );
        assert_eq!(
            Some(HistoryDirection::Next),
            history_direction("down", &Modifiers::default())
        );
        assert_eq!(None, history_direction("left", &Modifiers::default()));
        assert_eq!(
            None,
            history_direction(
                "up",
                &Modifiers {
                    shift: true,
                    ..Modifiers::default()
                }
            )
        );
    }

    #[test]
    fn history_navigation_only_uses_document_boundaries() {
        let text = "first\nsecond\nthird";

        assert!(cursor_is_at_history_boundary(
            HistoryDirection::Previous,
            text,
            0
        ));
        assert!(!cursor_is_at_history_boundary(
            HistoryDirection::Previous,
            text,
            "fir".len()
        ));
        assert!(!cursor_is_at_history_boundary(
            HistoryDirection::Previous,
            text,
            "first\nsec".len()
        ));
        assert!(cursor_is_at_history_boundary(
            HistoryDirection::Next,
            text,
            text.len()
        ));
        assert!(!cursor_is_at_history_boundary(
            HistoryDirection::Next,
            text,
            "first\nsecond\nth".len()
        ));
        assert!(!cursor_is_at_history_boundary(
            HistoryDirection::Next,
            text,
            "first\nsec".len()
        ));
    }

    #[test]
    fn history_navigation_uses_utf8_byte_offsets_at_document_boundaries() {
        let text = "第一行\nsecond\n第三行";

        assert!(cursor_is_at_history_boundary(
            HistoryDirection::Previous,
            text,
            0
        ));
        assert!(!cursor_is_at_history_boundary(
            HistoryDirection::Previous,
            text,
            "第".len()
        ));
        assert!(cursor_is_at_history_boundary(
            HistoryDirection::Next,
            text,
            text.len()
        ));
        assert!(!cursor_is_at_history_boundary(
            HistoryDirection::Next,
            text,
            text.len() - "行".len()
        ));
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
        let add = ComposerResourcePoolItem::new(
            "ssh-b",
            "prod-b",
            "SH",
            "ssh",
            "ssh-b",
            "可添加",
            None::<&str>,
            0,
            false,
            false,
        );
        let remove = ComposerResourcePoolItem::new(
            "ssh-a",
            "prod-a",
            "SH",
            "ssh",
            "ssh-a",
            "已加入",
            None::<&str>,
            0,
            true,
            false,
        );
        let default = ComposerResourcePoolItem::new(
            "ssh-a",
            "prod-a",
            "SH",
            "ssh",
            "ssh-a",
            "已加入",
            Some("默认目标"),
            0,
            true,
            true,
        );

        assert_eq!(resource_pool_action_label(&add), "+");
        assert_eq!(resource_pool_action_label(&remove), "-");
        assert_eq!(resource_pool_action_label(&default), "-");
        assert!(!resource_pool_action_disabled(&default));
    }

    #[test]
    fn resource_source_option_label_includes_count_or_disabled_hint() {
        let enabled = ComposerResourceSourceOption::new("all", "全部", 3, true);
        let disabled = ComposerResourceSourceOption::new("workspace", "工作区", 0, false)
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
