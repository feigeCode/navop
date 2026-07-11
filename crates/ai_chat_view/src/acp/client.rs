use std::path::{Path, PathBuf};

use agent_client_protocol::schema::{
    ClientCapabilities, FileSystemCapabilities, InitializeRequest, ProtocolVersion,
    ReadTextFileRequest, ReadTextFileResponse, WriteTextFileRequest, WriteTextFileResponse,
};

pub(super) fn build_initialize_request() -> InitializeRequest {
    InitializeRequest::new(ProtocolVersion::V1).client_capabilities(build_client_capabilities())
}

fn build_client_capabilities() -> ClientCapabilities {
    ClientCapabilities::new().fs(FileSystemCapabilities::new()
        .read_text_file(true)
        .write_text_file(true))
}

pub(super) fn handle_read_text_file_request(
    request: &ReadTextFileRequest,
    root: &Path,
) -> Result<ReadTextFileResponse, agent_client_protocol::Error> {
    validate_workspace_path(&request.path, root)?;
    let text = std::fs::read_to_string(&request.path)
        .map_err(|err| agent_client_protocol::Error::internal_error().data(format!("{err}")))?;
    Ok(ReadTextFileResponse::new(read_text_slice(
        &text,
        request.line,
        request.limit,
    )))
}

pub(super) fn handle_write_text_file_request(
    request: &WriteTextFileRequest,
    root: &Path,
) -> Result<WriteTextFileResponse, agent_client_protocol::Error> {
    validate_workspace_path(&request.path, root)?;
    std::fs::write(&request.path, &request.content)
        .map_err(|err| agent_client_protocol::Error::internal_error().data(format!("{err}")))?;
    Ok(WriteTextFileResponse::new())
}

fn validate_workspace_path(path: &Path, root: &Path) -> Result<(), agent_client_protocol::Error> {
    if workspace_path_allowed(path, root) {
        Ok(())
    } else {
        Err(agent_client_protocol::Error::invalid_params()
            .data(format!("path is outside ACP workspace: {}", path.display())))
    }
}

fn read_text_slice(text: &str, line: Option<u32>, limit: Option<u32>) -> String {
    let start = line.unwrap_or(1).saturating_sub(1) as usize;
    let selected = text.lines().skip(start);
    match limit {
        Some(limit) => selected.take(limit as usize).collect::<Vec<_>>().join("\n"),
        None => selected.collect::<Vec<_>>().join("\n"),
    }
}

fn workspace_path_allowed(path: &Path, root: &Path) -> bool {
    path.is_absolute() && normalize_path(path).starts_with(normalize_path(root))
}

fn normalize_path(path: &Path) -> PathBuf {
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            std::path::Component::ParentDir => {
                normalized.pop();
            }
            std::path::Component::CurDir => {}
            other => normalized.push(other.as_os_str()),
        }
    }
    normalized
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use agent_client_protocol::schema::FileSystemCapabilities;

    use super::{build_client_capabilities, read_text_slice, workspace_path_allowed};

    #[test]
    fn client_capabilities_match_registered_client_handlers() {
        let capabilities = build_client_capabilities();
        assert_eq!(
            FileSystemCapabilities::new()
                .read_text_file(true)
                .write_text_file(true),
            capabilities.fs
        );
        assert!(!capabilities.terminal);
    }

    #[test]
    fn read_text_slice_uses_one_based_line_and_limit() {
        let text = "one\ntwo\nthree\nfour\n";
        assert_eq!("two\nthree", read_text_slice(text, Some(2), Some(2)));
        assert_eq!("one\ntwo", read_text_slice(text, None, Some(2)));
        assert_eq!("three\nfour", read_text_slice(text, Some(3), None));
    }

    #[test]
    fn workspace_path_allowed_rejects_paths_outside_root() {
        let root = Path::new("/workspace/project");
        assert!(workspace_path_allowed(
            Path::new("/workspace/project/src/main.rs"),
            root
        ));
        assert!(!workspace_path_allowed(
            Path::new("/workspace/project/../secret"),
            root
        ));
        assert!(!workspace_path_allowed(Path::new("relative/path"), root));
    }
}
