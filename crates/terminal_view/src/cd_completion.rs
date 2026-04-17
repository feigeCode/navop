#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CdCompletionQuery {
    pub parent_dir: String,
    pub typed_prefix: String,
    pub needle: String,
}

pub fn parse_cd_completion_query(
    input: &str,
    current_working_dir: Option<&str>,
) -> Option<CdCompletionQuery> {
    let current_working_dir = current_working_dir?;
    let rest = input.strip_prefix("cd")?;
    if rest.is_empty() || !rest.starts_with(char::is_whitespace) {
        return None;
    }
    if has_unterminated_shell_quote(input) {
        return None;
    }

    let path = rest.trim_start_matches(char::is_whitespace);
    if path.contains(['\n', '\r', ';', '|', '&', '`']) {
        return None;
    }

    Some(parse_path_query(path, current_working_dir))
}

pub fn build_cd_completion_suggestions(
    query: &CdCompletionQuery,
    directory_names: &[String],
) -> Vec<String> {
    let mut matches: Vec<&String> = directory_names
        .iter()
        .filter(|name| name.starts_with(&query.needle))
        .collect();
    matches.sort();

    matches
        .into_iter()
        .map(|name| {
            format!(
                "cd {}{}/",
                query.typed_prefix,
                shell_escape_path_segment(name)
            )
        })
        .collect()
}

fn parse_path_query(path: &str, current_working_dir: &str) -> CdCompletionQuery {
    if path.is_empty() {
        return CdCompletionQuery {
            parent_dir: normalize_remote_path(current_working_dir),
            typed_prefix: String::new(),
            needle: String::new(),
        };
    }

    if path.ends_with('/') {
        return CdCompletionQuery {
            parent_dir: resolve_remote_parent(current_working_dir, path),
            typed_prefix: path.to_string(),
            needle: String::new(),
        };
    }

    match path.rsplit_once('/') {
        Some((parent, needle)) => CdCompletionQuery {
            parent_dir: resolve_remote_parent(current_working_dir, parent),
            typed_prefix: format!("{parent}/"),
            needle: needle.to_string(),
        },
        None => CdCompletionQuery {
            parent_dir: normalize_remote_path(current_working_dir),
            typed_prefix: String::new(),
            needle: path.to_string(),
        },
    }
}

fn resolve_remote_parent(current_working_dir: &str, path: &str) -> String {
    if path.starts_with('/') {
        normalize_remote_path(path)
    } else {
        normalize_remote_path(&format!(
            "{}/{}",
            current_working_dir.trim_end_matches('/'),
            path
        ))
    }
}

fn normalize_remote_path(path: &str) -> String {
    let mut parts = Vec::new();
    for part in path.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            _ => parts.push(part),
        }
    }

    if parts.is_empty() {
        "/".to_string()
    } else {
        format!("/{}", parts.join("/"))
    }
}

fn shell_escape_path_segment(segment: &str) -> String {
    let mut escaped = String::with_capacity(segment.len());
    for ch in segment.chars() {
        if ch.is_alphanumeric() || matches!(ch, '-' | '_' | '.' | '~') {
            escaped.push(ch);
        } else {
            escaped.push('\\');
            escaped.push(ch);
        }
    }
    escaped
}

fn has_unterminated_shell_quote(text: &str) -> bool {
    let mut in_single_quote = false;
    let mut in_double_quote = false;
    let mut escaped = false;

    for ch in text.chars() {
        if in_single_quote {
            if ch == '\'' {
                in_single_quote = false;
            }
            continue;
        }

        if escaped {
            escaped = false;
            continue;
        }

        match ch {
            '\\' => escaped = true,
            '\'' => in_single_quote = true,
            '"' => in_double_quote = !in_double_quote,
            _ => {}
        }
    }

    in_single_quote || in_double_quote
}

#[cfg(test)]
mod tests {
    use super::{build_cd_completion_suggestions, parse_cd_completion_query, CdCompletionQuery};

    #[test]
    fn cd_completion_parses_empty_child_directory_query_from_current_working_dir() {
        let query =
            parse_cd_completion_query("cd ", Some("/srv/project")).expect("应识别 cd 空路径查询");

        assert_eq!(
            query,
            CdCompletionQuery {
                parent_dir: "/srv/project".to_string(),
                typed_prefix: String::new(),
                needle: String::new(),
            }
        );
    }

    #[test]
    fn cd_completion_parses_relative_parent_and_absolute_path_queries() {
        let parent_query = parse_cd_completion_query("cd ../Do", Some("/srv/project/app"))
            .expect("应识别 ../ 相对路径");
        assert_eq!(
            parent_query,
            CdCompletionQuery {
                parent_dir: "/srv/project".to_string(),
                typed_prefix: "../".to_string(),
                needle: "Do".to_string(),
            }
        );

        let absolute_query = parse_cd_completion_query("cd /usr/lo", Some("/srv/project/app"))
            .expect("应识别绝对路径");
        assert_eq!(
            absolute_query,
            CdCompletionQuery {
                parent_dir: "/usr".to_string(),
                typed_prefix: "/usr/".to_string(),
                needle: "lo".to_string(),
            }
        );
    }

    #[test]
    fn cd_completion_formats_directory_suggestions_with_trailing_slash_and_shell_escaping() {
        let query = CdCompletionQuery {
            parent_dir: "/srv/project".to_string(),
            typed_prefix: String::new(),
            needle: "My".to_string(),
        };

        let suggestions = build_cd_completion_suggestions(
            &query,
            &[
                "My Docs".to_string(),
                "MyApp".to_string(),
                "notes".to_string(),
            ],
        );

        assert_eq!(
            suggestions,
            vec!["cd My\\ Docs/".to_string(), "cd MyApp/".to_string(),]
        );
    }
}
