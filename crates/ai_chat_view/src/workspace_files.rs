//! 从工作区根目录收集可用于 `@` 提及的工作区文件。
//!
//! 工作台的 `@` 提及原本只有数据库连接（见 `resource_builder`），工作区文件完全
//! 缺席。本模块把「本地目录里的文件」变成 [`MentionItem`]，让用户能直接引用
//! 工作区内的文件让 agent 处理。
//!
//! 遍历是**有界**的：目录深度、文件总数都有上限，构建/依赖目录被整枝跳过。
//! 工作区可能有几万个文件，全量收进补全菜单只会拖慢输入框。

use std::collections::VecDeque;
use std::path::Path;

use crate::input::MentionItem;

/// 整枝跳过的目录名：版本控制、构建产物、依赖与编辑器元数据。
pub const IGNORED_DIRS: &[&str] = &[
    ".git",
    "target",
    "node_modules",
    ".workbuddy",
    ".idea",
    ".vscode",
    "dist",
    "build",
    "__pycache__",
    ".venv",
];

/// 最多收集的文件数。超过即停止，先到的文件（浅层、字典序在前）优先。
pub const DEFAULT_MAX_FILES: usize = 300;
/// 目录遍历的最大深度（根为 0）。
pub const DEFAULT_MAX_DEPTH: usize = 8;

/// 构建产物与依赖目录不应出现在 `@` 菜单里。
pub fn is_ignored_dir(name: &str) -> bool {
    IGNORED_DIRS.contains(&name)
}

/// 相对路径 → 提及条目。id 与插入标签都用相对路径（对 agent 有意义的就是它）。
pub fn file_mention(relative_path: &str) -> MentionItem {
    MentionItem::new(relative_path, relative_path, "file", "file")
}

/// 从工作区根目录收集文件提及项（广度优先，目录内按文件名排序，输出稳定）。
///
/// `root` 不存在或不可读时返回空列表，绝不 panic：工作台不能因为一个坏目录
/// 起不来。
pub fn collect_workspace_files(
    root: &Path,
    max_files: usize,
    max_depth: usize,
) -> Vec<MentionItem> {
    let mut out = Vec::new();
    if max_files == 0 {
        return out;
    }
    let mut queue = VecDeque::from([(root.to_path_buf(), 0usize)]);
    while let Some((dir, depth)) = queue.pop_front() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        let mut entries: Vec<_> = entries.flatten().collect();
        entries.sort_by_key(|entry| entry.file_name());
        for entry in entries {
            if out.len() >= max_files {
                return out;
            }
            let Ok(file_type) = entry.file_type() else {
                continue;
            };
            let name = entry.file_name().to_string_lossy().into_owned();
            if file_type.is_dir() {
                if !is_ignored_dir(&name) && depth < max_depth {
                    queue.push_back((entry.path(), depth + 1));
                }
                continue;
            }
            if !file_type.is_file() {
                // 符号链接等非常规条目跳过：读到的内容不可预期。
                continue;
            }
            let path = entry.path();
            let relative = path
                .strip_prefix(root)
                .unwrap_or(&path)
                .to_string_lossy()
                .replace('\\', "/");
            out.push(file_mention(&relative));
        }
    }
    out
}

/// [`collect_workspace_files`] 的默认参数版。
pub fn collect_workspace_files_default(root: &Path) -> Vec<MentionItem> {
    collect_workspace_files(root, DEFAULT_MAX_FILES, DEFAULT_MAX_DEPTH)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::PathBuf;

    /// 独立临时目录：不引 tempfile，用进程 id + 计数保证唯一。
    fn temp_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "aiwb-workspace-files-{}-{}-{tag}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn ignores_build_and_metadata_dirs() {
        assert!(is_ignored_dir("target"));
        assert!(is_ignored_dir(".git"));
        assert!(is_ignored_dir("node_modules"));
        assert!(!is_ignored_dir("src"));
    }

    #[test]
    fn collects_files_in_breadth_first_order_and_skips_ignored_trees() {
        let root = temp_dir("bfs");
        fs::write(root.join("b.txt"), "").unwrap();
        fs::write(root.join("a.txt"), "").unwrap();
        fs::create_dir_all(root.join("src/nested")).unwrap();
        fs::write(root.join("src/nested/lib.rs"), "").unwrap();
        // 整枝跳过。
        fs::create_dir_all(root.join("target/debug")).unwrap();
        fs::write(root.join("target/debug/x.bin"), "").unwrap();

        let mentions = collect_workspace_files_default(&root);

        let labels: Vec<&str> = mentions.iter().map(|item| item.label.as_str()).collect();
        assert_eq!(
            vec!["a.txt", "b.txt", "src/nested/lib.rs"],
            labels,
            "同层按文件名排序、忽略目录整枝跳过"
        );
        assert!(mentions.iter().all(|item| item.kind == "file"));

        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn respects_the_file_cap_and_depth_cap() {
        let root = temp_dir("caps");
        for index in 0..5 {
            fs::write(root.join(format!("f{index}.txt")), "").unwrap();
        }
        // 深度 1（根为 0）在 max_depth=1 时不可达。
        fs::create_dir_all(root.join("deep/deeper")).unwrap();
        fs::write(root.join("deep/deeper/x.txt"), "").unwrap();

        let capped = collect_workspace_files(&root, 3, DEFAULT_MAX_DEPTH);
        assert_eq!(3, capped.len(), "总数上限生效");

        let shallow = collect_workspace_files(&root, DEFAULT_MAX_FILES, 1);
        let labels: Vec<&str> = shallow.iter().map(|item| item.label.as_str()).collect();
        assert_eq!(
            vec!["f0.txt", "f1.txt", "f2.txt", "f3.txt", "f4.txt"],
            labels,
            "深度上限生效：deep/ 下内容不可达"
        );

        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn missing_root_yields_no_mentions() {
        assert!(
            collect_workspace_files_default(Path::new("/nonexistent/aiwb-certainly-missing"))
                .is_empty()
        );
    }
}
