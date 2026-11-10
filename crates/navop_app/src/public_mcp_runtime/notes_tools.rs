use notes::NotesStorage;
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tool_runtime::{
    RiskLevel, ToolAdapter, ToolAnnotations, ToolContext, ToolDescriptor, ToolError, ToolFuture,
    ToolHandler, ToolMode, ToolRegistry, ToolResult,
};

const MAX_NOTE_BYTES: u64 = 1024 * 1024;

#[derive(Clone, Copy)]
enum NotesTool {
    List,
    Read,
    Create,
    Write,
}

impl NotesTool {
    const ALL: [Self; 4] = [Self::List, Self::Read, Self::Create, Self::Write];

    fn id(self) -> &'static str {
        match self {
            Self::List => "notes.list",
            Self::Read => "notes.read",
            Self::Create => "notes.create",
            Self::Write => "notes.write",
        }
    }

    fn title(self) -> &'static str {
        match self {
            Self::List => "List notes",
            Self::Read => "Read note",
            Self::Create => "Create note",
            Self::Write => "Write note",
        }
    }

    fn description(self) -> &'static str {
        match self {
            Self::List => {
                "List the Navop notebook metadata and its Markdown tree. Call this before resolving a note path."
            }
            Self::Read => {
                "Read one Markdown note from the Navop notebook by its root-relative path."
            }
            Self::Create => {
                "Create a Markdown note in an existing notebook directory. Use notes.list first to resolve the parent path."
            }
            Self::Write => {
                "Replace the complete content of an existing Markdown note in the Navop notebook."
            }
        }
    }

    fn input_schema(self) -> Value {
        match self {
            Self::List => json!({
                "type": "object",
                "properties": {},
                "additionalProperties": false
            }),
            Self::Read => json!({
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "description": "Root-relative Markdown path returned by notes.list"
                    }
                },
                "required": ["path"],
                "additionalProperties": false
            }),
            Self::Create => json!({
                "type": "object",
                "properties": {
                    "parent": {
                        "type": "string",
                        "description": "Existing root-relative directory; use an empty string for the notebook root"
                    },
                    "title": {
                        "type": "string",
                        "description": "Display title without a .md suffix"
                    },
                    "content": {
                        "type": "string",
                        "description": "Initial Markdown content"
                    }
                },
                "required": ["title"],
                "additionalProperties": false
            }),
            Self::Write => json!({
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "description": "Root-relative Markdown path returned by notes.list"
                    },
                    "content": {
                        "type": "string",
                        "description": "Complete replacement Markdown content"
                    }
                },
                "required": ["path", "content"],
                "additionalProperties": false
            }),
        }
    }

    fn annotations(self) -> ToolAnnotations {
        match self {
            Self::List | Self::Read => {
                ToolAnnotations::read_only(self.title()).with_parallel_support(true)
            }
            Self::Create => {
                let mut annotations =
                    ToolAnnotations::mutating(self.title()).with_risk(RiskLevel::Low);
                annotations.destructive = false;
                annotations
            }
            Self::Write => ToolAnnotations::mutating(self.title()).with_risk(RiskLevel::High),
        }
    }
}

#[derive(Clone)]
struct NotesToolHandler {
    tool: NotesTool,
    storage: Option<NotesStorage>,
}

impl NotesToolHandler {
    fn storage(&self) -> Result<NotesStorage, ToolError> {
        match &self.storage {
            Some(storage) => Ok(storage.clone()),
            None => NotesStorage::configured_storage().map_err(tool_error),
        }
    }

    fn call_tool(&self, input: Value) -> Result<Value, ToolError> {
        let storage = self.storage()?;
        ensure_notebook_ready(&storage)?;
        match self.tool {
            NotesTool::List => list_notes(&storage),
            NotesTool::Read => read_note(&storage, &input),
            NotesTool::Create => create_note(&storage, &input),
            NotesTool::Write => write_note(&storage, &input),
        }
    }
}

impl ToolHandler for NotesToolHandler {
    fn descriptor(&self) -> ToolDescriptor {
        ToolDescriptor {
            id: self.tool.id().to_string(),
            title: self.tool.title().to_string(),
            description: self.tool.description().to_string(),
            input_schema: self.tool.input_schema(),
            output_schema: json!({ "type": "object" }),
            permissions: Vec::new(),
            mode: ToolMode::Deterministic,
            adapters: vec![ToolAdapter::Mcp, ToolAdapter::FunctionCalling],
            annotations: self.tool.annotations(),
        }
    }

    fn call(&self, input: Value, _context: ToolContext) -> ToolFuture {
        let handler = self.clone();
        Box::pin(async move { handler.call_tool(input).map(ToolResult::structured) })
    }
}

pub(super) fn notes_tool_registry() -> ToolRegistry {
    notes_tool_registry_with_storage(None)
}

fn notes_tool_registry_with_storage(storage: Option<NotesStorage>) -> ToolRegistry {
    ToolRegistry::new(
        NotesTool::ALL
            .into_iter()
            .map(|tool| {
                Arc::new(NotesToolHandler {
                    tool,
                    storage: storage.clone(),
                }) as Arc<dyn ToolHandler>
            })
            .collect(),
    )
}

fn ensure_notebook_ready(storage: &NotesStorage) -> Result<(), ToolError> {
    if storage.load_notebook().map_err(tool_error)?.is_none() {
        return Err(ToolError::Failed {
            message:
                "Navop Notes is not initialized. Open Notes and choose a storage location first."
                    .to_string(),
        });
    }
    Ok(())
}

fn list_notes(storage: &NotesStorage) -> Result<Value, ToolError> {
    let notebook = storage.load_notebook().map_err(tool_error)?;
    let tree = storage.scan_tree().map_err(tool_error)?;
    Ok(json!({
        "notebook": notebook,
        "tree": tree
    }))
}

fn read_note(storage: &NotesStorage, input: &Value) -> Result<Value, ToolError> {
    let relative_path = required_path(input)?;
    let descriptor = storage.descriptor(&relative_path).map_err(tool_error)?;
    let metadata = std::fs::metadata(&descriptor.absolute_path).map_err(tool_error)?;
    if metadata.len() > MAX_NOTE_BYTES {
        return Err(ToolError::Failed {
            message: format!(
                "Note is too large to read through MCP ({} bytes, limit {} bytes)",
                metadata.len(),
                MAX_NOTE_BYTES
            ),
        });
    }
    let (descriptor, content) = storage.read_document(&relative_path).map_err(tool_error)?;
    Ok(json!({
        "path": display_path(&descriptor.relative_path),
        "documentId": descriptor.document_id,
        "format": descriptor.format,
        "content": content,
        "bytes": metadata.len()
    }))
}

fn create_note(storage: &NotesStorage, input: &Value) -> Result<Value, ToolError> {
    let parent = input
        .get("parent")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let title = required_string(input, "title")?;
    let content = input
        .get("content")
        .and_then(Value::as_str)
        .unwrap_or_default();
    validate_content_size(content)?;

    let descriptor = storage
        .create_document(Path::new(parent), title)
        .map_err(tool_error)?;
    if !content.is_empty() {
        storage
            .write_document(&descriptor.relative_path, content)
            .map_err(tool_error)?;
    }

    Ok(json!({
        "path": display_path(&descriptor.relative_path),
        "documentId": descriptor.document_id,
        "format": descriptor.format,
        "bytes": content.len()
    }))
}

fn write_note(storage: &NotesStorage, input: &Value) -> Result<Value, ToolError> {
    let relative_path = required_path(input)?;
    let content = required_string(input, "content")?;
    validate_content_size(content)?;
    let descriptor = storage
        .write_document(&relative_path, content)
        .map_err(tool_error)?;
    Ok(json!({
        "path": display_path(&descriptor.relative_path),
        "documentId": descriptor.document_id,
        "format": descriptor.format,
        "bytes": content.len()
    }))
}

fn required_path(input: &Value) -> Result<PathBuf, ToolError> {
    let path = required_string(input, "path")?;
    if path.trim().is_empty() {
        return Err(ToolError::Failed {
            message: "path must not be empty".to_string(),
        });
    }
    Ok(PathBuf::from(path))
}

fn required_string<'a>(input: &'a Value, key: &str) -> Result<&'a str, ToolError> {
    input
        .get(key)
        .and_then(Value::as_str)
        .ok_or_else(|| ToolError::Failed {
            message: format!("{key} must be a string"),
        })
}

fn validate_content_size(content: &str) -> Result<(), ToolError> {
    if content.len() as u64 > MAX_NOTE_BYTES {
        return Err(ToolError::Failed {
            message: format!(
                "Note content is too large for MCP ({} bytes, limit {} bytes)",
                content.len(),
                MAX_NOTE_BYTES
            ),
        });
    }
    Ok(())
}

fn display_path(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

fn tool_error(error: impl std::fmt::Display) -> ToolError {
    ToolError::Failed {
        message: error.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> (tempfile::TempDir, ToolRegistry) {
        let temp = tempfile::tempdir().expect("temporary notes root");
        let storage = NotesStorage::open_with_layout(temp.path().join("notes"), false)
            .expect("notes storage");
        storage.create_notebook("Team Notes", "").expect("notebook");
        let registry = notes_tool_registry_with_storage(Some(storage));
        (temp, registry)
    }

    async fn call(registry: &ToolRegistry, id: &str, input: Value) -> Value {
        registry
            .call(id, input, ToolContext::for_adapter(ToolAdapter::Mcp))
            .await
            .expect("notes tool should succeed")
            .structured_content
    }

    #[test]
    fn registry_exposes_a_dedicated_notes_tool_group() {
        let (_, registry) = fixture();
        let ids = registry
            .list(ToolAdapter::Mcp)
            .into_iter()
            .map(|tool| tool.id)
            .collect::<Vec<_>>();

        assert_eq!(
            vec!["notes.list", "notes.read", "notes.create", "notes.write"],
            ids
        );
        assert_eq!(
            RiskLevel::Read,
            registry
                .get_runtime("notes.list", ToolAdapter::Mcp)
                .expect("list descriptor")
                .annotations
                .risk
        );
        assert_eq!(
            RiskLevel::High,
            registry
                .get_runtime("notes.write", ToolAdapter::Mcp)
                .expect("write descriptor")
                .annotations
                .risk
        );
    }

    #[tokio::test]
    async fn create_read_write_and_list_notes_through_mcp_tools() {
        let (_temp, registry) = fixture();

        let created = call(
            &registry,
            "notes.create",
            json!({
                "title": "Runbook",
                "content": "# Deploy\n"
            }),
        )
        .await;
        assert_eq!("Runbook.md", created["path"]);

        let listed = call(&registry, "notes.list", json!({})).await;
        assert_eq!("Team Notes", listed["notebook"]["name"]);
        assert!(listed["tree"].to_string().contains("Runbook.md"));

        let read = call(&registry, "notes.read", json!({ "path": "Runbook.md" })).await;
        assert_eq!("# Deploy\n", read["content"]);

        call(
            &registry,
            "notes.write",
            json!({
                "path": "Runbook.md",
                "content": "# Updated\n"
            }),
        )
        .await;
        let read = call(&registry, "notes.read", json!({ "path": "Runbook.md" })).await;
        assert_eq!("# Updated\n", read["content"]);
    }

    #[tokio::test]
    async fn note_tools_reject_paths_outside_the_notebook_root() {
        let (temp, registry) = fixture();
        std::fs::write(temp.path().join("outside.md"), "secret").expect("outside note");

        let error = registry
            .call(
                "notes.read",
                json!({ "path": "../outside.md" }),
                ToolContext::for_adapter(ToolAdapter::Mcp),
            )
            .await
            .expect_err("path traversal must be rejected");

        assert!(error.to_string().contains("notes root"));
    }

    #[tokio::test]
    async fn note_tools_explain_when_notes_are_not_initialized() {
        let temp = tempfile::tempdir().expect("temporary notes root");
        let storage = NotesStorage::open_with_layout(temp.path().join("notes"), false)
            .expect("notes storage");
        let registry = notes_tool_registry_with_storage(Some(storage));

        let error = registry
            .call(
                "notes.list",
                json!({}),
                ToolContext::for_adapter(ToolAdapter::Mcp),
            )
            .await
            .expect_err("uninitialized notes should be reported");

        assert!(error.to_string().contains("not initialized"));
    }
}
