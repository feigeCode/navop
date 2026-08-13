use std::time::{Duration, Instant};

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

pub(super) fn spawn_login_presentation_refresh(
    generation: u64,
    delay: Duration,
    cx: &mut Context<SmokeView>,
) -> Task<()> {
    cx.spawn(async move |view, cx| {
        cx.background_executor().timer(delay).await;
        let _ = view.update_in(cx, |view, window, _cx| {
            let current_generation = view
                .session
                .as_ref()
                .map(|session| session.host.generation());
            if current_generation != Some(generation) || !view.login_complete {
                println!(
                    "presentation: delayed login refresh skipped scheduled_generation={generation} current_generation={current_generation:?} login_complete={}",
                    view.login_complete
                );
                return;
            }
            println!(
                "presentation: delayed login refresh running generation={generation} delay_ms={}",
                delay.as_millis()
            );
            view.refresh_connected_presentation(
                window,
                "host refreshed 300ms after login complete",
            );
        });
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
