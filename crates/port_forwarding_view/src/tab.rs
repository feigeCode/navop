use std::sync::Arc;
use std::time::Instant;

use gpui::{
    App, AsyncApp, Context, EventEmitter, FocusHandle, Focusable, IntoElement, Render,
    SharedString, Task, Window,
};
use gpui_component::{Icon, IconName, Sizable, Size};
use one_core::gpui_tokio::Tokio;
use one_core::storage::{ActiveConnections, PortForwardingKind, StoredConnection};
use one_core::tab_container::{TabContent, TabContentEvent};
use port_forwarding::{
    DynamicForwardingRequest, LocalForwardingRequest, LocalPortForwardActivity,
    PortForwardingRuntime, build_dynamic_forwarding_request, build_local_forwarding_request,
};
use rust_i18n::t;
use tokio::sync::{mpsc, oneshot};

use crate::PortForwardingTabConfig;
use crate::tab_render::render_tab;
use crate::tab_state::PortForwardingTabState;

enum StartRequest {
    Local(LocalForwardingRequest),
    Dynamic(DynamicForwardingRequest),
}
pub struct PortForwardingTab {
    pub(crate) connection_id: i64,
    pub(crate) name: String,
    pub(crate) kind: PortForwardingKind,
    pub(crate) bind_label: String,
    pub(crate) target_label: String,
    pub(crate) ssh_label: String,
    pub(crate) state: PortForwardingTabState,
    pub(crate) events: Vec<String>,
    pub(crate) started_at: Option<Instant>,
    runtime: Arc<tokio::sync::Mutex<PortForwardingRuntime>>,
    connection: StoredConnection,
    ssh_connection: StoredConnection,
    start_in_flight: bool,
    pending_close: Option<oneshot::Sender<bool>>,
    pub(crate) focus_handle: FocusHandle,
}

impl PortForwardingTab {
    pub fn new(config: PortForwardingTabConfig, cx: &mut Context<Self>) -> Self {
        let mut this = Self {
            connection_id: config.connection_id,
            name: config.connection.name.clone(),
            kind: config.kind,
            bind_label: config.bind_label,
            target_label: config.target_label,
            ssh_label: config.ssh_label,
            state: PortForwardingTabState::starting(),
            events: vec![t!("PortForwardingTab.event_starting").to_string()],
            started_at: None,
            runtime: config.runtime,
            connection: config.connection,
            ssh_connection: config.ssh_connection,
            start_in_flight: false,
            pending_close: None,
            focus_handle: cx.focus_handle(),
        };
        this.start_forwarding(cx);
        this
    }

    fn start_forwarding(&mut self, cx: &mut Context<Self>) {
        let (activity_tx, activity_rx) = mpsc::unbounded_channel();
        let request = match build_request(
            &self.connection,
            &self.ssh_connection,
            self.kind,
            activity_tx,
        ) {
            Ok(request) => request,
            Err(error) => {
                self.finish_start(Err(error), cx);
                return;
            }
        };
        self.listen_for_activity(activity_rx, cx);
        self.start_in_flight = true;
        let runtime = Arc::clone(&self.runtime);
        let connection_id = self.connection_id;
        let task = Tokio::spawn_result(cx, async move {
            let mut runtime = runtime.lock().await;
            match request {
                StartRequest::Local(request) => runtime.start_local(connection_id, request).await,
                StartRequest::Dynamic(request) => {
                    runtime.start_dynamic(connection_id, request).await
                }
            }
        });
        cx.spawn(async move |this, cx: &mut AsyncApp| {
            let result = task.await;
            let _ = this.update(cx, |this, cx| this.finish_start(result, cx));
        })
        .detach();
    }

    fn finish_start(
        &mut self,
        result: anyhow::Result<std::net::SocketAddr>,
        cx: &mut Context<Self>,
    ) {
        self.start_in_flight = false;
        if let Some(reply) = self.pending_close.take() {
            self.finish_start_for_pending_close(result, reply, cx);
            return;
        }
        match result {
            Ok(addr) => {
                self.state = self.state.clone().started(addr);
                self.started_at = Some(Instant::now());
                self.events.push(
                    t!("PortForwardingTab.event_started", addr = addr.to_string()).to_string(),
                );
                cx.global_mut::<ActiveConnections>().add(self.connection_id);
            }
            Err(error) => {
                self.state = self.state.clone().start_failed(error.to_string());
                self.events.push(
                    t!(
                        "PortForwardingTab.event_start_failed",
                        error = error.to_string()
                    )
                    .to_string(),
                );
            }
        }
        cx.emit(TabContentEvent::StateChanged);
        cx.notify();
    }

    fn finish_start_for_pending_close(
        &mut self,
        result: anyhow::Result<std::net::SocketAddr>,
        reply: oneshot::Sender<bool>,
        cx: &mut Context<Self>,
    ) {
        match result {
            Ok(addr) => {
                self.state = self.state.clone().started(addr);
                cx.global_mut::<ActiveConnections>().add(self.connection_id);
                self.stop_for_close(Some(reply), cx);
            }
            Err(error) => {
                self.state = self.state.clone().start_failed(error.to_string());
                self.events.push(
                    t!(
                        "PortForwardingTab.event_start_finished",
                        error = error.to_string()
                    )
                    .to_string(),
                );
                let _ = reply.send(true);
            }
        }
    }

    pub(crate) fn retry_forwarding(&mut self, cx: &mut Context<Self>) {
        if self.start_in_flight {
            return;
        }
        self.state = self.state.clone().retry();
        self.events
            .push(t!("PortForwardingTab.event_retry").to_string());
        self.start_forwarding(cx);
        cx.notify();
    }

    pub(crate) fn stop_forwarding(&mut self, cx: &mut Context<Self>) {
        self.stop_for_close(None, cx);
    }

    pub(crate) fn stop_for_close(
        &mut self,
        reply: Option<oneshot::Sender<bool>>,
        cx: &mut Context<Self>,
    ) {
        self.state = self.state.clone().begin_stop();
        self.events
            .push(t!("PortForwardingTab.event_stopping").to_string());
        if self.start_in_flight {
            self.pending_close = reply;
            cx.notify();
            return;
        }
        let runtime = Arc::clone(&self.runtime);
        let connection_id = self.connection_id;
        let task =
            Tokio::spawn_result(
                cx,
                async move { runtime.lock().await.stop(connection_id).await },
            );
        cx.spawn(async move |this, cx: &mut AsyncApp| {
            let result = task.await;
            let _ = this.update(cx, |this, cx| this.finish_stop(result, reply, cx));
        })
        .detach();
    }

    fn finish_stop(
        &mut self,
        result: anyhow::Result<bool>,
        reply: Option<oneshot::Sender<bool>>,
        cx: &mut Context<Self>,
    ) {
        let can_close = match result {
            Ok(_) => {
                self.state = self.state.clone().stop_succeeded();
                self.events
                    .push(t!("PortForwardingTab.event_stopped").to_string());
                cx.global_mut::<ActiveConnections>()
                    .remove(self.connection_id);
                true
            }
            Err(error) => {
                self.state = self.state.clone().stop_failed(error.to_string());
                self.events.push(
                    t!(
                        "PortForwardingTab.event_stop_failed",
                        error = error.to_string()
                    )
                    .to_string(),
                );
                false
            }
        };
        if let Some(reply) = reply {
            let _ = reply.send(can_close);
        }
        cx.notify();
    }
}

fn build_request(
    connection: &StoredConnection,
    ssh: &StoredConnection,
    kind: PortForwardingKind,
    activity_tx: mpsc::UnboundedSender<LocalPortForwardActivity>,
) -> anyhow::Result<StartRequest> {
    match kind {
        PortForwardingKind::Local => {
            build_local_forwarding_request(connection, ssh).map(|mut request| {
                request.activity_tx = Some(activity_tx);
                StartRequest::Local(request)
            })
        }
        PortForwardingKind::Dynamic => {
            build_dynamic_forwarding_request(connection, ssh).map(StartRequest::Dynamic)
        }
    }
}

impl EventEmitter<TabContentEvent> for PortForwardingTab {}

impl Focusable for PortForwardingTab {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for PortForwardingTab {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        render_tab(self, window, cx)
    }
}

impl TabContent for PortForwardingTab {
    fn content_key(&self) -> &'static str {
        "PortForwarding"
    }

    fn title(&self, _cx: &App) -> SharedString {
        self.name.clone().into()
    }

    fn icon(&self, _cx: &App) -> Option<Icon> {
        Some(
            IconName::PortForwardingColor
                .color()
                .with_size(Size::Medium),
        )
    }

    fn can_rename(&self, _cx: &App) -> bool {
        false
    }

    fn try_close(
        &mut self,
        tab_id: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Task<bool> {
        crate::tab_close::try_close(self, tab_id, window, cx)
    }
}
