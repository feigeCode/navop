use super::{
    copy_entry, create_directory, create_file, delete_entry, move_entry, read_directory,
    rename_entry, root_ignore_matcher,
};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT_TEMP_ID: AtomicU64 = AtomicU64::new(0);

struct TestDirectory(PathBuf);

impl TestDirectory {
    fn new(label: &str) -> Self {
        let unique = NEXT_TEMP_ID.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "navop-workspace-{label}-{}-{unique}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&path);
        fs::create_dir_all(&path).unwrap();
        Self(path)
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TestDirectory {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn entry_names(entries: Vec<crate::model::ExplorerEntry>) -> Vec<String> {
    entries.into_iter().map(|entry| entry.name).collect()
}

#[test]
fn hidden_and_ignored_visibility_are_independent() {
    let temp = TestDirectory::new("explorer-filter");
    fs::write(temp.path().join(".gitignore"), "target\n").unwrap();
    fs::write(temp.path().join(".env.local"), "SECRET=test").unwrap();
    fs::create_dir(temp.path().join("target")).unwrap();
    fs::write(temp.path().join("Cargo.toml"), "[package]").unwrap();

    let matcher = root_ignore_matcher(temp.path()).unwrap();
    let default_names =
        entry_names(read_directory(temp.path(), Some(matcher.as_ref()), false, false).unwrap());
    assert_eq!(vec!["Cargo.toml"], default_names);

    let hidden_names =
        entry_names(read_directory(temp.path(), Some(matcher.as_ref()), true, false).unwrap());
    assert!(hidden_names.contains(&".env.local".to_string()));
    assert!(!hidden_names.contains(&"target".to_string()));

    let ignored_names =
        entry_names(read_directory(temp.path(), Some(matcher.as_ref()), false, true).unwrap());
    assert!(ignored_names.contains(&"target".to_string()));
    assert!(!ignored_names.contains(&".env.local".to_string()));
}

#[test]
fn basic_file_operations_create_rename_and_delete_entries() {
    let temp = TestDirectory::new("file-operations");

    let file = create_file(temp.path(), "new.txt").unwrap();
    let directory = create_directory(temp.path(), "folder").unwrap();
    let renamed = rename_entry(&file, "renamed.txt").unwrap();

    assert!(renamed.is_file());
    assert!(directory.is_dir());
    assert!(create_file(temp.path(), "../outside").is_err());

    delete_entry(&renamed).unwrap();
    delete_entry(&directory).unwrap();
    assert!(!renamed.exists());
    assert!(!directory.exists());
}

#[test]
fn copies_file_without_removing_source() {
    let temp = TestDirectory::new("copy-file");
    let source_dir = temp.path().join("source");
    let destination_dir = temp.path().join("destination");
    fs::create_dir_all(&source_dir).unwrap();
    fs::create_dir_all(&destination_dir).unwrap();
    let source = source_dir.join("notes.txt");
    fs::write(&source, "copied content").unwrap();

    let copied = copy_entry(&source, &destination_dir).unwrap();

    assert_eq!(destination_dir.join("notes.txt"), copied);
    assert_eq!("copied content", fs::read_to_string(&copied).unwrap());
    assert!(source.is_file());
}

#[test]
fn copies_directory_recursively() {
    let temp = TestDirectory::new("copy-directory");
    let source = temp.path().join("source");
    let destination_dir = temp.path().join("destination");
    fs::create_dir_all(source.join("nested")).unwrap();
    fs::create_dir_all(&destination_dir).unwrap();
    fs::write(source.join("nested/data.txt"), "nested content").unwrap();

    let copied = copy_entry(&source, &destination_dir).unwrap();

    assert_eq!(destination_dir.join("source"), copied);
    assert_eq!(
        "nested content",
        fs::read_to_string(copied.join("nested/data.txt")).unwrap()
    );
    assert!(source.is_dir());
}

#[test]
fn moves_entry_and_removes_source() {
    let temp = TestDirectory::new("move-file");
    let source_dir = temp.path().join("source");
    let destination_dir = temp.path().join("destination");
    fs::create_dir_all(&source_dir).unwrap();
    fs::create_dir_all(&destination_dir).unwrap();
    let source = source_dir.join("move.txt");
    fs::write(&source, "moved content").unwrap();

    let moved = move_entry(&source, &destination_dir).unwrap();

    assert_eq!(destination_dir.join("move.txt"), moved);
    assert_eq!("moved content", fs::read_to_string(&moved).unwrap());
    assert!(!source.exists());
}

#[test]
fn rejects_existing_destination() {
    let temp = TestDirectory::new("copy-conflict");
    let source_dir = temp.path().join("source");
    let destination_dir = temp.path().join("destination");
    fs::create_dir_all(&source_dir).unwrap();
    fs::create_dir_all(&destination_dir).unwrap();
    let source = source_dir.join("duplicate.txt");
    fs::write(&source, "source").unwrap();
    fs::write(destination_dir.join("duplicate.txt"), "destination").unwrap();

    let error = copy_entry(&source, &destination_dir).unwrap_err();

    assert!(error.to_string().contains("already exists"));
    assert_eq!(
        "destination",
        fs::read_to_string(destination_dir.join("duplicate.txt")).unwrap()
    );
}

#[test]
fn rejects_pasting_directory_into_its_descendant() {
    let temp = TestDirectory::new("copy-descendant");
    let source = temp.path().join("source");
    let descendant = source.join("nested");
    fs::create_dir_all(&descendant).unwrap();

    let copy_error = copy_entry(&source, &descendant).unwrap_err();
    let move_error = move_entry(&source, &descendant).unwrap_err();

    assert!(copy_error.to_string().contains("inside itself"));
    assert!(move_error.to_string().contains("inside itself"));
}
