use crate::storage_support::write_text_atomic;
use anyhow::{Context, Result};
use sha2::{Digest, Sha256};
use std::fs;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
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
    state: Arc<Mutex<MarkdownStoreState>>,
}

#[derive(Debug)]
struct MarkdownStoreState {
    path: PathBuf,
    fingerprint: Option<FileFingerprint>,
}

impl MarkdownFileStore {
    pub(crate) fn path(&self) -> Result<PathBuf> {
        Ok(self.state()?.path.clone())
    }

    pub(crate) fn new(path: PathBuf) -> Self {
        Self {
            state: Arc::new(Mutex::new(MarkdownStoreState {
                path,
                fingerprint: None,
            })),
        }
    }

    pub(crate) fn set_path(&self, path: PathBuf) -> Result<()> {
        self.state()?.path = path;
        Ok(())
    }

    pub(crate) fn load(&self) -> Result<MarkdownSnapshot> {
        let mut state = self.state()?;
        let path = state.path.clone();
        let source = fs::read_to_string(&path)
            .with_context(|| format!("read Markdown {}", path.display()))?;
        let snapshot = snapshot(&path, source)?;
        state.fingerprint = Some(snapshot.fingerprint.clone());
        Ok(snapshot)
    }

    /// Load a watcher event only when the on-disk fingerprint differs from the
    /// fingerprint already observed or written by this store.
    ///
    /// A local save can finish after the editor has already produced a newer
    /// dirty revision. Its watcher event must not be mistaken for an external
    /// edit merely because the current editor source is newer than the bytes
    /// that were just written.
    pub(crate) fn load_external_change(&self) -> Result<Option<MarkdownSnapshot>> {
        let mut state = self.state()?;
        let path = state.path.clone();
        let snapshot = snapshot(
            &path,
            fs::read_to_string(&path)
                .with_context(|| format!("read Markdown {}", path.display()))?,
        )?;
        if state.fingerprint.as_ref() == Some(&snapshot.fingerprint) {
            return Ok(None);
        }
        state.fingerprint = Some(snapshot.fingerprint.clone());
        Ok(Some(snapshot))
    }

    pub(crate) fn save(&self, source: &str) -> Result<MarkdownSaveOutcome> {
        let mut state = self.state()?;
        let path = state.path.clone();
        if let Some(expected) = state.fingerprint.as_ref() {
            let disk = snapshot(&path, fs::read_to_string(&path)?)?;
            if disk.fingerprint != *expected {
                return Ok(MarkdownSaveOutcome::Conflict(disk));
            }
        }
        write_text_atomic(&path, source)?;
        let fingerprint = snapshot(&path, source.to_owned())?.fingerprint;
        state.fingerprint = Some(fingerprint.clone());
        Ok(MarkdownSaveOutcome::Saved(fingerprint))
    }

    /// Overwrite the file on disk regardless of external changes.
    ///
    /// Used when the user explicitly chooses to keep their local changes
    /// after an external-modification conflict.
    pub(crate) fn force_save(&self, source: &str) -> Result<FileFingerprint> {
        let mut state = self.state()?;
        let path = state.path.clone();
        write_text_atomic(&path, source)?;
        let fingerprint = snapshot(&path, source.to_owned())?.fingerprint;
        state.fingerprint = Some(fingerprint.clone());
        Ok(fingerprint)
    }

    fn state(&self) -> Result<std::sync::MutexGuard<'_, MarkdownStoreState>> {
        self.state
            .lock()
            .map_err(|_| anyhow::anyhow!("Markdown store lock is poisoned"))
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
        store.load()?;
        fs::write(&path, "external")?;

        let outcome = store.save("local")?;
        assert!(matches!(outcome, MarkdownSaveOutcome::Conflict(_)));
        assert_eq!("external", fs::read_to_string(path)?);
        Ok(())
    }

    #[test]
    fn force_save_overwrites_external_changes() -> Result<()> {
        let temp = tempfile::tempdir()?;
        let path = temp.path().join("note.md");
        fs::write(&path, "first")?;
        let store = MarkdownFileStore::new(path.clone());
        store.load()?;
        fs::write(&path, "external")?;

        store.force_save("local")?;
        assert_eq!("local", fs::read_to_string(&path)?);
        // Subsequent normal saves see a clean fingerprint again.
        let outcome = store.save("second")?;
        assert!(matches!(outcome, MarkdownSaveOutcome::Saved(_)));
        assert_eq!("second", fs::read_to_string(path)?);
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
