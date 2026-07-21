use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ExplorerEntry {
    pub(crate) path: PathBuf,
    pub(crate) name: String,
    pub(crate) is_dir: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ExplorerRow {
    pub(crate) entry: ExplorerEntry,
    pub(crate) depth: usize,
    pub(crate) expanded: bool,
}

pub(crate) fn sort_entries(entries: &mut [ExplorerEntry]) {
    entries.sort_by(|left, right| {
        right
            .is_dir
            .cmp(&left.is_dir)
            .then_with(|| left.name.to_lowercase().cmp(&right.name.to_lowercase()))
            .then_with(|| left.name.cmp(&right.name))
    });
}

pub(crate) fn visible_rows(
    root: &Path,
    listings: &HashMap<PathBuf, Vec<ExplorerEntry>>,
    expanded: &HashSet<PathBuf>,
) -> Vec<ExplorerRow> {
    let mut rows = Vec::new();
    append_visible_rows(root, 0, listings, expanded, &mut rows);
    rows
}

fn append_visible_rows(
    parent: &Path,
    depth: usize,
    listings: &HashMap<PathBuf, Vec<ExplorerEntry>>,
    expanded: &HashSet<PathBuf>,
    rows: &mut Vec<ExplorerRow>,
) {
    let Some(entries) = listings.get(parent) else {
        return;
    };
    for entry in entries {
        let is_expanded = entry.is_dir && expanded.contains(&entry.path);
        rows.push(ExplorerRow {
            entry: entry.clone(),
            depth,
            expanded: is_expanded,
        });
        if is_expanded {
            append_visible_rows(&entry.path, depth + 1, listings, expanded, rows);
        }
    }
}

pub(crate) fn active_index_after_open(paths: &[PathBuf], path: &Path) -> usize {
    paths
        .iter()
        .position(|candidate| candidate == path)
        .unwrap_or(paths.len())
}

pub(crate) fn active_index_after_close(
    tab_count: usize,
    active_index: usize,
    closed_index: usize,
) -> Option<usize> {
    let remaining = tab_count.checked_sub(1)?;
    if remaining == 0 {
        return None;
    }
    if closed_index < active_index {
        Some(active_index - 1)
    } else if closed_index == active_index {
        Some(active_index.saturating_sub(1).min(remaining - 1))
    } else {
        Some(active_index.min(remaining - 1))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(path: &str, is_dir: bool) -> ExplorerEntry {
        ExplorerEntry {
            path: PathBuf::from(path),
            name: PathBuf::from(path)
                .file_name()
                .unwrap()
                .to_string_lossy()
                .into_owned(),
            is_dir,
        }
    }

    #[test]
    fn directories_sort_before_files_case_insensitively() {
        let mut entries = vec![
            entry("/workspace/z.rs", false),
            entry("/workspace/Beta", true),
            entry("/workspace/alpha.rs", false),
            entry("/workspace/assets", true),
        ];

        sort_entries(&mut entries);

        let names = entries
            .into_iter()
            .map(|item| item.name)
            .collect::<Vec<_>>();
        assert_eq!(vec!["assets", "Beta", "alpha.rs", "z.rs"], names);
    }

    #[test]
    fn visible_rows_only_descend_into_expanded_directories() {
        let root = PathBuf::from("/workspace");
        let src = entry("/workspace/src", true);
        let readme = entry("/workspace/README.md", false);
        let lib = entry("/workspace/src/lib.rs", false);
        let mut listings = HashMap::new();
        listings.insert(root.clone(), vec![src.clone(), readme]);
        listings.insert(src.path.clone(), vec![lib]);

        let collapsed = visible_rows(&root, &listings, &HashSet::new());
        assert_eq!(2, collapsed.len());
        assert_eq!(0, collapsed[0].depth);

        let expanded = visible_rows(&root, &listings, &HashSet::from([src.path]));
        assert_eq!(3, expanded.len());
        assert_eq!(1, expanded[1].depth);
        assert_eq!("lib.rs", expanded[1].entry.name);
    }

    #[test]
    fn opening_existing_path_reuses_its_tab() {
        let paths = vec![PathBuf::from("a.rs"), PathBuf::from("b.rs")];
        assert_eq!(1, active_index_after_open(&paths, Path::new("b.rs")));
        assert_eq!(2, active_index_after_open(&paths, Path::new("c.rs")));
    }

    #[test]
    fn closing_tab_keeps_nearest_remaining_tab_active() {
        assert_eq!(Some(0), active_index_after_close(3, 1, 1));
        assert_eq!(Some(1), active_index_after_close(3, 2, 0));
        assert_eq!(None, active_index_after_close(1, 0, 0));
    }
}
