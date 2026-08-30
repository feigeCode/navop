use crate::document_index::{DOCUMENT_INDEX_FILE, DocumentIndex, PendingDocumentOperation};
use crate::path_policy::document_file_name;
use crate::{DocumentFormat, NotesStorage, validate_node_name};
use anyhow::Result;
use std::path::{Path, PathBuf};

#[test]
fn creates_and_manages_markdown_notebook_tree() -> Result<()> {
    let temp = tempfile::tempdir()?;
    let storage = NotesStorage::open(temp.path().join("notes"))?;
    let metadata = storage.create_notebook("My Notes", "Local")?;
    assert_eq!(Some(metadata), storage.load_notebook()?);
    let work = storage.create_directory(Path::new(""), "工作")?;
    let document = storage.create_document(&work, "项目计划")?;
    assert_eq!(Path::new("工作/项目计划.md"), document.relative_path);
    let renamed = storage.rename_node(&document.relative_path, "计划")?;
    assert_eq!(Path::new("工作/计划.md"), renamed);
    assert_eq!(
        document.document_id,
        storage.descriptor(&renamed)?.document_id
    );
    assert_eq!(1, storage.delete_node(&work)?.documents);
    Ok(())
}

#[test]
fn configured_root_defaults_and_persists_custom_location() -> Result<()> {
    let temp = tempfile::tempdir()?;
    let config = temp.path().join("config/notes-location.json");
    let default = temp.path().join("default-notes");
    assert!(!NotesStorage::has_configured_root_at(&config)?);
    assert_eq!(
        default,
        NotesStorage::configured_root_from(&config, &default)?
    );
    let custom = temp.path().join("custom-notes");
    std::fs::create_dir_all(&custom)?;
    NotesStorage::save_configured_root_to(&config, &custom)?;
    assert!(NotesStorage::has_configured_root_at(&config)?);
    assert_eq!(
        custom.canonicalize()?,
        NotesStorage::configured_root_from(&config, &default)?
    );
    Ok(())
}

#[test]
fn creating_notebook_does_not_overwrite_existing_notebook() -> Result<()> {
    let temp = tempfile::tempdir()?;
    let storage = NotesStorage::open(temp.path().join("notes"))?;
    let created = storage.create_notebook("Original", "kept")?;
    assert!(storage.create_notebook("Replacement", "lost").is_err());
    assert_eq!(Some(created), storage.load_notebook()?);
    Ok(())
}

#[test]
fn markdown_documents_use_stable_index_ids() -> Result<()> {
    let temp = tempfile::tempdir()?;
    let storage = NotesStorage::open(temp.path().join("notes"))?;
    storage.create_notebook("Notes", "")?;
    let created = storage.create_document(Path::new(""), "README")?;
    assert_eq!(Path::new("README.md"), created.relative_path);
    assert_eq!(DocumentFormat::Markdown, created.format);
    let renamed = storage.rename_node(&created.relative_path, "Guide")?;
    let reopened = storage.descriptor(&renamed)?;
    assert_eq!(created.document_id, reopened.document_id);
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
fn scan_ignores_legacy_cditor_documents() -> Result<()> {
    let temp = tempfile::tempdir()?;
    let root = temp.path().join("notes");
    let storage = NotesStorage::open(root.clone())?;
    storage.create_notebook("Notes", "")?;
    std::fs::write(root.join("files/legacy.cditor.json"), "{}")?;
    assert!(
        storage
            .scan_tree()?
            .iter()
            .all(|node| node.display_name != "legacy")
    );
    Ok(())
}

#[test]
fn pending_rename_finishes_after_file_move() -> Result<()> {
    let temp = tempfile::tempdir()?;
    let root = temp.path().join("notes");
    let storage = NotesStorage::open(root.clone())?;
    storage.create_notebook("Notes", "")?;
    let document = storage.create_document(Path::new(""), "before")?;
    let mut index = DocumentIndex::load(&root.join(DOCUMENT_INDEX_FILE))?;
    index.pending_operation = Some(PendingDocumentOperation::Rename {
        from: PathBuf::from("before.md"),
        to: PathBuf::from("after.md"),
    });
    index.save(&root.join(DOCUMENT_INDEX_FILE))?;
    std::fs::rename(root.join("files/before.md"), root.join("files/after.md"))?;
    assert_eq!(
        document.document_id,
        storage.descriptor(Path::new("after.md"))?.document_id
    );
    Ok(())
}

#[test]
fn rejects_unsafe_names_and_paths() -> Result<()> {
    for name in ["", "..", "a/b", "a.md"] {
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

#[test]
fn document_names_accept_the_markdown_extension() -> Result<()> {
    assert_eq!(
        "Guide.md",
        document_file_name("Guide.md", DocumentFormat::Markdown)?
    );
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

#[test]
fn user_selected_location_stores_content_directly_without_files_dir() -> Result<()> {
    let temp = tempfile::tempdir()?;
    let root = temp.path().join("my-notes");
    let storage = NotesStorage::open_with_layout(root.clone(), false)?;
    storage.create_notebook("My Notes", "Local")?;
    let document = storage.create_document(Path::new(""), "README")?;
    assert_eq!(Path::new("README.md"), document.relative_path);
    assert_eq!(
        document.document_id,
        storage.descriptor(Path::new("README.md"))?.document_id
    );
    assert!(root.join("notebook.json").exists());
    assert!(root.join("README.md").exists());
    assert!(!root.join("files").exists());
    assert!(storage
        .scan_tree()?
        .iter()
        .any(|node| node.display_name == "README"));
    Ok(())
}

#[test]
fn default_location_keeps_files_subdir_layout() -> Result<()> {
    let temp = tempfile::tempdir()?;
    let root = temp.path().join("notes");
    let storage = NotesStorage::open(root.clone())?;
    storage.create_notebook("Notes", "")?;
    storage.create_document(Path::new(""), "README")?;
    assert!(root.join("files").exists());
    assert!(root.join("files").join("README.md").exists());
    assert!(!root.join("files").join("notebook.json").exists());
    Ok(())
}

#[test]
fn legacy_location_config_without_flag_keeps_files_layout() -> Result<()> {
    let temp = tempfile::tempdir()?;
    let config = temp.path().join("config/notes-location.json");
    let default = temp.path().join("default-notes");
    let custom = temp.path().join("custom-notes");
    std::fs::create_dir_all(config.parent().unwrap())?;
    std::fs::create_dir_all(&custom)?;
    std::fs::write(&config, format!(r#"{{"root":"{}"}}"#, custom.display()))?;

    let (root, use_files_subdir) = NotesStorage::configured_location_from(&config, &default)?;
    assert_eq!(custom, root);
    assert!(use_files_subdir);
    Ok(())
}

#[test]
fn saving_location_keeps_files_layout_only_for_legacy_data() -> Result<()> {
    let temp = tempfile::tempdir()?;
    let config = temp.path().join("config/notes-location.json");
    let default = temp.path().join("default-notes");

    // 目录里已有旧版 files/notebook.json 数据：沿用 files/ 布局
    let legacy = temp.path().join("legacy-notes");
    std::fs::create_dir_all(legacy.join("files"))?;
    std::fs::write(legacy.join("files").join("notebook.json"), "{}")?;
    NotesStorage::save_configured_root_to(&config, &legacy)?;
    let (_, use_files_subdir) = NotesStorage::configured_location_from(&config, &default)?;
    assert!(use_files_subdir);

    // 全新的用户选择目录：直接作为笔记根
    let fresh = temp.path().join("fresh-notes");
    std::fs::create_dir_all(&fresh)?;
    NotesStorage::save_configured_root_to(&config, &fresh)?;
    let (_, use_files_subdir) = NotesStorage::configured_location_from(&config, &default)?;
    assert!(!use_files_subdir);
    Ok(())
}
