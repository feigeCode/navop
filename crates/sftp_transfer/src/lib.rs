#[cfg(test)]
mod cancellation_watcher;
mod conflict;
mod executor;
mod global;
mod history;
mod model;
mod operation;
mod progress;
mod provider;
mod record;
mod scheduler;
mod ssh_config;

pub use conflict::UploadConflictResolver;
pub use executor::{SftpTransferExecutor, SftpTransferReservation};
pub use global::{global, init, init_with_provider};
pub use model::{
    SftpConnectionIdentity, SftpDeleteRemoteExecution, SftpDeleteRemoteRequest,
    SftpDownloadExecution, SftpDownloadRequest, SftpRemoteDeleteEntry, SftpTransferEvent,
    SftpTransferId, SftpTransferOperation, SftpTransferSnapshot, SftpTransferState,
    SftpUploadConnection, SftpUploadExecution, SftpUploadRequest, connection_endpoint_label,
    delete_remote_task_key, download_task_key, ftp_connect_config_from_stored, upload_task_key,
};
pub use provider::{RusshSftpTransferProvider, SftpTransferProvider};
pub use ssh_config::{
    ResolvedSshTarget, resolve_ssh_target, sftp_initial_directory, sftp_initial_directory_of,
    ssh_auth, ssh_config_for,
};

#[cfg(test)]
mod tests;
