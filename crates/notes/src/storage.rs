use crate::document_index::{DOCUMENT_INDEX_FILE, DocumentIndex, PendingDocumentOperation};
use crate::model::{
    DeleteSummary, DocumentDescriptor, DocumentFormat, FileNode, NotebookMetadata, NotebookUiState,
    NoteDocument,
};
use crate::path_policy::{document_file_name, validate_node_name, validate_relative_path};
use crate::storage_support::{
    collect_documents, count_nodes, document_format, read_optional_json, recover_pending_operation,
    scan_directory, write_json_atomic, write_text_atomic,
};
use anyhow::{Context, Result, bail};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;
use uuid::Uuid;

const NOTEBOOK_FILE: &str = "notebook.json";
const STATE_FILE: &str = "state.json";
const FILES_DIR: &str = "files";

#[derive(Debug, Clone)]
pub struct NotesStorage {
    root: PathBuf,
    /// 内容树是否放在 root 下的 `files/` 子目录。
    ///
    /// 默认存储位置保持 `files/` 布局；用户显式选择的自定义目录直接作为
    /// 笔记根，不再额外创建 `files/` 子目录（见 `storage_location`）。
    use_files_subdir: bool,
}

impl NotesStorage {
    pub fn open(root: PathBuf) -> Result<Self> {
        Self::open_with_layout(root, true)
    }

    pub fn open_with_layout(root: PathBuf, use_files_subdir: bool) -> Result<Self> {
        fs::create_dir_all(&root).with_context(|| format!("create {}", root.display()))?;
        Ok(Self {
            root,
            use_files_subdir,
        })
    }

    pub fn create_notebook(&self, name: &str, description: &str) -> Result<NotebookMetadata> {
        if self.load_notebook()?.is_some() {
            bail!("notebook already exists: {}", self.root.display());
        }
        let name = validate_node_name(name)?;
        let now = chrono::Utc::now();
        let metadata = NotebookMetadata {
            id: Uuid::new_v4(),
            name: name.to_owned(),
            description: description.trim().to_owned(),
            created_at: now,
            updated_at: now,
        };
        fs::create_dir_all(self.files_root())?;
        write_json_atomic(&self.root.join(NOTEBOOK_FILE), &metadata)?;
        write_json_atomic(&self.root.join(STATE_FILE), &NotebookUiState::default())?;
        DocumentIndex::default().save(&self.index_path())?;
        self.create_document(Path::new(""), "欢迎")?;
        Ok(metadata)
    }

    pub fn load_notebook(&self) -> Result<Option<NotebookMetadata>> {
        read_optional_json(&self.root.join(NOTEBOOK_FILE))
    }

    pub fn load_state(&self) -> Result<NotebookUiState> {
        Ok(read_optional_json(&self.root.join(STATE_FILE))?.unwrap_or_default())
    }

    pub fn save_state(&self, state: &NotebookUiState) -> Result<()> {
        write_json_atomic(&self.root.join(STATE_FILE), state)
    }

    pub fn create_directory(&self, parent: &Path, name: &str) -> Result<PathBuf> {
        let parent = self.resolve_directory(parent)?;
        let name = validate_node_name(name)?;
        let path = parent.join(name);
        fs::create_dir(&path).with_context(|| format!("create {}", path.display()))?;
        self.relative_to_files(&path)
    }

    pub fn create_document(&self, parent: &Path, name: &str) -> Result<DocumentDescriptor> {
        self.create_document_with_format(parent, name, DocumentFormat::Markdown)
    }

    pub fn create_document_with_format(
        &self,
        parent: &Path,
        name: &str,
        format: DocumentFormat,
    ) -> Result<DocumentDescriptor> {
        let parent = self.resolve_directory(parent)?;
        let path = parent.join(document_file_name(name, format)?);
        if path.exists() {
            bail!("document already exists: {}", path.display());
        }
        let document_id = Uuid::new_v4().to_string();
        write_text_atomic(&path, "")?;
        let descriptor = DocumentDescriptor {
            document_id,
            format,
            relative_path: self.relative_to_files(&path)?,
            absolute_path: path,
        };
        self.record_descriptor(&descriptor)?;
        Ok(descriptor)
    }

    pub fn descriptor(&self, relative_path: &Path) -> Result<DocumentDescriptor> {
        let path = self.resolve_existing(relative_path)?;
        if !path.is_file() {
            bail!("not a document: {}", relative_path.display());
        }
        let relative_path = validate_relative_path(relative_path)?;
        let format = document_format(&path)?;
        let document_id = self.document_id(&relative_path, &path, format)?;
        Ok(DocumentDescriptor {
            document_id,
            format,
            relative_path,
            absolute_path: path,
        })
    }

    pub(crate) fn absolute_path(&self, relative_path: &Path) -> Result<PathBuf> {
        if relative_path.as_os_str().is_empty() {
            self.resolve_directory(relative_path)
        } else {
            self.resolve_existing(relative_path)
        }
    }

    pub fn rename_node(&self, relative_path: &Path, new_name: &str) -> Result<PathBuf> {
        let source = self.resolve_existing(relative_path)?;
        let parent = source.parent().context("node has no parent")?;
        let relative_source = validate_relative_path(relative_path)?;
        let target_name = if source.is_dir() {
            validate_node_name(new_name)?.to_owned()
        } else {
            document_file_name(new_name, document_format(&source)?)?
        };
        let target = parent.join(target_name);
        if target.exists() {
            bail!("target already exists: {}", target.display());
        }
        let relative_target = self.relative_to_files(&target)?;
        let mut index = self.load_index()?;
        index.pending_operation = Some(PendingDocumentOperation::Rename {
            from: relative_source.clone(),
            to: relative_target.clone(),
        });
        index.save(&self.index_path())?;
        fs::rename(&source, &target)?;
        index.remap_prefix(&relative_source, &relative_target);
        index.pending_operation = None;
        index.save(&self.index_path())?;
        Ok(relative_target)
    }

    pub fn delete_node(&self, relative_path: &Path) -> Result<DeleteSummary> {
        let relative_path = validate_relative_path(relative_path)?;
        let path = self.resolve_existing(&relative_path)?;
        let summary = count_nodes(&path)?;
        let mut index = self.load_index()?;
        index.pending_operation = Some(PendingDocumentOperation::Delete {
            path: relative_path.clone(),
        });
        index.save(&self.index_path())?;
        if path.is_dir() {
            fs::remove_dir_all(path)?;
        } else {
            fs::remove_file(path)?;
        }
        index.remove_prefix(&relative_path);
        index.pending_operation = None;
        index.save(&self.index_path())?;
        Ok(summary)
    }

    pub fn scan_tree(&self) -> Result<Vec<FileNode>> {
        fs::create_dir_all(self.files_root())?;
        let nodes = scan_directory(&self.files_root(), &self.files_root())?;
        self.reconcile_index(&nodes)?;
        Ok(nodes)
    }

    fn files_root(&self) -> PathBuf {
        if self.use_files_subdir {
            self.root.join(FILES_DIR)
        } else {
            self.root.clone()
        }
    }

    fn index_path(&self) -> PathBuf {
        self.root.join(DOCUMENT_INDEX_FILE)
    }

    pub(crate) fn record_descriptor(&self, descriptor: &DocumentDescriptor) -> Result<()> {
        let mut index = self.load_index()?;
        index.record(
            descriptor.relative_path.clone(),
            Uuid::parse_str(&descriptor.document_id)?,
            descriptor.format,
        );
        index.save(&self.index_path())
    }

    // ---------- 同步专用：以稳定 document_id 为身份 ----------

    /// 枚举全部文档，附带持久化索引里的稳定 `document_id`。
    ///
    /// 索引里还没有的文档会被分配一个新 UUID 并**写回索引**，保证下一次枚举
    /// 仍是同一个 id——身份一旦确定就不再随路径变化。
    pub fn sync_documents(&self) -> Result<Vec<NoteDocument>> {
        let files_root = self.files_root();
        if !files_root.exists() {
            return Ok(Vec::new());
        }
        let nodes = scan_directory(&files_root, &files_root)?;
        let mut index = self.load_index()?;
        let mut changed = false;
        let mut documents = Vec::new();

        for (relative_path, format) in collect_documents(&nodes) {
            let absolute = files_root.join(&relative_path);
            let Ok(content) = fs::read_to_string(&absolute) else {
                continue;
            };
            // 先取出 id 再登记，避免在持有索引借用的同时可变借用索引
            let known = index.documents.get(&relative_path).map(|record| record.id);
            let id = match known {
                Some(id) => id,
                None => {
                    let id = Uuid::new_v4();
                    index.record(relative_path.clone(), id, format);
                    changed = true;
                    id
                }
            };
            documents.push(NoteDocument {
                document_id: id.to_string(),
                relative_path: relative_path.to_string_lossy().replace('\\', "/"),
                modified_at: modified_seconds(&absolute),
                content,
            });
        }

        if changed {
            index.save(&self.index_path())?;
        }
        documents.sort_by(|a, b| a.relative_path.cmp(&b.relative_path));
        Ok(documents)
    }

    /// 按稳定 `document_id` 写入一篇笔记。
    ///
    /// 三种情形：
    /// - 该 id 已登记在别的路径 ⇒ 把文件挪到新路径再写内容（即「重命名」，
    ///   不会留下旧副本）；
    /// - 目标路径已存在但登记了别的 id ⇒ 覆盖成传入 id；
    /// - 完全陌生 ⇒ 创建文件并登记 id。
    ///
    /// 这样对端把「同一篇笔记换了路径」同步过来时，本地表现为重命名而非新增。
    pub fn sync_write_document(&self, document: &NoteDocument) -> Result<()> {
        // id 不是合法 UUID（例如早期/异常记录）时兜底分配一个新的：文件照写，
        // 之后这篇笔记就有了稳定身份，不会因此写不进去。
        let id = Uuid::parse_str(&document.document_id).unwrap_or_else(|_| Uuid::new_v4());
        let relative = PathBuf::from(&document.relative_path);
        let target = self.files_root().join(&relative);
        let mut index = self.load_index()?;

        if let Some(previous) = index_path_by_id(&index, id) {
            if previous != relative {
                let source = self.files_root().join(&previous);
                if source.is_file() {
                    if let Some(parent) = target.parent() {
                        fs::create_dir_all(parent)?;
                    }
                    fs::rename(&source, &target)?;
                }
                index.documents.remove(&previous);
            }
        }

        index.record(relative.clone(), id, DocumentFormat::Markdown);
        index.save(&self.index_path())?;

        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent)?;
        }
        write_text_atomic(&target, &document.content)?;
        Ok(())
    }

    /// 按稳定 `document_id` 删除一篇笔记；返回是否确实删掉了。
    pub fn sync_delete_document(&self, document_id: &str) -> Result<bool> {
        let Ok(id) = Uuid::parse_str(document_id) else {
            return Ok(false);
        };
        let mut index = self.load_index()?;
        let Some(path) = index_path_by_id(&index, id) else {
            return Ok(false);
        };
        let absolute = self.files_root().join(&path);
        if absolute.is_file() {
            fs::remove_file(&absolute)?;
        }
        index.documents.remove(&path);
        index.save(&self.index_path())?;
        Ok(true)
    }

    fn document_id(
        &self,
        relative_path: &Path,
        absolute_path: &Path,
        format: DocumentFormat,
    ) -> Result<String> {
        let mut index = self.load_index()?;
        let _ = absolute_path;
        let id = index
            .documents
            .get(relative_path)
            .map(|record| record.id)
            .unwrap_or_else(Uuid::new_v4);
        let needs_update = index
            .documents
            .get(relative_path)
            .is_none_or(|record| record.id != id || record.format != format);
        if needs_update {
            index.record(relative_path.to_path_buf(), id, format);
            index.save(&self.index_path())?;
        }
        Ok(id.to_string())
    }

    fn load_index(&self) -> Result<DocumentIndex> {
        let mut index = DocumentIndex::load(&self.index_path())?;
        if recover_pending_operation(&self.files_root(), &mut index)? {
            index.save(&self.index_path())?;
        }
        Ok(index)
    }

    fn reconcile_index(&self, nodes: &[FileNode]) -> Result<()> {
        let mut index = self.load_index()?;
        let documents = collect_documents(nodes);
        let mut changed = false;
        for (path, format) in &documents {
            if index.documents.contains_key(path) {
                continue;
            }
            let id = Uuid::new_v4();
            index.record(path.clone(), id, *format);
            changed = true;
        }
        let before = index.documents.len();
        index
            .documents
            .retain(|path, _| documents.contains_key(path));
        changed |= before != index.documents.len();
        if changed {
            index.save(&self.index_path())?;
        }
        Ok(())
    }

    fn resolve_directory(&self, relative: &Path) -> Result<PathBuf> {
        fs::create_dir_all(self.files_root())?;
        let path = self.files_root().join(validate_relative_path(relative)?);
        let canonical = path
            .canonicalize()
            .with_context(|| format!("open {}", path.display()))?;
        let root = self.files_root().canonicalize()?;
        if !canonical.starts_with(&root) || !canonical.is_dir() {
            bail!("directory escapes notes root");
        }
        Ok(canonical)
    }

    fn resolve_existing(&self, relative: &Path) -> Result<PathBuf> {
        let clean = validate_relative_path(relative)?;
        if clean.as_os_str().is_empty() {
            bail!("notes root cannot be modified");
        }
        let path = self.files_root().join(clean);
        let metadata = fs::symlink_metadata(&path)?;
        if metadata.file_type().is_symlink() {
            bail!("symbolic links are not allowed");
        }
        let canonical = path.canonicalize()?;
        if !canonical.starts_with(self.files_root().canonicalize()?) {
            bail!("path escapes notes root");
        }
        Ok(canonical)
    }

    fn relative_to_files(&self, path: &Path) -> Result<PathBuf> {
        Ok(path
            .strip_prefix(self.files_root().canonicalize()?)?
            .to_path_buf())
    }
}

/// 反向查询：某个稳定 `document_id` 当前登记在哪个路径。
fn index_path_by_id(index: &DocumentIndex, id: Uuid) -> Option<PathBuf> {
    index
        .documents
        .iter()
        .find(|(_, record)| record.id == id)
        .map(|(path, _)| path.clone())
}

/// 文件修改时间（秒）；取不到时回退为 0，交给同步层按内容判定。
fn modified_seconds(path: &Path) -> i64 {
    fs::metadata(path)
        .ok()
        .and_then(|meta| meta.modified().ok())
        .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
        .map(|duration| duration.as_secs() as i64)
        .unwrap_or(0)
}
