use gpui::{Context, Window};
use one_core::gpui_tokio::Tokio;
use rust_i18n::t;
use tokio::sync::mpsc::error::TrySendError;

use super::SendContext;
use crate::api_test_view::ApiTestView;
use crate::api_test_view::socket_io_state::{EventDisposition, SocketIoTransition};
use crate::socket_io::encode_event_input;
use crate::websocket::{ConnectionCommand, ConnectionTask, build_client_request, start_connection};

impl ApiTestView {
    pub(super) fn connect_socket_io(&mut self, context: SendContext, cx: &mut Context<Self>) {
        self.cancel_socket_io();
        let request = match build_client_request(&context.prepared) {
            Ok(request) => request,
            Err(error) => {
                self.fail_socket_io_connection(error.to_string(), cx);
                return;
            }
        };
        self.socket_io_generation = self.socket_io_generation.wrapping_add(1);
        let generation = self.socket_io_generation;
        self.socket_io_state.begin(generation);
        self.prepared_request = Some(context.prepared);
        self.sending = true;

        let ConnectionTask {
            commands,
            events,
            cancel,
        } = start_connection(&Tokio::handle(cx), request);
        self.socket_io_commands = Some(commands);
        self.socket_io_cancel = Some(cancel);
        self.spawn_socket_io_events(events, generation, cx);
        cx.notify();
    }

    fn spawn_socket_io_events(
        &self,
        mut events: tokio::sync::mpsc::Receiver<crate::websocket::ConnectionEventEnvelope>,
        generation: u64,
        cx: &mut Context<Self>,
    ) {
        cx.spawn(async move |this, cx| {
            while let Some(event) = events.recv().await {
                let disposition = this.update(cx, |view, cx| {
                    let transition = view.socket_io_state.apply_event(generation, event);
                    let disposition = view.apply_socket_io_transition(transition, generation);
                    if disposition != EventDisposition::Ignored {
                        view.sending = false;
                        if disposition == EventDisposition::Terminal {
                            view.clear_socket_io_transport();
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
                if view.socket_io_state.finish(generation) {
                    view.clear_socket_io_transport();
                    view.sending = false;
                    cx.notify();
                }
            });
        })
        .detach();
    }

    fn apply_socket_io_transition(
        &mut self,
        transition: SocketIoTransition,
        generation: u64,
    ) -> EventDisposition {
        let Some(packet) = transition.outbound else {
            return transition.disposition;
        };
        let result = self
            .socket_io_commands
            .as_ref()
            .map(|commands| commands.try_send(ConnectionCommand::Text(packet)));
        match result {
            Some(Ok(())) => transition.disposition,
            Some(Err(TrySendError::Full(_))) => {
                let error = t!("ApiTest.socketio_queue_full").to_string();
                self.notice = Some(error.clone());
                if generation == self.socket_io_generation {
                    self.socket_io_state.fail(error);
                }
                EventDisposition::Terminal
            }
            Some(Err(TrySendError::Closed(_))) | None => {
                let error = t!("ApiTest.socketio_connection_closed").to_string();
                self.notice = Some(error.clone());
                if generation == self.socket_io_generation {
                    self.socket_io_state.fail(error);
                }
                EventDisposition::Terminal
            }
        }
    }

    pub(in crate::api_test_view) fn disconnect_socket_io(&mut self, cx: &mut Context<Self>) {
        let namespace_connected = self.socket_io_state.state.is_connected();
        self.socket_io_state.set_closing();

        let mut queue_full = false;
        let mut channel_closed = false;
        if let Some(commands) = self.socket_io_commands.as_ref() {
            if namespace_connected {
                match commands.try_send(ConnectionCommand::Text("41".into())) {
                    Ok(()) => {}
                    Err(TrySendError::Full(_)) => queue_full = true,
                    Err(TrySendError::Closed(_)) => channel_closed = true,
                }
            }

            match commands.try_send(ConnectionCommand::Close) {
                Ok(()) => {}
                Err(TrySendError::Full(_)) => queue_full = true,
                Err(TrySendError::Closed(_)) => channel_closed = true,
            }
        } else {
            channel_closed = true;
        }

        if queue_full {
            self.notice = Some(t!("ApiTest.socketio_queue_full").to_string());
            if let Some(cancel) = self.socket_io_cancel.take() {
                _ = cancel.send(());
            }
            self.socket_io_commands = None;
        } else if channel_closed {
            self.finish_socket_io_transport();
        } else {
            self.notice = None;
        }
        cx.notify();
    }

    pub(in crate::api_test_view) fn cancel_socket_io(&mut self) {
        if let Some(cancel) = self.socket_io_cancel.take() {
            _ = cancel.send(());
        }
        self.socket_io_commands = None;
        self.socket_io_generation = self.socket_io_generation.wrapping_add(1);
        self.socket_io_state.cancel(self.socket_io_generation);
        self.sending = false;
    }

    pub(in crate::api_test_view) fn send_socket_io_message(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let input = self
            .socket_io_message_input
            .read(cx)
            .value()
            .trim_end_matches(['\r', '\n'])
            .to_string();
        if input.trim().is_empty() || !self.socket_io_state.state.is_connected() {
            return;
        }
        let packet = match encode_event_input(&input) {
            Ok(packet) => packet,
            Err(error) => {
                self.notice = Some(error.to_string());
                cx.notify();
                return;
            }
        };
        let Some(commands) = self.socket_io_commands.as_ref() else {
            return;
        };
        match commands.try_send(ConnectionCommand::Text(packet.clone())) {
            Ok(()) => {
                self.socket_io_state.push_sent_text(packet);
                self.socket_io_message_input.update(cx, |state, cx| {
                    state.set_value("", window, cx);
                });
                self.notice = None;
            }
            Err(TrySendError::Full(_)) => {
                self.notice = Some(t!("ApiTest.socketio_queue_full").to_string());
            }
            Err(TrySendError::Closed(_)) => {
                self.notice = Some(t!("ApiTest.socketio_connection_closed").to_string());
                self.finish_socket_io_transport();
            }
        }
        cx.notify();
    }

    fn fail_socket_io_connection(&mut self, error: String, cx: &mut Context<Self>) {
        self.socket_io_generation = self.socket_io_generation.wrapping_add(1);
        let generation = self.socket_io_generation;
        self.socket_io_state.begin(generation);
        self.socket_io_state.fail(error.clone());
        self.notice = Some(error);
        self.sending = false;
        self.clear_socket_io_transport();
        cx.notify();
    }

    fn finish_socket_io_transport(&mut self) {
        self.clear_socket_io_transport();
        self.socket_io_state.finish(self.socket_io_generation);
        self.sending = false;
    }

    fn clear_socket_io_transport(&mut self) {
        self.socket_io_commands = None;
        self.socket_io_cancel = None;
    }
}
