//! Rendered standalone image runtime state.

use super::*;

impl Block {
    pub(crate) fn image_runtime(&self) -> Option<&ImageRuntime> {
        self.image_runtime.as_ref()
    }

    pub(super) fn can_present_as_image(&self) -> bool {
        self.is_table_cell()
            || matches!(
                self.kind(),
                BlockKind::Paragraph
                    | BlockKind::BulletedListItem
                    | BlockKind::NumberedListItem
                    | BlockKind::TaskListItem { .. }
            )
    }

    /// Whether this block's text is a lone image that renders as a
    /// self-contained image widget. Unlike `showing_rendered_image`, this is
    /// derived from the title text rather than the computed runtime, so it is
    /// valid before image runtimes are (re)built.
    pub(crate) fn renders_as_standalone_image(&self) -> bool {
        self.can_present_as_image() && self.standalone_image_markdown_for_runtime().is_some()
    }

    pub(super) fn compute_image_runtime(
        &self,
        base_dir: Option<&Path>,
        syntax: ImageSyntax,
    ) -> Option<ImageRuntime> {
        let resolved_target = syntax.resolve_target(&self.image_reference_definitions)?;
        self.can_present_as_image().then(|| ImageRuntime {
            alt: syntax.alt.clone(),
            src: resolved_target.src.clone(),
            title: resolved_target.title.clone(),
            resolved_source: resolve_image_source(&resolved_target.src, base_dir),
        })
    }

    pub(crate) fn image_runtime_for_syntax(&self, syntax: ImageSyntax) -> Option<ImageRuntime> {
        self.compute_image_runtime(self.image_base_dir.as_deref(), syntax)
    }

    pub(crate) fn image_base_dir(&self) -> Option<&Path> {
        self.image_base_dir.as_deref()
    }

    pub(super) fn sync_image_runtime(&mut self) {
        let next_runtime = if self.can_present_as_image() {
            self.standalone_image_markdown_for_runtime()
                .and_then(|markdown| parse_standalone_image(&markdown))
                .and_then(|syntax| {
                    self.compute_image_runtime(self.image_base_dir.as_deref(), syntax)
                })
        } else {
            None
        };

        if next_runtime.is_none() {
            self.image_edit_expanded = false;
            self.image_expand_requested = false;
        }
        self.image_runtime = next_runtime;
    }

    fn standalone_image_markdown_for_runtime(&self) -> Option<String> {
        let visible = self.record.title.visible_text();
        if parse_standalone_image(&visible).is_some() {
            return Some(visible);
        }

        let serialized = self.record.title.serialize_markdown();
        parse_standalone_image(&serialized)
            .is_some()
            .then_some(serialized)
    }

    pub(crate) fn request_image_edit_expansion(&mut self) {
        if self.image_runtime.is_some() {
            self.image_expand_requested = true;
        }
    }

    pub(super) fn consume_requested_image_edit_expansion(&mut self) -> bool {
        if self.image_runtime.is_some() && self.image_expand_requested && !self.image_edit_expanded
        {
            self.image_expand_requested = false;
            self.image_edit_expanded = true;
            self.clear_inline_projection();
            self.assign_collapsed_selection_offset(
                self.visible_len(),
                CollapsedCaretAffinity::Default,
                None,
            );
            self.cursor_blink_epoch = Instant::now();
            self.clear_vertical_motion();
            return true;
        }

        false
    }

    pub(crate) fn sync_image_focus_state(&mut self, focused: bool) -> bool {
        if self.image_runtime.is_none() {
            if self.image_edit_expanded || self.image_expand_requested {
                self.image_edit_expanded = false;
                self.image_expand_requested = false;
                self.clear_inline_projection();
                return true;
            }
            return false;
        }

        if focused {
            return self.consume_requested_image_edit_expansion();
        }

        if self.image_edit_expanded {
            self.image_edit_expanded = false;
            self.clear_inline_projection();
            return true;
        }

        false
    }

    pub(crate) fn showing_rendered_image(&self) -> bool {
        self.image_runtime.is_some() && !self.is_source_raw_mode() && !self.image_edit_expanded
    }
}
