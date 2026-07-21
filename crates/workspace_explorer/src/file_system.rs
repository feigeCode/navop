use crate::model::{ExplorerEntry, sort_entries};
use anyhow::{Context as _, Result};
use ignore::gitignore::{Gitignore, GitignoreBuilder};
use remote_file_editor::{
    FilePolicy, decode_text_content, determine_file_policy, language_for_path,
};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

pub(crate) struct LoadedFile {
    pub(crate) text: String,
    pub(crate) policy: FilePolicy,
    pub(crate) file_size: usize,
    pub(crate) language: String,
}

pub(crate) fn read_directory(
    path: &Path,
    root_matcher: Option<&Gitignore>,
    show_hidden: bool,
    show_ignored: bool,
) -> Result<Vec<ExplorerEntry>> {
    let local_matcher = if show_ignored {
        None
    } else {
        local_gitignore(path)
    };
    let mut entries = Vec::new();
    for item in fs::read_dir(path).with_context(|| format!("Unable to read {}", path.display()))? {
        let item = item?;
        let name = item.file_name().to_string_lossy().into_owned();
        if name == ".git" {
            continue;
        }
        let file_type = item.file_type()?;
        let item_path = item.path();
        if should_hide_entry(
            &name,
            &item_path,
            file_type.is_dir(),
            root_matcher,
            local_matcher.as_ref(),
            show_hidden,
            show_ignored,
        ) {
            continue;
        }
        entries.push(ExplorerEntry {
            path: item_path,
            name,
            is_dir: file_type.is_dir(),
        });
    }
    sort_entries(&mut entries);
    Ok(entries)
}

fn should_hide_entry(
    name: &str,
    path: &Path,
    is_dir: bool,
    root_matcher: Option<&Gitignore>,
    local_matcher: Option<&Gitignore>,
    show_hidden: bool,
    show_ignored: bool,
) -> bool {
    if !show_hidden && name.starts_with('.') {
        return true;
    }
    if show_ignored {
        return false;
    }
    let ignored_by_root =
        root_matcher.is_some_and(|matcher| matcher.matched(path, is_dir).is_ignore());
    let ignored_by_local =
        local_matcher.is_some_and(|matcher| matcher.matched(path, is_dir).is_ignore());
    ignored_by_root || ignored_by_local
}

fn local_gitignore(dir: &Path) -> Option<Gitignore> {
    let file = dir.join(".gitignore");
    if !file.is_file() {
        return None;
    }
    let (matcher, _) = Gitignore::new(file);
    (!matcher.is_empty()).then_some(matcher)
}

/// 从工作区根目录的 `.gitignore` 与 `.git/info/exclude` 构建共享匹配器。
pub(crate) fn root_ignore_matcher(root: &Path) -> Option<Arc<Gitignore>> {
    let mut builder = GitignoreBuilder::new(root);
    let mut added = false;
    for file in [root.join(".gitignore"), root.join(".git/info/exclude")] {
        if file.is_file() {
            let _ = builder.add(file);
            added = true;
        }
    }
    if !added {
        return None;
    }
    builder.build().ok().map(Arc::new)
}

pub(crate) fn load_file(path: &Path) -> Result<LoadedFile> {
    let bytes = fs::read(path).with_context(|| format!("Unable to read {}", path.display()))?;
    let file_size = bytes.len();
    let policy = determine_file_policy(file_size)?;
    let text = decode_text_content(&bytes)?;
    let language = language_for_path(&path.to_string_lossy(), policy.is_large_file);
    Ok(LoadedFile {
        text,
        policy,
        file_size,
        language,
    })
}

pub(crate) fn save_file(path: &Path, text: &str) -> Result<()> {
    fs::write(path, text.as_bytes()).with_context(|| format!("Unable to save {}", path.display()))
}

pub(crate) fn canonical_workspace_root(path: PathBuf) -> Result<PathBuf> {
    path.canonicalize()
        .with_context(|| format!("Unable to open workspace {}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::{read_directory, root_ignore_matcher};
    use std::fs;

    fn entry_names(entries: Vec<crate::model::ExplorerEntry>) -> Vec<String> {
        entries.into_iter().map(|entry| entry.name).collect()
    }

    #[test]
    fn hidden_and_ignored_visibility_are_independent() {
        let temp = std::env::temp_dir().join(format!(
            "navop-workspace-explorer-filter-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&temp);
        fs::create_dir_all(&temp).unwrap();
        fs::write(temp.join(".gitignore"), "target\n").unwrap();
        fs::write(temp.join(".env.local"), "SECRET=test").unwrap();
        fs::create_dir(temp.join("target")).unwrap();
        fs::write(temp.join("Cargo.toml"), "[package]").unwrap();

        let matcher = root_ignore_matcher(&temp).unwrap();
        let default_names =
            entry_names(read_directory(&temp, Some(matcher.as_ref()), false, false).unwrap());
        assert_eq!(vec!["Cargo.toml"], default_names);

        let hidden_names =
            entry_names(read_directory(&temp, Some(matcher.as_ref()), true, false).unwrap());
        assert!(hidden_names.contains(&".env.local".to_string()));
        assert!(!hidden_names.contains(&"target".to_string()));

        let ignored_names =
            entry_names(read_directory(&temp, Some(matcher.as_ref()), false, true).unwrap());
        assert!(ignored_names.contains(&"target".to_string()));
        assert!(!ignored_names.contains(&".env.local".to_string()));

        fs::remove_dir_all(temp).unwrap();
    }
}
