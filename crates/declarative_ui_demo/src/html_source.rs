use std::borrow::Cow;

pub(crate) fn expand_self_closing_tags(source: &str) -> Cow<'_, str> {
    let mut expansion = Expansion::new(source);
    expansion.scan();
    expansion.finish()
}

struct Expansion<'a> {
    source: &'a str,
    output: String,
    copied_until: usize,
    search_from: usize,
    raw_text_tag: Option<&'static str>,
}

impl<'a> Expansion<'a> {
    fn new(source: &'a str) -> Self {
        Self {
            source,
            output: String::new(),
            copied_until: 0,
            search_from: 0,
            raw_text_tag: None,
        }
    }

    fn scan(&mut self) {
        while self.search_from < self.source.len() && self.advance_raw_text() {
            if !self.scan_next() {
                break;
            }
        }
    }

    fn advance_raw_text(&mut self) -> bool {
        let Some(tag) = self.raw_text_tag.take() else {
            return true;
        };
        if tag == "plaintext" {
            return false;
        }
        let Some(close_start) = find_raw_text_close(self.source, self.search_from, tag) else {
            return false;
        };
        self.search_from = close_start;
        true
    }

    fn scan_next(&mut self) -> bool {
        let Some(open) = find_byte(self.source, self.search_from, b'<') else {
            return false;
        };
        match scan_markup(self.source, open) {
            Markup::Start(tag) => {
                self.apply_start_tag(tag);
                true
            }
            Markup::End(end) | Markup::Ignored(end) => {
                self.search_from = end + 1;
                true
            }
            Markup::Text => {
                self.search_from = open + 1;
                true
            }
            Markup::Unterminated => false,
        }
    }

    fn apply_start_tag(&mut self, tag: ScannedTag<'_>) {
        if let Some(slash) = tag.self_closing_at.filter(|_| !is_html_void(tag.name)) {
            self.output.push_str(&self.source[self.copied_until..slash]);
            self.output.push('>');
            self.output.push_str("</");
            self.output.push_str(tag.name);
            self.output.push('>');
            self.copied_until = tag.end + 1;
        } else if tag.self_closing_at.is_none() {
            self.raw_text_tag = raw_text_element(tag.name);
        }
        self.search_from = tag.end + 1;
    }

    fn finish(mut self) -> Cow<'a, str> {
        if self.output.is_empty() {
            return Cow::Borrowed(self.source);
        }
        self.output.push_str(&self.source[self.copied_until..]);
        Cow::Owned(self.output)
    }
}

enum Markup<'a> {
    Start(ScannedTag<'a>),
    End(usize),
    Ignored(usize),
    Text,
    Unterminated,
}

struct ScannedTag<'a> {
    name: &'a str,
    end: usize,
    self_closing_at: Option<usize>,
}

fn scan_markup(source: &str, open: usize) -> Markup<'_> {
    let bytes = source.as_bytes();
    let Some(marker) = bytes.get(open + 1).copied() else {
        return Markup::Text;
    };
    if marker == b'!' || marker == b'?' {
        return scan_ignored_markup(source, open, marker);
    }
    let (is_end, name_start) = if marker == b'/' {
        (true, open + 2)
    } else if marker.is_ascii_alphabetic() {
        (false, open + 1)
    } else {
        return Markup::Text;
    };
    let name_end = scan_tag_name(bytes, name_start);
    let Some(end) = find_tag_end(bytes, name_end) else {
        return Markup::Unterminated;
    };
    if is_end {
        return Markup::End(end);
    }
    Markup::Start(ScannedTag {
        name: &source[name_start..name_end],
        end,
        self_closing_at: (end > name_end && bytes[end - 1] == b'/').then_some(end - 1),
    })
}

fn scan_ignored_markup(source: &str, open: usize, marker: u8) -> Markup<'_> {
    if marker == b'!'
        && source
            .get(open..)
            .is_some_and(|rest| rest.starts_with("<!--"))
    {
        return source[open + 4..]
            .find("-->")
            .map(|offset| Markup::Ignored(open + 4 + offset + 2))
            .unwrap_or(Markup::Unterminated);
    }
    find_tag_end(source.as_bytes(), open + 2)
        .map(Markup::Ignored)
        .unwrap_or(Markup::Unterminated)
}

fn scan_tag_name(bytes: &[u8], start: usize) -> usize {
    let mut cursor = start;
    while bytes
        .get(cursor)
        .is_some_and(|byte| !byte.is_ascii_whitespace() && !matches!(byte, b'/' | b'>'))
    {
        cursor += 1;
    }
    cursor
}

fn find_tag_end(bytes: &[u8], start: usize) -> Option<usize> {
    let mut quote = None;
    for (index, byte) in bytes.iter().copied().enumerate().skip(start) {
        match (quote, byte) {
            (Some(expected), actual) if expected == actual => quote = None,
            (None, b'\'' | b'"') => quote = Some(byte),
            (None, b'>') => return Some(index),
            _ => {}
        }
    }
    None
}

fn find_raw_text_close(source: &str, start: usize, tag: &str) -> Option<usize> {
    let mut search_from = start;
    while let Some(open) = find_byte(source, search_from, b'<') {
        if let Markup::End(_) = scan_markup(source, open) {
            let name_start = open + 2;
            let name_end = scan_tag_name(source.as_bytes(), name_start);
            if source[name_start..name_end].eq_ignore_ascii_case(tag) {
                return Some(open);
            }
        }
        search_from = open + 1;
    }
    None
}

fn find_byte(source: &str, start: usize, needle: u8) -> Option<usize> {
    source.as_bytes()[start..]
        .iter()
        .position(|byte| *byte == needle)
        .map(|offset| start + offset)
}

fn raw_text_element(tag: &str) -> Option<&'static str> {
    const RAW_TEXT_ELEMENTS: &[&str] = &[
        "iframe",
        "noembed",
        "noframes",
        "plaintext",
        "script",
        "style",
        "textarea",
        "title",
        "xmp",
    ];
    RAW_TEXT_ELEMENTS
        .iter()
        .copied()
        .find(|candidate| tag.eq_ignore_ascii_case(candidate))
}

fn is_html_void(tag: &str) -> bool {
    const VOID_ELEMENTS: &[&str] = &[
        "area", "base", "basefont", "bgsound", "br", "col", "embed", "frame", "hr", "img", "input",
        "keygen", "link", "meta", "param", "source", "track", "wbr",
    ];
    VOID_ELEMENTS
        .iter()
        .any(|candidate| tag.eq_ignore_ascii_case(candidate))
}

#[cfg(test)]
mod tests {
    use super::expand_self_closing_tags;

    #[test]
    fn expands_non_void_tags_without_touching_void_tags() {
        let source = r#"<DIV /><input /><sql-editor data-label="a > b" hint='keep /> literal'/>"#;
        assert_eq!(
            r#"<DIV ></DIV><input /><sql-editor data-label="a > b" hint='keep /> literal'></sql-editor>"#,
            expand_self_closing_tags(source)
        );
    }

    #[test]
    fn ignores_comments_and_raw_text_contents() {
        let source = "<!-- <fake /> --><textarea>literal <fake /></textarea><real />";
        assert_eq!(
            "<!-- <fake /> --><textarea>literal <fake /></textarea><real ></real>",
            expand_self_closing_tags(source)
        );
    }

    #[test]
    fn leaves_plaintext_remainder_unchanged() {
        let source = "<plaintext>literal <fake /><real />";
        assert_eq!(source, expand_self_closing_tags(source));
    }
}
