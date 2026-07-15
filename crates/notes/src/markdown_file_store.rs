use crate::storage_support::write_text_atomic;
use anyhow::{Context, Result};
use sha2::{Digest, Sha256};
use std::fs;
use std::path::PathBuf;
use std::sync::{Arc, RwLock};
use std::time::SystemTime;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct FileFingerprint {
    pub size: u64,
    pub modified_at: Option<SystemTime>,
    pub content_hash: [u8; 32],
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct MarkdownSnapshot {
    pub source: String,
    pub fingerprint: FileFingerprint,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum MarkdownSaveOutcome {
    Saved(FileFingerprint),
    Conflict(MarkdownSnapshot),
}

#[derive(Debug, Clone)]
pub(crate) struct MarkdownFileStore {
    path: Arc<RwLock<PathBuf>>,
}

impl MarkdownFileStore {
    pub(crate) fn new(path: PathBuf) -> Self {
        Self {
            path: Arc::new(RwLock::new(path)),
        }
    }

    pub(crate) fn set_path(&self, path: PathBuf) -> Result<()> {
        *self
            .path
            .write()
            .map_err(|_| anyhow::anyhow!("Markdown path lock is poisoned"))? = path;
        Ok(())
    }

    pub(crate) fn load(&self) -> Result<MarkdownSnapshot> {
        let path = self.path()?;
        let source = fs::read_to_string(&path)
            .with_context(|| format!("read Markdown {}", path.display()))?;
        snapshot(&path, source)
    }

    pub(crate) fn save(
        &self,
        source: &str,
        expected: Option<&FileFingerprint>,
    ) -> Result<MarkdownSaveOutcome> {
        let path = self.path()?;
        if let Some(expected) = expected {
            let disk = snapshot(&path, fs::read_to_string(&path)?)?;
            if disk.fingerprint != *expected {
                return Ok(MarkdownSaveOutcome::Conflict(disk));
            }
        }
        write_text_atomic(&path, source)?;
        Ok(MarkdownSaveOutcome::Saved(
            snapshot(&path, source.to_owned())?.fingerprint,
        ))
    }

    fn path(&self) -> Result<PathBuf> {
        self.path
            .read()
            .map(|path| path.clone())
            .map_err(|_| anyhow::anyhow!("Markdown path lock is poisoned"))
    }
}

fn snapshot(path: &PathBuf, source: String) -> Result<MarkdownSnapshot> {
    let metadata = fs::metadata(path)?;
    let content_hash: [u8; 32] = Sha256::digest(source.as_bytes()).into();
    Ok(MarkdownSnapshot {
        source,
        fingerprint: FileFingerprint {
            size: metadata.len(),
            modified_at: metadata.modified().ok(),
            content_hash,
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_external_changes_before_overwrite() -> Result<()> {
        let temp = tempfile::tempdir()?;
        let path = temp.path().join("note.md");
        fs::write(&path, "first")?;
        let store = MarkdownFileStore::new(path.clone());
        let baseline = store.load()?;
        fs::write(&path, "external")?;

        let outcome = store.save("local", Some(&baseline.fingerprint))?;
        assert!(matches!(outcome, MarkdownSaveOutcome::Conflict(_)));
        assert_eq!("external", fs::read_to_string(path)?);
        Ok(())
    }

    #[test]
    fn follows_internal_rename() -> Result<()> {
        let temp = tempfile::tempdir()?;
        let old = temp.path().join("old.md");
        let new = temp.path().join("new.md");
        fs::write(&old, "old")?;
        let store = MarkdownFileStore::new(old);
        fs::write(&new, "new")?;
        store.set_path(new)?;
        assert_eq!("new", store.load()?.source);
        Ok(())
    }
}
