//! ACP 输入框：外部 agent 会话专用的那一份输入组件。
//!
//! 与 [`AgentInput`](super::AgentInput) 是**两份独立的组件**，不是同一个组件换皮：
//! - One agent（本地运行时）那份是"驾驶舱"：计划 / 子代理 / 上下文资源 / Skill
//!   四张能力面板、资源池、目标与权限档位，全都围绕本地运行时自己的概念；
//! - ACP 那份是"对讲机"：会话模式、模型与 `/` 命令都是 agent 自己声明的，能力栏
//!   只留计划与子代理两栏 —— ACP 的 `Plan` 更新同样落进计划栏，那是 agent 自己的
//!   待办，不是 navop 替它排的。上下文资源池与 Skill 才是本地专属：前者来自
//!   `AgentConfig.available_resources`、后者来自工作区扫描，ACP 侧没有对应输入源。
//!
//! 位置的取舍同理由"谁声明"决定：模式与模型是"这次怎么跑"，跟底栏走；身份与能力栏
//! 是"这是谁、在干什么"，留在顶部。工作区 / 分支 / Worktree 是本地下拉里长大的那套
//! 东西，ACP 侧照旧留在底栏；但分支名又长又杂，与模型一样改用组件库 `Select`，
//! 可搜可滚，不再是只能肉眼翻的自绘列表。
//!
//! 两份共用的是**数据类型**（[`AgentComposerContext`] 视图模型、[`AgentInputEvent`]
//! 事件、[`ImageAttachment`] 附件、[`PromptHistory`] 历史）—— 事件枚举共用是有意的：
//! 上层 [`AgentChatView`](crate::agent_view::AgentChatView) 对两条输入通路只写一套
//! 事件处理逻辑，组件本身各自演化。

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::rc::Rc;

use gpui::{
    App, AppContext, Context, Entity, EntityInputHandler, EventEmitter, ExternalPaths, FocusHandle,
    Focusable, InteractiveElement, IntoElement, Modifiers, ParentElement, PathPromptOptions,
    Pixels, Render, SharedString, StatefulInteractiveElement, StyleRefinement, Styled,
    Subscription, Window, div, img, prelude::FluentBuilder, px, relative,
};
use gpui_component::button::{Button, ButtonVariants};
use gpui_component::input::{Editor, EditorState, Input, InputEvent, InputState};
use gpui_component::popover::Popover;
use gpui_component::progress::ProgressCircle;
use gpui_component::searchable_list::SearchableVec;
use gpui_component::select::{Select, SelectEvent, SelectGroup, SelectItem, SelectState};
use gpui_component::switch::Switch;
use gpui_component::tooltip::Tooltip;
use gpui_component::{ActiveTheme, Disableable, Icon, IndexPath, Sizable, h_flex, v_flex};
use one_assets::IconName;
use rust_i18n::t;
use serde_json::Value;

use super::agent_input::{AgentInputEvent, QueuedPromptPreview};
use crate::acp::{
    AcpElicitationField, AcpElicitationFieldKind, AcpElicitationMode, AcpElicitationRequest,
};
use crate::input::PromptHistory;
use crate::input::attachment::ImageAttachment;
use crate::input::completion::ComposerCompletionProvider;
use crate::input::composer_chrome::{
    CHIP_HEIGHT, CHIP_MAX_WIDTH, CHIP_MIN_WIDTH, CHIP_RADIUS, action_button_radius,
    action_button_size, capability_chip, composer_capability_row, composer_capability_slot,
    composer_card, composer_chip_label, composer_context_row, composer_editor_frame,
    composer_model_chip, composer_popover_surface, composer_root, composer_toolbar,
    estimated_label_width, ghost_chip, ghost_chip_muted, model_menu_width,
};
use crate::input::context::{
    AgentComposerContext, ComposerBranchOption, ComposerMenuOption, ComposerModelOption,
    ComposerPlanItem, ComposerSubAgentItem, ComposerWorkspaceOption,
};
use crate::input::model_picker::{ModelChoice, model_groups, selected_model_index};
use crate::input::slash::{SlashCommandItem, SlashCompletionProvider};
use crate::theme::{AgentChatTheme, active_agent_chat_theme, sp};
use crate::usage::{ContextUsage, UsagePressure, format_token_count};

const COMPOSER_EDITOR_MIN_ROWS: usize = 3;
const COMPOSER_EDITOR_MAX_ROWS: usize = 10;
const COMPOSER_EDITOR_FALLBACK_LINE_HEIGHT: f32 = 20.0;
const COMPOSER_EDITOR_VERTICAL_PADDING: f32 = 12.0;
/// 用量圆环的直径；比 chip 小一号，收在上下文行尾端。
const USAGE_GAUGE_DIAMETER: f32 = 16.0;
/// 分支下拉弹层的宽度区间：分支名长短差别很大（`main` 到 `feature/xxx-yyy`），
/// 按最长的那个估，保证点开能读到完整的名字。
const BRANCH_MENU_MIN_WIDTH: f32 = 260.0;
const BRANCH_MENU_MAX_WIDTH: f32 = 420.0;
const BRANCH_MENU_CHROME_WIDTH: f32 = 56.0;

/// ACP 输入框里受控开合的面板种类。
///
/// 模型与分支不在这里：两者都交给组件库的 `Select`，开合与搜索由组件自己管。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum AcpComposerMenu {
    Plan,
    SubAgent,
    Mode,
    Workspace,
}

/// 编辑器高度：按行数在 3~10 行之间自适应。
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

/// 上下键历史浏览的方向。
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

/// 光标是否处在「浏览器里上下键应该翻历史」的边界上。
fn cursor_is_at_history_boundary(direction: HistoryDirection, text: &str, cursor: usize) -> bool {
    match direction {
        HistoryDirection::Previous => cursor == 0,
        HistoryDirection::Next => cursor == text.len(),
    }
}

/// 模式 chip 的当前文案：agent 没报模式时退回「模式」。
fn current_mode_label(label: &SharedString) -> SharedString {
    if label.is_empty() {
        SharedString::from(t!("AgentUi.mode").to_string())
    } else {
        label.clone()
    }
}

/// ACP 会话的输入框。
///
/// 与 One agent 的 [`AgentInput`](super::AgentInput) 一一对应的能力（编辑器、附件、
/// 历史、草稿、队列、agent 提问）在这里各有一份实现：两条通路互不影响，
/// ACP 侧的改动不会动到 One agent 的观感。
pub struct AcpAgentInput {
    focus_handle: FocusHandle,
    input_state: Entity<EditorState>,
    /// 可被 `/` 补全的命令，来源是 ACP agent 推送的 `available_commands`。
    slash_commands: Vec<SlashCommandItem>,
    /// 当前图片附件。
    attachments: Vec<ImageAttachment>,
    /// 正在放大预览的附件 id（覆盖层由面板根部绘制，见 `agent_view`）。
    attachment_preview: Option<String>,
    /// 已提交过的文本历史，上下键浏览。
    history: PromptHistory,
    /// 当前会话等待下一轮执行的提交摘要。
    queued_submissions: Vec<QueuedPromptPreview>,
    /// 队列因连接或会话过渡失败而暂停，可由用户显式停止并清空。
    pending_queue_blocked: bool,
    /// 当前挂着的「agent 提问」，答完清空。
    pending_elicitation: Option<AcpPendingElicitation>,
    /// 是否正在运行（运行中显示「停止」）。
    is_running: bool,
    /// 上层注入的展示上下文：会话标题、模式文案、用量读数、工作区与分支。
    context: AgentComposerContext,
    /// 模型候选（agent 声明的 configOptions，由上层注入）。
    model_options: Vec<ComposerModelOption>,
    /// 模型下拉的组件库状态；注入点时拿不到窗口，置脏后在 `render` 里灌。
    model_select: Entity<SelectState<SearchableVec<SelectGroup<ModelChoice>>>>,
    model_select_dirty: bool,
    branch_select: Entity<SelectState<SearchableVec<BranchChoice>>>,
    branch_select_dirty: bool,
    /// 会话模式候选（agent 声明的 modes，id 已带 `acp-mode:` 前缀）。
    mode_options: Vec<ComposerMenuOption>,
    /// 当前展开的下拉（受控开合）。
    open_menu: Option<AcpComposerMenu>,
    /// 用量详情 popover 的开合（受控：「查看历史」要能把它关上）。
    usage_detail_open: bool,
    /// 可选的局部主题（终端侧边栏会注入终端主题）。
    theme: Option<AgentChatTheme>,
    edge_to_edge: bool,
    _subscriptions: Vec<Subscription>,
}

impl AcpAgentInput {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let input_state = cx.new(|cx| {
            let mut state = EditorState::new(window, cx)
                .line_number(false)
                .soft_wrap(true)
                .submit_on_enter(true)
                .placeholder(t!("AgentUi.acp_input_placeholder").to_string());
            // ACP 侧没有 `@` 引用（引用语法是本地运行时的概念），只留 `/` 命令。
            state.lsp_mut().completion_provider =
                Some(Rc::new(ComposerCompletionProvider::slash(Vec::new())));
            state
        });

        let enter_sub = cx.subscribe_in(&input_state, window, |this, _state, event, window, cx| {
            match event {
                InputEvent::PressEnter {
                    secondary, shift, ..
                } if !secondary && !shift => {
                    this.submit(window, cx);
                }
                // 文字变了就转发一条：草稿要保存（防抖在上层做）。
                InputEvent::Change => {
                    cx.emit(AgentInputEvent::DraftChanged);
                }
                _ => {}
            }
        });

        // 聚焦时 cmd/ctrl-v 把剪贴板图片补进附件（文本粘贴仍交给 InputState）。
        let weak = cx.entity().downgrade();
        let paste_sub = cx.intercept_keystrokes(move |ev, window, cx| {
            if ev.keystroke.key != "v" || !ev.keystroke.modifiers.secondary(cx) {
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

        let branch_select: Entity<SelectState<SearchableVec<BranchChoice>>> = cx.new(|cx| {
            SelectState::<SearchableVec<BranchChoice>>::new(
                SearchableVec::new(Vec::new()),
                None,
                window,
                cx,
            )
            .searchable(true)
        });
        let branch_select_sub = cx.subscribe(
            &branch_select,
            |_this: &mut Self,
             _state: Entity<SelectState<SearchableVec<BranchChoice>>>,
             event: &SelectEvent<SearchableVec<BranchChoice>>,
             cx: &mut Context<Self>| {
                let SelectEvent::Confirm(Some(option)) = event else {
                    return;
                };
                cx.emit(AgentInputEvent::SelectBranch {
                    name: option.name.clone(),
                });
            },
        );

        Self {
            focus_handle: cx.focus_handle(),
            input_state,
            slash_commands: Vec::new(),
            attachments: Vec::new(),
            attachment_preview: None,
            history: PromptHistory::default(),
            queued_submissions: Vec::new(),
            pending_queue_blocked: false,
            pending_elicitation: None,
            is_running: false,
            context: AgentComposerContext::default(),
            model_options: Vec::new(),
            model_select,
            model_select_dirty: false,
            branch_select,
            branch_select_dirty: false,
            mode_options: Vec::new(),
            open_menu: None,
            usage_detail_open: false,
            theme: None,
            edge_to_edge: false,
            _subscriptions: vec![
                enter_sub,
                paste_sub,
                history_sub,
                model_select_sub,
                branch_select_sub,
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

    /// 更新 `/` 命令补全来源（ACP agent 推送的 `available_commands`）。
    pub fn set_slash_commands(&mut self, commands: Vec<SlashCommandItem>, cx: &mut Context<Self>) {
        // 推送可能很频繁（每次 `session/update` 都可能带），内容没变就不重建 provider。
        if self.slash_commands == commands {
            return;
        }
        self.slash_commands = commands;
        let commands = self.slash_commands.clone();
        self.input_state.update(cx, |state, _| {
            state.lsp_mut().completion_provider =
                Some(Rc::new(ComposerCompletionProvider::slash(commands)));
        });
    }

    /// 注入展示上下文（会话标题 / 模式文案 / 用量 / 工作区 / 分支）。
    pub fn set_context(&mut self, context: AgentComposerContext, cx: &mut Context<Self>) {
        if self.context.branch_options != context.branch_options {
            // 同模型那份：`SelectState` 要窗口才能更新，这里只有 `Context`。
            self.branch_select_dirty = true;
        }
        self.context = context;
        cx.notify();
    }

    /// 注入模式与模型候选（都由 agent 自己声明）。
    pub fn set_menu_options(
        &mut self,
        model_options: Vec<ComposerModelOption>,
        mode_options: Vec<ComposerMenuOption>,
        cx: &mut Context<Self>,
    ) {
        if self.model_options != model_options {
            self.model_options = model_options;
            // `SelectState` 的更新要窗口，而这里只有 `Context`；置脏，`render` 时同步。
            self.model_select_dirty = true;
        }
        self.mode_options = mode_options;
        cx.notify();
    }

    /// 设置运行状态（决定显示「发送」还是「停止」）。
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

    /// 设置待执行队列是否处于阻塞状态。
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

    /// 当前输入框挂着的图片附件（会话草稿捕获用，只读）。
    pub fn composer_attachments(&self) -> &[ImageAttachment] {
        &self.attachments
    }

    /// 正在放大预览的附件（没有则 `None`）。
    pub fn previewed_attachment(&self) -> Option<&ImageAttachment> {
        let id = self.attachment_preview.as_deref()?;
        self.attachments.iter().find(|att| att.id == id)
    }

    /// 打开/关闭某张附件的预览（再点一下同一张就是关）。
    pub fn toggle_attachment_preview(&mut self, id: &str, cx: &mut Context<Self>) {
        if !self.attachments.iter().any(|att| att.id == id) {
            return;
        }
        self.attachment_preview = if self.attachment_preview.as_deref() == Some(id) {
            None
        } else {
            Some(id.to_string())
        };
        cx.notify();
    }

    /// 关掉附件预览（点背板 / 点关闭按钮）。
    pub fn close_attachment_preview(&mut self, cx: &mut Context<Self>) {
        if self.attachment_preview.take().is_some() {
            cx.notify();
        }
    }

    /// 把一段排队提交放回输入框并聚焦（供「编辑队列项」使用）。
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

    /// 设置输入框文本，不动附件、不抢焦点（会话草稿恢复 / 起手话填词专用）。
    pub fn set_composer_text(&mut self, text: &str, window: &mut Window, cx: &mut Context<Self>) {
        self.input_state.update(cx, |state, cx| {
            state.set_value(text, window, cx);
        });
        // `set_value` 是程序化写入，不产生 `InputEvent::Change`，但内容确实变了。
        cx.emit(AgentInputEvent::DraftChanged);
        cx.notify();
    }

    /// 整体替换输入框的图片附件（会话草稿恢复专用）。
    pub fn set_composer_attachments(
        &mut self,
        images: Vec<ImageAttachment>,
        cx: &mut Context<Self>,
    ) {
        self.attachments = images;
        cx.emit(AgentInputEvent::DraftChanged);
        cx.notify();
    }

    fn submit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let text = self.input_state.read(cx).value().to_string();
        if text.trim().is_empty() && self.attachments.is_empty() {
            return;
        }

        self.history.record(&text);
        let images = std::mem::take(&mut self.attachments);

        // ACP 侧没有 `@` 引用：mentions 恒为空，文本原样交给 agent。
        cx.emit(AgentInputEvent::Submit {
            text,
            mentions: Vec::new(),
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

    /// 从 Finder 拖进来的文件：图片落成附件，其余把路径插到光标处。
    fn add_dropped_paths(
        &mut self,
        paths: &[PathBuf],
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if paths.is_empty() {
            return;
        }
        let mut attachments = Vec::new();
        let mut texts = Vec::new();
        for path in paths {
            match ImageAttachment::from_path(path) {
                Some(att) => attachments.push(att),
                None => texts.push(format!("{} ", path.display())),
            }
        }
        let staged = !attachments.is_empty() || !texts.is_empty();
        self.add_attachments(attachments, cx);
        for text in texts {
            self.input_state
                .update(cx, |state, cx| state.insert(text, window, cx));
        }
        if staged {
            self.focus_input(window, cx);
        }
    }

    fn add_attachments(&mut self, mut atts: Vec<ImageAttachment>, cx: &mut Context<Self>) {
        if atts.is_empty() {
            return;
        }
        self.attachments.append(&mut atts);
        cx.emit(AgentInputEvent::DraftChanged);
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

    fn remove_attachment(&mut self, id: &str, cx: &mut Context<Self>) {
        self.attachments.retain(|a| a.id != id);
        cx.emit(AgentInputEvent::DraftChanged);
        if self.attachment_preview.as_deref() == Some(id) {
            self.attachment_preview = None;
        }
        cx.notify();
    }

    /// 把当前选项与选中项灌进组件库的 `Select`（只在置脏时调用，保证一次同步即收敛）。
    fn sync_model_select(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let groups = model_groups(&self.model_options);
        let selected = selected_model_index(&self.model_options, self.context.model.as_ref());
        self.model_select.update(cx, |state, cx| {
            state.set_items(groups, window, cx);
            state.set_selected_index(selected, window, cx);
        });
    }

    /// 把分支候选与当前分支灌进组件库的 `Select`（与模型那份同一套收敛方式）。
    ///
    /// 选中项按 `current` 反查下标：上下文里没有"当前分支"时宁可置空，
    /// 也不能顺手选中列表里的第一条。
    fn sync_branch_select(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let items = SearchableVec::new(
            self.context
                .branch_options
                .iter()
                .cloned()
                .map(BranchChoice::new)
                .collect::<Vec<_>>(),
        );
        let selected = self
            .context
            .branch_options
            .iter()
            .position(|branch| branch.current)
            .map(IndexPath::new);
        self.branch_select.update(cx, |state, cx| {
            state.set_items(items, window, cx);
            state.set_selected_index(selected, window, cx);
        });
    }
}

impl Focusable for AcpAgentInput {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl EventEmitter<AgentInputEvent> for AcpAgentInput {}

// ---------------------------------------------------------------------------
// agent 提问（ACP elicitation）
//
// 提问只可能来自 ACP agent，所以这套面板放在 ACP 输入框里。One agent 那份保留
// 同样的实现（它历史上也接过 elicitation），两边互不调用。
// ---------------------------------------------------------------------------

struct AcpPendingElicitation {
    request_id: String,
    message: String,
    form: Option<AcpPendingElicitationForm>,
    /// URL 模式：用户去外部页面完成，面板只提供入口与取消。
    url: Option<String>,
    /// 协议新增、本客户端不认识的方式：只提示，不猜渲染。
    unsupported_mode: Option<String>,
    /// 校验提示（必填未填 / 数字解析失败）。
    error: Option<String>,
}

struct AcpPendingElicitationForm {
    title: Option<String>,
    description: Option<String>,
    fields: Vec<AcpPendingElicitationField>,
}

struct AcpPendingElicitationField {
    name: String,
    title: String,
    description: Option<String>,
    required: bool,
    control: AcpPendingElicitationControl,
}

enum AcpPendingElicitationControl {
    /// 文本 / 整数 / 小数共用一个输入框，提交时按 `kind` 解析。
    Text {
        kind: AcpElicitationTextKind,
        /// 输入框在首次渲染时创建——`InputState::new` 需要 `&mut Window`。
        input: Option<Entity<InputState>>,
        placeholder: String,
    },
    Boolean {
        value: bool,
    },
    SingleSelect {
        options: Vec<AcpPendingElicitationChoice>,
        selected: Option<String>,
    },
    MultiSelect {
        options: Vec<AcpPendingElicitationChoice>,
        selected: Vec<String>,
    },
    Unsupported {
        type_name: String,
    },
}

impl AcpPendingElicitationControl {
    /// 有没有能真正收集输入的控件。协议新增的类型不算。
    fn is_editable(&self) -> bool {
        !matches!(self, Self::Unsupported { .. })
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum AcpElicitationTextKind {
    Plain,
    Integer,
    Number,
}

#[derive(Clone)]
struct AcpPendingElicitationChoice {
    value: String,
    title: String,
}

impl AcpPendingElicitation {
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
                pending.form = Some(AcpPendingElicitationForm {
                    title: form.title.clone(),
                    description: form.description.clone(),
                    fields: form
                        .fields
                        .iter()
                        .map(AcpPendingElicitationField::new)
                        .collect(),
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
                AcpPendingElicitationControl::Text { kind, input, .. } => {
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
                        AcpElicitationTextKind::Plain => Value::String(raw.to_string()),
                        AcpElicitationTextKind::Integer => {
                            raw.parse::<i64>().map(Value::from).map_err(|_| {
                                t!(
                                    "AgentUi.elicitation_not_integer",
                                    field = field.title.clone()
                                )
                                .to_string()
                            })?
                        }
                        AcpElicitationTextKind::Number => {
                            raw.parse::<f64>().map(Value::from).map_err(|_| {
                                t!(
                                    "AgentUi.elicitation_not_number",
                                    field = field.title.clone()
                                )
                                .to_string()
                            })?
                        }
                    };
                    content.insert(field.name.clone(), value);
                }
                AcpPendingElicitationControl::Boolean { value } => {
                    // 布尔字段永远给值：不填与「否」在这里没有区别，给 false 更符合直觉。
                    content.insert(field.name.clone(), Value::Bool(*value));
                }
                AcpPendingElicitationControl::SingleSelect { selected, .. } => match selected {
                    Some(value) => {
                        content.insert(field.name.clone(), Value::String(value.clone()));
                    }
                    None if field.required => return Err(required_hint(field)),
                    None => {}
                },
                AcpPendingElicitationControl::MultiSelect { selected, .. } => {
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
                AcpPendingElicitationControl::Unsupported { .. } => {}
            }
        }
        Ok(content)
    }
}

fn required_hint(field: &AcpPendingElicitationField) -> String {
    t!("AgentUi.elicitation_required", field = field.title.clone()).to_string()
}

impl AcpPendingElicitationField {
    fn new(field: &AcpElicitationField) -> Self {
        let control = match &field.kind {
            AcpElicitationFieldKind::Text { format } => AcpPendingElicitationControl::Text {
                kind: AcpElicitationTextKind::Plain,
                input: None,
                placeholder: format
                    .clone()
                    .map(|format| t!("AgentUi.elicitation_format", format = format).to_string())
                    .unwrap_or_default(),
            },
            AcpElicitationFieldKind::Integer => AcpPendingElicitationControl::Text {
                kind: AcpElicitationTextKind::Integer,
                input: None,
                placeholder: String::new(),
            },
            AcpElicitationFieldKind::Number => AcpPendingElicitationControl::Text {
                kind: AcpElicitationTextKind::Number,
                input: None,
                placeholder: String::new(),
            },
            AcpElicitationFieldKind::Boolean => AcpPendingElicitationControl::Boolean {
                value: field
                    .default
                    .as_ref()
                    .and_then(Value::as_bool)
                    .unwrap_or(false),
            },
            AcpElicitationFieldKind::SingleSelect { options } => {
                AcpPendingElicitationControl::SingleSelect {
                    options: elicitation_choices(options),
                    selected: None,
                }
            }
            AcpElicitationFieldKind::MultiSelect { options } => {
                AcpPendingElicitationControl::MultiSelect {
                    options: elicitation_choices(options),
                    selected: Vec::new(),
                }
            }
            AcpElicitationFieldKind::Unsupported { type_name } => {
                AcpPendingElicitationControl::Unsupported {
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

fn elicitation_choices(
    options: &[crate::acp::AcpElicitationOption],
) -> Vec<AcpPendingElicitationChoice> {
    options
        .iter()
        .map(|option| AcpPendingElicitationChoice {
            value: option.value.clone(),
            title: option.title.clone(),
        })
        .collect()
}

impl AcpAgentInput {
    /// 推入 / 清空「agent 提问」。传 `None` 即收起面板。
    pub fn set_pending_elicitation(
        &mut self,
        request: Option<&AcpElicitationRequest>,
        cx: &mut Context<Self>,
    ) {
        self.pending_elicitation = request.map(AcpPendingElicitation::from_request);
        cx.notify();
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
            let AcpPendingElicitationControl::Text {
                input, placeholder, ..
            } = &mut field.control
            else {
                continue;
            };
            if input.is_some() {
                continue;
            }
            let placeholder = placeholder.clone();
            *input =
                Some(cx.new(|cx| InputState::new(window, cx).placeholder(placeholder.clone())));
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

    fn with_elicitation_field(
        &mut self,
        name: &str,
        edit: impl FnOnce(&mut AcpPendingElicitationControl),
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

    fn toggle_elicitation_boolean(&mut self, name: &str, value: bool, cx: &mut Context<Self>) {
        self.with_elicitation_field(
            name,
            |control| {
                if let AcpPendingElicitationControl::Boolean { value: current } = control {
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
                if let AcpPendingElicitationControl::SingleSelect { selected, .. } = control {
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
                if let AcpPendingElicitationControl::MultiSelect { selected, .. } = control {
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

// ---------------------------------------------------------------------------
// 渲染
// ---------------------------------------------------------------------------

impl AcpAgentInput {
    /// 计划栏：agent 报上来的待办（ACP 的 `Plan` 更新）。
    fn render_plan_tab(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let view = cx.entity();
        let is_open = self.open_menu == Some(AcpComposerMenu::Plan);
        let items = self.context.plan_items.clone();
        let label = acp_plan_trigger_label(&items);

        Popover::new("acp-plan-popover")
            .p_0()
            .open(is_open)
            .on_open_change({
                let view = view.clone();
                move |open, _window, cx| {
                    let open = *open;
                    view.update(cx, |this, cx| {
                        this.open_menu = open.then_some(AcpComposerMenu::Plan);
                        cx.notify();
                    });
                }
            })
            .trigger(self.render_capability_trigger(
                "acp-plan-trigger",
                label,
                IconName::ListChecks,
                cx,
            ))
            .content(move |_state, _window, cx| render_acp_plan_content(items.clone(), cx))
    }

    /// 子代理栏：当前会话的子代理卡片。
    fn render_subagent_tab(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let view = cx.entity();
        let is_open = self.open_menu == Some(AcpComposerMenu::SubAgent);
        let subagents = self.context.subagent_items.clone();
        let label = acp_subagent_trigger_label(&subagents);

        Popover::new("acp-subagents-popover")
            .p_0()
            .open(is_open)
            .on_open_change({
                let view = view.clone();
                move |open, _window, cx| {
                    let open = *open;
                    view.update(cx, |this, cx| {
                        this.open_menu = open.then_some(AcpComposerMenu::SubAgent);
                        cx.notify();
                    });
                }
            })
            .trigger(self.render_capability_trigger(
                "acp-subagents-trigger",
                label,
                IconName::Bot,
                cx,
            ))
            .content(move |_state, _window, cx| render_acp_subagent_content(subagents.clone(), cx))
    }

    /// 参数行里的能力 chip（计划 / 子代理）：26 高的 ghost chip，宽度随内容。
    ///
    /// 形态与 One agent 那份共用 [`capability_chip`]：两份输入框的能力 chip 本来就
    /// 该长一个样，只有挂的面板内容不同。标签截断时 tooltip 给出全文。
    fn render_capability_trigger(
        &self,
        id: &'static str,
        label: impl Into<SharedString>,
        icon: IconName,
        cx: &mut Context<Self>,
    ) -> Button {
        let theme = self.local_theme(cx);
        capability_chip(id, label, icon, theme.muted_foreground, &theme, cx)
    }

    /// 模式 chip：选中后 emit `SelectExecutionMode`，id 里带着 `acp-mode:` 前缀，
    /// 上层据此走 `session/set_mode`。
    ///
    /// 与工作区 chip 同一个形态：无描边、无箭头，宽度随当前模式名。图标用 `Key`
    /// （与 One agent 那份的权限 chip 同一个），因为模式本质上是 agent 的执行档位。
    fn render_mode_chip(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let view = cx.entity();
        let is_open = self.open_menu == Some(AcpComposerMenu::Mode);
        let label = current_mode_label(&self.context.execution_mode_label);
        let options = self.mode_options.clone();
        let theme = self.local_theme(cx);

        let trigger = ghost_chip(
            Button::new("acp-mode-trigger")
                .debug_selector(|| "agent-input-acp-mode".to_string())
                .small()
                .w_full()
                .justify_between()
                .disabled(self.is_running)
                .tooltip(label.to_string())
                .child(composer_chip_label(
                    IconName::Settings,
                    theme.foreground,
                    label.clone(),
                )),
            &theme,
            cx,
        );

        div()
            .min_w(sp(CHIP_MIN_WIDTH))
            .max_w(sp(CHIP_MAX_WIDTH))
            .flex_shrink_0()
            .h(sp(CHIP_HEIGHT))
            .overflow_hidden()
            .child(
                Popover::new("acp-mode-popover")
                    .p_0()
                    .open(is_open)
                    .trigger_style(StyleRefinement::default().w_full())
                    .on_open_change({
                        let view = view.clone();
                        move |open, _window, cx| {
                            let open = *open;
                            view.update(cx, |this, cx| {
                                this.open_menu = if open && !this.is_running {
                                    Some(AcpComposerMenu::Mode)
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
                        let current = label.clone();
                        move |_state, _window, cx| {
                            render_mode_content(view.clone(), options.clone(), current.clone(), cx)
                        }
                    }),
            )
    }

    /// 模型下拉：agent 声明的模型候选（`configOptions` 里的 select 值）。
    ///
    /// 组件库 `Select` 在 `appearance(false)` 下是透明底、透明描边，正好当 chip 用；
    /// 形态与 One agent 那份共用 [`composer_model_chip`]。名字被省略时 tooltip 给全名。
    fn render_model_menu(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let theme = self.local_theme(cx);
        let model_label = match &self.context.model {
            Some(model) if model.model_only => model.model.clone(),
            Some(model) => SharedString::from(format!("{} / {}", model.provider, model.model)),
            None => SharedString::from(t!("AgentUi.select_model").to_string()),
        };
        composer_model_chip(
            &self.model_select,
            "acp-model-control",
            "acp-model",
            "agent-input-acp-model-control",
            model_label,
            model_menu_width(&self.model_options),
            self.is_running,
            &theme,
        )
    }

    /// 上下文行里的「工作区」下拉。
    ///
    /// 与分支 / Worktree 一并在卡片外的那一行：它们回答的是“这次跑在哪”。
    fn render_workspace_menu(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let view = cx.entity();
        let is_open = self.open_menu == Some(AcpComposerMenu::Workspace);
        let label = if self.context.workspace.label.is_empty() {
            SharedString::from(t!("AgentUi.composer_pick_workspace").to_string())
        } else {
            self.context.workspace.label.clone()
        };
        let options = self.context.workspace_options.clone();
        let current_path = self.context.workspace.path.clone();
        let theme = self.local_theme(cx);

        let trigger = ghost_chip_muted(
            Button::new("acp-workspace")
                .debug_selector(|| "agent-input-workspace".to_string())
                .small()
                .w_full()
                .justify_between()
                .disabled(self.is_running)
                .tooltip(
                    current_path
                        .clone()
                        .map(|path| path.to_string())
                        .unwrap_or_else(|| t!("AgentUi.composer_pick_workspace").to_string()),
                )
                .child(composer_chip_label(
                    IconName::Workspace,
                    theme.muted_foreground,
                    label.clone(),
                )),
            &theme,
            cx,
        );

        div()
            .min_w(sp(CHIP_MIN_WIDTH))
            .max_w(sp(CHIP_MAX_WIDTH))
            .flex_shrink_0()
            .h(sp(CHIP_HEIGHT))
            .overflow_hidden()
            .child(
                Popover::new("acp-workspace-popover")
                    .p_0()
                    .open(is_open)
                    .trigger_style(StyleRefinement::default().w_full())
                    .on_open_change({
                        let view = view.clone();
                        move |open, _window, cx| {
                            let open = *open;
                            view.update(cx, |this, cx| {
                                this.open_menu = if open && !this.is_running {
                                    Some(AcpComposerMenu::Workspace)
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

    /// 上下文行里的「分支」下拉。
    ///
    /// 与模型同一套控件（组件库 `Select`）：长名字能搜、能滚、键盘能导航——
    /// 分支名动辄 `feature/xxx-yyy`，靠肉眼在下拉里翻慢得多。
    /// 宽度仍按当前分支名估（夹在 chip 区间里），不抢旁边的控件。
    ///
    /// `appearance(false)` 去掉描边与底色，`.icon()` 拿分支图标顶掉 `⌄`；
    /// 两者合起来才是一颗 chip，而不是“套了壳的下拉框”。
    fn render_branch_menu(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let theme = self.local_theme(cx);
        let current = self
            .context
            .branch_options
            .iter()
            .find(|branch| branch.current)
            .map(|branch| branch.name.clone())
            .unwrap_or_else(|| SharedString::from(t!("AgentUi.composer_branch").to_string()));

        div()
            .id("acp-branch-control")
            .debug_selector(|| "agent-input-branch".to_string())
            .min_w(sp(CHIP_MIN_WIDTH))
            .max_w(sp(CHIP_MAX_WIDTH))
            .flex_shrink_0()
            .h(sp(CHIP_HEIGHT))
            .rounded(px(CHIP_RADIUS))
            .overflow_hidden()
            .hover(|this| this.bg(theme.overlay))
            .text_color(theme.muted_foreground)
            .tooltip({
                let full = current.clone();
                move |window, cx| Tooltip::new(full.clone()).build(window, cx)
            })
            .child(
                Select::new(&self.branch_select)
                    .id("acp-branch")
                    .w_full()
                    .h(sp(CHIP_HEIGHT))
                    .small()
                    .icon(IconName::GitBranch)
                    .appearance(false)
                    .placeholder(current)
                    .search_placeholder(t!("AgentUi.search_branch").to_string())
                    .menu_width(branch_menu_width(&self.context.branch_options))
                    .menu_max_h(sp(320.0))
                    .disabled(self.is_running),
            )
    }

    /// 上下文行里的「Worktree」开关。
    ///
    /// 标签固定用短词 `Worktree`：早前直接拿 `worktree.label` 当标签，底栏上就会
    /// 出现两个一模一样的 `navop-wt-ai-workbench`（工作区 chip 一个、这里一个）。
    /// 完整名字进 tooltip，不跟工作区 chip 在字面上打架。
    ///
    /// 开关态画在 `☑/☐` 图标上（开 = accent），不再用组件库 `Checkbox` 的实心
    /// 主色方块 —— 那颗方块是这一行最亮的元素，而这一行只是元数据。
    fn render_worktree_toggle(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let theme = self.local_theme(cx);
        let label = SharedString::from(t!("AgentUi.composer_worktree").to_string());
        let checked = self.context.worktree.enabled;
        let hint = if self.context.worktree.pending {
            t!("AgentUi.composer_worktree_pending").to_string()
        } else {
            t!("AgentUi.composer_worktree_hint").to_string()
        };
        let tooltip = match self.context.worktree.label.clone() {
            Some(full) => format!("{full} · {hint}"),
            None => hint,
        };

        // `☑/☐` 一对字形 + 开启时走 accent：开关语义留在图标上，
        // 不再借组件库 `Checkbox` 那颗实心主色方块——它是这一行最亮的元素，
        // 而这一行只是元数据。
        div()
            .debug_selector(|| "agent-input-worktree".to_string())
            .min_w(sp(CHIP_MIN_WIDTH))
            .flex_shrink_0()
            .overflow_hidden()
            .child(ghost_chip_muted(
                Button::new("acp-worktree-toggle")
                    .disabled(self.is_running)
                    .tooltip(tooltip)
                    .child(composer_chip_label(
                        if checked {
                            IconName::SquareCheck
                        } else {
                            IconName::Square
                        },
                        if checked {
                            theme.accent
                        } else {
                            theme.muted_foreground
                        },
                        label,
                    ))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.open_menu = None;
                        cx.emit(AgentInputEvent::ToggleWorktree { enabled: !checked });
                        cx.notify();
                    })),
                &theme,
                cx,
            ))
    }
}

impl AcpAgentInput {
    fn render_attachments(&self, cx: &mut Context<Self>) -> Option<gpui::AnyElement> {
        if self.attachments.is_empty() {
            return None;
        }
        let theme = self.local_theme(cx);
        let thumbs: Vec<gpui::AnyElement> = self
            .attachments
            .iter()
            .map(|att| {
                let preview_id = att.id.clone();
                let remove_id = att.id.clone();
                let element_id = att.id.clone();
                let previewing = self.attachment_preview.as_deref() == Some(att.id.as_str());
                div()
                    .id(SharedString::from(format!("agent-attachment-{element_id}")))
                    .relative()
                    .debug_selector(move || format!("agent-attachment-{element_id}"))
                    .cursor_pointer()
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.toggle_attachment_preview(&preview_id, cx);
                    }))
                    .child(
                        img(att.image.clone())
                            .w(sp(56.0))
                            .h(sp(56.0))
                            .rounded(cx.theme().radius)
                            .border_1()
                            .border_color(if previewing {
                                theme.accent
                            } else {
                                theme.border
                            }),
                    )
                    .child(
                        div().absolute().top_0().right_0().child(
                            Button::new(SharedString::from(format!("rm-att-{remove_id}")))
                                .icon(IconName::Close)
                                .ghost()
                                .xsmall()
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.remove_attachment(&remove_id, cx);
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
                        Button::new(SharedString::from(format!("acp-queued-send-{index}")))
                            .debug_selector(|| "agent-input-queued-send".to_string())
                            .icon(IconName::ArrowUp)
                            .ghost()
                            .small()
                            .tooltip(t!("AgentUi.send_queued_now").to_string())
                            .on_click(cx.listener(move |_this, _, _window, cx| {
                                cx.emit(AgentInputEvent::SendQueuedNow { index });
                                cx.notify();
                            })),
                    )
                    .child(
                        Button::new(SharedString::from(format!("acp-queued-edit-{index}")))
                            .icon(IconName::Edit)
                            .ghost()
                            .small()
                            .tooltip(t!("AgentUi.edit_queued_item").to_string())
                            .on_click(cx.listener(move |_this, _, window, cx| {
                                cx.emit(AgentInputEvent::EditQueued { index });
                                window.refresh();
                                cx.notify();
                            })),
                    )
                    .child(
                        Button::new(SharedString::from(format!("acp-queued-remove-{index}")))
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

    /// 上下文用量圆环（点开是明细）。
    ///
    /// 现在收在上下文行的尾端，只剩一个环：百分比数字本来就写在明细里，常驻在环
    /// 旁边的那串字每时每刻都在给“正在写的东西”添杂音。
    fn render_usage_gauge(&self, cx: &mut Context<Self>) -> Option<impl IntoElement + use<>> {
        let usage = self.context.context_usage.clone()?;
        let theme = self.local_theme(cx);
        let color = usage_color(&usage, &theme);
        let percent = usage.percent().unwrap_or(0.0);
        let label = usage.gauge_label();
        let detail = usage.detail_text();
        let view = cx.entity();
        let trigger = ghost_chip(
            Button::new("acp-usage-gauge")
                .debug_selector(|| "agent-input-usage-gauge".to_string())
                .child(
                    div().size(sp(USAGE_GAUGE_DIAMETER)).child(
                        ProgressCircle::new("acp-usage")
                            .value(percent)
                            .color(color)
                            .accessibility_label(detail),
                    ),
                )
                .small()
                .size(sp(CHIP_HEIGHT))
                .flex_shrink_0()
                .tooltip(label),
            &theme,
            cx,
        );
        Some(
            Popover::new("acp-usage-popover")
                .trigger(trigger)
                // 受控开合：详情里的「查看历史」要能把它关上。
                .open(self.usage_detail_open)
                .on_open_change({
                    let view = view.clone();
                    move |open, _window, cx| {
                        let open = *open;
                        view.update(cx, |this, cx| {
                            if this.usage_detail_open != open {
                                this.usage_detail_open = open;
                                cx.notify();
                            }
                        });
                    }
                })
                .content({
                    let view = view.clone();
                    move |_state, _window, _cx| render_usage_detail(&usage, &theme, view.clone())
                }),
        )
    }

    /// 能力行（卡片顶部）：`[计划][子代理]`。
    ///
    /// agent 自己声明的两类能力放在编辑器上方，参数行只剩模型 / 模式 / 附件 /
    /// 发送键。侧栏一窄，一行里两类控件会一起退到 `CHIP_MIN_WIDTH`、标签只剩
    /// 省略号；分成两行后各自都有地方把标签写全，与 One agent 那份同一把尺子。
    fn render_capability_row(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let theme = self.local_theme(cx);
        composer_capability_row(&theme)
            .child(composer_capability_slot().child(self.render_plan_tab(cx)))
            .child(composer_capability_slot().child(self.render_subagent_tab(cx)))
    }

    /// 参数行：`… [模型][模式][附件][发送 / 停止]`。
    ///
    /// 卡片的最后一行，管的是“这一轮怎么跑、怎么发出去”：左边留白，右边依次是
    /// 模型、模式、附件与发送键。
    ///
    /// 与上一版的三处不同：工作区 / 分支 / Worktree 搬去了卡片外的上下文行；
    /// 附件从“图标 + 数量 + 竖线”收成一颗纯图标钮（有附件时输入框上方本来就长
    /// 一排缩略图，再写一遍数字是重复）；发送键改成 26 的圆形，与 chip 同高。
    fn render_toolbar(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let theme = self.local_theme(cx);
        let queue_mode = self.is_running || self.pending_queue_blocked;
        let attach_count = self.attachments.len();
        let attach_tooltip = if attach_count == 0 {
            t!("AgentUi.attach_images").to_string()
        } else {
            format!(
                "{} · {}",
                t!("AgentUi.attach_images"),
                t!("AgentUi.attachment_count", count = attach_count)
            )
        };

        let action_size = action_button_size();
        let mut row = composer_toolbar(&theme);
        // 参数行只放「怎么跑、怎么发」：模型 / 模式 / 附件 / 发送。计划与子代理
        // 搬去了卡片顶部的能力行 —— 一行塞不下两类控件时，两边都会退成省略号。
        row = row
            // 空白把参数区推到右端，与 One agent 那份同一种排法。
            .child(div().flex_1().min_w_0())
            .child(self.render_model_menu(cx))
            .child(self.render_mode_chip(cx))
            .child(ghost_chip(
                Button::new("acp-attach")
                    .debug_selector(|| "agent-input-attach".to_string())
                    .icon(IconName::File)
                    .xsmall()
                    .glyph_size(px(16.0))
                    .size(action_size)
                    .flex_shrink_0()
                    .tooltip(attach_tooltip)
                    .on_click(cx.listener(|this, _, window, cx| this.open_file_picker(window, cx))),
                &theme,
                cx,
            ));

        if queue_mode {
            row = row
                .child(
                    div()
                        .w(action_size)
                        .h(action_size)
                        .flex_shrink_0()
                        .debug_selector(|| "agent-input-queue-send".to_string())
                        .child(
                            Button::new("acp-queue-send")
                                .debug_selector(|| "agent-queue-send-button".to_string())
                                .icon(IconName::ArrowUp)
                                .primary()
                                .small()
                                .glyph_size(px(16.0))
                                .size(action_size)
                                .rounded(action_button_radius())
                                .cursor_pointer()
                                .tooltip(t!("AgentUi.queue_for_next_turn").to_string())
                                .on_click(
                                    cx.listener(|this, _, window, cx| this.submit(window, cx)),
                                ),
                        ),
                )
                .child(
                    div()
                        .w(action_size)
                        .h(action_size)
                        .flex_shrink_0()
                        .debug_selector(|| "agent-input-stop".to_string())
                        .child(
                            Button::new("acp-stop")
                                .debug_selector(|| "agent-stop-button".to_string())
                                .icon(IconName::Close)
                                .danger()
                                .small()
                                .glyph_size(px(16.0))
                                .size(action_size)
                                .rounded(action_button_radius())
                                .cursor_pointer()
                                .tooltip(t!("AgentUi.stop").to_string())
                                .on_click(cx.listener(|this, _, _, cx| this.stop(cx))),
                        ),
                );
        } else {
            row = row.child(
                div()
                    .w(action_size)
                    .h(action_size)
                    .flex_shrink_0()
                    .debug_selector(|| "agent-input-send-control".to_string())
                    .child(
                        Button::new("acp-send")
                            .debug_selector(|| "agent-send-button".to_string())
                            .icon(IconName::ArrowUp)
                            .primary()
                            .small()
                            .glyph_size(px(16.0))
                            .size(action_size)
                            .rounded(action_button_radius())
                            .cursor_pointer()
                            .tooltip(t!("AgentUi.send").to_string())
                            .on_click(cx.listener(|this, _, window, cx| this.submit(window, cx))),
                    ),
            );
        }

        row
    }

    /// 卡片外的上下文行：`[工作区][分支][Worktree] … [用量]`。
    ///
    /// 这一行在卡片**外面**：工作区 / 分支 / Worktree 回答的是“这次跑在哪”，与
    /// “这次怎么跑”（模型 / 模式）不是一件事。搬出去之后，卡片里只剩下说话的地方
    /// 和它的参数行，卡片的边界也终于等于“一段对话”的边界。
    ///
    /// 行高 28、内边距 10、组内间距 2——waku 那一行的实测值，比卡片内部的控件松、
    /// 比卡片外围紧，读起来是“贴在卡片下的一行注释”，而不是第二个工具栏。
    fn render_context_row(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let theme = self.local_theme(cx);
        let is_git_repo =
            self.context.workspace.is_git_repo || !self.context.branch_options.is_empty();

        // 一行放不下就换行，不横向硬挤：这三颗都是"要读得出内容"的元数据，
        // 压到 44px 下限只剩省略号，等于没显示（窄面板下必然发生）。
        let mut group = h_flex()
            .debug_selector(|| "agent-input-context-group".to_string())
            .flex_1()
            .min_w_0()
            .flex_wrap()
            .items_center()
            .gap(sp(2.0))
            .child(self.render_workspace_menu(cx));
        if is_git_repo {
            group = group
                .child(self.render_branch_menu(cx))
                .child(self.render_worktree_toggle(cx));
        }

        composer_context_row(&theme)
            .child(group)
            .children(self.render_usage_gauge(cx))
    }
}

impl AcpAgentInput {
    fn render_pending_elicitation(&self, cx: &mut Context<Self>) -> Option<gpui::AnyElement> {
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
                            "acp-elicitation-open-{}",
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
            body = body.child(div().text_xs().text_color(theme.muted_foreground).child(
                t!("AgentUi.elicitation_unsupported_mode", mode = mode.clone()).to_string(),
            ));
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
                "acp-elicitation-cancel-{}",
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
                        "acp-elicitation-decline-{}",
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
                        "acp-elicitation-submit-{}",
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
        field: &AcpPendingElicitationField,
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
            label = label.child(div().text_xs().text_color(cx.theme().danger).child("*"));
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
            AcpPendingElicitationControl::Text { input, .. } => {
                if let Some(input) = input {
                    column = column.child(Input::new(input).cleanable(true).small().w_full());
                }
            }
            AcpPendingElicitationControl::Boolean { value } => {
                let checked = *value;
                column = column.child(
                    Switch::new(SharedString::from(format!("acp-elicitation-bool-{name}")))
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
            AcpPendingElicitationControl::SingleSelect { options, selected } => {
                let mut row = h_flex().w_full().min_w_0().flex_wrap().gap_1();
                for option in options {
                    let is_selected = selected.as_deref() == Some(option.value.as_str());
                    row = row.child(
                        Button::new(SharedString::from(format!(
                            "acp-elicitation-radio-{name}-{}",
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
                            move |this, _, _, cx| this.select_elicitation_option(&name, &value, cx)
                        })),
                    );
                }
                column = column.child(row);
            }
            AcpPendingElicitationControl::MultiSelect { options, selected } => {
                let mut row = h_flex().w_full().min_w_0().flex_wrap().gap_1();
                for option in options {
                    let is_selected = selected.contains(&option.value);
                    row = row.child(
                        Button::new(SharedString::from(format!(
                            "acp-elicitation-checkbox-{name}-{}",
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
                            move |this, _, _, cx| this.toggle_elicitation_option(&name, &value, cx)
                        })),
                    );
                }
                column = column.child(row);
            }
            AcpPendingElicitationControl::Unsupported { type_name } => {
                column = column.child(
                    div().text_xs().text_color(theme.muted_foreground).child(
                        t!(
                            "AgentUi.elicitation_unsupported_field",
                            kind = type_name.clone()
                        )
                        .to_string(),
                    ),
                );
            }
        }
        column.into_any_element()
    }
}

impl Render for AcpAgentInput {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // 模型下拉换了选项：趁手里有窗口时灌进组件库的 state。
        if self.model_select_dirty {
            self.sync_model_select(window, cx);
            self.model_select_dirty = false;
        }
        if self.branch_select_dirty {
            self.sync_branch_select(window, cx);
            self.branch_select_dirty = false;
        }

        let attachments = self.render_attachments(cx);
        let queued_submissions = self.render_queued_submissions(cx);
        self.ensure_elicitation_inputs(window, cx);
        let pending_elicitation = self.render_pending_elicitation(cx);
        let capability_row = self.render_capability_row(cx);
        let toolbar = self.render_toolbar(cx);
        let context_row = self.render_context_row(cx);
        let theme = self.local_theme(cx);
        let input_state = self.input_state.read(cx);
        let input_focused = input_state.focus_handle(cx).is_focused(window);
        let editor_height = composer_editor_height(input_state);
        // 拖放悬停时的染色。`drag_over` 的闭包要 `'static`，所以先把颜色取成
        // 本地副本（Hsla 是 Copy），而不是把 `theme` 移进去。
        let drop_wash = theme.accent.opacity(0.08);
        let drop_ring = theme.accent;
        let card = composer_card(&theme, self.edge_to_edge)
            // 从 Finder 拖文件进来：悬停给卡片染一层，松手落成附件或路径。
            .drag_over::<ExternalPaths>(move |style, _, _, _| {
                style.bg(drop_wash).border_color(drop_ring)
            })
            .on_drop(cx.listener(|this, paths: &ExternalPaths, window, cx| {
                this.add_dropped_paths(paths.paths(), window, cx);
            }))
            // 顶部：计划 / 子代理（agent 自己声明的能力）
            .child(capability_row)
            // 附件紧贴输入框上方：它是“这一轮要发出去的东西”
            .children(attachments)
            .when_some(queued_submissions, |this, queued| this.child(queued))
            // agent 提问：放在输入框正上方，答完才收起
            .when_some(pending_elicitation, |this, pending| this.child(pending))
            // 中部：多行输入框。它自己带一个框——与 One agent 那份共用同一套
            // 尺子：`theme.inset` 打底（面板调色板里「内嵌一层」的那个表面），
            // 聚焦时这里的描边转 accent，卡片外框保持安静。
            .child(composer_editor_frame(
                &theme,
                self.edge_to_edge,
                input_focused,
                Editor::new(&self.input_state)
                    .w_full()
                    .h(editor_height)
                    .appearance(false)
                    .bordered(false)
                    .text_color(theme.foreground),
            ))
            // 底部：模型 / 模式 / 附件 / 发送，一整行
            .child(toolbar);

        composer_root(&theme, self.edge_to_edge, "agent-input-acp-root")
            .track_focus(&self.focus_handle)
            .child(card)
            // 卡片外：上下文行（工作区 / 分支 / Worktree / 用量）
            .child(context_row)
    }
}

/// 模式下拉的菜单内容：agent 声明了哪些模式、当前是哪一个。
///
/// 选中项靠**标签**比对（与 One agent 同一套约定）：选项里没有"是否选中"字段，
/// 当前模式只有 agent 报上来的那一份标签（见 [`current_mode_label`]）。
fn render_mode_content(
    view: Entity<AcpAgentInput>,
    options: Vec<ComposerMenuOption>,
    current_label: SharedString,
    cx: &mut Context<gpui_component::popover::PopoverState>,
) -> gpui::AnyElement {
    let theme = active_agent_chat_theme(cx);
    let mut col = composer_popover_surface(&theme).min_w(sp(200.0));

    col = col.child(context_group_label(t!("AgentUi.mode").to_string(), &theme));

    if options.is_empty() {
        col = col.child(
            div()
                .px_2()
                .py_2()
                .text_sm()
                .text_color(theme.muted_foreground)
                .child(t!("AgentUi.acp_mode_empty").to_string()),
        );
    }

    for option in options {
        let selected = option.label == current_label;
        col = col.child(mode_option_row(view.clone(), option, selected, &theme, cx));
    }

    col.into_any_element()
}

fn mode_option_row(
    view: Entity<AcpAgentInput>,
    option: ComposerMenuOption,
    selected: bool,
    theme: &AgentChatTheme,
    cx: &mut Context<gpui_component::popover::PopoverState>,
) -> gpui::AnyElement {
    let muted = theme.muted_foreground;
    let hover_bg = theme.hover_background();
    let selected_bg = theme.selection_background();
    let selected_fg = theme.accent;
    let id = option.id.clone();
    let row_id = SharedString::from(format!("agent-acp-mode-opt-{id}"));
    let mut inner = v_flex()
        .flex_1()
        .min_w_0()
        .gap(px(1.0))
        .child(div().text_sm().truncate().child(option.label));
    if let Some(hint) = option.hint {
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
        .when(selected, |this| {
            this.bg(selected_bg).text_color(theme.foreground)
        })
        .hover(move |this| this.bg(hover_bg))
        .child(inner)
        .when(selected, |this| {
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
    view: Entity<AcpAgentInput>,
    options: Vec<ComposerWorkspaceOption>,
    current_path: Option<SharedString>,
    theme: &AgentChatTheme,
    cx: &mut Context<gpui_component::popover::PopoverState>,
) -> gpui::AnyElement {
    let mut col = composer_popover_surface(theme).min_w(sp(260.0));

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
    col.child(div().my_1().h(px(1.0)).w_full().bg(theme.border))
        .child(browse_workspace_row(view, theme, cx))
        .into_any_element()
}

fn workspace_option_row(
    view: Entity<AcpAgentInput>,
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
    view: Entity<AcpAgentInput>,
    theme: &AgentChatTheme,
    cx: &mut Context<gpui_component::popover::PopoverState>,
) -> gpui::AnyElement {
    let hover_bg = theme.hover_background();

    h_flex()
        .id("acp-composer-workspace-browse")
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

/// 分支下拉的列表项。
///
/// 值直接就是 [`ComposerBranchOption`]：选中回调拿到的就是完整选项（含「是不是远端」），
/// 不用再对着名字反查。
#[derive(Clone, Debug)]
struct BranchChoice {
    option: ComposerBranchOption,
}

impl BranchChoice {
    fn new(option: ComposerBranchOption) -> Self {
        Self { option }
    }
}

impl SelectItem for BranchChoice {
    type Value = ComposerBranchOption;

    fn title(&self) -> SharedString {
        self.option.name.clone()
    }

    fn value(&self) -> &Self::Value {
        &self.option
    }

    /// 分支名常常很长，按子串不区分大小写匹配；空查询视为全命中。
    fn matches(&self, query: &str) -> bool {
        let query = query.trim().to_lowercase();
        query.is_empty() || self.option.name.to_lowercase().contains(&query)
    }

    fn render(&self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let muted = crate::theme::active_agent_chat_theme(cx).muted_foreground;
        // 行高由列表按**第一行**量一次、全体共用（`List::prepare_items_if_needed`），
        // 所以这里必须是死的一行：名字下面再压一行（远端标记）会撑破后面的行 ——
        // 屏幕上就是一行叠在一行上。远端标记因此并到同一行的右侧。
        h_flex()
            .debug_selector(|| {
                if self.option.remote {
                    "agent-branch-option-remote".to_string()
                } else {
                    "agent-branch-option-local".to_string()
                }
            })
            .w_full()
            .min_w_0()
            .gap_1()
            .items_center()
            .child(
                div()
                    // 共享选择器：点开/点选那类用例不需要区分本地与远端。
                    .debug_selector(|| "agent-branch-option".to_string())
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .text_sm()
                    .child(self.option.name.clone()),
            )
            .when(self.option.remote, |this| {
                this.child(
                    div()
                        .flex_none()
                        .text_xs()
                        .text_color(muted)
                        .child(t!("AgentUi.composer_branch_remote").to_string()),
                )
            })
    }
}

/// 分支下拉弹层的宽度：按最长的分支名估，保证点开能读到完整的名字。
fn branch_menu_width(options: &[ComposerBranchOption]) -> gpui::Rems {
    let widest = options
        .iter()
        .map(|option| estimated_label_width(option.name.as_ref()))
        .fold(0.0_f32, f32::max);
    sp((widest + BRANCH_MENU_CHROME_WIDTH).clamp(BRANCH_MENU_MIN_WIDTH, BRANCH_MENU_MAX_WIDTH))
}

/// 计划栏的触发标签：`0/3 进行中`。计数比单独的"计划"更能说明该不该点开。
fn acp_plan_trigger_label(items: &[ComposerPlanItem]) -> SharedString {
    if items.is_empty() {
        return SharedString::from(t!("AgentUi.plan").to_string());
    }
    let total = items.len();
    let completed = items
        .iter()
        .filter(|item| item.status.as_ref() == "completed")
        .count();
    let label = if completed == total {
        t!("AgentUi.completed").to_string()
    } else if items
        .iter()
        .any(|item| matches!(item.status.as_ref(), "running" | "in_progress"))
    {
        t!("AgentUi.in_progress").to_string()
    } else {
        t!("AgentUi.pending").to_string()
    };
    SharedString::from(format!("{completed}/{total} {label}"))
}

/// 子代理栏的触发标签：空的时候写名字，有内容时写数量。
fn acp_subagent_trigger_label(subagents: &[ComposerSubAgentItem]) -> SharedString {
    if subagents.is_empty() {
        SharedString::from(t!("AgentUi.subagents").to_string())
    } else {
        SharedString::from(t!("AgentUi.subagent_count", count = subagents.len()).to_string())
    }
}

fn acp_plan_status_label(status: &str) -> String {
    match status {
        "completed" => t!("AgentUi.completed_state").to_string(),
        "running" | "in_progress" => t!("AgentUi.in_progress").to_string(),
        "failed" => t!("AgentUi.failed").to_string(),
        "cancelled" => t!("AgentUi.cancelled").to_string(),
        _ => t!("AgentUi.pending").to_string(),
    }
}

/// 计划面板：agent 自己报上来的待办。
///
/// 比 One agent 那份简：那份的每一行能展开看工具、风险与细节，这里只列标题 /
/// 说明 / 状态——ACP 报上来的 `Plan` 本来就只有这几项，展开了也是空的。
fn render_acp_plan_content(
    items: Vec<ComposerPlanItem>,
    cx: &mut Context<gpui_component::popover::PopoverState>,
) -> gpui::AnyElement {
    let theme = active_agent_chat_theme(cx);
    let muted = theme.muted_foreground;
    let mut col = composer_popover_surface(&theme).min_w(sp(320.0));

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

    for (index, item) in items.into_iter().enumerate() {
        col = col.child(acp_plan_item_row(index, item, &theme));
    }
    col.into_any_element()
}

fn acp_plan_item_row(
    index: usize,
    item: ComposerPlanItem,
    theme: &AgentChatTheme,
) -> gpui::AnyElement {
    let muted = theme.muted_foreground;
    let success = theme.success;
    let warning = theme.warning;
    let (icon, icon_color) = match item.status.as_ref() {
        "completed" => (IconName::CircleCheck, success),
        "running" | "in_progress" => (IconName::LoaderCircle, warning),
        _ => (IconName::CircleCheck, muted),
    };

    let mut inner = v_flex()
        .flex_1()
        .min_w_0()
        .gap(px(1.0))
        .child(div().text_sm().truncate().child(item.title));
    if !item.description.is_empty() {
        inner = inner.child(
            div()
                .text_xs()
                .text_color(muted)
                .truncate()
                .child(item.description),
        );
    }

    h_flex()
        .id(SharedString::from(format!("acp-plan-item-{index}")))
        .w_full()
        .items_center()
        .gap_2()
        .px_2()
        .py_1()
        .border_b_1()
        .border_color(theme.border)
        .child(Icon::new(icon).xsmall().text_color(icon_color))
        .child(inner)
        .child(
            div()
                .flex_shrink_0()
                .text_xs()
                .text_color(muted)
                .child(acp_plan_status_label(item.status.as_ref())),
        )
        .into_any_element()
}

/// 子代理面板：当前会话的子代理卡片。
fn render_acp_subagent_content(
    subagents: Vec<ComposerSubAgentItem>,
    cx: &mut Context<gpui_component::popover::PopoverState>,
) -> gpui::AnyElement {
    let theme = active_agent_chat_theme(cx);
    let muted = theme.muted_foreground;
    let mut col = composer_popover_surface(&theme).min_w(sp(300.0));

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
        col = col.child(acp_subagent_item_row(subagent, &theme, cx));
    }
    col.into_any_element()
}

fn acp_subagent_item_row(
    item: ComposerSubAgentItem,
    theme: &AgentChatTheme,
    cx: &mut Context<gpui_component::popover::PopoverState>,
) -> gpui::AnyElement {
    let muted = theme.muted_foreground;
    let (icon, icon_color) = match item.status.as_ref() {
        "completed" => (IconName::CircleCheck, cx.theme().success),
        "failed" => (IconName::CircleX, cx.theme().danger),
        "running" | "in_progress" => (IconName::LoaderCircle, cx.theme().warning),
        _ => (IconName::Bot, muted),
    };

    h_flex()
        .id(SharedString::from(format!(
            "acp-running-subagent-{}",
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
                .flex_shrink_0()
                .text_xs()
                .text_color(muted)
                .child(acp_plan_status_label(item.status.as_ref())),
        )
        .into_any_element()
}

fn context_group_label(label: impl Into<SharedString>, theme: &AgentChatTheme) -> gpui::AnyElement {
    div()
        .px_2()
        .py_1()
        .text_xs()
        .text_color(theme.muted_foreground)
        .child(label.into())
        .into_any_element()
}

/// 占用分档 → 圆环颜色：常规、70% 起警告、90% 起危险。
fn usage_color(usage: &ContextUsage, theme: &AgentChatTheme) -> gpui::Hsla {
    match usage.pressure() {
        UsagePressure::Calm => theme.gauge,
        UsagePressure::Elevated => theme.gauge_warning,
        UsagePressure::Critical => theme.gauge_danger,
    }
}

/// 用量详情面板：点圆环展开的完整读数。
fn render_usage_detail(
    usage: &ContextUsage,
    theme: &AgentChatTheme,
    view: Entity<AcpAgentInput>,
) -> impl IntoElement + use<> {
    let accent = usage_color(usage, theme);
    let rows = usage_detail_rows(usage)
        .into_iter()
        .map(|(label, value)| usage_detail_row(label, value, theme))
        .collect::<Vec<_>>();

    v_flex()
        .debug_selector(|| "agent-usage-detail".to_string())
        .w(sp(220.0))
        .gap_2()
        .p_3()
        .bg(theme.raised)
        .border_1()
        .border_color(theme.border)
        .rounded(theme.surface_radius)
        .shadow_md()
        .child(
            div()
                .text_xs()
                .text_color(theme.muted_foreground)
                .child(t!("AgentUi.usage_title").to_string()),
        )
        .children(match usage.percent() {
            Some(percent) => Some(usage_percent_bar(percent, accent, theme)),
            None => None,
        })
        .children(rows)
        .child(
            div().pt_2().border_t_1().border_color(theme.border).child(
                Button::new("acp-usage-history-open")
                    .small()
                    .ghost()
                    .label(t!("AgentUi.usage_history_open").to_string())
                    .on_click(move |_event, _window, cx| {
                        view.update(cx, |this, cx| {
                            this.usage_detail_open = false;
                            cx.emit(AgentInputEvent::OpenUsageHistory);
                            cx.notify();
                        });
                    }),
            ),
        )
}

/// 详情面板里的「标签 / 数值」列表。
fn usage_detail_rows(usage: &ContextUsage) -> Vec<(String, String)> {
    let mut rows = vec![(
        t!("AgentUi.usage_used").to_string(),
        format_token_count(usage.used),
    )];
    match usage.window.filter(|window| *window > 0) {
        Some(window) => {
            rows.push((
                t!("AgentUi.usage_limit").to_string(),
                format_token_count(window),
            ));
            rows.push((
                t!("AgentUi.usage_remaining").to_string(),
                format_token_count(window.saturating_sub(usage.used)),
            ));
        }
        None => rows.push((
            t!("AgentUi.usage_limit").to_string(),
            t!("AgentUi.usage_unknown").to_string(),
        )),
    }
    if let Some(cost) = usage.cost.as_deref() {
        rows.push((t!("AgentUi.usage_cost").to_string(), cost.to_string()));
    }
    rows
}

fn usage_detail_row(
    label: String,
    value: String,
    theme: &AgentChatTheme,
) -> impl IntoElement + use<> {
    h_flex()
        .w_full()
        .items_center()
        .justify_between()
        .gap_3()
        .child(
            div()
                .flex_shrink_0()
                .text_xs()
                .text_color(theme.muted_foreground)
                .child(label),
        )
        .child(
            div()
                .min_w_0()
                .text_xs()
                .text_color(theme.foreground)
                .child(value),
        )
}

/// 占用比例条：轨道走 `inset`，已完成部分走当前档位色。
fn usage_percent_bar(
    percent: f32,
    accent: gpui::Hsla,
    theme: &AgentChatTheme,
) -> impl IntoElement + use<> {
    div()
        .w_full()
        .h(sp(6.0))
        .rounded_full()
        .bg(theme.inset)
        .child(
            div()
                .h_full()
                .w(relative((percent / 100.0).clamp(0.0, 1.0)))
                .rounded_full()
                .bg(accent),
        )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plan_item(status: &str) -> ComposerPlanItem {
        ComposerPlanItem::new("步骤", status)
    }

    /// 计划栏的标签把「做完了几步」写在最前面，一眼看得出该不该点开。
    #[test]
    fn plan_trigger_label_counts_steps() {
        assert_eq!(
            acp_plan_trigger_label(&[]).as_ref(),
            t!("AgentUi.plan").as_ref(),
            "没有计划时只写名字"
        );

        let items = [plan_item("completed"), plan_item("running")];
        let label = acp_plan_trigger_label(&items);
        assert!(label.starts_with("1/2"), "计数写在前面：{label}");

        let done = [plan_item("completed"), plan_item("completed")];
        let label = acp_plan_trigger_label(&done);
        assert!(label.starts_with("2/2"), "{label}");
    }

    /// 子代理栏空的时候写名字，有内容时写数量。
    #[test]
    fn subagent_trigger_label_reports_the_count() {
        assert_eq!(
            acp_subagent_trigger_label(&[]).as_ref(),
            t!("AgentUi.subagents").as_ref()
        );

        let items = [ComposerSubAgentItem::new(
            "id-1",
            "审阅者",
            "看事件流",
            "running",
        )];
        assert_eq!(
            acp_subagent_trigger_label(&items).as_ref(),
            t!("AgentUi.subagent_count", count = 1).as_ref()
        );
    }

    /// 分支下拉的搜索：按名字子串、不区分大小写；空查询不把候选滤空。
    #[test]
    fn branch_search_matches_the_name_case_insensitively() {
        let choice = BranchChoice::new(ComposerBranchOption::new("feature/Impl-AI", false, false));

        assert!(choice.matches("impl"));
        assert!(choice.matches("FEATURE"), "大小写不该影响命中");
        assert!(choice.matches("ature/impl-ai"));
        assert!(choice.matches("   "), "空查询=全部命中，不能把列表滤空");
        assert!(choice.matches(""));
        assert!(!choice.matches("main"));
    }

    /// 弹层宽度跟最长的分支名走，并夹在区间里：短名字不拖一个太空的层。
    #[test]
    fn branch_menu_width_tracks_the_widest_name_within_bounds() {
        let narrow = branch_menu_width(&[ComposerBranchOption::new("main", true, false)]);
        let wide = branch_menu_width(&[ComposerBranchOption::new(
            "feature/a-very-long-branch-name-for-the-test",
            true,
            false,
        )]);
        assert!(
            wide.0 > narrow.0,
            "最长的名字越宽，弹层越宽：{narrow:?} vs {wide:?}"
        );
        assert!(
            narrow.0 >= sp(BRANCH_MENU_MIN_WIDTH).0,
            "再短也不低于下限：{narrow:?}"
        );
        assert!(
            branch_menu_width(&[ComposerBranchOption::new(
                "feature/".repeat(20),
                true,
                false
            )])
            .0 <= sp(BRANCH_MENU_MAX_WIDTH).0,
            "再长也不超过上限"
        );
    }
}
