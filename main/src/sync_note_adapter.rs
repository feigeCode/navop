//! 笔记同步适配器（main 侧实现）
//!
//! `notes` crate 依赖 `one-core`，反向依赖会成环，所以 one-core 只定义 trait，
//! 由 main（同时依赖两者）实现并注入。
//!
//! 实现策略：直接遍历笔记根目录下的 `.md` 文件，用「相对路径」作为稳定键。
//! 这样不依赖 `NotesStorage` 的内部索引结构，也天然覆盖用户自定义目录。

use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use one_core::cloud_sync::scope::notes::{NoteSyncAdapter, NoteSyncItem};

/// 基于文件系统的笔记数据源
#[derive(Default)]
pub struct FileSystemNoteAdapter;

impl FileSystemNoteAdapter {
    fn root() -> Option<PathBuf> {
        notes::NotesStorage::configured_root()
            .or_else(|_| notes::NotesStorage::default_root())
            .ok()
    }
}

impl NoteSyncAdapter for FileSystemNoteAdapter {
    fn list(&self) -> Vec<NoteSyncItem> {
        let Some(root) = Self::root() else {
            return Vec::new();
        };
        let mut items = Vec::new();
        for (relative, modified_at, content) in collect_markdown(&root) {
            items.push(NoteSyncItem {
                relative_path: relative,
                modified_at,
                content,
            });
        }
        items.sort_by(|a, b| a.relative_path.cmp(&b.relative_path));
        items
    }

    fn write(&self, item: &NoteSyncItem) {
        let Some(root) = Self::root() else {
            return;
        };
        let target = root.join(&item.relative_path);
        if let Some(parent) = target.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let _ = std::fs::write(target, item.content.as_bytes());
    }

    fn delete(&self, relative_path: &str) {
        if let Some(root) = Self::root() {
            let _ = std::fs::remove_file(root.join(relative_path));
        }
    }
}

/// 递归收集 `.md` 文件，返回（相对路径, mtime 秒, 内容）
fn collect_markdown(root: &Path) -> Vec<(String, i64, String)> {
    let mut results = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
                continue;
            }
            if path.extension().and_then(|e| e.to_str()) != Some("md") {
                continue;
            }
            let Ok(content) = std::fs::read_to_string(&path) else {
                continue;
            };
            let Ok(relative) = path.strip_prefix(root) else {
                continue;
            };
            let modified_at = std::fs::metadata(&path)
                .ok()
                .and_then(|meta| meta.modified().ok())
                .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
                .map(|d| d.as_secs() as i64)
                .unwrap_or(0);
            results.push((
                relative.to_string_lossy().replace('\\', "/"),
                modified_at,
                content,
            ));
        }
    }
    results
}
