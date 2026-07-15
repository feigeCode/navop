use crate::model::{
    DeleteSummary, DocumentDescriptor, FileNode, NodeKind, NotebookMetadata, NotebookUiState,
};
use crate::path_policy::{
    document_display_name, document_file_name, validate_node_name, validate_relative_path,
};
use anyhow::{Context, Result, bail};
use cditor_app::EditorDocument;
use serde::{Serialize, de::DeserializeOwned};
use std::fs::{self, File};
use std::io::Write;
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

    pub fn default_root() -> Result<PathBuf> {
        let base = dirs::data_local_dir().context("local data directory is unavailable")?;
        Ok(base.join("navop").join("notes"))
    }

    pub fn create_notebook(&self, name: &str, description: &str) -> Result<NotebookMetadata> {
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
        let parent = self.resolve_directory(parent)?;
        let path = parent.join(document_file_name(name)?);
        if path.exists() {
            bail!("document already exists: {}", path.display());
        }
        let document_id = Uuid::new_v4().to_string();
        let document = EditorDocument::from_markdown(&document_id, "")?;
        write_text_atomic(&path, &document.to_json()?)?;
        Ok(DocumentDescriptor {
            document_id,
            relative_path: self.relative_to_files(&path)?,
            absolute_path: path,
        })
    }

    pub fn descriptor(&self, relative_path: &Path) -> Result<DocumentDescriptor> {
        let path = self.resolve_existing(relative_path)?;
        if !path.is_file() {
            bail!("not a document: {}", relative_path.display());
        }
        let document = EditorDocument::from_json(&fs::read_to_string(&path)?)?;
        Ok(DocumentDescriptor {
            document_id: document.document_id,
            relative_path: validate_relative_path(relative_path)?,
            absolute_path: path,
        })
    }

    pub fn rename_node(&self, relative_path: &Path, new_name: &str) -> Result<PathBuf> {
        let source = self.resolve_existing(relative_path)?;
        let parent = source.parent().context("node has no parent")?;
        let target_name = if source.is_dir() {
            validate_node_name(new_name)?.to_owned()
        } else {
            document_file_name(new_name)?
        };
        let target = parent.join(target_name);
        if target.exists() {
            bail!("target already exists: {}", target.display());
        }
        fs::rename(&source, &target)?;
        self.relative_to_files(&target)
    }

    pub fn delete_node(&self, relative_path: &Path) -> Result<DeleteSummary> {
        let path = self.resolve_existing(relative_path)?;
        let summary = count_nodes(&path)?;
        if path.is_dir() {
            fs::remove_dir_all(path)?;
        } else {
            fs::remove_file(path)?;
        }
        Ok(summary)
    }

    pub fn scan_tree(&self) -> Result<Vec<FileNode>> {
        fs::create_dir_all(self.files_root())?;
        scan_directory(&self.files_root(), &self.files_root())
    }

    fn files_root(&self) -> PathBuf {
        self.root.join(FILES_DIR)
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

fn scan_directory(root: &Path, directory: &Path) -> Result<Vec<FileNode>> {
    let mut nodes = Vec::new();
    for entry in fs::read_dir(directory)? {
        let entry = entry?;
        let metadata = fs::symlink_metadata(entry.path())?;
        if metadata.file_type().is_symlink() {
            tracing::warn!(path = %entry.path().display(), "ignoring notes symlink");
            continue;
        }
        let name = entry.file_name().to_string_lossy().into_owned();
        let relative_path = entry.path().strip_prefix(root)?.to_path_buf();
        if metadata.is_dir() {
            nodes.push(FileNode {
                relative_path,
                display_name: name,
                kind: NodeKind::Directory,
                children: scan_directory(root, &entry.path())?,
            });
        } else if let Some(display_name) = document_display_name(&name) {
            nodes.push(FileNode {
                relative_path,
                display_name: display_name.to_owned(),
                kind: NodeKind::Document,
                children: Vec::new(),
            });
        }
    }
    nodes.sort_by(|a, b| {
        (a.kind != NodeKind::Directory, &a.display_name)
            .cmp(&(b.kind != NodeKind::Directory, &b.display_name))
    });
    Ok(nodes)
}

fn count_nodes(path: &Path) -> Result<DeleteSummary> {
    let metadata = fs::symlink_metadata(path)?;
    if metadata.file_type().is_symlink() {
        return Ok(DeleteSummary::default());
    }
    if metadata.is_file() {
        return Ok(DeleteSummary {
            documents: 1,
            directories: 0,
        });
    }
    let mut summary = DeleteSummary {
        directories: 1,
        documents: 0,
    };
    for entry in fs::read_dir(path)? {
        let child = count_nodes(&entry?.path())?;
        summary.directories += child.directories;
        summary.documents += child.documents;
    }
    Ok(summary)
}

pub(crate) fn write_text_atomic(path: &Path, text: &str) -> Result<()> {
    let name = path
        .file_name()
        .context("file has no name")?
        .to_string_lossy();
    let temporary = path.with_file_name(format!(".{name}.tmp"));
    let mut file = File::create(&temporary)?;
    file.write_all(text.as_bytes())?;
    file.flush()?;
    fs::rename(temporary, path)?;
    Ok(())
}

fn write_json_atomic(path: &Path, value: &impl Serialize) -> Result<()> {
    write_text_atomic(path, &serde_json::to_string_pretty(value)?)
}

fn read_optional_json<T: DeserializeOwned>(path: &Path) -> Result<Option<T>> {
    if !path.exists() {
        return Ok(None);
    }
    Ok(Some(serde_json::from_str(&fs::read_to_string(path)?)?))
}
