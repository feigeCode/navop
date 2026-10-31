//! Tab 恢复契约：把会话快照里的 `TabItemState` 还原成可渲染的视图。
//!
//! 快照本身由 `one_core::tab_container` 定义（`TabContainer::dump` /
//! `TabContainerState`），`app_kit::tabs` 负责落盘与重启后重建；各领域只
//! 需要声明自己能恢复哪些 content key，以及如何把一条快照还原成视图。

use gpui::{App, Window};
use one_core::tab_container::{TabContentView, TabItemState};
use std::sync::Arc;

/// 单个领域的 Tab 恢复器。
///
/// `app_kit::tabs` 启动时给每个 `content_keys()` 在
/// `one_core::tab_container::TabContentRegistry` 里登记一个构造器，
/// 恢复快照时按 `TabItemState::key` 分派回这里。
pub trait TabRestorer: Send + Sync {
    /// 本领域支持的 content key，取值与 [`TabContentView::content_key`] 一致
    /// （如 `Terminal`、`SFTP`、`Database`）。
    fn content_keys(&self) -> Vec<&'static str>;

    /// 由快照重建单个视图。
    ///
    /// 快照里引用的资源（连接、工作区）已被删除时返回 `None`，该标签会被
    /// 静默跳过，不影响其它标签的恢复。
    fn restore(
        &self,
        state: &TabItemState,
        window: &mut Window,
        cx: &mut App,
    ) -> Option<Arc<dyn TabContentView>>;
}
