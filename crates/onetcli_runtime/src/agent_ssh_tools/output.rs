use agent_runtime::{ToolObservation, tools::ObservationData, tools::ToolInvocation};
use serde_json::{Value, json};
use std::time::UNIX_EPOCH;

pub(super) fn file_entry_json(entry: sftp::FileEntry) -> Value {
    let modified_unix_secs = entry
        .modified
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or_default();
    json!({
        "name": entry.name,
        "path": entry.path,
        "size": entry.size,
        "modified_unix_secs": modified_unix_secs,
        "is_dir": entry.is_dir,
        "permissions": entry.permissions
    })
}

pub(super) fn stat_json(path: String, metadata: Option<sftp::PathMetadata>) -> Value {
    match metadata {
        Some(metadata) => json!({
            "path": path,
            "exists": true,
            "metadata": path_metadata_json(metadata)
        }),
        None => json!({"path": path, "exists": false}),
    }
}

pub(super) fn success_json(
    invocation: ToolInvocation,
    summary: impl Into<String>,
    value: Value,
) -> ToolObservation {
    ToolObservation::success(
        invocation.call_id,
        invocation.tool_name,
        summary,
        ObservationData::Json(value),
    )
}

fn path_metadata_json(metadata: sftp::PathMetadata) -> Value {
    let modified_unix_secs = metadata
        .modified
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or_default();
    json!({
        "size": metadata.size,
        "modified_unix_secs": modified_unix_secs,
        "is_dir": metadata.is_dir,
        "permissions": metadata.permissions
    })
}
