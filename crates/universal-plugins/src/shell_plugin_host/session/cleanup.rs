use std::time::Duration;

use super::*;
use crate::shell_plugin_host::error::{ErrorCode, navop_error, service_error};

/// provider 侧关闭的总预算;超时后放弃剩余句柄,交由 runtime 最终 shutdown 兜底。
const CLOSE_ALL_TIMEOUT: Duration = Duration::from_secs(5);

/// 从 resource 记录派生、需要在 provider 侧一并关闭的子句柄。
#[derive(Default)]
pub(crate) struct ScopedHandles {
    pub(super) blobs: Vec<ProviderHandle>,
    pub(super) events: Vec<ProviderHandle>,
    pub(super) jobs: Vec<JobHandle>,
}

impl ScopedHandles {
    pub(crate) async fn close(self, service: &UniversalPluginService) {
        close_all(service, Vec::new(), self.blobs, self.events, self.jobs).await;
    }
}

pub(super) fn new_handle(kind: &str) -> String {
    format!("{kind}-{}", uuid::Uuid::new_v4())
}

pub(super) fn invalid_handle(kind: &str, handle: &str) -> HostError {
    navop_error(
        ErrorCode::InvalidHandle,
        format!("invalid {kind} handle `{handle}`"),
    )
}

pub(super) fn stale_handle(kind: &str, handle: &str) -> HostError {
    navop_error(
        ErrorCode::StaleHandle,
        format!("stale {kind} handle `{handle}` after provider restart"),
    )
}

pub(super) fn remove_matching(
    registry: &Mutex<HashMap<String, ProviderHandle>>,
    handle: &str,
    provider_id: &str,
) {
    if let Ok(mut records) = registry.lock()
        && records
            .get(handle)
            .is_some_and(|record| record.provider_id == provider_id)
    {
        records.remove(handle);
    }
}

/// 取出派生自 `resource` 的所有记录;其余记录原位保留。
pub(super) fn drain_scoped(
    registry: &Mutex<HashMap<String, ProviderHandle>>,
    resource: &str,
) -> Vec<ProviderHandle> {
    let Ok(mut records) = registry.lock() else {
        return Vec::new();
    };
    let (scoped, rest): (Vec<_>, Vec<_>) = std::mem::take(&mut *records)
        .into_iter()
        .partition(|(_, record)| record.resource.as_deref() == Some(resource));
    records.extend(rest);
    scoped.into_iter().map(|(_, record)| record).collect()
}

impl ShellMountSession {
    /// 先取消所有在飞行中的调用,再在有界时间内关闭 provider 侧句柄。
    pub(crate) async fn close_all(&self) {
        self.cancel();
        let closing = close_all(
            &self.service,
            take_provider_handles(&self.resources),
            take_provider_handles(&self.blobs),
            take_provider_handles(&self.events),
            take_jobs(&self.jobs),
        );
        if tokio::time::timeout(CLOSE_ALL_TIMEOUT, closing)
            .await
            .is_err()
        {
            tracing::warn!(
                timeout_secs = CLOSE_ALL_TIMEOUT.as_secs(),
                "shell mount cleanup timed out; remaining provider handles left to runtime shutdown"
            );
        }
    }
}

impl Drop for ShellMountSession {
    fn drop(&mut self) {
        self.cancel();
        let resources = take_provider_handles(&self.resources);
        let blobs = take_provider_handles(&self.blobs);
        let events = take_provider_handles(&self.events);
        let jobs = take_jobs(&self.jobs);
        if resources.is_empty() && blobs.is_empty() && events.is_empty() && jobs.is_empty() {
            return;
        }
        let service = self.service.clone();
        self.tokio.spawn(async move {
            let _ = tokio::time::timeout(
                CLOSE_ALL_TIMEOUT,
                close_all(&service, resources, blobs, events, jobs),
            )
            .await;
        });
    }
}

pub(super) fn take_provider_handles(
    registry: &Mutex<HashMap<String, ProviderHandle>>,
) -> Vec<ProviderHandle> {
    registry
        .lock()
        .map(|mut records| std::mem::take(&mut *records))
        .unwrap_or_default()
        .into_values()
        .collect()
}

pub(super) fn take_jobs(registry: &Mutex<HashMap<String, JobHandle>>) -> Vec<JobHandle> {
    registry
        .lock()
        .map(|mut records| std::mem::take(&mut *records))
        .unwrap_or_default()
        .into_values()
        .collect()
}

pub(super) async fn close_all(
    service: &UniversalPluginService,
    resources: Vec<ProviderHandle>,
    blobs: Vec<ProviderHandle>,
    events: Vec<ProviderHandle>,
    jobs: Vec<JobHandle>,
) {
    close_jobs(service, jobs).await;
    close_events(service, events).await;
    close_blobs(service, blobs).await;
    close_resources(service, resources).await;
}

async fn close_jobs(service: &UniversalPluginService, jobs: Vec<JobHandle>) {
    for job in jobs {
        if let Ok(client) = service.universal_plugin_client(&job.provider.runtime_id) {
            let _ = client.cancel_job(&job.provider).await;
            let _ = client.close_job(&job.provider).await;
        }
    }
}

async fn close_events(service: &UniversalPluginService, records: Vec<ProviderHandle>) {
    for record in records {
        if let Ok(client) = current_client(service, &record) {
            let _ = client
                .close_event_stream(&EventCloseParams {
                    stream_id: record.provider_id,
                })
                .await;
        }
    }
}

async fn close_blobs(service: &UniversalPluginService, records: Vec<ProviderHandle>) {
    for record in records {
        if let Ok(client) = current_client(service, &record) {
            let _ = client
                .close_blob(&BlobCloseParams {
                    blob_id: record.provider_id,
                })
                .await;
        }
    }
}

async fn close_resources(service: &UniversalPluginService, records: Vec<ProviderHandle>) {
    for record in records {
        if !record.owned {
            continue;
        }
        if let Ok(client) = current_client(service, &record) {
            let _ = client
                .client()
                .close_resource(&ResourceCloseParams {
                    resource_id: record.provider_id,
                })
                .await;
        }
    }
}

fn current_client(
    service: &UniversalPluginService,
    record: &ProviderHandle,
) -> Result<ManagedUniversalPluginClient, HostError> {
    let client = service
        .universal_plugin_client(&record.runtime_id)
        .map_err(service_error)?;
    if client.generation != record.generation {
        return Err(navop_error(
            ErrorCode::StaleHandle,
            "provider generation changed",
        ));
    }
    Ok(client)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record(resource: Option<&str>) -> ProviderHandle {
        ProviderHandle {
            alias: "search".into(),
            runtime_id: "ext::provider".into(),
            generation: 1,
            provider_id: new_handle("p"),
            owned: true,
            resource: resource.map(str::to_owned),
        }
    }

    #[test]
    fn drain_scoped_only_takes_records_derived_from_the_resource() {
        let registry = Mutex::new(HashMap::from([
            ("a".to_string(), record(Some("resource-1"))),
            ("b".to_string(), record(Some("resource-2"))),
            ("c".to_string(), record(Some("resource-1"))),
        ]));

        let drained = drain_scoped(&registry, "resource-1");

        assert_eq!(drained.len(), 2);
        let remaining = registry.lock().unwrap();
        assert_eq!(remaining.len(), 1);
        assert!(remaining.contains_key("b"));
    }

    #[test]
    fn borrowed_resource_cleanup_guard_is_present() {
        let source = include_str!("cleanup.rs");
        let close_resources = source
            .split("async fn close_resources")
            .nth(1)
            .expect("close_resources exists");
        assert!(
            close_resources.contains("if !record.owned"),
            "page cleanup must skip borrowed primary resources"
        );
    }
}
