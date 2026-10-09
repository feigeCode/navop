//! 转录里消息图的放大预览。
//!
//! 与其他覆盖层一样挂在**面板根部**：转录区是滚动容器，绝对定位的子元素
//! 会被裁在滚动视口内，还可能被后续内容盖住。
//!
//! 状态只存一张 `MessageImage`（`Arc<Image>` + 文件名），不做「按 id 反查
//! 转录」：会话切换、消息被淘汰都会让反查落空，表现为「点了没反应」。

use gpui::{AnyElement, ClickEvent, MouseDownEvent, WeakEntity};

use super::*;

impl AgentChatView {
    /// 转录消息图预览覆盖层；没有要放的图时返回 `None`。
    pub(super) fn render_message_image_preview(
        &self,
        theme: &AgentChatTheme,
        view: Entity<Self>,
    ) -> Option<AnyElement> {
        let preview = self.message_image_preview.clone()?;
        // 关闭回调要能反查本视图：拿弱引用，免得覆盖层把面板钉在内存里。
        let weak = view.downgrade();

        let backdrop = {
            let weak = weak.clone();
            move |_: &MouseDownEvent, _: &mut Window, cx: &mut App| {
                close_message_image_preview(&weak, cx);
            }
        };
        let button = {
            let weak = weak.clone();
            move |_: &ClickEvent, _: &mut Window, cx: &mut App| {
                close_message_image_preview(&weak, cx);
            }
        };

        Some(super::attachment_preview::image_preview_overlay(
            "ai-chat-image-preview",
            preview.image.clone(),
            preview.name.map(Into::into),
            theme,
            backdrop,
            button,
        ))
    }
}

fn close_message_image_preview(view: &WeakEntity<AgentChatView>, cx: &mut App) {
    let _ = view.update(cx, |this, cx| this.close_message_image_preview(cx));
}
