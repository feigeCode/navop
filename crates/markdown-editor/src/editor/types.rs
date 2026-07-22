#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MarkdownEditorEvent {
    Changed { source: String, revision: u64 },
}

#[derive(Debug, thiserror::Error)]
pub enum MarkdownEditorError {
    #[error(transparent)]
    Parse(#[from] markdown_source::SourceParseError),
    #[error(transparent)]
    Patch(#[from] markdown_source::PatchError),
    #[error(transparent)]
    Operation(#[from] markdown_source::SourceOperationError),
}
