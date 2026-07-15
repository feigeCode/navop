use crate::{NotesStorage, validate_node_name};
use anyhow::Result;
use std::path::Path;

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
