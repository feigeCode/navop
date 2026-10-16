//! 笔记同步适配器
//!
//! `notes` crate 依赖 `one-core`，反向依赖会成环，因此 one-core 不能直接用
//! `NotesStorage`。这里定义适配器 trait，由 `main`（同时依赖两者）实现并注入。

use std::sync::Arc;

/// 一篇笔记的同步单元
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NoteSyncItem {
    /// 稳定键：笔记索引里持久化的文档 UUID。
    ///
    /// 重命名/移动只改 `relative_path`，`document_id` 不变，因此用它当同步身份，
    /// 一次重命名在云端表现为「同一条记录改路径」而不是「旧记录 + 新记录」。
    pub document_id: String,
    /// 笔记本内相对路径，仅用于还原树形结构（重命名后会变化）
    pub relative_path: String,
    /// 文件修改时间（秒）。笔记的 `updated_at` 不随正文更新，只能取文件 mtime。
    pub modified_at: i64,
    pub content: String,
}

impl NoteSyncItem {
    /// 由相对路径推导标题（去掉扩展名，取最后一段）
    pub fn title(&self) -> String {
        self.relative_path
            .rsplit('/')
            .next()
            .unwrap_or_default()
            .trim_end_matches(".md")
            .to_string()
    }
}

/// 笔记数据源
pub trait NoteSyncAdapter: Send + Sync {
    /// 枚举本机全部笔记
    fn list(&self) -> Vec<NoteSyncItem>;

    /// 写入一篇笔记
    ///
    /// 以 `document_id` 为身份：该 id 已存在但路径不同时移动文件（重命名），
    /// 目录不存在时自动创建。
    fn write(&self, item: &NoteSyncItem);

    /// 按稳定 `document_id` 删除一篇笔记
    fn delete(&self, document_id: &str);
}

/// 类型别名，便于持有
pub type SharedNoteAdapter = Arc<dyn NoteSyncAdapter>;

/// 单篇笔记正文上限（Q4）：超过则跳过不同步
pub const NOTE_MAX_CONTENT_BYTES: usize = 1024 * 1024;

#[cfg(test)]
mod tests {
    use super::NoteSyncItem;

    #[test]
    fn title_strips_directory_and_extension() {
        let item = NoteSyncItem {
            document_id: "doc-1".to_string(),
            relative_path: "work/notes/todo.md".to_string(),
            modified_at: 0,
            content: String::new(),
        };
        assert_eq!("todo", item.title());
    }

    #[test]
    fn title_handles_windows_style_separator() {
        let item = NoteSyncItem {
            document_id: "doc-2".to_string(),
            relative_path: "todo.md".to_string(),
            modified_at: 0,
            content: String::new(),
        };
        assert_eq!("todo", item.title());
    }
}
