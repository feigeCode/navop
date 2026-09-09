use super::*;

pub(super) fn new_handle(kind: &str) -> String {
    format!("{kind}-{}", uuid::Uuid::new_v4())
}

pub(super) fn invalid_handle(kind: &str, handle: &str) -> HostError {
    HostError::new(format!("invalid {kind} handle `{handle}`"))
}

pub(super) fn stale_handle(kind: &str, handle: &str) -> HostError {
    HostError::new(format!(
        "stale {kind} handle `{handle}` after provider restart"
    ))
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

impl ShellMountSession {
    pub(crate) async fn close_all(&self) {
        close_all(
            &self.service,
            take_provider_handles(&self.resources),
            take_provider_handles(&self.blobs),
            take_provider_handles(&self.events),
            take_jobs(&self.jobs),
        )
        .await;
    }
}

impl Drop for ShellMountSession {
    fn drop(&mut self) {
        let resources = take_provider_handles(&self.resources);
        let blobs = take_provider_handles(&self.blobs);
        let events = take_provider_handles(&self.events);
        let jobs = take_jobs(&self.jobs);
        if resources.is_empty() && blobs.is_empty() && events.is_empty() && jobs.is_empty() {
            return;
        }
        let service = self.service.clone();
        self.tokio.spawn(async move {
            close_all(&service, resources, blobs, events, jobs).await;
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
        .map_err(|error| HostError::new(error.to_string()))?;
    if client.generation != record.generation {
        return Err(HostError::new("provider generation changed"));
    }
    Ok(client)
}
