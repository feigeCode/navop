//! Inline footnote references and resolved footnote registry state.

use std::collections::HashMap;

use gpui::EntityId;
use uuid::Uuid;

/// Inline footnote reference parsed from `[^id]` syntax.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InlineFootnoteReference {
    /// Footnote identifier without the `[^` and `]` markers.
    pub id: String,
    /// Resolved document ordinal, if the referenced definition exists.
    pub ordinal: Option<usize>,
    /// Zero-based occurrence count within the block.
    pub occurrence_index: usize,
}

/// Hit-test payload for a rendered footnote reference.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InlineFootnoteHit {
    pub id: String,
    pub ordinal: usize,
    pub occurrence_index: usize,
}

impl InlineFootnoteReference {
    pub(crate) fn raw_markdown(&self) -> String {
        format!("[^{}]", self.id)
    }

    pub(crate) fn hit(&self) -> Option<InlineFootnoteHit> {
        Some(InlineFootnoteHit {
            id: self.id.clone(),
            ordinal: self.ordinal?,
            occurrence_index: self.occurrence_index,
        })
    }
}

/// Location of the first resolved inline reference for one footnote id.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct FootnoteReferenceLocation {
    pub(crate) entity_id: EntityId,
    pub(crate) occurrence_index: usize,
}

/// Definition block and first-reference metadata for one footnote id.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct FootnoteDefinitionBinding {
    pub(crate) ordinal: Option<usize>,
    pub(crate) definition_entity_id: EntityId,
    pub(crate) first_reference: Option<FootnoteReferenceLocation>,
}

/// Document-wide registry that binds references to definition blocks.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct FootnoteRegistry {
    pub(crate) bindings: HashMap<String, FootnoteDefinitionBinding>,
    pub(crate) block_occurrences: HashMap<Uuid, Vec<FootnoteResolvedOccurrence>>,
}

impl FootnoteRegistry {
    pub(crate) fn binding(&self, id: &str) -> Option<&FootnoteDefinitionBinding> {
        self.bindings.get(id)
    }

    pub(crate) fn ordinal(&self, id: &str) -> Option<usize> {
        self.binding(id).and_then(|binding| binding.ordinal)
    }

    pub(crate) fn occurrences_for_block(
        &self,
        block_id: Uuid,
    ) -> Option<&[FootnoteResolvedOccurrence]> {
        self.block_occurrences.get(&block_id).map(Vec::as_slice)
    }
}

/// Resolved occurrence stored per block for inline rendering.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct FootnoteResolvedOccurrence {
    pub(crate) id: String,
    pub(crate) ordinal: Option<usize>,
    pub(crate) occurrence_index: usize,
}

pub(crate) fn is_valid_footnote_id(id: &str) -> bool {
    !id.is_empty()
        && !id
            .chars()
            .any(|ch| matches!(ch, ' ' | '\t' | '\n' | '\r' | '^' | '[' | ']'))
}

pub(crate) fn parse_footnote_definition_head(line: &str) -> Option<(String, String)> {
    let trimmed_end = line.trim_end();
    let leading_spaces = trimmed_end.bytes().take_while(|b| *b == b' ').count();
    if leading_spaces > 3 {
        return None;
    }

    let rest = &trimmed_end[leading_spaces..];
    let after_open = rest.strip_prefix("[^")?;
    let label_end = after_open.find("]:")?;
    let id = after_open[..label_end].to_string();
    if !is_valid_footnote_id(&id) {
        return None;
    }

    let remainder = after_open[label_end + 2..]
        .strip_prefix(' ')
        .unwrap_or(&after_open[label_end + 2..])
        .to_string();
    Some((id, remainder))
}

pub(crate) fn parse_inline_footnote_reference(markdown: &str) -> Option<String> {
    let rest = markdown.strip_prefix("[^")?;
    let id = rest.strip_suffix(']')?;
    is_valid_footnote_id(id).then(|| id.to_string())
}

pub(crate) fn superscript_ordinal(ordinal: usize) -> String {
    const SUPERSCRIPT_DIGITS: [char; 10] = [
        '\u{2070}', '\u{00B9}', '\u{00B2}', '\u{00B3}', '\u{2074}', '\u{2075}', '\u{2076}',
        '\u{2077}', '\u{2078}', '\u{2079}',
    ];

    ordinal
        .to_string()
        .chars()
        .filter_map(|ch| {
            ch.to_digit(10)
                .map(|digit| SUPERSCRIPT_DIGITS[digit as usize])
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::{
        is_valid_footnote_id, parse_footnote_definition_head, parse_inline_footnote_reference,
        superscript_ordinal,
    };

    #[test]
    fn validates_and_parses_reference_footnote_syntax() {
        assert!(is_valid_footnote_id("long-note"));
        assert!(!is_valid_footnote_id("bad id"));
        assert_eq!(
            parse_inline_footnote_reference("[^ref-1]"),
            Some("ref-1".to_string())
        );
        assert_eq!(
            parse_footnote_definition_head("[^ref-1]: body"),
            Some(("ref-1".to_string(), "body".to_string()))
        );
    }

    #[test]
    fn formats_superscript_ordinals() {
        assert_eq!(superscript_ordinal(1), "\u{00B9}");
        assert_eq!(superscript_ordinal(12), "\u{00B9}\u{00B2}");
    }
}
