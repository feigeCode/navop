use std::time::Instant;

use gpui::{Context, Subscription, Task, Window};

use super::{POLL_INTERVAL, SmokeView};
use crate::cli::Config;
use crate::windows_app::session;

pub(super) fn spawn_poll_task(cx: &mut Context<SmokeView>) -> Task<()> {
    cx.spawn(async move |view, cx| {
        loop {
            cx.background_executor().timer(POLL_INTERVAL).await;
            if view
                .update_in(cx, |view, window, cx| view.poll_host(window, cx))
                .is_err()
            {
                break;
            }
        }
    })
}

pub(super) fn defer_initialization(
    config: Config,
    window: &mut Window,
    cx: &mut Context<SmokeView>,
) {
    cx.defer_in(window, move |view, window, cx| {
        println!("initialize: deferred GPUI window callback started");
        let (session, status, last_bounds) = session::initialize(config, window);
        view.session = session;
        view.status = status;
        view.started_at = Instant::now();
        view.last_bounds = last_bounds;
        cx.notify();
    });
}

pub(super) fn observe_bounds(window: &mut Window, cx: &mut Context<SmokeView>) -> Subscription {
    cx.observe_window_bounds(window, |view, window, cx| {
        view.synchronize_presentation(window);
        cx.notify();
    })
}

pub(super) fn register_close_handler(window: &mut Window, cx: &mut Context<SmokeView>) {
    let weak_view = cx.entity().downgrade();
    window.on_window_should_close(cx, move |_window, cx| {
        weak_view
            .update(cx, |view, _cx| view.prepare_close())
            .unwrap_or(true)
    });
}
