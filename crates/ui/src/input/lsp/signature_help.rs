//! Signature help host (LSP `textDocument/signatureHelp`)
//!
//! The host is generic: it renders whatever the [`SignatureHelpProvider`]
//! returns (protocol [`lsp_types::SignatureHelp`]) at the cursor. SQL routines
//! and overload resolution live in the provider, not here.

use anyhow::Result;
use gpui::{App, Context, Task, Window};
use ropey::Rope;

use crate::input::{InputState, popovers::SignatureHelpPopover};

/// Signature help provider
///
/// https://microsoft.github.io/language-server-protocol/specifications/lsp/3.17/specification/#textDocument_signatureHelp
pub trait SignatureHelpProvider {
    /// Resolve signature help at the given byte `offset`.
    ///
    /// Return `None` when the cursor is not inside a function call's argument
    /// list; the host then closes the popover.
    ///
    /// textDocument/signatureHelp
    ///
    /// https://microsoft.github.io/language-server-protocol/specifications/lsp/3.17/specification/#textDocument_signatureHelp
    fn signature_help(
        &self,
        text: &Rope,
        offset: usize,
        window: &mut Window,
        cx: &mut App,
    ) -> Task<Result<Option<lsp_types::SignatureHelp>>>;
}

/// Decide whether an edit should refresh signature help.
///
/// Typing `(` or `,` likely enters or advances a function call. Otherwise the
/// popover is only refreshed when it is already open, so ordinary typing never
/// spawns useless requests outside a call.
fn signature_help_edit_action(new_text: &str, popover_open: bool) -> bool {
    let typing_in_call = new_text.chars().any(|c| matches!(c, '(' | ','));
    typing_in_call || popover_open
}

impl InputState {
    /// Handle a document edit for signature help.
    pub(crate) fn handle_signature_help_edit(
        &mut self,
        new_text: &str,
        window: &mut Window,
        cx: &mut Context<InputState>,
    ) {
        if !signature_help_edit_action(new_text, self.signature_help_popover.is_some()) {
            return;
        }
        self.refresh_signature_help(window, cx);
    }

    /// Close the signature help popover and invalidate any in-flight request.
    pub fn close_signature_help(&mut self, cx: &mut Context<Self>) {
        self.signature_help_popover = None;
        self.last_signature_help_request = None;
        self.lsp.invalidate_signature_help();
        cx.notify();
    }

    /// Re-request signature help at the current cursor position.
    ///
    /// The provider decides whether the cursor is inside a call; an empty
    /// response closes the popover. Stale responses (from an older request or
    /// document revision) are dropped via the generation + revision guard
    /// (spec §25.2).
    pub fn refresh_signature_help(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(provider) = self.lsp.signature_help_provider.clone() else {
            self.signature_help_popover = None;
            return;
        };

        let offset = self.cursor();
        let revision = self.document_revision;
        if self.last_signature_help_request == Some((offset, revision)) {
            return;
        }
        self.last_signature_help_request = Some((offset, revision));

        let request_generation = self.lsp.next_signature_help_generation();
        let request_revision = revision;
        let task = provider.signature_help(&self.text, offset, window, cx);
        let editor = cx.entity();

        self.lsp._signature_help_task = cx.spawn_in(window, async move |_, cx| {
            let result = task.await.ok().flatten();
            let _ = editor.update(cx, |editor, cx| {
                if !is_current_signature_help_request(
                    request_generation,
                    editor.lsp.signature_help_generation,
                    request_revision,
                    editor.document_revision,
                ) {
                    return;
                }

                editor.signature_help_popover = result.and_then(|help| {
                    (!help.signatures.is_empty())
                        .then(|| SignatureHelpPopover::new(cx.entity(), offset, help, cx))
                });
                cx.notify();
            });
        });
    }

    /// Cycle the highlighted overload when multiple signatures exist.
    pub fn cycle_signature_help(&mut self, delta: isize, cx: &mut Context<Self>) {
        if let Some(popover) = self.signature_help_popover.as_mut() {
            popover.update(cx, |popover, cx| {
                popover.cycle_signature(delta);
                cx.notify();
            });
        }
    }
}

/// Reject signature help responses that belong to an older request or document.
fn is_current_signature_help_request(
    expected_generation: u64,
    current_generation: u64,
    expected_revision: u64,
    current_revision: u64,
) -> bool {
    expected_generation == current_generation && expected_revision == current_revision
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stale_signature_help_request_is_rejected() {
        assert!(is_current_signature_help_request(2, 2, 7, 7));
        assert!(!is_current_signature_help_request(1, 2, 7, 7));
        assert!(!is_current_signature_help_request(2, 2, 6, 7));
    }

    #[test]
    fn call_characters_refresh_signature_help() {
        for text in ["(", ",", "foo(", "foo(a,"] {
            assert!(signature_help_edit_action(text, false), "{text:?}");
        }
    }

    #[test]
    fn open_popover_keeps_refreshing_on_plain_edits() {
        assert!(signature_help_edit_action("x", true));
        assert!(signature_help_edit_action("=", true));
    }

    #[test]
    fn plain_edits_do_not_spawn_requests_when_closed() {
        for text in ["x", "=", "1", "\n", "SELECT"] {
            assert!(!signature_help_edit_action(text, false), "{text:?}");
        }
    }
}