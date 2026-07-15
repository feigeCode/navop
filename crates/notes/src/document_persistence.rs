use crate::storage_support::write_text_atomic;
use cditor_app::{EditorDocument, EditorPersistence, EditorPersistenceError, EditorSaveRequest};
use std::fs;
use std::path::PathBuf;
use std::sync::{Arc, RwLock};

#[derive(Debug, Clone)]
pub struct FileDocumentPersistence {
    path: Arc<RwLock<PathBuf>>,
}

impl FileDocumentPersistence {
    pub fn new(path: PathBuf) -> Self {
        Self {
            path: Arc::new(RwLock::new(path)),
        }
    }

    pub fn set_path(&self, path: PathBuf) -> Result<(), EditorPersistenceError> {
        *self
            .path
            .write()
            .map_err(|_| EditorPersistenceError::new("document path lock is poisoned"))? = path;
        Ok(())
    }

    fn path(&self) -> Result<PathBuf, EditorPersistenceError> {
        self.path
            .read()
            .map(|path| path.clone())
            .map_err(|_| EditorPersistenceError::new("document path lock is poisoned"))
    }
}

impl EditorPersistence for FileDocumentPersistence {
    fn load(&self, document_id: &str) -> Result<Option<EditorDocument>, EditorPersistenceError> {
        let path = self.path()?;
        if !path.exists() {
            return Ok(None);
        }
        let json = fs::read_to_string(path).map_err(persistence_error)?;
        let document = EditorDocument::from_json(&json).map_err(persistence_error)?;
        if document.document_id != document_id {
            return Err(EditorPersistenceError::new(format!(
                "document id mismatch: expected {document_id}, found {}",
                document.document_id
            )));
        }
        Ok(Some(document))
    }

    fn save(&self, request: EditorSaveRequest) -> Result<(), EditorPersistenceError> {
        if request.document.document_id != request.document_id {
            return Err(EditorPersistenceError::new(
                "save request document id mismatch",
            ));
        }
        let json = request.document.to_json().map_err(persistence_error)?;
        write_text_atomic(&self.path()?, &json).map_err(persistence_error)
    }
}

fn persistence_error(error: impl std::fmt::Display) -> EditorPersistenceError {
    EditorPersistenceError::new(error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use cditor_app::{EditorSaveReason, EditorSaveRequest};

    #[test]
    fn saves_and_loads_native_document() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("doc.cditor.json");
        let persistence = FileDocumentPersistence::new(path);
        let document = EditorDocument::from_markdown("stable-id", "# Note").unwrap();
        persistence
            .save(EditorSaveRequest {
                document_id: "stable-id".into(),
                document: document.clone(),
                document_version: 1,
                reason: EditorSaveReason::Manual,
            })
            .unwrap();
        assert_eq!(Some(document), persistence.load("stable-id").unwrap());
    }

    #[test]
    fn follows_document_rename() {
        let temp = tempfile::tempdir().unwrap();
        let old_path = temp.path().join("old.cditor.json");
        let new_path = temp.path().join("new.cditor.json");
        let persistence = FileDocumentPersistence::new(old_path);
        persistence.set_path(new_path.clone()).unwrap();
        let document = EditorDocument::from_markdown("stable-id", "renamed").unwrap();
        persistence
            .save(EditorSaveRequest {
                document_id: "stable-id".into(),
                document,
                document_version: 1,
                reason: EditorSaveReason::Manual,
            })
            .unwrap();
        assert!(new_path.is_file());
    }
}
