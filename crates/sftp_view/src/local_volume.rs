//! 本地磁盘 / 挂载点枚举，以及地址栏输入路径的规范化。
//!
//! 左侧本地栏需要把「这台机器上有哪些盘」直接列给用户看，
//! 同时地址栏输入 `D:` 这类裸盘符必须补成盘根，否则按 `Path` 语义会
//! 落到「D 盘的当前目录」而不是资源管理器展示的 `D:\`。

use std::collections::HashSet;
use std::path::{Path, PathBuf};

/// 裸盘符长度：盘符字符 + 冒号，如 `D:`。
const BARE_DRIVE_LETTER_LEN: usize = 2;

/// 一个可快速跳转的本地卷（Windows 盘符、类 Unix 挂载点或根目录）。
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct LocalVolume {
    path: PathBuf,
    label: String,
}

impl LocalVolume {
    pub(crate) fn path(&self) -> &Path {
        &self.path
    }

    pub(crate) fn label(&self) -> &str {
        &self.label
    }

    /// 完整路径文本；与 `label` 相同时（例如根目录 `/`）不必重复展示。
    pub(crate) fn location(&self) -> String {
        self.path.to_string_lossy().to_string()
    }
}

/// 磁盘下拉只在 Windows 上展示：盘符切换是 Windows 特有的使用习惯，
/// macOS / Linux 的挂载点对用户没有同等价值。
pub(crate) fn volume_menu_enabled() -> bool {
    cfg!(windows)
}

/// 枚举当前系统上可浏览的本地卷，保持盘符 / 挂载点的自然顺序。
pub(crate) fn local_volumes() -> Vec<LocalVolume> {
    let mut seen = HashSet::new();
    let mut volumes = Vec::new();
    for path in volume_candidates() {
        if !path.is_dir() {
            continue;
        }
        if !seen.insert(path.to_string_lossy().to_lowercase()) {
            continue;
        }
        let label = display_label(&path);
        volumes.push(LocalVolume { path, label });
    }
    volumes
}

/// 命中路径最长的卷即为「当前卷」，根目录 `/` 不会盖掉具体挂载点。
pub(crate) fn current_volume_index(volumes: &[LocalVolume], current: &Path) -> Option<usize> {
    volumes
        .iter()
        .enumerate()
        .filter(|(_, volume)| current.starts_with(&volume.path))
        .max_by_key(|(_, volume)| volume.path.as_os_str().len())
        .map(|(index, _)| index)
}

/// 把地址栏里输入的本地路径规范化成可直接使用的目录路径。
///
/// - Windows 上裸盘符 `D:` 补成 `D:\`，与资源管理器一致；
/// - 去掉首尾空白，以及资源管理器「复制路径」常见的外层引号。
pub(crate) fn normalize_local_path_input(input: &str) -> PathBuf {
    normalize_local_path_input_for(input, cfg!(windows))
}

fn normalize_local_path_input_for(input: &str, windows: bool) -> PathBuf {
    let unquoted = strip_wrapping_quotes(input.trim());
    if windows && is_bare_drive_letter(unquoted) {
        return PathBuf::from(format!("{unquoted}\\"));
    }
    PathBuf::from(unquoted)
}

fn strip_wrapping_quotes(text: &str) -> &str {
    for quote in ['"', '\''] {
        if let Some(inner) = text
            .strip_prefix(quote)
            .and_then(|rest| rest.strip_suffix(quote))
        {
            return inner;
        }
    }
    text
}

fn is_bare_drive_letter(text: &str) -> bool {
    let bytes = text.as_bytes();
    bytes.len() == BARE_DRIVE_LETTER_LEN && bytes[0].is_ascii_alphabetic() && bytes[1] == b':'
}

fn volume_candidates() -> Vec<PathBuf> {
    if cfg!(windows) {
        windows_candidates()
    } else {
        unix_candidates()
    }
}

/// Windows：探测 `A:\`..`Z:\`，存在的盘（含映射网络盘）即为可用卷。
fn windows_candidates() -> Vec<PathBuf> {
    (b'A'..=b'Z')
        .map(|letter| PathBuf::from(format!("{}:\\", letter as char)))
        .collect()
}

/// 类 Unix：根目录加常见挂载点下的一级子目录。
///
/// 只展开一层：`/media/<user>/<disk>` 这类两级结构由 `/media/<user>` 入口覆盖，
/// 避免把挂载点内部的普通目录也当成卷列出来。
fn unix_candidates() -> Vec<PathBuf> {
    let mut bases = vec![
        PathBuf::from("/Volumes"),
        PathBuf::from("/media"),
        PathBuf::from("/mnt"),
        PathBuf::from("/run/media"),
    ];
    if let Some(user) = std::env::var_os("USER") {
        bases.push(Path::new("/media").join(&user));
        bases.push(Path::new("/run/media").join(&user));
    }

    let mut candidates = vec![PathBuf::from("/")];
    for base in bases {
        candidates.extend(child_dirs(&base));
    }
    candidates
}

fn child_dirs(base: &Path) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(base) else {
        return Vec::new();
    };
    let mut dirs: Vec<PathBuf> = entries
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| !is_hidden(path) && path.is_dir())
        .collect();
    dirs.sort();
    dirs
}

fn is_hidden(path: &Path) -> bool {
    path.file_name()
        .is_some_and(|name| name.to_string_lossy().starts_with('.'))
}

/// 卷在列表里的显示名：取最后一级名称，盘根去掉结尾分隔符（`C:\` → `C:`）。
fn display_label(path: &Path) -> String {
    let text = path.to_string_lossy();
    let trimmed = text.trim_end_matches(['\\', '/']);
    if trimmed.is_empty() {
        return text.into_owned();
    }
    match trimmed.rsplit(['\\', '/']).next() {
        Some(name) if !name.is_empty() => name.to_string(),
        _ => trimmed.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::{
        LocalVolume, current_volume_index, display_label, local_volumes, normalize_local_path_input,
        normalize_local_path_input_for, volume_menu_enabled,
    };
    use std::path::{Path, PathBuf};

    #[test]
    fn bare_drive_letter_gains_the_root_separator_on_windows() {
        assert_eq!(
            PathBuf::from("D:\\"),
            normalize_local_path_input_for("D:", true)
        );
        assert_eq!(
            PathBuf::from("d:\\"),
            normalize_local_path_input_for("  d:  ", true)
        );
    }

    #[test]
    fn paths_with_a_separator_or_more_segments_are_untouched_on_windows() {
        assert_eq!(
            PathBuf::from("D:\\"),
            normalize_local_path_input_for("D:\\", true)
        );
        assert_eq!(
            PathBuf::from("D:\\Games"),
            normalize_local_path_input_for("D:\\Games", true)
        );
        assert_eq!(
            PathBuf::from("\\\\server\\share"),
            normalize_local_path_input_for("\\\\server\\share", true)
        );
    }

    #[test]
    fn bare_drive_letter_is_untouched_off_windows() {
        assert_eq!(
            PathBuf::from("D:"),
            normalize_local_path_input_for("D:", false)
        );
    }

    #[test]
    fn explorer_copied_quotes_and_padding_are_stripped() {
        assert_eq!(
            PathBuf::from("C:\\Program Files"),
            normalize_local_path_input_for("\"C:\\Program Files\"", true)
        );
        assert_eq!(
            PathBuf::from("/tmp/docs"),
            normalize_local_path_input_for("  '/tmp/docs'  ", false)
        );
        // 只有成对引号才算包裹，单个引号可能是合法目录名。
        assert_eq!(
            PathBuf::from("\""),
            normalize_local_path_input_for("\"", true)
        );
    }

    #[test]
    fn normalize_uses_the_platform_drive_rule() {
        let expected = if cfg!(windows) {
            PathBuf::from("D:\\")
        } else {
            PathBuf::from("D:")
        };
        assert_eq!(expected, normalize_local_path_input("D:"));
    }

    #[test]
    fn volume_labels_use_the_last_path_segment() {
        assert_eq!("C:", display_label(Path::new("C:\\")));
        assert_eq!("/", display_label(Path::new("/")));
        assert_eq!("USB", display_label(Path::new("/Volumes/USB")));
        assert_eq!("data", display_label(Path::new("/mnt/data")));
    }

    #[test]
    fn volume_menu_follows_the_platform_gate() {
        assert_eq!(cfg!(windows), volume_menu_enabled());
    }

    #[test]
    fn enumerated_volumes_are_existing_absolute_directories() {
        let volumes = local_volumes();
        assert!(!volumes.is_empty(), "至少应能列出根目录或系统盘");
        for volume in &volumes {
            assert!(volume.path().is_absolute(), "{}", volume.path().display());
            assert!(
                volume.path().is_dir(),
                "{} 不是目录",
                volume.path().display()
            );
            assert!(!volume.label().is_empty());
        }
    }

    #[test]
    fn current_volume_prefers_the_longest_match() {
        let volumes = vec![
            LocalVolume {
                path: PathBuf::from("/"),
                label: "/".to_string(),
            },
            LocalVolume {
                path: PathBuf::from("/Volumes/USB"),
                label: "USB".to_string(),
            },
        ];
        assert_eq!(
            Some(1),
            current_volume_index(&volumes, Path::new("/Volumes/USB/docs"))
        );
        assert_eq!(Some(0), current_volume_index(&volumes, Path::new("/Users/me")));
        assert_eq!(None, current_volume_index(&[], Path::new("/")));
    }
}
