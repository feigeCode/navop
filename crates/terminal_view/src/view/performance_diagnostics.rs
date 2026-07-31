use super::*;
use std::time::Instant;

const TERMINAL_PERFORMANCE_LOG_INTERVAL: Duration = Duration::from_secs(5);

impl TerminalView {
    pub(super) fn start_performance_diagnostics(
        &self,
        connection_id: Option<i64>,
        connection_kind: TerminalConnectionKind,
        cx: &mut Context<Self>,
    ) {
        let Some(metrics) = self.performance_metrics.clone() else {
            return;
        };

        cx.spawn(async move |this, cx| {
            let mut previous = metrics.snapshot_for_window();
            let mut previous_at = Instant::now();

            loop {
                cx.background_executor()
                    .timer(TERMINAL_PERFORMANCE_LOG_INTERVAL)
                    .await;

                if this.read_with(cx, |_, _| ()).is_err() {
                    break;
                }

                let now = Instant::now();
                let current = metrics.snapshot_for_window();
                let window =
                    current.delta_since(&previous, now.saturating_duration_since(previous_at));

                tracing::debug!(
                    target: "terminal_performance",
                    ?connection_kind,
                    ?connection_id,
                    ingress_bytes = window.ingress_bytes,
                    ingress_bytes_per_second = window.ingress_bytes_per_second,
                    ingress_pending_bytes = window.ingress_pending_bytes,
                    ingress_pending_bytes_window_max = window.ingress_pending_bytes_window_max,
                    ingress_pending_bytes_lifetime_max = window.ingress_pending_bytes_lifetime_max,
                    parser_chunks = window.parser_chunks,
                    average_parser_chunk_bytes = window.average_parser_chunk_bytes,
                    user_input_bytes = window.user_input_bytes,
                    terminal_response_bytes = window.terminal_response_bytes,
                    term_lock_samples = window.term_lock_samples,
                    average_term_lock_wait_ns = window.average_term_lock_wait_ns,
                    average_term_lock_hold_ns = window.average_term_lock_hold_ns,
                    wakeup_requests = window.wakeup_requests,
                    wakeup_queued = window.wakeup_queued,
                    wakeup_coalesced = window.wakeup_coalesced,
                    render_samples = window.render_samples,
                    average_render_ns = window.average_render_ns,
                    ssh_connects = window.ssh_connects,
                    ssh_reconnects = window.ssh_reconnects,
                    ssh_invalidations = window.ssh_invalidations,
                    activity = ?current.activity(),
                    "terminal performance window"
                );

                previous = current;
                previous_at = now;
            }
        })
        .detach();
    }
}
