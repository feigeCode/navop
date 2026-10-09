use super::*;
use std::path::{Path, PathBuf};

/// 两个文件的整轮快照 patch，`src/lib.rs` 在前。
const TWO_FILES: &str = concat!(
    "diff --git a/src/lib.rs b/src/lib.rs\n",
    "--- a/src/lib.rs\n",
    "+++ b/src/lib.rs\n",
    "@@ -1,1 +1,1 @@\n",
    "-old\n",
    "+new\n",
    "diff --git a/src/main.rs b/src/main.rs\n",
    "--- a/src/main.rs\n",
    "+++ b/src/main.rs\n",
    "@@ -1,1 +1,1 @@\n",
    "-before\n",
    "+after\n",
);

#[test]
fn section_is_picked_by_its_file_path() {
    let section = patch_section_for_path(TWO_FILES, "src/main.rs").expect("第二段应命中");

    assert!(
        section.starts_with("diff --git a/src/main.rs "),
        "{section}"
    );
    assert!(section.contains("+after"), "{section}");
    assert!(
        !section.contains("+new"),
        "裁出来的必须只是这一个文件: {section}"
    );
}

#[test]
fn the_first_section_is_reachable_too() {
    let section = patch_section_for_path(TWO_FILES, "src/lib.rs").expect("第一段应命中");

    assert!(section.contains("+new"), "{section}");
    assert!(!section.contains("+after"), "{section}");
}

#[test]
fn absolute_path_is_matched_after_stripping_the_workspace_root() {
    let root = PathBuf::from("/w/proj");
    let requested = PathBuf::from("/w/proj/src/main.rs");

    let relative = relative_to_root(&requested, &root);
    assert_eq!("src/main.rs", relative);
    assert!(
        patch_section_for_path(TWO_FILES, &relative).is_some(),
        "卡片给的是绝对路径，patch 里是仓库相对路径"
    );
}

#[test]
fn a_workspace_root_that_is_only_a_prefix_of_the_request_is_not_stripped() {
    let root = PathBuf::from("/w/proj");
    let requested = PathBuf::from("/w/proj-other/src/main.rs");

    let relative = relative_to_root(&requested, &root);

    assert_eq!("/w/proj-other/src/main.rs", relative);
    assert!(
        patch_section_for_path(TWO_FILES, &relative).is_none(),
        "同名前缀的兄弟目录不能被当成工作区内路径"
    );
}

#[test]
fn path_without_a_workspace_root_is_used_as_is() {
    let relative = relative_to_root(Path::new("src/main.rs"), Path::new("/w/proj"));

    assert_eq!("src/main.rs", relative);
}

#[test]
fn candidates_cover_the_workspace_root_then_the_repository_root() {
    // 工作区是仓库的子目录：同一个绝对路径，两个根各切出一种写法。
    let path = PathBuf::from("/repo/crates/db/src/lib.rs");
    let workspace = PathBuf::from("/repo/crates/db");
    let repository = PathBuf::from("/repo");

    assert_eq!(
        vec!["src/lib.rs".to_string(), "crates/db/src/lib.rs".to_string()],
        relative_candidates(&path, &[workspace.as_path(), repository.as_path()])
    );
}

#[test]
fn identical_roots_do_not_produce_a_duplicate_candidate() {
    let path = PathBuf::from("/repo/src/lib.rs");
    let root = PathBuf::from("/repo");
    let repository = PathBuf::from("/repo");

    assert_eq!(
        vec!["src/lib.rs".to_string()],
        relative_candidates(&path, &[root.as_path(), repository.as_path()])
    );
}

#[test]
fn a_repository_relative_candidate_still_finds_the_section() {
    let path = PathBuf::from("/repo/crates/db/src/lib.rs");
    let workspace = PathBuf::from("/repo/crates/db");
    let repository = PathBuf::from("/repo");
    let patch = concat!(
        "diff --git a/crates/db/src/lib.rs b/crates/db/src/lib.rs\n",
        "--- a/crates/db/src/lib.rs\n",
        "+++ b/crates/db/src/lib.rs\n",
        "@@ -1,1 +1,1 @@\n",
        "-old\n",
        "+new\n",
    );

    let section = relative_candidates(&path, &[workspace.as_path(), repository.as_path()])
        .into_iter()
        .find_map(|relative| patch_section_for_path(patch, &relative));

    assert!(section.is_some(), "工作区根切不出来时还要能按仓库根对上");
}

#[test]
fn windows_separators_and_backslash_paths_still_match() {
    assert!(same_path("src/main.rs", r"src\main.rs"));
    assert!(same_path("./src/main.rs", "src/main.rs"));
    assert!(!same_path("src/main.rs", "src/lib.rs"));
}

#[test]
fn renamed_file_matches_on_either_side() {
    let patch = concat!(
        "diff --git a/old/name.rs b/new/name.rs\n",
        "similarity index 90%\n",
        "rename from old/name.rs\n",
        "rename to new/name.rs\n",
        "--- a/old/name.rs\n",
        "+++ b/new/name.rs\n",
        "@@ -1,1 +1,1 @@\n",
        "-before\n",
        "+after\n",
    );

    assert!(patch_section_for_path(patch, "new/name.rs").is_some());
    assert!(
        patch_section_for_path(patch, "old/name.rs").is_some(),
        "旧路径也指向同一个文件，点了也该看到这轮改动"
    );
    assert!(patch_section_for_path(patch, "other/name.rs").is_none());
}

#[test]
fn hunk_content_mentioning_a_git_header_does_not_split_the_file() {
    let patch = concat!(
        "diff --git a/notes.md b/notes.md\n",
        "--- a/notes.md\n",
        "+++ b/notes.md\n",
        "@@ -1,1 +1,1 @@\n",
        "-plain\n",
        "+diff --git a/notes.md b/notes.md\n",
    );

    let section = patch_section_for_path(patch, "notes.md").expect("整段应命中");

    assert!(section.contains("-plain"), "{section}");
    assert!(section.contains("+diff --git a/notes.md"), "{section}");
}

#[test]
fn patch_without_git_headers_is_a_single_section() {
    let patch = concat!(
        "--- a/src/lib.rs\n",
        "+++ b/src/lib.rs\n",
        "@@ -1,1 +1,1 @@\n",
        "-old\n",
        "+new\n",
    );

    let section = patch_section_for_path(patch, "src/lib.rs").expect("整份就是一段");

    assert_eq!(patch, section);
    assert!(patch_section_for_path(patch, "src/main.rs").is_none());
}

#[test]
fn empty_patch_has_no_section() {
    assert!(patch_section_for_path("", "src/lib.rs").is_none());
}

#[test]
fn a_section_cut_before_its_hunk_ends_is_not_reported_as_a_match() {
    // 段被切在半途（hunk 声明的行数没走完）。读不懂的片段必须落到下一档来源，
    // 而不是拿一段残缺 patch 去开一个看起来「没有改动」的 diff 页。
    let patch = concat!(
        "diff --git a/src/lib.rs b/src/lib.rs\n",
        "--- a/src/lib.rs\n",
        "+++ b/src/lib.rs\n",
        "@@ -1,3 +1,3 @@\n",
        " a\n",
    );

    assert!(patch_section_for_path(patch, "src/lib.rs").is_none());
}
