use crate::onetcli_app::GlobalHomePage;
use gpui::{App, AsyncApp};
use gpui_component::WindowExt;
use one_core::connection_notifier::{ConnectionDataEvent, get_notifier};
use one_core::storage::StoredConnection;
use one_core::tab_container::TabOpenMode;
use serde_json::{Value, json};
use std::sync::Arc;
use tokio::sync::{mpsc, oneshot};
use tool_runtime::{ToolError, ToolFuture, ToolResult};

use onetcli_runtime::connections::{
    ConnectionSaveEvent, ConnectionSaveNotifier, ConnectionSaveNotifyFuture,
};

struct OpenConnectionRequest {
    connection: StoredConnection,
    reply: oneshot::Sender<Result<Value, String>>,
}

struct SaveNotificationRequest {
    event: ConnectionSaveEvent,
}

#[derive(Clone)]
struct GpuiConnectionSessionOpener {
    requests: mpsc::UnboundedSender<OpenConnectionRequest>,
}

#[derive(Clone)]
struct GpuiConnectionSaveNotifier {
    requests: mpsc::UnboundedSender<SaveNotificationRequest>,
}

pub(super) fn connection_session_opener(
    cx: &mut App,
    open_mode: TabOpenMode,
) -> Arc<dyn onetcli_runtime::connections::ConnectionSessionOpener> {
    let (tx, mut rx) = mpsc::unbounded_channel::<OpenConnectionRequest>();
    cx.spawn(async move |cx: &mut AsyncApp| {
        while let Some(request) = rx.recv().await {
            let result = cx
                .update(|cx| open_connection_on_active_window(request.connection, open_mode, cx))
                .map_err(|error| error.to_string());
            let _ = request.reply.send(result);
        }
        Ok::<(), anyhow::Error>(())
    })
    .detach();
    Arc::new(GpuiConnectionSessionOpener { requests: tx })
}

pub(super) fn connection_save_notifier(cx: &mut App) -> Arc<dyn ConnectionSaveNotifier> {
    let (tx, mut rx) = mpsc::unbounded_channel::<SaveNotificationRequest>();
    cx.spawn(async move |cx: &mut AsyncApp| {
        while let Some(request) = rx.recv().await {
            cx.update(|cx| emit_connection_save_event(request.event, cx));
        }
        Ok::<(), anyhow::Error>(())
    })
    .detach();
    Arc::new(GpuiConnectionSaveNotifier { requests: tx })
}

impl onetcli_runtime::connections::ConnectionSessionOpener for GpuiConnectionSessionOpener {
    fn open_session(&self, connection: StoredConnection) -> ToolFuture {
        let requests = self.requests.clone();
        Box::pin(async move {
            let (reply, response) = oneshot::channel();
            requests
                .send(OpenConnectionRequest { connection, reply })
                .map_err(|_| ToolError::Failed {
                    message: "connection session opener is no longer available".to_string(),
                })?;
            let opened = response.await.map_err(|_| ToolError::Failed {
                message: "connection session opener dropped before replying".to_string(),
            })?;
            Ok(ToolResult::structured(
                opened.map_err(|message| ToolError::Failed { message })?,
            ))
        })
    }
}

impl ConnectionSaveNotifier for GpuiConnectionSaveNotifier {
    fn notify_save(&self, event: ConnectionSaveEvent) -> ConnectionSaveNotifyFuture {
        let requests = self.requests.clone();
        Box::pin(async move {
            requests
                .send(SaveNotificationRequest { event })
                .map_err(|_| ToolError::Failed {
                    message: "connection save notifier is no longer available".to_string(),
                })?;
            Ok(())
        })
    }
}

fn emit_connection_save_event(event: ConnectionSaveEvent, cx: &mut App) {
    let Some(notifier) = get_notifier(cx) else {
        tracing::warn!("connection save notification skipped because notifier is not initialized");
        return;
    };
    notifier.update(cx, |_, cx| {
        cx.emit(match event {
            ConnectionSaveEvent::Created(connection) => {
                ConnectionDataEvent::ConnectionCreated { connection }
            }
            ConnectionSaveEvent::Updated(connection) => {
                ConnectionDataEvent::ConnectionUpdated { connection }
            }
        });
    });
}

fn open_connection_on_active_window(
    connection: StoredConnection,
    open_mode: TabOpenMode,
    cx: &mut App,
) -> Result<Value, String> {
    let active_window = cx
        .active_window()
        .ok_or_else(|| "no active Navop window is available".to_string())?;
    let home_page = cx
        .try_global::<GlobalHomePage>()
        .ok_or_else(|| "home page is not initialized".to_string())?
        .home_page
        .clone();
    let connection_id = connection.id;
    let connection_name = connection.name.clone();
    let connection_type = connection.connection_type.label().to_string();

    active_window
        .update(cx, |_, window, cx| {
            if open_mode == TabOpenMode::Activate && window.has_active_dialog(cx) {
                window.close_all_dialogs(cx);
            }
            home_page.update(cx, |home, cx| {
                home.open_connection_from_quick_with_mode(&connection, open_mode, window, cx);
            });
        })
        .map_err(|error| error.to_string())?;

    Ok(json!({
        "target": "active_window",
        "connection_id": connection_id,
        "connection_name": connection_name,
        "connection_type": connection_type,
        "activated": open_mode == TabOpenMode::Activate
    }))
}
