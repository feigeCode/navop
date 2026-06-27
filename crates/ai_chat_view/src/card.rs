//! 聊天卡片渲染机制:通用、可扩展的自定义卡片注册表。
//!
//! 这是 `ai_chat_view` 的核心扩展点。任意业务模块(数据库 / SSH / 监控 ...)
//! 通过实现 [`ChatCard`] 并注册到全局 [`CardRegistry`],即可让聊天消息以
//! 自定义方式渲染:当一条消息携带 `MessageVariant::Card { kind }` 时,消息
//! 列表会按 `kind` 在注册表中查找对应卡片来渲染。
//!
//! 设计参考项目既有的 `TabContentRegistry`(按 key 注册渲染器 + GPUI global)。
//! 本机制完全不绑定任何具体业务,仅以字符串 `kind` 作为分发依据。

use gpui::{AnyElement, App, Window};
use std::collections::HashMap;
use std::sync::Arc;

/// 渲染卡片时传入的消息只读快照。
///
/// 卡片实现可从 `content` 解析自身所需数据(如 JSON),或用 `id` 作为键去
/// 自己维护的 per-message 状态(例如 `HashMap<id, Entity<...>>`)里取有状态视图。
pub struct CardMessage<'a> {
    /// 消息唯一标识。
    pub id: &'a str,
    /// 卡片类型标识,等于 `MessageVariant::Card { kind }` 中的 `kind`。
    pub kind: &'a str,
    /// 消息内容,作为卡片数据载体(可为纯文本 / JSON 等)。
    pub content: &'a str,
    /// 是否仍在流式输出。
    pub is_streaming: bool,
}

/// 卡片渲染器:各业务模块实现自己的卡片类型。
pub trait ChatCard: Send + Sync + 'static {
    /// 卡片类型标识,需与 `MessageVariant::Card { kind }` 一致。
    fn kind(&self) -> &'static str;

    /// 把卡片消息渲染为元素。
    fn render(&self, msg: &CardMessage, window: &mut Window, cx: &mut App) -> AnyElement;
}

/// 卡片注册表(GPUI global)。
///
/// 以 `kind` 为键保存卡片渲染器。各模块在自身 `init` 阶段通过
/// [`CardRegistry::register_global`] 注册。
#[derive(Clone, Default)]
pub struct CardRegistry {
    cards: HashMap<String, Arc<dyn ChatCard>>,
}

impl gpui::Global for CardRegistry {}

impl CardRegistry {
    /// 创建空注册表。
    pub fn new() -> Self {
        Self {
            cards: HashMap::new(),
        }
    }

    /// 注册一个卡片渲染器,按其 [`ChatCard::kind`] 索引;同 `kind` 重复注册会覆盖。
    pub fn register(&mut self, card: Arc<dyn ChatCard>) {
        self.cards.insert(card.kind().to_string(), card);
    }

    /// 按 `kind` 取卡片渲染器(返回克隆的 `Arc`,以便随后独占借用 `App`)。
    pub fn get(&self, kind: &str) -> Option<Arc<dyn ChatCard>> {
        self.cards.get(kind).cloned()
    }

    /// 是否已注册某 `kind`。
    pub fn has(&self, kind: &str) -> bool {
        self.cards.contains_key(kind)
    }

    /// 已注册卡片数量。
    pub fn len(&self) -> usize {
        self.cards.len()
    }

    /// 注册表是否为空。
    pub fn is_empty(&self) -> bool {
        self.cards.is_empty()
    }

    // ===== GPUI global 存取 =====

    /// 确保全局注册表已初始化(幂等)。
    pub fn init_global(cx: &mut App) {
        if !cx.has_global::<CardRegistry>() {
            cx.set_global(CardRegistry::new());
        }
    }

    /// 读取全局注册表(需先 [`CardRegistry::init_global`])。
    pub fn global(cx: &App) -> &CardRegistry {
        cx.global::<CardRegistry>()
    }

    /// 向全局注册表注册卡片(自动初始化全局表)。
    pub fn register_global(cx: &mut App, card: Arc<dyn ChatCard>) {
        Self::init_global(cx);
        cx.global_mut::<CardRegistry>().register(card);
    }

    /// 按 `msg.kind` 从全局注册表分发渲染;未注册时返回 `None`,由调用方回退占位符。
    ///
    /// 先取出卡片的 `Arc`(结束对 `App` 的不可变借用),再以 `&mut App` 渲染,
    /// 从而避免「读注册表」与「渲染需可变 `App`」之间的借用冲突。
    pub fn render_global(
        msg: &CardMessage,
        window: &mut Window,
        cx: &mut App,
    ) -> Option<AnyElement> {
        let card = cx.try_global::<CardRegistry>()?.get(msg.kind)?;
        Some(card.render(msg, window, cx))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::{IntoElement, div};

    struct DummyCard;
    impl ChatCard for DummyCard {
        fn kind(&self) -> &'static str {
            "dummy"
        }
        fn render(&self, _msg: &CardMessage, _window: &mut Window, _cx: &mut App) -> AnyElement {
            // 测试只验证注册/查找逻辑,render 不会被实际调用。
            div().into_any_element()
        }
    }

    #[test]
    fn register_and_lookup() {
        let mut reg = CardRegistry::new();
        assert!(reg.is_empty());
        assert!(!reg.has("dummy"));

        reg.register(Arc::new(DummyCard));

        assert!(reg.has("dummy"));
        assert!(reg.get("dummy").is_some());
        assert_eq!(reg.len(), 1);
    }

    #[test]
    fn unknown_kind_returns_none() {
        let reg = CardRegistry::new();
        assert!(reg.get("does-not-exist").is_none());
    }

    #[test]
    fn register_same_kind_overwrites() {
        let mut reg = CardRegistry::new();
        reg.register(Arc::new(DummyCard));
        reg.register(Arc::new(DummyCard));
        assert_eq!(reg.len(), 1, "同 kind 重复注册应覆盖而非新增");
    }
}
