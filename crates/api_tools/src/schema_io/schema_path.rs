use crate::http::KeyValue;

const TEMPLATE_OPEN: &str = "__NAVOP_TEMPLATE_OPEN__";
const TEMPLATE_CLOSE: &str = "__NAVOP_TEMPLATE_CLOSE__";

pub fn request_url(path: &str, has_server: bool) -> String {
    let path = schema_path_to_request(path);
    if has_server {
        format!("{{{{baseUrl}}}}{path}")
    } else {
        path
    }
}

pub fn request_path(url: &str) -> String {
    let raw = absolute_path(url).unwrap_or_else(|| relative_path(url));
    let path = if raw.starts_with('/') {
        raw
    } else {
        format!("/{raw}")
    };
    path.replace("{{", "{").replace("}}", "}")
}

pub fn path_parameter_rows(path: &str, rows: &[KeyValue]) -> Vec<KeyValue> {
    path_parameter_names(path)
        .into_iter()
        .map(|name| {
            rows.iter()
                .find(|row| row.enabled && row.key == name)
                .cloned()
                .unwrap_or_else(|| KeyValue::new(name, ""))
        })
        .collect()
}

fn absolute_path(url: &str) -> Option<String> {
    let protected = url
        .replace("{{", TEMPLATE_OPEN)
        .replace("}}", TEMPLATE_CLOSE);
    let parsed = url::Url::parse(&protected).ok()?;
    Some(
        parsed
            .path()
            .replace(TEMPLATE_OPEN, "{{")
            .replace(TEMPLATE_CLOSE, "}}"),
    )
}

fn relative_path(url: &str) -> String {
    url.split('?')
        .next()
        .unwrap_or(url)
        .replace("{{baseUrl}}", "")
}

fn schema_path_to_request(path: &str) -> String {
    let mut output = String::with_capacity(path.len() + 8);
    let mut chars = path.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '{' && chars.peek() != Some(&'{') {
            output.push_str("{{");
        } else if ch == '}' {
            output.push_str("}}");
        } else {
            output.push(ch);
        }
    }
    output
}

fn path_parameter_names(path: &str) -> Vec<String> {
    let mut names = Vec::new();
    let mut remainder = path;
    while let Some(start) = remainder.find('{') {
        let after_open = &remainder[start + 1..];
        let Some(end) = after_open.find('}') else {
            break;
        };
        let name = &after_open[..end];
        if !name.is_empty() && !names.iter().any(|existing| existing == name) {
            names.push(name.to_string());
        }
        remainder = &after_open[end + 1..];
    }
    names
}
