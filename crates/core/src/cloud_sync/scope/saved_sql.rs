//! 已保存 SQL 的稳定身份索引
//!
//! 同步身份不能用相对路径：重命名会让同一条记录分裂成「旧记录 + 新记录」两条。
//!
//! 笔记有应用内入口 `rename_node` 可以顺带迁移索引；而 `queries/` 下的 SQL
//! **没有应用内重命名入口**（`sql-library` 是另一棵目录树），重命名只会发生在
//! 应用之外（资源管理器里改文件名）。所以这里除 UUID 外还记录正文哈希：枚举时若
//! 某个 id 登记的路径已不在磁盘上、且出现了内容一致的新路径，就把 id 继承给新
//! 路径——这样应用内外的重命名都能保住身份。

use std::collections::{BTreeMap, HashSet};
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use serde::{Deserialize, Serialize};

use super::personal::saved_sql_root;
use crate::cloud_sync::service::CloudSyncService;
use crate::storage::get_config_dir;

/// 索引文件放在配置目录（与 `queries/` 同级），避免污染用户的 SQL 目录
const INDEX_FILE: &str = "saved-sql-index.json";
const SCHEMA_VERSION: u32 = 1;

/// 一条 SQL 的身份记录
#[derive(Debug, Clone, Serialize, Deserialize)]
struct DocumentRecord {
    /// 相对 `queries/` 的路径（重命名后会变）
    path: String,
    /// 正文哈希，用于识别「路径变了、内容没变」的重命名
    content_hash: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct SavedSqlIndex {
    #[serde(default = "schema_version")]
    schema_version: u32,
    /// document_id -> 身份记录
    #[serde(default)]
    documents: BTreeMap<String, DocumentRecord>,
}

fn schema_version() -> u32 {
    SCHEMA_VERSION
}

impl Default for SavedSqlIndex {
    fn default() -> Self {
        Self {
            schema_version: SCHEMA_VERSION,
            documents: BTreeMap::new(),
        }
    }
}

/// 一篇已保存 SQL 的同步单元
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SavedSqlDocument {
    /// 稳定身份，跨重命名/移动保持一致
    pub document_id: String,
    /// 相对 `queries/` 的路径，仅用于还原文件位置
    pub relative_path: String,
    /// 文件修改时间（秒）
    pub modified_at: i64,
    pub content: String,
}

fn index_path() -> Option<PathBuf> {
    get_config_dir().ok().map(|dir| dir.join(INDEX_FILE))
}

fn load_index() -> SavedSqlIndex {
    index_path()
        .and_then(|path| std::fs::read_to_string(&path).ok())
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or_default()
}

fn save_index(index: &SavedSqlIndex) {
    let Some(path) = index_path() else {
        return;
    };
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if let Ok(text) = serde_json::to_string_pretty(index) {
        let _ = std::fs::write(&path, text);
    }
}

fn content_hash(content: &str) -> String {
    CloudSyncService::calculate_blob_checksum(content)
}

/// 枚举 `queries/` 下的全部 SQL，附带稳定 `document_id`。
///
/// - 路径没变 ⇒ 沿用原 id（并刷新哈希）；
/// - 原 id 的路径已消失、且出现内容一致的新路径 ⇒ 判定为重命名，id 继承给新路径；
/// - 其余新路径 ⇒ 分配新 id。新分配的 id 会**写回索引**，保证下一次枚举不变。
pub fn saved_sql_documents() -> Vec<SavedSqlDocument> {
    let Some(root) = saved_sql_root() else {
        return Vec::new();
    };
    let files = collect_sql_files(&root);
    let mut index = load_index();
    let mut changed = false;

    let existing_paths = files
        .iter()
        .map(|(relative, _, _)| relative.as_str())
        .collect::<HashSet<_>>();
    let id_by_path = index
        .documents
        .iter()
        .map(|(id, record)| (record.path.clone(), id.clone()))
        .collect::<BTreeMap<_, _>>();
    // 路径已消失的条目：只能被继承一次，用完即移除，避免两份文件抢同一个 id
    let mut orphaned = index
        .documents
        .iter()
        .filter(|(_, record)| !existing_paths.contains(record.path.as_str()))
        .map(|(id, record)| (id.clone(), record.content_hash.clone()))
        .collect::<Vec<_>>();

    let mut documents = Vec::new();
    for (relative, content, modified_at) in &files {
        let hash = content_hash(content);
        let id = match id_by_path.get(relative) {
            Some(id) => id.clone(),
            None => match orphaned
                .iter()
                .position(|(_, candidate)| candidate == &hash)
            {
                Some(position) => orphaned.remove(position).0,
                None => uuid::Uuid::new_v4().to_string(),
            },
        };
        let needs_update = match index.documents.get(&id) {
            Some(record) => {
                record.path.as_str() != relative.as_str() || record.content_hash != hash
            }
            None => true,
        };
        if needs_update {
            index.documents.insert(
                id.clone(),
                DocumentRecord {
                    path: relative.clone(),
                    content_hash: hash,
                },
            );
            changed = true;
        }
        documents.push(SavedSqlDocument {
            document_id: id,
            relative_path: relative.clone(),
            modified_at: *modified_at,
            content: content.clone(),
        });
    }

    // 清掉确实被删除的条目，避免索引无限膨胀
    let before = index.documents.len();
    index
        .documents
        .retain(|_, record| existing_paths.contains(record.path.as_str()));
    changed |= before != index.documents.len();

    if changed {
        save_index(&index);
    }
    documents.sort_by(|a, b| a.relative_path.cmp(&b.relative_path));
    documents
}

/// 按稳定 `document_id` 写入一篇 SQL。
///
/// 该 id 已登记在别的路径时先移动文件（重命名语义），不会留下重复副本。
pub fn write_saved_sql_document(document: &SavedSqlDocument) {
    let Some(root) = saved_sql_root() else {
        return;
    };
    let mut index = load_index();
    // id 不是合法 UUID（早期/异常记录）时兜底分配一个新的：文件照写
    let id = match uuid::Uuid::parse_str(&document.document_id) {
        Ok(_) => document.document_id.clone(),
        Err(_) => uuid::Uuid::new_v4().to_string(),
    };
    let target = root.join(&document.relative_path);

    if let Some(record) = index.documents.get(&id) {
        if record.path != document.relative_path {
            let source = root.join(&record.path);
            if source.is_file() {
                if let Some(parent) = target.parent() {
                    let _ = std::fs::create_dir_all(parent);
                }
                let _ = std::fs::rename(&source, &target);
            }
        }
    }

    if let Some(parent) = target.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if std::fs::write(&target, document.content.as_bytes()).is_err() {
        return;
    }

    index.documents.insert(
        id,
        DocumentRecord {
            path: document.relative_path.clone(),
            content_hash: content_hash(&document.content),
        },
    );
    save_index(&index);
}

/// 按稳定 `document_id` 删除一篇 SQL；返回是否确实删掉了。
pub fn delete_saved_sql_document(document_id: &str) -> bool {
    let Some(root) = saved_sql_root() else {
        return false;
    };
    let mut index = load_index();
    let Some(record) = index.documents.remove(document_id) else {
        return false;
    };
    let _ = std::fs::remove_file(root.join(&record.path));
    save_index(&index);
    true
}

/// 递归收集 `queries/` 下的 `.sql` 文件，返回（相对路径, 内容, mtime 秒）
fn collect_sql_files(root: &Path) -> Vec<(String, String, i64)> {
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
            if path.extension().and_then(|e| e.to_str()) != Some("sql") {
                continue;
            }
            let Ok(content) = std::fs::read_to_string(&path) else {
                continue;
            };
            let Ok(relative) = path.strip_prefix(root) else {
                continue;
            };
            results.push((
                relative.to_string_lossy().replace('\\', "/"),
                content,
                modified_seconds(&path),
            ));
        }
    }
    results
}

/// 文件修改时间（秒）；取不到时回退为 0，交给同步层按内容判定。
fn modified_seconds(path: &Path) -> i64 {
    std::fs::metadata(path)
        .ok()
        .and_then(|meta| meta.modified().ok())
        .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
        .map(|duration| duration.as_secs() as i64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    // 索引依赖真实配置目录，单测会污染用户环境：这里只覆盖纯函数行为。
    use super::content_hash;

    #[test]
    fn content_hash_is_stable_and_content_sensitive() {
        assert_eq!(content_hash("SELECT 1"), content_hash("SELECT 1"));
        assert_ne!(content_hash("SELECT 1"), content_hash("SELECT 2"));
    }
}
