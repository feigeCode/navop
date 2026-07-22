use std::ops::Range;

pub(super) fn directive_ranges(source: &str) -> Vec<Range<usize>> {
    let lines = source_line_ranges(source);
    let mut result = Vec::new();
    let mut index = 0;
    while index < lines.len() {
        let line = source[lines[index].clone()].trim();
        if line.starts_with(":::") && line.len() > 3 {
            let closing = closing_line(source, &lines, index);
            result.push(lines[index].start..lines[closing].end);
            index = closing + 1;
        } else {
            index += 1;
        }
    }
    result
}

fn closing_line(source: &str, lines: &[Range<usize>], opening: usize) -> usize {
    ((opening + 1)..lines.len())
        .find(|candidate| source[lines[*candidate].clone()].trim() == ":::")
        .unwrap_or(lines.len() - 1)
}

fn source_line_ranges(source: &str) -> Vec<Range<usize>> {
    let mut ranges = Vec::new();
    let mut start = 0;
    for line in source.split_inclusive('\n') {
        let raw_end = start + line.len();
        let without_newline = line.strip_suffix('\n').unwrap_or(line);
        let content = without_newline
            .strip_suffix('\r')
            .unwrap_or(without_newline);
        ranges.push(start..start + content.len());
        start = raw_end;
    }
    if source.is_empty() {
        ranges.push(0..0);
    }
    ranges
}
