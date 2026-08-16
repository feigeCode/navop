use gpui::{Context, Window};
use one_core::gpui_tokio::Tokio;
use rust_i18n::t;
use tokio::sync::mpsc::error::TrySendError;

use super::SendContext;
use crate::api_test_view::ApiTestView;
use crate::api_test_view::websocket_state::EventDisposition;
use crate::websocket::{ConnectionCommand, ConnectionTask, build_client_request, start_connection};

impl ApiTestView {
    pub(super) fn connect_websocket(&mut self, context: SendContext, cx: &mut Context<Self>) {
        self.cancel_websocket();
        let request = match build_client_request(&context.prepared) {
            Ok(request) => request,
            Err(error) => {
                self.fail_websocket_connection(error.to_string(), cx);
                return;
            }
        };
        self.websocket_generation = self.websocket_generation.wrapping_add(1);
        let generation = self.websocket_generation;
        self.websocket_state.begin(generation);
        self.prepared_request = Some(context.prepared);
        self.sending = true;

        let ConnectionTask {
            commands,
            events,
            cancel,
        } = start_connection(&Tokio::handle(cx), request);
        self.websocket_commands = Some(commands);
        self.websocket_cancel = Some(cancel);
        self.spawn_websocket_events(events, generation, cx);
        cx.notify();
    }

    fn spawn_websocket_events(
        &self,
        mut events: tokio::sync::mpsc::Receiver<crate::websocket::ConnectionEventEnvelope>,
        generation: u64,
        cx: &mut Context<Self>,
    ) {
        cx.spawn(async move |this, cx| {
            while let Some(event) = events.recv().await {
                let disposition = this.update(cx, |view, cx| {
                    let disposition = view.websocket_state.apply_event(generation, event);
                    if disposition != EventDisposition::Ignored {
                        view.sending = false;
                        if disposition == EventDisposition::Terminal {
                            view.clear_websocket_transport();
                        }
                        cx.notify();
                    }
                    disposition
                });
                if !matches!(disposition, Ok(EventDisposition::Continue)) {
                    break;
                }
            }
            _ = this.update(cx, |view, cx| {
                if view.websocket_state.finish(generation) {
                    view.clear_websocket_transport();
                    view.sending = false;
                    cx.notify();
                }
            });
        })
        .detach();
    }

    pub(in crate::api_test_view) fn disconnect_websocket(&mut self, cx: &mut Context<Self>) {
        let result = self
            .websocket_commands
            .as_ref()
            .map(|commands| commands.try_send(ConnectionCommand::Close));
        match result {
            Some(Ok(())) => {
                self.websocket_state.set_closing();
                self.notice = None;
            }
            Some(Err(TrySendError::Full(_))) => {
                self.notice = Some(t!("ApiTest.websocket_queue_full").to_string());
            }
            Some(Err(TrySendError::Closed(_))) | None => self.finish_websocket_transport(),
        }
        cx.notify();
    }

    pub(in crate::api_test_view) fn cancel_websocket(&mut self) {
        if let Some(cancel) = self.websocket_cancel.take() {
            _ = cancel.send(());
        }
        self.websocket_commands = None;
        self.websocket_generation = self.websocket_generation.wrapping_add(1);
        self.websocket_state.cancel(self.websocket_generation);
        self.sending = false;
    }

    pub(in crate::api_test_view) fn send_websocket_message(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let message = self
            .websocket_message_input
            .read(cx)
            .value()
            .trim_end_matches(['\r', '\n'])
            .to_string();
        if message.trim().is_empty() || !self.websocket_state.state.is_connected() {
            return;
        }
        let Some(commands) = self.websocket_commands.as_ref() else {
            return;
        };
        match commands.try_send(ConnectionCommand::Text(message.clone())) {
            Ok(()) => {
                self.websocket_state.push_sent_text(message);
                self.websocket_message_input.update(cx, |state, cx| {
                    state.set_value("", window, cx);
                });
                self.notice = None;
            }
            Err(TrySendError::Full(_)) => {
                self.notice = Some(t!("ApiTest.websocket_queue_full").to_string());
            }
            Err(TrySendError::Closed(_)) => {
                self.notice = Some(t!("ApiTest.websocket_connection_closed").to_string());
                self.finish_websocket_transport();
            }
        }
        cx.notify();
    }

    fn fail_websocket_connection(&mut self, error: String, cx: &mut Context<Self>) {
        self.websocket_generation = self.websocket_generation.wrapping_add(1);
        let generation = self.websocket_generation;
        self.websocket_state.begin(generation);
        self.websocket_state.apply_event(
            generation,
            crate::websocket::ConnectionEventEnvelope::Error(error.clone()),
        );
        self.notice = Some(error);
        self.sending = false;
        self.clear_websocket_transport();
        cx.notify();
    }

    fn finish_websocket_transport(&mut self) {
        self.clear_websocket_transport();
        self.websocket_state.finish(self.websocket_generation);
        self.sending = false;
    }

    fn clear_websocket_transport(&mut self) {
        self.websocket_commands = None;
        self.websocket_cancel = None;
    }
}
