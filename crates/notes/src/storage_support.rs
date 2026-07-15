use crate::document_index::{DocumentIndex, PendingDocumentOperation};
use crate::path_policy::document_display_name;
use crate::{DeleteSummary, DocumentFormat, FileNode, NodeKind};
use anyhow::{Context, Result, bail};
use serde::{Serialize, de::DeserializeOwned};
use std::collections::BTreeMap;
use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};
use uuid::Uuid;

pub(crate) fn scan_directory(root: &Path, directory: &Path) -> Result<Vec<FileNode>> {
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
                format: None,
                children: scan_directory(root, &entry.path())?,
            });
        } else if let Some((display_name, format)) = document_display_name(&name) {
            nodes.push(FileNode {
                relative_path,
                display_name: display_name.to_owned(),
                kind: NodeKind::Document,
                format: Some(format),
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

pub(crate) fn collect_documents(nodes: &[FileNode]) -> BTreeMap<PathBuf, DocumentFormat> {
    let mut documents = BTreeMap::new();
    for node in nodes {
        if let Some(format) = node.format {
            documents.insert(node.relative_path.clone(), format);
        }
        documents.extend(collect_documents(&node.children));
    }
    documents
}

pub(crate) fn document_format(path: &Path) -> Result<DocumentFormat> {
    let name = path
        .file_name()
        .context("document has no file name")?
        .to_string_lossy();
    document_display_name(&name)
        .map(|(_, format)| format)
        .context("unsupported notes document extension")
}

pub(crate) fn recover_pending_operation(root: &Path, index: &mut DocumentIndex) -> Result<bool> {
    let Some(operation) = index.pending_operation.clone() else {
        return Ok(false);
    };
    match operation {
        PendingDocumentOperation::Rename { from, to } => {
            recover_pending_rename(root, index, from, to)?;
        }
        PendingDocumentOperation::Delete { path } => {
            if !root.join(&path).exists() {
                index.remove_prefix(&path);
            }
        }
    }
    index.pending_operation = None;
    Ok(true)
}

fn recover_pending_rename(
    root: &Path,
    index: &mut DocumentIndex,
    from: PathBuf,
    to: PathBuf,
) -> Result<()> {
    match (root.join(&from).exists(), root.join(&to).exists()) {
        (false, true) => index.remap_prefix(&from, &to),
        (true, false) => {}
        _ => bail!(
            "ambiguous pending notes rename: {} -> {}",
            from.display(),
            to.display()
        ),
    }
    Ok(())
}

pub(crate) fn count_nodes(path: &Path) -> Result<DeleteSummary> {
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

pub(crate) fn write_text_atomic_new(path: &Path, text: &str) -> Result<()> {
    let name = path
        .file_name()
        .context("file has no name")?
        .to_string_lossy();
    let temporary = path.with_file_name(format!(".{name}.{}.tmp", Uuid::new_v4()));
    let result = write_new_file(&temporary, text).and_then(|_| {
        fs::hard_link(&temporary, path)
            .with_context(|| format!("create new file {}", path.display()))
    });
    let _ = fs::remove_file(temporary);
    result
}

fn write_new_file(path: &Path, text: &str) -> Result<()> {
    let mut file = File::options().write(true).create_new(true).open(path)?;
    file.write_all(text.as_bytes())?;
    file.flush()?;
    Ok(())
}

pub(crate) fn write_json_atomic(path: &Path, value: &impl Serialize) -> Result<()> {
    write_text_atomic(path, &serde_json::to_string_pretty(value)?)
}

pub(crate) fn read_optional_json<T: DeserializeOwned>(path: &Path) -> Result<Option<T>> {
    if !path.exists() {
        return Ok(None);
    }
    Ok(Some(serde_json::from_str(&fs::read_to_string(path)?)?))
}
