use gpui::{Context, Window};
use one_core::gpui_tokio::Tokio;
use rust_i18n::t;
use tokio::sync::mpsc::error::TrySendError;

use super::SendContext;
use crate::api_test_view::ApiTestView;
use crate::api_test_view::tcp_state::EventDisposition;
use crate::tcp::{
    ConnectionCommand, ConnectionTask, decode_payload, start_connection, target_from_url,
};

impl ApiTestView {
    pub(super) fn connect_tcp(&mut self, context: SendContext, cx: &mut Context<Self>) {
        self.cancel_tcp();
        let target = match target_from_url(&context.prepared.url) {
            Ok(target) => target,
            Err(error) => {
                self.fail_tcp_connection(error.to_string(), cx);
                return;
            }
        };
        self.tcp_generation = self.tcp_generation.wrapping_add(1);
        let generation = self.tcp_generation;
        self.tcp_state.begin(generation);
        self.prepared_request = Some(context.prepared);
        self.sending = true;

        let ConnectionTask {
            commands,
            events,
            cancel,
        } = start_connection(&Tokio::handle(cx), target);
        self.tcp_commands = Some(commands);
        self.tcp_cancel = Some(cancel);
        self.spawn_tcp_events(events, generation, cx);
        cx.notify();
    }

    fn spawn_tcp_events(
        &self,
        mut events: tokio::sync::mpsc::Receiver<crate::tcp::ConnectionEvent>,
        generation: u64,
        cx: &mut Context<Self>,
    ) {
        cx.spawn(async move |this, cx| {
            while let Some(event) = events.recv().await {
                let disposition = this.update(cx, |view, cx| {
                    let disposition = view.tcp_state.apply_event(generation, event);
                    if disposition != EventDisposition::Ignored {
                        view.sending = false;
                        if disposition == EventDisposition::Terminal {
                            view.clear_tcp_transport();
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
                if view.tcp_state.finish(generation) {
                    view.clear_tcp_transport();
                    view.sending = false;
                    cx.notify();
                }
            });
        })
        .detach();
    }

    pub(in crate::api_test_view) fn disconnect_tcp(&mut self, cx: &mut Context<Self>) {
        let result = self
            .tcp_commands
            .as_ref()
            .map(|commands| commands.try_send(ConnectionCommand::Close));
        match result {
            Some(Ok(())) => {
                self.tcp_state.set_closing();
                self.notice = None;
            }
            Some(Err(TrySendError::Full(_))) => {
                self.notice = Some(t!("ApiTest.tcp_queue_full").to_string());
            }
            Some(Err(TrySendError::Closed(_))) | None => self.finish_tcp_transport(),
        }
        cx.notify();
    }

    pub(in crate::api_test_view) fn cancel_tcp(&mut self) {
        if let Some(cancel) = self.tcp_cancel.take() {
            _ = cancel.send(());
        }
        self.tcp_commands = None;
        self.tcp_generation = self.tcp_generation.wrapping_add(1);
        self.tcp_state.cancel(self.tcp_generation);
        self.sending = false;
    }

    pub(in crate::api_test_view) fn send_tcp_message(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let message = self
            .tcp_message_input
            .read(cx)
            .value()
            .trim_end_matches(['\r', '\n'])
            .to_string();
        if message.trim().is_empty() || !self.tcp_state.state.is_connected() {
            return;
        }
        let bytes = match decode_payload(&message) {
            Ok(bytes) => bytes,
            Err(error) => {
                self.notice = Some(error.to_string());
                cx.notify();
                return;
            }
        };
        let Some(commands) = self.tcp_commands.as_ref() else {
            return;
        };
        match commands.try_send(ConnectionCommand::Send(bytes.clone())) {
            Ok(()) => {
                self.tcp_state.push_sent(bytes);
                self.tcp_message_input.update(cx, |state, cx| {
                    state.set_value("", window, cx);
                });
                self.notice = None;
            }
            Err(TrySendError::Full(_)) => {
                self.notice = Some(t!("ApiTest.tcp_queue_full").to_string());
            }
            Err(TrySendError::Closed(_)) => {
                self.notice = Some(t!("ApiTest.tcp_connection_closed").to_string());
                self.finish_tcp_transport();
            }
        }
        cx.notify();
    }

    fn fail_tcp_connection(&mut self, error: String, cx: &mut Context<Self>) {
        self.tcp_generation = self.tcp_generation.wrapping_add(1);
        let generation = self.tcp_generation;
        self.tcp_state.begin(generation);
        self.tcp_state.apply_event(
            generation,
            crate::tcp::ConnectionEvent::Error(error.clone()),
        );
        self.notice = Some(error);
        self.sending = false;
        self.clear_tcp_transport();
        cx.notify();
    }

    fn finish_tcp_transport(&mut self) {
        self.clear_tcp_transport();
        self.tcp_state.finish(self.tcp_generation);
        self.sending = false;
    }

    fn clear_tcp_transport(&mut self) {
        self.tcp_commands = None;
        self.tcp_cancel = None;
    }
}
