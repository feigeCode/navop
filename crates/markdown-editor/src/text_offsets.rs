//! Unicode coordinate conversion shared by native input handlers.
//!
//! Stored selections use UTF-8 byte offsets; GPUI's IME interface uses UTF-16
//! code units. These helpers clip scalars, not graphemes: an IME must be able to
//! replace a combining mark without deleting its base character.

use std::ops::Range;

/// Clips a byte offset to the preceding UTF-8 scalar boundary, including EOF.
pub(crate) fn floor_char_boundary(text: &str, offset: usize) -> usize {
    let mut offset = offset.min(text.len());
    while !text.is_char_boundary(offset) {
        offset -= 1;
    }
    offset
}

/// Clips a byte offset to the following UTF-8 scalar boundary, including EOF.
fn ceil_char_boundary(text: &str, offset: usize) -> usize {
    let mut offset = offset.min(text.len());
    while !text.is_char_boundary(offset) {
        offset += 1;
    }
    offset
}

/// Orders and clips a replacement range without splitting a UTF-8 scalar.
///
/// Nonempty ranges expand outward; a collapsed caret stays collapsed at the
/// preceding boundary. Selection direction is retained separately by callers.
pub(crate) fn normalize_utf8_range(text: &str, range: Range<usize>) -> Range<usize> {
    let start = floor_char_boundary(text, range.start.min(range.end));
    if range.start == range.end {
        start..start
    } else {
        start..ceil_char_boundary(text, range.start.max(range.end))
    }
}

/// Converts UTF-16 code units to a byte offset, rounding inside a surrogate down.
pub(crate) fn utf16_to_utf8(text: &str, offset: usize) -> usize {
    utf16_boundary(text, offset, false)
}

/// Converts a UTF-16 position with an explicit bias inside a surrogate pair.
fn utf16_boundary(text: &str, offset: usize, round_up: bool) -> usize {
    let mut units = 0;
    for (byte, ch) in text.char_indices() {
        let end = units + ch.len_utf16();
        if offset < end {
            return if round_up && offset > units {
                byte + ch.len_utf8()
            } else {
                byte
            };
        }
        units = end;
    }
    text.len()
}

/// Converts a byte offset to UTF-16 after clipping it to a scalar boundary.
pub(crate) fn utf8_to_utf16(text: &str, offset: usize) -> usize {
    text[..floor_char_boundary(text, offset)]
        .encode_utf16()
        .count()
}

/// Converts an IME range, expanding partial surrogates without expanding carets.
pub(crate) fn utf16_range_to_utf8(text: &str, range: &Range<usize>) -> Range<usize> {
    let start = utf16_to_utf8(text, range.start.min(range.end));
    if range.start == range.end {
        start..start
    } else {
        start..utf16_boundary(text, range.start.max(range.end), true)
    }
}

/// Converts an internal selection into an ordered UTF-16 range for GPUI.
pub(crate) fn utf8_range_to_utf16(text: &str, range: &Range<usize>) -> Range<usize> {
    let range = normalize_utf8_range(text, range.clone());
    utf8_to_utf16(text, range.start)..utf8_to_utf16(text, range.end)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unicode_ranges_always_land_on_scalar_boundaries() {
        for text in ["*测试", "中文😀かな", "e\u{301}", "👨‍👩‍👧‍👦", "བོད་"] {
            for start in 0..=text.len() + 1 {
                for end in 0..=text.len() + 1 {
                    let range = normalize_utf8_range(text, start..end);
                    assert!(text.get(range.clone()).is_some(), "{text:?}: {range:?}");
                    if start == end {
                        assert!(range.is_empty());
                    }
                }
            }
            for (byte, _) in text
                .char_indices()
                .chain(std::iter::once((text.len(), '\0')))
            {
                assert_eq!(utf16_to_utf8(text, utf8_to_utf16(text, byte)), byte);
            }
        }
    }

    #[test]
    fn unicode_partial_surrogate_ranges_expand_but_carets_do_not() {
        assert_eq!(utf16_range_to_utf8("a😀b", &(2..2)), 1..1);
        assert_eq!(utf16_range_to_utf8("a😀b", &(1..2)), 1..5);
        assert_eq!(utf16_range_to_utf8("a😀b", &(2..3)), 1..5);
        assert_eq!(utf16_range_to_utf8("a😀b", &(3..1)), 1..5);
        assert_eq!(utf16_range_to_utf8("a😀b", &(99..99)), 6..6);
    }

    #[test]
    fn unicode_combining_mark_replacement_does_not_expand_to_grapheme() {
        let text = "e\u{301}";
        assert_eq!(utf16_range_to_utf8(text, &(1..2)), 1..3);
        assert_eq!(normalize_utf8_range(text, 1..3), 1..3);
    }
}
