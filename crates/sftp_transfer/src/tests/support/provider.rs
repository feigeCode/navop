use std::{
    collections::HashMap,
    path::PathBuf,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
};

use anyhow::{Result, anyhow};
use async_trait::async_trait;
use sftp::ProgressCallback;
use tokio::sync::oneshot;

use super::super::super::{
    SftpDeleteRemoteExecution, SftpDownloadExecution, SftpRemoteDeleteEntry, SftpTransferId,
    SftpTransferProvider, SftpUploadConnection, SftpUploadExecution,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum TestConnectionSourceKind {
    SessionManager,
    Config,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum TestTransferOperation {
    Upload,
    Download,
    DeleteRemote,
}

#[derive(Clone, Default)]
pub(crate) struct TestProvider {
    state: Arc<Mutex<TestProviderState>>,
}

#[derive(Default)]
struct TestProviderState {
    started: Vec<SftpTransferId>,
    operations: HashMap<SftpTransferId, TestTransferOperation>,
    paths: HashMap<SftpTransferId, (String, PathBuf, bool)>,
    delete_entries: HashMap<SftpTransferId, Vec<SftpRemoteDeleteEntry>>,
    connection_sources: HashMap<SftpTransferId, TestConnectionSourceKind>,
    completions: HashMap<SftpTransferId, oneshot::Sender<Result<()>>>,
    progress: HashMap<SftpTransferId, ProgressCallback>,
    cancellations: HashMap<SftpTransferId, Arc<AtomicBool>>,
}

struct TestExecution {
    id: SftpTransferId,
    operation: TestTransferOperation,
    connection_source: SftpUploadConnection,
    remote_path: String,
    local_path: PathBuf,
    is_dir: bool,
    cancelled: Arc<AtomicBool>,
}

impl TestProvider {
    pub(crate) fn started(&self) -> Vec<SftpTransferId> {
        self.state.lock().unwrap().started.clone()
    }

    pub(crate) fn operation(&self, id: SftpTransferId) -> Option<TestTransferOperation> {
        self.state.lock().unwrap().operations.get(&id).copied()
    }

    pub(crate) fn paths(&self, id: SftpTransferId) -> Option<(String, PathBuf, bool)> {
        self.state.lock().unwrap().paths.get(&id).cloned()
    }

    pub(crate) fn delete_entries(&self, id: SftpTransferId) -> Option<Vec<SftpRemoteDeleteEntry>> {
        self.state.lock().unwrap().delete_entries.get(&id).cloned()
    }

    pub(crate) fn connection_source(&self, id: SftpTransferId) -> Option<TestConnectionSourceKind> {
        self.state
            .lock()
            .unwrap()
            .connection_sources
            .get(&id)
            .copied()
    }

    pub(crate) fn complete(&self, id: SftpTransferId, result: Result<()>) {
        let sender = {
            let mut state = self.state.lock().unwrap();
            state.progress.remove(&id);
            state.cancellations.remove(&id);
            state
                .completions
                .remove(&id)
                .expect("transfer should be waiting for completion")
        };
        let _ = sender.send(result);
    }

    pub(crate) fn retained_transfer_resource_count(&self) -> usize {
        let state = self.state.lock().unwrap();
        state.progress.len() + state.cancellations.len()
    }

    pub(crate) fn report_progress(&self, id: SftpTransferId, progress: sftp::TransferProgress) {
        let state = self.state.lock().unwrap();
        let callback = state
            .progress
            .get(&id)
            .expect("transfer should expose a progress callback");
        callback(progress);
    }

    pub(crate) fn is_cancelled(&self, id: SftpTransferId) -> bool {
        self.state
            .lock()
            .unwrap()
            .cancellations
            .get(&id)
            .is_some_and(|cancelled| cancelled.load(Ordering::Relaxed))
    }

    async fn run(&self, execution: TestExecution, progress: ProgressCallback) -> Result<()> {
        let receiver = {
            let (sender, receiver) = oneshot::channel();
            let mut state = self.state.lock().unwrap();
            state.started.push(execution.id);
            state.operations.insert(execution.id, execution.operation);
            state.paths.insert(
                execution.id,
                (
                    execution.remote_path,
                    execution.local_path,
                    execution.is_dir,
                ),
            );
            state.connection_sources.insert(
                execution.id,
                connection_source_kind(&execution.connection_source),
            );
            state.completions.insert(execution.id, sender);
            state.progress.insert(execution.id, progress);
            state
                .cancellations
                .insert(execution.id, execution.cancelled);
            receiver
        };
        receiver
            .await
            .map_err(|_| anyhow!("test completion channel closed"))?
    }
}

#[async_trait]
impl SftpTransferProvider for TestProvider {
    async fn upload(
        &self,
        execution: SftpUploadExecution,
        progress: ProgressCallback,
    ) -> Result<()> {
        self.run(
            TestExecution {
                id: execution.id,
                operation: TestTransferOperation::Upload,
                connection_source: execution.connection_source,
                remote_path: execution.remote_path,
                local_path: execution.local_path,
                is_dir: execution.is_dir,
                cancelled: execution.cancelled,
            },
            progress,
        )
        .await
    }

    async fn download(
        &self,
        execution: SftpDownloadExecution,
        progress: ProgressCallback,
    ) -> Result<()> {
        self.run(
            TestExecution {
                id: execution.id,
                operation: TestTransferOperation::Download,
                connection_source: execution.connection_source,
                remote_path: execution.remote_path,
                local_path: execution.local_path,
                is_dir: execution.is_dir,
                cancelled: execution.cancelled,
            },
            progress,
        )
        .await
    }

    async fn delete_remote(
        &self,
        execution: SftpDeleteRemoteExecution,
        progress: ProgressCallback,
    ) -> Result<()> {
        self.state
            .lock()
            .unwrap()
            .delete_entries
            .insert(execution.id, execution.entries);
        self.run(
            TestExecution {
                id: execution.id,
                operation: TestTransferOperation::DeleteRemote,
                connection_source: execution.connection_source,
                remote_path: execution.remote_dir,
                local_path: PathBuf::new(),
                is_dir: false,
                cancelled: execution.cancelled,
            },
            progress,
        )
        .await
    }
}

fn connection_source_kind(source: &SftpUploadConnection) -> TestConnectionSourceKind {
    match source {
        SftpUploadConnection::SessionManager(_) => TestConnectionSourceKind::SessionManager,
        SftpUploadConnection::Config(_) => TestConnectionSourceKind::Config,
    }
}
