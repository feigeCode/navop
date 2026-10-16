//! Source-line navigation used by host-rendered document outlines.
//!
//! Ported from velotype #116 (`reveal_outline_heading`). Velotype's sidebar
//! outline lives in its own app workspace module, which navop's fork dropped;
//! this keeps the editor-side capability so a host can drive its own outline UI.

use super::Editor;
use gpui::Context;

impl Editor {
    /// Focuses and scrolls to the block that owns a heading's 0-based source line.
    ///
    /// The outline is parsed from the canonical Markdown serialization, whose
    /// line offsets align with the per-block source spans produced by
    /// [`Self::build_source_target_mappings`], so the byte offset of the requested
    /// line lands inside the owning block's full source range.
    pub fn reveal_outline_heading(&mut self, line: usize, cx: &mut Context<Self>) {
        let source = self.markdown(cx);
        let target = source
            .lines()
            .take(line)
            .map(|line| line.len() + 1)
            .sum::<usize>();

        let mappings = self.build_source_target_mappings(cx);
        let entity_id = mappings
            .iter()
            .find(|mapping| mapping.full_source_range.contains(&target))
            .or_else(|| {
                mappings.iter().min_by_key(|mapping| {
                    Self::source_offset_distance(&mapping.full_source_range, target)
                })
            })
            .map(|mapping| mapping.entity.entity_id());

        let Some(entity_id) = entity_id else {
            return;
        };
        let Some(block) = self.focusable_entity_by_id(entity_id) else {
            return;
        };
        self.focus_block_range(&block, 0..0, cx);
    }
}
