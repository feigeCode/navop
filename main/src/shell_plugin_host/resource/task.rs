use extension_host::CancellationToken;
use gpui_shell::{HostAsyncTask, HostError, HostResult};

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
            task.await
                .map_err(|error| HostError::new(format!("provider task failed: {error}")))?
        },
        move || cancel.cancel(),
    )
}

pub(crate) fn host_error(error: extension_host::HostError) -> HostError {
    HostError::new(error.to_string())
}
