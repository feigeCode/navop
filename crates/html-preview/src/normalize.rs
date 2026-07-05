const DOCTYPE: &str = "<!doctype html>";
const AUTO_CLOSE_TAGS: [&str; 9] = [
    "main", "section", "article", "div", "span", "p", "ul", "ol", "li",
];

pub fn normalize_html_document(source: &str) -> String {
    let trimmed = source.trim();
    if is_complete_document(trimmed) {
        return trimmed.to_string();
    }

    let without_doctype = strip_doctype(trimmed);
    let without_html = strip_html_shell(without_doctype);
    let (head, body) = split_head_and_body(without_html);
    format!("{DOCTYPE}<html><head>{head}</head><body>{body}</body></html>")
}

fn is_complete_document(source: &str) -> bool {
    let lower = source.to_ascii_lowercase();
    lower.starts_with(DOCTYPE) && lower.contains("<html") && lower.ends_with("</html>")
}

fn strip_doctype(source: &str) -> &str {
    if source.to_ascii_lowercase().starts_with(DOCTYPE) {
        source[DOCTYPE.len()..].trim()
    } else {
        source
    }
}

fn strip_html_shell(source: &str) -> &str {
    let lower = source.to_ascii_lowercase();
    if !lower.starts_with("<html") {
        return source;
    }
    let start = source.find('>').map(|ix| ix + 1).unwrap_or(0);
    let end = lower.rfind("</html>").unwrap_or(source.len());
    source[start..end].trim()
}

fn split_head_and_body(source: &str) -> (String, String) {
    if let Some((head, body)) = split_with_body_tag(source) {
        return (clean_head(head), close_body_tags(body));
    }
    if let Some((head, body)) = split_with_head_tag(source) {
        return (clean_head(head), close_body_tags(body));
    }
    (String::new(), close_body_tags(source))
}

fn split_with_body_tag(source: &str) -> Option<(&str, &str)> {
    let lower = source.to_ascii_lowercase();
    let body_start = lower.find("<body")?;
    let body_open_end = source[body_start..].find('>')? + body_start + 1;
    let body_end = lower.rfind("</body>").unwrap_or(source.len());
    Some((
        &source[..body_start],
        source[body_open_end..body_end].trim(),
    ))
}

fn split_with_head_tag(source: &str) -> Option<(&str, &str)> {
    let lower = source.to_ascii_lowercase();
    let head_start = lower.find("<head")?;
    let head_open_end = source[head_start..].find('>')? + head_start + 1;
    let head_end = lower.find("</head>").unwrap_or(head_open_end);
    Some((
        source[head_open_end..head_end].trim(),
        source[(head_end + "</head>".len()).min(source.len())..].trim(),
    ))
}

fn clean_head(source: &str) -> String {
    let lower = source.to_ascii_lowercase();
    let mut head = source.trim();
    if let Some(ix) = lower.find("<head") {
        let open_end = source[ix..].find('>').map(|end| ix + end + 1).unwrap_or(ix);
        head = &source[open_end..];
    }
    head.replace("</head>", "").trim().to_string()
}

fn close_body_tags(source: &str) -> String {
    let mut body = source.trim().replace("</body>", "");
    for tag in AUTO_CLOSE_TAGS {
        let opens = count_open_tags(&body, tag);
        let closes = body
            .to_ascii_lowercase()
            .matches(&format!("</{tag}>"))
            .count();
        for _ in closes..opens {
            body.push_str(&format!("</{tag}>"));
        }
    }
    body
}

fn count_open_tags(source: &str, tag: &str) -> usize {
    let lower = source.to_ascii_lowercase();
    lower
        .match_indices(&format!("<{tag}"))
        .filter(|(ix, _)| {
            lower[*ix + tag.len() + 1..]
                .chars()
                .next()
                .is_some_and(|ch| ch == '>' || ch.is_ascii_whitespace())
        })
        .count()
}
