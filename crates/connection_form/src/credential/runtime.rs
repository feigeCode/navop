use gpui::App;
use one_core::storage::{
    ConnectionRepository, CredentialRepository, GlobalStorageState, SshParams, StoredConnection,
};

/// Resolves every password-book reference needed by an actual connection attempt.
///
/// The returned connection can contain plaintext secrets. It is runtime-only
/// and must never be persisted, synchronized, logged, shared, or exported.
pub fn resolve_connection_for_runtime(
    connection: StoredConnection,
    cx: &App,
) -> Result<StoredConnection, String> {
    let repository = cx
        .try_global::<GlobalStorageState>()
        .and_then(|state| state.storage.get::<ConnectionRepository>())
        .ok_or_else(|| "ConnectionRepository not found".to_string())?;

    repository
        .resolve_runtime_connection(&connection)
        .map_err(|error| format!("{error:#}"))
}

/// Resolves password-book references in SSH parameters without applying the
/// persistence sanitization performed by `StoredConnection::new_ssh`.
///
/// This is required for form-level connection attempts where prompt-only
/// username or password values may intentionally exist only in memory.
pub fn resolve_ssh_for_runtime(params: SshParams, cx: &App) -> Result<SshParams, String> {
    let repository = cx
        .try_global::<GlobalStorageState>()
        .and_then(|state| state.storage.get::<CredentialRepository>())
        .ok_or_else(|| "CredentialRepository not found".to_string())?;

    repository
        .resolve_ssh(params)
        .map_err(|error| format!("{error:#}"))
}
