use std::sync::Arc;

use base64::Engine as _;
use extension_host::CancellationToken;
use extension_protocol::blob::{BlobCloseParams, BlobReadParams, MAX_BLOB_CHUNK_BYTES};
use gpui_shell::{HostAsyncTask, HostError, HostModule, HostObject, HostValue};

use super::{
    resource::task::{host_error, spawn_provider_task},
    session::ShellMountSession,
};

pub(super) fn blob_module(session: Arc<ShellMountSession>) -> HostModule {
    HostModule::new("navop.blob")
        .declarations(
            r#"
            export function read(handle: string, maxBytes?: number): Promise<{ data: string; bytesRead: number; done: boolean }>;
            export function close(handle: string): Promise<void>;
            "#,
        )
        .cancellable_async_function("read", read_blob(Arc::clone(&session)))
        .cancellable_async_function("close", close_blob(session))
}

fn read_blob(
    session: Arc<ShellMountSession>,
) -> impl Fn(&gpui_shell::HostArguments) -> Result<HostAsyncTask, HostError> {
    move |arguments| {
        let handle = arguments.string(0)?.to_owned();
        let max_bytes = arguments
            .get(1)
            .map(|_| arguments.integer(1))
            .transpose()?
            .map(valid_chunk_size)
            .transpose()?;
        let (client, blob_id) = session.blob(&handle)?;
        let cancel = CancellationToken::new();
        Ok(spawn_provider_task(
            &session.tokio,
            async move {
                let result = client
                    .read_blob(&BlobReadParams { blob_id, max_bytes })
                    .await
                    .map_err(host_error)?;
                let _ = base64::engine::general_purpose::STANDARD
                    .decode(&result.data)
                    .map_err(|_| HostError::new("provider returned invalid blob base64"))?;
                Ok(HostObject::new()
                    .field("data", result.data)
                    .field("bytesRead", result.bytes_read)
                    .field("done", result.done)
                    .into())
            },
            cancel,
        ))
    }
}

fn close_blob(
    session: Arc<ShellMountSession>,
) -> impl Fn(&gpui_shell::HostArguments) -> Result<HostAsyncTask, HostError> {
    move |arguments| {
        let handle = arguments.string(0)?.to_owned();
        let (client, blob_id) = session.blob(&handle)?;
        let task_session = Arc::clone(&session);
        let cancel = CancellationToken::new();
        Ok(spawn_provider_task(
            &session.tokio,
            async move {
                client
                    .close_blob(&BlobCloseParams {
                        blob_id: blob_id.clone(),
                    })
                    .await
                    .map_err(host_error)?;
                task_session.close_blob_record(&handle, &blob_id);
                Ok(HostValue::Null)
            },
            cancel,
        ))
    }
}

fn valid_chunk_size(value: i64) -> Result<u32, HostError> {
    u32::try_from(value)
        .ok()
        .filter(|value| (1..=MAX_BLOB_CHUNK_BYTES).contains(value))
        .ok_or_else(|| {
            HostError::new(format!(
                "maxBytes must be between 1 and {MAX_BLOB_CHUNK_BYTES}"
            ))
        })
}
