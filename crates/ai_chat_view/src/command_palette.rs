//! 命令面板。
//!
//! `cmd-k`（可换 `ctrl-k`）拉起一个输入框加结果列表：上面几条是固定命令，
//! 下面是要跳过去的会话。用来把「新建会话 / 切会话 / 开会话内搜索 / 看用量」
//! 这类动作从「先找到按钮再点」变成「打两个字回车」。
//!
//! 交互分工：
//! - 查询框吃输入（`InputEvent`），`Enter` 执行当前高亮；上下键由绑定层转发成
//!   [`PaletteSelectPrev`] / [`PaletteSelectNext`]，不走输入组件自己的光标移动。
//! - 焦点在查询框里，所以上下键的绑定必须是**复合上下文**
//!   [`AI_CHAT_PALETTE_INPUT_CONTEXT`]，否则会被 `Input` 自己的绑定压住
//!   （同 [`crate::find_shortcut`] 里 `cmd-f` 的处理）。
//! - `escape` 关面板并把焦点还给 composer：面板消失后焦点若留在已出树的输入框上，
//!   键盘事件就再没人接。

use std::rc::Rc;

use gpui::prelude::FluentBuilder;
use gpui::{
    AnyElement, App, InteractiveElement, IntoElement, KeyBinding, MouseButton, MouseDownEvent,
    ParentElement, SharedString, StatefulInteractiveElement, Styled, Window, div, px,
};
use gpui_component::{Icon, Sizable, h_flex, v_flex};
use one_assets::IconName;
use rust_i18n::t;

use crate::find_shortcut::{AI_CHAT_COMPOSER_CONTEXT, AI_CHAT_SEARCH_CONTEXT};
use crate::theme::{AgentChatTheme, sp};

/// 面板自身的按键上下文（挂在浮层根节点上）。
pub const AI_CHAT_PALETTE_CONTEXT: &str = "AiChatCommandPalette";
/// 查询框聚焦时的复合上下文：压过 `Input` 自己的方向键绑定。
pub const AI_CHAT_PALETTE_INPUT_CONTEXT: &str = "AiChatCommandPalette > Input";
/// 打开/关闭面板的按键上下文：与 `cmd-f` 一样，要在根节点与 composer 两种栈下都生效。
const PALETTE_TOGGLE_CONTEXTS: [&str; 4] = [
    AI_CHAT_SEARCH_CONTEXT,
    AI_CHAT_COMPOSER_CONTEXT,
    AI_CHAT_PALETTE_CONTEXT,
    AI_CHAT_PALETTE_INPUT_CONTEXT,
];
/// 面板里生效的上下文（自身 + 查询框聚焦）。
const PALETTE_NAV_CONTEXTS: [&str; 2] = [AI_CHAT_PALETTE_CONTEXT, AI_CHAT_PALETTE_INPUT_CONTEXT];

const MACOS_TOGGLE_SHORTCUT: &str = "cmd-k";
const OTHER_TOGGLE_SHORTCUT: &str = "ctrl-k";

/// 面板宽度。够放「图标 + 会话标题 + 工作区名」，又不至于盖满正文。
const PALETTE_WIDTH: f32 = 520.0;
/// 一次最多列几条。多了就要滚，滚动列表里用方向键选不如直接打字过滤。
const PALETTE_MAX_ROWS: usize = 7;
/// 浮层距面板顶部的距离：留出「从上方落下」的观感，也不遮住输入框。
const PALETTE_TOP_INSET: f32 = 56.0;
/// 会话候选最多列出的条数（按最近更新排序后取前 N）。
pub(crate) const PALETTE_MAX_SESSIONS: usize = 40;

gpui::actions!(
    ai_chat_command_palette,
    [
        ToggleCommandPalette,
        CloseCommandPalette,
        PaletteSelectFirst,
        PaletteSelectLast,
        PaletteSelectPageUp,
        PaletteSelectPageDown,
        PaletteSelectPrev,
        PaletteSelectNext
    ]
);

pub fn init(cx: &mut App) {
    cx.bind_keys(keybindings());
}

fn keybindings() -> Vec<KeyBinding> {
    let toggle = if cfg!(target_os = "macos") {
        MACOS_TOGGLE_SHORTCUT
    } else {
        OTHER_TOGGLE_SHORTCUT
    };
    let mut keybindings = Vec::new();
    for context in PALETTE_TOGGLE_CONTEXTS {
        keybindings.push(KeyBinding::new(toggle, ToggleCommandPalette, Some(context)));
    }
    for context in PALETTE_NAV_CONTEXTS {
        keybindings.push(KeyBinding::new("up", PaletteSelectPrev, Some(context)));
        keybindings.push(KeyBinding::new("down", PaletteSelectNext, Some(context)));
        // 首/末/翻页：候选多起来（尤其是加了正文命中那一段）以后，光靠 ↑↓ 一条条走
        // 太慢。`home`/`end`/`pageup`/`pagedown` 在输入框里本来没有别的用途。
        keybindings.push(KeyBinding::new("home", PaletteSelectFirst, Some(context)));
        keybindings.push(KeyBinding::new("end", PaletteSelectLast, Some(context)));
        keybindings.push(KeyBinding::new(
            "pageup",
            PaletteSelectPageUp,
            Some(context),
        ));
        keybindings.push(KeyBinding::new(
            "pagedown",
            PaletteSelectPageDown,
            Some(context),
        ));
        keybindings.push(KeyBinding::new(
            "escape",
            CloseCommandPalette,
            Some(context),
        ));
    }
    keybindings
}

/// 面板里一条可选中项。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PaletteItem {
    pub label: String,
    /// 右侧补充说明：会话的来源 agent 或所在工作区目录名。
    pub detail: Option<String>,
    pub action: PaletteAction,
}

impl PaletteItem {
    fn command(label: String, detail: Option<String>, action: PaletteAction) -> Self {
        Self {
            label,
            detail,
            action,
        }
    }

    /// 正文命中的一条：标题为主、片段为说明。
    fn body_match(title: String, snippet: String, uid: String, query: String) -> Self {
        Self {
            label: title,
            detail: Some(snippet),
            action: PaletteAction::OpenSessionAtMatch { uid, query },
        }
    }
}

/// 正文命中项的入参（视图从存储层的检索结果映射过来）。
pub struct PaletteBodyHit {
    pub uid: String,
    pub title: String,
    pub snippet: String,
    /// 触发这次命中的查询词，随项一起带下去。
    pub query: String,
}

/// 选中一条之后要干的事。
///
/// 只描述意图、不带回调：面板是纯函数，怎么执行由视图决定。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PaletteAction {
    /// 新建会话。
    NewSession,
    /// 跳到某个会话（uid）。
    OpenSession(String),
    /// 打开用量历史浮层。
    OpenUsageHistory,
    /// 打开/关闭本会话内的搜索栏。
    ToggleFindInSession,
    /// 切换「显示已归档会话」。
    ToggleArchivedSessions,
    /// 跳到某个会话，且顺手把正文里那段查询词丢给会话内搜索。
    ///
    /// 带上 `query` 而不是只带 uid：正文命中只是「这条会话里大概有」，具体落在
    /// 哪儿由会话自己的搜索去定位并高亮——否则这里得额外维护一套消息坐标。
    OpenSessionAtMatch { uid: String, query: String },
}

/// 会话候选的入参（视图从会话摘要映射过来，模块不依赖视图类型）。
pub struct PaletteSession {
    pub uid: String,
    pub title: String,
    /// 来源 agent 短标识或工作区目录名。
    pub detail: Option<String>,
}

/// 固定命令项。
pub fn command_items(archived_visible: bool) -> Vec<PaletteItem> {
    vec![
        PaletteItem::command(
            t!("AgentUi.palette_new_session").to_string(),
            None,
            PaletteAction::NewSession,
        ),
        PaletteItem::command(
            t!("AgentUi.palette_find").to_string(),
            None,
            PaletteAction::ToggleFindInSession,
        ),
        PaletteItem::command(
            t!("AgentUi.palette_usage_history").to_string(),
            None,
            PaletteAction::OpenUsageHistory,
        ),
        PaletteItem::command(
            if archived_visible {
                t!("AgentUi.palette_hide_archived").to_string()
            } else {
                t!("AgentUi.palette_show_archived").to_string()
            },
            None,
            PaletteAction::ToggleArchivedSessions,
        ),
    ]
}

/// 会话候选项。
pub fn session_items(sessions: &[PaletteSession]) -> Vec<PaletteItem> {
    sessions
        .iter()
        .map(|session| PaletteItem {
            label: session.title.clone(),
            detail: session.detail.clone(),
            action: PaletteAction::OpenSession(session.uid.clone()),
        })
        .collect()
}

/// 正文命中候选项。
pub fn body_match_items(hits: &[PaletteBodyHit]) -> Vec<PaletteItem> {
    hits.iter()
        .map(|hit| {
            PaletteItem::body_match(
                hit.title.clone(),
                hit.snippet.clone(),
                hit.uid.clone(),
                hit.query.clone(),
            )
        })
        .collect()
}

/// 按查询过滤并截断到一次能显示的条数。
///
/// 匹配「命令与说明的任意子串」，且**保持原顺序**：命令在前、会话在后。
/// 这里刻意不做打分排序——顺序会随输入跳来跳去的话，眼睛刚锁定那一行就没了。
pub fn visible_items(items: &[PaletteItem], query: &str) -> Vec<PaletteItem> {
    let query = query.trim().to_lowercase();
    items
        .iter()
        .filter(|item| matches_query(item, &query))
        .take(PALETTE_MAX_ROWS)
        .cloned()
        .collect()
}

fn matches_query(item: &PaletteItem, query: &str) -> bool {
    if query.is_empty() {
        return true;
    }
    item.label.to_lowercase().contains(query)
        || item
            .detail
            .as_deref()
            .is_some_and(|detail| detail.to_lowercase().contains(query))
}

/// 上下移动高亮，两端绕回。
///
/// 绕回而不是夹住：列表短的时候，从最后一条按一下「下」回到第一条比「按了没反应」
/// 好判断——用户能确认键盘是活的。
pub fn move_selection(selected: usize, len: usize, delta: isize) -> usize {
    if len == 0 {
        return 0;
    }
    let len = len as isize;
    ((((selected as isize + delta) % len) + len) % len) as usize
}

/// 翻页步长：一屏的行数。
pub(crate) const PALETTE_PAGE_STEP: usize = PALETTE_MAX_ROWS;

/// 首项 / 末项的下标。列表为空时留在 0——否则 `len - 1` 会下溢。
pub fn edge_selection(len: usize, last: bool) -> usize {
    if len == 0 || !last { 0 } else { len - 1 }
}

/// 工作区目录名（会话条右侧展示用）。
///
/// 只取最后一段：整条绝对路径在窄面板里会被截成省略号，反而认不出来。
pub fn workspace_label(root: &str) -> Option<String> {
    let trimmed = root.trim().trim_end_matches(['/', '\\']);
    let name = trimmed.rsplit(['/', '\\']).next()?;
    (!name.is_empty()).then(|| name.to_string())
}

/// 命令面板浮层。
///
/// `query_input` 由视图传入（需要 `Input` 实体），`on_run` 拿到被选中的项。
pub fn render_command_palette(
    theme: &AgentChatTheme,
    items: &[PaletteItem],
    selected: usize,
    query_input: AnyElement,
    on_close: impl Fn(&MouseDownEvent, &mut Window, &mut App) + 'static,
    on_run: Rc<dyn Fn(&PaletteItem, &mut Window, &mut App)>,
) -> AnyElement {
    let mut rows: Vec<AnyElement> = items
        .iter()
        .enumerate()
        .map(|(index, item)| palette_row(theme, item, index, index == selected, on_run.clone()))
        .collect();
    if rows.is_empty() {
        rows.push(
            div()
                .debug_selector(|| "ai-chat-palette-empty".to_string())
                .px(sp(10.0))
                .py(sp(12.0))
                .text_sm()
                .text_color(theme.text_ghost)
                .child(t!("AgentUi.palette_empty").to_string())
                .into_any_element(),
        );
    }

    div()
        .id("ai-chat-palette-layer")
        .debug_selector(|| "ai-chat-palette-layer".to_string())
        .key_context(AI_CHAT_PALETTE_CONTEXT)
        .absolute()
        .inset_0()
        .occlude()
        .flex()
        .flex_col()
        .items_center()
        .pt(sp(PALETTE_TOP_INSET))
        .bg(theme.overlay_strong)
        .on_mouse_down(MouseButton::Left, on_close)
        .child(
            v_flex()
                .id("ai-chat-palette")
                .debug_selector(|| "ai-chat-palette".to_string())
                // 点面板本身（输入框、结果、提示条）不该被当成「点背板」关掉。
                .occlude()
                .w(px(PALETTE_WIDTH))
                .min_w_0()
                .rounded_lg()
                .border_1()
                .border_color(theme.border_strong)
                .bg(theme.overlay)
                .overflow_hidden()
                .child(query_input)
                .child(
                    v_flex()
                        .debug_selector(|| "ai-chat-palette-results".to_string())
                        .w_full()
                        .min_w_0()
                        .p(sp(4.0))
                        .gap(sp(1.0))
                        .children(rows),
                )
                .child(
                    div()
                        .debug_selector(|| "ai-chat-palette-hint".to_string())
                        .w_full()
                        .min_w_0()
                        .px(sp(10.0))
                        .py(sp(6.0))
                        .border_t_1()
                        .border_color(theme.border)
                        .text_xs()
                        .text_color(theme.text_ghost)
                        .child(t!("AgentUi.palette_hint").to_string()),
                ),
        )
        .into_any_element()
}

fn palette_row(
    theme: &AgentChatTheme,
    item: &PaletteItem,
    index: usize,
    selected: bool,
    on_run: Rc<dyn Fn(&PaletteItem, &mut Window, &mut App)>,
) -> AnyElement {
    let icon = match &item.action {
        PaletteAction::NewSession => IconName::Plus,
        PaletteAction::OpenSession(_) => IconName::Bot,
        PaletteAction::OpenUsageHistory => IconName::ChartPie,
        PaletteAction::ToggleFindInSession => IconName::Search,
        PaletteAction::ToggleArchivedSessions => IconName::Archive,
        // 正文命中用放大镜：一眼认出「这条是搜正文搜出来的」，而不是又一个会话。
        PaletteAction::OpenSessionAtMatch { .. } => IconName::Search,
    };
    let item_for_click = item.clone();

    h_flex()
        .id(SharedString::from(format!("ai-chat-palette-row-{index}")))
        .debug_selector(move || format!("ai-chat-palette-row-{index}"))
        .w_full()
        .min_w_0()
        .items_center()
        .gap(sp(8.0))
        .px(sp(8.0))
        .py(sp(6.0))
        .rounded_md()
        .cursor_pointer()
        .when(selected, |this| this.bg(theme.panel_hover))
        .hover(|style| style.bg(theme.panel_hover))
        .on_click(move |_, window, cx| on_run(&item_for_click, window, cx))
        .child(Icon::new(icon).xsmall().text_color(theme.muted_foreground))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .truncate()
                .text_sm()
                .text_color(theme.foreground)
                .child(item.label.clone()),
        )
        .when_some(item.detail.clone(), |this, detail| {
            this.child(
                div()
                    .flex_none()
                    .max_w(px(160.0))
                    .truncate()
                    .text_xs()
                    .text_color(theme.text_ghost)
                    .child(detail),
            )
        })
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(label: &str, detail: Option<&str>) -> PaletteItem {
        PaletteItem::command(
            label.to_string(),
            detail.map(str::to_string),
            PaletteAction::NewSession,
        )
    }

    fn labels(items: &[PaletteItem]) -> Vec<String> {
        items.iter().map(|item| item.label.clone()).collect()
    }

    #[test]
    fn filtering_is_case_insensitive_and_looks_at_the_detail_too() {
        let items = vec![
            item("New session", None),
            item("budget review", Some("codex")),
            item("release notes", Some("navop-wt-ai-workbench")),
        ];

        assert_eq!(
            vec!["budget review".to_string()],
            labels(&visible_items(&items, "BUDGET"))
        );
        // 说明文字也参与匹配：记得住 agent 名但记不住会话标题时照样找得到。
        assert_eq!(
            vec!["budget review".to_string()],
            labels(&visible_items(&items, "CODEX"))
        );
        assert_eq!(
            vec!["release notes".to_string()],
            labels(&visible_items(&items, "ai-workbench"))
        );
        assert!(visible_items(&items, "nothing-matches-this").is_empty());
    }

    #[test]
    fn filtering_keeps_the_source_order_and_truncates_to_the_row_cap() {
        let items: Vec<PaletteItem> = (0..PALETTE_MAX_ROWS + 4)
            .map(|index| item(&format!("command {index}"), None))
            .collect();

        // 空查询按原顺序列到行数上限，不是把列表整理成另一种顺序。
        let visible = visible_items(&items, "");
        assert_eq!(PALETTE_MAX_ROWS, visible.len());
        assert_eq!(vec!["command 0".to_string()], labels(&visible[..1]));

        // 前十个是 `command 0`…`command 9`，最后一个是 `command 10`：
        // 命中的两条都在上限之内，顺序与列表一致。
        let visible = visible_items(&items, "command 1");
        assert_eq!(
            vec!["command 1".to_string(), "command 10".to_string()],
            labels(&visible)
        );
    }

    #[test]
    fn selection_wraps_at_both_ends() {
        assert_eq!(1, move_selection(0, 4, 1));
        assert_eq!(0, move_selection(3, 4, 1));
        assert_eq!(3, move_selection(0, 4, -1));
        assert_eq!(2, move_selection(3, 4, -1));
        // 空列表上不动：没有可以指的东西。
        assert_eq!(0, move_selection(0, 0, 1));
        assert_eq!(0, move_selection(0, 0, -1));
    }

    #[test]
    fn body_hits_become_rows_that_carry_the_query_along() {
        let hits = vec![PaletteBodyHit {
            uid: "sess_a".to_string(),
            title: "预算审批".to_string(),
            snippet: "…这轮把预算压到 8 万…".to_string(),
            query: "预算".to_string(),
        }];

        let items = body_match_items(&hits);
        assert_eq!(1, items.len());
        assert_eq!("预算审批", items[0].label, "标题当主行");
        assert_eq!(
            Some("…这轮把预算压到 8 万…".to_string()),
            items[0].detail,
            "片段当说明——列表里就靠它认出是哪一句"
        );
        assert_eq!(
            PaletteAction::OpenSessionAtMatch {
                uid: "sess_a".to_string(),
                query: "预算".to_string(),
            },
            items[0].action,
            "查询词要跟着一起带下去，否则跳过去以后还得再打一遍"
        );
    }

    #[test]
    fn body_hit_rows_survive_the_query_filter() {
        let hits = vec![PaletteBodyHit {
            uid: "sess_a".to_string(),
            title: "一个标题里没有查询词的会话".to_string(),
            snippet: "…这里有预算两个字…".to_string(),
            query: "预算".to_string(),
        }];
        let items = body_match_items(&hits);

        // 主行不含查询词，靠片段通过过滤——不然命中会被自己的过滤器筛掉。
        assert_eq!(1, visible_items(&items, "预算").len());
    }

    #[test]
    fn edge_selection_clamps_to_the_list() {
        assert_eq!(0, edge_selection(5, false));
        assert_eq!(4, edge_selection(5, true));
        // 空列表：两个方向都留在 0，不做 `len - 1` 下溢。
        assert_eq!(0, edge_selection(0, false));
        assert_eq!(0, edge_selection(0, true));
    }

    #[test]
    fn workspace_label_keeps_only_the_last_segment() {
        assert_eq!(
            Some("navop-wt-ai-workbench".to_string()),
            workspace_label("/Users/hufei/RustroverProjects/navop-wt-ai-workbench")
        );
        // 尾随分隔符不该变成空名字。
        assert_eq!(Some("repo".to_string()), workspace_label("/home/me/repo/"));
        assert_eq!(None, workspace_label("/"));
        assert_eq!(None, workspace_label("   "));
    }

    #[test]
    fn sessions_become_open_actions() {
        let items = session_items(&[
            PaletteSession {
                uid: "s-1".into(),
                title: "预算审批".into(),
                detail: Some("codex".into()),
            },
            PaletteSession {
                uid: "s-2".into(),
                title: "发布说明".into(),
                detail: None,
            },
        ]);

        assert_eq!(
            vec!["预算审批".to_string(), "发布说明".to_string()],
            labels(&items)
        );
        assert_eq!(Some("codex".to_string()), items[0].detail);
        assert_eq!(
            PaletteAction::OpenSession("s-2".to_string()),
            items[1].action
        );
    }

    #[test]
    fn the_archive_command_follows_the_current_view() {
        let label_of = |items: &[PaletteItem]| {
            items
                .iter()
                .find(|item| item.action == PaletteAction::ToggleArchivedSessions)
                .expect("archive toggle command")
                .label
                .clone()
        };

        let showing = command_items(false);
        let hiding = command_items(true);

        assert_eq!(4, showing.len());
        // 同一条命令，文案随「已归档是否可见」翻面。
        assert_ne!(label_of(&showing), label_of(&hiding));
        // 其余命令不受影响。
        assert_eq!(showing[0], hiding[0]);
    }
}
