//! 转录里的块级选中与复制。
//!
//! waku 的转录支持拖过一段文字再 `Cmd+C`。我们的正文是整块渲染的
//! （`TextView` / Markdown 产物），**没有**可靠的「字符 → 消息」映射，
//! 硬做字符级选区等于自研一套渲染器。所以这里取的是能诚实做到的那一档：
//! **消息块粒度**——拖过哪几条消息就选中哪几条，复制内容与逐条点「复制」
//! 拼起来完全一致（同一个取值函数），不会出现两套复制口径。
//!
//! 状态放在 [`TranscriptSelection`] 里，渲染层拿 [`TranscriptSelectionHandle`]
//! 挂鼠标事件并读「这一条是否在选区内」；谁提供顺序、复制到剪贴板由视图负责。

use std::cell::RefCell;
use std::rc::Rc;

use gpui::{App, KeyBinding, Window};

use crate::find_shortcut::AI_CHAT_SEARCH_CONTEXT;

gpui::actions!(
    ai_chat_transcript_selection,
    [CopyTranscriptSelection, ClearTranscriptSelection]
);

/// 注册快捷键。与搜索共用面板根节点的上下文（见 `find_shortcut` 的说明）。
///
/// 只绑根上下文、**不**绑 `AiChatTranscript > Input`：那条会把 composer 自己的
/// `Cmd+C` 压住，用户选中输入框里的文字再复制就失灵了。代价是得在按下鼠标选中转录
/// 时把焦点交给转录（见 `AgentChatView::begin_transcript_selection`），换来两条
/// 复制路径互不打扰。
pub fn init(cx: &mut App) {
    cx.bind_keys(keybindings());
}

fn keybindings() -> Vec<KeyBinding> {
    let copy = [
        KeyBinding::new(
            "secondary-c",
            CopyTranscriptSelection,
            Some(AI_CHAT_SEARCH_CONTEXT),
        ),
        KeyBinding::new(
            "ctrl-c",
            CopyTranscriptSelection,
            Some(AI_CHAT_SEARCH_CONTEXT),
        ),
    ];
    let mut bindings = copy.to_vec();
    // 取消选中：有选区时才处理，没有就 `propagate`，不挡住面板里其他 Esc 语义
    // （findbar 自有关闭键、附件预览走 `AiChatFindbar` 那类更深的上下文）。
    bindings.push(KeyBinding::new(
        "escape",
        ClearTranscriptSelection,
        Some(AI_CHAT_SEARCH_CONTEXT),
    ));
    bindings
}

/// 一次块级选中：锚点 + 当前头，都是消息 id。
///
/// 用 id 而不是下标：流式追加、工具块合并在渲染间会改变下标，id 不会。
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TranscriptSelection {
    anchor: Option<String>,
    head: Option<String>,
    /// 正在拖拽；为 false 时 [`Self::extend`] 是 no-op（避免残留的 hover 事件乱改选区）。
    dragging: bool,
}

impl TranscriptSelection {
    pub fn is_empty(&self) -> bool {
        self.anchor.is_none()
    }

    pub fn is_dragging(&self) -> bool {
        self.dragging
    }

    /// 按下左键：从这条消息重新开一段。
    pub fn begin(&mut self, id: &str) {
        self.anchor = Some(id.to_string());
        self.head = Some(id.to_string());
        self.dragging = true;
    }

    /// 拖到这条消息：只在拖拽中生效。
    pub fn extend(&mut self, id: &str) {
        if self.dragging {
            self.head = Some(id.to_string());
        }
    }

    /// 松开左键：这次拖拽结束，选区留着（复制按钮还要用）。
    pub fn finish(&mut self) {
        self.dragging = false;
    }

    pub fn clear(&mut self) {
        *self = Self::default();
    }

    /// 选中的下标区间（含端点，方向无关）。
    ///
    /// 端点里有一个不在当前顺序里（那条消息被删了 / 还没渲染）就返回 `None`：
    /// 半截选区复制出来的内容是错的，宁可不给。
    pub fn range(&self, order: &[String]) -> Option<(usize, usize)> {
        let anchor = self.anchor.as_deref()?;
        let head = self.head.as_deref()?;
        let anchor = order.iter().position(|id| id == anchor)?;
        let head = order.iter().position(|id| id == head)?;
        Some((anchor.min(head), anchor.max(head)))
    }

    /// 这条消息在选区内吗（渲染时用来决定要不要铺底色）。
    pub fn contains(&self, id: &str, order: &[String]) -> bool {
        let Some((start, end)) = self.range(order) else {
            return false;
        };
        order
            .iter()
            .position(|candidate| candidate == id)
            .is_some_and(|index| index >= start && index <= end)
    }

    /// 选区内消息的条数；没有选区时 0。
    pub fn count(&self, order: &[String]) -> usize {
        self.range(order)
            .map(|(start, end)| end - start + 1)
            .unwrap_or(0)
    }
}

/// 交给渲染层的选区句柄：状态 + 当前消息顺序 + 「变了请重绘」。
///
/// 渲染层只有 `&mut App`，没法自己 notify 视图；回调由视图提供，这样渲染层不必
/// 认识视图类型（`message_turn_view` 是公开渲染入口，不该反向依赖 `AgentChatView`）。
#[derive(Clone)]
pub struct TranscriptSelectionHandle {
    state: Rc<RefCell<TranscriptSelection>>,
    order: Rc<Vec<String>>,
    /// 开一段新选中时的回调：宿主借此把焦点收到转录上（快捷键挂在根上下文，
    /// 焦点还在 composer 里的话 `Cmd+C` 会被编辑器吃掉）。
    on_begin: Rc<dyn Fn(&mut Window, &mut App)>,
    on_change: Rc<dyn Fn(&mut App)>,
}

impl TranscriptSelectionHandle {
    pub fn new(
        state: Rc<RefCell<TranscriptSelection>>,
        order: Vec<String>,
        on_begin: Rc<dyn Fn(&mut Window, &mut App)>,
        on_change: Rc<dyn Fn(&mut App)>,
    ) -> Self {
        Self {
            state,
            order: Rc::new(order),
            on_begin,
            on_change,
        }
    }

    pub fn contains(&self, id: &str) -> bool {
        self.state.borrow().contains(id, &self.order)
    }

    pub fn begin(&self, id: &str, window: &mut Window, cx: &mut App) {
        self.state.borrow_mut().begin(id);
        (self.on_begin)(window, cx);
    }

    pub fn extend(&self, id: &str, cx: &mut App) {
        let mut state = self.state.borrow_mut();
        if !state.is_dragging() {
            return;
        }
        state.extend(id);
        drop(state);
        (self.on_change)(cx);
    }

    pub fn finish(&self, cx: &mut App) {
        self.state.borrow_mut().finish();
        (self.on_change)(cx);
    }

    /// 选中的消息 id，按当前顺序。
    pub fn selected_ids(&self) -> Vec<String> {
        let state = self.state.borrow();
        let Some((start, end)) = state.range(&self.order) else {
            return Vec::new();
        };
        self.order[start..=end].to_vec()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn order(ids: &[&str]) -> Vec<String> {
        ids.iter().map(|id| id.to_string()).collect()
    }

    #[test]
    fn dragging_normalizes_direction() {
        let ids = order(&["a", "b", "c", "d"]);
        let mut selection = TranscriptSelection::default();
        assert!(selection.is_empty());

        selection.begin("c");
        assert_eq!(Some((2, 2)), selection.range(&ids));
        selection.extend("a");
        assert_eq!(Some((0, 2)), selection.range(&ids), "反向拖拽要归一化");
        assert_eq!(3, selection.count(&ids));
        assert!(selection.contains("b", &ids));
        assert!(!selection.contains("d", &ids));

        selection.finish();
        selection.extend("d");
        assert_eq!(Some((0, 2)), selection.range(&ids), "松手后不再跟着扩张");
    }

    #[test]
    fn extend_without_begin_is_ignored() {
        let ids = order(&["a", "b"]);
        let mut selection = TranscriptSelection::default();
        selection.extend("b");
        assert!(selection.is_empty(), "没按下左键就没有选区");
        assert_eq!(None, selection.range(&ids));
    }

    #[test]
    fn unknown_endpoint_drops_the_range() {
        let ids = order(&["a", "b"]);
        let mut selection = TranscriptSelection::default();
        selection.begin("a");
        selection.extend("gone");
        assert_eq!(
            None,
            selection.range(&ids),
            "端点不在顺序里时宁可不给区间，也别复制半截"
        );
        assert!(!selection.contains("a", &ids));
        assert_eq!(0, selection.count(&ids));
    }

    #[test]
    fn clear_resets_to_empty() {
        let ids = order(&["a", "b"]);
        let mut selection = TranscriptSelection::default();
        selection.begin("a");
        selection.extend("b");
        selection.clear();
        assert!(selection.is_empty());
        assert_eq!(None, selection.range(&ids));
    }
}
