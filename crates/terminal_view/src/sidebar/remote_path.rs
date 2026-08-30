//! 远端路径拼接与归一化
//!
//! SFTP 侧边栏的路径来源多样（OSC 7、用户输入、面包屑点击、目录项拼接），
//! 统一在此折叠 `.`/`..`、清理重复与尾部斜杠，避免 `current_path` 被写坏后
//! 面包屑显示重复路径、目录列表加载失败。

/// 归一化绝对远端路径：折叠 `.`/`..`、合并连续 `/`、去尾部 `/`。
///
/// 非绝对路径原样返回（由 [`resolve_remote_path`] 负责先解析为绝对路径）。
pub fn normalize_remote_path(path: &str) -> String {
    if !path.starts_with('/') {
        return path.to_string();
    }
    let mut parts: Vec<&str> = Vec::new();
    for segment in path.split('/') {
        match segment {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            other => parts.push(other),
        }
    }
    if parts.is_empty() {
        "/".to_string()
    } else {
        format!("/{}", parts.join("/"))
    }
}

/// 以 `base` 为基准拼接子路径并归一化。
///
/// `base` 的尾部斜杠会被清理，避免拼出 `a//b`。
pub fn join_remote_path(base: &str, name: &str) -> String {
    let base = base.trim_end_matches('/');
    let base = if base.is_empty() { "/" } else { base };
    normalize_remote_path(&format!("{base}/{name}"))
}

/// 把外部来源的路径解析为绝对路径。
///
/// 绝对路径直接归一化；相对路径（如终端 OSC 7 之外的来源）基于 `current` 解析。
pub fn resolve_remote_path(current: &str, path: &str) -> String {
    let path = path.trim();
    if path.starts_with('/') {
        normalize_remote_path(path)
    } else {
        join_remote_path(current, path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_collapses_duplicate_and_trailing_slashes() {
        assert_eq!(normalize_remote_path("//var//log//"), "/var/log");
        assert_eq!(normalize_remote_path("/"), "/");
        assert_eq!(normalize_remote_path("/var/log/"), "/var/log");
    }

    #[test]
    fn normalize_resolves_dot_segments() {
        assert_eq!(normalize_remote_path("/var/./log"), "/var/log");
        assert_eq!(normalize_remote_path("/var/log/.."), "/var");
        assert_eq!(normalize_remote_path("/var/log/../.."), "/");
        assert_eq!(normalize_remote_path("/.."), "/");
        assert_eq!(normalize_remote_path("/../var"), "/var");
    }

    #[test]
    fn normalize_keeps_non_absolute_path_untouched() {
        assert_eq!(normalize_remote_path("var/log"), "var/log");
        assert_eq!(normalize_remote_path(""), "");
    }

    #[test]
    fn join_handles_root_and_trailing_slash_base() {
        assert_eq!(join_remote_path("/", "logs"), "/logs");
        assert_eq!(join_remote_path("/var/", "log"), "/var/log");
        assert_eq!(join_remote_path("/var//", "log"), "/var/log");
        assert_eq!(join_remote_path("", "log"), "/log");
    }

    #[test]
    fn resolve_normalizes_absolute_and_resolves_relative() {
        assert_eq!(resolve_remote_path("/var", "/log/../tmp"), "/tmp");
        assert_eq!(resolve_remote_path("/var", "log"), "/var/log");
        assert_eq!(resolve_remote_path("/var", "./log"), "/var/log");
        assert_eq!(resolve_remote_path("/var", "../etc"), "/etc");
        assert_eq!(resolve_remote_path("/var", "  /log  "), "/log");
    }
}
