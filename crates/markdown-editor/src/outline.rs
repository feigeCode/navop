//! Fence-aware ATX heading outline extracted from a document's Markdown source.
//!
//! Ported from velotype's workspace outline builder (#116) so a host-rendered
//! outline lists exactly the headings velotype's sidebar would list. Line
//! indices refer to the canonical Markdown serialization, which is what
//! [`crate::Editor::reveal_outline_heading`] navigates by.

use crate::components::BlockKind;

/// One heading in a document outline.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OutlineEntry {
    /// Heading level, 1-6.
    pub level: u8,
    /// Heading text with the leading hashes and any closing hashes removed.
    pub title: String,
    /// Zero-based line index in the canonical Markdown serialization.
    pub line: usize,
}

/// Collects ATX headings in document order, skipping fenced code blocks.
pub fn outline_entries(markdown: &str) -> Vec<OutlineEntry> {
    let mut entries = Vec::new();
    let mut fence: Option<(char, usize)> = None;

    for (line_index, line) in markdown.lines().enumerate() {
        let trimmed = line.trim_start();
        if let Some((marker, len)) = fence {
            if is_closing_fence(trimmed, marker, len) {
                fence = None;
            }
            continue;
        }
        if let Some(opener) = opening_fence(trimmed) {
            fence = Some(opener);
            continue;
        }
        let Some((level, title)) = BlockKind::parse_atx_heading_line(line) else {
            continue;
        };
        entries.push(OutlineEntry {
            level,
            title,
            line: line_index,
        });
    }

    entries
}

/// Recognizes a backtick or tilde fence of at least three markers.
fn opening_fence(trimmed: &str) -> Option<(char, usize)> {
    let marker = trimmed.chars().next()?;
    if marker != '`' && marker != '~' {
        return None;
    }
    let len = trimmed.chars().take_while(|ch| *ch == marker).count();
    (len >= 3).then_some((marker, len))
}

/// Accepts a matching fence run at least as long as the opener, with no trailing content.
fn is_closing_fence(trimmed: &str, marker: char, len: usize) -> bool {
    let count = trimmed.chars().take_while(|ch| *ch == marker).count();
    count >= len && trimmed[count..].trim().is_empty()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn collects_atx_headings_with_their_source_lines() {
        let markdown = "# Alpha\n\nPara under alpha.\n\n## Beta\n\n### Gamma\n";
        let entries = outline_entries(markdown);
        assert_eq!(
            entries,
            vec![
                OutlineEntry {
                    level: 1,
                    title: "Alpha".into(),
                    line: 0,
                },
                OutlineEntry {
                    level: 2,
                    title: "Beta".into(),
                    line: 4,
                },
                OutlineEntry {
                    level: 3,
                    title: "Gamma".into(),
                    line: 6,
                },
            ]
        );
    }

    #[test]
    fn ignores_headings_inside_fenced_code_blocks() {
        let markdown = "# Real\n\n```markdown\n# Not a heading\n```\n\n## Also real\n";
        let entries = outline_entries(markdown);
        assert_eq!(
            entries.iter().map(|entry| entry.line).collect::<Vec<_>>(),
            vec![0, 6]
        );
    }

    #[test]
    fn strips_closing_hashes_and_ignores_non_headings() {
        let markdown = "## Title ##\n\n#hashtag without space\n\n####### Too deep\n";
        let entries = outline_entries(markdown);
        assert_eq!(
            entries,
            vec![OutlineEntry {
                level: 2,
                title: "Title".into(),
                line: 0,
            }]
        );
    }
}
