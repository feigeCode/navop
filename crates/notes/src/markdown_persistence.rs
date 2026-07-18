use crate::markdown_file_store::{MarkdownFileStore, MarkdownSaveOutcome};
use cditor_app::{
    EditorDocument, EditorPersistence, EditorPersistenceError, EditorSaveRequest,
    MarkdownBundleOptions, MarkdownExportMode,
};

#[derive(Debug, Clone)]
pub(crate) struct MarkdownDocumentPersistence {
    store: MarkdownFileStore,
}

impl MarkdownDocumentPersistence {
    pub(crate) fn new(store: MarkdownFileStore) -> Self {
        Self { store }
    }
}

impl EditorPersistence for MarkdownDocumentPersistence {
    fn load(&self, document_id: &str) -> Result<Option<EditorDocument>, EditorPersistenceError> {
        let snapshot = self.store.load().map_err(persistence_error)?;
        let imported = EditorDocument::from_markdown_bundle_with_report(
            document_id,
            &snapshot.source,
            &self.store,
        )
        .map_err(persistence_error)?;
        Ok(Some(imported.document))
    }

    fn save(&self, request: EditorSaveRequest) -> Result<(), EditorPersistenceError> {
        let exported = request
            .document
            .export_markdown_bundle(
                MarkdownExportMode::Strict,
                &MarkdownBundleOptions {
                    asset_directory: self.store.asset_directory().map_err(persistence_error)?,
                    ..MarkdownBundleOptions::default()
                },
            )
            .map_err(persistence_error)?;
        self.store
            .write_assets(&exported.assets)
            .map_err(persistence_error)?;
        match self
            .store
            .save(&exported.markdown)
            .map_err(persistence_error)?
        {
            MarkdownSaveOutcome::Saved(_) => Ok(()),
            MarkdownSaveOutcome::Conflict(_) => Err(EditorPersistenceError::new(
                "Markdown file changed outside Navop",
            )),
        }
    }
}

fn persistence_error(error: impl std::fmt::Display) -> EditorPersistenceError {
    EditorPersistenceError::new(error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use cditor_app::{EditorSaveReason, EditorSaveRequest};
    use std::fs;

    #[test]
    fn strict_save_writes_markdown_file() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("note.md");
        fs::write(&path, "before").unwrap();
        let persistence = MarkdownDocumentPersistence::new(MarkdownFileStore::new(path.clone()));
        persistence.load("doc-1").unwrap();
        let document = EditorDocument::from_markdown("doc-1", "**after**").unwrap();
        persistence
            .save(EditorSaveRequest {
                document_id: "doc-1".to_owned(),
                document,
                document_version: 1,
                reason: EditorSaveReason::Manual,
            })
            .unwrap();
        assert_eq!("**after**", fs::read_to_string(path).unwrap());
    }

    #[test]
    fn external_change_fails_cditor_save() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("note.md");
        fs::write(&path, "before").unwrap();
        let persistence = MarkdownDocumentPersistence::new(MarkdownFileStore::new(path.clone()));
        persistence.load("doc-1").unwrap();
        fs::write(&path, "external").unwrap();
        let document = EditorDocument::from_markdown("doc-1", "local").unwrap();
        let result = persistence.save(EditorSaveRequest {
            document_id: "doc-1".to_owned(),
            document,
            document_version: 1,
            reason: EditorSaveReason::Autosave,
        });
        assert!(result.is_err());
        assert_eq!("external", fs::read_to_string(path).unwrap());
    }

    #[test]
    fn whiteboard_save_writes_preview_and_editable_source_bundle() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("note.md");
        fs::write(&path, "before").unwrap();
        let persistence = MarkdownDocumentPersistence::new(MarkdownFileStore::new(path.clone()));
        persistence.load("doc-1").unwrap();
        let runtime = cditor_app::runtime::DocumentRuntime::from_payloads(
            1,
            vec![cditor_app::core::rich_text::BlockPayloadRecord {
                block_id: 1,
                content_version: 1,
                kind: cditor_app::core::rich_text::RichBlockKind::Whiteboard,
                payload: cditor_app::core::rich_text::BlockPayload::Whiteboard(
                    cditor_app::core::rich_text::WhiteboardPayload {
                        scene_json: r#"{"camera":{"x":0.0,"y":0.0,"zoom":1.0},"elements":[]}"#
                            .to_owned(),
                    },
                ),
            }],
            720.0,
        );
        let document = EditorDocument::from_runtime("doc-1", &runtime).unwrap();

        persistence
            .save(EditorSaveRequest {
                document_id: "doc-1".to_owned(),
                document,
                document_version: 1,
                reason: EditorSaveReason::Manual,
            })
            .unwrap();

        let markdown = fs::read_to_string(&path).unwrap();
        assert!(markdown.contains("cditor:whiteboard"));
        assert!(markdown.contains("![Whiteboard](<note.assets/whiteboard-1.svg>)"));
        assert!(temp.path().join("note.assets/whiteboard-1.svg").is_file());
        assert!(
            temp.path()
                .join("note.assets/whiteboard-1.cditor-board.json")
                .is_file()
        );
        let restored = persistence.load("doc-1").unwrap().unwrap();
        assert_eq!(
            restored.blocks[0].payload.kind,
            cditor_app::core::rich_text::RichBlockKind::Whiteboard
        );
    }
}
