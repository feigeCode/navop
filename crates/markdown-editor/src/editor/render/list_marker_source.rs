use super::active_list_markers::{ListMarker, MarkerKind};
use markdown_source::{SourceBlock, SourceBlockKind};
use std::collections::BTreeMap;

struct OrderedSequence {
    next: u64,
    delimiter: char,
}

pub(super) fn list_markers(
    block: &SourceBlock,
    display: impl Fn(usize) -> usize,
) -> Vec<ListMarker> {
    let mut line_start = block.source_range.start;
    let mut ordered_sequences = BTreeMap::new();
    block
        .original_source
        .split_inclusive('\n')
        .filter_map(|line| {
            let result = marker_for_line(line, line_start, &display, &mut ordered_sequences);
            line_start += line.len();
            result
        })
        .collect()
}

fn marker_for_line(
    line: &str,
    line_start: usize,
    display: &impl Fn(usize) -> usize,
    ordered_sequences: &mut BTreeMap<usize, OrderedSequence>,
) -> Option<ListMarker> {
    let line = line.strip_suffix('\n').unwrap_or(line);
    let indent = line.len().saturating_sub(line.trim_start().len());
    let trimmed = &line[indent..];
    ordered_sequences.retain(|level, _| *level <= indent);
    let (length, kind) = marker_kind(trimmed, indent, ordered_sequences)?;
    Some(ListMarker {
        display_offset: display(line_start + indent + length),
        kind,
    })
}

fn marker_kind(
    line: &str,
    indent: usize,
    ordered_sequences: &mut BTreeMap<usize, OrderedSequence>,
) -> Option<(usize, MarkerKind)> {
    if let Some(rest) = unordered_content(line) {
        ordered_sequences.remove(&indent);
        let base = line.len() - rest.len();
        if let Some(content) = rest.strip_prefix("[ ] ") {
            return Some((line.len() - content.len(), MarkerKind::Task(false)));
        }
        if let Some(content) = rest
            .strip_prefix("[x] ")
            .or_else(|| rest.strip_prefix("[X] "))
        {
            return Some((line.len() - content.len(), MarkerKind::Task(true)));
        }
        return Some((base, MarkerKind::Text("•".to_owned())));
    }
    let marker = ordered_marker(line)?;
    let whitespace = line[marker.len()..].len() - line[marker.len()..].trim_start().len();
    let delimiter = marker.chars().last()?;
    let source_value = marker[..marker.len().saturating_sub(delimiter.len_utf8())]
        .parse::<u64>()
        .ok()?;
    let sequence = ordered_sequences
        .entry(indent)
        .or_insert_with(|| OrderedSequence {
            next: source_value,
            delimiter,
        });
    let marker = format!("{}{}", sequence.next, sequence.delimiter);
    sequence.next = sequence.next.saturating_add(1);
    Some((
        ordered_marker(line)?.len() + whitespace,
        MarkerKind::Text(marker),
    ))
}

fn unordered_content(line: &str) -> Option<&str> {
    line.strip_prefix("- ")
        .or_else(|| line.strip_prefix("* "))
        .or_else(|| line.strip_prefix("+ "))
}

fn ordered_marker(line: &str) -> Option<&str> {
    let whitespace = line.find(char::is_whitespace)?;
    let marker = &line[..whitespace];
    let delimiter = marker.chars().last()?;
    let number = &marker[..marker.len().saturating_sub(delimiter.len_utf8())];
    (matches!(delimiter, '.' | ')') && number.parse::<u64>().is_ok()).then_some(marker)
}

pub(super) fn list_gutter_width(block: &SourceBlock) -> Option<f32> {
    match block.kind {
        SourceBlockKind::OrderedList { .. } => Some(ordered_list_gutter(block)),
        SourceBlockKind::UnorderedList if has_tasks(block) => Some(20.),
        SourceBlockKind::UnorderedList => Some(20.),
        _ => None,
    }
}

fn has_tasks(block: &SourceBlock) -> bool {
    ["[ ] ", "[x] ", "[X] "]
        .iter()
        .any(|marker| block.original_source.contains(marker))
}

fn ordered_list_gutter(block: &SourceBlock) -> f32 {
    block
        .original_source
        .lines()
        .filter_map(|line| ordered_marker(line.trim_start()))
        .map(|marker| marker.chars().count() as f32 * 8. + 6.)
        .fold(22., f32::max)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ordered_list_gutter_expands_for_long_markers() {
        let document =
            markdown_source::SourceMarkdownDocument::parse("99. item\n100. next item").unwrap();
        assert!(ordered_list_gutter(&document.blocks[0]) > 30.);
    }

    #[test]
    fn ordered_markers_follow_rendered_sequence_when_source_repeats_one() {
        let document =
            markdown_source::SourceMarkdownDocument::parse("1. first\n1. second\n1. third")
                .unwrap();
        let markers = list_markers(&document.blocks[0], |offset| offset);
        let labels = markers
            .into_iter()
            .map(|marker| match marker.kind {
                MarkerKind::Text(text) => text,
                MarkerKind::Task(_) => panic!("ordered marker must be text"),
            })
            .collect::<Vec<_>>();

        assert_eq!(["1.", "2.", "3."], labels.as_slice());
    }

    #[test]
    fn ordered_markers_keep_custom_start_and_delimiter() {
        let document =
            markdown_source::SourceMarkdownDocument::parse("9) first\n1) second").unwrap();
        let markers = list_markers(&document.blocks[0], |offset| offset);
        let labels = markers
            .into_iter()
            .map(|marker| match marker.kind {
                MarkerKind::Text(text) => text,
                MarkerKind::Task(_) => panic!("ordered marker must be text"),
            })
            .collect::<Vec<_>>();

        assert_eq!(["9)", "10)"], labels.as_slice());
    }

    #[test]
    fn nested_ordered_markers_keep_independent_sequences() {
        let document = markdown_source::SourceMarkdownDocument::parse(
            "3. parent\n   7) child\n   1) child again\n3. next parent",
        )
        .unwrap();
        let markers = list_markers(&document.blocks[0], |offset| offset);
        let labels = markers
            .into_iter()
            .map(|marker| match marker.kind {
                MarkerKind::Text(text) => text,
                MarkerKind::Task(_) => panic!("ordered marker must be text"),
            })
            .collect::<Vec<_>>();

        assert_eq!(["3.", "7)", "8)", "4."], labels.as_slice());
    }

    #[test]
    fn mixed_task_markers_map_to_each_projected_line_start() {
        let document =
            markdown_source::SourceMarkdownDocument::parse("- First\n- [ ] Todo\n- [x] Done")
                .unwrap();
        let block = &document.blocks[0];
        let projection =
            crate::MarkdownProjection::build_range(&document, None, block.source_range.clone());
        let markers = list_markers(block, |offset| projection.source_to_display(offset));

        assert_eq!("First\nTodo\nDone", projection.text);
        assert_eq!(
            vec![0, 6, 11],
            markers.iter().map(|m| m.display_offset).collect::<Vec<_>>()
        );
    }
}
