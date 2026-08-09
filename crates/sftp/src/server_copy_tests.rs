use super::{ServerCopyItem, build_copy_plan, join_copy_path};
use crate::FileEntry;
use std::time::SystemTime;

fn file(path: &str, size: u64) -> FileEntry {
    FileEntry {
        name: path.rsplit('/').next().unwrap_or(path).to_string(),
        path: path.to_string(),
        size,
        modified: SystemTime::UNIX_EPOCH,
        is_dir: false,
        permissions: 0,
    }
}

#[test]
fn joining_copy_paths_does_not_duplicate_slashes() {
    assert_eq!("/srv/app/a.txt", join_copy_path("/srv/app/", "a.txt"));
    assert_eq!("/a.txt", join_copy_path("/", "a.txt"));
}

#[test]
fn directory_plan_keeps_relative_layout_and_sizes() {
    let items = vec![ServerCopyItem {
        source_path: "/src/app".to_string(),
        target_path: "/dst/app".to_string(),
        is_dir: true,
        size: 0,
    }];
    let plan = build_copy_plan(&items, &[vec![file("/src/app/bin", 42)]]);
    assert_eq!(plan[1].target_path, "/dst/app/bin");
    assert_eq!(plan[1].size, 42);
}
