use std::collections::VecDeque;

const MAX_CD_COMPLETION_CACHE_ENTRIES: usize = 32;
const MAX_CD_COMPLETION_NAMES_PER_DIRECTORY: usize = 512;
const MAX_CD_COMPLETION_NAME_BYTES_PER_DIRECTORY: usize = 64 * 1024;
const MAX_CD_COMPLETION_PARENT_DIR_BYTES: usize = 4 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CdCompletionQuery {
    pub parent_dir: String,
    pub typed_prefix: String,
    pub needle: String,
}

#[derive(Debug)]
pub struct CdCompletionCache {
    entries: VecDeque<(String, Vec<String>)>,
    max_entries: usize,
    max_names_per_directory: usize,
    max_name_bytes_per_directory: usize,
    max_parent_dir_bytes: usize,
}

impl Default for CdCompletionCache {
    fn default() -> Self {
        Self {
            entries: VecDeque::new(),
            max_entries: MAX_CD_COMPLETION_CACHE_ENTRIES,
            max_names_per_directory: MAX_CD_COMPLETION_NAMES_PER_DIRECTORY,
            max_name_bytes_per_directory: MAX_CD_COMPLETION_NAME_BYTES_PER_DIRECTORY,
            max_parent_dir_bytes: MAX_CD_COMPLETION_PARENT_DIR_BYTES,
        }
    }
}

impl CdCompletionCache {
    pub fn get(&mut self, parent_dir: &str) -> Option<&[String]> {
        let position = self
            .entries
            .iter()
            .position(|(cached_parent, _)| cached_parent == parent_dir)?;
        let entry = self
            .entries
            .remove(position)
            .expect("cache position must remain valid");
        self.entries.push_back(entry);
        self.entries.back().map(|(_, names)| names.as_slice())
    }

    pub fn insert(
        &mut self,
        parent_dir: String,
        directory_names: impl IntoIterator<Item = String>,
    ) -> bool {
        if self.max_entries == 0 || parent_dir.len() > self.max_parent_dir_bytes {
            self.remove(&parent_dir);
            return false;
        }

        let directory_names = directory_names.into_iter();
        let initial_capacity = directory_names
            .size_hint()
            .1
            .unwrap_or_default()
            .min(self.max_names_per_directory);
        let mut retained_names = Vec::with_capacity(initial_capacity);
        let mut retained_bytes = 0usize;
        for name in directory_names.take(self.max_names_per_directory) {
            let Some(next_bytes) = retained_bytes.checked_add(name.len()) else {
                break;
            };
            if next_bytes > self.max_name_bytes_per_directory {
                break;
            }
            retained_bytes = next_bytes;
            retained_names.push(name);
        }

        self.remove(&parent_dir);
        while self.entries.len() >= self.max_entries {
            self.entries.pop_front();
        }
        self.entries.push_back((parent_dir, retained_names));
        true
    }

    pub fn clear(&mut self) {
        self.entries.clear();
    }

    fn remove(&mut self, parent_dir: &str) {
        if let Some(position) = self
            .entries
            .iter()
            .position(|(cached_parent, _)| cached_parent == parent_dir)
        {
            self.entries.remove(position);
        }
    }

    #[cfg(test)]
    fn with_limits(
        max_entries: usize,
        max_names_per_directory: usize,
        max_name_bytes_per_directory: usize,
        max_parent_dir_bytes: usize,
    ) -> Self {
        Self {
            entries: VecDeque::new(),
            max_entries,
            max_names_per_directory,
            max_name_bytes_per_directory,
            max_parent_dir_bytes,
        }
    }
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
    use std::cell::Cell;

    use super::{
        CdCompletionCache, CdCompletionQuery, build_cd_completion_suggestions,
        parse_cd_completion_query,
    };

    fn names(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| (*value).to_string()).collect()
    }

    #[test]
    fn cd_completion_cache_evicts_the_least_recently_used_directory() {
        let mut cache = CdCompletionCache::with_limits(2, 10, 1024, 1024);
        assert!(cache.insert("/a".to_string(), names(&["alpha"])));
        assert!(cache.insert("/b".to_string(), names(&["beta"])));

        assert_eq!(cache.get("/a"), Some(names(&["alpha"]).as_slice()));
        assert!(cache.insert("/c".to_string(), names(&["charlie"])));

        assert!(cache.get("/b").is_none());
        assert_eq!(cache.get("/a"), Some(names(&["alpha"]).as_slice()));
        assert_eq!(cache.get("/c"), Some(names(&["charlie"]).as_slice()));
    }

    #[test]
    fn cd_completion_cache_bounds_names_and_utf8_bytes_per_directory() {
        let mut cache = CdCompletionCache::with_limits(2, 3, 7, 1024);
        assert!(cache.insert("/srv".to_string(), names(&["a", "资料", "bb", "ccc"])));

        assert_eq!(cache.get("/srv"), Some(names(&["a", "资料"]).as_slice()));
    }

    #[test]
    fn cd_completion_cache_stops_consuming_names_at_the_configured_limit() {
        let mut cache = CdCompletionCache::with_limits(2, 3, 1024, 1024);
        let consumed = Cell::new(0);
        let directory_names = (0..100).map(|index| {
            consumed.set(consumed.get() + 1);
            format!("directory-{index}")
        });

        assert!(cache.insert("/srv".to_string(), directory_names));

        assert_eq!(consumed.get(), 3);
        assert_eq!(
            cache.get("/srv"),
            Some(names(&["directory-0", "directory-1", "directory-2"]).as_slice())
        );
    }

    #[test]
    fn cd_completion_cache_refreshes_replaced_entries_and_rejects_oversized_keys() {
        let mut cache = CdCompletionCache::with_limits(2, 10, 1024, 4);
        assert!(cache.insert("/a".to_string(), names(&["old"])));
        assert!(cache.insert("/b".to_string(), names(&["beta"])));
        assert!(cache.insert("/a".to_string(), names(&["new"])));
        assert!(cache.insert("/c".to_string(), names(&["charlie"])));

        assert!(cache.get("/b").is_none());
        assert_eq!(cache.get("/a"), Some(names(&["new"]).as_slice()));
        assert!(!cache.insert("/toolong".to_string(), names(&["ignored"])));
        assert!(cache.get("/toolong").is_none());
    }

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
