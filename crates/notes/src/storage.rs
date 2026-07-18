use crate::document_index::{DOCUMENT_INDEX_FILE, DocumentIndex, PendingDocumentOperation};
use crate::model::{
    DeleteSummary, DocumentDescriptor, DocumentFormat, FileNode, NotebookMetadata, NotebookUiState,
};
use crate::path_policy::{document_file_name, validate_node_name, validate_relative_path};
use crate::storage_support::{
    collect_documents, count_nodes, document_format, read_optional_json, recover_pending_operation,
    scan_directory, write_json_atomic, write_text_atomic,
};
use anyhow::{Context, Result, bail};
use cditor_app::EditorDocument;
use std::fs;
use std::path::{Path, PathBuf};
use uuid::Uuid;

const NOTEBOOK_FILE: &str = "notebook.json";
const STATE_FILE: &str = "state.json";
const FILES_DIR: &str = "files";

#[derive(Debug, Clone)]
pub struct NotesStorage {
    root: PathBuf,
}

impl NotesStorage {
    pub fn open(root: PathBuf) -> Result<Self> {
        fs::create_dir_all(&root).with_context(|| format!("create {}", root.display()))?;
        Ok(Self { root })
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
        self.create_document_with_format(parent, name, DocumentFormat::RichText)
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
        match format {
            DocumentFormat::RichText => {
                let document = EditorDocument::from_markdown(&document_id, "")?;
                write_text_atomic(&path, &document.to_json()?)?;
            }
            DocumentFormat::Markdown => write_text_atomic(&path, "")?,
        }
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
        self.root.join(FILES_DIR)
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

    fn document_id(
        &self,
        relative_path: &Path,
        absolute_path: &Path,
        format: DocumentFormat,
    ) -> Result<String> {
        let mut index = self.load_index()?;
        let id = match format {
            DocumentFormat::RichText => {
                EditorDocument::from_json(&fs::read_to_string(absolute_path)?)?
                    .document_id
                    .parse()?
            }
            DocumentFormat::Markdown => index
                .documents
                .get(relative_path)
                .map(|record| record.id)
                .unwrap_or_else(Uuid::new_v4),
        };
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
            let absolute = self.files_root().join(path);
            let id = match format {
                DocumentFormat::RichText => {
                    EditorDocument::from_json(&fs::read_to_string(absolute)?)?
                        .document_id
                        .parse()?
                }
                DocumentFormat::Markdown => Uuid::new_v4(),
            };
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
