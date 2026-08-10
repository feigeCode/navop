use super::{remote_path_is_same_or_descendant, total_file_size};
use crate::FileEntry;
use std::time::SystemTime;

fn entry(path: &str, size: u64, is_dir: bool) -> FileEntry {
    FileEntry {
        name: path.rsplit('/').next().unwrap_or(path).to_string(),
        path: path.to_string(),
        size,
        modified: SystemTime::UNIX_EPOCH,
        is_dir,
        permissions: 0,
        uid: None,
        gid: None,
        user: None,
        group: None,
    }
}

#[test]
fn directory_size_only_counts_regular_files() {
    let entries = vec![
        entry("/root/folder", 4096, true),
        entry("/root/folder/a.txt", 10, false),
        entry("/root/folder/nested", 8192, true),
        entry("/root/folder/nested/b.txt", 32, false),
    ];

    assert_eq!(42, total_file_size(&entries));
}

#[test]
fn empty_directory_size_is_zero() {
    assert_eq!(0, total_file_size(&[]));
}

#[test]
fn descendant_check_respects_path_component_boundaries() {
    assert!(remote_path_is_same_or_descendant("/srv/app", "/srv/app"));
    assert!(remote_path_is_same_or_descendant(
        "/srv/app",
        "/srv/app/cache"
    ));
    assert!(remote_path_is_same_or_descendant(
        "/srv/app/./data",
        "/srv/app/data/tmp/../cache"
    ));
    assert!(!remote_path_is_same_or_descendant(
        "/srv/app",
        "/srv/application"
    ));
    assert!(!remote_path_is_same_or_descendant("/srv/app", "/srv/other"));
}
