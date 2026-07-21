use crate::model::{ExplorerEntry, sort_entries};
use anyhow::{Context as _, Result};
use remote_file_editor::{
    FilePolicy, decode_text_content, determine_file_policy, language_for_path,
};
use std::fs;
use std::path::{Path, PathBuf};

pub(crate) struct LoadedFile {
    pub(crate) text: String,
    pub(crate) policy: FilePolicy,
    pub(crate) file_size: usize,
    pub(crate) language: String,
}

pub(crate) fn read_directory(path: &Path) -> Result<Vec<ExplorerEntry>> {
    let mut entries = Vec::new();
    for item in fs::read_dir(path).with_context(|| format!("Unable to read {}", path.display()))? {
        let item = item?;
        let name = item.file_name().to_string_lossy().into_owned();
        if name == ".git" {
            continue;
        }
        let file_type = item.file_type()?;
        entries.push(ExplorerEntry {
            path: item.path(),
            name,
            is_dir: file_type.is_dir(),
        });
    }
    sort_entries(&mut entries);
    Ok(entries)
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
