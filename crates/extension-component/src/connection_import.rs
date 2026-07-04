use connection_import_protocol::{
    CandidateFile, DirectoryEntry, HostAccessError, Platform, SecretQuery, SecretResult,
};

use crate::PermissionSet;

pub trait ExtensionConnectionImportHost: Send + Sync {
    fn current_platform(&self) -> Platform;

    fn list_candidate_files(&self, importer_id: &str) -> Vec<CandidateFile>;

    fn read_file(&self, candidate_id: &str) -> Result<Vec<u8>, HostAccessError>;

    fn read_directory(&self, candidate_id: &str) -> Result<Vec<DirectoryEntry>, HostAccessError>;

    fn read_candidate_child_file(
        &self,
        candidate_id: &str,
        _relative_path: &str,
    ) -> Result<Vec<u8>, HostAccessError> {
        Err(HostAccessError::UndeclaredCandidate(
            candidate_id.to_string(),
        ))
    }

    fn read_secret(&self, query: SecretQuery) -> SecretResult;

    fn log(&self, level: &str, message: &str);
}

#[derive(Clone, Debug)]
pub struct CandidateFileAccess {
    candidates: Vec<CandidateFile>,
    permissions: PermissionSet,
}

impl CandidateFileAccess {
    pub fn new(candidates: Vec<CandidateFile>, permissions: PermissionSet) -> Self {
        Self {
            candidates,
            permissions,
        }
    }

    pub fn candidate(&self, candidate_id: &str) -> Result<&CandidateFile, HostAccessError> {
        let candidate = self
            .candidates
            .iter()
            .find(|candidate| candidate.id == candidate_id)
            .ok_or_else(|| HostAccessError::UndeclaredCandidate(candidate_id.to_string()))?;
        if self.permissions.allows_fs_read(&candidate.path) {
            Ok(candidate)
        } else {
            Err(HostAccessError::PermissionDenied(candidate.path.clone()))
        }
    }
}

#[derive(Default)]
pub struct NoopConnectionImportHost;

impl ExtensionConnectionImportHost for NoopConnectionImportHost {
    fn current_platform(&self) -> Platform {
        Platform::Macos
    }

    fn list_candidate_files(&self, _importer_id: &str) -> Vec<CandidateFile> {
        Vec::new()
    }

    fn read_file(&self, candidate_id: &str) -> Result<Vec<u8>, HostAccessError> {
        Err(HostAccessError::UndeclaredCandidate(
            candidate_id.to_string(),
        ))
    }

    fn read_directory(&self, candidate_id: &str) -> Result<Vec<DirectoryEntry>, HostAccessError> {
        Err(HostAccessError::UndeclaredCandidate(
            candidate_id.to_string(),
        ))
    }

    fn read_secret(&self, _query: SecretQuery) -> SecretResult {
        SecretResult::Unsupported
    }

    fn log(&self, _level: &str, _message: &str) {}
}

#[cfg(test)]
mod tests {
    use connection_import_protocol::{
        CandidateFile, HostAccessError, Platform, SecretQuery, SecretResult,
    };

    use super::{CandidateFileAccess, ExtensionConnectionImportHost, NoopConnectionImportHost};
    use crate::PermissionSet;

    #[test]
    fn candidate_access_rejects_undeclared_candidate_ids() {
        let access = CandidateFileAccess::new(
            vec![CandidateFile {
                id: "navicat-conn".to_string(),
                platform: Some(Platform::Macos),
                path: "~/Library/Application Support/Navicat/conn.plist".to_string(),
            }],
            PermissionSet::new(["fs:read:~/Library/Application Support/Navicat/conn.plist"]),
        );

        let error = access
            .candidate("arbitrary-path")
            .expect_err("undeclared candidate id must be rejected");

        assert_eq!(
            HostAccessError::UndeclaredCandidate("arbitrary-path".to_string()),
            error
        );
    }

    #[test]
    fn noop_secret_backend_reports_unsupported() {
        let host = NoopConnectionImportHost::default();

        let result = host.read_secret(SecretQuery {
            service: "Navicat".to_string(),
            account: "root@localhost".to_string(),
            namespace: None,
            key: None,
        });

        assert_eq!(SecretResult::Unsupported, result);
    }
}
