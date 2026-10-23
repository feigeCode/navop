//! 笔记同步适配器（main 侧实现）
//!
//! `notes` crate 依赖 `one-core`，反向依赖会成环，所以 one-core 只定义 trait，
//! 由 main（同时依赖两者）实现并注入。
//!
//! 实现策略：走 `NotesStorage` 的同步接口，以 `documents.json` 里持久化的稳定
//! `document_id` 作为身份。重命名或移动只改 `relative_path`、不改 id，同步时因此
//! 表现为「同一条记录改路径」，不会在云端留下「旧记录 + 新记录」的重复。
//!
//! 相对路径仅用于还原树形结构，不再承担身份职责。

use one_core::cloud_sync::scope::notes::{NoteSyncAdapter, NoteSyncItem};
use notes::{NoteDocument, NotesStorage};

/// 基于文件系统的笔记数据源
#[derive(Default)]
pub struct FileSystemNoteAdapter;

impl FileSystemNoteAdapter {
    /// 打开配置里的笔记存储；未配置时退回默认位置。
    fn storage() -> Option<NotesStorage> {
        let (root, use_files_subdir) = NotesStorage::configured_location()
            .or_else(|_| NotesStorage::default_root().map(|root| (root, true)))
            .ok()?;
        NotesStorage::open_with_layout(root, use_files_subdir).ok()
    }
}

impl NoteSyncAdapter for FileSystemNoteAdapter {
    fn list(&self) -> Vec<NoteSyncItem> {
        let Some(storage) = Self::storage() else {
            return Vec::new();
        };
        storage
            .sync_documents()
            .unwrap_or_default()
            .into_iter()
            .map(|document| NoteSyncItem {
                document_id: document.document_id,
                relative_path: document.relative_path,
                modified_at: document.modified_at,
                content: document.content,
            })
            .collect()
    }

    fn write(&self, item: &NoteSyncItem) {
        let Some(storage) = Self::storage() else {
            return;
        };
        let _ = storage.sync_write_document(&NoteDocument {
            document_id: item.document_id.clone(),
            relative_path: item.relative_path.clone(),
            modified_at: item.modified_at,
            content: item.content.clone(),
        });
    }

    fn delete(&self, document_id: &str) {
        let Some(storage) = Self::storage() else {
            return;
        };
        let _ = storage.sync_delete_document(document_id);
    }
}
