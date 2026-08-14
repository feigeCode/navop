use std::time::Duration;

use connection_form::credential::resolve_connection_for_runtime;
use gpui::prelude::FluentBuilder;
use gpui::{
    AppContext, AsyncApp, Context, IntoElement, ParentElement, Styled, WeakEntity, Window, div, px,
};
use gpui_component::{
    ActiveTheme, Disableable, Sizable,
    button::{Button, ButtonVariants as _},
    h_flex,
    scroll::ScrollableElement,
};
use one_core::{
    gpui_tokio::Tokio,
    storage::{RemoteDesktopProtocol, StoredConnection},
};
use rust_i18n::t;

use super::RemoteDesktopFormWindow;

const CONNECTION_TEST_TIMEOUT: Duration = Duration::from_secs(15);

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum ConnectionTestResult {
    Success,
    Failure(String),
}

#[derive(Default)]
pub(super) struct ConnectionTestState {
    generation: u64,
    is_testing: bool,
    result: Option<ConnectionTestResult>,
}

impl ConnectionTestState {
    fn begin(&mut self) -> Option<u64> {
        if self.is_testing {
            return None;
        }
        self.generation = self.generation.wrapping_add(1);
        self.is_testing = true;
        self.result = None;
        Some(self.generation)
    }

    fn complete(&mut self, generation: u64, result: ConnectionTestResult) -> bool {
        if generation != self.generation || !self.is_testing {
            return false;
        }
        self.is_testing = false;
        self.result = Some(result);
        true
    }

    fn fail_validation(&mut self, reason: String) {
        self.is_testing = false;
        self.result = Some(ConnectionTestResult::Failure(reason));
    }

    pub(super) fn is_testing(&self) -> bool {
        self.is_testing
    }

    pub(super) fn result(&self) -> Option<&ConnectionTestResult> {
        self.result.as_ref()
    }
}

impl RemoteDesktopFormWindow {
    pub(super) fn on_test_connection(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.connection_test.is_testing() {
            return;
        }
        let params = match self.build_params(cx) {
            Ok(params) => params,
            Err(reason) => {
                self.connection_test.fail_validation(reason);
                cx.notify();
                return;
            }
        };
        let params = match resolve_connection_for_runtime(
            StoredConnection::new_remote_desktop(self.connection_name(&params, cx), params, None),
            cx,
        )
        .and_then(|connection| {
            connection
                .to_remote_desktop_params()
                .map_err(|error| error.to_string())
        }) {
            Ok(params) => params,
            Err(reason) => {
                self.connection_test.fail_validation(reason);
                cx.notify();
                return;
            }
        };
        let Some(generation) = self.connection_test.begin() else {
            return;
        };
        self.error = None;
        cx.notify();

        let options = remote_desktop::RemoteDesktopConnectionOptions::from_storage_params(params);
        let window_handle = window.window_handle();
        cx.spawn(async move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            let spawn_result = Tokio::spawn_result(cx, async move {
                let result = tokio::task::spawn_blocking(move || {
                    remote_desktop::test_connection(options, CONNECTION_TEST_TIMEOUT)
                })
                .await?;
                result.map_err(anyhow::Error::new)
            })
            .await;
            let result = match spawn_result {
                Ok(()) => ConnectionTestResult::Success,
                Err(error) => ConnectionTestResult::Failure(error.to_string()),
            };

            let _ = cx.update_window(window_handle, |_, _, cx| {
                let _ = this.update(cx, |this, cx| {
                    if this.connection_test.complete(generation, result) {
                        cx.notify();
                    }
                });
            });
        })
        .detach();
    }

    pub(super) fn render_connection_test_result(
        &self,
        result: ConnectionTestResult,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        match result {
            ConnectionTestResult::Success => h_flex().justify_center().px_6().pb_2().child(
                div()
                    .text_sm()
                    .text_color(cx.theme().success)
                    .child(t!("RemoteDesktopForm.credentials_valid").to_string()),
            ),
            ConnectionTestResult::Failure(reason) => h_flex().px_6().pb_2().child(
                div()
                    .w_full()
                    .max_h(px(120.0))
                    .overflow_y_scrollbar()
                    .px_3()
                    .py_2()
                    .rounded_md()
                    .bg(cx.theme().danger.opacity(0.12))
                    .text_sm()
                    .text_color(cx.theme().danger)
                    .child(reason),
            ),
        }
    }

    pub(super) fn render_footer(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let is_testing = self.connection_test.is_testing();
        h_flex()
            .justify_end()
            .gap_2()
            .px_6()
            .py_4()
            .border_t_1()
            .border_color(cx.theme().border)
            .when(self.protocol == RemoteDesktopProtocol::Rdp, |footer| {
                footer.child(
                    Button::new("test-remote-desktop")
                        .small()
                        .outline()
                        .loading(is_testing)
                        .disabled(is_testing)
                        .label(
                            if is_testing {
                                t!("RemoteDesktopForm.testing_connection")
                            } else {
                                t!("RemoteDesktopForm.test_connection")
                            }
                            .to_string(),
                        )
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.on_test_connection(window, cx);
                        })),
                )
            })
            .child(
                Button::new("cancel-remote-desktop")
                    .small()
                    .label(t!("Common.cancel").to_string())
                    .on_click(cx.listener(|_, _, window, _| window.remove_window())),
            )
            .child(
                Button::new("save-remote-desktop")
                    .small()
                    .primary()
                    .disabled(is_testing)
                    .label(t!("Common.ok").to_string())
                    .on_click(cx.listener(|this, _, window, cx| this.on_save(window, cx))),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::{ConnectionTestResult, ConnectionTestState};

    #[test]
    fn repeated_begin_is_rejected_while_test_is_running() {
        let mut state = ConnectionTestState::default();

        assert_eq!(Some(1), state.begin());
        assert_eq!(None, state.begin());
    }

    #[test]
    fn stale_completion_does_not_replace_current_test() {
        let mut state = ConnectionTestState::default();
        let first = state.begin().unwrap();
        assert!(state.complete(first, ConnectionTestResult::Success));
        let second = state.begin().unwrap();

        assert!(!state.complete(first, ConnectionTestResult::Failure("stale".to_string())));
        assert!(state.is_testing());
        assert_eq!(None, state.result());
        assert_eq!(2, second);
    }

    #[test]
    fn current_completion_updates_state() {
        let mut state = ConnectionTestState::default();
        let generation = state.begin().unwrap();

        assert!(state.complete(generation, ConnectionTestResult::Success));
        assert!(!state.is_testing());
        assert_eq!(Some(&ConnectionTestResult::Success), state.result());
    }

    #[test]
    fn new_test_clears_previous_result() {
        let mut state = ConnectionTestState::default();
        let generation = state.begin().unwrap();
        state.complete(generation, ConnectionTestResult::Success);

        assert_eq!(Some(2), state.begin());
        assert_eq!(None, state.result());
    }
}
