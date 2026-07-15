use crate::document_index::{DOCUMENT_INDEX_FILE, DocumentIndex, PendingDocumentOperation};
use crate::{DocumentFormat, NotesStorage, validate_node_name};
use anyhow::Result;
use std::path::{Path, PathBuf};

#[test]
fn creates_and_manages_notebook_tree() -> Result<()> {
    let temp = tempfile::tempdir()?;
    let storage = NotesStorage::open(temp.path().join("notes"))?;
    let metadata = storage.create_notebook("My Notes", "Local")?;
    assert_eq!(Some(metadata), storage.load_notebook()?);
    let work = storage.create_directory(Path::new(""), "工作")?;
    let document = storage.create_document(&work, "项目计划")?;
    assert_eq!(
        Path::new("工作/项目计划.cditor.json"),
        document.relative_path
    );
    let renamed = storage.rename_node(&document.relative_path, "计划")?;
    assert_eq!(Path::new("工作/计划.cditor.json"), renamed);
    assert_eq!(
        document.document_id,
        storage.descriptor(&renamed)?.document_id
    );
    assert_eq!(1, storage.delete_node(&work)?.documents);
    Ok(())
}

#[test]
fn markdown_documents_use_md_files_and_stable_index_ids() -> Result<()> {
    let temp = tempfile::tempdir()?;
    let storage = NotesStorage::open(temp.path().join("notes"))?;
    storage.create_notebook("Notes", "")?;
    let created =
        storage.create_document_with_format(Path::new(""), "README", DocumentFormat::Markdown)?;
    assert_eq!(Path::new("README.md"), created.relative_path);
    assert_eq!(DocumentFormat::Markdown, created.format);
    assert!(created.absolute_path.is_file());

    let renamed = storage.rename_node(&created.relative_path, "Guide")?;
    let reopened = storage.descriptor(&renamed)?;
    assert_eq!(created.document_id, reopened.document_id);
    assert_eq!(DocumentFormat::Markdown, reopened.format);
    Ok(())
}

#[test]
fn scan_discovers_external_markdown_once() -> Result<()> {
    let temp = tempfile::tempdir()?;
    let root = temp.path().join("notes");
    let storage = NotesStorage::open(root.clone())?;
    storage.create_notebook("Notes", "")?;
    std::fs::write(root.join("files/external.md"), "# External\n")?;

    let first = storage.descriptor(Path::new("external.md"))?;
    storage.scan_tree()?;
    let second = storage.descriptor(Path::new("external.md"))?;
    assert_eq!(first.document_id, second.document_id);
    Ok(())
}

#[test]
fn pending_rename_finishes_after_file_move() -> Result<()> {
    let temp = tempfile::tempdir()?;
    let root = temp.path().join("notes");
    let storage = NotesStorage::open(root.clone())?;
    storage.create_notebook("Notes", "")?;
    let document =
        storage.create_document_with_format(Path::new(""), "before", DocumentFormat::Markdown)?;
    let mut index = DocumentIndex::load(&root.join(DOCUMENT_INDEX_FILE))?;
    index.pending_operation = Some(PendingDocumentOperation::Rename {
        from: PathBuf::from("before.md"),
        to: PathBuf::from("after.md"),
    });
    index.save(&root.join(DOCUMENT_INDEX_FILE))?;
    std::fs::rename(root.join("files/before.md"), root.join("files/after.md"))?;

    let recovered = storage.descriptor(Path::new("after.md"))?;
    assert_eq!(document.document_id, recovered.document_id);
    Ok(())
}

#[test]
fn rich_text_descriptor_repairs_stale_index_id() -> Result<()> {
    let temp = tempfile::tempdir()?;
    let root = temp.path().join("notes");
    let storage = NotesStorage::open(root.clone())?;
    storage.create_notebook("Notes", "")?;
    let document = storage.create_document(Path::new(""), "native")?;
    let index_path = root.join(DOCUMENT_INDEX_FILE);
    let mut index = DocumentIndex::load(&index_path)?;
    index.record(
        document.relative_path.clone(),
        uuid::Uuid::new_v4(),
        DocumentFormat::RichText,
    );
    index.save(&index_path)?;

    let reopened = storage.descriptor(&document.relative_path)?;
    assert_eq!(document.document_id, reopened.document_id);
    let repaired = DocumentIndex::load(&index_path)?;
    assert_eq!(
        document.document_id,
        repaired.documents[&document.relative_path].id.to_string()
    );
    Ok(())
}

#[test]
fn rejects_unsafe_names_and_paths() -> Result<()> {
    for name in ["", "..", "a/b", "a.cditor.json"] {
        assert!(validate_node_name(name).is_err());
    }
    let temp = tempfile::tempdir()?;
    let storage = NotesStorage::open(temp.path().join("notes"))?;
    storage.create_notebook("Notes", "")?;
    assert!(
        storage
            .create_directory(Path::new("../escape"), "bad")
            .is_err()
    );
    assert!(storage.delete_node(Path::new("")).is_err());
    Ok(())
}

#[cfg(unix)]
#[test]
fn scan_ignores_symbolic_links() -> Result<()> {
    use std::os::unix::fs::symlink;

    let temp = tempfile::tempdir()?;
    let storage = NotesStorage::open(temp.path().join("notes"))?;
    storage.create_notebook("Notes", "")?;
    let outside = temp.path().join("outside");
    std::fs::create_dir(&outside)?;
    symlink(&outside, temp.path().join("notes/files/link"))?;
    assert!(
        storage
            .scan_tree()?
            .iter()
            .all(|node| node.display_name != "link")
    );
    Ok(())
}
