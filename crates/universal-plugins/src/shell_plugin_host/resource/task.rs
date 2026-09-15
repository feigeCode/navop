use extension_host::CancellationToken;
use gpui_shell::{HostAsyncTask, HostResult};

use super::super::error::{ErrorCode, navop_error};

pub(crate) fn spawn_provider_task<F>(
    tokio: &tokio::runtime::Handle,
    future: F,
    cancel: CancellationToken,
) -> HostAsyncTask
where
    F: std::future::Future<Output = HostResult> + Send + 'static,
{
    let task = tokio.spawn(future);
    HostAsyncTask::new(
        async move {
            task.await.map_err(|error| {
                navop_error(
                    ErrorCode::RuntimeUnavailable,
                    format!("provider task failed: {error}"),
                )
            })?
        },
        move || cancel.cancel(),
    )
}

pub(crate) use super::super::error::host_error;
