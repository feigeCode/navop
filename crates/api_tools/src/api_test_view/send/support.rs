use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use super::SendContext;
use crate::api_test_view::ApiTestView;
use crate::http::HttpResponse;
use crate::request_store::{RequestHistoryEntry, StoredRequest};
use crate::scripting::{self, ScriptResult};
use crate::sse::SseProgress;

const STREAM_POLL_INTERVAL_MS: u64 = 120;

pub(super) struct CompletedRequest {
    pub(super) request: StoredRequest,
    pub(super) history_url: String,
    pub(super) response: HttpResponse,
    pub(super) test_result: Option<ScriptResult>,
    pub(super) generation: u64,
}

pub(super) struct SsePollContext {
    generation: u64,
    progress: Arc<Mutex<SseProgress>>,
    stop: Arc<AtomicBool>,
}

impl SsePollContext {
    pub(super) fn new(
        generation: u64,
        progress: Arc<Mutex<SseProgress>>,
        stop: Arc<AtomicBool>,
    ) -> Self {
        Self {
            generation,
            progress,
            stop,
        }
    }
}

pub(super) struct StreamStopGuard(Arc<AtomicBool>);

impl StreamStopGuard {
    pub(super) fn new(stop: Arc<AtomicBool>) -> Self {
        Self(stop)
    }
}

impl Drop for StreamStopGuard {
    fn drop(&mut self) {
        self.0.store(true, Ordering::SeqCst);
    }
}

pub(super) async fn poll_sse_progress(
    view: &gpui::WeakEntity<ApiTestView>,
    context: SsePollContext,
    cx: &mut gpui::AsyncWindowContext,
) {
    loop {
        smol::Timer::after(Duration::from_millis(STREAM_POLL_INTERVAL_MS)).await;
        let Some(snapshot) = context.progress.lock().ok().map(|value| value.clone()) else {
            return;
        };
        let done = snapshot.done;
        let update = view.update_in(cx, |this, _, cx| {
            if snapshot.response.streaming || done {
                if this.request_generation == context.generation {
                    this.response = Some(snapshot.response);
                    cx.notify();
                }
            }
        });
        if update.is_err() {
            context.stop.store(true, Ordering::SeqCst);
            return;
        }
        if done {
            return;
        }
    }
}

pub(super) fn complete_request(context: SendContext, response: HttpResponse) -> CompletedRequest {
    let test_result = if context.request.tests.trim().is_empty() {
        None
    } else {
        Some(scripting::run_post_request(
            &context.request.tests,
            &context.vars,
            &response,
        ))
    };
    CompletedRequest {
        history_url: context.prepared.url,
        request: context.request,
        response,
        test_result,
        generation: context.generation,
    }
}

pub(super) fn apply_variable_effects(vars: &mut BTreeMap<String, String>, result: &ScriptResult) {
    for effect in &result.effects {
        if let scripting::SideEffect::SetVariable { name, value, .. } = effect {
            vars.insert(name.clone(), value.clone());
        }
    }
}

pub(super) fn push_history(history: &mut Vec<RequestHistoryEntry>, completion: &CompletedRequest) {
    crate::history::push_history(
        history,
        RequestHistoryEntry {
            id: uuid::Uuid::new_v4().simple().to_string(),
            sent_at: chrono::Utc::now().timestamp_millis(),
            request_id: Some(completion.request.id.clone()),
            request_name: completion.request.name.clone(),
            method: completion.request.method,
            url: completion.history_url.clone(),
            status: completion.response.status,
            status_text: completion.response.status_text.clone(),
            time_ms: completion.response.time_ms,
            size: completion.response.size,
            error: completion.response.error.clone(),
            request: completion.request.clone(),
        },
    );
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};

    use super::StreamStopGuard;

    #[test]
    fn dropping_stream_stop_guard_requests_transport_cancellation() {
        let stop = Arc::new(AtomicBool::new(false));
        drop(StreamStopGuard::new(stop.clone()));
        assert!(stop.load(Ordering::SeqCst));
    }
}
