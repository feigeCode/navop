use anyhow::Result;
use gpui::{App, Context, Task, Window};
use instant::Duration;
use ropey::Rope;

use crate::input::{InputState, RopeExt, popovers::HoverPopover};
use sum_tree::Bias;

/// Hover provider
///
/// https://microsoft.github.io/language-server-protocol/specifications/lsp/3.17/specification/#textDocument_hover
pub trait HoverProvider {
    /// textDocument/hover
    ///
    /// https://microsoft.github.io/language-server-protocol/specifications/lsp/3.17/specification/#textDocument_hover
    fn hover(
        &self,
        _text: &Rope,
        _offset: usize,
        _window: &mut Window,
        _cx: &mut App,
    ) -> Task<Result<Option<lsp_types::Hover>>>;
}

impl InputState {
    /// Handle hover trigger LSP request.
    pub(super) fn handle_hover_popover(
        &mut self,
        offset: usize,
        window: &mut Window,
        cx: &mut Context<InputState>,
    ) {
        if self.selecting {
            return;
        }

        let offset = self
            .text
            .clip_offset(offset.min(self.text.len()), Bias::Left);
        let Some(provider) = self.lsp.hover_provider.clone() else {
            return;
        };

        if let Some(hover_popover) = self.hover_popover.as_ref() {
            if hover_popover.read(cx).is_same(offset) {
                return;
            }
        }

        let request_generation = self.lsp.next_hover_generation();
        let request_revision = self.document_revision;
        let task = provider.hover(&self.text, offset, window, cx);
        let mut symbol_range = self.text.word_range(offset).unwrap_or(offset..offset);
        let editor = cx.entity();
        let should_delay = self.hover_popover.is_none();
        self.lsp._hover_task = cx.spawn_in(window, async move |_, cx| {
            if should_delay {
                cx.background_executor()
                    .timer(Duration::from_millis(150))
                    .await;
            }

            let result = task.await?;

            _ = editor.update(cx, |editor, cx| {
                if !is_current_hover_request(
                    request_generation,
                    editor.lsp.hover_generation,
                    request_revision,
                    editor.document_revision,
                ) {
                    return;
                }

                match result {
                    Some(hover) => {
                        if let Some(range) = hover.range {
                            let start = editor.text.position_to_offset(&range.start);
                            let end = editor.text.position_to_offset(&range.end);
                            symbol_range = start..end;
                        }
                        let hover_popover =
                            HoverPopover::new(cx.entity(), symbol_range, &hover, cx);
                        editor.hover_popover = Some(hover_popover);
                    }
                    None => {
                        editor.hover_popover = None;
                    }
                }
            });

            Ok(())
        });
    }
}

fn is_current_hover_request(
    expected_generation: u64,
    current_generation: u64,
    expected_revision: u64,
    current_revision: u64,
) -> bool {
    expected_generation == current_generation && expected_revision == current_revision
}

#[cfg(test)]
mod tests {
    use super::is_current_hover_request;

    #[test]
    fn stale_hover_identity_is_rejected() {
        assert!(is_current_hover_request(2, 2, 7, 7));
        assert!(!is_current_hover_request(1, 2, 7, 7));
        assert!(!is_current_hover_request(2, 2, 6, 7));
    }
}
