use std::sync::Arc;

use extension_host::{CancellationToken, RequestOptions};
use extension_protocol::event_stream::{
    EventCloseParams, EventOpenParams, EventReadParams, MAX_EVENT_MAX_EVENTS,
};
use gpui_shell::{HostAsyncTask, HostError, HostModule, HostObject, HostValue};

use super::{
    resource::task::{host_error, spawn_provider_task},
    session::ShellMountSession,
    value::json_to_host,
};

pub(super) fn event_module(session: Arc<ShellMountSession>) -> HostModule {
    HostModule::new("navop.event")
        .declarations(
            r#"
            export function open(resource: string, kind: string, capacity?: number): Promise<{ handle: string }>;
            export function read(handle: string, maxEvents?: number, waitMs?: number): Promise<{ events: unknown[]; closed: boolean; droppedCount: number }>;
            export function close(handle: string): Promise<void>;
            "#,
        )
        .cancellable_async_function("open", open_event(Arc::clone(&session)))
        .cancellable_async_function("read", read_event(Arc::clone(&session)))
        .cancellable_async_function("close", close_event(session))
}

fn open_event(
    session: Arc<ShellMountSession>,
) -> impl Fn(&gpui_shell::HostArguments) -> Result<HostAsyncTask, HostError> {
    move |arguments| {
        let resource = arguments.string(0)?.to_owned();
        let kind = arguments.string(1)?.to_owned();
        let capacity = optional_u32(arguments, 2, MAX_EVENT_MAX_EVENTS)?;
        let (alias, client, _) = session.resource(&resource)?;
        let generation = client.generation;
        let task_session = Arc::clone(&session);
        let cancel = CancellationToken::new();
        Ok(spawn_provider_task(
            &session.tokio,
            async move {
                let result = client
                    .open_event_stream(&EventOpenParams {
                        conn_id: None,
                        kind,
                        capacity,
                    })
                    .await
                    .map_err(host_error)?;
                let handle = task_session.register_event(alias, generation, &result.stream_id)?;
                Ok(HostObject::new().field("handle", handle).into())
            },
            cancel,
        ))
    }
}

fn read_event(
    session: Arc<ShellMountSession>,
) -> impl Fn(&gpui_shell::HostArguments) -> Result<HostAsyncTask, HostError> {
    move |arguments| {
        let handle = arguments.string(0)?.to_owned();
        let max_events = optional_u32(arguments, 1, MAX_EVENT_MAX_EVENTS)?;
        let wait_ms = optional_u32(arguments, 2, 60_000)?;
        let (client, stream_id) = session.event(&handle)?;
        let cancel = CancellationToken::new();
        let request_cancel = cancel.clone();
        Ok(spawn_provider_task(
            &session.tokio,
            async move {
                let result = client
                    .read_event_stream_with_options(
                        &EventReadParams {
                            stream_id,
                            max_events,
                            wait_ms,
                        },
                        RequestOptions::default().with_cancel(request_cancel),
                    )
                    .await
                    .map_err(host_error)?;
                json_to_host(
                    &serde_json::to_value(result).map_err(|e| HostError::new(e.to_string()))?,
                )
            },
            cancel,
        ))
    }
}

fn close_event(
    session: Arc<ShellMountSession>,
) -> impl Fn(&gpui_shell::HostArguments) -> Result<HostAsyncTask, HostError> {
    move |arguments| {
        let handle = arguments.string(0)?.to_owned();
        let (client, stream_id) = session.event(&handle)?;
        let task_session = Arc::clone(&session);
        let cancel = CancellationToken::new();
        Ok(spawn_provider_task(
            &session.tokio,
            async move {
                client
                    .close_event_stream(&EventCloseParams {
                        stream_id: stream_id.clone(),
                    })
                    .await
                    .map_err(host_error)?;
                task_session.close_event_record(&handle, &stream_id);
                Ok(HostValue::Null)
            },
            cancel,
        ))
    }
}

fn optional_u32(
    arguments: &gpui_shell::HostArguments,
    index: usize,
    max: u32,
) -> Result<Option<u32>, HostError> {
    arguments
        .get(index)
        .map(|_| arguments.integer(index))
        .transpose()?
        .map(|value| {
            u32::try_from(value)
                .ok()
                .filter(|value| *value <= max)
                .ok_or_else(|| {
                    HostError::new(format!(
                        "argument {} must be between 0 and {max}",
                        index + 1
                    ))
                })
        })
        .transpose()
}
