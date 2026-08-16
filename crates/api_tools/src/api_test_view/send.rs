use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use gpui::{AppContext as _, Context, Window};
use rust_i18n::t;

use super::{ApiTestView, REQUEST_TIMEOUT_SECS, ResponseTab};
use crate::Protocol;
use crate::http::{self, HttpResponse, KeyValue, PreparedRequest};
use crate::request_store::{self, StoredRequest};
use crate::scripting::{self, ScriptResult};
use crate::sse::{self, SseProgress};
use crate::tree_model::ancestor_folder_ids;

mod grpc_web;
mod socket_io;
mod support;
mod tcp;
mod websocket;

use support::{
    CompletedRequest, SsePollContext, StreamStopGuard, apply_variable_effects, complete_request,
    poll_sse_progress,
};

struct SendContext {
    request: StoredRequest,
    prepared: PreparedRequest,
    vars: BTreeMap<String, String>,
    generation: u64,
}

impl ApiTestView {
    pub fn send(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.stream_stop.is_some() {
            self.stop_stream(cx);
            return;
        }
        if self.sending {
            return;
        }
        let Some(context) = self.build_send_context(window, cx) else {
            return;
        };
        match context.request.protocol {
            Protocol::Http | Protocol::Graphql => self.send_http(context, window, cx),
            Protocol::GrpcWeb => self.send_grpc_web(context, window, cx),
            Protocol::Sse => self.send_sse(context, window, cx),
            Protocol::Tcp => self.connect_tcp(context, cx),
            Protocol::WebSocket => self.connect_websocket(context, cx),
            Protocol::SocketIo => self.connect_socket_io(context, cx),
        }
    }

    fn build_send_context(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<SendContext> {
        self.commit_current_to_store(cx);
        let request = self.snapshot_request(cx);
        if !matches!(
            request.protocol,
            Protocol::Http
                | Protocol::Graphql
                | Protocol::GrpcWeb
                | Protocol::Sse
                | Protocol::Tcp
                | Protocol::WebSocket
                | Protocol::SocketIo
        ) {
            self.reject_unsupported_protocol(request.protocol, cx);
            return None;
        }
        let mut vars = self.effective_vars(&request);
        self.inject_base_url(&request, &mut vars);
        let pre_result = self.evaluate_pre_request(&request, &mut vars, cx)?;
        self.apply_pre_request_effects(pre_result.as_ref(), window, cx);
        let prepared = self.prepare_request(&request, &vars, cx)?;
        let generation = self.begin_send(&prepared, pre_result, cx);
        Some(SendContext {
            request,
            prepared,
            vars,
            generation,
        })
    }

    fn evaluate_pre_request(
        &mut self,
        request: &StoredRequest,
        vars: &mut BTreeMap<String, String>,
        cx: &mut Context<Self>,
    ) -> Option<Option<ScriptResult>> {
        if request.pre_script.trim().is_empty() {
            return Some(None);
        }
        let result = scripting::run_pre_request(&request.pre_script, vars);
        if let Some(error) = &result.error {
            self.pre_result = Some(result.clone());
            self.notice = Some(format!("{}: {error}", t!("ApiTest.pre_request_failed")));
            self.active_response_tab = ResponseTab::Console;
            cx.notify();
            return None;
        }
        apply_variable_effects(vars, &result);
        Some(Some(result))
    }

    fn apply_pre_request_effects(
        &mut self,
        result: Option<&ScriptResult>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(result) = result else {
            return;
        };
        if self.persist_environment_effects(result) {
            self.refresh_environment_rows(window, cx);
        }
        self.save_store();
    }

    fn prepare_request(
        &mut self,
        request: &StoredRequest,
        vars: &BTreeMap<String, String>,
        cx: &mut Context<Self>,
    ) -> Option<PreparedRequest> {
        let default_scheme = match request.protocol {
            Protocol::Tcp => "tcp",
            Protocol::WebSocket | Protocol::SocketIo => "ws",
            _ => "http",
        };
        let params = self.effective_folder_params(request);
        let headers = self.effective_folder_headers(request);
        let cookies = self.effective_cookies(request);
        let result = http::prepare_full_with_default_scheme(
            request.method,
            &request.url,
            &params,
            &headers,
            &cookies,
            &request.auth,
            request.body_type,
            &request.body,
            &request.body_rows,
            request.raw_language,
            vars,
            default_scheme,
        );
        match result {
            Ok(prepared) => {
                let prepared = match request.protocol {
                    Protocol::GrpcWeb => {
                        crate::grpc_web::prepare_grpc_web_request(prepared, REQUEST_TIMEOUT_SECS)
                    }
                    Protocol::Tcp => Ok(crate::tcp::prepare_tcp_request(prepared)),
                    Protocol::WebSocket => {
                        Ok(crate::websocket::prepare_websocket_request(prepared))
                    }
                    Protocol::SocketIo => crate::socket_io::prepare_socket_io_request(prepared),
                    _ => Ok(prepared),
                };
                match prepared {
                    Ok(prepared) => Some(prepared),
                    Err(error) => {
                        self.notice = Some(format!("{}: {error}", t!("ApiTest.request_failed")));
                        cx.notify();
                        None
                    }
                }
            }
            Err(error) => {
                self.notice = Some(format!("{}: {error}", t!("ApiTest.request_failed")));
                cx.notify();
                None
            }
        }
    }

    fn effective_folder_params(&self, request: &StoredRequest) -> Vec<KeyValue> {
        let mut scopes = vec![self.global_params.as_slice()];
        scopes.extend(
            ancestor_folder_ids(&self.folders, request.folder_id.as_deref())
                .into_iter()
                .filter_map(|folder_id| {
                    self.folders
                        .iter()
                        .find(|folder| folder.id == folder_id)
                        .map(|folder| folder.params.as_slice())
                }),
        );
        merge_inherited_key_values(&scopes, &request.params)
    }

    fn effective_folder_headers(&self, request: &StoredRequest) -> Vec<KeyValue> {
        let mut scopes = vec![self.global_headers.as_slice()];
        scopes.extend(
            ancestor_folder_ids(&self.folders, request.folder_id.as_deref())
                .into_iter()
                .filter_map(|folder_id| {
                    self.folders
                        .iter()
                        .find(|folder| folder.id == folder_id)
                        .map(|folder| folder.headers.as_slice())
                }),
        );
        merge_inherited_key_values(&scopes, &request.header_rows)
    }

    fn effective_cookies(&self, request: &StoredRequest) -> Vec<KeyValue> {
        merge_inherited_key_values(&[self.global_cookies.as_slice()], &request.cookies)
    }

    fn inject_base_url(&self, request: &StoredRequest, vars: &mut BTreeMap<String, String>) {
        let base = match &request.base_url_override {
            Some(Some(value)) => Some(
                http::substitute(value, vars)
                    .trim_end_matches('/')
                    .to_string(),
            ),
            Some(None) => None,
            None => self.effective_folder_base_url(request, vars).map(|base| {
                http::substitute(&base, vars)
                    .trim_end_matches('/')
                    .to_string()
            }),
        };
        if let Some(base) = base.filter(|base| !base.trim().is_empty()) {
            vars.insert("__folder_base_url__".to_string(), base.clone());
            vars.entry("baseUrl".to_string()).or_insert(base);
        }
    }

    fn effective_folder_base_url(
        &self,
        request: &StoredRequest,
        vars: &BTreeMap<String, String>,
    ) -> Option<String> {
        ancestor_folder_ids(&self.folders, request.folder_id.as_deref())
            .into_iter()
            .rev()
            .filter_map(|folder_id| {
                self.folders
                    .iter()
                    .find(|folder| folder.id == folder_id)
                    .and_then(|folder| folder.base_url.as_deref())
            })
            .find(|base| !base.trim().is_empty())
            .map(|base| http::substitute(base, vars))
    }

    fn begin_send(
        &mut self,
        prepared: &PreparedRequest,
        pre_result: Option<ScriptResult>,
        cx: &mut Context<Self>,
    ) -> u64 {
        self.sending = true;
        self.request_generation = self.request_generation.wrapping_add(1);
        self.response = None;
        self.prepared_request = Some(prepared.clone());
        self.pre_result = pre_result;
        self.test_result = None;
        self.notice = None;
        cx.notify();
        self.request_generation
    }

    fn send_http(&mut self, context: SendContext, window: &mut Window, cx: &mut Context<Self>) {
        let client = cx.http_client();
        let task = cx.background_spawn(async move {
            let response = http::execute(
                client.as_ref(),
                context.prepared.clone(),
                REQUEST_TIMEOUT_SECS,
            )
            .await;
            complete_request(context, response)
        });
        cx.spawn_in(window, async move |this, cx| {
            let completion = task.await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.finish_request(completion, window, cx);
            });
        })
        .detach();
    }

    fn send_sse(&mut self, context: SendContext, window: &mut Window, cx: &mut Context<Self>) {
        let client = cx.http_client();
        let stop = Arc::new(AtomicBool::new(false));
        let progress = Arc::new(Mutex::new(SseProgress::default()));
        self.stream_stop = Some(stop.clone());
        self.response = Some(HttpResponse {
            streaming: true,
            ..Default::default()
        });
        cx.notify();

        let poll_progress = progress.clone();
        let generation = context.generation;
        let transport_stop = stop.clone();
        let poll_stop = stop.clone();
        let task = cx.background_spawn(async move {
            let response = sse::execute(
                client.as_ref(),
                context.prepared.clone(),
                REQUEST_TIMEOUT_SECS,
                transport_stop,
                progress,
            )
            .await;
            complete_request(context, response)
        });
        cx.spawn_in(window, async move |this, cx| {
            let _stop_on_drop = StreamStopGuard::new(poll_stop.clone());
            poll_sse_progress(
                &this,
                SsePollContext::new(generation, poll_progress, poll_stop),
                cx,
            )
            .await;
            let completion = task.await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.finish_request(completion, window, cx);
            });
        })
        .detach();
    }

    pub(super) fn stop_stream(&mut self, cx: &mut Context<Self>) {
        if let Some(stop) = &self.stream_stop {
            stop.store(true, Ordering::SeqCst);
            cx.notify();
        }
    }

    pub(super) fn cancel_stream(&mut self) {
        if let Some(stop) = self.stream_stop.take() {
            stop.store(true, Ordering::SeqCst);
        }
    }

    fn reject_unsupported_protocol(&mut self, protocol: Protocol, cx: &mut Context<Self>) {
        self.sending = false;
        self.notice = Some(
            t!(
                "ApiTest.protocol_not_implemented",
                protocol = protocol.label()
            )
            .to_string(),
        );
        cx.notify();
    }

    fn finish_request(
        &mut self,
        completion: CompletedRequest,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.request_generation != completion.generation {
            return;
        }
        self.sending = false;
        self.stream_stop = None;
        if let Some(request) = self
            .requests
            .iter_mut()
            .find(|request| request.id == completion.request.id)
        {
            request.last_response = Some(completion.response.clone());
            request_store::apply_response_example_autosave(
                request,
                &completion.response,
                self.response_example_autosave,
            );
        }
        support::push_history(&mut self.history, &completion);
        self.response = Some(completion.response);
        if let Some(result) = &completion.test_result
            && self.persist_environment_effects(result)
        {
            self.refresh_environment_rows(window, cx);
        }
        self.test_result = completion.test_result;
        self.save_store();
        cx.notify();
    }
}

fn merge_inherited_key_values(
    folder_scopes: &[&[KeyValue]],
    request: &[KeyValue],
) -> Vec<KeyValue> {
    let mut merged = Vec::new();
    for scope in folder_scopes
        .iter()
        .copied()
        .chain(std::iter::once(request))
    {
        let overrides = scope
            .iter()
            .filter(|row| row.enabled && !row.key.trim().is_empty())
            .map(|row| normalize_key(&row.key))
            .collect::<std::collections::HashSet<_>>();
        merged.retain(|row: &KeyValue| !overrides.contains(&normalize_key(&row.key)));
        merged.extend(
            scope
                .iter()
                .filter(|row| row.enabled && !row.key.trim().is_empty())
                .cloned(),
        );
    }
    merged
}

fn normalize_key(key: &str) -> String {
    key.trim().to_ascii_uppercase()
}

#[cfg(test)]
mod tests {
    use super::merge_inherited_key_values;
    use crate::http::KeyValue;

    #[test]
    fn inherited_key_values_follow_global_folder_and_request_precedence() {
        let global = vec![
            KeyValue::new("tenant", "global"),
            KeyValue::new("global-only", "yes"),
        ];
        let parent = vec![
            KeyValue::new("tenant", "parent"),
            KeyValue::new("parent-only", "yes"),
        ];
        let child = vec![KeyValue::new("tenant", "child")];
        let request = vec![KeyValue::new("TENANT", "request")];

        let merged = merge_inherited_key_values(
            &[global.as_slice(), parent.as_slice(), child.as_slice()],
            &request,
        );

        assert_eq!(
            merged,
            vec![
                KeyValue::new("global-only", "yes"),
                KeyValue::new("parent-only", "yes"),
                KeyValue::new("TENANT", "request"),
            ]
        );
    }

    #[test]
    fn disabled_and_empty_rows_are_not_sent_or_used_as_overrides() {
        let parent = vec![
            KeyValue::new("keep", "parent"),
            KeyValue {
                key: "disabled".into(),
                value: "ignored".into(),
                enabled: false,
                ..KeyValue::default()
            },
        ];
        let request = vec![
            KeyValue {
                key: "keep".into(),
                value: "ignored".into(),
                enabled: false,
                ..KeyValue::default()
            },
            KeyValue::new(" ", "ignored"),
        ];

        assert_eq!(
            merge_inherited_key_values(&[parent.as_slice()], &request),
            vec![KeyValue::new("keep", "parent")]
        );
    }

    #[test]
    fn header_keys_are_case_insensitive() {
        let folder = vec![KeyValue::new("Authorization", "folder-token")];
        let request = vec![KeyValue::new("authorization", "request-token")];

        assert_eq!(
            merge_inherited_key_values(&[folder.as_slice()], &request),
            vec![KeyValue::new("authorization", "request-token")]
        );
    }
}
