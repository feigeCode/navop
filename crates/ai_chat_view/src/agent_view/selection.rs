//! 转录块级选中：给渲染层的句柄、复制、以及选中后的操作条。
//!
//! 状态本身在 `crate::transcript_selection`（纯逻辑 + 测试），这里只负责把它与
//! 视图接起来：谁提供消息顺序、复制进剪贴板、操作条长什么样。

use super::*;
use gpui::AnyElement;

impl AgentChatView {
    /// 渲染层用的选区句柄。
    ///
    /// 顺序取**全部**消息 id：拖动跨越工具块时，区间里自然会包含它们，而复制时
    /// 会按「有没有可复制文本」跳过——与逐条复制的口径一致。
    pub(crate) fn transcript_selection_handle(
        &self,
        cx: &Context<Self>,
    ) -> TranscriptSelectionHandle {
        let order: Vec<String> = self
            .transcript
            .messages
            .iter()
            .map(|message| message.id.clone())
            .collect();
        let view = cx.entity().downgrade();
        let view_for_begin = view.clone();
        let focus = self.transcript_focus.clone();
        // 按下：收焦点（快捷键挂在根上下文，焦点留在 composer 里时 `Cmd+C` 会被
        // 编辑器吃掉）+ 重绘。
        let on_begin: Rc<dyn Fn(&mut Window, &mut App)> = Rc::new(move |window, cx| {
            focus.focus(window, cx);
            let _ = view_for_begin.update(cx, |_, cx| cx.notify());
        });
        // 拖动 / 松手：只要重绘。
        let on_change: Rc<dyn Fn(&mut App)> = Rc::new(move |cx| {
            let _ = view.update(cx, |_, cx| cx.notify());
        });
        TranscriptSelectionHandle::new(
            self.transcript_selection.clone(),
            order,
            on_begin,
            on_change,
        )
    }

    /// 选中的消息文本：与单条消息的复制按钮逐条拼起来完全一致（同一取值函数）。
    pub(crate) fn selected_transcript_text(&self) -> Option<String> {
        let order: Vec<String> = self
            .transcript
            .messages
            .iter()
            .map(|message| message.id.clone())
            .collect();
        let selected = self.transcript_selection.borrow().range(&order)?;
        let parts: Vec<String> = self.transcript.messages[selected.0..=selected.1]
            .iter()
            .filter_map(crate::message_view::message_copy_text)
            .map(|text| text.to_string())
            .collect();
        (!parts.is_empty()).then(|| parts.join("\n\n"))
    }

    /// 选区内可复制的消息条数（操作条上的「已选 N 条」）。
    fn selected_message_count(&self) -> usize {
        let order: Vec<String> = self
            .transcript
            .messages
            .iter()
            .map(|message| message.id.clone())
            .collect();
        let Some((start, end)) = self.transcript_selection.borrow().range(&order) else {
            return 0;
        };
        self.transcript.messages[start..=end]
            .iter()
            .filter(|message| crate::message_view::message_copy_text(message).is_some())
            .count()
    }

    /// 复制当前选区。返回是否有东西被复制（没有就交给别人处理这次快捷键）。
    pub(crate) fn copy_transcript_selection(&mut self, cx: &mut App) -> bool {
        let Some(text) = self.selected_transcript_text() else {
            return false;
        };
        cx.write_to_clipboard(ClipboardItem::new_string(text));
        true
    }

    /// 收起选区（`Esc` 或点操作条上的取消）。
    pub(crate) fn clear_transcript_selection(&mut self, cx: &mut Context<Self>) -> bool {
        if self.transcript_selection.borrow().is_empty() {
            return false;
        }
        self.transcript_selection.borrow_mut().clear();
        cx.notify();
        true
    }

    /// 选中后的操作条：告诉用户选了几条，以及两个明确动作。
    ///
    /// 不做「复制后自动收选区」：用户常要接着选下一段，选区留着更顺手，
    /// 收起来的入口是 `Esc` 与「取消」。
    pub(crate) fn render_selection_bar(
        &self,
        theme: &AgentChatTheme,
        cx: &Context<Self>,
    ) -> Option<AnyElement> {
        let count = self.selected_message_count();
        if count == 0 {
            return None;
        }
        // 按钮回调拿不到视图，用弱引用回访：操作条与快捷键走同一个入口，
        // 避免「按钮复制的内容」和「Cmd+C 复制的内容」两套口径。
        let view_for_copy = cx.entity().downgrade();
        let view_for_cancel = cx.entity().downgrade();
        Some(
            div()
                .debug_selector(|| "ai-chat-selection-bar".to_string())
                .absolute()
                .left_0()
                .right_0()
                .bottom(sp(16.0))
                .flex()
                .justify_center()
                .child(
                    h_flex()
                        .gap_2()
                        .items_center()
                        .px(sp(10.0))
                        .py(sp(6.0))
                        .rounded_md()
                        .bg(cx.theme().tokens.popover)
                        .border_1()
                        .border_color(theme.border)
                        .shadow_lg()
                        .child(div().text_sm().text_color(theme.muted_foreground).child(
                            t!("AgentUi.transcript_selection_count", count = count).to_string(),
                        ))
                        .child(
                            Button::new("ai-chat-selection-copy")
                                .label(t!("AgentUi.copy").to_string())
                                .small()
                                .on_click(move |_, _window, cx| {
                                    view_for_copy
                                        .update(cx, |this, cx| {
                                            this.copy_transcript_selection(cx);
                                        })
                                        .ok();
                                }),
                        )
                        .child(
                            Button::new("ai-chat-selection-cancel")
                                .label(t!("AgentUi.cancel").to_string())
                                .small()
                                .ghost()
                                .on_click(move |_, _window, cx| {
                                    view_for_cancel
                                        .update(cx, |this, cx| {
                                            this.clear_transcript_selection(cx);
                                        })
                                        .ok();
                                }),
                        ),
                )
                .into_any_element(),
        )
    }
}
